use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::error_color;
use tui_pane::label_color;
use tui_pane::on_error_color;
use tui_pane::secondary_text_color;
use tui_pane::text_default;
use tui_pane::title_color;
use unicode_width::UnicodeWidthStr;

use super::constants::COL_CI;
use super::constants::COL_DISK;
use super::constants::COL_GIT_PATH;
use super::constants::COL_LANG;
use super::constants::COL_LINT;
use super::constants::COL_MAIN;
use super::constants::COL_NAME;
use super::constants::COL_SYNC;
use super::constants::NUM_COLS;
use crate::ci::CiStatus;
use crate::config::LintIndicator;
use crate::constants::IN_SYNC;
use crate::project::GitStatus;
use crate::project::WorktreeHealth;
use crate::project::WorktreeHealth::Normal;
use crate::tui::render;
use crate::tui::theme_roles;

// Imported `COL_*` constants index the `ColumnDef` array returned by `column_defs`.

// `ColumnWidth`, `Align` and `HeaderMode` describe each `ColumnDef` layout policy.

#[derive(Clone, Copy)]
pub(super) enum ColumnWidth {
    Fixed(usize),
    Fit { min: usize },
}

#[derive(Clone, Copy)]
pub(in crate::tui) enum Align {
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
    /// Header labels in ascending width order. The renderer picks the longest
    /// label whose display width fits the resolved column. Single-element
    /// slices behave like a static header.
    pub header_levels: &'static [&'static str],
    pub width:         ColumnWidth,
    pub align:         Align,
    pub gap:           usize,
    pub header_mode:   HeaderMode,
}

impl ColumnDef {
    /// Shortest label — defines the column's minimum header footprint.
    pub(super) fn header_min(&self) -> &'static str {
        self.header_levels.first().copied().unwrap_or("")
    }

    /// Longest label — used by `BorrowLeft` columns when computing how much
    /// space to borrow from neighbors.
    pub(super) fn header_max(&self) -> &'static str {
        self.header_levels.last().copied().unwrap_or("")
    }

    /// Pick the longest label whose display width fits `width`. Falls back to
    /// the shortest label so the header is never empty when a label exists.
    pub(super) fn header_for_width(&self, width: usize) -> &'static str {
        self.header_levels
            .iter()
            .rev()
            .copied()
            .find(|label| display_width(label) <= width)
            .unwrap_or_else(|| self.header_min())
    }

    pub(super) fn seed_width(&self) -> usize {
        let base = match self.width {
            ColumnWidth::Fixed(width) | ColumnWidth::Fit { min: width } => width,
        };
        if matches!(self.width, ColumnWidth::Fit { .. }) && self.header_mode == HeaderMode::Standard
        {
            base.max(display_width(self.header_min()))
        } else {
            base
        }
    }
}

