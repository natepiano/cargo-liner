#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Repository builders release their temporary storage on return and unwinding.

use std::env;
use std::fs;
use std::panic;
use std::process::Command;

use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::IntegrationRepository;

const FIXTURE_EXIT_ENVIRONMENT: &str = "CARGO_BERTH_TEMPORARY_FIXTURE_EXIT";
const FIXTURE_PANIC: &str = "exercise repository cleanup while unwinding";
const TEST_NAME: &str = "repository_builders_leave_no_temporary_directory_after_return_or_panic";

#[test]
fn shared_shell_scripts_use_the_repository_fixture_cache_root() {
    use std::os::unix::fs::MetadataExt;

    let temporary = tempfile::tempdir().expect("script destination should exist");
    let script = temporary.path().join("script");
    DirectorySnapshot::write_shared_shell_script(&script, "#!/bin/sh\nprintf cache-root\n");
    let installed = fs::metadata(&script).expect("installed script should exist");
    let contents = fs::read(&script).expect("installed script should read");
    let cache =
        std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("cargo-berth-templates/executables");
    let cached = fs::read_dir(&cache)
        .expect("shared scripts should live beside the repository fixture caches")
        .map(|entry| entry.expect("cached executable entry should read").path())
        .any(|path| {
            if fs::read(&path).expect("cached executable should read") != contents {
                return false;
            }
            let metadata = fs::metadata(path).expect("cached executable should exist");
            if metadata.dev() == installed.dev() {
                assert_eq!(
                    metadata.ino(),
                    installed.ino(),
                    "same-device installation should hard link"
                );
            }
            true
        });
    assert!(
        cached,
        "installed script should link to {}",
        cache.display()
    );
}

#[cfg(target_os = "linux")]
#[test]
fn shared_shell_scripts_restore_across_filesystems() {
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;

    let temporary = tempfile::tempdir_in("/dev/shm")
        .expect("a private directory on the shared-memory filesystem should exist");
    let build =
        fs::metadata(env!("CARGO_TARGET_TMPDIR")).expect("fixture build directory should exist");
    assert_ne!(
        fs::metadata(temporary.path())
            .expect("destination metadata should read")
            .dev(),
        build.dev(),
        "this regression needs a filesystem distinct from the build cache"
    );
    let script = temporary.path().join("script");
    DirectorySnapshot::write_shared_shell_script(&script, "#!/bin/sh\nprintf cross-device\n");
    let contents = fs::read(&script).expect("copied script should read");
    assert!(contents.ends_with(b"printf cross-device\n"));
    assert_eq!(
        fs::metadata(&script)
            .expect("copied script should exist")
            .permissions()
            .mode()
            & 0o777,
        0o755,
        "cross-filesystem copies must retain executable permissions"
    );
    fs::remove_file(&script).expect("private script should unlink");
    let managed_hook = "#!/bin/sh\n# cargo-berth managed hook: cross-device\nexit 0\n";
    fs::write(&script, managed_hook).expect("managed hook should write");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755))
        .expect("managed hook should execute");
    let snapshot = DirectorySnapshot::capture(temporary.path());
    snapshot.restore(temporary.path());
    assert_eq!(
        fs::read_to_string(&script).expect("restored hook should read"),
        managed_hook
    );
    assert_eq!(
        fs::metadata(&script)
            .expect("restored hook should exist")
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}

