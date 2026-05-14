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
use tui_pane::Viewport;

use super::PaneId;
use super::cpu;
use super::git;
use super::lang;
use super::output;
use super::package;
use super::package::RenderStyles;
use super::targets;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::cpu::CpuPoller;
use crate::tui::cpu::CpuUsage;
use crate::tui::pane;
use crate::tui::pane::DismissTarget;
use crate::tui::pane::Hittable;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneRenderCtx;

// ── Package ─────────────────────────────────────────────────────
pub struct PackagePane {
    pub viewport: Viewport,
    content:      Option<super::PackageData>,
}

impl PackagePane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn content(&self) -> Option<&super::PackageData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::PackageData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Pane for PackagePane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        package::render_package_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for PackagePane {
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
    pub viewport: Viewport,
}

impl LangPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }
}

impl Pane for LangPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        lang::render_lang_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for LangPane {
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
    pub viewport: Viewport,
    content:      Option<CpuUsage>,
    poller:       CpuPoller,
    /// Per-rendered-row `(Rect, logical_row)` recorded each frame
    /// so `Hittable::hit_test_at` can map `pos` back to the logical
    /// row. CPU rows are non-uniform (aggregate, per-core,
    /// breakdown, GPU) so a flat `viewport.pos_to_local_row` won't
    /// work.
    row_rects:    Vec<(Rect, usize)>,
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

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Pane for CpuPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        cpu::render_cpu_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for CpuPane {
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
    pub viewport:           Viewport,
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
            readonly_label: ratatui::style::Style::default().fg(tui_pane::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        git::render_git_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for GitPane {
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

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane {
    pub viewport: Viewport,
    content:      Option<super::TargetsData>,
}

impl TargetsPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            content:  None,
        }
    }

    pub const fn content(&self) -> Option<&super::TargetsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::TargetsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }
}

impl Hittable<HoverTarget> for TargetsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Targets,
            row,
        })
    }
}

impl Pane for TargetsPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::LABEL_COLOR),
            chrome:         pane::default_pane_chrome(),
        };
        targets::render_targets_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── ProjectList ─────────────────────────────────────────────────
pub struct ProjectListPane {
    pub viewport:    Viewport,
    /// Per-row dismiss `[x]` rects recorded each frame, alongside
    /// the resolved `DismissTarget`. The action region wins over
    /// the row body in `Hittable::hit_test_at`.
    dismiss_actions: Vec<(Rect, DismissTarget)>,
    /// Rect occupied by the list body, recorded during render and
    /// read by input dispatch for click / scroll hit-testing.
    pub body_rect:   Rect,
}

impl ProjectListPane {
    pub const fn new() -> Self {
        Self {
            viewport:        Viewport::new(),
            dismiss_actions: Vec::new(),
            body_rect:       Rect::ZERO,
        }
    }

    pub fn set_dismiss_actions(&mut self, actions: Vec<(Rect, DismissTarget)>) {
        self.dismiss_actions = actions;
    }
}

impl Pane for ProjectListPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        super::project_list::render_project_list_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for ProjectListPane {
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
    pub viewport: Viewport,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
        }
    }
}

impl Pane for OutputPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        output::render_output_pane_body(frame, area, ctx);
    }
}

// `OutputPane` is not `Hittable` — the output panel is read-only,
// not click-targeted today.

// ── Helpers ─────────────────────────────────────────────────────

/// Hit-test a table-shaped pane (Lints, CI, Finder) where the
/// first line of the inner area is a column header and rows start
/// at `inner.y + 1`. `viewport.content_area` is the full inner
/// rect (including the header); `viewport.scroll_offset` is the
/// `TableState::offset()` recorded at render time.
pub fn hit_test_table_row(viewport: &Viewport, pos: Position) -> Option<usize> {
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
