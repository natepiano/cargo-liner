//! Git command construction.

use std::ffi::OsStr;
use std::io;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::thread;

use super::constants::GIT_BINARY;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARG;

/// Whether a git invocation may execute repository hooks.
#[derive(Clone, Copy)]
pub(super) enum GitHookExecutionPolicy {
    /// Execute hooks according to the repository's effective configuration.
    Enabled,
    /// Suppress hooks because the invocation only maintains berth's private retention refs.
    SuppressedForRetentionRef,
}

/// Whether a git invocation produced a complete process output.
pub(crate) enum GitCommandExecution {
    /// The process ran to completion, including a possible non-zero exit status.
    Completed(Output),
    /// The invocation returned an I/O error instead of a process output.
    CouldNotRun(io::Error),
}

impl From<io::Result<Output>> for GitCommandExecution {
    fn from(output: io::Result<Output>) -> Self {
        output.map_or_else(Self::CouldNotRun, Self::Completed)
    }
}

/// Build a git subprocess rooted at `repository_root` without optional locks.
pub(super) fn git_command(repository_root: &Path) -> Command {
    git_command_with_hook_execution_policy(repository_root, GitHookExecutionPolicy::Enabled)
}

/// Build a git subprocess with an explicit repository-hook execution policy.
fn git_command_with_hook_execution_policy(
    repository_root: &Path,
    hook_execution_policy: GitHookExecutionPolicy,
) -> Command {
    let mut command = Command::new(GIT_BINARY);
    command.arg(GIT_NO_OPTIONAL_LOCKS_ARG);
    match hook_execution_policy {
        GitHookExecutionPolicy::Enabled => {},
        GitHookExecutionPolicy::SuppressedForRetentionRef => {
            // A count already at the integer ceiling leaves no slot to append to, so it is
            // treated as absent and the suppression entry lands at slot zero.
            let inherited_config_count = std::env::var("GIT_CONFIG_COUNT")
                .ok()
                .and_then(|count| count.parse::<usize>().ok())
                .filter(|&count| count < usize::MAX)
                .unwrap_or_default();
            let suppression_key = format!("GIT_CONFIG_KEY_{inherited_config_count}");
            let suppression_value = format!("GIT_CONFIG_VALUE_{inherited_config_count}");
            command
                .env("GIT_CONFIG_COUNT", (inherited_config_count + 1).to_string())
                .env(suppression_key, "core.hooksPath")
                .env(suppression_value, "/dev/null");
        },
    }
    command.current_dir(repository_root);
    command
}

/// Run one git operation and return its complete output.
pub(super) fn git_output<const ARGUMENT_COUNT: usize>(
    repository_root: &Path,
    arguments: [&str; ARGUMENT_COUNT],
) -> io::Result<Output> {
    git_command(repository_root).args(arguments).output()
}

/// Run one read-only git operation through the typed process-execution boundary.
pub(crate) fn git_execution(repository_root: &Path, arguments: &[&str]) -> GitCommandExecution {
    git_command(repository_root).args(arguments).output().into()
}

/// Run one git operation whose revision arguments are assembled at runtime.
pub(super) fn git_output_dynamic(
    repository_root: &Path,
    arguments: &[String],
) -> io::Result<Output> {
    git_command(repository_root).args(arguments).output()
}

/// Run one dynamically assembled git operation with explicit environment values.
pub(super) fn git_output_dynamic_with_environment(
    repository_root: &Path,
    arguments: &[String],
    environment: &[(&str, &OsStr)],
) -> io::Result<Output> {
    git_command(repository_root)
        .args(arguments)
        .envs(environment.iter().copied())
        .output()
}

/// Run one dynamically assembled git operation with complete standard input.
pub(super) fn git_output_dynamic_with_input(
    repository_root: &Path,
    arguments: &[String],
    input: &[u8],
) -> io::Result<Output> {
    let mut command = git_command(repository_root);
    command.args(arguments);
    command_output_with_input(command, input)
}

/// Run one dynamically assembled git operation with a repository-hook execution policy.
pub(super) fn git_output_dynamic_with_hook_execution_policy(
    repository_root: &Path,
    arguments: &[String],
    hook_execution_policy: GitHookExecutionPolicy,
) -> io::Result<Output> {
    git_command_with_hook_execution_policy(repository_root, hook_execution_policy)
        .args(arguments)
        .output()
}

