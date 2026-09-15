#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

//! Built-binary tests for ordering-edge replay, locked mutation, and limits.

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: no optional locks, with nothing held back from a hook.
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

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
const TRACE_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_TRACE";
const UNAVAILABLE_TARGET_ENVIRONMENT: &str = "CARGO_BERTH_TEST_UNAVAILABLE_TARGET";
const STALE_MARKER_GIT_WRAPPER: &str = r#"#!/bin/sh
if [ "$1" = "--no-optional-locks" ] && [ "$2" = "rev-parse" ] && [ "$3" = "$CARGO_BERTH_TEST_ALERT_BRANCH" ]; then
    printf '%s\n' "$CARGO_BERTH_TEST_STALE_RUN" > "$CARGO_BERTH_TEST_MARKER_PATH"
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;
const STALE_RUN_ENVIRONMENT: &str = "CARGO_BERTH_TEST_STALE_RUN";
const THIRD_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";
const TRACING_GIT_WRAPPER: &str = r#"#!/bin/sh

if [ "$1" = "--no-optional-locks" ]; then
    command_name="$2"
    (
        shift 2
        command_line="$command_name"
        for argument in "$@"; do command_line="$command_line $argument"; done
        printf '%s\n' "$command_line" >> "$CARGO_BERTH_TEST_GIT_TRACE"
    )
fi
if { [ "$2" = "merge-tree" ] || { [ "$2" = "rev-list" ] && [ "$3" = "--cherry-mark" ]; }; } \
    && [ -n "$CARGO_BERTH_TEST_UNAVAILABLE_TARGET" ]; then
    if [ "$CARGO_BERTH_TEST_UNAVAILABLE_TARGET" = "*" ]; then
        exit 2
    fi
    for argument in "$@"; do
        case "$argument" in
            --merge-base=*) ;;
            *"$CARGO_BERTH_TEST_UNAVAILABLE_TARGET") exit 2 ;;
        esac
    done
fi
exec "$CARGO_BERTH_TEST_REAL_GIT" "$@"
"#;

#[derive(Clone, Copy)]
enum AbandonedEndpoint {
    Predecessor,
    Successor,
}

struct EdgeReadinessFixture {
    repository:       TempDir,
    worktrees:        TempDir,
    base_head:        String,
    baseline_journal: Vec<u8>,
}

impl EdgeReadinessFixture {
    fn new() -> Self {
        let repository = initialized_repository();
        commit_configuration(repository.path());
        let base_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
        let baseline_journal =
            fs::read(repository.path().join(JOURNAL_PATH)).expect("initial journal should read");
        Self {
            repository,
            worktrees: tempdir().expect("worktree parent should exist"),
            base_head,
            baseline_journal,
        }
    }

    fn start_case(&self, case_index: usize) -> (PathBuf, PathBuf) {
        git(
            self.repository.path(),
            &["reset", "--hard", &self.base_head],
        );
        fs::write(
            self.repository.path().join(JOURNAL_PATH),
            &self.baseline_journal,
        )
        .expect("journal should restore before independent readiness case");
        let projection = self
            .repository
            .path()
            .join(".git/cargo-berth/reservations.json");
        if projection.exists() {
            fs::remove_file(projection).expect("prior projection should remove");
        }
        let predecessor_root = add_worktree(
            self.repository.path(),
            self.worktrees.path(),
            &format!("predecessor-{case_index}"),
        );
        let successor_root = add_worktree(
            self.repository.path(),
            self.worktrees.path(),
            &format!("successor-{case_index}"),
        );
        (predecessor_root, successor_root)
    }
}

#[derive(Clone, Copy)]
enum CheckpointReadiness {
    NotIntegrated,
    Incorporated,
    MissingObject,
}

struct CheckpointedPair {
    predecessor_root: PathBuf,
    successor_root:   PathBuf,
    predecessor_id:   String,
    successor_id:     String,
    journal:          Vec<u8>,
}

impl CheckpointedPair {
    fn new(fixture: &EdgeReadinessFixture) -> Self {
        let (predecessor_root, successor_root) = fixture.start_case(0);
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
        let predecessor_id = reservation_id(&claim(&predecessor_root, "tree:src", FIRST_RUN));
        let successor_id = reservation_id(&defer_claim(
            &successor_root,
            "file:src/lib.rs",
            SECOND_RUN,
            &predecessor_id,
        ));
        assert!(
            run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
                .status
                .success()
        );
        let journal = fs::read(fixture.repository.path().join(JOURNAL_PATH))
            .expect("checkpoint journal should read");
        Self {
            predecessor_root,
            successor_root,
            predecessor_id,
            successor_id,
            journal,
        }
    }
}

#[test]
fn checkpoint_readiness_covers_incorporation_and_missing_objects() {
    let fixture = EdgeReadinessFixture::new();
    let pair = CheckpointedPair::new(&fixture);
    for readiness in [
        CheckpointReadiness::NotIntegrated,
        CheckpointReadiness::Incorporated,
        CheckpointReadiness::MissingObject,
    ] {
        assert_checkpoint_transition(&fixture, &pair, readiness);
    }
}

fn assert_checkpoint_transition(
    fixture: &EdgeReadinessFixture,
    pair: &CheckpointedPair,
    readiness: CheckpointReadiness,
) {
    let expected = match readiness {
        CheckpointReadiness::NotIntegrated => {
            serde_json::json!({"state": "holding", "hold": {"reason": "predecessor_not_on_trunk", "evidence": "not_integrated"}})
        },
        CheckpointReadiness::Incorporated => {
            git(
                fixture.repository.path(),
                &["merge", "--quiet", "predecessor-0"],
            );
            git(&pair.successor_root, &["merge", "--quiet", "main"]);
            serde_json::json!({"state": "fulfilled"})
        },
        CheckpointReadiness::MissingObject => {
            let protected_tip =
                git_stdout(fixture.repository.path(), &["rev-parse", "predecessor-0"]);
            git(
                fixture.repository.path(),
                &["reset", "--hard", &fixture.base_head],
            );
            git(
                &pair.successor_root,
                &["reset", "--hard", &fixture.base_head],
            );
            git(
                fixture.repository.path(),
                &[
                    "worktree",
                    "remove",
                    "--force",
                    pair.predecessor_root
                        .to_str()
                        .expect("worktree path should be UTF-8"),
                ],
            );
            git(
                fixture.repository.path(),
                &["branch", "-D", "predecessor-0"],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", &reservation_ref(&pair.predecessor_id)],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", &reservation_ref(&pair.successor_id)],
            );
            git(
                fixture.repository.path(),
                &["update-ref", "-d", "ORIG_HEAD"],
            );
            git(&pair.successor_root, &["update-ref", "-d", "ORIG_HEAD"]);
            git(
                fixture.repository.path(),
                &["reflog", "expire", "--expire=now", "--all"],
            );
            git(fixture.repository.path(), &["gc", "--prune=now"]);
            assert!(
                !git_status(
                    fixture.repository.path(),
                    &["cat-file", "-e", &protected_tip]
                ),
                "predecessor object must be absent after pruning"
            );
            serde_json::json!({"state": "holding", "hold": {"reason": "predecessor_not_on_trunk", "evidence": "object_unknown"}})
        },
    };
    fs::write(fixture.repository.path().join(JOURNAL_PATH), &pair.journal)
        .expect("checkpoint journal should restore before each readiness observation");
    let projection = fixture
        .repository
        .path()
        .join(".git/cargo-berth/reservations.json");
    if projection.exists() {
        fs::remove_file(projection).expect("prior readiness projection should remove");
    }
    let sequence = sequence(
        fixture.repository.path(),
        &pair.predecessor_id,
        &pair.successor_id,
        "observe checkpoint readiness",
    );
    assert!(sequence.status.success());
    assert_eq!(
        json_output(&sequence)["payload"]["data"]["readiness"],
        expected
    );
}

