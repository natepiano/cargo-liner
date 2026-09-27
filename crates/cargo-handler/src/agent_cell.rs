//! One agent's cell: a header naming the agent, its directory, the
//! agent that opened it, then what it is running, each child indented
//! under the one that started it.
//!
//! Every agent someone can talk to has a cell of its own: each
//! top-level agent, followed by the sessions it opened in tmux.
//! [`cell_order`] lays the cells out across the machines, and
//! [`height`] and [`draw`] fill one in.
//!
//! A cell is laid out for its width. Where the header's one line would
//! be cut, it stands as a labelled block, one fact to a line; the
//! directory breaks onto further lines; and where the table would cut a
//! child's name, each child stands as an entry of its own with its name
//! in full below it. A name with a cell of its own is drawn in that
//! cell's hue.

use std::collections::HashSet;
use std::iter;

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
use tui_pane::label_color;
use tui_pane::text_default;

use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::Machine;
use crate::constants::AGENT_HEADER_GAP_HEIGHT;
use crate::constants::CHILD_AGE_COLUMN;
use crate::constants::CHILD_HEADERS;
use crate::constants::CHILD_NAME_COLUMN;
use crate::constants::CHILD_PID_COLUMN;
use crate::constants::CHILD_RUNS_COLUMN;
use crate::constants::CHILD_VIA_COLUMN;
use crate::constants::CHILD_VIA_INDENT;
use crate::constants::HEADER_AGE_LABEL;
use crate::constants::HEADER_AGENT_LABEL;
use crate::constants::HEADER_DESKTOP_LABEL;
use crate::constants::HEADER_MACHINE_LABEL;
use crate::constants::HEADER_STATUS_LABEL;
use crate::constants::LAUNCHED_BY_LABEL;
use crate::constants::LAUNCHER_LINE_HEIGHT;
use crate::constants::MISSING_VALUE;
use crate::constants::NOTHING_RUNNING_HEIGHT;
use crate::constants::NOTHING_RUNNING_NOTE;
use crate::constants::PID_LABEL;
use crate::constants::STACKED_CHILD_HEAD_HEIGHT;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::summary;
use crate::summary::age;
use crate::theme::RainbowHue;
use crate::theme::Role;
use crate::tiles::AgentCell;
use crate::wrap;

/// One agent's cell, as the grid lays it out and draws it.
#[derive(Clone, Debug)]
pub(crate) struct AgentEntry<'a> {
    /// The cell's id in the grid.
    pub(crate) id:  AgentCell,
    /// The agent the cell draws.
    pub(crate) row: &'a AgentRow,
    /// The name of the agent that opened this one in tmux, where that
    /// agent is listed on the same machine.
    launcher:       Option<&'a str>,
    /// The heading of the machine the agent runs on.
    machine:        &'a str,
    /// The hue the cell's title and the agent's name in the summary are
    /// drawn in: the next of the rainbow, in cell order.
    pub(crate) hue: RainbowHue,
}

/// Every agent's cell across `machines`, in the order the grid shows
/// them: machine by machine, and within a machine each top-level agent
/// oldest first, followed by the sessions it opened, depth first and
/// oldest first. A session whose launcher is not listed stands as a
/// top-level agent. Each cell takes the next hue of the rainbow.
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
        hue: RainbowHue::of_cell(cells.len()),
    });
    for session in rows
        .iter()
        .filter(|session| session.launched_by == Some(row.pid))
    {
        place(machine, rows, session, placed, cells);
    }
}

/// Rows `entry`'s cell draws at `width` cells across, with ages
/// measured to `now`: exactly the rows [`draw`] fills at that width.
/// Its header, its directory and a stacked child's name each take as
/// many rows as the width leaves them.
pub(crate) fn height(entry: &AgentEntry<'_>, width: u16, now: u64) -> usize {
    let row = entry.row;
    let launcher = if row.launched_by.is_some() {
        usize::from(LAUNCHER_LINE_HEIGHT)
    } else {
        0
    };
    let children_width = children_width(width);
    let children = match Children::fitted(&row.children, children_width, now) {
        Children::Nothing => usize::from(NOTHING_RUNNING_HEIGHT),
        Children::Table(_) => usize::from(TABLE_HEADER_HEIGHT) + row.children.len(),
        Children::Stacked => row
            .children
            .iter()
            .map(|child| {
                usize::from(STACKED_CHILD_HEAD_HEIGHT) + name_lines(child, children_width).len()
            })
            .sum(),
    };
    header(row, entry.machine, width, now).len()
        + directory(row, width).len()
        + launcher
        + usize::from(AGENT_HEADER_GAP_HEIGHT)
        + children
}

