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
use cargo_berth_test_support::claim_id;
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

/// An outstanding checkpoint followed by an unmerged commit.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckpointWithLaterWork {
    pub(super) reservation_id: String,
    pub(super) checkpoint_tip: String,
}

pub(super) const CHECKPOINT_WITH_LATER_WORK: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-with-later-work",
        build: |root| build_checkpoint_with_later_work(root, "integration"),
        ..ENDED_FIRST_COVER.repository
    },
    ..ENDED_FIRST_COVER
};

pub(super) const TRUNK_CHECKPOINT_WITH_LATER_WORK: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "trunk-checkpoint-with-later-work",
        build: |root| build_checkpoint_with_later_work(root, "main"),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const CHECKPOINT_LANE: &str = "partly-landed-lane";

pub(super) const ACTIVE_BEFORE_LATER_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-before-later-phase",
        build: |root| build_shared_phase_history(root, EarlierPhase::Active),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-before-later-kept-checkpoint",
        build: |root| build_active_before_later_kept_checkpoint(root, KeptWork::Checkpoint),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_BEFORE_LATER_KEPT_SUFFIX: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-before-later-kept-suffix",
        build: |root| build_active_before_later_kept_checkpoint(root, KeptWork::AfterCheckpoint),
        ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT.repository
    },
    ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT
};

pub(super) const ACTIVE_WITH_PRUNED_KEPT_BRANCH: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-pruned-kept-branch",
        build: |root| build_active_before_later_kept_checkpoint(root, KeptWork::PrunedBranch),
        ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT.repository
    },
    ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT
};

pub(super) const FOREIGN_KEPT_ANCESTOR: &str = ".git/foreign-kept-ancestor";

pub(super) const ACTIVE_WITH_SHARED_KEPT_ANCESTOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-shared-kept-ancestor",
        build: |root| build_active_before_later_kept_checkpoint(root, KeptWork::SharedAncestor),
        ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT.repository
    },
    ..ACTIVE_BEFORE_LATER_KEPT_CHECKPOINT
};

#[derive(Clone, Copy)]
enum KeptWork {
    Checkpoint,
    AfterCheckpoint,
    PrunedBranch,
    SharedAncestor,
}

fn build_active_before_later_kept_checkpoint(root: &Path, kept_work: KeptWork) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let first = repo.claim(&holder, "file:phase.rs", FIRST_RUN, None);
    assert_success(&first);
    repo.commit_file(&holder, "phase.rs", "first phase\n", "first phase");
    let later = repo.claim(&holder, "file:later.rs", FIRST_RUN, None);
    assert_success(&later);
    let later_id = claim_id(&later);
    repo.commit_file(&holder, "later.rs", "later phase\n", "later phase");
    let later_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let checkpoint = repo.run(&holder, &["release", &later_id, "--json"]);
    assert_success(&checkpoint);
    assert_eq!(json(&checkpoint)["status"], "outstanding");
    if !matches!(kept_work, KeptWork::Checkpoint) {
        repo.commit_file(&holder, "later.rs", "kept suffix\n", "kept suffix");
    }
    let kept_parent = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    if matches!(kept_work, KeptWork::SharedAncestor) {
        repo.commit_file(&holder, "later.rs", "kept tip\n", "kept tip");
    }
    let kept_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let kept = repo.run(
        &holder,
        &[
            "release",
            &later_id,
            "--keep-work",
            "--why",
            "keep later phase",
            "--json",
        ],
    );
    assert_success(&kept);
    assert_eq!(json(&kept)["status"], "released");
    if !matches!(kept_work, KeptWork::Checkpoint) {
        repo.git(repo.root(), &["merge", "--quiet", "--ff-only", &later_tip]);
        match kept_work {
            KeptWork::PrunedBranch => {
                repo.git(&holder, &["branch", "discarded-work", &kept_tip]);
                repo.git(&holder, &["reset", "--hard", "main"]);
                repo.git(&holder, &["branch", "-D", "discarded-work"]);
            },
            KeptWork::SharedAncestor => repo.git(&holder, &["reset", "--hard", &kept_parent]),
            KeptWork::Checkpoint | KeptWork::AfterCheckpoint => {},
        }
        repo.commit_file(
            &holder,
            "phase.rs",
            "first phase continues\n",
            "first phase continues",
        );
    }
    if matches!(kept_work, KeptWork::PrunedBranch | KeptWork::SharedAncestor) {
        let _ = repo.board();
        if matches!(kept_work, KeptWork::SharedAncestor) {
            repo.git(
                repo.root(),
                &[
                    "update-ref",
                    "-d",
                    &format!("refs/cargo-berth/kept-work/{later_id}"),
                ],
            );
        }
        repo.git(repo.root(), &["reflog", "expire", "--expire=now", "--all"]);
        repo.git(repo.root(), &["gc", "--prune=now"]);
    }
    record_checkpoint_fixture(root, &repo, claim_id(&first), kept_tip);
    if matches!(kept_work, KeptWork::SharedAncestor) {
        fs::write(root.join(FOREIGN_KEPT_ANCESTOR), kept_parent)
            .expect("foreign ancestor should record");
    }
}

