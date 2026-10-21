//! Account ordering, reset colors and headers share measurement and drawing lines.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::DateTime;
use chrono::Duration;
use chrono::Local;
use chrono::Utc;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::label_color;
use tui_pane::text_default;

use super::age;
use super::cell_width;
use super::constants::ACCOUNT_ROLE_SEPARATOR;
use super::constants::ACCOUNTS_HEADING;
use super::constants::LOW_WEEKLY_USAGE_COLOR;
use super::constants::RESET_ROUNDING_SECONDS;
use super::constants::RESET_TIME_FORMAT;
use super::constants::STALE_ACCOUNT_SUFFIX;
use super::constants::UNLIMITED_CREDITS_LABEL;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditUse;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::constants::ACCOUNTS_LOW_PERCENT;
use crate::constants::ACCOUNTS_STALE_SECONDS;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::MISSING_VALUE;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::sort::AccountsColumn;
use crate::sort::AccountsSort;
use crate::sort::ColumnSort;
use crate::sort::HeaderSpan;
use crate::sort::SortDirection;
use crate::theme::ResetRank;

/// Missing readings stay after real values even when the direction reverses.
fn compare_readings<T>(
    left: &Reading<T>,
    right: &Reading<T>,
    direction: SortDirection,
    compare: impl FnOnce(&T, &T) -> Ordering,
) -> Ordering {
    match (left, right) {
        (Reading::Known(left), Reading::Known(right)) => direction.order(compare(left, right)),
        (Reading::Known(_), Reading::Unavailable) => Ordering::Less,
        (Reading::Unavailable, Reading::Known(_)) => Ordering::Greater,
        (Reading::Unavailable, Reading::Unavailable) => Ordering::Equal,
    }
}

/// Numerically equal readings retain source order, including signed zeroes.
fn compare_numbers(left: f64, right: f64) -> Ordering {
    if left.partial_cmp(&right) == Some(Ordering::Equal) {
        Ordering::Equal
    } else {
        left.total_cmp(&right)
    }
}

/// Special balances retain their place after numeric balances in both directions.
fn compare_credits(left: &Credits, right: &Credits, direction: SortDirection) -> Ordering {
    match (left, right) {
        (Credits::Amount(left), Credits::Amount(right)) => {
            direction.order(compare_numbers(*left, *right))
        },
        (Credits::Amount(_), _) | (Credits::Unlimited, Credits::Unavailable) => Ordering::Less,
        (_, Credits::Amount(_)) | (Credits::Unavailable, Credits::Unlimited) => Ordering::Greater,
        _ => Ordering::Equal,
    }
}

/// Apply the shared direction to real column values; source positions break ties.
fn compare_accounts(
    left: &AccountUsage,
    right: &AccountUsage,
    sort: ColumnSort<AccountsColumn>,
) -> Ordering {
    match sort.column {
        AccountsColumn::Account => sort.direction.order(left.label.cmp(&right.label)),
        AccountsColumn::Roles => match (left.roles.is_empty(), right.roles.is_empty()) {
            (false, false) => sort.direction.order(
                left.roles
                    .iter()
                    .map(|role| role.text())
                    .cmp(right.roles.iter().map(|role| role.text())),
            ),
            (left, right) => left.cmp(&right),
        },
        AccountsColumn::WeeklyLeft => compare_readings(
            &left.weekly_left_percent,
            &right.weekly_left_percent,
            sort.direction,
            |left, right| compare_numbers(*left, *right),
        ),
        AccountsColumn::Resets => {
            compare_readings(&left.resets_at, &right.resets_at, sort.direction, Ord::cmp)
        },
        AccountsColumn::Credits => compare_credits(&left.credits, &right.credits, sort.direction),
    }
}

/// Distinct chronological instants receive evenly spaced semantic colors.
fn reset_colors(reading: &AccountsReading) -> BTreeMap<DateTime<Utc>, Color> {
    let mut colors: BTreeMap<_, _> = reading
        .accounts
        .iter()
        .filter_map(|row| match row.resets_at {
            Reading::Known(at) => Some((at, tui_pane::success_color())),
            Reading::Unavailable => None,
        })
        .collect();
    let ranks = ResetRank::positions(colors.len());
    for (color, rank) in colors.values_mut().zip(ranks) {
        *color = rank.color();
    }
    colors
}

/// A failure note belongs to exactly the reading it accompanies.
enum FailureNote<'a> {
    Clear,
    Failed(&'a AccountsFailure),
}

/// Resolve states without carrying independent optional rows and errors.
enum AccountsSection<'a> {
    Hidden,
    Failure(&'a AccountsFailure),
    Reading {
        reading: &'a AccountsReading,
        note:    FailureNote<'a>,
    },
}

