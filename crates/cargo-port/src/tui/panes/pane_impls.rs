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
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::CopySelectionResult;
use tui_pane::CpuMonitor;
use tui_pane::CpuUsage;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use super::PaneId;
use super::copy_payload_for_output;
use super::cpu;
use super::git;
use super::git::GitVisualRowSpan;
use super::lang;
use super::output;
use super::package;
use super::package::RenderStyles;
use super::project_list;
use super::targets;
use super::targets::CargoGroup;
use crate::channel::Receiver;
use crate::config::CpuConfig;
use crate::project::AbsolutePath;
use crate::tui::pane::DismissTarget;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::PaneRenderCtx;

// ── Package ─────────────────────────────────────────────────────
pub struct PackagePane {
    pub viewport:        Viewport,
    pub focus:           RenderFocus,
    content:             Option<super::PackageData>,
    row_rects:           Vec<(Rect, usize)>,
    /// Scroll offset of the Tests box in the stats column, held across
    /// frames so the box stays put while the cursor is on a pinned row.
    /// Separate from `viewport.scroll_offset`, which the metadata column owns.
    tests_scroll_offset: usize,
}

impl PackagePane {
    pub const fn new() -> Self {
        Self {
            viewport:            Viewport::new(),
            focus:               RenderFocus::inactive(),
            content:             None,
            row_rects:           Vec::new(),
            tests_scroll_offset: 0,
        }
    }

    pub const fn content(&self) -> Option<&super::PackageData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::PackageData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }

    pub const fn tests_scroll_offset(&self) -> usize { self.tests_scroll_offset }

    pub const fn set_tests_scroll_offset(&mut self, offset: usize) {
        self.tests_scroll_offset = offset;
    }
}

impl Renderable<PaneRenderCtx<'_>> for PackagePane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        package::render_package_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for PackagePane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Package,
            row,
        })
    }
}

// ── Lang ────────────────────────────────────────────────────────
pub struct LangPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
}

impl LangPane {
    pub const fn new() -> Self {
        Self {
            viewport: Viewport::new(),
            focus:    RenderFocus::inactive(),
        }
    }
}

impl Renderable<PaneRenderCtx<'_>> for LangPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
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
    pub focus:    RenderFocus,
    content:      Option<CpuUsage>,
    monitor:      CpuMonitor,
    /// Per-rendered-row `(Rect, logical_row)` recorded each frame
    /// so `Hittable::hit_test_at` can map `pos` back to the logical
    /// row. CPU rows are non-uniform (aggregate, per-core,
    /// breakdown, GPU) so a flat `viewport.pos_to_local_row` won't
    /// work.
    row_rects:    Vec<(Rect, usize)>,
}

impl CpuPane {
    pub fn new(cpu_config: &CpuConfig) -> Self {
        let mut pane = Self {
            viewport:  Viewport::new(),
            focus:     RenderFocus::inactive(),
            content:   None,
            monitor:   CpuMonitor::new(cpu_config.poll_ms),
            row_rects: Vec::new(),
        };
        pane.install_placeholder();
        pane
    }

    pub fn tick(&mut self) {
        if let Some(usage) = self.monitor.latest() {
            self.content = Some(usage);
        }
    }

    /// The monitor's sample-channel receiver, for registering in the
    /// render-loop `Select` so a new CPU sample wakes the loop. Register
    /// only — `tick` remains the sole drain. Gate registration on
    /// [`is_sampling`](Self::is_sampling).
    pub const fn sample_rx(&self) -> &Receiver<CpuUsage> { self.monitor.receiver() }

    /// Whether the monitor's worker spawned and is producing samples.
    /// `false` means [`sample_rx`](Self::sample_rx) is disconnected and
    /// must not be registered in a `Select` (it would report permanently
    /// ready and busy-spin the loop).
    pub const fn is_sampling(&self) -> bool { self.monitor.is_sampling() }

    pub fn reset(&mut self, cpu_config: &CpuConfig) {
        self.monitor = CpuMonitor::new(cpu_config.poll_ms);
        self.install_placeholder();
    }

    pub fn install_placeholder(&mut self) {
        self.content = Some(self.monitor.placeholder_cpu_usage());
    }

    pub const fn content(&self) -> Option<&CpuUsage> { self.content.as_ref() }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }
}

impl Renderable<PaneRenderCtx<'_>> for CpuPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
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
    pub focus:              RenderFocus,
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
    description_rect: Option<Rect>,
    content_area:     Rect,
    scroll_offset:    usize,
    row_offset:       usize,
    row_spans:        Vec<GitVisualRowSpan>,
}

