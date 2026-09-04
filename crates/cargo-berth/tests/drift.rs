#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end drift fingerprint, selection, classification, replay, and hook tests.

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, clearing what its fixtures set for themselves.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[BYPASS_ENVIRONMENT, RUN_ENVIRONMENT],
};

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::fs::File;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;
use std::time::Duration;

use tempfile::TempDir;
use tempfile::tempdir;

const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";
const BERTH_BINARY_ENVIRONMENT: &str = "CARGO_BERTH_TEST_BINARY";
const COLLISION_SCOPE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_COLLISION_SCOPE";
const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const FOREIGN_CLAIM_ENVIRONMENT: &str = "CARGO_BERTH_TEST_FOREIGN_CLAIM";
const FOREIGN_ROOT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_FOREIGN_ROOT";
const FOREIGN_RUN_ENVIRONMENT: &str = "CARGO_BERTH_TEST_FOREIGN_RUN";
const GIT_BINARY: &str = "git";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const LOCK_PATH: &str = ".git/cargo-berth/mutation.lock";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const MARKER_RELEASE_OUTPUT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MARKER_RELEASE_OUTPUT";
const MARKER_RELEASE_RESERVATION_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MARKER_RELEASE_RESERVATION";
const MARKER_RELEASE_ROOT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MARKER_RELEASE_ROOT";
const MARKER_RELEASE_TRIGGER_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MARKER_RELEASE_TRIGGER";
/// Test-only path a process waiting on the mutation lock writes, making the wait observable.
const MUTATION_LOCK_READY_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MUTATION_LOCK_READY_PATH";
const MUTATION_LOCK_WAIT_ATTEMPTS: usize = 500;
const MUTATION_LOCK_WAIT_INTERVAL: Duration = Duration::from_millis(10);
const POST_COMMIT_HOOK_PATH: &str = ".git/hooks/post-commit";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const POST_COMMIT_ENVIRONMENT: &str = "CARGO_BERTH_POST_COMMIT";
const REAL_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_PATH";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const WORKTREE_ID_PATH: &str = ".git/cargo-berth-worktree-id";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    printf '%s\n' "$2" >> "$CARGO_BERTH_TEST_GIT_TRACE"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const COLLISION_GIT_WRAPPER: &str = r#"#!/bin/sh
set -eu
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "status" ]; then
    (
        cd "$CARGO_BERTH_TEST_FOREIGN_ROOT"
        PATH="$CARGO_BERTH_TEST_REAL_PATH" \
            "$CARGO_BERTH_TEST_BINARY" claim "$CARGO_BERTH_TEST_COLLISION_SCOPE" \
            --run "$CARGO_BERTH_TEST_FOREIGN_RUN" \
            --why "deterministic drift collision" --json
    ) > "$CARGO_BERTH_TEST_FOREIGN_CLAIM"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const MARKER_RELEASE_GIT_WRAPPER: &str = r#"#!/bin/sh

set -eu
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "status" ] \
    && [ ! -e "$CARGO_BERTH_TEST_MARKER_RELEASE_TRIGGER" ]; then
    : > "$CARGO_BERTH_TEST_MARKER_RELEASE_TRIGGER"
    (
        cd "$CARGO_BERTH_TEST_MARKER_RELEASE_ROOT"
        PATH="$CARGO_BERTH_TEST_REAL_PATH" \
            "$CARGO_BERTH_TEST_BINARY" release \
            "$CARGO_BERTH_TEST_MARKER_RELEASE_RESERVATION" --json
    ) > "$CARGO_BERTH_TEST_MARKER_RELEASE_OUTPUT"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;

#[test]
fn unconfigured_drift_does_not_create_a_worktree_identity() {
    let repository = scratch_repository();
    let worktree_id_path = repository.path().join(WORKTREE_ID_PATH);
    assert!(!worktree_id_path.exists());

    let output = drift(repository.path(), &["--full"]);
    let envelope = json_output(&output);

    assert_eq!(envelope["status"], "unconfigured");
    assert!(!worktree_id_path.exists());
}

#[test]
fn post_commit_drift_rejects_a_malformed_recorded_worktree_identity() {
    let repository = initialized_repository();
    claim(repository.path(), "file:claimed.txt", FIRST_RUN);
    let worktree_id_path = repository.path().join(WORKTREE_ID_PATH);
    fs::write(&worktree_id_path, "not-a-worktree-identity\n")
        .expect("malformed worktree identity should write");

    let rejected = post_commit_drift(repository.path(), &[]);
    let envelope = json_output(&rejected);

    assert_eq!(rejected.status.code(), Some(4));
    assert_eq!(envelope["status"], "ledger_unreadable");
    assert_eq!(envelope["payload"]["kind"], "no_facts");
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("invalid stored worktree identity")),
        "malformed identity failure lost its diagnostic: {envelope}"
    );
}

#[test]
fn full_classification_covers_silent_and_widen_rows() {
    let covered_repository = initialized_repository();
    let covered_id = claim(covered_repository.path(), "file:claimed.txt", FIRST_RUN);
    fs::write(covered_repository.path().join("claimed.txt"), "claimed\n")
        .expect("claimed path should write");
    let covered = drift(
        covered_repository.path(),
        &["--full", "--reservation", &covered_id],
    );
    assert!(covered.status.success());
    assert_eq!(json_output(&covered)["status"], "clear");
    assert!(!journal_text(covered_repository.path()).contains("\"op\":\"widen\""));

    let widened_repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&widened_repository, "second");
    let widened_id = claim(widened_repository.path(), "file:claimed.txt", FIRST_RUN);
    fs::write(widened_repository.path().join("untracked.txt"), "new\n")
        .expect("untracked path should write");
    let widened = drift(
        widened_repository.path(),
        &["--full", "--reservation", &widened_id],
    );
    let widened_envelope = json_output(&widened);
    assert!(widened.status.success());
    assert_eq!(widened_envelope["status"], "widened");
    assert_eq!(
        widened_envelope["payload"]["data"]["results"][0]["effects"][0]["kind"],
        "widened"
    );
    let widen_event = journal_events(widened_repository.path())
        .into_iter()
        .find(|event| event["op"] == "widen")
        .expect("drift should append a widen");
    assert_eq!(widen_event["added_scopes"][0]["path"], "untracked.txt");
    assert_eq!(widen_event["added_scopes"][0]["kind"], "file");
    assert_eq!(widen_event["cause"]["kind"], "drift");
    assert!(widen_event["cause"].get("observed_paths").is_none());
    assert_eq!(widen_event["edit_blocking_status"], "blocking");
    fs::remove_file(widened_repository.path().join(PROJECTION_PATH))
        .expect("projection should delete");
    fs::remove_file(widened_repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");
    let replayed_block = run_berth_with_run(
        &second_root,
        &["check", "file:untracked.txt", "--json"],
        SECOND_RUN,
    );
    assert_eq!(replayed_block.status.code(), Some(1));
}

#[test]
fn drift_rejects_every_invalid_coordination_identity_with_executable_recovery() {
    assert_drift_revalidates_marker_after_observation();

    let session_repository = initialized_repository();
    let stale_session = "stale-drift-session";
    let session_reservation_id = claim_with_session(
        session_repository.path(),
        "file:session",
        SECOND_RUN,
        stale_session,
    );
    let mapping_path = session_repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(
            session_repository.path(),
            &["release", &session_reservation_id, "--json"],
        )
        .status
        .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale mapping should write");
    let session_rejection = run_berth_with_session(
        session_repository.path(),
        &[
            "drift",
            "--full",
            "--reservation",
            &session_reservation_id,
            "--json",
        ],
        stale_session,
    );
    assert_eq!(session_rejection.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&session_rejection),
        "stale_session_mapping",
        &["clear_session_mapping"],
    );

    let mismatch_repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&mismatch_repository, "drift-second");
    let mismatch_session = "foreign-drift-session";
    let live_reservation_id = claim_with_session(
        mismatch_repository.path(),
        "file:live-session",
        THIRD_RUN,
        mismatch_session,
    );
    let mismatch_rejection = run_berth_with_session(
        &second_root,
        &[
            "drift",
            "--full",
            "--reservation",
            &live_reservation_id,
            "--json",
        ],
        mismatch_session,
    );
    assert_eq!(mismatch_rejection.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&mismatch_rejection),
        "session_worktree_mismatch",
        &["rerun_from_holding_worktree", "claim_separately_here"],
    );
}

fn assert_drift_revalidates_marker_after_observation() {
    let marker_repository = initialized_repository();
    let marker_reservation_id = claim(marker_repository.path(), "file:marker", FIRST_RUN);
    fs::write(
        marker_repository.path().join("marker-change.txt"),
        "changed\n",
    )
    .expect("marker observation path should write");
    let wrapper_directory = tempdir().expect("marker wrapper directory should exist");
    let wrapper_path = wrapper_directory.path().join(GIT_BINARY);
    let release_output_path = wrapper_directory.path().join("release.json");
    let release_trigger_path = wrapper_directory.path().join("release-triggered");
    fs::write(&wrapper_path, MARKER_RELEASE_GIT_WRAPPER).expect("marker wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("marker wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("marker wrapper should be executable");
    let real_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(wrapper_directory.path().to_path_buf())
            .chain(std::env::split_paths(&real_path)),
    )
    .expect("marker wrapper PATH should join");
    let marker_rejection = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args([
            "drift",
            "--full",
            "--reservation",
            &marker_reservation_id,
            "--json",
        ])
        .current_dir(marker_repository.path())
        .env("PATH", wrapped_path)
        .env(BERTH_BINARY_ENVIRONMENT, env!("CARGO_BIN_EXE_cargo-berth"))
        .env(MARKER_RELEASE_OUTPUT_ENVIRONMENT, &release_output_path)
        .env(
            MARKER_RELEASE_RESERVATION_ENVIRONMENT,
            &marker_reservation_id,
        )
        .env(MARKER_RELEASE_ROOT_ENVIRONMENT, marker_repository.path())
        .env(MARKER_RELEASE_TRIGGER_ENVIRONMENT, &release_trigger_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(REAL_PATH_ENVIRONMENT, real_path)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .output()
        .expect("marker revalidation drift should run");
    assert!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(release_output_path).expect("marker release output should read")
        )
        .is_ok()
    );
    assert_eq!(marker_rejection.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &json_output(&marker_rejection),
        "stale_marker_run",
        &["reconcile_and_sweep_marker"],
    );
}

#[test]
fn post_write_drift_creates_a_first_touch_reservation_when_none_exists() {
    let repository = initialized_repository();
    fs::write(repository.path().join("written-by-bash.rs"), "new\n")
        .expect("post-write path should write");

    let observed = post_commit_drift(repository.path(), &[]);
    let envelope = json_output(&observed);

    assert!(observed.status.success());
    assert_eq!(envelope["status"], "widened");
    let events = journal_events(repository.path());
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["op"], "claim");
    assert_eq!(events[0]["source"]["kind"], "first_touch");
    assert_eq!(events[0]["scopes"][0]["kind"], "file");
    assert_eq!(events[0]["scopes"][0]["path"], "written-by-bash.rs");
}

#[test]
fn post_write_drift_does_not_claim_a_path_the_edit_restored() {
    let repository = initialized_repository();
    fs::write(repository.path().join("design.md"), "original\n").expect("design path should write");
    git(repository.path(), &["add", "design.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "design"]);
    fs::write(repository.path().join("design.md"), "edited\n").expect("design path should edit");
    let claimed = cheap_post_commit_drift(repository.path());
    let claimed_envelope = json_output(&claimed);
    assert_eq!(claimed_envelope["status"], "widened");
    let reservation_id =
        claimed_envelope["payload"]["data"]["widening"]["acquisition"]["reservation_id"]
            .as_str()
            .expect("the post-write claim should return a reservation id")
            .to_owned();
    let abandoned = run_berth(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--abandon",
            "--why",
            "the first-touch edit is deliberately discarded",
            "--json",
        ],
    );
    assert!(
        abandoned.status.success(),
        "abandon failed: {}",
        String::from_utf8_lossy(&abandoned.stdout)
    );
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("released first-touch marker should remove");
    fs::write(repository.path().join("design.md"), "original\n")
        .expect("design path should restore");
    assert_eq!(
        git_stdout(repository.path(), &["status", "--porcelain"]),
        ""
    );
    let journal_before = journal_events(repository.path());

    let restored = cheap_post_commit_drift(repository.path());
    let envelope = json_output(&restored);

    assert!(restored.status.success());
    assert_eq!(envelope["payload"]["data"]["comparison"], "cheap_delta");
    assert_eq!(envelope["status"], "clear");
    assert_eq!(
        envelope["payload"]["data"]["widening"]["status"],
        "not_needed"
    );
    let full = post_commit_drift(repository.path(), &[]);
    let full_envelope = json_output(&full);
    assert_eq!(full_envelope["status"], envelope["status"]);
    assert_eq!(
        full_envelope["payload"]["data"]["widening"]["status"],
        envelope["payload"]["data"]["widening"]["status"]
    );
    assert_eq!(journal_events(repository.path()), journal_before);
}

#[test]
fn drift_does_not_widen_onto_a_path_the_worktree_restored() {
    let repository = initialized_repository();
    fs::write(repository.path().join("design.md"), "original\n").expect("design path should write");
    git(repository.path(), &["add", "design.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "design"]);
    let holder_id = claim(repository.path(), "file:design.md", FIRST_RUN);
    fs::write(repository.path().join("design.md"), "edited\n").expect("design path should edit");
    // Fingerprint the edit from inside a reservation that already covers it, so the
    // cheap comparison has a previous observation naming design.md and nothing has
    // widened onto it.
    assert_eq!(
        json_output(&cheap_post_commit_drift(repository.path()))["status"],
        "clear"
    );
    let abandoned = run_berth(
        repository.path(),
        &[
            "resolve",
            &holder_id,
            "--abandon",
            "--why",
            "the design edit is deliberately discarded",
            "--json",
        ],
    );
    assert!(
        abandoned.status.success(),
        "abandon failed: {}",
        String::from_utf8_lossy(&abandoned.stdout)
    );
    fs::write(repository.path().join("design.md"), "original\n")
        .expect("design path should restore");
    assert_eq!(
        git_stdout(repository.path(), &["status", "--porcelain"]),
        ""
    );
    let widening_id = claim(repository.path(), "file:claimed.txt", FIRST_RUN);

    let restored = cheap_post_commit_drift(repository.path());
    let envelope = json_output(&restored);

    assert!(restored.status.success());
    assert_eq!(envelope["payload"]["data"]["comparison"], "cheap_delta");
    assert_eq!(
        envelope["status"], "clear",
        "a path back at its committed content carries no work to acquire"
    );
    assert_eq!(
        envelope["payload"]["data"]["widening"]["status"],
        "not_needed"
    );
    assert!(!journal_text(repository.path()).contains("\"op\":\"widen\""));

    // The filter must not reach a path that does carry work.
    fs::write(repository.path().join("other.md"), "new work\n").expect("other path should write");
    let widened = cheap_post_commit_drift(repository.path());

    assert!(widened.status.success());
    assert_eq!(json_output(&widened)["status"], "widened");
    let widen_event = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "widen")
        .expect("drift should widen onto the path that carries work");
    assert_eq!(widen_event["reservation_id"], widening_id);
    assert_eq!(widen_event["added_scopes"][0]["path"], "other.md");
}

#[test]
fn post_write_drift_detects_but_cannot_prevent_a_foreign_incursion() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "post-write-holder");
    let foreign_id = claim(&foreign_root, "file:foreign-owned.rs", FIRST_RUN);
    fs::write(
        repository.path().join("foreign-owned.rs"),
        "already written\n",
    )
    .expect("foreign path should be written before the post-write hook");
    let journal_before = journal_events(repository.path());

    let observed = post_commit_drift(repository.path(), &[]);
    let envelope = json_output(&observed);

    assert_eq!(observed.status.code(), Some(1));
    assert_eq!(envelope["status"], "incursion");
    assert_eq!(envelope["blocked_by"], serde_json::json!([foreign_id]));
    assert!(
        envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("already happened"))
    );
    assert_eq!(journal_events(repository.path()), journal_before);
}

