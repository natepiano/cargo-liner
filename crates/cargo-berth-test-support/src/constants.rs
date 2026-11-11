//! Constants shared by more than one fixture module.

/// The variable Claude Code sets to its session id in every command a session runs.
///
/// `cargo-berth` reads it as the harness session when `CARGO_BERTH_SESSION_ID` is
/// unset. A test suite started from a Claude Code session inherits it, so every
/// process a test starts clears it; a test that needs it sets it on its own command.
pub const CLAUDE_CODE_SESSION_ENVIRONMENT: &str = "CLAUDE_CODE_SESSION_ID";

include!("../../cargo-berth/src/git/constants/child.rs");

/// The first coordination run used by integration fixtures.
pub const FIRST_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b";
/// The second coordination run used by integration fixtures.
pub const SECOND_RUN: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c";
/// The main coordination run used by ledger identity fixtures.
pub const MAIN_COORDINATION_RUN_ID: &str = "01a03f63-03e7-7fb2-ae63-5b297177f59f";
