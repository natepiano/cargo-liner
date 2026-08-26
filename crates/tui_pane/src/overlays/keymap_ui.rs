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
use super::constants::DESCRIPTION_KEY_GAP;
use super::constants::KEYMAP_COLUMN_GAP;
use super::constants::KEYMAP_MARGIN_HEIGHT;
use super::constants::KEYMAP_POPUP_HEIGHT_PERCENT;
pub use super::constants::KEYMAP_POPUP_MAX_HEIGHT;
use super::constants::OVERLAY_RIGHT_PADDING_WIDTH;
use super::constants::PERCENT_DENOMINATOR;
use super::constants::POPUP_BORDER_HEIGHT;
use super::constants::POPUP_BORDER_WIDTH;
use super::constants::POPUP_SIDE_MARGIN_WIDTH;
use super::keymap::KeymapRowSpan;
use super::line_width;
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

/// One line of the overlay before it is dealt into columns.
struct OverlayRow {
    /// The line as it will be drawn, padded to the column's width at
    /// composition rather than here.
    line:      Line<'static>,
    /// Which selectable row it is. A section heading is `None`, which
    /// is also what says it opens a section.
    selection: Option<usize>,
}

/// A run of rows that belongs together: a heading and what stands
/// under it.
///
/// The unit the columns are filled by rather than the line, so a
/// heading is never left at the foot of a column with its rows in the
/// next one.
struct Section {
    /// Index of the heading row, where the run has one. Rows ahead of
    /// the first heading are a section with none.
    heading: Option<usize>,
    /// Indices of the rows standing under it.
    rows:    Vec<usize>,
}

/// The lines of the composed popup and where each selectable row was
/// drawn on them.
struct KeymapLines {
    lines: Vec<Line<'static>>,
    spans: Vec<KeymapRowSpan>,
}

