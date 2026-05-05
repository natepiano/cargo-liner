//! Per-pane unit structs, their `Pane` impls, and `Hittable` impls.
//!
//! `Hittable: Pane` is the sub-trait implemented by the eleven
//! clickable panes. Each pane records the hit-test layout it needs
//! during render (uniform-row panes lean on
//! `Viewport::content_area` + `scroll_offset`; non-uniform panes —
//! `Cpu`, `Git`, `ProjectList`, `Toasts` — store explicit per-row
//! rect lists). Click and hover dispatch walks `HITTABLE_Z_ORDER`,
//! asking each pane for the target at `pos`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;

use super::PaneId;
use super::ci;
use super::cpu;
use super::git;
use super::lang;
use super::lints;
use super::package;
use super::package::RenderStyles;
#[cfg(test)]
use crate::ci::CiRun;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::app::DismissTarget;
use crate::tui::cpu::CpuPoller;
use crate::tui::cpu::CpuUsage;
use crate::tui::pane;
use crate::tui::pane::Hittable;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::pane::Viewport;

// ── Package ─────────────────────────────────────────────────────
pub struct PackagePane {
    viewport: Viewport,
    content:  Option<super::PackageData>,
}

impl PackagePane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::PackageData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::PackageData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for PackagePane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        package::render_package_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable for PackagePane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Package,
            row,
        })
    }
}

// ── Lang ────────────────────────────────────────────────────────
pub struct LangPane {
    viewport: Viewport,
}

impl LangPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Pane for LangPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        lang::render_lang_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable for LangPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Lang,
            row,
        })
    }
}

// ── Cpu ─────────────────────────────────────────────────────────
pub struct CpuPane {
    viewport:  Viewport,
    content:   Option<CpuUsage>,
    poller:    CpuPoller,
    /// Per-rendered-row `(Rect, logical_row)` recorded each frame
    /// so `Hittable::hit_test_at` can map `pos` back to the logical
    /// row. CPU rows are non-uniform (aggregate, per-core,
    /// breakdown, GPU) so a flat `viewport.pos_to_local_row` won't
    /// work.
    row_rects: Vec<(Rect, usize)>,
}

impl CpuPane {
    pub fn new(cfg: &CpuConfig) -> Self {
        let mut pane = Self {
            viewport:  Viewport::new(),
            content:   None,
            poller:    CpuPoller::new(cfg),
            row_rects: Vec::new(),
        };
        pane.install_placeholder();
        pane
    }

    pub fn tick(&mut self, now: Instant) {
        if let Some(usage) = self.poller.poll_if_due(now) {
            self.content = Some(usage);
        }
    }

    pub fn reset(&mut self, cfg: &CpuConfig) {
        self.poller = CpuPoller::new(cfg);
        self.install_placeholder();
    }

    pub fn install_placeholder(&mut self) {
        self.content = Some(self.poller.placeholder_cpu_usage());
    }

    pub const fn content(&self) -> Option<&CpuUsage> { self.content.as_ref() }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Pane for CpuPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        cpu::render_cpu_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable for CpuPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Cpu,
            row,
        })
    }
}

// ── Git ─────────────────────────────────────────────────────────
pub struct GitPane {
    viewport:               Viewport,
    content:                Option<super::GitData>,
    worktree_summary_cache: RefCell<HashMap<AbsolutePath, Vec<super::WorktreeInfo>>>,
    /// Per-row `inner_y` positions recorded each frame, indexed by
    /// logical row. `content_area` is the absolute Rect on screen.
    /// `Hittable::hit_test_at` walks this list with the recorded
    /// scroll offset to map `pos.y` back to a row index.
    row_layout:             GitRowLayout,
}

#[derive(Clone, Default)]
struct GitRowLayout {
    content_area:  Rect,
    scroll_offset: usize,
    row_line_ys:   Vec<usize>,
}

