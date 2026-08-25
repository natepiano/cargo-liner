#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! End-to-end drift fingerprint, selection, classification, replay, and hook tests.

use std::fs;
use std::fs::File;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;

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
const POST_COMMIT_HOOK_PATH: &str = ".git/hooks/post-commit";
const PROJECTION_PATH: &str = ".git/cargo-berth/reservations.json";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const POST_COMMIT_ENVIRONMENT: &str = "CARGO_BERTH_POST_COMMIT";
const REAL_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_PATH";
const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ]; then
    printf '%s\n' "$2" >> "$CARGO_BERTH_TEST_GIT_TRACE"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const COLLISION_GIT_WRAPPER: &str = r#"#!/bin/sh
set -eu
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "ls-files" ]; then
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
        widened_repository.path(),
        &["check", "file:untracked.txt", "--json"],
        SECOND_RUN,
    );
    assert_eq!(replayed_block.status.code(), Some(1));
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
    assert_eq!(json_output(&resolved)["status"], "incursion_resolved");
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

    assert_new_incident_after_resolution(&incursion_repository, &subject_id, &incident_id);
}

fn assert_new_incident_after_resolution(
    incursion_repository: &TempDir,
    subject_id: &str,
    original_incident_id: &str,
) {
    let observed_after_resolution = drift(
        incursion_repository.path(),
        &["--full", "--reservation", subject_id],
    );
    assert_eq!(observed_after_resolution.status.code(), Some(1));
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
    assert_eq!(incident_ids.len(), 2);
    assert_eq!(incident_ids[0], original_incident_id);
    assert_ne!(incident_ids[0], incident_ids[1]);
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
fn post_commit_treats_another_run_in_the_same_worktree_as_foreign() {
    let repository = initialized_repository();
    let holder_id = claim(repository.path(), "file:held.txt", FIRST_RUN);
    let subject_id = claim(repository.path(), "file:subject.txt", SECOND_RUN);
    fs::write(repository.path().join("held.txt"), "entered holder scope\n")
        .expect("holder path should write");
    git(repository.path(), &["add", "held.txt"]);

    let committed = git_output(repository.path(), &["commit", "-m", "enter other run"]);
    let warning = String::from_utf8_lossy(&committed.stderr);

    assert!(committed.status.success());
    assert!(warning.contains("Incursion"));
    assert!(warning.contains(&holder_id));
    assert!(warning.contains(&subject_id));
    let incursion = journal_events(repository.path())
        .into_iter()
        .find(|event| event["op"] == "incursion" && event["reservation_id"] == subject_id)
        .expect("the other run should record an incursion");
    let incident_id = incursion["incident_id"]
        .as_str()
        .expect("incursion should carry an incident id");
    assert!(warning.contains(&format!("resolve {subject_id} --incursion {incident_id}")));
    assert_eq!(
        incursion["foreign_reservation_ids"],
        serde_json::json!([holder_id])
    );
}

#[test]
fn markerless_post_commit_reports_every_incursion_without_ambiguous_widens() {
    let repository = initialized_repository();
    let worktrees = tempdir().expect("worktree parent should exist");
    let foreign_root = add_worktree(repository.path(), worktrees.path(), "foreign");
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", SECOND_RUN);
    let foreign_id = claim(&foreign_root, "tree:shared", THIRD_RUN);
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

    let markerless = post_commit_drift(repository.path(), &[]);
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
    let first_id = claim(repository.path(), "file:first.txt", FIRST_RUN);
    let second_id = claim(repository.path(), "file:second.txt", FIRST_RUN);
    let selected_id = claim(repository.path(), "file:selected.txt", SECOND_RUN);
    fs::write(
        repository.path().join("outside.txt"),
        "outside every scope\n",
    )
    .expect("outside path should write");

    let widened = post_commit_drift(repository.path(), &[]);

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

    let ambiguous = post_commit_drift(repository.path(), &[]);
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
    let second_id = claim(repository.path(), "file:second.txt", SECOND_RUN);
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
    let holder_id = claim(repository.path(), "tree:shared", FIRST_RUN);
    let subject_id = claim_with_override(
        repository.path(),
        "file:shared/approved.txt",
        SECOND_RUN,
        &holder_id,
    );
    fs::write(repository.path().join("outside.txt"), "new scope\n")
        .expect("new scope path should write");

    let widened = run_berth_with_run(
        repository.path(),
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
        vec!["diff", "diff", "diff", "ls-files"]
    );
    assert_no_expensive_or_metadata_command(&full.commands());

    let cheap = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(cheap.output.status.success());
    assert_eq!(cheap.fingerprint_commands(), vec!["status", "ls-files"]);
    assert_no_expensive_or_metadata_command(&cheap.commands());

    fs::remove_file(fingerprint_cache(repository.path())).expect("fingerprint cache should delete");
    let missing_cache = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(missing_cache.output.status.success());
    assert_eq!(
        missing_cache.fingerprint_commands(),
        vec!["diff", "diff", "diff", "ls-files"]
    );
    assert_eq!(
        json_output(&missing_cache.output)["payload"]["data"]["comparison"],
        "full_phase_start_fallback"
    );

    fs::write(fingerprint_cache(repository.path()), "not json")
        .expect("corrupt fingerprint should write");
    let corrupt_cache = traced_drift(repository.path(), &["--reservation", &reservation_id]);
    assert!(corrupt_cache.output.status.success());
    assert_eq!(
        corrupt_cache.fingerprint_commands(),
        vec!["diff", "diff", "diff", "ls-files"]
    );
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

impl TracedDrift {
    fn commands(&self) -> Vec<String> {
        fs::read_to_string(&self.trace_path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect()
    }

    fn fingerprint_commands(&self) -> Vec<String> {
        self.commands()
            .into_iter()
            .filter(|command| matches!(command.as_str(), "diff" | "status" | "ls-files"))
            .collect()
    }
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

fn assert_no_expensive_or_metadata_command(commands: &[String]) {
    assert!(!commands.iter().any(|command| command == "rev-list"));
    assert!(!commands.iter().any(|command| command == "metadata"));
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
    let output = git_output(repository_root, arguments);
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

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = git_output(repository_root, arguments);
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_output(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(BYPASS_ENVIRONMENT)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("git should run")
}

fn git_output_with_environment(
    repository_root: &Path,
    arguments: &[&str],
    name: &str,
    value: &str,
) -> Output {
    Command::new(GIT_BINARY)
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .env(name, value)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("git should run")
}
