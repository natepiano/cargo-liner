//! Recorded work before checkpoint-free integration recovery.

use std::fs;
use std::path::Path;
use std::path::PathBuf;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::IntegrationRepository;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::claim_id;

use crate::recorded_history;

const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

pub(super) const MERGED_ACTIVE_HOLDER: LedgerTemplate =
    recorded_history::template("liveness-merged-active-holder-without-checkpoint", |root| {
        build(root, WorkLanding::Merged);
    });
pub(super) const LATER_UNMERGED_HOLDER: LedgerTemplate = recorded_history::template(
    "liveness-later-unmerged-holder-without-checkpoint",
    |root| build(root, WorkLanding::LaterUnmerged),
);

pub(super) const SWITCHED_BRANCH_HOLDER: LedgerTemplate = recorded_history::template(
    "liveness-switched-branch-holder-without-checkpoint",
    |root| build(root, WorkLanding::SwitchedBranch),
);

pub(super) const CLEAN_MERGED_HOLDER: LedgerTemplate =
    recorded_history::template("liveness-clean-merged-holder-without-checkpoint", |root| {
        build(root, WorkLanding::MergedBeforeClaim);
    });

#[derive(Clone, Copy)]
enum WorkLanding {
    Merged,
    LaterUnmerged,
    SwitchedBranch,
    MergedBeforeClaim,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AbsentHolderHistory {
    pub(super) reservation_id: String,
    pub(super) holder:         PathBuf,
    pub(super) claim_head:     String,
    pub(super) work_head:      String,
    pub(super) target:         String,
}

fn build(destination: &Path, landing: WorkLanding) {
    let repository = IntegrationRepository::new(EXECUTABLE);
    let holder = repository.lane("absent-holder", "main");
    if matches!(landing, WorkLanding::MergedBeforeClaim) {
        commit_without_hooks(&repository, &holder, "first work\n");
        merge_holder(&repository, "absent-holder");
    }
    let claim_head = repository.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let claim = repository.claim(&holder, "file:work.txt", FIRST_RUN, Some("main"));
    assert_success(&claim);
    let reservation_id = claim_id(&claim);
    let work_branch = if matches!(landing, WorkLanding::SwitchedBranch) {
        repository.git(&holder, &["checkout", "--quiet", "-b", "switched-holder"]);
        "switched-holder"
    } else {
        "absent-holder"
    };
    if matches!(landing, WorkLanding::MergedBeforeClaim) {
        assert_success(&repository.run(repository.root(), &["board", "--json"]));
    } else {
        commit_without_hooks(&repository, &holder, "first work\n");
        assert_success(&repository.run(repository.root(), &["board", "--json"]));
        merge_holder(&repository, work_branch);
    }
    let target = repository.git_stdout(repository.root(), &["rev-parse", "HEAD"]);
    if matches!(landing, WorkLanding::LaterUnmerged) {
        commit_without_hooks(&repository, &holder, "later work not merged\n");
        assert_success(&repository.run(repository.root(), &["board", "--json"]));
    }
    let work_head = repository.git_stdout(&holder, &["rev-parse", "HEAD"]);
    let journal = fs::read_to_string(repository.root().join(".git/cargo-berth/journal.ndjson"))
        .expect("fixture journal should read");
    assert!(!journal.contains("\"op\":\"checkpoint\""));
    assert!(journal.contains(&work_head));
    recorded_history::save(
        destination,
        repository.root(),
        &AbsentHolderHistory {
            reservation_id,
            holder,
            claim_head,
            work_head,
            target,
        },
    );
}

fn merge_holder(repository: &IntegrationRepository, branch: &str) {
    repository.git(
        repository.root(),
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--ff-only",
            branch,
        ],
    );
}

fn commit_without_hooks(repository: &IntegrationRepository, holder: &Path, content: &str) {
    fs::write(holder.join("work.txt"), content).expect("holder work should write");
    repository.git(holder, &["add", "work.txt"]);
    repository.git(
        holder,
        &[
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            "holder work",
        ],
    );
}
