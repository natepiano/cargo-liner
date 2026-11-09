#![allow(
    clippy::expect_used,
    reason = "fixture setup should stop on an unexpected command or filesystem failure"
)]

//! nextest setup entry point and the cold single-target setup regression.

use std::env;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use cargo_berth_test_support::FixtureCommand as Command;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::fixture_output_with_deadline;

#[path = "support/git_configuration.rs"]
mod git_configuration;
#[path = "support/loose_refs.rs"]
mod loose_refs;

#[path = "support/answers_fixtures.rs"]
mod answers_fixtures;

#[path = "support/compaction_fixture.rs"]
mod compaction_fixture;

#[path = "support/edges_templates.rs"]
mod edges_templates;
#[path = "support/linked_checkout.rs"]
mod linked_checkout;
#[path = "support/successor_templates.rs"]
mod successor_templates;
pub use edges_templates::COVER_LANDING;
pub use edges_templates::CROSS_TARGET_AFTER_PAIR;
pub use edges_templates::CROSS_TARGET_PAIR;
pub use edges_templates::DIFFERENT_SUCCESSOR;
pub use edges_templates::EQUIVALENT_SUCCESSOR;
pub use edges_templates::HISTORICAL_PROOF;
pub use edges_templates::INITIAL_INTEGRATION_PAIR;
pub use edges_templates::LEFT_RIGHT_TREES;
pub use edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP;
pub use edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP;
pub use edges_templates::MERGE_EXTENT_MAIN;
pub use edges_templates::MERGE_EXTENT_PAIR;
pub use edges_templates::ONE_PREDECESSOR;
pub use edges_templates::RECORDED_PROJECTION;
pub use edges_templates::RewrittenSuccessorState;
pub use edges_templates::SuccessorGap;
pub use edges_templates::TWO_PREDECESSORS;
pub use edges_templates::append_edge_fixture_event;
pub use edges_templates::build_rewritten_successor;
pub use edges_templates::commit_successor_fixture_file;
pub use edges_templates::legacy_negative_state;
pub use edges_templates::legacy_negative_template;
pub use edges_templates::retain_release;
pub use edges_templates::successor_history_state;
#[path = "support/covered_branch_template.rs"]
mod covered_branch_template;
#[path = "support/ordered_target_pair_template.rs"]
mod ordered_target_pair_template;
#[path = "support/pending_rewrite_templates.rs"]
mod pending_rewrite_templates;
pub use pending_rewrite_templates::PENDING_ORDERED_REWRITE;
pub use pending_rewrite_templates::TWO_PENDING_ORDERED_REWRITES;
#[path = "support/drift_templates.rs"]
mod drift_templates;
#[path = "support/gate_templates.rs"]
mod gate_templates;
#[path = "support/parallel_templates.rs"]
mod parallel_templates;
pub use gate_templates::CrossTargetLandingWitnesses;
pub use gate_templates::ENROLLED_PAIR;
pub use gate_templates::EnrolledPair;
pub use gate_templates::REVALIDATED_CROSS_TARGET_LANDING;
pub use gate_templates::UNENROLLED_PAIR;
pub use gate_templates::build_scratch_repository;
pub use gate_templates::enroll_pair;
pub use gate_templates::read_cross_target_landing;
pub use gate_templates::read_enrolled_pair;
pub use gate_templates::set_gate_mode;
#[path = "support/reanchoring_templates.rs"]
mod reanchoring_templates;
pub use reanchoring_templates::SplitRefChange;
#[path = "support/split_rebase.rs"]
mod split_rebase;

#[path = "support/gate_fixtures.rs"]
mod gate_fixtures;

#[path = "support/edges_fixtures.rs"]
mod edges_fixtures;

#[path = "support/overlap_fixtures.rs"]
mod overlap_fixtures;

#[path = "support/liveness_fixtures.rs"]
mod liveness_fixtures;

#[path = "support/drift_fixtures.rs"]
mod drift_fixtures;

#[path = "support/recorded_history.rs"]
mod recorded_history;

// Four workers leave descriptor capacity for nested fixture locks and child pipes.
const MAX_PARALLEL_TEMPLATE_BUILDS: usize = 4;
// The cold probe builds every template through the setup wrapper.
const TEMPLATE_SETUP_PROBE_DEADLINE: Duration = Duration::from_secs(120);

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: cargo_berth_test_support::GitDriver = cargo_berth_test_support::GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      cargo_berth_test_support::OptionalLocks::Taken,
    cleared_environment: &[],
};
use cargo_berth_test_support::git_command;

