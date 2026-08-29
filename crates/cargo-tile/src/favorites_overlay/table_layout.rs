//! Column measurement and row formatting for one attract mode's favorites table.

use std::ops::Range;

use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use unicode_width::UnicodeWidthStr;

use super::bindings::FavoritesSurfaceBindings;
use super::constants::COLUMN_GAP;
use super::constants::FAVORITE_ROW_PREFIX_WIDTH;
use super::content::FavoriteModeSection;
use super::content::FavoriteRowView;
use super::parameter_column;
use super::parameter_column::ParameterColumnDescriptor;

#[derive(Clone, Debug)]
pub(super) struct FavoriteSectionTableLayout {
    pub(super) saved_width:      u16,
    pub(super) parameter_widths: ColumnWidths,
}

impl FavoriteSectionTableLayout {
    pub(super) fn measure(
        section: &FavoriteModeSection,
        bindings: &FavoritesSurfaceBindings,
    ) -> Self {
        let descriptors = parameter_column::column_descriptors(section.mode);
        let key_labels = bindings.column_labels(section.mode);
        Self {
            saved_width:      measured_saved_width(&section.rows),
            parameter_widths: measured_parameter_widths(descriptors, key_labels, &section.rows),
        }
    }

    pub(super) fn visible_parameter_columns(
        &self,
        horizontal_page: usize,
        width: u16,
    ) -> Range<usize> {
        visible_parameter_columns(
            horizontal_page,
            width,
            self.saved_width,
            &self.parameter_widths,
        )
    }

    pub(super) fn last_horizontal_column_page(&self, width: u16) -> usize {
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
        for (column, descriptor) in descriptors.iter().enumerate() {
            let value = descriptor.render_value(row.settings);
            widths.observe_cell_usize(column, UnicodeWidthStr::width(value.as_str()));
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
    let pinned = FAVORITE_ROW_PREFIX_WIDTH.saturating_add(usize::from(saved_width));
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

pub(super) fn format_table_line<T: AsRef<str>>(
    saved: &str,
    cells: &[T],
    saved_width: u16,
    parameter_widths: &ColumnWidths,
    visible: Range<usize>,
) -> String {
    format!(
        "{}{}",
        " ".repeat(FAVORITE_ROW_PREFIX_WIDTH),
        format_table_tail(saved, cells, saved_width, parameter_widths, visible)
    )
}

pub(super) fn format_table_tail<T: AsRef<str>>(
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

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::TempDir;
    use tui_pane::FocusedPane;
    use tui_pane::Framework;
    use tui_pane::Keymap;

    use super::*;
    use crate::app::App;
    use crate::app::AppPaneId;
    use crate::attract::AttractMode;
    use crate::favorites;
    use crate::favorites_overlay::content::FavoriteRowsView;
    use crate::favorites_overlay::parameter_column::BAND_COLUMNS_FOR_TEST as BAND_COLUMNS;
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
            FAVORITE_ROW_PREFIX_WIDTH
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
            + FAVORITE_ROW_PREFIX_WIDTH
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
        let cells = BAND_COLUMNS.map(|descriptor| descriptor.render_value(row.settings));
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
            &cells,
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
}
