#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for ordering-edge replay, locked mutation, and limits.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::thread;

use tempfile::TempDir;
use tempfile::tempdir;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
const ALERT_BRANCH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_ALERT_BRANCH";
const EXECUTABLE_PERMISSIONS: u32 = 0o755;
const FIFTH_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const FOURTH_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e";
const GIT_BINARY: &str = "git";
const JOURNAL_PATH: &str = ".git/cargo-berth/journal.ndjson";
const MARKER_PATH: &str = ".git/cargo-berth-run-id";
const MARKER_PATH_ENVIRONMENT: &str = "CARGO_BERTH_TEST_MARKER_PATH";
const REAL_GIT_ENVIRONMENT: &str = "CARGO_BERTH_TEST_REAL_GIT";
const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";
const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";
const STALE_MARKER_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "rev-parse" ] && [ "$3" = "$CARGO_BERTH_TEST_ALERT_BRANCH" ]; then
    printf '%s\n' "$CARGO_BERTH_TEST_STALE_RUN" > "$CARGO_BERTH_TEST_MARKER_PATH"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const STALE_RUN_ENVIRONMENT: &str = "CARGO_BERTH_TEST_STALE_RUN";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

#[derive(Clone, Copy)]
enum ObservedPredecessorLiveness {
    Live,
    Unavailable,
    OrphanCandidate,
    Unknown,
}

#[test]
fn claim_time_directions_keep_the_embedded_edge_and_claim_event_id() {
    assert_claim_time_direction("--after", "holder_before_requester");
    assert_claim_time_direction("--before", "requester_before_holder");
}

#[test]
fn deferred_ordering_is_replayable_and_duplicate_or_reverse_resolution_is_rejected() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);

    let sequenced = sequence(
        repository.path(),
        &holder_id,
        &requester_id,
        "the holder API must land first",
    );
    let sequenced_json = json_output(&sequenced);
    assert!(
        sequenced.status.success(),
        "sequence failed: {}",
        String::from_utf8_lossy(&sequenced.stdout)
    );
    assert_eq!(sequenced_json["status"], "sequenced");
    assert_eq!(sequenced_json["exit_code"], 0);
    assert_eq!(sequenced_json["payload"]["kind"], "sequence");
    assert_eq!(sequenced_json["payload"]["data"]["status"], "sequenced");
    assert_eq!(
        sequenced_json["payload"]["data"]["edge"]["declaration"],
        "deferred_resolution"
    );
    assert_eq!(
        sequenced_json["payload"]["data"]["edge"]["before"],
        holder_id
    );
    assert_eq!(
        sequenced_json["payload"]["data"]["edge"]["after"],
        requester_id
    );
    assert_eq!(
        sequenced_json["payload"]["data"]["readiness"],
        serde_json::json!({
            "state": "holding",
            "hold": {"reason": "awaiting_predecessor_checkpoint"}
        })
    );
    let resolution = last_journal_event(repository.path());
    assert_eq!(resolution["op"], "resolve_defer");
    assert_eq!(
        resolution["edge_id"],
        sequenced_json["payload"]["data"]["edge"]["edge_id"]
    );
    assert_eq!(
        resolution["event_id"],
        sequenced_json["payload"]["data"]["edge"]["declaration_event_id"]
    );
    assert!(!journal_text(repository.path()).contains("declare_ordering_edge"));

    let duplicate = sequence(repository.path(), &holder_id, &requester_id, "repeat");
    assert_eq!(duplicate.status.code(), Some(2));
    let duplicate_json = json_output(&duplicate);
    assert_eq!(duplicate_json["status"], "duplicate_ordering_edge");
    assert_eq!(duplicate_json["exit_code"], 2);
    assert_eq!(duplicate_json["blocked_by"], serde_json::json!([holder_id]));

    let reverse = sequence(repository.path(), &requester_id, &holder_id, "reverse");
    assert_eq!(reverse.status.code(), Some(2));
    let reverse_json = json_output(&reverse);
    assert_eq!(reverse_json["status"], "ordering_cycle");
    assert_eq!(reverse_json["exit_code"], 2);
    assert_eq!(reverse_json["blocked_by"], serde_json::json!([holder_id]));
    assert_eq!(resolve_defer_count(repository.path()), 1);
}

