//! Acceptance checks using production discovery, rendering, and installer entry points.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;
use std::time::UNIX_EPOCH;

use crate::capture_root::RootOwner;
use crate::capture_root::SharedCaptureDirectory;
use crate::config::Config;
use crate::constants::SHIM_MARKER;
use crate::hook::AccountInstallOutcome;
use crate::hook::InstallAccount;
use crate::hook::account_groups;
use crate::hook::install_accounts;
use crate::hook::stage_installer;
use crate::processes::AccountName;
use crate::processes::RootReadStatus;
use crate::progress::Capture;
use crate::progress::CaptureCleanup;
use crate::progress::CaptureRoots;
use crate::settings::capture_root_status;
use crate::settings::shared_directory_status;

/// Every fixture account uses the caller's credentials, so no root access is needed.
fn account_at(home: &Path, name: &str) -> InstallAccount {
    InstallAccount {
        name: name.to_owned(),
        uid:  rustix::process::geteuid().as_raw(),
        gid:  rustix::process::getegid().as_raw(),
        home: home.to_owned(),
    }
}

fn installer() -> &'static Path { Path::new(env!("CARGO_BIN_EXE_cargo-tile")) }

fn original_cargo(account: &InstallAccount, toolchain: &str) -> std::path::PathBuf {
    let bin = account
        .home
        .join(format!(".rustup/toolchains/{toolchain}/bin"));
    fs::create_dir_all(&bin).expect("toolchain bin");
    fs::write(bin.join("cargo"), b"#!/bin/sh\nexit 37\n").expect("original cargo");
    set_mode(&bin.join("cargo"), 0o751);
    bin
}

#[test]
fn injected_accounts_install_each_toolchain_through_the_built_binary() {
    let directory = tempfile::tempdir().expect("account fixture");
    let accounts: Vec<_> = ["developer", "runner", "empty", "absent"]
        .into_iter()
        .map(|name| account_at(&directory.path().join(name), name))
        .collect();
    for account in &accounts[..2] {
        for toolchain in ["stable", "nightly"] {
            original_cargo(account, toolchain);
        }
    }
    fs::create_dir_all(accounts[2].home.join(".rustup/toolchains"))
        .expect("empty toolchains directory");
    let reports = install_accounts(&accounts, installer());
    assert_eq!(reports.len(), 3, "accounts without .rustup have no report");
    assert_eq!(reports[0].account, "developer");
    assert_eq!(reports[1].account, "runner");
    assert_eq!(reports[2].account, "empty");
    assert_eq!(
        reports[2].outcome,
        AccountInstallOutcome::Skipped("no toolchains".to_owned())
    );
    for (account, report) in accounts.iter().zip(&reports).take(2) {
        assert_eq!(report.outcome, AccountInstallOutcome::Installed);
        for toolchain in ["stable", "nightly"] {
            let bin = account
                .home
                .join(format!(".rustup/toolchains/{toolchain}/bin"));
            assert_eq!(
                fs::read(bin.join("cargo")).expect("installed shim"),
                include_bytes!("../../src/cargo-capture-shim.sh")
            );
            let shim = fs::metadata(bin.join("cargo")).expect("shim metadata");
            assert_eq!((shim.uid(), shim.gid()), (account.uid, account.gid));
            let real = fs::metadata(bin.join("cargo-tile-real")).expect("saved cargo metadata");
            assert_eq!(real.mode() & 0o7777, 0o751);
            assert_eq!(
                fs::read(bin.join("cargo-tile-real")).expect("saved cargo"),
                b"#!/bin/sh\nexit 37\n"
            );
        }
    }
    for report in install_accounts(&accounts[..2], installer()) {
        assert_eq!(report.outcome, AccountInstallOutcome::AlreadyInstalled);
    }
}