fn main() {
    if env::args().any(|argument| argument == "--probe-stalled-template") {
        let (_sender, receiver) = mpsc::channel();
        require_template_progress(&receiver, &["stalled-fixture"], Duration::from_millis(25));
        return;
    }
    let templates: Vec<_> = recorded_populations()
        .into_iter()
        .chain([
            gate_fixtures::THREE_COMMIT_REBASE,
            compaction_fixture::PREPARED_COMPACTION,
            answers_fixtures::HOLDER_LIFECYCLE_START,
            edges_fixtures::ORPHANED_MIDDLE_START,
            reanchoring_templates::SPLIT_LOWER_CHECKPOINT_SWITCHED_CHECKOUT,
            gate_templates::MAIN_PREDECESSOR_INTEGRATION_SUCCESSOR,
            gate_templates::DEFAULT_ANSWERED_GATE,
            gate_templates::ENROLLED_BEFORE_EDITS,
            gate_templates::ENROLLED_AFTER_FIRST_EDIT,
            drift_templates::TWO_PHASE_REBASE,
            parallel_templates::NAMED_SEED,
            parallel_templates::NAMED_OVERLAP,
            parallel_templates::NAMED_METADATA,
        ])
        .chain(GATE_SLOW_HISTORIES.iter().copied())
        .chain(EDGES_SLOW_HISTORIES.iter().copied())
        .chain(OVERLAP_SLOW_HISTORIES.iter().copied())
        .chain(LIVENESS_SLOW_HISTORIES.iter().copied())
        .chain(DRIFT_HELPER_HISTORIES.iter().copied())
        .collect();
    let directory = template_cache_directory();
    if env::args().any(|argument| argument == "--list") {
        fs::create_dir_all(&directory).expect("builder directory should exist");
        let link = directory.join("builder");
        let replacement = directory.join(format!("builder-{}", std::process::id()));
        symlink(
            env::current_exe().expect("builder path should resolve"),
            &replacement,
        )
        .expect("replacement builder link should write");
        fs::rename(replacement, link).expect("builder link should replace atomically");
        if let Some(run_id) = env::var_os("NEXTEST_RUN_ID") {
            fs::write(directory.join("builder-run-id"), run_id.as_encoded_bytes())
                .expect("the run that listed this builder should record");
        }
        if !env::args().any(|argument| argument == "--ignored") {
            println!("cold_single_target_setup_builds_every_recorded_template: test");
            println!("stalled_template_build_fails_with_a_clear_deadline: test");
        }
        return;
    }
    if env::args().any(|argument| argument == "stalled_template_build_fails_with_a_clear_deadline")
    {
        assert_stalled_template_deadline();
        return;
    }
    if env::args()
        .any(|argument| argument == "cold_single_target_setup_builds_every_recorded_template")
    {
        assert_single_target_setup(&templates);
        return;
    }
    let builds = directory.join("before-run").join(
        templates[0]
            .repository
            .before_run_directory()
            .file_name()
            .expect("template build should have a key"),
    );
    prepare_template_builds(&templates, &builds);
    if env::var_os("CARGO_BERTH_SINGLE_TARGET_SETUP_PROBE").is_none() {
        prepare_single_target_setup_probe(&directory);
    }
    if let Some(environment) = env::var_os("NEXTEST_ENV") {
        writeln!(
            OpenOptions::new()
                .append(true)
                .open(environment)
                .expect("nextest environment should open"),
            "CARGO_BERTH_PREBUILT_TEMPLATES={}",
            builds.display()
        )
        .expect("template directory should publish");
    }
}

