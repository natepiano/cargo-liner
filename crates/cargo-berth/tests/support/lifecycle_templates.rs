//! The starting repositories of the lifecycle tests, built by the nextest setup script.

use std::fs;
use std::path::Path;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::berth_command;

use crate::git_configuration;
use crate::recorded_history;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(crate) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How the lifecycle tests drive git: an ordinary checkout, with nothing held back from a hook.
pub(crate) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

pub(crate) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";
pub(crate) const INITIAL_COMMIT_TAG: &str = "initial-state";
pub(crate) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";
pub(crate) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

#[derive(Clone, Copy)]
pub(crate) enum GitReferenceStorage {
    Loose,
    Reftable,
}

/// One commit tagged `INITIAL_COMMIT_TAG`, then `cargo-berth init`.
const INITIALIZED: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "lifecycle-initialized",
    build:            |root| build_initialized_repository(root, GitReferenceStorage::Loose),
};

/// The repository most lifecycle tests start from.
pub(crate) const INITIALIZED_REPOSITORY: LedgerTemplate = INITIALIZED.before_run();

/// The empty-ledger main checkout with the ignored policy deliberately committed.
pub(crate) const COMMITTED_POLICY_REPOSITORY: LedgerTemplate = RepositoryTemplate {
    name: "lifecycle-committed-policy",
    build: |root| {
        INITIALIZED_REPOSITORY.restore_into(root);
        GIT.run(root, ["add", "--force", CONFIGURATION_PATH]);
        GIT.run_without_hooks(
            root,
            ["commit", "--quiet", "-m", "track berth configuration"],
        );
    },
    ..INITIALIZED
}
.before_run();

/// `INITIALIZED_REPOSITORY` with its references kept in a reftable.
pub(crate) const INITIALIZED_REFTABLE_REPOSITORY: LedgerTemplate = RepositoryTemplate {
    name: "lifecycle-initialized-reftable",
    build: |root| build_initialized_repository(root, GitReferenceStorage::Reftable),
    ..INITIALIZED
}
.before_run();

fn build_initialized_repository(root: &Path, git_reference_storage: GitReferenceStorage) {
    match git_reference_storage {
        GitReferenceStorage::Loose => GIT.run(root, ["init", "--quiet", "--initial-branch=main"]),
        GitReferenceStorage::Reftable => GIT.run(
            root,
            [
                "init",
                "--quiet",
                "--initial-branch=main",
                "--ref-format=reftable",
            ],
        ),
    }
    git_configuration::append_git_configuration(root, "\n[user]\n name = Berth Test\n email = berth@example.invalid\n[maintenance]\n auto = false\n[gc]\n auto = 0\n").expect("fixture git config should write");
    fs::write(root.join("README.md"), "scratch repository\n").expect("scratch file should write");
    GIT.run(root, ["add", "README.md"]);
    GIT.run(root, ["commit", "--quiet", "-m", "initial"]);
    GIT.run(root, ["tag", INITIAL_COMMIT_TAG]);
    let initialized = berth_command(BERTH_EXECUTABLE)
        .arg("init")
        .current_dir(root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run");
    assert!(
        initialized.status.success(),
        "cargo-berth init should succeed"
    );
}

/// The released holder before the trunk reference disappears.
pub(crate) const RELEASED_BEFORE_TRUNK_DISAPPEARS: LedgerTemplate = recorded_history::template(
    "lifecycle-released-before-trunk-disappears",
    build_released_before_trunk_disappears,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct ReleasedBeforeMissingTrunk {
    pub(crate) second_root:    std::path::PathBuf,
    pub(crate) reservation_id: String,
}

fn build_released_before_trunk_disappears(destination: &Path) {
    let repository = INITIALIZED_REPOSITORY.instantiate();
    let second_root = repository.path().join(".git/fixture-worktrees/second");
    GIT.add_worktree_without_hooks(repository.path(), &second_root, "second", "HEAD");
    fs::create_dir_all(second_root.join(".claude/config"))
        .expect("foreign configuration directory should exist");
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        second_root.join(CONFIGURATION_PATH),
    )
    .expect("foreign configuration should copy");
    GIT.run_without_hooks(repository.path(), ["switch", "--quiet", "-c", "phase"]);
    fs::create_dir_all(repository.path().join("src")).expect("phase source directory should exist");
    fs::write(repository.path().join("src/lib.rs"), "phase work\n")
        .expect("phase source should write");
    GIT.run(repository.path(), ["add", "src/lib.rs"]);
    GIT.run(repository.path(), ["commit", "--quiet", "-m", "phase work"]);
    let claim = run_berth(
        repository.path(),
        &[
            "claim",
            "tree:src",
            "--run",
            cargo_berth_test_support::FIRST_RUN,
            "--json",
        ],
    );
    let reservation_id = cargo_berth_test_support::claim_id(&claim);
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    GIT.run(repository.path(), ["switch", "--quiet", "main"]);
    GIT.run(
        repository.path(),
        ["merge", "--quiet", "--ff-only", "phase"],
    );
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    assert!(
        run_berth(repository.path(), &["release", &reservation_id])
            .status
            .success()
    );
    recorded_history::save(
        destination,
        repository.path(),
        &ReleasedBeforeMissingTrunk {
            second_root,
            reservation_id,
        },
    );
}

fn run_berth(root: &Path, arguments: &[&str]) -> std::process::Output {
    berth_command(BERTH_EXECUTABLE)
        .current_dir(root)
        .args(arguments)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("lifecycle fixture command should run")
}