/// The style the name of the agent with process `pid` on `machine` is
/// drawn in wherever it shows: the hue of that agent's cell among
/// `cells`, so the name pairs with the cell's title, or the default
/// text color for a process with no cell.
pub(crate) fn name_style(cells: &[AgentEntry<'_>], machine: &str, pid: u32) -> Style {
    cells
        .iter()
        .find(|cell| cell.machine == machine && cell.row.pid == pid)
        .map_or_else(
            || Style::default().fg(text_default()),
            |cell| Role::Rainbow(cell.hue).style(),
        )
}

/// Draw `entry`'s cell into `area`: its header, its directory, the agent
/// that opened it when one did, then what it runs, with ages measured
/// to `now` in unix seconds. A name with a cell of its own among `cells`
/// -- the launcher's, a session's -- is drawn in that cell's hue.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    now: u64,
) {
    let row = entry.row;
    let label = Style::default().fg(label_color());
    let mut above = header(row, entry.machine, area.width, now);
    above.extend(directory(row, area.width));
    if let Some(launcher) = row.launched_by {
        let name = entry
            .launcher
            .map_or_else(|| format!("{PID_LABEL} {launcher}"), str::to_string);
        above.push(Line::from(vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(format!("{LAUNCHED_BY_LABEL} "), label),
            Span::styled(name, name_style(cells, entry.machine, launcher)),
        ]));
    }
    let above_height = u16::try_from(above.len()).unwrap_or(u16::MAX);
    Paragraph::new(above).render(
        Rect {
            height: above_height.min(area.height),
            ..area
        },
        buffer,
    );

    let skipped = above_height.saturating_add(AGENT_HEADER_GAP_HEIGHT);
    let children = Rect {
        y: area.y.saturating_add(skipped),
        height: area.height.saturating_sub(skipped),
        ..summary::indented(area)
    };
    if children.is_empty() {
        return;
    }
    let child_name_style = |child: &ChildRow| {
        child.pid.map_or_else(
            || Style::default().fg(text_default()),
            |pid| name_style(cells, entry.machine, pid),
        )
    };
    match Children::fitted(&row.children, children.width, now) {
        Children::Nothing => {
            Paragraph::new(Line::from(Span::styled(
                NOTHING_RUNNING_NOTE,
                Style::default().fg(text_default()),
            )))
            .render(children, buffer);
        },
        Children::Table(constraints) => Table::new(
            row.children
                .iter()
                .map(|child| child_row(child, child_name_style(child), now)),
            constraints,
        )
        .header(Row::new(
            CHILD_HEADERS.map(|header| Span::styled(header, label)),
        ))
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(children, buffer),
        Children::Stacked => {
            let lines: Vec<Line<'static>> = row
                .children
                .iter()
                .flat_map(|child| {
                    stacked_child(child, children.width, child_name_style(child), now)
                })
                .collect();
            Paragraph::new(lines).render(children, buffer);
        },
    }
}

/// The header at `width` cells across: [`header_line`] while it fits
/// uncut, else [`header_block`].
fn header(row: &AgentRow, machine: &str, width: u16, now: u64) -> Vec<Line<'static>> {
    let line = header_line(row, machine, now);
    if line.width() <= usize::from(width) {
        vec![line]
    } else {
        header_block(row, machine, now)
    }
}

/// The header on one line: `pid <pid> · <agent> · <status> · <age> ·
/// <machine> · <desktop>`, colored as the summary colors the same
/// values. It has no label column, so no part of it takes the label
/// color: `pid` reads as part of its value.
fn header_line(row: &AgentRow, machine: &str, now: u64) -> Line<'static> {
    let text = Style::default().fg(text_default());
    let separator = summary::separator;
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(format!("{PID_LABEL} {}", row.pid), text),
        separator(),
        Span::styled(row.agent.label(), summary::agent_role(row.agent).style()),
        separator(),
        Span::styled(
            summary::status_text(row).to_string(),
            summary::status_role(row).style(),
        ),
        separator(),
        Span::styled(age::age_label(now.saturating_sub(row.started)), text),
        separator(),
        Span::styled(machine.to_string(), text),
        separator(),
        Span::styled(summary::desktop_text(row).to_string(), text),
    ])
}