/// The shared executable preserves installation reports and exists only while its guard lives.
#[test]
fn staged_installer_copies_executable_installs_toolchains_and_cleans_up() {
    let directory = tempfile::tempdir().expect("staged installer fixture");
    let original = account_at(&directory.path().join("original"), "runner");
    let account = account_at(&directory.path().join("staged"), "runner");
    for fixture in [&original, &account] {
        for toolchain in ["stable", "nightly"] {
            original_cargo(fixture, toolchain);
        }
    }
    let staged = stage_installer(directory.path(), installer()).expect("stage the built installer");
    let copy = staged.path().to_owned();
    let parent = copy.parent().expect("staged directory").to_owned();
    assert_eq!(parent.parent(), Some(directory.path()));
    assert_eq!(copy.file_name(), Some(std::ffi::OsStr::new("cargo-tile")));
    assert!(parent.is_dir());
    assert!(copy.is_file());
    for path in [&parent, &copy] {
        assert_eq!(
            fs::metadata(path).expect("staged permissions").mode() & 0o7777,
            0o755,
            "{} must be readable and executable by every account",
            path.display()
        );
    }
    assert_eq!(
        fs::read(&copy).expect("staged bytes"),
        fs::read(installer()).expect("original installer bytes")
    );
    let reports = install_accounts(std::slice::from_ref(&account), staged.path());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountInstallOutcome::Installed);
    assert_eq!(
        reports,
        install_accounts(std::slice::from_ref(&original), installer())
    );
    for toolchain in ["stable", "nightly"] {
        let bin = account
            .home
            .join(format!(".rustup/toolchains/{toolchain}/bin"));
        assert_eq!(
            fs::read(bin.join("cargo")).expect("staged installer writes shim"),
            include_bytes!("../../src/cargo-capture-shim.sh")
        );
        assert_eq!(
            fs::read(bin.join("cargo-tile-real")).expect("staged installer preserves cargo"),
            b"#!/bin/sh\nexit 37\n"
        );
    }
    let reports = install_accounts(std::slice::from_ref(&account), staged.path());
    assert_eq!(reports[0].outcome, AccountInstallOutcome::AlreadyInstalled);
    assert_eq!(
        reports,
        install_accounts(std::slice::from_ref(&original), installer())
    );
    drop(staged);
    assert!(!copy.exists(), "dropping the guard removes the executable");
    assert!(!parent.exists(), "dropping the guard removes its directory");
    assert!(directory.path().is_dir(), "the supplied parent survives");
}

/// A failed exec names the affected account and does not stop subsequent account reports.
#[test]
fn unstartable_installer_reports_each_account_separately() {
    let directory = tempfile::tempdir().expect("failed installer fixture");
    let accounts: Vec<_> = ["developer", "runner"]
        .into_iter()
        .map(|name| account_at(&directory.path().join(name), name))
        .collect();
    for account in &accounts {
        original_cargo(account, "stable");
    }
    let reports = install_accounts(&accounts, directory.path());
    assert_eq!(reports.len(), accounts.len());
    for (account, report) in accounts.iter().zip(&reports) {
        assert_eq!(report.account, account.name);
        let prefix = format!("could not start the installer as {}:", account.name);
        assert!(
            matches!(&report.outcome, AccountInstallOutcome::Skipped(reason)
                if reason.starts_with(&prefix) && !reason[prefix.len()..].trim().is_empty()),
            "each failed start retains the account and OS error: {report:?}"
        );
        let bin = account.home.join(".rustup/toolchains/stable/bin");
        assert_eq!(
            fs::read(bin.join("cargo")).expect("original cargo survives failed exec"),
            b"#!/bin/sh\nexit 37\n"
        );
        assert!(!bin.join("cargo-tile-real").exists());
    }
}

/// Database groups include the primary gid even when session memberships differ.
#[test]
fn resolved_account_groups_include_the_callers_primary_gid() {
    let uid = rustix::process::geteuid().as_raw();
    let users = sysinfo::Users::new_with_refreshed_list();
    let user = users
        .iter()
        .find(|user| **user.id() == uid)
        .expect("caller has an account database entry");
    let gid = *user.group_id();
    let groups = account_groups(user.name(), gid).expect("resolve the caller's groups");
    assert!(
        groups.contains(&gid),
        "{} must retain primary gid {gid} in {groups:?}",
        user.name()
    );
}

