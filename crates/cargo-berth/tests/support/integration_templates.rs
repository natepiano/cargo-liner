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

pub(super) const MERGED_TRUNK_ONLY: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "merged-trunk-only",
        build: |root| build_merged_trunk(root, BranchWork::TrunkOnly),
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

pub(super) const MERGED_TRUNK_WITH_OWN_WORK: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "merged-trunk-with-own-work",
        build: |root| build_merged_trunk(root, BranchWork::OwnChange),
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

#[derive(Clone, Copy)]
enum BranchWork {
    TrunkOnly,
    OwnChange,
}

pub(super) const PENDING_TRUNK_MERGE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "pending-trunk-merge",
        build: build_pending_trunk_merge,
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

pub(super) const NEWER_TRUNK: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "newer-trunk-extent",
        build: build_newer_trunk,
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

fn build_newer_trunk(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane("older-lane", "integration");
    let holder = repo.lane("foreign-holder", "main");
    let own = repo.claim(&lane, "file:own.txt", FIRST_RUN, None);
    let foreign = repo.claim(&holder, "file:foreign.txt", SECOND_RUN, None);
    assert_success(&own);
    assert_success(&foreign);
    let id = json(&own)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane reservation")
        .to_owned();
    repo.commit_file(&lane, "own.txt", "own work\n", "unit work");
    repo.commit_file(
        &repo.integration,
        "foreign.txt",
        "other unit's landed work\n",
        "newer trunk",
    );
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("lane reservation id should write");
}

fn build_pending_trunk_merge(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    repo.commit_file(&repo.integration, "conflict.txt", "base\n", "conflict base");
    repo.commit_file(
        &repo.integration,
        "own.txt",
        "trunk base\n\n\n\nbranch base\n",
        "own base",
    );
    let lane = repo.lane("pending-merge-lane", "integration");
    drop(repo.lane("pending-merge-requester", "integration"));
    let claimed = repo.claim(&lane, "file:declared.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane reservation id")
        .to_owned();
    repo.commit_file(
        &lane,
        "conflict.txt",
        "branch conflict\n",
        "branch conflict",
    );
    repo.commit_file(
        &lane,
        "own.txt",
        "trunk base\n\n\n\nbranch work\n",
        "own branch work",
    );
    repo.commit_file(
        &repo.integration,
        "conflict.txt",
        "trunk conflict\n",
        "trunk conflict",
    );
    repo.commit_file(
        &repo.integration,
        "own.txt",
        "trunk work\n\n\n\nbranch base\n",
        "trunk work on the same path",
    );
    repo.commit_file(
        &repo.integration,
        "imported.txt",
        "trunk work\n",
        "imported path",
    );
    repo.commit_file(
        &repo.integration,
        "manual.txt",
        "trunk work\n",
        "manual base",
    );
    let merge = cargo_berth_test_support::git_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(["merge", "--no-ff", "--no-commit", "integration"])
        .current_dir(&lane)
        .output()
        .expect("pending merge should run");
    assert_eq!(
        merge.status.code(),
        Some(1),
        "the merge should have a conflict"
    );
    fs::write(lane.join("manual.txt"), "manual branch work\n").expect("manual edit should write");
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("lane reservation id should write");
}

fn build_merged_trunk(root: &Path, branch_work: BranchWork) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    repo.commit_file(
        &repo.integration,
        "shared.txt",
        "trunk base\n\n\n\nbranch base\n",
        "shared base",
    );
    let lane = repo.lane("merged-trunk-lane", "integration");
    drop(repo.lane("merge-requester", "integration"));
    let claimed = repo.claim(&lane, "file:shared.txt", FIRST_RUN, None);
    assert_success(&claimed);
    let id = json(&claimed)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("lane reservation id")
        .to_owned();
    match branch_work {
        BranchWork::TrunkOnly => {
            repo.git(
                &lane,
                &["commit", "--quiet", "--allow-empty", "-m", "lane marker"],
            );
        },
        BranchWork::OwnChange => repo.commit_file(
            &lane,
            "shared.txt",
            "trunk base\n\n\n\nbranch work\n",
            "branch work",
        ),
    }
    repo.commit_file(
        &repo.integration,
        "shared.txt",
        "trunk work\n\n\n\nbranch base\n",
        "trunk work on the shared path",
    );
    repo.commit_file(
        &repo.integration,
        "foreign.txt",
        "trunk work\n",
        "trunk file",
    );
    repo.git(
        &lane,
        &["merge", "--quiet", "--no-ff", "--no-edit", "integration"],
    );
    repo.commit_file(
        &repo.integration,
        "foreign.txt",
        "later trunk work\n",
        "advance trunk after the lane merge",
    );
    repo.write_recorded_template(root);
    fs::write(root.join(STATE), id).expect("lane reservation id should write");
}

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
