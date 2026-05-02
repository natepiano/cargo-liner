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

use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::data::PaneDataStore;
use super::dispatch::HITTABLE_Z_ORDER;
use super::dispatch::Hittable;
use super::dispatch::HittableId;
use super::dispatch::HoverTarget;
use super::dispatch::Pane;
use super::dispatch::PaneRenderCtx;
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
use crate::tui::config_state::Config;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::Viewport;
use crate::tui::scan_state::Scan;

/// Bundle of refs the dispatchers need to construct a
/// `PaneRenderCtx`. Constructed at the call site from
/// `App::split_panes_for_render` and the pane-specific focus
/// args, then handed to the `dispatch_*_render` method.
pub struct DispatchArgs<'a> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     std::time::Duration,
    pub config:                &'a Config,
    pub scan:                  &'a Scan,
    pub selected_project_path: Option<&'a Path>,
}

const fn build_ctx<'a>(args: &DispatchArgs<'a>) -> PaneRenderCtx<'a> {
    PaneRenderCtx {
        focus_state:           args.focus_state,
        is_focused:            args.is_focused,
        animation_elapsed:     args.animation_elapsed,
        config:                args.config,
        scan:                  args.scan,
        selected_project_path: args.selected_project_path,
    }
}

/// Owns every pane-related piece of state. App holds a single `panes:
/// Panes` field instead of the eight raw fields it carried before
/// Phase 1.
pub struct Panes {
    // ── Per-pane state (Phase 8 migrated). Phase 9 brings the
    //    remaining seven panes in alongside their state.
    package:      PackagePane,
    lang:         LangPane,
    cpu:          CpuPane,
    git:          GitPane,
    lints:        LintsPane,
    ci_runs:      CiPane,
    toasts:       ToastsPane,
    keymap:       KeymapPane,
    settings:     SettingsPane,
    finder:       FinderPane,
    output:       OutputPane,
    targets:      TargetsPane,
    project_list: ProjectListPane,

    // ── Phase 1 grab-bag (residual after Phase 10.2):
    data:        PaneDataStore,
    visited:     HashSet<PaneId>,
    hovered_row: Option<HoveredPaneRow>,
    // `layout_cache` was here; absorbed onto App-shell in Phase 10.2.
    // `worktree_summary_cache` was here; absorbed onto `GitPane` in Phase 10.1.
    // `pane_manager` was here; deleted in Phase 9.8.
    // `ci_display_modes` was here; absorbed onto `CiPane` in Phase 8.7.
    // `cpu_poller` was here; absorbed onto `CpuPane` in Phase 8.1a.
}

impl Panes {
    pub fn new(cpu_cfg: &CpuConfig) -> Self {
        Self {
            package:      PackagePane::new(),
            lang:         LangPane::new(),
            cpu:          CpuPane::new(cpu_cfg),
            git:          GitPane::new(),
            lints:        LintsPane::new(),
            ci_runs:      CiPane::new(),
            toasts:       ToastsPane::new(),
            keymap:       KeymapPane::new(),
            settings:     SettingsPane::new(),
            finder:       FinderPane::new(),
            output:       OutputPane::new(),
            targets:      TargetsPane::new(),
            project_list: ProjectListPane::new(),

            data:        PaneDataStore::new(),
            visited:     std::iter::once(PaneId::ProjectList).collect(),
            hovered_row: None,
        }
    }

    /// Typed accessor for the CPU pane. Used by callers that
    /// need to read CPU-pane state (content snapshot, etc.) —
    /// e.g., the render path and `is_pane_tabbable`.
    pub const fn cpu(&self) -> &CpuPane { &self.cpu }

    /// Mutable typed accessor for the CPU pane.
    pub const fn cpu_mut(&mut self) -> &mut CpuPane { &mut self.cpu }

    /// Mutable typed accessor for the Lang pane.
    pub const fn lang_mut(&mut self) -> &mut LangPane { &mut self.lang }

    /// Typed accessor for the Lints pane.
    pub const fn lints(&self) -> &LintsPane { &self.lints }