const fn recorded_populations() -> [LedgerTemplate; 34] {
    [
        edges_templates::RECORDED_PROJECTION,
        edges_templates::MERGE_EXTENT_MAIN,
        edges_templates::MERGE_EXTENT_PAIR,
        edges_templates::INITIAL_INTEGRATION_PAIR,
        edges_templates::EQUIVALENT_SUCCESSOR,
        edges_templates::DIFFERENT_SUCCESSOR,
        edges_templates::ONE_PREDECESSOR,
        edges_templates::TWO_PREDECESSORS,
        successor_templates::ONE_SUCCESSOR,
        successor_templates::TWO_SUCCESSORS,
        successor_templates::FOUR_SUCCESSORS,
        edges_templates::HISTORICAL_PROOF,
        edges_templates::COVER_LANDING,
        edges_templates::CROSS_TARGET_PAIR,
        edges_templates::CROSS_TARGET_AFTER_PAIR,
        edges_templates::LEGACY_NEGATIVE_UNRELATED_GAP,
        edges_templates::LEGACY_NEGATIVE_PROTECTED_GAP,
        edges_templates::LEFT_RIGHT_TREES,
        covered_branch_template::COVERED_BRANCH_HOOK,
        ordered_target_pair_template::SEPARATE_TARGET_PAIR,
        ordered_target_pair_template::MERGED_TARGET_PAIR,
        PENDING_ORDERED_REWRITE,
        TWO_PENDING_ORDERED_REWRITES,
        gate_templates::ENROLLED_PAIR,
        gate_templates::UNENROLLED_PAIR,
        gate_templates::REVALIDATED_CROSS_TARGET_LANDING,
        reanchoring_templates::TWO_REBASE_HISTORY,
        reanchoring_templates::SPLIT_ACTIVE_PHASE,
        reanchoring_templates::SPLIT_OUTSTANDING_PHASE,
        reanchoring_templates::DRAINED_SPLIT_RETENTION_CHECKPOINT,
        reanchoring_templates::DRAINED_SPLIT_SIBLING_CHECKPOINT,
        reanchoring_templates::STACKED_UPPER_CHECKPOINT,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_ACTIVE_RETENTION,
        reanchoring_templates::SPLIT_LOWER_CHECKPOINT_UPDATED_SIBLING,
    ]
}

/// Bound stalled setup rather than leave nextest waiting on a stuck fixture child.
fn require_template_progress(progress: &mpsc::Receiver<&str>, names: &[&str], deadline: Duration) {
    let mut unfinished = names.to_vec();
    while !unfinished.is_empty() {
        match progress.recv_timeout(deadline) {
            Ok(name) => {
                unfinished.retain(|pending| *pending != name);
                eprintln!("cargo-berth template complete: {name}");
            },
            Err(error) => {
                eprintln!(
                    "cargo-berth template SETUP failed: no template completed within {deadline:?}; unfinished templates: {}: {error}",
                    unfinished.join(", ")
                );
                std::process::exit(1);
            },
        }
    }
}

fn assert_stalled_template_deadline() {
    let output = Command::new(env::current_exe().expect("builder should resolve"))
        .arg("--probe-stalled-template")
        .output()
        .expect("stalled setup probe should run");
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("template SETUP failed: no template completed within 25ms; unfinished templates: stalled-fixture")
    );
}

fn template_cache_directory() -> PathBuf {
    env::var_os("CARGO_BERTH_TEMPLATE_CACHE").map_or_else(
        || Path::new(env!("CARGO_TARGET_TMPDIR")).join("cargo-berth-templates"),
        PathBuf::from,
    )
}

