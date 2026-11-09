use std::collections::HashSet;
use std::path::Path;

use crate::constants::GIT_DIR;
use crate::constants::TARGET_DIR;
use crate::project::AbsolutePath;

/// Trees whose contents must never receive filesystem watches.
#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct WatchSkipSet {
    target_directories: HashSet<AbsolutePath>,
}

/// The subscription changes between two exclusion policies.
#[derive(Clone, Copy)]
pub(super) enum ExclusionTransition {
    Equivalent,
    TreesExcluded,
    TreesRestored,
    TreesExcludedAndRestored,
}

impl WatchSkipSet {
    /// Describe the watch changes needed for the next policy.
    pub(super) fn transition_to(&self, next: &Self) -> ExclusionTransition {
        let restores = self.target_directories.iter().any(|root| !next.skips(root));
        let excludes = next.target_directories.iter().any(|root| !self.skips(root));
        match (excludes, restores) {
            (false, false) => ExclusionTransition::Equivalent,
            (true, false) => ExclusionTransition::TreesExcluded,
            (false, true) => ExclusionTransition::TreesRestored,
            (true, true) => ExclusionTransition::TreesExcludedAndRestored,
        }
    }

    pub(super) fn add_target_directory(&mut self, path: AbsolutePath) {
        if let Ok(resolved) = path.canonicalize() {
            self.target_directories.insert(AbsolutePath::from(resolved));
        }
        self.target_directories.insert(path);
    }

    pub(super) fn skips(&self, path: &Path) -> bool {
        let mut previous_is_git = false;
        for component in path.components() {
            let name = component.as_os_str();
            if name == TARGET_DIR || previous_is_git && name == "objects" {
                return true;
            }
            previous_is_git = name == GIT_DIR;
        }
        self.target_directories
            .iter()
            .any(|root| path.starts_with(root))
    }

    pub(super) fn skips_resolved(&self, path: &Path) -> bool {
        self.skips(path)
            || path
                .canonicalize()
                .is_ok_and(|resolved| self.skips(&resolved))
    }
}

#[cfg(test)]
#[path = "skip_alias_tests.rs"]
mod alias_tests;