#[test]
fn incursion_incident_round_trip_deduplicates_and_resolves() {
    let incursion_repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(incursion_repository.path(), worktrees.path(), "foreign");
    let subject_id = claim(incursion_repository.path(), "file:owned.txt", FIRST_RUN);
    let foreign_id = claim(&foreign_root, "tree:shared", SECOND_RUN);
    fs::create_dir_all(incursion_repository.path().join("shared"))
        .expect("shared directory should exist");
    fs::write(
        incursion_repository.path().join("shared/entered.txt"),
        "incursion\n",
    )
    .expect("incursion path should write");
    let incursion = drift(
        incursion_repository.path(),
        &["--full", "--reservation", &subject_id],
    );
    let incursion_envelope = json_output(&incursion);
    assert_eq!(incursion.status.code(), Some(1));
    assert_eq!(incursion_envelope["exit_code"], 1);
    assert_eq!(incursion_envelope["status"], "incursion");
    assert_eq!(
        incursion_envelope["blocked_by"],
        serde_json::json!([foreign_id])
    );
    assert!(
        incursion_envelope["message"]
            .as_str()
            .is_some_and(|message| message.contains("Stop"))
    );
    let incursion_event = journal_events(incursion_repository.path())
        .into_iter()
        .find(|event| event["op"] == "incursion")
        .expect("drift should append an incursion");
    let incident_id = incursion_event["incident_id"]
        .as_str()
        .expect("incursion should carry an incident id")
        .to_owned();
    assert_eq!(incursion_event["reservation_id"], subject_id);
    assert_eq!(
        incursion_event["foreign_reservation_ids"],
        serde_json::json!([foreign_id])
    );
    assert_eq!(
        incursion_event["paths"],
        serde_json::json!(["shared/entered.txt"])
    );
    assert_eq!(
        incursion_envelope["payload"]["data"]["results"][0]["effects"][0]["incident_id"],
        incident_id
    );

    let repeated = drift(
        incursion_repository.path(),
        &["--full", "--reservation", &subject_id],
    );
    assert_eq!(repeated.status.code(), Some(1));
    let repeated_events = journal_events(incursion_repository.path());
    assert_eq!(
        repeated_events
            .iter()
            .filter(|event| event["op"] == "incursion")
            .count(),
        1
    );
    assert_eq!(
        json_output(&repeated)["payload"]["data"]["results"][0]["effects"][0]["incident_id"],
        incident_id
    );

    let resolved = run_berth(
        incursion_repository.path(),
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );
    assert!(resolved.status.success());
    let resolved_envelope = json_output(&resolved);
    assert_eq!(resolved_envelope["status"], "incursion_resolved");
    assert_eq!(
        resolved_envelope["payload"]["data"]["status"],
        "recorded_now"
    );
    let resolved_events = journal_events(incursion_repository.path());
    assert_eq!(
        resolved_events
            .iter()
            .filter(|event| event["op"] == "incursion")
            .count(),
        1
    );
    assert_eq!(
        resolved_events
            .iter()
            .filter(|event| event["op"] == "resolve_incursion")
            .count(),
        1
    );

    assert_resolution_silences_the_same_overlap(&incursion_repository, &subject_id, &incident_id);
}

#[test]
fn incursion_resolution_requires_current_enrollment() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "unenrolled-foreign");
    let subject_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    let _foreign_id = claim(&foreign_root, "tree:shared", SECOND_RUN);
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(repository.path().join("shared/entered.txt"), "incursion\n")
        .expect("incursion path should write");
    let observed = drift(repository.path(), &["--full", "--reservation", &subject_id]);
    assert_eq!(json_output(&observed)["status"], "incursion");
    let incident_id = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "incursion")
        .and_then(|event| event["incident_id"].as_str().map(str::to_owned))
        .expect("drift should append an incident");
    fs::remove_file(repository.path().join(CONFIGURATION_PATH))
        .expect("configuration should remove");

    let rejected = run_berth(
        repository.path(),
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );

    assert_eq!(rejected.status.code(), Some(4));
    assert_eq!(json_output(&rejected)["status"], "unconfigured");
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "resolve_incursion")
            .count(),
        0
    );
}

#[test]
fn linked_worktree_resolve_reports_recorded_same_actor_and_foreign_actor_outcomes() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let subject_root = add_worktree(repository.path(), worktrees.path(), "resolve-subject");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "resolve-foreign");
    let subject_id = claim(&subject_root, "file:owned.txt", FIRST_RUN);
    let wrong_subject_id = claim(&subject_root, "file:not-the-incident-owner.txt", FIRST_RUN);
    let _foreign_id = claim(&foreign_root, "tree:shared", SECOND_RUN);
    fs::create_dir_all(subject_root.join("shared")).expect("shared directory should exist");
    fs::write(subject_root.join("shared/entered.txt"), "incursion\n")
        .expect("incursion path should write");

    let incursion = drift(&subject_root, &["--full", "--reservation", &subject_id]);
    assert_eq!(json_output(&incursion)["status"], "incursion");
    let incident_id = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "incursion")
        .and_then(|event| event["incident_id"].as_str().map(str::to_owned))
        .expect("drift should append an incident");

    let first = run_berth(
        &subject_root,
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );
    assert_successful_incursion_resolution(&first, &subject_id, &incident_id, "recorded_now");

    let resolution_event = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "resolve_incursion")
        .expect("the first resolve should append its decision");
    assert_eq!(resolution_event["actor"]["run"], FIRST_RUN);

    let same_actor_repeat = run_berth(
        &subject_root,
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );
    assert_successful_incursion_resolution(
        &same_actor_repeat,
        &subject_id,
        &incident_id,
        "already_recorded_by_same_coordination_actor",
    );

    let same_actor_wrong_reservation = run_berth(
        &subject_root,
        &[
            "resolve",
            &wrong_subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );
    assert_eq!(same_actor_wrong_reservation.status.code(), Some(5));
    let mismatch_envelope = json_output(&same_actor_wrong_reservation);
    assert_eq!(mismatch_envelope["status"], "invalid_input");
    assert_eq!(mismatch_envelope["payload"]["kind"], "no_facts");
    assert_eq!(
        mismatch_envelope["message"],
        format!(
            "incursion incident {incident_id} does not belong to reservation {wrong_subject_id}"
        )
    );

    let foreign_actor_repeat = run_berth(
        &foreign_root,
        &[
            "resolve",
            &subject_id,
            "--incursion",
            &incident_id,
            "--json",
        ],
    );
    assert_foreign_incursion_resolution_rejection(
        &foreign_actor_repeat,
        &subject_id,
        &incident_id,
        &resolution_event,
    );
    assert_eq!(
        journal_events(repository.path())
            .iter()
            .filter(|event| event["op"] == "resolve_incursion")
            .count(),
        1,
        "replayed repeat resolves must not append another disposition"
    );
}

fn assert_successful_incursion_resolution(
    output: &Output,
    reservation_id: &str,
    incident_id: &str,
    expected_payload_status: &str,
) {
    assert!(
        output.status.success(),
        "resolve failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let envelope = json_output(output);
    assert_eq!(envelope["exit_code"], 0);
    assert_eq!(envelope["status"], "incursion_resolved");
    assert_eq!(envelope["payload"]["kind"], "resolve");
    assert_eq!(
        envelope["payload"]["data"]["status"],
        expected_payload_status
    );
    assert_eq!(
        envelope["payload"]["data"]["reservation_id"],
        reservation_id
    );
    assert_eq!(envelope["payload"]["data"]["incident_id"], incident_id);
}

fn assert_foreign_incursion_resolution_rejection(
    output: &Output,
    reservation_id: &str,
    incident_id: &str,
    resolution_event: &serde_json::Value,
) {
    assert_eq!(output.status.code(), Some(5));
    let envelope = json_output(output);
    assert_eq!(envelope["exit_code"], 5);
    assert_eq!(envelope["status"], "invalid_input");
    assert_eq!(envelope["payload"]["kind"], "resolve");
    let rejection = &envelope["payload"]["data"];
    assert_eq!(
        rejection["status"],
        "already_recorded_by_different_coordination_actor"
    );
    assert_eq!(rejection["reservation_id"], reservation_id);
    assert_eq!(rejection["incident_id"], incident_id);
    assert_eq!(
        rejection["resolving_worktree_id"],
        resolution_event["actor"]["worktree"]
    );
    assert_eq!(
        rejection["resolving_coordination_run_id"],
        resolution_event["actor"]["run"]
    );
    assert_eq!(
        rejection["resolution_event_id"],
        resolution_event["event_id"]
    );
    assert_eq!(rejection["resolved_at"], resolution_event["at"]);
    assert_eq!(envelope["blocked_by"], serde_json::json!([]));
}

/// The straying edit stays on disk, so a disposition has to settle the overlap for good.
fn assert_resolution_silences_the_same_overlap(
    incursion_repository: &TempDir,
    subject_id: &str,
    original_incident_id: &str,
) {
    let observed_after_resolution = drift(
        incursion_repository.path(),
        &["--full", "--reservation", subject_id],
    );
    assert_eq!(observed_after_resolution.status.code(), Some(0));
    let incident_ids = journal_events(incursion_repository.path())
        .into_iter()
        .filter(|event| event["op"] == "incursion")
        .map(|event| {
            event["incident_id"]
                .as_str()
                .expect("incursion should carry an incident id")
                .to_owned()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        incident_ids,
        vec![original_incident_id.to_owned()],
        "an answered incursion must not be raised again for the same overlap"
    );
}

#[test]
fn a_backlog_of_incursions_reports_its_size_and_clears_in_one_disposition() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "backlog-holder");
    // Two holders means two runs, and one run holds one worktree, so the second holder
    // needs a checkout of its own.
    let second_foreign_root = add_worktree(repository.path(), worktrees.path(), "backlog-second");
    let subject_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    let first_holder = claim(&foreign_root, "file:first-held.txt", SECOND_RUN);
    let second_holder = claim(&second_foreign_root, "file:second-held.txt", THIRD_RUN);
    fs::write(repository.path().join("first-held.txt"), "entered\n")
        .expect("first held path should write");
    let first = drift(repository.path(), &["--full", "--reservation", &subject_id]);
    assert_eq!(json_output(&first)["status"], "incursion");
    fs::write(repository.path().join("second-held.txt"), "entered\n")
        .expect("second held path should write");
    let second = drift(repository.path(), &["--full", "--reservation", &subject_id]);
    assert_eq!(json_output(&second)["status"], "incursion");

    let board = run_berth(repository.path(), &["board", "--json"]);
    let entries = json_output(&board)["payload"]["data"]["outstanding_incursions"]["entries"]
        .as_array()
        .expect("the board should list outstanding incursions")
        .clone();

    let straying = entries
        .iter()
        .filter(|entry| entry["straying_reservation_id"] == subject_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        straying.len(),
        2,
        "two incidents stand outstanding: {board:?}"
    );
    for entry in &straying {
        assert_eq!(
            entry["outstanding_count"], 2,
            "a notice must say how many it stands for"
        );
        assert_eq!(
            entry["resolution"]["every_flag"],
            format!("resolve {subject_id} --every-incursion")
        );
    }
    assert!(
        entries
            .iter()
            .filter(|entry| entry["straying_reservation_id"] != subject_id.as_str())
            .all(|entry| entry["outstanding_count"] == 1),
        "the count is per reservation, not a total"
    );

    let cleared = run_berth(
        repository.path(),
        &["resolve", &subject_id, "--every-incursion", "--json"],
    );

    assert!(
        cleared.status.success(),
        "every-incursion failed: {}",
        String::from_utf8_lossy(&cleared.stdout)
    );
    let cleared_envelope = json_output(&cleared);
    assert_eq!(cleared_envelope["status"], "incursion_resolved");
    assert_eq!(
        cleared_envelope["payload"]["data"]["incident_ids"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    let after = run_berth(repository.path(), &["board", "--json"]);
    assert!(
        json_output(&after)["payload"]["data"]["outstanding_incursions"]["entries"]
            .as_array()
            .expect("the board should list outstanding incursions")
            .iter()
            .all(|entry| entry["straying_reservation_id"] != subject_id.as_str()),
        "one disposition clears the whole set"
    );
    let named_holders = straying
        .iter()
        .filter_map(|entry| entry["foreign_reservation_ids"].as_array())
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        named_holders.contains(&first_holder.as_str())
            && named_holders.contains(&second_holder.as_str()),
        "the backlog stands against both holders: {named_holders:?}"
    );
}

#[test]
fn resolve_rejects_an_unknown_incursion_incident() {
    let repository = initialized_repository();
    let reservation_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);

    let rejected = run_berth(
        repository.path(),
        &[
            "resolve",
            &reservation_id,
            "--incursion",
            "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d",
            "--json",
        ],
    );

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(json_output(&rejected)["status"], "invalid_input");
    assert!(!journal_text(repository.path()).contains("\"op\":\"resolve_incursion\""));
}

#[test]
fn full_classification_reports_a_locked_widen_collision_without_journaling_it() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "collision-holder");
    let subject_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    let colliding_path = "shared/colliding.txt";
    let collision_scope = format!("file:{colliding_path}");
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(
        repository.path().join(colliding_path),
        "unheld before drift\n",
    )
    .expect("colliding path should write");

    let directory = tempdir().expect("wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let foreign_claim_path = directory.path().join("foreign-claim.json");
    fs::write(&wrapper_path, COLLISION_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let real_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf()).chain(std::env::split_paths(&real_path)),
    )
    .expect("wrapped PATH should join");

    let collision = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["drift", "--full", "--reservation", &subject_id, "--json"])
        .current_dir(repository.path())
        .env("PATH", wrapped_path)
        .env(BERTH_BINARY_ENVIRONMENT, env!("CARGO_BIN_EXE_cargo-berth"))
        .env(COLLISION_SCOPE_ENVIRONMENT, &collision_scope)
        .env(FOREIGN_CLAIM_ENVIRONMENT, &foreign_claim_path)
        .env(FOREIGN_ROOT_ENVIRONMENT, &foreign_root)
        .env(FOREIGN_RUN_ENVIRONMENT, SECOND_RUN)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(REAL_PATH_ENVIRONMENT, real_path)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .output()
        .expect("collision drift should run");
    let foreign_claim: serde_json::Value = serde_json::from_slice(
        &fs::read(foreign_claim_path).expect("foreign claim output should read"),
    )
    .expect("foreign claim should render JSON");
    let foreign_id = foreign_claim["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("foreign claim should return a reservation id");
    let collision_envelope = json_output(&collision);
    let collision_effect = &collision_envelope["payload"]["data"]["results"][0]["effects"][0];

    assert_eq!(collision.status.code(), Some(1));
    assert_eq!(collision_effect["kind"], "collision");
    assert_eq!(
        collision_effect["foreign_reservation_ids"],
        serde_json::json!([foreign_id])
    );
    assert_eq!(
        collision_effect["paths"],
        serde_json::json!([colliding_path])
    );
    let events = journal_events(repository.path());
    assert!(!events.iter().any(|event| {
        event["op"] == "widen"
            && event["added_scopes"]
                .as_array()
                .is_some_and(|scopes| scopes.iter().any(|scope| scope["path"] == colliding_path))
    }));
    assert!(!events.iter().any(|event| event["op"] == "collision"));
}

#[test]
fn post_commit_uses_same_run_and_worktree_reservations_as_coverage() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::write(repository.path().join("first.txt"), "first reservation\n")
        .expect("first reservation path should write");
    git(repository.path(), &["add", "first.txt"]);

    let committed = git_output(
        repository.path(),
        &["commit", "-m", "first reservation work"],
    );

    assert!(committed.status.success());
    assert!(committed.stderr.is_empty());
    let journal = journal_text(repository.path());
    assert!(!journal.contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{second_id}\""
    )));
    assert!(!journal.contains(&format!(
        "\"op\":\"incursion\",\"reservation_id\":\"{second_id}\""
    )));
    assert!(journal.contains(&first_id));
}

#[test]
fn a_committed_incursion_names_the_commits_that_introduced_its_paths() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "provenance");
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    claim(repository.path(), "file:also-held.txt", FIRST_RUN);
    let subject_id = claim(&foreign_root, "file:own.txt", SECOND_RUN);
    fs::write(foreign_root.join("held.txt"), "entered holder scope\n")
        .expect("held path should write");
    git(&foreign_root, &["add", "held.txt"]);
    git(
        &foreign_root,
        &["commit", "--quiet", "-m", "enter the holder scope"],
    );
    let entering_commit = git_stdout(&foreign_root, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    // A second held path entered but never committed, so the commit list must not claim it.
    fs::write(foreign_root.join("also-held.txt"), "entered uncommitted\n")
        .expect("second held path should write");

    let reported = run_berth_with_run(
        &foreign_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);

    assert_eq!(envelope["status"], "incursion");
    // Each held path has its own holder, so each is reported against that holder alone.
    let effects = incursion_effects(&envelope);
    assert_eq!(
        effects.len(),
        2,
        "each held path has its own holder: {envelope}"
    );
    let committed = incursion_for(&effects, "held.txt");
    let uncommitted = incursion_for(&effects, "also-held.txt");

    let commits = committed["commits"]
        .as_array()
        .expect("an incursion should carry a commit list");
    assert_eq!(
        commits.len(),
        1,
        "only the committed path came from a commit: {committed}"
    );
    assert_eq!(commits[0]["commit"], entering_commit);
    assert_eq!(commits[0]["subject"], "enter the holder scope");
    assert_eq!(
        commits[0]["origin"], "phase_authored",
        "trunk does not carry the entering commit"
    );
    assert_eq!(commits[0]["paths"][0], "held.txt");
    assert_eq!(
        commits[0]["paths"].as_array().map(Vec::len),
        Some(1),
        "the uncommitted path has no commit behind it"
    );
    assert_eq!(
        uncommitted["commits"].as_array().map(Vec::len),
        Some(0),
        "the uncommitted path has no commit behind it: {uncommitted}"
    );
    let message = envelope["payload"]["message"]
        .as_str()
        .or_else(|| envelope["message"].as_str())
        .unwrap_or_default()
        .to_owned();
    let rendered = format!("{message}{}", String::from_utf8_lossy(&reported.stderr));
    assert!(
        rendered.contains(&entering_commit[..8]) && rendered.contains("this phase authored it"),
        "the incursion message should name the commit: {rendered}"
    );
    assert!(rendered.contains(&holder_id));
}

