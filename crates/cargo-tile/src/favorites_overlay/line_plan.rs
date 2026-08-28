//! Cached favorites lines, table layout, navigation, and rendering.

use std::ops::Range;
use std::path::Path;
use std::time::Instant;

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::PaneFocusState;
use tui_pane::blend_color;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::selection_style;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::bindings::FavoritesSurfaceBindings;
use super::bindings::ParameterColumnDescriptor;
use super::bindings::column_descriptors;
use super::bindings::mode_label;
use super::content::FavoriteModeSection;
use super::content::FavoriteRowLifecycle;
use super::content::FavoriteRowLookup;
use super::content::FavoriteRowView;
use super::content::FavoritesOverlayContent;
use super::content::UnrecognizedFavoriteView;
use crate::app::AppOverlay;
use crate::attract;
use crate::constants::COLUMN_GAP;
use crate::constants::CURSOR_WIDTH;
use crate::constants::FAVORITE_REMOVAL_FADE;
use crate::constants::FOOTER_HEIGHT;
use crate::constants::POPUP_CHROME_HEIGHT;
use crate::constants::POPUP_CHROME_WIDTH;
use crate::constants::POPUP_MAX_WIDTH;
use crate::constants::POPUP_SIDE_MARGIN;
use crate::favorites::FavoriteId;
use crate::favorites::UnrecognizedFavoriteRemovalLocator;

