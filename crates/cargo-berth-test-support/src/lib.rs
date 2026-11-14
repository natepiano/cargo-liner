#![allow(
    clippy::expect_used,
    reason = "tests should stop immediately when git or a fixture is wrong"
)]

//! Fixtures shared by the `cargo-berth` integration tests.
//!
//! Each integration test is its own crate, so what two test files share lives
//! here: `berth_command` starts `cargo-berth` without the test process's Claude
//! Code session, `GitDriver` runs git under one test file's policy,
//! `IntegrationRepository` builds a repository whose lanes target an
//! `integration` branch, `DirectorySnapshot` restores a fixture built once
//! before each case that starts from it, `RepositoryTemplate` copies a
//! repository built once per test binary into each test, and
//! `observes_merge_extent_of` reads a merge extent observation in either
//! record form. Only a test crate of the
//! `cargo-berth` package can expand `env!("CARGO_BIN_EXE_cargo-berth")`, so
//! each entry point here that runs `cargo-berth`, directly or through a git
//! hook, takes that path as its `executable`.

// Git and fixture commands compile the same deadline and cleanup implementation.
#[path = "../../cargo-berth/src/git/child.rs"]
mod child_process;
#[path = "../../cargo-berth/src/git/spawn_lock.rs"]
mod spawn_lock;

mod berth_command;
mod command_deadline;
mod constants;
mod directory_snapshot;
mod fixture_command;
mod fixture_directory;
mod git_driver;
mod git_trace;
mod hook_command;
mod hook_history;
mod integration_repository;
mod journal_records;
mod recorded_history;
mod repository_template;
mod scratch_repository;
mod spawn_observation;
mod test_process_group;

use std::process::Command;
use std::process::Output;
use std::time::Duration;

pub use berth_command::berth_command;
pub use berth_command::use_git_read_subprocess;
pub use command_deadline::ChildDeadlineExt;
pub use command_deadline::CommandDeadlineExt;
pub use constants::CLAUDE_CODE_SESSION_ENVIRONMENT;
pub use constants::FIRST_RUN;
pub use constants::MAIN_COORDINATION_RUN_ID;
pub use constants::SECOND_RUN;
pub use directory_snapshot::DirectorySnapshot;
pub use fixture_command::FixtureCommand;
pub use fixture_directory::fixture_tempdir;
pub use git_driver::EXECUTABLE_ENVIRONMENT;
pub use git_driver::GitDriver;
pub use git_driver::OptionalLocks;
pub use git_driver::fixture_git_command;
pub use git_driver::git_command;
pub use git_trace::GitTraceError;
pub use git_trace::capture_git_trace;
pub use hook_command::AmbientHarnessSession;
pub use hook_command::spawn_hook_verb;
pub use hook_command::spawn_hook_verb_with_ceiling;
pub use hook_history::ObservedIncursion;
pub use hook_history::ReleasedWorkEvidence;
pub use integration_repository::CrossTargetOrdering;
pub use integration_repository::CrossTargetPair;
pub use integration_repository::IntegrationRepository;
pub use integration_repository::assert_success;
pub use integration_repository::board_reservation_entries;
pub use integration_repository::board_reservation_ids;
pub use integration_repository::claim_id;
pub use integration_repository::copy_git_repository_seed;
pub use integration_repository::deferring_run_path;
pub use integration_repository::deferring_run_scope;
pub use integration_repository::json;
pub use integration_repository::reservation_row;
pub use integration_repository::worktree_identity_and_marker_run;
pub use integration_repository::write_file;
pub use journal_records::HOLDER_MERGE_EXTENT_OBSERVED;
pub use journal_records::MERGE_EXTENT_OBSERVED;
pub use journal_records::is_merge_extent_observation;
pub use journal_records::observes_merge_extent_of;
pub use recorded_history::copy_recorded_repository;
pub use recorded_history::relocate_tree;
pub use recorded_history::save_recorded_history;
pub use recorded_history::write_recorded_state;
pub use repository_template::LedgerFixture;
pub use repository_template::LedgerTemplate;
pub use repository_template::RepositoryTemplate;
pub use repository_template::TemplateBuildTiming;
pub use scratch_repository::build_scratch_repository;
pub use spawn_observation::SpawnContention;
pub use spawn_observation::observe_spawn_contention;
pub use spawn_observation::with_spawn_lock;
pub use test_process_group::TestProcessGroup;

/// Capture a fixture command with bounded exit and pipe waits.
///
/// # Errors
/// Returns a named timeout, spawn error, or pipe error.
pub fn command_output(command: &mut Command) -> std::io::Result<Output> {
    child_process::command_output(command, child_process::GIT_COMMAND_DEADLINE)
}

/// Capture a fixture whose full setup needs a caller-selected deadline.
///
/// # Errors
/// Returns a spawn error, a named timeout, or a pipe error.
pub fn fixture_output_with_deadline(
    command: &mut FixtureCommand,
    deadline: Duration,
) -> std::io::Result<Output> {
    child_process::command_output(command.command_mut(), deadline)
}