impl GitPane {
    pub fn new() -> Self {
        Self {
            viewport:               Viewport::new(),
            focus:                  RenderFocus::inactive(),
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
        self.worktree_summary_cache
            .borrow_mut()
            .insert(AbsolutePath::from(group_root), infos.clone());
        infos
    }

    pub fn clear_worktree_summary_cache(&self) { self.worktree_summary_cache.borrow_mut().clear(); }

    pub fn clear_row_layout(&mut self) { self.row_layout = GitRowLayout::default(); }

    pub(super) fn set_row_layout(
        &mut self,
        description_rect: Option<Rect>,
        content_area: Rect,
        row_offset: usize,
        row_spans: Vec<GitVisualRowSpan>,
    ) {
        self.row_layout = GitRowLayout {
            description_rect,
            content_area,
            scroll_offset: self.viewport.scroll_offset(),
            row_offset,
            row_spans,
        };
    }
}

impl Renderable<PaneRenderCtx<'_>> for GitPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        git::render_git_pane_body(frame, area, self, &styles, ctx);
    }
}

impl Hittable<HoverTarget> for GitPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let layout = &self.row_layout;
        if let Some(rect) = layout.description_rect
            && rect.contains(pos)
        {
            return Some(HoverTarget::PaneRow {
                pane: PaneId::Git,
                row:  0,
            });
        }
        let inner = layout.content_area;
        if !inner.contains(pos) {
            return None;
        }
        let visible_top = inner.y;
        let visible_bottom = inner.y.saturating_add(inner.height);
        for (row_index, span) in layout.row_spans.iter().enumerate() {
            if span.start_y.saturating_add(span.height) <= layout.scroll_offset {
                continue;
            }
            let offset = span.start_y.saturating_sub(layout.scroll_offset);
            let screen_y = inner
                .y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            let screen_bottom = screen_y.saturating_add(u16::try_from(span.height).unwrap_or(1));
            if screen_bottom <= visible_top || screen_y >= visible_bottom {
                continue;
            }
            if pos.y >= screen_y && pos.y < screen_bottom {
                return Some(HoverTarget::PaneRow {
                    pane: PaneId::Git,
                    row:  layout.row_offset + row_index,
                });
            }
        }
        None
    }
}

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane {
    pub viewport:       Viewport,
    pub focus:          RenderFocus,
    content:            Option<super::TargetsData>,
    /// Per-rendered-row `(Rect, logical_row)` recorded each frame so
    /// `Hittable::hit_test_at` can map `pos` back to the logical row.
    /// The pane stacks two boxes (the table above the Running list), so
    /// a flat `viewport.pos_to_local_row` won't work.
    row_rects:          Vec<(Rect, usize)>,
    /// PID of the Running-box instance under the highlight, `None` while
    /// the highlight is in the table or on the `cargo` group header. The
    /// render pass follows it as rows reorder (D2); navigation and clicks
    /// re-derive it; the `K` keymap gating reads `is_some()` as "the
    /// highlight is on a killable Running row".
    running_cursor_pid: Option<u32>,
    /// Expansion state of the Running list's `cargo` group; `Enter` on
    /// its header row toggles it.
    cargo_group:        CargoGroup,
    /// Outline parents (Running rows with sub-process children) the user
    /// has expanded; absent means collapsed (the default). Retained
    /// against the live row set each frame so a reused PID starts
    /// collapsed.
    expanded_parents:   HashSet<u32>,
}

impl TargetsPane {
    pub fn new() -> Self {
        Self {
            viewport:           Viewport::new(),
            focus:              RenderFocus::inactive(),
            content:            None,
            row_rects:          Vec::new(),
            running_cursor_pid: None,
            cargo_group:        CargoGroup::Collapsed,
            expanded_parents:   HashSet::new(),
        }
    }

    pub const fn content(&self) -> Option<&super::TargetsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: super::TargetsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    pub fn set_row_rects(&mut self, rects: Vec<(Rect, usize)>) { self.row_rects = rects; }

    pub fn clear_row_rects(&mut self) { self.row_rects.clear(); }

    pub const fn running_cursor_pid(&self) -> Option<u32> { self.running_cursor_pid }

    pub const fn set_running_cursor_pid(&mut self, pid: Option<u32>) {
        self.running_cursor_pid = pid;
    }

    pub const fn cargo_group(&self) -> CargoGroup { self.cargo_group }