impl<'a> From<&'a AccountsTable> for AccountsSection<'a> {
    fn from(table: &'a AccountsTable) -> Self {
        match table {
            AccountsTable::NotConfigured | AccountsTable::AwaitingFirstReading => Self::Hidden,
            AccountsTable::Ready(reading) => Self::Reading {
                reading,
                note: FailureNote::Clear,
            },
            AccountsTable::Failed(failure) => Self::Failure(failure),
            AccountsTable::FailedKeepingLast { last, failure } => Self::Reading {
                reading: last,
                note:    FailureNote::Failed(failure),
            },
        }
    }
}

pub(super) fn height(table: &AccountsTable) -> usize {
    match AccountsSection::from(table) {
        AccountsSection::Hidden => 0,
        AccountsSection::Failure(_) => 1,
        AccountsSection::Reading { reading, note } => {
            2 + reading.accounts.len()
                + reading.problems.len()
                + usize::from(matches!(note, FailureNote::Failed(_)))
        },
    }
}

/// Sorted headers participate in demand, including their direction arrows.
pub(super) fn sorted_width(table: &AccountsTable, now: u64, sort: AccountsSort) -> u16 {
    sorted_lines(table, now, &FooterColors::default(), sort)
        .iter()
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or_default()
}

/// The same lines supply source-ordered and user-sorted account tables.
pub(super) fn draw_sorted(
    buffer: &mut Buffer,
    area: Rect,
    table: &AccountsTable,
    now: u64,
    colors: &FooterColors,
    sort: AccountsSort,
) {
    Paragraph::new(sorted_lines(table, now, colors, sort)).render(area, buffer);
}

/// Read the measured header spans back from the same lines that are drawn.
pub(super) fn header_spans(
    area: Rect,
    table: &AccountsTable,
    now: u64,
    sort: AccountsSort,
) -> Vec<HeaderSpan<AccountsColumn>> {
    if !matches!(
        AccountsSection::from(table),
        AccountsSection::Reading { .. }
    ) || area.height < 2
    {
        return Vec::new();
    }
    let lines = sorted_lines(table, now, &FooterColors::default(), sort);
    let mut x = area.x;
    let mut spans = Vec::new();
    let mut columns = AccountsColumn::ALL.into_iter();
    for (index, span) in lines[1].spans.iter().enumerate() {
        let width = cell_width(&span.content);
        // The first span is indentation; headers and their following gaps alternate.
        if index % 2 == 1
            && let Some(column) = columns.next()
        {
            spans.push(HeaderSpan {
                column,
                columns: x..x.saturating_add(width).min(area.right()),
                row: area.y + 1,
            });
        }
        x = x.saturating_add(width);
    }
    spans
}

fn dim() -> Style {
    Style::default()
        .fg(text_default())
        .add_modifier(Modifier::DIM)
}

fn note(text: impl Into<String>) -> Line<'static> {
    Line::from(Span::styled(
        format!("{SECTION_ITEM_INDENT}{}", text.into()),
        dim(),
    ))
}

/// One reading state supplies both the account suffix and the row style.
enum Freshness {
    Fresh,
    Stale(String),
    Unread,
}

impl Freshness {
    fn new(row: &AccountUsage, now: u64) -> Self {
        match row.read_at {
            Reading::Unavailable => Self::Unread,
            Reading::Known(read_at) => {
                let seconds =
                    now.saturating_sub(u64::try_from(read_at.timestamp()).unwrap_or_default());
                if seconds > ACCOUNTS_STALE_SECONDS {
                    Self::Stale(age::age_label(seconds))
                } else {
                    Self::Fresh
                }
            },
        }
    }

    fn label(&self, account: &str) -> String {
        match self {
            Self::Stale(age) => format!("{account}{HEADING_SEPARATOR}{age} {STALE_ACCOUNT_SUFFIX}"),
            Self::Fresh | Self::Unread => account.to_owned(),
        }
    }

    fn style(&self) -> Style {
        match self {
            Self::Fresh => Style::default().fg(text_default()),
            Self::Stale(_) | Self::Unread => dim(),
        }
    }
}

