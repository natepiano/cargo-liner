#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Repository builders release their temporary storage on return and unwinding.

use std::env;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::symlink;
use std::os::unix::process::ExitStatusExt as _;
use std::panic;
use std::path::Path;
use std::process::Stdio;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::DirectorySnapshot;
use cargo_berth_test_support::FixtureCommand as Command;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::TestProcessGroup;
use cargo_berth_test_support::copy_git_repository_seed;
use cargo_berth_test_support::fixture_git_command;

const EXPECTED_SCRATCH_PARENT: &str = "CARGO_BERTH_SCRATCH_EXPECTED_PARENT";
const FIXTURE_EXIT_ENVIRONMENT: &str = "CARGO_BERTH_TEMPORARY_FIXTURE_EXIT";
const FIXTURE_PANIC: &str = "exercise repository cleanup while unwinding";
const SCRATCH_PARENT_OVERRIDE: &str = "CARGO_BERTH_TEST_SCRATCH_PARENT";
const TEST_NAME: &str = "repository_builders_leave_no_temporary_directory_after_return_or_panic";

#[test]
fn scratch_wrapper_names_leaks_and_preserves_clean_panics() -> Result<(), Box<dyn std::error::Error>>
{
    const CHILD_MODE: &str = "CARGO_BERTH_SCRATCH_PROOF";
    const NAME: &str = "scratch_wrapper_names_leaks_and_preserves_clean_panics";
    if let Ok(mode) = env::var(CHILD_MODE) {
        return run_scratch_proof_child(&mode);
    }

    let wrapper =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/check-test-scratch.sh");
    for mode in ["tmp", "fixture", "panic", "exit", "signal"] {
        let proof = cargo_berth_test_support::fixture_tempdir("scratch-proof-");
        let binaries = proof.path().join("debug/deps");
        fs::create_dir_all(&binaries)?;
        let executable = binaries.join("proof-test");
        symlink(env::current_exe()?, &executable)?;
        let proof_root = fs::canonicalize(proof.path())?;
        let mut command = Command::new(&wrapper);
        let expected_parent = if mode == "tmp" {
            command.env_remove(SCRATCH_PARENT_OVERRIDE);
            proof_root.join("tmp/cargo-berth-templates/private")
        } else {
            let override_parent = proof_root.join("override");
            command.env(SCRATCH_PARENT_OVERRIDE, &override_parent);
            override_parent
        };
        command
            .arg(executable)
            .args(["--exact", NAME, "--nocapture"])
            .env(CHILD_MODE, mode)
            .env("NEXTEST_TEST_NAME", format!("{NAME}::{mode}"))
            .env(EXPECTED_SCRATCH_PARENT, &expected_parent);
        let output = command.bounded_output()?;
        let stderr = String::from_utf8(output.stderr)?;
        let remaining = fs::read_dir(&expected_parent)?.collect::<Result<Vec<_>, _>>()?;
        if matches!(mode, "tmp" | "fixture") {
            assert_eq!(output.status.code(), Some(1), "{stderr}");
            assert!(
                stderr.contains(&format!("test {NAME}::{mode} left scratch paths:")),
                "{stderr}"
            );
            assert_eq!(remaining.len(), 0);
            let leaked = stderr
                .lines()
                .find(|line| line.starts_with("  "))
                .map(|line| Path::new(line.trim()))
                .ok_or("the wrapper should report the leaked path")?;
            assert!(!leaked.exists());
            assert!(stderr.contains(&leaked.display().to_string()), "{stderr}");
            println!("{mode}: {stderr}");
        } else {
            if mode == "signal" {
                assert_eq!(output.status.signal(), Some(15), "{stderr}");
            } else {
                assert_eq!(
                    output.status.code(),
                    Some(if mode == "panic" { 101 } else { 7 }),
                    "{stderr}"
                );
            }
            assert!(remaining.is_empty(), "{mode}: {remaining:?}");
            assert!(!stderr.contains("left scratch paths"), "{stderr}");
            if mode == "panic" {
                assert!(
                    stderr.contains("deliberate panic after TempDir ownership"),
                    "{stderr}"
                );
            }
        }
    }
    Ok(())
}

/// Execute one scratch ownership scenario in the wrapper's child process.
fn run_scratch_proof_child(mode: &str) -> Result<(), Box<dyn std::error::Error>> {
    let expected_parent = env::var_os(EXPECTED_SCRATCH_PARENT)
        .ok_or("the proof needs its expected scratch parent")?;
    assert_eq!(
        env::temp_dir().parent(),
        Some(Path::new(&expected_parent)),
        "scratch parent for {mode}"
    );
    if mode == "panic" {
        let temporary = tempfile::tempdir()?;
        let fixture = cargo_berth_test_support::fixture_tempdir("panic-fixture-");
        assert_eq!(temporary.path().parent(), fixture.path().parent());
        eprintln!("deliberate panic after TempDir ownership is established");
        panic::resume_unwind(Box::new("exercise scratch cleanup while unwinding"));
    }
    if mode == "exit" {
        std::process::exit(7);
    }
    if mode == "signal" {
        rustix::process::kill_process(rustix::process::getpid(), rustix::process::Signal::TERM)?;
        return Err("signal proof process should terminate".into());
    }
    // Only this subprocess creates a deliberate leak. The wrapper reports and removes it.
    let root = match mode {
        "tmp" => env::temp_dir(),
        "fixture" => {
            let fixture = cargo_berth_test_support::fixture_tempdir("proof-");
            fixture
                .path()
                .parent()
                .ok_or("fixture needs a parent")?
                .to_owned()
        },
        _ => return Err("unknown scratch proof mode".into()),
    };
    let discovery = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&root)
        .bounded_output()?;
    assert!(!discovery.status.success());
    assert!(String::from_utf8(discovery.stderr)?.contains("not a git repository"));
    fs::create_dir(root.join("deliberately-left-folder"))?;
    Ok(())
}

