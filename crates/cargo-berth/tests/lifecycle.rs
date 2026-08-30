#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for reservation lifecycle and retained git evidence.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const EXECUTABLE_PERMISSIONS: u32 = 0o755;
const FAILED_REFERENCE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_FAILED_REFERENCE";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const GIT_WRAPPER_TIMEOUT: Duration = Duration::from_secs(60);
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
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const REFERENCE_FAILURE_DIAGNOSTIC: &str = "injected reference backend failure";
const REFERENCE_FAILURE_DIAGNOSTIC_ENVIRONMENT: &str =
    "CARGO_BERTH_TEST_REFERENCE_FAILURE_DIAGNOSTIC";
const REFERENCE_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    command_name="$2"
    (
        shift 2
        command_line="$command_name"
        for argument in "$@"; do command_line="$command_line $argument"; done
        printf '%s\n' "$command_line" >> "$CARGO_BERTH_TEST_REFERENCE_TRACE"
    )
    if [ "$command_name" = "rev-parse" ]; then
        for argument in "$@"; do
            if [ "$argument" = "$CARGO_BERTH_TEST_FAILED_REFERENCE" ]; then
                reference_query_count=0
                if [ -f "$CARGO_BERTH_TEST_REFERENCE_QUERY_COUNT" ]; then
                    IFS= read -r reference_query_count < "$CARGO_BERTH_TEST_REFERENCE_QUERY_COUNT"
                fi
                reference_query_count=$((reference_query_count + 1))
                printf '%s\n' "$reference_query_count" > "$CARGO_BERTH_TEST_REFERENCE_QUERY_COUNT"
                if [ "$reference_query_count" -gt 1 ]; then
                    printf '%s\n' "$CARGO_BERTH_TEST_REFERENCE_FAILURE_DIAGNOSTIC" >&2
                    exit 128
                fi
            fi
        done
    fi
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const REFERENCE_QUERY_COUNT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REFERENCE_QUERY_COUNT";
const REFERENCE_TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REFERENCE_TRACE";
const RETENTION_REF_PREFIX: &str = "refs/cargo-berth/reservations/";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

#[derive(Clone, Copy)]
enum GitReferenceStorage {
    Loose,
    Reftable,
}

#[derive(Clone, Copy)]
enum ReferenceQueryBehavior<'revision> {
    Observe,
    FailTrunkRevision(&'revision str),
}

struct ClaimGitTrace {
    output:   Output,
    commands: Vec<String>,
}

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
fn resolve_reports_failed_session_mapping_retirement() {
    let repository = initialized_repository();
    let session_id = "resolve-retirement-failure";
    let claim = run_berth_with_session(
        repository.path(),
        &["claim", "file:session-mapped", "--run", FIRST_RUN, "--json"],
        session_id,
    );
    assert!(claim.status.success());
    let reservation_id = reservation_id(&claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    assert!(
        fs::read_to_string(&mapping_path)
            .expect("session mapping should read")
            .contains(session_id)
    );
    fs::remove_file(&mapping_path).expect("session mapping should remove");
    fs::create_dir(&mapping_path).expect("mapping destination directory should exist");

    let resolved = run_berth(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--abandon",
            "--why",
            "confirmed mapped work abandonment",
            "--json",
        ],
    );
    let resolved_json = json_output(&resolved);

    assert!(resolved.status.success());
    assert_eq!(resolved_json["payload"]["data"]["status"], "released");
    assert_eq!(
        resolved_json["payload"]["data"]["session_mapping_publication"]["status"],
        "unavailable"
    );
    assert!(
        resolved_json["message"].as_str().is_some_and(
            |message| message.contains("harness session mapping could not be published")
        )
    );
}

#[test]
fn released_reservation_stays_clear_after_trunk_rewrite_without_git_on_check() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
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
        .current_dir(&second_root)
        .env("PATH", empty_path.path())
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .output()
        .expect("check should run without git");
    assert!(check.status.success());
    assert_eq!(json_output(&check)["status"], "clear");
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
fn missing_trunk_is_recorded_as_an_unresolved_reference() {
    let repository = initialized_repository();
    let missing_reference = "refs/heads/missing-trunk";
    configure_trunk(repository.path(), "missing-trunk");

    let claimed = claim(repository.path(), "file:README.md", FIRST_RUN);

    assert!(
        claimed.status.success(),
        "claim with a missing trunk failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    assert_eq!(
        last_claim_event(repository.path())["trunk_at_claim"]["reference"],
        missing_reference
    );
}