/// The canonical column layout — single source of truth.
pub(super) const fn column_defs(lint_column: LintColumnStatus) -> [ColumnDef; NUM_COLS] {
    let (lint_headers, lint_width, lint_header_mode): (&[&str], usize, HeaderMode) =
        match lint_column {
            LintColumnStatus::Enabled => (&["Lint"], 2, HeaderMode::BorrowLeft),
            LintColumnStatus::Hidden => (&[""], 0, HeaderMode::Hidden),
        };
    [
        // `COL_NAME` uses this `ColumnDef` for the project name.
        ColumnDef {
            header_levels: &[""],
            width:         ColumnWidth::Fit { min: 10 },
            align:         Align::Left,
            gap:           0,
            header_mode:   HeaderMode::Standard,
        },
        // `COL_LINT` uses `HeaderMode::BorrowLeft` to borrow "Li" from `COL_NAME` padding.
        ColumnDef {
            header_levels: lint_headers,
            width:         ColumnWidth::Fixed(lint_width),
            align:         Align::Left,
            gap:           0,
            header_mode:   lint_header_mode,
        },
        // `COL_CI` reserves two columns for the CI glyph.
        ColumnDef {
            header_levels: &["CI"],
            width:         ColumnWidth::Fixed(2),
            align:         Align::Left,
            gap:           1,
            header_mode:   HeaderMode::Standard,
        },
        // `COL_LANG` reserves two columns with `HeaderMode::Hidden`.
        ColumnDef {
            header_levels: &[""],
            width:         ColumnWidth::Fixed(2),
            align:         Align::Left,
            gap:           1,
            header_mode:   HeaderMode::Hidden,
        },
        // `COL_GIT_PATH` labels its status glyph "Git"; `Align::Right` places "t" above the right
        // edge of the two-column emoji.
        ColumnDef {
            header_levels: &["Git"],
            width:         ColumnWidth::Fixed(2),
            align:         Align::Right,
            gap:           1,
            header_mode:   HeaderMode::BorrowLeft,
        },
        // `COL_SYNC` shows origin/upstream sync status, promoting "Og" to "Orig" to "Origin" from
        // `header_levels` as `ColumnWidth::Fit` widens to fit cell content.
        ColumnDef {
            header_levels: &["Og", "Orig", "Origin"],
            width:         ColumnWidth::Fit { min: 0 },
            align:         Align::Right,
            gap:           1,
            header_mode:   HeaderMode::Standard,
        },
        // `COL_MAIN` shows local main delta, promoting "M" to "Mn" to "Main" from `header_levels`
        // as `ColumnWidth::Fit` widens.
        ColumnDef {
            header_levels: &["M", "Mn", "Main"],
            width:         ColumnWidth::Fit { min: 0 },
            align:         Align::Right,
            gap:           1,
            header_mode:   HeaderMode::Standard,
        },
        // `COL_DISK` sets `gap: 1` so its size stays separated even when `COL_MAIN` is nonempty
        // (for example "↑5" beside "107.7 GiB").
        ColumnDef {
            header_levels: &["Disk"],
            width:         ColumnWidth::Fit { min: 4 },
            align:         Align::Right,
            gap:           1,
            header_mode:   HeaderMode::Standard,
        },
    ]
}

// `CellContent`, `StyledSegment`, `LintCell` and `ProjectRow` hold styled table output.

#[derive(Default)]
pub(in crate::tui) struct CellContent {
    pub text:           String,
    pub style:          Style,
    pub segments:       Option<Vec<StyledSegment>>,
    pub align_override: Option<Align>,
    pub suffix:         Option<String>,
    pub suffix_style:   Option<Style>,
}

#[derive(Clone)]
pub(in crate::tui) struct StyledSegment {
    pub text:  String,
    pub style: Style,
}

/// Resolved Lint column cell — bundles the icon glyph and its style so the
/// two cannot drift. Production code constructs one via `App::lint_cell`
/// (see `tui/app/lint.rs`), which derives both fields from a single
/// [`LintStatus`](crate::lint::LintStatus). Non-Rust child rows use
/// [`Self::hidden`]; tests/fixtures that just need a glyph use
/// `Self::with_icon`.
#[derive(Clone, Copy)]
pub(in crate::tui) struct LintCell {
    icon:  &'static str,
    style: Style,
}

impl LintCell {
    /// Empty cell — used for non-Rust child rows that have no lint state.
    pub(in crate::tui) const fn hidden() -> Self {
        Self {
            icon:  " ",
            style: Style::new(),
        }
    }

    /// Test/fixture helper: a specific icon with the default style.
    /// Production code should use `App::lint_cell` instead so the style
    /// stays in sync with the status.
    #[cfg(test)]
    pub const fn with_icon(icon: &'static str) -> Self {
        Self {
            icon,
            style: Style::new(),
        }
    }

    /// Construct from already-resolved icon + style. Visible to the App
    /// layer so `App::lint_cell` can populate both fields from a single
    /// [`LintStatus`](crate::lint::LintStatus).
    pub(in crate::tui) const fn from_parts(icon: &'static str, style: Style) -> Self {
        Self { icon, style }
    }

    pub(super) const fn icon(&self) -> &'static str { self.icon }
    pub(super) const fn style(&self) -> Style { self.style }
}

/// Deleted rows strike their content while suffix spans retain their own style.
#[derive(Clone)]
pub(in crate::tui) enum RowLifecycle {
    Present,
    Deleted,
}

impl From<bool> for RowLifecycle {
    fn from(deleted: bool) -> Self {
        if deleted {
            Self::Deleted
        } else {
            Self::Present
        }
    }
}

/// Lint visibility selects the column schema and seeds the width cache.
/// Configuration changes must reset fit widths because this changes the available columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::tui) enum LintColumnStatus {
    Enabled,
    Hidden,
}

