//! The summary cell: each machine's heading, then, when it lists
//! agents, a column-label row and its top-level agents, oldest first,
//! with a blank row between machines. Every machine's table is laid out
//! with the same column widths, so the columns line up down the cell.
//! Each agent's name is drawn in the hue of its cell, so a row and that
//! agent's cell can be paired at a glance.

pub(crate) mod age;

use std::cmp::Ordering;

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Flex;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
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

use self::age::age_label;
use crate::agent_cell;
use crate::agent_cell::AgentEntry;
use crate::census::Account;
use crate::census::AccountColumn;
use crate::census::Agent;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::Machine;
use crate::census::MachineState;
use crate::constants::AGENT_PLURAL;
use crate::constants::AGENT_SINGULAR;
use crate::constants::BUSY_STATUS;
use crate::constants::GROUP_GAP_HEIGHT;
use crate::constants::GROUP_HEADER_HEIGHT;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::MISSING_VALUE;
use crate::constants::NAME_COLUMN_MAX;
use crate::constants::NO_AGENTS_NOTE;
use crate::constants::SCANNING_NOTE;
use crate::constants::SHELL_STATUS;
use crate::constants::SUMMARY_HEADERS;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TRUNCATION_MARK;
use crate::footer_colors::FooterColors;
use crate::sort;
use crate::sort::HeaderSpan;
use crate::sort::SummaryColumn;
use crate::sort::SummarySort;
use crate::theme::Role;

/// Rows the summary draws for `machines`: each machine's heading and
/// table, and a gap between one machine and the next.
pub(crate) fn height(machines: &[Machine<'_>], placement: &AgentPlacement<'_>) -> usize {
    let groups: usize = machines
        .iter()
        .map(|machine| {
            usize::from(GROUP_HEADER_HEIGHT) + table_height(root_rows(*machine, placement).count())
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
pub(crate) fn width(
    machines: &[Machine<'_>],
    placement: &AgentPlacement<'_>,
    now: u64,
    account_column: AccountColumn,
    sort: SummarySort,
) -> u16 {
    let headings = machines
        .iter()
        .map(|machine| heading(machine, root_rows(*machine, placement).count()).width())
        .max()
        .map_or(0, |widest| u16::try_from(widest).unwrap_or(u16::MAX));
    if machines
        .iter()
        .all(|machine| root_rows(*machine, placement).next().is_none())
    {
        return headings;
    }
    let widths = column_widths(machines, placement, now, account_column, sort);
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

/// The rows titling cells on this machine, in the census's oldest-first order.
fn root_rows<'a>(
    machine: Machine<'a>,
    placement: &AgentPlacement<'_>,
) -> impl Iterator<Item = &'a AgentRow> {
    machine
        .state
        .rows()
        .iter()
        .filter(move |row| placement.is_root(machine.name, row.pid))
}

/// Shared column widths, account visibility and the footer palette.
struct SummaryColumns<'a> {
    footer_colors:  &'a FooterColors,
    constraints:    Vec<Constraint>,
    account_column: AccountColumn,
    sort:           SummarySort,
}

/// The census, palette and ordering shared throughout one summary draw.
pub(crate) struct SummaryFrame<'a, 'data> {
    pub(crate) machines:       &'a [Machine<'data>],
    pub(crate) placement:      &'a AgentPlacement<'data>,
    pub(crate) cells:          &'a [AgentEntry<'data>],
    pub(crate) now:            u64,
    pub(crate) account_column: AccountColumn,
    pub(crate) footer_colors:  &'a FooterColors,
    pub(crate) sort:           SummarySort,
}

/// Draw machine groups with the palette and ordering shared by this frame.
pub(crate) fn draw(buffer: &mut Buffer, area: Rect, frame: &SummaryFrame<'_, '_>) {
    let SummaryHeader {
        constraints,
        machines,
        ..
    } = summary_header(area, frame);
    let columns = SummaryColumns {
        footer_colors: frame.footer_colors,
        constraints,
        account_column: frame.account_column,
        sort: frame.sort,
    };
    for machine in machines {
        draw_machine(buffer, machine, frame.cells, &columns, frame.now);
    }
}

/// A visible machine group with its heading, table and rows resolved together.
struct MachineTable<'a> {
    /// Machine identity and scan state used by its heading.
    machine: Machine<'a>,
    /// The visible part of the machine heading.
    heading: Rect,
    /// The visible table, whose top row is its column header.
    table:   Rect,
    /// Root rows before applying the summary ordering.
    rows:    Vec<&'a AgentRow>,
}