/// Account-owned files may belong to any permitted group, including a setgid parent's.
#[test]
fn account_owned_cargo_in_permitted_groups_keeps_its_saved_group() {
    let directory = tempfile::tempdir().expect("group fixture");
    let groups = Command::new("id")
        .arg("-G")
        .output()
        .expect("current process groups");
    assert!(groups.status.success(), "{groups:?}");
    let groups = String::from_utf8(groups.stdout).expect("numeric groups");
    assert!(!groups.trim().is_empty());
    for group in groups.split_whitespace() {
        let gid = group.parse::<u32>().expect("numeric gid");
        let account = account_at(&directory.path().join(group), "runner");
        let bin = original_cargo(&account, "stable");
        std::os::unix::fs::chown(bin.join("cargo"), None, Some(gid))
            .expect("assign one of the caller's permitted groups");
        std::os::unix::fs::chown(&bin, None, Some(gid)).expect("setgid directory group");
        set_mode(&bin, 0o2755);
        let reports = install_accounts(std::slice::from_ref(&account), installer());
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].outcome,
            AccountInstallOutcome::Installed,
            "group {gid}"
        );
        for name in ["cargo", "cargo-tile-real"] {
            let owner = fs::metadata(bin.join(name)).expect("installed file metadata");
            assert_eq!(owner.uid(), account.uid);
            assert_eq!(
                owner.gid(),
                gid,
                "{name} retains directory or original group"
            );
        }
        let reports = install_accounts(std::slice::from_ref(&account), installer());
        assert_eq!(reports[0].outcome, AccountInstallOutcome::AlreadyInstalled);
        assert_eq!(
            fs::metadata(bin.join("cargo"))
                .expect("current shim group")
                .gid(),
            gid
        );
    }
}

#[test]
fn injected_account_repairs_interrupted_and_refreshes_outdated_installs() {
    let directory = tempfile::tempdir().expect("account fixture");
    let account = account_at(directory.path(), "runner");
    let bin = original_cargo(&account, "stable");
    fs::rename(bin.join("cargo"), bin.join("cargo-tile-real")).expect("interrupted rename");
    fs::write(bin.join("cargo-tile-shim.staging"), b"partial shim").expect("interrupted staging");
    for outdated in [false, true] {
        if outdated {
            fs::write(
                bin.join("cargo"),
                format!("#!/bin/sh\n# {SHIM_MARKER}\n# outdated\n"),
            )
            .expect("outdated shim");
        }
        let reports = install_accounts(std::slice::from_ref(&account), installer());
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, AccountInstallOutcome::Installed);
        assert_eq!(
            fs::read(bin.join("cargo")).expect("repaired shim"),
            include_bytes!("../../src/cargo-capture-shim.sh")
        );
        assert_eq!(
            fs::read(bin.join("cargo-tile-real")).expect("saved cargo survives"),
            b"#!/bin/sh\nexit 37\n"
        );
        assert!(!bin.join("cargo-tile-shim.staging").exists());
    }
}

#[test]
fn injected_account_reports_child_errors_and_continues_other_toolchains() {
    let directory = tempfile::tempdir().expect("account fixture");
    let account = account_at(directory.path(), "runner");
    let broken = original_cargo(&account, "a-broken");
    fs::create_dir(broken.join("cargo-tile-shim.lock")).expect("lock open must fail");
    let good = original_cargo(&account, "z-working");
    let reports = install_accounts(&[account], installer());
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(&reports[0].outcome, AccountInstallOutcome::Skipped(reason)
            if reason.contains("a-broken")
                && !reason.contains('\n')
                && !reason.contains("interrupted install found")),
        "the first child error must become one actionable skipped report: {reports:?}"
    );
    assert_eq!(
        fs::read(good.join("cargo")).expect("later toolchain installs"),
        include_bytes!("../../src/cargo-capture-shim.sh")
    );
    assert_eq!(
        fs::read(broken.join("cargo")).expect("failed toolchain untouched"),
        b"#!/bin/sh\nexit 37\n"
    );
}

#[test]
fn injected_account_reports_child_toolchain_discovery_failure() {
    let directory = tempfile::tempdir().expect("account fixture");
    let account = account_at(directory.path(), "runner");
    fs::create_dir(account.home.join(".rustup")).expect("rustup exists");
    fs::write(account.home.join(".rustup/toolchains"), b"not a directory")
        .expect("child cannot list toolchains");
    let reports = install_accounts(&[account], installer());
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(&reports[0].outcome, AccountInstallOutcome::Skipped(reason)
            if !reason.is_empty()
                && !reason.contains('\n')
                && reason != "no toolchains"
                && !reason.contains("home unreadable")),
        "the child's own discovery failure must become one skipped report: {reports:?}"
    );
}

