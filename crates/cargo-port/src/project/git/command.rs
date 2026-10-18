use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::ErrorKind;
use std::path::Path;
use std::process::Command;
use std::process::Output;

use tui_pane::PERF_LOG_TARGET;

use super::constants::GIT_BINARY;
use super::constants::GIT_NO_OPTIONAL_LOCKS_ARG;

/// Standard output of a git command that exited 0.
#[derive(Debug)]
pub(super) struct GitStdout(String);

impl GitStdout {
    pub(super) fn trimmed(&self) -> &str { self.0.trim() }
    pub(super) fn lines(&self) -> impl Iterator<Item = &str> { self.0.lines() }
}

/// Why git gave no answer. The runner logs it once at warn with its operation.
#[derive(Debug)]
pub(super) struct GitReadError {
    op:    String,
    cause: GitReadCause,
}

#[derive(Debug)]
enum GitReadCause {
    /// The git process could not be started.
    NotStarted(ErrorKind),
    /// git exited with a status its caller does not read as an answer.
    Exited { code: i32, stderr: String },
    /// git was killed by a signal.
    Signalled,
    /// git exited 0, but its output did not parse.
    Unparsable,
}

impl GitReadError {
    fn logged(op: &str, cause: GitReadCause) -> Self {
        let error = Self {
            op: op.to_string(),
            cause,
        };
        tracing::warn!(op, error = %error, "git read failed");
        error
    }

    pub(super) fn unparsable(op: &str) -> Self { Self::logged(op, GitReadCause::Unparsable) }
}

impl Display for GitReadError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.op)?;
        match &self.cause {
            GitReadCause::NotStarted(kind) => write!(formatter, "could not start git: {kind}"),
            GitReadCause::Exited { code, stderr } => {
                write!(formatter, "git exited {code}: {stderr}")
            },
            GitReadCause::Signalled => formatter.write_str("git was killed by a signal"),
            GitReadCause::Unparsable => formatter.write_str("git output did not parse"),
        }
    }
}

impl Error for GitReadError {}

/// A command whose documented exit 1 means "no".
#[derive(Debug)]
pub(super) enum GitAnswer {
    Yes(GitStdout),
    No,
}

/// Prevent read-only commands from refreshing the index and feeding the watcher.
fn git_command(repo_root: &Path) -> Command {
    let mut command = Command::new(GIT_BINARY);
    command
        .arg(GIT_NO_OPTIONAL_LOCKS_ARG)
        .current_dir(repo_root);
    command
}

fn run(repo_root: &Path, op: &str, args: &[&str]) -> Result<Output, GitReadError> {
    let started = std::time::Instant::now();
    let output = git_command(repo_root).args(args).output();
    let status = output
        .as_ref()
        .ok()
        .and_then(|out| out.status.code())
        .map_or_else(|| "signal".to_string(), |code| code.to_string());
    tracing::trace!(
        target: PERF_LOG_TARGET,
        elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        op,
        status,
        "git_info_get_call"
    );
    output.map_err(|error| GitReadError::logged(op, GitReadCause::NotStarted(error.kind())))
}

fn rejected(op: &str, output: &Output) -> GitReadError {
    let cause = output
        .status
        .code()
        .map_or(GitReadCause::Signalled, |code| GitReadCause::Exited {
            code,
            stderr: String::from_utf8_lossy(&output.stderr)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string(),
        });
    GitReadError::logged(op, cause)
}

pub(super) fn git_output_logged(
    repo_root: &Path,
    op: &str,
    args: &[&str],
) -> Result<GitStdout, GitReadError> {
    let output = run(repo_root, op, args)?;
    if output.status.success() {
        Ok(GitStdout(
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ))
    } else {
        Err(rejected(op, &output))
    }
}

pub(super) fn git_answer_logged(
    repo_root: &Path,
    op: &str,
    args: &[&str],
) -> Result<GitAnswer, GitReadError> {
    let output = run(repo_root, op, args)?;
    match output.status.code() {
        Some(0) => Ok(GitAnswer::Yes(GitStdout(
            String::from_utf8_lossy(&output.stdout).into_owned(),
        ))),
        Some(1) => Ok(GitAnswer::No),
        Some(_) | None => Err(rejected(op, &output)),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn git_output_logged_fails_on_a_non_zero_exit() {
        let tmp = tempfile::tempdir().expect("create runner fixture");
        crate::test_support::init_git_repo(tmp.path());
        let missing = git_output_logged(
            tmp.path(),
            "missing",
            &["rev-parse", "--verify", "refs/heads/nope"],
        );
        assert!(matches!(
            missing,
            Err(GitReadError {
                cause: GitReadCause::Exited { code: 128, .. },
                ..
            })
        ));
        assert!(git_output_logged(tmp.path(), "head", &["rev-parse", "HEAD"]).is_ok());
        assert!(matches!(
            git_answer_logged(
                tmp.path(),
                "missing",
                &["show-ref", "--verify", "--quiet", "refs/heads/nope"]
            ),
            Ok(GitAnswer::No)
        ));
        let head = super::super::branches::get_current_branch(tmp.path()).expect("current branch");
        let reference = format!("refs/heads/{head}");
        assert!(matches!(
            git_answer_logged(
                tmp.path(),
                "present",
                &["show-ref", "--verify", "--quiet", &reference]
            ),
            Ok(GitAnswer::Yes(_))
        ));
        std::fs::write(tmp.path().join(".git/HEAD"), "garbage\n").expect("corrupt HEAD");
        assert!(
            git_answer_logged(
                tmp.path(),
                "corrupt",
                &["show-ref", "--verify", "--quiet", &reference]
            )
            .is_err()
        );
    }
}
