use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;

use crate::ci::Conclusion;
use crate::constants::GIT_CLONE;
use crate::constants::GIT_FORK;
use crate::constants::GIT_LOCAL;
use crate::constants::IN_SYNC;

// ── Column indices ──────────────────────────────────────────────────
pub(super) const COL_NAME: usize = 0;
pub(super) const COL_LINT: usize = 1;
pub(super) const COL_DISK: usize = 2;
pub(super) const COL_LANG: usize = 3;
pub(super) const COL_SYNC: usize = 4;
pub(super) const COL_GIT: usize = 5;
pub(super) const COL_CI: usize = 6;
pub(super) const NUM_COLS: usize = 7;

// ── Column definition types ─────────────────────────────────────────

#[derive(Clone, Copy)]
pub(super) enum ColumnWidth {
    Fixed(usize),
    Fit { min: usize },
}

#[derive(Clone, Copy)]
pub(super) enum Align {
    Left,
    Right,
    Center,
}

#[derive(Clone, Copy)]
pub(super) struct ColumnDef {
    pub header:              &'static str,
    pub width:               ColumnWidth,
    pub align:               Align,
    pub gap:                 usize,
    pub header_borrows_left: bool,
}

/// The canonical column layout — single source of truth.
pub(super) const fn column_defs(lint_enabled: bool) -> [ColumnDef; NUM_COLS] {
    [
        // 0: Name
        ColumnDef {
            header:              "",
            width:               ColumnWidth::Fit { min: 10 },
            align:               Align::Left,
            gap:                 0,
            header_borrows_left: false,
        },
        // 1: Lint — borrows "Li" from Name padding
        ColumnDef {
            header:              if lint_enabled { "Lint" } else { "" },
            width:               ColumnWidth::Fixed(if lint_enabled { 2 } else { 0 }),
            align:               Align::Left,
            gap:                 0,
            header_borrows_left: lint_enabled,
        },
        // 2: Disk
        ColumnDef {
            header:              "Disk",
            width:               ColumnWidth::Fit { min: 4 },
            align:               Align::Right,
            gap:                 1,
            header_borrows_left: false,
        },
        // 3: Lang
        ColumnDef {
            header:              "R",
            width:               ColumnWidth::Fixed(2),
            align:               Align::Left,
            gap:                 1,
            header_borrows_left: false,
        },
        // 4: Sync
        ColumnDef {
            header:              "",
            width:               ColumnWidth::Fit { min: 0 },
            align:               Align::Left,
            gap:                 1,
            header_borrows_left: false,
        },
        // 5: Git
        ColumnDef {
            header:              "Git",
            width:               ColumnWidth::Fixed(2),
            align:               Align::Left,
            gap:                 1,
            header_borrows_left: false,
        },
        // 6: CI
        ColumnDef {
            header:              "CI",
            width:               ColumnWidth::Fixed(2),
            align:               Align::Left,
            gap:                 1,
            header_borrows_left: false,
        },
    ]
}

// ── Cell / row types ────────────────────────────────────────────────

#[derive(Default)]
pub(super) struct CellContent {
    pub text:           String,
    pub style:          Style,
    pub align_override: Option<Align>,
}

#[derive(Clone, Copy)]
pub(super) struct ProjectRow<'a> {
    pub prefix:     &'a str,
    pub name:       &'a str,
    pub lint_icon:  &'a str,
    pub disk:       &'a str,
    pub disk_style: Style,
    pub lang_icon:  &'a str,
    pub git_sync:   &'a str,
    pub git_icon:   &'a str,
    pub ci:         Option<Conclusion>,
    pub deleted:    bool,
}

pub(super) struct RowCells {
    pub cells:   [CellContent; NUM_COLS],
    pub prefix:  String,
    pub deleted: bool,
}

// ── Resolved widths ─────────────────────────────────────────────────

pub(super) struct ResolvedWidths {
    widths:         [usize; NUM_COLS],
    lint_enabled:   bool,
    pub generation: u64,
}

impl Default for ResolvedWidths {
    fn default() -> Self { Self::new(true) }
}

