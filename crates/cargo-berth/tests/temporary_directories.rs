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
