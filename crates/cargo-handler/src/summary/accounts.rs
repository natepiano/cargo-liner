//! Account ordering, reset colors and headers share measurement and drawing lines.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::slice::Iter;

use chrono::DateTime;
use chrono::Duration;
use chrono::Local;
use chrono::Utc;
#[cfg(test)]
use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
#[cfg(test)]
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
#[cfg(test)]
use ratatui::widgets::Paragraph;
#[cfg(test)]
use ratatui::widgets::Widget;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::label_color;
use tui_pane::text_default;

use super::cell_width;
use super::constants::ACCOUNT_READ_DOT;
use super::constants::ACCOUNTS_HEADING;
use super::constants::LOW_WEEKLY_USAGE_COLOR;
use super::constants::RESET_ROUNDING_SECONDS;
use super::constants::RESET_TIME_FORMAT;
use super::constants::UNLIMITED_CREDITS_LABEL;
use crate::census::accounts::AccountReadStatus;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountRoleUsage;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditPermission;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::census::accounts::RoleSessionCounts;
use crate::constants::ACCOUNTS_LOW_PERCENT;
use crate::constants::ACCOUNTS_STALE_SECONDS;
use crate::constants::MISSING_VALUE;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::footer_colors;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::interaction::RowSpan;
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
                    .map(AccountRoleUsage::text)
                    .cmp(right.roles.iter().map(AccountRoleUsage::text)),
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
        AccountsColumn::Enabled => match (left.credit_permission, right.credit_permission) {
            (CreditPermission::Enabled, CreditPermission::Disabled) => Ordering::Less,
            (CreditPermission::Disabled, CreditPermission::Enabled) => Ordering::Greater,
            _ => Ordering::Equal,
        },
        AccountsColumn::Busy
        | AccountsColumn::Starting
        | AccountsColumn::Waiting
        | AccountsColumn::Idle
        | AccountsColumn::Paused
        | AccountsColumn::Exited
        | AccountsColumn::Total => compare_readings(
            &account_count(left, sort.column),
            &account_count(right, sort.column),
            sort.direction,
            Ord::cmp,
        ),
    }
}

/// Sum known role counts with the same rule for sorting and the total row.
fn summed_counts<'a>(
    roles: impl Iterator<Item = &'a AccountRoleUsage>,
) -> Reading<RoleSessionCounts> {
    roles
        .filter_map(|role| match &role.counts {
            Reading::Known(counts) => Some(counts),
            Reading::Unavailable => None,
        })
        .try_fold(
            RoleSessionCounts {
                busy:     0,
                starting: 0,
                waiting:  0,
                idle:     0,
                paused:   0,
                exited:   0,
                total:    0,
            },
            RoleSessionCounts::checked_add,
        )
        .into()
}

/// Count sorts act on the whole account and preserve its role group.
fn account_count(account: &AccountUsage, column: AccountsColumn) -> Reading<u64> {
    count_value(&summed_counts(account.roles.iter()), column)
}

const fn role_count(role: &AccountRoleUsage, column: AccountsColumn) -> Reading<u64> {
    count_value(&role.counts, column)
}