#[test]
fn failed_reference_query_is_not_reported_as_a_missing_trunk() {
    let repository = initialized_repository_with_reference_storage(GitReferenceStorage::Reftable);
    let failed_reference = "refs/heads/main";

    let traced = run_claim_with_git_trace(
        repository.path(),
        ReferenceQueryBehavior::FailTrunkRevision(failed_reference),
    );
    let envelope = json_output(&traced.output);

    assert_eq!(traced.output.status.code(), Some(4));
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert!(
        envelope["message"].as_str().is_some_and(|message| {
            message.contains("git rev-parse failed")
                && message.contains(REFERENCE_FAILURE_DIAGNOSTIC)
                && !message.contains("does not exist")
        }),
        "reference query failure lost its diagnostic: {envelope}"
    );
    assert!(rev_parse_query_count(&traced.commands, failed_reference) > 0);
    assert!(
        journal_events(repository.path())
            .iter()
            .all(|event| event["op"] != "claim"),
        "the failed reference query must not take the absent-trunk claim path"
    );
}

#[test]
fn reftable_references_are_resolved_through_git() {
    let repository = initialized_repository_with_reference_storage(GitReferenceStorage::Reftable);
    let reference = "refs/heads/main";
    let head = git_stdout(repository.path(), &["rev-parse", reference]);

    assert_eq!(
        git_stdout(
            repository.path(),
            &["config", "--get", "extensions.refstorage"]
        ),
        "reftable"
    );
    assert!(!repository.path().join(".git/refs/heads/main").exists());
    assert!(!repository.path().join(".git/packed-refs").exists());

    let traced = run_claim_with_git_trace(repository.path(), ReferenceQueryBehavior::Observe);

    assert!(
        traced.output.status.success(),
        "reftable claim failed: {}",
        String::from_utf8_lossy(&traced.output.stdout)
    );
    assert_eq!(last_claim_event(repository.path())["trunk_at_claim"], head);
    assert!(rev_parse_query_count(&traced.commands, reference) > 0);
}

#[test]
fn loose_reference_reads_spawn_no_git_process() {
    let repository = initialized_repository();
    let loose_reference = repository.path().join(".git/refs/heads/main");
    assert!(loose_reference.is_file());

    let traced = run_claim_with_git_trace(repository.path(), ReferenceQueryBehavior::Observe);

    assert!(
        traced.output.status.success(),
        "loose-reference claim failed: {}",
        String::from_utf8_lossy(&traced.output.stdout)
    );
    assert_eq!(
        reference_lookup_command_count(&traced.commands, "refs/heads/main"),
        0,
        "loose reference read spawned a reference lookup: {:?}",
        traced.commands,
    );
}