#[derive(Clone, Debug)]
pub(super) enum CachedOverlayLine {
    NonRow(Line<'static>),
    Row {
        identity: FavoriteRowIdentity,
        tail:     String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FavoriteRowIdentity {
    Recognized(FavoriteId),
    Unrecognized(UnrecognizedFavoriteRemovalLocator),
}

#[derive(Clone, Debug, Default)]
pub(super) struct CachedLinePlan {
    pub(super) lines:                       Vec<CachedOverlayLine>,
    selectable_line_index:                  Vec<usize>,
    pub(super) navigation_line_index:       Vec<usize>,
    pub(super) last_horizontal_column_page: usize,
}

impl CachedLinePlan {
    #[cfg(test)]
    pub(super) fn selectable_line_index(&self) -> &[usize] { &self.selectable_line_index }

    fn finish_navigation(&mut self) {
        self.navigation_line_index
            .clone_from(&self.selectable_line_index);
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CachedSurfaceWidth {
    #[default]
    NeverRendered,
    Rendered(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FavoriteSelection {
    NoRowSelected,
    Row(FavoriteRowIdentity),
}

impl FavoriteSelection {
    fn identifies(&self, identity: &FavoriteRowIdentity) -> bool {
        matches!(self, Self::Row(selected) if selected == identity)
    }
}

pub(super) fn rendered_line(
    line: &CachedOverlayLine,
    selected: &FavoriteSelection,
    lifecycle: FavoriteRowLifecycle,
    now: Instant,
) -> Line<'static> {
    match line {
        CachedOverlayLine::NonRow(line) => line.clone(),
        CachedOverlayLine::Row { identity, tail } => {
            let is_selected = selected.identifies(identity);
            let marker = if is_selected { "▸ " } else { "  " };
            let line = Line::from(vec![Span::raw(marker), Span::raw(tail.clone())]);
            if is_selected {
                line.style(selection_style(PaneFocusState::Active))
            } else {
                let color = match identity {
                    FavoriteRowIdentity::Recognized(_) => blend_color(
                        text_default(),
                        attract::ground(),
                        removal_alpha(lifecycle, now),
                    ),
                    FavoriteRowIdentity::Unrecognized(_) => blend_color(
                        error_color(),
                        attract::ground(),
                        removal_alpha(lifecycle, now),
                    ),
                };
                line.style(Style::default().fg(color))
            }
        },
    }
}

pub(super) fn row_lifecycle(state: &AppOverlay, line: &CachedOverlayLine) -> FavoriteRowLifecycle {
    let CachedOverlayLine::Row { identity, .. } = line else {
        return FavoriteRowLifecycle::Active;
    };
    match (state, identity) {
        (
            AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)),
            FavoriteRowIdentity::Recognized(favorite_id),
        ) => match rows.row(*favorite_id) {
            FavoriteRowLookup::Found(row) => row.lifecycle,
            FavoriteRowLookup::Missing => FavoriteRowLifecycle::Active,
        },
        (
            AppOverlay::Favorites(FavoritesOverlayContent::Rows(rows)),
            FavoriteRowIdentity::Unrecognized(removal_locator),
        ) => rows
            .unrecognized
            .iter()
            .find(|row| row.removal_locator == *removal_locator)
            .map_or(FavoriteRowLifecycle::Active, |row| row.lifecycle),
        (
            AppOverlay::Favorites(FavoritesOverlayContent::OnlyUnrecognized(rows)),
            FavoriteRowIdentity::Unrecognized(removal_locator),
        ) => rows
            .rows
            .iter()
            .find(|row| row.removal_locator == *removal_locator)
            .map_or(FavoriteRowLifecycle::Active, |row| row.lifecycle),
        (
            AppOverlay::Closed
            | AppOverlay::Favorites(
                FavoritesOverlayContent::NoneSaved
                | FavoritesOverlayContent::OnlyUnrecognized(_)
                | FavoritesOverlayContent::LocationUnavailable
                | FavoritesOverlayContent::Unparseable { .. }
                | FavoritesOverlayContent::Unreadable { .. },
            ),
            FavoriteRowIdentity::Recognized(_) | FavoriteRowIdentity::Unrecognized(_),
        ) => FavoriteRowLifecycle::Active,
    }
}

fn removal_alpha(lifecycle: FavoriteRowLifecycle, now: Instant) -> u8 {
    let FavoriteRowLifecycle::Removing { since } = lifecycle else {
        return 0;
    };
    let elapsed = now.duration_since(since);
    if elapsed >= FAVORITE_REMOVAL_FADE {
        return u8::MAX;
    }
    let scaled =
        elapsed.as_nanos().saturating_mul(u128::from(u8::MAX)) / FAVORITE_REMOVAL_FADE.as_nanos();
    u8::try_from(scaled).unwrap_or(u8::MAX)
}

#[cfg(test)]
pub(super) fn removal_alpha_for_test(lifecycle: FavoriteRowLifecycle, now: Instant) -> u8 {
    removal_alpha(lifecycle, now)
}

pub(super) fn build_line_plan(
    content: &FavoritesOverlayContent,
    bindings: &FavoritesSurfaceBindings,
    width: u16,
    horizontal_page: usize,
) -> CachedLinePlan {
    let mut plan = CachedLinePlan::default();
    match content {
        FavoritesOverlayContent::Rows(rows) => {
            let table_layouts = rows
                .sections
                .iter()
                .map(|section| FavoriteSectionTableLayout::measure(section, bindings))
                .collect::<Vec<_>>();
            plan.last_horizontal_column_page = table_layouts
                .iter()
                .map(|layout| layout.last_horizontal_column_page(width))
                .max()
                .unwrap_or(0);
            let horizontal_page = horizontal_page.min(plan.last_horizontal_column_page);
            for (section, table_layout) in rows.sections.iter().zip(&table_layouts) {
                append_section(
                    &mut plan,
                    section,
                    table_layout,
                    bindings,
                    width,
                    horizontal_page,
                );
            }
            append_unrecognized(&mut plan, &rows.unrecognized);
        },
        FavoritesOverlayContent::NoneSaved => plan.lines.push(non_row_line(
            bindings.empty_notice(),
            Style::default().fg(text_default()),
        )),
        FavoritesOverlayContent::OnlyUnrecognized(rows) => {
            append_unrecognized(&mut plan, &rows.rows);
        },
        FavoritesOverlayContent::LocationUnavailable => plan.lines.push(non_row_line(
            "Favorites location unavailable -- no OS configuration directory".to_string(),
            Style::default().fg(error_color()),
        )),
        FavoritesOverlayContent::Unparseable { path, error } => {
            append_failure(&mut plan, "Favorites file is unparseable", path, error);
        },
        FavoritesOverlayContent::Unreadable { path, error } => {
            append_failure(&mut plan, "Favorites file is unreadable", path, error);
        },
    }
    plan.finish_navigation();
    plan
}

fn append_failure(plan: &mut CachedLinePlan, heading: &str, path: &Path, error: &str) {
    plan.lines.push(non_row_line(
        heading.to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.push(non_row_line(
        format!("  {}: {error}", path.display()),
        Style::default().fg(text_default()),
    ));
}

fn append_section(
    plan: &mut CachedLinePlan,
    section: &FavoriteModeSection,
    table_layout: &FavoriteSectionTableLayout,
    bindings: &FavoritesSurfaceBindings,
    width: u16,
    horizontal_page: usize,
) {
    if !plan.lines.is_empty() {
        plan.lines
            .push(non_row_line(String::new(), Style::default()));
    }
    plan.lines.push(non_row_line(
        format!("Attract: {}", mode_label(section.mode)),
        Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
    ));

    let descriptors = column_descriptors(section.mode);
    let key_labels = bindings.column_labels(section.mode);
    let visible = table_layout.visible_parameter_columns(horizontal_page, width);
    let headings = descriptors
        .iter()
        .map(|descriptor| descriptor.heading)
        .collect::<Vec<_>>();
    plan.lines.push(non_row_line(
        format_table_line(
            "Saved",
            &headings,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        ),
        Style::default()
            .fg(label_color())
            .add_modifier(Modifier::BOLD),
    ));
    plan.lines.push(non_row_line(
        format_table_line(
            "",
            key_labels,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        ),
        Style::default().fg(label_color()),
    ));
    for row in &section.rows {
        let line_index = plan.lines.len();
        if row.lifecycle == FavoriteRowLifecycle::Active {
            plan.selectable_line_index.push(line_index);
        }
        plan.lines.push(CachedOverlayLine::Row {
            identity: FavoriteRowIdentity::Recognized(row.id),
            tail:     format_table_tail(
                &row.saved,
                &row.cells,
                table_layout.saved_width,
                &table_layout.parameter_widths,
                visible.clone(),
            ),
        });
    }
}

fn append_unrecognized(plan: &mut CachedLinePlan, rows: &[UnrecognizedFavoriteView]) {
    if rows.is_empty() {
        return;
    }
    if !plan.lines.is_empty() {
        plan.lines
            .push(non_row_line(String::new(), Style::default()));
    }
    plan.lines.push(non_row_line(
        "Unrecognized favorites".to_string(),
        Style::default()
            .fg(error_color())
            .add_modifier(Modifier::BOLD),
    ));
    for row in rows {
        let line_index = plan.lines.len();
        if row.lifecycle == FavoriteRowLifecycle::Active {
            plan.selectable_line_index.push(line_index);
        }
        plan.lines.push(CachedOverlayLine::Row {
            identity: FavoriteRowIdentity::Unrecognized(row.removal_locator.clone()),
            tail:     format!("{} = {:?} is not recognized", row.key, row.spelling),
        });
    }
}

fn non_row_line(text: String, style: Style) -> CachedOverlayLine {
    CachedOverlayLine::NonRow(Line::from(text).style(style))
}

#[derive(Clone, Debug)]
struct FavoriteSectionTableLayout {
    saved_width:      u16,
    parameter_widths: ColumnWidths,
}

impl FavoriteSectionTableLayout {
    fn measure(section: &FavoriteModeSection, bindings: &FavoritesSurfaceBindings) -> Self {
        let descriptors = column_descriptors(section.mode);
        let key_labels = bindings.column_labels(section.mode);
        Self {
            saved_width:      measured_saved_width(&section.rows),
            parameter_widths: measured_parameter_widths(descriptors, key_labels, &section.rows),
        }
    }

    fn visible_parameter_columns(&self, horizontal_page: usize, width: u16) -> Range<usize> {
        visible_parameter_columns(
            horizontal_page,
            width,
            self.saved_width,
            &self.parameter_widths,
        )
    }

    fn last_horizontal_column_page(&self, width: u16) -> usize {
        let column_count = self.parameter_widths.to_constraints().len();
        (0..column_count)
            .find(|page| self.visible_parameter_columns(*page, width).end == column_count)
            .unwrap_or_else(|| column_count.saturating_sub(1))
    }
}

#[cfg(test)]
pub(super) struct FavoriteSectionTableLayoutForTest {
    pub(super) saved_width:      u16,
    pub(super) parameter_widths: ColumnWidths,
}

#[cfg(test)]
pub(super) fn favorite_section_table_layout_for_test(
    section: &FavoriteModeSection,
    bindings: &FavoritesSurfaceBindings,
) -> FavoriteSectionTableLayoutForTest {
    let table_layout = FavoriteSectionTableLayout::measure(section, bindings);
    FavoriteSectionTableLayoutForTest {
        saved_width:      table_layout.saved_width,
        parameter_widths: table_layout.parameter_widths,
    }
}

fn measured_saved_width(rows: &[FavoriteRowView]) -> u16 {
    rows.iter().fold(5_u16, |width, row| {
        width.max(u16::try_from(UnicodeWidthStr::width(row.saved.as_str())).unwrap_or(u16::MAX))
    })
}

fn measured_parameter_widths(
    descriptors: &[ParameterColumnDescriptor],
    key_labels: &[String],
    rows: &[FavoriteRowView],
) -> ColumnWidths {
    let specs = descriptors
        .iter()
        .map(|descriptor| {
            ColumnSpec::fit(
                u16::try_from(UnicodeWidthStr::width(descriptor.heading)).unwrap_or(u16::MAX),
            )
        })
        .collect();
    let mut widths = ColumnWidths::new(specs);
    for (column, label) in key_labels.iter().enumerate() {
        widths.observe_cell_usize(column, UnicodeWidthStr::width(label.as_str()));
    }
    for row in rows {
        for (column, cell) in row.cells.iter().enumerate() {
            widths.observe_cell_usize(column, UnicodeWidthStr::width(cell.as_str()));
        }
    }
    widths
}

fn visible_parameter_columns(
    horizontal_page: usize,
    width: u16,
    saved_width: u16,
    parameter_widths: &ColumnWidths,
) -> Range<usize> {
    let column_count = parameter_widths.to_constraints().len();
    if column_count == 0 {
        return 0..0;
    }
    let start = horizontal_page.min(column_count - 1);
    let pinned = CURSOR_WIDTH.saturating_add(usize::from(saved_width));
    let available = usize::from(width).saturating_sub(pinned);
    let mut used: usize = 0;
    let mut end = start;
    for column in start..column_count {
        let cost = usize::from(parameter_widths.get(column)).saturating_add(COLUMN_GAP);
        if end > start && used.saturating_add(cost) > available {
            break;
        }
        used = used.saturating_add(cost);
        end = column + 1;
    }
    start..end
}

fn format_table_line<T: AsRef<str>>(
    saved: &str,
    cells: &[T],
    saved_width: u16,
    parameter_widths: &ColumnWidths,
    visible: Range<usize>,
) -> String {
    format!(
        "  {}",
        format_table_tail(saved, cells, saved_width, parameter_widths, visible)
    )
}

fn format_table_tail<T: AsRef<str>>(
    saved: &str,
    cells: &[T],
    saved_width: u16,
    parameter_widths: &ColumnWidths,
    visible: Range<usize>,
) -> String {
    let mut line = String::new();
    push_display_padded(&mut line, saved, usize::from(saved_width));
    for column in visible {
        line.push_str(&" ".repeat(COLUMN_GAP));
        if let Some(cell) = cells.get(column) {
            push_display_padded(
                &mut line,
                cell.as_ref(),
                usize::from(parameter_widths.get(column)),
            );
        }
    }
    line
}

fn push_display_padded(line: &mut String, value: &str, width: usize) {
    line.push_str(value);
    line.push_str(&" ".repeat(width.saturating_sub(UnicodeWidthStr::width(value))));
}

pub(super) fn popup_width(area: Rect) -> u16 {
    area.width
        .saturating_sub(POPUP_SIDE_MARGIN)
        .min(POPUP_MAX_WIDTH)
        .max(area.width.min(POPUP_CHROME_WIDTH))
}

pub(super) fn popup_height_cap(area: Rect) -> u16 {
    let eighty_percent = u32::from(area.height).saturating_mul(80) / 100;
    u16::try_from(eighty_percent)
        .unwrap_or(u16::MAX)
        .max((POPUP_CHROME_HEIGHT + FOOTER_HEIGHT).min(area.height))
}

pub(super) fn wrapped_notice_height(message: &str, surface_width: u16) -> u16 {
    let available_width = usize::from(surface_width);
    if message.is_empty() || available_width == 0 {
        return 0;
    }

    let mut wrapped_lines = 0_usize;
    for logical_line in message.split('\n') {
        wrapped_lines = wrapped_lines.saturating_add(1);
        let mut used_width = 0_usize;
        for word in logical_line.split_whitespace() {
            let word_width = UnicodeWidthStr::width(word);
            let separator_width = usize::from(used_width > 0);
            if used_width
                .saturating_add(separator_width)
                .saturating_add(word_width)
                <= available_width
            {
                used_width = used_width
                    .saturating_add(separator_width)
                    .saturating_add(word_width);
                continue;
            }

            if used_width > 0 {
                wrapped_lines = wrapped_lines.saturating_add(1);
            }
            let full_lines = word_width / available_width;
            let remainder = word_width % available_width;
            if remainder == 0 && full_lines > 0 {
                wrapped_lines = wrapped_lines.saturating_add(full_lines.saturating_sub(1));
                used_width = available_width;
            } else {
                wrapped_lines = wrapped_lines.saturating_add(full_lines);
                used_width = remainder;
            }
        }
    }

    u16::try_from(wrapped_lines).unwrap_or(u16::MAX)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::Keymap;

    use super::super::bindings::BAND_COLUMNS_FOR_TEST as BAND_COLUMNS;
    use super::super::content::FavoriteRowsView;
    use super::*;
    use crate::app::App;
    use crate::app::AppPaneId;
    use crate::attract::AttractMode;
    use crate::favorites;
    use crate::favorites::FavoriteRowRecognition;
    use crate::keymap;

    const MOVING_BAND_ROW: &str = r#"
[[favorite]]
id = "01a03f60-9c14-7b41-8a02-1de4c7c9b332"
saved = "2026-08-26T11:02:44-07:00"
mode = "moving_band"
direction = "left"
width = 10
speed = 32
tail_speed = 72
fraying = "leading"
"#;

    const UNRECOGNIZED_ROW: &str = r#"
[[favorite]]
id = "01a03f62-9c14-7b41-8a02-1de4c7c9b334"
saved = "2026-08-26T14:31:05-07:00"
mode = "future_mode"
"#;

    fn keymap_from(toml: &str) -> Keymap<App> {
        let directory = TempDir::new().expect("temporary directory should be created");
        let path = directory.path().join("keymap.toml");
        if !toml.is_empty() {
            fs::write(&path, toml).expect("test keymap should be written");
        }
        let mut framework = Framework::new(FocusedPane::App(AppPaneId::Main));
        keymap::build_keymap(&mut framework, (!toml.is_empty()).then_some(path))
            .expect("test keymap should resolve")
    }

    fn display_column(line: &str, value: &str) -> usize {
        let byte_index = line
            .find(value)
            .expect("rendered table value should be present");
        UnicodeWidthStr::width(&line[..byte_index])
    }

    fn moving_band_table_layout(keymap: &Keymap<App>) -> FavoriteSectionTableLayout {
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from(&rows);
        let bindings = FavoritesSurfaceBindings::resolve(keymap);
        FavoriteSectionTableLayout::measure(&view.sections[0], &bindings)
    }

    #[test]
    fn exactly_fitting_parameter_column_is_visible() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let width = u16::try_from(
            CURSOR_WIDTH
                + usize::from(table_layout.saved_width)
                + COLUMN_GAP
                + usize::from(table_layout.parameter_widths.get(0)),
        )
        .expect("exact table width should fit u16");

        assert_eq!(table_layout.visible_parameter_columns(0, width), 0..1);
        let header = format_table_line(
            "Saved",
            &BAND_COLUMNS.map(|descriptor| descriptor.heading),
            table_layout.saved_width,
            &table_layout.parameter_widths,
            0..1,
        );
        assert_eq!(UnicodeWidthStr::width(header.as_str()), usize::from(width));
    }

    #[test]
    fn too_narrow_table_still_renders_one_clipped_parameter_column() {
        let keymap = keymap_from("");
        let table_layout = moving_band_table_layout(&keymap);
        let exact_width = usize::from(table_layout.saved_width)
            + CURSOR_WIDTH
            + COLUMN_GAP
            + usize::from(table_layout.parameter_widths.get(0));
        let width = u16::try_from(exact_width - 1).expect("narrow table width should fit u16");
        let visible = table_layout.visible_parameter_columns(0, width);

        assert_eq!(visible, 0..1);
        let headings = BAND_COLUMNS.map(|descriptor| descriptor.heading);
        let labels = FavoritesSurfaceBindings::resolve(&keymap)
            .column_labels(AttractMode::MovingBand)
            .to_vec();
        let rows = favorites::parse_rows_for_overlay_test(MOVING_BAND_ROW)
            .expect("moving-band fixture should parse");
        let view = FavoriteRowsView::from(&rows);
        let row = &view.sections[0].rows[0];
        let header = format_table_line(
            "Saved",
            &headings,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        );
        let key_line = format_table_line(
            "",
            &labels,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible.clone(),
        );
        let cell_line = format_table_line(
            &row.saved,
            &row.cells,
            table_layout.saved_width,
            &table_layout.parameter_widths,
            visible,
        );

        assert!(UnicodeWidthStr::width(header.as_str()) > usize::from(width));
        assert_eq!(
            display_column(&header, "Direction"),
            display_column(&key_line, "←↑↓→")
        );
        assert_eq!(
            display_column(&header, "Direction"),
            display_column(&cell_line, "left")
        );
    }

    #[test]
    fn navigation_indexes_only_rows_and_unrecognized_selection_uses_cursor_style() {
        let keymap = keymap_from("");
        let rows = favorites::parse_rows_for_overlay_test(&format!(
            "{MOVING_BAND_ROW}\n{UNRECOGNIZED_ROW}"
        ))
        .expect("mixed fixture should parse");
        let content = FavoritesOverlayContent::Rows(FavoriteRowsView::from(&rows));
        let bindings = FavoritesSurfaceBindings::resolve(&keymap);
        let plan = build_line_plan(&content, &bindings, 100, 0);

        assert_eq!(plan.navigation_line_index.len(), 2);
        assert_eq!(
            plan.navigation_line_index,
            plan.selectable_line_index().to_vec()
        );
        assert!(
            plan.navigation_line_index
                .iter()
                .all(|index| matches!(plan.lines[*index], CachedOverlayLine::Row { .. }))
        );

        let unrecognized_line = plan
            .navigation_line_index
            .iter()
            .find_map(|index| match &plan.lines[*index] {
                CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Unrecognized(_),
                    ..
                } => Some(&plan.lines[*index]),
                CachedOverlayLine::NonRow(_)
                | CachedOverlayLine::Row {
                    identity: FavoriteRowIdentity::Recognized(_),
                    ..
                } => None,
            })
            .expect("fixture should include an unrecognized row");
        let CachedOverlayLine::Row { identity, .. } = unrecognized_line else {
            panic!("selected line should be a row");
        };
        let rendered = rendered_line(
            unrecognized_line,
            &FavoriteSelection::Row(identity.clone()),
            FavoriteRowLifecycle::Active,
            Instant::now(),
        );

        assert_eq!(rendered.spans[0].content, "▸ ");
        assert_eq!(rendered.style, selection_style(PaneFocusState::Active));
    }

    #[test]
    fn removing_unrecognized_row_fades_from_its_error_color() {
        let rows = favorites::parse_rows_for_overlay_test(UNRECOGNIZED_ROW)
            .expect("unrecognized fixture should parse");
        let removal_locator = rows
            .iter()
            .find_map(|recognition| match recognition {
                FavoriteRowRecognition::Unrecognized {
                    removal_locator, ..
                } => Some(removal_locator.clone()),
                FavoriteRowRecognition::Recognized(_) => None,
            })
            .expect("fixture should include an unrecognized row");
        let line = CachedOverlayLine::Row {
            identity: FavoriteRowIdentity::Unrecognized(removal_locator),
            tail:     "unrecognized".to_string(),
        };
        let started = Instant::now();
        let active = rendered_line(
            &line,
            &FavoriteSelection::NoRowSelected,
            FavoriteRowLifecycle::Active,
            started,
        );
        let removing = rendered_line(
            &line,
            &FavoriteSelection::NoRowSelected,
            FavoriteRowLifecycle::Removing { since: started },
            started + FAVORITE_REMOVAL_FADE / 2,
        );

        assert_eq!(
            active.style.fg,
            Some(blend_color(error_color(), attract::ground(), 0))
        );
        assert_ne!(removing.style.fg, active.style.fg);
    }
}