/// Resolve vertical machine placement once for drawing and header clicks.
fn machine_tables<'a>(area: Rect, frame: &SummaryFrame<'_, 'a>) -> Vec<MachineTable<'a>> {
    let mut remaining = area;
    let mut tables = Vec::new();
    for &machine in frame.machines {
        if remaining.is_empty() {
            break;
        }
        let rows: Vec<_> = root_rows(machine, frame.placement).collect();
        let heading = Rect {
            height: GROUP_HEADER_HEIGHT.min(remaining.height),
            ..remaining
        };
        let drawn = remaining
            .height
            .saturating_sub(GROUP_HEADER_HEIGHT)
            .min(u16::try_from(table_height(rows.len())).unwrap_or(u16::MAX));
        let table = Rect {
            y: remaining.y.saturating_add(GROUP_HEADER_HEIGHT),
            height: drawn,
            ..indented(remaining)
        };
        let used = GROUP_HEADER_HEIGHT
            .saturating_add(drawn)
            .saturating_add(GROUP_GAP_HEIGHT);
        tables.push(MachineTable {
            machine,
            heading,
            table,
            rows,
        });
        remaining.y = remaining.y.saturating_add(used);
        remaining.height = remaining.height.saturating_sub(used);
    }
    tables
}

/// Draw one machine using the same resolved rectangles as header hit testing.
fn draw_machine(
    buffer: &mut Buffer,
    machine: MachineTable<'_>,
    cells: &[AgentEntry<'_>],
    columns: &SummaryColumns<'_>,
    now: u64,
) {
    let mut rows = machine.rows;
    rows.sort_by(|left, right| compare_rows(left, right, columns.sort, columns.account_column));
    let label = Style::default().fg(label_color());
    Paragraph::new(heading(&machine.machine, rows.len())).render(machine.heading, buffer);
    Table::new(
        rows.iter().map(|row| {
            agent_row(
                row,
                agent_cell::name_style(cells, machine.machine.name, row.pid),
                now,
                columns.account_column,
                columns.footer_colors,
            )
        }),
        columns.constraints.iter().copied(),
    )
    .header(Row::new(
        SummaryColumn::ALL.map(|column| Span::styled(columns.sort.header(column), label)),
    ))
    .column_spacing(TABLE_COLUMN_SPACING)
    .render(machine.table, buffer);
}

/// Compare values within one machine; ties keep the census's original order.
fn compare_rows(
    left: &AgentRow,
    right: &AgentRow,
    sort: SummarySort,
    accounts: AccountColumn,
) -> Ordering {
    let primary = match sort.column {
        SummaryColumn::Pid => left.pid.cmp(&right.pid),
        SummaryColumn::Agent => left.agent_label(accounts).cmp(&right.agent_label(accounts)),
        SummaryColumn::Name => left.name.cmp(&right.name),
        SummaryColumn::Status => status_text(left).cmp(status_text(right)),
        SummaryColumn::Age => right.started.cmp(&left.started),
        SummaryColumn::Desktop => desktop_text(left).cmp(desktop_text(right)),
        SummaryColumn::Directory => left.directory.cmp(&right.directory),
    };
    sort.direction
        .order(primary)
        .then_with(|| (left.started, left.pid).cmp(&(right.started, right.pid)))
}

/// Shared horizontal columns and vertical machine placement for the summary.
struct SummaryHeader<'a> {
    /// Column widths used by every machine table.
    constraints: Vec<Constraint>,
    /// The visible machine tables and their heading rectangles.
    machines:    Vec<MachineTable<'a>>,
    /// Header cells used to resolve clicks.
    spans:       Vec<HeaderSpan<SummaryColumn>>,
}