/// Exercise the actual wrapper with no listing or cache, under nextest's setup
/// clock, reusing this executable. The test checks the real template artifacts.
fn prepare_single_target_setup_probe(directory: &Path) {
    let probe = directory.join("single-target-setup");
    if probe.exists() {
        fs::remove_dir_all(&probe).expect("previous setup probe should remove");
    }
    fs::create_dir(&probe).expect("empty setup probe should exist");
    assert!(
        fs::read_dir(&probe)
            .expect("empty setup probe should read")
            .next()
            .is_none()
    );
    let environment = directory.join("single-target-environment");
    fs::write(&environment, "").expect("probe environment should start empty");
    let mut command = Command::new("bash");
    command
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/setup-recorded-templates.sh"))
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .env("CARGO_BERTH_TEMPLATE_CACHE", &probe)
        .env(
            "CARGO_BERTH_TEMPLATE_BUILDER",
            env::current_exe().expect("running builder should resolve"),
        )
        .env("CARGO_BERTH_SINGLE_TARGET_SETUP_PROBE", "1")
        .env("NEXTEST_ENV", &environment);
    let output = fixture_output_with_deadline(&mut command, TEMPLATE_SETUP_PROBE_DEADLINE)
        .expect("cold single-target setup should run");
    assert!(
        output.status.success(),
        "cold single-target setup failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .lines()
            .any(|line| line == "cargo-berth template builder source: explicit builder"),
        "cold setup should reuse the running builder: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let publication = fs::read_to_string(environment).expect("probe publication should read");
    let builds = publication
        .trim()
        .strip_prefix("CARGO_BERTH_PREBUILT_TEMPLATES=")
        .expect("cold single-target setup should publish its builds");
    fs::write(directory.join("single-target-builds"), builds)
        .expect("actual cold setup artifacts should be recorded");
}

fn assert_single_target_setup(templates: &[LedgerTemplate]) {
    let directory = template_cache_directory();
    let probe = directory.join("single-target-setup");
    assert!(
        !probe.join("builder-run-id").exists(),
        "the regression must take the path without a listed builder"
    );
    let publication = fs::read_to_string(directory.join("single-target-builds"))
        .expect("cold setup should have published its actual builds");
    let builds = Path::new(publication.trim());
    assert!(builds.starts_with(&probe));
    for template in templates {
        let root = builds.join(template.repository.name);
        assert!(
            root.with_extension("complete").is_file(),
            "single-target setup did not complete {}",
            template.repository.name
        );
        assert!(
            root.join(".git/HEAD").is_file(),
            "single-target setup did not build repository {}",
            template.repository.name
        );
        let journal = root.join(".git/cargo-berth/journal.ndjson");
        match template.ledger {
            LedgerFixture::Recorded => assert!(
                journal.is_file(),
                "single-target setup did not build recorded ledger {}",
                template.repository.name
            ),
            LedgerFixture::Empty => {
                let records = match fs::read(&journal) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    records => records.expect("single-target setup journal should be readable"),
                };
                assert!(
                    records.is_empty(),
                    "single-target setup added journal records to empty fixture {}",
                    template.repository.name
                );
            },
        }
    }
}

const GATE_SLOW_HISTORIES: &[LedgerTemplate] = &[
    gate_fixtures::HISTORY_LOWER_BEFORE_UPDATE_REFS,
    gate_fixtures::HISTORY_PREPARED_SUBJECT_PASS_1,
    gate_fixtures::HISTORY_PREPARED_SUBJECT_PASS_2,
    gate_fixtures::HISTORY_PREPARED_SUBJECT_PASS_3,
    gate_fixtures::HISTORY_PREPARED_SUBJECT_PASS_4,
    gate_fixtures::HISTORY_LOCK_DEADLINE,
    gate_fixtures::HISTORY_POST_COMMIT_CELL,
    gate_fixtures::HISTORY_DEFERRED_ENDPOINT_REQUESTER,
    gate_fixtures::HISTORY_DEFERRED_ENDPOINT_FORCE,
    gate_fixtures::HISTORY_DEFERRED_ENDPOINT_REPLAY,

    gate_fixtures::HISTORY_SETTLED_PREDECESSOR_GATE,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P33C0,
    gate_fixtures::BEFORE_OBSERVE_ENFORCE_AND_ONE_USE_FORCE_APPLY_TO_BOTH_DEFERRED_ENDPOINTS,
    gate_fixtures::BEFORE_ACTIVE_LOWER_RESERVATION_KEEPS_BOTH_SPLIT_COMMITS_AFTER_UPDATE_REFS,
    gate_fixtures::BEFORE_PREPARED_GATE_SETTLES_ACTUAL_TRUNK_EVIDENCE_BUT_NEVER_A_PROPOSED_WITNESS,
    gate_fixtures::BEFORE_LISTED_TARGET_IN_REPOSITORY_PATH_WITH_BACKSLASH_IS_GATED,
    gate_fixtures::BEFORE_TARGET_GATE_HOLDS_NEITHER_LANE_AFTER_A_DEFAULT_ANSWERED_OVERLAP,
    gate_fixtures::BEFORE_RENAMED_TRUNK_REFUSAL_NAMES_ITS_BRANCH,
    gate_fixtures::BEFORE_UNINTERRUPTED_APPLY_REBASE_REANCHORS_ACTIVE_RESERVATION_ABOVE_UNRELATED_TRUNK_WORK,
    gate_fixtures::BEFORE_FORCED_CHECKPOINT_CONSUMES_ITS_PERMIT_BEFORE_ORDINARY_RECONCILIATION_SETTLES_IT,
    gate_fixtures::BEFORE_PERMIT_CONSUMPTION_WAITS_FOR_COMMITTED_AND_ABORTED_DOES_NOT_SPEND_IT,
    gate_fixtures::BEFORE_PREPARED_GATE_SCOPED_COMPARISON,
    gate_fixtures::HISTORY_POST_COMMIT_RESERVATION_CARDINALITY_TRACE_1,
    gate_fixtures::HISTORY_POST_COMMIT_RESERVATION_CARDINALITY_TRACE_3,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C1,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C2,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C1,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C2,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C0,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C1,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C2,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C14,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P0C100,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P1C0,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P2C0,
    gate_fixtures::HISTORY_POST_COMMIT_PATH_COMMIT_CARDINALITY_TRACE_P4C0,
    gate_fixtures::HISTORY_TRACE_RETENTION_REF_RECONCILIATION_REPAIR_1,
    gate_fixtures::HISTORY_TRACE_RETENTION_REF_RECONCILIATION_REPAIR_3,
    gate_fixtures::HISTORY_TRACE_RETENTION_REF_RECONCILIATION_DELETE_1,
    gate_fixtures::HISTORY_TRACE_RETENTION_REF_RECONCILIATION_DELETE_3,
    gate_fixtures::HISTORY_PREPARED_TRUNK_UPDATE_HOOK_PHASES_READY,
    gate_fixtures::HISTORY_COMMITTED_FEATURE_REBASE_HOOK_PHASES_READY,
    gate_fixtures::HISTORY_INTEGRATE_MOVES_THE_PRESENT_TARGET_AND_MISSING_TARGET_FALLS_BACK_TO_MAIN_PRESENT,
    gate_fixtures::HISTORY_INTEGRATE_MOVES_THE_PRESENT_TARGET_AND_MISSING_TARGET_FALLS_BACK_TO_MAIN_MISSING,
    gate_fixtures::HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_STALE_MARKER_PREPARED,
    gate_fixtures::HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_STALE_SESSION_MAPPING_PREPARED,
    gate_fixtures::HISTORY_ASSERT_REFERENCE_TRANSACTION_REJECTS_SESSION_WORKTREE_MISMATCH_PREPARED,
    gate_fixtures::HISTORY_FORCED_INTEGRATION_PHASES,
    gate_fixtures::RECOVERY_BRANCH_HISTORIES[0].1,
    gate_fixtures::RECOVERY_BRANCH_HISTORIES[1].1,
    gate_fixtures::RECOVERY_BRANCH_HISTORIES[2].1,
    gate_fixtures::HISTORY_LISTED_TARGET,
];

