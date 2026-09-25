//! The summary cell: one label row for the whole cell, then each
//! machine's heading and its top-level agents, oldest first, with a
//! blank row between machines.

pub(crate) mod age;

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use tui_pane::ColumnSpec;
use tui_pane::ColumnWidths;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::accent_color;
use tui_pane::label_color;
use tui_pane::text_default;

use self::age::age_label;
use crate::census::Agent;
use crate::census::AgentRow;
use crate::census::Machine;
use crate::census::MachineState;
use crate::constants::AGE_COLUMN;
use crate::constants::AGENT_COLUMN;
use crate::constants::AGENT_PLURAL;
use crate::constants::AGENT_SINGULAR;
use crate::constants::BUSY_STATUS;
use crate::constants::DIRECTORY_COLUMN;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::MISSING_VALUE;
use crate::constants::NAME_COLUMN;
use crate::constants::NAME_COLUMN_MAX;
use crate::constants::NO_AGENTS_NOTE;
use crate::constants::SCANNING_NOTE;
use crate::constants::SHELL_STATUS;
use crate::constants::STATUS_COLUMN;
use crate::constants::SUMMARY_HEADERS;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TRUNCATION_MARK;
use crate::theme::Role;

/// Rows the summary draws for `machines`: the label row, each machine's
/// heading and rows, and a gap between one machine and the next.
pub(crate) fn height(machines: &[Machine<'_>]) -> usize {
    let groups: usize = machines
        .iter()
        .map(|machine| usize::from(GROUP_HEADER_HEIGHT) + machine.state.rows().len())
        .sum();
    let gaps = machines.len().saturating_sub(1) * usize::from(GROUP_GAP_HEIGHT);
    usize::from(TABLE_HEADER_HEIGHT) + groups + gaps
}

/// Draw `machines` into `area`, with ages measured to `now` in unix
/// seconds.
pub(crate) fn draw(buffer: &mut Buffer, area: Rect, machines: &[Machine<'_>], now: u64) {
    // One label row for the whole cell: every machine's table is laid
    // out with the same constraints and indent, so the labels stay over
    // their columns.
    let constraints = fitted_constraints(machines, now);
    let label = Style::default().fg(label_color());
    Table::new(Vec::<Row>::new(), constraints.iter().copied())
        .header(Row::new(
            SUMMARY_HEADERS.map(|header| Span::styled(header, label)),
        ))
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(
            Rect {
                height: TABLE_HEADER_HEIGHT.min(area.height),
                ..indented(area)
            },
            buffer,
        );

    let mut remaining = area;
    remaining.y = remaining.y.saturating_add(TABLE_HEADER_HEIGHT);
    remaining.height = remaining.height.saturating_sub(TABLE_HEADER_HEIGHT);
    for machine in machines {
        if remaining.height == 0 {
            break;
        }
        let used = draw_machine(buffer, remaining, machine, &constraints, now);
        remaining.y = remaining.y.saturating_add(used);
        remaining.height = remaining.height.saturating_sub(used);
    }
}

/// Draw one machine's heading and rows into the top of `area`,
/// answering how many rows that took including the gap below it.
fn draw_machine(
    buffer: &mut Buffer,
    area: Rect,
    machine: &Machine<'_>,
    constraints: &[Constraint],
    now: u64,
) -> u16 {
    let label = Style::default().fg(label_color());
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(machine.name, Style::default().fg(accent_color())),
    ];
    match machine.state {
        MachineState::Scanning => {
            heading.push(Span::styled(
                format!("{HEADING_SEPARATOR}{SCANNING_NOTE}"),
                label,
            ));
        },
        MachineState::Answered(rows) => {
            heading.push(Span::styled(
                format!("{HEADING_SEPARATOR}{}", count_note(rows.len())),
                label,
            ));
        },
        MachineState::Failed(reason) => {
            heading.push(Span::styled(HEADING_SEPARATOR, label));
            heading.push(Span::styled(reason.as_str(), Role::Unreachable.style()));
        },
    }
    Paragraph::new(Line::from(heading)).render(
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..area
        },
        buffer,
    );

    let rows = machine.state.rows();
    let table_height = area
        .height
        .saturating_sub(GROUP_HEADER_HEIGHT)
        .min(u16::try_from(rows.len()).unwrap_or(u16::MAX));
    Table::new(
        rows.iter().map(|row| agent_row(row, now)),
        constraints.iter().copied(),
    )
    .column_spacing(TABLE_COLUMN_SPACING)
    .render(
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: table_height,
            ..indented(area)
        },
        buffer,
    );
    GROUP_HEADER_HEIGHT
        .saturating_add(table_height)
        .saturating_add(GROUP_GAP_HEIGHT)
}

