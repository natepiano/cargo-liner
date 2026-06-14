use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::Viewport;

use super::render::GitVisualRowSpan;
use crate::project::AbsolutePath;
use crate::tui::hit_test::HoverTarget;
use crate::tui::panes::GitData;
use crate::tui::panes::PaneId;
use crate::tui::panes::RenderStyles;
use crate::tui::panes::WorktreeInfo;
use crate::tui::panes::git;
use crate::tui::render_context::PaneRenderCtx;

// ── Git ─────────────────────────────────────────────────────────
pub struct GitPane {
    pub viewport:           Viewport,
    pub focus:              RenderFocus,
    content:                Option<GitData>,
    worktree_summary_cache: RefCell<HashMap<AbsolutePath, Vec<WorktreeInfo>>>,
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

    pub const fn content(&self) -> Option<&GitData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: GitData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

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
