//! The `Selection` subsystem.
//!
//! Owns the eight selection-cluster fields:
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

use super::app;
use super::app::ExpandKey;
use super::app::FinderState;
use super::app::ProjectListWidths;
use super::app::SelectionPaths;
use super::app::SelectionSync;
use super::app::VisibleRow;
use crate::project_list::ProjectList;

/// Owns every selection-related piece of state. App holds a single
/// `selection: Selection` field.
pub(super) struct Selection {
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
    pub(super) fn new(lint_enabled: bool) -> Self {
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

    pub(super) const fn paths(&self) -> &SelectionPaths { &self.paths }

    pub(super) const fn paths_mut(&mut self) -> &mut SelectionPaths { &mut self.paths }

    // ── sync flag ───────────────────────────────────────────────────

    pub(super) const fn sync(&self) -> SelectionSync { self.sync }

    pub(super) const fn mark_sync_changed(&mut self) { self.sync = SelectionSync::Changed; }

    pub(super) const fn mark_sync_stable(&mut self) { self.sync = SelectionSync::Stable; }

    // ── expansion set ───────────────────────────────────────────────

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    /// Mutable access to the expansion set. Kept directly
    /// accessible because most callers (rebuild paths in
    /// `tui::app::async_tasks` and `tui::app::navigation`)
    /// populate the set without wanting the per-mutation recompute
    /// the `SelectionMutation` guard fires. The guard covers
    /// single-key toggle paths where the recompute is the whole
    /// point.
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> { &mut self.expanded }

    // ── finder state ────────────────────────────────────────────────

    pub(super) const fn finder(&self) -> &FinderState { &self.finder }

    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.finder }

    // ── cached visible rows ─────────────────────────────────────────

    pub(super) fn visible_rows(&self) -> &[VisibleRow] { &self.cached_visible_rows }

    /// Recompute `cached_visible_rows` from the current `expanded`
    /// set and `projects`. Called by [`SelectionMutation::drop`] and
    /// (via App) from `TreeMutation::drop` so externally-driven tree
    /// mutations also keep the visible-rows cache fresh.
    pub(super) fn recompute_visibility(&mut self, projects: &ProjectList, include_non_rust: bool) {
        self.cached_visible_rows =
            app::build_visible_rows(projects, &self.expanded, include_non_rust);
    }

    // ── disk-sort caches ────────────────────────────────────────────

    pub(super) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(super) fn set_disk_caches(
        &mut self,
        root_sorted: Vec<u64>,
        child_sorted: HashMap<usize, Vec<u64>>,
    ) {
        self.cached_root_sorted = root_sorted;
        self.cached_child_sorted = child_sorted;
    }

    // ── fit widths ──────────────────────────────────────────────────

    pub(super) const fn fit_widths(&self) -> &ProjectListWidths { &self.cached_fit_widths }

    /// Test-only — production paths replace the whole snapshot via
    /// [`Self::set_fit_widths`] and never observe individual columns
    /// after seeding.
    #[cfg(test)]
    pub(super) const fn fit_widths_mut(&mut self) -> &mut ProjectListWidths {
        &mut self.cached_fit_widths
    }

    pub(super) fn set_fit_widths(&mut self, widths: ProjectListWidths) {
        self.cached_fit_widths = widths;
    }

    pub(super) fn reset_fit_widths(&mut self, lint_enabled: bool) {
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
        reason = "tui::app::navigation (try_expand / try_collapse) still calls \
                  expanded_mut directly because it recomputes via a separate \
                  ensure_visible_rows_cached() call in the same code path."
    )]
    pub(super) const fn mutate<'a>(
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
    reason = "guard ships alongside Selection so the type is in place \
              while call sites still use the direct accessors"
)]
pub(super) struct SelectionMutation<'a> {
    selection:        &'a mut Selection,
    projects:         &'a ProjectList,
    include_non_rust: bool,
}

#[allow(
    dead_code,
    reason = "guard methods ship alongside the type while call sites \
              still use the direct accessors"
)]
impl SelectionMutation<'_> {
    /// Toggle membership of `key` in the expansion set. Returns
    /// `true` if the key was newly inserted.
    pub(super) fn toggle_expand(&mut self, key: ExpandKey) -> bool {
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
    pub(super) fn expand(&mut self, key: ExpandKey) -> bool { self.selection.expanded.insert(key) }

    /// Remove `key` from the expansion set. Returns `true` if the
    /// key was present.
    pub(super) fn collapse(&mut self, key: &ExpandKey) -> bool {
        self.selection.expanded.remove(key)
    }

    /// Mutable access to the underlying expansion set, for bulk
    /// operations (e.g. `clear`, multi-key inserts) that still want
    /// the drop-recompute to fire afterward.
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        &mut self.selection.expanded
    }

    /// Mutable access to the finder state, for callers that update
    /// the finder query / results inline. The drop-recompute fires
    /// on guard release.
    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { &mut self.selection.finder }
}

impl Drop for SelectionMutation<'_> {
    fn drop(&mut self) {
        self.selection
            .recompute_visibility(self.projects, self.include_non_rust);
    }
}
