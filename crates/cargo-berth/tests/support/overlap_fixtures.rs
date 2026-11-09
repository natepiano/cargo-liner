// Shared starting histories and fixture operations.

pub(super) use cargo_berth_test_support::GitDriver;
pub(super) use cargo_berth_test_support::IntegrationRepository;
pub(super) use cargo_berth_test_support::OptionalLocks;
pub(super) use cargo_berth_test_support::RepositoryTemplate;
pub(super) use cargo_berth_test_support::assert_success;
pub(super) use cargo_berth_test_support::berth_command;
pub(super) use cargo_berth_test_support::json;

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file drives git: an ordinary checkout, with nothing held back from a hook.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

/// An initialized main checkout with untracked policy and no reservations.
pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "overlap-initialized",
    build:            build_initialized_repository,
};

/// Initialized main with committed policy for integration-target scenarios.
pub(super) const INTEGRATION_MAIN: RepositoryTemplate = RepositoryTemplate {
    name: "integration-main",
    build: |root| IntegrationRepository::build_main_template(root, BERTH_EXECUTABLE),
    ..INITIALIZED_REPOSITORY
};

pub(super) use std::fs;
pub(super) use std::path::Path;
pub(super) use std::path::PathBuf;
pub(super) use std::process::Output;

pub(super) use tempfile::TempDir;
pub(super) use tempfile::tempdir;

use crate::recorded_history;

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";

pub(super) const RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

pub(super) const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

pub(super) fn initialized_repository() -> TempDir { INITIALIZED_REPOSITORY.instantiate() }