pub(super) const ACTIVE_WITH_SHARED_PHASE_START: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-shared-phase-start",
        build: |root| build_shared_phase_history(root, EarlierPhase::SharedStart),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_WITH_LATER_PHASE_AND_MISSING_TARGET: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-later-phase-and-missing-target",
        build: |root| build_shared_phase_history(root, EarlierPhase::MissingTarget),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const CHECKPOINT_BEFORE_BACKDATED_CLAIM: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-before-backdated-claim",
        build: build_checkpoint_before_backdated_claim,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const CHECKPOINT_BEFORE_DISJOINT_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-before-disjoint-phase",
        build: |root| build_shared_phase_history(root, EarlierPhase::Checkpointed),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const TWO_OUTSTANDING_IN_ONE_CHECKOUT: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "two-outstanding-in-one-checkout",
        build: build_two_outstanding_in_one_checkout,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

fn build_two_outstanding_in_one_checkout(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let _requester = repo.main_lane("conflict-requester");
    let first = repo.claim(&holder, "file:first.rs", FIRST_RUN, None);
    assert_success(&first);
    let oldest = claim_id(&first);
    repo.commit_file(
        &holder,
        "first.rs",
        "first checkpoint\n",
        "first checkpoint",
    );
    let first_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let second = repo.claim(&holder, "file:second.rs", FIRST_RUN, None);
    assert_success(&second);
    let newer = claim_id(&second);
    repo.commit_file(
        &holder,
        "second.rs",
        "second checkpoint\n",
        "second checkpoint",
    );
    for id in [&oldest, &newer] {
        let released = repo.run(&holder, &["release", id, "--json"]);
        assert_success(&released);
        assert_eq!(json(&released)["status"], "outstanding");
    }
    record_checkpoint_fixture(root, &repo, oldest, first_tip);
}

pub(super) const CHECKPOINT_WITH_FOREIGN_MERGE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-with-foreign-merge",
        build: build_checkpoint_with_foreign_merge,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_WITH_INTERLEAVED_OLDER_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-interleaved-older-phase",
        build: build_interleaved_older_phase,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_AFTER_OUTSTANDING_PHASE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-after-outstanding-phase",
        build: build_active_after_outstanding_phase,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_WITH_DISJOINT_FOREIGN_MERGE: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-disjoint-foreign-merge",
        build: build_disjoint_foreign_merge,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const ACTIVE_WITH_BACKDATED_FOREIGN_START: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "active-with-backdated-foreign-start",
        build: build_backdated_foreign_start,
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

pub(super) const CHECKPOINT_WITH_EDIT_BEFORE_OVERLAP: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "checkpoint-with-edit-before-overlap",
        build: |root| build_shared_phase_history(root, EarlierPhase::EditedBeforeOverlap),
        ..CHECKPOINT_WITH_LATER_WORK.repository
    },
    ..CHECKPOINT_WITH_LATER_WORK
};

#[derive(Clone, Copy)]
enum EarlierPhase {
    Active,
    SharedStart,
    MissingTarget,
    Checkpointed,
    EditedBeforeOverlap,
}