#[test]
fn confirmed_abandonment_of_either_endpoint_cancels_the_edge() {
    let fixture = EdgeReadinessFixture::new();
    for (case_index, case) in [AbandonedEndpoint::Predecessor, AbandonedEndpoint::Successor]
        .into_iter()
        .enumerate()
    {
        assert_abandoned_endpoint(&fixture, case, case_index);
    }
}

fn assert_abandoned_endpoint(
    fixture: &EdgeReadinessFixture,
    case: AbandonedEndpoint,
    case_index: usize,
) {
    let (predecessor_root, successor_root) = fixture.start_case(case_index);
    match case {
        AbandonedEndpoint::Predecessor => {
            assert_predecessor_abandonment(fixture, &predecessor_root, &successor_root);
        },
        AbandonedEndpoint::Successor => {
            assert_successor_abandonment(fixture, &predecessor_root, &successor_root);
        },
    }
}

fn assert_predecessor_abandonment(
    fixture: &EdgeReadinessFixture,
    predecessor_root: &Path,
    successor_root: &Path,
) {
    dirty_source(predecessor_root, "src/lib.rs");
    let predecessor = claim(predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    fs::remove_dir_all(predecessor_root).expect("predecessor worktree should be removable");
    git(
        fixture.repository.path(),
        &["worktree", "prune", "--expire", "now"],
    );
    let abandoned = run_berth(
        fixture.repository.path(),
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
        fixture.repository.path(),
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

fn assert_successor_abandonment(
    fixture: &EdgeReadinessFixture,
    predecessor_root: &Path,
    successor_root: &Path,
) {
    dirty_source(predecessor_root, "src/lib.rs");
    let predecessor = claim(predecessor_root, "tree:src", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);
    fs::remove_dir_all(successor_root).expect("successor worktree should be removable");
    git(
        fixture.repository.path(),
        &["worktree", "prune", "--expire", "now"],
    );
    let abandoned = run_berth(
        fixture.repository.path(),
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
        fixture.repository.path(),
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

struct RewrittenSuccessorFixture {
    repository:       TempDir,
    worktrees:        TempDir,
    predecessor_id:   String,
    successor_id:     String,
    phase_start_head: String,
    protected_tip:    String,
    successor_head:   String,
}

struct SuccessorScaleFixture {
    repository:      TempDir,
    _worktrees:      TempDir,
    successor_heads: Vec<String>,
}

struct PredecessorScaleFixture {
    repository: TempDir,
    _worktrees: TempDir,
}

struct TracedBerth {
    output:     Output,
    trace_path: PathBuf,
    _directory: TempDir,
}

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
fn sequence_rejects_each_invalid_identity_and_carries_alerts() {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    let (holder_id, requester_id) = deferred_pair(repository.path(), &second_root);
    let worktrees = tempdir().expect("worktree parent should exist");
    let orphan_root = add_worktree(repository.path(), worktrees.path(), "orphan-alert");
    fs::write(orphan_root.join("orphan.txt"), "orphan work\n").expect("orphan source should write");
    git(&orphan_root, &["add", "."]);
    git(&orphan_root, &["commit", "--quiet", "-m", "orphan work"]);
    let orphan = claim(&orphan_root, "file:orphan.txt", FIFTH_RUN);
    let orphan_id = reservation_id(&orphan);
    assert!(
        run_berth(&orphan_root, &["release", &orphan_id, "--json"])
            .status
            .success()
    );
    fs::remove_dir_all(&orphan_root).expect("orphan worktree should be removable");
    git(repository.path(), &["worktree", "prune", "--expire", "now"]);

    let (_third_directory, third_root) = foreign_worktree(&repository, "third");
    let session_id = "foreign-sequence-session";
    let mapped_claim = run_berth_with_session(
        &third_root,
        &[
            "claim",
            "file:session-sequence",
            "--run",
            THIRD_RUN,
            "--why",
            "hold the mapped reservation in the third checkout",
            "--json",
        ],
        session_id,
    );
    assert!(mapped_claim.status.success());

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

    assert_eq!(rejected.status.code(), Some(5));
    assert_coordination_identity_rejection(
        &rejected_json,
        "session_worktree_mismatch",
        &["rerun_from_holding_worktree", "claim_separately_here"],
    );
    assert_eq!(resolve_defer_count(repository.path()), 0);
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(&third_root, &["release", &mapped_reservation_id, "--json"])
            .status
            .success()
    );
    // Complete automatic settlement before restoring a deliberately stale mapping.
    assert!(
        run_berth(repository.path(), &["board", "--json"])
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
    assert_coordination_identity_rejection(
        &rejected_json,
        "stale_session_mapping",
        &["clear_session_mapping"],
    );
    assert!(diagnostic.contains("Harness session mapping"));
    assert!(!diagnostic.contains("coordination-run marker"));
    assert_eq!(resolve_defer_count(repository.path()), 0);
    assert_sequence_stale_marker_rejection(
        repository.path(),
        &holder_id,
        &requester_id,
        &orphan_id,
    );
}

fn assert_sequence_stale_marker_rejection(
    repository_root: &Path,
    holder_id: &str,
    requester_id: &str,
    orphan_id: &str,
) {
    let rejected = run_berth_with_stale_marker(
        repository_root,
        "refs/heads/orphan-alert",
        &[
            "sequence",
            holder_id,
            requester_id,
            "--why",
            "the holder must land first",
            "--json",
        ],
    );
    let rejected_json = json_output(&rejected);

    assert_eq!(rejected.status.code(), Some(5));
    assert_eq!(rejected_json["exit_code"], 5);
    assert_eq!(rejected_json["status"], "invalid_input");
    assert_coordination_identity_rejection(
        &rejected_json,
        "stale_marker_run",
        &["reconcile_and_sweep_marker"],
    );
    assert_eq!(
        rejected_json["payload"]["data"]["coordination_run_id"],
        FOURTH_RUN
    );
    assert_eq!(rejected_json["blocked_by"], serde_json::json!([]));
    let orphan_alert = rejected_json["payload"]["alerts"]
        .as_array()
        .expect("rejection should carry alerts")
        .iter()
        .find(|alert| alert["kind"] == "orphaned_outstanding")
        .expect("rejection should retain the orphan alert alongside derivation failure");
    assert_eq!(orphan_alert["data"]["reservation_id"], orphan_id);
    assert_eq!(resolve_defer_count(repository_root), 0);
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
    // One run, because the limit is what this claim must run into. A second run in this
    // worktree is refused occupancy before any limit is consulted.
    let over_limit = claim(reservation_repository.path(), "tree:tests", FIRST_RUN);
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
fn rewritten_successor_content_is_cached_for_fulfilled_and_holding_edges() {
    for successor_is_equivalent in [true, false] {
        let fixture = rewritten_successor_fixture(successor_is_equivalent);
        retain_protected_tip_release(&fixture);
        assert!(!git_status(
            fixture.repository.path(),
            &[
                "merge-base",
                "--is-ancestor",
                &fixture.protected_tip,
                &fixture.successor_head,
            ],
        ));

        let first = run_berth_with_git_trace(
            fixture.repository.path(),
            &[
                "sequence",
                &fixture.predecessor_id,
                &fixture.successor_id,
                "--why",
                "predecessor content must reach the successor",
                "--json",
            ],
            "",
        );
        assert!(
            first.output.status.success(),
            "{}",
            String::from_utf8_lossy(&first.output.stdout)
        );
        assert_eq!(
            scoped_patch_comparisons_for_target(
                &first,
                &fixture.phase_start_head,
                &fixture.successor_head,
            ),
            1
        );
        let expected_readiness = if successor_is_equivalent {
            serde_json::json!({"state": "fulfilled"})
        } else {
            serde_json::json!({
                "state": "holding",
                "hold": {"reason": "awaiting_successor_incorporation"}
            })
        };
        assert_eq!(
            json_output(&first.output)["payload"]["data"]["readiness"],
            expected_readiness
        );
        assert_eq!(
            journal_operation_count(
                fixture.repository.path(),
                "successor_scoped_patch_equivalence_checked"
            ),
            1
        );
        let stable_journal_records = journal_record_count(fixture.repository.path());

        for _ in 0..2 {
            let replayed =
                run_berth_with_git_trace(fixture.repository.path(), &["board", "--json"], "");
            assert!(replayed.output.status.success());
            assert_eq!(
                scoped_patch_comparisons_for_target(
                    &replayed,
                    &fixture.phase_start_head,
                    &fixture.successor_head,
                ),
                0
            );
        }
        assert_eq!(
            journal_record_count(fixture.repository.path()),
            stable_journal_records
        );
    }
}

#[test]
fn legacy_negative_successor_verdict_retries_once_under_scoped_contiguity() {
    for successor_gap in [SuccessorGap::Unrelated, SuccessorGap::Protected] {
        let fixture = successor_with_separated_matches(successor_gap);
        let root = fixture.repository.path();
        retain_protected_tip_release(&fixture);
        append_edge_fixture_event(
            root,
            serde_json::json!({
                "op": "successor_scoped_patch_equivalence_checked",
                "predecessor_reservation_id": fixture.predecessor_id,
                "subject": 1, "successor_head": fixture.successor_head,
                "verdict": "different",
            }),
        );
        let first = run_berth_with_git_trace(
            root,
            &[
                "sequence",
                &fixture.predecessor_id,
                &fixture.successor_id,
                "--why",
                "retry a legacy successor proof under scoped contiguity",
                "--json",
            ],
            "",
        );
        assert!(
            first.output.status.success(),
            "{}",
            json_output(&first.output)
        );
        let expected = match successor_gap {
            SuccessorGap::Protected => {
                serde_json::json!({"state": "holding", "hold": {"reason": "awaiting_successor_incorporation"}})
            },
            SuccessorGap::Unrelated => serde_json::json!({"state": "fulfilled"}),
        };
        assert_eq!(
            json_output(&first.output)["payload"]["data"]["readiness"],
            expected
        );
        assert_eq!(successor_cherry_queries(&first, &fixture.successor_head), 1);
        let verdicts = successor_verdicts(root);
        assert_eq!(verdicts.len(), 2);
        assert!(verdicts[0].get("evaluator_version").is_none());
        assert_eq!(
            verdicts[1]["verdict"],
            match successor_gap {
                SuccessorGap::Protected => "different",
                SuccessorGap::Unrelated => "equivalent",
            }
        );
        assert_eq!(verdicts[1]["evaluator_version"], "historical_candidate");
        for _ in 0..2 {
            let restarted = run_berth_with_git_trace(root, &["board", "--json"], "");
            assert!(restarted.output.status.success());
            assert_eq!(
                successor_cherry_queries(&restarted, &fixture.successor_head),
                0
            );
            assert_eq!(scoped_patch_comparison_count(&restarted), 0);
            assert_eq!(successor_verdicts(root), verdicts);
        }
    }
}

/// The work a successor inserts between its two equivalent commits.
#[derive(Clone, Copy)]
enum SuccessorGap {
    Unrelated,
    Protected,
}

/// A two-commit phase whose successor inserts either unrelated or protected work between matches.
fn successor_with_separated_matches(successor_gap: SuccessorGap) -> RewrittenSuccessorFixture {
    let mut fixture = rewritten_successor_fixture(true);
    let root = fixture.repository.path();
    git(root, &["config", "core.hooksPath", "/dev/null"]);
    let predecessor_root = fixture.worktrees.path().join("predecessor");
    let successor_root = fixture.worktrees.path().join("successor");
    let old_tip = fixture.protected_tip.clone();
    commit_successor_fixture_file(
        &predecessor_root,
        "src/second.rs",
        "pub fn second() {}\n",
        "second phase commit",
    );
    fixture.protected_tip = git_stdout(&predecessor_root, &["rev-parse", "HEAD"]);
    let (gap, gap_content) = match successor_gap {
        SuccessorGap::Protected => (
            "src/lib.rs",
            "pub fn rewritten_predecessor() {}\npub fn intervening() {}\n",
        ),
        SuccessorGap::Unrelated => ("tests/gap.rs", "// intervening work\n"),
    };
    commit_successor_fixture_file(&successor_root, gap, gap_content, "gap between equivalents");
    commit_successor_fixture_file(
        &successor_root,
        "src/second.rs",
        "pub fn second() {}\n",
        "second equivalent",
    );
    fixture.successor_head = git_stdout(&successor_root, &["rev-parse", "HEAD"]);
    commit_successor_fixture_file(
        root,
        "src/second.rs",
        "pub fn second() {}\n",
        "complete phase on trunk",
    );
    // Assemble the old checkpoint directly, before any evaluator sees the two-commit subject.
    let journal = journal_text(root)
        .lines()
        .map(|line| {
            let mut event: serde_json::Value =
                serde_json::from_str(line).expect("event should decode");
            if event["reservation_id"] == fixture.predecessor_id {
                if event["op"] == "claim" {
                    event["scopes"] = serde_json::json!([{"path": "src", "kind": "tree"}]);
                }
                if event["protected_tip"] == old_tip {
                    event["protected_tip"] = serde_json::json!(fixture.protected_tip);
                }
            }
            event.to_string() + "\n"
        })
        .collect::<String>();
    fs::write(root.join(JOURNAL_PATH), journal).expect("two-commit checkpoint should write");
    fixture
}

/// Commit one file without letting the fixture's construction trigger reconciliation.
fn commit_successor_fixture_file(root: &Path, path: &str, content: &str, message: &str) {
    fs::write(root.join(path), content).expect("fixture source should write");
    git(root, &["add", path]);
    git(
        root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

/// Append a legacy event with valid journal envelope metadata.
fn append_edge_fixture_event(root: &Path, mut event: serde_json::Value) {
    let previous = last_journal_event(root);
    for field in ["schema_version", "actor", "at"] {
        event[field] = previous[field].clone();
    }
    event["event_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
    event["projection_generation"] = serde_json::json!(
        previous["projection_generation"]
            .as_u64()
            .expect("generation should exist")
            + 1
    );
    let mut journal = journal_text(root);
    journal.push_str(&event.to_string());
    journal.push('\n');
    fs::write(root.join(JOURNAL_PATH), journal).expect("legacy successor event should append");
}

/// Count the successor comparator even when a protected-path gap rejects before tree replay.
fn successor_cherry_queries(traced: &TracedBerth, head: &str) -> usize {
    git_trace(traced)
        .iter()
        .filter(|line| {
            line.starts_with("rev-list --cherry-mark --left-right ") && line.contains(head)
        })
        .count()
}

/// Read both the legacy and replacement successor verdicts after replay.
fn successor_verdicts(root: &Path) -> Vec<serde_json::Value> {
    journal_text(root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should decode"))
        .filter(|event| event["op"] == "successor_scoped_patch_equivalence_checked")
        .collect()
}

#[test]
fn witness_survives_pruning_and_controls_successors() {
    let fixture = rewritten_successor_fixture(true);
    let root = fixture.repository.path();
    append_edge_fixture_event(
        root,
        serde_json::json!({
            "op": "successor_scoped_patch_equivalence_checked",
            "predecessor_reservation_id": fixture.predecessor_id,
            "subject": 1, "successor_head": fixture.successor_head,
            "verdict": "different",
        }),
    );

    let witness = prepare_pruned_witness(&fixture);

    let revalidated = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert_witness_evidence(
        &revalidated,
        &fixture.predecessor_id,
        "integrated",
        LostEvidenceAlert::Absent,
    );
    assert_explicit_witness_evidence(root, &fixture.predecessor_id, "integrated", &witness);

    git(
        root,
        &[
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "advance beyond witness",
        ],
    );
    let evaluated_trunk = git_stdout(root, &["rev-parse", "HEAD"]);
    assert_ne!(evaluated_trunk, witness);
    let advanced = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert_witness_evidence(
        &advanced,
        &fixture.predecessor_id,
        "integrated",
        LostEvidenceAlert::Absent,
    );
    let advanced_board = json_output(&advanced.output);
    let predecessor = witness_reservation_snapshot(&advanced_board, &fixture.predecessor_id);
    assert_eq!(
        predecessor["lifecycle"]["disposition"],
        serde_json::json!({"kind": "rewritten_integration", "evidence": witness})
    );
    assert_eq!(
        predecessor["integration_evidence"]["status"],
        serde_json::json!({
            "status": "integrated",
            "trunk_oid": evaluated_trunk,
            "proof": "rewritten_witness_ancestor",
            "witness": {"kind": "historical", "commit": witness},
        })
    );
    assert_explicit_witness_evidence(root, &fixture.predecessor_id, "integrated", &witness);

    let holding = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert!(holding.output.status.success());
    assert_eq!(scoped_patch_comparison_count(&holding), 0);
    assert_eq!(
        successor_cherry_queries(&holding, &fixture.successor_head),
        0
    );
    assert_eq!(successor_verdicts(root).len(), 1);
    assert_successor_round_robin_progress(&holding.output, 1, 0);

    let successor_root = fixture.worktrees.path().join("successor");
    git(&successor_root, &["reset", "--hard", &witness]);
    let fulfilled = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert_witness_evidence(
        &fulfilled,
        &fixture.predecessor_id,
        "integrated",
        LostEvidenceAlert::Absent,
    );
    assert_successor_round_robin_progress(&fulfilled.output, 1, 1);

    // Equivalent replacement content cannot reaffirm a witness removed from actual trunk.
    git(root, &["reset", "--hard", &fixture.successor_head]);
    let lost = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert_witness_evidence(
        &lost,
        &fixture.predecessor_id,
        "trunk_rewritten",
        LostEvidenceAlert::Raised,
    );
    assert_explicit_witness_evidence(root, &fixture.predecessor_id, "trunk_rewritten", &witness);
    git(root, &["reset", "--hard", &witness]);
    let recovered = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert_witness_evidence(
        &recovered,
        &fixture.predecessor_id,
        "integrated",
        LostEvidenceAlert::Absent,
    );
    assert_successor_round_robin_progress(&recovered.output, 1, 1);
    assert!(!git_status(
        root,
        &["cat-file", "-e", &fixture.protected_tip]
    ));
    assert_eq!(journal_operation_count(root, "release"), 1);
}

/// Explicit release revalidates the same surviving witness without consulting the old phase.
fn assert_explicit_witness_evidence(
    root: &Path,
    reservation_id: &str,
    status: &str,
    witness: &str,
) {
    let released = run_berth_with_git_trace(root, &["release", reservation_id, "--json"], "");
    assert!(released.output.status.success());
    assert_eq!(scoped_patch_comparison_count(&released), 0);
    let output = json_output(&released.output);
    assert_eq!(output["payload"]["data"]["status"], "already_settled");
    assert_eq!(output["payload"]["data"]["evidence"]["status"], status);
    if status == "integrated" {
        assert_eq!(
            output["payload"]["data"]["evidence"],
            serde_json::json!({
                "status": "integrated",
                "trunk_oid": git_stdout(root, &["rev-parse", "HEAD"]),
                "proof": "rewritten_witness_ancestor",
                "witness": {"kind": "historical", "commit": witness},
            })
        );
    }
}

/// Rebase the holder, settle against its trunk witness, and collect the original protected tip.
fn prepare_pruned_witness(fixture: &RewrittenSuccessorFixture) -> String {
    let root = fixture.repository.path();
    assert_eq!(journal_operation_count(root, "release"), 0);
    git(root, &["config", "core.hooksPath", "/dev/null"]);
    let witness = git_stdout(root, &["rev-parse", "HEAD"]);
    assert_ne!(witness, fixture.protected_tip);
    assert_ne!(witness, fixture.successor_head);
    assert!(!git_status(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &fixture.protected_tip,
            &witness
        ]
    ));
    assert!(!git_status(
        root,
        &[
            "merge-base",
            "--is-ancestor",
            &witness,
            &fixture.successor_head
        ]
    ));
    assert_eq!(
        git_stdout(root, &["rev-parse", &format!("{witness}^{{tree}}")]),
        git_stdout(
            root,
            &["rev-parse", &format!("{}^{{tree}}", fixture.successor_head)]
        ),
        "the successor has equivalent content without the recorded witness"
    );

    // Establish the dependency before trunk integration so settlement must retain its witness.
    git(root, &["reset", "--hard", &fixture.phase_start_head]);
    let ordered = sequence(
        root,
        &fixture.predecessor_id,
        &fixture.successor_id,
        "the successor must contain the integration witness",
    );
    assert!(ordered.status.success(), "{}", json_output(&ordered));
    git(root, &["reset", "--hard", &witness]);

    // Model the holder after its branch was rebased onto the equivalent trunk commit.
    let predecessor_root = fixture.worktrees.path().join("predecessor");
    git(&predecessor_root, &["reset", "--hard", &witness]);
    commit_successor_fixture_file(
        root,
        "src/lib.rs",
        "pub fn later_trunk_work() {}\n",
        "rewrite the integrated hunk after its witness",
    );
    let evaluated_trunk = git_stdout(root, &["rev-parse", "HEAD"]);
    assert_ne!(evaluated_trunk, witness);
    assert_equivalence_settlement(fixture, &witness);
    assert_eq!(
        git_stdout(
            root,
            &["rev-parse", &reservation_ref(&fixture.predecessor_id)]
        ),
        witness
    );

    // Remove every ref and reflog retaining the old tip, then prove actual collection.
    git(
        &predecessor_root,
        &["reset", "--hard", &fixture.phase_start_head],
    );
    git(
        root,
        &[
            "update-ref",
            "-d",
            &reservation_ref(&fixture.predecessor_id),
        ],
    );
    git(root, &["reflog", "expire", "--expire=now", "--all"]);
    git(root, &["gc", "--prune=now"]);
    assert!(!git_status(
        root,
        &["cat-file", "-e", &fixture.protected_tip]
    ));
    assert!(git_status(root, &["cat-file", "-e", &witness]));
    witness
}

/// Settlement records the affirmative scoped proof directly before its witness disposition.
fn assert_equivalence_settlement(fixture: &RewrittenSuccessorFixture, witness: &str) {
    let root = fixture.repository.path();
    let settled = run_berth_with_git_trace(root, &["board", "--json"], "");
    assert!(settled.output.status.success());
    let settled_board = json_output(&settled.output);
    let predecessor = witness_reservation_snapshot(&settled_board, &fixture.predecessor_id);
    assert_eq!(predecessor["lifecycle"]["stage"], "released");
    assert_eq!(
        predecessor["lifecycle"]["disposition"],
        serde_json::json!({"kind": "rewritten_integration", "evidence": witness})
    );
    assert_eq!(
        predecessor["integration_evidence"]["status"]["status"],
        "integrated"
    );
    assert_eq!(
        predecessor["integration_evidence"]["status"]["proof"],
        "scoped_patch_equivalent"
    );
    assert!(
        settled_board["payload"]["data"]["alerts"]["entries"]
            .as_array()
            .expect("board alerts should exist")
            .iter()
            .all(|alert| alert["reservation_id"] != fixture.predecessor_id)
    );
    let events = journal_text(root)
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("event should decode"))
        .collect::<Vec<_>>();
    let release_index = events
        .iter()
        .position(|event| {
            event["op"] == "release" && event["reservation_id"] == fixture.predecessor_id
        })
        .expect("equivalence must append a release");
    let evidence = &events[release_index
        .checked_sub(1)
        .expect("release must follow evidence")];
    assert_eq!(evidence["op"], "evidence_revalidated");
    assert_eq!(evidence["reservation_id"], fixture.predecessor_id);
    assert_eq!(evidence["status"]["proof"], "scoped_patch_equivalent");
    assert_eq!(
        evidence["status"]["trunk_oid"],
        git_stdout(root, &["rev-parse", "HEAD"])
    );
    assert_ne!(evidence["status"]["trunk_oid"], witness);
    assert_eq!(
        evidence["status"]["witness"],
        serde_json::json!({"kind": "historical", "commit": witness})
    );
}

/// Find a settled predecessor in the board's durable reservation snapshots.
fn witness_reservation_snapshot<'board>(
    board: &'board serde_json::Value,
    reservation_id: &str,
) -> &'board serde_json::Value {
    ["ready_now", "unconstrained_reservations", "resolved"]
        .into_iter()
        .flat_map(|section| {
            board["payload"]["data"][section]["entries"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|snapshot| snapshot["reservation_id"] == reservation_id)
        .expect("predecessor should have a board snapshot")
}

/// Whether the board must report a lost-integration-evidence alert for the reservation.
#[derive(Clone, Copy, Eq, PartialEq)]
enum LostEvidenceAlert {
    Absent,
    Raised,
}

/// Revalidation remains ancestry-only even when the original protected object is absent.
fn assert_witness_evidence(
    traced: &TracedBerth,
    reservation_id: &str,
    status: &str,
    lost_evidence_alert: LostEvidenceAlert,
) {
    assert!(
        traced.output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&traced.output.stdout),
        String::from_utf8_lossy(&traced.output.stderr)
    );
    assert_eq!(
        scoped_patch_comparison_count(traced),
        0,
        "witness revalidation must never replay patches"
    );
    let board = json_output(&traced.output);
    let snapshot = witness_reservation_snapshot(&board, reservation_id);
    assert_eq!(snapshot["lifecycle"]["stage"], "released");
    assert_eq!(snapshot["integration_evidence"]["status"]["status"], status);
    if status == "integrated" {
        assert_eq!(
            snapshot["integration_evidence"]["status"]["proof"],
            "rewritten_witness_ancestor"
        );
        assert_eq!(
            snapshot["integration_evidence"]["status"]["witness"],
            serde_json::json!({
                "kind": "historical",
                "commit": snapshot["lifecycle"]["disposition"]["evidence"],
            })
        );
    }
    let alerts = board["payload"]["data"]["alerts"]["entries"]
        .as_array()
        .expect("board alerts should exist");
    assert_eq!(
        alerts.iter().any(|alert| {
            alert["kind"] == "lost_integration_evidence"
                && alert["reservation_id"] == reservation_id
        }),
        lost_evidence_alert == LostEvidenceAlert::Raised
    );
    assert!(alerts.iter().all(|alert| {
        alert["kind"] != "orphaned_outstanding" || alert["reservation_id"] != reservation_id
    }));
}

#[test]
fn unavailable_successor_comparison_is_retried_instead_of_cached() {
    let fixture = rewritten_successor_fixture(false);
    retain_protected_tip_release(&fixture);
    let unavailable = run_berth_with_git_trace(
        fixture.repository.path(),
        &[
            "sequence",
            &fixture.predecessor_id,
            &fixture.successor_id,
            "--why",
            "transient successor evidence remains pending",
            "--json",
        ],
        &fixture.successor_head,
    );
    assert!(
        unavailable.output.status.success(),
        "{}",
        String::from_utf8_lossy(&unavailable.output.stdout)
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "successor_scoped_patch_comparison_attempted"
        ),
        1,
        "unavailable successor argv: {:?}",
        git_trace(&unavailable),
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "successor_scoped_patch_equivalence_checked"
        ),
        0
    );

    let retried = run_berth_with_git_trace(fixture.repository.path(), &["board", "--json"], "");
    assert!(retried.output.status.success());
    assert_eq!(
        scoped_patch_comparisons_for_target(
            &retried,
            &fixture.phase_start_head,
            &fixture.successor_head,
        ),
        1
    );
    assert_eq!(
        journal_operation_count(
            fixture.repository.path(),
            "successor_scoped_patch_equivalence_checked"
        ),
        1
    );
}

#[test]
fn successor_round_robin_has_fixed_cold_cost_and_covers_every_head() {
    let one = successor_scale_fixture(1);
    let one_cold = run_berth_with_git_trace(one.repository.path(), &["board", "--json"], "*");
    assert!(one_cold.output.status.success());
    let one_argv = git_trace(&one_cold);

    let four = successor_scale_fixture(4);
    let four_cold = run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "*");
    assert!(four_cold.output.status.success());
    let four_argv = git_trace(&four_cold);
    assert!(!one_argv.is_empty());
    assert_merge_observation_budget(&one_argv, 2);
    assert_merge_observation_budget(&four_argv, 5);
    assert_eq!(
        canonical_git_command_sequence(&four_argv).len(),
        canonical_git_command_sequence(&one_argv).len(),
        "one successor argv: {one_argv:?}; four successor argv: {four_argv:?}"
    );
    assert_eq!(
        canonical_git_command_sequence(&four_argv),
        canonical_git_command_sequence(&one_argv)
    );

    let mut latest_unavailable = four_cold;
    for _ in 1..four.successor_heads.len() {
        latest_unavailable =
            run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "*");
        assert!(latest_unavailable.output.status.success());
    }
    let attempted_heads = journal_operation_field_values(
        four.repository.path(),
        "successor_scoped_patch_comparison_attempted",
        "successor_head",
    );
    assert_eq!(attempted_heads.len(), four.successor_heads.len());
    for successor_head in &four.successor_heads {
        assert!(attempted_heads.contains(successor_head));
    }
    let unavailable_board =
        String::from_utf8(latest_unavailable.output.stdout).expect("board output should be UTF-8");
    assert!(unavailable_board.contains("successor_must_incorporate_predecessor"));
    assert!(!unavailable_board.contains("\"state\":\"fulfilled\""));

    let available_comparison_count = four.successor_heads.len() / 2;
    let mut partially_checked =
        run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "");
    assert!(partially_checked.output.status.success());
    for _ in 1..available_comparison_count {
        partially_checked =
            run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "");
        assert!(partially_checked.output.status.success());
    }
    assert_successor_round_robin_progress(
        &partially_checked.output,
        four.successor_heads.len(),
        available_comparison_count,
    );

    for _ in available_comparison_count..four.successor_heads.len() {
        let remaining = run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "");
        assert!(remaining.output.status.success());
    }
    let checked_heads = journal_operation_field_values(
        four.repository.path(),
        "successor_scoped_patch_equivalence_checked",
        "successor_head",
    );
    assert_eq!(checked_heads.len(), four.successor_heads.len());
    for successor_head in &four.successor_heads {
        assert!(checked_heads.contains(successor_head));
    }
    let stable_journal_records = journal_record_count(four.repository.path());
    for _ in 0..2 {
        let cached = run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "");
        assert!(cached.output.status.success());
        assert_eq!(
            scoped_patch_comparison_count(&cached),
            0,
            "every unchanged successor head should read from its durable cache"
        );
    }
    assert_eq!(
        journal_record_count(four.repository.path()),
        stable_journal_records
    );
}