    pub const fn toggle_cargo_group(&mut self) { self.cargo_group = self.cargo_group.toggled(); }

    pub const fn expanded_parents(&self) -> &HashSet<u32> { &self.expanded_parents }

    /// Flip one outline parent between expanded and collapsed.
    pub fn toggle_expanded_parent(&mut self, pid: u32) {
        if !self.expanded_parents.insert(pid) {
            self.expanded_parents.remove(&pid);
        }
    }

    pub fn collapse_parent(&mut self, pid: u32) { self.expanded_parents.remove(&pid); }

    /// Drop expanded-outline entries whose PID left the Running list, so
    /// a reused PID starts collapsed (the default).
    pub fn retain_expanded_parents(&mut self, live: &HashSet<u32>) {
        self.expanded_parents.retain(|pid| live.contains(pid));
    }
}

impl Hittable<HoverTarget> for TargetsPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let (_rect, row) = self
            .row_rects
            .iter()
            .find(|(rect, _)| rect.contains(pos))
            .copied()?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Targets,
            row,
        })
    }
}

impl Renderable<PaneRenderCtx<'_>> for TargetsPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        let styles = RenderStyles {
            readonly_label: ratatui::style::Style::default().fg(tui_pane::label_color()),
            chrome:         tui_pane::default_pane_chrome(),
        };
        targets::render_targets_pane_body(frame, area, self, &styles, ctx);
    }
}

// ── ProjectList ─────────────────────────────────────────────────
pub struct ProjectListPane {
    pub viewport:    Viewport,
    pub focus:       RenderFocus,
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
            focus:           RenderFocus::inactive(),
            dismiss_actions: Vec::new(),
            body_rect:       Rect::ZERO,
        }
    }

    pub fn set_dismiss_actions(&mut self, actions: Vec<(Rect, DismissTarget)>) {
        self.dismiss_actions = actions;
    }
}

impl Renderable<PaneRenderCtx<'_>> for ProjectListPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        project_list::render_project_list_pane_body(frame, area, self, ctx);
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

/// The output pane's selection sub-mode.
///
/// In `Normal` the selection is the single row under the cursor and plain
/// motions move it whole (the anchor follows the cursor). In `Visual` —
/// the vim visual-line sub-mode (`V`) — plain motions grow the range from
/// the fixed anchor.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SelectionMode {
    Normal,
    Visual,
}

/// Linewise selection state for the output pane.
///
/// There is always a selection — at minimum the single row under the
/// cursor — so the pane has no separate select/deselect mode. `anchor`
/// is the fixed end; the moving end is [`OutputPane::viewport`]'s `pos`,
/// and the selected range runs between them. `mode` is the
/// [`SelectionMode`] that decides how plain motions read.
///
/// `snapshot` freezes the buffer once the selection stops tracking the
/// live tail, so a streaming child process can't drift a pinned range.
/// While the selection follows the tail it stays `None` and render/yank
/// read the live buffer.
pub struct OutputSelection {
    anchor:   usize,
    mode:     SelectionMode,
    snapshot: Option<Rc<[String]>>,
}

impl OutputSelection {
    const fn new() -> Self {
        Self {
            anchor:   0,
            mode:     SelectionMode::Normal,
            snapshot: None,
        }
    }

    /// Whether the vim visual-line sub-mode is active.
    pub const fn is_visual(&self) -> bool { matches!(self.mode, SelectionMode::Visual) }

    /// The frozen buffer snapshot, present once the selection has stopped
    /// following the live tail.
    pub const fn snapshot(&self) -> Option<&Rc<[String]>> { self.snapshot.as_ref() }
}

pub struct OutputPane {
    pub viewport: Viewport,
    pub focus:    RenderFocus,
    selection:    OutputSelection,
}

impl OutputPane {
    pub const fn new() -> Self {
        Self {
            viewport:  Viewport::new(),
            focus:     RenderFocus::inactive(),
            selection: OutputSelection::new(),
        }
    }

    /// The current selection state.
    pub const fn selection(&self) -> &OutputSelection { &self.selection }

    /// Whether the single-row selection is pinned to the streaming tail:
    /// not in visual mode and the cursor on the last row. Following means
    /// render and yank track the live tail.
    pub const fn is_following(&self) -> bool {
        matches!(self.selection.mode, SelectionMode::Normal)
            && self.viewport.pos() >= self.viewport.len().saturating_sub(1)
    }