#[test]
fn sequence_rejects_a_stale_post_reconciliation_marker_and_carries_alerts() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);
    let worktrees = tempdir().expect("worktree parent should exist");
    let orphan_root = add_worktree(repository.path(), worktrees.path(), "orphan-alert");
    fs::write(orphan_root.join("orphan.txt"), "orphan work\n").expect("orphan source should write");
    git(&orphan_root, &["add", "."]);
    git(&orphan_root, &["commit", "--quiet", "-m", "orphan work"]);
    let orphan = claim(&orphan_root, "file:orphan.txt", THIRD_RUN);
    let orphan_id = reservation_id(&orphan);
    assert!(
        run_berth(&orphan_root, &["release", &orphan_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&orphan_root).expect("orphan worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let rejected = run_berth_with_stale_marker(
        repository.path(),
        "refs/heads/orphan-alert",
        &[
            "sequence",
            &holder_id,
            &requester_id,
            "--why",
            "the holder must land first",
            "--json",
        ],
    );
    let rejected_json = json_output(&rejected);

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_json["exit_code"], 5);
    assert_eq!(rejected_json["status"], "invalid_input");
    assert_eq!(
        rejected_json["payload"]["data"]["reason"]["kind"],
        "inactive_marker_run"
    );
    assert_eq!(
        rejected_json["payload"]["data"]["reason"]["coordination_run_id"],
        FOURTH_RUN
    );
    assert_eq!(rejected_json["blocked_by"], serde_json::json!([]));
    assert_eq!(
        rejected_json["payload"]["alerts"][0]["kind"],
        "orphaned_outstanding"
    );
    assert_eq!(
        rejected_json["payload"]["alerts"][0]["data"]["reservation_id"],
        orphan_id
    );
    assert_eq!(resolve_defer_count(repository.path()), 0);
}

#[test]
fn sequence_reports_an_inactive_session_mapping_without_a_marker_diagnostic() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);
    let session_id = "stale-sequence-session";
    let mapped_claim = run_berth_with_session(
        &third_root,
        &[
            "claim",
            "file:session-sequence",
            "--run",
            THIRD_RUN,
            "--why",
            "establish sequence session mapping",
            "--json",
        ],
        session_id,
    );
    assert!(mapped_claim.status.success());
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(&third_root, &["release", &mapped_reservation_id, "--json"])
            .status
            .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale session mapping should write");

    let rejected = run_berth_with_session(
        repository.path(),
        &[
            "sequence",
            &holder_id,
            &requester_id,
            "--why",
            "the holder must land first",
            "--json",
        ],
        session_id,
    );
    let rejected_json = json_output(&rejected);
    let diagnostic = rejected_json["message"]
        .as_str()
        .expect("sequence rejection should have a message");

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(
        rejected_json["payload"]["data"]["reason"]["kind"],
        "inactive_session_mapping"
    );
    assert!(diagnostic.contains("Harness session mapping"));
    assert!(!diagnostic.contains("coordination-run marker"));
    assert_eq!(resolve_defer_count(repository.path()), 0);
}

#[test]
fn concurrent_opposite_resolutions_append_exactly_one_edge() {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);
    let repository_root = repository.path().to_path_buf();
    let first_holder = holder_id.clone();
    let first_requester = requester_id.clone();
    let first = thread::spawn(move || {
        sequence(
            &repository_root,
            &first_holder,
            &first_requester,
            "holder first",
        )
    });
    let repository_root = repository.path().to_path_buf();
    let second = thread::spawn(move || {
        sequence(
            &repository_root,
            &requester_id,
            &holder_id,
            "requester first",
        )
    });
    let first = first.join().expect("first sequence process should finish");
    let second = second
        .join()
        .expect("second sequence process should finish");
    let successes = usize::from(first.status.success()) + usize::from(second.status.success());
    assert_eq!(successes, 1);
    assert_eq!(resolve_defer_count(repository.path()), 1);
}

