use std::io;
use std::path::Path;
use std::process::Command;
use std::process::Output;

/// Build a git subprocess rooted at `repo_root` with `--no-optional-locks`
/// set. The flag prevents `git status` (and any other read-only command
/// that touches the index stat cache) from rewriting `.git/index`,
/// which the file watcher would otherwise observe as an external
/// change and re-emit a refresh signal — a self-sustaining CPU and
/// rate-limit loop.
pub(super) fn git_command(repo_root: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("--no-optional-locks").current_dir(repo_root);
    cmd
}

pub(super) fn git_output_logged<const N: usize>(
    repo_root: &Path,
    op: &str,
    args: [&str; N],
) -> io::Result<Output> {
    let started = std::time::Instant::now();
    let output = git_command(repo_root).args(args).output();
    let status = output
        .as_ref()
        .ok()
        .and_then(|out| out.status.code())
        .map_or_else(|| "signal".to_string(), |code| code.to_string());
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        repo_root = %repo_root.display(),
        op,
        status,
        "git_info_get_call"
    );
    output
}