    /// Reset to the open-time state: a collapsed selection following the
    /// streaming tail.
    pub fn reset_for_open(&mut self) {
        self.selection = OutputSelection::new();
        self.viewport.end();
    }

    /// The source the selection reads from: the frozen snapshot once
    /// pinned, otherwise the live buffer it is following.
    fn source<'a>(&'a self, live: &'a [String]) -> &'a [String] {
        self.selection.snapshot.as_deref().unwrap_or(live)
    }

    /// Freeze the live buffer into the snapshot if it is not already
    /// frozen — called whenever the selection stops following the tail.
    fn freeze(&mut self, live: &[String]) {
        if self.selection.snapshot.is_none() {
            self.selection.snapshot = Some(Rc::from(live.to_vec()));
        }
    }

    /// Enter visual mode from the cursor: anchor the fixed end at the
    /// current cursor row and freeze `live`. A no-op when already in visual
    /// mode, so a started range keeps its anchor. The `anchor` field is
    /// meaningful only in [`SelectionMode::Visual`]; entering this mode is
    /// the one place it is set, so it can never drift from a plain cursor
    /// move.
    fn enter_visual(&mut self, live: &[String]) {
        if matches!(self.selection.mode, SelectionMode::Normal) {
            self.selection.mode = SelectionMode::Visual;
            self.selection.anchor = self.viewport.pos();
            self.freeze(live);
        }
    }

    /// Toggle the vim visual-line sub-mode. Entering anchors at the cursor
    /// and freezes `live`; leaving collapses the selection back to the
    /// single cursor row. Vim-mode only — bound to `V`.
    pub fn toggle_visual(&mut self, live: &[String]) {
        match self.selection.mode {
            SelectionMode::Visual => self.exit_visual(),
            SelectionMode::Normal => self.enter_visual(live),
        }
    }

    /// Leave visual mode, collapsing the selection back to the single
    /// cursor row. A no-op when not in visual mode. Bound to `Esc` while a
    /// visual selection is active.
    pub const fn exit_visual(&mut self) { self.selection.mode = SelectionMode::Normal; }

    /// Select every line: anchor on the first row, cursor on the last, so
    /// the range spans the whole buffer. Freezes `live` first. Bound to
    /// Ctrl-A.
    pub fn select_all(&mut self, live: &[String]) {
        self.freeze(live);
        self.selection.mode = SelectionMode::Visual;
        self.selection.anchor = 0;
        let last = self.source(live).len().saturating_sub(1);
        self.viewport.set_pos(last);
    }

    /// Apply a plain navigation motion. In visual mode the motion grows
    /// the range from the anchor; otherwise it moves the single-row
    /// selection, which re-follows the tail when it lands on the last row
    /// or freezes `live` when it parks off the tail.
    pub fn navigate(&mut self, live: &[String], motion: impl FnOnce(&mut Viewport)) {
        motion(&mut self.viewport);
        match self.selection.mode {
            SelectionMode::Visual => self.freeze(live),
            SelectionMode::Normal => {
                if self.viewport.pos() >= self.viewport.len().saturating_sub(1) {
                    self.selection.snapshot = None;
                } else {
                    self.freeze(live);
                }
            },
        }
    }

    /// Extend the selection up one row, entering visual mode at the cursor
    /// first if needed. Bound to Shift+Up: the editor-style select gesture.
    pub fn select_extend_up(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.up();
    }

    /// Extend the selection down one row, the mirror of
    /// [`select_extend_up`](Self::select_extend_up). Bound to Shift+Down.
    pub fn select_extend_down(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.down();
    }

    /// Extend the selection from the cursor to the first row. Bound to
    /// Ctrl+Shift+Up.
    pub fn select_extend_to_top(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.home();
    }

    /// Extend the selection from the cursor to the last row, the mirror of
    /// [`select_extend_to_top`](Self::select_extend_to_top). Bound to
    /// Ctrl+Shift+Down.
    pub fn select_extend_to_bottom(&mut self, live: &[String]) {
        self.enter_visual(live);
        self.viewport.end();
    }

    /// Position the selection on `row` (a buffer index from
    /// [`Viewport::pos_to_local_row`]) as a fresh left-button press does:
    /// collapse any visual range back to the single clicked line — Normal
    /// mode, anchor on that row — so a release-then-click starts a new
    /// selection rather than extending the old one. Re-follows the tail
    /// when `row` is the last line, freezes `live` otherwise.
    pub fn click_select_row(&mut self, live: &[String], row: usize) {
        self.selection.mode = SelectionMode::Normal;
        self.viewport.set_pos(row);
        self.selection.anchor = row;
        if self.viewport.pos() >= self.viewport.len().saturating_sub(1) {
            self.selection.snapshot = None;
        } else {
            self.freeze(live);
        }
    }