#[test]
fn predecessor_graph_has_fixed_cold_cost() {
    let two = predecessor_scale_fixture(2);
    let two_cold = run_berth_with_git_trace(two.repository.path(), &["board", "--json"], "*");
    assert!(two_cold.output.status.success());
    let two_argv = git_trace(&two_cold);

    let four = predecessor_scale_fixture(4);
    let four_cold = run_berth_with_git_trace(four.repository.path(), &["board", "--json"], "*");
    assert!(four_cold.output.status.success());
    let four_argv = git_trace(&four_cold);
    assert!(!two_argv.is_empty());
    assert_merge_observation_budget(&two_argv, 3);
    assert_merge_observation_budget(&four_argv, 5);
    assert_eq!(
        canonical_git_command_sequence(&four_argv).len(),
        canonical_git_command_sequence(&two_argv).len(),
        "two predecessor argv: {two_argv:?}; four predecessor argv: {four_argv:?}"
    );
    assert_eq!(
        canonical_git_command_sequence(&four_argv),
        canonical_git_command_sequence(&two_argv)
    );
}

/// Derivation scales once per holder checkout, independently of the fixed ancestry batch.
fn assert_merge_observation_budget(queries: &[String], worktrees: usize) {
    assert_eq!(
        queries
            .iter()
            .filter(|query| query.starts_with("status "))
            .count(),
        worktrees
    );
    assert!(
        queries
            .iter()
            .filter(|query| query.starts_with("diff --merge-base "))
            .count()
            <= worktrees
    );
}

