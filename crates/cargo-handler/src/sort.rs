//! Column choices and directions shared by header drawing and click handling.

use std::cmp::Ordering;
use std::ops::Range;

use ratatui::layout::Rect;
use tui_pane::CellPoint;

use crate::agent_cell::TreeColumn;
use crate::constants::SUMMARY_HEADERS;

/// Alphabetical session order, with original spelling breaking case-insensitive ties.
pub(crate) fn compare_names(left: &str, right: &str) -> Ordering {
    left.chars()
        .flat_map(char::to_lowercase)
        .cmp(right.chars().flat_map(char::to_lowercase))
        .then_with(|| left.cmp(right))
}

/// How values in the selected column are ordered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SortDirection {
    /// Smaller values precede larger values.
    Ascending,
    /// Larger values precede smaller values.
    Descending,
}

impl SortDirection {
    const fn reversed(self) -> Self {
        match self {
            Self::Ascending => Self::Descending,
            Self::Descending => Self::Ascending,
        }
    }

    /// Direction marker appended only to the selected header.
    const fn arrow(self) -> &'static str {
        match self {
            Self::Ascending => " ▲",
            Self::Descending => " ▼",
        }
    }

    /// Apply direction only to the primary comparison, leaving ties unchanged.
    pub(crate) const fn order(self, ordering: Ordering) -> Ordering {
        match self {
            Self::Ascending => ordering,
            Self::Descending => ordering.reverse(),
        }
    }
}

/// A column supplies its label and the order selected on the first click.
pub(crate) trait SortColumn: Copy + Eq {
    /// The initial column when a table has no saved preference.
    const DEFAULT: Self;

    /// The direction selected when switching to this column.
    fn natural_direction(self) -> SortDirection;

    /// The header without its direction marker.
    fn label(self) -> &'static str;
}

/// A table's selected column and ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ColumnSort<C: SortColumn> {
    /// The column whose values order siblings or machine-group rows.
    pub(crate) column:    C,
    /// The order within the selected column.
    pub(crate) direction: SortDirection,
}

impl<C: SortColumn> Default for ColumnSort<C> {
    fn default() -> Self {
        Self {
            column:    C::DEFAULT,
            direction: C::DEFAULT.natural_direction(),
        }
    }
}

impl<C: SortColumn> ColumnSort<C> {
    /// Reverse `ColumnSort.direction` for the selected column; for another
    /// column, use `SortColumn::natural_direction`.
    pub(crate) fn select(&mut self, column: C) {
        self.direction = if self.column == column {
            self.direction.reversed()
        } else {
            column.natural_direction()
        };
        self.column = column;
    }

    /// Mark only the selected header with its direction.
    pub(crate) fn header(self, column: C) -> String {
        format!(
            "{}{}",
            column.label(),
            if self.column == column {
                self.direction.arrow()
            } else {
                ""
            }
        )
    }
}

/// One agent tile's sibling ordering.
pub(crate) type TreeSort = ColumnSort<TreeColumn>;

/// A column of each machine's summary table, in drawing order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SummaryColumn {
    Pid,
    Agent,
    Name,
    Status,
    Age,
    Desktop,
    Directory,
}

impl SummaryColumn {
    /// All columns in their table order.
    pub(crate) const ALL: [Self; 7] = [
        Self::Pid,
        Self::Agent,
        Self::Name,
        Self::Status,
        Self::Age,
        Self::Desktop,
        Self::Directory,
    ];

    /// Position of this column in the summary table.
    pub(crate) const fn index(self) -> usize {
        match self {
            Self::Pid => 0,
            Self::Agent => 1,
            Self::Name => 2,
            Self::Status => 3,
            Self::Age => 4,
            Self::Desktop => 5,
            Self::Directory => 6,
        }
    }

    /// The visible label without a direction marker.
    pub(crate) const fn header(self) -> &'static str { SUMMARY_HEADERS[self.index()] }
}

