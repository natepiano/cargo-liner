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
use super::dispatch::Pane;
use super::layout::LayoutCache;
use super::pane_impls::CiPane;
use super::pane_impls::CpuPane;
use super::pane_impls::FinderPane;
use super::pane_impls::GitPane;
use super::pane_impls::KeymapPane;
use super::pane_impls::LangPane;
use super::pane_impls::LintsPane;
use super::pane_impls::OutputPane;
use super::pane_impls::PackagePane;
use super::pane_impls::ProjectListPane;
use super::pane_impls::SettingsPane;
use super::pane_impls::TargetsPane;
use super::pane_impls::ToastsPane;
use super::spec::PaneId;
use super::support::WorktreeInfo;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::app::HoveredPaneRow;
use crate::tui::pane::PaneManager;

/// Owns every pane-related piece of state. App holds a single `panes:
/// Panes` field instead of the eight raw fields it carried before
/// Phase 1.
pub struct Panes {
    // ── Phase 7: per-pane registry (unit structs today; absorb
    //    state in Phases 8–9). Methods `pane(id)` and
    //    `pane_mut(id)` dispatch through the trait via these
    //    fields.
    project_list: ProjectListPane,
    package:      PackagePane,
    lang:         LangPane,
    cpu:          CpuPane,
    git:          GitPane,
    targets:      TargetsPane,
    lints:        LintsPane,
    ci_runs:      CiPane,
    output:       OutputPane,
    toasts:       ToastsPane,
    settings:     SettingsPane,
    finder:       FinderPane,
    keymap:       KeymapPane,

    // ── Phase 1 grab-bag (dissolves in Phases 8–10):
    manager:                PaneManager,
    data:                   PaneDataStore,
    visited:                HashSet<PaneId>,
    layout_cache:           LayoutCache,
    /// See `tui::app::mod.rs` doc comment on the original field —
    /// `RefCell` because `worktree_summary_or_compute` runs inside
    /// `build_pane_data_common`, which only has `&App`.
    worktree_summary_cache: RefCell<HashMap<AbsolutePath, Vec<WorktreeInfo>>>,
    hovered_row:            Option<HoveredPaneRow>,
    ci_display_modes:       HashMap<AbsolutePath, CiRunDisplayMode>,
    // `cpu_poller` was here; absorbed onto `CpuPane` in Phase 8.1a.
}

impl Panes {
    pub fn new(cpu_cfg: &CpuConfig) -> Self {
        Self {
            project_list: ProjectListPane,
            package:      PackagePane,
            lang:         LangPane::new(),
            cpu:          CpuPane::new(cpu_cfg),
            git:          GitPane,
            targets:      TargetsPane,
            lints:        LintsPane,
            ci_runs:      CiPane,
            output:       OutputPane,
            toasts:       ToastsPane,
            settings:     SettingsPane,
            finder:       FinderPane,
            keymap:       KeymapPane,

            manager:                PaneManager::new(),
            data:                   PaneDataStore::new(),
            visited:                std::iter::once(PaneId::ProjectList).collect(),
            layout_cache:           LayoutCache::default(),
            worktree_summary_cache: RefCell::new(HashMap::new()),
            hovered_row:            None,
            ci_display_modes:       HashMap::new(),
        }
    }

    /// Typed accessor for the CPU pane. Used by callers that
    /// need to read CPU-pane state (content snapshot, etc.) —
    /// e.g., the render path and `is_pane_tabbable`.
    pub const fn cpu(&self) -> &CpuPane { &self.cpu }

    /// Mutable typed accessor for the CPU pane.
    pub const fn cpu_mut(&mut self) -> &mut CpuPane { &mut self.cpu }

    /// Typed accessor for the Lang pane.
    pub const fn lang(&self) -> &LangPane { &self.lang }

    /// Mutable typed accessor for the Lang pane.
    pub const fn lang_mut(&mut self) -> &mut LangPane { &mut self.lang }

