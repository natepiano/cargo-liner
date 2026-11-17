//! Ordered holders prepared before the board test clocks start.

use std::fs;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::SECOND_RUN;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::claim_id;

use super::proof_subject_templates::INITIALIZED_REPOSITORY;
use super::proof_subject_templates::STATE;

const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Refused,
    cleared_environment: &[],
};

/// Two dirty holders with their accepted ordering constraint.
pub(crate) const ORDERED_HOLDERS: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "board-ordered-holders",
        build: build_ordered_holders,
        ..INITIALIZED_REPOSITORY
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// The predecessor has checkpointed work that has not reached main.
pub(crate) const CHECKPOINTED_PREDECESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "board-checkpointed-predecessor",
        build: build_checkpointed_predecessor,
        ..ORDERED_HOLDERS.repository
    },
    ..ORDERED_HOLDERS
};

/// Main contains the checkpoint while the successor still needs it.
pub(crate) const LANDED_PREDECESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "board-landed-predecessor",
        build: build_landed_predecessor,
        ..ORDERED_HOLDERS.repository
    },
    ..ORDERED_HOLDERS
};

/// Both main and the successor contain the predecessor checkpoint.
pub(crate) const INCORPORATED_PREDECESSOR: LedgerTemplate = LedgerTemplate {
    repository: RepositoryTemplate {
        name: "board-incorporated-predecessor",
        build: build_incorporated_predecessor,
        ..ORDERED_HOLDERS.repository
    },
    ..ORDERED_HOLDERS
};

/// Reservation identities after the restored ledger renews its owners.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct OrderedHolders {
    pub(crate) predecessor_id: String,
    pub(crate) successor_id:   String,
}

fn build_ordered_holders(root: &Path) {
    INITIALIZED_REPOSITORY.restore_into(root);
    GIT.run(root, ["add", "--force", ".claude/config/berth.toml"]);
    GIT.run_without_hooks(root, ["commit", "--quiet", "-m", "configure berth"]);
    let worktrees = root.join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees).expect("ordered worktree parent should exist");
    for branch in ["predecessor", "successor"] {
        GIT.add_worktree_without_hooks(root, &worktrees.join(branch), branch, "main");
    }
    let predecessor = worktrees.join("predecessor");
    fs::write(
        predecessor.join("src/lib.rs"),
        "// uncommitted holder work\n",
    )
    .expect("predecessor source should write");
    let claimed = run(
        &predecessor,
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            FIRST_RUN,
            "--why",
            "protect board test work",
            "--json",
        ],
    );
    let predecessor_id = claim_id(&claimed);
    let answered = run(
        &worktrees.join("successor"),
        &[
            "claim",
            "file:src/lib.rs",
            "--run",
            SECOND_RUN,
            "--after",
            &predecessor_id,
            "--overlap-why",
            "predecessor must land first",
            "--why",
            "protect answered board work",
            "--json",
        ],
    );
    let successor_id = claim_id(&answered);
    fs::write(
        root.join(STATE),
        serde_json::to_vec(&OrderedHolders {
            predecessor_id,
            successor_id,
        })
        .expect("ordered identities should encode"),
    )
    .expect("ordered identities should write");
}

fn build_checkpointed_predecessor(root: &Path) {
    ORDERED_HOLDERS.restore_into(root);
    drop(run(root, &["board", "--json"]));
    let state: OrderedHolders = serde_json::from_slice(
        &fs::read(root.join(STATE)).expect("ordered identities should read"),
    )
    .expect("ordered identities should decode");
    let predecessor = root.join(".git/fixture-worktrees/predecessor");
    fs::write(predecessor.join("src/lib.rs"), "pub fn predecessor() {}\n")
        .expect("predecessor source should write");
    GIT.run(&predecessor, ["add", "src/lib.rs"]);
    GIT.run(
        &predecessor,
        ["commit", "--quiet", "-m", "predecessor work"],
    );
    let released = berth_command(BERTH_EXECUTABLE)
        .current_dir(predecessor)
        .args(["release", &state.predecessor_id, "--json"])
        .env("CARGO_BERTH_RUN", FIRST_RUN)
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("predecessor release should run");
    assert_success(&released);
}

fn build_landed_predecessor(root: &Path) {
    CHECKPOINTED_PREDECESSOR.restore_into(root);
    drop(run(root, &["board", "--json"]));
    GIT.run(root, ["merge", "--quiet", "--ff-only", "predecessor"]);
}

fn build_incorporated_predecessor(root: &Path) {
    LANDED_PREDECESSOR.restore_into(root);
    drop(run(root, &["board", "--json"]));
    GIT.run(
        &root.join(".git/fixture-worktrees/successor"),
        ["merge", "--quiet", "--ff-only", "main"],
    );
}

fn run(root: &Path, arguments: &[&str]) -> Output {
    let output = berth_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("ordered fixture command should run");
    assert_success(&output);
    output
}
