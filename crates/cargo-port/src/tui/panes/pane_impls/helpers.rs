use ratatui::layout::Position;
use tui_pane::Viewport;

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
