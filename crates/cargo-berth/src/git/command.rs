//! Git command construction.

use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;
use std::sync::Mutex;
use std::thread;

use super::constants::BERTH_EXECUTABLE_ENVIRONMENT;
use super::constants::GIT_BINARY;
use super::constants::GIT_CONFIG_COUNT_ENVIRONMENT;
use super::constants::GIT_CONFIG_KEY_PREFIX;
use super::constants::GIT_CONFIG_VALUE_PREFIX;
use super::constants::GIT_DIAGNOSTIC_LOCALE;
use super::constants::GIT_LOCALE_ENVIRONMENT;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARG;
use super::constants::GIT_REPOSITORY_ENVIRONMENT;
use super::constants::RETENTION_MAINTENANCE_CONFIG;
use super::constants::RETENTION_REF_CONFIG;

// Pipe creation and child spawn form one critical section. A persistent child
// must never inherit another concurrent command's not-yet-sealed pipe ends.
static PROCESS_SPAWN: Mutex<()> = Mutex::new(());

/// Seal every piped stream before another child can inherit file descriptors.
pub(super) fn spawn_with_piped_output(command: &mut Command) -> io::Result<Child> {
    let spawn = PROCESS_SPAWN
        .lock()
        .map_err(|_| io::Error::other("Git spawn lock was poisoned"))?;
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    drop(spawn);
    child
}

/// Capture output without holding the spawn lock while the child runs.
pub(super) fn command_output(command: &mut Command) -> io::Result<Output> {
    spawn_with_piped_output(command.stdin(Stdio::null()))?.wait_with_output()
}

/// Whether a git invocation may execute repository hooks.
#[derive(Clone, Copy)]
pub(super) enum GitHookExecutionPolicy {
    /// Execute hooks according to the repository's effective configuration.
    Enabled,
    /// Suppress hooks because the invocation only maintains berth's private retention refs.
    SuppressedForRetentionRef,
    /// Deferrable maintenance suppresses hooks and yields immediately to ref locks.
    DeferrableRetentionMaintenance,
}

/// Whether a git invocation left a complete process output available.
pub(crate) enum GitCommandOutputAvailability {
    /// The process ran to completion, including a possible non-zero exit status.
    Available(Output),
    /// No process output exists; the invocation returned this I/O error instead.
    Unavailable(Error),
}

impl From<io::Result<Output>> for GitCommandOutputAvailability {
    fn from(output: io::Result<Output>) -> Self {
        output.map_or_else(Self::Unavailable, Self::Available)
    }
}

/// Build a git subprocess rooted at `repository_root` without optional locks.
pub(super) fn git_command(repository_root: &Path) -> io::Result<Command> {
    git_command_with_hook_execution_policy(repository_root, GitHookExecutionPolicy::Enabled)
}

/// Build a git subprocess with an explicit repository-hook execution policy.
pub(super) fn git_command_with_hook_execution_policy(
    repository_root: &Path,
    hook_execution_policy: GitHookExecutionPolicy,
) -> io::Result<Command> {
    let mut command = Command::new(GIT_BINARY);
    command.arg(GIT_NO_OPTIONAL_LOCKS_ARG);
    // Refusal classification and the query grammars use Git's stable English diagnostics.
    command.env(GIT_LOCALE_ENVIRONMENT, GIT_DIAGNOSTIC_LOCALE);
    // A hook inherits its invoking checkout's selectors, while reconciliation also reads
    // other checkouts. Every subprocess must discover its repository from current_dir.
    for variable in GIT_REPOSITORY_ENVIRONMENT {
        command.env_remove(variable);
    }
    match hook_execution_policy {
        GitHookExecutionPolicy::Enabled => {
            // A nested hook must understand the journal its invoking berth just wrote.
            // Preserve deliberate overrides, including paths that are not UTF-8.
            let executable = match std::env::var_os(BERTH_EXECUTABLE_ENVIRONMENT) {
                Some(executable) if !executable.is_empty() => executable,
                Some(_) | None => std::env::current_exe()?.into_os_string(),
            };
            command.env(BERTH_EXECUTABLE_ENVIRONMENT, executable);
        },
        GitHookExecutionPolicy::SuppressedForRetentionRef => {
            append_config_overlay(&mut command, RETENTION_REF_CONFIG);
        },
        GitHookExecutionPolicy::DeferrableRetentionMaintenance => {
            append_config_overlay(&mut command, RETENTION_MAINTENANCE_CONFIG);
        },
    }

    command.current_dir(repository_root);
    Ok(command)
}