fn build_shared_phase_history(root: &Path, earlier: EarlierPhase) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = if matches!(earlier, EarlierPhase::MissingTarget) {
        repo.lane(CHECKPOINT_LANE, "integration")
    } else {
        repo.main_lane(CHECKPOINT_LANE)
    };
    let claim = repo.claim(&holder, "file:phase.rs", FIRST_RUN, None);
    assert_success(&claim);
    let reservation_id = claim_id(&claim);
    if matches!(earlier, EarlierPhase::SharedStart) {
        assert_success(&repo.claim(&holder, "file:later.rs", FIRST_RUN, None));
    }
    repo.commit_file(&holder, "phase.rs", "first phase\n", "first phase");
    let checkpoint_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let later_path = match earlier {
        EarlierPhase::Active | EarlierPhase::MissingTarget => {
            assert_success(&repo.claim(&holder, "file:later.rs", FIRST_RUN, None));
            "later.rs"
        },
        EarlierPhase::SharedStart => "later.rs",
        EarlierPhase::Checkpointed => {
            assert_success(&repo.run(&holder, &["release", &reservation_id, "--json"]));
            repo.git(repo.root(), &["config", "core.ignorecase", "true"]);
            assert_success(&repo.claim(&holder, "file:LATER.rs", SECOND_RUN, None));
            "later.rs"
        },
        EarlierPhase::EditedBeforeOverlap => {
            assert_success(&repo.run(&holder, &["release", &reservation_id, "--json"]));
            fs::write(holder.join("phase.rs"), "first phase pending edit\n")
                .expect("pending edit should write");
            assert_success(&repo.claim(&holder, "file:phase.rs", SECOND_RUN, None));
            "phase.rs"
        },
    };
    let content = if matches!(earlier, EarlierPhase::EditedBeforeOverlap) {
        "first phase pending edit\n"
    } else {
        "second phase\n"
    };
    repo.commit_file(&holder, later_path, content, "pending work");
    if matches!(
        earlier,
        EarlierPhase::Checkpointed | EarlierPhase::EditedBeforeOverlap
    ) {
        repo.git(
            repo.root(),
            &["merge", "--quiet", "--ff-only", &checkpoint_tip],
        );
    }
    record_checkpoint_fixture(root, &repo, reservation_id, checkpoint_tip);
}

fn build_interleaved_older_phase(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    assert_success(&repo.claim(&holder, "file:b.rs", FIRST_RUN, None));
    repo.commit_file(&holder, "b.rs", "older work\n", "older work");
    let boundary = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let claimed = repo.claim(&holder, "file:a.rs", FIRST_RUN, None);
    assert_success(&claimed);
    repo.commit_file(&holder, "b.rs", "more older work\n", "more older work");
    repo.commit_file(&holder, "a.rs", "newer work\n", "newer work");
    record_checkpoint_fixture(root, &repo, claim_id(&claimed), boundary);
}

fn build_active_after_outstanding_phase(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let earlier = repo.claim(&holder, "file:b.rs", FIRST_RUN, None);
    assert_success(&earlier);
    let earlier = claim_id(&earlier);
    repo.commit_file(&holder, "b.rs", "first checkpoint\n", "first checkpoint");
    assert_success(&repo.run(&holder, &["release", &earlier, "--json"]));
    let claimed = repo.claim(&holder, "file:a.rs", FIRST_RUN, None);
    assert_success(&claimed);
    repo.commit_file(
        &holder,
        "b.rs",
        "older pending work\n",
        "older pending work",
    );
    let earlier_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let updated = repo.run(&holder, &["release", &earlier, "--json"]);
    assert_success(&updated);
    assert_eq!(
        repo.git_stdout(
            repo.root(),
            &[
                "rev-parse",
                &format!("refs/cargo-berth/reservations/{earlier}")
            ],
        ),
        earlier_tip
    );
    repo.commit_file(&holder, "a.rs", "newer work\n", "newer work");
    record_checkpoint_fixture(root, &repo, claim_id(&claimed), earlier_tip);
}

fn build_disjoint_foreign_merge(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let foreign = repo.main_lane("foreign-phase");
    assert_success(&repo.claim(&foreign, "file:b.rs", SECOND_RUN, None));
    repo.commit_file(&foreign, "b.rs", "foreign work\n", "foreign work");
    let foreign_tip = repo.git_stdout(&foreign, &["rev-parse", "HEAD"]);
    let claimed = repo.claim(&holder, "file:a.rs", FIRST_RUN, None);
    assert_success(&claimed);
    repo.commit_file(&holder, "a.rs", "holder work\n", "holder work");
    repo.git(
        &holder,
        &["merge", "--quiet", "--no-ff", "--no-edit", "foreign-phase"],
    );
    record_checkpoint_fixture(root, &repo, claim_id(&claimed), foreign_tip);
}