    /// Extend a mouse drag-select to `row` (a buffer index from
    /// [`Viewport::pos_to_local_row`]), entering visual mode anchored at
    /// the press row (the cursor [`click_select_row`](Self::click_select_row)
    /// just positioned) on the first call. Bound to a left-button drag in
    /// the output pane.
    pub fn select_drag_to(&mut self, live: &[String], row: usize) {
        self.enter_visual(live);
        self.viewport.set_pos(row);
    }

    /// Collapse the selection back to the single cursor row and resume
    /// following the tail. Used after a yank, where returning to the live
    /// tail is the expected next state.
    pub fn collapse_to_tail(&mut self) {
        self.selection = OutputSelection::new();
        self.viewport.end();
    }

    /// Number of lines the selection spans against `live` (the frozen
    /// snapshot when pinned). At rest this is `1` — the cursor row.
    pub fn selection_line_count(&self, live: &[String]) -> usize {
        self.selected_range(live).map_or(0, |(lo, hi)| hi - lo + 1)
    }

    /// Inclusive `[lo, hi]` row range of the selection, clamped to the
    /// source bounds (the frozen snapshot when pinned, else `live`).
    /// Outside visual mode the range is the single cursor row; the
    /// `anchor` is read only in visual mode. `None` only when the buffer
    /// is empty.
    pub fn selected_range(&self, live: &[String]) -> Option<(usize, usize)> {
        let last = self.source(live).len().checked_sub(1)?;
        let cursor = self.viewport.pos().min(last);
        match self.selection.mode {
            SelectionMode::Visual => {
                let anchor = self.selection.anchor.min(last);
                Some((anchor.min(cursor), anchor.max(cursor)))
            },
            SelectionMode::Normal => Some((cursor, cursor)),
        }
    }

    /// Build the clipboard payload for the current selection, reading the
    /// frozen snapshot when pinned or `live` while following the tail.
    pub fn copy_payload(&self, live: &[String]) -> CopySelectionResult {
        let Some((lo, hi)) = self.selected_range(live) else {
            return CopySelectionResult::Nothing;
        };
        copy_payload_for_output(self.source(live), lo, hi)
    }

    /// Resume following the tail when a process exits, unless the user is
    /// in visual mode selecting to copy. A collapsed single-row selection —
    /// the at-rest state, whether following or just scrolled — snaps to the
    /// new tail so the final output shows.
    pub fn on_process_exit(&mut self) {
        if matches!(self.selection.mode, SelectionMode::Normal) {
            self.selection.snapshot = None;
            self.viewport.end();
        }
    }

    /// Sync the viewport surface to the rendered rows and compute the
    /// scroll offset. While the collapsed selection follows the tail, the
    /// cursor (and its anchor) stick to the new last row so streaming
    /// output stays visible; otherwise the offset keeps the cursor on
    /// screen at its pinned position.
    pub const fn sync_viewport(&mut self, len: usize, visible_rows: usize, content_area: Rect) {
        let following = self.is_following();
        self.viewport.set_len(len);
        self.viewport.set_viewport_rows(visible_rows);
        self.viewport.set_content_area(content_area);
        if following {
            self.viewport.end();
            self.selection.anchor = self.viewport.pos();
        }
        self.viewport.set_scroll_offset(scroll_to_show_cursor(
            self.viewport.pos(),
            self.viewport.scroll_offset(),
            visible_rows,
            len,
        ));
    }
}

/// Smallest scroll offset that keeps `cursor` on screen, starting from
/// the `current` offset and clamped so the view never scrolls past the
/// end.
const fn scroll_to_show_cursor(
    cursor: usize,
    current: usize,
    visible_rows: usize,
    len: usize,
) -> usize {
    if visible_rows == 0 {
        return 0;
    }
    let mut offset = if cursor < current { cursor } else { current };
    if cursor + 1 > offset + visible_rows {
        offset = cursor + 1 - visible_rows;
    }
    let max_offset = len.saturating_sub(visible_rows);
    if offset > max_offset {
        max_offset
    } else {
        offset
    }
}

impl Renderable<PaneRenderCtx<'_>> for OutputPane {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        output::render_output_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for OutputPane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = self.viewport.pos_to_local_row(pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Output,
            row,
        })
    }
}

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