impl GitPane {
    pub fn new() -> Self {
        Self {
            viewport:               Viewport::new(),
            content:                None,
            worktree_summary_cache: std::cell::RefCell::new(std::collections::HashMap::new()),
            row_layout:             GitRowLayout::default(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::GitData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::GitData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    pub fn worktree_summary_or_compute(
        &self,
        group_root: &Path,
        compute: impl FnOnce() -> Vec<super::WorktreeInfo>,
    ) -> Vec<super::WorktreeInfo> {
        if let Some(infos) = self.worktree_summary_cache.borrow().get(group_root) {
            return infos.clone();
        }
        let infos = compute();
        self.worktree_summary_cache.borrow_mut().insert(
            crate::project::AbsolutePath::from(group_root),
            infos.clone(),
        );
        infos
    }

    pub fn clear_worktree_summary_cache(&self) { self.worktree_summary_cache.borrow_mut().clear(); }

    pub fn set_row_layout(&mut self, content_area: Rect, row_line_ys: Vec<usize>) {
        self.row_layout = GitRowLayout {
            content_area,
            scroll_offset: self.viewport.scroll_offset(),
            row_line_ys,
        };
    }
}

impl Pane for GitPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(crate::tui::constants::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        git::render_git_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable for GitPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let layout = &self.row_layout;
        let inner = layout.content_area;
        if !inner.contains(pos) {
            return None;
        }
        let visible_top = inner.y;
        let visible_bottom = inner.y.saturating_add(inner.height);
        for (row_index, &inner_y) in layout.row_line_ys.iter().enumerate() {
            if inner_y < layout.scroll_offset {
                continue;
            }
            let offset = inner_y - layout.scroll_offset;
            let screen_y = inner
                .y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            if screen_y < visible_top || screen_y >= visible_bottom {
                continue;
            }
            if pos.y == screen_y {
                return Some(HoverTarget::PaneRow {
                    pane: PaneId::Git,
                    row:  row_index,
                });
            }
        }
        None
    }
}

// ── Lints ───────────────────────────────────────────────────────
pub struct LintsPane {
    viewport: Viewport,
    content:  Option<super::LintsData>,
}

impl LintsPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::LintsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::LintsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for LintsPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        lints::render_lints_pane_body(frame, area, self, ctx);
    }
}

impl Hittable for LintsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Lints,
            row,
        })
    }
}

// ── CiRuns ──────────────────────────────────────────────────────
//
// Per-path `display_modes` (BranchOnly / All) live on
// `tui::ci_state::Ci` as domain state; `CiPane` holds only the
// viewport and content cache.
pub struct CiPane {
    viewport: Viewport,
    content:  Option<super::CiData>,
}

impl CiPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::CiData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::CiData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    #[cfg(test)]
    pub fn override_runs_for_test(&mut self, runs: Vec<CiRun>) {
        if let Some(ci) = self.content.as_mut() {
            ci.runs = runs;
            ci.mode_label = None;
        }
    }
}

impl Pane for CiPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        ci::render_ci_pane_body(frame, area, self, ctx);
    }
}

impl Hittable for CiPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::CiRuns,
            row,
        })
    }
}

// Phase 14 absorption: `ToastsPane` was deleted. The toasts viewport
// and hit rects now live on `ToastManager` itself, which directly
// `impl Pane` and `impl Hittable` (see `tui/toasts/manager.rs`).

// ── Keymap ──────────────────────────────────────────────────────
pub struct KeymapPane {
    viewport: Viewport,
}

impl KeymapPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Pane for KeymapPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {
        // Overlay path in `keymap_ui::render_keymap_popup`.
    }
}

impl Hittable for KeymapPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Keymap,
            row,
        })
    }
}

// ── Settings ────────────────────────────────────────────────────
pub struct SettingsPane {
    viewport:     Viewport,
    /// Per-rendered-line mapping from line index (relative to the
    /// settings popup's content area) to the underlying setting row
    /// index. Spacer / header lines are `None`. Recorded by
    /// `settings::render_settings_popup`.
    line_targets: Vec<Option<usize>>,
}

