//! The `Panes` subsystem.
//!
//! Phase 1 of the App-API carve (see `docs/app-api.md`). Absorbs the
//! eight pane-related fields that previously lived on `App`:
//! `pane_manager`, `pane_data`, `visited_panes`, `layout_cache`,
//! `worktree_summary_cache`, `hovered_pane_row`, `ci_display_modes`,
//! `cpu_poller`. Exposes a small facade so App's impl-files and the
//! `panes/` siblings stop reaching into App's private guts directly.
//!
//! Phase 1 is field-cluster absorption only. The per-pane `Pane` trait
//! split is Phase 7. `handle_input`-style methods that need cross-
//! subsystem access are not added here yet — they remain free functions
//! taking `&mut App` until later phases.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use super::data::PaneDataStore;
use super::layout::LayoutCache;
use super::spec::PaneId;
use super::support::WorktreeInfo;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::app::HoveredPaneRow;
use crate::tui::cpu::CpuPoller;
use crate::tui::pane::PaneManager;

/// Owns every pane-related piece of state. App holds a single `panes:
/// Panes` field instead of the eight raw fields it carried before
/// Phase 1.
#[allow(
    clippy::struct_field_names,
    reason = "fields are named for the App fields they absorb; keeping the names \
              intact preserves grep-ability across the carve"
)]
pub(in crate::tui) struct Panes {
    pane_manager:           PaneManager,
    pane_data:              PaneDataStore,
    visited_panes:          HashSet<PaneId>,
    layout_cache:           LayoutCache,
    /// See `tui::app::mod.rs` doc comment on the original field —
    /// `RefCell` because `worktree_summary_or_compute` runs inside
    /// `build_pane_data_common`, which only has `&App`.
    worktree_summary_cache: RefCell<HashMap<AbsolutePath, Vec<WorktreeInfo>>>,
    hovered_pane_row:       Option<HoveredPaneRow>,
    ci_display_modes:       HashMap<AbsolutePath, CiRunDisplayMode>,
    cpu_poller:             CpuPoller,
}

impl Panes {
    pub(in crate::tui) fn new(cpu_cfg: &CpuConfig) -> Self {
        Self {
            pane_manager:           PaneManager::new(),
            pane_data:              PaneDataStore::new(),
            visited_panes:          std::iter::once(PaneId::ProjectList).collect(),
            layout_cache:           LayoutCache::default(),
            worktree_summary_cache: RefCell::new(HashMap::new()),
            hovered_pane_row:       None,
            ci_display_modes:       HashMap::new(),
            cpu_poller:             CpuPoller::new(cpu_cfg),
        }
    }

    pub(in crate::tui) const fn pane_manager(&self) -> &PaneManager { &self.pane_manager }

    pub(in crate::tui) const fn pane_manager_mut(&mut self) -> &mut PaneManager {
        &mut self.pane_manager
    }

    pub(in crate::tui) const fn pane_data(&self) -> &PaneDataStore { &self.pane_data }

    pub(in crate::tui) const fn pane_data_mut(&mut self) -> &mut PaneDataStore {
        &mut self.pane_data
    }

    pub(in crate::tui) const fn layout_cache(&self) -> &LayoutCache { &self.layout_cache }

    pub(in crate::tui) const fn layout_cache_mut(&mut self) -> &mut LayoutCache {
        &mut self.layout_cache
    }

    pub(in crate::tui) fn mark_visited(&mut self, pane: PaneId) { self.visited_panes.insert(pane); }

    pub(in crate::tui) fn unvisit(&mut self, pane: PaneId) { self.visited_panes.remove(&pane); }

    pub(in crate::tui) fn remembers_visited(&self, pane: PaneId) -> bool {
        self.visited_panes.contains(&pane)
    }

    pub(in crate::tui) const fn set_hover(&mut self, hovered: Option<HoveredPaneRow>) {
        self.hovered_pane_row = hovered;
    }