/// What a heading says about a machine that answered with `count`
/// agents.
fn count_note(count: usize) -> String {
    match count {
        0 => NO_AGENTS_NOTE.to_string(),
        1 => format!("1 {AGENT_SINGULAR}"),
        count => format!("{count} {AGENT_PLURAL}"),
    }
}

/// One agent's table row.
fn agent_row(row: &AgentRow, now: u64) -> Row<'static> {
    let agent_role = match row.agent {
        Agent::Claude => Role::Claude,
        Agent::Codex => Role::Codex,
    };
    let status_role = match row.status.as_deref() {
        Some(BUSY_STATUS) => Role::Busy,
        Some(SHELL_STATUS) => Role::Shell,
        _ => Role::Idle,
    };
    let label = Style::default().fg(label_color());
    Row::new([
        Span::styled(row.agent.label(), agent_role.style()),
        Span::styled(truncated(&row.name), Style::default().fg(text_default())),
        Span::styled(status_text(row).to_string(), status_role.style()),
        Span::styled(age_label(now.saturating_sub(row.started)), label),
        Span::styled(row.directory.clone(), label),
    ])
}

/// The status cell: the status, or [`MISSING_VALUE`].
fn status_text(row: &AgentRow) -> &str { row.status.as_deref().unwrap_or(MISSING_VALUE) }

/// `name` cut to [`NAME_COLUMN_MAX`] cells, ending in
/// [`TRUNCATION_MARK`] when it was longer.
fn truncated(name: &str) -> String {
    let max = usize::from(NAME_COLUMN_MAX);
    if name.chars().count() <= max {
        return name.to_string();
    }
    name.chars()
        .take(max - 1)
        .chain(std::iter::once(TRUNCATION_MARK))
        .collect()
}

