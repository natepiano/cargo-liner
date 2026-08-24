#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for reservation lifecycle and retained git evidence.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;

const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT_WRAPPER_TIMEOUT: Duration = Duration::from_secs(5);
const INITIAL_COMMIT_TAG: &str = "initial-state";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const PAUSING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "merge-base" ] && [ "$3" = "--is-ancestor" ]; then
    : > "$CARGO_BERTH_TEST_GIT_READY"
    while [ ! -e "$CARGO_BERTH_TEST_GIT_CONTINUE" ]; do
        sleep 0.01
    done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";

#[test]
fn checkpoint_retains_commit_after_branch_deletion_and_git_gc() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let claim = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&claim);

    let release = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert!(release.status.success());
    assert_eq!(json_output(&release)["status"], "outstanding");
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    assert_eq!(
        git_stdout(
            repository.path(),
            &[
                "rev-parse",
                &format!("{RETENTION_REF_PREFIX}{reservation_id}")
            ],
        ),
        protected_tip
    );

    git(repository.path(), &["switch", "--quiet", "main"]);
    git(repository.path(), &["branch", "-D", "phase"]);
    git(
        repository.path(),
        &["reflog", "expire", "--expire=now", "--all"],
    );
    git(repository.path(), &["gc", "--prune=now"]);

    assert_eq!(
        git_stdout(repository.path(), &["cat-file", "-t", &protected_tip],),
        "commit"
    );
}

#[test]
fn integrated_evidence_returns_to_blocking_after_trunk_rewrite_without_git_on_check() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let claim = claim(repository.path(), "tree:src", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );

    git(repository.path(), &["switch", "--quiet", "main"]);
    git(
        repository.path(),
        &["merge", "--quiet", "--ff-only", "phase"],
    );
    let integrated = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(json_output(&integrated)["status"], "integrated");
    let terminal = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(
        json_output(&terminal)["payload"]["data"]["status"],
        "released"
    );

    git(
        repository.path(),
        &["reset", "--hard", "--quiet", INITIAL_COMMIT_TAG],
    );
    let rewritten = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(json_output(&rewritten)["status"], "trunk_rewritten");
    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(run_berth(repository.path(), &["init"]).status.success());
    let empty_path = tempdir().expect("empty PATH should exist");
    let check = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(repository.path())
        .env("PATH", empty_path.path())
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .output()
        .expect("check should run without git");
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(json_output(&check)["status"], "blocked_by_overlap");
}

#[test]
fn stored_integrated_evidence_is_revalidated_before_release() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let claim = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    git(repository.path(), &["switch", "--quiet", "main"]);
    git(
        repository.path(),
        &["merge", "--quiet", "--ff-only", "phase"],
    );
    let integrated = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(
        json_output(&integrated)["payload"]["data"]["status"],
        "evidence_revalidated"
    );

    git(
        repository.path(),
        &["reset", "--hard", "--quiet", INITIAL_COMMIT_TAG],
    );
    let rewritten = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    let rewritten_json = json_output(&rewritten);

    assert_eq!(rewritten_json["status"], "trunk_rewritten");
    assert_eq!(
        rewritten_json["payload"]["data"]["status"],
        "evidence_revalidated"
    );
    assert_eq!(
        rewritten_json["payload"]["data"]["evidence"]["status"],
        "trunk_rewritten"
    );
}

#[test]
fn foreign_active_reservation_cannot_checkpoint_the_invoking_head() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let foreign_worktree = worktree_parent.path().join("foreign");
    let foreign_worktree_text = foreign_worktree
        .to_str()
        .expect("worktree path should be UTF-8");
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "phase",
            foreign_worktree_text,
        ],
    );
    assert!(run_berth(&foreign_worktree, &["init"]).status.success());
    commit_file(
        &foreign_worktree,
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let claim = claim(&foreign_worktree, "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&claim);

    let release = run_berth(repository.path(), &["release", &reservation_id, "--json"]);

    assert_eq!(release.status.code(), Some(5));
    assert_eq!(json_output(&release)["status"], "invalid_input");
    let retention_ref = Command::new("git")
        .args([
            "rev-parse",
            "--verify",
            &format!("{RETENTION_REF_PREFIX}{reservation_id}"),
        ])
        .current_dir(repository.path())
        .output()
        .expect("retention ref lookup should run");
    assert!(!retention_ref.status.success());
}

#[test]
fn rebase_resnapshot_updates_protected_tip_and_retention_ref() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "first\n",
        "first phase commit",
    );
    let claim = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    commit_file(
        repository.path(),
        "src/lib.rs",
        "second\n",
        "rebased result",
    );
    let replacement_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);

    let resnapshot = run_berth(repository.path(), &["release", &reservation_id, "--json"]);

    assert_eq!(
        json_output(&resnapshot)["payload"]["data"]["status"],
        "resnapshotted"
    );
    assert_eq!(
        git_stdout(
            repository.path(),
            &[
                "rev-parse",
                &format!("{RETENTION_REF_PREFIX}{reservation_id}")
            ],
        ),
        replacement_tip
    );
}

