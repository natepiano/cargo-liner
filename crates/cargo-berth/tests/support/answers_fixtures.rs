//! Starting histories and command helpers for answer tests.

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

pub(super) use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::berth_command;
use tempfile::TempDir;
use tempfile::tempdir;

use crate::git_configuration;
use crate::recorded_history;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

/// An initialized main checkout with untracked policy and no reservations.
const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "initialized",
    build:            build_initialized_repository,
};

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

pub(super) fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

pub(super) fn build_initialized_repository(root: &Path) {
    git(root, ["init", "--quiet", "--initial-branch=main"]);
    git_configuration::append_git_configuration(root, "\n[user]\n name = Berth Test\n email = berth@example.invalid\n[core]\n ignoreCase = false\n[maintenance]\n auto = false\n[gc]\n auto = 0\n").expect("fixture git config should write");
    fs::write(root.join("README.md"), "scratch repository\n").expect("scratch file should write");
    git(root, ["add", "README.md"]);
    git(root, ["commit", "--quiet", "-m", "initial"]);
    assert!(run_berth(root, ["init", "--json"]).status.success());
}

/// Add a real worktree beside the repository, the second party berth has always refused.
///
/// A distinct `--run` inside one worktree now names a second party too, but only a real
/// worktree can hold a reservation of its own alongside another run's. The returned
/// directory owns the worktree and must outlive its use.
pub(super) fn foreign_worktree(repository: &TempDir, name: &str) -> (TempDir, PathBuf) {
    let directory = tempdir().expect("foreign worktree parent should exist");
    let root = directory.path().join(name);
    GIT.add_worktree_without_hooks(repository.path(), &root, name, "HEAD");
    let configuration = root.join(CONFIGURATION_PATH);
    if let Some(parent) = configuration.parent() {
        fs::create_dir_all(parent).expect("foreign worktree configuration should have a directory");
    }
    fs::copy(repository.path().join(CONFIGURATION_PATH), configuration)
        .expect("foreign worktree should share the repository configuration");
    (directory, root)
}

pub(super) fn commit_file(repository_root: &Path, path: &str, contents: &str, message: &str) {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("file parent should exist");
    }
    fs::write(file_path, contents).expect("committed file should write");
    git(repository_root, ["add", path]);
    git(
        repository_root,
        [
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "--quiet",
            "-m",
            message,
        ],
    );
}

pub(super) fn claim(
    repository_root: &Path,
    scope: &str,
    run: &str,
    plan: &str,
    phase: &str,
    purpose: &str,
) -> Output {
    let output = run_berth(
        repository_root,
        [
            "claim", "--run", run, "--plan", plan, "--phase", phase, "--why", purpose, scope,
            "--json",
        ],
    );
    assert!(
        output.status.success(),
        "claim failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

pub(super) fn run_berth<Arguments, Argument>(repository_root: &Path, arguments: Arguments) -> Output
where
    Arguments: IntoIterator<Item = Argument>,
    Argument: AsRef<OsStr>,
{
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git<Arguments, Argument>(repository_root: &Path, arguments: Arguments)
where
    Arguments: IntoIterator<Item = Argument>,
    Argument: AsRef<OsStr>,
{
    GIT.run(repository_root, arguments);
}

pub(super) fn reservation_id(output: &Output) -> String {
    json_output(output)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("claim should return a reservation id")
        .to_owned()
}

pub(super) fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}

/// The holder and requester exist before the tested answer claim.
pub(super) const HOLDER_LIFECYCLE_START: LedgerTemplate = recorded_history::template(
    "answers-holder-lifecycle-start",
    build_holder_lifecycle_start,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct HolderLifecycleStart {
    pub(super) second_root: PathBuf,
    pub(super) holder_id:   String,
}

fn build_holder_lifecycle_start(destination: &Path) {
    let repository = initialized_repository();
    let (second_directory, second_root) = foreign_worktree(&repository, "second");
    git(repository.path(), ["switch", "--quiet", "-c", "phase"]);
    commit_file(
        repository.path(),
        "src/lib.rs",
        "phase work\n",
        "phase work",
    );
    let holder = claim(
        repository.path(),
        "tree:src",
        FIRST_RUN,
        "docs/holder.md",
        "phase-a",
        "protect source",
    );
    let holder_id = reservation_id(&holder);
    recorded_history::save(
        destination,
        repository.path(),
        &HolderLifecycleStart {
            second_root,
            holder_id,
        },
    );
    drop(second_directory);
}