/// Observe the account child's protocol without knowing or passing its hidden flag.
#[test]
fn account_child_has_account_environment_and_only_machine_report_lines() {
    let directory = tempfile::tempdir().expect("account child fixture");
    let account = account_at(&directory.path().join("account"), "runner");
    let current = original_cargo(&account, "a-current");
    assert_eq!(
        install_accounts(std::slice::from_ref(&account), installer())[0].outcome,
        AccountInstallOutcome::Installed
    );
    original_cargo(&account, "b-new");
    let repair = original_cargo(&account, "c-repair");
    fs::rename(repair.join("cargo"), repair.join("cargo-tile-real")).expect("interrupted install");
    let orphan = original_cargo(&account, "d-orphan");
    fs::copy(current.join("cargo"), orphan.join("cargo")).expect("orphan shim without real cargo");
    let broken = original_cargo(&account, "e-error");
    fs::create_dir(broken.join("cargo-tile-shim.lock")).expect("child toolchain failure");
    let outdated = original_cargo(&account, "f-refresh");
    fs::rename(outdated.join("cargo"), outdated.join("cargo-tile-real")).expect("saved cargo");
    fs::write(
        outdated.join("cargo"),
        format!("#!/bin/sh\n# {SHIM_MARKER}\n# old\n"),
    )
    .expect("outdated shim");
    let observer = directory.path().join("observe-install");
    let binary = installer()
        .to_str()
        .expect("installer path")
        .replace('\'', "'\"'\"'");
    fs::write(
        &observer,
        format!(
            r#"#!/bin/sh
id -u > "$HOME/child-uid"
id -g > "$HOME/child-gid"
printf '%s\n' "$@" > "$HOME/child-arguments"
printf '%s\n' "$HOME" "$RUSTUP_HOME" > "$HOME/child-environment"
'{binary}' "$@" > "$HOME/child-output" 2> "$HOME/child-error"
result=$?
cat "$HOME/child-output"
cat "$HOME/child-error" >&2
exit "$result"
"#
        ),
    )
    .expect("installer observation wrapper");
    set_mode(&observer, 0o755);
    let reports = install_accounts(std::slice::from_ref(&account), &observer);
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(&reports[0].outcome, AccountInstallOutcome::Skipped(reason) if reason.contains("e-error"))
    );
    assert_eq!(
        fs::read_to_string(account.home.join("child-uid"))
            .expect("child uid")
            .trim(),
        account.uid.to_string()
    );
    assert_eq!(
        fs::read_to_string(account.home.join("child-gid"))
            .expect("child gid")
            .trim(),
        account.gid.to_string()
    );
    assert_eq!(
        fs::read_to_string(account.home.join("child-environment")).expect("child environment"),
        format!(
            "{}\n{}\n",
            account.home.display(),
            account.home.join(".rustup").display()
        )
    );
    assert!(
        fs::read(account.home.join("child-error"))
            .expect("child stderr")
            .is_empty()
    );
    let output = fs::read_to_string(account.home.join("child-output")).expect("child stdout");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 6, "one line per toolchain only: {output}");
    assert_eq!(
        &lines[..4],
        [
            "a-current\talready installed",
            "b-new\tinstalled",
            "c-repair\tinstalled",
            "d-orphan\torphaned"
        ]
    );
    assert!(lines[4].starts_with("e-error\terror\t"), "{output}");
    assert_eq!(lines[4].split('\t').count(), 3);
    assert_eq!(lines[5], "f-refresh\trefreshed");
    assert_report_flag_is_hidden(&account);
}

/// Learn the internal flag from the parent's invocation and verify ordinary help omits it.
fn assert_report_flag_is_hidden(account: &InstallAccount) {
    let arguments = fs::read_to_string(account.home.join("child-arguments"))
        .expect("observe child arguments without knowing the hidden flag");
    let arguments: Vec<_> = arguments.lines().collect();
    assert_eq!(arguments.len(), 2);
    assert_eq!(arguments[0], "install");
    let help = Command::new(installer())
        .args(["install", "--help"])
        .output()
        .expect("ordinary install help");
    assert!(help.status.success());
    assert!(
        !String::from_utf8_lossy(&help.stdout).contains(arguments[1]),
        "the machine-report argument must remain hidden"
    );
}

