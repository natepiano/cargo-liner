//! Account ordering, reset colors and headers share measurement and drawing lines.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use chrono::DateTime;
use chrono::Duration;
use chrono::Local;
use chrono::Utc;
#[cfg(test)]
use ratatui::buffer::Buffer;
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
use super::constants::ACCOUNT_ROLE_SEPARATOR;
use super::constants::ACCOUNTS_HEADING;
use super::constants::LOW_WEEKLY_USAGE_COLOR;
use super::constants::RESET_ROUNDING_SECONDS;
use super::constants::RESET_TIME_FORMAT;
use super::constants::UNLIMITED_CREDITS_LABEL;
use crate::census::accounts::AccountReadStatus;
use crate::census::accounts::AccountRole;
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditActivity;
use crate::census::accounts::CreditPermission;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
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
        AccountsColumn::Enabled => match (left.credit_permission, right.credit_permission) {
            (CreditPermission::Enabled, CreditPermission::Disabled) => Ordering::Less,
            (CreditPermission::Disabled, CreditPermission::Enabled) => Ordering::Greater,
            _ => Ordering::Equal,
        },
        AccountsColumn::InUse => match (left.credit_activity, right.credit_activity) {
            (CreditActivity::InUse, CreditActivity::Idle) => Ordering::Less,
            (CreditActivity::Idle, CreditActivity::InUse) => Ordering::Greater,
            _ => Ordering::Equal,
        },
        AccountsColumn::Read => compare_readings(
            &left.read_at,
            &right.read_at,
            sort.direction,
            |left, right| right.cmp(left),
        ),
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

#[cfg(test)]
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