const fn count_value(counts: &Reading<RoleSessionCounts>, column: AccountsColumn) -> Reading<u64> {
    match column {
        AccountsColumn::Busy => match counts {
            Reading::Known(counts) => Reading::Known(counts.busy),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Starting => match counts {
            Reading::Known(counts) => Reading::Known(counts.starting),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Waiting => match counts {
            Reading::Known(counts) => Reading::Known(counts.waiting),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Idle => match counts {
            Reading::Known(counts) => Reading::Known(counts.idle),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Paused => match counts {
            Reading::Known(counts) => Reading::Known(counts.paused),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Exited => match counts {
            Reading::Known(counts) => Reading::Known(counts.exited),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Total => match counts {
            Reading::Known(counts) => Reading::Known(counts.total),
            Reading::Unavailable => Reading::Unavailable,
        },
        AccountsColumn::Account
        | AccountsColumn::WeeklyLeft
        | AccountsColumn::Roles
        | AccountsColumn::Resets
        | AccountsColumn::Credits
        | AccountsColumn::Enabled => Reading::Unavailable,
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
#[derive(Clone, Copy)]
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

#[cfg(test)]
pub(super) fn height(table: &AccountsTable) -> usize {
    height_at_width(table, 0, AccountsSort::SourceOrder, u16::MAX)
}

/// Measure exactly the lines drawn at this width, including wrapped accounts.
#[cfg(test)]
pub(super) fn height_at_width(
    table: &AccountsTable,
    now: u64,
    sort: AccountsSort,
    width: u16,
) -> usize {
    build_lines(table, now, &FooterColors::default(), sort, width)
        .lines
        .len()
}

/// Sorted headers participate in preferred width, including direction arrows.
#[cfg(test)]
pub(super) fn sorted_width(table: &AccountsTable, now: u64, sort: AccountsSort) -> u16 {
    sorted_width_at_width(table, now, sort, u16::MAX)
}

#[cfg(test)]
pub(super) fn sorted_width_at_width(
    table: &AccountsTable,
    now: u64,
    sort: AccountsSort,
    width: u16,
) -> u16 {
    build_lines(table, now, &FooterColors::default(), sort, width)
        .lines
        .iter()
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or_default()
}

/// The same measured lines supply painting and demand at the actual table width.
#[cfg(test)]
pub(super) fn draw_sorted(
    buffer: &mut Buffer,
    area: Rect,
    table: &AccountsTable,
    now: u64,
    colors: &FooterColors,
    sort: AccountsSort,
) {
    Paragraph::new(
        build_lines(table, now, colors, sort, area.width)
            .lines
            .clone(),
    )
    .render(area, buffer);
}

/// Both header lines use the coordinates recorded while building the drawing.
#[cfg(test)]
pub(super) fn header_spans(
    area: Rect,
    table: &AccountsTable,
    now: u64,
    sort: AccountsSort,
) -> Vec<HeaderSpan<AccountsColumn>> {
    build_lines(table, now, &FooterColors::default(), sort, area.width)
        .headers
        .iter()
        .filter(|&header| header.row < area.height)
        .map(|header| HeaderSpan {
            column:  header.column,
            row:     area.y.saturating_add(header.row),
            columns: area.x.saturating_add(header.columns.start)
                ..area.x.saturating_add(header.columns.end).min(area.right()),
        })
        .collect()
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

/// Reading freshness controls account and role emphasis, independently of the read-status dot.
enum Freshness {
    Fresh,
    Stale,
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
                    Self::Stale
                } else {
                    Self::Fresh
                }
            },
        }
    }

    fn style(&self) -> Style {
        match self {
            Self::Fresh => Style::default().fg(text_default()),
            Self::Stale | Self::Unread => dim(),
        }
    }
}

/// A reset uses the same calendar cutoffs as the accounts board, in local time.
fn reset_label(at: DateTime<Utc>, now: u64) -> String {
    let at = (at + Duration::seconds(RESET_ROUNDING_SECONDS)).with_timezone(&Local);
    let format = i64::try_from(now)
        .ok()
        .and_then(|seconds| DateTime::from_timestamp(seconds, 0))
        .map_or("%d %b", |now| {
            match at
                .date_naive()
                .signed_duration_since(now.with_timezone(&Local).date_naive())
                .num_days()
                .unsigned_abs()
            {
                0 => "%H:%M",
                1..=6 => RESET_TIME_FORMAT,
                _ => "%d %b",
            }
        });
    at.format(format).to_string()
}

/// Round the source balance once, then group its integer digits in thousands.
fn credit_amount(amount: f64) -> String {
    let rounded = format!("{:.0}", amount.round());
    let (sign, digits) = rounded
        .strip_prefix('-')
        .map_or(("", rounded.as_str()), |digits| ("-", digits));
    let mut grouped = sign.to_owned();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    grouped
}

fn values(row: &AccountUsage, now: u64) -> [String; AccountsColumn::ALL.len()] {
    AccountsColumn::ALL.map(|column| match column {
        AccountsColumn::Account => row.label.clone(),
        AccountsColumn::WeeklyLeft => match row.weekly_left_percent {
            Reading::Known(left) => format!("{:.0}%", left.round()),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        AccountsColumn::Resets => match row.resets_at {
            Reading::Known(at) => reset_label(at, now),
            Reading::Unavailable => String::new(),
        },
        AccountsColumn::Credits => match row.credits {
            Credits::Amount(amount) => credit_amount(amount),
            Credits::Unlimited => UNLIMITED_CREDITS_LABEL.to_owned(),
            Credits::Unavailable => String::new(),
        },
        AccountsColumn::Enabled => match row.credit_permission {
            CreditPermission::Enabled => "yes".to_owned(),
            CreditPermission::Disabled => String::new(),
        },
        _ => String::new(),
    })
}

/// `total_values` sums the seven counts from recorded roles.
fn total_values(reading: &AccountsReading) -> [String; AccountsColumn::ALL.len()] {
    let counts = summed_counts(reading.accounts.iter().flat_map(|account| &account.roles));
    AccountsColumn::ALL.map(|column| match column {
        AccountsColumn::Account => "total".to_owned(),
        AccountsColumn::Busy
        | AccountsColumn::Starting
        | AccountsColumn::Waiting
        | AccountsColumn::Idle
        | AccountsColumn::Paused
        | AccountsColumn::Exited
        | AccountsColumn::Total => match count_value(&counts, column) {
            Reading::Known(count) => count.to_string(),
            Reading::Unavailable => String::new(),
        },
        AccountsColumn::WeeklyLeft
        | AccountsColumn::Roles
        | AccountsColumn::Resets
        | AccountsColumn::Credits
        | AccountsColumn::Enabled => String::new(),
    })
}

/// The width chooses exactly one presentation for headers, rows and selection.
#[derive(Clone, Copy)]
enum AccountsLayout {
    Full,
    TwoLine,
    Stacked,
}

/// A checked column set and its presentation always travel together.
struct AccountsFit {
    columns: Vec<AccountsColumn>,
    layout:  AccountsLayout,
    widths:  [u16; AccountsColumn::ALL.len()],
}

/// Keeps the start of a management role readable when one-line rows need room.
const MINIMUM_ROLES_WIDTH: u16 = 6;

impl AccountsFit {
    fn for_width(width: u16, widths: &[u16; AccountsColumn::ALL.len()]) -> Self {
        let mut widths = *widths;
        let mut columns = AccountsColumn::ALL.to_vec();
        for drop in AccountsColumnDrop::ORDER {
            if column_width(&widths, &columns, account_name_indent()) <= width {
                break;
            }
            columns.retain(|&column| column != drop.column());
        }
        let full_width = column_width(&widths, &columns, account_name_indent());
        let roles = &mut widths[AccountsColumn::Roles.index()];
        *roles = roles
            .saturating_sub(full_width.saturating_sub(width))
            .max(MINIMUM_ROLES_WIDTH.min(*roles));
        let layout = if column_width(&widths, &columns, account_name_indent()) <= width {
            AccountsLayout::Full
        } else {
            let split = columns.partition_point(|column| on_first_line(*column));
            if column_width(&widths, &columns[..split], account_name_indent()) <= width
                && column_width(&widths, &columns[split..], account_name_indent()) <= width
            {
                AccountsLayout::TwoLine
            } else {
                AccountsLayout::Stacked
            }
        };
        Self {
            columns,
            layout,
            widths,
        }
    }
}

/// Role lines contain the counts. One account detail line contains the remaining columns.
const fn on_first_line(column: AccountsColumn) -> bool {
    match column {
        AccountsColumn::Account
        | AccountsColumn::Roles
        | AccountsColumn::WeeklyLeft
        | AccountsColumn::Busy
        | AccountsColumn::Starting
        | AccountsColumn::Waiting
        | AccountsColumn::Idle
        | AccountsColumn::Paused
        | AccountsColumn::Exited
        | AccountsColumn::Total => true,
        AccountsColumn::Resets | AccountsColumn::Credits | AccountsColumn::Enabled => false,
    }
}

/// Only these columns can leave the accounts table, in this order.
#[derive(Clone, Copy)]
enum AccountsColumnDrop {
    Enabled,
    Credits,
}

impl AccountsColumnDrop {
    const ORDER: [Self; 2] = [Self::Enabled, Self::Credits];

    const fn column(self) -> AccountsColumn {
        match self {
            Self::Enabled => AccountsColumn::Enabled,
            Self::Credits => AccountsColumn::Credits,
        }
    }
}

/// Both table lines start their values after the read dot and its column gap.
fn account_name_indent() -> u16 {
    cell_width(SECTION_ITEM_INDENT) + cell_width(ACCOUNT_READ_DOT) + TABLE_COLUMN_SPACING
}

fn column_width(
    widths: &[u16; AccountsColumn::ALL.len()],
    columns: &[AccountsColumn],
    indent: u16,
) -> u16 {
    let count = u16::try_from(columns.len()).unwrap_or(u16::MAX);
    columns
        .iter()
        .map(|column| widths[column.index()])
        .fold(indent, u16::saturating_add)
        .saturating_add(TABLE_COLUMN_SPACING.saturating_mul(count.saturating_sub(1)))
}

/// Coordinates belong to the very lines supplying demand and painting.
pub(super) struct AccountLines {
    pub(super) lines:      Vec<Line<'static>>,
    pub(super) headers:    Vec<HeaderSpan<AccountsColumn>>,
    pub(super) selections: Vec<(String, RowSpan)>,
}

/// Each line set uses its own indentation and the same numeric alignment.
fn column_line(
    prefix: Vec<Span<'static>>,
    cells: &[Line<'static>; AccountsColumn::ALL.len()],
    widths: &[u16; AccountsColumn::ALL.len()],
    columns: &[AccountsColumn],
) -> Line<'static> {
    let mut spans = prefix;
    for (position, &column) in columns.iter().enumerate() {
        let index = column.index();
        let value = &cells[index];
        let padding =
            widths[index].saturating_sub(u16::try_from(value.width()).unwrap_or(u16::MAX));
        let right = column.alignment() == Alignment::Right;
        if right {
            spans.push(Span::raw(" ".repeat(usize::from(padding))));
        }
        spans.extend(value.spans.iter().cloned());
        if position + 1 < columns.len() {
            let padding = if right { 0 } else { padding };
            spans.push(Span::raw(
                " ".repeat(usize::from(TABLE_COLUMN_SPACING + padding)),
            ));
        }
    }
    Line::from(spans)
}

/// Pack styled words. Keep separators separate and punctuation with the preceding word.
fn wrap_line(line: Line<'static>, width: u16, indent: u16) -> Vec<Line<'static>> {
    let room = width.saturating_sub(indent).max(1);
    let mut words: Vec<Line<'static>> = Vec::new();
    let mut word = Line::default();
    for span in line.spans {
        for part in span.content.split_inclusive(char::is_whitespace) {
            let text = part.trim_end_matches(char::is_whitespace);
            if !text.is_empty() {
                word.spans.push(Span::styled(text.to_owned(), span.style));
            }
            if part.ends_with(char::is_whitespace) && !word.spans.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        }
    }
    if !word.spans.is_empty() {
        words.push(word);
    }
    let mut attached: Vec<Line<'static>> = Vec::new();
    for word in words {
        let punctuation = word
            .spans
            .iter()
            .all(|span| span.content.chars().all(|ch| matches!(ch, ',' | ':' | '·')));
        if punctuation && let Some(previous) = attached.last_mut() {
            if word.spans.iter().any(|span| span.content.contains('·')) {
                previous.spans.push(Span::raw(" "));
            }
            previous.spans.extend(word.spans);
        } else {
            attached.push(word);
        }
    }
    let mut result = Vec::new();
    let mut line = Line::default();
    for word in attached {
        let word = super::truncated_line(word, usize::from(room));
        let gap = usize::from(!line.spans.is_empty());
        if line.width() + gap + word.width() > usize::from(room) && !line.spans.is_empty() {
            line.spans
                .insert(0, Span::raw(" ".repeat(usize::from(indent))));
            result.push(std::mem::take(&mut line));
        }
        if !line.spans.is_empty() {
            line.spans.push(Span::raw(" "));
        }
        line.spans.extend(word.spans);
    }
    line.spans
        .insert(0, Span::raw(" ".repeat(usize::from(indent))));
    result.push(line);
    result
}

/// Facts stay whole unless a single fact exceeds the available line width.
fn stacked_facts(
    cells: &[Line<'static>; AccountsColumn::ALL.len()],
    width: u16,
    fit: &AccountsFit,
) -> Vec<Line<'static>> {
    let mut facts = Vec::new();
    let facts_columns = std::iter::once((AccountsColumn::WeeklyLeft, String::new(), " left"))
        .chain(AccountsColumn::COUNTS.map(|column| (column, format!("{} ", column.header()), "")))
        .chain([
            (AccountsColumn::Resets, "resets ".to_owned(), ""),
            (AccountsColumn::Credits, "credits ".to_owned(), ""),
            (AccountsColumn::Enabled, String::new(), ""),
        ]);
    for (column, prefix, suffix) in facts_columns {
        let index = column.index();
        if !fit.columns.contains(&column) || cells[index].width() == 0 {
            continue;
        }
        let mut fact = cells[index].clone();
        if column == AccountsColumn::Enabled {
            fact = Line::from(Span::styled("enabled", fact.spans[0].style));
        } else {
            let style = fact
                .spans
                .first()
                .map_or_else(Style::default, |span| span.style);
            fact.spans.insert(0, Span::styled(prefix, style));
            fact.spans.push(Span::styled(suffix, style));
        }
        facts.push(fact);
    }
    let indent = (cell_width(SECTION_ITEM_INDENT) + 2).min(width.saturating_sub(1));
    let room = width.saturating_sub(indent);
    let mut result = Vec::new();
    let mut line = Line::default();
    for fact in facts {
        let gap = if line.width() == 0 { 0 } else { 3 };
        if line.width() + gap + fact.width() > usize::from(room) && line.width() > 0 {
            result.extend(wrap_line(std::mem::take(&mut line), width, indent));
        }
        if fact.width() > usize::from(room) {
            result.extend(wrap_line(fact, width, indent));
        } else {
            if line.width() > 0 {
                line.spans.push(Span::raw(" · "));
            }
            line.spans.extend(fact.spans);
        }
    }
    if line.width() > 0 {
        result.extend(wrap_line(line, width, indent));
    }
    result
}

impl AccountLines {
    /// Draw one role row with the fitted columns or stacked facts.
    fn push_role_row(
        &mut self,
        fit: &AccountsFit,
        dot: Span<'static>,
        cells: &[Line<'static>; AccountsColumn::ALL.len()],
        width: u16,
    ) {
        match fit.layout {
            AccountsLayout::Full => self.push_columns(fit, &dot, cells, false),
            AccountsLayout::TwoLine => {
                let split = fit.columns.partition_point(|column| on_first_line(*column));
                self.push_column_set(fit, &dot, cells, &fit.columns[..split], false);
            },
            AccountsLayout::Stacked => {
                let mut heading = vec![dot, Span::raw(" ")];
                heading.extend(cells[0].spans.iter().cloned());
                if cells[AccountsColumn::Roles.index()].width() > 0 {
                    heading.push(Span::raw("  "));
                    heading.extend(cells[AccountsColumn::Roles.index()].spans.iter().cloned());
                }
                self.lines.extend(wrap_line(
                    Line::from(heading),
                    width,
                    cell_width(SECTION_ITEM_INDENT).min(width.saturating_sub(1)),
                ));
                self.lines.extend(stacked_facts(cells, width, fit));
            },
        }
    }

    /// Append source problems and the last refresh failure.
    fn push_notes(
        &mut self,
        reading: &AccountsReading,
        failure: FailureNote<'_>,
        width: u16,
        now: u64,
    ) {
        for problem in &reading.problems {
            self.lines
                .extend(wrap_line(note(problem.clone()), width, 0));
        }
        if let FailureNote::Failed(failure) = failure {
            self.lines.extend(wrap_line(
                note(format!(
                    "{failure}; showing the reading from {} min ago",
                    now.saturating_sub(reading.received_at) / 60
                )),
                width,
                0,
            ));
        }
    }

    /// Build the section heading and fitted column headers with their hit regions.
    fn push_headers(&mut self, fit: &AccountsFit, sort: AccountsSort) {
        self.lines.push(Line::from(Span::styled(
            format!("{SECTION_HEADER_INDENT}{ACCOUNTS_HEADING}"),
            Style::default()
                .fg(text_default())
                .add_modifier(Modifier::BOLD),
        )));
        let headers = std::array::from_fn(|index| {
            let column = AccountsColumn::ALL[index];
            super::truncated_line(
                Line::from(Span::styled(
                    sort.header(column),
                    Style::default().fg(label_color()),
                )),
                usize::from(fit.widths[index]),
            )
        });
        self.push_columns(fit, &Span::raw(" "), &headers, true);
    }

    fn push_columns(
        &mut self,
        fit: &AccountsFit,
        dot: &Span<'static>,
        cells: &[Line<'static>; AccountsColumn::ALL.len()],
        headers: bool,
    ) {
        let split = fit.columns.partition_point(|column| on_first_line(*column));
        let sets: &[&[AccountsColumn]] = match fit.layout {
            AccountsLayout::Full => &[&fit.columns],
            AccountsLayout::TwoLine => &[&fit.columns[..split], &fit.columns[split..]],
            AccountsLayout::Stacked => &[],
        };
        for &columns in sets {
            self.push_column_set(fit, dot, cells, columns, headers);
        }
    }

    fn push_column_set(
        &mut self,
        fit: &AccountsFit,
        dot: &Span<'static>,
        cells: &[Line<'static>; AccountsColumn::ALL.len()],
        columns: &[AccountsColumn],
        headers: bool,
    ) {
        let widths = &fit.widths;
        let start = columns[0];
        let prefix = if start == AccountsColumn::Account {
            vec![
                Span::raw(SECTION_ITEM_INDENT),
                dot.clone(),
                Span::raw(" ".repeat(usize::from(TABLE_COLUMN_SPACING))),
            ]
        } else {
            vec![Span::raw(" ".repeat(usize::from(account_name_indent())))]
        };
        if headers {
            let mut x = prefix
                .iter()
                .map(|span| cell_width(&span.content))
                .sum::<u16>();
            for &column in columns {
                let width = widths[column.index()];
                self.headers.push(HeaderSpan {
                    column,
                    columns: x..x.saturating_add(width),
                    row: u16::try_from(self.lines.len()).unwrap_or(u16::MAX),
                });
                x = x.saturating_add(width).saturating_add(TABLE_COLUMN_SPACING);
            }
        }
        self.lines.push(column_line(prefix, cells, widths, columns));
    }
}

/// Management roles use footer ink; other words and punctuation keep the row style.
fn role_words(
    role: &AccountRoleUsage,
    freshness: &Freshness,
    colors: &FooterColors,
) -> Line<'static> {
    let base = freshness.style();
    let tint = match role.role {
        AccountRole::Units => colors.role(FooterRole::UnitDirector),
        AccountRole::Showrunners => colors.role(FooterRole::Showrunner),
        AccountRole::Workers | AccountRole::Spare => FooterTint::Plain,
    };
    let style = match freshness {
        Freshness::Fresh => tint.style(base),
        Freshness::Stale | Freshness::Unread => tint.subdued_style(base),
    };
    Line::from(Span::styled(role.text(), style))
}

/// Source positions resolve equal sort keys in the same order for painting and selection.
fn ordered_rows(reading: &AccountsReading, sort: AccountsSort) -> Vec<(usize, &AccountUsage)> {
    let mut ordered: Vec<_> = reading.accounts.iter().enumerate().collect();
    if let AccountsSort::Sorted(sort) = sort {
        ordered.sort_by(|(left_index, left), (right_index, right)| {
            compare_accounts(left, right, sort).then_with(|| left_index.cmp(right_index))
        });
    }
    ordered
}

/// Apply the account, role and readable numeric inks before choosing row geometry.
fn row_cells(
    row: &AccountUsage,
    values: [String; AccountsColumn::ALL.len()],
    freshness: &Freshness,
    colors: &FooterColors,
    palette: &BTreeMap<DateTime<Utc>, Color>,
    count_style: Style,
) -> [Line<'static>; AccountsColumn::ALL.len()] {
    let base = freshness.style();
    let mut cells = values.map(|value| Span::styled(value, base));
    cells[0].style = match freshness {
        Freshness::Fresh => colors.account(&row.label).style(base),
        Freshness::Stale | Freshness::Unread => colors.account(&row.label).subdued_style(base),
    };
    let numeric = Style::default().fg(text_default());
    cells[AccountsColumn::WeeklyLeft.index()].style = match row.weekly_left_percent {
        Reading::Known(left) if left < ACCOUNTS_LOW_PERCENT => {
            footer_colors::readable_status_style(numeric.fg(LOW_WEEKLY_USAGE_COLOR))
        },
        Reading::Known(left) if left < 25.0 => {
            footer_colors::readable_status_style(numeric.fg(Color::Yellow))
        },
        Reading::Known(_) | Reading::Unavailable => numeric,
    };
    cells[AccountsColumn::Resets.index()].style = match row.resets_at {
        Reading::Known(at) => footer_colors::readable_status_style(numeric.fg(palette[&at])),
        Reading::Unavailable => numeric,
    };
    cells[AccountsColumn::Credits.index()].style = match row.credit_permission {
        CreditPermission::Enabled => numeric,
        CreditPermission::Disabled => {
            footer_colors::readable_status_style(FooterTint::Plain.subdued_style(numeric))
        },
    };
    for column in AccountsColumn::COUNTS {
        cells[column.index()].style = count_style;
    }
    cells.map(Line::from)
}

/// Set the role and count cells for one account row.
fn fill_role_values(values: &mut [String; AccountsColumn::ALL.len()], role: &AccountRoleUsage) {
    role.text()
        .clone_into(&mut values[AccountsColumn::Roles.index()]);
    for column in AccountsColumn::COUNTS {
        values[column.index()] = match role_count(role, column) {
            Reading::Known(count) => count.to_string(),
            Reading::Unavailable => String::new(),
        };
    }
}

/// One drawn role row belongs to an account group.
#[derive(Clone)]
struct AccountRoleRow {
    values: [String; AccountsColumn::ALL.len()],
    role:   Line<'static>,
}

/// An account group owns its first row, freshness and role continuations.
struct AccountGroup<'a> {
    account:       &'a AccountUsage,
    freshness:     Freshness,
    first:         AccountRoleRow,
    continuations: Iter<'a, AccountRoleUsage>,
}

impl<'a> AccountGroup<'a> {
    fn new(account: &'a AccountUsage, now: u64, colors: &FooterColors) -> Self {
        let freshness = Freshness::new(account, now);
        let mut roles = account.roles.iter();
        let mut first = AccountRoleRow {
            values: values(account, now),
            role:   Line::default(),
        };
        if let Some(role) = roles.next() {
            fill_role_values(&mut first.values, role);
            first.role = role_words(role, &freshness, colors);
        }
        Self {
            account,
            freshness,
            first,
            continuations: roles,
        }
    }

    fn rows<'b>(&'b self, colors: &'b FooterColors) -> impl Iterator<Item = AccountRoleRow> + 'b {
        std::iter::once(self.first.clone()).chain(self.continuations.clone().map(|role| {
            let mut values = std::array::from_fn(|_| String::new());
            fill_role_values(&mut values, role);
            AccountRoleRow {
                values,
                role: role_words(role, &self.freshness, colors),
            }
        }))
    }

    fn draw(
        self,
        result: &mut AccountLines,
        fit: &AccountsFit,
        colors: &FooterColors,
        palette: &BTreeMap<DateTime<Utc>, Color>,
        count_style: Style,
        width: u16,
    ) {
        let first = result.lines.len();
        let dot = match self.account.read_status {
            AccountReadStatus::Ok => tui_pane::success_color(),
            AccountReadStatus::Failed => tui_pane::error_color(),
        };
        let mut mark = Span::styled(ACCOUNT_READ_DOT, Style::default().fg(dot));
        for row in self.rows(colors) {
            let mut cells = row_cells(
                self.account,
                row.values,
                &self.freshness,
                colors,
                palette,
                count_style,
            );
            cells[AccountsColumn::Roles.index()] = super::truncated_line(
                row.role,
                usize::from(fit.widths[AccountsColumn::Roles.index()]),
            );
            result.push_role_row(fit, mark, &cells, width);
            mark = Span::raw(" ");
        }
        if matches!(fit.layout, AccountsLayout::TwoLine) {
            let cells = row_cells(
                self.account,
                self.first.values,
                &self.freshness,
                colors,
                palette,
                count_style,
            );
            let split = fit.columns.partition_point(|column| on_first_line(*column));
            result.push_column_set(fit, &Span::raw(" "), &cells, &fit.columns[split..], false);
        }
        result.selections.push((
            self.account.label.clone(),
            RowSpan::single(first).with_continuations(result.lines.len() - first - 1),
        ));
    }
}

/// Resolve one reading into ordered, measured lines for drawing and hit testing.
pub(super) fn build_lines(
    table: &AccountsTable,
    now: u64,
    colors: &FooterColors,
    sort: AccountsSort,
    width: u16,
) -> AccountLines {
    let mut result = AccountLines {
        lines:      Vec::new(),
        headers:    Vec::new(),
        selections: Vec::new(),
    };
    let (reading, failure) = match AccountsSection::from(table) {
        AccountsSection::Hidden => return result,
        AccountsSection::Failure(failure) => {
            result.lines = wrap_line(note(failure.to_string()), width, 0);
            return result;
        },
        AccountsSection::Reading { reading, note } => (reading, note),
    };
    let palette = reset_colors(reading);
    let count_style = footer_colors::readable_status_style(Style::default().fg(text_default()));
    let groups: Vec<_> = ordered_rows(reading, sort)
        .into_iter()
        .map(|(_, row)| AccountGroup::new(row, now, colors))
        .collect();
    let totals = total_values(reading);
    let mut widths = std::array::from_fn(|column| {
        cell_width(&sort.header(AccountsColumn::ALL[column])).max(cell_width(&totals[column]))
    });
    for row in groups.iter().flat_map(|group| group.rows(colors)) {
        for column in AccountsColumn::ALL {
            let index = column.index();
            widths[index] = widths[index].max(cell_width(&row.values[index]));
        }
    }
    let fit = AccountsFit::for_width(width, &widths);
    result.push_headers(&fit, sort);
    for group in groups {
        group.draw(&mut result, &fit, colors, &palette, count_style, width);
    }
    if !reading.accounts.is_empty() {
        let mut cells = totals.map(|value| Line::from(Span::styled(value, count_style)));
        for span in &mut cells[AccountsColumn::Account.index()].spans {
            span.style = span.style.add_modifier(Modifier::BOLD);
        }
        result.push_role_row(&fit, Span::raw(" "), &cells, width);
    }
    result.push_notes(reading, failure, width, now);
    result
}