impl ResolvedWidths {
    /// Seed from column definitions: Fixed columns get their width, Fit columns
    /// get their minimum.
    pub(super) fn new(lint_enabled: bool) -> Self {
        let defs = column_defs(lint_enabled);
        let mut widths = [0usize; NUM_COLS];
        for (i, def) in defs.iter().enumerate() {
            widths[i] = match def.width {
                ColumnWidth::Fixed(w) => w,
                ColumnWidth::Fit { min } => min,
            };
        }
        Self {
            widths,
            lint_enabled,
            generation: u64::MAX,
        }
    }

    /// Update a Fit column with observed content width. No-op for Fixed columns.
    pub(super) fn observe(&mut self, col: usize, width: usize) {
        if let ColumnWidth::Fit { .. } = column_defs(self.lint_enabled)[col].width {
            self.widths[col] = self.widths[col].max(width);
        }
    }

    /// Get the resolved width for a column.
    pub(super) const fn get(&self, col: usize) -> usize { self.widths[col] }

    /// Total display width of all columns including gaps.
    pub(super) fn total_width(&self) -> usize {
        let defs = column_defs(self.lint_enabled);
        let mut total = 0;
        for (i, def) in defs.iter().enumerate() {
            total += def.gap + self.widths[i];
        }
        total
    }

    pub(super) const fn lint_enabled(&self) -> bool { self.lint_enabled }
}

// ── Display-width helpers ───────────────────────────────────────────

/// Terminal display width of a string, accounting for multi-byte and wide
/// characters. Use this for ALL layout calculations — never `.len()`.
pub(super) fn display_width(s: &str) -> usize { UnicodeWidthStr::width(s) }

/// Pad a string to a target display width using trailing spaces (left-aligned).
pub(super) fn pad_right(s: &str, target: usize) -> String {
    let w = display_width(s);
    let pad = target.saturating_sub(w);
    format!("{s}{}", " ".repeat(pad))
}

/// Pad a string to a target display width using leading spaces (right-aligned).
pub(super) fn pad_left(s: &str, target: usize) -> String {
    let w = display_width(s);
    let pad = target.saturating_sub(w);
    format!("{}{s}", " ".repeat(pad))
}

/// Pad a string to a target display width, centered.
fn pad_center(s: &str, target: usize) -> String {
    let w = display_width(s);
    let total_pad = target.saturating_sub(w);
    let left = total_pad / 2;
    let right = total_pad - left;
    format!("{}{s}{}", " ".repeat(left), " ".repeat(right))
}

// ── Row rendering ───────────────────────────────────────────────────

/// Render a `RowCells` into a styled `Line` using the column definitions and
/// resolved widths. Replaces `project_row_spans`.
pub(super) fn row_to_line(row: &RowCells, widths: &ResolvedWidths) -> Line<'static> {
    let defs = column_defs(widths.lint_enabled());
    let mut spans = Vec::with_capacity(NUM_COLS);

    for (i, cell) in row.cells.iter().enumerate() {
        let col_width = widths.get(i);
        let align = cell.align_override.unwrap_or(defs[i].align);

        let content = if col_width == 0 {
            String::new()
        } else if i == COL_NAME {
            let prefix_w = display_width(&row.prefix);
            let available = col_width.saturating_sub(prefix_w);
            format!("{}{}", row.prefix, pad_right(&cell.text, available))
        } else if i == COL_SYNC && cell.text == IN_SYNC {
            // IN_SYNC gets special centering treatment
            let padded = if col_width <= 2 {
                pad_left(&cell.text, col_width)
            } else {
                pad_center(&cell.text, col_width)
            };
            format!("{}{padded}", " ".repeat(defs[i].gap))
        } else {
            let padded = match align {
                Align::Left => pad_right(&cell.text, col_width),
                Align::Right => pad_left(&cell.text, col_width),
                Align::Center => pad_center(&cell.text, col_width),
            };
            format!("{}{padded}", " ".repeat(defs[i].gap))
        };

        spans.push(Span::styled(content, cell.style));
    }

    if row.deleted {
        let strike = Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::CROSSED_OUT);
        for span in &mut spans {
            span.style = strike;
        }
    }

    Line::from(spans)
}