#[test]
fn reservation_and_ordering_edge_limits_have_distinct_typed_outcomes() {
    let reservation_repository = initialized_repository();
    set_config_limit(reservation_repository.path(), "maximum_reservations", 1);
    let first = claim(reservation_repository.path(), "tree:src", FIRST_RUN);
    assert!(first.status.success());
    let over_limit = claim(reservation_repository.path(), "tree:tests", SECOND_RUN);
    let over_limit_json = json_output(&over_limit);
    assert_eq!(over_limit.status.code(), Some(1));
    assert_eq!(over_limit_json["status"], "reservation_limit_reached");
    assert_eq!(
        over_limit_json["payload"]["data"]["status"],
        "reservation_limit_reached"
    );

    let edge_repository = initialized_repository();
    let (_edge_second_directory, edge_second_root) = foreign_worktree(&edge_repository, "second");
    let (holder_id, requester_id) = deferred_pair(edge_repository.path(), &edge_second_root);
    set_config_limit(edge_repository.path(), "maximum_ordering_edges", 0);
    let over_limit = sequence(
        edge_repository.path(),
        &holder_id,
        &requester_id,
        "the holder must land first",
    );
    let over_limit_json = json_output(&over_limit);
    assert_eq!(over_limit.status.code(), Some(2));
    assert_eq!(over_limit_json["status"], "ordering_edge_limit_reached");
    assert_eq!(
        over_limit_json["payload"]["data"]["reason"]["kind"],
        "ordering_edge_limit_reached"
    );
    assert_eq!(over_limit_json["blocked_by"], serde_json::json!([]));
    assert_eq!(resolve_defer_count(edge_repository.path()), 0);
}

#[test]
fn successor_head_and_current_trunk_both_control_readiness() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    fs::create_dir_all(repository.path().join("left")).expect("left directory should exist");
    fs::create_dir_all(repository.path().join("right")).expect("right directory should exist");
    fs::write(repository.path().join("left/shared.rs"), "// left\n")
        .expect("left source should write");
    fs::write(repository.path().join("right/shared.rs"), "// right\n")
        .expect("right source should write");
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "shared paths"],
    );
    let base = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let first_successor_root = add_worktree(repository.path(), worktrees.path(), "successor-one");
    let second_successor_root = add_worktree(repository.path(), worktrees.path(), "successor-two");
    fs::write(
        predecessor_root.join("left/shared.rs"),
        "// predecessor left\n",
    )
    .expect("predecessor left source should write");
    fs::write(
        predecessor_root.join("right/shared.rs"),
        "// predecessor right\n",
    )
    .expect("predecessor right source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim_scopes(&predecessor_root, &["tree:left", "tree:right"], FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let first_successor = defer_claim(
        &first_successor_root,
        "file:left/shared.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let first_successor_id = reservation_id(&first_successor);
    let second_successor = defer_claim(
        &second_successor_root,
        "file:right/shared.rs",
        THIRD_RUN,
        &predecessor_id,
    );
    let second_successor_id = reservation_id(&second_successor);
    let checkpoint = run_berth(&predecessor_root, &["release", &predecessor_id, "--json"]);
    assert!(checkpoint.status.success());
    git(repository.path(), &["merge", "--quiet", "predecessor"]);

    let awaiting_incorporation = sequence(
        repository.path(),
        &predecessor_id,
        &first_successor_id,
        "predecessor first",
    );
    let awaiting_incorporation_json = json_output(&awaiting_incorporation);
    assert!(awaiting_incorporation.status.success());
    assert_eq!(
        awaiting_incorporation_json["payload"]["data"]["readiness"],
        serde_json::json!({
            "state": "holding",
            "hold": {"reason": "awaiting_successor_incorporation"}
        })
    );
    assert_eq!(
        awaiting_incorporation_json["blocked_by"],
        serde_json::json!([predecessor_id])
    );

    git(repository.path(), &["reset", "--hard", &base]);
    let rewritten = sequence(
        repository.path(),
        &predecessor_id,
        &second_successor_id,
        "predecessor first after rewrite",
    );
    assert!(rewritten.status.success());
    assert_eq!(
        json_output(&rewritten)["payload"]["data"]["readiness"],
        serde_json::json!({
            "state": "holding",
            "hold": {
                "reason": "predecessor_not_on_trunk",
                "evidence": "trunk_rewritten"
            }
        })
    );
}

