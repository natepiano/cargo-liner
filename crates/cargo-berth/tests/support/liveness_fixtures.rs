// Shared starting histories and fixture operations.

pub(super) use std::fs;
pub(super) use std::path::Path;
pub(super) use std::path::PathBuf;
pub(super) use std::process::Output;

pub(super) use cargo_berth_test_support::FIRST_RUN;
pub(super) use cargo_berth_test_support::GitDriver;
pub(super) use cargo_berth_test_support::IntegrationRepository;
pub(super) use cargo_berth_test_support::LedgerFixture;
pub(super) use cargo_berth_test_support::LedgerTemplate;
pub(super) use cargo_berth_test_support::OptionalLocks;
pub(super) use cargo_berth_test_support::RepositoryTemplate;
pub(super) use cargo_berth_test_support::SECOND_RUN;
pub(super) use cargo_berth_test_support::TemplateBuildTiming;
pub(super) use cargo_berth_test_support::assert_success;
pub(super) use cargo_berth_test_support::berth_command;
pub(super) use cargo_berth_test_support::json;
pub(super) use tempfile::TempDir;

use crate::git_configuration;
use crate::loose_refs;
use crate::recorded_history;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

/// The repository most tests here start from: a README committed on `main`, then
/// `cargo-berth init` and its configuration committed.
pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "liveness-initialized",
    build:            build_initialized_repository,
};

/// The integration pair prepared by `recorded_templates` before tests start.
pub(super) const INITIAL_INTEGRATION_PAIR: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "initial-integration-pair",
        build: |root| IntegrationRepository::build_initial_integration_pair(root, BERTH_EXECUTABLE),
        ..INITIALIZED_REPOSITORY
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// Unclaimed terminal work and a separate rewritten source, before any recovery answer.
pub(super) const RECOVERY_SOURCES: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "recovery-sources",
    build:            build_recovery_sources,
};

pub(super) const MARKER_PATH: &str = ".git/cargo-berth-run-id";

pub(super) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

pub(super) const SESSION_MAPPING_PATH: &str = ".git/cargo-berth/session-identities.json";

pub(super) fn record_terminal_recovery_dispositions(repository: &Path) -> (String, String) {
    // The template contains only unclaimed source commits. Each reservation still
    // checkpoints and resolves separately before replay tests their distinction.
    let abandoned_claim = run_berth(
        repository,
        &["claim", "file:abandoned", "--run", FIRST_RUN, "--json"],
    );
    let abandoned_id = reservation_id(&abandoned_claim);
    assert!(
        run_berth(repository, &["release", &abandoned_id])
            .status
            .success()
    );
    let abandoned = run_berth(
        repository,
        &[
            "resolve",
            &abandoned_id,
            "--abandon",
            "--why",
            "user confirmed discard",
            "--json",
        ],
    );
    assert_eq!(
        json_output(&abandoned)["payload"]["data"]["disposition"]["kind"],
        "abandoned"
    );

    let retired_claim = run_berth(
        repository,
        &["claim", "file:retired", "--run", FIRST_RUN, "--json"],
    );
    let retired_id = reservation_id(&retired_claim);
    assert!(
        run_berth(repository, &["release", &retired_id])
            .status
            .success()
    );
    let retired = run_berth(
        repository,
        &[
            "resolve",
            &retired_id,
            "--retire-orphan",
            "--why",
            "user confirmed retirement",
            "--json",
        ],
    );
    assert_eq!(
        json_output(&retired)["payload"]["data"]["disposition"]["kind"],
        "retired_orphan"
    );
    (abandoned_id, retired_id)
}

pub(super) fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

pub(super) fn build_recovery_sources(repository: &Path) {
    // No ledger answer or reservation is cached. The source commits run their
    // real hooks, just as they did before this setup was shared across processes.
    build_initialized_repository(repository);
    GIT.run_without_hooks(repository, ["switch", "--quiet", "-c", "abandoned"]);
    fs::write(repository.join("abandoned"), "work\n").expect("abandoned work should write");
    fs::write(repository.join("retired"), "work\n").expect("retired work should write");
    git(repository, &["add", "abandoned", "retired"]);
    git(
        repository,
        &["commit", "--quiet", "-m", "terminal recovery work"],
    );
    GIT.run_without_hooks(repository, ["switch", "--quiet", "-c", "rewritten", "main"]);
    commit_file(repository, "rewritten", "work\n", "rewritten source");
    GIT.run_without_hooks(repository, ["switch", "--quiet", "abandoned"]);
}