/// Append only this invocation's settings, preserving a valid inherited overlay.
fn append_config_overlay(command: &mut Command, entries: &[(&str, &str)]) {
    let inherited_config_count = std::env::var(GIT_CONFIG_COUNT_ENVIRONMENT)
        .ok()
        .and_then(|count| count.parse::<usize>().ok())
        .filter(|&count| count.checked_add(entries.len()).is_some())
        .unwrap_or_default();
    for (offset, (key, value)) in entries.iter().enumerate() {
        let index = inherited_config_count + offset;
        command
            .env(format!("{GIT_CONFIG_KEY_PREFIX}{index}"), key)
            .env(format!("{GIT_CONFIG_VALUE_PREFIX}{index}"), value);
    }
    command.env(
        GIT_CONFIG_COUNT_ENVIRONMENT,
        (inherited_config_count + entries.len()).to_string(),
    );
}

/// Run one git operation and return its complete output.
pub(super) fn git_output<const ARGUMENT_COUNT: usize>(
    repository_root: &Path,
    arguments: [&str; ARGUMENT_COUNT],
) -> io::Result<Output> {
    command_output(git_command(repository_root)?.args(arguments))
}

/// Run one read-only git operation through the typed process-execution boundary.
pub(crate) fn git_execution(
    repository_root: &Path,
    arguments: &[&str],
) -> GitCommandOutputAvailability {
    git_command(repository_root)
        .and_then(|mut command| command_output(command.args(arguments)))
        .into()
}

/// Run one git operation whose revision arguments are assembled at runtime.
pub(super) fn git_output_dynamic(
    repository_root: &Path,
    arguments: &[String],
) -> io::Result<Output> {
    command_output(git_command(repository_root)?.args(arguments))
}

/// Run one dynamically assembled git operation with complete standard input.
pub(super) fn git_output_dynamic_with_input(
    repository_root: &Path,
    arguments: &[String],
    input: &[u8],
) -> io::Result<Output> {
    let mut command = git_command(repository_root)?;
    command.args(arguments);
    command_output_with_input(command, input)
}

/// Run one dynamically assembled git operation with a repository-hook execution policy.
pub(super) fn git_output_dynamic_with_hook_execution_policy(
    repository_root: &Path,
    arguments: &[String],
    hook_execution_policy: GitHookExecutionPolicy,
) -> io::Result<Output> {
    command_output(
        git_command_with_hook_execution_policy(repository_root, hook_execution_policy)?
            .args(arguments),
    )
}

/// Run one dynamically assembled git operation with a repository-hook execution policy.
pub(super) fn git_output_dynamic_with_hook_execution_policy_and_input(
    repository_root: &Path,
    arguments: &[String],
    hook_execution_policy: GitHookExecutionPolicy,
    input: &[u8],
) -> io::Result<Output> {
    let mut command =
        git_command_with_hook_execution_policy(repository_root, hook_execution_policy)?;
    command.args(arguments);
    command_output_with_input(command, input)
}

/// Run a configured git command while writing its complete standard input concurrently.
fn command_output_with_input(mut command: Command, input: &[u8]) -> io::Result<Output> {
    child_output_with_input(
        spawn_with_piped_output(command.stdin(Stdio::piped()))?,
        input,
    )
}