    /// Mutable typed accessor for the Lints pane.
    pub const fn lints_mut(&mut self) -> &mut LintsPane { &mut self.lints }

    /// Typed accessor for the `CiRuns` pane.
    pub const fn ci(&self) -> &CiPane { &self.ci_runs }

    /// Mutable typed accessor for the `CiRuns` pane.
    pub const fn ci_mut(&mut self) -> &mut CiPane { &mut self.ci_runs }

    /// Typed accessor for the Package pane.
    pub const fn package(&self) -> &PackagePane { &self.package }

    /// Mutable typed accessor for the Package pane.
    pub const fn package_mut(&mut self) -> &mut PackagePane { &mut self.package }

    /// Typed accessor for the Git pane.
    pub const fn git(&self) -> &GitPane { &self.git }

    /// Mutable typed accessor for the Git pane.
    pub const fn git_mut(&mut self) -> &mut GitPane { &mut self.git }

    /// Typed accessor for the Toasts pane.
    pub const fn toasts(&self) -> &ToastsPane { &self.toasts }

    /// Mutable typed accessor for the Toasts pane.
    pub const fn toasts_mut(&mut self) -> &mut ToastsPane { &mut self.toasts }

    /// Typed accessor for the Keymap pane.
    pub const fn keymap(&self) -> &KeymapPane { &self.keymap }

    /// Mutable typed accessor for the Keymap pane.
    pub const fn keymap_mut(&mut self) -> &mut KeymapPane { &mut self.keymap }

    /// Typed accessor for the Settings pane.
    pub const fn settings(&self) -> &SettingsPane { &self.settings }

    /// Mutable typed accessor for the Settings pane.
    pub const fn settings_mut(&mut self) -> &mut SettingsPane { &mut self.settings }

    /// Typed accessor for the Finder pane.
    pub const fn finder(&self) -> &FinderPane { &self.finder }

    /// Mutable typed accessor for the Finder pane.
    pub const fn finder_mut(&mut self) -> &mut FinderPane { &mut self.finder }

    /// Typed accessor for the Targets pane.
    pub const fn targets(&self) -> &TargetsPane { &self.targets }

    /// Mutable typed accessor for the Targets pane.
    pub const fn targets_mut(&mut self) -> &mut TargetsPane { &mut self.targets }

    /// Typed accessor for the `ProjectList` pane.
    pub const fn project_list(&self) -> &ProjectListPane { &self.project_list }

    /// Mutable typed accessor for the `ProjectList` pane.
    pub const fn project_list_mut(&mut self) -> &mut ProjectListPane { &mut self.project_list }

    /// Write the detail-set content across the four migrated detail
    /// panes (Package/Git/CI/Lints) plus the targets slot in
    /// `PaneDataStore`, and update the detail stamp. The "all five
    /// panes coherent for this stamp" invariant is preserved by this
    /// orchestrator: callers cannot write one detail member without
    /// writing the others.
    pub fn set_detail_data(
        &mut self,
        stamp: super::data::DetailCacheKey,
        package: super::PackageData,
        git: super::GitData,
        targets: super::TargetsData,
        ci: super::CiData,
        lints: super::LintsData,
    ) {
        self.package.set_content(package);
        self.git.set_content(git);
        self.ci_runs.set_content(ci);
        self.lints.set_content(lints);
        self.targets.set_content(targets);
        self.data.set_detail_stamp(Some(stamp));
    }

    /// Clear the detail set across the five migrated detail panes,
    /// stamping with `stamp`. Mirrors `set_detail_data`'s fan-out.
    pub fn clear_detail_data(&mut self, stamp: Option<super::data::DetailCacheKey>) {
        self.package.clear_content();
        self.git.clear_content();
        self.ci_runs.clear_content();
        self.lints.clear_content();
        self.targets.clear_content();
        self.data.set_detail_stamp(stamp);
    }

    /// Test-only override for lints content. Mirrors the previous
    /// `PaneDataStore::override_lints_for_test`.
    #[cfg(test)]
    pub fn override_lints_for_test(&mut self, data: super::LintsData) {
        self.lints.set_content(data);
    }

