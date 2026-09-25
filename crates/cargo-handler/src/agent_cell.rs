//! One agent's cell: a header naming the agent, then a table of what it
//! is running, each row indented under the row that started it.
//!
//! Every agent someone can talk to has a cell of its own: each
//! top-level agent, followed by the sessions it opened in tmux.
//! [`cell_order`] lays the cells out across the machines, and
//! [`height`] and [`draw`] fill one in.

use std::collections::HashSet;

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
use tui_pane::accent_color;
use tui_pane::label_color;
use tui_pane::text_default;

use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::Machine;
use crate::constants::AGENT_HEADER_GAP_HEIGHT;
use crate::constants::AGENT_HEADER_HEIGHT;
use crate::constants::CHILD_AGE_COLUMN;
use crate::constants::CHILD_HEADERS;
use crate::constants::CHILD_KIND_COLUMN;
use crate::constants::CHILD_KIND_INDENT;
use crate::constants::CHILD_NAME_COLUMN;
use crate::constants::CHILD_PID_COLUMN;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::LAUNCHED_BY_LABEL;
use crate::constants::LAUNCHER_LINE_HEIGHT;
use crate::constants::MISSING_VALUE;
use crate::constants::NOTHING_RUNNING_HEIGHT;
use crate::constants::NOTHING_RUNNING_NOTE;
use crate::constants::PID_LABEL;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::summary;
use crate::summary::age;
use crate::theme::Role;
use crate::tiles::AgentCell;

/// One agent's cell, as the grid lays it out and draws it.
#[derive(Clone, Debug)]
pub(crate) struct AgentEntry<'a> {
    /// The cell's id in the grid.
    pub(crate) id:       AgentCell,
    /// The agent the cell draws.
    pub(crate) row:      &'a AgentRow,
    /// The name of the agent that opened this one in tmux, where that
    /// agent is listed on the same machine.
    pub(crate) launcher: Option<&'a str>,
    /// The heading of the machine the agent runs on.
    pub(crate) machine:  &'a str,
}

/// Every agent's cell across `machines`, in the order the grid shows
/// them: machine by machine, and within a machine each top-level agent
/// oldest first, followed by the sessions it opened, depth first and
/// oldest first. A session whose launcher is not listed stands as a
/// top-level agent.
pub(crate) fn cell_order<'a>(machines: &[Machine<'a>]) -> Vec<AgentEntry<'a>> {
    let mut cells = Vec::new();
    for machine in machines {
        let rows = machine.state.rows();
        let listed: HashSet<u32> = rows.iter().map(|row| row.pid).collect();
        let mut placed = HashSet::new();
        let roots = rows.iter().filter(|row| {
            row.launched_by
                .is_none_or(|launcher| launcher == row.pid || !listed.contains(&launcher))
        });
        for root in roots {
            place(machine.name, rows, root, &mut placed, &mut cells);
        }
        // Rows that only name one another as launcher reach no root, so
        // each stands as one rather than going without a cell.
        for row in rows {
            place(machine.name, rows, row, &mut placed, &mut cells);
        }
    }
    cells
}

/// Put `row`'s cell into `cells`, then the cells of the sessions it
/// opened among `rows`, skipping any row already `placed`.
fn place<'a>(
    machine: &'a str,
    rows: &'a [AgentRow],
    row: &'a AgentRow,
    placed: &mut HashSet<u32>,
    cells: &mut Vec<AgentEntry<'a>>,
) {
    if !placed.insert(row.pid) {
        return;
    }
    let launcher = row.launched_by.and_then(|launcher| {
        rows.iter()
            .find(|other| other.pid == launcher)
            .map(|other| other.name.as_str())
    });
    cells.push(AgentEntry {
        id: AgentCell {
            machine: machine.to_string(),
            pid:     row.pid,
            started: row.started,
        },
        row,
        launcher,
        machine,
    });
    for session in rows
        .iter()
        .filter(|session| session.launched_by == Some(row.pid))
    {
        place(machine, rows, session, placed, cells);
    }
}

/// Rows `row`'s cell draws: its header, a blank row, then its table's
/// label row and rows, or the note saying it runs nothing.
pub(crate) fn height(row: &AgentRow) -> usize {
    let launcher = if row.launched_by.is_some() {
        LAUNCHER_LINE_HEIGHT
    } else {
        0
    };
    let table = if row.children.is_empty() {
        usize::from(NOTHING_RUNNING_HEIGHT)
    } else {
        usize::from(TABLE_HEADER_HEIGHT) + row.children.len()
    };
    usize::from(AGENT_HEADER_HEIGHT + launcher + AGENT_HEADER_GAP_HEIGHT) + table
}

