//! The `Selection` subsystem.
//!
//! Phase 3 of the App-API carve (see `docs/app-api.md`). Absorbs the
//! eight selection-cluster fields that previously lived on `App`:
//! `cached_visible_rows`, `cached_root_sorted`, `cached_child_sorted`,
//! `cached_fit_widths`, `selection_paths`, `selection`
//! (`SelectionSync`), `expanded`, `finder`. Exposes both raw
//! field-level accessors (used by App's existing impl-files via
//! delegation) and the documented mutation guard
//! [`SelectionMutation`] for visibility-changing operations.
//!
//! ## Mutation guard pattern (RAII, self-only flavor)
//!
//! Visibility-changing methods are not callable on [`Selection`]
//! directly. Callers obtain a [`SelectionMutation`] via
//! [`Selection::mutate`], call `toggle_expand` / `apply_finder` on the
//! guard, then drop the guard — its `Drop` recomputes
//! `cached_visible_rows`. Cursor moves do **not** change visibility,
//! so they stay as direct methods on `Selection` and bypass the
//! guard. This matches the "Mutation guard (RAII) — self-only
//! flavor" entry in the App-module pattern index
//! (`src/tui/app/mod.rs`).

use std::collections::HashMap;
use std::collections::HashSet;

use crate::project_list::ProjectList;
use crate::tui::app::ExpandKey;
use crate::tui::app::FinderState;
use crate::tui::app::ProjectListWidths;
use crate::tui::app::SelectionPaths;
use crate::tui::app::SelectionSync;
use crate::tui::app::VisibleRow;
use crate::tui::app::snapshots;

/// Owns every selection-related piece of state. App holds a single
/// `selection: Selection` field instead of the eight raw fields it
/// carried before Phase 3.
pub(in crate::tui) struct Selection {
    paths:               SelectionPaths,
    sync:                SelectionSync,
    expanded:            HashSet<ExpandKey>,
    finder:              FinderState,
    cached_visible_rows: Vec<VisibleRow>,
    cached_root_sorted:  Vec<u64>,
    cached_child_sorted: HashMap<usize, Vec<u64>>,
    cached_fit_widths:   ProjectListWidths,
}

impl Selection {
    pub(in crate::tui) fn new(lint_enabled: bool) -> Self {
        Self {
            paths:               SelectionPaths::new(),
            sync:                SelectionSync::Stable,
            expanded:            HashSet::new(),
            finder:              FinderState::new(),
            cached_visible_rows: Vec::new(),
            cached_root_sorted:  Vec::new(),
            cached_child_sorted: HashMap::new(),
            cached_fit_widths:   ProjectListWidths::new(lint_enabled),
        }
    }

    // ── path tracking ───────────────────────────────────────────────

    pub(in crate::tui) const fn paths(&self) -> &SelectionPaths { &self.paths }

    pub(in crate::tui) const fn paths_mut(&mut self) -> &mut SelectionPaths { &mut self.paths }

    // ── sync flag ───────────────────────────────────────────────────

    pub(in crate::tui) const fn sync(&self) -> SelectionSync { self.sync }

    pub(in crate::tui) const fn mark_sync_changed(&mut self) { self.sync = SelectionSync::Changed; }

    pub(in crate::tui) const fn mark_sync_stable(&mut self) { self.sync = SelectionSync::Stable; }

    // ── expansion set ───────────────────────────────────────────────