impl From<LintIndicator> for LintColumnStatus {
    fn from(indicator: LintIndicator) -> Self {
        match indicator {
            LintIndicator::Enabled => Self::Enabled,
            LintIndicator::Disabled => Self::Hidden,
        }
    }
}

impl From<bool> for LintColumnStatus {
    fn from(lint_enabled: bool) -> Self {
        if lint_enabled {
            Self::Enabled
        } else {
            Self::Hidden
        }
    }
}

#[derive(Clone)]
pub(in crate::tui) struct ProjectRow<'a> {
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
    pub ci:                Option<CiStatus>,
    pub lifecycle:         RowLifecycle,
    pub worktree_health:   WorktreeHealth,
}

pub(in crate::tui) struct RowCells {
    pub cells:           [CellContent; NUM_COLS],
    pub prefix:          String,
    pub lifecycle:       RowLifecycle,
    pub worktree_health: WorktreeHealth,
}

// `ProjectListWidths` wraps `ColumnWidths` with lint visibility and generation invalidation.

/// Project-list column widths. Thin wrapper around the generic
/// [`ColumnWidths`] primitive that adds lint column visibility,
/// the generation counter App uses to invalidate cached widths
/// after tree changes, and the project-list-specific seeding
/// from `column_defs`.
pub(in crate::tui) struct ProjectListWidths {
    inner:          ColumnWidths,
    lint_column:    LintColumnStatus,
    pub generation: u64,
}

impl Default for ProjectListWidths {
    fn default() -> Self { Self::new(LintColumnStatus::Enabled) }
}

impl ProjectListWidths {
    /// Seed from column definitions: Fixed columns get their width,
    /// Fit columns get their minimum.
    pub(in crate::tui) fn new(lint_column: LintColumnStatus) -> Self {
        Self {
            inner: ColumnWidths::new(project_list_specs(lint_column)),
            lint_column,
            generation: u64::MAX,
        }
    }

    /// Update a Fit column with observed content width. No-op for
    /// Fixed columns (`ColumnSpec::fixed` caps `max == min`).
    pub(in crate::tui) fn observe(&mut self, col: usize, width: usize) {
        self.inner.observe_cell_usize(col, width);
    }

    /// Resolved width for a column.
    pub(in crate::tui) fn get(&self, col: usize) -> usize { usize::from(self.inner.get(col)) }

    /// Total display width of all columns including gaps.
    pub(in crate::tui) fn total_width(&self) -> usize {
        let defs = column_defs(self.lint_column);
        let mut total = 0;
        for (i, def) in defs.iter().enumerate() {
            total += def.gap + self.get(i);
        }
        total
    }

    pub(in crate::tui) const fn lint_column_status(&self) -> LintColumnStatus { self.lint_column }
}

