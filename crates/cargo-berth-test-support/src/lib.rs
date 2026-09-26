#![allow(
    clippy::expect_used,
    reason = "tests should stop immediately when git or a fixture is wrong"
)]

//! Fixtures shared by the `cargo-berth` integration tests.
//!
//! Each integration test is its own crate, so what two test files share lives
//! here: `GitDriver` runs git under one test file's policy, and
//! `IntegrationRepository` builds a repository whose lanes target an
//! `integration` branch. Only a test crate of the `cargo-berth` package can
//! expand `env!("CARGO_BIN_EXE_cargo-berth")`, so each entry point here that
//! runs `cargo-berth`, directly or through a git hook, takes that path as its
//! `executable`.

mod git_driver;
mod integration_repository;

pub use git_driver::EXECUTABLE_ENVIRONMENT;
pub use git_driver::GitDriver;
pub use git_driver::OptionalLocks;
pub use git_driver::git_command;
pub use integration_repository::IntegrationRepository;
pub use integration_repository::assert_success;
pub use integration_repository::claim_id;
pub use integration_repository::json;
pub use integration_repository::reservation_row;
pub use integration_repository::worktree_identity_and_marker_run;
pub use integration_repository::write_file;