impl SettingsPane {
    pub const fn new() -> Self {
        Self {
            viewport:     Viewport::new(),
            line_targets: Vec::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub fn set_line_targets(&mut self, targets: Vec<Option<usize>>) { self.line_targets = targets; }
}

impl Pane for SettingsPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {
        // Overlay path in `settings::render_settings_popup`.
    }
}

impl Hittable for SettingsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let inner = self.viewport.content_area();
        if inner.width == 0 || inner.height == 0 {
            return None;
        }
        if !inner.contains(pos) {
            return None;
        }
        let line_index = usize::from(pos.y.saturating_sub(inner.y));
        let row = self.line_targets.get(line_index).copied().flatten()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Settings,
            row,
        })
    }
}

// ── Finder ──────────────────────────────────────────────────────
pub struct FinderPane {
    viewport: Viewport,
}

impl FinderPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

impl Pane for FinderPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {
        // Overlay path in `finder::render_finder_popup`.
    }
}

impl Hittable for FinderPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Finder,
            row,
        })
    }
}

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane {
    viewport: Viewport,
    content:  Option<super::TargetsData>,
}

impl TargetsPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub const fn content(&self) -> Option<&super::TargetsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::TargetsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Hittable for TargetsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Targets,
            row,
        })
    }
}

// TargetsPane is rendered by free functions in `panes/targets.rs`
// (no `Pane` trait impl yet). To still participate in
// `Hittable`-trait dispatch we provide a no-op `Pane` impl here so
// `Hittable: Pane` is satisfied.
impl Pane for TargetsPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {
        // Render handled by `panes::render_targets_panel` /
        // `render_empty_targets_panel`.
    }
}

// ── ProjectList ─────────────────────────────────────────────────
pub struct ProjectListPane {
    viewport:        Viewport,
    /// Per-row dismiss `[x]` rects recorded each frame, alongside
    /// the resolved `DismissTarget`. The action region wins over
    /// the row body in `Hittable::hit_test_at`.
    dismiss_actions: Vec<(Rect, DismissTarget)>,
}

impl ProjectListPane {
    pub const fn new() -> Self {
        Self {
            viewport:        Viewport::new(),
            dismiss_actions: Vec::new(),
        }
    }

    pub const fn viewport(&self) -> &Viewport { &self.viewport }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }

    pub fn set_dismiss_actions(&mut self, actions: Vec<(Rect, DismissTarget)>) {
        self.dismiss_actions = actions;
    }
}

impl Pane for ProjectListPane {
    fn render(&mut self, _frame: &mut Frame<'_>, _area: Rect, _ctx: &PaneRenderCtx<'_>) {
        // Render handled by `panes::project_list::render_left_panel`.
    }
}

impl Hittable for ProjectListPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        for (rect, target) in &self.dismiss_actions {
            if rect.contains(pos) {
                return Some(HoverTarget::Dismiss(target.clone()));
            }
        }
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::ProjectList,
            row,
        })
    }
}

// ── Output ──────────────────────────────────────────────────────
pub struct OutputPane {
    viewport: Viewport,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }

    pub const fn viewport_mut(&mut self) -> &mut Viewport { &mut self.viewport }
}

// `OutputPane` is not `Hittable` — the output panel is read-only,
// not click-targeted today.

// ── Helpers ─────────────────────────────────────────────────────

/// Hit-test a table-shaped pane (Lints, CI, Finder) where the
/// first line of the inner area is a column header and rows start
/// at `inner.y + 1`. `viewport.content_area` is the full inner
/// rect (including the header); `viewport.scroll_offset` is the
/// `TableState::offset()` recorded at render time.
fn hit_test_table_row(viewport: &Viewport, pos: Position) -> Option<usize> {
    let inner = viewport.content_area();
    if inner.width == 0 || inner.height == 0 {
        return None;
    }
    if !inner.contains(pos) {
        return None;
    }
    if pos.y < inner.y.saturating_add(1) {
        return None;
    }
    let visual_row = pos.y - inner.y - 1;
    let row = viewport.scroll_offset() + usize::from(visual_row);
    if row >= viewport.len() {
        return None;
    }
    Some(row)
}