    /// Trait-dispatch entry: returns the per-pane struct for
    /// `id` as `&dyn Pane`. Phase 7 supports only the
    /// `PaneId`-pure trait methods (`id`, `has_row_hitboxes`,
    /// `size_spec`, `input_context`); Phase 8/9 fill in render,
    /// input, and viewport accessors as bodies migrate.
    #[allow(
        dead_code,
        reason = "Phase 7 registry entry; first dispatch site wires up in Phase 8"
    )]
    pub fn pane(&self, id: PaneId) -> &dyn Pane {
        match id {
            PaneId::ProjectList => &self.project_list,
            PaneId::Package => &self.package,
            PaneId::Lang => &self.lang,
            PaneId::Cpu => &self.cpu,
            PaneId::Git => &self.git,
            PaneId::Targets => &self.targets,
            PaneId::Lints => &self.lints,
            PaneId::CiRuns => &self.ci_runs,
            PaneId::Output => &self.output,
            PaneId::Toasts => &self.toasts,
            PaneId::Settings => &self.settings,
            PaneId::Finder => &self.finder,
            PaneId::Keymap => &self.keymap,
        }
    }

    /// Mutable trait-dispatch entry. See `pane`.
    #[allow(
        dead_code,
        reason = "Phase 7 registry entry; first dispatch site wires up in Phase 8"
    )]
    pub fn pane_mut(&mut self, id: PaneId) -> &mut dyn Pane {
        match id {
            PaneId::ProjectList => &mut self.project_list,
            PaneId::Package => &mut self.package,
            PaneId::Lang => &mut self.lang,
            PaneId::Cpu => &mut self.cpu,
            PaneId::Git => &mut self.git,
            PaneId::Targets => &mut self.targets,
            PaneId::Lints => &mut self.lints,
            PaneId::CiRuns => &mut self.ci_runs,
            PaneId::Output => &mut self.output,
            PaneId::Toasts => &mut self.toasts,
            PaneId::Settings => &mut self.settings,
            PaneId::Finder => &mut self.finder,
            PaneId::Keymap => &mut self.keymap,
        }
    }

    pub const fn pane_manager(&self) -> &PaneManager { &self.manager }

    pub const fn pane_manager_mut(&mut self) -> &mut PaneManager { &mut self.manager }

    pub const fn pane_data(&self) -> &PaneDataStore { &self.data }

    pub const fn pane_data_mut(&mut self) -> &mut PaneDataStore { &mut self.data }

    pub const fn layout_cache(&self) -> &LayoutCache { &self.layout_cache }

    pub const fn layout_cache_mut(&mut self) -> &mut LayoutCache { &mut self.layout_cache }

    pub fn mark_visited(&mut self, pane: PaneId) { self.visited.insert(pane); }

    pub fn unvisit(&mut self, pane: PaneId) { self.visited.remove(&pane); }

    pub fn remembers_visited(&self, pane: PaneId) -> bool { self.visited.contains(&pane) }

    pub const fn set_hover(&mut self, hovered: Option<HoveredPaneRow>) {
        self.hovered_row = hovered;
    }

    /// Push the current `hovered_pane_row` into the underlying pane
    /// manager. Clears any prior hover first, then sets the row on the
    /// pane indicated by `hovered_pane_row` (if any).
    pub fn apply_hovered_pane_row(&mut self) {
        self.manager.clear_hover();
        let Some(hovered) = self.hovered_row else {
            return;
        };
        self.manager
            .pane_mut(hovered.pane)
            .set_hovered(Some(hovered.row));
    }

    pub fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci_display_modes.get(path).copied().unwrap_or_default()
    }

    pub fn set_ci_display_mode(&mut self, path: AbsolutePath, mode: CiRunDisplayMode) {
        self.ci_display_modes.insert(path, mode);
    }

    pub fn remove_ci_display_mode(&mut self, path: &Path) { self.ci_display_modes.remove(path); }

    pub fn clear_ci_display_modes(&mut self) { self.ci_display_modes.clear(); }

    /// Return the cached worktree-summary for `group_root` if present;
    /// otherwise compute via `compute` (the shell-out path), cache, and
    /// return. Cache is sticky — only `clear_for_tree_change`
    /// invalidates it, called from tree-rebuild paths.
    pub fn worktree_summary_or_compute(
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
    /// Takes `&self` because the only cache cleared today lives
    /// behind a `RefCell`. Phase 6 may widen to `&mut self` if it
    /// adds caches that need exclusive access.
    pub fn clear_for_tree_change(&self) { self.worktree_summary_cache.borrow_mut().clear(); }

    /// Tick the CPU pane's poller. Delegates to `CpuPane::tick`
    /// after Phase 8.1a moved the poller and content slot onto the
    /// pane.
    pub fn cpu_tick(&mut self, now: Instant) { self.cpu.tick(now); }

    /// Reset the CPU pane after a config reload changes CPU poll
    /// behavior. Delegates to `CpuPane::reset`.
    pub fn reset_cpu(&mut self, cfg: &CpuConfig) { self.cpu.reset(cfg); }

    /// Seed the CPU pane's content with the current poller's
    /// placeholder snapshot. Delegates to
    /// `CpuPane::install_placeholder`. Used from `App::finish_new`.
    pub fn install_cpu_placeholder(&mut self) { self.cpu.install_placeholder(); }
}

#[cfg(test)]
mod registry_tests {
    //! Verify the registry returns the right concrete pane for every
    //! `PaneId` variant. Pinned now so Phase 8/9 migrations cannot
    //! silently mis-wire dispatch.
    use crate::config::CpuConfig;
    use crate::tui::panes::PaneId;
    use crate::tui::panes::system::Panes;

    fn fresh() -> Panes { Panes::new(&CpuConfig::default()) }

    fn all_ids() -> [PaneId; 13] {
        [
            PaneId::ProjectList,
            PaneId::Package,
            PaneId::Lang,
            PaneId::Cpu,
            PaneId::Git,
            PaneId::Targets,
            PaneId::Lints,
            PaneId::CiRuns,
            PaneId::Output,
            PaneId::Toasts,
            PaneId::Settings,
            PaneId::Finder,
            PaneId::Keymap,
        ]
    }

    #[test]
    fn pane_returns_matching_id() {
        let panes = fresh();
        for id in all_ids() {
            assert_eq!(panes.pane(id).id(), id, "{id:?}");
        }
    }

    #[test]
    fn pane_mut_returns_matching_id() {
        let mut panes = fresh();
        for id in all_ids() {
            assert_eq!(panes.pane_mut(id).id(), id, "{id:?}");
        }
    }
}