fn canonical_git_command_sequence(invocations: &[String]) -> Vec<&str> {
    let mut commands = invocations
        .iter()
        .filter(|line| !line.starts_with("status ") && !line.starts_with("diff --merge-base "))
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    commands.sort_unstable();
    commands
}

fn assert_successor_round_robin_progress(
    output: &Output,
    successor_count: usize,
    compared_count: usize,
) {
    let board = json_output(output);
    let data = &board["payload"]["data"];
    let waiting = data["waiting"]["entries"]
        .as_array()
        .expect("waiting entries should be an array");
    assert_eq!(waiting.len(), successor_count - compared_count);
    assert!(
        waiting
            .iter()
            .all(|entry| { entry["action"]["reason"] == "successor_must_incorporate_predecessor" })
    );
    let settled = data["settled_ordering_constraints"]["entries"]
        .as_array()
        .expect("settled entries should be an array");
    assert_eq!(settled.len(), compared_count);
    assert!(
        settled
            .iter()
            .all(|entry| { entry["settlement"] == "fulfilled_successor_contains_predecessor" })
    );
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
    dirty_source(repository.path(), "tests/repair.rs");
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
    dirty_source(&first_root, "left/shared.rs");
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
    commit_middle_work(&middle_root);
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
    git(
        &replacement_root,
        &["merge", "--quiet", "--ff-only", "middle"],
    );
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
    let fixture = EdgeReadinessFixture::new();
    let (predecessor_root, successor_root) = fixture.start_case(0);
    dirty_source(&predecessor_root, "src/lib.rs");
    let predecessor_id = reservation_id(&claim(&predecessor_root, "tree:src", FIRST_RUN));
    let successor_id = reservation_id(&defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    ));
    let identity_path =
        linked_worktree_administrative_directory(&predecessor_root).join("cargo-berth-worktree-id");
    let baseline_journal =
        fs::read(fixture.repository.path().join(JOURNAL_PATH)).expect("pair journal should read");
    let identity = fs::read(&identity_path).expect("worktree identity should read");
    let live = readiness_for_predecessor_liveness(
        &fixture,
        &predecessor_root,
        &predecessor_id,
        &successor_id,
        ObservedPredecessorLiveness::Live,
    );
    for worktree_liveness in [
        ObservedPredecessorLiveness::Unavailable,
        ObservedPredecessorLiveness::Unknown,
        ObservedPredecessorLiveness::OrphanCandidate,
    ] {
        fs::write(
            fixture.repository.path().join(JOURNAL_PATH),
            &baseline_journal,
        )
        .expect("pair journal should restore before each liveness observation");
        fs::remove_file(
            fixture
                .repository
                .path()
                .join(".git/cargo-berth/reservations.json"),
        )
        .expect("sequence projection should remove");
        assert_eq!(
            readiness_for_predecessor_liveness(
                &fixture,
                &predecessor_root,
                &predecessor_id,
                &successor_id,
                worktree_liveness
            ),
            live
        );
        match worktree_liveness {
            ObservedPredecessorLiveness::Unavailable => git(
                fixture.repository.path(),
                &[
                    "worktree",
                    "unlock",
                    predecessor_root
                        .to_str()
                        .expect("worktree path should be UTF-8"),
                ],
            ),
            ObservedPredecessorLiveness::Unknown => {
                fs::write(&identity_path, &identity).expect("worktree identity should restore");
            },
            ObservedPredecessorLiveness::Live | ObservedPredecessorLiveness::OrphanCandidate => {},
        }
    }
}