    pub(in crate::tui) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    /// Mutable access to the expansion set. Phase 3 keeps this
    /// directly accessible because most callers (rebuild paths
    /// in `tui::app::async_tasks` and `tui::app::navigation`) need
    /// to populate the set without triggering the per-mutation
    /// recompute the `SelectionMutation` guard fires. The guard
    /// covers single-key toggle paths where the recompute is the
    /// whole point.
    pub(in crate::tui) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        &mut self.expanded
    }

    // ── finder state ────────────────────────────────────────────────

    pub(in crate::tui) const fn finder(&self) -> &FinderState { &self.finder }

    pub(in crate::tui) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.finder }

    // ── cached visible rows ─────────────────────────────────────────

    pub(in crate::tui) fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Replace the cached visible rows directly (for callers that
    /// build the row list elsewhere — e.g. preserving a custom
    /// visibility filter). Most paths should use
    /// [`Self::recompute_visibility`] instead.
    #[allow(
        dead_code,
        reason = "facade method retained for future explicit-row callers; \
                  recompute_visibility covers every current call site"
    )]
    pub(in crate::tui) fn set_visible_rows(&mut self, rows: Vec<VisibleRow>) {
        self.cached_visible_rows = rows;
    }

    /// Recompute `cached_visible_rows` from the current `expanded`
    /// set and `projects`. Called by [`SelectionMutation::drop`] and
    /// (via App) from `TreeMutation::drop` so externally-driven tree
    /// mutations also keep the visible-rows cache fresh.
    pub(in crate::tui) fn recompute_visibility(
        &mut self,
        projects: &ProjectList,
        include_non_rust: bool,
    ) {
        self.cached_visible_rows =
            snapshots::build_visible_rows(projects, &self.expanded, include_non_rust);
    }

    // ── disk-sort caches ────────────────────────────────────────────

    pub(in crate::tui) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(in crate::tui) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(in crate::tui) fn set_disk_caches(
        &mut self,
        root_sorted: Vec<u64>,
        child_sorted: HashMap<usize, Vec<u64>>,
    ) {
        self.cached_root_sorted = root_sorted;
        self.cached_child_sorted = child_sorted;
    }

    // ── fit widths ──────────────────────────────────────────────────

    pub(in crate::tui) const fn fit_widths(&self) -> &ProjectListWidths { &self.cached_fit_widths }

    #[allow(
        dead_code,
        reason = "facade for callers that observe new widths into the cached set; \
                  current paths replace the whole snapshot via set_fit_widths"
    )]
    pub(in crate::tui) const fn fit_widths_mut(&mut self) -> &mut ProjectListWidths {
        &mut self.cached_fit_widths
    }

    pub(in crate::tui) fn set_fit_widths(&mut self, widths: ProjectListWidths) {
        self.cached_fit_widths = widths;
    }

    pub(in crate::tui) fn reset_fit_widths(&mut self, lint_enabled: bool) {
        self.cached_fit_widths = ProjectListWidths::new(lint_enabled);
    }

    // ── mutation guard entry point ──────────────────────────────────

    /// Borrow `Selection` for a visibility-changing mutation.
    ///
    /// **Type-level invariant:** the guard's mutating methods
    /// (`toggle_expand`, `expand`, `collapse`, `expanded_mut`,
    /// `finder_mut`) are only callable through the returned guard.
    /// The guard's `Drop` recomputes `cached_visible_rows`, so
    /// visibility-affecting mutations cannot drift out of sync with
    /// their derived rows. See the "Recurring patterns" section in
    /// `src/tui/app/mod.rs` for the pattern; this is the self-only
    /// flavor.
    #[allow(
        dead_code,
        reason = "Phase 3 lands the guard API. Existing call sites in \
                  tui::app::navigation (try_expand / try_collapse) still use \
                  expanded_mut directly because they recompute via a separate \
                  ensure_visible_rows_cached() call in the same code path; \
                  Phase 7 migrates them to take the guard."
    )]
    pub(in crate::tui) const fn mutate<'a>(
        &'a mut self,
        projects: &'a ProjectList,
        include_non_rust: bool,
    ) -> SelectionMutation<'a> {
        SelectionMutation {
            selection: self,
            projects,
            include_non_rust,
        }
    }
}

/// RAII guard for visibility-changing [`Selection`] mutations.
/// Obtained via [`Selection::mutate`]; `Drop` recomputes
/// `cached_visible_rows`. Mutation guard (RAII) — self-only flavor.
/// See `src/tui/app/mod.rs` § "Recurring patterns".
#[allow(
    dead_code,
    reason = "Phase 3 lands the guard alongside Selection. Migration of \
              existing call sites to take the guard happens in Phase 7; \
              the guard ships now so the type is in place when call sites \
              switch."
)]
pub(in crate::tui) struct SelectionMutation<'a> {
    selection:        &'a mut Selection,
    projects:         &'a ProjectList,
    include_non_rust: bool,
}

#[allow(
    dead_code,
    reason = "guard methods land in Phase 3 alongside the type; Phase 7 \
              migrates call sites to take the guard"
)]
impl SelectionMutation<'_> {
    /// Toggle membership of `key` in the expansion set. Returns
    /// `true` if the key was newly inserted.
    pub(in crate::tui) fn toggle_expand(&mut self, key: ExpandKey) -> bool {
        if self.selection.expanded.contains(&key) {
            self.selection.expanded.remove(&key);
            false
        } else {
            self.selection.expanded.insert(key);
            true
        }
    }

    /// Insert `key` into the expansion set. Returns `true` if the
    /// key was newly inserted.
    pub(in crate::tui) fn expand(&mut self, key: ExpandKey) -> bool {
        self.selection.expanded.insert(key)
    }

    /// Remove `key` from the expansion set. Returns `true` if the
    /// key was present.
    pub(in crate::tui) fn collapse(&mut self, key: &ExpandKey) -> bool {
        self.selection.expanded.remove(key)
    }

    /// Mutable access to the underlying expansion set, for bulk
    /// operations (e.g. `clear`, multi-key inserts) that still want
    /// the drop-recompute to fire afterward.
    pub(in crate::tui) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        &mut self.selection.expanded
    }

    /// Mutable access to the finder state, for callers that update
    /// the finder query / results inline. The drop-recompute fires
    /// on guard release.
    pub(in crate::tui) const fn finder_mut(&mut self) -> &mut FinderState {
        &mut self.selection.finder
    }
}

impl Drop for SelectionMutation<'_> {
    fn drop(&mut self) {
        self.selection
            .recompute_visibility(self.projects, self.include_non_rust);
    }
}