/// Build the header `Line` from column definitions and resolved widths.
/// `name_text` is the dynamic header for the Name column (e.g. "~/rust (42)").
pub(super) fn header_line(widths: &ResolvedWidths, name_text: &str) -> Line<'static> {
    let defs = column_defs(widths.lint_enabled());
    let header_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut spans = Vec::with_capacity(NUM_COLS);

    // The borrowing header only needs to steal the part that overflows its
    // actual column width. For "Lint" over a 2-char column, only 2 chars
    // borrow from Name.
    let borrow_overflow = if defs[COL_LINT].header_borrows_left {
        display_width(defs[COL_LINT].header).saturating_sub(widths.get(COL_LINT))
    } else {
        0
    };
    for i in 0..NUM_COLS {
        let header = defs[i].header;

        let content = if i == COL_NAME {
            let available = widths.get(i).saturating_sub(borrow_overflow);
            pad_right(name_text, available)
        } else if defs[i].header_borrows_left {
            // Borrowing column renders at its full header text width.
            let header_w = display_width(header);
            let padded = match defs[i].align {
                Align::Left => pad_right(header, header_w),
                Align::Right => pad_left(header, header_w),
                Align::Center => pad_center(header, header_w),
            };
            format!("{}{padded}", " ".repeat(defs[i].gap))
        } else if i == COL_SYNC && defs[i].header.is_empty() {
            // "Git" labels the sync+git region. Start it over the first sync
            // character cell so it visually covers the combined status area.
            format!(" {}", defs[COL_GIT].header)
        } else if i == COL_GIT {
            // The "Git" label is rendered from the preceding blank Sync header
            // span, so this span only contributes the git column's data width.
            " ".repeat(widths.get(i))
        } else {
            let padded = match defs[i].align {
                Align::Left => pad_right(header, widths.get(i)),
                Align::Right => pad_left(header, widths.get(i)),
                Align::Center => pad_center(header, widths.get(i)),
            };
            format!("{}{padded}", " ".repeat(defs[i].gap))
        };

        spans.push(Span::styled(content, header_style));
    }

    Line::from(spans)
}

// ── Row construction helpers ────────────────────────────────────────

/// Build a `RowCells` for a project row. Single construction site replaces all
/// scattered project row literals.
pub(super) fn build_row_cells(row: ProjectRow<'_>) -> RowCells {
    let ci_text = row
        .ci
        .map_or(String::new(), |conclusion| String::from(conclusion.icon()));

    let ci_style = super::render::conclusion_style(row.ci);
    let origin_style = match row.git_icon {
        GIT_FORK => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        GIT_CLONE => Style::default().fg(Color::White),
        GIT_LOCAL => Style::default().fg(Color::DarkGray),
        _ => Style::default(),
    };
    let sync_style = if row.git_sync == IN_SYNC {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::White)
    };

    let sync_align = if row.git_sync == IN_SYNC {
        Some(Align::Center)
    } else {
        None
    };

    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    cells[COL_NAME] = CellContent {
        text:           String::from(row.name),
        style:          Style::default(),
        align_override: None,
    };
    cells[COL_LINT] = CellContent {
        text:           String::from(row.lint_icon),
        style:          Style::default(),
        align_override: None,
    };
    cells[COL_DISK] = CellContent {
        text:           String::from(row.disk),
        style:          row.disk_style,
        align_override: None,
    };
    cells[COL_LANG] = CellContent {
        text:           String::from(row.lang_icon),
        style:          Style::default(),
        align_override: None,
    };
    cells[COL_SYNC] = CellContent {
        text:           String::from(row.git_sync),
        style:          sync_style,
        align_override: sync_align,
    };
    cells[COL_GIT] = CellContent {
        text:           String::from(row.git_icon),
        style:          origin_style,
        align_override: None,
    };
    cells[COL_CI] = CellContent {
        text:           ci_text,
        style:          ci_style,
        align_override: None,
    };

    RowCells {
        cells,
        prefix: String::from(row.prefix),
        deleted: row.deleted,
    }
}