#[test]
fn repository_builders_leave_no_temporary_directory_after_return_or_panic() {
    if let Ok(exit) = env::var(FIXTURE_EXIT_ENVIRONMENT) {
        assert!(
            matches!(exit.as_str(), "return" | "panic"),
            "unknown fixture exit: {exit}"
        );
        let result = panic::catch_unwind(|| {
            let repository = tempfile::tempdir().expect("repository destination should exist");
            IntegrationRepository::build_main_template(
                repository.path(),
                env!("CARGO_BIN_EXE_cargo-berth"),
            );
            if exit == "panic" {
                panic::resume_unwind(Box::new(FIXTURE_PANIC));
            }
        });
        if exit == "return" {
            assert!(result.is_ok(), "repository builder must complete");
        } else {
            assert_eq!(
                result
                    .expect_err("fixture must unwind")
                    .downcast_ref::<&str>(),
                Some(&FIXTURE_PANIC),
                "fixture must reach its deliberate panic after building the repository"
            );
        }
        return;
    }

    for exit in ["return", "panic"] {
        let temporary = tempfile::tempdir().expect("private temporary root should exist");
        let output = Command::new(env::current_exe().expect("test binary should resolve"))
            .args(["--exact", TEST_NAME, "--nocapture"])
            .env(FIXTURE_EXIT_ENVIRONMENT, exit)
            .env("TMPDIR", temporary.path())
            .env("TMP", temporary.path())
            .env("TEMP", temporary.path())
            .output()
            .expect("isolated fixture should run");
        assert!(output.status.success(), "{exit}: {output:?}");
        let remaining = fs::read_dir(temporary.path())
            .expect("private temporary root should read")
            .map(|entry| entry.expect("temporary entry should read").file_name())
            .collect::<Vec<_>>();
        assert!(
            remaining.is_empty(),
            "repository builder left temporary storage after {exit}: {remaining:?}"
        );
    }
}

#[test]
fn dropping_a_paused_test_process_ends_its_entire_process_group() {
    use std::process::Stdio;
    use std::thread;
    use std::time::Duration;
    use std::time::Instant;

    use cargo_berth_test_support::TestProcessGroup;

    let directory = tempfile::tempdir().expect("pause directory should exist");
    let fifo = directory.path().join("continue");
    let ready = directory.path().join("ready");
    let created = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo should run");
    assert!(created.success());
    let mut process = TestProcessGroup::spawn(
        Command::new("/bin/sh")
            .args([
                "-c",
                r#"
/bin/sh -c 'sleep 600 & printf ready > "$1"; read continuation < "$2"; wait' shim "$1" "$2" &
wait
"#,
                "parent",
            ])
            .arg(&ready)
            .arg(&fifo)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("paused process group should start");
    let group = process.child_mut().id();
    let deadline = Instant::now() + Duration::from_millis(250);
    while !ready.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(ready.exists(), "shim should reach the FIFO pause");
    let members_before = process_group_members(group);
    assert!(
        members_before.len() >= 3,
        "parent, shim and descendant should exist: {members_before:?}"
    );
    let (finished, cleanup) = std::sync::mpsc::channel();
    thread::spawn(move || {
        drop(process);
        finished
            .send(())
            .expect("test should still be observing cleanup");
    });
    let cleaned = cleanup.recv_timeout(Duration::from_millis(250));
    // This deadline is independent of the guard: even a broken Drop cannot hang the test.
    if cleaned.is_err() {
        let _ = Command::new("/bin/sh")
            .args([
                "-c",
                "kill -KILL \"$1\"",
                "cleanup-deadline",
                &format!("-{group}"),
            ])
            .status();
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut remaining = process_group_members(group);
    while !remaining.is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
        remaining = process_group_members(group);
    }
    // A broken guard must fail without leaving the negative-proof fixture running.
    if !remaining.is_empty() {
        let _ = Command::new("/bin/sh")
            .args([
                "-c",
                "kill -KILL \"$1\"",
                "cleanup-failed-test",
                &format!("-{group}"),
            ])
            .status();
    }
    assert!(
        cleaned.is_ok(),
        "guard cleanup exceeded its deadline: {cleaned:?}"
    );
    assert_eq!(
        remaining,
        Vec::<String>::new(),
        "all group members must be reaped after drop"
    );
}

fn process_group_members(group: u32) -> Vec<String> {
    let output = Command::new("ps")
        .args(["-e", "-o", "pid=,pgid=,stat="])
        .output()
        .expect("process table should read");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("process table should be UTF-8")
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .nth(1)
                .and_then(|value| value.parse::<u32>().ok())
                == Some(group)
        })
        .map(str::to_owned)
        .collect()
}