#[test]
fn failed_journal_append_does_not_move_the_retention_ref() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "first\n",
        "first phase commit",
    );
    let claim = claim(repository.path(), "file:src/lib.rs", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    let retention_ref = format!("{RETENTION_REF_PREFIX}{reservation_id}");
    let retained_tip_before = git_stdout(repository.path(), &["rev-parse", &retention_ref]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "replacement\n",
        "replacement phase commit",
    );
    let replacement_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    assert_ne!(retained_tip_before, replacement_tip);

    let wrapper_directory = tempdir().expect("wrapper directory should exist");
    let wrapper_path = wrapper_directory.path().join("git");
    fs::write(&wrapper_path, PAUSING_GIT_WRAPPER).expect("git wrapper should write");
    let mut wrapper_permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    wrapper_permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, wrapper_permissions)
        .expect("git wrapper should be executable");
    let ready_path = wrapper_directory.path().join("ready");
    let continue_path = wrapper_directory.path().join("continue");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let mut release = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["release", &reservation_id, "--json"])
        .current_dir(repository.path())
        .env("PATH", wrapped_path)
        .env("CARGO_BERTH_TEST_GIT_READY", &ready_path)
        .env("CARGO_BERTH_TEST_GIT_CONTINUE", &continue_path)
        .env("CARGO_BERTH_TEST_REAL_GIT", git_binary())
        .env_remove(RUN_ENVIRONMENT)
        .spawn()
        .expect("release should start");
    wait_for_path(&ready_path, &mut release);

    let journal_path = repository.path().join(JOURNAL_PATH);
    let original_permissions = fs::metadata(&journal_path)
        .expect("journal metadata should read")
        .permissions();
    let mut read_only_permissions = original_permissions.clone();
    read_only_permissions.set_mode(original_permissions.mode() & !0o222);
    fs::set_permissions(&journal_path, read_only_permissions)
        .expect("journal should become read-only");
    fs::write(&continue_path, b"continue\n").expect("git wrapper should continue");
    let release = release
        .wait_with_output()
        .expect("release should finish after append failure");
    fs::set_permissions(&journal_path, original_permissions)
        .expect("journal permissions should restore");

    assert_eq!(release.status.code(), Some(4));
    assert_eq!(
        git_stdout(repository.path(), &["rev-parse", &retention_ref]),
        retained_tip_before
    );
}

#[test]
fn unresolvable_trunk_materializes_object_unknown_for_git_free_checks() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let claim = claim(repository.path(), "tree:src", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    git(repository.path(), &["switch", "--quiet", "main"]);
    git(
        repository.path(),
        &["merge", "--quiet", "--ff-only", "phase"],
    );
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    git(repository.path(), &["switch", "--quiet", "--detach"]);
    git(repository.path(), &["update-ref", "-d", "refs/heads/main"]);

    let unknown = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert!(unknown.status.success());
    assert_eq!(json_output(&unknown)["status"], "object_unknown");
    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(run_berth(repository.path(), &["init"]).status.success());
    let empty_path = tempdir().expect("empty PATH should exist");
    let check = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(repository.path())
        .env("PATH", empty_path.path())
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .output()
        .expect("check should run without git");

    assert_eq!(check.status.code(), Some(1));
    assert_eq!(json_output(&check)["status"], "blocked_by_overlap");
}