fn assert_claim_time_direction(flag: &str, direction: &str) {
    let repository = initialized_repository();
    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    dirty_source(repository.path(), "src/lib.rs");
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
    dirty_source(holder_root, "src/lib.rs");
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

fn rewritten_successor_fixture(successor_is_equivalent: bool) -> RewrittenSuccessorFixture {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    let phase_start_head = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    dirty_source(&predecessor_root, "src/lib.rs");
    let predecessor = claim(&predecessor_root, "file:src/lib.rs", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let successor = defer_claim(
        &successor_root,
        "file:src/lib.rs",
        SECOND_RUN,
        &predecessor_id,
    );
    let successor_id = reservation_id(&successor);

    let rewritten_content = "pub fn rewritten_predecessor() {}\n";
    fs::write(predecessor_root.join("src/lib.rs"), rewritten_content)
        .expect("predecessor source should write");
    git(&predecessor_root, &["add", "src/lib.rs"]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "protected predecessor"],
    );
    let protected_tip = git_stdout(&predecessor_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );

    if successor_is_equivalent {
        fs::write(successor_root.join("src/lib.rs"), rewritten_content)
            .expect("equivalent successor source should write");
        git(&successor_root, &["add", "src/lib.rs"]);
    } else {
        fs::write(
            successor_root.join("tests/successor.rs"),
            "// unrelated successor work\n",
        )
        .expect("unrelated successor source should write");
        git(&successor_root, &["add", "tests/successor.rs"]);
    }
    git(
        &successor_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "successor work",
        ],
    );
    let successor_head = git_stdout(&successor_root, &["rev-parse", "HEAD"]);

    // Integrate last without hooks so the command under test owns the first settlement.
    fs::write(repository.path().join("src/lib.rs"), rewritten_content)
        .expect("rewritten trunk source should write");
    git(repository.path(), &["add", "src/lib.rs"]);
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "rewritten trunk integration",
        ],
    );

    RewrittenSuccessorFixture {
        repository,
        worktrees,
        predecessor_id,
        successor_id,
        phase_start_head,
        protected_tip,
        successor_head,
    }
}

