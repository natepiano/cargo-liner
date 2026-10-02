//! One cell for each top-level agent and the sessions it opened in
//! tmux: a header naming the agent, its branch and directory, the agent
//! that opened it where one did, the tree of its sessions, then what
//! they all run, each child indented under the one that started it and
//! each session's own children nested under the row that names it.
//!
//! [`cell_order`] lays the cells out across the machines, and
//! [`height`] and [`draw`] fill one in.
//!
//! A cell draws its full view only where it lies flat at the cell's
//! width -- the header on its one line, the branch and directory on
//! theirs, a line to each session in the tree and a table holding every
//! name whole, as [`FlatColumns::fitted`] finds -- and the grid gives
//! the cell the rows that view takes. Otherwise the cell draws its
//! compressed view: one line to each session in the tree -- its name
//! and status -- and to each child in the table -- what it runs and its
//! name -- cut where it would not fit, and for a top-level agent, which
//! the summary lists, no header and no branch and directory. [`height`]
//! asks the grid for the rows of the view the cell draws at its width.
//! Every agent's name is drawn in its own hue wherever it shows.

use std::collections::HashSet;
use std::iter;
use std::time::Duration;

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
use crate::census::Timer;
use crate::constants::AGENT_HEADER_GAP_HEIGHT;
use crate::constants::CHILD_AGE_COLUMN;
use crate::constants::CHILD_HEADERS;
use crate::constants::CHILD_NAME_COLUMN;
use crate::constants::CHILD_PID_COLUMN;
use crate::constants::CHILD_RUNS_COLUMN;
use crate::constants::CHILD_VIA_COLUMN;
use crate::constants::CHILD_VIA_INDENT;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::LAUNCHED_BY_LABEL;
use crate::constants::MISSING_VALUE;
use crate::constants::NOTHING_RUNNING_HEIGHT;
use crate::constants::NOTHING_RUNNING_NOTE;
use crate::constants::PID_LABEL;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TIMER_PIE;
use crate::constants::TIMER_PIE_FULL;
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
    /// Every part on its lines whole -- the header on its one line, the
    /// branch and directory on theirs, a line to each session and a row
    /// of the table to each child -- the tree and the table in the
    /// [`FlatColumns`] found for the width.
    Full(FlatColumns),
    /// One line to each session and to each child, cut where it would
    /// not fit, and no header or branch and directory for an agent the
    /// summary lists.
    Compressed,
}

impl CellView {
    /// The view `entry`'s cell takes `width` cells across, with ages
    /// measured to `now`, given the rows that view takes: full where
    /// [`FlatColumns::fitted`] finds it lies flat there, else compressed.
    fn at_width(entry: &AgentEntry<'_>, width: u16, now: u64) -> Self {
        FlatColumns::fitted(entry, width, now).map_or(Self::Compressed, Self::Full)
    }

    /// The view `entry`'s cell takes in `area`, with ages measured to
    /// `now`: [`CellView::at_width`] at the area's width, or compressed
    /// where the area has fewer rows than that view takes.
    fn fitted(entry: &AgentEntry<'_>, area: Rect, now: u64) -> Self {
        let view = Self::at_width(entry, area.width, now);
        if view_height(entry, area.width, now, view) > usize::from(area.height) {
            Self::Compressed
        } else {
            view
        }
    }
}

/// The columns a cell's full view draws its tree and its table in, each
/// fitted to its widest cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FlatColumns {
    /// The tree's: a session's name after its glyphs, its status,
    /// branch, directory and age.
    tree:  [Constraint; TREE_COLUMNS],
    /// The table's: a child's pid, `via`, `runs`, name and age.
    table: [Constraint; 5],
}

impl FlatColumns {
    /// The columns of `entry`'s full view `width` cells across, with ages
    /// measured to `now`, where that view lies flat there: its
    /// [`header_line`] and its [`place_line`] fit whole, [`tree_columns`]
    /// fits every session's line and [`table_columns`] every name. None
    /// where any of them would be cut.
    fn fitted(entry: &AgentEntry<'_>, width: u16, now: u64) -> Option<Self> {
        let row = entry.row;
        let lines = [header_line(row, entry.machine, now), place_line(row)];
        if lines.iter().any(|line| line.width() > usize::from(width)) {
            return None;
        }
        Some(Self {
            tree:  tree_columns(&entry.sessions, header_width(width), now)?,
            table: table_columns(&group_children(entry), children_width(width), now)?,
        })
    }
}

