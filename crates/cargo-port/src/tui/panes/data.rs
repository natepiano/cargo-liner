use crate::tui::project_list::VisibleRow;

/// Identifies the inputs that produced a built detail set.
/// Two keys match iff both the selected row and the app's data generation
/// match — neither changing means the built detail is still accurate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) struct DetailCacheKey {
    pub visible_row: VisibleRow,
    pub generation:  u64,
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
pub(in crate::tui) struct PaneDataStore {
    detail_stamp:  DetailCacheState,
    #[cfg(test)]
    detail_builds: u64,
}

impl PaneDataStore {
    pub(super) const fn new() -> Self {
        Self {
            detail_stamp:               DetailCacheState::Cleared,
            #[cfg(test)]
            detail_builds:              0,
        }
    }

    /// Return whether the stored details match the desired state.
    /// `Cleared` matches details for no selection.
    pub(in crate::tui) fn detail_is_current(&self, desired: DetailCacheState) -> bool {
        self.detail_stamp == desired
    }

    /// Internal: write the stamp. Called only by `Panes::set_detail_data`
    /// / `Panes::clear_detail_data` so the detail-set "all five together"
    /// invariant survives.
    pub(super) const fn set_detail_stamp(&mut self, stamp: DetailCacheState) {
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

/// Cleared panes are current for no selection; selected details name their inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum DetailCacheState {
    Cleared,
    Current(DetailCacheKey),
}