    /// Push the current `hovered_pane_row` into the underlying pane
    /// manager. Clears any prior hover first, then sets the row on the
    /// pane indicated by `hovered_pane_row` (if any).
    pub(in crate::tui) fn apply_hovered_pane_row(&mut self) {
        self.pane_manager.clear_hover();
        let Some(hovered) = self.hovered_pane_row else {
            return;
        };
        self.pane_manager
            .pane_mut(hovered.pane)
            .set_hovered(Some(hovered.row));
    }

    pub(in crate::tui) fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci_display_modes.get(path).copied().unwrap_or_default()
    }

    pub(in crate::tui) fn set_ci_display_mode(
        &mut self,
        path: AbsolutePath,
        mode: CiRunDisplayMode,
    ) {
        self.ci_display_modes.insert(path, mode);
    }

    pub(in crate::tui) fn remove_ci_display_mode(&mut self, path: &Path) {
        self.ci_display_modes.remove(path);
    }

    pub(in crate::tui) fn clear_ci_display_modes(&mut self) { self.ci_display_modes.clear(); }

    /// Return the cached worktree-summary for `group_root` if present;
    /// otherwise compute via `compute` (the shell-out path), cache, and
    /// return. Cache is sticky — only `clear_for_tree_change`
    /// invalidates it, called from tree-rebuild paths.
    pub(in crate::tui) fn worktree_summary_or_compute(
        &self,
        group_root: &Path,
        compute: impl FnOnce() -> Vec<WorktreeInfo>,
    ) -> Vec<WorktreeInfo> {
        if let Some(infos) = self.worktree_summary_cache.borrow().get(group_root) {
            return infos.clone();
        }
        let infos = compute();
        self.worktree_summary_cache
            .borrow_mut()
            .insert(AbsolutePath::from(group_root), infos.clone());
        infos
    }

    /// Drop tree-derived caches owned by `Panes`. Called by
    /// `TreeMutation::drop` (Phase 1: invoked from the existing guard
    /// in `tui::app::mod.rs`; Phase 6 will re-wire the new fan-out
    /// guard to call this directly). Currently clears
    /// `worktree_summary_cache`; future tree-shape-dependent caches
    /// owned by `Panes` add their clear here.
    ///
    /// Takes `&mut self` even though the cache lives behind a
    /// `RefCell` — Phase 6 will add caches that genuinely need `&mut
    /// self`, and pinning the receiver type now keeps the public
    /// signature stable across that change.
    #[allow(
        clippy::needless_pass_by_ref_mut,
        reason = "Phase 6 will add &mut-only invalidations to this method"
    )]
    pub(in crate::tui) fn clear_for_tree_change(&mut self) {
        self.worktree_summary_cache.borrow_mut().clear();
    }

    /// Tick the CPU poller. If a fresh snapshot is produced, hand it
    /// to `pane_data` so the CPU pane redraws with current values.
    pub(in crate::tui) fn cpu_tick(&mut self, now: Instant) {
        if let Some(snapshot) = self.cpu_poller.poll_if_due(now) {
            self.pane_data.set_cpu(snapshot);
        }
    }

    /// Recreate the CPU poller for `cfg` and seed `pane_data` with a
    /// placeholder snapshot. Used at startup and after a config reload
    /// changes CPU poll behavior.
    pub(in crate::tui) fn reset_cpu(&mut self, cfg: &CpuConfig) {
        self.cpu_poller = CpuPoller::new(cfg);
        let placeholder = self.cpu_poller.placeholder_snapshot();
        self.pane_data.set_cpu(placeholder);
    }

    /// Seed `pane_data` with the current poller's placeholder snapshot
    /// without recreating the poller. Used from `App::finish_new`.
    pub(in crate::tui) fn install_cpu_placeholder(&mut self) {
        let placeholder = self.cpu_poller.placeholder_snapshot();
        self.pane_data.set_cpu(placeholder);
    }
}
