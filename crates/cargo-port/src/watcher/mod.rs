//! Watches source folders individually, excluding build trees and Git objects.
//! New folders receive non-recursive watches. Disk polling covers changes in
//! unwatched build trees; filesystem events still drive Git and source updates.

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

use runtime::ProjectEntry;
use runtime::WatchDrainResult;
pub(crate) use runtime::WatchRequest;
use runtime::WatchState;
use runtime::WatcherLoopState;
pub(crate) use runtime::WatcherMsg;
pub(crate) use runtime::WatcherSpawn;
pub(crate) use runtime::spawn_watcher;