/// The header as a block, one fact to a line after a label column:
/// the agent, its pid, status, age, machine and desktop. Only the label
/// column takes the label color; each value is colored as
/// [`header_line`] colors it.
fn header_block(row: &AgentRow, machine: &str, now: u64) -> Vec<Line<'static>> {
    let label = Style::default().fg(label_color());
    let text = Style::default().fg(text_default());
    let facts = [
        (
            HEADER_AGENT_LABEL,
            Span::styled(row.agent.label(), summary::agent_role(row.agent).style()),
        ),
        (PID_LABEL, Span::styled(row.pid.to_string(), text)),
        (
            HEADER_STATUS_LABEL,
            Span::styled(
                summary::status_text(row).to_string(),
                summary::status_role(row).style(),
            ),
        ),
        (
            HEADER_AGE_LABEL,
            Span::styled(age::age_label(now.saturating_sub(row.started)), text),
        ),
        (
            HEADER_MACHINE_LABEL,
            Span::styled(machine.to_string(), text),
        ),
        (
            HEADER_DESKTOP_LABEL,
            Span::styled(summary::desktop_text(row).to_string(), text),
        ),
    ];
    let label_width = facts
        .iter()
        .map(|(name, _)| name.chars().count())
        .max()
        .unwrap_or_default()
        + usize::from(TABLE_COLUMN_SPACING);
    facts
        .into_iter()
        .map(|(name, value)| {
            Line::from(vec![
                Span::raw(SECTION_HEADER_INDENT),
                Span::styled(format!("{name:<label_width$}"), label),
                value,
            ])
        })
        .collect()
}

/// The agent's directory at `width` cells across, broken onto as many
/// lines as it takes, after a `/` where it can.
fn directory(row: &AgentRow, width: u16) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    let room = width.saturating_sub(summary::cell_width(SECTION_HEADER_INDENT));
    wrap::wrapped(&row.directory, usize::from(room))
        .into_iter()
        .map(|line| {
            Line::from(vec![
                Span::raw(SECTION_HEADER_INDENT),
                Span::styled(line, text),
            ])
        })
        .collect()
}

/// Cells across what an agent runs is drawn in, within a cell `width`
/// cells across: the width [`summary::indented`] leaves.
fn children_width(width: u16) -> u16 {
    width.saturating_sub(summary::cell_width(SECTION_ITEM_INDENT))
}

/// How an agent cell draws what the agent runs, at one width.
enum Children {
    /// The agent runs nothing, and a note says so.
    Nothing,
    /// A table, its columns fitted to their widest cells, every name
    /// whole.
    Table([Constraint; 5]),
    /// One entry after another, for a width where the table would cut a
    /// name.
    Stacked,
}

impl Children {
    /// The way `children` are drawn `width` cells across, with ages
    /// measured to `now`: a table while every name fits its column
    /// whole, else stacked entries.
    fn fitted(children: &[ChildRow], width: u16, now: u64) -> Self {
        if children.is_empty() {
            return Self::Nothing;
        }
        table_columns(children, width, now).map_or(Self::Stacked, Self::Table)
    }
}

/// One row of the table, its name drawn in `name_style`.
fn child_row(child: &ChildRow, name_style: Style, now: u64) -> Row<'static> {
    let text = Style::default().fg(text_default());
    let pid = child.pid.map_or_else(
        || Span::styled(MISSING_VALUE, text),
        |pid| Span::styled(pid.to_string(), text),
    );
    Row::new([
        pid,
        Span::styled(via_text(child), text),
        Span::styled(child.kind.runs(), runs_role(child.kind).style()),
        Span::styled(child.name.clone(), name_style),
        Span::styled(age::age_label(now.saturating_sub(child.started)), text),
    ])
}