fn build_backdated_foreign_start(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let foreign = repo.main_lane("foreign-phase");
    let foreign_claim = repo.claim(&foreign, "file:phase.rs", SECOND_RUN, None);
    assert_success(&foreign_claim);
    repo.commit_file(&foreign, "phase.rs", "foreign work\n", "foreign work");
    let foreign_tip = repo.git_stdout(&foreign, &["rev-parse", "HEAD"]);
    repo.commit_file(&holder, "other.rs", "holder boundary\n", "holder boundary");
    let claimed = repo.run(
        &holder,
        &[
            "claim",
            "file:phase.rs",
            "--run",
            FIRST_RUN,
            "--head",
            &foreign_tip,
            "--override",
            &claim_id(&foreign_claim),
            "--overlap-why",
            "reviewed phase overlap",
            "--json",
        ],
    );
    assert_success(&claimed);
    repo.git(
        &holder,
        &["merge", "--quiet", "--no-ff", "--no-edit", "foreign-phase"],
    );
    repo.commit_file(&holder, "phase.rs", "holder work\n", "holder work");
    record_checkpoint_fixture(root, &repo, claim_id(&claimed), foreign_tip);
}

fn build_checkpoint_before_backdated_claim(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let claim = repo.claim(&holder, "file:phase.rs", FIRST_RUN, None);
    assert_success(&claim);
    let reservation_id = claim_id(&claim);
    repo.commit_file(&holder, "phase.rs", "first phase\n", "first phase");
    let checkpoint_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    assert_success(&repo.run(&holder, &["release", &reservation_id, "--json"]));
    repo.commit_file(
        &holder,
        "phase.rs",
        "more first phase work\n",
        "more first phase work",
    );
    assert_success(&repo.run(
        &holder,
        &[
            "claim",
            "file:phase.rs",
            "--head",
            &checkpoint_tip,
            "--run",
            SECOND_RUN,
            "--json",
        ],
    ));
    repo.git(
        repo.root(),
        &["merge", "--quiet", "--ff-only", &checkpoint_tip],
    );
    record_checkpoint_fixture(root, &repo, reservation_id, checkpoint_tip);
}

fn build_checkpoint_with_foreign_merge(root: &Path) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    repo.git(repo.root(), &["config", "core.hooksPath", "/dev/null"]);
    repo.commit_file(
        repo.root(),
        "phase.rs",
        "first line\n\n\n\n\nlast line\n",
        "base",
    );
    let holder = repo.main_lane(CHECKPOINT_LANE);
    let outsider = repo.main_lane("foreign-phase");
    let claim = repo.claim(&holder, "file:phase.rs", FIRST_RUN, None);
    assert_success(&claim);
    let reservation_id = claim_id(&claim);
    repo.commit_file(
        &holder,
        "phase.rs",
        "first phase\n\n\n\n\nlast line\n",
        "first phase",
    );
    let checkpoint_tip = repo.git_stdout(&holder, &["rev-parse", "HEAD"]);
    assert_success(&repo.run(&holder, &["release", &reservation_id, "--json"]));
    assert_success(&repo.run(
        &outsider,
        &[
            "claim",
            "file:phase.rs",
            "--run",
            SECOND_RUN,
            "--override",
            &reservation_id,
            "--overlap-why",
            "reviewed phase overlap",
            "--json",
        ],
    ));
    repo.commit_file(
        &outsider,
        "phase.rs",
        "first line\n\n\n\n\nforeign phase\n",
        "foreign phase",
    );
    repo.git(
        &holder,
        &["merge", "--quiet", "--no-ff", "--no-edit", "foreign-phase"],
    );
    record_checkpoint_fixture(root, &repo, reservation_id, checkpoint_tip);
}

fn build_checkpoint_with_later_work(root: &Path, target: &str) {
    let repo = IntegrationRepository::from_recorded_template(crate::INITIAL_INTEGRATION_PAIR);
    let lane = repo.lane(CHECKPOINT_LANE, target);
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
    record_checkpoint_fixture(root, &repo, reservation_id, checkpoint_tip);
}

fn record_checkpoint_fixture(
    root: &Path,
    repo: &IntegrationRepository,
    reservation_id: String,
    checkpoint_tip: String,
) {
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