/// One column layout supplies both drawing and header hit testing.
fn summary_header<'a>(area: Rect, frame: &SummaryFrame<'_, 'a>) -> SummaryHeader<'a> {
    let constraints = fitted_constraints(
        frame.machines,
        frame.placement,
        frame.now,
        frame.account_column,
        frame.sort,
    );
    let machines = machine_tables(area, frame);
    let mut spans = Vec::new();
    for machine in &machines {
        if machine.rows.is_empty() || machine.table.is_empty() {
            continue;
        }
        let rects = Layout::horizontal(constraints.iter().copied())
            .spacing(TABLE_COLUMN_SPACING)
            .flex(Flex::Start)
            .split(machine.table);
        spans.extend(sort::column_spans(
            &SummaryColumn::ALL,
            &rects,
            machine.table.y,
        ));
    }
    SummaryHeader {
        constraints,
        machines,
        spans,
    }
}

/// Header spans for every visible machine table in summary-content coordinates.
pub(crate) fn header_spans(
    area: Rect,
    frame: &SummaryFrame<'_, '_>,
) -> Vec<HeaderSpan<SummaryColumn>> {
    summary_header(area, frame).spans
}

/// `machine`'s heading: its name in bold, then how many agents it
/// lists, that it is still being scanned, or why it could not be. The
/// note is plain text, never the label color the table's column
/// headers use.
fn heading<'a>(machine: &Machine<'a>, roots: usize) -> Line<'a> {
    let note = Style::default().fg(text_default());
    let mut heading = vec![
        Span::raw(SECTION_HEADER_INDENT),
        Span::styled(
            machine.name,
            Style::default()
                .fg(text_default())
                .add_modifier(Modifier::BOLD),
        ),
        separator(),
    ];
    match machine.state {
        MachineState::Scanning => heading.push(Span::styled(SCANNING_NOTE, note)),
        MachineState::Answered(_, _) => heading.push(Span::styled(count_note(roots), note)),
        MachineState::Failed(reason) => {
            heading.push(Span::styled(reason.as_str(), Role::Unreachable.style()));
        },
    }
    Line::from(heading)
}

/// The ` · ` between facts on one line, in plain text: only a label
/// takes the label color.
pub(crate) fn separator() -> Span<'static> {
    Span::styled(HEADING_SEPARATOR, Style::default().fg(text_default()))
}

/// What a heading says about a machine with `count` top-level agents.
fn count_note(count: usize) -> String {
    match count {
        0 => NO_AGENTS_NOTE.to_string(),
        1 => format!("1 {AGENT_SINGULAR}"),
        count => format!("{count} {AGENT_PLURAL}"),
    }
}

/// One agent's table row, its name drawn in `name_style`.
fn agent_row(
    row: &AgentRow,
    name_style: Style,
    now: u64,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) -> Row<'static> {
    let text = Style::default().fg(text_default());
    Row::new([
        Line::from(Span::styled(row.pid.to_string(), text)),
        agent_label(row, account_column, footer_colors),
        Line::from(Span::styled(
            truncated(&row.name, usize::from(NAME_COLUMN_MAX)),
            name_style,
        )),
        Line::from(Span::styled(
            status_text(row).to_string(),
            status_role(row).style(),
        )),
        Line::from(Span::styled(
            age_label(now.saturating_sub(row.started)),
            text,
        )),
        Line::from(Span::styled(desktop_text(row).to_string(), text)),
        Line::from(Span::styled(row.directory.clone(), text)),
    ])
}

/// An account label with its tier kept in the existing program style.
pub(crate) fn agent_label(
    row: &AgentRow,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) -> Line<'static> {
    let program_style = agent_role(row.agent).style();
    let label = row.agent_label(account_column);
    match (account_column, &row.account) {
        (AccountColumn::Shown, Account::Home(home)) => {
            let (account, tier) = label.split_at(home.label().len());
            Line::from(vec![
                Span::styled(
                    account.to_string(),
                    footer_colors.account(home.label()).style(program_style),
                ),
                Span::styled(tier.to_string(), program_style),
            ])
        },
        _ => Line::from(Span::styled(label, program_style)),
    }
}