/// Draw `row`'s cell into `area`: the header, naming `launcher_name` as
/// the agent that opened it when it was opened by one, and `machine` as
/// where it runs, then its table, with ages measured to `now` in unix
/// seconds.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    row: &AgentRow,
    launcher_name: Option<&str>,
    machine: &str,
    now: u64,
) {
    let label = Style::default().fg(label_color());
    let mut header = vec![
        summary_line(row, machine, now),
        Line::from(vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(row.directory.clone(), label),
        ]),
    ];
    if let Some(launcher) = row.launched_by {
        let name = launcher_name.map_or_else(|| format!("{PID_LABEL} {launcher}"), str::to_string);
        header.push(Line::from(vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(format!("{LAUNCHED_BY_LABEL} "), label),
            Span::styled(name, Style::default().fg(text_default())),
        ]));
    }
    let header_height = u16::try_from(header.len()).unwrap_or(u16::MAX);
    Paragraph::new(header).render(
        Rect {
            height: header_height.min(area.height),
            ..area
        },
        buffer,
    );

    let above = header_height.saturating_add(AGENT_HEADER_GAP_HEIGHT);
    let table = Rect {
        y: area.y.saturating_add(above),
        height: area.height.saturating_sub(above),
        ..summary::indented(area)
    };
    if table.is_empty() {
        return;
    }
    if row.children.is_empty() {
        Paragraph::new(Line::from(Span::styled(NOTHING_RUNNING_NOTE, label))).render(table, buffer);
        return;
    }
    let (constraints, name_width) = fitted_columns(&row.children, table.width, now);
    Table::new(
        row.children
            .iter()
            .map(|child| child_row(child, name_width, now)),
        constraints,
    )
    .header(Row::new(
        CHILD_HEADERS.map(|header| Span::styled(header, label)),
    ))
    .column_spacing(TABLE_COLUMN_SPACING)
    .render(table, buffer);
}

/// The header's first line: `pid <pid> · <agent> · <status> · <age> ·
/// <machine> · <desktop>`, colored as the summary colors the same
/// values.
fn summary_line(row: &AgentRow, machine: &str, now: u64) -> Line<'static> {
    let label = Style::default().fg(label_color());
    let separator = || Span::styled(HEADING_SEPARATOR, label);
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(format!("{PID_LABEL} "), label),
        Span::styled(row.pid.to_string(), Style::default().fg(text_default())),
        separator(),
        Span::styled(row.agent.label(), summary::agent_role(row.agent).style()),
        separator(),
        Span::styled(
            summary::status_text(row).to_string(),
            summary::status_role(row).style(),
        ),
        separator(),
        Span::styled(age::age_label(now.saturating_sub(row.started)), label),
        separator(),
        Span::styled(machine.to_string(), Style::default().fg(accent_color())),
        separator(),
        Span::styled(summary::desktop_text(row).to_string(), label),
    ])
}

/// One row of the table, its name cut to `name_width` cells.
fn child_row(child: &ChildRow, name_width: usize, now: u64) -> Row<'static> {
    let label = Style::default().fg(label_color());
    let text = Style::default().fg(text_default());
    let pid = child.pid.map_or_else(
        || Span::styled(MISSING_VALUE, label),
        |pid| Span::styled(pid.to_string(), text),
    );
    Row::new([
        pid,
        Span::styled(kind_text(child), kind_role(child.kind).style()),
        Span::styled(summary::truncated(&child.name, name_width), text),
        Span::styled(age::age_label(now.saturating_sub(child.started)), label),
    ])
}

/// The `kind` cell: the kind's label, indented [`CHILD_KIND_INDENT`]
/// cells for each level the row sits below the agent.
fn kind_text(child: &ChildRow) -> String {
    let indent = usize::from(child.depth) * CHILD_KIND_INDENT;
    format!("{:indent$}{}", "", child.kind.label())
}

/// The role a row's `kind` is drawn in: a shell as a shell status, a
/// subagent as Claude Code, a thread as Codex, and a session or process
/// as its program.
const fn kind_role(kind: ChildKind) -> Role {
    match kind {
        ChildKind::Shell => Role::Shell,
        ChildKind::Subagent => Role::Claude,
        ChildKind::Session(agent) | ChildKind::Process(agent) => summary::agent_role(agent),
        ChildKind::Thread => Role::Codex,
    }
}