const LIVENESS_SLOW_HISTORIES: &[LedgerTemplate] = &[
    liveness_fixtures::ROUND_THREE_RETIRE_ORPHAN_REPLACES_LOST_INTEGRATION_EVIDENCE_AND_SURVIVES_REPLAY,
    liveness_fixtures::ROUND_THREE_RELEASE_USES_TRUNK_AFTER_THE_RESERVATION_TARGET_DISAPPEARS,
    liveness_fixtures::ROUND_THREE_RETARGET_REFUSES_RELEASED_RESERVATION,
    liveness_fixtures::ROUND_THREE_PRUNABLE_AND_PRUNED_WORKTREES_KEEP_BLOCKING_AND_PRUNED_WORK_REPORTS_RECOVERY,

    liveness_fixtures::HISTORY_MISSING_TARGET_RECOVERY,
    liveness_fixtures::HISTORY_MANUAL_ORPHAN_RECOVERY_ACCEPTS_THE_RESERVATION_TARGET,
    liveness_fixtures::HISTORY_REWRITTEN_INTEGRATION_REPLACEMENT_ACCEPTS_EQUIVALENT_CONTENT,
    liveness_fixtures::HISTORY_REWRITTEN_INTEGRATION_REPLACEMENT_ACCEPTS_MERGED_ANCESTRY,
    liveness_fixtures::BEFORE_ABANDON_AND_RETIRE_DISPOSITIONS_REMAIN_DISTINCT_AFTER_REPLAY,
    liveness_fixtures::BEFORE_RELEASED_INTEGRATION_TARGET_WITNESS_CAN_BE_REPLACED,
    liveness_fixtures::BEFORE_ORPHAN_ALERT_AND_MANUAL_RECOVERY_USE_THE_RESERVATION_TARGET,
    liveness_fixtures::BEFORE_REWRITTEN_INTEGRATION_REPLACEMENT_VALIDATES_EVIDENCE,
    liveness_fixtures::BEFORE_RELEASE_RECOGNIZES_A_CHECKPOINT_ON_THE_RESERVATION_TARGET,
    liveness_fixtures::BEFORE_UNRESOLVED_TRUNK_ALERT_SURVIVES_AND_DEFERS_INTEGRATED_AS_RECOVERY,
    liveness_fixtures::HISTORY_DIRECT_RELEASE_WAITS_FOR_A_COVER_AT_A_PRESENT_INTEGRATION_TARGET_CHECKPOINT,
    liveness_fixtures::HISTORY_ASSERT_MARKER_CLAIM_REVALIDATION_PREPARED,
    liveness_fixtures::HISTORY_ASSERT_STALE_MARKER_CHECK_AND_NEW_CLAIM_PREPARED,
    liveness_fixtures::HISTORY_ASSERT_CHECK_SESSION_REJECTIONS_PREPARED,
];

