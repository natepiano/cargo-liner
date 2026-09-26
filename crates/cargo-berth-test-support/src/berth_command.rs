//! `cargo-berth` invocation that keeps the test process's Claude Code session out.

use std::ffi::OsStr;
use std::process::Command;

/// The variable Claude Code sets to its session id in every command a session runs.
///
/// `cargo-berth` reads it as the harness session when `CARGO_BERTH_SESSION_ID` is
/// unset. A test suite started from a Claude Code session inherits it, so every
/// process a test starts clears it; a test that needs it sets it on its own command.
pub const CLAUDE_CODE_SESSION_ENVIRONMENT: &str = "CLAUDE_CODE_SESSION_ID";

/// Start the `cargo-berth` under test without this process's Claude Code session.
///
/// `executable` is the test crate's own `env!("CARGO_BIN_EXE_cargo-berth")`, which
/// only that crate can expand.
#[must_use]
pub fn berth_command(executable: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(executable);
    command.env_remove(CLAUDE_CODE_SESSION_ENVIRONMENT);
    command
}
