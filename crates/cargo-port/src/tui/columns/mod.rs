mod widths;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use unicode_width::UnicodeWidthStr;
pub(super) use widths::ColumnSpec;
pub(super) use widths::ColumnWidths;

use super::constants::COLUMN_HEADER_COLOR;
use super::constants::DISCOVERY_SHIMMER_COLOR;
use super::constants::ERROR_COLOR;
use super::constants::LABEL_COLOR;
use super::constants::SECONDARY_TEXT_COLOR;
use super::constants::TITLE_COLOR;
use super::render;
use crate::ci::Conclusion;
use crate::constants::GIT_IGNORED_COLOR;
use crate::constants::GIT_MODIFIED_COLOR;
use crate::constants::GIT_UNTRACKED_COLOR;
use crate::constants::IN_SYNC;
use crate::project::GitStatus;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;

// ── Column indices ──────────────────────────────────────────────────
pub(super) const COL_NAME: usize = 0;
pub(super) const COL_LINT: usize = 1;
pub(super) const COL_CI: usize = 2;
pub(super) const COL_LANG: usize = 3;
pub(super) const COL_GIT_PATH: usize = 4;
pub(super) const COL_SYNC: usize = 5;
pub(super) const COL_MAIN: usize = 6;
pub(super) const COL_DISK: usize = 7;
pub(super) const NUM_COLS: usize = 8;

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HeaderMode {
    Standard,
    BorrowLeft,
    Hidden,
}

#[derive(Clone, Copy)]
pub(super) struct ColumnDef {
    pub header:      &'static str,
    pub width:       ColumnWidth,
    pub align:       Align,
    pub gap:         usize,
    pub header_mode: HeaderMode,
}

impl ColumnDef {
    pub(super) fn seed_width(&self) -> usize {
        let base = match self.width {
            ColumnWidth::Fixed(width) | ColumnWidth::Fit { min: width } => width,
        };
        if matches!(self.width, ColumnWidth::Fit { .. }) && self.header_mode == HeaderMode::Standard
        {
            base.max(display_width(self.header))
        } else {
            base
        }
    }
}

/// The canonical column layout — single source of truth.
pub(super) const fn column_defs(lint_enabled: bool) -> [ColumnDef; NUM_COLS] {
    [
        // 0: Name
        ColumnDef {
            header:      "",
            width:       ColumnWidth::Fit { min: 10 },
            align:       Align::Left,
            gap:         0,
            header_mode: HeaderMode::Standard,
        },
        // 1: Lint — borrows "Li" from Name padding
        ColumnDef {
            header:      if lint_enabled { "Lint" } else { "" },
            width:       ColumnWidth::Fixed(if lint_enabled { 2 } else { 0 }),
            align:       Align::Left,
            gap:         0,
            header_mode: if lint_enabled {
                HeaderMode::BorrowLeft
            } else {
                HeaderMode::Hidden
            },
        },
        // 2: CI
        ColumnDef {
            header:      "CI",
            width:       ColumnWidth::Fixed(2),
            align:       Align::Left,
            gap:         1,
            header_mode: HeaderMode::Standard,
        },
        // 3: Lang
        ColumnDef {
            header:      "",
            width:       ColumnWidth::Fixed(2),
            align:       Align::Left,
            gap:         1,
            header_mode: HeaderMode::Hidden,
        },
        // 4: Git path status glyph, labeled as Git in the header.
        // Right-align so "t" sits above the right edge of the 2-wide emoji.
        ColumnDef {
            header:      "Git",
            width:       ColumnWidth::Fixed(2),
            align:       Align::Right,
            gap:         1,
            header_mode: HeaderMode::BorrowLeft,
        },
        // 5: Origin/upstream sync status
        ColumnDef {
            header:      "Og",
            width:       ColumnWidth::Fit { min: 0 },
            align:       Align::Right,
            gap:         1,
            header_mode: HeaderMode::Standard,
        },
        // 6: Local main delta
        ColumnDef {
            header:      "M",
            width:       ColumnWidth::Fit { min: 0 },
            align:       Align::Right,
            gap:         1,
            header_mode: HeaderMode::Standard,
        },
        // 7: Disk
        ColumnDef {
            header:      "Disk",
            width:       ColumnWidth::Fit { min: 4 },
            align:       Align::Right,
            gap:         0,
            header_mode: HeaderMode::Standard,
        },
    ]
}