pub(super) fn build_initialized_repository(root: &Path) {
    git(root, &["init", "--quiet", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Berth Test"]);
    git(root, &["config", "user.email", "berth@example.invalid"]);
    git(root, &["config", "core.ignoreCase", "false"]);
    fs::write(root.join("README.md"), "scratch repository\n").expect("scratch file should write");
    git(root, &["add", "README.md"]);
    git(root, &["commit", "--quiet", "-m", "initial"]);
    let initialized = run_berth(root, &["init", "--json"]);
    assert!(initialized.status.success());
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

/// Give a foreign reservation a real uncommitted merge surface.
pub(super) fn make_dirty(root: &Path, path: &str) {
    let file = root.join(path);
    fs::create_dir_all(file.parent().expect("dirty fixture has a parent"))
        .expect("dirty fixture parent should exist");
    fs::write(file, "uncommitted holder work\n").expect("dirty fixture should write");
}

/// Separate reconciliation's durable observation from the command mutation under test.
pub(super) fn reconcile_fixture(root: &Path) {
    assert!(run_berth(root, &["board", "--json"]).status.success());
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> Output {
    berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove(RUN_ENVIRONMENT)
        .env_remove(SESSION_ENVIRONMENT)
        .output()
        .expect("cargo-berth should run")
}

pub(super) fn git(repository_root: &Path, arguments: &[&str]) {
    GIT.run(repository_root, arguments);
}

pub(super) fn json_output(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).expect("command should render a JSON envelope")
}

/// Starting history before
/// `check_uses_the_current_runs_target_when_an_older_run_is_still_outstanding`.
pub(super) const BEFORE_CHECK_USES_THE_CURRENT_RUNS_TARGET_WHEN_AN_OLDER_RUN_IS_STILL_OUTSTANDING: cargo_berth_test_support::LedgerTemplate = recorded_history::template("overlap-check-uses-the-current-runs-target-when-an-older-run-is-still-outstanding", build_before_check_uses_the_current_runs_target_when_an_older_run_is_still_outstanding);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct CheckUsesTheCurrentRunsTargetWhenAnOlderRunIsStillOutstandingState {
    pub(super) lane:     PathBuf,
    pub(super) cover_id: String,
}

fn build_before_check_uses_the_current_runs_target_when_an_older_run_is_still_outstanding(
    destination: &Path,
) {
    let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
    let lane = repo.lane("two-run-lane", "integration");
    repo.git(&lane, &["reset", "--hard", "main"]);
    let older = repo.claim(&lane, "file:older.txt", FIRST_RUN, Some("main"));
    assert_success(&older);
    let older_id = json(&older)["payload"]["data"]["reservation_id"]
        .as_str()
        .expect("older reservation ID")
        .to_owned();
    repo.commit_file(&lane, "older.txt", "older\n", "older work");
    repo.commit_file(
        &lane,
        "integration.txt",
        "lane version\n",
        "lane changes target path",
    );
    let checkpoint = repo.run(&lane, &["release", &older_id, "--json"]);
    assert_success(&checkpoint);
    let current = repo.claim(&lane, "file:current.txt", SECOND_RUN, None);
    assert_success(&current);
    let _ = repo.board();
    let cover_id = repo.cover_claims()[0]["reservation_id"]
        .as_str()
        .expect("cover reservation ID")
        .to_owned();

    recorded_history::save(
        destination,
        repo.root(),
        &CheckUsesTheCurrentRunsTargetWhenAnOlderRunIsStillOutstandingState { lane, cover_id },
    );
}

/// Starting history before `a_first_touch_holder_block_names_the_verbs_that_clear_it`.
pub(super) const BEFORE_A_FIRST_TOUCH_HOLDER_BLOCK_NAMES_THE_VERBS_THAT_CLEAR_IT:
    cargo_berth_test_support::LedgerTemplate = recorded_history::template(
    "overlap-a-first-touch-holder-block-names-the-verbs-that-clear-it",
    build_before_a_first_touch_holder_block_names_the_verbs_that_clear_it,
);

#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct AFirstTouchHolderBlockNamesTheVerbsThatClearItState {
    pub(super) second_root:    PathBuf,
    pub(super) reservation_id: String,
}

fn build_before_a_first_touch_holder_block_names_the_verbs_that_clear_it(destination: &Path) {
    let repository = initialized_repository();
    let first_touch = run_berth(repository.path(), &["check", "file:touched.rs", "--json"]);
    assert!(first_touch.status.success());
    let reservation_id =
        json_output(&first_touch)["payload"]["data"]["acquisition"]["reservation_id"]
            .as_str()
            .expect("a clear check should create a first-touch reservation")
            .to_owned();

    let (_second_directory, second_root) = foreign_worktree(&repository, "second");
    make_dirty(repository.path(), "touched.rs");
    reconcile_fixture(repository.path());

    recorded_history::save(
        destination,
        repository.path(),
        &AFirstTouchHolderBlockNamesTheVerbsThatClearItState {
            second_root,
            reservation_id,
        },
    );
}

pub(super) const HISTORY_SIBLING_LANDED_COVER: cargo_berth_test_support::LedgerTemplate =
    recorded_history::template("overlap-sibling-landed-cover", |destination| {
        let repo = IntegrationRepository::from_template(INTEGRATION_MAIN);
        let a = repo.lane("contained-a", "integration");
        let b = repo.lane("contained-b", "integration");
        let a_claim = repo.claim(&a, "file:shared.txt", FIRST_RUN, None);
        assert_success(&a_claim);
        let b_claim = repo.claim(&b, "file:b.txt", SECOND_RUN, None);
        assert_success(&b_claim);
        let _ = repo.board();
        repo.commit_file(&a, "shared.txt", "landed\n", "lane A work");
        repo.merge_by_commit("contained-a");
        let a_id = json(&a_claim)["payload"]["data"]["reservation_id"]
            .as_str()
            .expect("lane A ID")
            .to_owned();

        recorded_history::save(
            destination,
            repo.root(),
            &SiblingLandedCoverHistory { b, a_id },
        );
    });
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct SiblingLandedCoverHistory {
    pub(super) b:    PathBuf,
    pub(super) a_id: String,
}