#[test]
fn incursion_attribution_treats_pathspec_magic_as_literal_path_text() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "literal-pathspecs");
    let colon_path = ":held.txt";
    let glob_path = "held*[name].txt";
    claim(repository.path(), &format!("file:{colon_path}"), FIRST_RUN);
    claim(repository.path(), &format!("file:{glob_path}"), FIRST_RUN);
    let subject_id = claim(&foreign_root, "file:own.txt", SECOND_RUN);
    fs::write(foreign_root.join(colon_path), "colon path\n").expect("colon path should write");
    fs::write(foreign_root.join(glob_path), "glob path\n").expect("glob path should write");
    git(&foreign_root, &["add", "-A"]);
    git(
        &foreign_root,
        &["commit", "--quiet", "-m", "enter literal holder paths"],
    );

    let reported = run_berth_with_run(
        &foreign_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);
    // Each literal path has its own holder, so gather the commit paths across both.
    let effects = incursion_effects(&envelope);
    assert!(
        !effects.is_empty(),
        "drift should report an incursion: {envelope}"
    );
    let commit_paths: Vec<String> = effects
        .iter()
        .filter_map(|effect| effect["commits"].as_array())
        .flatten()
        .filter_map(|commit| commit["paths"].as_array())
        .flatten()
        .filter_map(|path| path.as_str().map(ToOwned::to_owned))
        .collect();

    assert!(commit_paths.iter().any(|path| path == colon_path));
    assert!(commit_paths.iter().any(|path| path == glob_path));
}

#[test]
fn conflict_resolution_only_path_is_attributed_to_the_merge_commit() {
    let repository = initialized_repository();
    fs::write(repository.path().join("conflict.txt"), "base\n")
        .expect("conflict base should write");
    git(repository.path(), &["add", "conflict.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "conflict base",
        ],
    );
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "dense-merge");
    claim(repository.path(), "file:held.txt", FIRST_RUN);
    let subject_id = claim(&foreign_root, "file:own.txt", SECOND_RUN);

    fs::write(repository.path().join("conflict.txt"), "main\n")
        .expect("main conflict side should write");
    git(repository.path(), &["add", "conflict.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "main conflict side",
        ],
    );
    fs::write(foreign_root.join("conflict.txt"), "feature\n")
        .expect("feature conflict side should write");
    git(&foreign_root, &["add", "conflict.txt"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "feature conflict side",
        ],
    );
    let conflicted = git_output(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-edit",
            "main",
        ],
    );
    assert!(!conflicted.status.success());
    fs::write(foreign_root.join("conflict.txt"), "resolved\n")
        .expect("conflict resolution should write");
    fs::write(foreign_root.join("held.txt"), "merge-only path\n")
        .expect("merge-only held path should write");
    git(&foreign_root, &["add", "-A"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "resolve conflict with held path",
        ],
    );
    let merge_commit = git_stdout(&foreign_root, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let reported = run_berth_with_run(
        &foreign_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);
    let commits = envelope["payload"]["data"]["results"]
        .as_array()
        .and_then(|results| {
            results.iter().find_map(|result| {
                result["effects"]
                    .as_array()?
                    .iter()
                    .find(|effect| effect["kind"] == "incursion")
                    .and_then(|effect| effect["commits"].as_array())
            })
        })
        .expect("the incursion should carry merge attribution");

    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["commit"], merge_commit);
    assert_eq!(commits[0]["paths"], serde_json::json!(["held.txt"]));
}

#[test]
fn batched_attribution_matches_per_path_history_across_git_path_cases() {
    let fixture = prepare_differential_attribution_repository();
    commit_path_encoding_history(&fixture.subject_root);
    commit_conflict_resolution_only_history(&fixture.trunk_root, &fixture.subject_root);
    let selected_paths = differential_attribution_paths();
    let reference = per_path_incursion_attribution(
        &fixture.subject_root,
        &fixture.phase_start,
        &selected_paths,
    );
    let reported = run_berth_with_run(
        &fixture.subject_root,
        &[
            "drift",
            "--full",
            "--reservation",
            &fixture.subject_reservation_id,
            "--json",
        ],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);
    assert_eq!(
        envelope["payload"]["kind"], "drift",
        "differential drift failed before reporting: {envelope:#}"
    );
    let batched = reported_incursion_attribution(&envelope, &fixture.subject_reservation_id);

    assert_eq!(batched, reference);
    assert!(batched.values().any(|commit| {
        commit.paths.contains("held/merged.txt") && commit.subject == "side branch held path"
    }));
    assert!(batched.values().any(|commit| {
        commit.paths.contains("held/conflict-only.txt")
            && commit.subject == "resolve with held path"
    }));
}

#[test]
fn stale_phase_anchor_does_not_suppress_valid_reservation_attribution() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "mixed-anchors");
    fs::write(foreign_root.join("stale-anchor.txt"), "stale anchor\n")
        .expect("stale anchor path should write");
    git(&foreign_root, &["add", "stale-anchor.txt"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "stale phase anchor",
        ],
    );
    let stale_id = claim(&foreign_root, "file:stale-owned.txt", SECOND_RUN);
    git(
        &foreign_root,
        &["-c", "core.hooksPath=/dev/null", "reset", "--hard", "main"],
    );
    let valid_id = claim(&foreign_root, "file:valid-owned.txt", SECOND_RUN);
    claim(repository.path(), "file:held.txt", FIRST_RUN);
    fs::write(foreign_root.join("held.txt"), "entered holder scope\n")
        .expect("held path should write");
    git(&foreign_root, &["add", "held.txt"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "valid anchored incursion",
        ],
    );
    let entering_commit = git_stdout(&foreign_root, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();

    let reported = post_commit_drift(&foreign_root, &[]);
    let envelope = json_output(&reported);
    let results = envelope["payload"]["data"]["results"]
        .as_array()
        .expect("mixed-anchor drift should report both reservations");
    let commits_for = |reservation_id: &str| {
        results
            .iter()
            .find(|result| result["reservation_id"] == reservation_id)
            .and_then(|result| result["effects"].as_array())
            .and_then(|effects| effects.iter().find(|effect| effect["kind"] == "incursion"))
            .and_then(|effect| effect["commits"].as_array())
            .expect("each mixed-anchor reservation should report its incursion")
    };

    assert!(commits_for(&stale_id).is_empty());
    assert_eq!(commits_for(&valid_id)[0]["commit"], entering_commit);
}

#[test]
fn mixed_anchor_batch_preserves_nineteen_valid_and_one_independent_history() {
    const VALID_ANCHOR_COUNT: usize = 19;

    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "anchor-matrix");
    claim(repository.path(), "tree:held", FIRST_RUN);
    let valid_ids = create_valid_anchor_reservations(&foreign_root, VALID_ANCHOR_COUNT);
    let independent_id =
        claim_independent_history_reservation(&foreign_root, repository.path(), "anchor-matrix");
    let stale_id = claim_stale_anchor_reservation(&foreign_root);
    let unreadable = claim_unreadable_phase_start_reservation(&foreign_root);

    fs::create_dir_all(foreign_root.join("held")).expect("held directory should exist");
    fs::write(
        foreign_root.join("held/entered.txt"),
        "entered holder scope\n",
    )
    .expect("held path should write");
    git(&foreign_root, &["add", "held/entered.txt"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "anchor matrix incursion",
        ],
    );
    let entering_commit = git_stdout(&foreign_root, &["rev-parse", "HEAD"]);

    let reported = post_commit_drift(&foreign_root, &[]);
    let envelope = json_output(&reported);
    let results = envelope["payload"]["data"]["results"]
        .as_array()
        .expect("anchor matrix should report every reservation");
    for reservation_id in valid_ids.iter().chain(std::iter::once(&independent_id)) {
        let result = results
            .iter()
            .find(|result| result["reservation_id"] == reservation_id.as_str())
            .expect("valid reservation should remain in the report");
        assert_eq!(
            result["status"], "changed",
            "reservation {reservation_id} did not report a changed result: {result:#}"
        );
        assert!(
            result["effects"]
                .as_array()
                .is_some_and(|effects| effects.iter().any(|effect| effect["kind"] == "incursion")),
            "reservation {reservation_id} did not report an incursion: {result:#}"
        );
        let commits = result_incursion_commits(results, reservation_id);
        assert!(
            commits
                .iter()
                .any(|commit| commit["commit"] == entering_commit),
            "reservation {reservation_id} lost valid attribution: {commits:?}"
        );
    }
    let stale_result = results
        .iter()
        .find(|result| result["reservation_id"] == stale_id)
        .expect("stale reservation should remain in the report");
    assert!(
        stale_result["effects"]
            .as_array()
            .is_some_and(|effects| effects.iter().any(|effect| effect["kind"] == "incursion")),
        "stale reservation did not report an incursion: {stale_result:#}"
    );
    assert!(result_incursion_commits(results, &stale_id).is_empty());
    let missing_result = results
        .iter()
        .find(|result| result["reservation_id"] == unreadable.reservation_id)
        .expect("missing-anchor reservation should remain in the report");
    assert_eq!(missing_result["status"], "phase_start_object_unknown");
    assert_eq!(missing_result["phase_start"], unreadable.phase_start);
}

#[test]
fn an_incursion_from_merged_trunk_work_says_the_phase_did_not_author_it() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "receives-trunk");
    let subject_id = claim(&foreign_root, "file:own.txt", SECOND_RUN);
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    fs::write(repository.path().join("held.txt"), "trunk work\n").expect("held path should write");
    git(repository.path(), &["add", "held.txt"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "trunk work"],
    );
    let trunk_commit = git_stdout(repository.path(), &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    // Taking trunk's work into the phase puts a commit the phase never wrote inside
    // <phase_start>..HEAD, which is how a false incursion arrives.
    git(&foreign_root, &["merge", "--quiet", "main"]);

    let reported = run_berth_with_run(
        &foreign_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);

    assert_eq!(envelope["status"], "incursion");
    let effect = envelope["payload"]["data"]["results"]
        .as_array()
        .expect("drift should report results")
        .iter()
        .find_map(|result| {
            result["effects"]
                .as_array()?
                .iter()
                .find(|effect| effect["kind"] == "incursion")
        })
        .expect("drift should report an incursion")
        .clone();
    assert_eq!(effect["paths"][0], "held.txt");
    assert_eq!(effect["commits"][0]["commit"], trunk_commit);
    assert_eq!(
        effect["commits"][0]["origin"], "already_on_trunk",
        "the phase received this commit rather than writing it: {effect}"
    );
    assert_eq!(effect["commits"][0]["paths"][0], "held.txt");
    assert_eq!(effect["foreign_reservation_ids"][0], holder_id);
}

#[test]
fn unresolved_trunk_keeps_commit_attribution_and_marks_its_origin_unknown() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "missing-trunk");
    claim(repository.path(), "file:held.txt", FIRST_RUN);
    let subject_id = claim(&foreign_root, "file:own.txt", SECOND_RUN);
    fs::write(foreign_root.join("held.txt"), "entered holder scope\n")
        .expect("held path should write");
    git(&foreign_root, &["add", "held.txt"]);
    git(
        &foreign_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "enter with unresolved trunk",
        ],
    );
    let entering_commit = git_stdout(&foreign_root, &["rev-parse", "HEAD"])
        .trim()
        .to_owned();
    let configuration_path = foreign_root.join(CONFIGURATION_PATH);
    let configuration = fs::read_to_string(&configuration_path)
        .expect("foreign worktree configuration should read");
    fs::write(
        configuration_path,
        configuration.replace("\"main\"", "\"does-not-exist\""),
    )
    .expect("foreign worktree configuration should select a missing trunk");

    let reported = run_berth_with_run(
        &foreign_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );
    let envelope = json_output(&reported);
    let commit = envelope["payload"]["data"]["results"]
        .as_array()
        .and_then(|results| {
            results.iter().find_map(|result| {
                result["effects"]
                    .as_array()?
                    .iter()
                    .find(|effect| effect["kind"] == "incursion")
                    .map(|effect| &effect["commits"][0])
            })
        })
        .expect("the incursion should retain commit attribution");

    assert_eq!(commit["commit"], entering_commit);
    assert_eq!(commit["origin"], "unknown");
}

