//! Watches the scan root recursively for filesystem changes and maps
//! events to discovered projects for disk-usage and git-sync updates.
//!
//! Recursive `notify` subscriptions cover the configured scan roots, plus
//! discovered project roots that are not already covered by a scan root. Events
//! are matched to projects by prefix, debounced, and result in both
//! `BackgroundMsg::DiskUsage` and `BackgroundMsg::CheckoutInfo` / `BackgroundMsg::RepoInfo`
//! updates. New project directories are detected automatically; removed directories trigger a
//! zero-byte update so the app can mark them as deleted.
//!
//! On macOS (`FSEvents`) this stays a small set of kernel subscriptions: scan
//! roots cover normal discovery, and late per-project roots are added only
//! when no recursive root already covers the path. Linux / Windows may want a
//! different approach in the future to avoid inotify watch limits.

mod events;
mod probe;
mod refresh;
mod roots;
mod runtime;

use runtime::ProjectEntry;
use runtime::WatchState;
use runtime::WatcherLoopState;
pub(crate) use runtime::*;