/// Precomputed inputs for the keymap-help overlay render path.
///
/// Built by [`KeymapPane::prepare_overlay_inputs`] while the caller
/// still holds `&Ctx`. Subsequently consumed by
/// [`KeymapPane::render_overlay`], which takes `&mut self` and the
/// borrow-split inputs separately — sidestepping the lifetime
/// conflict that arises from passing the same `App` for both.
pub struct KeymapOverlayInputs {
    rows:           Vec<OverlayRow>,
    selectable_len: usize,
    column_width:   u16,
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
        let help_rows = Self::ordered_help_rows(ctx, keymap);
        let is_capturing = ctx.framework().keymap_pane.is_capturing();
        let rows = build_rows(&help_rows, ctx, is_capturing, description_width(&help_rows));
        let selectable_len = help_rows
            .iter()
            .filter(|r| r.row_kind != KeymapHelpRowKind::Header)
            .count();
        // Measured off the lines that were actually built rather than
        // reckoned up from the parts, so a column fits whatever is in
        // it -- the inline error that replaces a key while one is being
        // captured included.
        let column_width = rows
            .iter()
            .map(|row| line_width(&row.line))
            .max()
            .and_then(|width| u16::try_from(width.saturating_add(OVERLAY_RIGHT_PADDING_WIDTH)).ok())
            .unwrap_or(BASE_POPUP_WIDTH);
        KeymapOverlayInputs {
            rows,
            selectable_len,
            column_width,
        }
    }

    /// Build action rows in the same order used by the keymap renderer.
    ///
    /// Input controllers use this when translating a visible selection
    /// into the stable scope/action pair that will be rebound. Keeping
    /// ordering here prevents rendered sorting from changing which action
    /// a selected row edits.
    #[must_use]
    pub fn ordered_help_rows<Ctx>(ctx: &Ctx, keymap: &Keymap<Ctx>) -> Vec<KeymapHelpRow>
    where
        Ctx: KeymapUiContext + 'static,
    {
        let order = ctx.keymap_pane_display_order();
        let mut rows = keymap.keymap_help_rows(order);
        sort_rows_in_sections(ctx, &mut rows);
        rows
    }

    /// Render the keymap-help overlay using pre-built `inputs` from
    /// [`Self::prepare_overlay_inputs`].
    pub fn render_overlay(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        inputs: &KeymapOverlayInputs,
    ) {
        let stride = inputs.column_width.saturating_add(KEYMAP_COLUMN_GAP);
        let outer_cap = area.width.saturating_sub(POPUP_SIDE_MARGIN_WIDTH);
        let columns = column_layout(
            &inputs.rows,
            usize::from(rows_cap(area.height)),
            usize::from(columns_that_fit(outer_cap, stride)),
        );
        let composed = compose(
            &inputs.rows,
            &columns,
            inputs.column_width,
            KEYMAP_COLUMN_GAP,
        );

        let content_width = u16::try_from(columns.len())
            .unwrap_or(1)
            .saturating_mul(stride)
            .saturating_sub(KEYMAP_COLUMN_GAP)
            .max(BASE_POPUP_WIDTH);
        let width = content_width
            .saturating_add(POPUP_BORDER_WIDTH)
            .min(outer_cap);
        let row_count = composed.lines.len();
        let height = keymap_popup_height(row_count, area.height);

        let popup = PopupFrame {
            title: Some(" Keymap ".to_string()),
            border_color: title_color(),
            width,
            height,
        }
        .render_with_areas(frame);
        let inner = popup.inner;

        self.viewport_mut().set_len(inputs.selectable_len);
        self.viewport_mut().set_content_area(inner);
        self.replace_row_spans(composed.spans);

        let selected_pos = self.viewport().pos();
        let line_count = composed.lines.len();
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

        let para =
            Paragraph::new(composed.lines).scroll((u16::try_from(scroll_offset).unwrap_or(0), 0));
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

/// How wide the description column stands: the widest description
/// there is to show, plus a gap before the keys.
///
/// Measured rather than fixed. Every description was padded to the
/// same fixed width, and one longer than that pushed its own key right
/// and nothing else's -- so the key column stopped being a column, and
/// a long description ran straight into the key beside it.
fn description_width(rows: &[KeymapHelpRow]) -> usize {
    rows.iter()
        .filter(|row| row.row_kind != KeymapHelpRowKind::Header)
        .map(|row| line_width(&Line::from(row.description)))
        .max()
        .unwrap_or(0)
        .saturating_add(DESCRIPTION_KEY_GAP)
}

fn build_rows<Ctx>(
    rows: &[KeymapHelpRow],
    ctx: &Ctx,
    is_capturing: bool,
    desc_width: usize,
) -> Vec<OverlayRow>
where
    Ctx: KeymapUiContext + 'static,
{
    let mut selectable_index = 0usize;
    let mut built: Vec<OverlayRow> = Vec::with_capacity(rows.len());

    let pane = &ctx.framework().keymap_pane;
    let overlay_open = ctx.framework().overlay() == Some(FrameworkOverlayId::Keymap);

    for row in rows {
        if row.row_kind == KeymapHelpRowKind::Header {
            built.push(OverlayRow {
                line:      keymap_header_line(row),
                selection: None,
            });
            continue;
        }

        let focus = if overlay_open {
            PaneFocusState::Active
        } else {
            ctx.keymap_pane_focus_state()
        };
        let selection = crate::selection_state(pane.viewport(), selectable_index, focus);
        let key_text = if selection != PaneSelectionState::Unselected && is_capturing {
            ctx.keymap_inline_error().map_or_else(
                || "Press key...".to_string(),
                std::string::ToString::to_string,
            )
        } else {
            row.bind.as_ref().map_or_else(String::new, |bind| {
                bind.platform_label(crate::AltModifierLabel::current())
            })
        };

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

        built.push(OverlayRow {
            line,
            selection: Some(selectable_index),
        });
        selectable_index += 1;
    }

    built
}

/// Group the rows into the runs the columns are filled by: a heading
/// and the rows standing under it.
fn sections_of(rows: &[OverlayRow]) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        match (row.selection, sections.last_mut()) {
            (None, _) => sections.push(Section {
                heading: Some(index),
                rows:    Vec::new(),
            }),
            (Some(_), Some(section)) => section.rows.push(index),
            (Some(_), None) => sections.push(Section {
                heading: None,
                rows:    vec![index],
            }),
        }
    }
    sections
}