/// Map the project-list `column_defs` into [`ColumnSpec`]s for
/// `ColumnWidths`.
fn project_list_specs(lint_column: LintColumnStatus) -> Vec<ColumnSpec> {
    column_defs(lint_column)
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

// `display_width` and the `pad_*` helpers measure and align terminal text.

/// Terminal display width of a string, accounting for multi-byte and wide
/// characters. Use this for ALL layout calculations — never `.len()`.
pub(in crate::tui) fn display_width(s: &str) -> usize { UnicodeWidthStr::width(s) }

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

// `row_to_line` renders `RowCells`, and `header_line` renders the column headings.

/// Render a `RowCells` into a styled `Line` using the column definitions and
/// resolved widths. Replaces `project_row_spans`.
pub(in crate::tui) fn row_to_line(row: &RowCells, widths: &ProjectListWidths) -> Line<'static> {
    let defs = column_defs(widths.lint_column_status());
    let mut spans = Vec::with_capacity(NUM_COLS);
    // `suffix_indices` identifies `spans` that carry suffixes and are exempt from deleted-row
    // strikethrough.
    let mut suffix_indices: Vec<usize> = Vec::new();

    for (i, cell) in row.cells.iter().enumerate() {
        let col_width = widths.get(i);
        let align = cell.align_override.unwrap_or(defs[i].align);

        if col_width == 0 {
            spans.push(Span::styled(String::new(), cell.style));
            continue;
        }

        // Split `cell.text` and `cell.suffix` into separate `Span` values so the suffix retains its
        // own style.
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

    match row.lifecycle {
        RowLifecycle::Deleted => {
            let strike = Style::default()
                .fg(label_color())
                .add_modifier(Modifier::CROSSED_OUT);
            for (i, span) in spans.iter_mut().enumerate() {
                if !suffix_indices.contains(&i) {
                    span.style = strike;
                }
            }
        },
        RowLifecycle::Present => match row.worktree_health {
            WorktreeHealth::Normal => {},
            WorktreeHealth::Broken => {
                let broken_style = Style::default().fg(on_error_color()).bg(error_color());
                for span in &mut spans {
                    span.style = broken_style;
                }
            },
        },
    }

    Line::from(spans)
}

/// Build the header `Line` from column definitions and resolved widths.
/// `name_text` is the dynamic header for the Name column (e.g. "~/rust (42)").
pub(in crate::tui) fn header_line(widths: &ProjectListWidths, name_text: &str) -> Line<'static> {
    let defs = column_defs(widths.lint_column_status());
    let header_style = Style::default()
        .fg(theme_roles::column_header_color())
        .add_modifier(Modifier::BOLD);

    let mut spans = Vec::with_capacity(NUM_COLS);
    let mut slot_widths =
        std::array::from_fn::<usize, NUM_COLS, _>(|i| defs[i].gap + widths.get(i));

    for (i, def) in defs.iter().enumerate() {
        if def.header_mode != HeaderMode::BorrowLeft {
            continue;
        }

        // For `HeaderMode::BorrowLeft`, transfer `borrow_needed` columns from the nearest left
        // `slot_widths` so the header stretches without shifting unrelated columns farther
        // left.
        let mut borrow_needed = display_width(def.header_max()).saturating_sub(widths.get(i));
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
        let slot_width = slot_widths[i];

        let content = if i == COL_NAME {
            pad_right(name_text, slot_width)
        } else {
            match def.header_mode {
                HeaderMode::Standard => {
                    let gap = def.gap.min(slot_width);
                    let content_width = slot_width.saturating_sub(gap);
                    let header = def.header_for_width(content_width);
                    let padded = match def.align {
                        Align::Left => pad_right(header, content_width),
                        Align::Right => pad_left(header, content_width),
                        Align::Center => pad_center(header, content_width),
                    };
                    format!("{}{padded}", " ".repeat(gap))
                },
                HeaderMode::BorrowLeft => {
                    let header = def.header_for_width(slot_width);
                    match def.align {
                        Align::Left => pad_right(header, slot_width),
                        Align::Right => pad_left(header, slot_width),
                        Align::Center => pad_center(header, slot_width),
                    }
                },
                HeaderMode::Hidden => " ".repeat(slot_width),
            }
        };

        spans.push(Span::styled(content, header_style));
    }

    Line::from(spans)
}

// `build_row_cells`, `build_summary_cells` and `build_group_header_cells` construct table cells.

/// Build a `RowCells` for a project row. Single construction site replaces all
/// scattered project row literals.
pub(in crate::tui) fn build_row_cells(row: ProjectRow<'_>) -> RowCells {
    let (git_path_icon, name_style, origin_sync, main_sync) =
        match (row.worktree_health, row.git_status) {
            (
                WorktreeHealth::Broken,
                Some(
                    GitStatus::Unreadable
                    | GitStatus::Clean
                    | GitStatus::Modified
                    | GitStatus::Untracked
                    | GitStatus::Ignored,
                )
                | None,
            ) => ("", Style::default(), "", ""),
            (WorktreeHealth::Normal, status) => (
                status.map_or("", GitStatus::icon),
                project_name_style(status),
                row.git_origin_sync,
                row.git_main,
            ),
        };

    let compact_status_style = |value: &str| {
        if value == IN_SYNC {
            Style::default().fg(theme_roles::git_untracked_color())
        } else {
            Style::default().fg(text_default())
        }
    };

    let compact_status_align = |value: &str| {
        if value == IN_SYNC {
            Some(Align::Center)
        } else {
            None
        }
    };

    let origin_sync_style = compact_status_style(origin_sync);
    let main_style = compact_status_style(main_sync);
    let origin_sync_align = compact_status_align(origin_sync);
    let main_align = compact_status_align(main_sync);

    let ci_style = render::ci_status_style(row.ci);
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
        text: row
            .ci
            .map_or(String::new(), |ci_status| String::from(ci_status.icon())),
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
        text: String::from(origin_sync),
        style: origin_sync_style,
        align_override: origin_sync_align,
        ..CellContent::default()
    };
    cells[COL_MAIN] = CellContent {
        text: String::from(main_sync),
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
        lifecycle: row.lifecycle,
        worktree_health: row.worktree_health,
    }
}

