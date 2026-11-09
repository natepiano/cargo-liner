//! Prepare the compaction journal before the live reader and compaction operations.

use std::error::Error;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::assert_success;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::claim_id;
use cargo_berth_test_support::json;
use cargo_berth_test_support::write_file;

pub(super) const EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[
        "CARGO_BERTH_RUN",
        "CARGO_BERTH_SESSION_ID",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
    ],
};
/// Fixture commits skip the managed hooks, which no check here is about.
const HOOKS_DISABLED: &str = "core.hooksPath=/dev/null";
pub(super) const HELD_PATH: &str = "src/held.rs";
const HOLDER_COMMITS: usize = 3;
const HOLDER_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
const STRAYING_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a2c";

/// A committed policy with no journal records, before the holder/incursion scenario begins.
const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       EXECUTABLE,
    name:             "compaction-main",
    build:            |root| {
        let built = build_initialized_repository(root);
        assert!(built.is_ok(), "compaction base should build: {built:?}");
    },
};

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The complete journal precedes the first reader sequence and live compaction.
pub(super) const PREPARED_COMPACTION: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "compaction-reader-state",
        build: |root| {
            let built = build_compaction_history(root);
            assert!(built.is_ok(), "compaction history should build: {built:?}");
        },
        ..INITIALIZED_REPOSITORY
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

fn build_compaction_history(root: &Path) -> TestResult {
    INITIALIZED_REPOSITORY.restore_into(root);
    std::fs::create_dir_all(root.join(".git/fixture-worktrees"))?;
    let holder = root.join(".git/fixture-worktrees/holder");
    GIT.add_worktree_without_hooks(root, &holder, "holder", "main");
    let holder = std::fs::canonicalize(holder)?;

    let holder_id = claim_id(&run(
        &holder,
        &["claim", HELD_PATH, "--run", HOLDER_RUN, "--json"],
    )?);
    let straying_id = claim_id(&run(
        root,
        &["claim", "owned.txt", "--run", STRAYING_RUN, "--json"],
    )?);
    // Each commit moves the holder's merge extent, so each board run observes it again and
    // supersedes the observation before.
    for edit in 1..=HOLDER_COMMITS {
        write_file(&holder, HELD_PATH, &format!("held\nedit {edit}\n"));
        GIT.run(&holder, ["add", HELD_PATH]);
        commit(&holder, &format!("holder edit {edit}"));
        assert_success(&run(root, &["board", "--json"])?);
    }

    write_file(root, HELD_PATH, "entered\n");
    let drift = run(
        root,
        &["drift", "--full", "--reservation", &straying_id, "--json"],
    )?;
    assert_eq!(json(&drift)["status"], "incursion");
    let resolved = run(
        root,
        &["resolve", &straying_id, "--every-incursion", "--json"],
    )?;
    assert_success(&resolved);
    let released = berth_command(EXECUTABLE)
        .args(["release", &holder_id, "--json"])
        .current_dir(&holder)
        .env("CARGO_BERTH_RUN", HOLDER_RUN)
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()?;
    assert_success(&released);
    // An unclaimed file gives the PostToolUse hook a widening to report.
    write_file(root, "unclaimed.txt", "unclaimed\n");
    Ok(())
}

fn build_initialized_repository(root: &Path) -> TestResult {
    GIT.run(root, ["init", "--quiet", "--initial-branch=main"]);
    GIT.run(root, ["config", "user.name", "Berth Test"]);
    GIT.run(root, ["config", "user.email", "berth@example.invalid"]);
    GIT.run(root, ["config", "gc.auto", "0"]);
    write_file(root, "README.md", "initial\n");
    write_file(root, HELD_PATH, "held\n");
    GIT.run(root, ["add", "README.md", HELD_PATH]);
    commit(root, "initial");
    assert_success(&run(root, &["init", "--json"])?);
    GIT.run(root, ["add", "--force", ".claude/config/berth.toml"]);
    GIT.run_without_hooks(
        root,
        ["commit", "--quiet", "-m", "track berth configuration"],
    );
    Ok(())
}

/// Run `cargo-berth` in `checkout` without this process's coordination run or session.
pub(super) fn run(checkout: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(berth_command(EXECUTABLE)
        .args(arguments)
        .current_dir(checkout)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()?)
}

fn commit(checkout: &Path, message: &str) {
    GIT.run(
        checkout,
        ["-c", HOOKS_DISABLED, "commit", "--quiet", "-m", message],
    );
}