/// Add a heading and `rows` to the column being filled.
fn push_section(columns: &mut [Vec<usize>], heading: Option<usize>, rows: &[usize]) {
    let Some(column) = columns.last_mut() else {
        return;
    };
    if let Some(heading) = heading {
        column.push(heading);
    }
    column.extend_from_slice(rows);
}

/// Deal the rows into columns no taller than `height`, and say which
/// rows each column holds.
///
/// Filled the way a newspaper fills its columns: a section that will
/// not fit in what is left of a column starts the next one instead of
/// being cut, and only a section taller than a whole column is split --
/// its heading drawn again above the part that carries on, so a column
/// never opens with rows that answer to nothing.
fn fill_columns(rows: &[OverlayRow], height: usize) -> Vec<Vec<usize>> {
    if height == 0 || rows.is_empty() {
        return vec![(0..rows.len()).collect()];
    }
    let mut columns: Vec<Vec<usize>> = vec![Vec::new()];
    for section in sections_of(rows) {
        let head = usize::from(section.heading.is_some());
        let mut rest = section.rows.as_slice();
        loop {
            let taken = columns.last().map_or(0, Vec::len);
            let room = height.saturating_sub(taken);
            let whole = head.saturating_add(rest.len());
            if room >= whole {
                push_section(&mut columns, section.heading, rest);
                break;
            }
            // It would fit a column of its own, so it gets one rather
            // than being cut for the sake of the lines left here.
            let fits = room.saturating_sub(head);
            if (taken > 0 && height >= whole) || fits == 0 {
                if taken == 0 {
                    // Shorter than a heading and one row: nothing will
                    // ever fit, so draw it and let the popup scroll.
                    push_section(&mut columns, section.heading, rest);
                    break;
                }
                columns.push(Vec::new());
                continue;
            }
            push_section(&mut columns, section.heading, &rest[..fits]);
            rest = &rest[fits..];
            columns.push(Vec::new());
        }
    }
    columns.retain(|column| !column.is_empty());
    if columns.is_empty() {
        columns.push(Vec::new());
    }
    columns
}

/// How many columns of `stride` cells stand side by side in a popup no
/// wider than `outer_cap`. Always at least one.
fn columns_that_fit(outer_cap: u16, stride: u16) -> u16 {
    let inside = outer_cap.saturating_sub(POPUP_BORDER_WIDTH);
    if stride == 0 {
        return 1;
    }
    (inside.saturating_add(KEYMAP_COLUMN_GAP) / stride).max(1)
}

/// How many rows of content the popup can show without scrolling.
fn rows_cap(area_height: u16) -> u16 {
    percent_of_height(area_height, KEYMAP_POPUP_HEIGHT_PERCENT)
        .saturating_sub(POPUP_BORDER_HEIGHT)
        .saturating_sub(KEYMAP_MARGIN_HEIGHT)
        .max(1)
}

/// Deal the rows into as few columns as will show them without
/// scrolling, and no more than there is width for.
///
/// Two questions, asked in that order. How many columns are needed is
/// answered at the tallest the popup is allowed to stand, so a display
/// with room for one column of everything gets one. How tall the
/// columns then stand is the shortest height that still fits in that
/// many, which is what evens them up -- filled at the full height
/// instead, the last column comes out nearly empty beside the first.
///
/// Where the width cannot carry the columns that were needed, the count
/// is what gives way and the popup scrolls, because a column drawn past
/// the right edge cannot be reached at all.
fn column_layout(rows: &[OverlayRow], rows_cap: usize, columns_fit: usize) -> Vec<Vec<usize>> {
    let wanted = fill_columns(rows, rows_cap).len().min(columns_fit).max(1);
    let mut low = 1usize;
    let mut high = rows.len().max(1);
    while low < high {
        let mid = low + (high - low) / 2;
        if fill_columns(rows, mid).len() <= wanted {
            high = mid;
        } else {
            low = mid.saturating_add(1);
        }
    }
    fill_columns(rows, low)
}