    /// Test-only override for ci runs. Mirrors the previous
    /// `PaneDataStore::override_ci_runs_for_test`.
    #[cfg(test)]
    pub fn override_ci_runs_for_test(&mut self, runs: Vec<CiRun>) {
        self.ci_runs.override_runs_for_test(runs);
    }

    /// Dispatch `CiPane`'s render through the `Pane` trait.
    pub fn dispatch_ci_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.ci_runs, frame, area, ctx);
    }

    /// Dispatch `LintsPane`'s render through the `Pane` trait.
    pub fn dispatch_lints_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.lints, frame, area, ctx);
    }

    /// Dispatch `CpuPane`'s render through the `Pane` trait.
    pub fn dispatch_cpu_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.cpu, frame, area, ctx);
    }

    /// Dispatch `LangPane`'s render through the `Pane` trait.
    pub fn dispatch_lang_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.lang, frame, area, ctx);
    }

    /// Dispatch `PackagePane`'s render through the `Pane` trait.
    pub fn dispatch_package_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.package, frame, area, ctx);
    }

    /// Dispatch `GitPane`'s render through the `Pane` trait.
    pub fn dispatch_git_render(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        args: &DispatchArgs<'_>,
    ) {
        let ctx = build_ctx(args);
        let ctx = &ctx;
        Pane::render(&mut self.git, frame, area, ctx);
    }

    /// Set the cursor position for `id`'s viewport, routing to
    /// each migrated pane's per-pane `Viewport` for those that
    /// have absorbed it, falling back to the still-vestigial
    /// `PaneManager` slot for un-migrated panes. Used by the
    /// generic click handler in `interaction.rs` and any other
    /// code that needs to set a cursor position by `PaneId`.
    /// Each Phase-8 / Phase-9 sub-commit moves one pane out of
    /// the fallback arm.
    pub const fn set_pane_pos(&mut self, id: PaneId, row: usize) {
        match id {
            PaneId::Cpu => self.cpu.viewport_mut().set_pos(row),
            PaneId::Lang => self.lang.viewport_mut().set_pos(row),
            PaneId::Lints => self.lints.viewport_mut().set_pos(row),
            PaneId::CiRuns => self.ci_runs.viewport_mut().set_pos(row),
            PaneId::Package => self.package.viewport_mut().set_pos(row),
            PaneId::Git => self.git.viewport_mut().set_pos(row),
            PaneId::Toasts => self.toasts.viewport_mut().set_pos(row),
            PaneId::Keymap => self.keymap.viewport_mut().set_pos(row),
            PaneId::Settings => self.settings.viewport_mut().set_pos(row),
            PaneId::Finder => self.finder.viewport_mut().set_pos(row),
            PaneId::Output => self.output.viewport_mut().set_pos(row),
            PaneId::Targets => self.targets.viewport_mut().set_pos(row),
            PaneId::ProjectList => self.project_list.viewport_mut().set_pos(row),
        }
    }

    pub const fn pane_data(&self) -> &PaneDataStore { &self.data }

    pub fn mark_visited(&mut self, pane: PaneId) { self.visited.insert(pane); }

    pub fn unvisit(&mut self, pane: PaneId) { self.visited.remove(&pane); }

    pub fn remembers_visited(&self, pane: PaneId) -> bool { self.visited.contains(&pane) }

    pub const fn set_hover(&mut self, hovered: Option<HoveredPaneRow>) {
        self.hovered_row = hovered;
    }

    /// Push the current `hovered_pane_row` into the per-pane viewports.
    /// Clears any prior hover across every pane first, then sets the row
    /// on the pane indicated by `hovered_pane_row` (if any). After Phase
    /// 9.8 every pane owns its own `Viewport`, so the clear is a flat
    /// fan-out across all 13 per-pane structs.
    pub const fn apply_hovered_pane_row(&mut self) {
        self.package.viewport_mut().set_hovered(None);
        self.lang.viewport_mut().set_hovered(None);
        self.cpu.viewport_mut().set_hovered(None);
        self.git.viewport_mut().set_hovered(None);
        self.lints.viewport_mut().set_hovered(None);
        self.ci_runs.viewport_mut().set_hovered(None);
        self.toasts.viewport_mut().set_hovered(None);
        self.keymap.viewport_mut().set_hovered(None);
        self.settings.viewport_mut().set_hovered(None);
        self.finder.viewport_mut().set_hovered(None);
        self.output.viewport_mut().set_hovered(None);
        self.targets.viewport_mut().set_hovered(None);
        self.project_list.viewport_mut().set_hovered(None);
        let Some(hovered) = self.hovered_row else {
            return;
        };
        self.viewport_mut_for(hovered.pane)
            .set_hovered(Some(hovered.row));
    }

    /// Mutable counterpart to `viewport_for`. Routes to the per-pane
    /// `Viewport` for migrated panes, falls back to the vestigial
    /// `PaneManager` slot for un-migrated panes (none remain after
    /// Phase 9.7a, but the match still covers the full `PaneId` set).
    pub const fn viewport_mut_for(&mut self, id: PaneId) -> &mut Viewport {
        match id {
            PaneId::Cpu => self.cpu.viewport_mut(),
            PaneId::Lang => self.lang.viewport_mut(),
            PaneId::Lints => self.lints.viewport_mut(),
            PaneId::CiRuns => self.ci_runs.viewport_mut(),
            PaneId::Package => self.package.viewport_mut(),
            PaneId::Git => self.git.viewport_mut(),
            PaneId::Toasts => self.toasts.viewport_mut(),
            PaneId::Keymap => self.keymap.viewport_mut(),
            PaneId::Settings => self.settings.viewport_mut(),
            PaneId::Finder => self.finder.viewport_mut(),
            PaneId::Output => self.output.viewport_mut(),
            PaneId::Targets => self.targets.viewport_mut(),
            PaneId::ProjectList => self.project_list.viewport_mut(),
        }
    }

    pub fn ci_display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.ci_runs.display_mode_for(path)
    }

    pub fn set_ci_display_mode(&mut self, path: AbsolutePath, mode: CiRunDisplayMode) {
        self.ci_runs.set_display_mode(path, mode);
    }

    pub fn remove_ci_display_mode(&mut self, path: &Path) {
        self.ci_runs.remove_display_mode(path);
    }

    pub fn clear_ci_display_modes(&mut self) { self.ci_runs.clear_display_modes(); }

    /// Return the cached worktree-summary for `group_root` if present;
    /// otherwise compute via `compute` (the shell-out path), cache, and
    /// return. Cache lives on `GitPane` (Phase 10.1) — Panes is now a
    /// pass-through. Sticky cache; only `clear_for_tree_change`
    /// invalidates it.
    pub fn worktree_summary_or_compute(
        &self,
        group_root: &Path,
        compute: impl FnOnce() -> Vec<WorktreeInfo>,
    ) -> Vec<WorktreeInfo> {
        self.git.worktree_summary_or_compute(group_root, compute)
    }

    /// Drop tree-derived caches owned by per-pane structs. After
    /// Phase 10.1 the only such cache is `GitPane`'s
    /// worktree-summary map.
    pub fn clear_for_tree_change(&self) { self.git.clear_worktree_summary_cache(); }

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

    /// Walk `HITTABLE_Z_ORDER` top-to-bottom and return the first
    /// pane's `hit_test_at` answer. Phase 10.3 dispatch entry
    /// point: replaces the global `ui_hitboxes` vec walk.
    pub fn hit_test_at(&self, pos: ratatui::layout::Position) -> Option<HoverTarget> {
        for id in HITTABLE_Z_ORDER {
            let pane: &dyn Hittable = match id {
                HittableId::Toasts => &self.toasts,
                HittableId::Finder => &self.finder,
                HittableId::Settings => &self.settings,
                HittableId::Keymap => &self.keymap,
                HittableId::ProjectList => &self.project_list,
                HittableId::Package => &self.package,
                HittableId::Lang => &self.lang,
                HittableId::Cpu => &self.cpu,
                HittableId::Git => &self.git,
                HittableId::Targets => &self.targets,
                HittableId::Lints => &self.lints,
                HittableId::CiRuns => &self.ci_runs,
            };
            if let Some(hit) = pane.hit_test_at(pos) {
                return Some(hit);
            }
        }
        None
    }
}

