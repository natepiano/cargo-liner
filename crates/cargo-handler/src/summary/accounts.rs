//! The source-ordered account table shares measurement and drawing lines.

use chrono::Duration;
use chrono::Local;
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
use crate::census::accounts::AccountUsage;
use crate::census::accounts::AccountsFailure;
use crate::census::accounts::AccountsReading;
use crate::census::accounts::AccountsTable;
use crate::census::accounts::CreditUse;
use crate::census::accounts::Credits;
use crate::census::accounts::Reading;
use crate::constants::ACCOUNTS_HEADERS;
use crate::constants::ACCOUNTS_LOW_PERCENT;
use crate::constants::ACCOUNTS_STALE_SECONDS;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::MISSING_VALUE;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::footer_colors::FooterColors;

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

pub(super) fn width(table: &AccountsTable, now: u64) -> u16 {
    lines(table, now, &FooterColors::default())
        .iter()
        .map(|line| u16::try_from(line.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or_default()
}

pub(super) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    table: &AccountsTable,
    now: u64,
    colors: &FooterColors,
) {
    Paragraph::new(lines(table, now, colors)).render(area, buffer);
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
            Self::Stale(age) => format!("{account}{HEADING_SEPARATOR}{age} old"),
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
            .join(", ")
    };
    [
        freshness.label(&row.label),
        roles,
        match row.weekly_left_percent {
            Reading::Known(left) => format!("{left}%"),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        match row.resets_at {
            Reading::Known(at) => (at + Duration::seconds(30))
                .with_timezone(&Local)
                .format("%a %H:%M")
                .to_string(),
            Reading::Unavailable => MISSING_VALUE.to_owned(),
        },
        match row.credits {
            Credits::Amount(amount) => amount.to_string(),
            Credits::Unlimited => "unlimited".to_owned(),
            Credits::Unavailable => MISSING_VALUE.to_owned(),
        },
    ]
}

/// Pad with display widths so account labels and notes determine the same demand drawn.
fn columns(values: [Span<'static>; 5], widths: &[u16; 5]) -> Line<'static> {
    let mut spans = vec![Span::raw(SECTION_ITEM_INDENT)];
    for (index, value) in values.into_iter().enumerate() {
        let width = cell_width(&value.content);
        spans.push(value);
        if index + 1 < widths.len() {
            spans.push(Span::raw(
                " ".repeat(usize::from(
                    widths[index]
                        .saturating_sub(width)
                        .saturating_add(TABLE_COLUMN_SPACING),
                )),
            ));
        }
    }
    Line::from(spans)
}

fn lines(table: &AccountsTable, now: u64, colors: &FooterColors) -> Vec<Line<'static>> {
    let (reading, failure) = match AccountsSection::from(table) {
        AccountsSection::Hidden => return Vec::new(),
        AccountsSection::Failure(failure) => return vec![note(failure.to_string())],
        AccountsSection::Reading { reading, note } => (reading, note),
    };
    let rows: Vec<_> = reading
        .accounts
        .iter()
        .map(|row| {
            let freshness = Freshness::new(row, now);
            (values(row, &freshness), freshness)
        })
        .collect();
    let widths = std::array::from_fn(|column| {
        rows.iter()
            .map(|(values, _)| cell_width(&values[column]))
            .fold(cell_width(ACCOUNTS_HEADERS[column]), u16::max)
    });
    let mut result = vec![
        Line::from(Span::styled(
            format!("{SECTION_HEADER_INDENT}accounts"),
            Style::default()
                .fg(text_default())
                .add_modifier(Modifier::BOLD),
        )),
        columns(
            ACCOUNTS_HEADERS
                .map(|header| Span::styled(header.to_owned(), Style::default().fg(label_color()))),
            &widths,
        ),
    ];
    for (row, (values, freshness)) in reading.accounts.iter().zip(rows) {
        let base = freshness.style();
        let mut cells = values.map(|value| Span::styled(value, base));
        cells[0].style = colors.account(&row.label).style(base);
        if matches!(row.weekly_left_percent, Reading::Known(left) if left <= ACCOUNTS_LOW_PERCENT) {
            cells[2].style = base.fg(Color::Red);
        }
        if row.credit_use == CreditUse::Held {
            cells[4].style = base.add_modifier(Modifier::DIM);
        }
        result.push(columns(cells, &widths));
    }
    result.extend(reading.problems.iter().map(|problem| note(problem.clone())));
    if let FailureNote::Failed(failure) = failure {
        result.push(note(failure.to_string()));
    }
    result
}