#[test]
fn released_reservation_remains_clear_after_git_confirms_an_unresolvable_trunk() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
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
    let check = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(&second_root)
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .output()
        .expect("check should run after git confirms the missing trunk");

    assert!(check.status.success());
    let check_json = json_output(&check);
    assert_eq!(check_json["status"], "clear");
    assert_eq!(
        check_json["payload"]["data"]["acquisition"]["kind"],
        "appended"
    );
    let journal = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should contain the first-touch claim");
    let first_touch_event: serde_json::Value = serde_json::from_str(
        journal
            .lines()
            .next_back()
            .expect("journal should contain a final event"),
    )
    .expect("final journal event should be valid JSON");
    assert_eq!(
        first_touch_event["trunk_at_claim"]["reference"],
        "refs/heads/main"
    );

    fs::remove_file(repository.path().join(PROJECTION_PATH)).expect("projection should delete");
    assert!(run_berth(repository.path(), &["init"]).status.success());
    let replayed_check = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["check", "file:src/lib.rs", "--json"])
        .current_dir(&second_root)
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .output()
        .expect("check should run after replay and missing-trunk confirmation");
    assert!(replayed_check.status.success());
    let replayed_check_json = json_output(&replayed_check);
    assert_eq!(replayed_check_json["status"], "clear");
    assert_eq!(
        replayed_check_json["payload"]["data"]["acquisition"]["kind"],
        "already_held"
    );
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
    let checkpoint_event = fs::read_to_string(newer_run_repository.path().join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .rev()
        .find(|event| {
            event["op"] == "checkpoint" && event["reservation_id"] == newer_run_reservation_id
        })
        .expect("release should append a checkpoint event");
    assert_ne!(
        checkpoint_event["actor"]["run"], SECOND_RUN,
        "the checkpoint must not be authored by the run whose marker reconciliation retired"
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

/// Add a real worktree beside the repository, the only actor berth treats as foreign.
///
/// Two coordination runs inside one worktree are one actor, so a distinct `--run`
/// no longer names a second party. The returned directory owns the worktree and
/// must outlive its use.
fn foreign_worktree(repository: &TempDir, name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("foreign worktree parent should exist");
    let root = directory.path().join(name);
    git(
        repository.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            name,
            root.to_str()
                .expect("foreign worktree path should be UTF-8"),
        ],
    );
    let configuration = root.join(CONFIGURATION_PATH);
    if let Some(parent) = configuration.parent() {
        fs::create_dir_all(parent).expect("foreign worktree configuration should have a directory");
    }
    fs::copy(repository.path().join(CONFIGURATION_PATH), configuration)
        .expect("foreign worktree should share the repository configuration");
    (directory, root)
}

fn initialized_repository() -> TempDir {
    initialized_repository_with_reference_storage(GitReferenceStorage::Loose)
}

fn initialized_repository_with_reference_storage(
    git_reference_storage: GitReferenceStorage,
) -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    match git_reference_storage {
        GitReferenceStorage::Loose => git(
            repository.path(),
            &["init", "--quiet", "--initial-branch=main"],
        ),
        GitReferenceStorage::Reftable => git(
            repository.path(),
            &[
                "init",
                "--quiet",
                "--initial-branch=main",
                "--ref-format=reftable",
            ],
        ),
    }
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

fn configure_trunk(repository_root: &Path, trunk: &str) {
    let configuration_path = repository_root.join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path).expect("configuration should read");
    let configured = configuration.replacen("trunk = \"main\"", &format!("trunk = \"{trunk}\""), 1);
    assert_ne!(configuration, configured, "main trunk setting should exist");
    fs::write(configuration_path, configured).expect("configured trunk should write");
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
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn run_claim_with_git_trace(
    repository_root: &Path,
    reference_query_behavior: ReferenceQueryBehavior<'_>,
) -> ClaimGitTrace {
    let wrapper_directory = tempdir().expect("git wrapper directory should exist");
    let wrapper_path = wrapper_directory.path().join("git");
    let query_count_path = wrapper_directory.path().join("reference-query-count");
    let trace_path = wrapper_directory.path().join("trace");
    fs::write(&wrapper_path, REFERENCE_GIT_WRAPPER).expect("git wrapper should write");
    fs::write(&query_count_path, "0\n").expect("reference query count should initialize");
    fs::write(&trace_path, "").expect("git trace should initialize");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(EXECUTABLE_PERMISSIONS);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let failed_reference = match reference_query_behavior {
        ReferenceQueryBehavior::Observe => "",
        ReferenceQueryBehavior::FailTrunkRevision(reference) => reference,
    };
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["claim", "file:README.md", "--run", FIRST_RUN, "--json"])
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(FAILED_REFERENCE_ENVIRONMENT, failed_reference)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(
            REFERENCE_FAILURE_DIAGNOSTIC_ENVIRONMENT,
            REFERENCE_FAILURE_DIAGNOSTIC,
        )
        .env(REFERENCE_QUERY_COUNT_ENVIRONMENT, &query_count_path)
        .env(REFERENCE_TRACE_ENVIRONMENT, &trace_path)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run with traced git");
    let commands = fs::read_to_string(trace_path)
        .expect("git trace should read")
        .lines()
        .map(str::to_owned)
        .collect();
    ClaimGitTrace { output, commands }
}

fn reference_lookup_command_count(commands: &[String], reference: &str) -> usize {
    commands
        .iter()
        .filter(|command| {
            (command.starts_with("show-ref --exists ") || command.starts_with("rev-parse "))
                && command
                    .split_whitespace()
                    .any(|argument| argument == reference)
        })
        .count()
}

fn rev_parse_query_count(commands: &[String], reference: &str) -> usize {
    commands
        .iter()
        .filter(|command| {
            command.starts_with("rev-parse ")
                && command
                    .split_whitespace()
                    .any(|argument| argument == reference)
        })
        .count()
}

fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    fs::read_to_string(repository_root.join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn last_claim_event(repository_root: &Path) -> serde_json::Value {
    journal_events(repository_root)
        .into_iter()
        .rev()
        .find(|event| event["op"] == "claim")
        .expect("journal should contain a claim event")
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