/// The two second parties post-commit answers, and it refuses only one of them a berth.
///
/// A second run in the holder's own worktree is refused a reservation of its own and told so
/// by name. That refusal governs acquisition alone, so the commit is observed and classified
/// first, and its entry into the holder's scopes is reported as an incursion naming the
/// holder as the blocking reservation.
///
/// Reported, not journalled. A journal incursion is always recorded against a subject
/// reservation, and a run refused a berth holds none, so no incident can name it. Another
/// worktree is refused nothing, holds a reservation of its own, and its entry is both
/// reported and journalled against it.
#[test]
fn post_commit_refuses_another_run_here_and_reports_another_worktree_as_foreign() {
    let repository = initialized_repository();
    let (_foreign_directory, foreign_root) = foreign_worktree(&repository, "foreign");
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    fs::write(repository.path().join("held.txt"), "entered holder scope\n")
        .expect("holder path should write");
    git(repository.path(), &["add", "held.txt"]);
    let claims_before_the_second_run = claim_event_count(repository.path());

    let same_worktree = git_output_with_environment(
        repository.path(),
        &["commit", "-m", "enter the other run"],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    let same_worktree_warning = String::from_utf8_lossy(&same_worktree.stderr);

    assert!(same_worktree.status.success());
    assert_eq!(
        git_stdout(repository.path(), &["log", "-1", "--format=%s"]),
        "enter the other run",
        "a refused post-commit check leaves the commit in place"
    );
    assert!(
        journal_events(repository.path())
            .into_iter()
            .all(|event| event["op"] != "incursion"),
        "a journal incursion is recorded against a subject reservation and the refused run \
         holds none, so its entry is reported rather than journalled"
    );
    assert_eq!(
        claim_event_count(repository.path()),
        claims_before_the_second_run,
        "the refusal governs acquisition, so the refused run takes no reservation"
    );
    assert!(
        same_worktree_warning.contains("already holds active reservation"),
        "a second run in the holder's worktree should be refused by name: {same_worktree_warning}"
    );
    assert!(same_worktree_warning.contains(&holder_id));
    assert!(same_worktree_warning.contains(FIRST_RUN));
    assert!(same_worktree_warning.contains(SECOND_RUN));
    assert!(
        same_worktree_warning.contains(&format!(
            "Acquisition is all this refuses: no reservation was taken or widened for \
             coordination run {SECOND_RUN}."
        )),
        "the refusal should name what it refused, not claim the invocation recorded nothing: \
         {same_worktree_warning}"
    );
    assert!(
        same_worktree_warning.contains(
            "Whatever else this invocation reports, it observed and recorded regardless."
        ),
        "the refusal should say the observation it did not condition still stands: \
         {same_worktree_warning}"
    );
    assert!(
        same_worktree_warning.contains(&format!(
            "Post-write detection found changed paths held.txt inside foreign reservations \
             {holder_id}."
        )),
        "the refused run's entry into the holder's scopes is reported to the committer by \
         path and by holder, not swallowed by the refusal: {same_worktree_warning}"
    );
    assert!(
        !same_worktree_warning.contains("could not complete the post-commit drift check"),
        "a refused run completes its observation and reports it, rather than aborting the \
         check the way the refusal once did: {same_worktree_warning}"
    );

    assert_the_refused_entry_is_reported_but_not_journalled(repository.path(), &holder_id);
    assert_a_foreign_worktree_entry_is_recorded_against_its_own_reservation(
        repository.path(),
        &foreign_root,
        &holder_id,
    );
}

/// Assert the refused run's entry into the incumbent's scopes is reported, and only reported.
///
/// The git hook renders text, so the same observation is read again as JSON. Nothing was
/// published for it -- a blocking report withholds the fingerprint -- so the second read
/// observes the same entry and, holding no reservation to be a subject of, journals nothing
/// for it either time.
fn assert_the_refused_entry_is_reported_but_not_journalled(
    repository_root: &Path,
    holder_id: &str,
) {
    let reported = post_commit_drift_under_run(repository_root, SECOND_RUN);
    let reported_envelope = json_output(&reported);
    let entry = &reported_envelope["payload"]["data"]["widening"];

    assert_eq!(
        reported_envelope["status"], "incursion",
        "the refused run's entry into the holder's scopes is the report's headline: \
         {reported_envelope}"
    );
    assert_eq!(entry["status"], "post_write_incursion");
    assert_eq!(entry["paths"], serde_json::json!(["held.txt"]));
    assert_eq!(entry["conflicts"][0]["reservation_id"], holder_id);
    assert_eq!(
        reported_envelope["blocked_by"],
        serde_json::json!([holder_id]),
        "the reported incursion names the holder as the blocking reservation: \
         {reported_envelope}"
    );
    assert_eq!(
        reported_envelope["payload"]["data"]["scope_acquisition"]["status"],
        "refused_to_second_run",
        "one report states both the entry it observed and the berth it was refused: \
         {reported_envelope}"
    );
    assert!(
        journal_events(repository_root)
            .into_iter()
            .all(|event| event["op"] != "incursion"),
        "reporting the entry a second time still journals no incident for a run that holds \
         no reservation to record one against"
    );
}

/// Assert another worktree is refused nothing and its entry is recorded against its holder.
fn assert_a_foreign_worktree_entry_is_recorded_against_its_own_reservation(
    repository_root: &Path,
    foreign_root: &Path,
    holder_id: &str,
) {
    let foreign_id = claim(foreign_root, "file:foreign.txt", THIRD_RUN);
    fs::write(foreign_root.join("held.txt"), "entered holder scope\n")
        .expect("foreign path should write");
    git(foreign_root, &["add", "held.txt"]);

    let foreign = git_output(foreign_root, &["commit", "-m", "enter the holder scope"]);
    let warning = String::from_utf8_lossy(&foreign.stderr);

    assert!(foreign.status.success());
    assert!(warning.contains("Incursion"));
    assert!(warning.contains(holder_id));
    assert!(warning.contains(&foreign_id));
    let incursion = journal_events(repository_root)
        .into_iter()
        .find(|event| event["op"] == "incursion" && event["reservation_id"] == foreign_id)
        .expect("the foreign worktree should record an incursion");
    let incident_id = incursion["incident_id"]
        .as_str()
        .expect("incursion should carry an incident id");
    assert!(warning.contains(&format!("resolve {foreign_id} --incursion {incident_id}")));
    assert_eq!(
        incursion["foreign_reservation_ids"],
        serde_json::json!([holder_id])
    );
}

/// An incumbent holding `held.txt` here, which committed that path itself before a second run
/// ever arrived.
///
/// The incumbent's own commit sits inside its `phase_start..HEAD` range. That range is a commit
/// range and not an authorship record, so once a second run commits onto the same branch in the
/// same worktree, anything reading the range as what the second run wrote is handed `held.txt`
/// too. Both directions of that mistake are pinned against this one standing: the run that
/// entered nothing must not be told it entered `held.txt`, and the run that entered `held.txt`
/// must still be told so.
struct IncumbentThatCommittedItsOwnScope {
    /// The worktree both runs stand in; owns the repository for the fixture's lifetime.
    repository: TempDir,
    /// The reservation the incumbent holds over `held.txt`.
    holder_id:  String,
}

impl IncumbentThatCommittedItsOwnScope {
    /// Claim `held.txt` for the incumbent and commit it here under the incumbent's own run.
    fn stand_up() -> Self {
        let repository = initialized_repository();
        let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
        fs::write(
            repository.path().join("held.txt"),
            "the incumbent's own work\n",
        )
        .expect("holder path should write");
        git(repository.path(), &["add", "held.txt"]);
        let committed = git_output_with_environment(
            repository.path(),
            &["commit", "-m", "the incumbent commits its own scope"],
            RUN_ENVIRONMENT,
            FIRST_RUN,
        );
        assert!(
            committed.status.success(),
            "the incumbent's own commit should stand: {}",
            String::from_utf8_lossy(&committed.stderr)
        );
        Self {
            repository,
            holder_id,
        }
    }

    /// Commit one path here under a second presented run, which the occupancy rule refuses.
    fn the_second_run_commits(&self, path: &str, contents: &str) -> Output {
        fs::write(self.repository.path().join(path), contents)
            .expect("the second run's path should write");
        git(self.repository.path(), &["add", path]);
        let committed = git_output_with_environment(
            self.repository.path(),
            &["commit", "-m", "the second run commits"],
            RUN_ENVIRONMENT,
            SECOND_RUN,
        );
        assert!(
            committed.status.success(),
            "a refused post-commit check leaves the commit in place: {}",
            String::from_utf8_lossy(&committed.stderr)
        );
        committed
    }

    /// Read the same post-commit observation again as JSON, under the refused run.
    fn reported_under_the_second_run(&self) -> serde_json::Value {
        json_output(&post_commit_drift_under_run(
            self.repository.path(),
            SECOND_RUN,
        ))
    }
}

/// A refused run is told what it wrote, not what the worktree's history happens to contain.
///
/// The incumbent committed `held.txt` — its own scope — before the second run existed. The
/// second run then commits `other.txt` and nothing else. Reading the incumbent's whole
/// `phase_start..HEAD` range as the second run's writes offers `held.txt` back as a path the
/// second run entered, which it never touched.
#[test]
fn a_refused_run_is_not_told_it_entered_what_the_incumbent_committed_itself() {
    let worktree = IncumbentThatCommittedItsOwnScope::stand_up();

    let committed = worktree.the_second_run_commits("other.txt", "outside every scope\n");
    let warning = String::from_utf8_lossy(&committed.stderr);
    let reported = worktree.reported_under_the_second_run();
    let holder_id = &worktree.holder_id;

    assert!(
        !warning.contains("held.txt"),
        "the refused run wrote other.txt alone, so no path of the incumbent's may be read back \
         to it as an entry: {warning}"
    );
    assert_ne!(
        reported["status"], "incursion",
        "a refused run that entered nobody's scopes has no incursion to report: {reported}"
    );
    assert!(
        incursion_effects(&reported).is_empty(),
        "the incumbent's own commit is not an incursion by the run that followed it: {reported}"
    );
    let entry = &reported["payload"]["data"]["widening"];
    assert_ne!(
        entry["status"], "post_write_incursion",
        "nothing the second run wrote entered a holder's scopes: {reported}"
    );
    assert!(
        !entry["paths"].to_string().contains("held.txt"),
        "the incumbent's own committed path is not one the second run entered: {reported}"
    );
    assert!(
        !reported["payload"]["data"]["widening"]["conflicts"]
            .to_string()
            .contains(holder_id.as_str()),
        "no holder blocked a path the second run wrote, so none may be named: {reported}"
    );
}

/// The other direction: a real entry is still reported.
///
/// Narrowing what a refused run is told it entered must not silence the report. The second run
/// commits `held.txt` itself, into the incumbent's scope, and that entry is reported on the same
/// invocation that reports the refusal, with the commit left in place.
#[test]
fn a_refused_run_that_committed_into_the_incumbents_scope_is_still_reported() {
    let worktree = IncumbentThatCommittedItsOwnScope::stand_up();

    let committed = worktree.the_second_run_commits("held.txt", "the second run's own work\n");
    let warning = String::from_utf8_lossy(&committed.stderr);
    let reported = worktree.reported_under_the_second_run();
    let holder_id = &worktree.holder_id;

    assert_eq!(
        git_stdout(worktree.repository.path(), &["log", "-1", "--format=%s"]),
        "the second run commits",
        "a reported entry never removes the commit"
    );
    assert!(
        warning.contains(&format!(
            "Post-write detection found changed paths held.txt inside foreign reservations \
             {holder_id}."
        )),
        "the refused run's own entry into the incumbent's scope is reported by path and by \
         holder: {warning}"
    );
    assert_eq!(
        reported["status"], "incursion",
        "the entry the refused run really made is the report's headline: {reported}"
    );
    let entry = &reported["payload"]["data"]["widening"];
    assert_eq!(entry["status"], "post_write_incursion");
    assert_eq!(entry["paths"], serde_json::json!(["held.txt"]));
    assert_eq!(entry["conflicts"][0]["reservation_id"], holder_id.as_str());
    assert_eq!(
        reported["blocked_by"],
        serde_json::json!([holder_id]),
        "the reported incursion names the incumbent as the blocking reservation: {reported}"
    );
    assert_eq!(
        reported["payload"]["data"]["scope_acquisition"]["status"], "refused_to_second_run",
        "one report states both the entry it observed and the berth it was refused: {reported}"
    );
}

/// A merge at `HEAD` contributes its own resolution to refused-run attribution.
///
/// Both parents add `held.txt` differently, and the merge resolves it to a third value. That
/// makes the merge commit itself -- rather than either parent's history -- the source of the
/// entry into the incumbent's scope.
#[test]
fn a_refused_merge_head_reports_the_path_it_introduced() {
    let repository = initialized_repository();
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);

    git(repository.path(), &["switch", "--quiet", "-c", "incoming"]);
    fs::write(repository.path().join("held.txt"), "incoming parent\n")
        .expect("incoming parent path should write");
    git(repository.path(), &["add", "held.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "incoming parent",
        ],
    );
    git(repository.path(), &["switch", "--quiet", "main"]);
    fs::write(repository.path().join("held.txt"), "main parent\n")
        .expect("main parent path should write");
    git(repository.path(), &["add", "held.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "main parent",
        ],
    );

    let conflicted = git_output(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-ff",
            "incoming",
        ],
    );
    assert!(
        !conflicted.status.success(),
        "the fixture should require a merge resolution"
    );
    fs::write(repository.path().join("held.txt"), "merge resolution\n")
        .expect("merge resolution should write");
    git(repository.path(), &["add", "held.txt"]);
    let committed = git_output_with_environment(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "resolve held path",
        ],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    assert!(committed.status.success(), "merge commit should stand");
    assert_eq!(
        git_stdout(
            repository.path(),
            &["rev-list", "--parents", "-n", "1", "HEAD"]
        )
        .split_whitespace()
        .count(),
        3,
        "HEAD should be a two-parent merge commit"
    );

    assert_refused_head_entry(
        &post_commit_drift_under_run(repository.path(), SECOND_RUN),
        &holder_id,
    );
}

/// A parentless `HEAD` contributes its tree to refused-run attribution.
#[test]
fn a_refused_root_head_reports_the_path_it_introduced() {
    let repository = tempdir().expect("root-commit repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    fs::write(repository.path().join("held.txt"), "root commit entry\n")
        .expect("root path should write");
    git(repository.path(), &["add", "held.txt"]);
    let committed = git_output_with_environment(
        repository.path(),
        &["commit", "--quiet", "-m", "root entry"],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    assert!(committed.status.success(), "root commit should stand");
    assert_eq!(
        git_stdout(
            repository.path(),
            &["rev-list", "--parents", "-n", "1", "HEAD"]
        )
        .split_whitespace()
        .count(),
        1,
        "HEAD should be parentless"
    );
    assert!(
        run_berth(repository.path(), &["init", "--json"])
            .status
            .success()
    );
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);

    assert_refused_head_entry(
        &post_commit_drift_under_run(repository.path(), SECOND_RUN),
        &holder_id,
    );
}

/// Assert a refused invocation reports `HEAD` entering the incumbent's held path.
fn assert_refused_head_entry(reported: &Output, holder_id: &str) {
    assert_eq!(
        reported.status.code(),
        Some(1),
        "a real entry is a blocking drift result, not a plain usage rejection: {}",
        String::from_utf8_lossy(&reported.stdout)
    );
    let envelope = json_output(reported);
    let entry = &envelope["payload"]["data"]["widening"];
    assert_eq!(envelope["status"], "incursion", "{envelope}");
    assert_eq!(entry["status"], "post_write_incursion", "{envelope}");
    assert_eq!(
        entry["paths"],
        serde_json::json!(["held.txt"]),
        "{envelope}"
    );
    assert_eq!(
        entry["conflicts"][0]["reservation_id"], holder_id,
        "{envelope}"
    );
    assert_eq!(
        envelope["payload"]["data"]["scope_acquisition"]["rejection"]["incumbent_reservation_id"],
        holder_id,
        "{envelope}"
    );
    assert_eq!(
        envelope["blocked_by"],
        serde_json::json!([holder_id]),
        "the reported incursion names the incumbent as its blocker: {envelope}"
    );
    assert_eq!(
        envelope["payload"]["data"]["scope_acquisition"]["status"], "refused_to_second_run",
        "{envelope}"
    );
}

/// A refused run that entered nothing is told it was refused, not that the check failed.
///
/// The incumbent claims a path and commits nothing, so nothing in this worktree's history can
/// be read back to the second run as an entry however the range is sliced. The second run then
/// commits an unclaimed path. What is left is a refusal carrying no drift effect at all, which
/// is the one standing of the three that no earlier test reaches.
///
/// A refusal is not a broken check: the observation ran, found nothing outside anyone's
/// coverage, and recorded what it saw. Telling the committer the check could not be completed
/// contradicts the same message's own account of what it did, and sending them to run the check
/// by hand sends them to the same refusal.
#[test]
fn a_refusal_with_nothing_entered_is_not_reported_as_a_failed_check() {
    let repository = initialized_repository();
    let holder_id = claim(repository.path(), "file:mine.txt", FIRST_RUN);
    fs::write(repository.path().join("free.txt"), "unclaimed work\n")
        .expect("the second run's path should write");
    git(repository.path(), &["add", "free.txt"]);

    let committed = git_output_with_environment(
        repository.path(),
        &["commit", "-m", "write an unclaimed path"],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    let warning = String::from_utf8_lossy(&committed.stderr);

    assert!(
        committed.status.success(),
        "a refused post-commit check leaves the commit in place: {warning}"
    );
    assert!(
        warning.contains("already holds active reservation"),
        "a second run in the incumbent's worktree is refused by name even when it entered \
         nothing: {warning}"
    );
    assert!(warning.contains(&holder_id));
    assert!(warning.contains(SECOND_RUN));
    assert!(
        !warning.contains("could not complete the post-commit drift check"),
        "the check completed and the refusal withheld acquisition alone, so the committer is \
         not told the check failed: {warning}"
    );
    assert!(
        !warning.contains("cargo-berth drift --full"),
        "rerunning the check by hand under the same run earns the same refusal, so it is not \
         offered as the remedy: {warning}"
    );
}

/// An untracked file created by `init` does not change direct drift's occupancy answer.
#[test]
fn an_init_untracked_path_does_not_hide_direct_post_commit_refusal() {
    let repository = repository_with_uncommitted_berth_configuration();
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    let claims_before = claim_event_count(repository.path());

    let refused = post_commit_drift_under_run(repository.path(), SECOND_RUN);
    let envelope = json_output(&refused);

    assert_eq!(refused.status.code(), Some(5), "{envelope}");
    assert_eq!(envelope["status"], "invalid_input", "{envelope}");
    assert_eq!(
        envelope["payload"]["data"]["scope_acquisition"]["status"], "refused_to_second_run",
        "{envelope}"
    );
    assert_eq!(
        envelope["payload"]["data"]["scope_acquisition"]["rejection"]["incumbent_reservation_id"],
        holder_id,
        "{envelope}"
    );
    assert_eq!(
        envelope["blocked_by"],
        serde_json::json!([]),
        "a refusal with no incursion has no blocking drift holder: {envelope}"
    );
    assert_eq!(
        claim_event_count(repository.path()),
        claims_before,
        "direct refusal must not first-touch the init-created path"
    );
}

/// The installed post-commit hook preserves the second run's presented identity.
///
/// This is the smoke fixture's two-variable combination: a real commit reaches the installed
/// hook while `init`'s configuration remains untracked. The hook leaves the commit in place,
/// but its engine invocation still refuses acquisition and journals no first-touch claim.
#[test]
fn installed_hook_refuses_a_second_run_with_the_init_path_still_untracked() {
    let repository = repository_with_uncommitted_berth_configuration();
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    let claims_before = claim_event_count(repository.path());
    fs::write(repository.path().join("free.txt"), "second run work\n")
        .expect("unreserved commit path should write");
    git(repository.path(), &["add", "free.txt"]);

    let committed = git_output_with_environment(
        repository.path(),
        &[
            "commit",
            "--quiet",
            "-m",
            "second run through installed hook",
        ],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    let warning = String::from_utf8_lossy(&committed.stderr);

    assert!(
        committed.status.success(),
        "the hook must leave the commit in place: {warning}"
    );
    assert!(
        warning.contains("already holds active reservation"),
        "{warning}"
    );
    assert!(warning.contains(&holder_id), "{warning}");
    assert!(warning.contains(FIRST_RUN), "{warning}");
    assert!(warning.contains(SECOND_RUN), "{warning}");
    assert_eq!(
        claim_event_count(repository.path()),
        claims_before,
        "the installed hook must not first-touch either unreserved path for the refused run"
    );
}

/// No summary over a refusal may say the footprint grew, because a refusal withholds widening.
///
/// The incumbent's range carries an entry into another worktree's holder, so the report's
/// results carry an incursion effect and the presentation is decided by live incursion state.
/// The refusal is appended to the widening detail there, under a summary that announces a
/// widening that by construction did not happen. Front ends render `presentation`, so this is
/// asserted on the blocks rather than on the message text.
#[test]
fn a_refusal_is_never_summarized_as_a_widened_footprint() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "holds-shared");
    claim(&foreign_root, "tree:shared", THIRD_RUN);
    claim(repository.path(), "file:held.txt", FIRST_RUN);
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(
        repository.path().join("shared/s.txt"),
        "entered the other worktree\n",
    )
    .expect("entered path should write");
    git(repository.path(), &["add", "shared/s.txt"]);
    let committed = git_output_with_environment(
        repository.path(),
        &["commit", "-m", "enter the other worktree"],
        RUN_ENVIRONMENT,
        SECOND_RUN,
    );
    assert!(
        committed.status.success(),
        "a refused post-commit check leaves the commit in place: {}",
        String::from_utf8_lossy(&committed.stderr)
    );

    let reported = json_output(&post_commit_drift_under_run(repository.path(), SECOND_RUN));
    let blocks = reported["presentation"]["blocks"]
        .as_array()
        .expect("the refused run's report should render blocks")
        .clone();
    let refusals = blocks
        .iter()
        .filter(|block| {
            block["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("already holds active reservation"))
        })
        .collect::<Vec<_>>();

    assert!(
        !refusals.is_empty(),
        "the refusal should reach the front end as rendered detail: {reported}"
    );
    for block in refusals {
        assert!(
            !block["summary"]
                .as_str()
                .is_some_and(|summary| summary.contains("widened")),
            "a refusal takes and widens nothing, so no summary carrying one may claim the \
             footprint grew: {block}"
        );
    }
}

/// Refusing a berth here says nothing about a holder standing in another worktree.
///
/// The occupancy rule governs what a second run may take in the worktree it stands in.
/// Whether a commit entered another worktree's scopes is a different question, and it is
/// answered the same way whether or not the committing run was refused a berth of its own.
#[test]
fn a_refused_second_run_still_reports_a_foreign_worktrees_holder() {
    let refused = cross_worktree_entry(CommittingRun::ASecondPresentedRun);
    let unrefused = cross_worktree_entry(CommittingRun::TheIncumbent);

    assert!(
        unrefused.names_the_foreign_holder(),
        "the unrefused commit must record the incursion this test compares against: {:?}",
        unrefused.incursions
    );
    assert!(
        refused.names_the_foreign_holder(),
        "a refusal in this worktree must not silence reporting about another worktree: {:?}",
        refused.incursions
    );
}

/// The occupancy answer is the same asked before the mutation lock and asked under it.
///
/// A second run whose post-commit drift starts while the worktree is still unoccupied reads
/// an empty answer before the lock. Acquisition happens under the lock, so the question is
/// asked again there, and the incumbent that arrived meanwhile refuses it.
///
/// The interleaving is staged rather than raced. The ledger is rolled back to its unoccupied
/// state, the second run is held at the mutation lock — which it reports through
/// `MUTATION_LOCK_READY_ENVIRONMENT`, so its pre-lock read is known to have already
/// happened — and the incumbent's claim is restored before the lock is released.
#[test]
fn a_second_run_that_read_an_unoccupied_worktree_is_refused_under_the_lock() {
    let repository = initialized_repository();
    let incumbent_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    let occupied = LedgerSnapshot::capture(repository.path());
    stage_an_unoccupied_ledger(repository.path());
    fs::write(repository.path().join("own.txt"), "second run work\n")
        .expect("second run path should write");
    let lock_file = File::options()
        .read(true)
        .write(true)
        .open(repository.path().join(LOCK_PATH))
        .expect("mutation lock should open");
    lock_file.lock().expect("mutation lock should lock");
    let signals = tempdir().expect("lock signal directory should exist");
    let waiting_path = signals.path().join("waiting-at-the-mutation-lock");

    let second_run = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["drift", "--full", "--json"])
        .current_dir(repository.path())
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .env(RUN_ENVIRONMENT, SECOND_RUN)
        .env(MUTATION_LOCK_READY_ENVIRONMENT, &waiting_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the second run's post-commit drift should start");
    wait_until_held_at_the_mutation_lock(&waiting_path);
    occupied.restore(repository.path());
    std::mem::drop(lock_file);
    let refused = second_run
        .wait_with_output()
        .expect("the second run's post-commit drift should finish");
    let reported = format!(
        "{}{}",
        String::from_utf8_lossy(&refused.stdout),
        String::from_utf8_lossy(&refused.stderr)
    );

    assert_eq!(
        claim_event_count(repository.path()),
        1,
        "an incumbent that arrived under the lock must still refuse the acquisition: {reported}"
    );
    assert!(
        reported.contains(&incumbent_id),
        "the refusal should name the incumbent the locked answer found: {reported}"
    );

    let unoccupied_repository = initialized_repository();
    fs::write(
        unoccupied_repository.path().join("own.txt"),
        "second run work\n",
    )
    .expect("second run path should write");
    let admitted = post_commit_drift_under_run(unoccupied_repository.path(), SECOND_RUN);
    assert_eq!(
        claim_event_count(unoccupied_repository.path()),
        1,
        "an unoccupied worktree admits the first touch the incumbent must refuse: {}",
        String::from_utf8_lossy(&admitted.stdout)
    );
}

#[test]
fn markerless_post_commit_reports_every_incursion_without_ambiguous_widens() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "foreign");
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    let foreign_id = claim(&foreign_root, "tree:shared", SECOND_RUN);
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(
        repository.path().join("shared/entered.txt"),
        "foreign scope\n",
    )
    .expect("foreign path should write");
    fs::write(repository.path().join("outside.txt"), "outside both runs\n")
        .expect("outside path should write");
    git(
        repository.path(),
        &["add", "shared/entered.txt", "outside.txt"],
    );

    let markerless = text_post_commit_drift(repository.path(), &[]);
    assert_eq!(markerless.status.code(), Some(1));
    let markerless_warning = String::from_utf8_lossy(&markerless.stderr);
    assert!(markerless_warning.contains("no coordination run was identified"));
    assert!(markerless_warning.contains("CARGO_BERTH_RUN"));

    let committed = git_output(repository.path(), &["commit", "-m", "markerless widening"]);
    let warning = String::from_utf8_lossy(&committed.stderr);

    assert!(committed.status.success());
    assert!(warning.contains(&first_id));
    assert!(warning.contains(&second_id));
    assert!(warning.contains(&foreign_id));
    assert!(warning.contains("no coordination run was identified"));
    assert!(warning.contains("CARGO_BERTH_RUN"));
    let journal_events = journal_events(repository.path());
    let incursion_events = journal_events
        .iter()
        .filter(|event| event["op"] == "incursion")
        .collect::<Vec<_>>();
    assert_eq!(incursion_events.len(), 2);
    assert!(incursion_events.iter().any(|event| {
        event["reservation_id"] == first_id
            && event["foreign_reservation_ids"] == serde_json::json!([foreign_id])
    }));
    assert!(incursion_events.iter().any(|event| {
        event["reservation_id"] == second_id
            && event["foreign_reservation_ids"] == serde_json::json!([foreign_id])
    }));
    assert!(!journal_events.iter().any(|event| event["op"] == "widen"));
}