#[test]
fn predecessor_checkpoint_not_on_trunk_reports_not_integrated() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    let checkpoint = run_berth(&predecessor_root, &["release", &predecessor_id, "--json"]);
    assert!(checkpoint.status.success());

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "predecessor checkpoint has not landed",
    );

    assert!(sequence.status.success());
    assert_eq!(
        json_output(&sequence)["payload"]["data"]["readiness"],
        serde_json::json!({
            "state": "holding",
            "hold": {
                "reason": "predecessor_not_on_trunk",
                "evidence": "not_integrated"
            }
        })
    );
}

#[test]
fn successor_incorporation_fulfills_an_integrated_edge() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    git(repository.path(), &["merge", "--quiet", "predecessor"]);
    git(&successor_root, &["merge", "--quiet", "main"]);

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "successor already contains predecessor",
    );

    assert!(sequence.status.success());
    assert_eq!(
        json_output(&sequence)["payload"]["data"]["readiness"],
        serde_json::json!({"state": "fulfilled"})
    );
}

#[test]
fn confirmed_abandonment_cancels_an_edge() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    fs::remove_dir_all(&predecessor_root).expect("predecessor worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);
    let abandoned = run_berth(
        repository.path(),
        &[
            "resolve",
            &predecessor_id,
            "--abandon",
            "--why",
            "confirmed predecessor abandonment",
            "--json",
        ],
    );
    assert!(abandoned.status.success());

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "record the resolved order",
    );

    assert!(sequence.status.success());
    assert_eq!(
        json_output(&sequence)["payload"]["data"]["readiness"],
        serde_json::json!({"state": "cancelled"})
    );
}

#[test]
fn confirmed_successor_abandonment_cancels_and_releases_the_edge() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    fs::remove_dir_all(&successor_root).expect("successor worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);
    let abandoned = run_berth(
        repository.path(),
        &[
            "resolve",
            &successor_id,
            "--abandon",
            "--why",
            "confirmed successor abandonment",
            "--json",
        ],
    );
    assert!(abandoned.status.success());

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "record the terminal successor edge",
    );
    let sequence_json = json_output(&sequence);

    assert!(sequence.status.success());
    assert_eq!(
        sequence_json["payload"]["data"]["readiness"],
        serde_json::json!({"state": "cancelled"})
    );
    assert_eq!(sequence_json["blocked_by"], serde_json::json!([]));
}

