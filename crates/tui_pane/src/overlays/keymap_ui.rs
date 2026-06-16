//! Framework-owned keymap overlay UI: trait + rendering.
//!
//! Builds [`KeymapHelpRow`] data from [`Keymap::keymap_help_rows`] and
//! draws the popup. Apps implement [`KeymapUiContext`] for the few
//! domain-specific bits the overlay needs (current inline-error
//! string, per-pane focus state, custom row ordering).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;

use super::constants::BASE_POPUP_WIDTH;
use super::constants::KEYMAP_POPUP_HEIGHT_PERCENT;
pub use super::constants::KEYMAP_POPUP_MAX_HEIGHT;
use super::constants::PERCENT_DENOMINATOR;
use super::constants::POPUP_BORDER_HEIGHT;
use crate::AppContext;
use crate::FrameworkOverlayId;
use crate::Keymap;
use crate::KeymapHelpRow;
use crate::KeymapHelpRowKind;
use crate::KeymapPane;
use crate::PaneFocusState;
use crate::PaneSelectionState;
use crate::PopupFrame;
use crate::ViewportOverflow;
use crate::active_border_color;
use crate::constants::SECTION_HEADER_INDENT;
use crate::constants::SECTION_ITEM_INDENT;
use crate::error_color;
use crate::label_color;
use crate::layout;
use crate::text_default;
use crate::title_color;

/// App-side callbacks the framework's keymap-help overlay needs.
///
/// Every method either reads app-managed state the framework can't
/// own (inline-error UI string, per-pane focus tracking) or accepts a
/// scope/action and returns a sort key for custom ordering. Apps
/// without special needs can leave the defaults in place.
pub trait KeymapUiContext: AppContext {
    /// Current inline-error message, if any. Displayed on the
    /// selected row when [`KeymapPane::is_capturing`] is `true` and a
    /// previous capture attempt conflicted.
    fn keymap_inline_error(&self) -> Option<&str>;

    /// Focus state of the overlay's container pane when the overlay
    /// itself is closed. Used for the keymap-pane row's rendering
    /// when it is shown inline (e.g. some apps surface the keymap pane
    /// inside the tile grid as well). The default treats it as
    /// inactive.
    fn keymap_pane_focus_state(&self) -> PaneFocusState { PaneFocusState::Inactive }

    /// Sort priority within a section. Lower values render earlier;
    /// the default returns `255` (alphabetical fallback). Override
    /// when a section needs a custom order beyond description sort.
    fn keymap_pane_sort_priority(&self, _: &str, _: &str) -> u8 { u8::MAX }

    /// The app-pane id ordering the keymap-help overlay walks. Apps
    /// returning an empty slice get no app-pane sections in the
    /// overlay (the framework / nav / overlay sections still render).
    fn keymap_pane_display_order(&self) -> &[<Self as AppContext>::AppPaneId];
}

/// Lines + per-line target table built for the keymap overlay.
struct KeymapLines {
    lines:        Vec<Line<'static>>,
    line_targets: Vec<Option<usize>>,
}

/// Precomputed inputs for the keymap-help overlay render path.
///
/// Built by [`KeymapPane::prepare_overlay_inputs`] while the caller
/// still holds `&Ctx`. Subsequently consumed by
/// [`KeymapPane::render_overlay`], which takes `&mut self` and the
/// borrow-split inputs separately — sidestepping the lifetime
/// conflict that arises from passing the same `App` for both.
pub struct KeymapOverlayInputs {
    lines:          Vec<Line<'static>>,
    line_targets:   Vec<Option<usize>>,
    selectable_len: usize,
    content_width:  u16,
}

impl KeymapPane {
    /// Build the rows + lines the overlay will render. Caller holds
    /// `&Ctx` here; the result is then passed to
    /// [`Self::render_overlay`] alongside `&mut self`.
    #[must_use]
    pub fn prepare_overlay_inputs<Ctx>(ctx: &Ctx, keymap: &Keymap<Ctx>) -> KeymapOverlayInputs
    where
        Ctx: KeymapUiContext + 'static,
    {
        let order = ctx.keymap_pane_display_order();
        let mut rows = keymap.keymap_help_rows(order);
        sort_rows_in_sections(ctx, &mut rows);
        let is_capturing = ctx.framework().keymap_pane.is_capturing();
        let KeymapLines {
            lines,
            line_targets,
        } = build_lines(&rows, ctx, is_capturing);
        let selectable_len = rows
            .iter()
            .filter(|r| r.row_kind != KeymapHelpRowKind::Header)
            .count();
        let content_width = ctx.keymap_inline_error().map_or(BASE_POPUP_WIDTH, |msg| {
            // 2 indent + 25 desc + msg len + 2 pad
            let needed = u16::try_from(2 + 25 + msg.len() + 2).unwrap_or(u16::MAX);
            BASE_POPUP_WIDTH.max(needed)
        });
        KeymapOverlayInputs {
            lines,
            line_targets,
            selectable_len,
            content_width,
        }
    }

