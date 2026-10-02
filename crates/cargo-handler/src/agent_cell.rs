//! One cell for each top-level agent and the sessions it opened in
//! tmux: a header naming the agent, its branch and directory, the agent
//! that opened it where one did, the tree of its sessions, then what
//! they all run, each child indented under the one that started it and
//! each session's own children nested under the row that names it.
//!
//! [`cell_order`] lays the cells out across the machines, and
//! [`height`] and [`draw`] fill one in.
//!
//! A cell is laid out for its width. Where the header's one line would
//! be cut, it stands as a labelled block, one fact to a line; the
//! branch and directory break onto further lines; where the tree's lines
//! would be cut, each session stands as an entry of lines of its own;
//! and where the table would cut a child's name, each child stands as an
//! entry of its own with its name in full below it. Every agent's name
//! is drawn in its own hue wherever it shows.
//!
//! Where the grid gives a cell fewer rows than [`height`] asks for, the
//! cell draws its compressed view instead: one line to each session in
//! the tree and to each child in the table, cut where it would not fit,
//! and for a top-level agent, which the summary lists, no header and no
//! branch and directory.

use std::collections::HashSet;
use std::iter;

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
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
use crate::constants::HEADING_SEPARATOR;
use crate::constants::LAUNCHED_BY_LABEL;
use crate::constants::LAUNCHER_LINE_HEIGHT;
use crate::constants::MISSING_VALUE;
use crate::constants::NOTHING_RUNNING_HEIGHT;
use crate::constants::NOTHING_RUNNING_NOTE;
use crate::constants::PID_LABEL;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TREE_BRANCH;
use crate::constants::TREE_COLUMNS;
use crate::constants::TREE_LAST_BRANCH;
use crate::constants::TREE_RAIL;
use crate::constants::TREE_SPACE;
use crate::summary;
use crate::summary::age;
use crate::theme::RainbowHue;
use crate::theme::Role;
use crate::tiles::AgentCell;
use crate::wrap;

/// One cell, as the grid lays it out and draws it: an agent, and every
/// session it opened in tmux.
#[derive(Clone, Debug)]
pub(crate) struct AgentEntry<'a> {
    /// The cell's id in the grid.
    pub(crate) id:  AgentCell,
    /// The agent the cell is titled with.
    pub(crate) row: &'a AgentRow,
    /// The name of the agent that opened this one in tmux, where that
    /// agent is listed on the same machine.
    launcher:       Option<&'a str>,
    /// The heading of the machine the agent runs on.
    machine:        &'a str,
    /// The hue the cell's title and the agent's name in the summary are
    /// drawn in.
    pub(crate) hue: RainbowHue,
    /// The sessions the agent opened, each followed by the sessions it
    /// opened in turn: the tree the cell draws, top to bottom.
    sessions:       Vec<SessionEntry<'a>>,
}

impl AgentEntry<'_> {
    /// The hue of the agent with process `pid` in this cell: the cell's
    /// own agent, or one of its sessions.
    fn hue_of(&self, pid: u32) -> Option<RainbowHue> {
        if self.row.pid == pid {
            return Some(self.hue);
        }
        self.sessions
            .iter()
            .find(|session| session.row.pid == pid)
            .map(|session| session.hue)
    }
}

/// One session in a cell's tree.
#[derive(Clone, Debug)]
struct SessionEntry<'a> {
    /// The session.
    row:   &'a AgentRow,
    /// The hue its name is drawn in, in the tree and in the table.
    hue:   RainbowHue,
    /// For each level between the cell's agent and the session's
    /// launcher, top down, whether the tree's line carries on down past
    /// the session there: whether that level's session has a later one
    /// beside it.
    rails: Vec<bool>,
    /// Whether the session is the last its launcher opened.
    last:  bool,
}

impl SessionEntry<'_> {
    /// The glyphs before the session's name: the lines carried down from
    /// the levels above, then its own branch off its launcher's line.
    fn lead(&self) -> String {
        let branch = if self.last {
            TREE_LAST_BRANCH
        } else {
            TREE_BRANCH
        };
        self.rails
            .iter()
            .map(|&rail| rail_glyphs(rail))
            .chain(iter::once(branch))
            .collect()
    }

    /// The glyphs before each further line of the session's stacked
    /// entry: the lines carried down from the levels above, and past the
    /// session's own level while a later session hangs beside it.
    fn continuation(&self) -> String {
        self.rails
            .iter()
            .chain(iter::once(&!self.last))
            .map(|&rail| rail_glyphs(rail))
            .collect()
    }
}

/// The glyphs a level of the tree takes on a line that does not branch
/// at it: its line carried down, or blank.
const fn rail_glyphs(rail: bool) -> &'static str { if rail { TREE_RAIL } else { TREE_SPACE } }

/// Every cell across `machines`, in the order the grid shows them:
/// machine by machine, one for each top-level agent, holding the
/// sessions it opened, depth first and oldest first. The agents that
/// opened sessions come first, then the rest, each oldest first. A
/// session whose launcher is not listed stands as a top-level agent.
/// Each agent takes the next hue of the rainbow in that order, a
/// session as much as the agent titling its cell.
pub(crate) fn cell_order<'a>(machines: &[Machine<'a>]) -> Vec<AgentEntry<'a>> {
    let mut cells = Vec::new();
    let mut agents = 0;
    for machine in machines {
        let rows = machine.state.rows();
        let listed: HashSet<u32> = rows.iter().map(|row| row.pid).collect();
        let mut placed = HashSet::new();
        let (groups, lone): (Vec<&AgentRow>, Vec<&AgentRow>) = rows
            .iter()
            .filter(|row| {
                row.launched_by
                    .is_none_or(|launcher| launcher == row.pid || !listed.contains(&launcher))
            })
            .partition(|row| {
                rows.iter()
                    .any(|other| other.pid != row.pid && other.launched_by == Some(row.pid))
            });
        // Rows that only name one another as launcher reach no root, so
        // the first of them not yet placed stands as one rather than
        // going without a cell.
        for row in groups.into_iter().chain(lone).chain(rows) {
            if !placed.insert(row.pid) {
                continue;
            }
            let hue = RainbowHue::of_agent(agents);
            agents += 1;
            let mut sessions = Vec::new();
            let mut placing = Placing {
                rows,
                placed: &mut placed,
                agents: &mut agents,
                sessions: &mut sessions,
            };
            placing.sessions_of(row.pid, &mut Vec::new());
            let launcher = row.launched_by.and_then(|launcher| {
                rows.iter()
                    .find(|other| other.pid == launcher)
                    .map(|other| other.name.as_str())
            });
            cells.push(AgentEntry {
                id: AgentCell {
                    machine: machine.name.to_string(),
                    pid:     row.pid,
                    started: row.started,
                },
                row,
                launcher,
                machine: machine.name,
                hue,
                sessions,
            });
        }
    }
    cells
}

