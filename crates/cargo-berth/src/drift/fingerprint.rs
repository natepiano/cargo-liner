//! The cached working-tree path fingerprint the cheap comparison comes from.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use super::constants::DRIFT_CACHE_FILE_PREFIX;
use super::constants::DRIFT_CACHE_FILE_SUFFIX;
use super::ordering;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(super) struct WorkingTreeFingerprint {
    pub(super) tracked_paths:   Vec<ReservationScopePath>,
    pub(super) untracked_paths: Vec<ReservationScopePath>,
}

impl WorkingTreeFingerprint {
    /// Every path this fingerprint reports as modified in the working tree.
    pub(super) fn modified_paths(&self) -> HashSet<&ReservationScopePath> {
        self.tracked_paths
            .iter()
            .chain(&self.untracked_paths)
            .collect()
    }

    pub(super) fn normalized(mut self) -> Self {
        ordering::normalize_paths(&mut self.tracked_paths);
        ordering::normalize_paths(&mut self.untracked_paths);
        self
    }
}

pub(super) enum StoredWorkingTreeFingerprint {
    Available(WorkingTreeFingerprint),
    Unavailable,
}

pub(super) fn read_fingerprint(path: &Path) -> StoredWorkingTreeFingerprint {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .map_or(
            StoredWorkingTreeFingerprint::Unavailable,
            StoredWorkingTreeFingerprint::Available,
        )
}

pub(super) fn publish_fingerprint(path: &Path, fingerprint: &WorkingTreeFingerprint) {
    if let Ok(serialized) = serde_json::to_vec(fingerprint) {
        std::mem::drop(fs::write(path, serialized));
    }
}

pub(super) fn fingerprint_cache_path(
    common_git_directory: &Path,
    worktree_id: WorktreeId,
) -> PathBuf {
    common_git_directory.join("cargo-berth").join(format!(
        "{DRIFT_CACHE_FILE_PREFIX}{worktree_id}{DRIFT_CACHE_FILE_SUFFIX}"
    ))
}