#[cfg(test)]
mod detail_set_tests {
    //! Pin the detail-set "all five panes coherent for this stamp"
    //! invariant on the new `Panes` orchestrators. Phase 8.8 moved
    //! the invariant out of `PaneDataStore` (which only tracks
    //! targets + stamp now) into `Panes::set_detail_data` /
    //! `Panes::clear_detail_data`, which fan out across the four
    //! migrated detail panes' content slots plus the targets slot.
    use crate::config::CpuConfig;
    use crate::tui::app::VisibleRow;
    use crate::tui::panes::CiData;
    use crate::tui::panes::CiEmptyState;
    use crate::tui::panes::GitData;
    use crate::tui::panes::LintsData;
    use crate::tui::panes::PackageData;
    use crate::tui::panes::TargetsData;
    use crate::tui::panes::data::DetailCacheKey;
    use super::Panes;

    fn fresh() -> Panes { Panes::new(&CpuConfig::default()) }

    fn any_row() -> VisibleRow { VisibleRow::Root { node_index: 0 } }

    fn other_row() -> VisibleRow {
        VisibleRow::Member {
            node_index:   0,
            group_index:  0,
            member_index: 0,
        }
    }

    fn empty_detail() -> (PackageData, GitData, TargetsData, CiData, LintsData) {
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
    fn new_panes_detail_is_current_only_with_no_selection() {
        let panes = fresh();
        assert!(panes.pane_data().detail_is_current(None));
        assert!(!panes.pane_data().detail_is_current(Some(DetailCacheKey {
            row:        any_row(),
            generation: 0,
        })));
    }

    #[test]
    fn set_detail_data_writes_all_panes_and_stamps() {
        let mut panes = fresh();
        let key = DetailCacheKey {
            row:        any_row(),
            generation: 3,
        };
        let (pkg, git, targets, ci, lints) = empty_detail();
        panes.set_detail_data(key, pkg, git, targets, ci, lints);

        assert!(panes.pane_data().detail_is_current(Some(key)));
        assert!(panes.package().content().is_some());
        assert!(panes.git().content().is_some());
        assert!(panes.ci().content().is_some());
        assert!(panes.lints().content().is_some());
        assert!(panes.targets().content().is_some());

        // Different stamps don't match.
        assert!(!panes.pane_data().detail_is_current(None));
        assert!(!panes.pane_data().detail_is_current(Some(DetailCacheKey {
            row:        any_row(),
            generation: 4,
        })));
        assert!(!panes.pane_data().detail_is_current(Some(DetailCacheKey {
            row:        other_row(),
            generation: 3,
        })));
    }

    #[test]
    fn clear_detail_data_clears_all_panes_and_records_stamp() {
        let mut panes = fresh();
        let key = DetailCacheKey {
            row:        any_row(),
            generation: 7,
        };
        let (pkg, git, targets, ci, lints) = empty_detail();
        panes.set_detail_data(key, pkg, git, targets, ci, lints);

        let clear_key = DetailCacheKey {
            row:        other_row(),
            generation: 7,
        };
        panes.clear_detail_data(Some(clear_key));
        assert!(panes.pane_data().detail_is_current(Some(clear_key)));
        assert!(panes.package().content().is_none());
        assert!(panes.git().content().is_none());
        assert!(panes.ci().content().is_none());
        assert!(panes.lints().content().is_none());
        assert!(panes.targets().content().is_none());
    }

    #[test]
    fn clear_detail_with_none_matches_none() {
        let mut panes = fresh();
        panes.clear_detail_data(None);
        assert!(panes.pane_data().detail_is_current(None));
    }
}