/// Draw the columns side by side, one line of the popup per row of the
/// tallest of them, and say where each selectable row landed.
fn compose(
    rows: &[OverlayRow],
    columns: &[Vec<usize>],
    column_width: u16,
    gap: u16,
) -> KeymapLines {
    let stride = usize::from(column_width.saturating_add(gap));
    let height = columns.iter().map(Vec::len).max().unwrap_or(0);
    // The blank line the popup opens with, which is why every span
    // below is recorded against `lines.len()` after it.
    let mut lines = vec![Line::from("")];
    let mut spans: Vec<KeymapRowSpan> = Vec::new();

    for row in 0..height {
        let mut drawn: Vec<Span<'static>> = Vec::new();
        for (index, column) in columns.iter().enumerate() {
            let start = index.saturating_mul(stride);
            let last = index.saturating_add(1) == columns.len();
            let held = column.get(row).and_then(|held| rows.get(*held));
            let width = held.map_or(0, |source| {
                drawn.extend(source.line.spans.iter().cloned());
                if let Some(selection) = source.selection {
                    spans.push(KeymapRowSpan {
                        line: lines.len(),
                        start: u16::try_from(start).unwrap_or(u16::MAX),
                        width: column_width.saturating_add(gap),
                        selection,
                    });
                }
                line_width(&source.line)
            });
            if !last {
                drawn.push(Span::raw(" ".repeat(stride.saturating_sub(width))));
            }
        }
        lines.push(Line::from(drawn));
    }

    lines.push(Line::from(""));
    KeymapLines { lines, spans }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    const MANY_ROWS: usize = 100;
    /// A column narrow enough to read in a test and wide enough that
    /// the rows built here do not overrun it.
    const COLUMN_WIDTH: u16 = 20;
    const TALL_TERMINAL_HEIGHT: u16 = 80;
    const SHORT_TERMINAL_HEIGHT: u16 = 30;
    const COMPACT_ROWS: usize = 5;

    /// A row long enough to overrun the fixed column the descriptions
    /// used to be padded to.
    const LONG: &str = "Cycle which of the band's edges fray";

    fn action_row(description: &'static str) -> KeymapHelpRow {
        KeymapHelpRow {
            section: "Attract: Moving Band",
            scope: "attract_moving_band",
            action: "cycle_fraying",
            description,
            bind: None,
            row_kind: KeymapHelpRowKind::Action,
        }
    }

    /// The description column stands as wide as the widest description
    /// there is, so every key starts in the same place.
    ///
    /// Padding to a fixed width instead let a longer description push
    /// its own key right and nothing else's, which put the key hard
    /// against the last word of the description and read as part of it.
    #[test]
    fn the_description_column_fits_the_widest_description() {
        let rows = [action_row("Thin the band"), action_row(LONG)];

        assert_eq!(
            description_width(&rows),
            LONG.len().saturating_add(DESCRIPTION_KEY_GAP),
        );
    }

    /// Section headings are drawn on lines of their own with no key
    /// beside them, so a long one is not the column's business.
    #[test]
    fn a_section_heading_does_not_widen_the_description_column() {
        let short = action_row("Quit");
        let heading = KeymapHelpRow {
            row_kind: KeymapHelpRowKind::Header,
            description: LONG,
            ..action_row(LONG)
        };

        assert_eq!(
            description_width(&[short, heading]),
            "Quit".len().saturating_add(DESCRIPTION_KEY_GAP),
        );
    }

    /// One heading with `rows` selectable rows under it.
    fn section(heading: &str, rows: usize, from: usize) -> Vec<OverlayRow> {
        let mut built = vec![OverlayRow {
            line:      Line::from(heading.to_string()),
            selection: None,
        }];
        built.extend((0..rows).map(|index| OverlayRow {
            line:      Line::from(format!("row {}", from + index)),
            selection: Some(from + index),
        }));
        built
    }

    /// Three sections of `sizes` rows each, numbered straight through.
    fn field(sizes: &[usize]) -> Vec<OverlayRow> {
        let mut built = Vec::new();
        let mut from = 0usize;
        for (index, size) in sizes.iter().enumerate() {
            built.extend(section(&format!("Section {index}:"), *size, from));
            from += size;
        }
        built
    }

    /// Which selectable rows a column holds, in the order they were
    /// drawn.
    fn selections(rows: &[OverlayRow], column: &[usize]) -> Vec<usize> {
        column
            .iter()
            .filter_map(|index| rows.get(*index).and_then(|row| row.selection))
            .collect()
    }

    /// A section that will not fit in what is left of a column starts
    /// the next one rather than being cut. A heading left at the foot
    /// of a column with its rows in the next reads as a heading over
    /// nothing.
    #[test]
    fn a_section_that_does_not_fit_starts_the_next_column() {
        let rows = field(&[4, 4]);
        // Room for the first section and one line more.
        let columns = fill_columns(&rows, 6);

        assert_eq!(columns.len(), 2, "the second section should start a column");
        assert_eq!(selections(&rows, &columns[0]), vec![0, 1, 2, 3]);
        assert_eq!(selections(&rows, &columns[1]), vec![4, 5, 6, 7]);
    }

    /// A section taller than a whole column is split, and its heading
    /// drawn again above the part that carries on -- otherwise a column
    /// opens with rows that answer to nothing.
    #[test]
    fn a_section_taller_than_a_column_repeats_its_heading() {
        let rows = field(&[6]);
        let columns = fill_columns(&rows, 4);

        assert_eq!(columns.len(), 2);
        assert_eq!(selections(&rows, &columns[0]), vec![0, 1, 2]);
        assert_eq!(selections(&rows, &columns[1]), vec![3, 4, 5]);
        assert_eq!(
            columns[0].first(),
            columns[1].first(),
            "both columns should open with the same heading row"
        );
    }

    /// The columns are filled at the shortest height that still fits
    /// in the number of them needed, so they come out even. Filled at
    /// the full height the popup allows, the last would stand nearly
    /// empty beside the first.
    #[test]
    fn the_columns_even_up_rather_than_filling_the_first() {
        let rows = field(&[3, 3, 3, 3]);
        let columns = column_layout(&rows, 8, 4);

        assert_eq!(columns.len(), 2, "eight rows of room needs two columns");
        assert_eq!(
            columns[0].len(),
            columns[1].len(),
            "and four even sections should divide evenly between them"
        );
    }

    /// Everything that fits in one column is drawn in one, whatever
    /// width there is: columns are what a popup taller than the
    /// terminal is worth, not a look to reach for.
    #[test]
    fn content_that_fits_is_drawn_in_one_column() {
        let rows = field(&[3, 3]);

        assert_eq!(column_layout(&rows, 40, 6).len(), 1);
    }

    /// Where the width will not carry the columns that were needed,
    /// the count gives way and the popup scrolls. A column drawn past
    /// the right edge cannot be reached at all.
    #[test]
    fn columns_never_run_past_the_width_there_is() {
        let rows = field(&[8, 8, 8, 8]);

        assert_eq!(column_layout(&rows, 6, 2).len(), 2);
    }

    /// Each column's rows are recorded at that column's own offset, so
    /// a click lands on the row it was aimed at rather than on the
    /// leftmost row of the line.
    #[test]
    fn a_row_is_recorded_where_its_column_was_drawn() {
        let rows = field(&[2, 2]);
        let columns = fill_columns(&rows, 3);
        let composed = compose(&rows, &columns, COLUMN_WIDTH, KEYMAP_COLUMN_GAP);

        let first = composed
            .spans
            .iter()
            .find(|span| span.selection == 0)
            .expect("the first row should be recorded");
        let second = composed
            .spans
            .iter()
            .find(|span| span.selection == 2)
            .expect("the first row of the second column should be recorded");

        assert_eq!(first.line, second.line, "both stand on the same line");
        assert_eq!(first.start, 0);
        assert_eq!(second.start, COLUMN_WIDTH + KEYMAP_COLUMN_GAP);
    }

    /// Every column is padded to the same width, so the second column
    /// starts in the same place on every line however short the line
    /// beside it is.
    #[test]
    fn a_short_row_still_holds_its_column_open() {
        let rows = field(&[2, 2]);
        let columns = fill_columns(&rows, 3);
        let composed = compose(&rows, &columns, COLUMN_WIDTH, KEYMAP_COLUMN_GAP);

        let widths: Vec<usize> = composed
            .lines
            .iter()
            .skip(1)
            .take(columns[0].len())
            .map(line_width)
            .collect();

        for width in widths {
            assert!(
                width >= usize::from(COLUMN_WIDTH),
                "a line should stand at least one whole column wide, not {width}"
            );
        }
    }

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