#[test]
fn released_predecessor_ref_retires_only_after_every_successor_is_terminal() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "predecessor work"],
    );
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "predecessor first",
    );
    assert!(sequence.status.success());
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    git(repository.path(), &["merge", "--quiet", "predecessor"]);
    assert!(
        run_berth(repository.path(), &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    assert!(
        run_berth(repository.path(), &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    let predecessor_ref = reservation_ref(&predecessor_id);
    assert!(reference_exists(repository.path(), &predecessor_ref));

    let repair_subject = claim(repository.path(), "tree:tests", THIRD_RUN);
    let repair_subject_id = reservation_id(&repair_subject);
    assert!(
        run_berth(
            repository.path(),
            &["release", &repair_subject_id, "--json"]
        )
        .status
        .success()
    );
    let repair_ref = reservation_ref(&repair_subject_id);
    git(repository.path(), &["update-ref", "-d", &repair_ref]);
    assert!(!reference_exists(repository.path(), &repair_ref));

    let terminal = run_berth(
        repository.path(),
        &[
            "resolve",
            &successor_id,
            "--abandon",
            "--why",
            "the successor work is deliberately discarded",
            "--json",
        ],
    );
    assert!(terminal.status.success());
    assert!(!reference_exists(repository.path(), &predecessor_ref));

    let reconciliation = run_berth(repository.path(), &["renew", &predecessor_id, "--json"]);
    assert_eq!(reconciliation.status.code(), Some(5));
    assert!(!reference_exists(repository.path(), &predecessor_ref));
    assert!(reference_exists(repository.path(), &repair_ref));
    let further_reconciliation =
        run_berth(repository.path(), &["renew", &predecessor_id, "--json"]);
    assert_eq!(further_reconciliation.status.code(), Some(5));
    assert!(!reference_exists(repository.path(), &predecessor_ref));
}

#[test]
fn orphaned_middle_predecessor_recovers_without_losing_its_outgoing_edge() {
    let repository = commit_left_and_right_trees();
    let worktrees = tempdir().expect("worktree parent should exist");
    let first_root = add_worktree(repository.path(), worktrees.path(), "first");
    let middle_root = add_worktree(repository.path(), worktrees.path(), "middle");
    let before_root = add_worktree(repository.path(), worktrees.path(), "before-orphaning");
    let orphaned_root = add_worktree(repository.path(), worktrees.path(), "while-orphaned");
    let recovered_root = add_worktree(repository.path(), worktrees.path(), "after-recovery");
    let first = claim(&first_root, "tree:left", FIRST_RUN);
    let first_id = reservation_id(&first);
    let middle = defer_claim_scopes(
        &middle_root,
        &["file:left/shared.rs", "tree:right"],
        SECOND_RUN,
        &first_id,
    );
    let middle_id = reservation_id(&middle);
    let first_edge = sequence(
        repository.path(),
        &first_id,
        &middle_id,
        "first before middle",
    );
    assert!(first_edge.status.success());
    let before = defer_claim(&before_root, "file:right/before.rs", THIRD_RUN, &middle_id);
    let before_id = reservation_id(&before);
    let before_edge = sequence(
        repository.path(),
        &middle_id,
        &before_id,
        "middle before the comparison successor",
    );
    assert!(before_edge.status.success());
    let ordinary_readiness = json_output(&before_edge)["payload"]["data"]["readiness"].clone();
    fs::remove_dir_all(&middle_root).expect("middle worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let orphaned = defer_claim(
        &orphaned_root,
        "file:right/orphaned.rs",
        FOURTH_RUN,
        &middle_id,
    );
    let orphaned_id = reservation_id(&orphaned);
    let orphaned_edge = sequence(
        repository.path(),
        &middle_id,
        &orphaned_id,
        "middle before the orphan-time successor",
    );
    assert!(orphaned_edge.status.success());
    assert_eq!(
        json_output(&orphaned_edge)["payload"]["data"]["readiness"],
        ordinary_readiness
    );

    let replacement_root = add_worktree(repository.path(), worktrees.path(), "replacement-middle");
    let recovered = run_berth(
        &replacement_root,
        &["resolve", &middle_id, "--recovered", "--json"],
    );
    assert!(recovered.status.success());
    let duplicate = sequence(
        repository.path(),
        &middle_id,
        &orphaned_id,
        "verify the retained outgoing edge",
    );
    assert_eq!(duplicate.status.code(), Some(2));
    assert_eq!(json_output(&duplicate)["status"], "duplicate_ordering_edge");

    let after_recovery = defer_claim(
        &recovered_root,
        "file:right/recovered.rs",
        FIFTH_RUN,
        &middle_id,
    );
    let after_recovery_id = reservation_id(&after_recovery);
    let recovered_edge = sequence(
        repository.path(),
        &middle_id,
        &after_recovery_id,
        "middle before the recovered successor",
    );
    assert!(recovered_edge.status.success());
    assert_eq!(
        json_output(&recovered_edge)["payload"]["data"]["readiness"],
        ordinary_readiness
    );
    let journal = journal_text(repository.path());
    assert!(!journal.contains(&format!(
        "\"before\":\"{first_id}\",\"after\":\"{orphaned_id}\""
    )));
}

#[test]
fn predecessor_liveness_observations_have_live_edge_readiness_parity() {
    let live = readiness_for_predecessor_liveness(ObservedPredecessorLiveness::Live);
    for worktree_liveness in [
        ObservedPredecessorLiveness::Unavailable,
        ObservedPredecessorLiveness::OrphanCandidate,
        ObservedPredecessorLiveness::Unknown,
    ] {
        assert_eq!(readiness_for_predecessor_liveness(worktree_liveness), live);
    }
}

#[test]
fn unavailable_predecessor_object_holds_instead_of_satisfying_the_edge() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    fs::write(
        predecessor_root.join("src/lib.rs"),
        "pub fn predecessor() {}\n",
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "."]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "unreachable predecessor"],
    );
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    let checkpoint = run_berth(&predecessor_root, &["release", &predecessor_id, "--json"]);
    assert!(checkpoint.status.success());
    git(
        repository.path(),
        &[
            "worktree",
            "remove",
            "--force",
            predecessor_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    git(repository.path(), &["branch", "-D", "predecessor"]);
    git(
        repository.path(),
        &["update-ref", "-d", &reservation_ref(&predecessor_id)],
    );
    git(
        repository.path(),
        &["reflog", "expire", "--expire=now", "--all"],
    );
    git(repository.path(), &["gc", "--prune=now"]);

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "missing evidence must keep holding",
    );
    assert!(sequence.status.success());
    assert_eq!(
        json_output(&sequence)["payload"]["data"]["readiness"],
        serde_json::json!({
            "state": "holding",
            "hold": {
                "reason": "predecessor_not_on_trunk",
                "evidence": "object_unknown"
            }
        })
    );
}

fn assert_claim_time_direction(flag: &str, direction: &str) {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let holder = claim(repository.path(), "tree:src", FIRST_RUN);
    let holder_id = reservation_id(&holder);
    let proposal = run_berth(
        &second_root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            flag,
            &holder_id,
            "--overlap-why",
            "the shared API needs an explicit order",
            "--why",
            "update the requester",
            "--json",
        ],
    );
    let proposal_token = proposal_token(&proposal);
    let applied = run_berth(
        &second_root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            flag,
            &holder_id,
            "--overlap-why",
            "the shared API needs an explicit order",
            "--why",
            "update the requester",
            "--proposal",
            &proposal_token,
            "--json",
        ],
    );
    assert!(applied.status.success());
    let event = last_journal_event(repository.path());
    assert_eq!(event["authorization"]["kind"], "sequence");
    assert_eq!(event["authorization"]["blocker"], holder_id);
    assert_eq!(event["authorization"]["direction"], direction);
    assert_ne!(
        event["authorization"]["edge_id"], event["event_id"],
        "edge and event identities have different roles"
    );
}