#[test]
fn post_commit_attribution_candidates_belong_to_the_identified_run() {
    let repository = initialized_repository();
    let (_other_run_directory, other_run_root) = foreign_worktree(&repository, "other-run");
    let first_id = claim(&other_run_root, "file:first.txt", SECOND_RUN);
    let second_id = claim(&other_run_root, "file:second.txt", SECOND_RUN);
    let selected_id = claim(repository.path(), "file:selected.txt", FIRST_RUN);
    fs::write(
        repository.path().join("outside.txt"),
        "outside every scope\n",
    )
    .expect("outside path should write");

    let widened = text_post_commit_drift(repository.path(), &[]);

    assert!(widened.status.success());
    assert!(String::from_utf8_lossy(&widened.stderr).contains(&selected_id));
    let journal = journal_text(repository.path());
    assert!(!journal.contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{first_id}\""
    )));
    assert!(!journal.contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{second_id}\""
    )));
}

#[test]
fn post_commit_widens_the_only_active_reservation() {
    let repository = initialized_repository();
    let reservation_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    fs::write(repository.path().join("outside.txt"), "outside\n")
        .expect("outside path should write");

    let widened = post_commit_drift(repository.path(), &[]);

    assert!(widened.status.success());
    let widen_events = journal_events(repository.path())
        .into_iter()
        .filter(|event| event["op"] == "widen")
        .collect::<Vec<_>>();
    assert_eq!(widen_events.len(), 1);
    assert_eq!(widen_events[0]["reservation_id"], reservation_id);
}

#[test]
fn json_post_commit_reports_every_active_reservation_without_warning_rendering() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::write(
        repository.path().join("outside.txt"),
        "outside both scopes\n",
    )
    .expect("outside path should write");

    // One worktree holds one run, so two live reservations are two candidates: the
    // attribution is named here so this test reports on rendering, not on ambiguity.
    let widened = post_commit_drift(repository.path(), &["--reservation", &first_id]);
    let envelope = json_output(&widened);

    assert!(widened.status.success());
    assert_eq!(envelope["status"], "widened");
    assert_eq!(envelope["exit_code"], 0);
    assert!(widened.stderr.is_empty());
    let results = envelope["payload"]["data"]["results"]
        .as_array()
        .expect("drift results should be an array");
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .any(|result| result["reservation_id"] == first_id)
    );
    assert!(
        results
            .iter()
            .any(|result| result["reservation_id"] == second_id)
    );
}

#[test]
fn explicit_post_commit_attribution_widens_only_the_named_reservation() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::write(repository.path().join("outside.txt"), "outside\n")
        .expect("outside path should write");

    let widened = post_commit_drift(repository.path(), &["--reservation", &first_id]);

    assert!(widened.status.success());
    let widen_events = journal_events(repository.path())
        .into_iter()
        .filter(|event| event["op"] == "widen")
        .collect::<Vec<_>>();
    assert_eq!(widen_events.len(), 1);
    assert_eq!(widen_events[0]["reservation_id"], first_id);
    assert!(!journal_text(repository.path()).contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{second_id}\""
    )));
}

#[test]
fn ambiguous_post_commit_keeps_changes_for_an_explicit_cheap_retry() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::write(
        repository.path().join("outside.txt"),
        "outside both scopes\n",
    )
    .expect("outside path should write");

    let ambiguous = text_post_commit_drift(repository.path(), &[]);
    assert_eq!(ambiguous.status.code(), Some(1));
    let warning = String::from_utf8_lossy(&ambiguous.stderr);
    assert!(warning.contains(&first_id));
    assert!(warning.contains(&second_id));
    assert!(warning.contains("drift --reservation <id>"));

    let widened = drift(repository.path(), &["--reservation", &first_id]);

    assert!(widened.status.success());
    assert_eq!(
        json_output(&widened)["payload"]["data"]["comparison"],
        "full_phase_start_fallback"
    );
    assert!(journal_text(repository.path()).contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{first_id}\""
    )));
    assert!(!journal_text(repository.path()).contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{second_id}\""
    )));
}

#[test]
fn session_mapping_attributes_post_commit_widening_with_two_active_reservations() {
    let repository = initialized_repository();
    let session_id = "drift-attribution-session";
    let first_id = claim_with_session(repository.path(), "file:first.txt", FIRST_RUN, session_id);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::remove_file(repository.path().join(MARKER_PATH))
        .expect("coordination marker should remove");
    fs::write(repository.path().join("outside.txt"), "outside\n")
        .expect("outside path should write");

    let widened = post_commit_drift_with_session(repository.path(), session_id);

    assert!(widened.status.success());
    let widen_events = journal_events(repository.path())
        .into_iter()
        .filter(|event| event["op"] == "widen")
        .collect::<Vec<_>>();
    assert_eq!(widen_events.len(), 1);
    assert_eq!(widen_events[0]["reservation_id"], first_id);
    assert!(!journal_text(repository.path()).contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{second_id}\""
    )));
}

#[test]
fn drift_widen_records_existing_answer_coverage_for_a_scope_bound_answer() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder_id = claim(repository.path(), "tree:shared", FIRST_RUN);
    let subject_id = claim_with_override(
        &second_root,
        "file:shared/approved.txt",
        SECOND_RUN,
        &holder_id,
    );
    fs::write(second_root.join("outside.txt"), "new scope\n").expect("new scope path should write");

    let widened = run_berth_with_run(
        &second_root,
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );

    assert!(widened.status.success());
    let widen = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "widen" && event["reservation_id"] == subject_id)
        .expect("drift should append a widen for the answered reservation");
    assert_eq!(
        widen["authorization"]["kind"],
        "existing_answers_cover_every_overlap"
    );
    assert_eq!(
        widen["authorization"]["overlaps"][0]["reservation_id"],
        holder_id
    );
    assert_eq!(
        widen["authorization"]["overlaps"][0]["scopes"][0]["path"],
        "shared/approved.txt"
    );
}

/// Where the holder standing over a widening subject's scopes is, and in what state.
///
/// A widening asks the one foreignness question the pre-edit hook asks, so a same-worktree
/// holder is foreign only while another *presented* run holds it `Active`. These are the
/// three same-worktree standings that question declines --- released and awaiting
/// integration, claimed under an identity the engine issued for itself, and claimed before
/// provenance was recorded at all --- and the one standing it accepts.
#[derive(Clone, Copy, Debug)]
enum OverlappedHolderStanding {
    /// This worktree, another run, released and awaiting integration.
    HereAwaitingIntegration,
    /// This worktree, claimed by post-commit drift under an identity nobody presented.
    HereUnderAnEngineIssuedIdentity,
    /// This worktree, claimed by a build that recorded no identity provenance at all.
    HereFromBeforeProvenanceWasRecorded,
    /// Another worktree, held `Active` by a run that presented its identity.
    InAnotherWorktreeUnderAPresentedRun,
}

/// What a drift widening recorded about the holder its subject's scopes overlap.
#[derive(Debug, Eq, PartialEq)]
enum RecordedWidenAuthorization {
    /// The widening bound no foreign overlap, so no answer was required of it.
    NoOverlapAnswerRequired,
    /// Earlier answers cover the overlap, and the widen names every holder they cover.
    ExistingAnswersCover { holder_ids: Vec<String> },
    /// The overlap is foreign and unanswered, so no widen was recorded at all.
    RefusedPendingAnOverlapAnswer,
}

/// Whether the holder still blocked a foreign edit when the widening was decided.
///
/// A holder the projection has stopped treating as blocking is skipped before foreignness is
/// ever asked, so a permissive arm of this test would record nothing for a reason that has
/// nothing to do with the rule under test.
#[derive(Debug, Eq, PartialEq)]
enum HolderEditBlocking {
    BlocksAForeignEdit,
    DoesNotBlockAForeignEdit,
}

struct WideningOverAHolder {
    holder_id:       String,
    holder_blocking: HolderEditBlocking,
    authorization:   RecordedWidenAuthorization,
}

/// A widening asks an overlap answer only of a holder that is foreign to the widening run.
///
/// Widening once tested the run and the worktree by hand, with no lifecycle and no
/// provenance term, so it demanded an answer from a same-worktree holder the pre-edit hook
/// had already decided this identity may edit. It now asks
/// `Reservation::is_foreign_to_coordination_run_in_worktree`, the same predicate every other
/// site asks, which relaxes exactly three same-worktree standings: their overlaps stop
/// forcing an answer and stop appearing among the authorized overlaps the widen records.
///
/// The contrast is drawn on the worktree because the same-worktree `Active` and `Presented`
/// standing cannot reach a widening at all. Occupancy refuses that run's acquisition first,
/// and the drift transaction then selects no widening subject, so the widening path is never
/// entered and there is no authorization to compare.
#[test]
fn widening_asks_an_overlap_answer_only_of_a_holder_foreign_to_it() {
    for standing in [
        OverlappedHolderStanding::HereAwaitingIntegration,
        OverlappedHolderStanding::HereUnderAnEngineIssuedIdentity,
        OverlappedHolderStanding::HereFromBeforeProvenanceWasRecorded,
    ] {
        let widening = widening_over_an_overlapped_holder(standing);

        assert_eq!(
            widening.holder_blocking,
            HolderEditBlocking::BlocksAForeignEdit,
            "{standing:?} must still block a foreign edit, or this arm records nothing for the \
             wrong reason"
        );
        assert_eq!(
            widening.authorization,
            RecordedWidenAuthorization::NoOverlapAnswerRequired,
            "{standing:?} is not foreign to the widening run, so its overlap must neither \
             require an answer nor be recorded as an authorized one"
        );
    }

    let foreign = widening_over_an_overlapped_holder(
        OverlappedHolderStanding::InAnotherWorktreeUnderAPresentedRun,
    );

    assert_eq!(
        foreign.holder_blocking,
        HolderEditBlocking::BlocksAForeignEdit
    );
    assert_eq!(
        foreign.authorization,
        RecordedWidenAuthorization::ExistingAnswersCover {
            holder_ids: vec![foreign.holder_id.clone()],
        },
        "a presented run's active holder in another worktree still requires an answer, and the \
         widen still records it"
    );
}