#[test]
fn shared_shell_scripts_use_the_repository_fixture_cache_root() {
    // This assertion checks hard links on the cache filesystem.
    let parent = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cargo-berth-templates/private");
    fs::create_dir_all(&parent).expect("cache fixture parent should exist");
    let temporary = tempfile::Builder::new()
        .prefix("shared-script-")
        .tempdir_in(parent)
        .expect("script fixture should share the cache filesystem");
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
            assert_eq!(metadata.dev(), installed.dev());
            assert_eq!(
                metadata.ino(),
                installed.ino(),
                "same-device installation should hard link"
            );
            true
        });
    assert!(
        cached,
        "installed script should link to {}",
        cache.display()
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
            let repository = cargo_berth_test_support::fixture_tempdir("repository-builder-");
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
            .bounded_output()
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
    let directory = tempfile::tempdir().expect("pause directory should exist");
    let fifo = directory.path().join("continue");
    let ready = directory.path().join("ready");
    let created = Command::new("mkfifo")
        .arg(&fifo)
        .bounded_status()
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
            .bounded_status();
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
            .bounded_status();
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
        .bounded_output()
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

#[test]
fn copied_git_seeds_keep_staged_changes_and_refs_private() -> Result<(), Box<dyn std::error::Error>>
{
    fn git(root: &Path, arguments: &[&str]) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        let output = fixture_git_command()
            .args(arguments)
            .current_dir(root)
            .bounded_output()?;
        assert!(output.status.success(), "{output:?}");
        Ok(output.stdout)
    }
    fn build(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
        git(root, &["init", "--quiet", "--initial-branch=main"])?;
        git(root, &["config", "user.name", "Seed Test"])?;
        git(root, &["config", "user.email", "seed@example.invalid"])?;
        fs::write(root.join("tracked.txt"), "initial\n")?;
        git(root, &["add", "tracked.txt"])?;
        git(root, &["commit", "--quiet", "-m", "initial"])?;
        Ok(())
    }
    let first = copy_git_repository_seed("private-index-and-refs", build);
    fs::write(first.path().join("tracked.txt"), "changed\n")?;
    git(first.path(), &["add", "tracked.txt"])?;
    git(first.path(), &["branch", "only-first"])?;
    let second = copy_git_repository_seed("private-index-and-refs", build);
    assert_eq!(
        fs::read_to_string(second.path().join("tracked.txt"))?,
        "initial\n"
    );
    assert_eq!(
        git(first.path(), &["diff", "--cached", "--name-only"])?,
        b"tracked.txt\n"
    );
    assert_eq!(
        git(second.path(), &["diff", "--cached", "--name-only"])?,
        [] as [u8; 0]
    );
    assert_eq!(
        git(second.path(), &["branch", "--list", "only-first"])?,
        [] as [u8; 0]
    );
    git(
        second.path(),
        &["commit", "--allow-empty", "--quiet", "-m", "only second"],
    )?;
    assert_ne!(
        git(first.path(), &["rev-parse", "HEAD"])?,
        git(second.path(), &["rev-parse", "HEAD"])?
    );
    Ok(())
}

#[test]
fn collecting_test_process_output_drains_both_pipes_and_ends_descendants() {
    let directory = tempfile::tempdir().expect("output fixture should exist");
    let payload = vec![b'x'; 1024 * 1024];
    let payload_path = directory.path().join("payload");
    fs::write(&payload_path, &payload).expect("pipe payload should write");
    let mut process = TestProcessGroup::spawn(
        Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 600 & cat \"$1\"; cat \"$1\" >&2",
                "output-parent",
            ])
            .arg(&payload_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
    .expect("output process group should start");
    let group = process.child_mut().id();
    let (finished, output) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let _ = finished.send(process.wait_with_output());
    });
    let collected = output.recv_timeout(Duration::from_millis(500));
    // This deadline also bounds a guard that waits for inherited pipe writers.
    if collected.is_err() {
        let _ = Command::new("/bin/sh")
            .args([
                "-c",
                "kill -KILL \"$1\"",
                "output-deadline",
                &format!("-{group}"),
            ])
            .bounded_status();
    }
    let deadline = Instant::now() + Duration::from_millis(250);
    let mut remaining = process_group_members(group);
    while !remaining.is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
        remaining = process_group_members(group);
    }
    let captured = collected
        .expect("normal output collection must not wait for a surviving descendant")
        .expect("both output pipes should read");
    assert!(captured.status.success());
    assert_eq!(captured.stdout, payload);
    assert_eq!(captured.stderr, payload);
    assert_eq!(
        remaining,
        Vec::<String>::new(),
        "all output writers must end"
    );
}