pub(in crate::tui) fn project_name_style(git_status: Option<GitStatus>) -> Style {
    match git_status {
        Some(GitStatus::Unreadable) => Style::default().fg(error_color()),
        Some(GitStatus::Modified) => Style::default().fg(theme_roles::git_modified_color()),
        Some(GitStatus::Untracked) => Style::default().fg(theme_roles::git_untracked_color()),
        Some(GitStatus::Ignored) => Style::default().fg(theme_roles::git_ignored_color()),
        Some(GitStatus::Clean) | None => Style::default(),
    }
}

pub(in crate::tui) fn project_name_shimmer_style(git_status: Option<GitStatus>) -> Style {
    match git_status {
        Some(GitStatus::Unreadable) => Style::default().fg(error_color()),
        Some(GitStatus::Modified) => Style::default().fg(theme_roles::git_modified_color()),
        Some(GitStatus::Untracked) => Style::default().fg(theme_roles::git_untracked_color()),
        Some(GitStatus::Ignored) => Style::default().fg(secondary_text_color()),
        Some(GitStatus::Clean) | None => {
            Style::default().fg(theme_roles::discovery_shimmer_color())
        },
    }
}

/// The shimmer segment's emphasis selects its style as consecutive characters are grouped.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ShimmerInk {
    Base,
    Accent,
}

impl ShimmerInk {
    const fn style(self, base: Style, accent: Style) -> Style {
        match self {
            Self::Base => base,
            Self::Accent => accent,
        }
    }
}

pub(in crate::tui) fn build_shimmer_segments(
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
    let mut ink = ShimmerInk::Base;

    for (index, ch) in chars.iter().enumerate() {
        let next_ink = if (index + len - head) % len < window_len {
            ShimmerInk::Accent
        } else {
            ShimmerInk::Base
        };
        if current.is_empty() {
            ink = next_ink;
        } else if next_ink != ink {
            segments.push(StyledSegment {
                text:  std::mem::take(&mut current),
                style: ink.style(base_style, accent_style),
            });
            ink = next_ink;
        }
        current.push(*ch);
    }

    if !current.is_empty() {
        segments.push(StyledSegment {
            text:  current,
            style: ink.style(base_style, accent_style),
        });
    }

    segments
}

/// Build a `RowCells` for a group header (only Name column has content).
pub(in crate::tui) fn build_group_header_cells(prefix: &str, label: &str) -> RowCells {
    let mut cells = std::array::from_fn::<CellContent, NUM_COLS, _>(|_| CellContent::default());
    cells[COL_NAME] = CellContent {
        text: String::from(label),
        style: Style::default().fg(title_color()),
        align_override: None,
        ..CellContent::default()
    };
    RowCells {
        cells,
        prefix: String::from(prefix),
        lifecycle: RowLifecycle::Present,
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
pub(in crate::tui) fn build_summary_cells(widths: &ProjectListWidths, disk: &str) -> RowCells {
    let total_style = Style::default()
        .fg(title_color())
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
        lifecycle: RowLifecycle::Present,
        worktree_health: Normal,
    }
}

/// Build the project-volume availability row below the Σ disk total.
///
/// The label spans the normally-empty metadata columns so its final `e`
/// right-aligns with the Σ above it, without disturbing the Disk column.
/// `style` colors both the label and the value — the caller bands it by
/// remaining free space.
pub(in crate::tui) fn build_available_line(
    widths: &ProjectListWidths,
    disk: &str,
    style: Style,
) -> Line<'static> {
    let defs = column_defs(widths.lint_column_status());
    let sigma_col = summary_label_col(widths);
    let sigma_end = (0..=sigma_col)
        .map(|col| rendered_column_width(widths, &defs, col))
        .sum();
    let mut spans = vec![Span::styled(pad_left("Available", sigma_end), style)];
    for col in sigma_col.saturating_add(1)..COL_DISK {
        let width = rendered_column_width(widths, &defs, col);
        if width > 0 {
            spans.push(Span::raw(" ".repeat(width)));
        }
    }

    let disk_width = widths.get(COL_DISK);
    if disk_width > 0 {
        let disk_cell = format!(
            "{}{}",
            " ".repeat(defs[COL_DISK].gap),
            pad_left(disk, disk_width)
        );
        spans.push(Span::styled(disk_cell, style));
    }

    Line::from(spans)
}