/// The role an agent's program is drawn in.
const fn agent_role(agent: Agent) -> Role {
    match agent {
        Agent::Claude => Role::Claude,
        Agent::Codex | Agent::CodexThreadHost => Role::Codex,
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
fn fitted_constraints(
    machines: &[Machine<'_>],
    placement: &AgentPlacement<'_>,
    now: u64,
    account_column: AccountColumn,
    sort: SummarySort,
) -> Vec<Constraint> {
    column_widths(machines, placement, now, account_column, sort)
        .to_constraints()
        .into_iter()
        .enumerate()
        .map(|(column, constraint)| {
            if column == SummaryColumn::Directory.index() {
                Constraint::Min(cell_width(&sort.header(SummaryColumn::Directory)))
            } else {
                constraint
            }
        })
        .collect()
}

/// The widest cell in each column across every machine's rows, `name`
/// stopping at [`NAME_COLUMN_MAX`] and `directory` written out in full.
fn column_widths(
    machines: &[Machine<'_>],
    placement: &AgentPlacement<'_>,
    now: u64,
    account_column: AccountColumn,
    sort: SummarySort,
) -> ColumnWidths {
    let mut widths = ColumnWidths::new(
        SummaryColumn::ALL
            .into_iter()
            .map(|column| ColumnSpec {
                min: cell_width(&sort.header(column)),
                max: (column == SummaryColumn::Name).then_some(NAME_COLUMN_MAX),
            })
            .collect(),
    );
    for row in machines
        .iter()
        .flat_map(|machine| root_rows(*machine, placement))
    {
        widths.observe_cell_usize(
            SummaryColumn::Pid.index(),
            row.pid.to_string().chars().count(),
        );
        widths.observe_cell_usize(
            SummaryColumn::Agent.index(),
            row.agent_label(account_column).chars().count(),
        );
        widths.observe_cell_usize(SummaryColumn::Name.index(), row.name.chars().count());
        widths.observe_cell_usize(
            SummaryColumn::Status.index(),
            status_text(row).chars().count(),
        );
        let age = age_label(now.saturating_sub(row.started));
        widths.observe_cell_usize(SummaryColumn::Age.index(), age.chars().count());
        widths.observe_cell_usize(
            SummaryColumn::Desktop.index(),
            desktop_text(row).chars().count(),
        );
        widths.observe_cell_usize(
            SummaryColumn::Directory.index(),
            row.directory.chars().count(),
        );
    }
    widths
}

/// `area` indented one level, where every machine's label row and rows
/// sit, under headings at the outer level.
fn indented(area: Rect) -> Rect {
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
    use crate::census::Account;
    use crate::census::Census;
    use crate::census::CensusUpdate;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyRole;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;

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
            session_id: SessionIdentity::Unrecorded,
            agent,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            name: name.to_string(),
            status: status.map(str::to_string),
            started: NOW - age,
            process_started: NOW - age,
            pid,
            desktop: None,
            directory: directory.to_string(),
            branch: None,
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
        MachineState::Answered(
            vec![
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
            ],
            crate::census::HierarchyEdges::default(),
        )
    }

    /// A named Claude session with a stable hierarchy identity.
    fn session(pid: u32, id: &str) -> AgentRow {
        AgentRow {
            session_id: SessionIdentity::Claude(id.to_string()),
            ..row(pid, Agent::Claude, id, None, HOUR, "~")
        }
    }

    /// Management links emitted by the machine owning these members.
    fn links(pairs: &[(&str, &str)]) -> HierarchyEdges {
        HierarchyEdges(
            pairs
                .iter()
                .map(|(manager, member)| HierarchyEdge {
                    manager: (*manager).to_string(),
                    member:  (*member).to_string(),
                    role:    HierarchyRole::Unit,
                })
                .collect(),
        )
    }

    /// Check actual rendered table membership against both the expected
    /// roots and the grid's cell identities, including machine-qualified pids.
    fn assert_summary_roots(machines: &[Machine<'_>], expected: &[(&str, u32)]) -> Vec<String> {
        let placement = AgentPlacement::from_machines(machines);
        let cells = agent_cell::cell_order_placed(&placement);
        let area = Rect::new(
            0,
            0,
            width(
                machines,
                &placement,
                NOW,
                AccountColumn::Hidden,
                SummarySort::default(),
            ),
            u16::try_from(height(machines, &placement)).expect("the summary height should fit"),
        );
        let mut buffer = Buffer::empty(area);
        draw(
            &mut buffer,
            area,
            &SummaryFrame {
                machines,
                placement: &placement,
                cells: &cells,
                now: NOW,
                account_column: AccountColumn::Hidden,
                footer_colors: &FooterColors::default(),
                sort: SummarySort::default(),
            },
        );
        let drawn = lines(&buffer);
        let mut machine = "";
        let mut actual = Vec::new();
        for line in &drawn {
            if let Some((name, _)) = line.trim_start().split_once(HEADING_SEPARATOR) {
                machine = name;
            } else if let Some(pid) = line
                .split_whitespace()
                .next()
                .and_then(|text| text.parse::<u32>().ok())
            {
                actual.push((machine, pid));
            }
        }
        assert_eq!(actual, expected, "the rendered summary rows");
        let mut grid_roots: Vec<_> = cells
            .iter()
            .map(|cell| (cell.id.machine.as_str(), cell.id.pid))
            .collect();
        let mut expected_roots = expected.to_vec();
        grid_roots.sort_unstable();
        expected_roots.sort_unstable();
        assert_eq!(
            grid_roots, expected_roots,
            "the same roots title grid cells"
        );
        drawn
    }

    /// Managed showrunners and units disappear from the table, while a
    /// session whose launcher is missing stays visible with its own cell.
    #[test]
    fn the_summary_lists_cell_roots_including_orphans_but_no_managed_descendants() {
        let state = MachineState::Answered(
            vec![
                session(10, "showrunner"),
                session(20, "nested-showrunner"),
                session(30, "unit"),
                AgentRow {
                    launched_by: Some(999),
                    ..session(40, "orphan")
                },
            ],
            links(&[
                ("showrunner", "nested-showrunner"),
                ("nested-showrunner", "unit"),
            ]),
        );
        let machines = [Machine {
            name:  "natedev",
            state: &state,
        }];
        let drawn = assert_summary_roots(&machines, &[("natedev", 10), ("natedev", 40)]);

        assert_eq!(drawn[0], " natedev · 2 top-level agents");
        assert_eq!(drawn.len(), 4);
        assert!(!drawn.iter().any(|line| line.contains("nested-showrunner")));
    }

    /// A remote member with the same pid as a local root must not hide
    /// that root or produce a table under its own machine's heading.
    #[test]
    fn cross_machine_membership_uses_both_machine_and_pid() {
        let local = MachineState::Answered(
            vec![session(7, "showrunner"), session(9, "local-root")],
            HierarchyEdges::default(),
        );
        let remote =
            MachineState::Answered(vec![session(9, "unit")], links(&[("showrunner", "unit")]));
        let machines = [
            Machine {
                name:  "natedev",
                state: &local,
            },
            Machine {
                name:  "mac",
                state: &remote,
            },
        ];
        let drawn = assert_summary_roots(&machines, &[("natedev", 7), ("natedev", 9)]);

        assert_eq!(drawn[0], " natedev · 2 top-level agents");
        assert_eq!(drawn[5], " mac · no agents");
        assert_eq!(drawn.len(), 6);
    }

    /// An unresolved manager preserves a live launch relationship, and
    /// an unresolved launcher gives the member back its summary entry.
    #[test]
    fn unresolved_managers_fall_back_to_listed_launchers_or_visible_roots() {
        let state = MachineState::Answered(
            vec![
                session(1, "launcher"),
                AgentRow {
                    launched_by: Some(1),
                    ..session(2, "launched")
                },
                AgentRow {
                    launched_by: Some(999),
                    ..session(3, "orphan")
                },
            ],
            links(&[("missing", "launched"), ("missing", "orphan")]),
        );
        assert_summary_roots(
            &[Machine {
                name:  "natedev",
                state: &state,
            }],
            &[("natedev", 1), ("natedev", 3)],
        );
    }

    /// Classification shows both roots, completed links hide the member
    /// and its long directory, and absent links restore both table and sizing.
    #[test]
    fn completed_and_cleared_links_update_summary_membership_counts_and_width() {
        let manager = session(1, "showrunner");
        let member = AgentRow {
            directory: format!("~/{}", "long/".repeat(30)),
            ..session(2, "unit")
        };
        let manager_only = MachineState::Answered(vec![manager.clone()], HierarchyEdges::default());
        let machines = [Machine {
            name:  "natedev",
            state: &manager_only,
        }];
        let root_width = width(
            &machines,
            &AgentPlacement::from_machines(&machines),
            NOW,
            AccountColumn::Hidden,
            SummarySort::default(),
        );
        let mut census = Census::new("natedev".to_string());

        for (edges, expected, count) in [
            (
                HierarchyEdges::default(),
                vec![("natedev", 1), ("natedev", 2)],
                "2 top-level agents",
            ),
            (
                links(&[("showrunner", "unit")]),
                vec![("natedev", 1)],
                "1 top-level agent",
            ),
            (
                HierarchyEdges::default(),
                vec![("natedev", 1), ("natedev", 2)],
                "2 top-level agents",
            ),
        ] {
            census.apply(CensusUpdate::Local(
                vec![manager.clone(), member.clone()],
                edges,
            ));
            let machines = census.machines(&[]);
            let drawn = assert_summary_roots(&machines, &expected);
            assert_eq!(drawn[0], format!(" natedev · {count}"));
            let measured = width(
                &machines,
                &AgentPlacement::from_machines(&machines),
                NOW,
                AccountColumn::Hidden,
                SummarySort::default(),
            );
            if expected.len() == 1 {
                assert_eq!(
                    measured, root_width,
                    "a managed directory takes no summary width"
                );
            } else {
                assert!(
                    measured > root_width,
                    "a root's full directory contributes width"
                );
            }
        }
    }

    /// A remote manager may leave and return without any change to the
    /// local member's identity or links; its summary entry follows that liveness.
    #[test]
    fn a_remote_manager_leaving_and_returning_restores_and_hides_the_member() {
        let mut census = Census::new("natedev".to_string());
        census.apply(CensusUpdate::Local(
            vec![session(5, "unit")],
            links(&[("showrunner", "unit")]),
        ));
        let configured = ["mac".to_string()];
        assert_summary_roots(&census.machines(&configured), &[("natedev", 5)]);
        let manager =
            MachineState::Answered(vec![session(5, "showrunner")], HierarchyEdges::default());
        for (state, expected) in [
            (manager.clone(), vec![("mac", 5)]),
            (
                MachineState::Failed("unreachable".to_string()),
                vec![("natedev", 5)],
            ),
            (manager, vec![("mac", 5)]),
        ] {
            census.apply(CensusUpdate::Remote {
                host: "mac".to_string(),
                state,
            });
            assert_summary_roots(&census.machines(&configured), &expected);
        }
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
        let mac = MachineState::Answered(
            vec![row(
                12_055,
                Agent::Claude,
                "natemccoy-30",
                Some("idle"),
                23 * HOUR,
                "~",
            )],
            crate::census::HierarchyEdges::default(),
        );
        let studio = MachineState::Failed("unreachable".to_string());
        let pi = MachineState::Scanning;
        let machines = [
            ("natedev", &natedev),
            ("mac", &mac),
            ("studio", &studio),
            ("pi", &pi),
        ]
        .map(|(name, state)| Machine { name, state });
        let placement = AgentPlacement::from_machines(&machines);
        let height =
            u16::try_from(height(&machines, &placement)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);
        let cells = agent_cell::cell_order_placed(&placement);

        draw(
            &mut buffer,
            area,
            &SummaryFrame {
                machines:       &machines,
                placement:      &placement,
                cells:          &cells,
                now:            NOW,
                account_column: AccountColumn::Hidden,
                footer_colors:  &FooterColors::default(),
                sort:           SummarySort::default(),
            },
        );

        assert_eq!(
            lines(&buffer),
            [
                " natedev · 4 top-level agents",
                " pid      agent   name                                  status  age ▼   desktop        directory",
                " 1579022  claude  boss of bosses                        idle    21h     boss           ~/rust/hana_catalyst/docs/hana",
                " 2747564  claude  tmp cleanup then merge to berth and…  shell   2h 29m  berth_fix      ~/rust/cargo-handler",
                " 428044   claude  enh/handler                           busy    1h 36m  cargo handler  ~/rust/handler",
                " 4039085  codex   --model gpt-5                         —       12m     —              ~/rust/handler",
                "",
                " mac · 1 top-level agent",
                " pid      agent   name                                  status  age ▼   desktop        directory",
                " 12055    claude  natemccoy-30                          idle    23h     —              ~",
                "",
                " studio · unreachable",
                "",
                " pi · scanning",
            ]
        );
        assert_eq!(buffer[(1, 2)].fg, text_default());
        assert_eq!(buffer[(1, 0)].fg, text_default(), "the machine's name");
        assert!(
            buffer[(1, 0)].modifier.contains(Modifier::BOLD),
            "the machine's name is bold"
        );
        assert_eq!(buffer[(9, 0)].fg, text_default(), "the separator");
        assert_eq!(buffer[(11, 0)].fg, text_default(), "the count");
        assert_eq!(buffer[(7, 13)].fg, text_default(), "scanning");
        // Only the label rows take the header color: no heading, note,
        // or row shares it.
        let header = buffer[(1, 1)].fg;
        for y in [0, 2, 3, 4, 5, 7, 9, 11, 13] {
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

    /// A Codex agent's tier follows its program in the `agent` column,
    /// in the Codex color, and the column widens to hold it.
    #[test]
    fn a_codex_tier_follows_its_program() {
        let natedev = MachineState::Answered(
            vec![
                row(
                    428_044,
                    Agent::Claude,
                    "enh/handler",
                    Some("busy"),
                    HOUR,
                    "~/rust/handler",
                ),
                AgentRow {
                    service_tier: ServiceTier::Fast,
                    account: Account::Unrecorded,
                    ..row(
                        4_039_085,
                        Agent::Codex,
                        "--model gpt-5",
                        None,
                        12 * MINUTE,
                        "~/rust/handler",
                    )
                },
            ],
            crate::census::HierarchyEdges::default(),
        );
        let machines = [Machine {
            name:  "natedev",
            state: &natedev,
        }];
        let placement = AgentPlacement::from_machines(&machines);
        let height =
            u16::try_from(height(&machines, &placement)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);

        draw(
            &mut buffer,
            area,
            &SummaryFrame {
                machines:       &machines,
                placement:      &placement,
                cells:          &agent_cell::cell_order_placed(&placement),
                now:            NOW,
                account_column: AccountColumn::Hidden,
                footer_colors:  &FooterColors::default(),
                sort:           SummarySort::default(),
            },
        );

        assert_eq!(
            lines(&buffer),
            [
                " natedev · 2 top-level agents",
                " pid      agent       name           status  age ▼  desktop  directory",
                " 428044   claude      enh/handler    busy    1h     —        ~/rust/handler",
                " 4039085  codex fast  --model gpt-5  —       12m    —        ~/rust/handler",
            ]
        );
        for x in [10, 19] {
            assert_eq!(
                Some(buffer[(x, 3)].fg),
                Role::Codex.style().fg,
                "column {x}"
            );
        }
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
        let placement = AgentPlacement::from_machines(&machines);
        let fitted = width(
            &machines,
            &placement,
            NOW,
            AccountColumn::Hidden,
            SummarySort::default(),
        );
        let height =
            u16::try_from(height(&machines, &placement)).expect("the height should fit a u16");
        let area = Rect::new(0, 0, WIDTH, height);
        let mut buffer = Buffer::empty(area);
        draw(
            &mut buffer,
            area,
            &SummaryFrame {
                machines:       &machines,
                placement:      &placement,
                cells:          &agent_cell::cell_order_placed(&placement),
                now:            NOW,
                account_column: AccountColumn::Hidden,
                footer_colors:  &FooterColors::default(),
                sort:           SummarySort::default(),
            },
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
        let longer = MachineState::Answered(rows, crate::census::HierarchyEdges::default());
        let machines = [Machine {
            name:  "natedev",
            state: &longer,
        }];
        assert_eq!(
            usize::from(width(
                &machines,
                &AgentPlacement::from_machines(&machines),
                NOW,
                AccountColumn::Hidden,
                SummarySort::default()
            )),
            usize::from(fitted) + LONG_DIRECTORY.chars().count() - longest_before,
            "the table widens by what the long directory adds"
        );

        let quiet = MachineState::Answered(Vec::new(), crate::census::HierarchyEdges::default());
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
            usize::from(width(
                &machines,
                &AgentPlacement::from_machines(&machines),
                NOW,
                AccountColumn::Hidden,
                SummarySort::default()
            )),
            " studio · unreachable".chars().count(),
            "the wider of the two headings"
        );
    }
}

#[cfg(test)]
#[path = "footer_tests.rs"]
mod footer_tests;

#[cfg(test)]
#[path = "sort_tests.rs"]
mod sort_tests;
