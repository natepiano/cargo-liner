//! Acceptance checks using production discovery, rendering, and installer entry points.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::config::Config;
use crate::constants::RUSTUP_HOME_ENV;
use crate::constants::SHIM_LOCK_RETRY_ATTEMPTS;
use crate::constants::SHIM_LOCK_RETRY_DELAY;
use crate::constants::SHIM_MARKER;
use crate::constants::SHIM_MARKER_SEARCH_BYTES;
use crate::constants::SHIM_VERSION_PREFIX;
use crate::constants::SUPPORTED_REGISTRATION_VERSION;
use crate::hook;
use crate::hook::AccountHookOutcome;
use crate::hook::AccountHookReport;
use crate::hook::DarwinGroupMembership;
use crate::hook::HookAccount;
use crate::hook::HookOperation;
use crate::hook::HookOperationOutcome;
use crate::hook::HookState;
use crate::hook::ToolchainHookOutcome;
use crate::progress::capture::Capture;
use crate::progress::capture_diagnostic::CaptureDiagnostic;
use crate::progress::capture_roots::AccountName;
use crate::progress::capture_roots::CaptureCleanup;
use crate::progress::capture_roots::CaptureRoots;
use crate::progress::capture_roots::RootReadStatus;
use crate::root_scan::RootOwner;
use crate::root_scan::SharedCaptureDirectory;
use crate::settings;

/// Every fixture account uses the caller's credentials, so no root access is needed.
fn account_at(home: &Path, name: &str) -> HookAccount {
    HookAccount {
        name: name.to_owned(),
        uid:  rustix::process::geteuid().as_raw(),
        gid:  rustix::process::getegid().as_raw(),
        home: home.to_owned(),
    }
}

/// Bin unit tests receive no `CARGO_BIN_EXE_*`, and the binary is not a build dependency
/// of its own test harness, so build it here when it is missing or older than the harness.
fn cargo_tile() -> &'static Path {
    static BINARY: LazyLock<PathBuf> = LazyLock::new(|| {
        let harness = std::env::current_exe().expect("bin test executable");
        let binary = harness
            .parent()
            .expect("test dependencies directory")
            .parent()
            .expect("profile output directory")
            .join("cargo-tile");
        let harness_modified: SystemTime = fs::metadata(&harness)
            .expect("bin test executable metadata")
            .modified()
            .expect("bin test executable modification time");
        if !binary.exists()
            || fs::metadata(&binary)
                .expect("cargo-tile executable metadata")
                .modified()
                .expect("cargo-tile executable modification time")
                < harness_modified
        {
            let mut command =
                Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
            command.args(["build", "-p", "cargo-tile", "--bin", "cargo-tile"]);
            let profile = binary
                .parent()
                .expect("profile output directory")
                .file_name()
                .expect("profile directory name")
                .to_str()
                .expect("Cargo profile name is UTF-8");
            match profile {
                "debug" => {},
                "release" => {
                    command.arg("--release");
                },
                profile => {
                    command.args(["--profile", profile]);
                },
            }
            let status = command
                .status()
                .expect("build cargo-tile for bin unit tests");
            assert!(status.success(), "cargo-tile build failed: {status}");
            assert!(
                binary.exists(),
                "cargo-tile build succeeded but {} is missing",
                binary.display()
            );
        }
        binary
    });
    BINARY.as_path()
}