fn values(row: &AccountUsage, freshness: &Freshness) -> [String; 5] {
    let roles = if row.roles.is_empty() {
        MISSING_VALUE.to_owned()
    } else {
        row.roles
            .iter()
            .map(|role| role.text())
            .collect::<Vec<_>>()
            .join(ACCOUNT_ROLE_SEPARATOR)
    };
    [
        freshness.label(&row.label),
        roles,
        match row.weekly_left_percent {
            Reading::Known(left) => format!("{left}%"),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        match row.resets_at {
            Reading::Known(at) => (at + Duration::seconds(RESET_ROUNDING_SECONDS))
                .with_timezone(&Local)
                .format(RESET_TIME_FORMAT)
                .to_string(),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        match row.credits {
            Credits::Amount(amount) => amount.to_string(),
            Credits::Unlimited => UNLIMITED_CREDITS_LABEL.to_owned(),
            Credits::Unavailable => MISSING_VALUE.to_owned(),
        },
    ]
}

/// Pad with display widths so account labels and notes determine the same demand drawn.
fn columns(values: [Line<'static>; 5], widths: &[u16; 5]) -> Line<'static> {
    let mut spans = vec![Span::raw(SECTION_ITEM_INDENT)];
    for ((column, mut value), &column_width) in
        AccountsColumn::ALL.into_iter().zip(values).zip(widths)
    {
        let width = value.spans.iter().fold(0u16, |width, span| {
            width.saturating_add(cell_width(&span.content))
        });
        let padding = column_width.saturating_sub(width);
        let trailing_padding = match column {
            AccountsColumn::WeeklyLeft | AccountsColumn::Resets | AccountsColumn::Credits => {
                if let Some(first) = value.spans.first_mut() {
                    first.content =
                        format!("{}{}", " ".repeat(usize::from(padding)), first.content).into();
                }
                0
            },
            AccountsColumn::Account | AccountsColumn::Roles => padding,
        };
        spans.append(&mut value.spans);
        if column != AccountsColumn::Credits {
            spans.push(Span::raw(
                " ".repeat(usize::from(TABLE_COLUMN_SPACING + trailing_padding)),
            ));
        }
    }
    Line::from(spans)
}

/// Management roles use footer ink; other words and punctuation keep the row style.
fn role_words(row: &AccountUsage, freshness: &Freshness, colors: &FooterColors) -> Line<'static> {
    let base = freshness.style();
    if row.roles.is_empty() {
        return Line::from(Span::styled(MISSING_VALUE, base));
    }
    let mut spans = Vec::new();
    for role in &row.roles {
        if !spans.is_empty() {
            spans.push(Span::styled(ACCOUNT_ROLE_SEPARATOR, base));
        }
        let tint = match role {
            AccountRole::Units => colors.role(FooterRole::UnitDirector),
            AccountRole::Showrunners => colors.role(FooterRole::Showrunner),
            AccountRole::Workers | AccountRole::Spare => FooterTint::Plain,
        };
        let style = match freshness {
            Freshness::Fresh => tint.style(base),
            Freshness::Stale(_) | Freshness::Unread => tint.subdued_style(base),
        };
        spans.push(Span::styled(role.text(), style));
    }
    Line::from(spans)
}

/// Resolve one reading into ordered, measured lines for drawing and hit testing.
fn sorted_lines(
    table: &AccountsTable,
    now: u64,
    colors: &FooterColors,
    sort: AccountsSort,
) -> Vec<Line<'static>> {
    let (reading, failure) = match AccountsSection::from(table) {
        AccountsSection::Hidden => return Vec::new(),
        AccountsSection::Failure(failure) => return vec![note(failure.to_string())],
        AccountsSection::Reading { reading, note } => (reading, note),
    };
    let palette = reset_colors(reading);
    let mut ordered: Vec<_> = reading.accounts.iter().enumerate().collect();
    if let AccountsSort::Sorted(sort) = sort {
        ordered.sort_by(|(left_index, left), (right_index, right)| {
            compare_accounts(left, right, sort).then_with(|| left_index.cmp(right_index))
        });
    }
    let rows: Vec<_> = ordered
        .iter()
        .map(|(_, row)| *row)
        .map(|row| {
            let freshness = Freshness::new(row, now);
            (values(row, &freshness), freshness)
        })
        .collect();
    let widths = std::array::from_fn(|column| {
        rows.iter()
            .map(|(values, _)| cell_width(&values[column]))
            .fold(
                cell_width(&sort.header(AccountsColumn::ALL[column])),
                u16::max,
            )
    });
    let mut result = vec![
        Line::from(Span::styled(
            format!("{SECTION_HEADER_INDENT}{ACCOUNTS_HEADING}"),
            Style::default()
                .fg(text_default())
                .add_modifier(Modifier::BOLD),
        )),
        columns(
            AccountsColumn::ALL.map(|column| {
                Line::from(Span::styled(
                    sort.header(column),
                    Style::default().fg(label_color()),
                ))
            }),
            &widths,
        ),
    ];
    for ((_, row), (values, freshness)) in ordered.into_iter().zip(rows) {
        let base = freshness.style();
        let mut cells = values.map(|value| Span::styled(value, base));
        cells[0].style = match freshness {
            Freshness::Fresh => colors.account(&row.label).style(base),
            Freshness::Stale(_) | Freshness::Unread => {
                colors.account(&row.label).subdued_style(base)
            },
        };
        if matches!(row.weekly_left_percent, Reading::Known(left) if left <= ACCOUNTS_LOW_PERCENT) {
            cells[2].style = base.fg(LOW_WEEKLY_USAGE_COLOR);
        }
        if let Reading::Known(at) = row.resets_at {
            cells[3].style = base.fg(palette[&at]);
        }
        if row.credit_use == CreditUse::Held {
            cells[4].style = base.add_modifier(Modifier::DIM);
        }
        let mut cells = cells.map(Line::from);
        cells[1] = role_words(row, &freshness, colors);
        result.push(columns(cells, &widths));
    }
    result.extend(reading.problems.iter().map(|problem| note(problem.clone())));
    if let FailureNote::Failed(failure) = failure {
        result.push(note(failure.to_string()));
    }
    result
}
