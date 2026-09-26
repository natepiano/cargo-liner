//! The summary cell: each machine's heading, then, when it lists
//! agents, a column-label row and its top-level agents, oldest first,
//! with a blank row between machines. Every machine's table is laid out
//! with the same column widths, so the columns line up down the cell.
//! Each agent's name is drawn in the hue of its cell, so a row and that
//! agent's cell can be paired at a glance.

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
use crate::agent_cell::AgentEntry;
use crate::census::Agent;
use crate::census::AgentRow;
use crate::census::Machine;
use crate::census::MachineState;
use crate::constants::AGE_COLUMN;
use crate::constants::AGENT_COLUMN;
use crate::constants::AGENT_PLURAL;
use crate::constants::AGENT_SINGULAR;
use crate::constants::BUSY_STATUS;
use crate::constants::DESKTOP_COLUMN;
use crate::constants::DIRECTORY_COLUMN;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::MISSING_VALUE;
use crate::constants::NAME_COLUMN;
use crate::constants::NAME_COLUMN_MAX;
use crate::constants::NO_AGENTS_NOTE;
use crate::constants::PID_COLUMN;
use crate::constants::SCANNING_NOTE;
use crate::constants::SHELL_STATUS;
use crate::constants::STATUS_COLUMN;
use crate::constants::SUMMARY_HEADERS;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TRUNCATION_MARK;
use crate::theme::Role;