/// Widen one subject whose scopes overlap a holder standing as `standing` describes.
///
/// Every arm uses the same shapes: the holder covers `shared/held.txt`, the subject covers
/// `tree:shared` around it, and the subject widens onto an unclaimed `outside.txt`. Only the
/// holder's worktree and state differ, so the recorded authorization differs for one reason.
fn widening_over_an_overlapped_holder(standing: OverlappedHolderStanding) -> WideningOverAHolder {
    let repository = initialized_repository();
    let (_probe_directory, probe_root) = foreign_worktree(&repository, "probe");
    let (_holder_directory, holder_root) = foreign_worktree(&repository, "holder");
    let (holder_id, subject_id) = match standing {
        OverlappedHolderStanding::HereAwaitingIntegration => {
            // The holder must carry unintegrated work, or releasing it leaves nothing to
            // integrate, its evidence reads as clear, and it stops blocking edits at all.
            git(
                repository.path(),
                &["switch", "--quiet", "-c", "holder-phase"],
            );
            fs::create_dir_all(repository.path().join("shared"))
                .expect("held scope directory should exist");
            fs::write(repository.path().join("shared/held.txt"), "holder work\n")
                .expect("held path should write");
            git(repository.path(), &["add", "shared/held.txt"]);
            git(
                repository.path(),
                &["commit", "--quiet", "-m", "holder work"],
            );
            let holder_id = claim(repository.path(), "file:shared/held.txt", FIRST_RUN);
            let checkpointed = run_berth(repository.path(), &["release", &holder_id, "--json"]);
            assert!(checkpointed.status.success());
            assert_eq!(
                json_output(&checkpointed)["payload"]["data"]["status"],
                "checkpointed"
            );
            (
                holder_id,
                claim(repository.path(), "tree:shared", SECOND_RUN),
            )
        },
        OverlappedHolderStanding::HereUnderAnEngineIssuedIdentity => {
            fs::create_dir_all(repository.path().join("shared"))
                .expect("held scope directory should exist");
            fs::write(repository.path().join("shared/held.txt"), "engine-issued\n")
                .expect("held path should write");
            let first_touched = post_commit_drift(repository.path(), &[]);
            assert!(first_touched.status.success());
            let holder_id = json_output(&first_touched)["payload"]["data"]["widening"]
                ["acquisition"]["reservation_id"]
                .as_str()
                .expect("the post-write first touch should return a reservation id")
                .to_owned();
            (
                holder_id,
                claim(repository.path(), "tree:shared", SECOND_RUN),
            )
        },
        OverlappedHolderStanding::HereFromBeforeProvenanceWasRecorded => {
            let subject_id = claim(repository.path(), "tree:shared", SECOND_RUN);
            (
                append_claim_recording_no_provenance(
                    repository.path(),
                    FIRST_RUN,
                    "shared/held.txt",
                ),
                subject_id,
            )
        },
        OverlappedHolderStanding::InAnotherWorktreeUnderAPresentedRun => {
            let holder_id = claim(&holder_root, "file:shared/held.txt", FIRST_RUN);
            let subject_id =
                claim_with_override(repository.path(), "tree:shared", SECOND_RUN, &holder_id);
            (holder_id, subject_id)
        },
    };
    let holder_blocking = holder_edit_blocking(&probe_root, "file:shared/held.txt", &holder_id);
    fs::write(repository.path().join("outside.txt"), "new scope\n")
        .expect("new scope path should write");

    let widened = run_berth_with_run(
        repository.path(),
        &["drift", "--full", "--reservation", &subject_id, "--json"],
        SECOND_RUN,
    );

    assert!(
        widened.status.success(),
        "drift failed for {standing:?}: {}{}",
        String::from_utf8_lossy(&widened.stdout),
        String::from_utf8_lossy(&widened.stderr)
    );
    let authorization = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "widen" && event["reservation_id"] == subject_id)
        .map_or(
            RecordedWidenAuthorization::RefusedPendingAnOverlapAnswer,
            |widen| recorded_widen_authorization(&widen["authorization"]),
        );
    WideningOverAHolder {
        holder_id,
        holder_blocking,
        authorization,
    }
}

/// Read one widen event's recorded authorization as the answer question it settles.
fn recorded_widen_authorization(authorization: &serde_json::Value) -> RecordedWidenAuthorization {
    let kind = authorization["kind"]
        .as_str()
        .expect("a widen should record an authorization kind");
    if kind == "no_conflict" {
        return RecordedWidenAuthorization::NoOverlapAnswerRequired;
    }
    assert_eq!(
        kind, "existing_answers_cover_every_overlap",
        "this fixture answers overlaps only by existing coverage: {authorization}"
    );
    RecordedWidenAuthorization::ExistingAnswersCover {
        holder_ids: authorization["overlaps"]
            .as_array()
            .expect("covered overlaps should be an array")
            .iter()
            .map(|overlap| {
                overlap["reservation_id"]
                    .as_str()
                    .expect("an authorized overlap should name its holder")
                    .to_owned()
            })
            .collect(),
    }
}

/// Ask a third worktree whether the named holder still refuses it the holder's own path.
fn holder_edit_blocking(probe_root: &Path, scope: &str, holder_id: &str) -> HolderEditBlocking {
    let probed = run_berth_with_run(probe_root, &["check", scope, "--json"], THIRD_RUN);
    let blocked_by_the_holder = json_output(&probed)["blocked_by"]
        .as_array()
        .is_some_and(|holders| holders.iter().any(|holder| holder == holder_id));
    if probed.status.code() == Some(1) && blocked_by_the_holder {
        HolderEditBlocking::BlocksAForeignEdit
    } else {
        HolderEditBlocking::DoesNotBlockAForeignEdit
    }
}

/// Append the claim a build that predates provenance recording left behind.
///
/// No verb can write one any more --- the field is recorded at every claim site --- so the
/// record an upgraded repository replays is appended directly. It is copied from a real
/// claim in the same journal so every other field stays exactly what the engine wrote, and
/// only the reservation, its scope, its run, and the absent provenance differ.
fn append_claim_recording_no_provenance(
    repository_root: &Path,
    run: &str,
    scope_path: &str,
) -> String {
    let events = journal_events(repository_root);
    let previous = events
        .last()
        .expect("the fixture should have written one event")
        .clone();
    let mut event = events
        .iter()
        .find(|event| event["op"] == "claim")
        .expect("the fixture should have written one claim")
        .as_object()
        .expect("a journal event should be an object")
        .clone();
    let reservation_id = uuid::Uuid::now_v7().to_string();
    event.insert(
        "event_id".to_owned(),
        serde_json::json!(uuid::Uuid::now_v7().to_string()),
    );
    event.insert(
        "reservation_id".to_owned(),
        serde_json::json!(reservation_id),
    );
    event.insert(
        "scopes".to_owned(),
        serde_json::json!([{"path": scope_path, "kind": "file"}]),
    );
    event.remove("coordination_identity_provenance");
    event.insert(
        "actor".to_owned(),
        serde_json::json!({
            "repository": previous["actor"]["repository"],
            "worktree": previous["actor"]["worktree"],
            "run": run,
        }),
    );
    event.insert("at".to_owned(), previous["at"].clone());
    event.insert(
        "projection_generation".to_owned(),
        serde_json::json!(
            previous["projection_generation"]
                .as_u64()
                .expect("a projection generation should be numeric")
                + 1
        ),
    );
    let mut journal = OpenOptions::new()
        .append(true)
        .open(repository_root.join(JOURNAL_PATH))
        .expect("the journal should open for the provenance-free claim");
    serde_json::to_writer(&mut journal, &serde_json::Value::Object(event))
        .expect("the provenance-free claim should serialize");
    journal
        .write_all(b"\n")
        .expect("the provenance-free claim terminator should write");
    reservation_id
}

#[test]
fn unchanged_full_drift_carries_reconciliation_alerts() {
    let repository = initialized_repository();
    let subject_id = claim(repository.path(), "file:subject.txt", FIRST_RUN);
    let worktrees = tempdir().expect("worktree parent should exist");
    let orphan_root = add_worktree(repository.path(), worktrees.path(), "orphan-alert");
    fs::write(orphan_root.join("orphan.txt"), "orphan work\n").expect("orphan path should write");
    git(&orphan_root, &["add", "orphan.txt"]);
    git(&orphan_root, &["commit", "--quiet", "-m", "orphan work"]);
    let orphan_id = claim(&orphan_root, "file:orphan.txt", SECOND_RUN);
    assert!(
        run_berth(&orphan_root, &["release", &orphan_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&orphan_root).expect("orphan worktree should remove");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let unchanged = drift(repository.path(), &["--full", "--reservation", &subject_id]);
    let envelope = json_output(&unchanged);

    assert!(unchanged.status.success());
    assert_eq!(envelope["status"], "clear");
    assert_eq!(
        envelope["payload"]["alerts"][0]["kind"],
        "orphaned_outstanding"
    );
    assert_eq!(
        envelope["payload"]["alerts"][0]["data"]["reservation_id"],
        orphan_id
    );
}

#[test]
fn first_drift_after_a_trunk_rewrite_reports_lost_released_evidence() {
    let repository = initialized_repository();
    let observer_id = claim(repository.path(), "file:observer.txt", FIRST_RUN);
    let released_id = claim(repository.path(), "file:released.txt", FIRST_RUN);
    fs::write(repository.path().join("released.txt"), "released work\n")
        .expect("released work should write");
    git(repository.path(), &["add", "released.txt"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "released work",
        ],
    );
    for (expected_status, expected_fact_status) in [
        ("outstanding", "checkpointed"),
        ("integrated", "evidence_revalidated"),
        ("integrated", "released"),
    ] {
        let release = run_berth(repository.path(), &["release", &released_id, "--json"]);
        assert!(release.status.success());
        assert_eq!(json_output(&release)["status"], expected_status);
        assert_eq!(
            json_output(&release)["payload"]["data"]["status"],
            expected_fact_status
        );
    }
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            "--quiet",
            "HEAD^",
        ],
    );

    let first_detection = drift(
        repository.path(),
        &["--full", "--reservation", &observer_id],
    );
    let envelope = json_output(&first_detection);

    assert!(first_detection.status.success());
    let alert = envelope["payload"]["alerts"]
        .as_array()
        .and_then(|alerts| {
            alerts.iter().find(|alert| {
                alert["kind"] == "lost_integration_evidence"
                    && alert["data"]["reservation_id"] == released_id
            })
        })
        .expect("the first drift reconciliation should report the lost evidence");
    assert_eq!(
        alert["data"]["evidence_status"]["status"],
        "trunk_rewritten"
    );
    assert_eq!(alert["data"]["recovery"]["kind"], "verify_resolved_trunk");
}

#[test]
fn cheap_delta_parses_rename_with_worktree_modification() {
    let repository = initialized_repository();
    fs::create_dir_all(repository.path().join("files")).expect("files directory should exist");
    fs::write(repository.path().join("files/original.txt"), "original\n")
        .expect("original path should write");
    git(repository.path(), &["add", "files/original.txt"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "original file"],
    );
    let reservation_id = claim(repository.path(), "tree:files", FIRST_RUN);
    let baseline = drift(
        repository.path(),
        &["--full", "--reservation", &reservation_id],
    );
    assert!(baseline.status.success());
    git(
        repository.path(),
        &["mv", "files/original.txt", "files/renamed.txt"],
    );
    fs::write(repository.path().join("files/renamed.txt"), "modified\n")
        .expect("renamed path should modify");

    let delta = drift(repository.path(), &["--reservation", &reservation_id]);

    assert!(
        delta.status.success(),
        "cheap drift failed: {}",
        String::from_utf8_lossy(&delta.stdout)
    );
    assert_eq!(json_output(&delta)["status"], "clear");
}

#[test]
fn cheap_and_full_fingerprints_use_their_exact_command_budgets() {
    let repository = initialized_repository();
    let reservation_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);

    let full = traced_drift(
        repository.path(),
        &["--full", "--reservation", &reservation_id],
    );
    assert!(full.output.status.success());
    assert_eq!(
        full.fingerprint_commands(),
        vec!["diff-tree", "status"],
        "the phase range and HEAD's own commit ride one batched read, and the working tree is \
         the other"
    );
    assert_batched_full_attribution_commands(&full.commands());

    let cheap = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(cheap.output.status.success());
    assert_eq!(cheap.fingerprint_commands(), vec!["status"]);
    assert_no_phase_ancestry_or_metadata_command(&cheap.commands());
    let mut cheap_commands = cheap.commands();
    cheap_commands.sort_unstable();
    assert_eq!(
        cheap_commands,
        vec!["cat-file", "status", "worktree"],
        "the cheap PostToolUse engine path must reuse its discovered ledger and administrative directory",
    );

    fs::remove_file(fingerprint_cache(repository.path())).expect("fingerprint cache should delete");
    let missing_cache = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(missing_cache.output.status.success());
    assert_eq!(
        missing_cache.fingerprint_commands(),
        vec!["diff-tree", "status"]
    );
    assert_eq!(
        json_output(&missing_cache.output)["payload"]["data"]["comparison"],
        "full_phase_start_fallback"
    );
    assert_batched_full_attribution_commands(&missing_cache.commands());

    fs::write(fingerprint_cache(repository.path()), "not json")
        .expect("corrupt fingerprint should write");
    let corrupt_cache = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(corrupt_cache.output.status.success());
    assert_eq!(
        corrupt_cache.fingerprint_commands(),
        vec!["diff-tree", "status"]
    );
    assert_batched_full_attribution_commands(&corrupt_cache.commands());
}

#[test]
fn reservation_selection_requires_an_explicit_choice_only_when_ambiguous() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let single = drift(repository.path(), &["--full"]);
    assert!(single.status.success());
    assert_eq!(
        json_output(&single)["reservations"],
        serde_json::json!([first_id])
    );

    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    let ambiguous = drift(repository.path(), &["--full"]);
    let ambiguous_envelope = json_output(&ambiguous);
    assert_eq!(ambiguous.status.code(), Some(5));
    assert_eq!(ambiguous_envelope["status"], "invalid_input");
    let message = ambiguous_envelope["message"]
        .as_str()
        .expect("usage error should contain a message");
    assert!(message.contains(&first_id));
    assert!(message.contains(&second_id));
    assert!(message.contains("--reservation"));

    let selected = drift(repository.path(), &["--full", "--reservation", &second_id]);
    assert!(selected.status.success());
    assert_eq!(
        json_output(&selected)["reservations"],
        serde_json::json!([second_id])
    );
}

#[test]
fn post_commit_is_managed_separately_and_warns_without_rejecting_commits() {
    let repository = initialized_repository();
    let initialized_again = run_berth(repository.path(), &["init", "--json"]);
    let hooks = json_output(&initialized_again)["payload"]["data"]["hooks"]
        .as_array()
        .expect("hook results should be an array")
        .clone();
    assert_eq!(hooks.len(), 2);
    assert_eq!(hooks[0]["name"], "reference-transaction");
    assert_eq!(hooks[1]["name"], "post-commit");
    assert_eq!(hooks[0]["activation"]["status"], "active");
    assert_eq!(hooks[1]["activation"]["status"], "active");
    let installed = fs::read_to_string(repository.path().join(POST_COMMIT_HOOK_PATH))
        .expect("post-commit hook should read");
    assert!(installed.contains("drift --full"));
    assert!(!installed.contains("cd "));
    assert!(run_berth(repository.path(), &["init"]).status.success());
    assert_eq!(
        fs::read_to_string(repository.path().join(POST_COMMIT_HOOK_PATH))
            .expect("idempotent post-commit hook should read"),
        installed
    );

    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "foreign");
    let subject_id = claim(repository.path(), "file:owned.txt", FIRST_RUN);
    let foreign_id = claim(&foreign_root, "tree:shared", SECOND_RUN);
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(
        repository.path().join("shared/committed.txt"),
        "foreign path\n",
    )
    .expect("foreign path should write");
    git(repository.path(), &["add", "shared/committed.txt"]);
    let committed = git_output(repository.path(), &["commit", "-m", "incursion"]);
    assert!(
        committed.status.success(),
        "commit should stand: {}",
        String::from_utf8_lossy(&committed.stderr)
    );
    let warning = String::from_utf8_lossy(&committed.stderr);
    assert!(warning.contains("Incursion"));
    assert!(warning.contains(&subject_id));
    assert!(warning.contains(&foreign_id));
    assert!(journal_text(repository.path()).contains("\"op\":\"incursion\""));
}

#[test]
fn post_commit_reports_ambiguous_attribution_and_honors_bypass_before_corruption() {
    let repository = initialized_repository();
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    fs::write(repository.path().join("outside.txt"), "outside\n")
        .expect("outside path should write");
    git(repository.path(), &["add", "outside.txt"]);
    let committed = git_output(repository.path(), &["commit", "-m", "outside both"]);
    assert!(committed.status.success());
    let warning = String::from_utf8_lossy(&committed.stderr);
    assert!(warning.contains(&first_id));
    assert!(warning.contains(&second_id));
    assert!(warning.contains("drift --reservation <id>"));
    assert!(!journal_text(repository.path()).contains("\"op\":\"widen\""));

    let manual = drift(repository.path(), &["--full"]);
    let manual_message = json_output(&manual)["message"]
        .as_str()
        .expect("manual ambiguity should have a message")
        .to_owned();
    assert_eq!(manual.status.code(), Some(5));
    assert!(manual_message.contains(&first_id));
    assert!(manual_message.contains(&second_id));

    fs::write(repository.path().join("bypassed.txt"), "bypass\n")
        .expect("bypass path should write");
    git(repository.path(), &["add", "bypassed.txt"]);
    fs::write(repository.path().join(JOURNAL_PATH), "{}\n").expect("corrupt journal should write");
    let bypassed = git_output_with_environment(
        repository.path(),
        &["commit", "-m", "bypassed corrupt ledger"],
        BYPASS_ENVIRONMENT,
        "1",
    );
    assert!(bypassed.status.success());
    assert!(!String::from_utf8_lossy(&bypassed.stderr).contains("could not check"));
}

