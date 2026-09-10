#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for reservation lifecycle and retained git evidence.

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::git_command;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

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
const GIT_UNSPAWNABLE_DIAGNOSTIC: &str = "The reservation ledger could not be read: could not run git: No such file or directory (os error 2)";
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

/// The two runs and the one worktree a `worktree_held_by_another_run` refusal names.
struct WorktreeOccupancy<'facts> {
    /// The run already holding active work in the worktree.
    incumbent_run:            &'facts str,
    /// The active reservation that run holds.
    incumbent_reservation_id: &'facts str,
    /// The run the refused command presented.
    issuing_run:              &'facts str,
    /// The checkout both runs name.
    issuing_root:             &'facts Path,
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

/// Releases needed before a merged reservation carries its disposition.
///
/// One revalidates the integration evidence and one records what it proved; a
/// third would be the no-op this test is about.
const RELEASES_TO_A_DISPOSITION: usize = 2;

#[test]
fn a_release_that_changed_nothing_does_not_repeat_the_sentence_of_one_that_acted() {
    let repository = initialized_repository();
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "track berth configuration",
        ],
    );
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

    // Reaching a disposition takes one release to revalidate the evidence and one
    // to record what it proved; which of the two a given call performs depends on
    // what reconciliation had already materialized.
    let mut acted = String::new();
    for _ in 0..RELEASES_TO_A_DISPOSITION {
        let release = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
        let release = json_output(&release);
        if release["payload"]["data"]["status"] == "released" {
            acted = release["message"]
                .as_str()
                .expect("release should carry a message")
                .to_owned();
            break;
        }
    }
    assert!(
        !acted.is_empty(),
        "the reservation should reach a disposition"
    );

    let repeated = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    let repeated = json_output(&repeated);
    assert_eq!(repeated["payload"]["data"]["status"], "already_settled");
    let unchanged = repeated["message"]
        .as_str()
        .expect("repeated release should carry a message");
    assert_ne!(
        unchanged, acted,
        "a release that did nothing must read differently from one that acted"
    );
    assert!(
        unchanged.contains("was already released"),
        "a repeated release should name the disposition it found: {unchanged}"
    );
}

#[test]
fn released_reservation_stays_clear_after_trunk_rewrite_without_git_on_check() {
    let repository = initialized_repository();
    // This fixture exercises a released holder only; commits must not add live first touches.
    git(
        repository.path(),
        &["config", "core.hooksPath", "/dev/null"],
    );
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "track berth configuration"],
    );
    git(repository.path(), &["tag", "--force", INITIAL_COMMIT_TAG]);
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
    let claim_count = fs::read_to_string(repository.path().join(JOURNAL_PATH))
        .expect("journal should read")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should decode"))
        .filter(|event| event["op"] == "claim")
        .count();
    assert_eq!(
        claim_count, 1,
        "fixture must contain only the released holder"
    );
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
    let retention_ref = git_command(BERTH_EXECUTABLE)
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
fn unspawnable_git_preserves_the_complete_io_diagnostic() {
    let repository = initialized_repository();
    let empty_path = tempdir().expect("empty PATH directory should exist");

    let output = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["init", "--json"])
        .current_dir(repository.path())
        .env("PATH", empty_path.path())
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should remain spawnable when git is unavailable");
    let envelope = json_output(&output);
    let diagnostic = envelope["message"]
        .as_str()
        .expect("Git failure should carry a diagnostic");

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert_eq!(diagnostic.as_bytes(), GIT_UNSPAWNABLE_DIAGNOSTIC.as_bytes());
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

#[test]
fn claim_refuses_a_second_coordination_run_in_the_incumbents_worktree() {
    let repository = initialized_repository();
    let incumbent_id = reservation_id(&claim(repository.path(), "file:held.rs", FIRST_RUN));

    let refused = claim(repository.path(), "file:separate.rs", SECOND_RUN);
    let refused_json = json_output(&refused);

    assert_eq!(refused.status.code(), Some(5));
    assert_eq!(refused_json["status"], "invalid_input");
    assert_worktree_held_by_another_run(
        &refused_json,
        &WorktreeOccupancy {
            incumbent_run:            FIRST_RUN,
            incumbent_reservation_id: &incumbent_id,
            issuing_run:              SECOND_RUN,
            issuing_root:             repository.path(),
        },
    );
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        1,
        "the refused claim should journal nothing"
    );
}

#[test]
fn check_refuses_a_second_coordination_run_in_the_incumbents_worktree() {
    let repository = initialized_repository();
    let incumbent_id = reservation_id(&claim(repository.path(), "file:held.rs", FIRST_RUN));

    let refused = run_berth_with_run(
        repository.path(),
        &["check", "file:separate.rs", "--json"],
        SECOND_RUN,
    );
    let refused_json = json_output(&refused);

    assert_eq!(refused.status.code(), Some(5));
    assert_worktree_held_by_another_run(
        &refused_json,
        &WorktreeOccupancy {
            incumbent_run:            FIRST_RUN,
            incumbent_reservation_id: &incumbent_id,
            issuing_run:              SECOND_RUN,
            issuing_root:             repository.path(),
        },
    );
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        1,
        "the refused first-touch check should journal nothing"
    );
}

/// The action the refusal publishes must clear the refusal rather than repeat it.
///
/// A marker sweep is kept deliberately away from a run that still holds an `Active`
/// reservation, so an action that only sweeps returns the caller to this same refusal.
/// Releasing the incumbent is a repair: the holder stops being `Active`, and the worktree
/// admits the run it just refused.
#[test]
fn the_occupancy_refusals_recovery_action_admits_the_run_it_refused() {
    let repository = initialized_repository();
    let incumbent_id = reservation_id(&claim(repository.path(), "file:held.rs", FIRST_RUN));
    let refused = claim(repository.path(), "file:separate.rs", SECOND_RUN);
    let actions = json_output(&refused)["payload"]["data"]["recovery_actions"]
        .as_array()
        .expect("the refusal should carry recovery actions")
        .clone();
    assert!(!actions.is_empty());

    for action in &actions {
        let performed = run_recovery_action(action);
        assert!(
            performed.status.success(),
            "a published recovery action should run where it is published: {}",
            String::from_utf8_lossy(&performed.stderr)
        );
    }

    let retried = claim(repository.path(), "file:separate.rs", SECOND_RUN);
    assert!(
        retried.status.success(),
        "following the published recovery actions should admit the refused run: {}",
        String::from_utf8_lossy(&retried.stdout)
    );
    assert_ne!(
        reservation_id(&retried),
        incumbent_id,
        "the admitted run should take a reservation of its own"
    );
}

/// An incumbent that released but has not integrated must not lock its own worktree out.
///
/// Commit `1b74ad02` fixed exactly this: a worktree blocking itself with what its
/// previous session left behind. The occupancy rule is `Active`-only so that stays fixed.
#[test]
fn an_outstanding_holder_from_another_run_refuses_nothing() {
    let repository = initialized_repository();
    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "incumbent work\n",
        "incumbent work",
    );
    let incumbent_id = reservation_id(&claim(repository.path(), "tree:src", FIRST_RUN));
    let released = run_berth(repository.path(), &["release", &incumbent_id, "--json"]);
    assert!(released.status.success());
    assert_eq!(json_output(&released)["status"], "outstanding");

    let successor = claim(repository.path(), "tree:src", SECOND_RUN);
    assert!(
        successor.status.success(),
        "an outstanding holder should not refuse a new run: {}",
        String::from_utf8_lossy(&successor.stdout)
    );
    let checked = run_berth_with_run(
        repository.path(),
        &["check", "file:src/lib.rs", "--json"],
        SECOND_RUN,
    );
    assert!(
        checked.status.success(),
        "an outstanding holder should not refuse an edit: {}",
        String::from_utf8_lossy(&checked.stdout)
    );
    assert_eq!(json_output(&checked)["status"], "clear");

    commit_file(
        repository.path(),
        "src/lib.rs",
        "successor work\n",
        "successor work",
    );

    assert!(
        journal_events(repository.path())
            .iter()
            .all(|event| event["op"] != "incursion"),
        "an outstanding holder should raise no incursion against its own worktree"
    );
}