#[test]
fn current_admin_install_preserves_shim_and_saved_cargo_metadata() {
    let directory = tempfile::tempdir().expect("account fixture");
    let account = account_at(directory.path(), "runner");
    let bin = directory.path().join(".rustup/toolchains/stable/bin");
    fs::create_dir_all(&bin).expect("toolchain bin");
    let shim = bin.join("cargo");
    fs::write(&shim, b"#!/bin/sh\nexit 37\n").expect("original cargo");
    set_mode(&shim, 0o751);
    let reports = install_accounts(std::slice::from_ref(&account), installer());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountInstallOutcome::Installed);
    let paths = [shim, bin.join("cargo-tile-real")];
    let modified = UNIX_EPOCH;
    let before: Vec<_> = paths
        .iter()
        .map(|path| {
            fs::File::open(path)
                .expect("installed file")
                .set_modified(modified)
                .expect("distinct historical mtime");
            fs::metadata(path).expect("installed metadata")
        })
        .collect();
    let reports = install_accounts(std::slice::from_ref(&account), installer());
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountInstallOutcome::AlreadyInstalled);
    for (path, before) in paths.iter().zip(before) {
        let after = fs::metadata(path).expect("metadata after repeated install");
        assert_eq!(
            after.modified().expect("mtime"),
            modified,
            "{}",
            path.display()
        );
        assert_eq!(after.ino(), before.ino(), "{}", path.display());
        assert_eq!((after.uid(), after.gid()), (account.uid, account.gid));
        assert_eq!(after.mode(), before.mode());
    }
}

#[test]
fn reader_reports_foreign_owned_uid_directories_as_ignored() {
    let directory = tempfile::tempdir().expect("shared parent fixture");
    let parent = directory.path().join("capture");
    let uid = fs::metadata(directory.path()).expect("reader uid").uid();
    let own = parent.join(uid.to_string());
    let other = parent.join("4294967294");
    fs::create_dir_all(own.join("state/pids")).expect("own hierarchy");
    fs::create_dir_all(other.join("state/pids")).expect("other hierarchy");
    fs::create_dir(parent.join("unrelated")).expect("ignore non-account directory");
    symlink(&own, parent.join("4294967293")).expect("ignore account symlink");
    set_mode(&parent, 0o700);

    let roots = CaptureRoots::from_parent(&parent);
    let capture = Capture::take(&roots);
    assert_eq!(
        fs::metadata(&parent).expect("repaired parent").mode() & 0o7777,
        0o1777
    );
    assert_eq!(capture.root_status.len(), 2);
    let own_status = capture
        .root_status
        .iter()
        .find(|status| status.root.uid == uid)
        .expect("reader account");
    let other_status = capture
        .root_status
        .iter()
        .find(|status| status.root.uid == 4_294_967_294)
        .expect("unknown account");
    assert_eq!(own_status.root.cleanup, CaptureCleanup::Here);
    assert_eq!(other_status.root.cleanup, CaptureCleanup::AccountNextRun);
    let own_line = capture_root_status(own_status);
    assert!(
        own_line.contains("yours")
            && own_line.contains("readable")
            && own_line.contains("0 active captures")
            && own_line.contains("cleanup: here"),
        "{own_line}"
    );
    assert_eq!(other_status.owner, RootOwner::Uid(uid));
    assert_eq!(
        other_status.state,
        RootReadStatus::ForeignOwned {
            owner: own_status.account.clone(),
        }
    );
    let owner_name = match &own_status.account {
        AccountName::Resolved(name) => name.clone(),
        AccountName::Unavailable => uid.to_string(),
    };
    let other_line = capture_root_status(other_status);
    assert_eq!(
        other_line,
        format!(
            "{}: owned by {owner_name}, not by 4294967294 — ignored",
            other.display()
        )
    );
    assert_eq!(other_status.confirmed, 0);
    assert!(capture.confirmed().is_empty());
    let parent_line = shared_directory_status(&capture.shared_directory);
    assert!(
        parent_line.contains(parent.to_str().expect("parent text"))
            && parent_line.contains("1777")
            && parent_line.contains("owner"),
        "{parent_line}"
    );

    let inaccessible = parent.join("4294967292");
    fs::create_dir(&inaccessible).expect("later account");
    set_mode(&inaccessible, 0o000);
    let rescanned = Capture::take(&roots);
    set_mode(&inaccessible, 0o755);
    let status = rescanned
        .root_status
        .iter()
        .find(|status| status.root.uid == 4_294_967_292)
        .expect("new accounts appear on rescan");
    let line = capture_root_status(status);
    assert!(
        matches!(status.state, RootReadStatus::ForeignOwned { .. })
            && line
                == format!(
                    "{}: owned by {owner_name}, not by 4294967292 — ignored",
                    inaccessible.display()
                ),
        "{line}"
    );
}