/// The table's column widths within `width` cells, and the width the
/// `name` column's text is cut to.
///
/// `pid`, `kind` and `age` fit their widest cell. `name` fits its widest
/// too, but no wider than what the other three and the spacing leave, so
/// in a narrow cell the names are cut rather than the ages pushed out.
fn fitted_columns(children: &[ChildRow], width: u16, now: u64) -> ([Constraint; 4], usize) {
    let mut widths = ColumnWidths::new(
        CHILD_HEADERS
            .iter()
            .map(|header| ColumnSpec {
                min: summary::cell_width(header),
                max: None,
            })
            .collect(),
    );
    for child in children {
        let pid = child.pid.map_or_else(
            || MISSING_VALUE.chars().count(),
            |pid| pid.to_string().chars().count(),
        );
        widths.observe_cell_usize(CHILD_PID_COLUMN, pid);
        widths.observe_cell_usize(CHILD_KIND_COLUMN, kind_text(child).chars().count());
        widths.observe_cell_usize(CHILD_NAME_COLUMN, child.name.chars().count());
        let age = age::age_label(now.saturating_sub(child.started));
        widths.observe_cell_usize(CHILD_AGE_COLUMN, age.chars().count());
    }
    let spacing = TABLE_COLUMN_SPACING.saturating_mul(3);
    let fitted = widths
        .get(CHILD_PID_COLUMN)
        .saturating_add(widths.get(CHILD_KIND_COLUMN))
        .saturating_add(widths.get(CHILD_AGE_COLUMN))
        .saturating_add(spacing);
    let name = widths
        .get(CHILD_NAME_COLUMN)
        .min(width.saturating_sub(fitted));
    let constraints = [
        Constraint::Length(widths.get(CHILD_PID_COLUMN)),
        Constraint::Length(widths.get(CHILD_KIND_COLUMN)),
        Constraint::Length(name),
        Constraint::Length(widths.get(CHILD_AGE_COLUMN)),
    ];
    (constraints, usize::from(name))
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::census::Agent;
    use crate::census::MachineState;

    /// The unix second every age in these tests is measured to.
    const NOW: u64 = 1_790_372_800;
    /// Seconds in a minute.
    const MINUTE: u64 = 60;
    /// Seconds in an hour.
    const HOUR: u64 = 60 * MINUTE;
    /// boss of bosses, which opened the sessions below.
    const BOSS: u32 = 1_579_022;
    /// The cell's width in the drawing tests.
    const WIDTH: u16 = 72;

    /// A Claude Code row with process `pid`, named `name`, started `age`
    /// seconds before [`NOW`] and opened by `launched_by`.
    fn agent(pid: u32, name: &str, age: u64, launched_by: Option<u32>) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            name: name.to_string(),
            status: Some("busy".to_string()),
            started: NOW - age,
            pid,
            desktop: None,
            directory: format!("~/rust/{name}"),
            launched_by,
            children: Vec::new(),
        }
    }

    /// A row of a cell's table, `depth` levels down, started `age`
    /// seconds before [`NOW`].
    fn child(depth: u8, kind: ChildKind, pid: Option<u32>, name: &str, age: u64) -> ChildRow {
        ChildRow {
            depth,
            kind,
            pid,
            name: name.to_string(),
            started: NOW - age,
        }
    }

    /// `row`'s cell drawn at [`WIDTH`] and exactly its own height.
    fn drawn(row: &AgentRow, launcher: Option<&str>) -> Buffer {
        let height = u16::try_from(height(row)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);
        draw(&mut buffer, area, row, launcher, "natedev", NOW);
        buffer
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

    /// A session boss opened: the header names the launcher, and the
    /// table indents each row's kind under the row that started it,
    /// shows `—` for a row with no process, and cuts a name too long for
    /// what the other columns leave.
    #[test]
    fn a_launched_session_draws_its_header_and_tree() {
        let trunk = AgentRow {
            desktop: Some("berth_fix".to_string()),
            children: vec![
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_371_669),
                    "Launch the Phase 1 implementation seat",
                    12 * MINUTE,
                ),
                child(
                    1,
                    ChildKind::Process(Agent::Codex),
                    Some(2_372_720),
                    "app-server",
                    12 * MINUTE,
                ),
                child(
                    2,
                    ChildKind::Thread,
                    None,
                    "tool-based-ui-trunk-impl",
                    11 * MINUTE,
                ),
                child(
                    0,
                    ChildKind::Subagent,
                    None,
                    "Review the permission queue",
                    5 * MINUTE + 3,
                ),
                child(
                    1,
                    ChildKind::Shell,
                    Some(2_424_763),
                    "cargo nextest run -p hana_video --no-fail-fast -- permission_queue",
                    45,
                ),
                child(
                    0,
                    ChildKind::Session(Agent::Claude),
                    Some(3_337_048),
                    "tool-based-ui-arrange",
                    30,
                ),
            ],
            ..agent(3_266_367, "tool-based-ui-trunk", 23 * HOUR, Some(BOSS))
        };

        let buffer = drawn(&trunk, Some("boss of bosses"));

        assert_eq!(
            lines(&buffer),
            [
                " pid 3266367 · claude · busy · 23h · natedev · berth_fix",
                " ~/rust/tool-based-ui-trunk",
                " launched by boss of bosses",
                "",
                " pid      kind        name                                         age",
                " 2371669  shell       Launch the Phase 1 implementation seat       12m",
                " 2372720    codex     app-server                                   12m",
                " —            thread  tool-based-ui-trunk-impl                     11m",
                " —        subagent    Review the permission queue                  5m 3s",
                " 2424763    shell     cargo nextest run -p hana_video --no-fail-…  45s",
                " 3337048  session     tool-based-ui-arrange                        30s",
            ]
        );
        let role = |x, y| Some(buffer[(x, y)].fg);
        assert_eq!(buffer[(5, 0)].fg, text_default());
        assert_eq!(role(15, 0), Role::Claude.style().fg);
        assert_eq!(role(24, 0), Role::Busy.style().fg);
        assert_eq!(buffer[(37, 0)].fg, accent_color());
        assert_eq!(role(10, 5), Role::Shell.style().fg);
        assert_eq!(role(12, 6), Role::Codex.style().fg);
        assert_eq!(buffer[(1, 7)].fg, label_color());
        assert_eq!(role(14, 7), Role::Codex.style().fg);
        assert_eq!(role(10, 8), Role::Claude.style().fg);
    }

    /// An agent running nothing says so in place of its table, and a
    /// launcher that is not listed is named by its pid.
    #[test]
    fn an_agent_running_nothing_says_so() {
        let arrange = agent(3_337_048, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));

        assert_eq!(
            lines(&drawn(&arrange, None)),
            [
                " pid 3337048 · claude · busy · 2h · natedev · —",
                " ~/rust/tool-based-ui-arrange",
                " launched by pid 1579022",
                "",
                " nothing running",
            ]
        );
    }

    /// Machine by machine, each top-level agent comes oldest first,
    /// followed by the sessions it opened, depth first. A session whose
    /// launcher is not listed stands on its own, and two rows naming
    /// only each other still get a cell each.
    #[test]
    fn cells_follow_each_agent_with_the_sessions_it_opened() {
        let natedev = MachineState::Answered(vec![
            agent(BOSS, "boss of bosses", 23 * HOUR, None),
            agent(428_044, "enh/handler", 22 * HOUR, None),
            agent(3_266_367, "trunk", 21 * HOUR, Some(BOSS)),
            agent(3_337_048, "arrange", 3 * HOUR, Some(BOSS)),
            agent(3_400_000, "under trunk", 2 * HOUR, Some(3_266_367)),
            agent(3_500_000, "orphan", HOUR, Some(999)),
            agent(3_600_000, "left", 30 * MINUTE, Some(3_700_000)),
            agent(3_700_000, "right", 20 * MINUTE, Some(3_600_000)),
        ]);
        let mac = MachineState::Answered(vec![agent(12_055, "natemccoy-30", HOUR, None)]);
        let machines = [
            Machine {
                name:  "natedev",
                state: &natedev,
            },
            Machine {
                name:  "mac",
                state: &mac,
            },
        ];

        let cells = cell_order(&machines);

        let order: Vec<(&str, &str, Option<&str>)> = cells
            .iter()
            .map(|cell| {
                (
                    cell.id.machine.as_str(),
                    cell.row.name.as_str(),
                    cell.launcher,
                )
            })
            .collect();
        assert_eq!(
            order,
            [
                ("natedev", "boss of bosses", None),
                ("natedev", "trunk", Some("boss of bosses")),
                ("natedev", "under trunk", Some("trunk")),
                ("natedev", "arrange", Some("boss of bosses")),
                ("natedev", "enh/handler", None),
                ("natedev", "orphan", None),
                ("natedev", "left", Some("right")),
                ("natedev", "right", Some("left")),
                ("mac", "natemccoy-30", None),
            ]
        );
        assert_eq!(
            cells[1].id,
            AgentCell {
                machine: "natedev".to_string(),
                pid:     3_266_367,
                started: NOW - 21 * HOUR,
            }
        );
    }
}