/// Rows `entry`'s cell asks the grid for at `width` cells across, with
/// ages measured to `now`: the rows of the view [`CellView::at_width`]
/// gives it there -- its full view where that lies flat, else its
/// compressed view -- exactly the rows [`draw`] fills given them.
pub(crate) fn height(entry: &AgentEntry<'_>, width: u16, now: u64) -> usize {
    view_height(entry, width, now, CellView::at_width(entry, width, now))
}

/// Rows `entry`'s cell takes `width` cells across in `view`, with ages
/// measured to `now`, counted from the parts [`draw_view`] draws: the
/// lines [`above_tree`] gives it, down to [`children_top`], then the
/// rows of its [`Children`]. The styles `cells` would set do not change
/// how many lines [`above_tree`] gives, so none are passed.
fn view_height(entry: &AgentEntry<'_>, width: u16, now: u64, view: CellView) -> usize {
    let above = above_tree(entry, &[], width, now, view).len();
    let children = group_children(entry);
    children_top(entry, above) + Children::in_view(&children, view).height(children.len())
}

/// Rows from the top of `entry`'s cell to what its agent runs, below
/// `above` lines above its tree: those lines, a line to each session,
/// then [`AGENT_HEADER_GAP_HEIGHT`] blank rows where either drew one, so
/// a compressed top-level agent with no sessions starts with what it
/// runs.
fn children_top(entry: &AgentEntry<'_>, above: usize) -> usize {
    let drawn = above + entry.sessions.len();
    if drawn == 0 {
        0
    } else {
        drawn + usize::from(AGENT_HEADER_GAP_HEIGHT)
    }
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
/// seconds, in the view [`CellView::fitted`] gives it there. A listed
/// agent's name among `cells` -- the launcher's, a session's -- is drawn
/// in that agent's hue.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    now: u64,
) {
    draw_view(
        buffer,
        area,
        entry,
        cells,
        now,
        CellView::fitted(entry, area, now),
    );
}

/// Draw `entry`'s cell into `area` in `view`, with ages measured to
/// `now`: the lines [`above_tree`] gives it, the tree of its sessions,
/// then what they all run, each as `view` lays it out, cut at the bottom
/// where `area` is too short.
fn draw_view(
    buffer: &mut Buffer,
    area: Rect,
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    now: u64,
    view: CellView,
) {
    let above = above_tree(entry, cells, area.width, now, view);
    let above_rows = above.len();
    let above_height = u16::try_from(above_rows).unwrap_or(u16::MAX);
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

    let skipped = u16::try_from(children_top(entry, above_rows)).unwrap_or(u16::MAX);
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
    match Children::in_view(&children, view) {
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
                .map(|child| compressed_child(child, area.width, child_name_style(child), now))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
        Children::Table(constraints) => Table::new(
            children
                .iter()
                .map(|child| child_row(child, child_name_style(child), now)),
            constraints,
        )
        .header(Row::new(CHILD_HEADERS.map(|header| {
            Span::styled(header, Style::default().fg(label_color()))
        })))
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(area, buffer),
    }
}

/// The lines `entry`'s cell draws above its tree in `view`, `width`
/// cells across: its [`header_line`], its [`place_line`], and the agent
/// that opened it when one did, each one line cut by [`cut_line`] where
/// it would not fit. The compressed view of a top-level agent draws none
/// of them, as the summary lists that agent.
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
    let mut lines = vec![header_line(row, entry.machine, now), place_line(row)];
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
        .into_iter()
        .map(|line| cut_line(line.spans, width))
        .collect()
}

/// The header on one line: `pid <pid> · <agent> · <status> · <age> ·
/// <machine> · <desktop>`, colored as the summary colors the same
/// values. No part of it takes the label color: `pid` reads as part of
/// its value.
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

/// Where `row` works, as one value: `<branch> · <directory>`, or the
/// directory alone outside a repository.
fn workplace(row: &AgentRow) -> String {
    row.branch.as_ref().map_or_else(
        || row.directory.clone(),
        |branch| format!("{branch}{HEADING_SEPARATOR}{}", row.directory),
    )
}

/// The agent's branch and directory on one line, as [`workplace`] gives
/// them.
fn place_line(row: &AgentRow) -> Line<'static> {
    Line::from(vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(workplace(row), Style::default().fg(text_default())),
    ])
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