/// Run one dynamically assembled git operation with a repository-hook execution policy.
pub(super) fn git_output_dynamic_with_hook_execution_policy_and_input(
    repository_root: &Path,
    arguments: &[String],
    hook_execution_policy: GitHookExecutionPolicy,
    input: &[u8],
) -> io::Result<Output> {
    let mut command =
        git_command_with_hook_execution_policy(repository_root, hook_execution_policy);
    command.args(arguments);
    command_output_with_input(command, input)
}

/// Run one dynamically assembled git operation with environment values and standard input.
pub(super) fn git_output_dynamic_with_environment_and_input(
    repository_root: &Path,
    arguments: &[String],
    environment: &[(&str, &OsStr)],
    input: &[u8],
) -> io::Result<Output> {
    let mut command = git_command(repository_root);
    command.args(arguments).envs(environment.iter().copied());
    command_output_with_input(command, input)
}

/// Run a configured git command while writing its complete standard input concurrently.
fn command_output_with_input(mut command: Command, input: &[u8]) -> io::Result<Output> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("git child standard input was not piped"))?;
    thread::scope(|scope| {
        let input_writer = scope.spawn(move || {
            let result = stdin.write_all(input);
            drop(stdin);
            result
        });
        let output = child.wait_with_output();
        input_writer
            .join()
            .map_err(|_| io::Error::other("git child standard input writer panicked"))??;
        output
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use super::GitHookExecutionPolicy;
    use super::git_command_with_hook_execution_policy;

    const CHILD_PROCESS_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_COMMAND_ENVIRONMENT_CHILD";

    #[test]
    fn inherited_git_config_overlay_survives_hook_suppression() -> Result<(), Box<dyn Error>> {
        const TEST_FILTER: &str = "inherited_git_config_overlay_survives_hook_suppression";
        if std::env::var_os(CHILD_PROCESS_ENVIRONMENT).as_deref() != Some(OsStr::new(TEST_FILTER)) {
            return rerun_current_test_with_environment(
                TEST_FILTER,
                &[
                    ("GIT_CONFIG_COUNT", "1"),
                    ("GIT_CONFIG_KEY_0", "user.name"),
                    ("GIT_CONFIG_VALUE_0", "Inherited Overlay"),
                ],
            );
        }

        let mut command = git_command_with_hook_execution_policy(
            Path::new("."),
            GitHookExecutionPolicy::SuppressedForRetentionRef,
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_COUNT"),
            Some(OsStr::new("2"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_KEY_1"),
            Some(OsStr::new("core.hooksPath"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_VALUE_1"),
            Some(OsStr::new("/dev/null"))
        );

        let output = command.args(["config", "--get", "user.name"]).output()?;
        assert!(
            output.status.success(),
            "git rejected the inherited config overlay: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "Inherited Overlay"
        );
        Ok(())
    }

    #[test]
    fn maximum_git_config_count_falls_back_to_suppression_slot_zero() -> Result<(), Box<dyn Error>>
    {
        const TEST_FILTER: &str = "maximum_git_config_count_falls_back_to_suppression_slot_zero";
        if std::env::var_os(CHILD_PROCESS_ENVIRONMENT).as_deref() != Some(OsStr::new(TEST_FILTER)) {
            let maximum_count = usize::MAX.to_string();
            return rerun_current_test_with_environment(
                TEST_FILTER,
                &[("GIT_CONFIG_COUNT", maximum_count.as_str())],
            );
        }

        let command = git_command_with_hook_execution_policy(
            Path::new("."),
            GitHookExecutionPolicy::SuppressedForRetentionRef,
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_COUNT"),
            Some(OsStr::new("1"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_KEY_0"),
            Some(OsStr::new("core.hooksPath"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_VALUE_0"),
            Some(OsStr::new("/dev/null"))
        );
        Ok(())
    }

    fn explicit_environment_value<'command>(
        command: &'command Command,
        key: &str,
    ) -> Option<&'command OsStr> {
        command
            .get_envs()
            .find(|(candidate, _)| *candidate == OsStr::new(key))
            .and_then(|(_, value)| value)
    }

    fn rerun_current_test_with_environment(
        test_filter: &str,
        environment: &[(&str, &str)],
    ) -> Result<(), Box<dyn Error>> {
        let output = Command::new(std::env::current_exe()?)
            .arg(test_filter)
            .arg("--nocapture")
            .env(CHILD_PROCESS_ENVIRONMENT, test_filter)
            .envs(environment.iter().copied())
            .output()?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success(),
            "child test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            stdout.contains("running 1 test"),
            "child test filter selected no test:\n{stdout}"
        );
        Ok(())
    }
}