#[test]
fn unmanaged_post_commit_is_preserved_while_the_trunk_gate_stays_active() {
    let repository = scratch_repository();
    fs::create_dir_all(repository.path().join(".git/hooks")).expect("hooks directory should exist");
    let unmanaged = "#!/bin/sh\nprintf 'unmanaged\\n'\n";
    fs::write(repository.path().join(POST_COMMIT_HOOK_PATH), unmanaged)
        .expect("unmanaged hook should write");
    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    let initialized_envelope = json_output(&initialized);
    let hooks = initialized_envelope["payload"]["data"]["hooks"]
        .as_array()
        .expect("hook results should be an array");
    assert_eq!(hooks[0]["name"], "reference-transaction");
    assert_eq!(hooks[0]["activation"]["status"], "active");
    assert_eq!(hooks[1]["name"], "post-commit");
    assert_eq!(hooks[1]["activation"]["status"], "inactive");
    assert_eq!(
        hooks[1]["activation"]["reason"]["kind"],
        "preserved_unmanaged"
    );
    assert_eq!(
        fs::read_to_string(repository.path().join(POST_COMMIT_HOOK_PATH))
            .expect("unmanaged hook should remain"),
        unmanaged
    );
}

#[test]
fn post_commit_is_silent_for_covered_work_and_uses_the_invoking_linked_worktree() {
    let repository = initialized_repository();
    fs::write(repository.path().join("covered.txt"), "base\n").expect("covered base should write");
    git(repository.path(), &["add", "covered.txt"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "covered base"],
    );
    let covered_id = claim(repository.path(), "file:covered.txt", FIRST_RUN);
    fs::write(repository.path().join("covered.txt"), "changed\n")
        .expect("covered work should write");
    git(repository.path(), &["add", "covered.txt"]);
    let covered_commit = git_output(repository.path(), &["commit", "-m", "covered work"]);
    assert!(covered_commit.status.success());
    assert!(covered_commit.stderr.is_empty());
    assert!(!journal_text(repository.path()).contains(&format!(
        "\"op\":\"widen\",\"reservation_id\":\"{covered_id}\""
    )));

    let worktrees = tempdir().expect("worktree parent should exist");
    let linked_root = add_worktree(repository.path(), worktrees.path(), "linked");
    let linked_id = claim(&linked_root, "file:linked-owned.txt", SECOND_RUN);
    fs::write(linked_root.join("linked-outside.txt"), "linked drift\n")
        .expect("linked drift should write");
    git(&linked_root, &["add", "linked-outside.txt"]);
    let linked_commit = git_output(&linked_root, &["commit", "-m", "linked drift"]);
    assert!(linked_commit.status.success());
    let warning = String::from_utf8_lossy(&linked_commit.stderr);
    assert!(warning.contains(&linked_id));
    assert!(warning.contains("linked-outside.txt"));
    assert!(git_stdout(repository.path(), &["status", "--porcelain"]).is_empty());
}

#[test]
fn post_commit_reports_corrupt_ledger_and_lock_deadline_without_removing_the_commit() {
    let corrupt_repository = initialized_repository();
    git(
        corrupt_repository.path(),
        &["switch", "--quiet", "-c", "feature"],
    );
    claim(corrupt_repository.path(), "file:owned.txt", FIRST_RUN);
    fs::write(
        corrupt_repository.path().join("corrupt-check.txt"),
        "commit\n",
    )
    .expect("corrupt-check path should write");
    git(corrupt_repository.path(), &["add", "corrupt-check.txt"]);
    fs::write(corrupt_repository.path().join(JOURNAL_PATH), "{}\n")
        .expect("corrupt journal should write");
    let corrupt_commit = git_output(
        corrupt_repository.path(),
        &["commit", "-m", "corrupt drift check"],
    );
    assert!(corrupt_commit.status.success());
    let corrupt_warning = String::from_utf8_lossy(&corrupt_commit.stderr);
    assert!(corrupt_warning.contains("ledger was unreadable"));
    assert!(corrupt_warning.contains("cargo-berth drift --full"));
    assert!(corrupt_warning.contains("commit remains in place"));

    let locked_repository = initialized_repository();
    git(
        locked_repository.path(),
        &["switch", "--quiet", "-c", "feature"],
    );
    claim(locked_repository.path(), "file:owned.txt", FIRST_RUN);
    fs::write(
        locked_repository.path().join("locked-check.txt"),
        "commit\n",
    )
    .expect("locked-check path should write");
    git(locked_repository.path(), &["add", "locked-check.txt"]);
    let lock =
        File::open(locked_repository.path().join(LOCK_PATH)).expect("mutation lock should open");
    lock.lock().expect("test should hold mutation lock");
    let locked_commit = git_output(
        locked_repository.path(),
        &["commit", "-m", "contended drift check"],
    );
    lock.unlock().expect("test should release mutation lock");
    assert!(locked_commit.status.success());
    let lock_warning = String::from_utf8_lossy(&locked_commit.stderr);
    assert!(lock_warning.contains("lock deadline was exhausted"));
    assert!(lock_warning.contains("cargo-berth drift --full"));
    assert!(lock_warning.contains("commit remains in place"));
}

struct TracedDrift {
    output:     Output,
    trace_path: PathBuf,
    _directory: TempDir,
}

struct DifferentialAttributionRepository {
    _repository_lifetime:       TempDir,
    _side_worktree_lifetime:    TempDir,
    _subject_worktree_lifetime: TempDir,
    trunk_root:                 PathBuf,
    subject_root:               PathBuf,
    phase_start:                String,
    subject_reservation_id:     String,
}

fn prepare_differential_attribution_repository() -> DifferentialAttributionRepository {
    let repository = initialized_repository();
    let trunk_root = repository.path().to_path_buf();
    fs::create_dir_all(trunk_root.join("held")).expect("held directory should exist");
    for (path, contents) in [
        ("conflict-driver.txt", "base\n"),
        ("held/deleted.txt", "delete me\n"),
        ("held/rename-old.txt", "rename me\n"),
    ] {
        fs::write(trunk_root.join(path), contents).expect("differential base path should write");
    }
    git(&trunk_root, &["add", "-A"]);
    git(
        &trunk_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "differential base",
        ],
    );
    let side_worktree_lifetime = tempdir().expect("side worktree parent should exist");
    let side_root = add_worktree(
        &trunk_root,
        side_worktree_lifetime.path(),
        "differential-side",
    );
    let (subject_worktree_lifetime, subject_root) =
        foreign_worktree(&repository, "differential-subject");
    claim(&trunk_root, "tree:held", FIRST_RUN);
    fs::write(subject_root.join("anchor-advance.txt"), "anchor advance\n")
        .expect("anchor advance should write");
    git(&subject_root, &["add", "anchor-advance.txt"]);
    git(
        &subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "advance past side branch point",
        ],
    );
    let phase_start = git_stdout(&subject_root, &["rev-parse", "HEAD"]);
    let subject_reservation_id = claim(&subject_root, "file:own.txt", SECOND_RUN);
    fs::write(side_root.join("held/merged.txt"), "merged side work\n")
        .expect("merged path should write");
    git(&side_root, &["add", "held/merged.txt"]);
    git(
        &side_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "side branch held path",
        ],
    );
    git(
        &subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--quiet",
            "--no-ff",
            "--no-edit",
            "differential-side",
        ],
    );
    DifferentialAttributionRepository {
        _repository_lifetime: repository,
        _side_worktree_lifetime: side_worktree_lifetime,
        _subject_worktree_lifetime: subject_worktree_lifetime,
        trunk_root,
        subject_root,
        phase_start,
        subject_reservation_id,
    }
}

fn commit_path_encoding_history(subject_root: &Path) {
    for (path, contents) in [
        ("held/tab\tname.txt", "tab path\n"),
        ("held/line\nname.txt", "newline path\n"),
        ("held/café.txt", "non-ASCII path\n"),
    ] {
        fs::write(subject_root.join(path), contents).expect("encoded path should write");
    }
    git(
        subject_root,
        &["mv", "held/rename-old.txt", "held/rename-new.txt"],
    );
    fs::remove_file(subject_root.join("held/deleted.txt")).expect("deleted path should remove");
    git(subject_root, &["add", "-A"]);
    git(
        subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "path encoding rename and deletion",
        ],
    );
}

fn commit_conflict_resolution_only_history(trunk_root: &Path, subject_root: &Path) {
    fs::write(trunk_root.join("conflict-driver.txt"), "main side\n")
        .expect("main conflict side should write");
    git(trunk_root, &["add", "conflict-driver.txt"]);
    git(
        trunk_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "main conflict side",
        ],
    );
    fs::write(subject_root.join("conflict-driver.txt"), "subject side\n")
        .expect("subject conflict side should write");
    git(subject_root, &["add", "conflict-driver.txt"]);
    git(
        subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "subject conflict side",
        ],
    );
    let conflicted = git_output(
        subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-ff",
            "--no-edit",
            "main",
        ],
    );
    assert!(!conflicted.status.success());
    fs::write(subject_root.join("conflict-driver.txt"), "resolved\n")
        .expect("conflict resolution should write");
    fs::write(
        subject_root.join("held/conflict-only.txt"),
        "merge result\n",
    )
    .expect("merge-only held path should write");
    git(subject_root, &["add", "-A"]);
    git(
        subject_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "resolve with held path",
        ],
    );
}

fn differential_attribution_paths() -> Vec<String> {
    [
        "held/café.txt",
        "held/conflict-only.txt",
        "held/deleted.txt",
        "held/line\nname.txt",
        "held/merged.txt",
        "held/rename-new.txt",
        "held/rename-old.txt",
        "held/tab\tname.txt",
    ]
    .map(str::to_owned)
    .to_vec()
}

struct UnreadablePhaseStartReservation {
    reservation_id: String,
    phase_start:    String,
}

fn create_valid_anchor_reservations(
    repository_root: &Path,
    reservation_count: usize,
) -> Vec<String> {
    (0..reservation_count)
        .map(|index| {
            let path = format!("valid-{index}.txt");
            let reservation_id = claim(repository_root, &format!("file:{path}"), SECOND_RUN);
            fs::write(repository_root.join(&path), format!("valid {index}\n"))
                .expect("valid phase path should write");
            git(repository_root, &["add", &path]);
            git(
                repository_root,
                &[
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "--quiet",
                    "-m",
                    &format!("valid phase {index}"),
                ],
            );
            reservation_id
        })
        .collect()
}

fn claim_independent_history_reservation(
    repository_root: &Path,
    configuration_source_root: &Path,
    return_branch: &str,
) -> String {
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "switch",
            "--quiet",
            "--orphan",
            "independent-history",
        ],
    );
    git(
        repository_root,
        &["-c", "core.hooksPath=/dev/null", "clean", "-d", "-f", "-x"],
    );
    fs::create_dir_all(repository_root.join(".claude/config"))
        .expect("independent configuration directory should exist");
    fs::copy(
        configuration_source_root.join(CONFIGURATION_PATH),
        repository_root.join(CONFIGURATION_PATH),
    )
    .expect("independent history should copy berth configuration");
    fs::write(
        repository_root.join("independent.txt"),
        "independent root\n",
    )
    .expect("independent root should write");
    git(repository_root, &["add", "-A"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "independent root",
        ],
    );
    let reservation_id = claim(repository_root, "file:independent-owned.txt", SECOND_RUN);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "switch",
            "--quiet",
            return_branch,
        ],
    );
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--quiet",
            "--no-ff",
            "--no-edit",
            "--allow-unrelated-histories",
            "independent-history",
        ],
    );
    reservation_id
}

fn claim_stale_anchor_reservation(repository_root: &Path) -> String {
    let stale_parent = git_stdout(repository_root, &["rev-parse", "HEAD"]);
    fs::write(repository_root.join("stale-anchor.txt"), "stale\n")
        .expect("stale anchor path should write");
    git(repository_root, &["add", "stale-anchor.txt"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "stale anchor",
        ],
    );
    let stale_phase_start = git_stdout(repository_root, &["rev-parse", "HEAD"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "update-ref",
            "refs/test/cargo-berth-stale-anchor",
            &stale_phase_start,
        ],
    );
    let reservation_id = claim(repository_root, "file:stale-owned.txt", SECOND_RUN);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            "--quiet",
            &stale_parent,
        ],
    );
    reservation_id
}

fn claim_unreadable_phase_start_reservation(
    repository_root: &Path,
) -> UnreadablePhaseStartReservation {
    let phase_start_parent = git_stdout(repository_root, &["rev-parse", "HEAD"]);
    fs::write(repository_root.join("missing-anchor.txt"), "missing\n")
        .expect("missing anchor path should write");
    git(repository_root, &["add", "missing-anchor.txt"]);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "missing anchor",
        ],
    );
    let phase_start = git_stdout(repository_root, &["rev-parse", "HEAD"]);
    let reservation_id = claim(repository_root, "file:missing-owned.txt", SECOND_RUN);
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "reset",
            "--hard",
            "--quiet",
            &phase_start_parent,
        ],
    );
    delete_reservation_retention_and_prune(repository_root, &reservation_id);
    let object_status = git_output(
        repository_root,
        &["cat-file", "-e", &format!("{phase_start}^{{commit}}")],
    );
    assert!(!object_status.status.success());
    UnreadablePhaseStartReservation {
        reservation_id,
        phase_start,
    }
}

#[derive(Debug, Eq, PartialEq)]
struct ReferenceIncursionCommit {
    subject: String,
    paths:   BTreeSet<String>,
}

fn per_path_incursion_attribution(
    repository_root: &Path,
    phase_start: &str,
    paths: &[String],
) -> BTreeMap<String, ReferenceIncursionCommit> {
    let range = format!("{phase_start}..HEAD");
    let mut commits = BTreeMap::new();
    for path in paths {
        let literal_path = format!(":(top,literal){path}");
        let output = git_output(
            repository_root,
            &["log", "--format=%H%x1f%s", &range, "--", &literal_path],
        );
        assert!(
            output.status.success(),
            "per-path reference failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8(output.stdout).expect("per-path log should be UTF-8");
        for line in text.lines() {
            let (commit, subject) = line
                .split_once('\u{1f}')
                .expect("per-path log should delimit commit and subject");
            let entry =
                commits
                    .entry(commit.to_owned())
                    .or_insert_with(|| ReferenceIncursionCommit {
                        subject: subject.to_owned(),
                        paths:   BTreeSet::new(),
                    });
            assert_eq!(entry.subject, subject);
            entry.paths.insert(path.clone());
        }
    }
    commits
}

fn reported_incursion_attribution(
    envelope: &serde_json::Value,
    reservation_id: &str,
) -> BTreeMap<String, ReferenceIncursionCommit> {
    let results = envelope["payload"]["data"]["results"]
        .as_array()
        .expect("drift should report reservation results");
    result_incursion_commits(results, reservation_id)
        .iter()
        .map(|commit| {
            let object_id = commit["commit"]
                .as_str()
                .expect("reported commit should name an object")
                .to_owned();
            let subject = commit["subject"]
                .as_str()
                .expect("reported commit should name a subject")
                .to_owned();
            let paths = commit["paths"]
                .as_array()
                .expect("reported commit should name paths")
                .iter()
                .map(|path| {
                    path.as_str()
                        .expect("reported commit path should be text")
                        .to_owned()
                })
                .collect();
            (object_id, ReferenceIncursionCommit { subject, paths })
        })
        .collect()
}

fn result_incursion_commits<'results>(
    results: &'results [serde_json::Value],
    reservation_id: &str,
) -> &'results [serde_json::Value] {
    results
        .iter()
        .find(|result| result["reservation_id"] == reservation_id)
        .and_then(|result| result["effects"].as_array())
        .and_then(|effects| effects.iter().find(|effect| effect["kind"] == "incursion"))
        .and_then(|effect| effect["commits"].as_array())
        .map(Vec::as_slice)
        .expect("reservation should report incursion commits")
}

fn delete_reservation_retention_and_prune(repository_root: &Path, reservation_id: &str) {
    let retention_ref = format!("refs/cargo-berth/reservations/{reservation_id}");
    git(
        repository_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "update-ref",
            "-d",
            &retention_ref,
        ],
    );
    git(
        repository_root,
        &["reflog", "expire", "--expire=now", "--all"],
    );
    git(repository_root, &["gc", "--prune=now"]);
}

