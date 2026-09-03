//! Setup shared by the integration tests that drive git against a test repository.

use std::process::Command;

/// Names a specific `cargo-berth` for a managed hook, ahead of the installed one.
const EXECUTABLE_ENVIRONMENT: &str = "CARGO_BERTH_EXECUTABLE";

/// Start a git command whose managed hooks run the `cargo-berth` under test.
///
/// A managed hook resolves `cargo-berth` when it runs, and an installed copy
/// answers that resolution first. Every git command a test issues can fire a
/// hook, so each one names the built binary and the environment carries it
/// through git into the hook. Without this a test proves nothing about the code
/// it was compiled from: it reports on whatever `cargo install` last left behind.
pub fn git_command() -> Command {
    let mut command = Command::new("git");
    command.env(EXECUTABLE_ENVIRONMENT, env!("CARGO_BIN_EXE_cargo-berth"));
    command
}
