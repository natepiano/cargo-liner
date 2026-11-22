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
use crate::linked_checkout;
use crate::recorded_history;

const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
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
    GIT.run(root, ["add", "--force", CONFIGURATION_PATH]);
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

/// Claimed board subjects before the first observation.
pub(crate) const UNCONSTRAINED_HOLDERS: LedgerTemplate =
    recorded_history::template("board-unconstrained-holders", build_unconstrained_holders);

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UnconstrainedHolders {
    pub(crate) reservation_ids: Vec<String>,
    pub(crate) unrelated_id:    String,
}

fn build_unconstrained_holders(destination: &Path) {
    let repository = INITIALIZED_REPOSITORY.before_run().instantiate();
    git(repository.path(), &["add", "--force", CONFIGURATION_PATH]);
    GIT.run_without_hooks(
        repository.path(),
        ["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = repository.path().join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees).expect("fixture worktrees should exist");
    let live_root = add_worktree(repository.path(), &worktrees, "live-holder");
    let unavailable_root = add_worktree(repository.path(), &worktrees, "unavailable-holder");
    let candidate_root = add_worktree(repository.path(), &worktrees, "candidate-holder");
    let orphaned_root = add_worktree(repository.path(), &worktrees, "orphaned-holder");
    let unknown_root = add_worktree(repository.path(), &worktrees, "unknown-holder");
    let unrelated_root = add_worktree(repository.path(), &worktrees, "unrelated-holder");

    let roots = [
        &live_root,
        &unavailable_root,
        &candidate_root,
        &orphaned_root,
        &unknown_root,
    ];
    let reservation_ids = roots
        .iter()
        .enumerate()
        .map(|(index, root)| {
            claim_id(&claim(
                root,
                &format!("file:liveness-{index}.rs"),
                &uuid::Uuid::now_v7().to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let orphaned_administrative_directory = worktree_administrative_directory(&orphaned_root);
    let unknown_administrative_directory = worktree_administrative_directory(&unknown_root);

    git(
        repository.path(),
        &[
            "worktree",
            "lock",
            unavailable_root
                .to_str()
                .expect("unavailable root should be UTF-8"),
        ],
    );
    fs::remove_dir_all(&candidate_root).expect("candidate root should remove");
    fs::remove_dir_all(&orphaned_root).expect("orphaned root should remove");
    fs::remove_dir_all(orphaned_administrative_directory)
        .expect("orphaned registration should remove");
    fs::remove_file(unknown_administrative_directory.join("cargo-berth-worktree-id"))
        .expect("unknown holder identity should remove");

    git(
        &unrelated_root,
        &["checkout", "--quiet", "--orphan", "unrelated-root"],
    );
    git(&unrelated_root, &["add", "."]);
    git(
        &unrelated_root,
        &["commit", "--quiet", "-m", "unrelated root"],
    );
    let unrelated_id = claim_id(&claim(
        &unrelated_root,
        "file:unrelated.rs",
        &uuid::Uuid::now_v7().to_string(),
    ));

    recorded_history::save(
        destination,
        repository.path(),
        &UnconstrainedHolders {
            reservation_ids,
            unrelated_id,
        },
    );
}

/// Claimed board subjects before the first observation.
pub(crate) const UNRESOLVED_OVERLAP: LedgerTemplate =
    recorded_history::template("board-unresolved-overlap", build_unresolved_overlap);

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct UnresolvedOverlap {
    pub(crate) blocker_id:   String,
    pub(crate) deferred_id:  String,
    pub(crate) blocker_tip:  String,
    pub(crate) deferred_tip: String,
}

fn build_unresolved_overlap(destination: &Path) {
    let repository = INITIALIZED_REPOSITORY.before_run().instantiate();
    git(repository.path(), &["add", "--force", CONFIGURATION_PATH]);
    GIT.run_without_hooks(
        repository.path(),
        ["commit", "--quiet", "-m", "configure berth"],
    );
    let worktrees = repository.path().join(".git/fixture-worktrees");
    fs::create_dir_all(&worktrees).expect("overlap worktrees should exist");
    let blocker_root = add_worktree(repository.path(), &worktrees, "blocker");
    let deferred_root = add_worktree(repository.path(), &worktrees, "deferred");
    dirty_source(&blocker_root, "src/lib.rs");
    let blocker = claim(&blocker_root, "file:src/lib.rs", FIRST_RUN);
    let blocker_id = claim_id(&blocker);
    let deferred = defer_claim(&deferred_root, "file:src/lib.rs", SECOND_RUN, &blocker_id);
    let deferred_id = claim_id(&deferred);

    fs::write(blocker_root.join("src/lib.rs"), "pub fn blocker() {}\n")
        .expect("blocker source should write");
    git(&blocker_root, &["add", "src/lib.rs"]);
    git(&blocker_root, &["commit", "--quiet", "-m", "blocker work"]);
    let blocker_tip = git_stdout(&blocker_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth_with_run(
            &blocker_root,
            &["release", &blocker_id, "--json"],
            FIRST_RUN,
        )
        .status
        .success()
    );

    fs::write(deferred_root.join("src/lib.rs"), "pub fn deferred() {}\n")
        .expect("deferred source should write");
    git(&deferred_root, &["add", "src/lib.rs"]);
    git(
        &deferred_root,
        &["commit", "--quiet", "-m", "deferred work"],
    );
    let deferred_tip = git_stdout(&deferred_root, &["rev-parse", "HEAD"]);
    assert!(
        run_berth_with_run(
            &deferred_root,
            &["release", &deferred_id, "--json"],
            SECOND_RUN,
        )
        .status
        .success()
    );

    recorded_history::save(
        destination,
        repository.path(),
        &UnresolvedOverlap {
            blocker_id,
            deferred_id,
            blocker_tip,
            deferred_tip,
        },
    );
}

fn git(root: &Path, arguments: &[&str]) { GIT.run(root, arguments); }
fn git_stdout(root: &Path, arguments: &[&str]) -> String { GIT.stdout(root, arguments) }

fn add_worktree(root: &Path, parent: &Path, branch: &str) -> std::path::PathBuf {
    let checkout = parent.join(branch);
    GIT.add_worktree_without_hooks(root, &checkout, branch, "main");
    fs::canonicalize(checkout).expect("new worktree should canonicalize")
}

fn worktree_administrative_directory(root: &Path) -> std::path::PathBuf {
    linked_checkout::administrative_directory(root)
}

fn claim(root: &Path, scope: &str, run_id: &str) -> Output {
    run(
        root,
        &[
            "claim",
            scope,
            "--run",
            run_id,
            "--why",
            "protect board test work",
            "--json",
        ],
    )
}

fn dirty_source(root: &Path, path: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("dirty source should have a parent"))
        .expect("dirty source parent should exist");
    fs::write(file, "// uncommitted holder work\n").expect("dirty source should write");
}

fn defer_claim(root: &Path, scope: &str, run_id: &str, blocker: &str) -> Output {
    assert!(
        claim(
            root,
            &cargo_berth_test_support::deferring_run_scope(run_id),
            run_id
        )
        .status
        .success()
    );
    run(
        root,
        &[
            "claim",
            scope,
            "--run",
            run_id,
            "--defer",
            blocker,
            "--overlap-why",
            "the order is not known yet",
            "--why",
            "protect answered board work",
            "--json",
        ],
    )
}

fn run_berth_with_run(root: &Path, arguments: &[&str], run_id: &str) -> Output {
    berth_command(BERTH_EXECUTABLE)
        .current_dir(root)
        .args(arguments)
        .env("CARGO_BERTH_RUN", run_id)
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()
        .expect("identified fixture command should run")
}
