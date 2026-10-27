//! `cargo-berth` invocation that keeps the test process's Claude Code session out.

use std::ffi::OsStr;
use std::process::Command;

use crate::constants::CLAUDE_CODE_SESSION_ENVIRONMENT;
use crate::git_driver;

/// Start the `cargo-berth` under test without this process's Claude Code session.
///
/// Its git children inherit disabled auto maintenance, so they leave no background process in a
/// test repository.
///
/// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`, which
/// only that crate can expand.
#[must_use]
pub fn berth_command(executable: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(executable);
    command.env_remove(CLAUDE_CODE_SESSION_ENVIRONMENT);
    git_driver::disable_git_auto_maintenance(&mut command);
    command
}

/// Keep a Git fault or concurrency fixture on the subprocess read route.
/// The benign monitor override makes command configuration explicit without changing status.
pub fn use_git_read_subprocess(command: &mut Command) -> &mut Command {
    command
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", "false")
}
