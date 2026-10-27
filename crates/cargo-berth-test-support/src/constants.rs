//! Constants shared by more than one fixture module.

/// The variable Claude Code sets to its session id in every command a session runs.
///
/// `cargo-berth` reads it as the harness session when `CARGO_BERTH_SESSION_ID` is
/// unset. A test suite started from a Claude Code session inherits it, so every
/// process a test starts clears it; a test that needs it sets it on its own command.
pub const CLAUDE_CODE_SESSION_ENVIRONMENT: &str = "CLAUDE_CODE_SESSION_ID";