// ── Cell / row types ────────────────────────────────────────────────

#[derive(Default)]
pub(super) struct CellContent {
    pub text:           String,
    pub style:          Style,
    pub segments:       Option<Vec<StyledSegment>>,
    pub align_override: Option<Align>,
    pub suffix:         Option<String>,
    pub suffix_style:   Option<Style>,
}

#[derive(Clone)]
pub(super) struct StyledSegment {
    pub text:  String,
    pub style: Style,
}

/// Resolved Lint column cell — bundles the icon glyph and its style so the
/// two cannot drift. Production code constructs one via [`App::lint_cell`]
/// (see `tui/app/lint.rs`), which derives both fields from a single
/// [`LintStatus`](crate::lint::LintStatus). Non-Rust child rows use
/// [`Self::hidden`]; tests/fixtures that just need a glyph use
/// [`Self::with_icon`].
#[derive(Clone, Copy)]
pub(super) struct LintCell {
    icon:  &'static str,
    style: Style,
}

impl LintCell {
    /// Empty cell — used for non-Rust child rows that have no lint state.
    pub(super) const fn hidden() -> Self {
        Self {
            icon:  " ",
            style: Style::new(),
        }
    }

    /// Test/fixture helper: a specific icon with the default style.
    /// Production code should use [`App::lint_cell`] instead so the style
    /// stays in sync with the status.
    #[cfg(test)]
    pub(super) const fn with_icon(icon: &'static str) -> Self {
        Self {
            icon,
            style: Style::new(),
        }
    }

    /// Construct from already-resolved icon + style. Visible to the App
    /// layer so [`App::lint_cell`] can populate both fields from a single
    /// [`LintStatus`](crate::lint::LintStatus).
    pub(super) const fn from_parts(icon: &'static str, style: Style) -> Self {
        Self { icon, style }
    }

    pub(super) const fn icon(&self) -> &'static str { self.icon }
    pub(super) const fn style(&self) -> Style { self.style }
}

#[derive(Clone)]
pub(super) struct ProjectRow<'a> {
    pub prefix:            &'a str,
    pub name:              &'a str,
    pub name_segments:     Option<Vec<StyledSegment>>,
    pub git_status:        Option<GitStatus>,
    pub lint:              LintCell,
    pub disk:              &'a str,
    pub disk_style:        Style,
    pub disk_suffix:       Option<&'a str>,
    pub disk_suffix_style: Option<Style>,
    pub lang_icon:         &'a str,
    pub git_origin_sync:   &'a str,
    pub git_main:          &'a str,
    pub ci:                Option<Conclusion>,
    pub deleted:           bool,
    pub worktree_health:   WorktreeHealth,
}

pub(super) struct RowCells {
    pub cells:           [CellContent; NUM_COLS],
    pub prefix:          String,
    pub deleted:         bool,
    pub worktree_health: WorktreeHealth,
}

// ── Resolved widths ─────────────────────────────────────────────────

/// Project-list column widths. Thin wrapper around the generic
/// [`ColumnWidths`] primitive that adds the lint-enabled flag,
/// the generation counter App uses to invalidate cached widths
/// after tree changes, and the project-list-specific seeding
/// from `column_defs`.
pub(super) struct ProjectListWidths {
    inner:          ColumnWidths,
    lint_enabled:   bool,
    pub generation: u64,
}

impl Default for ProjectListWidths {
    fn default() -> Self { Self::new(true) }
}

impl ProjectListWidths {
    /// Seed from column definitions: Fixed columns get their width,
    /// Fit columns get their minimum.
    pub(super) fn new(lint_enabled: bool) -> Self {
        Self {
            inner: ColumnWidths::new(project_list_specs(lint_enabled)),
            lint_enabled,
            generation: u64::MAX,
        }
    }