fn deferred_pair(holder_root: &Path, requester_root: &Path) -> (String, String) {
    let holder = claim(holder_root, "tree:src", FIRST_RUN);
    let holder_id = reservation_id(&holder);
    let proposal = run_berth(
        requester_root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            "--defer",
            &holder_id,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "update the requester",
            "--json",
        ],
    );
    let proposal_token = proposal_token(&proposal);
    let requester = run_berth(
        requester_root,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            "--defer",
            &holder_id,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "update the requester",
            "--proposal",
            &proposal_token,
            "--json",
        ],
    );
    assert!(requester.status.success());
    (holder_id, reservation_id(&requester))
}

/// Commit a `left/shared.rs` tree and a `right/` tree of three per-successor files.
fn commit_left_and_right_trees() -> TempDir {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    fs::create_dir_all(repository.path().join("left")).expect("left directory should exist");
    fs::create_dir_all(repository.path().join("right")).expect("right directory should exist");
    fs::write(repository.path().join("left/shared.rs"), "// left\n")
        .expect("left source should write");
    for file_name in ["before.rs", "orphaned.rs", "recovered.rs"] {
        fs::write(
            repository.path().join("right").join(file_name),
            "// right\n",
        )
        .expect("right source should write");
    }
    git(repository.path(), &["add", "."]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "shared paths"],
    );
    repository
}

