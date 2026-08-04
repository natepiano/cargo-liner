//! Detect which cargo bin/example/bench targets are currently running.
//!
//! Each completed shared-observer cycle supplies identity-bound executable,
//! ancestry, and metrics records. Executables under known workspace
//! `target_directory` paths are classified as bin, example, or bench targets.

mod app_tick;
mod constants;
mod state;
mod termination;

pub(super) use constants::RUNNING_TARGETS_REFRESH_INTERVAL;
pub(super) use state::*;
pub(super) use termination::*;