    /// Render the keymap-help overlay using pre-built `inputs` from
    /// [`Self::prepare_overlay_inputs`].
    pub fn render_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        inputs: &KeymapOverlayInputs,
    ) {
        // +2 for left/right border
        let width = (inputs.content_width + 2).min(area.width.saturating_sub(4));
        let row_count = inputs.lines.len();
        let height = keymap_popup_height(row_count, area.height);

        let popup = PopupFrame {
            title: Some(" Keymap ".to_string()),
            border_color: active_border_color(),
            width,
            height,
        }
        .render_with_areas(frame);
        let inner = popup.inner;

        self.viewport_mut().set_len(inputs.selectable_len);
        self.viewport_mut().set_content_area(inner);
        self.replace_line_targets(inputs.line_targets.clone());

        let selected_pos = self.viewport().pos();
        let line_count = inputs.lines.len();
        let visible_height = usize::from(inner.height);
        let selected_line = self
            .line_for_selection(selected_pos)
            .unwrap_or(selected_pos);
        let scroll_offset = keep_visible_scroll_offset(
            self.viewport().scroll_offset(),
            selected_line,
            visible_height,
            line_count,
        );
        self.viewport_mut().set_viewport_rows(visible_height);
        self.viewport_mut().set_scroll_offset(scroll_offset);

        let para = Paragraph::new(inputs.lines.clone())
            .scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
        frame.render_widget(para, inner);
        layout::render_overflow_affordance(
            frame,
            popup.outer,
            ViewportOverflow::new(line_count, scroll_offset, visible_height, selected_line),
            Style::default().fg(label_color()),
        );
    }
}

/// Sort action rows within each section. Headers are anchors; rows
/// between two headers are sorted by `ctx.keymap_pane_sort_priority`
/// (when set) then by description.
fn sort_rows_in_sections<Ctx>(ctx: &Ctx, rows: &mut [KeymapHelpRow])
where
    Ctx: KeymapUiContext,
{
    let mut start = 0usize;
    while start < rows.len() {
        if rows[start].row_kind != KeymapHelpRowKind::Header {
            start += 1;
            continue;
        }
        let mut end = start + 1;
        while end < rows.len() && rows[end].row_kind != KeymapHelpRowKind::Header {
            end += 1;
        }
        if end - start > 1 {
            let slice = &mut rows[start + 1..end];
            slice.sort_by(|a, b| {
                let pa = ctx.keymap_pane_sort_priority(a.scope, a.action);
                let pb = ctx.keymap_pane_sort_priority(b.scope, b.action);
                pa.cmp(&pb).then_with(|| a.description.cmp(b.description))
            });
        }
        start = end;
    }
}

/// Clamp `scroll_offset` so the selected line stays on-screen.
fn keep_visible_scroll_offset(
    current_offset: usize,
    selected_line: usize,
    visible_height: usize,
    line_count: usize,
) -> usize {
    if visible_height == 0 || line_count <= visible_height {
        return 0;
    }
    let max_offset = line_count - visible_height;
    let clamped = current_offset.min(max_offset);
    if selected_line < clamped {
        selected_line
    } else if selected_line >= clamped + visible_height {
        selected_line + 1 - visible_height
    } else {
        clamped
    }
}

/// Bound the popup height to its content and 80% of the terminal height.
fn keymap_popup_height(row_count: usize, area_height: u16) -> u16 {
    let content_height = u16::try_from(row_count).unwrap_or(u16::MAX);
    let height_cap = percent_of_height(area_height, KEYMAP_POPUP_HEIGHT_PERCENT);
    content_height
        .saturating_add(POPUP_BORDER_HEIGHT)
        .min(height_cap)
}

fn percent_of_height(height: u16, percent: u16) -> u16 {
    let scaled = u32::from(height).saturating_mul(u32::from(percent)) / PERCENT_DENOMINATOR;
    u16::try_from(scaled).unwrap_or(u16::MAX)
}