/// The middle holder retains its real outgoing work while its checkout is absent.
fn commit_middle_work(middle_root: &Path) {
    for path in ["right/before.rs", "right/orphaned.rs", "right/recovered.rs"] {
        dirty_source(middle_root, path);
    }
    git(middle_root, &["add", "right"]);
    git(
        middle_root,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "middle work",
        ],
    );
}

/// Exercise legacy protected-tip cache subjects independently of automatic rewritten witnesses.
fn retain_protected_tip_release(fixture: &RewrittenSuccessorFixture) {
    let root = fixture.repository.path();
    assert_eq!(journal_operation_count(root, "release"), 0);
    let previous = last_journal_event(root);
    let mut generation = previous["projection_generation"]
        .as_u64()
        .expect("generation should be numeric");
    let mut journal = journal_text(root);
    for mut event in [
        serde_json::json!({
            "op": "evidence_revalidated",
            "reservation_id": fixture.predecessor_id,
            "status": {"status": "integrated", "trunk_oid": git_stdout(root, &["rev-parse", "HEAD"]), "proof": "scoped_patch_equivalent"},
            "edit_blocking_status": "clear",
        }),
        serde_json::json!({
            "op": "release",
            "reservation_id": fixture.predecessor_id,
            "disposition": {"kind": "integrated"},
        }),
    ] {
        generation += 1;
        for field in ["schema_version", "actor", "at"] {
            event[field] = previous[field].clone();
        }
        event["event_id"] = serde_json::json!(uuid::Uuid::now_v7().to_string());
        event["projection_generation"] = serde_json::json!(generation);
        journal.push_str(&event.to_string());
        journal.push('\n');
    }
    fs::write(root.join(JOURNAL_PATH), journal).expect("legacy proof and release should append");
}