/// Finish an already started command while draining its output and supplying its input.
pub(super) fn child_output_with_input(mut child: Child, input: &[u8]) -> io::Result<Output> {
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
        let written = input_writer
            .join()
            .map_err(|_| io::Error::other("git child standard input writer panicked"))?;
        let output = output?;
        match written {
            // A git that fails before reading all of its input closes the pipe under the writer;
            // its exit status and standard error, not the broken pipe, say why it failed.
            Err(error) if error.kind() == ErrorKind::BrokenPipe && !output.status.success() => {
                Ok(output)
            },
            written => written.map(|()| output),
        }
    })
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::ffi::OsStr;
    use std::io::ErrorKind;
    use std::path::Path;
    use std::process::Command;

    use super::GitHookExecutionPolicy;
    use super::command_output_with_input;
    use super::git_command_with_hook_execution_policy;
    use crate::git::constants::BERTH_EXECUTABLE_ENVIRONMENT;

    const CHILD_PROCESS_ENVIRONMENT: &str = "CARGO_BERTH_TEST_GIT_COMMAND_ENVIRONMENT_CHILD";
    /// Input past every default pipe capacity, so a child that exits without reading it always
    /// leaves the writer facing a closed pipe.
    const UNREAD_INPUT_BYTES: usize = 256 * 1024;

    #[test]
    fn enabled_hooks_use_current_berth_when_override_is_missing_or_empty()
    -> Result<(), Box<dyn Error>> {
        const TEST_FILTER: &str =
            "enabled_hooks_use_current_berth_when_override_is_missing_or_empty";
        if std::env::var_os(CHILD_PROCESS_ENVIRONMENT).as_deref() != Some(OsStr::new(TEST_FILTER)) {
            rerun_current_test_with_environment(TEST_FILTER, &[])?;
            return rerun_current_test_with_environment(
                TEST_FILTER,
                &[(BERTH_EXECUTABLE_ENVIRONMENT, "")],
            );
        }

        let command = git_command_with_hook_execution_policy(
            Path::new("."),
            GitHookExecutionPolicy::Enabled,
        )?;
        let executable = std::env::current_exe()?;
        assert_eq!(
            explicit_environment_value(&command, BERTH_EXECUTABLE_ENVIRONMENT),
            Some(executable.as_os_str()),
        );
        Ok(())
    }

    #[test]
    fn enabled_hooks_preserve_a_nonempty_explicit_berth_override() -> Result<(), Box<dyn Error>> {
        const TEST_FILTER: &str = "enabled_hooks_preserve_a_nonempty_explicit_berth_override";
        const EXPLICIT_EXECUTABLE: &str = "/explicit override/cargo-berth";
        if std::env::var_os(CHILD_PROCESS_ENVIRONMENT).as_deref() != Some(OsStr::new(TEST_FILTER)) {
            return rerun_current_test_with_environment(
                TEST_FILTER,
                &[(BERTH_EXECUTABLE_ENVIRONMENT, EXPLICIT_EXECUTABLE)],
            );
        }

        let command = git_command_with_hook_execution_policy(
            Path::new("."),
            GitHookExecutionPolicy::Enabled,
        )?;
        assert_eq!(
            explicit_environment_value(&command, BERTH_EXECUTABLE_ENVIRONMENT),
            Some(OsStr::new(EXPLICIT_EXECUTABLE)),
        );
        Ok(())
    }

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

        let repository = tempfile::tempdir()?;
        let initialized = git_command_with_hook_execution_policy(
            repository.path(),
            GitHookExecutionPolicy::DeferrableRetentionMaintenance,
        )?
        .args(["init", "--quiet"])
        .output()?;
        assert!(
            initialized.status.success(),
            "test repository did not initialize: {}",
            String::from_utf8_lossy(&initialized.stderr)
        );

        let mut command = git_command_with_hook_execution_policy(
            repository.path(),
            GitHookExecutionPolicy::DeferrableRetentionMaintenance,
        )?;
        assert_eq!(
            explicit_environment_value(&command, BERTH_EXECUTABLE_ENVIRONMENT),
            None,
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_COUNT"),
            Some(OsStr::new("4"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_KEY_1"),
            Some(OsStr::new("core.hooksPath"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_VALUE_1"),
            Some(OsStr::new("/dev/null"))
        );

        for (index, key) in [
            (2, "core.packedRefsTimeout"),
            (3, "core.filesRefLockTimeout"),
        ] {
            assert_eq!(
                explicit_environment_value(&command, &format!("GIT_CONFIG_KEY_{index}")),
                Some(OsStr::new(key))
            );
            assert_eq!(
                explicit_environment_value(&command, &format!("GIT_CONFIG_VALUE_{index}")),
                Some(OsStr::new("0"))
            );
        }

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
            GitHookExecutionPolicy::DeferrableRetentionMaintenance,
        )?;
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_COUNT"),
            Some(OsStr::new("3"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_KEY_0"),
            Some(OsStr::new("core.hooksPath"))
        );
        assert_eq!(
            explicit_environment_value(&command, "GIT_CONFIG_VALUE_0"),
            Some(OsStr::new("/dev/null"))
        );
        for (index, key) in [
            (1, "core.packedRefsTimeout"),
            (2, "core.filesRefLockTimeout"),
        ] {
            assert_eq!(
                explicit_environment_value(&command, &format!("GIT_CONFIG_KEY_{index}")),
                Some(OsStr::new(key))
            );
            assert_eq!(
                explicit_environment_value(&command, &format!("GIT_CONFIG_VALUE_{index}")),
                Some(OsStr::new("0"))
            );
        }
        Ok(())
    }

    #[test]
    fn explicit_retention_writes_preserve_inherited_lock_waits() -> Result<(), Box<dyn Error>> {
        const TEST_FILTER: &str = "explicit_retention_writes_preserve_inherited_lock_waits";
        if std::env::var_os(CHILD_PROCESS_ENVIRONMENT).as_deref() != Some(OsStr::new(TEST_FILTER)) {
            return rerun_current_test_with_environment(
                TEST_FILTER,
                &[
                    ("GIT_CONFIG_COUNT", "2"),
                    ("GIT_CONFIG_KEY_0", "core.packedRefsTimeout"),
                    ("GIT_CONFIG_VALUE_0", "47"),
                    ("GIT_CONFIG_KEY_1", "core.filesRefLockTimeout"),
                    ("GIT_CONFIG_VALUE_1", "53"),
                ],
            );
        }
        for (key, expected) in [
            ("core.packedRefsTimeout", "47"),
            ("core.filesRefLockTimeout", "53"),
            ("core.hooksPath", "/dev/null"),
        ] {
            let output = git_command_with_hook_execution_policy(
                Path::new("."),
                GitHookExecutionPolicy::SuppressedForRetentionRef,
            )?
            .args(["config", "--get", key])
            .output()?;
            assert!(output.status.success(), "{output:?}");
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
        }
        Ok(())
    }

    #[test]
    fn a_child_that_fails_before_reading_its_input_reports_its_own_failure()
    -> Result<(), Box<dyn Error>> {
        let mut command = Command::new("sh");
        command.args(["-c", "echo refused >&2; exit 3"]);
        let output = command_output_with_input(command, &vec![b'\n'; UNREAD_INPUT_BYTES])?;
        assert_eq!(output.status.code(), Some(3));
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "refused");
        Ok(())
    }

    #[test]
    fn a_child_that_succeeds_without_reading_its_input_reports_the_broken_pipe() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);
        let result = command_output_with_input(command, &vec![b'\n'; UNREAD_INPUT_BYTES]);
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.kind() == ErrorKind::BrokenPipe),
            "unread input should surface as a broken pipe: {result:?}"
        );
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
            .env_remove(BERTH_EXECUTABLE_ENVIRONMENT)
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
