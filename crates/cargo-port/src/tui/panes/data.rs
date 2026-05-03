use crate::tui::app::VisibleRow;

/// Identifies the inputs that produced a built detail set.
/// Two keys match iff both the selected row and the app's data generation
/// match — neither changing means the built detail is still accurate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailCacheKey {
    pub row:        VisibleRow,
    pub generation: u64,
}

/// Tracks the detail-set coherency stamp. Per-pane content
/// (`cpu`, `package`, `git`, `ci`, `lints`, `targets`) lives on
/// each per-pane struct; this store owns only the stamp.
///
/// The detail-set "all five panes are coherent for this stamp"
/// invariant is enforced by `Panes::set_detail_data` /
/// `Panes::clear_detail_data` — they fan out across the five
/// per-pane content slots and update this store's `detail_stamp`
/// in lockstep.
pub struct PaneDataStore {
    detail_stamp:  Option<DetailCacheKey>,
    #[cfg(test)]
    detail_builds: u64,
}

impl PaneDataStore {
    pub const fn new() -> Self {
        Self {
            detail_stamp:               None,
            #[cfg(test)]
            detail_builds:              0,
        }
    }

    /// True when the stored detail matches `desired` — caller can skip
    /// rebuilding. A desired key of `None` matches a cleared store.
    pub fn detail_is_current(&self, desired: Option<DetailCacheKey>) -> bool {
        self.detail_stamp == desired
    }

    /// Internal: write the stamp. Called only by `Panes::set_detail_data`
    /// / `Panes::clear_detail_data` so the detail-set "all five together"
    /// invariant survives.
    pub(super) const fn set_detail_stamp(&mut self, stamp: Option<DetailCacheKey>) {
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
