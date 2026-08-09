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
#[cfg(test)]
pub(super) use state::ChildProcess;
use state::ExactRunningTargetOwnerEvidence;
pub(super) use state::RunProfile;
#[cfg(test)]
pub(super) use state::RunningInstance;
#[cfg(test)]
pub(super) use state::RunningKey;
pub(super) use state::RunningProcessPlacement;
pub(super) use state::RunningTargetProjectAttribution;
pub(super) use state::RunningTargets;
pub(super) use state::RunningTargetsState;
pub(super) use termination::RunningTargetTerminationCapability;