/// Start successor scale fixtures with the same tracked scope files.
fn successor_scope_repository(successor_count: usize) -> TempDir {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    fs::create_dir_all(repository.path().join("successors"))
        .expect("successor scope directory should exist");
    for successor_index in 0..successor_count {
        fs::write(
            repository
                .path()
                .join(format!("successors/successor-{successor_index}.rs")),
            format!("// reserved successor scope {successor_index}\n"),
        )
        .expect("successor scope source should write");
    }
    git(repository.path(), &["add", "successors"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "successor reservation scopes"],
    );
    repository
}

fn successor_scale_fixture(successor_count: usize) -> SuccessorScaleFixture {
    let repository = successor_scope_repository(successor_count);
    let worktrees = tempdir().expect("worktree parent should exist");
    let predecessor_root = add_worktree(repository.path(), worktrees.path(), "predecessor");
    for index in 0..successor_count {
        dirty_source(
            &predecessor_root,
            &format!("successors/successor-{index}.rs"),
        );
    }
    let predecessor = claim(&predecessor_root, "tree:successors", FIRST_RUN);
    let predecessor_id = reservation_id(&predecessor);
    let protected_content = "pub fn protected_predecessor() {}\n";
    let mut successor_heads = Vec::new();
    for successor_index in 0..successor_count {
        let successor_root = add_worktree(
            repository.path(),
            worktrees.path(),
            &format!("successor-{successor_index}"),
        );
        let successor_path = format!("successors/successor-{successor_index}.rs");
        fs::write(
            successor_root.join("successors/protected.rs"),
            protected_content,
        )
        .expect("successor source should write");
        git(&successor_root, &["add", "successors/protected.rs"]);
        git(
            &successor_root,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("successor {successor_index}"),
            ],
        );
        successor_heads.push(git_stdout(&successor_root, &["rev-parse", "HEAD"]));
        let successor_run = uuid::Uuid::now_v7().to_string();
        let successor = defer_claim(
            &successor_root,
            &format!("file:{successor_path}"),
            &successor_run,
            &predecessor_id,
        );
        let successor_id = reservation_id(&successor);
        let sequenced = sequence(
            repository.path(),
            &predecessor_id,
            &successor_id,
            "predecessor lands before this successor",
        );
        assert!(sequenced.status.success());
    }

    fs::create_dir_all(predecessor_root.join("successors"))
        .expect("predecessor source directory should exist");
    fs::write(
        predecessor_root.join("successors/protected.rs"),
        protected_content,
    )
    .expect("predecessor source should write");
    git(&predecessor_root, &["add", "successors/protected.rs"]);
    git(
        &predecessor_root,
        &["commit", "--quiet", "-m", "protected predecessor"],
    );
    assert!(
        run_berth(&predecessor_root, &["release", &predecessor_id, "--json"])
            .status
            .success()
    );
    git(
        repository.path(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--quiet",
            "predecessor",
        ],
    );

    SuccessorScaleFixture {
        repository,
        _worktrees: worktrees,
        successor_heads,
    }
}

