use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use super::CargoGroup;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::PaneId;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::TargetsData;
use crate::tui::render_context::PaneRenderCtx;

// ── Targets ─────────────────────────────────────────────────────
pub struct TargetsPane {
    pub viewport:       Viewport,
    pub focus:          RenderFocus,
    content:            Option<TargetsData>,
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

    pub const fn content(&self) -> Option<&TargetsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: TargetsData) { self.content = Some(data); }

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
        super::render_targets_pane_body(frame, area, self, &styles, ctx);
    }
}
