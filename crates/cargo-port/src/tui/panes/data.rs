use super::CiData;
use super::GitData;
use super::LintsData;
use super::PackageData;
use super::TargetsData;
#[cfg(test)]
use crate::ci::CiRun;
use crate::tui::app::VisibleRow;
use crate::tui::cpu::CpuSnapshot;

/// Identifies the inputs that produced a built `PaneDataStore` detail set.
/// Two keys match iff both the selected row and the app's data generation
/// match — neither changing means the built detail is still accurate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in super::super) struct DetailCacheKey {
    pub row:        VisibleRow,
    pub generation: u64,
}

/// Pane data with internal invariants:
///
/// - `package`/`git`/`targets`/`ci`/`lints` are the "detail set" and can only change together via
///   `set_detail_data` / `clear_detail_data`. Every write records a stamp so `detail_is_current`
///   can tell callers whether a rebuild would be redundant.
/// - `cpu` is independent of the detail set (different cadence, different producer). It has its own
///   setter and is not covered by the stamp.
///
/// Fields are private so the stamp cannot go out of sync with the data.
pub(in super::super) struct PaneDataStore {
    package:       Option<PackageData>,
    git:           Option<GitData>,
    cpu:           Option<CpuSnapshot>,
    targets:       Option<TargetsData>,
    ci:            Option<CiData>,
    lints:         Option<LintsData>,
    detail_stamp:  Option<DetailCacheKey>,
    #[cfg(test)]
    detail_builds: u64,
}

impl PaneDataStore {
    pub(in super::super) const fn new() -> Self {
        Self {
            package:                    None,
            git:                        None,
            cpu:                        None,
            targets:                    None,
            ci:                         None,
            lints:                      None,
            detail_stamp:               None,
            #[cfg(test)]
            detail_builds:              0,
        }
    }

    pub(in super::super) const fn package(&self) -> Option<&PackageData> { self.package.as_ref() }
    pub(in super::super) const fn git(&self) -> Option<&GitData> { self.git.as_ref() }
    pub(in super::super) const fn targets(&self) -> Option<&TargetsData> { self.targets.as_ref() }
    pub(in super::super) const fn ci(&self) -> Option<&CiData> { self.ci.as_ref() }
    pub(in super::super) const fn lints(&self) -> Option<&LintsData> { self.lints.as_ref() }
    pub(in super::super) const fn cpu(&self) -> Option<&CpuSnapshot> { self.cpu.as_ref() }

    /// True when the stored detail matches `desired` — caller can skip
    /// rebuilding. A desired key of `None` matches a cleared store.
    pub(in super::super) fn detail_is_current(&self, desired: Option<DetailCacheKey>) -> bool {
        self.detail_stamp == desired
    }

    pub(in super::super) fn set_detail_data(
        &mut self,
        stamp: DetailCacheKey,
        package: PackageData,
        git: GitData,
        targets: TargetsData,
        ci: CiData,
        lints: LintsData,
    ) {
        self.package = Some(package);
        self.git = Some(git);
        self.targets = Some(targets);
        self.ci = Some(ci);
        self.lints = Some(lints);
        self.detail_stamp = Some(stamp);
        #[cfg(test)]
        {
            self.detail_builds += 1;
        }
    }

    /// Clear the detail set and stamp it with the key that produced the
    /// empty result. Stamping with `desired` (rather than always `None`)
    /// keeps a "tried to build for this key and got nothing" state from
    /// thrashing: the next frame with the same key short-circuits.
    pub(in super::super) fn clear_detail_data(&mut self, stamp: Option<DetailCacheKey>) {
        self.package = None;
        self.git = None;
        self.targets = None;
        self.ci = None;
        self.lints = None;
        self.detail_stamp = stamp;
        #[cfg(test)]
        {
            self.detail_builds += 1;
        }
    }

    pub(in super::super) fn set_cpu(&mut self, snapshot: CpuSnapshot) { self.cpu = Some(snapshot); }

    /// Number of times the detail set has been written (via
    /// `set_detail_data` or `clear_detail_data`). Lets tests prove that
    /// `ensure_detail_cached` actually short-circuits on a cache hit.
    #[cfg(test)]
    pub(in super::super) const fn detail_build_count(&self) -> u64 { self.detail_builds }

    /// Test-only override: replace the `lints` slot without disturbing the
    /// detail stamp. Used by render tests that need a specific lints
    /// payload after a normal `ensure_detail_cached`. Leaves the stamp in
    /// place so a subsequent `ensure_detail_cached` short-circuits and
    /// preserves the override.
    #[cfg(test)]
    pub(in super::super) fn override_lints_for_test(&mut self, data: LintsData) {
        self.lints = Some(data);
    }

    /// Test-only override: replace `ci.runs` on an already-populated
    /// detail set and drop the mode label. Mirrors what a production
    /// rebuild would produce for fixture CI data without forcing the test
    /// to assemble a whole `DetailPaneData`.
    #[cfg(test)]
    pub(in super::super) fn override_ci_runs_for_test(&mut self, runs: Vec<CiRun>) {
        if let Some(ci) = self.ci.as_mut() {
            ci.runs = runs;
            ci.mode_label = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::panes::CiEmptyState;

    fn any_row() -> VisibleRow { VisibleRow::Root { node_index: 0 } }

    fn other_row() -> VisibleRow {
        VisibleRow::Member {
            node_index:   0,
            group_index:  0,
            member_index: 0,
        }
    }

    fn build_empty_data() -> (PackageData, GitData, TargetsData, CiData, LintsData) {
        (
            PackageData::default(),
            GitData::default(),
            TargetsData::default(),
            CiData {
                runs:           Vec::new(),
                mode_label:     None,
                current_branch: None,
                empty_state:    CiEmptyState::Loading,
            },
            LintsData::default(),
        )
    }

    #[test]
    fn new_store_is_current_only_with_no_selection() {
        let store = PaneDataStore::new();
        assert!(store.detail_is_current(None));
        assert!(!store.detail_is_current(Some(DetailCacheKey {
            row:        any_row(),
            generation: 0,
        })));
    }

    #[test]
    fn set_detail_matches_its_stamp_and_differs_from_others() {
        let mut store = PaneDataStore::new();
        let key = DetailCacheKey {
            row:        any_row(),
            generation: 3,
        };
        let (pkg, git, targets, ci, lints) = build_empty_data();
        store.set_detail_data(key, pkg, git, targets, ci, lints);

        assert!(store.detail_is_current(Some(key)));
        assert!(!store.detail_is_current(None));
        assert!(!store.detail_is_current(Some(DetailCacheKey {
            row:        any_row(),
            generation: 4,
        })));
        assert!(!store.detail_is_current(Some(DetailCacheKey {
            row:        other_row(),
            generation: 3,
        })));
    }

    #[test]
    fn clear_detail_records_given_stamp() {
        let mut store = PaneDataStore::new();
        let key = DetailCacheKey {
            row:        any_row(),
            generation: 7,
        };
        store.clear_detail_data(Some(key));
        assert!(store.detail_is_current(Some(key)));
        assert!(store.package().is_none());
        assert!(store.git().is_none());
        assert!(store.targets().is_none());
        assert!(store.ci().is_none());
        assert!(store.lints().is_none());
    }

    #[test]
    fn clear_detail_with_none_matches_none() {
        let mut store = PaneDataStore::new();
        store.clear_detail_data(None);
        assert!(store.detail_is_current(None));
    }
}