fn sequence(repository_root: &Path, before: &str, after: &str, why: &str) -> Output {
    run_berth(
        repository_root,
        &["sequence", before, after, "--why", why, "--json"],
    )
}

fn defer_claim(repository_root: &Path, scope: &str, run: &str, blocker: &str) -> Output {
    defer_claim_scopes(repository_root, &[scope], run, blocker)
}

fn defer_claim_scopes(repository_root: &Path, scopes: &[&str], run: &str, blocker: &str) -> Output {
    let mut proposal_arguments = vec!["claim"];
    proposal_arguments.extend_from_slice(scopes);
    proposal_arguments.extend_from_slice(&[
        "--run",
        run,
        "--defer",
        blocker,
        "--overlap-why",
        "the order is not known yet",
        "--why",
        "protect deferred work",
        "--json",
    ]);
    let proposal = run_berth(repository_root, &proposal_arguments);
    let proposal_token = proposal_token(&proposal);
    let mut apply_arguments = vec!["claim"];
    apply_arguments.extend_from_slice(scopes);
    apply_arguments.extend_from_slice(&[
        "--run",
        run,
        "--defer",
        blocker,
        "--overlap-why",
        "the order is not known yet",
        "--why",
        "protect deferred work",
        "--proposal",
        &proposal_token,
        "--json",
    ]);
    run_berth(repository_root, &apply_arguments)
}

fn commit_configuration(repository_root: &Path) {
    git(repository_root, &["add", ".claude/config/berth.toml"]);
    git(
        repository_root,
        &["commit", "--quiet", "-m", "configure berth"],
    );
}

fn add_worktree(repository_root: &Path, parent: &Path, branch: &str) -> std::path::PathBuf {
    let worktree_root = parent.join(branch);
    git(
        repository_root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            worktree_root
                .to_str()
                .expect("worktree path should be UTF-8"),
        ],
    );
    worktree_root
}

fn linked_worktree_administrative_directory(worktree_root: &Path) -> PathBuf {
    let dot_git = fs::read_to_string(worktree_root.join(".git"))
        .expect("linked-worktree administrative pointer should read");
    let administrative_path = PathBuf::from(
        dot_git
            .trim()
            .strip_prefix("gitdir: ")
            .expect("linked worktree should name its administrative directory"),
    );
    if administrative_path.is_absolute() {
        administrative_path
    } else {
        worktree_root.join(administrative_path)
    }
}

fn readiness_for_predecessor_liveness(
    worktree_liveness: ObservedPredecessorLiveness,
) -> serde_json::Value {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor = claim(&predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);

    match worktree_liveness {
        ObservedPredecessorLiveness::Live => {},
        ObservedPredecessorLiveness::Unavailable => git(
            repository.path(),
            &[
                "worktree",
                "lock",
                predecessor_root
                    .to_str()
                    .expect("predecessor path should be UTF-8"),
            ],
        ),
        ObservedPredecessorLiveness::OrphanCandidate => {
            fs::remove_dir_all(&predecessor_root)
                .expect("predecessor worktree should be removable");
        },
        ObservedPredecessorLiveness::Unknown => {
            fs::remove_file(
                linked_worktree_administrative_directory(&predecessor_root)
                    .join("cargo-berth-worktree-id"),
            )
            .expect("predecessor identity should be removable");
        },
    }

    let sequence = sequence(
        repository.path(),
        &predecessor_id,
        &successor_id,
        "compare predecessor liveness",
    );
    let sequence_json = json_output(&sequence);
    assert!(sequence.status.success());
    assert_eq!(
        sequence_json["blocked_by"],
        serde_json::json!([predecessor_id])
    );
    sequence_json["payload"]["data"]["readiness"].clone()
}

fn reservation_ref(reservation_id: &str) -> String {
    format!("refs/cargo-berth/reservations/{reservation_id}")
}