/// One child as a stacked entry `width` cells across: `<via> · <runs> ·
/// <age> · pid <pid>`, the `via` indented as the table indents it and
/// the pid left out for a child with no process, then the name in full
/// on the lines below, indented under the `via` and drawn in
/// `name_style`. As in [`header_line`], `pid` reads as part of its
/// value.
fn stacked_child(child: &ChildRow, width: u16, name_style: Style, now: u64) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    let separator = summary::separator;
    let mut head = vec![
        Span::styled(via_text(child), text),
        separator(),
        Span::styled(child.kind.runs(), runs_role(child.kind).style()),
        separator(),
        Span::styled(age::age_label(now.saturating_sub(child.started)), text),
    ];
    if let Some(pid) = child.pid {
        head.extend([
            separator(),
            Span::styled(format!("{PID_LABEL} {pid}"), text),
        ]);
    }
    let indent = name_indent(child);
    iter::once(Line::from(head))
        .chain(name_lines(child, width).into_iter().map(|name| {
            Line::from(vec![
                Span::raw(format!("{:indent$}", "")),
                Span::styled(name, name_style),
            ])
        }))
        .collect()
}

/// Cells a stacked child's name is indented by: one level past its
/// `via`.
fn name_indent(child: &ChildRow) -> usize { (usize::from(child.depth) + 1) * CHILD_VIA_INDENT }

/// A stacked child's name broken onto the lines `width` cells across
/// leaves it past its indent.
fn name_lines(child: &ChildRow, width: u16) -> Vec<String> {
    wrap::wrapped(
        &child.name,
        usize::from(width).saturating_sub(name_indent(child)),
    )
}

/// The `via` cell: how the agent holds the row, indented
/// [`CHILD_VIA_INDENT`] cells for each level the row sits below the
/// agent.
fn via_text(child: &ChildRow) -> String {
    let indent = usize::from(child.depth) * CHILD_VIA_INDENT;
    format!("{:indent$}{}", "", child.kind.via())
}

/// The role a row's `runs` is drawn in: a command as a shell status,
/// and an agent as its program.
const fn runs_role(kind: ChildKind) -> Role {
    match kind.agent() {
        Some(agent) => summary::agent_role(agent),
        None => Role::Shell,
    }
}