/// Build a `RowCells` for a group header (only Name column has content).
pub(super) fn build_group_header_cells(prefix: &str, label: &str) -> RowCells {
    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    cells[COL_NAME] = CellContent {
        text:           String::from(label),
        style:          Style::default().fg(Color::Yellow),
        align_override: None,
    };
    RowCells {
        cells,
        prefix: String::from(prefix),
        deleted: false,
    }
}

/// Build a `RowCells` for the summary (Σ) row.
pub(super) fn build_summary_cells(name_width: usize, disk: &str, lint_enabled: bool) -> RowCells {
    let total_style = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    if lint_enabled {
        cells[COL_LINT] = CellContent {
            text:           String::from("Σ"),
            style:          total_style,
            align_override: Some(Align::Right),
        };
    } else {
        cells[COL_NAME] = CellContent {
            text:           String::from("Σ"),
            style:          total_style,
            align_override: None,
        };
    }
    cells[COL_DISK] = CellContent {
        text:           String::from(disk),
        style:          total_style,
        align_override: None,
    };
    cells[COL_LANG] = CellContent {
        text:           String::from("  "),
        style:          Style::default(),
        align_override: None,
    };
    cells[COL_GIT] = CellContent {
        text:           String::from(" "),
        style:          Style::default(),
        align_override: None,
    };

    RowCells {
        cells,
        prefix: if lint_enabled {
            " ".repeat(name_width)
        } else {
            " ".repeat(name_width.saturating_sub(1))
        },
        deleted: false,
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::*;

    #[test]
    fn resolved_widths_seeds_from_defs() {
        let widths = ResolvedWidths::new(true);
        // Fixed columns get their fixed width
        assert_eq!(widths.get(COL_LINT), 2);
        assert_eq!(widths.get(COL_LANG), 2);
        assert_eq!(widths.get(COL_GIT), 2);
        assert_eq!(widths.get(COL_CI), 2);
        // Fit columns get their min
        assert_eq!(widths.get(COL_NAME), 10);
        assert_eq!(widths.get(COL_DISK), 4);
        assert_eq!(widths.get(COL_SYNC), 0);
    }

    #[test]
    fn observe_grows_fit_columns() {
        let mut widths = ResolvedWidths::new(true);
        widths.observe(COL_NAME, 25);
        assert_eq!(widths.get(COL_NAME), 25);
        // Fixed column ignores observe
        widths.observe(COL_LINT, 99);
        assert_eq!(widths.get(COL_LINT), 2);
    }

    #[test]
    fn total_width_sums_gaps_and_widths() {
        let defs = column_defs(true);
        let widths = ResolvedWidths::new(true);
        let total = widths.total_width();
        let expected: usize = defs
            .iter()
            .enumerate()
            .map(|(i, d)| d.gap + widths.get(i))
            .sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn header_line_borrows_only_overflow_from_name() {
        let mut widths = ResolvedWidths::new(true);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);

        let line = header_line(&widths, "Projects");

        assert_eq!(display_width(line.spans[COL_NAME].content.as_ref()), 28);
        assert_eq!(display_width(line.spans[COL_LINT].content.as_ref()), 4);
        assert_eq!(line.spans[COL_SYNC].content.as_ref(), " Git");
        assert_eq!(display_width(line.spans[COL_GIT].content.as_ref()), 2);
        assert_eq!(line.width(), widths.total_width());
    }

    #[test]
    fn emoji_display_widths() {
        assert_eq!(display_width("🌲"), 2);
        assert_eq!(display_width("🦀"), 2);
        assert_eq!(display_width("bevy_brp"), 8);
        assert_eq!(display_width("bevy_brp 🌲:2"), 13);

        let padded = pad_right("bevy_brp 🌲:2", 27);
        assert_eq!(display_width(&padded), 27, "padded display width");

        let padded_ascii = pad_right("bevy_brp", 27);
        assert_eq!(
            display_width(&padded_ascii),
            27,
            "ascii padded display width"
        );
    }

    #[test]
    fn row_to_line_same_width_with_and_without_emoji() {
        let mut widths = ResolvedWidths::new(true);
        widths.observe(COL_NAME, 32);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);

        let row_emoji = build_row_cells(ProjectRow {
            prefix:     "▶ ",
            name:       "bevy_brp 🌲:2",
            lint_icon:  crate::constants::LINT_PASSED,
            disk:       "36.3 GiB",
            disk_style: Style::default(),
            lang_icon:  "🦀",
            git_sync:   "↑2",
            git_icon:   crate::constants::GIT_CLONE,
            ci:         Some(Conclusion::Success),
            deleted:    false,
        });
        let row_ascii = build_row_cells(ProjectRow {
            prefix:     "▶ ",
            name:       "bevy_mesh_outline_benchmark",
            lint_icon:  crate::constants::LINT_PASSED,
            disk:       "36.3 GiB",
            disk_style: Style::default(),
            lang_icon:  "🦀",
            git_sync:   "↑2",
            git_icon:   crate::constants::GIT_CLONE,
            ci:         Some(Conclusion::Success),
            deleted:    false,
        });

        let line_emoji = row_to_line(&row_emoji, &widths);
        let line_ascii = row_to_line(&row_ascii, &widths);

        let emoji_spans: Vec<usize> = line_emoji
            .spans
            .iter()
            .map(|s| display_width(s.content.as_ref()))
            .collect();
        let ascii_spans: Vec<usize> = line_ascii
            .spans
            .iter()
            .map(|s| display_width(s.content.as_ref()))
            .collect();
        assert_eq!(
            emoji_spans, ascii_spans,
            "per-span widths should match\nemoji: {emoji_spans:?}\nascii: {ascii_spans:?}"
        );
    }

    #[test]
    fn summary_row_places_sigma_next_to_disk_total() {
        let mut widths = ResolvedWidths::new(true);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);

        let row = build_summary_cells(widths.get(COL_NAME), "36.3 GiB", true);
        let line = row_to_line(&row, &widths);

        assert_eq!(
            line.spans[COL_NAME].content.as_ref(),
            " ".repeat(widths.get(COL_NAME))
        );
        assert_eq!(line.spans[COL_LINT].content.as_ref(), " Σ");
        assert_eq!(line.spans[COL_DISK].content.as_ref(), " 36.3 GiB");
    }

    #[test]
    fn lint_column_collapses_when_disabled() {
        let defs = column_defs(false);
        let mut widths = ResolvedWidths::new(false);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);

        let header = header_line(&widths, "Projects");
        let row = build_summary_cells(widths.get(COL_NAME), "36.3 GiB", false);
        let line = row_to_line(&row, &widths);

        assert_eq!(defs[COL_LINT].header, "");
        assert_eq!(widths.get(COL_LINT), 0);
        assert_eq!(display_width(header.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(defs[COL_CI].header, "CI");
        assert_eq!(widths.get(COL_CI), 2);
        assert!(header.spans[COL_CI].content.as_ref().ends_with("CI"));
        assert!(line.spans[COL_NAME].content.as_ref().ends_with('Σ'));
    }

    #[test]
    fn hidden_lint_column_does_not_shift_ci_cells() {
        let mut widths = ResolvedWidths::new(false);
        widths.observe(COL_NAME, 24);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);

        let row = build_row_cells(ProjectRow {
            prefix:     "▶ ",
            name:       "demo",
            lint_icon:  crate::constants::LINT_PASSED,
            disk:       "36.3 GiB",
            disk_style: Style::default(),
            lang_icon:  "🦀",
            git_sync:   "↑2",
            git_icon:   crate::constants::GIT_CLONE,
            ci:         Some(Conclusion::Success),
            deleted:    false,
        });
        let line = row_to_line(&row, &widths);

        assert_eq!(display_width(line.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(
            line.spans[COL_CI].content.as_ref(),
            &format!(" {}", Conclusion::Success.icon())
        );
        assert_eq!(line.width(), widths.total_width());
    }
}
