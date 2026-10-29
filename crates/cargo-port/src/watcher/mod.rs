//! Uses recursive root watches on macOS and individual folder watches elsewhere.
//! Build trees and Git objects are filtered from events and per-folder watches.
//! Disk polling covers build sizes; events drive Git and source updates.

mod events;
mod probe;
mod refresh;
#[cfg(test)]
mod repair_test_support;
mod roots;
mod runtime;
mod skip;
#[cfg(test)]
mod skip_tests;
#[cfg(test)]
mod watch_registration_tests;

use runtime::ProjectEntry;
use runtime::WatchDrainResult;
pub(crate) use runtime::WatchRequest;
use runtime::WatchState;
use runtime::WatcherLoopState;
pub(crate) use runtime::WatcherMsg;
pub(crate) use runtime::WatcherSpawn;
pub(crate) use runtime::spawn_watcher;
