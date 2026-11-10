//! Shared exclusions and process ancestry scope.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::PoisonError;
use std::sync::RwLock;

use sysinfo::Pid;

use crate::constants::PARENT_WALK_LIMIT;

/// `commands.excluded` as the scanner reads it, shared with the app so
/// a list edited in the settings overlay applies from the next scan on
/// without restarting the worker.
#[derive(Clone, Debug)]
pub(crate) struct ExcludedCommands(Arc<RwLock<Vec<String>>>);

impl ExcludedCommands {
    /// Share `subcommands` with the worker.
    pub(crate) fn new(subcommands: Vec<String>) -> Self { Self(Arc::new(RwLock::new(subcommands))) }

    /// Replace the list the next scan leaves out.
    pub(crate) fn replace(&self, subcommands: Vec<String>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = subcommands;
    }

    /// The list as it stands, copied out for one scan.
    pub(crate) fn snapshot(&self) -> Vec<String> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Which cargo processes a scan counts as invocations.
///
/// The executable takes [`CensusScope::default`], every cargo on the
/// machine. PTY tests take `CensusScope::descendants_of` their scenario
/// script, so another session's builds and a parallel test's writers
/// never reach the grid under test. Registrations need no scope: each
/// test already reads its own capture parent.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CensusScope {
    /// The process every counted cargo runs under, at any depth; `None`
    /// counts every cargo.
    root: Option<Pid>,
}

impl CensusScope {
    /// Only the cargo processes running under `root`.
    #[cfg(test)]
    pub(crate) const fn descendants_of(root: Pid) -> Self { Self { root: Some(root) } }

    /// Whether `pid` is counted, walking `parents` up from it.
    ///
    /// Bounded by [`PARENT_WALK_LIMIT`], like every other walk here, so a
    /// reparented chain that loops cannot spin.
    pub(super) fn contains(self, pid: Pid, parents: &HashMap<Pid, Pid>) -> bool {
        let Some(root) = self.root else {
            return true;
        };
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = parents.get(&current) else {
                return false;
            };
            if parent == root {
                return true;
            }
            current = parent;
        }
        false
    }
}
