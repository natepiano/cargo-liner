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
//! before each case that starts from it, and `observes_merge_extent_of` reads
//! a merge extent observation in either record form. Only a test crate of the
//! `cargo-berth` package can expand `env!("CARGO_BIN_EXE_cargo-berth")`, so
//! each entry point here that runs `cargo-berth`, directly or through a git
//! hook, takes that path as its `executable`.

mod berth_command;
mod directory_snapshot;
mod git_driver;
mod integration_repository;
mod journal_records;

pub use berth_command::CLAUDE_CODE_SESSION_ENVIRONMENT;
pub use berth_command::berth_command;
pub use directory_snapshot::DirectorySnapshot;
pub use git_driver::EXECUTABLE_ENVIRONMENT;
pub use git_driver::GitDriver;
pub use git_driver::OptionalLocks;
pub use git_driver::git_command;
pub use integration_repository::IntegrationRepository;
pub use integration_repository::assert_success;
pub use integration_repository::claim_id;
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