/// The table's column widths within `width` cells, or none when a name
/// would not fit its column whole.
///
/// Every column fits its widest cell. When the five and the spacing
/// between them come to more than `width`, the table would have to cut
/// a name, and the children are stacked instead.
fn table_columns(children: &[ChildRow], width: u16, now: u64) -> Option<[Constraint; 5]> {
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
        widths.observe_cell_usize(CHILD_VIA_COLUMN, via_text(child).chars().count());
        widths.observe_cell_usize(CHILD_RUNS_COLUMN, child.kind.runs().chars().count());
        widths.observe_cell_usize(CHILD_NAME_COLUMN, child.name.chars().count());
        let age = age::age_label(now.saturating_sub(child.started));
        widths.observe_cell_usize(CHILD_AGE_COLUMN, age.chars().count());
    }
    let columns = [
        CHILD_PID_COLUMN,
        CHILD_VIA_COLUMN,
        CHILD_RUNS_COLUMN,
        CHILD_NAME_COLUMN,
        CHILD_AGE_COLUMN,
    ]
    .map(|column| widths.get(column));
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let spacing = TABLE_COLUMN_SPACING.saturating_mul(gaps);
    let needed = columns
        .iter()
        .fold(spacing, |total, &column| total.saturating_add(column));
    (needed <= width).then(|| columns.map(Constraint::Length))
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Color;

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
    /// tool-based-ui-arrange, a session trunk opened.
    const ARRANGE: u32 = 3_337_048;
    /// A cell's width where everything fits on its line: the one-line
    /// header, and trunk's table with every name whole.
    const WIDE: u16 = 104;
    /// A narrow cell's width, as a grid of many columns leaves each of
    /// them.
    const NARROW: u16 = 38;
    /// The directory of the narrow cells: longer than [`NARROW`] leaves.
    const LONG_DIRECTORY: &str = "~/rust/tool-based-ui-geometry-material-impl";

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

    /// tool-based-ui-trunk, a session boss opened, on the `berth_fix`
    /// desktop: a Codex app server that has left its shell, a shell
    /// running a Codex app server with a thread, a subagent running a
    /// shell whose command is its name, and the session it opened,
    /// tool-based-ui-arrange.
    fn trunk() -> AgentRow {
        AgentRow {
            desktop: Some("berth_fix".to_string()),
            children: vec![
                child(
                    0,
                    ChildKind::Detached(Agent::Codex),
                    Some(468_060),
                    "app-server",
                    22 * HOUR,
                ),
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_371_669),
                    "Launch the Phase 1 implementation seat",
                    12 * MINUTE,
                ),
                child(
                    1,
                    ChildKind::UnderShell(Agent::Codex),
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
                    Some(ARRANGE),
                    "tool-based-ui-arrange",
                    30,
                ),
            ],
            ..agent(3_266_367, "tool-based-ui-trunk", 23 * HOUR, Some(BOSS))
        }
    }

    /// `row`'s cell as natedev lists it, opened by the agent named
    /// `launcher`.
    fn entry<'a>(row: &'a AgentRow, launcher: Option<&'a str>) -> AgentEntry<'a> {
        AgentEntry {
            id: AgentCell {
                machine: "natedev".to_string(),
                pid:     row.pid,
                started: row.started,
            },
            row,
            launcher,
            machine: "natedev",
            hue: RainbowHue::Red,
        }
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across and
    /// exactly its own height at that width.
    fn drawn_among(entry: &AgentEntry<'_>, cells: &[AgentEntry<'_>], width: u16) -> Buffer {
        let height = u16::try_from(height(entry, width, NOW)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, width, height);
        let mut buffer = Buffer::empty(area);
        draw(&mut buffer, area, entry, cells, NOW);
        buffer
    }

    /// `row`'s cell, opened by the agent named `launcher`, drawn `width`
    /// cells across with no other cell on screen.
    fn drawn(row: &AgentRow, launcher: Option<&str>, width: u16) -> Buffer {
        drawn_among(&entry(row, launcher), &[], width)
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

    /// The cell `text` starts at in `buffer`, on the first line holding
    /// it.
    fn find(buffer: &Buffer, text: &str) -> Option<(u16, u16)> {
        lines(buffer).iter().enumerate().find_map(|(y, line)| {
            line.find(text).map(|at| {
                let x = line[..at].chars().count();
                (
                    u16::try_from(x).expect("the column should fit a u16"),
                    u16::try_from(y).expect("the row should fit a u16"),
                )
            })
        })
    }

    /// Where it all fits, a session boss opened draws its one-line
    /// header, its directory, the agent that launched it, and a table
    /// that indents each row's `via` under the row that started it,
    /// draws its `runs` in the program's color, shows `—` for a row
    /// with no process, and holds every name whole.
    #[test]
    fn a_launched_session_draws_its_header_and_tree() {
        let trunk = trunk();

        let buffer = drawn(&trunk, Some("boss of bosses"), WIDE);

        assert_eq!(
            lines(&buffer),
            [
                " pid 3266367 · claude · busy · 23h · natedev · berth_fix",
                " ~/rust/tool-based-ui-trunk",
                " launched by boss of bosses",
                "",
                " pid      via         runs     name                                                                age",
                " 468060   detached    codex    app-server                                                          22h",
                " 2371669  shell       command  Launch the Phase 1 implementation seat                              12m",
                " 2372720    shell     codex    app-server                                                          12m",
                " —            thread  codex    tool-based-ui-trunk-impl                                            11m",
                " —        subagent    claude   Review the permission queue                                         5m 3s",
                " 2424763    shell     command  cargo nextest run -p hana_video --no-fail-fast -- permission_queue  45s",
                " 3337048  session     claude   tool-based-ui-arrange                                               30s",
            ]
        );
        let role = |x, y| Some(buffer[(x, y)].fg);
        assert_eq!(buffer[(5, 0)].fg, text_default());
        assert_eq!(role(15, 0), Role::Claude.style().fg);
        assert_eq!(role(24, 0), Role::Busy.style().fg);
        assert_eq!(buffer[(1, 0)].fg, text_default(), "the pid marker");
        assert_eq!(buffer[(13, 0)].fg, text_default(), "a separator");
        assert_eq!(buffer[(37, 0)].fg, text_default(), "the machine");
        assert_eq!(buffer[(10, 5)].fg, text_default(), "a via");
        assert_eq!(role(22, 5), Role::Codex.style().fg);
        assert_eq!(role(22, 6), Role::Shell.style().fg);
        assert_eq!(buffer[(1, 8)].fg, text_default());
        let header = buffer[(1, 4)].fg;
        for y in 5..=11 {
            for x in 0..buffer.area.width {
                assert_ne!(
                    buffer[(x, y)].fg,
                    header,
                    "row {y} draws column {x} in the header color"
                );
            }
        }
        assert_eq!(buffer[(14, 8)].fg, text_default(), "an indented via");
        assert_eq!(role(22, 8), Role::Codex.style().fg);
        assert_eq!(role(22, 9), Role::Claude.style().fg);
    }

    /// In a cell too narrow for them, the header stands as a labelled
    /// block, the directory breaks after a `/`, and each child stands
    /// as its `via`, `runs`, age and pid over its name in full.
    #[test]
    fn a_narrow_cell_stacks_its_header_and_children() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };

        let buffer = drawn(&trunk, Some("boss of bosses"), NARROW);

        assert_eq!(
            lines(&buffer),
            [
                " agent    claude",
                " pid      3266367",
                " status   busy",
                " age      23h",
                " machine  natedev",
                " desktop  berth_fix",
                " ~/rust/",
                " tool-based-ui-geometry-material-impl",
                " launched by boss of bosses",
                "",
                " detached · codex · 22h · pid 468060",
                "   app-server",
                " shell · command · 12m · pid 2371669",
                "   Launch the Phase 1 implementation",
                "   seat",
                "   shell · codex · 12m · pid 2372720",
                "     app-server",
                "     thread · codex · 11m",
                "       tool-based-ui-trunk-impl",
                " subagent · claude · 5m 3s",
                "   Review the permission queue",
                "   shell · command · 45s · pid 2424763",
                "     cargo nextest run -p hana_video",
                "     --no-fail-fast --",
                "     permission_queue",
                " session · claude · 30s · pid 3337048",
                "   tool-based-ui-arrange",
            ]
        );
    }

    /// An agent running nothing says so in place of its table, and a
    /// launcher that is not listed is named by its pid.
    #[test]
    fn an_agent_running_nothing_says_so() {
        let arrange = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));

        assert_eq!(
            lines(&drawn(&arrange, None, WIDE)),
            [
                " pid 3337048 · claude · busy · 2h · natedev · —",
                " ~/rust/tool-based-ui-arrange",
                " launched by pid 1579022",
                "",
                " nothing running",
            ]
        );
    }

    /// The header keeps its one line at the width that holds it whole,
    /// and one cell narrower stands as a block: a label column, then
    /// one fact to a line, each value in the color the line gives it.
    #[test]
    fn the_header_stands_as_a_block_where_its_line_would_be_cut() {
        let boss = agent(BOSS, "boss of bosses", 2 * 24 * HOUR, None);
        let line = u16::try_from(header_line(&boss, "natedev", NOW).width())
            .expect("the line should fit a u16");

        let whole = drawn(&boss, None, line);
        let block = drawn(&boss, None, line - 1);

        assert_eq!(
            lines(&whole)[0],
            " pid 1579022 · claude · busy · 2d · natedev · —"
        );
        assert_eq!(
            lines(&block)[..6],
            [
                " agent    claude",
                " pid      1579022",
                " status   busy",
                " age      2d",
                " machine  natedev",
                " desktop  —",
            ]
        );
        let label = label_color();
        for y in 0..6 {
            assert_eq!(block[(1, y)].fg, label, "row {y}'s label");
            for x in 10..block.area.width {
                assert_ne!(block[(x, y)].fg, label, "row {y}'s value at column {x}");
            }
        }
        assert_eq!(Some(block[(10, 0)].fg), Role::Claude.style().fg);
        assert_eq!(block[(10, 1)].fg, text_default(), "the pid");
        assert_eq!(Some(block[(10, 2)].fg), Role::Busy.style().fg);
        assert_eq!(block[(10, 3)].fg, text_default(), "the age");
        assert_eq!(block[(10, 4)].fg, text_default(), "the machine");
        assert_eq!(block[(10, 5)].fg, text_default(), "the desktop");
    }

    /// A directory too long for its line breaks after the last `/` that
    /// fits, as many times as it takes, each line indented as the first.
    #[test]
    fn a_long_directory_breaks_after_a_slash() {
        let arrange = AgentRow {
            directory: "~/rust/hana_catalyst/crates/hana_video/src".to_string(),
            ..agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, None)
        };

        let lines = lines(&drawn(&arrange, None, 18));

        assert_eq!(
            lines[6..],
            [
                " ~/rust/",
                " hana_catalyst/",
                " crates/",
                " hana_video/src",
                "",
                " nothing running",
            ]
        );
    }

    /// Children stand as a table at the width that holds every name
    /// whole, and one cell narrower, where the table would cut a name,
    /// as stacked entries with each name in full.
    #[test]
    fn a_name_the_table_would_cut_stacks_the_children() {
        let arrange = AgentRow {
            children: vec![
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_424_763),
                    "cargo nextest run -p hana_video",
                    45,
                ),
                child(
                    0,
                    ChildKind::Subagent,
                    None,
                    "Review the permission queue",
                    5 * MINUTE + 3,
                ),
            ],
            ..agent(ARRANGE, "arrange", 2 * HOUR, None)
        };
        // The indent, then pid, via, runs, name and age at their widest,
        // two cells apart.
        let table = 1 + 7 + 2 + 8 + 2 + 7 + 2 + 31 + 2 + 5;

        let fits = drawn(&arrange, None, table);
        let stacked = drawn(&arrange, None, table - 1);

        assert_eq!(
            lines(&fits)[3..],
            [
                " pid      via       runs     name                             age",
                " 2424763  shell     command  cargo nextest run -p hana_video  45s",
                " —        subagent  claude   Review the permission queue      5m 3s",
            ]
        );
        assert_eq!(
            lines(&stacked)[3..],
            [
                " shell · command · 45s · pid 2424763",
                "   cargo nextest run -p hana_video",
                " subagent · claude · 5m 3s",
                "   Review the permission queue",
            ]
        );
        let label = label_color();
        assert_eq!(stacked[(1, 3)].fg, text_default(), "the via");
        assert_eq!(Some(stacked[(9, 3)].fg), Role::Shell.style().fg);
        assert_eq!(stacked[(19, 3)].fg, text_default(), "the age");
        assert_eq!(stacked[(7, 3)].fg, text_default(), "a separator");
        assert_eq!(stacked[(25, 3)].fg, text_default(), "the pid marker");
        assert_eq!(stacked[(29, 3)].fg, text_default(), "the pid");
        assert_eq!(Some(stacked[(12, 5)].fg), Role::Claude.style().fg);
        for y in [4, 6] {
            for x in 0..stacked.area.width {
                assert_ne!(
                    stacked[(x, y)].fg,
                    label,
                    "the name on row {y} draws column {x} in the label color"
                );
            }
        }
    }

    /// The text of each run of cells drawn in `color`, row by row,
    /// trimmed.
    fn runs_in(buffer: &Buffer, color: Color) -> Vec<String> {
        let mut runs = Vec::new();
        for y in 0..buffer.area.height {
            let mut run = String::new();
            for x in 0..=buffer.area.width {
                if x < buffer.area.width && buffer[(x, y)].fg == color {
                    run.push_str(buffer[(x, y)].symbol());
                } else if !run.trim().is_empty() {
                    runs.push(run.trim().to_string());
                    run.clear();
                } else {
                    run.clear();
                }
            }
        }
        runs
    }

    /// The label color marks labels and nothing else, so a label never
    /// reads as part of the values beside it: at any width, every run
    /// of it is a label, and no value, `pid` marker, note or separator
    /// takes it.
    #[test]
    fn only_labels_take_the_label_color() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let idle = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));
        let labels: HashSet<&str> = [
            HEADER_AGENT_LABEL,
            PID_LABEL,
            HEADER_STATUS_LABEL,
            HEADER_AGE_LABEL,
            HEADER_MACHINE_LABEL,
            HEADER_DESKTOP_LABEL,
            LAUNCHED_BY_LABEL,
        ]
        .into_iter()
        .chain(CHILD_HEADERS)
        .collect();
        for row in [&trunk, &idle] {
            for width in [NARROW, WIDE] {
                let buffer = drawn(row, Some("boss of bosses"), width);

                let runs = runs_in(&buffer, label_color());

                assert!(!runs.is_empty(), "width {width} draws its labels");
                for run in runs {
                    assert!(
                        labels.contains(run.as_str()),
                        "{run:?} takes the label color at width {width}"
                    );
                }
            }
        }
    }

    /// Whatever the width, the rows [`height`] counts are exactly the
    /// rows [`draw`] fills: the last of them holds text, and none past
    /// it do.
    #[test]
    fn the_height_is_the_rows_drawn_at_every_width() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let arrange = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));
        for row in [&trunk, &arrange] {
            let entry = entry(row, Some("boss of bosses"));
            for width in [12, 20, 30, NARROW, 57, 72, WIDE, 120] {
                let rows = height(&entry, width, NOW);
                let area = Rect::new(0, 0, width, u16::try_from(rows + 3).expect("fits"));
                let mut buffer = Buffer::empty(area);
                draw(&mut buffer, area, &entry, &[], NOW);

                let drawn = lines(&buffer);
                let last = drawn.iter().rposition(|line| !line.is_empty());
                assert_eq!(
                    last.map(|last| last + 1),
                    Some(rows),
                    "{} at {width} cells: {drawn:#?}",
                    row.name
                );
            }
        }
    }

    /// A name with a cell of its own is drawn in that cell's hue: the
    /// launcher's name after `launched by`, and a session's name in the
    /// table and in a stacked entry. A name with no cell keeps the
    /// default text color.
    #[test]
    fn a_name_with_a_cell_takes_that_cells_hue() {
        let natedev = MachineState::Answered(vec![
            agent(BOSS, "boss of bosses", 23 * HOUR, None),
            trunk(),
            agent(ARRANGE, "tool-based-ui-arrange", 30, Some(3_266_367)),
        ]);
        let machines = [Machine {
            name:  "natedev",
            state: &natedev,
        }];
        let cells = cell_order(&machines);
        let hue = |name: &str| {
            cells
                .iter()
                .find(|cell| cell.row.name == name)
                .map(|cell| Role::Rainbow(cell.hue).style().fg)
                .expect("the agent should have a cell")
        };
        let trunk = cells
            .iter()
            .find(|cell| cell.row.name == "tool-based-ui-trunk")
            .expect("trunk should have a cell");

        for width in [WIDE, NARROW] {
            let buffer = drawn_among(trunk, &cells, width);

            let launcher = find(&buffer, "boss of bosses").expect("the launcher is drawn");
            assert_eq!(
                Some(buffer[launcher].fg),
                hue("boss of bosses"),
                "at {width}"
            );
            let session = find(&buffer, "tool-based-ui-arrange").expect("the session is drawn");
            assert_eq!(
                Some(buffer[session].fg),
                hue("tool-based-ui-arrange"),
                "at {width}"
            );
            let shell = find(&buffer, "Launch").expect("the shell is drawn");
            assert_eq!(buffer[shell].fg, text_default(), "at {width}");
        }
    }

    /// natedev's answer and the mac's: two top-level agents, the
    /// sessions they opened, one under another, a session whose
    /// launcher is not listed and two rows naming only each other, then
    /// the mac's one agent.
    fn natedev_and_mac() -> (MachineState, MachineState) {
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
        (natedev, mac)
    }

    /// Machine by machine, each top-level agent comes oldest first,
    /// followed by the sessions it opened, depth first. A session whose
    /// launcher is not listed stands on its own, and two rows naming
    /// only each other still get a cell each.
    #[test]
    fn cells_follow_each_agent_with_the_sessions_it_opened() {
        let (natedev, mac) = natedev_and_mac();
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

    /// Every cell takes the next hue of the rainbow in cell order, a
    /// launched session as much as a top-level agent, carrying on from
    /// one machine to the next and starting over at red past violet.
    #[test]
    fn each_cell_takes_the_next_hue_of_the_rainbow() {
        let (natedev, mac) = natedev_and_mac();
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

        let hues: Vec<(&str, RainbowHue)> = cell_order(&machines)
            .iter()
            .map(|cell| (cell.row.name.as_str(), cell.hue))
            .collect();

        assert_eq!(
            hues,
            [
                ("boss of bosses", RainbowHue::Red),
                ("trunk", RainbowHue::Orange),
                ("under trunk", RainbowHue::Yellow),
                ("arrange", RainbowHue::Green),
                ("enh/handler", RainbowHue::Cyan),
                ("orphan", RainbowHue::Blue),
                ("left", RainbowHue::Violet),
                ("right", RainbowHue::Red),
                ("natemccoy-30", RainbowHue::Orange),
            ]
        );
    }
}