/// Column widths fitted to the widest cell across every machine's rows.
/// `name` stops at [`NAME_COLUMN_MAX`], and `directory` takes whatever
/// the fitted columns leave.
fn fitted_constraints(machines: &[Machine<'_>], now: u64) -> Vec<Constraint> {
    let mut widths = ColumnWidths::new(
        SUMMARY_HEADERS
            .iter()
            .enumerate()
            .map(|(column, header)| ColumnSpec {
                min: cell_width(header),
                max: (column == NAME_COLUMN).then_some(NAME_COLUMN_MAX),
            })
            .collect(),
    );
    for row in machines.iter().flat_map(|machine| machine.state.rows()) {
        widths.observe_cell_usize(AGENT_COLUMN, row.agent.label().chars().count());
        widths.observe_cell_usize(NAME_COLUMN, row.name.chars().count());
        widths.observe_cell_usize(STATUS_COLUMN, status_text(row).chars().count());
        let age = age_label(now.saturating_sub(row.started));
        widths.observe_cell_usize(AGE_COLUMN, age.chars().count());
    }
    widths
        .to_constraints()
        .into_iter()
        .enumerate()
        .map(|(column, constraint)| {
            if column == DIRECTORY_COLUMN {
                Constraint::Min(cell_width(SUMMARY_HEADERS[DIRECTORY_COLUMN]))
            } else {
                constraint
            }
        })
        .collect()
}

/// `area` indented one level, where the label row and every machine's
/// rows sit, under headings at the outer level.
fn indented(area: Rect) -> Rect {
    let indent = cell_width(SECTION_ITEM_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area.width.saturating_sub(indent),
        ..area
    }
}

/// A string's width in cells, clamped into the column-width type.
fn cell_width(text: &str) -> u16 { u16::try_from(text.chars().count()).unwrap_or(u16::MAX) }

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    /// The unix second every age in these tests is measured to.
    const NOW: u64 = 1_000_000;
    /// Seconds in an hour.
    const HOUR: u64 = 3_600;
    /// Seconds in a minute.
    const MINUTE: u64 = 60;
    /// The summary's width in these tests.
    const WIDTH: u16 = 100;

    /// A row for `agent` named `name`, started `age` seconds before
    /// [`NOW`].
    fn row(agent: Agent, name: &str, status: Option<&str>, age: u64, directory: &str) -> AgentRow {
        AgentRow {
            agent,
            name: name.to_string(),
            status: status.map(str::to_string),
            started: NOW - age,
            pid: 1,
            directory: directory.to_string(),
        }
    }

    /// Each line of `buffer`, with trailing blanks trimmed.
    fn lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                let line: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                line.trim_end().to_string()
            })
            .collect()
    }

    /// Four machines: this one with a name past the column's cap, a busy
    /// session and a Codex with no status; a remote with one agent; a
    /// remote whose probe failed; and one that has not answered.
    #[test]
    fn machines_list_their_agents_under_one_label_row() {
        let natedev = MachineState::Answered(vec![
            row(
                Agent::Claude,
                "boss of bosses",
                Some("idle"),
                21 * HOUR,
                "~/rust/hana_catalyst/docs/hana",
            ),
            row(
                Agent::Claude,
                "tmp cleanup then merge to berth and handler",
                Some("shell"),
                2 * HOUR + 29 * MINUTE,
                "~/rust/cargo-handler",
            ),
            row(
                Agent::Claude,
                "enh/handler",
                Some("busy"),
                HOUR + 36 * MINUTE,
                "~/rust/handler",
            ),
            row(
                Agent::Codex,
                "--model gpt-5",
                None,
                12 * MINUTE,
                "~/rust/handler",
            ),
        ]);
        let mac = MachineState::Answered(vec![row(
            Agent::Claude,
            "natemccoy-30",
            Some("idle"),
            23 * HOUR,
            "~",
        )]);
        let studio = MachineState::Failed("unreachable".to_string());
        let pi = MachineState::Scanning;
        let machines = [
            Machine {
                name:  "natedev",
                state: &natedev,
            },
            Machine {
                name:  "mac",
                state: &mac,
            },
            Machine {
                name:  "studio",
                state: &studio,
            },
            Machine {
                name:  "pi",
                state: &pi,
            },
        ];
        let height = u16::try_from(height(&machines)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);

        draw(&mut buffer, area, &machines, NOW);

        assert_eq!(
            lines(&buffer),
            [
                " agent   name                                  status  age     directory",
                " natedev · 4 agents",
                " claude  boss of bosses                        idle    21h     ~/rust/hana_catalyst/docs/hana",
                " claude  tmp cleanup then merge to berth and…  shell   2h 29m  ~/rust/cargo-handler",
                " claude  enh/handler                           busy    1h 36m  ~/rust/handler",
                " codex   --model gpt-5                         —       12m     ~/rust/handler",
                "",
                " mac · 1 agent",
                " claude  natemccoy-30                          idle    23h     ~",
                "",
                " studio · unreachable",
                "",
                " pi · scanning",
            ]
        );
        assert_eq!(buffer[(1, 2)].fg, Color::Rgb(217, 119, 87));
        assert_eq!(buffer[(1, 5)].fg, Color::Rgb(175, 140, 255));
        assert_eq!(buffer[(47, 4)].fg, Color::Rgb(100, 220, 100));
        assert_eq!(buffer[(47, 5)].fg, Color::Rgb(140, 140, 140));
        assert_eq!(buffer[(10, 10)].fg, Color::Rgb(255, 100, 100));
    }
}