#[test]
fn reader_creates_parent_and_repairs_owned_modes_that_deny_reading() {
    let directory = tempfile::tempdir().expect("parent repair fixture");
    let parent = directory.path().join("capture");
    let roots = CaptureRoots::from_parent(&parent);
    assert_eq!(
        fs::metadata(&parent).expect("reader creates parent").mode() & 0o7777,
        0o1777
    );
    for mode in [0o000, 0o111, 0o555, 0o700, 0o777] {
        set_mode(&parent, mode);
        let capture = Capture::take(&roots);
        let observed_mode = fs::metadata(&parent).expect("owned parent metadata").mode() & 0o7777;
        set_mode(&parent, 0o1777);
        assert_eq!(
            observed_mode, 0o1777,
            "reader must repair owned mode {mode:04o}"
        );
        let status = shared_directory_status(&capture.shared_directory);
        assert!(status.contains("mode 1777"), "{status}");
    }
}

#[test]
fn shared_directory_settings_cover_missing_permissions_and_canonical_aliases() {
    let directory = tempfile::tempdir().expect("shared parent fixture");
    let parent = directory.path().join("capture");
    let missing = shared_directory_status(&SharedCaptureDirectory::inspect(&parent));
    assert!(
        missing.contains("created by the first captured cargo run"),
        "{missing}"
    );
    fs::create_dir(&parent).expect("unshared parent");
    set_mode(&parent, 0o755);
    let unshared = shared_directory_status(&SharedCaptureDirectory::inspect(&parent));
    assert!(
        unshared.contains("0755") && unshared.contains("owner"),
        "{unshared}"
    );
    assert!(
        unshared.contains("other accounts cannot register — run: sudo chmod 1777 /tmp/cargo-tile"),
        "{unshared}"
    );
    let alias = directory.path().join("tmp-alias");
    symlink(directory.path(), &alias).expect("model macOS /tmp alias");
    assert_eq!(
        SharedCaptureDirectory::inspect(&parent),
        SharedCaptureDirectory::inspect(&alias.join("capture"))
    );
}

#[test]
fn reader_reaps_its_ended_capture_and_leaves_foreign_owned_directory_untouched() {
    let directory = tempfile::tempdir().expect("shared parent fixture");
    let parent = directory.path().join("capture");
    let uid = fs::metadata(directory.path()).expect("reader uid").uid();
    let own = parent.join(uid.to_string());
    let other = parent.join("4294967294");
    let mut child = Command::new("sh")
        .args(["-c", "exit 0"])
        .spawn()
        .expect("short-lived process");
    let pid = child.id();
    assert!(child.wait().expect("reap child").success());
    let name = format!("{pid}.ended");
    let log = format!("run-ended-{pid}.log");
    let record = [
        "cargo-tile-v2",
        "ended",
        "previous-boot",
        "1",
        &log,
        "/work",
        "/home",
        "1",
        "build",
        "",
    ]
    .join("\0");
    for account in [&own, &other] {
        fs::create_dir_all(account.join("state/pids")).expect("account hierarchy");
        fs::write(account.join("state/pids").join(&name), &record).expect("ended registration");
        fs::write(account.join(&log), b"old progress").expect("ended log");
    }
    let capture = Capture::take(&CaptureRoots::from_parent(&parent));
    assert_eq!(capture.root_status.len(), 2);
    let rejected = capture
        .root_status
        .iter()
        .find(|status| status.root.path == other)
        .expect("foreign-owned account remains visible in settings");
    assert!(matches!(
        rejected.state,
        RootReadStatus::ForeignOwned { .. }
    ));
    assert_eq!(rejected.confirmed, 0);
    assert!(capture.registered_pids().is_empty());
    assert!(!own.join("state/pids").join(&name).exists());
    assert!(!own.join(&log).exists());
    assert_eq!(
        fs::read(other.join("state/pids").join(&name)).expect("other registration survives"),
        record.as_bytes()
    );
    assert_eq!(
        fs::read(other.join(&log)).expect("other log survives"),
        b"old progress"
    );
}

#[test]
fn obsolete_roots_key_is_ignored_and_auto_install_is_preserved() {
    for value in ["[]", "['/obsolete/private-root']", "42"] {
        let configuration: Config = toml::from_str(&format!(
            "[capture]\nauto_install = false\nroots = {value}\n"
        ))
        .expect("obsolete roots ignored");
        assert!(!configuration.capture.auto_install);
        let serialized = toml::to_string(&configuration).expect("serialize current configuration");
        assert!(!serialized.contains("roots"));
    }
}

fn set_mode(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("fixture permissions");
}
