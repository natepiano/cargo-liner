//! `RegisteredRoots` tracks recursive roots on macOS and folder watches elsewhere.
//! Discovery folders receive temporary candidate watches. Build trees and Git objects stay
//! unwatched. `DiskUsageTracker` supplies startup measurements and updates changed paths for the
//! watcher.

mod events;
mod probe;
mod refresh;
mod roots;
mod runtime;
mod skip;
#[cfg(test)]
mod skip_tests;
#[cfg(test)]
mod subscription_recovery_tests;
#[cfg(test)]
mod watch_registration_tests;
#[cfg(test)]
mod watch_role_tests;
#[cfg(test)]
mod watch_test_support;

use runtime::ProjectEntry;
use runtime::WatchDrainResult;
pub(crate) use runtime::WatchRequest;
use runtime::WatchState;
use runtime::WatcherLoopState;
pub(crate) use runtime::WatcherMsg;
pub(crate) use runtime::WatcherSpawn;
pub(crate) use runtime::spawn_watcher;
