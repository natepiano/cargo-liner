//! Recorded phase history before the drift test rebases its shared branch.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::CommandDeadlineExt as _;
use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::json;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &["CARGO_BERTH_BYPASS", "CARGO_BERTH_RUN"],
};
pub(super) const STATE_PATH: &str = ".git/two-phase-rebase.json";
pub(super) const FEATURE_PATH: &str = ".git/fixture-worktrees/feature";

/// Two active phases and an upstream commit before the tested rebase.
pub(super) const TWO_PHASE_REBASE: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       BERTH_EXECUTABLE,
        name:             "two-phase-rebase",
        build:            build_two_phase_rebase,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// The reservations whose separate anchors the real rebase must preserve.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TwoPhaseRebaseState {
    pub(super) first:  String,
    pub(super) second: String,
}

fn build_two_phase_rebase(root: &Path) {
    IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE);
    let feature = root.join(FEATURE_PATH);
    GIT.add_worktree_without_hooks(root, &feature, "feature", "HEAD");
    let first = claim_phase(&feature, "file:alpha.txt");
    fs::write(feature.join("alpha.txt"), "alpha\n").expect("alpha path should write");
    GIT.run(&feature, ["add", "alpha.txt"]);
    GIT.run(&feature, ["commit", "--quiet", "-m", "alpha"]);
    let second = claim_phase(&feature, "file:beta.txt");
    fs::write(feature.join("beta.txt"), "beta\n").expect("beta path should write");
    GIT.run(&feature, ["add", "beta.txt"]);
    GIT.run(&feature, ["commit", "--quiet", "-m", "beta"]);
    fs::write(root.join("upstream.txt"), "upstream\n").expect("upstream path should write");
    GIT.run(root, ["add", "upstream.txt"]);
    GIT.run(root, ["commit", "--quiet", "-m", "upstream work"]);
    fs::write(
        root.join(STATE_PATH),
        serde_json::to_vec(&TwoPhaseRebaseState { first, second })
            .expect("phase identifiers should encode"),
    )
    .expect("phase identifiers should write");
}

fn claim_phase(root: &Path, scope: &str) -> String {
    let claimed = berth_command(BERTH_EXECUTABLE)
        .args([
            "claim",
            scope,
            "--run",
            FIRST_RUN,
            "--why",
            "test work",
            "--json",
        ])
        .current_dir(root)
        .env_remove("CARGO_BERTH_BYPASS")
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .bounded_output()
        .expect("phase claim should run");
    assert!(
        claimed.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&claimed.stdout)
    );
    json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}