    /// Update a Fit column with observed content width. No-op for
    /// Fixed columns (`ColumnSpec::fixed` caps `max == min`).
    pub(super) fn observe(&mut self, col: usize, width: usize) {
        self.inner.observe_cell_usize(col, width);
    }

    /// Resolved width for a column.
    pub(super) fn get(&self, col: usize) -> usize { usize::from(self.inner.get(col)) }

    /// Total display width of all columns including gaps.
    pub(super) fn total_width(&self) -> usize {
        let defs = column_defs(self.lint_enabled);
        let mut total = 0;
        for (i, def) in defs.iter().enumerate() {
            total += def.gap + self.get(i);
        }
        total
    }

    pub const fn lint_enabled(&self) -> bool { self.lint_enabled }
}

/// Map the project-list `column_defs` into [`ColumnSpec`]s for
/// `ColumnWidths`.
fn project_list_specs(lint_enabled: bool) -> Vec<ColumnSpec> {
    column_defs(lint_enabled)
        .iter()
        .map(|def| {
            let seed = u16::try_from(def.seed_width()).unwrap_or(u16::MAX);
            match def.width {
                ColumnWidth::Fixed(_) => ColumnSpec::fixed(seed),
                ColumnWidth::Fit { .. } => ColumnSpec::fit(seed),
            }
        })
        .collect()
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
pub(super) fn row_to_line(row: &RowCells, widths: &ProjectListWidths) -> Line<'static> {
    let defs = column_defs(widths.lint_enabled());
    let mut spans = Vec::with_capacity(NUM_COLS);
    // Track which span indices are suffix spans (exempt from strikethrough).
    let mut suffix_indices: Vec<usize> = Vec::new();

    for (i, cell) in row.cells.iter().enumerate() {
        let col_width = widths.get(i);
        let align = cell.align_override.unwrap_or(defs[i].align);

        if col_width == 0 {
            spans.push(Span::styled(String::new(), cell.style));
            continue;
        }

        // Suffix handling: split the column into text + suffix spans.
        if let Some(suffix) = &cell.suffix {
            let suffix_w = display_width(suffix);
            let text_w = col_width.saturating_sub(suffix_w);
            let text_padded = pad_left(&cell.text, text_w);
            let gap = " ".repeat(defs[i].gap);
            spans.push(Span::styled(format!("{gap}{text_padded}"), cell.style));
            let suffix_style = cell.suffix_style.unwrap_or(cell.style);
            suffix_indices.push(spans.len());
            spans.push(Span::styled(suffix.clone(), suffix_style));
            continue;
        }

        if i == COL_NAME
            && let Some(segments) = &cell.segments
        {
            let prefix_w = display_width(&row.prefix);
            let available = col_width.saturating_sub(prefix_w);
            let content_w = segments
                .iter()
                .map(|segment| display_width(&segment.text))
                .sum();
            spans.push(Span::styled(row.prefix.clone(), cell.style));
            for segment in segments {
                spans.push(Span::styled(segment.text.clone(), segment.style));
            }
            let padding = available.saturating_sub(content_w);
            if padding > 0 {
                spans.push(Span::styled(" ".repeat(padding), cell.style));
            }
            continue;
        }

        let content = if i == COL_NAME {
            let prefix_w = display_width(&row.prefix);
            let available = col_width.saturating_sub(prefix_w);
            format!("{}{}", row.prefix, pad_right(&cell.text, available))
        } else if (i == COL_SYNC || i == COL_MAIN) && cell.text == IN_SYNC {
            let padded = pad_left(&cell.text, col_width);
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
            .fg(LABEL_COLOR)
            .add_modifier(Modifier::CROSSED_OUT);
        for (i, span) in spans.iter_mut().enumerate() {
            if !suffix_indices.contains(&i) {
                span.style = strike;
            }
        }
    } else if matches!(row.worktree_health, crate::project::WorktreeHealth::Broken) {
        let broken_style = Style::default().fg(Color::White).bg(ERROR_COLOR);
        for span in &mut spans {
            span.style = broken_style;
        }
    }

    Line::from(spans)
}

/// Build the header `Line` from column definitions and resolved widths.
/// `name_text` is the dynamic header for the Name column (e.g. "~/rust (42)").
pub(super) fn header_line(widths: &ProjectListWidths, name_text: &str) -> Line<'static> {
    let defs = column_defs(widths.lint_enabled());
    let header_style = Style::default()
        .fg(COLUMN_HEADER_COLOR)
        .add_modifier(Modifier::BOLD);

    let mut spans = Vec::with_capacity(NUM_COLS);
    let mut slot_widths =
        std::array::from_fn::<usize, NUM_COLS, _>(|i| defs[i].gap + widths.get(i));

    for (i, def) in defs.iter().enumerate() {
        if def.header_mode != HeaderMode::BorrowLeft {
            continue;
        }

        // Borrow overflow from the nearest columns on the left so headers can
        // stretch without shifting unrelated columns further left.
        let mut borrow_needed = display_width(def.header).saturating_sub(widths.get(i));
        let mut donor = i;
        while borrow_needed > 0 && donor > 0 {
            donor -= 1;
            let borrowed = slot_widths[donor].min(borrow_needed);
            slot_widths[donor] -= borrowed;
            slot_widths[i] += borrowed;
            borrow_needed -= borrowed;
        }
    }

    for (i, def) in defs.iter().enumerate() {
        let header = def.header;
        let slot_width = slot_widths[i];

        let content = if i == COL_NAME {
            pad_right(name_text, slot_width)
        } else if def.header_mode == HeaderMode::BorrowLeft {
            match def.align {
                Align::Left => pad_right(header, slot_width),
                Align::Right => pad_left(header, slot_width),
                Align::Center => pad_center(header, slot_width),
            }
        } else if def.header_mode == HeaderMode::Hidden {
            " ".repeat(slot_width)
        } else {
            let gap = def.gap.min(slot_width);
            let content_width = slot_width.saturating_sub(gap);
            let padded = match def.align {
                Align::Left => pad_right(header, content_width),
                Align::Right => pad_left(header, content_width),
                Align::Center => pad_center(header, content_width),
            };
            format!("{}{padded}", " ".repeat(gap))
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
    let git_path_icon = row.git_status.map_or("", GitStatus::icon);

    let compact_status_style = |value: &str| {
        if value == IN_SYNC {
            Style::default().fg(GIT_UNTRACKED_COLOR)
        } else {
            Style::default().fg(Color::White)
        }
    };

    let compact_status_align = |value: &str| {
        if value == IN_SYNC {
            Some(Align::Center)
        } else {
            None
        }
    };

    let origin_sync_style = compact_status_style(row.git_origin_sync);
    let main_style = compact_status_style(row.git_main);
    let origin_sync_align = compact_status_align(row.git_origin_sync);
    let main_align = compact_status_align(row.git_main);

    let name_style = project_name_style(row.git_status);
    let ci_style = render::conclusion_style(row.ci);
    let git_path_style = Style::default();

    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    cells[COL_NAME] = CellContent {
        text: String::from(row.name),
        style: name_style,
        segments: row.name_segments,
        align_override: None,
        ..CellContent::default()
    };
    cells[COL_LINT] = CellContent {
        text: String::from(row.lint.icon()),
        style: row.lint.style(),
        align_override: None,
        ..CellContent::default()
    };
    cells[COL_CI] = CellContent {
        text: ci_text,
        style: ci_style,
        align_override: None,
        ..CellContent::default()
    };
    cells[COL_LANG] = CellContent {
        text: String::from(row.lang_icon),
        style: Style::default(),
        align_override: None,
        ..CellContent::default()
    };
    cells[COL_GIT_PATH] = CellContent {
        text: String::from(git_path_icon),
        style: git_path_style,
        align_override: Some(Align::Center),
        ..CellContent::default()
    };
    cells[COL_SYNC] = CellContent {
        text: String::from(row.git_origin_sync),
        style: origin_sync_style,
        align_override: origin_sync_align,
        ..CellContent::default()
    };
    cells[COL_MAIN] = CellContent {
        text: String::from(row.git_main),
        style: main_style,
        align_override: main_align,
        ..CellContent::default()
    };
    cells[COL_DISK] = CellContent {
        text: String::from(row.disk),
        style: row.disk_style,
        align_override: None,
        suffix: row.disk_suffix.map(String::from),
        suffix_style: row.disk_suffix_style,
        ..CellContent::default()
    };

    RowCells {
        cells,
        prefix: String::from(row.prefix),
        deleted: row.deleted,
        worktree_health: row.worktree_health,
    }
}

pub(super) fn project_name_style(git_status: Option<GitStatus>) -> Style {
    match git_status {
        Some(GitStatus::Modified) => Style::default().fg(GIT_MODIFIED_COLOR),
        Some(GitStatus::Untracked) => Style::default().fg(GIT_UNTRACKED_COLOR),
        Some(GitStatus::Ignored) => Style::default().fg(GIT_IGNORED_COLOR),
        Some(GitStatus::Clean) | None => Style::default(),
    }
}

pub(super) fn project_name_shimmer_style(git_status: Option<GitStatus>) -> Style {
    match git_status {
        Some(GitStatus::Modified) => Style::default().fg(GIT_MODIFIED_COLOR),
        Some(GitStatus::Untracked) => Style::default().fg(GIT_UNTRACKED_COLOR),
        Some(GitStatus::Ignored) => Style::default().fg(SECONDARY_TEXT_COLOR),
        Some(GitStatus::Clean) | None => Style::default().fg(DISCOVERY_SHIMMER_COLOR),
    }
}

pub(super) fn build_shimmer_segments(
    name: &str,
    base_style: Style,
    accent_style: Style,
    head: usize,
    window_len: usize,
) -> Vec<StyledSegment> {
    let chars: Vec<char> = name.chars().collect();
    if chars.is_empty() || window_len == 0 {
        return vec![StyledSegment {
            text:  name.to_string(),
            style: base_style,
        }];
    }
    let len = chars.len();
    let head = head % len;
    let window_len = window_len.min(len);
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut highlighted = false;

    for (index, ch) in chars.iter().enumerate() {
        let is_highlighted = (index + len - head) % len < window_len;
        if current.is_empty() {
            highlighted = is_highlighted;
        } else if is_highlighted != highlighted {
            segments.push(StyledSegment {
                text:  std::mem::take(&mut current),
                style: if highlighted {
                    accent_style
                } else {
                    base_style
                },
            });
            highlighted = is_highlighted;
        }
        current.push(*ch);
    }

    if !current.is_empty() {
        segments.push(StyledSegment {
            text:  current,
            style: if highlighted {
                accent_style
            } else {
                base_style
            },
        });
    }

    segments
}

/// Build a `RowCells` for a group header (only Name column has content).
pub(super) fn build_group_header_cells(prefix: &str, label: &str) -> RowCells {
    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    cells[COL_NAME] = CellContent {
        text: String::from(label),
        style: Style::default().fg(TITLE_COLOR),
        align_override: None,
        ..CellContent::default()
    };
    RowCells {
        cells,
        prefix: String::from(prefix),
        deleted: false,
        worktree_health: Normal,
    }
}

fn summary_label_col(widths: &ProjectListWidths) -> usize {
    (0..COL_DISK)
        .rev()
        .find(|&col| widths.get(col) > 0)
        .unwrap_or(COL_NAME)
}

/// Build a `RowCells` for the summary (Σ) row.
pub(super) fn build_summary_cells(widths: &ProjectListWidths, disk: &str) -> RowCells {
    let total_style = Style::default()
        .fg(TITLE_COLOR)
        .add_modifier(Modifier::BOLD);

    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    let sigma_col = summary_label_col(widths);
    cells[sigma_col] = CellContent {
        text: String::from("Σ"),
        style: total_style,
        align_override: Some(Align::Right),
        ..CellContent::default()
    };
    cells[COL_DISK] = CellContent {
        text: String::from(disk),
        style: total_style,
        align_override: None,
        ..CellContent::default()
    };
    if sigma_col != COL_LANG {
        cells[COL_LANG] = CellContent {
            text: String::from("  "),
            style: Style::default(),
            align_override: None,
            ..CellContent::default()
        };
    }
    RowCells {
        cells,
        prefix: " ".repeat(widths.get(COL_NAME)),
        deleted: false,
        worktree_health: Normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::WorktreeHealth;

    fn seeded_width(index: usize) -> usize { column_defs(true)[index].seed_width() }

    #[test]
    fn resolved_widths_seeds_from_defs() {
        let widths = ProjectListWidths::new(true);
        // Fixed columns get their fixed width
        assert_eq!(widths.get(COL_LINT), seeded_width(COL_LINT));
        assert_eq!(widths.get(COL_LANG), seeded_width(COL_LANG));
        assert_eq!(widths.get(COL_CI), seeded_width(COL_CI));
        assert_eq!(widths.get(COL_GIT_PATH), seeded_width(COL_GIT_PATH));
        // Fit columns get their min
        assert_eq!(widths.get(COL_NAME), seeded_width(COL_NAME));
        assert_eq!(widths.get(COL_DISK), seeded_width(COL_DISK));
        assert_eq!(widths.get(COL_SYNC), seeded_width(COL_SYNC));
        assert_eq!(widths.get(COL_MAIN), seeded_width(COL_MAIN));
    }

    #[test]
    fn observe_grows_fit_columns() {
        let mut widths = ProjectListWidths::new(true);
        widths.observe(COL_NAME, 25);
        assert_eq!(widths.get(COL_NAME), 25);
        // Fixed column ignores observe
        widths.observe(COL_LINT, 99);
        assert_eq!(widths.get(COL_LINT), seeded_width(COL_LINT));
    }

    #[test]
    fn total_width_sums_gaps_and_widths() {
        let defs = column_defs(true);
        let widths = ProjectListWidths::new(true);
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
        let mut widths = ProjectListWidths::new(true);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let line = header_line(&widths, "Projects");

        assert_eq!(display_width(line.spans[COL_NAME].content.as_ref()), 28);
        assert_eq!(display_width(line.spans[COL_LINT].content.as_ref()), 4);
        assert_eq!(line.spans[COL_CI].content.as_ref(), " CI");
        assert_eq!(line.spans[COL_GIT_PATH].content.as_ref(), " Git");
        assert_eq!(line.spans[COL_SYNC].content.as_ref(), " Og");
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), "  M");
        assert_eq!(line.spans[COL_DISK].content.as_ref(), "    Disk");
        assert_eq!(line.width(), widths.total_width());
    }

    #[test]
    fn git_header_borrows_from_hidden_lang_column() {
        let mut widths = ProjectListWidths::new(true);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let line = header_line(&widths, "Projects");

        assert_eq!(line.spans[COL_CI].content.as_ref(), " CI");
        assert_eq!(display_width(line.spans[COL_LANG].content.as_ref()), 2);
        assert_eq!(line.spans[COL_GIT_PATH].content.as_ref(), " Git");
        assert_eq!(line.spans[COL_SYNC].content.as_ref(), " Og");
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), "  M");
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
        let mut widths = ProjectListWidths::new(true);
        widths.observe(COL_NAME, 32);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let row_emoji = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "bevy_brp 🌲:2",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(crate::constants::LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(Conclusion::Success),
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        let row_ascii = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "bevy_mesh_outline_benchmark",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(crate::constants::LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(Conclusion::Success),
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
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
        let mut widths = ProjectListWidths::new(true);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let row = build_summary_cells(&widths, "36.3 GiB");
        let line = row_to_line(&row, &widths);

        assert_eq!(
            line.spans[COL_NAME].content.as_ref(),
            " ".repeat(widths.get(COL_NAME))
        );
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), "  Σ");
        assert_eq!(line.spans[COL_CI].content.as_ref(), "   ");
        assert_eq!(line.spans[COL_DISK].content.as_ref(), "36.3 GiB");
    }

    #[test]
    fn lint_column_collapses_when_disabled() {
        let defs = column_defs(false);
        let mut widths = ProjectListWidths::new(false);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let header = header_line(&widths, "Projects");
        let row = build_summary_cells(&widths, "36.3 GiB");
        let line = row_to_line(&row, &widths);

        assert_eq!(defs[COL_LINT].header, "");
        assert_eq!(widths.get(COL_LINT), 0);
        assert_eq!(display_width(header.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(defs[COL_CI].header, "CI");
        assert_eq!(widths.get(COL_CI), 2);
        assert!(header.spans[COL_CI].content.as_ref().ends_with("CI"));
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), "  Σ");
    }

    #[test]
    fn hidden_lint_column_does_not_shift_ci_cells() {
        let mut widths = ProjectListWidths::new(false);
        widths.observe(COL_NAME, 24);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let row = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(crate::constants::LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(Conclusion::Success),
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        let line = row_to_line(&row, &widths);

        assert_eq!(display_width(line.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(
            line.spans[COL_CI].content.as_ref(),
            &format!(" {}", Conclusion::Success.icon())
        );
        assert_eq!(line.width(), widths.total_width());
    }

    #[test]
    fn git_status_changes_name_style() {
        let modified = build_row_cells(ProjectRow {
            prefix:            "  ",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Modified),
            lint:              LintCell::hidden(),
            disk:              "—",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "",
            git_main:          "",
            ci:                None,
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(modified.cells[COL_NAME].style.fg, Some(GIT_MODIFIED_COLOR));
        assert_eq!(
            modified.cells[COL_GIT_PATH].text,
            crate::constants::GIT_STATUS_MODIFIED
        );

        let untracked = build_row_cells(ProjectRow {
            prefix:            "  ",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Untracked),
            lint:              LintCell::hidden(),
            disk:              "—",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "",
            git_main:          "",
            ci:                None,
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(
            untracked.cells[COL_NAME].style.fg,
            Some(GIT_UNTRACKED_COLOR)
        );
        assert_eq!(
            untracked.cells[COL_GIT_PATH].text,
            crate::constants::GIT_STATUS_UNTRACKED
        );

        let clean = build_row_cells(ProjectRow {
            prefix:            "  ",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::hidden(),
            disk:              "—",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "",
            git_main:          "",
            ci:                None,
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(
            clean.cells[COL_GIT_PATH].text,
            crate::constants::GIT_STATUS_CLEAN
        );

        let ignored = build_row_cells(ProjectRow {
            prefix:            "  ",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Ignored),
            lint:              LintCell::hidden(),
            disk:              "—",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "",
            git_main:          "",
            ci:                None,
            deleted:           false,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(ignored.cells[COL_NAME].style.fg, Some(GIT_IGNORED_COLOR));
        assert!(ignored.cells[COL_GIT_PATH].text.is_empty());
    }

    #[test]
    fn build_shimmer_segments_wraps_around_name_end() {
        let segments = build_shimmer_segments(
            "abcd",
            Style::default(),
            Style::default().fg(TITLE_COLOR),
            3,
            2,
        );

        let actual: Vec<_> = segments
            .iter()
            .map(|segment| (segment.text.as_str(), segment.style.fg))
            .collect();
        assert_eq!(
            actual,
            vec![
                ("a", Some(TITLE_COLOR)),
                ("bc", None),
                ("d", Some(TITLE_COLOR)),
            ]
        );
    }

    #[test]
    fn shimmer_style_never_uses_bold() {
        for state in [
            Some(GitStatus::Clean),
            Some(GitStatus::Modified),
            Some(GitStatus::Untracked),
            Some(GitStatus::Ignored),
            None,
        ] {
            assert!(
                !project_name_shimmer_style(state)
                    .add_modifier
                    .contains(Modifier::BOLD)
            );
        }
    }

    #[test]
    fn clean_shimmer_style_uses_explicit_high_contrast_foreground() {
        assert_eq!(
            project_name_shimmer_style(Some(GitStatus::Clean)).fg,
            Some(DISCOVERY_SHIMMER_COLOR)
        );
        assert_eq!(
            project_name_shimmer_style(None).fg,
            Some(DISCOVERY_SHIMMER_COLOR)
        );
    }
}