fn rendered_column_width(
    widths: &ProjectListWidths,
    defs: &[ColumnDef; NUM_COLS],
    col: usize,
) -> usize {
    let width = widths.get(col);
    if width == 0 {
        0
    } else if col == COL_NAME {
        width
    } else {
        defs[col].gap.saturating_add(width)
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::constants::GIT_STATUS_CLEAN;
    use crate::constants::GIT_STATUS_MODIFIED;
    use crate::constants::GIT_STATUS_UNTRACKED;
    use crate::constants::LINT_PASSED;
    use crate::project::WorktreeHealth;

    fn seeded_width(index: usize) -> usize {
        column_defs(LintColumnStatus::Enabled)[index].seed_width()
    }

    #[test]
    fn resolved_widths_seeds_from_defs() {
        let widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        // `ProjectListWidths::new` seeds `ColumnWidth::Fixed` columns at `seeded_width`.
        assert_eq!(widths.get(COL_LINT), seeded_width(COL_LINT));
        assert_eq!(widths.get(COL_LANG), seeded_width(COL_LANG));
        assert_eq!(widths.get(COL_CI), seeded_width(COL_CI));
        assert_eq!(widths.get(COL_GIT_PATH), seeded_width(COL_GIT_PATH));
        // `ProjectListWidths::new` seeds `ColumnWidth::Fit` columns at their minimum widths.
        assert_eq!(widths.get(COL_NAME), seeded_width(COL_NAME));
        assert_eq!(widths.get(COL_DISK), seeded_width(COL_DISK));
        assert_eq!(widths.get(COL_SYNC), seeded_width(COL_SYNC));
        assert_eq!(widths.get(COL_MAIN), seeded_width(COL_MAIN));
    }

    #[test]
    fn observe_grows_fit_columns() {
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_NAME, 25);
        assert_eq!(widths.get(COL_NAME), 25);
        // `widths.observe(COL_LINT, 99)` cannot grow the `ColumnWidth::Fixed` lint column.
        widths.observe(COL_LINT, 99);
        assert_eq!(widths.get(COL_LINT), seeded_width(COL_LINT));
    }

    #[test]
    fn total_width_sums_gaps_and_widths() {
        let defs = column_defs(LintColumnStatus::Enabled);
        let widths = ProjectListWidths::new(LintColumnStatus::Enabled);
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
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
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
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), " Mn");
        assert_eq!(line.spans[COL_DISK].content.as_ref(), "     Disk");
        assert_eq!(line.width(), widths.total_width());
    }

    #[test]
    fn git_header_borrows_from_hidden_lang_column() {
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let line = header_line(&widths, "Projects");

        assert_eq!(line.spans[COL_CI].content.as_ref(), " CI");
        assert_eq!(display_width(line.spans[COL_LANG].content.as_ref()), 2);
        assert_eq!(line.spans[COL_GIT_PATH].content.as_ref(), " Git");
        assert_eq!(line.spans[COL_SYNC].content.as_ref(), " Og");
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), " Mn");
        assert_eq!(line.width(), widths.total_width());
    }

    #[test]
    fn header_levels_promote_with_observed_width() {
        // Empty: only the seed (1) for COL_MAIN, (2) for COL_SYNC.
        let widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        let defs = column_defs(LintColumnStatus::Enabled);
        assert_eq!(defs[COL_MAIN].header_for_width(widths.get(COL_MAIN)), "M");
        assert_eq!(defs[COL_SYNC].header_for_width(widths.get(COL_SYNC)), "Og");

        // The observed `COL_MAIN` and `COL_SYNC` widths select the second `header_levels` labels.
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_MAIN, 3);
        widths.observe(COL_SYNC, 5);
        assert_eq!(defs[COL_MAIN].header_for_width(widths.get(COL_MAIN)), "Mn");
        assert_eq!(
            defs[COL_SYNC].header_for_width(widths.get(COL_SYNC)),
            "Orig"
        );

        // These `widths.observe` calls make the longest `COL_MAIN` and `COL_SYNC` header labels
        // fit.
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_MAIN, 4);
        widths.observe(COL_SYNC, 6);
        assert_eq!(
            defs[COL_MAIN].header_for_width(widths.get(COL_MAIN)),
            "Main"
        );
        assert_eq!(
            defs[COL_SYNC].header_for_width(widths.get(COL_SYNC)),
            "Origin"
        );
    }

    #[test]
    fn header_line_uses_widest_label_that_fits() {
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 6);
        widths.observe(COL_MAIN, 4);

        let line = header_line(&widths, "Projects");

        assert_eq!(line.spans[COL_SYNC].content.as_ref(), " Origin");
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), " Main");
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
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_NAME, 32);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let row_emoji = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "bevy_brp 🌲:2",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(CiStatus::Passed),
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        let row_ascii = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "bevy_mesh_outline_benchmark",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(CiStatus::Passed),
            lifecycle:         RowLifecycle::Present,
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
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
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
        assert_eq!(line.spans[COL_DISK].content.as_ref(), " 36.3 GiB");
    }

    #[test]
    fn availability_label_ends_below_summary_sigma() {
        let mut widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let summary = build_summary_cells(&widths, "36.3 GiB");
        let summary_line = row_to_line(&summary, &widths);
        let available_line = build_available_line(&widths, "64.9 GiB", Style::default());
        let summary_text: String = summary_line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let available_text: String = available_line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        let sigma_start = summary_text.find('Σ').expect("summary has a sigma");
        let available_start = available_text
            .find("Available")
            .expect("availability row has a label");
        let summary_disk_start = summary_text
            .find("36.3 GiB")
            .expect("summary has disk bytes");
        let available_disk_start = available_text
            .find("64.9 GiB")
            .expect("availability row has disk bytes");

        assert_eq!(
            display_width(&available_text[..available_start]) + display_width("Available"),
            display_width(&summary_text[..sigma_start]) + display_width("Σ"),
        );
        assert_eq!(
            display_width(&available_text[..available_disk_start]),
            display_width(&summary_text[..summary_disk_start]),
        );
    }

    #[test]
    fn lint_column_status_maps_configuration_variants() {
        assert_eq!(
            LintColumnStatus::from(LintIndicator::Enabled),
            LintColumnStatus::Enabled
        );
        assert_eq!(
            LintColumnStatus::from(LintIndicator::Disabled),
            LintColumnStatus::Hidden
        );
    }

    #[test]
    fn lint_column_collapses_when_disabled() {
        let defs = column_defs(LintColumnStatus::Hidden);
        let mut widths = ProjectListWidths::new(LintColumnStatus::Hidden);
        assert_eq!(widths.lint_column_status(), LintColumnStatus::Hidden);
        widths.observe(COL_NAME, 30);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let header = header_line(&widths, "Projects");
        let row = build_summary_cells(&widths, "36.3 GiB");
        let line = row_to_line(&row, &widths);

        assert_eq!(defs[COL_LINT].header_max(), "");
        assert_eq!(widths.get(COL_LINT), 0);
        assert_eq!(display_width(header.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(defs[COL_CI].header_max(), "CI");
        assert_eq!(widths.get(COL_CI), 2);
        assert!(header.spans[COL_CI].content.as_ref().ends_with("CI"));
        assert_eq!(line.spans[COL_MAIN].content.as_ref(), "  Σ");
    }

    #[test]
    fn hidden_lint_column_does_not_shift_ci_cells() {
        let mut widths = ProjectListWidths::new(LintColumnStatus::Hidden);
        widths.observe(COL_NAME, 24);
        widths.observe(COL_DISK, 8);
        widths.observe(COL_SYNC, 2);
        widths.observe(COL_MAIN, 2);

        let row = build_row_cells(ProjectRow {
            prefix:            "▶",
            name:              "demo",
            name_segments:     None,
            git_status:        Some(GitStatus::Clean),
            lint:              LintCell::with_icon(LINT_PASSED),
            disk:              "36.3 GiB",
            disk_style:        Style::default(),
            disk_suffix:       None,
            disk_suffix_style: None,
            lang_icon:         "🦀",
            git_origin_sync:   "↑2",
            git_main:          "",
            ci:                Some(CiStatus::Passed),
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        let line = row_to_line(&row, &widths);

        assert_eq!(display_width(line.spans[COL_LINT].content.as_ref()), 0);
        assert_eq!(
            line.spans[COL_CI].content.as_ref(),
            &format!(" {}", CiStatus::Passed.icon())
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
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(
            modified.cells[COL_NAME].style.fg,
            Some(theme_roles::git_modified_color())
        );
        assert_eq!(modified.cells[COL_GIT_PATH].text, GIT_STATUS_MODIFIED);

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
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(
            untracked.cells[COL_NAME].style.fg,
            Some(theme_roles::git_untracked_color())
        );
        assert_eq!(untracked.cells[COL_GIT_PATH].text, GIT_STATUS_UNTRACKED);

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
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(clean.cells[COL_GIT_PATH].text, GIT_STATUS_CLEAN);

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
            lifecycle:         RowLifecycle::Present,
            worktree_health:   WorktreeHealth::Normal,
        });
        assert_eq!(
            ignored.cells[COL_NAME].style.fg,
            Some(theme_roles::git_ignored_color())
        );
        assert_eq!(ignored.cells[COL_GIT_PATH].text, "");
    }

    #[test]
    fn lifecycle_rendering_preserves_suffix_style_and_broken_precedence() {
        let widths = ProjectListWidths::new(LintColumnStatus::Enabled);
        let base = Style::default().fg(title_color());
        let suffix = Style::default().fg(error_color());
        let strike = Style::default()
            .fg(label_color())
            .add_modifier(Modifier::CROSSED_OUT);
        let broken = Style::default().fg(on_error_color()).bg(error_color());
        for (lifecycle, health, expected, expected_suffix) in [
            (RowLifecycle::Present, WorktreeHealth::Normal, base, suffix),
            (
                RowLifecycle::Present,
                WorktreeHealth::Broken,
                broken,
                broken,
            ),
            (
                RowLifecycle::Deleted,
                WorktreeHealth::Normal,
                strike,
                suffix,
            ),
            (
                RowLifecycle::Deleted,
                WorktreeHealth::Broken,
                strike,
                suffix,
            ),
        ] {
            let mut row = build_group_header_cells("", "demo");
            row.lifecycle = lifecycle;
            row.worktree_health = health;
            row.cells[COL_DISK] = CellContent {
                text: String::from("1"),
                style: base,
                suffix: Some(String::from("!")),
                suffix_style: Some(suffix),
                ..CellContent::default()
            };
            let line = row_to_line(&row, &widths);
            assert_eq!(line.spans[COL_NAME].style, expected);
            assert_eq!(line.spans[COL_DISK].style, expected);
            assert_eq!(line.spans[COL_DISK + 1].style, expected_suffix);
            assert_eq!(line.spans[COL_DISK + 1].content.as_ref(), "!");
        }
    }

    #[test]
    fn shimmer_ink_preserves_base_accent_and_empty_output() {
        let base = Style::default();
        let accent = Style::default().fg(title_color());
        for (name, window, expected_text, expected_style) in [
            ("abcd", 0, "abcd", base),
            ("abcd", 4, "abcd", accent),
            ("", 2, "", base),
        ] {
            let segments = build_shimmer_segments(name, base, accent, 0, window);
            assert_eq!(segments.len(), 1);
            assert_eq!(segments[0].text, expected_text);
            assert_eq!(segments[0].style, expected_style);
        }
    }

    #[test]
    fn build_shimmer_segments_wraps_around_name_end() {
        let segments = build_shimmer_segments(
            "abcd",
            Style::default(),
            Style::default().fg(title_color()),
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
                ("a", Some(title_color())),
                ("bc", None),
                ("d", Some(title_color())),
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
            Some(theme_roles::discovery_shimmer_color())
        );
        assert_eq!(
            project_name_shimmer_style(None).fg,
            Some(theme_roles::discovery_shimmer_color())
        );
    }
}