/// The tree's column widths within `width` cells, with ages measured to
/// `now`, or none where a session's line would not fit whole.
///
/// Every column fits its widest cell, [`TABLE_COLUMN_SPACING`] from the
/// next.
fn tree_columns(
    sessions: &[SessionEntry<'_>],
    width: u16,
    now: u64,
) -> Option<[Constraint; TREE_COLUMNS]> {
    let mut widths = [0; TREE_COLUMNS];
    for session in sessions {
        for (widest, cell) in widths.iter_mut().zip(tree_cells(session, now)) {
            *widest = (*widest).max(cell.chars().count());
        }
    }
    let spacing = usize::from(TABLE_COLUMN_SPACING) * (TREE_COLUMNS - 1);
    let needed = widths.iter().sum::<usize>() + spacing;
    (needed <= usize::from(width))
        .then(|| widths.map(|widest| Constraint::Length(u16::try_from(widest).unwrap_or(u16::MAX))))
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

/// Draw the tree of `sessions` into `area` in `view`, with ages
/// measured to `now`: in the full view a line to each session in the
/// tree's columns, and in the compressed view a line to each cut where
/// it would not fit. The glyphs are plain text and each name takes its
/// session's hue.
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
    match view {
        CellView::Full(columns) => Table::new(
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
            columns.tree,
        )
        .column_spacing(TABLE_COLUMN_SPACING)
        .render(area, buffer),
        CellView::Compressed => {
            let lines: Vec<Line<'static>> = sessions
                .iter()
                .map(|session| compressed_session(session, area.width))
                .collect();
            Paragraph::new(lines).render(area, buffer);
        },
    }
}

/// One session's line in the compressed view, `width` cells across: its
/// name after its glyphs, drawn in its hue, then [`gap`] and its status
/// in its role, as the tree's table spaces its columns, cut by
/// [`cut_line`] where it would not fit.
fn compressed_session(session: &SessionEntry<'_>, width: u16) -> Line<'static> {
    let row = session.row;
    cut_line(
        vec![
            Span::styled(session.lead(), Style::default().fg(text_default())),
            Span::styled(row.name.clone(), Role::Rainbow(session.hue).style()),
            gap(),
            Span::styled(
                summary::status_text(row).to_string(),
                summary::status_role(row).style(),
            ),
        ],
        width,
    )
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

/// How an agent cell draws what the agent runs, in one view.
enum Children {
    /// The agent runs nothing, and a note says so.
    Nothing,
    /// The full view's table, its columns fitted to their widest cells.
    Table([Constraint; 5]),
    /// The compressed view's line to each child: what it runs, then its
    /// name, indented by level and cut where it would not fit.
    Compressed,
}

impl Children {
    /// The way `children` are drawn in `view`: the note where there are
    /// none, else the view's table or lines.
    const fn in_view(children: &[ChildRow], view: CellView) -> Self {
        if children.is_empty() {
            return Self::Nothing;
        }
        match view {
            CellView::Full(columns) => Self::Table(columns.table),
            CellView::Compressed => Self::Compressed,
        }
    }

    /// Rows `count` children take drawn this way.
    fn height(&self, count: usize) -> usize {
        match self {
            Self::Nothing => usize::from(NOTHING_RUNNING_HEIGHT),
            Self::Table(_) => usize::from(TABLE_HEADER_HEIGHT) + count,
            Self::Compressed => count,
        }
    }
}

/// One child's line in the compressed view, `width` cells across:
/// indented as the table indents its `via`, what it `runs` in the role
/// the table draws it in, then [`gap`] and its [`child_name`] at `now`
/// in `name_style`, cut by [`cut_line`] where it would not fit.
fn compressed_child(child: &ChildRow, width: u16, name_style: Style, now: u64) -> Line<'static> {
    let text = Style::default().fg(text_default());
    cut_line(
        vec![
            Span::styled(depth_indent(child), text),
            Span::styled(child.runs_label(), runs_role(child.kind).style()),
            gap(),
            Span::styled(child_name(child, now), name_style),
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

/// One row of the table at `now`, its [`child_name`] drawn in
/// `name_style`.
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
        Span::styled(child_name(child, now), name_style),
        Span::styled(age::age_label(now.saturating_sub(child.started)), text),
    ])
}

/// What a row's `name` cell says at `now`: a timer's [`timer_label`],
/// else the row's name.
fn child_name(child: &ChildRow, now: u64) -> String {
    child
        .timer
        .map_or_else(|| child.name.clone(), |timer| timer_label(timer, now))
}

/// A timer row's name at `now`: its [`timer_pie`], then the time left
/// as [`time_left_label`] writes it.
fn timer_label(timer: Timer, now: u64) -> String {
    format!(
        "{} {}",
        timer_pie(timer, now),
        time_left_label(timer.deadline.saturating_sub(now))
    )
}

/// The glyph of [`TIMER_PIE`] for the share of `timer` gone at `now`,
/// and [`TIMER_PIE_FULL`] at or past its deadline.
fn timer_pie(timer: Timer, now: u64) -> char {
    let gone = Duration::from_secs(now.saturating_sub(timer.started));
    let span = Duration::from_secs(timer.deadline.saturating_sub(timer.started));
    // A timer of no length divides to NaN or infinity, under no bound.
    let share = gone.div_duration_f64(span);
    TIMER_PIE
        .iter()
        .find(|&&(bound, _)| share < bound)
        .map_or(TIMER_PIE_FULL, |&(_, pie)| pie)
}

/// `left` seconds on a timer, as `m:ss` under an hour and `h:mm:ss`
/// from an hour up.
fn time_left_label(left: u64) -> String {
    let hours = left / 3_600;
    let minutes = left / 60 % 60;
    let seconds = left % 60;
    if hours == 0 {
        format!("{minutes}:{seconds:02}")
    } else {
        format!("{hours}:{minutes:02}:{seconds:02}")
    }
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
/// a name, and the cell draws its compressed view instead.
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
        widths.observe_cell_usize(CHILD_NAME_COLUMN, child_name(child, now).chars().count());
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
            timer: None,
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

    /// An area `width` cells across and `rows` rows down.
    fn area(width: u16, rows: usize) -> Rect {
        Rect::new(
            0,
            0,
            width,
            u16::try_from(rows).expect("the rows should fit a u16"),
        )
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across into
    /// `rows` rows in the view [`CellView::fitted`] gives it there.
    fn drawn_into(
        entry: &AgentEntry<'_>,
        cells: &[AgentEntry<'_>],
        width: u16,
        rows: usize,
    ) -> Buffer {
        let area = area(width, rows);
        let mut buffer = Buffer::empty(area);
        draw(&mut buffer, area, entry, cells, NOW);
        buffer
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across into
    /// exactly the rows [`height`] asks for there.
    fn drawn_among(entry: &AgentEntry<'_>, cells: &[AgentEntry<'_>], width: u16) -> Buffer {
        drawn_into(entry, cells, width, height(entry, width, NOW))
    }

    /// `row`'s cell, opened by the agent named `launcher`, drawn `width`
    /// cells across into the rows [`height`] asks for there, with no
    /// other cell on screen.
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

    /// A cell draws its full view at the width that holds its header's
    /// line whole, and one cell narrower, where that line would be cut,
    /// its compressed view: for a top-level agent, which the summary
    /// lists, no header and no branch and directory.
    #[test]
    fn a_cell_compresses_where_its_header_would_be_cut() {
        let boss = agent(BOSS, "boss of bosses", 2 * 24 * HOUR, None);
        let line = u16::try_from(header_line(&boss, "natedev", NOW).width())
            .expect("the line should fit a u16");

        let whole = drawn(&boss, None, line);
        let cut = drawn(&boss, None, line - 1);

        assert_eq!(
            lines(&whole),
            [
                " pid 1579022 · claude · busy · 2d · natedev · —",
                " ~/rust/boss of bosses",
                "",
                " nothing running",
            ]
        );
        assert_eq!(lines(&cut), [" nothing running"]);
    }

    /// A cell draws its full view at the width that holds its branch and
    /// directory whole, and one cell narrower its compressed view: a
    /// launched session keeps the lines above its tree, the directory cut
    /// to its one line, then a line to each child.
    #[test]
    fn a_cell_compresses_where_its_directory_would_be_broken() {
        let arrange = AgentRow {
            directory: "~/rust/hana_catalyst/crates/hana_video/src/permission_queue".to_string(),
            children: vec![child(
                0,
                ChildKind::Shell,
                Some(2_424_763),
                "cargo nextest run",
                45,
            )],
            ..agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS))
        };
        let line = u16::try_from(place_line(&arrange).width()).expect("the line should fit a u16");
        assert!(
            usize::from(line) > header_line(&arrange, "natedev", NOW).width(),
            "the directory's line is the cell's widest"
        );

        let whole = drawn(&arrange, None, line);
        let cut = drawn(&arrange, None, line - 1);

        assert_eq!(
            lines(&whole),
            [
                " pid 3337048 · claude · busy · 2h · natedev · —",
                " ~/rust/hana_catalyst/crates/hana_video/src/permission_queue",
                " launched by pid 1579022",
                "",
                " pid      via    runs     name               age",
                " 2424763  shell  command  cargo nextest run  45s",
            ]
        );
        let gap = gap_text();
        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&cut),
            [
                " pid 3337048 · claude · busy · 2h · natedev · —".to_string(),
                format!(" ~/rust/hana_catalyst/crates/hana_video/src/permission_que{mark}"),
                " launched by pid 1579022".to_string(),
                String::new(),
                format!(" command{gap}cargo nextest run"),
            ]
        );
    }

    /// A cell draws what its agent runs as a table at the width that
    /// holds every name whole, and one cell narrower, where the table
    /// would cut a name, its compressed view: a line to each child, what
    /// it runs in its role, then its name.
    #[test]
    fn a_cell_compresses_where_its_table_would_cut_a_name() {
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
        let cut = drawn(&arrange, None, table - 1);

        assert_eq!(
            lines(&fits)[3..],
            [
                " pid      via       runs     name                             age",
                " 2424763  shell     command  cargo nextest run -p hana_video  45s",
                " —        subagent  claude   Review the permission queue      5m 3s",
            ]
        );
        let gap = gap_text();
        assert_eq!(
            lines(&cut),
            [
                format!(" command{gap}cargo nextest run -p hana_video"),
                format!(" claude{gap}Review the permission queue"),
            ]
        );
        assert_eq!(Some(cut[(1, 0)].fg), Role::Shell.style().fg);
        assert_eq!(Some(cut[(1, 1)].fg), Role::Claude.style().fg);
        assert_eq!(cut[(10, 0)].fg, text_default(), "a name");
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
    /// reads as part of the values beside it: in either view at any
    /// width, every run of it is `launched by` or a heading of the
    /// table, and no value, `pid` marker, note, separator or line of a
    /// cell's tree takes it.
    #[test]
    fn only_labels_take_the_label_color() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let idle = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));
        let natedev = boss_group();
        let group = natedev_cells(&natedev);
        let labels: HashSet<&str> = iter::once(LAUNCHED_BY_LABEL).chain(CHILD_HEADERS).collect();
        let entries = [
            entry(&trunk, Some("boss of bosses")),
            entry(&idle, Some("boss of bosses")),
            group[0].clone(),
        ];
        let mut drawn = HashSet::new();
        for entry in &entries {
            for width in [NARROW, WIDE] {
                let buffer = drawn_among(entry, &group, width);

                for run in runs_in(&buffer, label_color()) {
                    assert!(
                        labels.contains(run.as_str()),
                        "{run:?} takes the label color at width {width}"
                    );
                    drawn.insert(run);
                }
            }
        }
        let every_label: HashSet<String> = labels.into_iter().map(String::from).collect();
        assert_eq!(drawn, every_label, "every label is drawn");
    }

    /// Whatever the width, the rows [`height`] counts are exactly the
    /// rows [`draw`] fills given more: the last of them holds text, and
    /// none past it do.
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

                let drawn = lines(&drawn_into(entry, &group, width, rows + 3));
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
    /// session's in the tree and in the table, in either view, and a
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

    /// Too narrow to lie flat, a top-level agent's cell draws no header
    /// and no branch and directory, as the summary lists the agent: a
    /// line to each session -- its glyphs, name and status -- then a line
    /// to each child -- what it runs and its name, indented by level --
    /// each cut where it would not fit, every name and status in the
    /// color the full view gives it.
    #[test]
    fn a_compressed_top_level_cell_cuts_each_line_to_its_width() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];

        let buffer = drawn_into(boss, &cells, CUTTING, height(boss, CUTTING, NOW));

        let gap = gap_text();
        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&buffer),
            [
                format!(" {TREE_BRANCH}tool-based-ui-trunk{gap}busy"),
                format!(" {TREE_RAIL}{TREE_LAST_BRANCH}trunk-impl{gap}idle"),
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

    /// A wide cell, where every part of its full view lies flat, asks
    /// for exactly the rows of that view: given more, it draws its
    /// one-line header and its branch and directory, and its last line
    /// of text on the last of the rows [`height`] asked for. One row
    /// short of them, it draws its compressed view.
    #[test]
    fn a_wide_cell_given_its_height_draws_its_full_view() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];
        assert!(
            matches!(CellView::at_width(boss, WIDE, NOW), CellView::Full(_)),
            "boss lies flat {WIDE} across"
        );
        let rows = height(boss, WIDE, NOW);
        let header = format!("{SECTION_HEADER_INDENT}{PID_LABEL} {BOSS}");
        let workplace = workplace(boss.row);

        let full = lines(&drawn_into(boss, &cells, WIDE, rows + 1));
        let short = lines(&drawn_into(boss, &cells, WIDE, rows - 1));

        assert_eq!(
            full.iter()
                .rposition(|line| !line.is_empty())
                .map(|last| last + 1),
            Some(rows),
            "{full:#?}"
        );
        assert_eq!(full[..rows], lines(&drawn_among(boss, &cells, WIDE)));
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

    /// A narrow cell, where its header's line would be cut, draws its
    /// compressed view even given every row its full view takes at a
    /// wide width: a line to each session -- its glyphs, name and status,
    /// and no age -- then a line to each child -- what it runs and its
    /// name, and no pid, `via` or age.
    #[test]
    fn a_narrow_cell_compresses_however_many_rows_it_has() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];
        assert!(
            header_line(boss.row, boss.machine, NOW).width() > usize::from(NARROW),
            "the header's line would be cut {NARROW} across"
        );

        let drawn = lines(&drawn_into(boss, &cells, NARROW, height(boss, WIDE, NOW)));

        let gap = gap_text();
        let mark = TRUNCATION_MARK;
        let compressed = [
            format!(" {TREE_BRANCH}tool-based-ui-trunk{gap}busy"),
            format!(" {TREE_RAIL}{TREE_LAST_BRANCH}trunk-impl{gap}idle"),
            format!(" {TREE_LAST_BRANCH}tool-based-ui-arrange{gap}idle"),
            String::new(),
            format!(" command{gap}Run the tests"),
            format!(" claude{gap}tool-based-ui-trunk"),
            format!("   claude{gap}Review the permission queue"),
            format!("     command{gap}cargo nextest run -p ha{mark}"),
            format!("   claude{gap}trunk-impl"),
            format!(" claude{gap}tool-based-ui-arrange"),
            format!("   codex{gap}app-server"),
        ];
        assert_eq!(drawn[..compressed.len()], compressed);
        assert!(
            drawn[compressed.len()..].iter().all(String::is_empty),
            "{drawn:#?}"
        );
        let tree = &drawn[..boss.sessions.len()];
        for session in &boss.sessions {
            let age = age::age_label(NOW - session.row.started);
            assert!(
                !tree.iter().any(|line| line.contains(&age)),
                "{} shows its age {age}: {tree:#?}",
                session.row.name
            );
        }
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

    /// A cell too narrow to lie flat asks for the rows of its compressed
    /// view: drawn into more, it holds text down to the last of the rows
    /// [`height`] asked for and none past it, and those rows are the ones
    /// [`view_height`] counts for that view.
    #[test]
    fn a_narrow_cell_asks_for_the_rows_of_its_compressed_view() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let entries = [cells[0].clone(), entry(&trunk, Some("boss of bosses"))];
        for entry in &entries {
            let name = &entry.row.name;
            assert_eq!(
                CellView::at_width(entry, NARROW, NOW),
                CellView::Compressed,
                "{name} lies flat {NARROW} across"
            );
            let rows = height(entry, NARROW, NOW);

            let drawn = lines(&drawn_into(entry, &cells, NARROW, rows + 1));

            assert_eq!(
                rows,
                view_height(entry, NARROW, NOW, CellView::Compressed),
                "{name}"
            );
            assert_eq!(
                drawn
                    .iter()
                    .rposition(|line| !line.is_empty())
                    .map(|last| last + 1),
                Some(rows),
                "{name} at {NARROW} cells: {drawn:#?}"
            );
        }
    }

    /// A launched session too narrow to lie flat keeps its header in its
    /// compressed view as one line cut with the truncation mark, not a
    /// block of labelled facts, above its branch and directory and the
    /// agent that opened it, each cut to its one line.
    #[test]
    fn a_narrow_launched_cell_cuts_its_header_to_one_line() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        assert_eq!(
            CellView::at_width(&entry(&trunk, Some("boss of bosses")), NARROW, NOW),
            CellView::Compressed
        );

        let buffer = drawn(&trunk, Some("boss of bosses"), NARROW);

        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&buffer)[..4],
            [
                format!(" pid 3266367 · claude · busy · 23h · {mark}"),
                format!(" ~/rust/tool-based-ui-geometry-materi{mark}"),
                " launched by boss of bosses".to_string(),
                String::new(),
            ]
        );
        assert_eq!(runs_in(&buffer, label_color()), [LAUNCHED_BY_LABEL]);
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

    /// A timer's pie takes the next glyph at each quarter of it gone,
    /// and the full one at its deadline and past it.
    #[test]
    fn a_timers_pie_fills_by_the_quarter() {
        let timer = Timer {
            started:  NOW,
            deadline: NOW + 400,
        };
        let pie_at = |gone: u64| timer_pie(timer, NOW + gone);

        assert_eq!(pie_at(0), TIMER_PIE[0].1);
        assert_eq!(pie_at(99), TIMER_PIE[0].1);
        assert_eq!(pie_at(100), TIMER_PIE[1].1);
        assert_eq!(pie_at(199), TIMER_PIE[1].1);
        assert_eq!(pie_at(200), TIMER_PIE[2].1);
        assert_eq!(pie_at(299), TIMER_PIE[2].1);
        assert_eq!(pie_at(300), TIMER_PIE[3].1);
        assert_eq!(pie_at(399), TIMER_PIE[3].1);
        assert_eq!(pie_at(400), TIMER_PIE_FULL);
        assert_eq!(pie_at(500), TIMER_PIE_FULL);
    }

    /// The time left reads `m:ss` under an hour and `h:mm:ss` from an
    /// hour up, and a timer past its deadline reads `0:00`.
    #[test]
    fn time_left_reads_as_minutes_under_an_hour() {
        assert_eq!(time_left_label(0), "0:00");
        assert_eq!(time_left_label(9), "0:09");
        assert_eq!(time_left_label(228), "3:48");
        assert_eq!(time_left_label(HOUR - 1), "59:59");
        assert_eq!(time_left_label(HOUR), "1:00:00");
        assert_eq!(time_left_label(HOUR + MINUTE + 1), "1:01:01");
        assert_eq!(time_left_label(10 * HOUR), "10:00:00");
        let past = Timer {
            started:  NOW - 2 * MINUTE,
            deadline: NOW - MINUTE,
        };
        assert_eq!(timer_label(past, NOW), format!("{TIMER_PIE_FULL} 0:00"));
    }

    /// A shell row that is only a timer reads `timer` in its `runs`, in
    /// the color a command's takes, and its pie and the time left in
    /// place of its description, in the table and in the compressed
    /// view.
    #[test]
    fn a_timer_row_shows_its_pie_and_time_left() {
        let seat = AgentRow {
            children: vec![
                child(
                    0,
                    ChildKind::Shell,
                    Some(2_500_000),
                    "Run lint then the full suite",
                    4 * MINUTE,
                ),
                ChildRow {
                    timer: Some(Timer {
                        started:  NOW - 92,
                        deadline: NOW + 228,
                    }),
                    ..child(
                        0,
                        ChildKind::Shell,
                        Some(2_500_100),
                        "Re-arm the progress timer",
                        92,
                    )
                },
            ],
            ..agent(3_500_000, "timer-seat", HOUR, Some(BOSS))
        };

        let full = drawn(&seat, Some("boss of bosses"), WIDE);
        let compressed = drawn(&seat, Some("boss of bosses"), NARROW);

        assert_eq!(
            lines(&full)[4..],
            [
                " pid      via    runs     name                          age",
                " 2500000  shell  command  Run lint then the full suite  4m",
                " 2500100  shell  timer    ◔ 3:48                        1m 32s",
            ]
        );
        assert_eq!(
            runs_in(
                &full,
                Role::Shell.style().fg.expect("the shell role has a color")
            ),
            ["command", "timer"]
        );
        let compressed_lines = lines(&compressed);
        assert_eq!(
            compressed_lines[compressed_lines.len() - 2..],
            [" command  Run lint then the full suite", " timer  ◔ 3:48"]
        );
    }
}