/// Where [`cell_order`] stands while it gathers one cell's sessions.
struct Placing<'a, 'p> {
    /// The machine's rows.
    rows:     &'a [AgentRow],
    /// The rows given a place, in this cell or an earlier one.
    placed:   &'p mut HashSet<u32>,
    /// Agents given a hue so far, across every machine.
    agents:   &'p mut usize,
    /// The cell's sessions so far.
    sessions: &'p mut Vec<SessionEntry<'a>>,
}

impl Placing<'_, '_> {
    /// Put the sessions `launcher` opened among the rows into the cell,
    /// each followed by the sessions it opened, skipping any row already
    /// placed. `rails` says for each level above whether the tree's line
    /// carries on down past it.
    fn sessions_of(&mut self, launcher: u32, rails: &mut Vec<bool>) {
        let rows = self.rows;
        let opened: Vec<&AgentRow> = rows
            .iter()
            .filter(|row| row.launched_by == Some(launcher))
            .filter(|row| self.placed.insert(row.pid))
            .collect();
        for (index, row) in opened.iter().enumerate() {
            let last = index + 1 == opened.len();
            self.sessions.push(SessionEntry {
                row,
                hue: RainbowHue::of_agent(*self.agents),
                rails: rails.clone(),
                last,
            });
            *self.agents += 1;
            rails.push(!last);
            self.sessions_of(row.pid, rails);
            rails.pop();
        }
    }
}

/// How [`draw`] lays a cell out in the rows the grid gives it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CellView {
    /// Every row [`height`] counts: the header, the branch and
    /// directory, the tree and the table as the width lays them out.
    Full,
    /// For rows too few for the full view: one line to each session and
    /// to each child, cut where it would not fit, and no header or
    /// branch and directory for an agent the summary lists.
    Compressed,
}

impl CellView {
    /// The view `entry`'s cell takes in `area`, with ages measured to
    /// `now`: full while its [`height`] at the area's width fits the
    /// area's rows, else compressed.
    fn fitted(entry: &AgentEntry<'_>, area: Rect, now: u64) -> Self {
        if height(entry, area.width, now) > usize::from(area.height) {
            Self::Compressed
        } else {
            Self::Full
        }
    }
}

/// Rows `entry`'s cell draws at `width` cells across, with ages
/// measured to `now`: exactly the rows [`draw`]'s full view fills at
/// that width, and the rows the cell asks the grid for. Its header, its
/// branch and directory, a stacked session and a stacked child's name
/// each take as many rows as the width leaves them.
pub(crate) fn height(entry: &AgentEntry<'_>, width: u16, now: u64) -> usize {
    let row = entry.row;
    let launcher = if row.launched_by.is_some() {
        usize::from(LAUNCHER_LINE_HEIGHT)
    } else {
        0
    };
    let children = group_children(entry);
    let children_width = children_width(width);
    let children_height = match Children::fitted(&children, children_width, now, CellView::Full) {
        Children::Nothing => usize::from(NOTHING_RUNNING_HEIGHT),
        Children::Table(_) => usize::from(TABLE_HEADER_HEIGHT) + children.len(),
        Children::Compressed => children.len(),
        Children::Stacked => children
            .iter()
            .map(|child| {
                stacked_head(child, children_width, now).len()
                    + name_lines(child, children_width).len()
            })
            .sum(),
    };
    header(row, entry.machine, width, now).len()
        + place(row, width).len()
        + launcher
        + tree_height(&entry.sessions, header_width(width), now, CellView::Full)
        + usize::from(AGENT_HEADER_GAP_HEIGHT)
        + children_height
}

/// The style the name of the agent with process `pid` on `machine` is
/// drawn in wherever it shows: the hue that agent takes among `cells`,
/// so the name pairs with its cell's title or its line in a cell's tree,
/// or the default text color for a process that is not a listed agent.
pub(crate) fn name_style(cells: &[AgentEntry<'_>], machine: &str, pid: u32) -> Style {
    cells
        .iter()
        .filter(|cell| cell.machine == machine)
        .find_map(|cell| cell.hue_of(pid))
        .map_or_else(
            || Style::default().fg(text_default()),
            |hue| Role::Rainbow(hue).style(),
        )
}

/// Draw `entry`'s cell into `area`, with ages measured to `now` in unix
/// seconds: where the area has the rows [`height`] asks for, its full
/// view -- its header, its branch and directory, the agent that opened
/// it when one did, the tree of its sessions, then what they all run --
/// else its compressed view, cut at the bottom where even that does not
/// fit. A listed agent's name among `cells` -- the launcher's, a
/// session's -- is drawn in that agent's hue.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    now: u64,
) {
    let view = CellView::fitted(entry, area, now);
    let label = Style::default().fg(label_color());
    let above = above_tree(entry, cells, area.width, now, view);
    let above_height = u16::try_from(above.len()).unwrap_or(u16::MAX);
    Paragraph::new(above).render(
        Rect {
            height: above_height.min(area.height),
            ..area
        },
        buffer,
    );

    let tree = header_indented(Rect {
        y: area.y.saturating_add(above_height),
        height: area.height.saturating_sub(above_height),
        ..area
    });
    draw_tree(buffer, tree, &entry.sessions, now, view);

    let tree_height = tree_height(&entry.sessions, tree.width, now, view);
    let drawn = above_height.saturating_add(u16::try_from(tree_height).unwrap_or(u16::MAX));
    // A compressed top-level agent with no sessions draws nothing above
    // its table, so no blank row leads the cell.
    let gap = if drawn == 0 {
        0
    } else {
        AGENT_HEADER_GAP_HEIGHT
    };
    let skipped = drawn.saturating_add(gap);
    let area = Rect {
        y: area.y.saturating_add(skipped),
        height: area.height.saturating_sub(skipped),
        ..summary::indented(area)
    };
    if area.is_empty() {
        return;
    }
    let children = group_children(entry);
    let child_name_style = |child: &ChildRow| {
        child.pid.map_or_else(
            || Style::default().fg(text_default()),
            |pid| name_style(cells, entry.machine, pid),
        )
    };
    match Children::fitted(&children, area.width, now, view) {
        Children::Nothing => {
            Paragraph::new(Line::from(Span::styled(
                NOTHING_RUNNING_NOTE,
                Style::default().fg(text_default()),
            )))
            .render(area, buffer);
        },
        Children::Compressed => {
            let lines: Vec<Line<'static>> = children
                .iter()
                .map(|child| compressed_child(child, area.width, child_name_style(child)))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
        Children::Table(constraints) => Table::new(
            children
                .iter()
                .map(|child| child_row(child, child_name_style(child), now)),
            constraints,
        )
        .header(Row::new(
            CHILD_HEADERS.map(|header| Span::styled(header, label)),
        ))
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(area, buffer),
        Children::Stacked => {
            let lines: Vec<Line<'static>> = children
                .iter()
                .flat_map(|child| stacked_child(child, area.width, child_name_style(child), now))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
    }
}

/// The lines `entry`'s cell draws above its tree in `view`, `width`
/// cells across: its header, its branch and directory, and the agent
/// that opened it when one did. The compressed view of a top-level
/// agent draws none of them, as the summary lists that agent.
fn above_tree(
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    width: u16,
    now: u64,
    view: CellView,
) -> Vec<Line<'static>> {
    let row = entry.row;
    if view == CellView::Compressed && row.is_top_level() {
        return Vec::new();
    }
    let mut lines = header(row, entry.machine, width, now);
    lines.extend(place(row, width));
    if let Some(launcher) = row.launched_by {
        let name = entry
            .launcher
            .map_or_else(|| format!("{PID_LABEL} {launcher}"), str::to_string);
        lines.push(Line::from(vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(
                format!("{LAUNCHED_BY_LABEL} "),
                Style::default().fg(label_color()),
            ),
            Span::styled(name, name_style(cells, entry.machine, launcher)),
        ]));
    }
    lines
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
        Span::styled(row.agent_label(), summary::agent_role(row.agent).style()),
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
            Span::styled(row.agent_label(), summary::agent_role(row.agent).style()),
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

/// Where `row` works, as one value: `<branch> · <directory>`, or the
/// directory alone outside a repository.
fn workplace(row: &AgentRow) -> String {
    row.branch.as_ref().map_or_else(
        || row.directory.clone(),
        |branch| format!("{branch}{HEADING_SEPARATOR}{}", row.directory),
    )
}

/// The agent's branch and directory at `width` cells across, broken
/// onto as many lines as they take, after a `/` or before a space where
/// they can.
fn place(row: &AgentRow, width: u16) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    wrap::wrapped(&workplace(row), usize::from(header_width(width)))
        .into_iter()
        .map(|line| {
            Line::from(vec![
                Span::raw(SECTION_HEADER_INDENT),
                Span::styled(line, text),
            ])
        })
        .collect()
}