fn keymap_header_line(row: &KeymapHelpRow) -> Line<'static> {
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            format!("{}:", row.section),
            Style::default()
                .fg(title_color())
                .add_modifier(Modifier::BOLD),
        ),
    ])
}

fn build_lines<Ctx>(rows: &[KeymapHelpRow], ctx: &Ctx, is_capturing: bool) -> KeymapLines
where
    Ctx: KeymapUiContext + 'static,
{
    let mut selectable_index = 0usize;
    let mut lines = vec![Line::from("")];
    let mut line_targets: Vec<Option<usize>> = vec![None];

    let pane = &ctx.framework().keymap_pane;
    let overlay_open = ctx.framework().overlay() == Some(FrameworkOverlayId::Keymap);

    for row in rows {
        if row.row_kind == KeymapHelpRowKind::Header {
            lines.push(keymap_header_line(row));
            line_targets.push(None);
            continue;
        }

        let focus = if overlay_open {
            PaneFocusState::Active
        } else {
            ctx.keymap_pane_focus_state()
        };
        let selection = selection_state(pane, selectable_index, focus);
        let key_text = if selection != PaneSelectionState::Unselected && is_capturing {
            ctx.keymap_inline_error().map_or_else(
                || "Press key...".to_string(),
                std::string::ToString::to_string,
            )
        } else {
            row.bind
                .as_ref()
                .map(crate::KeySequence::display)
                .unwrap_or_default()
        };

        let desc_width = 25usize;
        let padded_desc = format!("{:<width$}", row.description, width = desc_width);

        let line = if selection != PaneSelectionState::Unselected
            && is_capturing
            && ctx.keymap_inline_error().is_some()
        {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    selection.patch(Style::default().fg(label_color())),
                ),
                Span::styled(
                    key_text,
                    selection.patch(Style::default().fg(error_color())),
                ),
            ])
        } else if selection != PaneSelectionState::Unselected {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}▸ {padded_desc}"),
                    selection.patch(Style::default().fg(label_color())),
                ),
                Span::styled(
                    key_text,
                    selection.patch(if is_capturing {
                        Style::default()
                            .fg(title_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(text_default())
                    }),
                ),
            ])
        } else {
            Line::from(vec![
                Span::styled(
                    format!("{SECTION_ITEM_INDENT}  {padded_desc}"),
                    Style::default().fg(label_color()),
                ),
                Span::styled(key_text, Style::default().fg(text_default())),
            ])
        };

        lines.push(line);
        line_targets.push(Some(selectable_index));
        selectable_index += 1;
    }

    lines.push(Line::from(""));
    line_targets.push(None);
    KeymapLines {
        lines,
        line_targets,
    }
}

fn selection_state(
    pane: &KeymapPane,
    selection_index: usize,
    focus: PaneFocusState,
) -> PaneSelectionState {
    let viewport = pane.viewport();
    if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Active) {
        PaneSelectionState::Active
    } else if viewport.hovered() == Some(selection_index) {
        PaneSelectionState::Hovered
    } else if selection_index == viewport.pos() && matches!(focus, PaneFocusState::Remembered) {
        PaneSelectionState::Remembered
    } else {
        PaneSelectionState::Unselected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANY_ROWS: usize = 100;
    const TALL_TERMINAL_HEIGHT: u16 = 80;
    const SHORT_TERMINAL_HEIGHT: u16 = 30;
    const COMPACT_ROWS: usize = 5;

    #[test]
    fn keymap_popup_height_caps_to_eighty_percent_of_terminal_height() {
        assert_eq!(
            keymap_popup_height(MANY_ROWS, TALL_TERMINAL_HEIGHT),
            percent_of_height(TALL_TERMINAL_HEIGHT, KEYMAP_POPUP_HEIGHT_PERCENT)
        );
        assert_eq!(
            keymap_popup_height(MANY_ROWS, SHORT_TERMINAL_HEIGHT),
            percent_of_height(SHORT_TERMINAL_HEIGHT, KEYMAP_POPUP_HEIGHT_PERCENT)
        );
    }

    #[test]
    fn keymap_popup_height_keeps_compact_content_height() {
        let compact_content_height = u16::try_from(COMPACT_ROWS)
            .unwrap_or(u16::MAX)
            .saturating_add(POPUP_BORDER_HEIGHT);

        assert_eq!(
            keymap_popup_height(COMPACT_ROWS, TALL_TERMINAL_HEIGHT),
            compact_content_height
        );
    }
}
