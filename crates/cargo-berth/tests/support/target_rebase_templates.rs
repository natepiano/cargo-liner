//! Outstanding checkpoints before their target changes and a live rebase starts.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::json;

pub(super) const STATE: &str = ".git/target-rebase-state.json";
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct TargetCheckpoint {
    pub(super) id:     String,
    pub(super) anchor: String,
}

pub(super) const MISSING_TARGET_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       env!("CARGO_BIN_EXE_cargo-berth"),
        name:             "missing-target-outstanding-checkpoint",
        build:            |root| build_checkpoint(root, TargetChange::Missing),
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};
pub(super) const ADVANCED_TARGET_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "advanced-target-outstanding-checkpoint",
        build: |root| build_checkpoint(root, TargetChange::Advanced),
        ..MISSING_TARGET_CHECKPOINT.repository
    },
    ..MISSING_TARGET_CHECKPOINT
};

#[derive(Clone, Copy)]
enum TargetChange {
    Missing,
    Advanced,
}

fn build_checkpoint(root: &Path, change: TargetChange) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let branch = match change {
        TargetChange::Missing => "missing-target-rebase-lane",
        TargetChange::Advanced => "target-rebase-lane",
    };
    let lane = repo.lane(branch, "integration");
    let claimed = repo.claim(&lane, "file:lane.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(&lane, "lane.txt", "lane work\n", "lane work");
    let checkpoint = repo.run(&lane, &["release", &id, "--json"]);
    assert_success(&checkpoint);
    let anchor = match change {
        TargetChange::Missing => {
            repo.commit_file(repo.root(), "new-trunk.txt", "new trunk\n", "advance trunk");
            repo.git_stdout(repo.root(), &["rev-parse", "HEAD"])
        },
        TargetChange::Advanced => {
            repo.commit_file(
                &repo.integration,
                "new-target.txt",
                "new target work\n",
                "advance integration",
            );
            repo.git_stdout(&repo.integration, &["rev-parse", "HEAD"])
        },
    };
    repo.write_recorded_template(root);
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&TargetCheckpoint { id, anchor })
            .expect("target checkpoint should encode"),
    )
    .expect("target checkpoint should write");
}