fn reference_exists(repository_root: &Path, reference: &str) -> bool {
    Command::new("git")
        .arg("--no-optional-locks")
        .args(["show-ref", "--verify", "--quiet", reference])
        .current_dir(repository_root)
        .status()
        .expect("git show-ref should run")
        .success()
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg("--no-optional-locks")
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
    let repository = tempdir().expect("temporary repository should exist");
    git(
        repository.path(),
        &["init", "--quiet", "--initial-branch", "main"],
    );
    git(
        repository.path(),
        &["config", "user.email", "test@example.com"],
    );
    git(repository.path(), &["config", "user.name", "Test User"]);
    fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    fs::write(repository.path().join("src/lib.rs"), "pub fn value() {}\n")
        .expect("source should write");
    fs::create_dir_all(repository.path().join("tests")).expect("tests directory should exist");
    fs::write(repository.path().join("tests/base.rs"), "// test\n")
        .expect("test source should write");
    git(repository.path(), &["add", "."]);
    git(repository.path(), &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(repository.path(), &["init", "--json"]);
    assert!(initialized.status.success());
    repository
}

fn claim(repository_root: &Path, scope: &str, run: &str) -> Output {
    claim_scopes(repository_root, &[scope], run)
}

fn claim_scopes(repository_root: &Path, scopes: &[&str], run: &str) -> Output {
    let mut arguments = vec!["claim"];
    arguments.extend_from_slice(scopes);
    arguments.extend_from_slice(&["--run", run, "--why", "protect test work", "--json"]);
    run_berth(repository_root, &arguments)
}

fn set_config_limit(repository_root: &Path, key: &str, limit: u32) {
    let config_path = repository_root.join(".claude/config/berth.toml");
    let config = fs::read_to_string(&config_path).expect("configuration should read");
    let updated = config
        .lines()
        .map(|line| {
            if line.starts_with(key) {
                format!("{key} = {limit}")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(config_path, format!("{updated}\n")).expect("configuration should write");
}

fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should report a reservation id")
        .to_owned()
}

fn proposal_token(output: &Output) -> String {
    assert_eq!(output.status.code(), Some(3));
    json_output(output)["payload"]["data"]["proposal_token"]
        .as_str()
        .expect("proposal should report its token")
        .to_owned()
}

fn resolve_defer_count(repository_root: &Path) -> usize {
    journal_text(repository_root)
        .lines()
        .filter(|line| line.contains("\"op\":\"resolve_defer\""))
        .count()
}

fn journal_text(repository_root: &Path) -> String {
    fs::read_to_string(repository_root.join(JOURNAL_PATH)).expect("journal should read")
}

fn last_journal_event(repository_root: &Path) -> serde_json::Value {
    serde_json::from_str(
        journal_text(repository_root)
            .lines()
            .last()
            .expect("journal should contain an event"),
    )
    .expect("journal event should decode")
}

fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should print JSON")
}

fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_session(repository_root: &Path, arguments: &[&str], session_id: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

fn run_berth_with_stale_marker(
    repository_root: &Path,
    alert_branch: &str,
    arguments: &[&str],
) -> Output {
    let wrapper_directory = tempdir().expect("git wrapper directory should exist");
    let wrapper_path = wrapper_directory.path().join(GIT_BINARY);
    fs::write(&wrapper_path, STALE_MARKER_GIT_WRAPPER).expect("git wrapper should write");
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
    Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(ALERT_BRANCH_ENVIRONMENT, alert_branch)
        .env(MARKER_PATH_ENVIRONMENT, repository_root.join(MARKER_PATH))
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(STALE_RUN_ENVIRONMENT, FOURTH_RUN)
        .env_remove("CARGO_BERTH_RUN")
        .output()
        .expect("cargo-berth should run with wrapped git")
}

fn git_binary() -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").expect("test PATH should exist"))
        .map(|directory| directory.join(GIT_BINARY))
        .find(|candidate| candidate.is_file())
        .expect("git should exist on PATH")
}

fn git(repository_root: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .arg("--no-optional-locks")
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
