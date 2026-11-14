//! The repositories the presentation tests start with, and the commands that build them.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::FIRST_RUN;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::LedgerFixture;
use cargo_berth_test_support::LedgerTemplate;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::TemplateBuildTiming;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::build_scratch_repository;
use cargo_berth_test_support::git_command;
use cargo_berth_test_support::save_recorded_history;
use serde_json::Value;
use tempfile::TempDir;
use tempfile::tempdir;

/// Where [`INTEGRATION_PHASE`] records its [`IntegrationPhase`].
pub(super) const PHASE_STATE: &str = ".git/integration-phase-state.json";

/// One scratch commit and `cargo-berth init`, with the berth policy left uncommitted.
pub(super) const INITIALIZED: LedgerTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "presentation-initialized",
    build:            build_initialized,
}
.before_run();

/// A phase worktree that claimed `integrated.rs` and committed it, before its checkpoint.
pub(super) const INTEGRATION_PHASE: LedgerTemplate = LedgerTemplate {
    repository:   RepositoryTemplate {
        name: "presentation-integration-phase",
        build: build_integration_phase,
        ..INITIALIZED.repository
    },
    ledger:       LedgerFixture::Recorded,
    build_timing: TemplateBuildTiming::BeforeRun,
};

/// The phase worktree and its reservation, after relocation into the test's copy.
#[derive(serde::Serialize, serde::Deserialize)]
pub(super) struct IntegrationPhase {
    pub(super) phase_root:     PathBuf,
    pub(super) reservation_id: String,
}

fn build_initialized(root: &Path) {
    build_scratch_repository(root, GIT);
    let initialized = run_berth(root, &["init", "--json"])
        .and_then(|output| require_success(&output, "cargo-berth init"));
    assert!(
        initialized.is_ok(),
        "cargo-berth init should succeed: {initialized:?}"
    );
}

fn build_integration_phase(destination: &Path) {
    let recorded = record_integration_phase(destination);
    assert!(
        recorded.is_ok(),
        "the integration phase should record: {recorded:?}"
    );
}

fn record_integration_phase(destination: &Path) -> TestResult {
    let repository = INITIALIZED.instantiate();
    let (_worktree_parent, phase_root) = add_worktree(&repository, "integration-phase")?;
    let claimed = json_output(&claim(&phase_root, "file:integrated.rs", FIRST_RUN)?)?;
    let reservation_id = required_string(&claimed, "/payload/data/reservation_id")?.to_owned();
    commit_file(&phase_root, "integrated.rs", "integrated work\n")?;
    save_recorded_history(
        BERTH_EXECUTABLE,
        destination,
        repository.path(),
        PHASE_STATE,
        serde_json::to_value(IntegrationPhase {
            phase_root,
            reservation_id,
        })?,
    );
    Ok(())
}

pub(super) type TestResult<T = ()> = Result<T, Box<dyn Error>>;

pub(super) struct RenderedBlock<'envelope> {
    pub(super) summary: &'envelope str,
    pub(super) detail:  &'envelope str,
}

/// The `cargo-berth` a managed hook must run, in place of any installed copy.
pub(super) const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");

/// How this file adds a fixture worktree: optional locks taken, nothing held back from git.
pub(super) const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};

pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const SESSION_ENVIRONMENT: &str = "CARGO_BERTH_SESSION_ID";

pub(super) fn claim(repository_root: &Path, scope: &str, run: &str) -> TestResult<Output> {
    let output = run_berth(repository_root, &["claim", scope, "--run", run, "--json"])?;
    require_success(&output, "claim")?;
    let envelope = json_output(&output)?;
    let reservation_id = required_string(&envelope, "/payload/data/reservation_id")?;
    let rendered_block = only_rendered_block(&envelope)?;
    assert!(rendered_block.summary.contains(reservation_id));
    assert!(rendered_block.detail.contains(reservation_id));
    Ok(output)
}

pub(super) fn commit_file(repository_root: &Path, path: &str, contents: &str) -> TestResult {
    let file_path = repository_root.join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(file_path, contents)?;
    run_git(repository_root, &["add", path])?;
    run_git(repository_root, &["commit", "--quiet", "-m", path])
}

pub(super) fn add_worktree(repository: &TempDir, branch: &str) -> TestResult<(TempDir, PathBuf)> {
    let directory = tempdir()?;
    let worktree_root = directory.path().join(branch);
    GIT.add_worktree_without_hooks(repository.path(), &worktree_root, branch, "HEAD");
    let configuration_path = worktree_root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("berth configuration should have a parent directory"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(
        repository.path().join(CONFIGURATION_PATH),
        configuration_path,
    )?;
    Ok((directory, worktree_root))
}

pub(super) fn run_git(repository_root: &Path, arguments: &[&str]) -> TestResult {
    let output = git_command(BERTH_EXECUTABLE)
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove(SESSION_ENVIRONMENT)
        .output()?)
}

pub(super) fn require_success(output: &Output, operation: &str) -> TestResult {
    if output.status.success() {
        Ok(())
    } else {
        Err(failure(format!(
            "{operation} failed with {:?}: stdout={} stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

pub(super) fn json_output(output: &Output) -> TestResult<Value> {
    serde_json::from_slice(&output.stdout).map_err(|error| {
        failure(format!(
            "cargo-berth stdout should be JSON: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })
}

pub(super) fn required_string<'value>(
    value: &'value Value,
    pointer: &str,
) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{pointer} should be a string in {value}")))
}

pub(super) fn rendered_blocks(envelope: &Value) -> TestResult<&[Value]> {
    let presentation_kind = required_string(envelope, "/presentation/kind")?;
    assert_ne!(
        presentation_kind, "not_provided",
        "contracted reservation fact serialized an unprovided presentation: {envelope}"
    );
    if presentation_kind != "rendered_blocks" {
        return Err(failure(format!(
            "engine left presentation unprovided: {}",
            envelope["presentation"]
        )));
    }
    required_array(envelope, "/presentation/blocks")
}

pub(super) fn only_rendered_block(envelope: &Value) -> TestResult<RenderedBlock<'_>> {
    let blocks = rendered_blocks(envelope)?;
    let [block] = blocks else {
        return Err(failure(format!(
            "expected one rendered presentation block, found {}",
            blocks.len()
        )));
    };
    Ok(RenderedBlock {
        summary: required_string(block, "/summary")?,
        detail:  required_string(block, "/detail")?,
    })
}

pub(super) fn required_array<'value>(
    value: &'value Value,
    pointer: &str,
) -> TestResult<&'value [Value]> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| failure(format!("{pointer} should be an array in {value}")))
}

pub(super) fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}