fn predecessor_scale_fixture(predecessor_count: usize) -> PredecessorScaleFixture {
    let repository = initialized_repository();
    commit_configuration(repository.path());
    fs::create_dir_all(repository.path().join("predecessors"))
        .expect("predecessor scope directory should exist");
    for predecessor_index in 0..predecessor_count {
        fs::write(
            repository
                .path()
                .join(format!("predecessors/predecessor-{predecessor_index}.rs")),
            format!("// reserved predecessor scope {predecessor_index}\n"),
        )
        .expect("predecessor scope source should write");
    }
    git(repository.path(), &["add", "predecessors"]);
    git(
        repository.path(),
        &["commit", "--quiet", "-m", "predecessor reservation scopes"],
    );
    let worktrees = tempdir().expect("worktree parent should exist");
    let successor_root = add_worktree(repository.path(), worktrees.path(), "successor");
    let predecessor_worktrees = (0..predecessor_count)
        .map(|predecessor_index| {
            let predecessor_branch = format!("predecessor-{predecessor_index}");
            let predecessor_root =
                add_worktree(repository.path(), worktrees.path(), &predecessor_branch);
            (predecessor_branch, predecessor_root)
        })
        .collect::<Vec<_>>();

    for (predecessor_index, (_, predecessor_root)) in predecessor_worktrees.iter().enumerate() {
        let predecessor_path = format!("predecessors/predecessor-{predecessor_index}.rs");
        dirty_source(predecessor_root, &predecessor_path);
        let predecessor_run = uuid::Uuid::now_v7().to_string();
        let predecessor = claim(
            predecessor_root,
            &format!("file:{predecessor_path}"),
            &predecessor_run,
        );
        let predecessor_id = reservation_id(&predecessor);
        let successor = defer_claim(
            &successor_root,
            &format!("file:{predecessor_path}"),
            SECOND_RUN,
            &predecessor_id,
        );
        let successor_id = reservation_id(&successor);
        let sequenced = sequence(
            repository.path(),
            &predecessor_id,
            &successor_id,
            "predecessor lands before this successor",
        );
        assert!(sequenced.status.success());

        fs::write(
            predecessor_root.join(&predecessor_path),
            format!("pub fn protected_predecessor_{predecessor_index}() {{}}\n"),
        )
        .expect("predecessor source should write");
        git(predecessor_root, &["add", &predecessor_path]);
        git(
            predecessor_root,
            &[
                "commit",
                "--quiet",
                "-m",
                &format!("protected predecessor {predecessor_index}"),
            ],
        );
        assert!(
            run_berth(predecessor_root, &["release", &predecessor_id, "--json"])
                .status
                .success()
        );
    }

    // Observe every cold predecessor together, before ordinary reconciliation can settle one.
    for (predecessor_branch, _) in &predecessor_worktrees {
        git(
            repository.path(),
            &[
                "-c",
                "core.hooksPath=/dev/null",
                "merge",
                "--quiet",
                "--no-ff",
                predecessor_branch.as_str(),
            ],
        );
    }

    PredecessorScaleFixture {
        repository,
        _worktrees: worktrees,
    }
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
    fixture: &EdgeReadinessFixture,
    predecessor_root: &Path,
    predecessor_id: &str,
    successor_id: &str,
    worktree_liveness: ObservedPredecessorLiveness,
) -> serde_json::Value {
    match worktree_liveness {
        ObservedPredecessorLiveness::Live => {},
        ObservedPredecessorLiveness::Unavailable => git(
            fixture.repository.path(),
            &[
                "worktree",
                "lock",
                predecessor_root
                    .to_str()
                    .expect("predecessor path should be UTF-8"),
            ],
        ),
        ObservedPredecessorLiveness::OrphanCandidate => {
            fs::remove_dir_all(predecessor_root).expect("predecessor worktree should be removable");
        },
        ObservedPredecessorLiveness::Unknown => {
            fs::remove_file(
                linked_worktree_administrative_directory(predecessor_root)
                    .join("cargo-berth-worktree-id"),
            )
            .expect("predecessor identity should be removable");
        },
    }

    let sequence = sequence(
        fixture.repository.path(),
        predecessor_id,
        successor_id,
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
    GIT.succeeds(
        repository_root,
        &["show-ref", "--verify", "--quiet", reference],
    )
}

fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
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

/// A declared overlap needs actual uncommitted work in the holder's branch.
fn dirty_source(root: &Path, path: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("dirty source has a parent"))
        .expect("dirty source parent should exist");
    fs::write(file, "// uncommitted holder work\n").expect("dirty source should write");
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

fn journal_record_count(repository_root: &Path) -> usize {
    journal_text(repository_root).lines().count()
}

fn journal_operation_count(repository_root: &Path, operation: &str) -> usize {
    journal_text(repository_root)
        .lines()
        .filter(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .is_ok_and(|event| event["op"] == operation)
        })
        .count()
}

fn journal_operation_field_values(
    repository_root: &Path,
    operation: &str,
    field: &str,
) -> Vec<String> {
    let mut values = journal_text(repository_root)
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|event| event["op"] == operation)
        .filter_map(|event| event[field].as_str().map(str::to_owned))
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
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

fn run_berth_with_git_trace(
    repository_root: &Path,
    arguments: &[&str],
    unavailable_target: &str,
) -> TracedBerth {
    let directory = tempdir().expect("git wrapper directory should exist");
    let wrapper_path = directory.path().join(GIT_BINARY);
    let trace_path = directory.path().join("trace");
    fs::write(&wrapper_path, TRACING_GIT_WRAPPER).expect("git wrapper should write");
    let mut permissions = fs::metadata(&wrapper_path)
        .expect("git wrapper metadata should read")
        .permissions();
    permissions.set_mode(EXECUTABLE_PERMISSIONS);
    fs::set_permissions(&wrapper_path, permissions).expect("git wrapper should be executable");
    let original_path = std::env::var_os("PATH").expect("test PATH should exist");
    let wrapped_path = std::env::join_paths(
        std::iter::once(directory.path().to_path_buf())
            .chain(std::env::split_paths(&original_path)),
    )
    .expect("wrapped PATH should join");
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env("PATH", wrapped_path)
        .env(REAL_GIT_ENVIRONMENT, git_binary())
        .env(TRACE_ENVIRONMENT, &trace_path)
        .env(UNAVAILABLE_TARGET_ENVIRONMENT, unavailable_target)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run with traced git");
    TracedBerth {
        output,
        trace_path,
        _directory: directory,
    }
}

fn git_trace(traced: &TracedBerth) -> Vec<String> {
    fs::read_to_string(&traced.trace_path)
        .expect("git trace should read")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn scoped_patch_comparison_count(traced: &TracedBerth) -> usize {
    git_trace(traced)
        .iter()
        .filter(|line| line.starts_with("merge-tree "))
        .count()
}

fn scoped_patch_comparisons_for_target(
    traced: &TracedBerth,
    phase_start_head: &str,
    target: &str,
) -> usize {
    git_trace(traced)
        .iter()
        .filter(|line| {
            line.starts_with("merge-tree ")
                && line.contains(&format!("--merge-base={phase_start_head}"))
                && line.split_whitespace().any(|argument| argument == target)
        })
        .count()
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

fn git(repository_root: &Path, arguments: &[&str]) { GIT.run(repository_root, arguments); }

fn git_status(repository_root: &Path, arguments: &[&str]) -> bool {
    GIT.succeeds(repository_root, arguments)
}