/// Cells across the header's lines and the tree leave within a cell
/// `width` cells across: what the header's indent leaves.
fn header_width(width: u16) -> u16 {
    width.saturating_sub(summary::cell_width(SECTION_HEADER_INDENT))
}

/// `area` less the header's indent.
fn header_indented(area: Rect) -> Rect {
    let indent = summary::cell_width(SECTION_HEADER_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area.width.saturating_sub(indent),
        ..area
    }
}

/// How a cell draws the tree of the sessions its agent opened, at one
/// width.
enum Tree {
    /// One line to a session, its columns -- the session's name after
    /// its glyphs, status, branch, directory and age -- fitted to their
    /// widest cells.
    Table([Constraint; TREE_COLUMNS]),
    /// Each session as an entry of lines of its own, for a width where
    /// a line would be cut.
    Stacked,
    /// One line to a session in the compressed view: its name after its
    /// glyphs, then its status and age, cut where it would not fit.
    Compressed,
}

impl Tree {
    /// The way `sessions` are drawn `width` cells across in `view`, with
    /// ages measured to `now`: in the full view a line to each while
    /// every line fits whole, else stacked entries.
    fn fitted(sessions: &[SessionEntry<'_>], width: u16, now: u64, view: CellView) -> Self {
        match view {
            CellView::Full => {
                let mut widths = [0; TREE_COLUMNS];
                for session in sessions {
                    for (widest, cell) in widths.iter_mut().zip(tree_cells(session, now)) {
                        *widest = (*widest).max(cell.chars().count());
                    }
                }
                let spacing = usize::from(TABLE_COLUMN_SPACING) * (TREE_COLUMNS - 1);
                let needed = widths.iter().sum::<usize>() + spacing;
                if needed > usize::from(width) {
                    return Self::Stacked;
                }
                Self::Table(
                    widths.map(|widest| {
                        Constraint::Length(u16::try_from(widest).unwrap_or(u16::MAX))
                    }),
                )
            },
            CellView::Compressed => Self::Compressed,
        }
    }
}

/// The text of a session's line in the tree, column by column: its
/// glyphs and name, its status, branch, directory and age.
fn tree_cells(session: &SessionEntry<'_>, now: u64) -> [String; TREE_COLUMNS] {
    let row = session.row;
    [
        format!("{}{}", session.lead(), row.name),
        summary::status_text(row).to_string(),
        row.branch
            .clone()
            .unwrap_or_else(|| MISSING_VALUE.to_string()),
        row.directory.clone(),
        age::age_label(now.saturating_sub(row.started)),
    ]
}

/// Rows the tree of `sessions` takes `width` cells across in `view`,
/// with ages measured to `now`.
fn tree_height(sessions: &[SessionEntry<'_>], width: u16, now: u64, view: CellView) -> usize {
    match Tree::fitted(sessions, width, now, view) {
        Tree::Table(_) | Tree::Compressed => sessions.len(),
        Tree::Stacked => sessions
            .iter()
            .map(|session| stacked_session(session, width, now).len())
            .sum(),
    }
}

/// Draw the tree of `sessions` into `area` in `view`, with ages
/// measured to `now`: in the full view a line to each session where
/// every line fits, else each as a stacked entry, and in the compressed
/// view a line to each cut where it would not fit. The glyphs are plain
/// text and each name takes its session's hue.
fn draw_tree(
    buffer: &mut Buffer,
    area: Rect,
    sessions: &[SessionEntry<'_>],
    now: u64,
    view: CellView,
) {
    if sessions.is_empty() || area.is_empty() {
        return;
    }
    let text = Style::default().fg(text_default());
    match Tree::fitted(sessions, area.width, now, view) {
        Tree::Table(constraints) => Table::new(
            sessions.iter().map(|session| {
                let row = session.row;
                Row::new([
                    Cell::from(Line::from(vec![
                        Span::styled(session.lead(), text),
                        Span::styled(row.name.clone(), Role::Rainbow(session.hue).style()),
                    ])),
                    Cell::from(Span::styled(
                        summary::status_text(row).to_string(),
                        summary::status_role(row).style(),
                    )),
                    Cell::from(Span::styled(
                        row.branch
                            .clone()
                            .unwrap_or_else(|| MISSING_VALUE.to_string()),
                        text,
                    )),
                    Cell::from(Span::styled(row.directory.clone(), text)),
                    Cell::from(Span::styled(
                        age::age_label(now.saturating_sub(row.started)),
                        text,
                    )),
                ])
            }),
            constraints,
        )
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(area, buffer),
        Tree::Stacked => {
            let lines: Vec<Line<'static>> = sessions
                .iter()
                .flat_map(|session| stacked_session(session, area.width, now))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
        Tree::Compressed => {
            let lines: Vec<Line<'static>> = sessions
                .iter()
                .map(|session| compressed_session(session, area.width, now))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
    }
}

/// One session's line in the compressed view, `width` cells across: its
/// name after its glyphs, drawn in its hue, then its status and age,
/// each [`gap`] after the one before as the tree's table spaces its
/// columns, cut by [`cut_line`] where it would not fit.
fn compressed_session(session: &SessionEntry<'_>, width: u16, now: u64) -> Line<'static> {
    let text = Style::default().fg(text_default());
    let row = session.row;
    cut_line(
        vec![
            Span::styled(session.lead(), text),
            Span::styled(row.name.clone(), Role::Rainbow(session.hue).style()),
            gap(),
            Span::styled(
                summary::status_text(row).to_string(),
                summary::status_role(row).style(),
            ),
            gap(),
            Span::styled(age::age_label(now.saturating_sub(row.started)), text),
        ],
        width,
    )
}

/// One session as a stacked entry `width` cells across: its name after
/// its glyphs, broken onto as many lines as it takes and drawn in its
/// hue, then `<status> · <age>`, then its branch and directory as
/// [`place`] gives the agent's, each further line led by the glyphs
/// that carry the tree's lines down past it.
fn stacked_session(session: &SessionEntry<'_>, width: u16, now: u64) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    let row = session.row;
    let lead = session.lead();
    let continuation = session.continuation();
    let room = usize::from(width).saturating_sub(lead.chars().count());
    let hue = Role::Rainbow(session.hue).style();
    let names = wrap::wrapped(&row.name, room)
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let glyphs = if index == 0 {
                lead.clone()
            } else {
                continuation.clone()
            };
            Line::from(vec![Span::styled(glyphs, text), Span::styled(name, hue)])
        });
    let facts = fact_lines(
        &continuation,
        vec![
            Span::styled(
                summary::status_text(row).to_string(),
                summary::status_role(row).style(),
            ),
            Span::styled(age::age_label(now.saturating_sub(row.started)), text),
        ],
        width,
    );
    let workplace = wrap::wrapped(&workplace(row), room)
        .into_iter()
        .map(|line| {
            Line::from(vec![
                Span::styled(continuation.clone(), text),
                Span::styled(line, text),
            ])
        });
    names.chain(facts).chain(workplace).collect()
}

/// What `entry`'s agent runs, in the order its table draws it, with
/// each session's own children nested under the row that names the
/// session, one level below it, as far down as the sessions go.
fn group_children(entry: &AgentEntry<'_>) -> Vec<ChildRow> {
    let mut children = Vec::new();
    let mut nested = HashSet::from([entry.row.pid]);
    nest(entry, entry.row, 0, &mut nested, &mut children);
    children
}

/// Put `agent`'s children into `children`, `below` levels deeper than
/// the agent puts them, each session among `entry`'s followed by its
/// own unless they are already `nested`.
fn nest(
    entry: &AgentEntry<'_>,
    agent: &AgentRow,
    below: u8,
    nested: &mut HashSet<u32>,
    children: &mut Vec<ChildRow>,
) {
    for child in &agent.children {
        let depth = child.depth.saturating_add(below);
        children.push(ChildRow {
            depth,
            ..child.clone()
        });
        let session = match (child.kind, child.pid) {
            (ChildKind::Session(_), Some(pid)) => {
                entry.sessions.iter().find(|session| session.row.pid == pid)
            },
            _ => None,
        };
        if let Some(session) = session
            && nested.insert(session.row.pid)
        {
            nest(
                entry,
                session.row,
                depth.saturating_add(1),
                nested,
                children,
            );
        }
    }
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
    /// One line to a child in the compressed view: what it runs, then
    /// its name, indented by level and cut where it would not fit.
    Compressed,
}

impl Children {
    /// The way `children` are drawn `width` cells across in `view`, with
    /// ages measured to `now`: in the full view a table while every name
    /// fits its column whole, else stacked entries.
    fn fitted(children: &[ChildRow], width: u16, now: u64, view: CellView) -> Self {
        if children.is_empty() {
            return Self::Nothing;
        }
        match view {
            CellView::Full => {
                table_columns(children, width, now).map_or(Self::Stacked, Self::Table)
            },
            CellView::Compressed => Self::Compressed,
        }
    }
}

/// One child's line in the compressed view, `width` cells across:
/// indented as the table indents its `via`, what it `runs` in the role
/// the table draws it in, then [`gap`] and its name in `name_style`, cut
/// by [`cut_line`] where it would not fit.
fn compressed_child(child: &ChildRow, width: u16, name_style: Style) -> Line<'static> {
    let text = Style::default().fg(text_default());
    cut_line(
        vec![
            Span::styled(depth_indent(child), text),
            Span::styled(child.runs_label(), runs_role(child.kind).style()),
            gap(),
            Span::styled(child.name.clone(), name_style),
        ],
        width,
    )
}

/// The blank cells between the facts of a line in the compressed view:
/// as many as [`TABLE_COLUMN_SPACING`] sets between the columns of the
/// full view's tables.
fn gap() -> Span<'static> {
    Span::styled(
        format!("{:width$}", "", width = usize::from(TABLE_COLUMN_SPACING)),
        Style::default().fg(text_default()),
    )
}

/// `spans` as one line cut to `width` cells as [`summary::truncated`]
/// cuts a name: whole while they fit, else ending in its mark, each
/// character kept in the style of the span it came from.
fn cut_line(spans: Vec<Span<'static>>, width: u16) -> Line<'static> {
    let text: String = spans.iter().map(|span| span.content.as_ref()).collect();
    let cut = summary::truncated(&text, usize::from(width));
    let mut kept = cut.chars();
    Line::from(
        spans
            .into_iter()
            .filter_map(|span| {
                let content: String = kept.by_ref().take(span.content.chars().count()).collect();
                (!content.is_empty()).then(|| Span::styled(content, span.style))
            })
            .collect::<Vec<_>>(),
    )
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
        Span::styled(child.runs_label(), runs_role(child.kind).style()),
        Span::styled(child.name.clone(), name_style),
        Span::styled(age::age_label(now.saturating_sub(child.started)), text),
    ])
}

/// One child as a stacked entry `width` cells across: `<via> · <runs> ·
/// <age> · pid <pid>` as [`fact_lines`] sets them, indented as the
/// table indents the `via` and the pid left out for a child with no
/// process, then the name in full on the lines below, indented under
/// the `via` and drawn in `name_style`. As in [`header_line`], `pid`
/// reads as part of its value.
fn stacked_child(child: &ChildRow, width: u16, name_style: Style, now: u64) -> Vec<Line<'static>> {
    let indent = name_indent(child);
    stacked_head(child, width, now)
        .into_iter()
        .chain(name_lines(child, width).into_iter().map(|name| {
            Line::from(vec![
                Span::raw(format!("{:indent$}", "")),
                Span::styled(name, name_style),
            ])
        }))
        .collect()
}