#[test]
fn release_removes_only_the_marker_for_a_run_without_other_active_reservations() {
    let sole_repository = initialized_repository();
    let sole_claim = claim(sole_repository.path(), "file:a", FIRST_RUN);
    let sole_reservation_id = reservation_id(&sole_claim);
    assert!(
        run_berth(sole_repository.path(), &["release", &sole_reservation_id])
            .status
            .success()
    );
    assert!(!sole_repository.path().join(MARKER_PATH).exists());

    let newer_run_repository = initialized_repository();
    let newer_run_claim = claim(newer_run_repository.path(), "file:a", FIRST_RUN);
    let newer_run_reservation_id = reservation_id(&newer_run_claim);
    fs::write(
        newer_run_repository.path().join(MARKER_PATH),
        format!("{SECOND_RUN}\n"),
    )
    .expect("newer run marker should write");
    assert!(
        run_berth(
            newer_run_repository.path(),
            &["release", &newer_run_reservation_id]
        )
        .status
        .success()
    );
    assert!(
        !newer_run_repository.path().join(MARKER_PATH).exists(),
        "reconciliation should sweep a marker with no matching active reservation"
    );

    let shared_run_repository = initialized_repository();
    let first_claim = claim(shared_run_repository.path(), "file:a", FIRST_RUN);
    let first_reservation_id = reservation_id(&first_claim);
    assert!(
        claim(shared_run_repository.path(), "file:b", FIRST_RUN)
            .status
            .success()
    );
    assert!(
        run_berth(
            shared_run_repository.path(),
            &["release", &first_reservation_id]
        )
        .status
        .success()
    );
    assert_eq!(
        fs::read_to_string(shared_run_repository.path().join(MARKER_PATH))
            .expect("shared marker should remain")
            .trim(),
        FIRST_RUN
    );
}

#[test]
fn lock_contention_is_retryable_while_corrupt_journal_is_unreadable() {
    let repository = initialized_repository();
    let claim = claim(repository.path(), "file:a", FIRST_RUN);
    let reservation_id = reservation_id(&claim);
    let lock_file = fs::File::options()
        .read(true)
        .write(true)
        .open(repository.path().join(LOCK_PATH))
        .expect("mutation lock should open");
    lock_file.lock().expect("mutation lock should lock");

    let init_contention = run_berth(repository.path(), &["init", "--json"]);
    let init_contention_json = json_output(&init_contention);
    assert_eq!(init_contention.status.code(), Some(6));
    assert_eq!(init_contention_json["exit_code"], 6);
    assert_eq!(init_contention_json["status"], "contention");

    let contention = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    let contention_json = json_output(&contention);
    assert_eq!(contention.status.code(), Some(6));
    assert_eq!(contention_json["exit_code"], 6);
    assert_eq!(contention_json["status"], "contention");
    assert!(
        contention_json["message"]
            .as_str()
            .is_some_and(|message| message.contains("retry"))
    );
    std::mem::drop(lock_file);

    fs::write(repository.path().join(JOURNAL_PATH), b"not-json\n")
        .expect("journal corruption should write");
    let unreadable = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    let unreadable_json = json_output(&unreadable);
    assert_eq!(unreadable.status.code(), Some(4));
    assert_eq!(unreadable_json["exit_code"], 4);
    assert_eq!(unreadable_json["status"], "ledger_unreadable");
    assert_eq!(unreadable_json["payload"]["kind"], "no_facts");
}

fn initialized_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch=main"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    git(
        repository.path(),
        &["config", "user.email", "berth@example.invalid"],
    );
    fs::write(repository.path().join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    git(repository.path(), &["tag", INITIAL_COMMIT_TAG]);
    assert!(run_berth(repository.path(), &["init"]).status.success());
    repository
}

fn claim(repository_root: &Path, scope: &str, run: &str) -> Output {
    run_berth(repository_root, &["claim", scope, "--run", run, "--json"])
}

fn reservation_id(claim: &Output) -> String {
    json_output(claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn commit_file(repository_root: &Path, path: &str, contents: &str, message: &str) {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("file parent should exist");
    }
    fs::write(file_path, contents).expect("committed file should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository_root)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output should be UTF-8")
        .trim()
        .to_owned()
}

fn git_binary() -> String {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .expect("git lookup should run");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("git path should be UTF-8")
        .trim()
        .to_owned()
}

fn wait_for_path(path: &Path, child: &mut Child) {
    let deadline = Instant::now() + GIT_WRAPPER_TIMEOUT;
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !path.exists() {
        assert!(child.kill().is_ok(), "timed-out release should stop");
        assert!(child.wait().is_ok(), "stopped release should be reaped");
    }
    assert!(path.exists(), "git wrapper did not reach its pause point");
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}