/// Two harness sessions sharing one checkout are one run, and this change must keep it so.
///
/// Identity resolution creates a run only when nothing else names the caller, so the
/// second session reads the worktree's own slot and continues the incumbent run.
#[test]
fn a_second_harness_session_adopts_the_incumbent_run_without_refusal() {
    let repository = initialized_repository();
    let incumbent = run_berth_with_session(
        repository.path(),
        &["claim", "file:held.rs", "--run", FIRST_RUN, "--json"],
        "incumbent-session",
    );
    assert!(incumbent.status.success());
    let incumbent_id = reservation_id(&incumbent);

    let adopted = run_berth_with_session(
        repository.path(),
        &["check", "file:adopted.rs", "--json"],
        "adopting-session",
    );
    let adopted_json = json_output(&adopted);

    assert!(
        adopted.status.success(),
        "a second harness session adopts the incumbent run: {}",
        String::from_utf8_lossy(&adopted.stdout)
    );
    assert_eq!(adopted_json["status"], "clear");
    assert_eq!(
        adopted_json["payload"]["data"]["acquisition"]["kind"], "widened",
        "one run's own reservation grows rather than gaining a sibling"
    );
    assert_eq!(
        adopted_json["reservations"],
        serde_json::json!([incumbent_id])
    );
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "claim")
            .count(),
        1,
        "the adopting session should grow the incumbent's reservation, not open a second"
    );
    assert_eq!(
        last_claim_event(repository.path())["actor"]["run"],
        FIRST_RUN
    );
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree exercises the cross-worktree half of these tests. The returned directory owns
/// the worktree and must outlive its use.
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

