//! Launch a shell command in a supplied directory.

use std::io::Result;
use std::path::Path;

/// Spawn a command through the platform shell with child output suppressed.
///
/// # Errors
/// Returns the I/O error when spawning the shell fails.
pub fn spawn_shell_command(command: &str, cwd: &Path) -> Result<()> {
    let mut process = if cfg!(windows) {
        let mut process = std::process::Command::new("cmd");
        process.arg("/C").arg(command);
        process
    } else {
        let mut process = std::process::Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process
        .current_dir(cwd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
}