impl SortColumn for SummaryColumn {
    const DEFAULT: Self = Self::Age;

    fn natural_direction(self) -> SortDirection {
        match self {
            Self::Age => SortDirection::Descending,
            _ => SortDirection::Ascending,
        }
    }

    fn label(self) -> &'static str { self.header() }
}

/// One ordering shared by all machine groups in the summary.
pub(crate) type SummarySort = ColumnSort<SummaryColumn>;

/// Account usage columns in their visible order, independent of agent columns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AccountsColumn {
    Account,
    Roles,
    WeeklyLeft,
    Resets,
    Credits,
    Enabled,
    InUse,
    Read,
}

impl AccountsColumn {
    /// Every labelled account column, in drawing order; the status dot has no header.
    pub(crate) const ALL: [Self; 8] = [
        Self::Account,
        Self::Roles,
        Self::WeeklyLeft,
        Self::Resets,
        Self::Credits,
        Self::Enabled,
        Self::InUse,
        Self::Read,
    ];

    /// Position in the measured account row.
    const fn index(self) -> usize {
        match self {
            Self::Account => 0,
            Self::Roles => 1,
            Self::WeeklyLeft => 2,
            Self::Resets => 3,
            Self::Credits => 4,
            Self::Enabled => 5,
            Self::InUse => 6,
            Self::Read => 7,
        }
    }

    /// The label before a sort is selected.
    pub(crate) const fn header(self) -> &'static str {
        const HEADERS: [&str; AccountsColumn::ALL.len()] = [
            "account", "role", "left", "resets", "credits", "enabled", "in use", "read",
        ];
        HEADERS[self.index()]
    }
}

impl SortColumn for AccountsColumn {
    const DEFAULT: Self = Self::Account;

    fn natural_direction(self) -> SortDirection { SortDirection::Ascending }

    fn label(self) -> &'static str { self.header() }
}

/// Preserve the source order until the user selects an accounts header.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum AccountsSort {
    #[default]
    SourceOrder,
    Sorted(ColumnSort<AccountsColumn>),
}

impl AccountsSort {
    /// Start with the column's natural direction, then use the shared toggle.
    pub(crate) fn select(&mut self, column: AccountsColumn) {
        match self {
            Self::SourceOrder => {
                *self = Self::Sorted(ColumnSort {
                    column,
                    direction: column.natural_direction(),
                });
            },
            Self::Sorted(sort) => sort.select(column),
        }
    }

    /// A source-ordered table has no selected header or direction marker.
    pub(crate) fn header(self, column: AccountsColumn) -> String {
        match self {
            Self::SourceOrder => column.header().to_owned(),
            Self::Sorted(sort) => sort.header(column),
        }
    }
}

/// A drawn header cell, excluding the gap after it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HeaderSpan<C> {
    /// The column represented by this header cell.
    pub(crate) column:  C,
    /// Horizontal extent, excluding the following gap.
    pub(crate) columns: Range<u16>,
    /// The visible header row.
    pub(crate) row:     u16,
}

/// Whether a content click names a column header.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HeaderHit<C> {
    Column(C),
    Miss,
}

/// Resolve a content press against visible header cells.
pub(crate) fn header_hit<C: Copy>(spans: &[HeaderSpan<C>], point: CellPoint) -> HeaderHit<C> {
    spans
        .iter()
        .find(|span| span.row == point.row && span.columns.contains(&point.column))
        .map_or(HeaderHit::Miss, |span| HeaderHit::Column(span.column))
}

/// Header cells from the table's resolved column rectangles.
pub(crate) fn column_spans<C: Copy>(columns: &[C], rects: &[Rect], row: u16) -> Vec<HeaderSpan<C>> {
    columns
        .iter()
        .zip(rects)
        .map(|(&column, rect)| HeaderSpan {
            column,
            columns: rect.x..rect.right(),
            row,
        })
        .collect()
}
