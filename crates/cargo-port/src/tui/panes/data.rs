use super::TargetsData;
use crate::tui::app::VisibleRow;

/// Identifies the inputs that produced a built detail set.
/// Two keys match iff both the selected row and the app's data generation
/// match — neither changing means the built detail is still accurate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailCacheKey {
    pub row:        VisibleRow,
    pub generation: u64,
}

/// Tracks `targets` content + the detail-set coherency stamp.
///
/// Phase 8.1a moved `cpu` out of this store onto `CpuPane`.
/// Phase 8.8 moved `package`/`git`/`ci`/`lints` content out onto
/// each per-pane struct (`PackagePane`/`GitPane`/`CiPane`/`LintsPane`).
/// `targets` remains here until Phase 9 migrates `TargetsPane`.
///
/// The detail-set "all five panes are coherent for this stamp"
/// invariant is now enforced by `Panes::set_detail_data` /
/// `Panes::clear_detail_data` — they fan out across the per-pane
/// content slots and update this store's `detail_stamp` in lockstep.
/// `PaneDataStore` itself only owns the targets slot and the stamp.
pub struct PaneDataStore {
    targets:       Option<TargetsData>,
    detail_stamp:  Option<DetailCacheKey>,
    #[cfg(test)]
    detail_builds: u64,
}

impl PaneDataStore {
    pub const fn new() -> Self {
        Self {
            targets:                    None,
            detail_stamp:               None,
            #[cfg(test)]
            detail_builds:              0,
        }
    }

    pub const fn targets(&self) -> Option<&TargetsData> { self.targets.as_ref() }

    /// True when the stored detail matches `desired` — caller can skip
    /// rebuilding. A desired key of `None` matches a cleared store.
    pub fn detail_is_current(&self, desired: Option<DetailCacheKey>) -> bool {
        self.detail_stamp == desired
    }

    /// Internal: write the targets slot + stamp. Called only by
    /// `Panes::set_detail_data` so the detail-set "all five together"
    /// invariant survives.
    pub(super) fn set_targets_with_stamp(&mut self, stamp: DetailCacheKey, targets: TargetsData) {
        self.targets = Some(targets);
        self.detail_stamp = Some(stamp);
        #[cfg(test)]
        {
            self.detail_builds += 1;
        }
    }

    /// Internal: clear the targets slot, set stamp. Called only by
    /// `Panes::clear_detail_data`.
    pub(super) fn clear_targets_with_stamp(&mut self, stamp: Option<DetailCacheKey>) {
        self.targets = None;
        self.detail_stamp = stamp;
        #[cfg(test)]
        {
            self.detail_builds += 1;
        }
    }

    /// Number of times the detail set has been written (via
    /// `Panes::set_detail_data` or `Panes::clear_detail_data`). Lets
    /// tests prove that `ensure_detail_cached` actually short-circuits
    /// on a cache hit.
    #[cfg(test)]
    pub const fn detail_build_count(&self) -> u64 { self.detail_builds }
}