impl TracedDrift {
    fn commands(&self) -> Vec<String> {
        fs::read_to_string(&self.trace_path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn fingerprint_commands(&self) -> Vec<String> {
        let mut commands = self
            .commands()
            .into_iter()
            .filter(|command| {
                matches!(
                    command.as_str(),
                    "diff" | "diff-tree" | "status" | "ls-files"
                )
            })
            .collect::<Vec<_>>();
        commands.sort();
        commands
    }
}

#[test]
fn a_rebase_reanchors_the_phase_so_the_new_base_is_not_read_as_its_work() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let feature = add_worktree(repository.path(), worktree_parent.path(), "feature");
    let reservation = claim(&feature, "file:claimed.txt", FIRST_RUN);

    // This phase commits its claimed file and one more the reservation does not cover.
    fs::write(feature.join("claimed.txt"), "phase\n").expect("claimed path should write");
    fs::write(feature.join("phase-extra.txt"), "phase\n").expect("extra path should write");
    git(&feature, &["add", "claimed.txt", "phase-extra.txt"]);
    git(&feature, &["commit", "--quiet", "-m", "phase work"]);

    // Trunk gains a file this worktree never opened.
    fs::write(repository.path().join("upstream.txt"), "upstream\n")
        .expect("upstream path should write");
    git(repository.path(), &["add", "upstream.txt"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "upstream work"],
    );

    git(&feature, &["rebase", "main"]);

    let events = journal_events(repository.path());
    let resnapshot = events
        .iter()
        .find(|event| event["op"] == "resnapshot")
        .expect("the rebase should re-anchor the phase");
    assert_eq!(resnapshot["reservation_id"], reservation.as_str());
    assert_eq!(resnapshot["snapshot"]["stage"], "active");

    let observed = drift(&feature, &["--full", "--reservation", &reservation]);
    let envelope = json_output(&observed);
    assert!(observed.status.success());
    assert_eq!(
        envelope["status"], "clear",
        "the replayed upstream commits are not this phase's work: {envelope}"
    );
    let journal = journal_text(repository.path());
    assert!(
        !journal.contains("upstream.txt"),
        "no upstream path may be widened onto or reported as an incursion: {journal}"
    );
    assert!(
        journal.contains("phase-extra.txt"),
        "this phase's own committed work still widens across the rebase: {journal}"
    );
}

#[test]
fn a_rebase_anchors_each_phase_sharing_a_branch_below_its_own_commits() {
    let repository = initialized_repository();
    let worktree_parent = tempdir().expect("worktree parent should exist");
    let feature = add_worktree(repository.path(), worktree_parent.path(), "feature");

    let first = claim(&feature, "file:alpha.txt", FIRST_RUN);
    fs::write(feature.join("alpha.txt"), "alpha\n").expect("alpha path should write");
    git(&feature, &["add", "alpha.txt"]);
    git(&feature, &["commit", "--quiet", "-m", "alpha"]);

    let second = claim(&feature, "file:beta.txt", FIRST_RUN);
    fs::write(feature.join("beta.txt"), "beta\n").expect("beta path should write");
    git(&feature, &["add", "beta.txt"]);
    git(&feature, &["commit", "--quiet", "-m", "beta"]);

    fs::write(repository.path().join("upstream.txt"), "upstream\n")
        .expect("upstream path should write");
    git(repository.path(), &["add", "upstream.txt"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "upstream work"],
    );

    git(&feature, &["rebase", "main"]);

    // The branch now reads: upstream, alpha, beta. Each phase anchors directly beneath
    // the commits it authored, so the earlier phase's work is never read as the later
    // phase's, and neither one owns the commit the rebase replayed them onto.
    let anchors = journal_events(repository.path())
        .into_iter()
        .filter(|event| event["op"] == "resnapshot")
        .map(|event| {
            (
                event["reservation_id"]
                    .as_str()
                    .expect("a resnapshot names its reservation")
                    .to_owned(),
                event["snapshot"]["claim_snapshot"]
                    .as_str()
                    .expect("an active resnapshot names its phase start")
                    .to_owned(),
            )
        })
        .collect::<Vec<_>>();
    let anchor_for = |reservation: &str| {
        anchors
            .iter()
            .find(|(id, _)| id == reservation)
            .map(|(_, anchor)| anchor.clone())
            .expect("every active reservation on the branch is re-anchored")
    };
    assert_eq!(
        anchor_for(&first),
        git_stdout(&feature, &["rev-parse", "HEAD~2"]).trim(),
        "the first phase anchors on the commit the rebase moved it onto"
    );
    assert_eq!(
        anchor_for(&second),
        git_stdout(&feature, &["rev-parse", "HEAD~1"]).trim(),
        "the second phase anchors above the first phase's replayed commit"
    );
}

fn traced_drift(repository_root: &Path, arguments: &[&str]) -> TracedDrift {
    let directory = tempdir().expect("wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let trace_path = directory.path().join("trace");
    fs::write(&wrapper_path, TRACING_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let mut command_arguments = vec!["drift"];
    command_arguments.extend_from_slice(arguments);
    command_arguments.push("--json");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(command_arguments)
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("traced drift should run");
    TracedDrift {
        output,
        trace_path,
        _directory: directory,
    }
}

fn assert_batched_full_attribution_commands(commands: &[String]) {
    assert_eq!(
        commands
            .iter()
            .filter(|command| command.as_str() == "rev-list")
            .count(),
        1,
        "full observation spends one batched phase-range ancestry walk",
    );
    assert!(!commands.iter().any(|command| command == "metadata"));
}

fn assert_no_phase_ancestry_or_metadata_command(commands: &[String]) {
    assert!(!commands.iter().any(|command| command == "rev-list"));
    assert!(!commands.iter().any(|command| command == "metadata"));
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree can hold a reservation of its own alongside another run's. The returned
/// directory owns the worktree and must outlive its use.
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
    let repository = scratch_repository();
    assert!(
        run_berth(repository.path(), &["init", "--json"])
            .status
            .success()
    );
    git(repository.path(), &["add", CONFIGURATION_PATH]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "configure berth"],
    );
    repository
}

/// Initialize berth while leaving its repository configuration as an untracked path.
fn repository_with_uncommitted_berth_configuration() -> TempDir {
    let repository = scratch_repository();
    assert!(
        run_berth(repository.path(), &["init", "--json"])
            .status
            .success()
    );
    let untracked = git_stdout(
        repository.path(),
        &["status", "--short", "--untracked-files=all"],
    );
    assert!(
        untracked
            .lines()
            .any(|line| line == format!("?? {CONFIGURATION_PATH}")),
        "init's configuration should remain the fixture's untracked change: {untracked}"
    );
    repository
}

fn scratch_repository() -> TempDir {
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "Berth Test"]);
    fs::write(repository.path().join("README.md"), "scratch\n").expect("base file should write");
    git(repository.path(), &["add", "README.md"]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    repository
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> PathBuf {
    let root = parent.join(branch);
    git(
        repository_root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            root.to_str().expect("worktree path should be UTF-8"),
        ],
    );
    root
}

fn claim(repository_root: &Path, scope: &str, run: &str) -> String {
    let claimed = run_berth(
        repository_root,
        &["claim", scope, "--run", run, "--why", "test work", "--json"],
    );
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    json_output(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn claim_with_session(repository_root: &Path, scope: &str, run: &str, session_id: &str) -> String {
    let claimed = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args([
            "claim",
            scope,
            "--run",
            run,
            "--why",
            "test mapped work",
            "--json",
        ])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("mapped claim should run");
    assert!(claimed.status.success());
    json_output(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

fn claim_with_override(repository_root: &Path, scope: &str, run: &str, holder_id: &str) -> String {
    let arguments = [
        "claim",
        scope,
        "--run",
        run,
        "--why",
        "test overlapping work",
        "--override",
        holder_id,
        "--overlap-why",
        "the shared edit was reviewed",
        "--json",
    ];
    let proposed = run_berth(repository_root, &arguments);
    assert_eq!(proposed.status.code(), Some(3));
    let proposed_envelope = json_output(&proposed);
    let proposal_token = proposed_envelope["payload"]["data"]["proposal_token"]
        .as_str()
        .expect("proposal should return a token");
    let mut applying_arguments = arguments.to_vec();
    applying_arguments.splice(
        applying_arguments.len() - 1..applying_arguments.len() - 1,
        ["--proposal", proposal_token],
    );
    let applied = run_berth(repository_root, &applying_arguments);
    assert!(
        applied.status.success(),
        "authorized claim failed: {}",
        String::from_utf8_lossy(&applied.stdout)
    );
    json_output(&applied)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("authorized claim should return a reservation id")
        .to_owned()
}

fn drift(repository_root: &Path, arguments: &[&str]) -> Output {
    let mut command_arguments = vec!["drift"];
    command_arguments.extend_from_slice(arguments);
    command_arguments.push("--json");
    run_berth(repository_root, &command_arguments)
}

fn post_commit_drift(repository_root: &Path, arguments: &[&str]) -> Output {
    let mut command_arguments = vec!["drift", "--full"];
    command_arguments.extend_from_slice(arguments);
    command_arguments.push("--json");
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(command_arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .output()
        .expect("post-commit drift should run")
}

fn cheap_post_commit_drift(repository_root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["drift", "--json"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .output()
        .expect("cheap post-commit drift should run")
}

fn text_post_commit_drift(repository_root: &Path, arguments: &[&str]) -> Output {
    let mut command_arguments = vec!["drift", "--full"];
    command_arguments.extend_from_slice(arguments);
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(command_arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .output()
        .expect("text post-commit drift should run")
}

fn post_commit_drift_with_session(repository_root: &Path, session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["drift", "--full", "--json"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("mapped post-commit drift should run")
}

fn fingerprint_cache(repository_root: &Path) -> PathBuf {
    fs::read_dir(repository_root.join(".git/cargo-berth"))
        .expect("ledger directory should read")
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("drift-fingerprint-"))
        })
        .expect("fingerprint cache should exist")
        .path()
}

/// Which coordination run makes the commit that enters another worktree's scopes.
#[derive(Clone, Copy)]
enum CommittingRun {
    /// A second presented run, which the occupancy rule refuses a berth of its own.
    ASecondPresentedRun,
    /// No run of its own, so the worktree's incumbent is the party that committed.
    TheIncumbent,
}

/// What a commit entering another worktree's scopes left in the journal.
struct CrossWorktreeEntry {
    /// The reservation the other worktree holds over the entered path.
    foreign_id: String,
    /// Every incursion the commit recorded.
    incursions: Vec<serde_json::Value>,
}

impl CrossWorktreeEntry {
    /// Whether some incursion names the other worktree's holder as a party entered.
    fn names_the_foreign_holder(&self) -> bool {
        self.incursions.iter().any(|incursion| {
            incursion["foreign_reservation_ids"]
                .as_array()
                .is_some_and(|holders| holders.iter().any(|held| held == &self.foreign_id))
        })
    }
}

/// Commit into a second worktree's scopes under the named run and report what it recorded.
fn cross_worktree_entry(committing_run: CommittingRun) -> CrossWorktreeEntry {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "foreign");
    let foreign_id = claim(&foreign_root, "tree:shared", THIRD_RUN);
    claim(repository.path(), "file:held.txt", FIRST_RUN);
    fs::create_dir_all(repository.path().join("shared")).expect("shared directory should exist");
    fs::write(
        repository.path().join("shared/s.txt"),
        "entered the other worktree\n",
    )
    .expect("entered path should write");
    git(repository.path(), &["add", "shared/s.txt"]);

    let committed = match committing_run {
        CommittingRun::ASecondPresentedRun => git_output_with_environment(
            repository.path(),
            &["commit", "-m", "enter the other worktree"],
            RUN_ENVIRONMENT,
            SECOND_RUN,
        ),
        CommittingRun::TheIncumbent => git_output(
            repository.path(),
            &["commit", "-m", "enter the other worktree"],
        ),
    };
    assert!(
        committed.status.success(),
        "a reported entry never removes the commit: {}",
        String::from_utf8_lossy(&committed.stderr)
    );
    let incursions = journal_events(repository.path())
        .into_iter()
        .filter(|event| event["op"] == "incursion")
        .collect();
    CrossWorktreeEntry {
        foreign_id,
        incursions,
    }
}

/// The journal and its projection at one moment, so an interleaving can be staged.
struct LedgerSnapshot {
    journal:    Vec<u8>,
    projection: Vec<u8>,
}

impl LedgerSnapshot {
    fn capture(repository_root: &Path) -> Self {
        Self {
            journal:    fs::read(repository_root.join(JOURNAL_PATH)).expect("journal should read"),
            projection: fs::read(repository_root.join(PROJECTION_PATH))
                .expect("projection should read"),
        }
    }

    fn restore(&self, repository_root: &Path) {
        fs::write(repository_root.join(JOURNAL_PATH), &self.journal)
            .expect("journal should restore");
        fs::write(repository_root.join(PROJECTION_PATH), &self.projection)
            .expect("projection should restore");
    }
}

/// Roll the ledger back to the state a worktree no run has claimed in would present.
fn stage_an_unoccupied_ledger(repository_root: &Path) {
    fs::write(repository_root.join(JOURNAL_PATH), b"").expect("journal should stage empty");
    let projection_path = repository_root.join(PROJECTION_PATH);
    if projection_path.exists() {
        fs::remove_file(projection_path).expect("projection should stage empty");
    }
}

/// Block until the spawned run reports that it is waiting on the mutation lock.
fn wait_until_held_at_the_mutation_lock(waiting_path: &Path) {
    let reached_the_lock = (0..MUTATION_LOCK_WAIT_ATTEMPTS).any(|_| {
        if waiting_path.is_file() {
            return true;
        }
        thread::sleep(MUTATION_LOCK_WAIT_INTERVAL);
        false
    });
    assert!(
        reached_the_lock,
        "the second run should reach the mutation lock and report waiting on it"
    );
}

/// Count the claims the journal records, the acquisition a refused run must not make.
fn claim_event_count(repository_root: &Path) -> usize {
    journal_events(repository_root)
        .iter()
        .filter(|event| event["op"] == "claim")
        .count()
}

fn post_commit_drift_under_run(repository_root: &Path, run: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["drift", "--full", "--json"])
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .env(POST_COMMIT_ENVIRONMENT, "1")
        .env(RUN_ENVIRONMENT, run)
        .output()
        .expect("post-commit drift under a presented run should run")
}

fn journal_text(repository_root: &Path) -> String {
    fs::read_to_string(repository_root.join(JOURNAL_PATH)).expect("journal should read")
}

fn journal_events(repository_root: &Path) -> Vec<serde_json::Value> {
    journal_text(repository_root)
        .lines()
        .map(|line| serde_json::from_str(line).expect("journal event should decode"))
        .collect()
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render JSON")
}

/// Collect every incursion effect the drift report carries, across all results.
///
/// Entered paths are reported against the holders that actually block them, so one
/// drift run reports one incursion per distinct set of holders.
fn incursion_effects(envelope: &serde_json::Value) -> Vec<serde_json::Value> {
    envelope["payload"]["data"]["results"]
        .as_array()
        .expect("drift should report results")
        .iter()
        .filter_map(|result| result["effects"].as_array())
        .flatten()
        .filter(|effect| effect["kind"] == "incursion")
        .cloned()
        .collect()
}

/// Select the one incursion reporting the named path.
fn incursion_for(effects: &[serde_json::Value], path: &str) -> serde_json::Value {
    effects
        .iter()
        .find(|effect| {
            effect["paths"]
                .as_array()
                .is_some_and(|paths| paths.iter().any(|entered| entered == path))
        })
        .expect("every entered path should be reported by some incursion")
        .clone()
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(POST_COMMIT_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn assert_coordination_identity_rejection(
    envelope: &serde_json::Value,
    expected_kind: &str,
    expected_action_kinds: &[&str],
) {
    assert_eq!(envelope["payload"]["kind"], "coordination_identity");
    assert_eq!(envelope["payload"]["data"]["kind"], expected_kind);
    let actions = envelope["payload"]["data"]["recovery_actions"]
        .as_array()
        .expect("identity rejection should carry recovery actions");
    assert!(!actions.is_empty());
    assert_eq!(
        actions
            .iter()
            .map(|action| action["kind"].as_str().expect("action kind should be text"))
            .collect::<Vec<_>>(),
        expected_action_kinds
    );
    for action in actions {
        assert!(
            action["argv"]
                .as_array()
                .is_some_and(|argv| !argv.is_empty())
        );
        assert!(
            action["cwd"]
                .as_str()
                .is_some_and(|cwd| Path::new(cwd).is_absolute())
        );
    }
}

fn run_berth_with_run(repository_root: &Path, arguments: &[&str], run: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env(RUN_ENVIRONMENT, run)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn git_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("test PATH should exist"))
        .map(|directory| directory.join(GIT_BINARY))
        .find(|candidate| candidate.is_file())
        .expect("git should exist on PATH")
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn git_output(repository_root: &Path, arguments: &[&str]) -> Output {
    GIT.output(repository_root, arguments)
}

fn git_output_with_environment(
    repository_root: &Path,
    arguments: &[&str],
    name: &str,
    value: &str,
) -> Output {
    GIT.output_with_environment(repository_root, arguments, name, value)
}