/// Reading ages use only their largest whole unit, without an "ago" suffix.
fn read_age(at: DateTime<Utc>, now: u64) -> String {
    let seconds = now.saturating_sub(u64::try_from(at.timestamp()).unwrap_or_default());
    let (size, unit) = [(86_400, "d"), (3_600, "h"), (60, "m"), (1, "s")]
        .into_iter()
        .find(|(size, _)| seconds >= *size)
        .unwrap_or((1, "s"));
    format!("{}{unit}", seconds / size)
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
    let roles = if row.roles.is_empty() {
        String::new()
    } else {
        row.roles
            .iter()
            .map(|role| role.text())
            .collect::<Vec<_>>()
            .join(ACCOUNT_ROLE_SEPARATOR)
    };
    [
        row.label.clone(),
        roles,
        match row.weekly_left_percent {
            Reading::Known(left) => format!("{:.0}%", left.round()),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        match row.resets_at {
            Reading::Known(at) => reset_label(at, now),
            Reading::Unavailable => String::new(),
        },
        match row.credits {
            Credits::Amount(amount) => credit_amount(amount),
            Credits::Unlimited => UNLIMITED_CREDITS_LABEL.to_owned(),
            Credits::Unavailable => String::new(),
        },
        match row.credit_permission {
            CreditPermission::Enabled => "yes".to_owned(),
            CreditPermission::Disabled => String::new(),
        },
        match row.credit_activity {
            CreditActivity::InUse => "yes".to_owned(),
            CreditActivity::Idle => String::new(),
        },
        match row.read_at {
            Reading::Known(at) => read_age(at, now),
            Reading::Unavailable => String::new(),
        },
    ]
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

/// The continuation starts at Resets; account, role and quota share the first line.
const fn on_first_line(column: AccountsColumn) -> bool {
    match column {
        AccountsColumn::Account | AccountsColumn::Roles | AccountsColumn::WeeklyLeft => true,
        AccountsColumn::Resets
        | AccountsColumn::Credits
        | AccountsColumn::Enabled
        | AccountsColumn::InUse
        | AccountsColumn::Read => false,
    }
}

/// Only these columns can leave the accounts table, in this order.
#[derive(Clone, Copy)]
enum AccountsColumnDrop {
    InUse,
    Enabled,
    Read,
    Credits,
}

impl AccountsColumnDrop {
    const ORDER: [Self; 4] = [Self::InUse, Self::Enabled, Self::Read, Self::Credits];

    const fn column(self) -> AccountsColumn {
        match self {
            Self::InUse => AccountsColumn::InUse,
            Self::Enabled => AccountsColumn::Enabled,
            Self::Read => AccountsColumn::Read,
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
        let right = matches!(
            column,
            AccountsColumn::WeeklyLeft
                | AccountsColumn::Resets
                | AccountsColumn::Credits
                | AccountsColumn::Read
        );
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

/// Pack styled words; punctuation belongs to the preceding word.
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
    for (column, prefix, suffix) in [
        (AccountsColumn::WeeklyLeft, "", " left"),
        (AccountsColumn::Resets, "resets ", ""),
        (AccountsColumn::Credits, "credits ", ""),
        (AccountsColumn::Enabled, "", ""),
        (AccountsColumn::InUse, "", ""),
        (AccountsColumn::Read, "read ", " ago"),
    ] {
        let index = column.index();
        if !fit.columns.contains(&column) || cells[index].width() == 0 {
            continue;
        }
        let mut fact = cells[index].clone();
        match column {
            AccountsColumn::Enabled => {
                fact = Line::from(Span::styled("enabled", fact.spans[0].style));
            },
            AccountsColumn::InUse => fact = Line::from(Span::styled("in use", fact.spans[0].style)),
            _ => {
                let style = fact
                    .spans
                    .first()
                    .map_or_else(Style::default, |span| span.style);
                fact.spans.insert(0, Span::styled(prefix, style));
                fact.spans.push(Span::styled(suffix, style));
            },
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
            let mut header = super::truncated_line(
                Line::from(Span::styled(
                    sort.header(column),
                    Style::default().fg(label_color()),
                )),
                usize::from(fit.widths[index]),
            );
            if matches!(fit.layout, AccountsLayout::TwoLine) && column == AccountsColumn::Resets {
                // The continuation header anchors the line; reset values stay right-aligned.
                let padding = usize::from(fit.widths[index]).saturating_sub(header.width());
                header.spans.push(Span::raw(" ".repeat(padding)));
            }
            header
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
        let widths = &fit.widths;
        let split = fit.columns.partition_point(|column| on_first_line(*column));
        let sets: &[&[AccountsColumn]] = match fit.layout {
            AccountsLayout::Full => &[&fit.columns],
            AccountsLayout::TwoLine => &[&fit.columns[..split], &fit.columns[split..]],
            AccountsLayout::Stacked => &[],
        };
        for &columns in sets {
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
}

/// Management roles use footer ink; other words and punctuation keep the row style.
fn role_words(row: &AccountUsage, freshness: &Freshness, colors: &FooterColors) -> Line<'static> {
    let base = freshness.style();
    if row.roles.is_empty() {
        return Line::default();
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
            Freshness::Stale | Freshness::Unread => tint.subdued_style(base),
        };
        spans.push(Span::styled(role.text(), style));
    }
    Line::from(spans)
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
) -> [Line<'static>; AccountsColumn::ALL.len()] {
    let base = freshness.style();
    let mut cells = values.map(|value| Span::styled(value, base));
    cells[0].style = match freshness {
        Freshness::Fresh => colors.account(&row.label).style(base),
        Freshness::Stale | Freshness::Unread => colors.account(&row.label).subdued_style(base),
    };
    let numeric = Style::default().fg(text_default());
    cells[2].style = match row.weekly_left_percent {
        Reading::Known(left) if left < ACCOUNTS_LOW_PERCENT => {
            footer_colors::readable_status_style(numeric.fg(LOW_WEEKLY_USAGE_COLOR))
        },
        Reading::Known(left) if left < 25.0 => {
            footer_colors::readable_status_style(numeric.fg(Color::Yellow))
        },
        Reading::Known(_) | Reading::Unavailable => numeric,
    };
    cells[3].style = match row.resets_at {
        Reading::Known(at) => footer_colors::readable_status_style(numeric.fg(palette[&at])),
        Reading::Unavailable => numeric,
    };
    cells[4].style = match row.credit_permission {
        CreditPermission::Enabled => numeric,
        CreditPermission::Disabled => {
            footer_colors::readable_status_style(FooterTint::Plain.subdued_style(numeric))
        },
    };
    cells[7].style = footer_colors::readable_status_style(numeric);
    let mut cells = cells.map(Line::from);
    cells[1] = role_words(row, freshness, colors);
    cells
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
    let ordered = ordered_rows(reading, sort);
    let rows: Vec<_> = ordered
        .iter()
        .map(|(_, row)| *row)
        .map(|row| {
            let freshness = Freshness::new(row, now);
            (values(row, now), freshness)
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
    let fit = AccountsFit::for_width(width, &widths);
    let widths = &fit.widths;
    result.push_headers(&fit, sort);
    for ((_, row), (values, freshness)) in ordered.into_iter().zip(rows) {
        let mut cells = row_cells(row, values, &freshness, colors, &palette);
        let roles = &mut cells[AccountsColumn::Roles.index()];
        *roles = super::truncated_line(
            std::mem::take(roles),
            usize::from(widths[AccountsColumn::Roles.index()]),
        );
        let dot = match row.read_status {
            AccountReadStatus::Ok => tui_pane::success_color(),
            AccountReadStatus::Failed => tui_pane::error_color(),
        };
        let first = result.lines.len();
        let dot = Span::styled(ACCOUNT_READ_DOT, Style::default().fg(dot));
        match fit.layout {
            AccountsLayout::Full | AccountsLayout::TwoLine => {
                result.push_columns(&fit, &dot, &cells, false);
            },
            AccountsLayout::Stacked => {
                let mut heading = vec![dot, Span::raw(" ")];
                heading.extend(cells[0].spans.iter().cloned());
                if cells[1].width() > 0 {
                    heading.push(Span::raw("  "));
                    heading.extend(cells[1].spans.iter().cloned());
                }
                result.lines.extend(wrap_line(
                    Line::from(heading),
                    width,
                    cell_width(SECTION_ITEM_INDENT).min(width.saturating_sub(1)),
                ));
                result.lines.extend(stacked_facts(&cells, width, &fit));
            },
        }
        result.selections.push((
            row.label.clone(),
            RowSpan::single(first).with_continuations(result.lines.len() - first - 1),
        ));
    }
    for problem in &reading.problems {
        result
            .lines
            .extend(wrap_line(note(problem.clone()), width, 0));
    }
    if let FailureNote::Failed(failure) = failure {
        result
            .lines
            .extend(wrap_line(note(failure.to_string()), width, 0));
    }
    result
}