/// Run one published recovery action exactly as the refusal published it.
fn run_recovery_action(action: &serde_json::Value) -> Output {
    let arguments = action["argv"]
        .as_array()
        .expect("a recovery action should carry an argument vector")
        .iter()
        .skip(1)
        .map(|argument| {
            argument
                .as_str()
                .expect("a recovery action argument should be text")
                .to_owned()
        })
        .collect::<Vec<_>>();
    let working_directory = action["cwd"]
        .as_str()
        .expect("a recovery action should carry a working directory");
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(&arguments)
        .current_dir(working_directory)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("a published recovery action should run")
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

fn run_berth_with_run(repository_root: &Path, arguments: &[&str], run: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env(RUN_ENVIRONMENT, run)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

/// Assert the refusal a second coordination run receives in an occupied worktree.
///
/// The refusal is caller-repairable, so it names both runs, the incumbent's reservation,
/// the checkout they share, and one recovery action that runs where the caller stands and
/// actually performs the repair. A sweep would not: it keeps a marker whose run still holds
/// an `Active` reservation, so it returns the caller to this same refusal.
fn assert_worktree_held_by_another_run(
    envelope: &serde_json::Value,
    occupancy: &WorktreeOccupancy<'_>,
) {
    assert_eq!(envelope["payload"]["kind"], "coordination_identity");
    let rejection = &envelope["payload"]["data"];
    assert_eq!(rejection["kind"], "worktree_held_by_another_run");
    assert_eq!(
        rejection["incumbent_coordination_run_id"],
        occupancy.incumbent_run
    );
    assert_eq!(
        rejection["incumbent_reservation_id"],
        occupancy.incumbent_reservation_id
    );
    assert_eq!(
        rejection["issuing_coordination_run_id"],
        occupancy.issuing_run
    );
    assert!(
        rejection["issuing_worktree_id"]
            .as_str()
            .is_some_and(|worktree_id| !worktree_id.is_empty())
    );
    let issuing_root = fs::canonicalize(occupancy.issuing_root)
        .expect("issuing root should canonicalize")
        .to_str()
        .expect("issuing root should be UTF-8")
        .to_owned();
    assert_eq!(rejection["issuing_root"], issuing_root);
    let actions = rejection["recovery_actions"]
        .as_array()
        .expect("the refusal should carry recovery actions");
    assert_eq!(
        actions
            .iter()
            .map(|action| action["kind"].as_str().expect("action kind should be text"))
            .collect::<Vec<_>>(),
        ["release_incumbent_reservation"]
    );
    for action in actions {
        assert_eq!(
            action["argv"],
            serde_json::json!([
                "cargo-berth",
                "release",
                occupancy.incumbent_reservation_id,
                "--json"
            ])
        );
        assert_eq!(action["cwd"], issuing_root);
    }
    let message = envelope["message"]
        .as_str()
        .expect("the refusal should carry a message");
    assert!(
        message.contains(occupancy.incumbent_reservation_id),
        "the refusal should name the incumbent reservation in its message: {message:?}"
    );
    assert!(
        message.contains("separate checkout"),
        "the refusal should name the remedy for an incumbent that is still working: {message:?}"
    );
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

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
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

mod merge_extent {
    #![allow(
        clippy::expect_used,
        reason = "integration fixtures should stop on invalid git histories or command output"
    )]

    //! A claim protects the net branch merge and the live run's editing scope independently.

    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;
    use std::process::Output;

    use cargo_berth_test_support::GitDriver;
    use cargo_berth_test_support::OptionalLocks;
    use serde_json::Value;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::RETENTION_REF_PREFIX;
    use super::SESSION_MAPPING_PATH;

    /// The built binary is also the only berth executable git hooks may invoke.
    const BERTH: &str = env!("CARGO_BIN_EXE_cargo-berth");
    /// Fixture commits avoid ambient coordination and machine maintenance settings.
    const GIT: GitDriver = GitDriver {
        executable:          BERTH,
        optional_locks:      OptionalLocks::Refused,
        cleared_environment: &[
            "CARGO_BERTH_RUN",
            "CARGO_BERTH_SESSION_ID",
            "CARGO_BERTH_BYPASS",
        ],
    };
    /// Distinct runs allow a shared checkout race to be tested without a second account.
    const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
    /// A later run in the holder's checkout.
    const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
    /// The independent branch making edit requests.
    const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
    /// Journal truth lives in the main checkout's common git directory.
    const JOURNAL: &str = ".git/cargo-berth/journal.ndjson";
    /// Removing only this cache forces ordinary journal replay on the next command.
    const PROJECTION: &str = ".git/cargo-berth/reservations.json";

    #[test]
    fn a_clean_claim_on_trunk_protects_no_paths_in_another_worktree() {
        let fixture = Repository::new();
        let id = claim(fixture.trunk(), "tree:crates", FIRST_RUN);
        let observed = board(fixture.trunk());
        let reservation = snapshot(&observed, &id);
        assert_eq!(reservation["merge_extent"]["status"], "empty");
        assert_eq!(reservation["race_extent"]["status"], "editing");

        assert_allowed(
            &fixture.outsider,
            "file:crates/cargo-tile/src/lib.rs",
            THIRD_RUN,
        );
        assert_refused(
            fixture.trunk(),
            "file:crates/cargo-tile/src/lib.rs",
            SECOND_RUN,
            &id,
        );
    }

    #[test]
    fn an_unmerged_committed_path_is_covered_even_outside_the_declared_race_scope() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        commit(&fixture.holder, "branch.rs", "branch work\n");
        let observed = board(fixture.trunk());
        assert_eq!(
            scope_paths(&snapshot(&observed, &id)["merge_extent"]["scopes"]),
            BTreeSet::from(["branch.rs".to_owned()])
        );
        assert_eq!(
            scope_paths(&snapshot(&observed, &id)["race_extent"]["scopes"]),
            BTreeSet::from(["declared.rs".to_owned()])
        );

        assert_refused(&fixture.outsider, "file:branch.rs", THIRD_RUN, &id);
        assert_allowed(&fixture.outsider, "file:declared.rs", THIRD_RUN);
    }

    #[test]
    fn advancing_only_trunk_releases_an_unchanged_clean_holder_on_the_next_read() {
        for locked in [false, true] {
            let fixture = Repository::new();
            let id = claim(&fixture.holder, "file:branch.rs", FIRST_RUN);
            commit(&fixture.holder, "branch.rs", "branch work\n");
            if locked {
                GIT.run(
                    fixture.trunk(),
                    [
                        "worktree",
                        "lock",
                        fixture
                            .holder
                            .to_str()
                            .expect("holder path should be UTF-8"),
                    ],
                );
            }
            let protected = board(fixture.trunk());
            assert_eq!(
                snapshot(&protected, &id)["merge_extent"]["status"],
                "protected"
            );
            assert_refused(&fixture.outsider, "file:branch.rs", THIRD_RUN, &id);
            let head_before = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]);
            let status_before = GIT.stdout(&fixture.holder, ["status", "--porcelain"]);

            GIT.run(fixture.trunk(), ["merge", "--quiet", "--ff-only", "holder"]);
            assert_eq!(
                GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]),
                head_before
            );
            assert_eq!(
                GIT.stdout(&fixture.holder, ["status", "--porcelain"]),
                status_before
            );
            assert!(status_before.is_empty());
            let integrated = board(fixture.trunk());
            assert_eq!(
                snapshot(&integrated, &id)["merge_extent"]["status"],
                "empty"
            );
            assert!(
                integrated["payload"]["alerts"]
                    .as_array()
                    .is_some_and(Vec::is_empty),
                "an accessible holder must remain observable when locked={locked}: {integrated}"
            );

            assert_allowed(&fixture.outsider, "file:branch.rs", THIRD_RUN);
            assert_refused(&fixture.holder, "file:branch.rs", SECOND_RUN, &id);
        }
    }

    #[test]
    fn an_integrated_holder_behind_trunk_does_not_cover_trunk_only_changes() {
        let fixture = Repository::new();
        claim(&fixture.holder, "tree:crates", FIRST_RUN);
        commit(&fixture.holder, "crates/one/lib.rs", "integrated work\n");
        GIT.run(fixture.trunk(), ["merge", "--quiet", "--ff-only", "holder"]);
        commit(fixture.trunk(), "crates/two/lib.rs", "later trunk work\n");
        board(fixture.trunk());

        assert_allowed(&fixture.outsider, "file:crates/one/lib.rs", THIRD_RUN);
        assert_allowed(&fixture.outsider, "file:crates/two/lib.rs", THIRD_RUN);
    }

    #[test]
    fn a_diverged_holder_never_covers_changes_made_only_on_trunk() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "tree:crates", FIRST_RUN);
        commit(&fixture.holder, "crates/one/lib.rs", "branch work\n");
        commit(fixture.trunk(), "crates/two/lib.rs", "trunk work\n");
        board(fixture.trunk());

        assert_refused(&fixture.outsider, "file:crates/one/lib.rs", THIRD_RUN, &id);
        assert_allowed(&fixture.outsider, "file:crates/two/lib.rs", THIRD_RUN);
    }

    #[test]
    fn a_change_reverted_on_the_branch_is_outside_the_merge_extent() {
        let fixture = Repository::new();
        claim(&fixture.holder, "file:tracked.rs", FIRST_RUN);
        commit(&fixture.holder, "tracked.rs", "temporary change\n");
        commit(&fixture.holder, "tracked.rs", "base\n");
        board(fixture.trunk());

        assert_allowed(&fixture.outsider, "file:tracked.rs", THIRD_RUN);
    }

    #[test]
    fn a_run_without_commits_covers_staged_unstaged_and_untracked_paths() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        write(&fixture.holder, "staged.rs", "staged work\n");
        GIT.run(&fixture.holder, ["add", "staged.rs"]);
        write(&fixture.holder, "tracked.rs", "unstaged work\n");
        write(&fixture.holder, "untracked.rs", "untracked work\n");
        assert_eq!(
            GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]),
            GIT.stdout(fixture.trunk(), ["rev-parse", "HEAD"])
        );
        board(fixture.trunk());

        for path in ["staged.rs", "tracked.rs", "untracked.rs"] {
            assert_refused(&fixture.outsider, &format!("file:{path}"), THIRD_RUN, &id);
        }
    }

    #[test]
    fn ending_a_run_allows_its_checkout_but_still_refuses_another_branch() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:tracked.rs", FIRST_RUN);
        commit(&fixture.holder, "tracked.rs", "unmerged work\n");
        assert_refused(&fixture.holder, "file:tracked.rs", SECOND_RUN, &id);
        succeed(&berth(
            &fixture.holder,
            &["release", &id, "--json"],
            FIRST_RUN,
        ));
        let observed = board(fixture.trunk());
        assert_eq!(snapshot(&observed, &id)["race_extent"]["status"], "ended");
        assert_eq!(
            snapshot(&observed, &id)["merge_extent"]["status"],
            "protected"
        );

        assert_allowed(&fixture.holder, "file:tracked.rs", SECOND_RUN);
        assert_refused(&fixture.outsider, "file:tracked.rs", THIRD_RUN, &id);
    }

    #[test]
    fn repeated_drift_cannot_leave_other_crates_in_the_merge_extent() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "tree:crates/cargo-berth", FIRST_RUN);
        commit(
            &fixture.holder,
            "crates/cargo-berth/src/lib.rs",
            "branch work\n",
        );

        for index in 0..3 {
            let path = format!("crates/cargo-tile/src/transient-{index}.rs");
            write(&fixture.holder, &path, "temporary work\n");
            succeed(&berth(
                &fixture.holder,
                &["drift", "--full", "--reservation", &id, "--json"],
                FIRST_RUN,
            ));
            fs::remove_file(fixture.holder.join(&path))
                .expect("temporary dirty path should remove");
            let observed = board(fixture.trunk());
            assert!(
                scope_paths(&snapshot(&observed, &id)["race_extent"]["scopes"]).contains(&path)
            );
            assert_eq!(
                scope_paths(&snapshot(&observed, &id)["merge_extent"]["scopes"]),
                BTreeSet::from(["crates/cargo-berth/src/lib.rs".to_owned()])
            );
            assert_allowed(&fixture.outsider, &format!("file:{path}"), THIRD_RUN);
        }

        assert_refused(
            &fixture.outsider,
            "file:crates/cargo-berth/src/lib.rs",
            THIRD_RUN,
            &id,
        );
    }

    #[test]
    fn first_touch_widening_does_not_protect_an_unmodified_path_from_other_branches() {
        let fixture = Repository::new();
        let first = berth(
            &fixture.holder,
            &["check", "file:first.rs", "--json"],
            FIRST_RUN,
        );
        succeed(&first);
        let id = json(&first)["reservations"][0]
            .as_str()
            .expect("first touch should name its reservation")
            .to_owned();
        let widened = berth(
            &fixture.holder,
            &["check", "file:second.rs", "--json"],
            FIRST_RUN,
        );
        succeed(&widened);
        assert_eq!(
            json(&widened)["payload"]["data"]["acquisition"]["kind"],
            "widened"
        );
        board(fixture.trunk());

        assert_refused(&fixture.holder, "file:second.rs", SECOND_RUN, &id);
        assert_allowed(&fixture.outsider, "file:second.rs", THIRD_RUN);
    }

    #[test]
    fn repeated_board_reads_do_not_journal_the_same_extent_again() {
        let fixture = Repository::new();
        claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        commit(&fixture.holder, "branch.rs", "unmerged work\n");
        let first = board(fixture.trunk());
        let journal = fs::read(fixture.trunk().join(JOURNAL)).expect("journal should read");
        assert_eq!(
            events(fixture.trunk())
                .iter()
                .filter(|event| event["op"] == "merge_extent_observed"
                    && event["extent"]["status"] == "protected")
                .count(),
            1
        );

        for _ in 0..3 {
            let repeated = board(fixture.trunk());
            assert_eq!(
                repeated["payload"]["data"]["journal_position"],
                first["payload"]["data"]["journal_position"]
            );
            assert_eq!(
                fs::read(fixture.trunk().join(JOURNAL)).expect("journal should read"),
                journal
            );
        }
    }

    #[test]
    fn losing_the_holder_keeps_the_last_observed_unmerged_paths_protected() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        commit(&fixture.holder, "branch.rs", "unmerged work\n");
        board(fixture.trunk());
        fs::rename(
            &fixture.holder,
            fixture.worktrees.path().join("unavailable-holder"),
        )
        .expect("holder checkout should move without updating git metadata");

        let observed = board(fixture.trunk());
        let unavailable = &snapshot(&observed, &id)["merge_extent"];
        assert_eq!(unavailable["status"], "unavailable");
        assert_eq!(unavailable["retained_evidence"]["status"], "protected");
        assert_eq!(
            scope_paths(&unavailable["retained_evidence"]["scopes"]),
            BTreeSet::from(["branch.rs".to_owned()])
        );
        assert!(
            !unavailable["failure"]
                .as_str()
                .expect("unavailable extent should explain its failure")
                .is_empty()
        );
        assert_refused(&fixture.outsider, "file:branch.rs", THIRD_RUN, &id);
    }

    #[test]
    fn journal_replay_restores_first_touch_race_protection() {
        let fixture = Repository::new();
        let first = berth(
            &fixture.holder,
            &["check", "file:first.rs", "--json"],
            FIRST_RUN,
        );
        succeed(&first);
        let id = json(&first)["reservations"][0]
            .as_str()
            .expect("first touch should name its reservation")
            .to_owned();
        succeed(&berth(
            &fixture.holder,
            &["check", "file:second.rs", "--json"],
            FIRST_RUN,
        ));
        fs::remove_file(fixture.trunk().join(PROJECTION)).expect("projection should remove");

        assert_refused(&fixture.holder, "file:second.rs", SECOND_RUN, &id);
        assert_allowed(&fixture.outsider, "file:second.rs", THIRD_RUN);
    }

    #[test]
    fn another_runs_commit_enters_the_branch_extent_after_the_first_run_ends() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:first.rs", FIRST_RUN);
        commit(&fixture.holder, "first.rs", "first run\n");
        let first_tip = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]);
        succeed(&berth(
            &fixture.holder,
            &["release", &id, "--json"],
            FIRST_RUN,
        ));
        // Run B commits without claiming or checking a path, so A is the only holder.
        // Its merge protection follows the branch even after trunk integrates A's tip.
        commit(&fixture.holder, "second.rs", "second run\n");
        GIT.run(
            fixture.trunk(),
            ["merge", "--quiet", "--ff-only", &first_tip],
        );
        let observed = board(fixture.trunk());
        let reservation = snapshot(&observed, &id);
        assert_eq!(
            events(fixture.trunk())
                .iter()
                .filter(|event| event["op"] == "claim")
                .count(),
            1,
            "B must have no reservation that could independently protect its commit"
        );
        assert_eq!(
            reservation["integration_evidence"]["status"]["status"],
            "integrated"
        );
        assert_eq!(
            scope_paths(&reservation["merge_extent"]["scopes"]),
            BTreeSet::from(["second.rs".to_owned()])
        );
        assert_eq!(reservation["race_extent"]["status"], "ended");
        assert!(!scope_paths(&reservation["scopes"]).contains("second.rs"));

        assert_refused(&fixture.outsider, "file:second.rs", THIRD_RUN, &id);

        write(&fixture.holder, "dirty.rs", "later uncommitted work\n");
        assert_refused(&fixture.outsider, "file:dirty.rs", THIRD_RUN, &id);
        let dirty_board = board(fixture.trunk());
        let dirty_reservation = snapshot(&dirty_board, &id);
        assert_eq!(
            scope_paths(&dirty_reservation["merge_extent"]["scopes"]),
            BTreeSet::from(["dirty.rs".to_owned(), "second.rs".to_owned()])
        );
        assert_eq!(dirty_reservation["race_extent"]["status"], "ended");
        assert_eq!(dirty_reservation["scopes"], reservation["scopes"]);
        assert_allowed(&fixture.holder, "file:first.rs", SECOND_RUN);
        assert_allowed(&fixture.outsider, "file:first.rs", THIRD_RUN);
    }

    #[test]
    fn a_clean_live_claim_keeps_its_session_mapping_until_the_run_ends() {
        let fixture = Repository::new();
        let session = "live-empty-merge-extent";
        let claimed = Command::new(BERTH)
            .args(["claim", "file:tracked.rs", "--run", FIRST_RUN, "--json"])
            .current_dir(&fixture.holder)
            .env("CARGO_BERTH_SESSION_ID", session)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("session claim should run");
        succeed(&claimed);
        let id = json(&claimed)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("session claim should name its reservation")
            .to_owned();
        board(fixture.trunk());
        let mapping_path = fixture
            .trunk()
            .join(".git/cargo-berth/session-identities.json");
        let mappings: Value =
            serde_json::from_slice(&fs::read(&mapping_path).expect("session mappings should read"))
                .expect("session mappings should be JSON");
        assert_eq!(mappings["identities"][session]["reservation_id"], id);
        assert_refused(&fixture.holder, "file:tracked.rs", SECOND_RUN, &id);
        assert_allowed(&fixture.outsider, "file:tracked.rs", THIRD_RUN);

        succeed(&berth(
            &fixture.holder,
            &["release", &id, "--json"],
            FIRST_RUN,
        ));
        board(fixture.trunk());
        let retired: Value =
            serde_json::from_slice(&fs::read(mapping_path).expect("retired mappings should read"))
                .expect("retired mappings should be JSON");
        assert!(
            retired["identities"].get(session).is_none(),
            "ended empty claim should retire its mapping: {retired}"
        );
    }

    #[test]
    fn a_refused_drift_widen_remains_visible_to_the_next_cheap_comparison() {
        let fixture = Repository::new();
        let subject = claim(&fixture.holder, "file:own.rs", FIRST_RUN);
        let clean = berth(
            &fixture.holder,
            &["drift", "--full", "--reservation", &subject, "--json"],
            FIRST_RUN,
        );
        succeed(&clean);
        let fingerprint = fs::read_dir(fixture.trunk().join(".git/cargo-berth"))
            .expect("ledger directory should read")
            .map(|entry| entry.expect("ledger entry should read").path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("drift-fingerprint-"))
            })
            .expect("full clean drift should publish a fingerprint");
        let published: Value = serde_json::from_slice(
            &fs::read(&fingerprint).expect("published fingerprint should read"),
        )
        .expect("published fingerprint should be JSON");
        assert_eq!(
            published,
            serde_json::json!({"tracked_paths": [], "untracked_paths": []})
        );
        let cached = berth(
            &fixture.holder,
            &["drift", "--reservation", &subject, "--json"],
            FIRST_RUN,
        );
        succeed(&cached);
        assert_eq!(
            json(&cached)["payload"]["data"]["comparison"],
            "cheap_delta"
        );
        assert_eq!(
            json(&cached)["payload"]["data"]["results"][0]["status"],
            "unchanged"
        );
        let foreign = claim(&fixture.outsider, "file:contested.rs", THIRD_RUN);
        write(
            &fixture.outsider,
            "contested.rs",
            "foreign uncommitted work\n",
        );
        board(fixture.trunk());
        commit(&fixture.holder, "contested.rs", "incursion\n");
        assert!(
            GIT.stdout(&fixture.holder, ["status", "--porcelain"])
                .is_empty()
        );

        for arguments in [
            vec!["drift", "--full", "--reservation", &subject, "--json"],
            vec!["drift", "--reservation", &subject, "--json"],
        ] {
            let refused = berth(&fixture.holder, &arguments, FIRST_RUN);
            let envelope = json(&refused);
            assert_eq!(
                refused.status.code(),
                Some(1),
                "a refused widen must remain visible: {envelope}"
            );
            assert_eq!(envelope["status"], "incursion");
            assert_eq!(envelope["blocked_by"], serde_json::json!([foreign]));
            assert!(envelope.to_string().contains("contested.rs"));
            assert!(
                !fingerprint.exists(),
                "a refused widening must invalidate the cache"
            );
        }
        let widened_contested_path = events(fixture.trunk()).iter().any(|event| {
            event["op"] == "widen"
                && event["reservation_id"] == subject
                && event["added_scopes"].to_string().contains("contested.rs")
        });
        assert!(
            !widened_contested_path,
            "a refused drift must not widen the race extent"
        );
    }

    #[test]
    fn a_failed_first_derivation_keeps_the_initial_declaration_protected() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        assert!(
            !events(fixture.trunk()).iter().any(
                |event| event["reservation_id"] == id && event["op"] == "merge_extent_observed"
            ),
            "the fixture must fail before its first derived answer"
        );
        fs::rename(
            &fixture.holder,
            fixture.worktrees.path().join("unavailable-holder"),
        )
        .expect("holder should become unavailable");

        let observed = board(fixture.trunk());
        let extent = &snapshot(&observed, &id)["merge_extent"];
        assert_eq!(extent["status"], "unavailable");
        assert_eq!(extent["retained_evidence"]["status"], "not_derived");
        assert_eq!(
            scope_paths(&extent["retained_evidence"]["protection"]),
            BTreeSet::from(["declared.rs".to_owned()])
        );
        assert_refused(&fixture.outsider, "file:declared.rs", THIRD_RUN, &id);
    }

    #[test]
    fn a_legacy_journal_without_derived_evidence_replays_to_initial_protection() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        let mut legacy_claim = events(fixture.trunk())
            .into_iter()
            .find(|event| event["op"] == "claim")
            .expect("fixture should record a claim");
        legacy_claim["projection_generation"] = serde_json::json!(1);
        fs::write(fixture.trunk().join(JOURNAL), format!("{legacy_claim}\n"))
            .expect("legacy journal should write");
        fs::remove_file(fixture.trunk().join(PROJECTION)).expect("legacy projection should remove");
        fs::rename(
            &fixture.holder,
            fixture.worktrees.path().join("unavailable-holder"),
        )
        .expect("legacy holder should become unavailable");

        let observed = board(fixture.trunk());
        let extent = &snapshot(&observed, &id)["merge_extent"];
        assert_eq!(extent["status"], "unavailable");
        assert_eq!(extent["retained_evidence"]["status"], "not_derived");
        assert_refused(&fixture.outsider, "file:declared.rs", THIRD_RUN, &id);
    }

    #[test]
    fn drift_widening_changes_only_the_race_extent_when_the_dirty_surface_is_unchanged() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        write(&fixture.holder, "first-touched.rs", "dirty work\n");
        let before = board(fixture.trunk());
        let before_extent = snapshot(&before, &id)["merge_extent"].clone();
        let widened = berth(
            &fixture.holder,
            &["drift", "--full", "--reservation", &id, "--json"],
            FIRST_RUN,
        );
        succeed(&widened);
        assert_eq!(json(&widened)["status"], "widened");
        let after = board(fixture.trunk());

        assert_eq!(snapshot(&after, &id)["merge_extent"], before_extent);
        assert!(
            scope_paths(&snapshot(&after, &id)["race_extent"]["scopes"])
                .contains("first-touched.rs")
        );
        assert_eq!(
            events(fixture.trunk())
                .iter()
                .filter(
                    |event| event["op"] == "merge_extent_observed" && event["reservation_id"] == id
                )
                .count(),
            1
        );
    }

    #[test]
    fn repairing_projection_replays_race_widening_without_observing_git_state() {
        let fixture = Repository::new();
        succeed(&berth(
            &fixture.holder,
            &["check", "file:first.rs", "--json"],
            FIRST_RUN,
        ));
        succeed(&berth(
            &fixture.holder,
            &["check", "file:second.rs", "--json"],
            FIRST_RUN,
        ));
        let original = fs::read(fixture.trunk().join(PROJECTION)).expect("projection should read");
        let journal = fs::read(fixture.trunk().join(JOURNAL)).expect("journal should read");
        fs::remove_file(fixture.trunk().join(PROJECTION)).expect("projection should remove");
        let wrapper = tempdir().expect("git wrapper directory should exist");
        let trace = wrapper.path().join("unexpected-git");
        let executable = wrapper.path().join("git");
        fs::write(
        &executable,
        "#!/bin/sh\ncase \"$*\" in *rev-parse*) exec \"$CARGO_BERTH_TEST_REAL_GIT\" \"$@\" ;; esac\nprintf '%s\\n' \"$*\" >> \"$CARGO_BERTH_TEST_UNEXPECTED_GIT\"\nexit 97\n",
    )
    .expect("failing git wrapper should write");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("git wrapper should execute");
        let repaired = Command::new(BERTH)
            .args(["init", "--repair-projection", "--json"])
            .current_dir(fixture.trunk())
            .env("PATH", wrapper.path())
            .env("CARGO_BERTH_TEST_UNEXPECTED_GIT", &trace)
            .env("CARGO_BERTH_TEST_REAL_GIT", real_git())
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("projection repair should run");

        succeed(&repaired);
        assert!(
            !trace.exists(),
            "journal-only repair must not rederive git state"
        );
        assert_eq!(
            fs::read(fixture.trunk().join(PROJECTION)).expect("rebuilt projection should read"),
            original
        );
        assert_eq!(
            fs::read(fixture.trunk().join(JOURNAL)).expect("replayed journal should read"),
            journal
        );
        let observed = board(fixture.trunk());
        let id = events(fixture.trunk())
            .into_iter()
            .find(|event| event["op"] == "claim")
            .expect("replayed claim should exist")["reservation_id"]
            .as_str()
            .expect("claim should name id")
            .to_owned();
        assert_eq!(
            scope_paths(&snapshot(&observed, &id)["race_extent"]["scopes"]),
            BTreeSet::from(["first.rs".to_owned(), "second.rs".to_owned()])
        );
    }

    #[test]
    fn an_empty_merge_extent_preserves_the_ref_an_unfinished_successor_depends_on() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:tracked.rs", FIRST_RUN);
        commit(&fixture.holder, "tracked.rs", "predecessor work\n");
        let tip = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]);
        let proposal = berth(
            &fixture.outsider,
            &[
                "claim",
                "file:tracked.rs",
                "--run",
                THIRD_RUN,
                "--after",
                &id,
                "--overlap-why",
                "successor edits the same source",
                "--why",
                "successor depends on predecessor",
                "--json",
            ],
            THIRD_RUN,
        );
        assert_eq!(
            proposal.status.code(),
            Some(3),
            "overlap should request its concrete proposal: {}",
            json(&proposal)
        );
        let token = json(&proposal)["payload"]["data"]["proposal_token"]
            .as_str()
            .expect("proposal should name token")
            .to_owned();
        let successor = berth(
            &fixture.outsider,
            &[
                "claim",
                "file:tracked.rs",
                "--run",
                THIRD_RUN,
                "--after",
                &id,
                "--overlap-why",
                "successor edits the same source",
                "--why",
                "successor depends on predecessor",
                "--proposal",
                &token,
                "--json",
            ],
            THIRD_RUN,
        );
        succeed(&successor);
        assert_allowed(&fixture.outsider, "file:tracked.rs", THIRD_RUN);
        succeed(&berth(
            &fixture.holder,
            &["release", &id, "--json"],
            FIRST_RUN,
        ));
        GIT.run(fixture.trunk(), ["merge", "--quiet", "--ff-only", "holder"]);
        let observed = board(fixture.trunk());

        assert_eq!(snapshot(&observed, &id)["merge_extent"]["status"], "empty");
        assert_eq!(
            GIT.stdout(
                fixture.trunk(),
                ["rev-parse", &format!("refs/cargo-berth/reservations/{id}")]
            ),
            tip
        );
    }

    #[test]
    fn an_incursion_gets_a_durable_disposition_when_its_subject_merge_extent_empties() {
        let fixture = Repository::new();
        let subject = claim(&fixture.holder, "file:own.rs", FIRST_RUN);
        claim(&fixture.outsider, "file:contested.rs", THIRD_RUN);
        write(&fixture.outsider, "contested.rs", "foreign work\n");
        write(&fixture.holder, "contested.rs", "incursion\n");
        let refused = berth(
            &fixture.holder,
            &["drift", "--full", "--reservation", &subject, "--json"],
            FIRST_RUN,
        );
        assert_eq!(json(&refused)["status"], "incursion");
        let incident = events(fixture.trunk())
            .into_iter()
            .find(|event| event["op"] == "incursion")
            .expect("incursion should be durable")["incident_id"]
            .as_str()
            .expect("incursion should name incident")
            .to_owned();
        commit(&fixture.holder, "contested.rs", "incursion\n");
        GIT.run(fixture.trunk(), ["merge", "--quiet", "--ff-only", "holder"]);
        let observed = board(fixture.trunk());

        assert_eq!(
            snapshot(&observed, &subject)["merge_extent"]["status"],
            "empty"
        );
        assert!(
        events(fixture.trunk())
            .iter()
            .any(|event| event["op"] == "resolve_incursion" && event["incident_id"] == incident),
        "the original incident must receive a separate durable disposition"
    );
        assert!(
            !observed["payload"]["data"]["outstanding_incursions"]
                .to_string()
                .contains(&incident)
        );
    }

    #[test]
    fn an_edit_check_refreshes_an_empty_foreign_extent_before_allowing_the_path() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:declared.rs", FIRST_RUN);
        let empty = board(fixture.trunk());
        assert_eq!(snapshot(&empty, &id)["merge_extent"]["status"], "empty");
        write(
            &fixture.holder,
            "fresh.rs",
            "uncommitted work after the board read\n",
        );

        assert_refused(&fixture.outsider, "file:fresh.rs", THIRD_RUN, &id);
        fs::remove_file(fixture.holder.join("fresh.rs")).expect("restored path should remove");
        assert_allowed(&fixture.outsider, "file:fresh.rs", THIRD_RUN);
    }

    #[test]
    fn reservations_sharing_a_checkout_share_derivation_and_reuse_its_unchanged_key() {
        let fixture = Repository::new();
        for path in ["first.rs", "second.rs", "third.rs"] {
            claim(&fixture.holder, &format!("file:{path}"), FIRST_RUN);
        }
        write(&fixture.holder, "tracked.rs", "dirty branch work\n");
        let wrapper = tempdir().expect("git tracing directory should exist");
        let trace = wrapper.path().join("git-trace");
        let executable = wrapper.path().join("git");
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CARGO_BERTH_TEST_GIT_TRACE\"\nexec \"$CARGO_BERTH_TEST_REAL_GIT\" \"$@\"\n").expect("tracing wrapper should write");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755))
            .expect("tracing wrapper should execute");
        let original_path = std::env::var_os("PATH").expect("test PATH should exist");
        let path = std::env::join_paths(
            std::iter::once(wrapper.path().to_owned()).chain(std::env::split_paths(&original_path)),
        )
        .expect("tracing PATH should join");

        for expected_merge_queries in [1, 0] {
            fs::write(&trace, "").expect("trace should reset");
            let output = Command::new(BERTH)
                .args(["board", "--json"])
                .current_dir(fixture.trunk())
                .env("PATH", &path)
                .env("CARGO_BERTH_TEST_GIT_TRACE", &trace)
                .env("CARGO_BERTH_TEST_REAL_GIT", real_git())
                .env_remove("CARGO_BERTH_RUN")
                .env_remove("CARGO_BERTH_SESSION_ID")
                .env_remove("CARGO_BERTH_BYPASS")
                .output()
                .expect("traced board should run");
            succeed(&output);
            let queries = fs::read_to_string(&trace).expect("git trace should read");
            assert_eq!(
                queries
                    .lines()
                    .filter(|line| line.split_whitespace().any(|arg| arg == "status"))
                    .count(),
                1,
                "all reservations share one dirty observation: {queries}"
            );
            assert_eq!(
                queries
                    .lines()
                    .filter(|line| line.contains("--merge-base") && line.contains("--name-only"))
                    .count(),
                expected_merge_queries,
                "only changed keys repeat the merge query: {queries}"
            );
        }
    }

    #[test]
    fn a_retired_session_can_edit_its_predecessors_checkout_before_first_touch() {
        let fixture = Repository::new();
        let session = "retired-predecessor-session";
        let claimed = Command::new(BERTH)
            .args(["claim", "file:branch.rs", "--run", FIRST_RUN, "--json"])
            .current_dir(&fixture.holder)
            .env("CARGO_BERTH_SESSION_ID", session)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("mapped predecessor claim should run");
        succeed(&claimed);
        let id = json(&claimed)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("predecessor claim should name its reservation")
            .to_owned();
        let mapping_path = fixture.trunk().join(SESSION_MAPPING_PATH);
        let live_mapping: Value =
            serde_json::from_slice(&fs::read(&mapping_path).expect("live mapping should read"))
                .expect("live mapping should be JSON");
        assert_eq!(live_mapping["identities"][session]["reservation_id"], id);
        commit(&fixture.holder, "branch.rs", "predecessor work\n");
        let released = berth(&fixture.holder, &["release", &id, "--json"], FIRST_RUN);
        succeed(&released);
        assert_eq!(json(&released)["status"], "outstanding");
        let retired_mapping: Value =
            serde_json::from_slice(&fs::read(&mapping_path).expect("retired mappings should read"))
                .expect("retired mappings should be JSON");
        assert!(retired_mapping["identities"].get(session).is_none());

        let checked = Command::new(BERTH)
            .args(["check", "file:branch.rs", "--json"])
            .current_dir(&fixture.holder)
            .env("CARGO_BERTH_SESSION_ID", session)
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("unidentified edit check should run");
        succeed(&checked);
        assert_eq!(json(&checked)["status"], "clear");
        assert_refused(&fixture.outsider, "file:branch.rs", THIRD_RUN, &id);
    }

    #[test]
    fn hook_git_environment_cannot_change_another_checkouts_merge_extent() {
        let fixture = Repository::new();
        let holder = claim(&fixture.holder, "file:branch.rs", FIRST_RUN);
        let outsider = claim(&fixture.outsider, "file:outsider.rs", THIRD_RUN);
        commit(&fixture.holder, "branch.rs", "holder branch work\n");
        write(&fixture.holder, "staged.rs", "holder index work\n");
        GIT.run(&fixture.holder, ["add", "staged.rs"]);
        let clean = board(&fixture.outsider);
        assert_eq!(
            snapshot(&clean, &outsider)["merge_extent"]["status"],
            "empty"
        );
        assert_eq!(
            scope_paths(&snapshot(&clean, &holder)["merge_extent"]["scopes"]),
            BTreeSet::from(["branch.rs".to_owned(), "staged.rs".to_owned()])
        );
        let git_directory = GIT.stdout(&fixture.holder, ["rev-parse", "--absolute-git-dir"]);
        let contaminated = Command::new(BERTH)
            .args(["board", "--json"])
            .current_dir(&fixture.outsider)
            .env("GIT_DIR", &git_directory)
            .env("GIT_WORK_TREE", &fixture.holder)
            .env("GIT_INDEX_FILE", Path::new(&git_directory).join("index"))
            .env("GIT_COMMON_DIR", fixture.trunk().join(".git"))
            .env("GIT_PREFIX", "hook-prefix/")
            .env_remove("CARGO_BERTH_RUN")
            .env_remove("CARGO_BERTH_SESSION_ID")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("board should run with inherited hook variables");
        succeed(&contaminated);
        let contaminated = json(&contaminated);
        for id in [&holder, &outsider] {
            assert_eq!(
                snapshot(&contaminated, id)["merge_extent"],
                snapshot(&clean, id)["merge_extent"],
                "hook repository variables must not change checkout observations"
            );
        }
        assert_eq!(
            contaminated["payload"]["data"]["alerts"]["entries"],
            serde_json::json!([])
        );
    }

    #[test]
    fn two_outstanding_reservations_have_one_answerable_foreign_merge_conflict() {
        let fixture = Repository::new();
        let oldest = claim(&fixture.holder, "file:first.rs", FIRST_RUN);
        let newer = claim(&fixture.holder, "file:second.rs", FIRST_RUN);
        commit(&fixture.holder, "first.rs", "first checkpoint\n");
        commit(&fixture.holder, "second.rs", "second checkpoint\n");
        for id in [&oldest, &newer] {
            let released = berth(&fixture.holder, &["release", id, "--json"], FIRST_RUN);
            succeed(&released);
            assert_eq!(json(&released)["status"], "outstanding");
        }
        let refused = berth(
            &fixture.outsider,
            &["claim", "file:second.rs", "--run", THIRD_RUN, "--json"],
            THIRD_RUN,
        );
        assert_eq!(refused.status.code(), Some(1));
        let refused = json(&refused);
        let conflicts = refused["payload"]["data"]["conflicts"]
            .as_array()
            .expect("refused claim should list its conflicts");
        assert_eq!(
            conflicts.len(),
            1,
            "one checkout has one merge conflict: {refused}"
        );
        assert_eq!(conflicts[0]["reservation_id"], oldest);

        let mut arguments = vec![
            "claim",
            "file:second.rs",
            "--run",
            THIRD_RUN,
            "--override",
            &oldest,
            "--overlap-why",
            "reviewed shared branch work",
            "--json",
        ];
        let proposed = berth(&fixture.outsider, &arguments, THIRD_RUN);
        assert_eq!(proposed.status.code(), Some(3), "{}", json(&proposed));
        let proposed = json(&proposed);
        let proposal = proposed["payload"]["data"]["proposal_token"]
            .as_str()
            .expect("override should publish an answerable proposal");
        arguments.extend(["--proposal", proposal]);
        let applied = berth(&fixture.outsider, &arguments, THIRD_RUN);
        succeed(&applied);
        assert_allowed(&fixture.outsider, "file:second.rs", THIRD_RUN);
        write(&fixture.outsider, "second.rs", "answered edit\n");
        let drift = berth(&fixture.outsider, &["drift", "--full", "--json"], THIRD_RUN);
        succeed(&drift);
        assert_eq!(json(&drift)["blocked_by"], serde_json::json!([]));
    }

    #[test]
    fn divergent_unavailable_holders_have_one_answerable_foreign_merge_conflict() {
        let fixture = Repository::new();
        let [oldest, _newer] = fixture.retain_divergent_protection();
        let arguments = ["claim", "file:b.rs", "--run", THIRD_RUN, "--json"];
        let refused = berth(&fixture.outsider, &arguments, THIRD_RUN);
        assert_eq!(refused.status.code(), Some(1), "{}", json(&refused));
        let refused = json(&refused);
        let conflicts = refused["payload"]["data"]["conflicts"]
            .as_array()
            .expect("refused claim should list its conflicts");
        assert_eq!(conflicts.len(), 1, "{refused}");
        assert_eq!(conflicts[0]["reservation_id"], oldest);
        assert_eq!(
            scope_paths(&conflicts[0]["overlap_scope_revision"]),
            BTreeSet::from(["a.rs".to_owned(), "b.rs".to_owned()])
        );
        assert_eq!(
            scope_paths(&conflicts[0]["overlapping_scopes"]),
            BTreeSet::from(["b.rs".to_owned()])
        );

        let mut arguments = vec![
            "claim",
            "file:b.rs",
            "--run",
            THIRD_RUN,
            "--override",
            &oldest,
            "--overlap-why",
            "reviewed retained branch protection",
            "--json",
        ];
        let proposed = berth(&fixture.outsider, &arguments, THIRD_RUN);
        assert_eq!(proposed.status.code(), Some(3), "{}", json(&proposed));
        let proposed = json(&proposed);
        let proposal = proposed["payload"]["data"]["proposal_token"]
            .as_str()
            .expect("override should publish an answerable proposal");
        fs::remove_file(fixture.trunk().join(PROJECTION))
            .expect("journal replay should reconstruct the same union and representative");
        arguments.extend(["--proposal", proposal]);
        succeed(&berth(&fixture.outsider, &arguments, THIRD_RUN));
        assert_allowed(&fixture.outsider, "file:b.rs", THIRD_RUN);
        write(&fixture.outsider, "b.rs", "answered edit\n");
        let drift = berth(&fixture.outsider, &["drift", "--full", "--json"], THIRD_RUN);
        succeed(&drift);
        assert_eq!(json(&drift)["blocked_by"], serde_json::json!([]));
    }

    #[test]
    fn younger_unavailable_holders_answer_authorizes_only_its_scope_after_replay() {
        let fixture = Repository::new();
        let outsider = claim(&fixture.outsider, "file:b.rs", THIRD_RUN);
        commit(&fixture.outsider, "b.rs", "outsider branch work\n");
        let oldest = claim(&fixture.holder, "file:a.rs", FIRST_RUN);
        commit(&fixture.holder, "a.rs", "holder branch work\n");
        let observed = board(fixture.trunk());
        for (id, path) in [(&outsider, "b.rs"), (&oldest, "a.rs")] {
            let extent = &snapshot(&observed, id)["merge_extent"];
            assert_eq!(extent["status"], "protected");
            assert_eq!(
                scope_paths(&extent["scopes"]),
                BTreeSet::from([path.to_owned()])
            );
        }

        let refused = berth(
            &fixture.holder,
            &["claim", "file:b.rs", "--run", FIRST_RUN, "--json"],
            FIRST_RUN,
        );
        assert_eq!(refused.status.code(), Some(1), "{}", json(&refused));
        let refused = json(&refused);
        let conflicts = refused["payload"]["data"]["conflicts"]
            .as_array()
            .expect("refused claim should list its conflicts");
        assert_eq!(conflicts.len(), 1, "{refused}");
        assert_eq!(conflicts[0]["reservation_id"], outsider);

        let mut arguments = vec![
            "claim",
            "file:b.rs",
            "--run",
            FIRST_RUN,
            "--override",
            &outsider,
            "--overlap-why",
            "reviewed shared b.rs work",
            "--json",
        ];
        let proposed = berth(&fixture.holder, &arguments, FIRST_RUN);
        assert_eq!(proposed.status.code(), Some(3), "{}", json(&proposed));
        let proposed = json(&proposed);
        let proposal = proposed["payload"]["data"]["proposal_token"]
            .as_str()
            .expect("override should publish an answerable proposal");
        arguments.extend(["--proposal", proposal]);
        let applied = berth(&fixture.holder, &arguments, FIRST_RUN);
        succeed(&applied);
        let applied = json(&applied);
        let newer = applied["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("accepted override should return the younger holder id");
        assert!(
            !events(fixture.trunk()).iter().any(|event| {
                event["reservation_id"] == newer && event["op"] == "merge_extent_observed"
            }),
            "the second holder must become unavailable before its first derivation"
        );
        fs::rename(
            &fixture.holder,
            fixture.worktrees.path().join("unavailable-holder"),
        )
        .expect("holder checkout should move without updating git metadata");

        let observed = board(fixture.trunk());
        let retained = &snapshot(&observed, &oldest)["merge_extent"];
        assert_eq!(retained["status"], "unavailable");
        assert_eq!(retained["retained_evidence"]["status"], "protected");
        assert_eq!(
            scope_paths(&retained["retained_evidence"]["scopes"]),
            BTreeSet::from(["a.rs".to_owned()])
        );
        let declared = &snapshot(&observed, newer)["merge_extent"];
        assert_eq!(declared["status"], "unavailable");
        assert_eq!(declared["retained_evidence"]["status"], "not_derived");
        assert_eq!(
            scope_paths(&declared["retained_evidence"]["protection"]),
            BTreeSet::from(["b.rs".to_owned()])
        );

        assert_allowed(&fixture.outsider, "file:b.rs", THIRD_RUN);
        let drift = berth(&fixture.outsider, &["drift", "--full", "--json"], THIRD_RUN);
        succeed(&drift);
        assert_eq!(json(&drift)["blocked_by"], serde_json::json!([]));
        let refused = berth(
            &fixture.outsider,
            &["check", "file:a.rs", "--json"],
            THIRD_RUN,
        );
        assert_eq!(refused.status.code(), Some(1), "{}", json(&refused));
        assert_eq!(json(&refused)["blocked_by"], serde_json::json!([oldest]));
        fs::remove_file(fixture.trunk().join(PROJECTION))
            .expect("journal replay should reconstruct contributor answer coverage");
        assert_allowed(&fixture.outsider, "file:b.rs", THIRD_RUN);
    }

    #[test]
    fn recovering_divergent_holders_clears_paths_outside_the_derived_merge_extent() {
        let fixture = Repository::new();
        let [oldest, newer] = fixture.retain_divergent_protection();
        assert_refused(&fixture.outsider, "file:b.rs", THIRD_RUN, &oldest);
        fs::rename(
            fixture.worktrees.path().join("unavailable-holder"),
            &fixture.holder,
        )
        .expect("holder checkout should return to its recorded path");

        let observed = board(fixture.trunk());
        for id in [&oldest, &newer] {
            let extent = &snapshot(&observed, id)["merge_extent"];
            assert_eq!(extent["status"], "protected");
            assert_eq!(
                scope_paths(&extent["scopes"]),
                BTreeSet::from(["a.rs".to_owned()])
            );
        }
        assert_refused(&fixture.outsider, "file:a.rs", THIRD_RUN, &oldest);
        claim(&fixture.outsider, "file:b.rs", THIRD_RUN);
        assert_allowed(&fixture.outsider, "file:b.rs", THIRD_RUN);
    }

    #[test]
    fn releasing_an_integrated_checkpoint_again_preserves_later_branch_work() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:checkpoint.rs", FIRST_RUN);
        commit(&fixture.holder, "checkpoint.rs", "checkpoint C\n");
        let checkpoint = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]);
        succeed(&berth(
            &fixture.holder,
            &["release", &id, "--json"],
            FIRST_RUN,
        ));
        GIT.run(
            fixture.trunk(),
            ["merge", "--quiet", "--ff-only", &checkpoint],
        );
        commit(&fixture.holder, "later.rs", "commit D\n");
        let later_tip = GIT.stdout(&fixture.holder, ["rev-parse", "HEAD"]);
        let integrated = board(fixture.trunk());
        assert_eq!(
            snapshot(&integrated, &id)["integration_evidence"]["status"]["status"],
            "integrated"
        );

        for root in [&fixture.outsider, &fixture.holder] {
            let released = berth(root, &["release", &id, "--json"], FIRST_RUN);
            succeed(&released);
            let observed = board(fixture.trunk());
            let reservation = snapshot(&observed, &id);
            assert_eq!(reservation["lifecycle"]["stage"], "outstanding");
            assert_eq!(reservation["merge_extent"]["status"], "protected");
            assert_eq!(
                scope_paths(&reservation["merge_extent"]["scopes"]),
                BTreeSet::from(["later.rs".to_owned()])
            );
            assert_refused(&fixture.outsider, "file:later.rs", THIRD_RUN, &id);
        }
        assert_eq!(
            GIT.stdout(
                fixture.trunk(),
                ["rev-parse", &format!("{RETENTION_REF_PREFIX}{id}")]
            ),
            later_tip
        );
        assert!(
            events(fixture.trunk())
                .iter()
                .all(|event| { event["reservation_id"] != id || event["op"] != "release" })
        );
    }

    #[test]
    fn releasing_an_unavailable_extent_stops_its_derivation_alerts() {
        let fixture = Repository::new();
        let id = claim(&fixture.holder, "file:branch.rs", FIRST_RUN);
        commit(&fixture.holder, "branch.rs", "unmerged work\n");
        board(fixture.trunk());
        fs::rename(
            &fixture.holder,
            fixture.worktrees.path().join("unavailable-holder"),
        )
        .expect("holder should become unavailable");
        let unavailable = board(fixture.trunk());
        assert_eq!(
            snapshot(&unavailable, &id)["merge_extent"]["status"],
            "unavailable"
        );
        assert!(
            unavailable["payload"]["data"]["alerts"]["entries"]
                .as_array()
                .expect("board should list alerts")
                .iter()
                .any(|alert| alert["kind"] == "merge_extent_unavailable"
                    && alert["reservation_id"] == id),
            "the unavailable holder should have a board alert: {unavailable}"
        );
        succeed(&berth(
            fixture.trunk(),
            &[
                "resolve",
                &id,
                "--abandon",
                "--why",
                "branch work deliberately discarded",
                "--json",
            ],
            THIRD_RUN,
        ));
        let released = board(fixture.trunk());
        assert_eq!(snapshot(&released, &id)["lifecycle"]["stage"], "released");
        assert_eq!(
            snapshot(&released, &id)["merge_extent"]["status"],
            "unavailable"
        );
        assert_eq!(
            released["payload"]["data"]["alerts"]["entries"],
            serde_json::json!([])
        );
    }

    fn real_git() -> String {
        let output = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .expect("git location should resolve");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("git location should be UTF-8")
            .trim()
            .to_owned()
    }

    /// Three checkouts share object storage but have independent working trees.
    struct Repository {
        trunk_directory: TempDir,
        worktrees:       TempDir,
        holder:          PathBuf,
        outsider:        PathBuf,
    }

    impl Repository {
        fn new() -> Self {
            let repository = tempdir().expect("repository parent should exist");
            let root = repository.path();
            GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
            for (key, value) in [
                ("user.name", "Berth Test"),
                ("user.email", "berth@example.invalid"),
                ("maintenance.auto", "false"),
                ("gc.auto", "0"),
            ] {
                GIT.run(root, ["config", key, value]);
            }
            write(root, "tracked.rs", "base\n");
            write(root, "staged.rs", "base\n");
            GIT.run(root, ["add", "."]);
            GIT.run(root, ["commit", "--quiet", "-m", "base"]);
            succeed(&berth(root, &["init", "--json"], FIRST_RUN));
            // No hook or lifecycle verb runs when these tests integrate a branch. Only the
            // following board read may discover its now-empty merge surface.
            GIT.run(root, ["config", "core.hooksPath", "/dev/null"]);
            GIT.run(root, ["add", ".claude/config/berth.toml"]);
            GIT.run(root, ["commit", "--quiet", "-m", "configure berth"]);
            let worktrees = tempdir().expect("worktree parent should exist");
            let holder = worktrees.path().join("holder");
            let outsider = worktrees.path().join("outsider");
            for (branch, path) in [("holder", &holder), ("outsider", &outsider)] {
                GIT.run(
                    root,
                    [
                        "worktree",
                        "add",
                        "--quiet",
                        "-b",
                        branch,
                        path.to_str().expect("worktree path should be UTF-8"),
                        "main",
                    ],
                );
            }
            Self {
                trunk_directory: repository,
                worktrees,
                holder,
                outsider,
            }
        }

        fn trunk(&self) -> &Path { self.trunk_directory.path() }

        /// Keep the older holder's derived path and the newer holder's declaration distinct.
        fn retain_divergent_protection(&self) -> [String; 2] {
            let oldest = claim(&self.holder, "file:a.rs", FIRST_RUN);
            commit(&self.holder, "a.rs", "unmerged work\n");
            let observed = board(self.trunk());
            let extent = &snapshot(&observed, &oldest)["merge_extent"];
            assert_eq!(extent["status"], "protected");
            assert_eq!(
                scope_paths(&extent["scopes"]),
                BTreeSet::from(["a.rs".to_owned()])
            );
            let newer = claim(&self.holder, "file:b.rs", FIRST_RUN);
            assert!(
                !events(self.trunk()).iter().any(|event| {
                    event["reservation_id"] == newer && event["op"] == "merge_extent_observed"
                }),
                "the second holder must become unavailable before its first derivation"
            );
            fs::rename(
                &self.holder,
                self.worktrees.path().join("unavailable-holder"),
            )
            .expect("holder checkout should move without updating git metadata");

            let observed = board(self.trunk());
            let retained = &snapshot(&observed, &oldest)["merge_extent"];
            assert_eq!(retained["status"], "unavailable");
            assert_eq!(retained["retained_evidence"]["status"], "protected");
            assert_eq!(
                scope_paths(&retained["retained_evidence"]["scopes"]),
                BTreeSet::from(["a.rs".to_owned()])
            );
            let declared = &snapshot(&observed, &newer)["merge_extent"];
            assert_eq!(declared["status"], "unavailable");
            assert_eq!(declared["retained_evidence"]["status"], "not_derived");
            assert_eq!(
                scope_paths(&declared["retained_evidence"]["protection"]),
                BTreeSet::from(["b.rs".to_owned()])
            );
            [oldest, newer]
        }
    }

    fn write(root: &Path, path: &str, contents: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().expect("fixture file should have a parent"))
            .expect("fixture parent should exist");
        fs::write(path, contents).expect("fixture file should write");
    }

    fn commit(root: &Path, path: &str, contents: &str) {
        write(root, path, contents);
        GIT.run(root, ["add", path]);
        GIT.run(root, ["commit", "--quiet", "-m", "fixture work"]);
    }

    fn berth(root: &Path, arguments: &[&str], run: &str) -> Output {
        Command::new(BERTH)
            .args(arguments)
            .current_dir(root)
            .env("CARGO_BERTH_RUN", run)
            .env_remove("CARGO_BERTH_SESSION_ID")
            .env_remove("CARGO_BERTH_BYPASS")
            .output()
            .expect("cargo-berth should run")
    }

    #[track_caller]
    fn succeed(output: &Output) {
        assert!(
            output.status.success(),
            "berth failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn json(output: &Output) -> Value {
        serde_json::from_slice(&output.stdout).expect("berth should emit JSON")
    }

    fn board(root: &Path) -> Value {
        let output = berth(root, &["board", "--json"], THIRD_RUN);
        succeed(&output);
        json(&output)
    }

    fn events(root: &Path) -> Vec<Value> {
        fs::read_to_string(root.join(JOURNAL))
            .expect("journal should read")
            .lines()
            .map(|line| serde_json::from_str(line).expect("journal event should decode"))
            .collect()
    }

    #[track_caller]
    fn snapshot<'value>(envelope: &'value Value, id: &str) -> &'value Value {
        let data = &envelope["payload"]["data"];
        ["ready_now", "unconstrained_reservations", "resolved"]
            .into_iter()
            .flat_map(|section| data[section]["entries"].as_array().into_iter().flatten())
            .map(|entry| entry.get("reservation").unwrap_or(entry))
            .find(|entry| entry["reservation_id"] == id)
            .expect("reservation should have a board snapshot")
    }

    fn scope_paths(scopes: &Value) -> BTreeSet<String> {
        scopes
            .as_array()
            .expect("protected extent should contain scope entries")
            .iter()
            .map(|scope| {
                scope["path"]
                    .as_str()
                    .expect("scope should name a path")
                    .to_owned()
            })
            .collect()
    }

    fn claim(root: &Path, scope: &str, run: &str) -> String {
        let output = berth(root, &["claim", scope, "--run", run, "--json"], run);
        succeed(&output);
        json(&output)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("claim should return an id")
            .to_owned()
    }

    #[track_caller]
    fn assert_allowed(root: &Path, scope: &str, run: &str) {
        let output = berth(root, &["check", scope, "--json"], run);
        succeed(&output);
        assert_eq!(json(&output)["status"], "clear");
    }

    #[track_caller]
    fn assert_refused(root: &Path, scope: &str, run: &str, holder: &str) {
        let output = berth(root, &["check", scope, "--json"], run);
        let envelope = json(&output);
        assert!(
            !output.status.success(),
            "{scope} should be refused by {holder}: {envelope}"
        );
        assert!(
            envelope.to_string().contains(holder),
            "refusal should name its holder: {envelope}"
        );
    }
}