/// The lines a stacked child's facts take `width` cells across, above
/// its name.
fn stacked_head(child: &ChildRow, width: u16, now: u64) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    let mut facts = vec![
        Span::styled(child.kind.via(), text),
        Span::styled(child.runs_label(), runs_role(child.kind).style()),
        Span::styled(age::age_label(now.saturating_sub(child.started)), text),
    ];
    if let Some(pid) = child.pid {
        facts.push(Span::styled(format!("{PID_LABEL} {pid}"), text));
    }
    fact_lines(&depth_indent(child), facts, width)
}

/// `facts` one after another with ` · ` between them, on as many lines
/// `width` cells across as they take: a fact that would be cut starts a
/// line of its own. Every line starts with `lead`, in the default text
/// color.
fn fact_lines(lead: &str, facts: Vec<Span<'static>>, width: u16) -> Vec<Line<'static>> {
    let text = Style::default().fg(text_default());
    let separator = HEADING_SEPARATOR.chars().count();
    let mut lines: Vec<Vec<Span<'static>>> = Vec::new();
    let mut used = 0;
    for fact in facts {
        let fact_width = fact.width();
        match lines.last_mut() {
            Some(line) if used + separator + fact_width <= usize::from(width) => {
                line.extend([summary::separator(), fact]);
                used += separator + fact_width;
            },
            _ => {
                lines.push(vec![Span::styled(lead.to_string(), text), fact]);
                used = lead.chars().count() + fact_width;
            },
        }
    }
    lines.into_iter().map(Line::from).collect()
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

/// The `via` cell: how the agent holds the row, after its
/// [`depth_indent`].
fn via_text(child: &ChildRow) -> String { format!("{}{}", depth_indent(child), child.kind.via()) }

/// The blank cells a row is indented by: [`CHILD_VIA_INDENT`] for each
/// level it sits below the agent.
fn depth_indent(child: &ChildRow) -> String {
    format!(
        "{:width$}",
        "",
        width = usize::from(child.depth) * CHILD_VIA_INDENT
    )
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
        widths.observe_cell_usize(CHILD_RUNS_COLUMN, child.runs_label().chars().count());
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
    use crate::census::ServiceTier;
    use crate::constants::TRUNCATION_MARK;

    /// The unix second every age in these tests is measured to.
    const NOW: u64 = 1_790_372_800;
    /// Seconds in a minute.
    const MINUTE: u64 = 60;
    /// Seconds in an hour.
    const HOUR: u64 = 60 * MINUTE;
    /// boss of bosses, which opened the sessions below.
    const BOSS: u32 = 1_579_022;
    /// tool-based-ui-arrange, a session boss opened.
    const ARRANGE: u32 = 3_337_048;
    /// tool-based-ui-trunk, a session boss opened.
    const TRUNK: u32 = 3_266_367;
    /// trunk-impl, a session trunk opened.
    const TRUNK_IMPL: u32 = 3_400_000;
    /// A cell's width where everything fits on its line: the one-line
    /// header, and trunk's table with every name whole.
    const WIDE: u16 = 104;
    /// A narrow cell's width, as a grid of many columns leaves each of
    /// them.
    const NARROW: u16 = 38;
    /// The directory of the narrow cells: longer than [`NARROW`] leaves.
    const LONG_DIRECTORY: &str = "~/rust/tool-based-ui-geometry-material-impl";
    /// A cell's width that cuts lines of a compressed cell's tree and
    /// of its table.
    const CUTTING: u16 = 30;

    /// A Claude Code row with process `pid`, named `name`, started `age`
    /// seconds before [`NOW`] and opened by `launched_by`.
    fn agent(pid: u32, name: &str, age: u64, launched_by: Option<u32>) -> AgentRow {
        AgentRow {
            agent: Agent::Claude,
            service_tier: ServiceTier::Unrecorded,
            name: name.to_string(),
            status: Some("busy".to_string()),
            started: NOW - age,
            pid,
            desktop: None,
            directory: format!("~/rust/{name}"),
            branch: None,
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
            service_tier: ServiceTier::Unrecorded,
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
            sessions: Vec::new(),
        }
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across into
    /// `rows` rows.
    fn drawn_into(
        entry: &AgentEntry<'_>,
        cells: &[AgentEntry<'_>],
        width: u16,
        rows: usize,
    ) -> Buffer {
        let rows = u16::try_from(rows).expect("the rows should fit a u16");
        let area = Rect::new(0, 0, width, rows);
        let mut buffer = Buffer::empty(area);
        draw(&mut buffer, area, entry, cells, NOW);
        buffer
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across and
    /// exactly its own height at that width.
    fn drawn_among(entry: &AgentEntry<'_>, cells: &[AgentEntry<'_>], width: u16) -> Buffer {
        drawn_into(entry, cells, width, height(entry, width, NOW))
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

    /// Where it all fits, a cell draws its agent's one-line header, its
    /// directory, the agent that launched it, and a table
    /// that indents each row's `via` under the row that started it,
    /// draws its `runs` in the program's color, shows `—` for a row
    /// with no process, and holds every name whole.
    #[test]
    fn a_cell_draws_its_header_and_what_its_agent_runs() {
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

    /// A Codex agent's tier follows its program in the header, and a
    /// Codex thread's in its `runs`, in the Codex color; a row no tier
    /// was found for shows its program alone.
    #[test]
    fn a_codex_tier_follows_its_program() {
        let codex = AgentRow {
            agent: Agent::Codex,
            service_tier: ServiceTier::Fast,
            status: None,
            children: vec![
                child(
                    0,
                    ChildKind::Detached(Agent::Codex),
                    Some(468_060),
                    "app-server",
                    11 * MINUTE,
                ),
                ChildRow {
                    service_tier: ServiceTier::Standard,
                    ..child(1, ChildKind::Thread, None, "trunk mesh", 10 * MINUTE)
                },
            ],
            ..agent(4_039_085, "handler", 12 * MINUTE, None)
        };

        let buffer = drawn(&codex, None, WIDE);

        assert_eq!(
            lines(&buffer),
            [
                " pid 4039085 · codex fast · — · 12m · natedev · —",
                " ~/rust/handler",
                "",
                " pid     via       runs      name        age",
                " 468060  detached  codex     app-server  11m",
                " —         thread  codex --  trunk mesh  10m",
            ]
        );
        let role = |x, y| Some(buffer[(x, y)].fg);
        for (x, y) in [(15, 0), (24, 0), (19, 5), (26, 5)] {
            assert_eq!(role(x, y), Role::Codex.style().fg, "column {x} of row {y}");
        }
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
    /// of it is a label, and no value, `pid` marker, note, separator or
    /// line of a cell's tree takes it.
    #[test]
    fn only_labels_take_the_label_color() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let idle = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));
        let natedev = boss_group();
        let group = natedev_cells(&natedev);
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
        let entries = [
            entry(&trunk, Some("boss of bosses")),
            entry(&idle, Some("boss of bosses")),
            group[0].clone(),
        ];
        for entry in &entries {
            for width in [NARROW, WIDE] {
                let buffer = drawn_among(entry, &group, width);

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
        let natedev = boss_group();
        let group = natedev_cells(&natedev);
        let entries = [
            entry(&trunk, Some("boss of bosses")),
            entry(&arrange, Some("boss of bosses")),
            group[0].clone(),
        ];
        for entry in &entries {
            for width in [12, 20, 30, NARROW, 57, 72, WIDE, 120] {
                let rows = height(entry, width, NOW);
                let area = Rect::new(0, 0, width, u16::try_from(rows + 3).expect("fits"));
                let mut buffer = Buffer::empty(area);
                draw(&mut buffer, area, entry, &group, NOW);

                let drawn = lines(&buffer);
                let last = drawn.iter().rposition(|line| !line.is_empty());
                assert_eq!(
                    last.map(|last| last + 1),
                    Some(rows),
                    "{} at {width} cells: {drawn:#?}",
                    entry.row.name
                );
            }
        }
    }

    /// natedev with boss of bosses and the sessions it opened: trunk,
    /// which opened trunk-impl in turn, and arrange. Each works on a
    /// branch of its own but trunk-impl, whose directory is no
    /// repository.
    fn boss_group() -> MachineState {
        let session = |pid, name: &str, age| {
            child(0, ChildKind::Session(Agent::Claude), Some(pid), name, age)
        };
        MachineState::Answered(vec![
            AgentRow {
                branch: Some("main".to_string()),
                directory: "~/rust/cargo-liner".to_string(),
                children: vec![
                    child(
                        0,
                        ChildKind::Shell,
                        Some(4_000_001),
                        "Run the tests",
                        2 * MINUTE,
                    ),
                    session(TRUNK, "tool-based-ui-trunk", 21 * HOUR),
                    session(ARRANGE, "tool-based-ui-arrange", 3 * HOUR),
                ],
                ..agent(BOSS, "boss of bosses", 23 * HOUR, None)
            },
            AgentRow {
                branch: Some("enh/trunk".to_string()),
                directory: "~/rust/ui-trunk".to_string(),
                children: vec![
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
                        "cargo nextest run -p hana_video",
                        45,
                    ),
                    session(TRUNK_IMPL, "trunk-impl", 2 * HOUR),
                ],
                ..agent(TRUNK, "tool-based-ui-trunk", 21 * HOUR, Some(BOSS))
            },
            AgentRow {
                status: Some("idle".to_string()),
                branch: Some("enh/arrange".to_string()),
                directory: "~/rust/ui-arrange".to_string(),
                children: vec![child(
                    0,
                    ChildKind::Detached(Agent::Codex),
                    Some(468_060),
                    "app-server",
                    HOUR,
                )],
                ..agent(ARRANGE, "tool-based-ui-arrange", 3 * HOUR, Some(BOSS))
            },
            AgentRow {
                status: Some("idle".to_string()),
                directory: "~/scratch/impl".to_string(),
                ..agent(TRUNK_IMPL, "trunk-impl", 2 * HOUR, Some(TRUNK))
            },
        ])
    }

    /// The cells `natedev` answers with.
    fn natedev_cells(natedev: &MachineState) -> Vec<AgentEntry<'_>> {
        cell_order(&[Machine {
            name:  "natedev",
            state: natedev,
        }])
    }

    /// Where it all fits, a top-level agent's cell hangs the tree of the
    /// sessions it opened from its header and its branch and directory:
    /// a line to each session, giving its status, branch, directory and
    /// age, with `—` for a session outside a repository. Its table
    /// nests what each session runs under the row naming the session.
    #[test]
    fn a_cell_draws_its_sessions_as_a_tree_and_nests_what_they_run() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        assert_eq!(cells.len(), 1, "the sessions share their agent's cell");

        let buffer = drawn_among(&cells[0], &cells, WIDE);

        assert_eq!(
            lines(&buffer),
            [
                " pid 1579022 · claude · busy · 23h · natedev · —",
                " main · ~/rust/cargo-liner",
                " ├─ tool-based-ui-trunk    busy  enh/trunk    ~/rust/ui-trunk    21h",
                " │  └─ trunk-impl          idle  —            ~/scratch/impl     2h",
                " └─ tool-based-ui-arrange  idle  enh/arrange  ~/rust/ui-arrange  3h",
                "",
                " pid      via         runs     name                             age",
                " 4000001  shell       command  Run the tests                    2m",
                " 3266367  session     claude   tool-based-ui-trunk              21h",
                " —          subagent  claude   Review the permission queue      5m 3s",
                " 2424763      shell   command  cargo nextest run -p hana_video  45s",
                " 3400000    session   claude   trunk-impl                       2h",
                " 3337048  session     claude   tool-based-ui-arrange            3h",
                " 468060     detached  codex    app-server                       1h",
            ]
        );
        let text = text_default();
        assert_eq!(buffer[(1, 2)].fg, text, "a branch of the tree");
        assert_eq!(buffer[(1, 3)].fg, text, "a line carried down");
        assert_eq!(buffer[(1, 4)].fg, text, "the last branch");
    }

    /// In a cell too narrow for its tree's lines, each session stands as
    /// an entry: its name after its glyphs, `<status> · <age>`, then its
    /// branch and directory, the tree's lines carried down past it. A
    /// stacked child nested too deep for its facts' one line breaks it
    /// before the fact that would be cut.
    #[test]
    fn a_narrow_cell_stacks_its_tree() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);

        let buffer = drawn_among(&cells[0], &cells, NARROW);

        assert_eq!(
            lines(&buffer),
            [
                " agent    claude",
                " pid      1579022",
                " status   busy",
                " age      23h",
                " machine  natedev",
                " desktop  —",
                " main · ~/rust/cargo-liner",
                " ├─ tool-based-ui-trunk",
                " │  busy · 21h",
                " │  enh/trunk · ~/rust/ui-trunk",
                " │  └─ trunk-impl",
                " │     idle · 2h",
                " │     ~/scratch/impl",
                " └─ tool-based-ui-arrange",
                "    idle · 3h",
                "    enh/arrange · ~/rust/ui-arrange",
                "",
                " shell · command · 2m · pid 4000001",
                "   Run the tests",
                " session · claude · 21h · pid 3266367",
                "   tool-based-ui-trunk",
                "   subagent · claude · 5m 3s",
                "     Review the permission queue",
                "     shell · command · 45s",
                "     pid 2424763",
                "       cargo nextest run -p hana_video",
                "   session · claude · 2h · pid 3400000",
                "     trunk-impl",
                " session · claude · 3h · pid 3337048",
                "   tool-based-ui-arrange",
                "   detached · codex · 1h · pid 468060",
                "     app-server",
            ]
        );
    }

    /// Every line of `buffer` holding `text`, at the first cell it
    /// starts at on that line.
    fn find_all(buffer: &Buffer, text: &str) -> Vec<(u16, u16)> {
        lines(buffer)
            .iter()
            .enumerate()
            .filter_map(|(y, line)| {
                line.find(text).map(|at| {
                    let x = line[..at].chars().count();
                    (
                        u16::try_from(x).expect("the column should fit a u16"),
                        u16::try_from(y).expect("the row should fit a u16"),
                    )
                })
            })
            .collect()
    }

    /// An agent's name is drawn in its hue wherever a cell shows it: a
    /// session's in the tree and in the table, stacked or not, and a
    /// listed launcher's after `launched by`. A name that is no agent's
    /// keeps the default text color.
    #[test]
    fn an_agents_name_takes_its_hue_wherever_it_shows() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let hue_of = |cells: &[AgentEntry<'_>], pid| name_style(cells, "natedev", pid).fg;
        for width in [WIDE, NARROW] {
            let buffer = drawn_among(&cells[0], &cells, width);

            for (name, pid) in [
                ("tool-based-ui-trunk", TRUNK),
                ("trunk-impl", TRUNK_IMPL),
                ("tool-based-ui-arrange", ARRANGE),
            ] {
                let found = find_all(&buffer, name);
                assert_eq!(
                    found.len(),
                    2,
                    "{name} in the tree and the table at {width}"
                );
                for at in found {
                    assert_eq!(
                        Some(buffer[at].fg),
                        hue_of(&cells, pid),
                        "{name} at {at:?}, {width} across"
                    );
                }
            }
            let shell = find(&buffer, "Run the tests").expect("the shell is drawn");
            assert_eq!(buffer[shell].fg, text_default(), "at {width}");
        }

        let (natedev, _) = natedev_and_mac();
        let cells = natedev_cells(&natedev);
        let left = cells
            .iter()
            .find(|cell| cell.row.name == "left")
            .expect("left should have a cell");
        let buffer = drawn_among(left, &cells, WIDE);
        let (x, y) = find(&buffer, "launched by right").expect("the launcher is drawn");
        let right = (x + summary::cell_width("launched by "), y);
        assert_eq!(Some(buffer[right].fg), hue_of(&cells, 3_700_000));
    }

    /// The blank cells between the facts of a compressed line.
    fn gap_text() -> String { " ".repeat(usize::from(TABLE_COLUMN_SPACING)) }

    /// In fewer rows than its full view takes, a top-level agent's cell
    /// draws no header and no branch and directory, as the summary lists
    /// the agent: a line to each session -- its glyphs, name, status and
    /// age -- then a line to each child -- what it runs and its name,
    /// indented by level -- each cut where it would not fit, every name
    /// and status in the color the full view gives it.
    #[test]
    fn a_top_level_cell_short_of_rows_compresses() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];
        let rows =
            boss.sessions.len() + usize::from(AGENT_HEADER_GAP_HEIGHT) + group_children(boss).len();
        assert!(
            rows < height(boss, CUTTING, NOW),
            "the full view takes more"
        );

        let buffer = drawn_into(boss, &cells, CUTTING, rows);

        let gap = gap_text();
        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&buffer),
            [
                format!(" {TREE_BRANCH}tool-based-ui-trunk{gap}busy{mark}"),
                format!(" {TREE_RAIL}{TREE_LAST_BRANCH}trunk-impl{gap}idle{gap}2h"),
                format!(" {TREE_LAST_BRANCH}tool-based-ui-arrange{gap}id{mark}"),
                String::new(),
                format!(" command{gap}Run the tests"),
                format!(" claude{gap}tool-based-ui-trunk"),
                format!("   claude{gap}Review the permiss{mark}"),
                format!("     command{gap}cargo nextest r{mark}"),
                format!("   claude{gap}trunk-impl"),
                format!(" claude{gap}tool-based-ui-arrange"),
                format!("   codex{gap}app-server"),
            ]
        );
        for (name, pid) in [
            ("tool-based-ui-trunk", TRUNK),
            ("trunk-impl", TRUNK_IMPL),
            ("tool-based-ui-arrange", ARRANGE),
        ] {
            let found = find_all(&buffer, name);
            assert_eq!(found.len(), 2, "{name} in the tree and the table");
            for at in found {
                assert_eq!(
                    buffer[at].fg,
                    name_style(&cells, "natedev", pid).fg.expect("a hue"),
                    "{name} at {at:?}"
                );
            }
        }
        let fg = |text| buffer[find(&buffer, text).expect("the text is drawn")].fg;
        assert_eq!(Some(fg("busy")), Role::Busy.style().fg);
        assert_eq!(Some(fg("idle")), Role::Idle.style().fg);
        assert_eq!(Some(fg("command")), Role::Shell.style().fg);
        assert_eq!(Some(fg("claude")), Role::Claude.style().fg);
        assert_eq!(Some(fg("codex")), Role::Codex.style().fg);
        assert_eq!(fg("Run the tests"), text_default());
    }

    /// A cell draws its full view in the rows [`height`] asks for, its
    /// header and its branch and directory among them, and its
    /// compressed view one row short of them.
    #[test]
    fn a_cell_given_its_height_draws_its_full_view() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];
        let rows = height(boss, CUTTING, NOW);
        let header = format!("{SECTION_HEADER_INDENT}{HEADER_AGENT_LABEL}");
        let workplace = workplace(boss.row);

        let full = lines(&drawn_into(boss, &cells, CUTTING, rows));
        let short = lines(&drawn_into(boss, &cells, CUTTING, rows - 1));

        assert!(full[0].starts_with(&header), "{full:#?}");
        assert!(
            full.iter().any(|line| line.contains(&workplace)),
            "{full:#?}"
        );
        assert!(!short[0].starts_with(&header), "{short:#?}");
        assert!(
            !short.iter().any(|line| line.contains(&workplace)),
            "{short:#?}"
        );
    }

    /// A launched session whose launcher is not listed is left out of the
    /// summary, so its compressed view keeps the lines its full view
    /// opens with -- its header, its directory and the agent that
    /// opened it -- above a line to each child.
    #[test]
    fn a_launched_cell_keeps_its_header_when_compressed() {
        let trunk = trunk();
        let trunk_entry = entry(&trunk, None);
        let full = lines(&drawn_among(&trunk_entry, &[], WIDE));

        let compressed = lines(&drawn_into(
            &trunk_entry,
            &[],
            WIDE,
            height(&trunk_entry, WIDE, NOW) - 1,
        ));

        assert!(full[2].contains(LAUNCHED_BY_LABEL), "{full:#?}");
        assert_eq!(compressed[..3], full[..3]);
        let gap = gap_text();
        assert_eq!(
            compressed[3..],
            [
                String::new(),
                format!(" codex{gap}app-server"),
                format!(" command{gap}Launch the Phase 1 implementation seat"),
                format!("   codex{gap}app-server"),
                format!("     codex{gap}tool-based-ui-trunk-impl"),
                format!(" claude{gap}Review the permission queue"),
                format!(
                    "   command{gap}cargo nextest run -p hana_video --no-fail-fast -- permission_queue"
                ),
                format!(" claude{gap}tool-based-ui-arrange"),
            ]
        );
    }

    /// A compressed top-level agent with no sessions has nothing above
    /// its children, so its first row holds the first child, not a
    /// blank.
    #[test]
    fn a_compressed_cell_with_nothing_above_its_children_starts_with_them() {
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
        let arrange_entry = entry(&arrange, None);

        let compressed = lines(&drawn_into(
            &arrange_entry,
            &[],
            WIDE,
            arrange.children.len(),
        ));

        let gap = gap_text();
        assert_eq!(
            compressed,
            [
                format!(" command{gap}cargo nextest run -p hana_video"),
                format!(" claude{gap}Review the permission queue"),
            ]
        );
    }

    /// natedev's answer and the mac's: two top-level agents, the older
    /// opening no session and the other the sessions under it, one under
    /// another, then a session whose launcher is not listed and two rows
    /// naming only each other, then the mac's one agent.
    fn natedev_and_mac() -> (MachineState, MachineState) {
        let natedev = MachineState::Answered(vec![
            agent(428_044, "enh/handler", 24 * HOUR, None),
            agent(BOSS, "boss of bosses", 23 * HOUR, None),
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

    /// natedev's and the mac's cells.
    fn natedev_and_mac_cells<'a>(
        natedev: &'a MachineState,
        mac: &'a MachineState,
    ) -> Vec<AgentEntry<'a>> {
        cell_order(&[
            Machine {
                name:  "natedev",
                state: natedev,
            },
            Machine {
                name:  "mac",
                state: mac,
            },
        ])
    }

    /// A cell as the order test reads it: its machine, its agent, the
    /// agent's launcher, and each session's glyphs and name.
    type ReadCell<'a> = (&'a str, &'a str, Option<&'a str>, Vec<(String, &'a str)>);

    /// Machine by machine, each top-level agent has a cell, those that
    /// opened sessions before the rest and each oldest first, holding
    /// the sessions it opened depth first, each led by the glyphs that
    /// hang it from its launcher. A session whose
    /// launcher is not listed has a cell of its own, and of two rows
    /// naming only each other the first has one holding the other.
    #[test]
    fn each_top_level_agent_holds_the_sessions_it_opened() {
        let (natedev, mac) = natedev_and_mac();

        let cells = natedev_and_mac_cells(&natedev, &mac);

        let order: Vec<ReadCell<'_>> = cells
            .iter()
            .map(|cell| {
                (
                    cell.id.machine.as_str(),
                    cell.row.name.as_str(),
                    cell.launcher,
                    cell.sessions
                        .iter()
                        .map(|session| (session.lead(), session.row.name.as_str()))
                        .collect(),
                )
            })
            .collect();
        let hung = |lead: &str, name| (lead.to_string(), name);
        assert_eq!(
            order,
            [
                (
                    "natedev",
                    "boss of bosses",
                    None,
                    vec![
                        hung("├─ ", "trunk"),
                        hung("│  └─ ", "under trunk"),
                        hung("└─ ", "arrange"),
                    ]
                ),
                ("natedev", "enh/handler", None, vec![]),
                ("natedev", "orphan", None, vec![]),
                ("natedev", "left", Some("right"), vec![hung("└─ ", "right")]),
                ("mac", "natemccoy-30", None, vec![]),
            ]
        );
        assert_eq!(
            cells[0].id,
            AgentCell {
                machine: "natedev".to_string(),
                pid:     BOSS,
                started: NOW - 23 * HOUR,
            }
        );
    }

    /// Every agent takes the next hue of the rainbow in cell order, a
    /// session as much as a top-level agent, carrying on from one
    /// machine to the next and starting over at red past violet.
    #[test]
    fn each_agent_takes_the_next_hue_of_the_rainbow() {
        let (natedev, mac) = natedev_and_mac();

        let hues: Vec<(&str, RainbowHue)> = natedev_and_mac_cells(&natedev, &mac)
            .iter()
            .flat_map(|cell| {
                iter::once((cell.row.name.as_str(), cell.hue)).chain(
                    cell.sessions
                        .iter()
                        .map(|session| (session.row.name.as_str(), session.hue)),
                )
            })
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
