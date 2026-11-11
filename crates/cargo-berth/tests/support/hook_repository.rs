//! Repository preparation shared by hook tests and recorded templates.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Output;

use cargo_berth_test_support::AmbientHarnessSession;
use cargo_berth_test_support::GitDriver;
use cargo_berth_test_support::OptionalLocks;
use cargo_berth_test_support::RepositoryTemplate;
use cargo_berth_test_support::berth_command;
use cargo_berth_test_support::git_command;
use cargo_berth_test_support::spawn_hook_verb;
use serde_json::Value;

use super::git_configuration;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
const BERTH_EXECUTABLE: &str = env!("CARGO_BIN_EXE_cargo-berth");
const GIT: GitDriver = GitDriver {
    executable:          BERTH_EXECUTABLE,
    optional_locks:      OptionalLocks::Taken,
    cleared_environment: &[],
};
pub(super) const CONFIGURATION_PATH: &str = ".claude/config/berth.toml";

pub(super) const GIT_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    target_directory: env!("CARGO_TARGET_TMPDIR"),
    executable:       BERTH_EXECUTABLE,
    name:             "hook-git-only",
    build:            |root| {
        let built = build_git_repository(root);
        assert!(built.is_ok(), "git repository template failed: {built:?}");
    },
};

pub(super) const INITIALIZED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    name: "hook-initialized",
    build: |root| {
        let built = build_initialized_repository(root);
        assert!(
            built.is_ok(),
            "initialized hook repository template failed: {built:?}"
        );
    },
    ..GIT_REPOSITORY
};

pub(super) const CONFIGURED_REPOSITORY: RepositoryTemplate = RepositoryTemplate {
    name: "hook-configured",
    build: |root| {
        INITIALIZED_REPOSITORY.restore_into(root);
        let built = (|| {
            run_git(root, &["add", "--force", CONFIGURATION_PATH])?;
            run_git(
                root,
                &[
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "core.hooksPath=/dev/null",
                    "commit",
                    "--quiet",
                    "-m",
                    "configure cargo-berth",
                ],
            )
        })();
        assert!(
            built.is_ok(),
            "configured hook repository template failed: {built:?}"
        );
    },
    ..GIT_REPOSITORY
};

pub(super) fn dirty_source(root: &Path, path: &str) -> TestResult {
    let file = root.join(path);
    let parent = file
        .parent()
        .ok_or_else(|| failure("dirty source needs a parent"))?;
    fs::create_dir_all(parent)?;
    fs::write(file, "// uncommitted holder work\n")?;
    Ok(())
}

pub(super) fn build_initialized_repository(root: &Path) -> TestResult {
    build_git_repository(root)?;
    let initialized = run_berth(root, &["init", "--json"])?;
    require_success(&initialized, "cargo-berth init")
}

pub(super) fn build_git_repository(root: &Path) -> TestResult {
    run_git(root, &["init", "--quiet", "--initial-branch", "main"])?;
    git_configuration::append_git_configuration(
        root,
        "\n[user]\n name = Hook Acceptance Test\n email = hooks@example.invalid\n",
    )?;
    fs::write(root.join("README.md"), "hook fixture\n")?;
    run_git(root, &["add", "README.md"])?;
    run_git(
        root,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ],
    )?;
    Ok(())
}

pub(super) fn run_berth(repository_root: &Path, arguments: &[&str]) -> TestResult<Output> {
    Ok(berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env_remove("CARGO_BERTH_SESSION_ID")
        .output()?)
}

pub(super) fn run_berth_with_session(
    repository_root: &Path,
    arguments: &[&str],
    session_id: &str,
) -> TestResult<Output> {
    Ok(berth_command(env!("CARGO_BIN_EXE_cargo-berth"))
        .args(arguments)
        .current_dir(repository_root)
        .env_remove("CARGO_BERTH_RUN")
        .env("CARGO_BERTH_SESSION_ID", session_id)
        .output()?)
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
            "git {arguments:?} failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
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
    serde_json::from_slice(&output.stdout).map_err(Into::into)
}

pub(super) fn required_string<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} should carry string {pointer}")))
}

pub(super) fn required_array<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value [Value]> {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| failure(format!("{context} should carry array {pointer}")))
}

pub(super) fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}

pub(super) fn claimed_reservation_id(claimed: &Output) -> TestResult<String> {
    let envelope = json_output(claimed)?;
    Ok(required_string(&envelope, "/payload/data/reservation_id", "claim")?.to_owned())
}

pub(super) fn bash_payload(working_directory: &Path, session_id: &str) -> Value {
    serde_json::json!({
        "tool_name": "Bash",
        "cwd": working_directory,
        "tool_input": {"command": "true"},
        "session_id": session_id,
    })
}

pub(super) fn add_named_worktree(repository: &Path, branch: &str, root: &Path) -> TestResult {
    GIT.add_worktree_without_hooks(repository, root, branch, "HEAD");
    copy_configuration(repository, root)
}

pub(super) fn copy_configuration(repository: &Path, root: &Path) -> TestResult {
    let configuration_path = root.join(CONFIGURATION_PATH);
    let configuration_directory = configuration_path
        .parent()
        .ok_or_else(|| failure("worktree configuration should have a parent"))?;
    fs::create_dir_all(configuration_directory)?;
    fs::copy(repository.join(CONFIGURATION_PATH), configuration_path)?;
    Ok(())
}

pub(super) fn run_post_tool_use(working_directory: &Path, payload: &Value) -> TestResult<Output> {
    run_post_tool_use_with_ambient_session(
        working_directory,
        payload,
        &AmbientHarnessSession::Absent,
    )
}

pub(super) fn run_post_tool_use_with_ambient_session(
    working_directory: &Path,
    payload: &Value,
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    spawn_hook_verb(
        Path::new(BERTH_EXECUTABLE),
        working_directory,
        "post-tool-use",
        &serde_json::to_vec(payload)?,
        ambient_session,
    )
}