const EDGES_SLOW_HISTORIES: &[LedgerTemplate] = &[
    edges_fixtures::HISTORY_SAME_TARGET_EDGE_REFUSES_INTEGRATION_BEFORE_THE_PREDECESSOR,
    edges_fixtures::HISTORY_SAME_TARGET_EDGE_REQUIRES_INCORPORATION_AFTER_THE_PREDECESSOR_LANDS,
    edges_fixtures::HISTORY_SAME_TARGET_EDGE_SETTLES_AFTER_SUCCESSOR_INCORPORATION,
    edges_fixtures::HISTORY_READINESS_NOT_INTEGRATED,
    edges_fixtures::HISTORY_READINESS_INCORPORATED,
    edges_fixtures::HISTORY_READINESS_MISSING_OBJECT,
    edges_fixtures::BEFORE_RELEASED_PREDECESSOR_REF_RETIRES_ONLY_AFTER_EVERY_SUCCESSOR_IS_TERMINAL,
    edges_fixtures::BEFORE_SUCCESSOR_HEAD_AND_CURRENT_TRUNK_BOTH_CONTROL_READINESS,
    edges_fixtures::BEFORE_SAME_TARGET_EDGE_WAITS_FOR_PREDECESSOR_TO_REACH_INTEGRATION,
    edges_fixtures::BEFORE_SEQUENCE_REJECTS_EACH_INVALID_IDENTITY_AND_CARRIES_ALERTS,
    edges_fixtures::HISTORY_CHECKPOINT_PAIR,
];

const OVERLAP_SLOW_HISTORIES: &[LedgerTemplate] = &[
    overlap_fixtures::HISTORY_SIBLING_LANDED_COVER,
    overlap_fixtures::BEFORE_CHECK_USES_THE_CURRENT_RUNS_TARGET_WHEN_AN_OLDER_RUN_IS_STILL_OUTSTANDING,
    overlap_fixtures::BEFORE_A_FIRST_TOUCH_HOLDER_BLOCK_NAMES_THE_VERBS_THAT_CLEAR_IT,
];

const DRIFT_HELPER_HISTORIES: &[LedgerTemplate] = &[
    drift_fixtures::COMMITTED_INCURSION,
    drift_fixtures::HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_CHECKPOINT,
    drift_fixtures::HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_ENGINE,
    drift_fixtures::HISTORY_WIDENING_OVER_AN_OVERLAPPED_HOLDER_FOREIGN,
];

fn prepare_template_builds(templates: &[LedgerTemplate], builds: &Path) {
    std::thread::scope(|scope| {
        let (completed, progress) = mpsc::channel();
        for worker in 0..MAX_PARALLEL_TEMPLATE_BUILDS {
            let completed = completed.clone();
            scope.spawn(move || {
                for template in templates
                    .iter()
                    .skip(worker)
                    .step_by(MAX_PARALLEL_TEMPLATE_BUILDS)
                {
                    template.prepare_before_run(builds);
                    completed
                        .send(template.repository.name)
                        .expect("setup progress should publish");
                }
            });
        }
        drop(completed);
        // Cold SETUP took 39.527 s under tracing. Allow over three times that
        // for one completion, so machine load does not look like a stalled child.
        let names: Vec<_> = templates
            .iter()
            .map(|template| template.repository.name)
            .collect();
        require_template_progress(&progress, &names, Duration::from_secs(120));
    });
}
