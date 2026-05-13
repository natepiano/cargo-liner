//! The `Panes` subsystem.
//!
//! Owns the pane-related state cluster (`pane_data`, `visited_panes`,
//! `hovered_pane_row`, plus the per-pane structs in `pane_impls`).
//! Exposes a facade so App's impl-files and the `panes/` siblings
//! don't reach into App's private guts directly.
//!
//! `handle_input`-style methods that need cross-subsystem access
//! remain free functions taking `&mut App`.

use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Rect;

use super::data::PaneDataStore;
use super::pane_impls::CpuPane;
use super::pane_impls::GitPane;
use super::pane_impls::LangPane;
use super::pane_impls::OutputPane;
use super::pane_impls::PackagePane;
use super::pane_impls::ProjectListPane;
use super::pane_impls::TargetsPane;
use crate::config::CpuConfig;
use crate::tui::app::HoveredPaneRow;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneFocusState;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::project_list::ProjectList;
use crate::tui::state::Config;

/// Bundle of refs the dispatchers need to construct a
/// `PaneRenderCtx`. Constructed at the call site from
/// `App::split_panes_for_render` and the pane-specific focus
/// args, then handed to the `dispatch_*_render` method.
pub struct DispatchArgs<'a> {
    pub focus_state:           PaneFocusState,
    pub is_focused:            bool,
    pub animation_elapsed:     Duration,
    pub config:                &'a Config,
    pub project_list:          &'a ProjectList,
    pub selected_project_path: Option<&'a Path>,
}

const fn build_ctx<'a>(args: &DispatchArgs<'a>) -> PaneRenderCtx<'a> {
    PaneRenderCtx {
        focus_state:           args.focus_state,
        is_focused:            args.is_focused,
        animation_elapsed:     args.animation_elapsed,
        config:                args.config,
        project_list:          args.project_list,
        selected_project_path: args.selected_project_path,
    }
}

/// Owns every pane-related piece of state. App holds a single `panes:
/// Panes` field.
pub struct Panes {
    pub package:      PackagePane,
    pub lang:         LangPane,
    pub cpu:          CpuPane,
    pub git:          GitPane,
    pub output:       OutputPane,
    pub targets:      TargetsPane,
    pub project_list: ProjectListPane,

    pub pane_data: PaneDataStore,
    hovered_row:   Option<HoveredPaneRow>,
}

impl Panes {
    pub fn new(cpu_cfg: &CpuConfig) -> Self {
        Self {
            package:      PackagePane::new(),
            lang:         LangPane::new(),
            cpu:          CpuPane::new(cpu_cfg),
            git:          GitPane::new(),
            output:       OutputPane::new(),
            targets:      TargetsPane::new(),
            project_list: ProjectListPane::new(),

            pane_data:   PaneDataStore::new(),
            hovered_row: None,
        }
    }

    /// Currently-hovered pane/row pair, or `None`. Used by the
    /// App-level `apply_hovered_pane_row` orchestrator.
    pub const fn hovered_row(&self) -> Option<HoveredPaneRow> { self.hovered_row }

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
    ) {
        self.package.set_content(package);
        self.git.set_content(git);
        self.targets.set_content(targets);
        self.pane_data.set_detail_stamp(Some(stamp));
    }

    /// Clear the detail set across the migrated detail panes owned by `Panes`,
    /// stamping with `stamp`. Mirrors `set_detail_data`'s fan-out. CI and lint
    /// content live on their own subsystems and are cleared by the caller.
    pub fn clear_detail_data(&mut self, stamp: Option<super::data::DetailCacheKey>) {
        self.package.clear_content();
        self.git.clear_content();
        self.targets.clear_content();
        self.pane_data.set_detail_stamp(stamp);
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

    pub const fn set_hover(&mut self, hovered: Option<HoveredPaneRow>) {
        self.hovered_row = hovered;
    }

    /// Drop tree-derived caches owned by per-pane structs.
    /// Currently only `GitPane`'s worktree-summary map.
    pub fn clear_for_tree_change(&self) { self.git.clear_worktree_summary_cache(); }

    /// Tick the CPU pane's poller. Delegates to `CpuPane::tick`.
    pub fn cpu_tick(&mut self, now: Instant) { self.cpu.tick(now); }

    /// Reset the CPU pane after a config reload changes CPU poll
    /// behavior. Delegates to `CpuPane::reset`.
    pub fn reset_cpu(&mut self, cfg: &CpuConfig) { self.cpu.reset(cfg); }

    /// Seed the CPU pane's content with the current poller's
    /// placeholder `CpuUsage`. Delegates to
    /// `CpuPane::install_placeholder`. Used from `App::finish_new`.
    pub fn install_cpu_placeholder(&mut self) { self.cpu.install_placeholder(); }
}

#[cfg(test)]
mod detail_set_tests {
    //! Pin the detail-set "all five panes coherent for this stamp"
    //! invariant on `Panes::set_detail_data` /
    //! `Panes::clear_detail_data`, which fan out across the four
    //! detail panes' content slots plus the targets slot.
    //! `PaneDataStore` itself only tracks the stamp.
    use super::Panes;
    use crate::config::CpuConfig;
    use crate::tui::app::VisibleRow;
    use crate::tui::panes::GitData;
    use crate::tui::panes::PackageData;
    use crate::tui::panes::TargetsData;
    use crate::tui::panes::data::DetailCacheKey;

    fn fresh() -> Panes { Panes::new(&CpuConfig::default()) }

    fn any_row() -> VisibleRow { VisibleRow::Root { node_index: 0 } }

    fn other_row() -> VisibleRow {
        VisibleRow::Member {
            node_index:   0,
            group_index:  0,
            member_index: 0,
        }
    }

    fn empty_detail() -> (PackageData, GitData, TargetsData) {
        (
            PackageData::default(),
            GitData::default(),
            TargetsData::default(),
        )
    }

    #[test]
    fn new_panes_detail_is_current_only_with_no_selection() {
        let panes = fresh();
        assert!(panes.pane_data.detail_is_current(None));
        assert!(!panes.pane_data.detail_is_current(Some(DetailCacheKey {
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
        let (pkg, git, targets) = empty_detail();
        panes.set_detail_data(key, pkg, git, targets);

        assert!(panes.pane_data.detail_is_current(Some(key)));
        assert!(panes.package.content().is_some());
        assert!(panes.git.content().is_some());
        assert!(panes.targets.content().is_some());

        // Different stamps don't match.
        assert!(!panes.pane_data.detail_is_current(None));
        assert!(!panes.pane_data.detail_is_current(Some(DetailCacheKey {
            row:        any_row(),
            generation: 4,
        })));
        assert!(!panes.pane_data.detail_is_current(Some(DetailCacheKey {
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
        let (pkg, git, targets) = empty_detail();
        panes.set_detail_data(key, pkg, git, targets);

        let clear_key = DetailCacheKey {
            row:        other_row(),
            generation: 7,
        };
        panes.clear_detail_data(Some(clear_key));
        assert!(panes.pane_data.detail_is_current(Some(clear_key)));
        assert!(panes.package.content().is_none());
        assert!(panes.git.content().is_none());
        assert!(panes.targets.content().is_none());
    }

    #[test]
    fn clear_detail_with_none_matches_none() {
        let mut panes = fresh();
        panes.clear_detail_data(None);
        assert!(panes.pane_data.detail_is_current(None));
    }
}