fn original_cargo(account: &HookAccount, toolchain: &str) -> PathBuf {
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
    let reports = hook::run_account_hooks(&accounts, cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 3, "accounts without .rustup have no report");
    assert_eq!(reports[0].account, "developer");
    assert_eq!(reports[1].account, "runner");
    assert_eq!(reports[2].account, "empty");
    assert_eq!(reports[2].outcome, AccountHookOutcome::NoToolchains);
    for (account, report) in accounts.iter().zip(&reports).take(2) {
        assert_eq!(report.outcome, AccountHookOutcome::Completed);
        for toolchain in ["stable", "nightly"] {
            let bin = account
                .home
                .join(format!(".rustup/toolchains/{toolchain}/bin"));
            assert_eq!(
                fs::read(bin.join("cargo")).expect("installed shim"),
                include_bytes!("../cargo-capture-shim.sh")
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
    for report in hook::run_account_hooks(&accounts[..2], cargo_tile(), HookOperation::Install) {
        assert_eq!(report.outcome, AccountHookOutcome::Completed);
    }
}

/// The shared executable preserves installation reports and exists only while its guard lives.
#[test]
fn staged_executable_installs_toolchains_and_cleans_up() {
    let directory = tempfile::tempdir().expect("staged installer fixture");
    let original = account_at(&directory.path().join("original"), "runner");
    let account = account_at(&directory.path().join("staged"), "runner");
    for fixture in [&original, &account] {
        for toolchain in ["stable", "nightly"] {
            original_cargo(fixture, toolchain);
        }
    }
    let staged =
        hook::stage_executable(directory.path(), cargo_tile()).expect("stage the built installer");
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
        fs::read(cargo_tile()).expect("original installer bytes")
    );
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        staged.path(),
        HookOperation::Install,
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
    assert_eq!(
        reports,
        hook::run_account_hooks(
            std::slice::from_ref(&original),
            cargo_tile(),
            HookOperation::Install
        )
    );
    for toolchain in ["stable", "nightly"] {
        let bin = account
            .home
            .join(format!(".rustup/toolchains/{toolchain}/bin"));
        assert_eq!(
            fs::read(bin.join("cargo")).expect("staged installer writes shim"),
            include_bytes!("../cargo-capture-shim.sh")
        );
        assert_eq!(
            fs::read(bin.join("cargo-tile-real")).expect("staged installer preserves cargo"),
            b"#!/bin/sh\nexit 37\n"
        );
    }
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        staged.path(),
        HookOperation::Install,
    );
    assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
    assert_eq!(
        reports,
        hook::run_account_hooks(
            std::slice::from_ref(&original),
            cargo_tile(),
            HookOperation::Install
        )
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
    let reports = hook::run_account_hooks(&accounts, directory.path(), HookOperation::Install);
    assert_eq!(reports.len(), accounts.len());
    for (account, report) in accounts.iter().zip(&reports) {
        assert_eq!(report.account, account.name);
        let prefix = format!("could not start the account child as {}:", account.name);
        assert!(
            matches!(&report.outcome, AccountHookOutcome::Incomplete(reason)
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

/// Database groups match `id -G` for the named caller, including the primary gid.
#[test]
fn resolved_account_groups_include_the_callers_primary_gid() -> std::io::Result<()> {
    let uid = rustix::process::geteuid().as_raw();
    let users = sysinfo::Users::new_with_refreshed_list();
    let user = users
        .iter()
        .find(|user| **user.id() == uid)
        .expect("caller has an account database entry");
    let gid = *user.group_id();
    let groups = hook::account_groups(user.name(), gid);
    assert!(
        groups.is_ok(),
        "{} must resolve its groups successfully; a failure here means the resize path did not complete: {groups:?}",
        user.name()
    );
    let mut groups = groups?;
    assert!(
        groups.contains(&gid),
        "{} must retain primary gid {gid} in {groups:?}",
        user.name()
    );
    let output = Command::new("id")
        .args(["-G", user.name()])
        .output()
        .expect("query the caller's account database groups with id -G");
    assert!(output.status.success(), "id -G failed: {output:?}");
    let expected = String::from_utf8(output.stdout).expect("id -G prints numeric groups");
    let mut expected: Vec<u32> = expected
        .split_whitespace()
        .map(|group| group.parse().expect("id -G prints numeric gids"))
        .collect();
    assert!(
        groups.len() >= expected.len(),
        "{} resolved {} groups, fewer than the {} groups reported by id -G: {groups:?} versus {expected:?}",
        user.name(),
        groups.len(),
        expected.len()
    );
    groups.sort_unstable();
    expected.sort_unstable();
    assert_eq!(
        groups,
        expected,
        "{} must resolve every group reported by id -G",
        user.name()
    );
    Ok(())
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
        let reports = hook::run_account_hooks(
            std::slice::from_ref(&account),
            cargo_tile(),
            HookOperation::Install,
        );
        assert_eq!(reports.len(), 1);
        assert_eq!(
            reports[0].outcome,
            AccountHookOutcome::Completed,
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
        let reports = hook::run_account_hooks(
            std::slice::from_ref(&account),
            cargo_tile(),
            HookOperation::Install,
        );
        assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
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
        let reports = hook::run_account_hooks(
            std::slice::from_ref(&account),
            cargo_tile(),
            HookOperation::Install,
        );
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
        assert_eq!(
            fs::read(bin.join("cargo")).expect("repaired shim"),
            include_bytes!("../cargo-capture-shim.sh")
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
    let reports = hook::run_account_hooks(&[account], cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 1);
    assert_admin_report(
        &reports[0],
        &[("a-broken", "error:"), ("z-working", "installed")],
    );
    assert!(
        matches!(&reports[0].outcome, AccountHookOutcome::Incomplete(reason)
            if reason.contains("a-broken")
                && !reason.contains('\n')
                && !reason.contains("interrupted install found")),
        "the first child error must become one actionable incomplete report: {reports:?}"
    );
    assert_eq!(
        fs::read(good.join("cargo")).expect("later toolchain installs"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert_eq!(
        fs::read(broken.join("cargo")).expect("failed toolchain untouched"),
        b"#!/bin/sh\nexit 37\n"
    );
}

#[test]
fn injected_account_reports_an_orphan_before_an_installed_toolchain() {
    assert_orphan_and_installed_reports("a-orphan", "z-working");
}

#[test]
fn injected_account_reports_an_orphan_after_an_installed_toolchain() {
    assert_orphan_and_installed_reports("z-orphan", "a-working");
}

/// Toolchain discovery orders the actual child reports by these fixture names.
fn assert_orphan_and_installed_reports(orphan_name: &str, installed_name: &str) {
    let directory = tempfile::tempdir().expect("mixed account reports fixture");
    let account = account_at(directory.path(), "runner");
    let orphan = original_cargo(&account, orphan_name);
    fs::write(
        orphan.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("orphan shim without saved cargo");
    let working = original_cargo(&account, installed_name);
    let reports = hook::run_account_hooks(&[account], cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_admin_report(
        report,
        &[(orphan_name, "orphaned"), (installed_name, "installed")],
    );
    assert_eq!(report.account, "runner");
    assert!(
        matches!(&report.outcome, AccountHookOutcome::Incomplete(reason) if reason.contains(orphan_name)),
        "an orphan prevents an unqualified installed account summary: {report:?}"
    );
    assert_eq!(report.toolchains.len(), 2);
    for (name, outcome) in [
        (
            orphan_name,
            ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned),
        ),
        (
            installed_name,
            ToolchainHookOutcome::Install(HookOperationOutcome::Installed),
        ),
    ] {
        let toolchain = report
            .toolchains
            .iter()
            .find(|entry| entry.toolchain == name)
            .expect("retain both child report lines");
        assert_eq!(toolchain.outcome, outcome, "{name}: {report:?}");
    }
    assert_eq!(
        fs::read(working.join("cargo")).expect("successful install"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert_eq!(
        fs::read(working.join("cargo-tile-real")).expect("saved original cargo"),
        b"#!/bin/sh\nexit 37\n"
    );
    assert_eq!(
        fs::read(orphan.join("cargo")).expect("orphan survives install"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert!(!orphan.join("cargo-tile-real").exists());
}

#[test]
fn injected_account_reports_a_downgrade_refusal_before_a_successful_install() {
    assert_downgrade_and_installed_reports("a-newer", "z-working");
}

#[test]
fn injected_account_reports_a_downgrade_refusal_after_a_successful_install() {
    assert_downgrade_and_installed_reports("z-newer", "a-working");
}

/// Exercise the built child, protocol decoder, and administrative renderer in both orders.
fn assert_downgrade_and_installed_reports(newer_name: &str, installed_name: &str) {
    let directory = tempfile::tempdir().expect("mixed shim versions fixture");
    let account = account_at(directory.path(), "runner");
    let newer = installed_cargo(&account, newer_name);
    let supported = SUPPORTED_REGISTRATION_VERSION;
    let installed = supported + 1;
    let contents = newer_shim(installed);
    fs::write(newer.join("cargo"), &contents).expect("newer installed shim");
    let before = fs::metadata(newer.join("cargo")).expect("newer shim metadata");
    let original = fs::read(newer.join("cargo-tile-real")).expect("saved real cargo");
    let working = original_cargo(&account, installed_name);
    let observer = observe_account_child(directory.path());
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        &observer,
        HookOperation::Install,
    );
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_eq!(report.toolchains.len(), 2);
    assert_admin_report(
        report,
        &[
            (newer_name, "downgrade refused"),
            (installed_name, "installed"),
        ],
    );
    assert!(
        matches!(&report.outcome, AccountHookOutcome::Incomplete(reason)
            if reason.contains(newer_name) && reason.contains("downgrade refused")),
        "a kept newer shim prevents an unqualified installed summary: {report:?}"
    );
    let mut expected = [
        (
            newer_name,
            ToolchainHookOutcome::Install(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            }),
        ),
        (
            installed_name,
            ToolchainHookOutcome::Install(HookOperationOutcome::Installed),
        ),
    ];
    expected.sort_by_key(|(name, _)| *name);
    for (actual, (name, outcome)) in report.toolchains.iter().zip(expected) {
        assert_eq!(actual.toolchain, name);
        assert_eq!(actual.outcome, outcome);
    }
    assert_account_child_environment(&account);
    let protocol = fs::read_to_string(account.home.join("child-output")).expect("child protocol");
    let mut expected = [
        format!("{newer_name}\tdowngrade refused\t{installed}\t{supported}"),
        format!("{installed_name}\tinstalled"),
    ];
    expected.sort();
    assert_eq!(protocol.lines().collect::<Vec<_>>(), expected);
    let rendered = report.to_string();
    let refusal = rendered
        .lines()
        .find(|line| line.trim().starts_with(&format!("runner: {newer_name}:")))
        .expect("named administrative refusal");
    for text in [
        format!("newer shim v{installed} kept"),
        format!("this reader supports v{supported}"),
        "upgrade and restart the reader".to_owned(),
    ] {
        assert!(refusal.contains(&text), "{rendered}");
    }
    assert!(
        HookOperation::Install.completion(&reports).is_ok(),
        "capture setup refusal must preserve job-start install exit behavior"
    );
    assert_eq!(
        fs::read(working.join("cargo")).expect("successful sibling install"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert_eq!(
        fs::read(newer.join("cargo")).expect("newer shim is kept"),
        contents.as_bytes()
    );
    assert_eq!(
        fs::read(newer.join("cargo-tile-real")).expect("newer shim's real cargo survives"),
        original
    );
    let after = fs::metadata(newer.join("cargo")).expect("retained shim metadata");
    assert_eq!(after.ino(), before.ino());
    assert_eq!(after.mode(), before.mode());
    assert_eq!(
        after.modified().expect("mtime"),
        before.modified().expect("original mtime")
    );
    assert!(!newer.join("cargo-tile-shim.lock").exists());
    assert!(!newer.join("cargo-tile-shim.staging").exists());
}

/// Only the installed header changes; all other embedded shim bytes remain current.
fn newer_shim(version: u64) -> String {
    let source = include_str!("../../src/cargo-capture-shim.sh");
    let header = format!("{SHIM_VERSION_PREFIX}{SUPPORTED_REGISTRATION_VERSION}");
    assert_eq!(source.lines().filter(|line| *line == header).count(), 1);
    source.replacen(&header, &format!("{SHIM_VERSION_PREFIX}{version}"), 1)
}

/// The ordinary CLI must preserve the installed newer shim and keep install nonfatal.
#[test]
fn local_install_refuses_to_replace_a_newer_shim() {
    let directory = tempfile::tempdir().expect("local downgrade fixture");
    let account = account_at(directory.path(), "runner");
    let bin = installed_cargo(&account, "newer");
    let installed = SUPPORTED_REGISTRATION_VERSION + 1;
    let contents = newer_shim(installed);
    fs::write(bin.join("cargo"), &contents).expect("newer shim");
    let original = fs::read(bin.join("cargo-tile-real")).expect("real cargo before refusal");
    let output = Command::new(cargo_tile())
        .arg("install")
        .env("HOME", &account.home)
        .env(RUSTUP_HOME_ENV, account.home.join(".rustup"))
        .output()
        .expect("install against newer shim");
    assert!(output.status.success(), "{output:?}");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(rendered.contains("newer: downgrade refused"), "{rendered}");
    assert!(
        rendered.contains(&format!("newer shim v{installed} kept")),
        "{rendered}"
    );
    assert!(
        rendered.contains("upgrade and restart the reader"),
        "{rendered}"
    );
    assert!(!rendered.contains("not installed"), "{rendered}");
    assert_eq!(
        fs::read(bin.join("cargo")).expect("kept newer shim"),
        contents.as_bytes()
    );
    assert_eq!(
        fs::read(bin.join("cargo-tile-real")).expect("kept real cargo"),
        original
    );
}

/// A declaration split at the inspection limit cannot authorize a shim refresh.
#[test]
fn local_install_refuses_a_truncated_version_without_changing_installed_files() {
    let directory = tempfile::tempdir().expect("truncated version fixture");
    let account = account_at(directory.path(), "runner");
    let bin = installed_cargo(&account, "truncated");
    let source = include_str!("../../src/cargo-capture-shim.sh");
    let header = format!("{SHIM_VERSION_PREFIX}{SUPPORTED_REGISTRATION_VERSION}");
    let header_start = source.find(&header).expect("embedded version header");
    let padding = SHIM_MARKER_SEARCH_BYTES - header_start - header.len();
    let contents = source.replacen(
        &header,
        &format!("#{}\n{header}0", " ".repeat(padding - 2)),
        1,
    );
    assert!(contents[..SHIM_MARKER_SEARCH_BYTES].ends_with(&header));
    assert_eq!(
        &contents.as_bytes()[SHIM_MARKER_SEARCH_BYTES..SHIM_MARKER_SEARCH_BYTES + 2],
        b"0\n",
        "the inspected prefix declares the supported version but the full line declares a newer one"
    );
    fs::write(bin.join("cargo"), &contents).expect("padded installed shim");
    let paths = [bin.join("cargo"), bin.join("cargo-tile-real")];
    let before: Vec<_> = paths
        .iter()
        .map(|path| {
            fs::File::open(path)
                .expect("installed file")
                .set_modified(UNIX_EPOCH)
                .expect("distinct historical mtime");
            (
                fs::read(path).expect("installed bytes"),
                fs::metadata(path).expect("installed metadata"),
            )
        })
        .collect();
    let output = Command::new(cargo_tile())
        .arg("install")
        .env("HOME", &account.home)
        .env(RUSTUP_HOME_ENV, account.home.join(".rustup"))
        .output()
        .expect("install against truncated shim version");
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(rendered.contains("truncated: error:"), "{rendered}");
    assert!(
        rendered.contains("incomplete shim version line"),
        "{rendered}"
    );
    for (path, (bytes, metadata)) in paths.iter().zip(before) {
        assert_eq!(
            fs::read(path).expect("installed bytes after refusal"),
            bytes,
            "{}",
            path.display()
        );
        let after = fs::metadata(path).expect("installed metadata after refusal");
        assert_eq!(after.ino(), metadata.ino(), "{}", path.display());
        assert_eq!(after.mode(), metadata.mode(), "{}", path.display());
        assert_eq!(
            after.modified().expect("mtime after refusal"),
            metadata.modified().expect("original mtime"),
            "{}",
            path.display()
        );
    }
    assert!(!bin.join("cargo-tile-shim.lock").exists());
    assert!(!bin.join("cargo-tile-shim.staging").exists());
}

#[test]
fn injected_account_reports_every_orphan_beside_a_successful_install() {
    let directory = tempfile::tempdir().expect("multiple orphans fixture");
    let account = account_at(directory.path(), "runner");
    for name in ["a-orphan", "z-orphan"] {
        let bin = original_cargo(&account, name);
        fs::write(
            bin.join("cargo"),
            include_bytes!("../cargo-capture-shim.sh"),
        )
        .expect("orphan shim without saved cargo");
    }
    original_cargo(&account, "m-working");
    let reports = hook::run_account_hooks(&[account], cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_admin_report(
        report,
        &[
            ("a-orphan", "orphaned"),
            ("m-working", "installed"),
            ("z-orphan", "orphaned"),
        ],
    );
    assert!(
        matches!(&report.outcome, AccountHookOutcome::Incomplete(_)),
        "multiple orphans cannot receive an installed account summary: {report:?}"
    );
    assert_eq!(report.toolchains.len(), 3);
    for (toolchain, (name, outcome)) in report.toolchains.iter().zip([
        (
            "a-orphan",
            ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned),
        ),
        (
            "m-working",
            ToolchainHookOutcome::Install(HookOperationOutcome::Installed),
        ),
        (
            "z-orphan",
            ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned),
        ),
    ]) {
        assert_eq!(toolchain.toolchain, name);
        assert_eq!(toolchain.outcome, outcome, "{report:?}");
    }
}

#[test]
fn injected_account_reports_an_orphan_and_install_after_a_child_error() {
    let directory = tempfile::tempdir().expect("error before orphan fixture");
    let account = account_at(directory.path(), "runner");
    let broken = original_cargo(&account, "a-broken");
    fs::create_dir(broken.join("cargo-tile-shim.lock")).expect("lock open must fail");
    let orphan = original_cargo(&account, "m-orphan");
    fs::write(
        orphan.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("orphan shim without saved cargo");
    let working = original_cargo(&account, "z-working");
    let reports = hook::run_account_hooks(&[account], cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 1);
    let report = &reports[0];
    assert_admin_report(
        report,
        &[
            ("a-broken", "error:"),
            ("m-orphan", "orphaned"),
            ("z-working", "installed"),
        ],
    );
    assert!(
        matches!(&report.outcome, AccountHookOutcome::Incomplete(reason) if reason.contains("a-broken")),
        "the account report retains the failed toolchain: {report:?}"
    );
    assert_eq!(report.toolchains.len(), 3, "retain reports after an error");
    assert_eq!(report.toolchains[0].toolchain, "a-broken");
    assert!(matches!(&report.toolchains[0].outcome,
        ToolchainHookOutcome::Failed(reason) if !reason.is_empty()));
    assert_eq!(report.toolchains[1].toolchain, "m-orphan");
    assert_eq!(
        report.toolchains[1].outcome,
        ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned)
    );
    assert_eq!(report.toolchains[2].toolchain, "z-working");
    assert_eq!(
        report.toolchains[2].outcome,
        ToolchainHookOutcome::Install(HookOperationOutcome::Installed)
    );
    assert_eq!(
        fs::read(broken.join("cargo")).expect("failed toolchain untouched"),
        b"#!/bin/sh\nexit 37\n"
    );
    assert_eq!(
        fs::read(orphan.join("cargo")).expect("orphan untouched"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert!(!orphan.join("cargo-tile-real").exists());
    assert_eq!(
        fs::read(working.join("cargo")).expect("later toolchain installs"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert_eq!(
        fs::read(working.join("cargo-tile-real")).expect("saved original cargo"),
        b"#!/bin/sh\nexit 37\n"
    );
}

/// Inspect the same report text that the administrative command prints.
fn assert_admin_report(report: &AccountHookReport, outcomes: &[(&str, &str)]) {
    let rendered = report.to_string();
    assert!(
        rendered
            .lines()
            .next()
            .expect("account summary")
            .starts_with("runner: install: incomplete:"),
        "incomplete account installation must not claim installed: {rendered}"
    );
    for (toolchain, outcome) in outcomes {
        let prefix = format!("{}: {toolchain}: ", report.account);
        let lines: Vec<_> = rendered
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with(&prefix))
            .collect();
        assert_eq!(
            lines.len(),
            1,
            "one named report for {toolchain}: {rendered}"
        );
        assert!(
            lines[0]
                .strip_prefix(&prefix)
                .expect("toolchain prefix")
                .starts_with(outcome),
            "{toolchain} must report {outcome}: {rendered}"
        );
        if *outcome == "orphaned" {
            assert!(lines[0].contains("real cargo is missing"), "{rendered}");
        }
        if *outcome == "error:" {
            assert!(lines[0].contains("cargo-tile-shim.lock"), "{rendered}");
        }
    }
}

/// A removal failure must not leave a later healthy toolchain installed.
#[test]
fn uninstall_reports_an_orphan_and_restores_later_toolchains_before_failing() {
    let directory = tempfile::tempdir().expect("uninstall fixture");
    let account = account_at(directory.path(), "runner");
    let orphan = original_cargo(&account, "a-orphan");
    let shim = include_bytes!("../cargo-capture-shim.sh");
    fs::write(orphan.join("cargo"), shim).expect("orphan shim without saved cargo");
    let orphan_before = fs::metadata(orphan.join("cargo")).expect("orphan metadata");
    let working = original_cargo(&account, "z-working");
    let original = fs::read(working.join("cargo")).expect("original cargo bytes");
    let original_mode = fs::metadata(working.join("cargo"))
        .expect("original cargo metadata")
        .mode();
    fs::rename(working.join("cargo"), working.join("cargo-tile-real"))
        .expect("healthy installed toolchain saves cargo");
    fs::write(working.join("cargo"), shim).expect("healthy installed shim");

    let output = Command::new(cargo_tile())
        .arg("uninstall")
        .env("HOME", &account.home)
        .env(RUSTUP_HOME_ENV, account.home.join(".rustup"))
        .output()
        .expect("uninstall both toolchains");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout
            .lines()
            .any(|line| line == "z-working: capture shim removed"),
        "the later toolchain must report successful removal: {output:?}"
    );
    assert!(
        stderr
            .lines()
            .any(|line| { line.contains("a-orphan:") && line.contains("real cargo is missing") }),
        "the failed removal must name the orphan and its cause: {output:?}"
    );
    assert_ne!(
        output.status.code().expect("uninstall exits normally"),
        0,
        "partial removal must exit nonzero: {output:?}"
    );
    assert_eq!(
        fs::read(working.join("cargo")).expect("restored cargo"),
        original
    );
    assert_eq!(
        fs::metadata(working.join("cargo"))
            .expect("restored cargo metadata")
            .mode(),
        original_mode
    );
    assert_eq!(
        Command::new(working.join("cargo"))
            .status()
            .expect("restored cargo remains executable")
            .code(),
        Some(37)
    );
    assert!(!working.join("cargo-tile-real").exists());
    assert_eq!(
        fs::read(orphan.join("cargo")).expect("untouched orphan"),
        shim
    );
    let orphan_after = fs::metadata(orphan.join("cargo")).expect("orphan survives removal");
    assert_eq!(orphan_after.ino(), orphan_before.ino());
    assert_eq!(orphan_after.mode(), orphan_before.mode());
    assert_eq!(
        orphan_after.modified().expect("orphan mtime"),
        orphan_before.modified().expect("original orphan mtime")
    );
    assert!(!orphan.join("cargo-tile-real").exists());
    for bin in [&orphan, &working] {
        assert!(!bin.join("cargo-tile-shim.lock").exists());
    }
}

#[test]
fn injected_account_reports_child_toolchain_discovery_failure() {
    let directory = tempfile::tempdir().expect("account fixture");
    let account = account_at(directory.path(), "runner");
    fs::create_dir(account.home.join(".rustup")).expect("rustup exists");
    fs::write(account.home.join(".rustup/toolchains"), b"not a directory")
        .expect("child cannot list toolchains");
    let reports = hook::run_account_hooks(&[account], cargo_tile(), HookOperation::Install);
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(&reports[0].outcome, AccountHookOutcome::Incomplete(reason)
            if !reason.is_empty()
                && !reason.contains('\n')
                && reason != "no toolchains"
                && !reason.contains("home unreadable")),
        "the child's own discovery failure must become one incomplete report: {reports:?}"
    );
}

/// Observe the account child's protocol without knowing or passing its hidden flag.
#[test]
fn account_child_has_account_environment_and_only_machine_report_lines() {
    let directory = tempfile::tempdir().expect("account child fixture");
    let account = account_at(&directory.path().join("account"), "runner");
    let current = original_cargo(&account, "a-current");
    assert_eq!(
        hook::run_account_hooks(
            std::slice::from_ref(&account),
            cargo_tile(),
            HookOperation::Install
        )[0]
        .outcome,
        AccountHookOutcome::Completed
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
    let observer = observe_account_child(directory.path());
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        &observer,
        HookOperation::Install,
    );
    assert_eq!(reports.len(), 1);
    assert!(
        matches!(&reports[0].outcome, AccountHookOutcome::Incomplete(reason) if reason.contains("e-error"))
    );
    assert_account_child_environment(&account);
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
    assert_report_flag_is_hidden(&account, "install");
}

/// Record the real child's environment and machine output before forwarding both streams.
fn observe_account_child(directory: &Path) -> PathBuf {
    let observer = directory.join("observe-account");
    let binary = cargo_tile()
        .to_str()
        .expect("executable path")
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
    .expect("account observation wrapper");
    set_mode(&observer, 0o755);
    observer
}

/// The account record, rather than the parent's environment, determines the child's home.
fn assert_account_child_environment(account: &HookAccount) {
    for (file, expected) in [("child-uid", account.uid), ("child-gid", account.gid)] {
        assert_eq!(
            fs::read_to_string(account.home.join(file))
                .expect("observed child identity")
                .trim(),
            expected.to_string()
        );
    }
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
}

/// Learn the internal flag from the parent's invocation and verify ordinary help omits it.
fn assert_report_flag_is_hidden(account: &HookAccount, operation: &str) {
    let arguments = fs::read_to_string(account.home.join("child-arguments"))
        .expect("observe child arguments without knowing the hidden flag");
    let arguments: Vec<_> = arguments.lines().collect();
    assert_eq!(arguments.len(), 2);
    assert_eq!(arguments[0], operation);
    let help = Command::new(cargo_tile())
        .args([operation, "--help"])
        .output()
        .expect("ordinary install help");
    assert!(help.status.success());
    assert!(
        !String::from_utf8_lossy(&help.stdout).contains(arguments[1]),
        "the machine-report argument must remain hidden"
    );
}

/// Status observes every hook state without repairing or locking any toolchain.
#[test]
fn account_status_child_reports_all_states_without_changing_toolchains() {
    let directory = tempfile::tempdir().expect("status child fixture");
    let account = account_at(&directory.path().join("runner"), "runner");
    let installed = installed_cargo(&account, "a-installed");
    let absent = original_cargo(&account, "b-absent");
    let repairable = original_cargo(&account, "c-repairable");
    fs::rename(repairable.join("cargo"), repairable.join("cargo-tile-real"))
        .expect("interrupted install has saved cargo only");
    let orphaned = original_cargo(&account, "d-orphaned");
    fs::write(
        orphaned.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("orphan shim");
    let unreadable = original_cargo(&account, "e-unreadable");
    set_mode(&unreadable.join("cargo"), 0o000);
    let unreadable_error = fs::read(unreadable.join("cargo"))
        .expect_err("fixture cargo cannot be read by the current account")
        .to_string();
    let paths = [
        installed.join("cargo"),
        installed.join("cargo-tile-real"),
        absent.join("cargo"),
        repairable.join("cargo-tile-real"),
        orphaned.join("cargo"),
        unreadable.join("cargo"),
    ];
    let before: Vec<_> = paths
        .iter()
        .map(|path| fs::metadata(path).expect("fixture metadata"))
        .collect();
    let observer = observe_account_child(directory.path());
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        &observer,
        HookOperation::Status,
    );
    assert_eq!(reports.len(), 1);
    assert_account_child_environment(&account);
    assert_report_flag_is_hidden(&account, "status");
    let output = fs::read_to_string(account.home.join("child-output")).expect("status protocol");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 5, "one status line per toolchain: {output}");
    assert_eq!(
        &lines[..4],
        [
            "a-installed\tinstalled",
            "b-absent\tabsent",
            "c-repairable\trepairable",
            "d-orphaned\torphaned",
        ]
    );
    assert!(
        lines[4].starts_with("e-unreadable\tunreadable\t"),
        "{output}"
    );
    assert_eq!(lines[4].split('\t').count(), 3);
    assert!(
        lines[4].contains("cargo"),
        "unreadable state keeps the path: {output}"
    );
    assert!(
        lines[4].contains(&unreadable_error),
        "unreadable state keeps the OS error: {output}"
    );
    let report = &reports[0];
    assert_eq!(report.toolchains.len(), 5);
    assert!(
        matches!(&report.outcome, AccountHookOutcome::Incomplete(reason)
        if reason.contains("e-unreadable") && reason.contains(&unreadable_error)
            && !reason.contains("no toolchains")),
        "{report:?}"
    );
    for (toolchain, state) in [
        ("a-installed", "installed"),
        ("b-absent", "absent"),
        ("c-repairable", "repairable"),
        ("d-orphaned", "orphaned"),
        ("e-unreadable", "unreadable"),
    ] {
        assert_rendered_toolchain(report, toolchain, state);
    }
    for (path, before) in paths.iter().zip(before) {
        let after = fs::metadata(path).expect("status preserves fixture file");
        assert_eq!(after.ino(), before.ino(), "{}", path.display());
        assert_eq!(after.mode(), before.mode(), "{}", path.display());
        assert_eq!(
            after.modified().expect("mtime"),
            before.modified().expect("original mtime")
        );
    }
    for bin in [&installed, &absent, &repairable, &orphaned, &unreadable] {
        assert!(!bin.join("cargo-tile-shim.lock").exists());
        assert!(!bin.join("cargo-tile-shim.staging").exists());
    }
    assert!(!absent.join("cargo-tile-real").exists());
    assert!(!repairable.join("cargo").exists());
    assert!(!orphaned.join("cargo-tile-real").exists());
    set_mode(&unreadable.join("cargo"), 0o751);
}

/// Empty discovery is successful; an unreadable toolchain directory retains its own failure.
#[test]
fn account_status_distinguishes_no_toolchains_from_failed_discovery_and_continues() {
    let directory = tempfile::tempdir().expect("status discovery fixture");
    let accounts = [
        account_at(&directory.path().join("empty"), "empty"),
        account_at(&directory.path().join("unreadable"), "unreadable"),
        account_at(&directory.path().join("later"), "later"),
    ];
    fs::create_dir_all(accounts[0].home.join(".rustup/toolchains"))
        .expect("empty toolchains directory");
    fs::create_dir_all(accounts[1].home.join(".rustup")).expect("account rustup home");
    let failed_path = accounts[1].home.join(".rustup/toolchains");
    fs::write(&failed_path, b"not a directory").expect("discovery cannot enumerate toolchains");
    let installed = installed_cargo(&accounts[2], "installed");
    fs::create_dir(installed.join("cargo-tile-shim.lock"))
        .expect("status must not try to acquire a mutation lock");
    original_cargo(&accounts[2], "absent");
    let repairable = original_cargo(&accounts[2], "repairable");
    fs::rename(repairable.join("cargo"), repairable.join("cargo-tile-real"))
        .expect("interrupted installation");
    let orphaned = original_cargo(&accounts[2], "orphaned");
    fs::write(
        orphaned.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("orphan shim");
    let reports = hook::run_account_hooks(&accounts, cargo_tile(), HookOperation::Status);
    assert_eq!(reports.len(), 3);
    assert_eq!(reports[0].account, "empty");
    assert_eq!(reports[0].outcome, AccountHookOutcome::NoToolchains);
    assert!(reports[0].toolchains.is_empty());
    assert_eq!(reports[0].to_string(), "empty: status: no toolchains");
    let failed = &reports[1];
    assert_eq!(failed.account, "unreadable");
    assert!(
        matches!(&failed.outcome, AccountHookOutcome::Incomplete(reason)
        if reason.contains(failed_path.to_str().expect("fixture path"))),
        "{failed:?}"
    );
    assert!(!failed.to_string().contains("no toolchains"));
    let later = &reports[2];
    assert_eq!(later.account, "later");
    assert_eq!(
        later.outcome,
        AccountHookOutcome::Completed,
        "repairable and orphaned are successful observations: {later:?}"
    );
    assert_eq!(later.toolchains.len(), 4);
    for state in ["installed", "absent", "repairable", "orphaned"] {
        assert_rendered_toolchain(later, state, state);
    }
    assert!(!repairable.join("cargo").exists());
    assert!(!orphaned.join("cargo-tile-real").exists());
    assert!(installed.join("cargo-tile-shim.lock").is_dir());
}

/// Uninstall restores installed and interrupted toolchains while retaining every refusal.
#[test]
fn account_uninstall_child_restores_recoverable_toolchains_and_continues_accounts() {
    let directory = tempfile::tempdir().expect("uninstall child fixture");
    let account = account_at(&directory.path().join("runner"), "runner");
    let later = account_at(&directory.path().join("later"), "later");
    let orphan = original_cargo(&account, "a-orphaned");
    fs::write(
        orphan.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("orphan shim");
    let broken = installed_cargo(&account, "b-failed");
    fs::create_dir(broken.join("cargo-tile-shim.lock")).expect("removal lock fails");
    let installed = installed_cargo(&account, "c-installed");
    let repairable = original_cargo(&account, "d-repairable");
    fs::rename(repairable.join("cargo"), repairable.join("cargo-tile-real"))
        .expect("saved cargo after interrupted installation");
    let absent = original_cargo(&account, "e-absent");
    let later_bin = installed_cargo(&later, "later-stable");
    let restored = [&installed, &repairable, &later_bin];
    let originals: Vec<_> = restored
        .iter()
        .map(|bin| fs::metadata(bin.join("cargo-tile-real")).expect("saved original"))
        .collect();
    let observer = observe_account_child(directory.path());
    let reports = hook::run_account_hooks(&[account, later], &observer, HookOperation::Uninstall);
    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].account, "runner");
    assert_eq!(reports[1].account, "later");
    assert!(
        matches!(&reports[0].outcome, AccountHookOutcome::Incomplete(reason)
        if reason.contains("a-orphaned") && reason.contains("b-failed")),
        "{reports:?}"
    );
    assert_eq!(reports[1].outcome, AccountHookOutcome::Completed);
    for (name, result) in [
        ("a-orphaned", "orphaned"),
        ("b-failed", "error:"),
        ("c-installed", "removed"),
        ("d-repairable", "removed"),
        ("e-absent", "absent"),
    ] {
        assert_rendered_toolchain(&reports[0], name, result);
    }
    let account = account_at(&directory.path().join("runner"), "runner");
    assert_account_child_environment(&account);
    assert_report_flag_is_hidden(&account, "uninstall");
    let output = fs::read_to_string(account.home.join("child-output")).expect("uninstall protocol");
    let lines: Vec<_> = output.lines().collect();
    assert_eq!(lines.len(), 5, "{output}");
    assert_eq!(lines[0], "a-orphaned\torphaned");
    assert!(lines[1].starts_with("b-failed\terror\t"), "{output}");
    assert_eq!(
        &lines[2..],
        [
            "c-installed\tremoved",
            "d-repairable\tremoved",
            "e-absent\tabsent"
        ]
    );
    for (bin, original) in restored.iter().zip(originals) {
        let path = bin.join("cargo");
        let after = fs::metadata(&path).expect("restored cargo");
        assert_eq!(
            fs::read(&path).expect("restored contents"),
            b"#!/bin/sh\nexit 37\n"
        );
        assert_eq!(after.ino(), original.ino());
        assert_eq!(after.mode(), original.mode());
        assert!(!bin.join("cargo-tile-real").exists());
        assert!(!bin.join("cargo-tile-shim.lock").exists());
    }
    for bin in [&orphan, &broken] {
        assert_eq!(
            fs::read(bin.join("cargo")).expect("refused shim survives"),
            include_bytes!("../cargo-capture-shim.sh")
        );
    }
    assert!(!absent.join("cargo-tile-real").exists());
}

/// Preserve the real cargo's inode so status and uninstall can be checked independently of install.
fn installed_cargo(account: &HookAccount, toolchain: &str) -> PathBuf {
    let bin = original_cargo(account, toolchain);
    fs::rename(bin.join("cargo"), bin.join("cargo-tile-real")).expect("save original cargo");
    fs::write(
        bin.join("cargo"),
        include_bytes!("../cargo-capture-shim.sh"),
    )
    .expect("installed shim");
    set_mode(&bin.join("cargo"), 0o751);
    bin
}

/// Account summaries must not replace the individual toolchain lines printed by the CLI.
fn assert_rendered_toolchain(report: &AccountHookReport, toolchain: &str, result: &str) {
    let rendered = report.to_string();
    let prefix = format!("{}: {toolchain}: ", report.account);
    let lines: Vec<_> = rendered
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect();
    assert_eq!(lines.len(), 1, "one named row for {toolchain}: {rendered}");
    assert!(
        lines[0].starts_with(result),
        "{toolchain} reports {result}: {rendered}"
    );
}

#[test]
fn install_retains_reports_around_a_malformed_line_and_continues_accounts() {
    assert_child_report_retention(HookOperation::Install, ChildReportFailure::MalformedLine);
}

#[test]
fn uninstall_retains_reports_around_a_malformed_line_and_continues_accounts() {
    assert_child_report_retention(HookOperation::Uninstall, ChildReportFailure::MalformedLine);
}

#[test]
fn status_retains_reports_around_a_malformed_line_and_continues_accounts() {
    assert_child_report_retention(HookOperation::Status, ChildReportFailure::MalformedLine);
}

#[test]
fn install_retains_reports_after_an_unsuccessful_child_exit_and_continues_accounts() {
    assert_child_report_retention(HookOperation::Install, ChildReportFailure::UnsuccessfulExit);
}

#[test]
fn uninstall_retains_reports_after_an_unsuccessful_child_exit_and_continues_accounts() {
    assert_child_report_retention(
        HookOperation::Uninstall,
        ChildReportFailure::UnsuccessfulExit,
    );
}

#[test]
fn status_retains_reports_after_an_unsuccessful_child_exit_and_continues_accounts() {
    assert_child_report_retention(HookOperation::Status, ChildReportFailure::UnsuccessfulExit);
}

#[test]
fn install_retains_completed_rows_when_the_real_child_is_terminated_mid_operation() {
    assert_terminated_child_retention(HookOperation::Install);
}

#[test]
fn uninstall_retains_completed_rows_when_the_real_child_is_terminated_mid_operation() {
    assert_terminated_child_retention(HookOperation::Uninstall);
}

/// Kill the real child after its first row while a later toolchain remains locked.
fn assert_terminated_child_retention(operation: HookOperation) {
    let directory = tempfile::tempdir().expect("terminated account child fixture");
    let accounts = [
        account_at(&directory.path().join("first"), "first"),
        account_at(&directory.path().join("later"), "later"),
    ];
    for (account, names) in [
        (&accounts[0], &["a-working", "z-waiting"][..]),
        (&accounts[1], &["later-stable"][..]),
    ] {
        for name in names {
            match operation {
                HookOperation::Install | HookOperation::Status => original_cargo(account, name),
                HookOperation::Uninstall => installed_cargo(account, name),
            };
        }
    }
    let waiting = accounts[0].home.join(".rustup/toolchains/z-waiting/bin");
    let waiting_cargo = fs::read(waiting.join("cargo")).expect("waiting cargo before child");
    let lock = waiting.join("cargo-tile-shim.lock");
    fs::write(&lock, b"fixture installer owns this lock").expect("hold later toolchain lock");
    let wrapper = terminating_account_child(directory.path());
    let reports = hook::run_account_hooks(&accounts, &wrapper, operation);
    assert_eq!(reports.len(), 2, "{reports:?}");
    let first = &reports[0];
    assert_eq!(first.account, "first");
    assert_eq!(first.operation, operation);
    assert_eq!(
        first.toolchains.len(),
        1,
        "retain the completed row without finishing the locked toolchain: {first:?}"
    );
    assert_eq!(first.toolchains[0].toolchain, "a-working");
    let (outcome, label) = match operation {
        HookOperation::Install => (
            ToolchainHookOutcome::Install(HookOperationOutcome::Installed),
            "installed",
        ),
        HookOperation::Uninstall => (
            ToolchainHookOutcome::Uninstall(HookOperationOutcome::Removed),
            "removed",
        ),
        HookOperation::Status => (ToolchainHookOutcome::Status(HookState::Absent), "absent"),
    };
    assert_eq!(first.toolchains[0].outcome, outcome);
    let reason = "account child terminated while z-waiting is locked";
    assert!(
        matches!(&first.outcome, AccountHookOutcome::Incomplete(error) if error.contains(reason)),
        "{first:?}"
    );
    assert!(first.to_string().contains(reason), "{first}");
    assert_rendered_toolchain(first, "a-working", label);
    assert_eq!(
        fs::read_to_string(accounts[0].home.join("child-termination"))
            .expect("real child was killed and reaped"),
        "SIGKILL\n"
    );
    assert_eq!(
        fs::read(waiting.join("cargo")).expect("waiting cargo"),
        waiting_cargo
    );
    assert_eq!(
        fs::read(&lock).expect("other installer's lock survives"),
        b"fixture installer owns this lock"
    );
    assert_eq!(reports[1].account, "later");
    assert_eq!(reports[1].outcome, AccountHookOutcome::Completed);
    assert_eq!(reports[1].toolchains.len(), 1);
    assert_eq!(reports[1].toolchains[0].outcome, outcome);
    assert_rendered_toolchain(&reports[1], "later-stable", label);
    for (account, toolchain) in [(&accounts[0], "a-working"), (&accounts[1], "later-stable")] {
        let bin = account
            .home
            .join(format!(".rustup/toolchains/{toolchain}/bin"));
        assert_finished_account_toolchain(&bin, operation);
    }
    assert_eq!(
        operation.completion(&reports).is_err(),
        operation == HookOperation::Uninstall
    );
}

/// Reports must agree with the real mutation, even when the owning child was killed.
fn assert_finished_account_toolchain(bin: &Path, operation: HookOperation) {
    let (cargo, real) = (bin.join("cargo"), bin.join("cargo-tile-real"));
    if operation == HookOperation::Install {
        assert_eq!(
            fs::read(&cargo).expect("completed shim"),
            include_bytes!("../cargo-capture-shim.sh")
        );
        assert_eq!(
            fs::read(real).expect("completed saved cargo"),
            b"#!/bin/sh\nexit 37\n"
        );
    } else {
        assert_eq!(
            fs::read(cargo).expect("completed original cargo"),
            b"#!/bin/sh\nexit 37\n"
        );
        assert!(!real.exists());
    }
    assert!(!bin.join("cargo-tile-shim.lock").exists());
}

/// Read an actual child row before killing it; never synthesize or discard protocol rows.
fn terminating_account_child(directory: &Path) -> PathBuf {
    let wrapper = directory.join("terminate-account-child");
    let binary = cargo_tile()
        .to_str()
        .expect("binary path")
        .replace('\'', "'\"'\"'");
    let report_wait = SHIM_LOCK_RETRY_DELAY
        .saturating_mul(u32::try_from(SHIM_LOCK_RETRY_ATTEMPTS).expect("lock retry count"))
        .as_secs_f64()
        / 2.0;
    fs::write(
        &wrapper,
        format!(
            r#"#!/bin/sh
exec python3 - '{binary}' "$@" <<'PY'
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import time

home = Path(os.environ['HOME'])
if home.name != 'first':
    os.execv(sys.argv[1], sys.argv[1:])
first = home / '.rustup/toolchains/a-working/bin'
original = (first / 'cargo').read_bytes()
child = subprocess.Popen(sys.argv[1:], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
prefix = b''
try:
    deadline = time.monotonic() + 5
    while True:
        try:
            changed = (first / 'cargo').read_bytes() != original
        except FileNotFoundError:
            changed = False
        if changed and not (first / 'cargo-tile-shim.lock').exists():
            break
        assert time.monotonic() < deadline, 'the first toolchain never finished'
        time.sleep(0.001)
    # Expire before the later lock can time out, including when stdout is still empty.
    ready, _, _ = select.select([child.stdout], [], [], {report_wait})
    if ready:
        prefix = os.read(child.stdout.fileno(), 65536)
finally:
    child.kill()
    stdout, stderr = child.communicate(timeout=5)
sys.stdout.buffer.write(prefix + stdout)
sys.stderr.buffer.write(stderr)
assert child.returncode == -signal.SIGKILL, 'the real child finished before termination'
(home / 'child-termination').write_text('SIGKILL\n')
sys.stderr.write('account child terminated while z-waiting is locked\n')
sys.exit(23)
PY
"#
        ),
    )
    .expect("write terminating account wrapper");
    set_mode(&wrapper, 0o755);
    wrapper
}

/// Each wrapper introduces exactly one failure in addition to a reported toolchain failure.
#[derive(Clone, Copy)]
enum ChildReportFailure {
    /// Insert a non-protocol line between valid rows, then exit successfully.
    MalformedLine,
    /// Forward every valid row, then exit unsuccessfully with a separate reason.
    UnsuccessfulExit,
}

fn assert_child_report_retention(operation: HookOperation, failure: ChildReportFailure) {
    let directory = tempfile::tempdir().expect("child report retention fixture");
    let accounts = [
        account_at(&directory.path().join("first"), "first"),
        account_at(&directory.path().join("later"), "later"),
    ];
    for (account, names) in [
        (&accounts[0], &["a-working", "m-failed", "z-working"][..]),
        (&accounts[1], &["later-stable"][..]),
    ] {
        for name in names {
            let bin = match operation {
                HookOperation::Install | HookOperation::Status => original_cargo(account, name),
                HookOperation::Uninstall => installed_cargo(account, name),
            };
            if *name == "m-failed" {
                match operation {
                    HookOperation::Install | HookOperation::Uninstall => {
                        fs::create_dir(bin.join("cargo-tile-shim.lock"))
                            .expect("toolchain operation fails");
                    },
                    HookOperation::Status => set_mode(&bin.join("cargo"), 0o000),
                }
            }
        }
    }
    let (wrapper, failure_reason) = failing_report_wrapper(directory.path(), failure);
    let reports = hook::run_account_hooks(&accounts, &wrapper, operation);
    assert_eq!(reports.len(), 2, "{reports:?}");
    let (expected, label) = match operation {
        HookOperation::Install => (
            ToolchainHookOutcome::Install(HookOperationOutcome::Installed),
            "installed",
        ),
        HookOperation::Uninstall => (
            ToolchainHookOutcome::Uninstall(HookOperationOutcome::Removed),
            "removed",
        ),
        HookOperation::Status => (ToolchainHookOutcome::Status(HookState::Absent), "absent"),
    };
    assert_retained_toolchains(&reports[0], operation, &expected, label, failure_reason);
    let later = &reports[1];
    assert_eq!(later.account, "later");
    assert_eq!(later.operation, operation);
    assert_eq!(later.outcome, AccountHookOutcome::Completed);
    assert_eq!(later.toolchains.len(), 1);
    assert_eq!(later.toolchains[0].outcome, expected);
    assert_rendered_toolchain(later, "later-stable", label);
    let later_bin = accounts[1].home.join(".rustup/toolchains/later-stable/bin");
    match operation {
        HookOperation::Install => assert_eq!(
            fs::read(later_bin.join("cargo")).expect("later shim"),
            include_bytes!("../cargo-capture-shim.sh")
        ),
        HookOperation::Uninstall | HookOperation::Status => assert_eq!(
            fs::read(later_bin.join("cargo")).expect("later original cargo"),
            b"#!/bin/sh\nexit 37\n"
        ),
    }
    set_mode(
        &accounts[0]
            .home
            .join(".rustup/toolchains/m-failed/bin/cargo"),
        0o751,
    );
}

/// Introduce a protocol failure in the first child while later children run normally.
fn failing_report_wrapper(
    directory: &Path,
    failure: ChildReportFailure,
) -> (PathBuf, &'static str) {
    let (forward_output, failure_reason) = match failure {
        ChildReportFailure::MalformedLine => (
            "awk '{ print; if (NR == 1) print \"malformed child report\"; }' \"$HOME/valid-output\"\nexit 0",
            "malformed child report",
        ),
        ChildReportFailure::UnsuccessfulExit => (
            "cat \"$HOME/valid-output\"\nprintf 'account wrapper exit failure\\n' >&2\nexit 23",
            "account wrapper exit failure",
        ),
    };
    let wrapper = directory.join("account-wrapper");
    let binary = cargo_tile()
        .to_str()
        .expect("binary path")
        .replace('\'', "'\"'\"'");
    fs::write(
        &wrapper,
        format!(
            r#"#!/bin/sh
case "$HOME" in
    */first)
        '{binary}' "$@" > "$HOME/valid-output"
        {forward_output}
        ;;
    *) exec '{binary}' "$@" ;;
esac
"#
        ),
    )
    .expect("write account child wrapper");
    set_mode(&wrapper, 0o755);
    (wrapper, failure_reason)
}

/// Retain successes on either side of a failed toolchain and the wrapper's separate failure.
fn assert_retained_toolchains(
    first: &AccountHookReport,
    operation: HookOperation,
    expected: &ToolchainHookOutcome,
    label: &str,
    failure_reason: &str,
) {
    assert_eq!(first.account, "first");
    assert_eq!(first.operation, operation);
    assert_eq!(first.toolchains.len(), 3, "{first:?}");
    for (row, name) in first
        .toolchains
        .iter()
        .zip(["a-working", "m-failed", "z-working"])
    {
        assert_eq!(row.toolchain, name);
        if name != "m-failed" {
            assert_eq!(&row.outcome, expected);
            assert_rendered_toolchain(first, name, label);
        }
    }
    let failed = &first.toolchains[1];
    match operation {
        HookOperation::Status => assert!(
            matches!(&failed.outcome,
            ToolchainHookOutcome::Unreadable(reason) if reason.contains("cargo")),
            "{failed:?}"
        ),
        HookOperation::Install | HookOperation::Uninstall => assert!(
            matches!(&failed.outcome,
            ToolchainHookOutcome::Failed(reason) if reason.contains("cargo-tile-shim.lock")),
            "{failed:?}"
        ),
    }
    let rendered = first.to_string();
    assert!(
        matches!(&first.outcome, AccountHookOutcome::Incomplete(reason)
        if reason.contains(&failed.to_string()) && reason.contains(failure_reason)
            && reason.contains("; ") && !reason.contains('\n')),
        "{first:?}"
    );
    assert!(
        rendered.starts_with(&format!("first: {}: incomplete:", operation.subcommand())),
        "{rendered}"
    );
    assert!(rendered.contains(failure_reason), "{rendered}");
    assert_rendered_toolchain(
        first,
        "m-failed",
        match operation {
            HookOperation::Status => "unreadable:",
            HookOperation::Install | HookOperation::Uninstall => "error:",
        },
    );
}

/// A resolver error precedes child execution and still reaches uninstall completion.
#[test]
fn credential_failure_is_incomplete_preserves_reason_and_continues_accounts() {
    let directory = tempfile::tempdir().expect("credential failure fixture");
    let accounts = [
        account_at(&directory.path().join("unresolved"), "unresolved"),
        account_at(&directory.path().join("later"), "later"),
    ];
    let first = installed_cargo(&accounts[0], "stable");
    let later = installed_cargo(&accounts[1], "stable");
    let observer = observe_account_child(directory.path());
    let reason = "fixture group resolver rejects membership count";
    let mut visited = Vec::new();
    let reports = hook::run_account_hooks_with(
        &accounts,
        &observer,
        HookOperation::Uninstall,
        |_, account| {
            visited.push(account.name.clone());
            if account.name == "unresolved" {
                Err(std::io::Error::other(reason))
            } else {
                Ok(())
            }
        },
    );
    assert_eq!(visited, ["unresolved", "later"]);
    assert_eq!(reports.len(), 2);
    let first_report = &reports[0];
    assert_eq!(first_report.account, "unresolved");
    assert!(
        matches!(&first_report.outcome, AccountHookOutcome::Incomplete(error)
        if error.contains(reason) && error.contains("unresolved")),
        "{first_report:?}"
    );
    assert!(first_report.toolchains.is_empty());
    let rendered = first_report.to_string();
    assert!(
        rendered.contains("unresolved: uninstall: incomplete:"),
        "{rendered}"
    );
    assert!(rendered.contains(reason), "{rendered}");
    assert!(!rendered.contains("no toolchains"), "{rendered}");
    assert!(
        !accounts[0].home.join("child-arguments").exists(),
        "resolver failure never starts a child"
    );
    assert_eq!(
        fs::read(first.join("cargo")).expect("first shim unchanged"),
        include_bytes!("../cargo-capture-shim.sh")
    );
    assert!(first.join("cargo-tile-real").exists());
    assert_eq!(reports[1].outcome, AccountHookOutcome::Completed);
    assert_rendered_toolchain(&reports[1], "stable", "removed");
    assert_account_child_environment(&accounts[1]);
    assert_eq!(
        fs::read(later.join("cargo")).expect("later cargo restored"),
        b"#!/bin/sh\nexit 37\n"
    );
    assert!(!later.join("cargo-tile-real").exists());
    assert!(
        HookOperation::Uninstall.completion(&reports).is_err(),
        "incomplete uninstall must fail"
    );
}

/// Parent preparation retains every membership and permits both account children.
/// The native administrator probe separately establishes kernel access after the switch.
#[test]
fn darwin_membership_beyond_the_credential_limit_keeps_account_children_running() {
    let directory = tempfile::tempdir().expect("full-membership account fixture");
    let accounts = [
        account_at(
            &directory.path().join("many-groups"),
            "runner-with-many-groups",
        ),
        account_at(&directory.path().join("later"), "later"),
    ];
    for account in &accounts {
        original_cargo(account, "stable");
    }
    let observer = observe_account_child(directory.path());
    let groups: Vec<_> = (1..=16)
        .map(|offset| accounts[0].gid.checked_add(offset).expect("fixture gid"))
        .chain(std::iter::once(accounts[0].gid))
        .collect();
    for operation in [
        HookOperation::Install,
        HookOperation::Status,
        HookOperation::Uninstall,
    ] {
        let mut prepared = Vec::new();
        let reports =
            hook::run_account_hooks_with(&accounts, &observer, operation, |_, account| {
                let resolved = if account.name == accounts[0].name {
                    groups.clone()
                } else {
                    vec![account.gid]
                };
                let membership =
                    DarwinGroupMembership::new(account.uid, account.gid, resolved, 16)?;
                assert_eq!(membership.groups[0], account.gid);
                assert_eq!(
                    u32::try_from(membership.uid).expect("membership uid"),
                    account.uid
                );
                if account.name == accounts[0].name {
                    assert_eq!(membership.count, 16);
                    assert_eq!(membership.groups.len(), 17);
                    let mut retained = membership.groups.clone();
                    retained.sort_unstable();
                    let mut expected = groups.clone();
                    expected.sort_unstable();
                    assert_eq!(
                        retained, expected,
                        "the complete resolved list survives preparation"
                    );
                    assert!(!membership.groups[..16].contains(&membership.groups[16]));
                } else {
                    assert_eq!(membership.count, 1);
                    assert_eq!(membership.groups, [account.gid]);
                }
                prepared.push(account.name.clone());
                Ok(())
            });
        assert_eq!(prepared, ["runner-with-many-groups", "later"]);
        assert_eq!(reports.len(), accounts.len());
        for (account, report) in accounts.iter().zip(&reports) {
            assert_eq!(report.account, account.name);
            assert_eq!(report.operation, operation);
            assert_eq!(report.outcome, AccountHookOutcome::Completed);
            assert_account_child_environment(account);
            assert_rendered_toolchain(
                report,
                "stable",
                match operation {
                    HookOperation::Install | HookOperation::Status => "installed",
                    HookOperation::Uninstall => "removed",
                },
            );
        }
        assert!(operation.completion(&reports).is_ok());
    }
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
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        cargo_tile(),
        HookOperation::Install,
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
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
    let reports = hook::run_account_hooks(
        std::slice::from_ref(&account),
        cargo_tile(),
        HookOperation::Install,
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].outcome, AccountHookOutcome::Completed);
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
    let parent = parent.canonicalize().expect("physical shared parent");
    let other = parent.join("4294967294");
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
    let own_line = settings::capture_root_status(own_status);
    assert!(
        own_line.contains("yours")
            && own_line.contains("readable")
            && own_line.contains("0 active captures"),
        "{own_line}"
    );
    assert!(!own_line.to_lowercase().contains("cleanup"), "{own_line}");
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
    let other_line = settings::capture_root_status(other_status);
    assert!(
        !other_line.to_lowercase().contains("cleanup"),
        "{other_line}"
    );
    assert_eq!(
        other_line,
        format!(
            "{}: owned by {owner_name}, not by 4294967294 — ignored",
            other.display()
        )
    );
    assert_eq!(other_status.confirmed, 0);
    assert!(capture.confirmed().is_empty());
    let parent_line = settings::shared_directory_status(&capture.shared_directory);
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
    let line = settings::capture_root_status(status);
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
        let status = settings::shared_directory_status(&capture.shared_directory);
        assert!(status.contains("mode 1777"), "{status}");
    }
}

#[test]
fn shared_directory_settings_cover_missing_permissions_and_canonical_aliases() {
    let directory = tempfile::tempdir().expect("shared parent fixture");
    let parent = directory.path().join("capture");
    let missing = settings::shared_directory_status(&SharedCaptureDirectory::inspect(&parent));
    assert!(
        missing.contains("created by the first captured cargo run"),
        "{missing}"
    );
    fs::create_dir(&parent).expect("unshared parent");
    set_mode(&parent, 0o755);
    let unshared = settings::shared_directory_status(&SharedCaptureDirectory::inspect(&parent));
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

/// An ancestor alias cannot cause a live registration and its named log to be swept.
#[test]
fn reader_preserves_live_capture_published_under_real_path_and_scanned_through_alias() {
    let directory = tempfile::tempdir().expect("capture alias fixture");
    let real = directory.path().join("real");
    let alias = directory.path().join("alias");
    let parent = real.join("capture");
    let uid = fs::metadata(directory.path()).expect("reader uid").uid();
    let own = parent.join(uid.to_string());
    fs::create_dir_all(own.join("state/pids")).expect("owned capture hierarchy");
    symlink(&real, &alias).expect("alias of the real ancestor");
    let own = own.canonicalize().expect("physical account directory");
    let pid = std::process::id();
    let log = format!("run-live-alias-{pid}.log");
    let registration = own.join("state/pids").join(format!("{pid}.live-alias"));
    let publication = Command::new("python3")
        .args([
            "-c",
            r"from datetime import datetime, timezone
import os
from pathlib import Path
import subprocess
import sys

pid, directory, log = sys.argv[1:]
if sys.platform == 'linux':
    boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
    birth = Path('/proc/' + pid + '/stat').read_text().rsplit(') ', 1)[1].split()[19]
else:
    environment = dict(os.environ, LC_ALL='C', TZ='UTC0')
    boot = subprocess.run(['sysctl', '-n', 'kern.bootsessionuuid'], check=True,
                          capture_output=True, text=True, env=environment).stdout.strip()
    started = subprocess.run(['ps', '-o', 'lstart=', '-p', pid], check=True,
                             capture_output=True, text=True, env=environment).stdout.strip()
    birth = str(int(datetime.strptime(started, '%a %b %d %H:%M:%S %Y')
                    .replace(tzinfo=timezone.utc).timestamp()))
fields = ['cargo-tile-v2', 'live-alias', boot, birth, log, directory, '', '1', 'build', '']
sys.stdout.buffer.write(b'\0'.join(os.fsencode(field) for field in fields))
",
        ])
        .arg(pid.to_string())
        .arg(&own)
        .arg(&log)
        .output()
        .expect("observe current process birth independently of the reader");
    assert!(publication.status.success(), "{publication:?}");
    assert!(publication.stderr.is_empty(), "{publication:?}");
    fs::write(&registration, &publication.stdout).expect("publish live registration");
    let log = own.join(log);
    let progress = b"Blocking waiting for file lock on build directory\n";
    fs::write(&log, progress).expect("publish live log");
    let roots = CaptureRoots::from_parent(&alias.join("capture"));

    for _ in 0..2 {
        let capture = Capture::take(&roots);
        assert_eq!(capture.root_status.len(), 1);
        let status = &capture.root_status[0];
        assert_eq!(status.root.path, own);
        assert_eq!(status.root.cleanup, CaptureCleanup::Here);
        assert_eq!(status.state, RootReadStatus::Readable);
        assert_eq!(status.confirmed, 1, "{:?}", status.diagnostics);
        assert_eq!(capture.confirmed().len(), 1);
        assert_eq!(capture.confirmed()[0].key.pid, pid);
        assert_eq!(
            fs::read(&registration).expect("live registration survives owned sweep"),
            publication.stdout
        );
        assert_eq!(
            fs::read(&log).expect("live log survives owned sweep"),
            progress
        );
    }

    // A running older shim may retain a calendar-derived Darwin boot stamp.
    let mut fields: Vec<_> = publication.stdout.split(|byte| *byte == 0).collect();
    fields[2] = b"{ sec = 100, usec = 23 }";
    let legacy = fields.join(&0);
    fs::write(&registration, &legacy).expect("publish legacy Darwin boot identity");
    let capture = Capture::take(&roots);
    assert!(capture.confirmed().is_empty());
    assert!(capture.root_status[0].diagnostics.iter().any(|diagnostic| {
        matches!(diagnostic, CaptureDiagnostic::IdentityUnknown(path) if path == &registration)
    }));
    assert_eq!(
        fs::read(&registration).expect("unknown identity never authorizes a live unlink"),
        legacy
    );
    assert_eq!(fs::read(&log).expect("unknown live log survives"), progress);
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
    let other = other
        .canonicalize()
        .expect("physical foreign-owned directory");
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