/// Rows the summary draws for `machines`: each machine's heading and
/// table, and a gap between one machine and the next.
pub(crate) fn height(machines: &[Machine<'_>]) -> usize {
    let groups: usize = machines
        .iter()
        .map(|machine| {
            usize::from(GROUP_HEADER_HEIGHT) + table_height(machine.state.top_level().count())
        })
        .sum();
    let gaps = machines.len().saturating_sub(1) * usize::from(GROUP_GAP_HEIGHT);
    groups + gaps
}

/// Columns the summary's widest line takes for `machines`: a machine's
/// heading, or the table with every column at its fitted width and every
/// directory written out in full.
///
/// This is what the summary asks the grid for across, so a directory is
/// only cut short once the summary has reached the right edge.
pub(crate) fn width(machines: &[Machine<'_>], now: u64) -> u16 {
    let headings = machines
        .iter()
        .map(|machine| heading(machine).width())
        .max()
        .map_or(0, |widest| u16::try_from(widest).unwrap_or(u16::MAX));
    if machines
        .iter()
        .all(|machine| machine.state.top_level().next().is_none())
    {
        return headings;
    }
    let widths = column_widths(machines, now);
    let columns = u16::try_from(SUMMARY_HEADERS.len()).unwrap_or(u16::MAX);
    let table = (0..SUMMARY_HEADERS.len())
        .map(|column| widths.get(column))
        .fold(cell_width(SECTION_ITEM_INDENT), u16::saturating_add)
        .saturating_add(TABLE_COLUMN_SPACING.saturating_mul(columns.saturating_sub(1)));
    headings.max(table)
}

/// Rows a machine's table takes for `rows` agents: none when it lists
/// none, else its label row and the agents.
fn table_height(rows: usize) -> usize {
    if rows == 0 {
        0
    } else {
        usize::from(TABLE_HEADER_HEIGHT) + rows
    }
}

/// Draw `machines` into `area`, with each agent's name in the hue of its
/// cell among `cells` and ages measured to `now` in unix seconds.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    machines: &[Machine<'_>],
    cells: &[AgentEntry<'_>],
    now: u64,
) {
    // Every machine's table is laid out with the same constraints and
    // indent, so its columns line up with every other machine's.
    let constraints = fitted_constraints(machines, now);
    let mut remaining = area;
    for machine in machines {
        if remaining.height == 0 {
            break;
        }
        let used = draw_machine(buffer, remaining, machine, cells, &constraints, now);
        remaining.y = remaining.y.saturating_add(used);
        remaining.height = remaining.height.saturating_sub(used);
    }
}

/// Draw one machine's heading, and its label row and rows when it lists
/// agents, into the top of `area`, answering how many rows that took
/// including the gap below it. Each name takes its hue from `cells`.
fn draw_machine(
    buffer: &mut Buffer,
    area: Rect,
    machine: &Machine<'_>,
    cells: &[AgentEntry<'_>],
    constraints: &[Constraint],
    now: u64,
) -> u16 {
    let label = Style::default().fg(label_color());
    Paragraph::new(heading(machine)).render(
        Rect {
            height: GROUP_HEADER_HEIGHT.min(area.height),
            ..area
        },
        buffer,
    );

    let rows: Vec<&AgentRow> = machine.state.top_level().collect();
    let drawn = area
        .height
        .saturating_sub(GROUP_HEADER_HEIGHT)
        .min(u16::try_from(table_height(rows.len())).unwrap_or(u16::MAX));
    Table::new(
        rows.iter()
            .map(|row| agent_row(row, name_style(cells, machine.name, row), now)),
        constraints.iter().copied(),
    )
    .header(Row::new(
        SUMMARY_HEADERS.map(|header| Span::styled(header, label)),
    ))
    .column_spacing(TABLE_COLUMN_SPACING)
    .render(
        Rect {
            y: area.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: drawn,
            ..indented(area)
        },
        buffer,
    );
    GROUP_HEADER_HEIGHT
        .saturating_add(drawn)
        .saturating_add(GROUP_GAP_HEIGHT)
}

/// `machine`'s heading: its name, then how many agents it lists, that
/// it is still being scanned, or why it could not be.
fn heading<'a>(machine: &Machine<'a>) -> Line<'a> {
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
        MachineState::Answered(_) => {
            heading.push(Span::styled(
                format!(
                    "{HEADING_SEPARATOR}{}",
                    count_note(machine.state.top_level().count())
                ),
                label,
            ));
        },
        MachineState::Failed(reason) => {
            heading.push(Span::styled(HEADING_SEPARATOR, label));
            heading.push(Span::styled(reason.as_str(), Role::Unreachable.style()));
        },
    }
    Line::from(heading)
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

/// The style `row`'s name is drawn in on `machine`: the hue of its cell
/// among `cells`, or the default text color for a row with no cell.
fn name_style(cells: &[AgentEntry<'_>], machine: &str, row: &AgentRow) -> Style {
    cells
        .iter()
        .find(|cell| cell.machine == machine && cell.row.pid == row.pid)
        .map_or_else(
            || Style::default().fg(text_default()),
            |cell| Role::Rainbow(cell.hue).style(),
        )
}

/// One agent's table row, its name drawn in `name_style`.
fn agent_row(row: &AgentRow, name_style: Style, now: u64) -> Row<'static> {
    let text = Style::default().fg(text_default());
    Row::new([
        Span::styled(row.pid.to_string(), text),
        Span::styled(row.agent.label(), agent_role(row.agent).style()),
        Span::styled(
            truncated(&row.name, usize::from(NAME_COLUMN_MAX)),
            name_style,
        ),
        Span::styled(status_text(row).to_string(), status_role(row).style()),
        Span::styled(age_label(now.saturating_sub(row.started)), text),
        Span::styled(desktop_text(row).to_string(), text),
        Span::styled(row.directory.clone(), text),
    ])
}

/// The role an agent's program is drawn in.
pub(crate) const fn agent_role(agent: Agent) -> Role {
    match agent {
        Agent::Claude => Role::Claude,
        Agent::Codex => Role::Codex,
    }
}

/// The role an agent's status is drawn in.
pub(crate) fn status_role(row: &AgentRow) -> Role {
    match row.status.as_deref() {
        Some(BUSY_STATUS) => Role::Busy,
        Some(SHELL_STATUS) => Role::Shell,
        _ => Role::Idle,
    }
}

/// The status cell: the status, or [`MISSING_VALUE`].
pub(crate) fn status_text(row: &AgentRow) -> &str { row.status.as_deref().unwrap_or(MISSING_VALUE) }

/// The desktop cell: the desktop, or [`MISSING_VALUE`].
pub(crate) fn desktop_text(row: &AgentRow) -> &str {
    row.desktop.as_deref().unwrap_or(MISSING_VALUE)
}

/// `name` cut to `max` cells, ending in [`TRUNCATION_MARK`] when it was
/// longer.
pub(crate) fn truncated(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    name.chars()
        .take(max.saturating_sub(1))
        .chain(std::iter::once(TRUNCATION_MARK).take(max.min(1)))
        .collect()
}

/// Column widths fitted to the widest cell across every machine's rows.
/// `name` stops at [`NAME_COLUMN_MAX`], and `directory` takes whatever
/// the fitted columns leave.
fn fitted_constraints(machines: &[Machine<'_>], now: u64) -> Vec<Constraint> {
    column_widths(machines, now)
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

/// The widest cell in each column across every machine's rows, `name`
/// stopping at [`NAME_COLUMN_MAX`] and `directory` written out in full.
fn column_widths(machines: &[Machine<'_>], now: u64) -> ColumnWidths {
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
    for row in machines
        .iter()
        .flat_map(|machine| machine.state.top_level())
    {
        widths.observe_cell_usize(PID_COLUMN, row.pid.to_string().chars().count());
        widths.observe_cell_usize(AGENT_COLUMN, row.agent.label().chars().count());
        widths.observe_cell_usize(NAME_COLUMN, row.name.chars().count());
        widths.observe_cell_usize(STATUS_COLUMN, status_text(row).chars().count());
        let age = age_label(now.saturating_sub(row.started));
        widths.observe_cell_usize(AGE_COLUMN, age.chars().count());
        widths.observe_cell_usize(DESKTOP_COLUMN, desktop_text(row).chars().count());
        widths.observe_cell_usize(DIRECTORY_COLUMN, row.directory.chars().count());
    }
    widths
}

/// `area` indented one level, where every machine's label row and rows
/// sit, under headings at the outer level.
pub(crate) fn indented(area: Rect) -> Rect {
    let indent = cell_width(SECTION_ITEM_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area.width.saturating_sub(indent),
        ..area
    }
}

/// A string's width in cells, clamped into the column-width type.
pub(crate) fn cell_width(text: &str) -> u16 {
    u16::try_from(text.chars().count()).unwrap_or(u16::MAX)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::agent_cell;

    /// The unix second every age in these tests is measured to.
    const NOW: u64 = 1_000_000;
    /// Seconds in an hour.
    const HOUR: u64 = 3_600;
    /// Seconds in a minute.
    const MINUTE: u64 = 60;
    /// The summary's width in these tests: wide enough that no
    /// directory is cut.
    const WIDTH: u16 = 120;
    /// The column each name starts at in these tests.
    const NAME_X: u16 = 18;

    /// Process `pid`, an `agent` named `name`, started `age` seconds
    /// before [`NOW`].
    fn row(
        pid: u32,
        agent: Agent,
        name: &str,
        status: Option<&str>,
        age: u64,
        directory: &str,
    ) -> AgentRow {
        AgentRow {
            agent,
            name: name.to_string(),
            status: status.map(str::to_string),
            started: NOW - age,
            pid,
            desktop: None,
            directory: directory.to_string(),
            launched_by: None,
            children: Vec::new(),
        }
    }

    /// `row` with its window on the desktop named `desktop`.
    fn on(desktop: &str, row: AgentRow) -> AgentRow {
        AgentRow {
            desktop: Some(desktop.to_string()),
            ..row
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

    /// natedev's answer: three agents on desktops of their own, one
    /// session boss launched, and a Codex agent with no window.
    fn natedev() -> MachineState {
        let launched = AgentRow {
            launched_by: Some(1_579_022),
            ..row(3_266_367, Agent::Claude, "trunk", None, HOUR, "~")
        };
        MachineState::Answered(vec![
            on(
                "boss",
                row(
                    1_579_022,
                    Agent::Claude,
                    "boss of bosses",
                    Some("idle"),
                    21 * HOUR,
                    "~/rust/hana_catalyst/docs/hana",
                ),
            ),
            launched,
            on(
                "berth_fix",
                row(
                    2_747_564,
                    Agent::Claude,
                    "tmp cleanup then merge to berth and handler",
                    Some("shell"),
                    2 * HOUR + 29 * MINUTE,
                    "~/rust/cargo-handler",
                ),
            ),
            on(
                "cargo handler",
                row(
                    428_044,
                    Agent::Claude,
                    "enh/handler",
                    Some("busy"),
                    HOUR + 36 * MINUTE,
                    "~/rust/handler",
                ),
            ),
            row(
                4_039_085,
                Agent::Codex,
                "--model gpt-5",
                None,
                12 * MINUTE,
                "~/rust/handler",
            ),
        ])
    }

    /// Four machines: this one with a name past the column's cap, a busy
    /// session, a Codex with no status and a session another agent
    /// launched, which is left out of the list and the count; a remote
    /// with one agent; a remote whose probe failed; and one that has not
    /// answered. Each machine that lists agents repeats the label row
    /// under its heading, with the columns sized across both machines'
    /// rows. Each name takes the hue of its cell: the launched session
    /// has a cell but no row, so the hue after boss's goes to it.
    #[test]
    fn each_machine_lists_its_agents_under_its_own_label_row() {
        let natedev = natedev();
        let mac = MachineState::Answered(vec![row(
            12_055,
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
        let cells = agent_cell::cell_order(&machines);

        draw(&mut buffer, area, &machines, &cells, NOW);

        assert_eq!(
            lines(&buffer),
            [
                " natedev · 4 agents",
                " pid      agent   name                                  status  age     desktop        directory",
                " 1579022  claude  boss of bosses                        idle    21h     boss           ~/rust/hana_catalyst/docs/hana",
                " 2747564  claude  tmp cleanup then merge to berth and…  shell   2h 29m  berth_fix      ~/rust/cargo-handler",
                " 428044   claude  enh/handler                           busy    1h 36m  cargo handler  ~/rust/handler",
                " 4039085  codex   --model gpt-5                         —       12m     —              ~/rust/handler",
                "",
                " mac · 1 agent",
                " pid      agent   name                                  status  age     desktop        directory",
                " 12055    claude  natemccoy-30                          idle    23h     —              ~",
                "",
                " studio · unreachable",
                "",
                " pi · scanning",
            ]
        );
        assert_eq!(buffer[(1, 2)].fg, text_default());
        let header = buffer[(1, 1)].fg;
        for y in [2, 3, 4, 5, 9] {
            for x in 0..buffer.area.width {
                assert_ne!(
                    buffer[(x, y)].fg,
                    header,
                    "row {y} draws column {x} in the header color"
                );
            }
        }
        assert_eq!(buffer[(10, 2)].fg, Color::Rgb(217, 119, 87));
        assert_eq!(buffer[(10, 5)].fg, Color::Rgb(175, 140, 255));
        assert_eq!(buffer[(56, 4)].fg, Color::Rgb(100, 220, 100));
        assert_eq!(buffer[(56, 5)].fg, Color::Rgb(140, 140, 140));
        assert_eq!(buffer[(10, 11)].fg, Color::Rgb(255, 100, 100));
        let names: Vec<Color> = [2, 3, 4, 5, 9]
            .into_iter()
            .map(|y| buffer[(NAME_X, y)].fg)
            .collect();
        assert_eq!(
            names,
            [
                Color::Rgb(255, 95, 95),
                Color::Rgb(240, 220, 80),
                Color::Rgb(110, 220, 110),
                Color::Rgb(80, 210, 230),
                Color::Rgb(100, 150, 255),
            ],
            "red, then yellow past trunk's orange, green, cyan, and the mac's blue"
        );
    }

    /// The summary asks for its widest line: the table with every
    /// directory written out in full, so a longer directory asks for
    /// that much more room, or the widest heading when no machine lists
    /// an agent.
    #[test]
    fn the_width_holds_the_longest_directory_or_the_widest_heading() {
        const LONG_DIRECTORY: &str =
            "~/rust/a/directory/far/longer/than/any/other/the/summary/lists";
        let natedev = natedev();
        let machines = [Machine {
            name:  "natedev",
            state: &natedev,
        }];
        let fitted = width(&machines, NOW);
        let height = u16::try_from(height(&machines)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);
        draw(
            &mut buffer,
            area,
            &machines,
            &agent_cell::cell_order(&machines),
            NOW,
        );
        let widest = lines(&buffer)
            .iter()
            .map(|line| line.chars().count())
            .max()
            .unwrap_or(0);
        assert_eq!(usize::from(fitted), widest, "the widest row drawn uncut");

        let mut rows = natedev.rows().to_vec();
        let longest_before = rows
            .iter()
            .map(|row| row.directory.chars().count())
            .max()
            .unwrap_or(0);
        rows.push(row(
            7_000_001,
            Agent::Claude,
            "far away",
            Some("idle"),
            MINUTE,
            LONG_DIRECTORY,
        ));
        let longer = MachineState::Answered(rows);
        let machines = [Machine {
            name:  "natedev",
            state: &longer,
        }];
        assert_eq!(
            usize::from(width(&machines, NOW)),
            usize::from(fitted) + LONG_DIRECTORY.chars().count() - longest_before,
            "the table widens by what the long directory adds"
        );

        let quiet = MachineState::Answered(Vec::new());
        let studio = MachineState::Failed("unreachable".to_string());
        let machines = [
            Machine {
                name:  "natedev",
                state: &quiet,
            },
            Machine {
                name:  "studio",
                state: &studio,
            },
        ];
        assert_eq!(
            usize::from(width(&machines, NOW)),
            " studio · unreachable".chars().count(),
            "the wider of the two headings"
        );
    }
}