pub(super) fn build_initialized_repository(repository_root: &Path) {
    git(
        repository_root,
        &["init", "--quiet", "--initial-branch=main"],
    );
    git_configuration::append_git_configuration(repository_root, "\n[user]\n name = Berth Test\n email = berth@example.invalid\n[maintenance]\n auto = false\n[gc]\n auto = 0\n").expect("fixture git config should write");
    fs::write(repository_root.join("README.md"), "scratch repository\n")
        .expect("scratch file should write");
    git(repository_root, &["add", "README.md"]);
    git(repository_root, &["commit", "--quiet", "-m", "initial"]);
    assert!(run_berth(repository_root, &["init"]).status.success());
    git(
        repository_root,
        &["add", "--force", ".claude/config/berth.toml"],
    );
    // Nothing is claimed yet, so the commit skips the hooks; see `GitDriver::run_without_hooks`.
    GIT.run_without_hooks(
        repository_root,
        ["commit", "--quiet", "-m", "track berth config"],
    );
}

pub(super) fn reservation_id(claim: &Output) -> String {
    json_output(claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

pub(super) fn commit_file(repository_root: &Path, path: &str, contents: &str, message: &str) {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("file parent should exist");
    }
    fs::write(file_path, contents).expect("committed file should write");
    git(repository_root, &["add", path]);
    git(repository_root, &["commit", "--quiet", "-m", message]);
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env(SESSION_ENVIRONMENT, session_id)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git(repository_root: &Path, arguments: &[&str]) {
    GIT.run(repository_root, arguments);
}

pub(super) fn git_stdout(repository_root: &Path, arguments: &[&str]) -> String {
    GIT.stdout(repository_root, arguments)
}

pub(super) fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}

/// Starting history before `abandon_and_retire_dispositions_remain_distinct_after_replay`.
pub(super) const BEFORE_ABANDON_AND_RETIRE_DISPOSITIONS_REMAIN_DISTINCT_AFTER_REPLAY:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-abandon-and-retire-dispositions-remain-distinct-after-replay",
    build_before_abandon_and_retire_dispositions_remain_distinct_after_replay,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AbandonAndRetireDispositionsRemainDistinctAfterReplayState {
    pub(super) abandoned_id: String,
    pub(super) retired_id:   String,
}

fn build_before_abandon_and_retire_dispositions_remain_distinct_after_replay(destination: &Path) {
    let repository = RECOVERY_SOURCES.instantiate();
    let (abandoned_id, retired_id) = record_terminal_recovery_dispositions(repository.path());

    recorded_history::save(
        destination,
        repository.path(),
        &AbandonAndRetireDispositionsRemainDistinctAfterReplayState {
            abandoned_id,
            retired_id,
        },
    );
}

/// Starting history before `released_integration_target_witness_can_be_replaced`.
pub(super) const BEFORE_RELEASED_INTEGRATION_TARGET_WITNESS_CAN_BE_REPLACED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-released-integration-target-witness-can-be-replaced",
    build_before_released_integration_target_witness_can_be_replaced,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ReleasedIntegrationTargetWitnessCanBeReplacedState {
    pub(super) integration:   PathBuf,
    pub(super) id:            String,
    pub(super) target_before: String,
}

fn build_before_released_integration_target_witness_can_be_replaced(destination: &Path) {
    // The existing main checkout can hold work aimed at integration. A third
    // checkout adds no target-reachability distinction to this scenario.
    let repo = IntegrationRepository::from_template(INITIALIZED_REPOSITORY);
    let lane = repo.root();
    let claimed = repo.claim(lane, "file:replacement.txt", FIRST_RUN, Some("integration"));
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(lane, "replacement.txt", "work\n", "protected work");
    let checkpoint = repo.run(lane, &["release", &id, "--json"]);
    assert_success(&checkpoint);
    let target_before = loose_refs::read_loose_ref(repo.root(), "refs/heads/integration");
    repo.commit_file(
        &repo.integration,
        "replacement.txt",
        "work\n",
        "squashed integration",
    );
    let landed = repo.run(lane, &["release", &id, "--json"]);
    assert_success(&landed);
    assert_eq!(json(&landed)["status"], "integrated");

    recorded_history::save(
        destination,
        repo.root(),
        &ReleasedIntegrationTargetWitnessCanBeReplacedState {
            id,
            target_before,
            integration: repo.integration.clone(),
        },
    );
}

/// Starting history before `orphan_alert_and_manual_recovery_use_the_reservation_target`.
pub(super) const BEFORE_ORPHAN_ALERT_AND_MANUAL_RECOVERY_USE_THE_RESERVATION_TARGET:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-orphan-alert-and-manual-recovery-use-the-reservation-target",
    build_before_orphan_alert_and_manual_recovery_use_the_reservation_target,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct OrphanAlertAndManualRecoveryUseTheReservationTargetState {
    pub(super) lane:          PathBuf,
    pub(super) id:            String,
    pub(super) protected_tip: String,
}

fn build_before_orphan_alert_and_manual_recovery_use_the_reservation_target(destination: &Path) {
    let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("target-orphan-lane", "integration");
    let claimed = repo.claim(&lane, "file:orphan.txt", SECOND_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(&lane, "orphan.txt", "work\n", "orphan work");
    let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
    assert_success(&checkpoint);
    let protected_tip = repo.git_stdout(&lane, &["rev-parse", "HEAD"]);
    repo.commit_file(&lane, "later.txt", "later\n", "later lane work");
    let later_head = repo.git_stdout(&lane, &["rev-parse", "HEAD"]);
    let before_prune = repo.run(repo.root(), &["board", "--json"]);
    assert_success(&before_prune);
    let before_prune = json(&before_prune);
    let reservation = ["ready_now", "unconstrained_reservations"]
        .into_iter()
        .flat_map(|section| {
            before_prune["payload"]["data"][section]["entries"]
                .as_array()
                .into_iter()
                .flatten()
        })
        .map(|entry| entry.get("reservation").unwrap_or(entry))
        .find(|entry| entry["reservation_id"] == id)
        .expect("outstanding reservation remains on the board");
    assert_eq!(reservation["merge_extent"]["status"], "protected");
    assert_ne!(later_head, protected_tip);

    recorded_history::save(
        destination,
        repo.root(),
        &OrphanAlertAndManualRecoveryUseTheReservationTargetState {
            lane,
            id,
            protected_tip,
        },
    );
}

/// Starting history before `rewritten_integration_replacement_validates_evidence`.
pub(super) const BEFORE_REWRITTEN_INTEGRATION_REPLACEMENT_VALIDATES_EVIDENCE:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-rewritten-integration-replacement-validates-evidence",
    build_before_rewritten_integration_replacement_validates_evidence,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RewrittenIntegrationReplacementValidatesEvidenceState {
    pub(super) rewritten_id:    String,
    pub(super) unreachable_tip: String,
}

fn build_before_rewritten_integration_replacement_validates_evidence(destination: &Path) {
    let sources = RECOVERY_SOURCES.instantiate();
    let repository = sources.path();
    // Each branch switch skips the hooks; see `GitDriver::run_without_hooks`.
    GIT.run_without_hooks(repository, ["switch", "--quiet", "rewritten"]);
    let unreachable_tip = loose_refs::read_loose_ref(repository, "refs/heads/rewritten");
    let rewritten_claim = run_berth(
        repository,
        &["claim", "file:rewritten", "--run", FIRST_RUN, "--json"],
    );
    let rewritten_id = reservation_id(&rewritten_claim);
    assert!(
        run_berth(repository, &["release", &rewritten_id])
            .status
            .success()
    );
    GIT.run_without_hooks(repository, ["switch", "--quiet", "main"]);

    recorded_history::save(
        destination,
        sources.path(),
        &RewrittenIntegrationReplacementValidatesEvidenceState {
            rewritten_id,
            unreachable_tip,
        },
    );
}

/// Starting history before `release_recognizes_a_checkpoint_on_the_reservation_target`.
pub(super) const BEFORE_RELEASE_RECOGNIZES_A_CHECKPOINT_ON_THE_RESERVATION_TARGET:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-release-recognizes-a-checkpoint-on-the-reservation-target",
    build_before_release_recognizes_a_checkpoint_on_the_reservation_target,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ReleaseRecognizesACheckpointOnTheReservationTargetState {
    pub(super) lane: PathBuf,
    pub(super) id:   String,
}

fn build_before_release_recognizes_a_checkpoint_on_the_reservation_target(destination: &Path) {
    let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("target-release-lane", "integration");
    let claim = repo.claim(&lane, "file:released.txt", FIRST_RUN, None);
    assert_success(&claim);
    let id = json(&claim)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(&lane, "released.txt", "landed\n", "release work");
    let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
    assert_success(&checkpoint);
    assert_eq!(
        json(&checkpoint)["payload"]["data"]["status"],
        "checkpointed"
    );

    recorded_history::save(
        destination,
        repo.root(),
        &ReleaseRecognizesACheckpointOnTheReservationTargetState { lane, id },
    );
}

/// Starting history before `unresolved_trunk_alert_survives_and_defers_integrated_as_recovery`.
pub(super) const BEFORE_UNRESOLVED_TRUNK_ALERT_SURVIVES_AND_DEFERS_INTEGRATED_AS_RECOVERY:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-unresolved-trunk-alert-survives-and-defers-integrated-as-recovery",
    build_before_unresolved_trunk_alert_survives_and_defers_integrated_as_recovery,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct UnresolvedTrunkAlertSurvivesAndDefersIntegratedAsRecoveryState {
    pub(super) reservation_id: String,
    pub(super) integrated_tip: String,
}

fn build_before_unresolved_trunk_alert_survives_and_defers_integrated_as_recovery(
    destination: &Path,
) {
    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &["claim", "file:unknown-trunk", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let reservation_id = reservation_id(&claim);
    commit_file(
        repository.path(),
        "unknown-trunk",
        "released work\n",
        "unknown trunk work",
    );
    // Work committed on trunk ends its run in the release command's own reconciliation.
    for (expected_status, expected_fact_status) in [
        ("integrated", "released"),
        ("integrated", "already_settled"),
    ] {
        let release = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
        assert!(release.status.success());
        assert_eq!(json_output(&release)["status"], expected_status);
        assert_eq!(
            json_output(&release)["payload"]["data"]["status"],
            expected_fact_status
        );
    }
    let integrated_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);

    recorded_history::save(
        destination,
        repository.path(),
        &UnresolvedTrunkAlertSurvivesAndDefersIntegratedAsRecoveryState {
            reservation_id,
            integrated_tip,
        },
    );
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct DirectReleaseWaitsForACoverAtAPresentIntegrationTargetHistory {
    pub(super) lane: PathBuf,
    pub(super) id:   String,
}

pub(super) const HISTORY_DIRECT_RELEASE_WAITS_FOR_A_COVER_AT_A_PRESENT_INTEGRATION_TARGET_CHECKPOINT: cargo_berth_test_support::LedgerTemplate = recorded_history::template("liveness-direct-release-waits-for-a-cover-at-a-present-integration-target-checkpoint", build_history_direct_release_waits_for_a_cover_at_a_present_integration_target);

fn build_history_direct_release_waits_for_a_cover_at_a_present_integration_target(
    destination: &Path,
) {
    let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("cover-release-lane", "integration");
    let claimed = repo.claim(&lane, "file:lane.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let _ = repo.board();
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane ID")
        .to_owned();
    repo.commit_file(&lane, "lane.txt", "landed\n", "lane work");
    let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
    assert_success(&checkpoint);
    let waiting = json(&checkpoint);
    assert_eq!(waiting["status"], "outstanding", "{waiting}");
    assert_eq!(waiting["payload"]["data"]["status"], "checkpointed");
    assert!(
        !waiting["message"]
            .as_str()
            .expect("release message")
            .contains("uncovered"),
        "a lane not yet merged keeps its existing outstanding answer: {waiting}"
    );

    recorded_history::save(
        destination,
        repo.root(),
        &DirectReleaseWaitsForACoverAtAPresentIntegrationTargetHistory { lane, id },
    );
}

pub(super) const HISTORY_ASSERT_MARKER_CLAIM_REVALIDATION_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-assert-marker-claim-revalidation-prepared",
    build_history_assert_marker_claim_revalidation,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertMarkerClaimRevalidationHistory {
    pub(super) raced_reservation_id: String,
}

fn build_history_assert_marker_claim_revalidation(destination: &Path) {
    let repository = initialized_repository();

    let first_claim = run_berth(
        repository.path(),
        &["claim", "file:first", "--run", FIRST_RUN, "--json"],
    );
    let raced_reservation_id = reservation_id(&first_claim);

    recorded_history::save(
        destination,
        repository.path(),
        &AssertMarkerClaimRevalidationHistory {
            raced_reservation_id,
        },
    );
}

pub(super) const HISTORY_ASSERT_STALE_MARKER_CHECK_AND_NEW_CLAIM_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-assert-stale-marker-check-and-new-claim-prepared",
    build_history_assert_stale_marker_check_and_new_claim,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertStaleMarkerCheckAndNewClaimHistory {}

fn build_history_assert_stale_marker_check_and_new_claim(destination: &Path) {
    let repository = initialized_repository();

    git(repository.path(), &["switch", "--quiet", "-c", "phase"]);
    commit_file(repository.path(), "a", "phase\n", "phase work");
    let first_claim = run_berth(
        repository.path(),
        &["claim", "file:a", "--run", FIRST_RUN, "--json"],
    );
    let first_reservation_id = reservation_id(&first_claim);
    assert!(
        run_berth(repository.path(), &["release", &first_reservation_id])
            .status
            .success()
    );
    fs::write(
        repository.path().join(MARKER_PATH),
        format!("{FIRST_RUN}\n"),
    )
    .expect("stale marker should write");

    recorded_history::save(
        destination,
        repository.path(),
        &AssertStaleMarkerCheckAndNewClaimHistory {},
    );
}

pub(super) const HISTORY_ASSERT_CHECK_SESSION_REJECTIONS_PREPARED:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-assert-check-session-rejections-prepared",
    build_history_assert_check_session_rejections,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AssertCheckSessionRejectionsHistory {}

fn build_history_assert_check_session_rejections(destination: &Path) {
    let repository = initialized_repository();

    let stale_session = "stale-check-session";
    let mapped_claim = run_berth_with_session(
        repository.path(),
        &["claim", "file:stale-mapped", "--run", FIRST_RUN, "--json"],
        stale_session,
    );
    assert!(mapped_claim.status.success());
    let mapped_reservation_id = reservation_id(&mapped_claim);
    let mapping_path = repository.path().join(SESSION_MAPPING_PATH);
    let stale_mapping = fs::read(&mapping_path).expect("session mapping should read");
    assert!(
        run_berth(
            repository.path(),
            &["release", &mapped_reservation_id, "--json"],
        )
        .status
        .success()
    );
    fs::write(&mapping_path, stale_mapping).expect("stale mapping should write");

    recorded_history::save(
        destination,
        repository.path(),
        &AssertCheckSessionRejectionsHistory {},
    );
}

pub(super) const HISTORY_MANUAL_ORPHAN_RECOVERY_ACCEPTS_THE_RESERVATION_TARGET:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-manual-orphan-recovery-accepts-the-reservation-target",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(
            BEFORE_ORPHAN_ALERT_AND_MANUAL_RECOVERY_USE_THE_RESERVATION_TARGET,
        );
        let OrphanAlertAndManualRecoveryUseTheReservationTargetState {
            lane,
            id,
            protected_tip,
        } = recorded_history::read(repo.root());
        std::fs::remove_dir_all(&lane).expect("remove lane worktree");
        repo.git(repo.root(), &["worktree", "prune", "--expire", "now"]);
        repo.git(repo.root(), &["branch", "-D", "target-orphan-lane"]);

        let target_before = repo.git_stdout(&repo.integration, &["rev-parse", "HEAD"]);
        let observed = repo.run(repo.root(), &["board", "--json"]);
        let response = json(&observed);
        let alert = response["payload"]["data"]["alerts"]["entries"]
            .as_array()
            .expect("alerts")
            .iter()
            .find(|alert| alert["kind"] == "orphaned_outstanding")
            .expect("orphan alert");
        assert_eq!(alert["resolution"]["recovery"]["trunk_oid"], target_before);

        repo.commit_file(repo.root(), "main-only.txt", "main\n", "main only");
        let main_only = repo.git_stdout(repo.root(), &["rev-parse", "HEAD"]);
        let refused = repo.run(
            repo.root(),
            &["resolve", &id, "--integrated-as", &main_only, "--json"],
        );
        assert!(!refused.status.success());
        let refusal = json(&refused);
        assert_eq!(refusal["status"], "invalid_input");
        assert_eq!(
            refusal["message"],
            "the --integrated-as commit must resolve in this repository and be reachable from the reservation's target"
        );

        recorded_history::save(
            destination,
            repo.root(),
            &ManualOrphanRecoveryAcceptsTheReservationTargetHistory { id, protected_tip },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ManualOrphanRecoveryAcceptsTheReservationTargetHistory {
    pub(super) id:            String,
    pub(super) protected_tip: String,
}

pub(super) const HISTORY_REWRITTEN_INTEGRATION_REPLACEMENT_ACCEPTS_EQUIVALENT_CONTENT:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-rewritten-integration-replacement-accepts-equivalent-content",
    |destination| {
        let sources = BEFORE_REWRITTEN_INTEGRATION_REPLACEMENT_VALIDATES_EVIDENCE.instantiate();
        let RewrittenIntegrationReplacementValidatesEvidenceState {
            rewritten_id,
            unreachable_tip,
        } = recorded_history::read(sources.path());
        let repository = sources.path();
        let rejected = run_berth(
            repository,
            &[
                "resolve",
                &rewritten_id,
                "--integrated-as",
                &unreachable_tip,
                "--json",
            ],
        );
        assert_eq!(rejected.status.code(), Some(5));

        let unrelated_commit = loose_refs::read_loose_ref(repository, "refs/heads/main");
        assert_integrated_as_lacks_the_work(
            repository,
            &rewritten_id,
            &unrelated_commit,
            &unreachable_tip,
        );

        commit_rewritten_trunk_without_hooks(repository, "squashed integration");
        let squashed_commit = loose_refs::read_loose_ref(repository, "refs/heads/main");
        let settlement = json_output(&run_berth(
            repository,
            &["release", &rewritten_id, "--json"],
        ));
        assert_eq!(settlement["status"], "integrated", "{settlement:#}");
        let settled = json_output(&run_berth(
            repository,
            &["release", &rewritten_id, "--json"],
        ));
        assert_eq!(settled["status"], "integrated", "{settled:#}");
        assert_eq!(
            settled["payload"]["data"]["evidence"]["witness"]["commit"],
            squashed_commit.as_str()
        );

        recorded_history::save(
            destination,
            sources.path(),
            &RewrittenIntegrationReplacementAcceptsEquivalentContentHistory {
                rewritten_id,
                unreachable_tip,
                unrelated_commit,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RewrittenIntegrationReplacementAcceptsEquivalentContentHistory {
    pub(super) rewritten_id:     String,
    pub(super) unreachable_tip:  String,
    pub(super) unrelated_commit: String,
}

pub(super) const HISTORY_REWRITTEN_INTEGRATION_REPLACEMENT_ACCEPTS_MERGED_ANCESTRY:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-rewritten-integration-replacement-accepts-merged-ancestry",
    |destination| {
        let sources = BEFORE_REWRITTEN_INTEGRATION_REPLACEMENT_VALIDATES_EVIDENCE.instantiate();
        let RewrittenIntegrationReplacementValidatesEvidenceState {
            rewritten_id,
            unreachable_tip,
        } = recorded_history::read(sources.path());
        let repository = sources.path();
        let rejected = run_berth(
            repository,
            &[
                "resolve",
                &rewritten_id,
                "--integrated-as",
                &unreachable_tip,
                "--json",
            ],
        );
        assert_eq!(rejected.status.code(), Some(5));

        let unrelated_commit = loose_refs::read_loose_ref(repository, "refs/heads/main");
        assert_integrated_as_lacks_the_work(
            repository,
            &rewritten_id,
            &unrelated_commit,
            &unreachable_tip,
        );

        commit_rewritten_trunk_without_hooks(repository, "squashed integration");
        let squashed_commit = loose_refs::read_loose_ref(repository, "refs/heads/main");
        let settlement = json_output(&run_berth(
            repository,
            &["release", &rewritten_id, "--json"],
        ));
        assert_eq!(settlement["status"], "integrated", "{settlement:#}");
        let settled = json_output(&run_berth(
            repository,
            &["release", &rewritten_id, "--json"],
        ));
        assert_eq!(settled["status"], "integrated", "{settled:#}");
        assert_eq!(
            settled["payload"]["data"]["evidence"]["witness"]["commit"],
            squashed_commit.as_str()
        );

        lose_the_integration_witness(repository, &rewritten_id, &unrelated_commit);
        assert_integrated_as_lacks_the_work(
            repository,
            &rewritten_id,
            &unrelated_commit,
            &unreachable_tip,
        );
        commit_rewritten_trunk_without_hooks(repository, "squashed again");
        let equivalent_commit = loose_refs::read_loose_ref(repository, "refs/heads/main");
        replace_the_integration_evidence(repository, &rewritten_id, &equivalent_commit);

        recorded_history::save(
            destination,
            sources.path(),
            &RewrittenIntegrationReplacementAcceptsMergedAncestryHistory {
                rewritten_id,
                unrelated_commit,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RewrittenIntegrationReplacementAcceptsMergedAncestryHistory {
    pub(super) rewritten_id:     String,
    pub(super) unrelated_commit: String,
}

pub(super) fn commit_rewritten_trunk_without_hooks(repository: &Path, message: &str) {
    fs::write(repository.join("rewritten"), "work\n").expect("trunk work should write");
    GIT.run_without_hooks(repository, ["add", "rewritten"]);
    GIT.run_without_hooks(repository, ["commit", "--quiet", "-m", message]);
}

pub(super) fn lose_the_integration_witness(repository: &Path, reservation_id: &str, trunk: &str) {
    GIT.run_without_hooks(repository, ["reset", "--hard", "--quiet", trunk]);
    let reblocked = json_output(&run_berth(
        repository,
        &["release", reservation_id, "--json"],
    ));
    assert_eq!(reblocked["status"], "trunk_rewritten");
    assert!(
        reblocked["payload"]["alerts"]
            .as_array()
            .is_none_or(|alerts| {
                alerts
                    .iter()
                    .all(|alert| alert["kind"] != "lost_integration_evidence")
            }),
        "one pass alone must not report integration evidence lost: {reblocked:#}"
    );
    let persisted = json_output(&run_berth(
        repository,
        &["release", reservation_id, "--json"],
    ));
    assert_eq!(persisted["status"], "trunk_rewritten");
    let lost_evidence = &persisted["payload"]["alerts"][0];
    assert_eq!(lost_evidence["kind"], "lost_integration_evidence");
    assert_eq!(
        lost_evidence["data"]["recovery"]["kind"],
        "name_carrying_trunk_commit"
    );
    assert_eq!(lost_evidence["data"]["recovery"]["trunk_oid"], trunk);
    assert_eq!(
        lost_evidence["data"]["recovery"]["retirement"]["action"],
        "resolve_retire_orphan"
    );
}

pub(super) fn replace_the_integration_evidence(
    repository: &Path,
    reservation_id: &str,
    commit: &str,
) {
    let replaced = run_berth(
        repository,
        &[
            "resolve",
            reservation_id,
            "--integrated-as",
            commit,
            "--json",
        ],
    );
    assert!(replaced.status.success(), "{:#}", json_output(&replaced));
    assert_eq!(
        json_output(&replaced)["payload"]["data"]["disposition"]["evidence"],
        commit
    );
}

pub(super) fn assert_integrated_as_lacks_the_work(
    repository: &Path,
    reservation_id: &str,
    commit: &str,
    protected_tip: &str,
) {
    let refused = run_berth(
        repository,
        &[
            "resolve",
            reservation_id,
            "--integrated-as",
            commit,
            "--json",
        ],
    );
    assert_eq!(refused.status.code(), Some(5));
    let message = json_output(&refused)["message"]
        .as_str()
        .expect("refusal should carry a message")
        .to_owned();
    assert!(message.contains(commit), "{message}");
    assert!(message.contains(protected_tip), "{message}");
    assert!(
        message.contains(&format!(
            "cargo-berth resolve {reservation_id} --retire-orphan --why <reason>"
        )),
        "{message}"
    );
}

pub(super) const HISTORY_MISSING_TARGET_RECOVERY: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("liveness-missing-target-recovery", |destination| {
        let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
        let lane = repo.lane("missing-target-recovery-lane", "integration");
        let claimed = repo.claim(&lane, "file:recovered.txt", SECOND_RUN, None);
        assert_success(&claimed);
        let id = json(&claimed)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("reservation id")
            .to_owned();
        repo.commit_file(&lane, "recovered.txt", "work\n", "protected work");
        let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
        assert_success(&checkpoint);
        repo.commit_file(&lane, "later.txt", "later\n", "later lane work");
        repo.git(
            repo.root(),
            &[
                "worktree",
                "remove",
                "--force",
                lane.to_str().expect("UTF-8 path"),
            ],
        );
        repo.git(
            repo.root(),
            &["branch", "-D", "missing-target-recovery-lane"],
        );
        repo.commit_file(repo.root(), "recovered.txt", "work\n", "trunk carries work");
        let trunk_commit = repo.git_stdout(repo.root(), &["rev-parse", "HEAD"]);
        let missing_target_commit = repo.git_stdout(&repo.integration, &["rev-parse", "HEAD"]);
        repo.remove_integration_branch();

        recorded_history::save(
            destination,
            repo.root(),
            &MissingTargetRecoveryHistory {
                id,
                trunk_commit,
                missing_target_commit,
            },
        );
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct MissingTargetRecoveryHistory {
    pub(super) id:                    String,
    pub(super) trunk_commit:          String,
    pub(super) missing_target_commit: String,
}

/// Check the retirement refusal for a reservation that already has a disposition.
pub(super) fn assert_retire_orphan_is_already_resolved(repository: &Path, reservation_id: &str) {
    let refused = run_berth(
        repository,
        &[
            "resolve",
            reservation_id,
            "--retire-orphan",
            "--why",
            "user confirmed retirement",
            "--json",
        ],
    );
    let refused_json = json_output(&refused);
    assert_eq!(refused.status.code(), Some(5));
    assert_eq!(refused_json["status"], "invalid_input");
    assert_eq!(
        refused_json["message"],
        "the reservation is already resolved"
    );
}

/// Find the lost integration evidence alert for the reservation.
pub(super) fn lost_evidence_alert<'board>(
    board: &'board serde_json::Value,
    reservation_id: &str,
) -> Option<&'board serde_json::Value> {
    board["payload"]["data"]["alerts"]["entries"]
        .as_array()?
        .iter()
        .find(|alert| {
            alert["kind"] == "lost_integration_evidence"
                && alert["reservation_id"] == reservation_id
        })
}

pub(super) const ROUND_THREE_RETIRE_ORPHAN_REPLACES_LOST_INTEGRATION_EVIDENCE_AND_SURVIVES_REPLAY: cargo_berth_test_support::LedgerTemplate = recorded_history::template("liveness-round-three-retire-orphan-replaces-lost-integration-evidence-and-survives-replay", |destination| {

    let repository = initialized_repository();
    let claim = run_berth(
        repository.path(),
        &["claim", "file:reworked", "--run", FIRST_RUN, "--json"],
    );
    assert!(claim.status.success());
    let reservation_id = reservation_id(&claim);
    let trunk_before_landing = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    commit_file(repository.path(), "reworked", "work\n", "landed work");
    let protected_tip = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    let released = run_berth(repository.path(), &["release", &reservation_id, "--json"]);
    assert_eq!(json_output(&released)["status"], "integrated");

    // Evidence that still proves the integration leaves no disposition to replace.
    assert_retire_orphan_is_already_resolved(repository.path(), &reservation_id);

    // Trunk is rewritten and the work lands again reworked, so git cannot match it and the
    // `--integrated-as` refusal names the retirement instead.
    lose_the_integration_witness(repository.path(), &reservation_id, &trunk_before_landing);
    commit_file(
        repository.path(),
        "reworked",
        "reworked work\n",
        "reworked landing",
    );
    let reworked_commit = git_stdout(repository.path(), &["rev-parse", "HEAD"]);
    assert_integrated_as_lacks_the_work(
        repository.path(),
        &reservation_id,
        &reworked_commit,
        &protected_tip,
    );
    let board = json_output(&run_berth(repository.path(), &["board", "--json"]));
    let alert = lost_evidence_alert(&board, &reservation_id)
        .expect("the board should report the lost evidence");
    assert_eq!(
        alert["recovery"]["retirement"],
        serde_json::json!({
            "action": "resolve_retire_orphan",
            "reservation_id": reservation_id,
        })
    );


        recorded_history::save(destination, repository.path(), &RetireOrphanReplacesLostIntegrationEvidenceAndSurvivesReplayHistory { reservation_id });
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RetireOrphanReplacesLostIntegrationEvidenceAndSurvivesReplayHistory {
    pub(super) reservation_id: String,
}

pub(super) const ROUND_THREE_RELEASE_USES_TRUNK_AFTER_THE_RESERVATION_TARGET_DISAPPEARS:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-round-three-release-uses-trunk-after-the-reservation-target-disappears",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
        let lane = repo.lane("missing-target-release-lane", "integration");
        let claimed = repo.claim(&lane, "file:landed.txt", FIRST_RUN, None);
        assert_success(&claimed);
        let id = json(&claimed)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("reservation id")
            .to_owned();
        repo.commit_file(&lane, "landed.txt", "landed\n", "lane work");
        let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
        assert_success(&checkpoint);
        repo.merge_by_commit("missing-target-release-lane");
        let first_release = repo.run(&lane, &["release", &id, "--json"]);
        assert_success(&first_release);
        assert_eq!(json(&first_release)["status"], "integrated");
        repo.git(repo.root(), &["merge", "--ff-only", "integration"]);

        recorded_history::save(
            destination,
            repo.root(),
            &ReleaseUsesTrunkAfterTheReservationTargetDisappearsHistory { lane, id },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct ReleaseUsesTrunkAfterTheReservationTargetDisappearsHistory {
    pub(super) lane: PathBuf,
    pub(super) id:   String,
}

pub(super) const ROUND_THREE_RETARGET_REFUSES_RELEASED_RESERVATION:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "liveness-round-three-retarget-refuses-released-reservation",
    |destination| {
        let repo = IntegrationRepository::from_recorded_template(INITIAL_INTEGRATION_PAIR);
        let lane = repo.lane("released-retarget-lane", "integration");
        repo.commit_file(
            &lane,
            "released-retarget.txt",
            "lane work\n",
            "work to release",
        );
        let claim = repo.claim(&lane, "file:released-retarget.txt", FIRST_RUN, None);
        assert_success(&claim);
        let reservation_id = json(&claim)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("claim reservation ID")
            .to_owned();
        let release = repo.run(&lane, &["release", &reservation_id, "--json"]);
        assert_success(&release);
        repo.merge_fast_forward("released-retarget-lane");
        let settled = repo.run(repo.root(), &["board", "--json"]);
        assert_success(&settled);
        assert!(
            repo.journal()
                .iter()
                .any(|event| event["op"] == "release" && event["reservation_id"] == reservation_id)
        );

        recorded_history::save(
            destination,
            repo.root(),
            &RetargetRefusesReleasedReservationHistory {
                lane,
                reservation_id,
            },
        );
    },
);
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct RetargetRefusesReleasedReservationHistory {
    pub(super) lane:           PathBuf,
    pub(super) reservation_id: String,
}

pub(super) const ROUND_THREE_PRUNABLE_AND_PRUNED_WORKTREES_KEEP_BLOCKING_AND_PRUNED_WORK_REPORTS_RECOVERY: cargo_berth_test_support::LedgerTemplate = recorded_history::template("liveness-round-three-prunable-and-pruned-worktrees-keep-blocking-and-pruned-work-reports-recovery", |destination| {

    let repository = initialized_repository();
    let worktree_parent = tempfile::tempdir().expect("worktree parent should exist");
    let phase_worktree = worktree_parent.path().join("phase");
    GIT.add_worktree_without_hooks(repository.path(), &phase_worktree, "phase", "HEAD");
    commit_file(&phase_worktree, "src/lib.rs", "phase work\n", "phase work");
    let claim = run_berth(
        &phase_worktree,
        &["claim", "tree:src", "--run", FIRST_RUN, "--json"],
    );
    assert!(
        claim.status.success(),
        "claim failed: stdout={} stderr={}",
        String::from_utf8_lossy(&claim.stdout),
        String::from_utf8_lossy(&claim.stderr)
    );
    let reservation_id = reservation_id(&claim);
    let checkpoint = run_berth(&phase_worktree, &["release", &reservation_id, "--json"]);
    assert!(checkpoint.status.success());
    let protected_tip = json_output(&checkpoint)["payload"]["data"]["protected_tip"]
        .as_str()
        .expect("checkpoint should report a protected tip")
        .to_owned();


        recorded_history::save(destination, repository.path(), &PrunableAndPrunedWorktreesKeepBlockingAndPrunedWorkReportsRecoveryHistory { phase_worktree, reservation_id, protected_tip });
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct PrunableAndPrunedWorktreesKeepBlockingAndPrunedWorkReportsRecoveryHistory {
    pub(super) phase_worktree: PathBuf,
    pub(super) reservation_id: String,
    pub(super) protected_tip:  String,
}
