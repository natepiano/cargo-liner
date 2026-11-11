//! Lane preparation before live integration and cover transitions.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::SECOND_RUN;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::json;

pub(super) const STATE: &str = ".git/integration-preparation-id";

pub(super) const ENDED_FIRST_COVER: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        target_directory: env!("CARGO_TARGET_TMPDIR"),
        executable:       env!("CARGO_BIN_EXE_cargo-berth"),
        name:             "ended-first-cover",
        build:            build_ended_first_cover,
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

pub(super) const MERGE_EXTENT_LANES_READY: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "merge-extent-lanes-ready",
        build: build_merge_extent_lanes,
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

pub(super) const RELEASE_LANE_READY: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "release-lane-ready",
        build: build_release_lane,
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

fn build_ended_first_cover(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let first = repo.lane("first-cover-lane", "integration");
    let claimed = repo.claim(&first, "file:first.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let _ = repo.board();
    let id = repo.cover_claims()[0]["reservation_id"]
        .as_str()
        .expect("first cover ID")
        .to_owned();
    repo.commit_file(&first, "first.txt", "first\n", "first lane work");
    repo.merge_fast_forward("first-cover-lane");
    let _ = repo.board();
    repo.git(repo.root(), &["merge", "--ff-only", "integration"]);
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("cover id should write");
}

fn build_merge_extent_lanes(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let a = repo.lane("lane-a", "integration");
    let b = repo.lane("lane-b", "integration");
    let claimed = repo.claim(&a, "file:shared.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane A reservation id")
        .to_owned();
    assert_success(&repo.claim(&b, "file:b.txt", SECOND_RUN, None));
    repo.commit_file(&a, "shared.txt", "lane A\n", "lane A work");
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("lane id should write");
}

fn build_release_lane(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("landed-lane", "integration");
    let claimed = repo.claim(&lane, "file:landed.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(&lane, "landed.txt", "landed\n", "landed lane");
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("lane id should write");
}

/// A released checkpoint with a later commit before either commit lands.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckpointWithLaterWork {
    pub(super) reservation_id: String,
    pub(super) checkpoint_tip: String,
}

pub(super) const CHECKPOINT_WITH_LATER_WORK: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-with-later-work",
        build: build_checkpoint_with_later_work,
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

fn build_checkpoint_with_later_work(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("partly-landed-lane", "integration");
    let claimed = repo.claim(&lane, "file:checkpoint.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let reservation_id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("reservation id")
        .to_owned();
    repo.commit_file(&lane, "checkpoint.txt", "checkpoint\n", "checkpoint work");
    let checkpoint_tip = repo.git_stdout(&lane, &["rev-parse", "HEAD"]);
    assert_success(&repo.run(&lane, &["release", &reservation_id, "--json"]));
    repo.commit_file(&lane, "later.txt", "later\n", "later work");
    repo.write_recorded_template(root);
    let state = CheckpointWithLaterWork {
        reservation_id,
        checkpoint_tip,
    };
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&state).expect("checkpoint state should encode"),
    )
    .expect("checkpoint state should write");
}
