//! Agent tiles draw one tree of members, managed threads and running work.
//! A label-colored header names each column. Full views keep the process
//! heading, workplace and every tree value whole; compressed views reserve
//! the fixed columns and truncate names. Heights count exactly these rows.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::iter;
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Constraint;
use ratatui::layout::Flex;
use ratatui::layout::Layout;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Cell;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Row;
use ratatui::widgets::Table;
use ratatui::widgets::Widget;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::label_color;
use tui_pane::text_default;

use crate::census::Account;
use crate::census::AccountColumn;
use crate::census::Agent;
use crate::census::AgentActivity;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::HierarchyRole;
use crate::census::Placement;
use crate::census::RoleLabel;
use crate::census::RowIdentity;
use crate::census::ServiceTier;
use crate::census::Timer;
use crate::census::worker_dispatch::DispatchLabel;
use crate::census::worker_dispatch::WorkerRoleInput;
use crate::census::worker_dispatch::WorkerRoleSource;
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
use crate::constants::TREE_ACCOUNT_HEADER;
use crate::constants::TREE_AGE_HEADER;
use crate::constants::TREE_BRANCH;
use crate::constants::TREE_BRANCH_HEADER;
use crate::constants::TREE_DIRECTORY_HEADER;
use crate::constants::TREE_LAST_BRANCH;
use crate::constants::TREE_RAIL;
use crate::constants::TREE_ROLE_HEADER;
use crate::constants::TREE_SESSION_HEADER;
use crate::constants::TREE_SPACE;
use crate::constants::TREE_STATUS_HEADER;
use crate::constants::TRUNCATION_MARK;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterRole;
use crate::footer_colors::FooterTint;
use crate::sort::HeaderSpan;
use crate::sort::SortColumn;
use crate::sort::SortDirection;
use crate::sort::TreeSort;
use crate::summary;
use crate::summary::age;
use crate::theme::RainbowHue;
use crate::theme::Role;
use crate::tiles::AgentCell;
use crate::tiles::TileKey;

/// A top-level agent and the tree of members and work drawn in its tile.
#[derive(Clone, Debug)]
pub(crate) struct AgentEntry<'a> {
    /// Stable grid identity of the root process.
    pub(crate) id:         AgentCell,
    /// The agent named by the tile title.
    pub(crate) row:        &'a AgentRow,
    /// The listed launcher, when its name is known.
    launcher:              Option<&'a str>,
    /// The root's machine heading.
    machine:               &'a str,
    /// The root's name color.
    pub(crate) hue:        RainbowHue,
    /// The accepted or inferred root role.
    pub(crate) role_label: RoleLabel,
    /// This tile's ordering, also used to measure its marked header.
    sort:                  TreeSort,
    /// Members and work, in the depth-first order the tile draws.
    lines:                 Vec<TreeLine<'a>>,
}

impl AgentEntry<'_> {
    /// The name color of a listed agent in this tile.
    fn hue_of(&self, machine: &str, pid: u32) -> Option<RainbowHue> {
        if self.machine == machine && self.row.pid == pid {
            return Some(self.hue);
        }
        self.lines
            .iter()
            .find(|line| {
                line.machine == machine
                    && line.row.pid == pid
                    && matches!(line.subject, LineSubject::Agent(_))
            })
            .map(|line| line.hue)
    }
}

/// Where a line runs relative to the tile's machine heading.
#[derive(Clone, Copy, Debug)]
enum SessionLocation {
    /// The heading already names the machine.
    SameMachine,
    /// The line includes its machine after the name.
    AnotherMachine,
}

/// The subject whose values a tree line draws.
#[derive(Clone, Copy, Debug)]
enum LineSubject<'a> {
    /// A listed agent opened or managed by a member of this tile.
    Agent(RoleLabel),
    /// A top-level session in the flat unassigned pool.
    UnassignedSession,
    /// A Codex thread held by a manager, with its stated responsibility.
    ManagedThread(&'a ChildRow, HierarchyRole),
    /// Work not represented by another member line.
    Work(Work<'a>),
}

/// One member or work item, with the rails leading to it in the tree.
#[derive(Clone, Debug)]
struct TreeLine<'a> {
    /// The responsibility of a launcher folded into this thread.
    dispatch_label: DispatchLabel<'a>,
    machine:        &'a str,
    location:       SessionLocation,
    row:            &'a AgentRow,
    hue:            RainbowHue,
    rails:          Vec<Rail>,
    branch:         Branch,
    subject:        LineSubject<'a>,
}

/// Whether an ancestor's vertical rail continues past this line.
#[derive(Clone, Copy, Debug)]
enum Rail {
    Continues,
    Ended,
}

impl Rail {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Continues => TREE_RAIL,
            Self::Ended => TREE_SPACE,
        }
    }
}

/// Whether this child has a later sibling.
#[derive(Clone, Copy, Debug)]
enum Branch {
    /// A session in the flat unassigned list.
    Flat,
    Middle,
    Last,
}

impl Branch {
    const fn glyph(self) -> &'static str {
        match self {
            Self::Flat => "",
            Self::Middle => TREE_BRANCH,
            Self::Last => TREE_LAST_BRANCH,
        }
    }

    const fn rail(self) -> Rail {
        match self {
            Self::Middle => Rail::Continues,
            Self::Flat | Self::Last => Rail::Ended,
        }
    }
}

/// The three kinds of work retained as tree lines.
#[derive(Clone, Copy, Debug)]
enum WorkKind {
    Shell,
    Subagent,
    Thread,
}

/// A work row paired with its classified role in the tree.
#[derive(Clone, Copy, Debug)]
struct Work<'a> {
    child: &'a ChildRow,
    kind:  WorkKind,
}

impl<'a> Work<'a> {
    const fn account(self) -> &'a Account {
        match self.kind {
            WorkKind::Thread => &self.child.account,
            WorkKind::Shell | WorkKind::Subagent => &Account::Unrecorded,
        }
    }

    fn status(self, now: u64) -> String {
        match self.kind {
            WorkKind::Shell => self.child.timer.map_or_else(
                || self.child.runs_label().into_owned(),
                |timer| timer_label(timer, now),
            ),
            WorkKind::Subagent => self.child.runs_label().into_owned(),
            WorkKind::Thread => self.child.activity.text().to_string(),
        }
    }
}

/// How a process child participates in the visible work tree.
enum ChildLine {
    Work(WorkKind),
    Carrier,
    Session,
}

impl From<ChildKind> for ChildLine {
    fn from(kind: ChildKind) -> Self {
        match kind {
            ChildKind::Shell => Self::Work(WorkKind::Shell),
            ChildKind::Subagent => Self::Work(WorkKind::Subagent),
            ChildKind::Thread => Self::Work(WorkKind::Thread),
            ChildKind::Direct(_) | ChildKind::UnderShell(_) | ChildKind::Detached(_) => {
                Self::Carrier
            },
            ChildKind::Session(_) => Self::Session,
        }
    }
}

/// A retained work item and the retained descendants nearest to it.
struct WorkBranch<'a> {
    work:     Work<'a>,
    branches: Vec<Self>,
}

/// A column in the single tree table.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TreeColumn {
    Session,
    Role,
    Status,
    Account,
    Branch,
    Directory,
    Age,
}

impl TreeColumn {
    /// The label above this column.
    const fn header(self) -> &'static str {
        match self {
            Self::Session => TREE_SESSION_HEADER,
            Self::Role => TREE_ROLE_HEADER,
            Self::Status => TREE_STATUS_HEADER,
            Self::Account => TREE_ACCOUNT_HEADER,
            Self::Branch => TREE_BRANCH_HEADER,
            Self::Directory => TREE_DIRECTORY_HEADER,
            Self::Age => TREE_AGE_HEADER,
        }
    }

    /// The header with this table's selected direction marker.
    fn sorted_header(self, sort: TreeSort) -> String { sort.header(self) }

    /// The marked header, aligned with this tile's session rows.
    fn sorted_header_text(self, sort: TreeSort, lead: SessionHeaderLead) -> String {
        if self == Self::Session {
            format!(
                "{}{}",
                " ".repeat(usize::from(lead.width())),
                self.sorted_header(sort)
            )
        } else {
            self.sorted_header(sort)
        }
    }
}

impl SortColumn for TreeColumn {
    const DEFAULT: Self = Self::Age;

    fn natural_direction(self) -> SortDirection {
        match self {
            Self::Age => SortDirection::Descending,
            _ => SortDirection::Ascending,
        }
    }

    fn label(self) -> &'static str { self.header() }
}

/// Alignment of the session label above hierarchical or flat roots.
#[derive(Clone, Copy, Debug)]
enum SessionHeaderLead {
    /// Leave room above a tree row's branch glyph.
    OverBranch,
    /// Align with a flat root's name.
    Flush,
}

impl SessionHeaderLead {
    /// Cells reserved before the session header text.
    fn width(self) -> u16 {
        match self {
            Self::OverBranch => summary::cell_width(TREE_BRANCH),
            Self::Flush => 0,
        }
    }
}

/// The visible responsibility shared by role text, footer tint and name style.
enum LineKind {
    /// A root shown in the flat unassigned pool.
    UnassignedSession,
    /// A listed session with its accepted or inferred hierarchy role.
    Session(RoleLabel),
    /// A Codex worker resolved from hierarchy, roster, then seat tokens.
    Worker(WorkerRoleSource),
    /// An ordinary shell keeps plain text and a missing role.
    Shell,
    /// An ordinary subagent keeps plain text.
    Subagent,
}

/// The layout whose columns are selected for an account decision.
enum TreeLayout {
    Full,
    Compressed,
}

impl TreeLayout {
    const fn columns(self, account_column: AccountColumn) -> &'static [TreeColumn] {
        match (self, account_column) {
            (Self::Full, AccountColumn::Shown) => FULL_ACCOUNTS,
            (Self::Full, AccountColumn::Hidden) => FULL,
            (Self::Compressed, AccountColumn::Shown) => COMPRESSED_ACCOUNTS,
            (Self::Compressed, AccountColumn::Hidden) => COMPRESSED,
        }
    }
}

impl TreeLine<'_> {
    /// The name, with its machine when it differs from the heading.
    fn name(&self) -> String {
        let original = match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => &self.row.name,
            LineSubject::ManagedThread(child, _) => &child.name,
            LineSubject::Work(work) => &work.child.name,
        };
        let name = match self.dispatch_label {
            DispatchLabel::Original => match self.subject {
                LineSubject::ManagedThread(child, role) => child.worker_task.label(
                    WorkerRoleSource::resolve(WorkerRoleInput::Hierarchy(role)),
                    original,
                ),
                LineSubject::Work(work) if matches!(work.kind, WorkKind::Thread) => {
                    work.child.worker_task.label(
                        WorkerRoleSource::resolve(WorkerRoleInput::Dispatch(self.dispatch_label)),
                        original,
                    )
                },
                _ => original.clone(),
            },
            DispatchLabel::Worker(dispatch) => dispatch.label(),
        };
        match self.location {
            SessionLocation::SameMachine => name,
            SessionLocation::AnotherMachine => format!("{name}{HEADING_SEPARATOR}{}", self.machine),
        }
    }

    /// Tree rails and the branch preceding this name.
    fn lead(&self) -> String {
        self.rails
            .iter()
            .map(|&rail| rail.glyph())
            .chain(iter::once(self.branch.glyph()))
            .collect()
    }

    /// The start of this subject, rather than its carrier process.
    const fn started(&self) -> u64 {
        match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => self.row.started,
            LineSubject::ManagedThread(child, _) => child.started,
            LineSubject::Work(work) => work.child.started,
        }
    }

    /// The account belongs to an agent or thread, never its shell or subagent.
    const fn account(&self) -> &Account {
        match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => &self.row.account,
            LineSubject::ManagedThread(child, _) => &child.account,
            LineSubject::Work(work) => work.account(),
        }
    }

    /// The tier follows the account rather than a thread's activity.
    const fn service_tier(&self) -> ServiceTier {
        match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => self.row.service_tier,
            LineSubject::ManagedThread(child, _)
            | LineSubject::Work(Work {
                child,
                kind: WorkKind::Thread,
            }) => child.service_tier,
            LineSubject::Work(_) => ServiceTier::Unrecorded,
        }
    }

    /// The program whose account this line names.
    const fn program(&self) -> Agent {
        match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => self.row.agent,
            LineSubject::ManagedThread(_, _) | LineSubject::Work(_) => Agent::Codex,
        }
    }

    /// Classify a visible row once for all of its presentation decisions.
    fn kind(&self) -> LineKind {
        match self.subject {
            LineSubject::UnassignedSession => LineKind::UnassignedSession,
            LineSubject::Agent(role) => LineKind::Session(role),
            LineSubject::ManagedThread(_, role) => {
                LineKind::Worker(WorkerRoleSource::resolve(WorkerRoleInput::Hierarchy(role)))
            },
            LineSubject::Work(Work {
                kind: WorkKind::Thread,
                ..
            }) => LineKind::Worker(WorkerRoleSource::resolve(WorkerRoleInput::Dispatch(
                self.dispatch_label,
            ))),
            LineSubject::Work(Work {
                kind: WorkKind::Shell,
                ..
            }) => LineKind::Shell,
            LineSubject::Work(Work {
                kind: WorkKind::Subagent,
                ..
            }) => LineKind::Subagent,
        }
    }

    /// Worker seats show their responsibility, or the generic worker label.
    fn role_text(&self) -> &'static str {
        match self.kind() {
            LineKind::Worker(source) => source.text(),
            LineKind::UnassignedSession | LineKind::Shell => MISSING_VALUE,
            LineKind::Session(role) => role.column_text(),
            LineKind::Subagent => "subagent",
        }
    }

    /// Every Codex worker inherits its parent session's hue.
    fn name_style(&self) -> Style {
        match self.kind() {
            LineKind::Shell | LineKind::Subagent => Style::default().fg(text_default()),
            LineKind::UnassignedSession | LineKind::Session(_) | LineKind::Worker(_) => {
                Role::Rainbow(self.hue).style()
            },
        }
    }

    /// Compare one selected value, preserving the default order on equal values.
    fn compare(&self, other: &Self, sort: TreeSort, now: u64) -> Ordering {
        let primary = match sort.column {
            // A larger age is an earlier start.
            TreeColumn::Age => other.started().cmp(&self.started()),
            TreeColumn::Session => self.name().cmp(&other.name()),
            TreeColumn::Account => {
                account_sort_text(self.account()).cmp(account_sort_text(other.account()))
            },
            column => self.value(column, now).cmp(&other.value(column, now)),
        };
        sort.direction
            .order(primary)
            .then_with(|| self.order_key().cmp(&other.order_key()))
    }

    /// The pre-sort ordering used to break equal selected values.
    fn order_key(&self) -> (u64, &str, u32) {
        line_order(
            RowIdentity {
                machine: self.machine,
                pid:     self.row.pid,
            },
            self.row,
            self.subject,
        )
    }

    /// Text in one active column.
    fn value(&self, column: TreeColumn, now: u64) -> String {
        match column {
            TreeColumn::Session => format!("{}{}", self.lead(), self.name()),
            TreeColumn::Role => self.role_text().to_string(),
            TreeColumn::Status => match self.subject {
                LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                    summary::status_text(self.row).to_string()
                },
                LineSubject::ManagedThread(child, _) => child.activity.text().to_string(),
                LineSubject::Work(work) => work.status(now),
            },
            TreeColumn::Account => match self.account() {
                Account::Unrecorded => MISSING_VALUE.to_string(),
                Account::Home(home) => self
                    .service_tier()
                    .mark(home.label().to_owned())
                    .into_owned(),
            },
            TreeColumn::Branch | TreeColumn::Directory
                if matches!(self.subject, LineSubject::Work(_)) =>
            {
                MISSING_VALUE.to_string()
            },
            TreeColumn::Branch => self
                .row
                .branch
                .clone()
                .unwrap_or_else(|| MISSING_VALUE.to_string()),
            TreeColumn::Directory => self.row.directory.clone(),
            TreeColumn::Age => age::age_label(now.saturating_sub(self.started())),
        }
    }

    /// The status color of the subject.
    const fn status_role(&self) -> Role {
        match self.subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                summary::status_role(self.row)
            },
            LineSubject::ManagedThread(child, _)
            | LineSubject::Work(Work {
                child,
                kind: WorkKind::Thread,
            }) => thread_status_role(child.activity),
            LineSubject::Work(work) => runs_role(work.kind),
        }
    }
}

/// Account labels sort without their decorative service-tier mark.
fn account_sort_text(account: &Account) -> &str {
    match account {
        Account::Home(home) => home.label(),
        Account::Unrecorded => MISSING_VALUE,
    }
}

/// The footer responsibility for each typed management role.
impl From<HierarchyRole> for FooterRole {
    fn from(role: HierarchyRole) -> Self {
        match role {
            HierarchyRole::Showrunner => Self::Showrunner,
            HierarchyRole::Unit => Self::UnitDirector,
            HierarchyRole::Impl
            | HierarchyRole::Test
            | HierarchyRole::Review
            | HierarchyRole::Fix => Self::Worker,
        }
    }
}

impl TreeLine<'_> {
    /// Only management roles and sessions use the footer's role colors.
    fn footer_role(&self, colors: &FooterColors) -> FooterTint {
        match self.kind() {
            LineKind::UnassignedSession | LineKind::Session(RoleLabel::Unlabeled) => {
                colors.role(FooterRole::Unassigned)
            },
            LineKind::Session(RoleLabel::Stated(role) | RoleLabel::Inferred(role))
            | LineKind::Worker(WorkerRoleSource::Hierarchy(role)) => colors.role(role.into()),
            LineKind::Worker(_) => colors.role(FooterRole::Worker),
            LineKind::Shell | LineKind::Subagent => FooterTint::Plain,
        }
    }
}

/// Decide once for all cells, including cells the grid cannot place.
pub(crate) fn account_column(entries: &[AgentEntry<'_>]) -> AccountColumn {
    AccountColumn::of(entries.iter().flat_map(|entry| {
        iter::once((entry.row.agent, &entry.row.account)).chain(
            entry
                .lines
                .iter()
                .map(|line| (line.program(), line.account())),
        )
    }))
}

/// Promote work through carrier processes and discard complete session subtrees.
fn work_branches<'a>(
    children: &'a [ChildRow],
    claimed: &HashSet<usize>,
    offset: usize,
) -> Vec<WorkBranch<'a>> {
    let mut branches = Vec::new();
    let mut index = 0;
    while index < children.len() {
        let child = &children[index];
        let end = (index + 1..children.len())
            .find(|&next| children[next].depth <= child.depth)
            .unwrap_or(children.len());
        let descendants = || work_branches(&children[index + 1..end], claimed, offset + index + 1);
        match ChildLine::from(child.kind) {
            ChildLine::Session => {},
            ChildLine::Carrier => branches.extend(descendants()),
            ChildLine::Work(kind) => {
                let descendants = descendants();
                if claimed.contains(&(offset + index)) {
                    branches.extend(descendants);
                } else {
                    branches.push(WorkBranch {
                        work:     Work { child, kind },
                        branches: descendants,
                    });
                }
            },
        }
        index = end;
    }
    branches.sort_by_key(|branch| (branch.work.child.started, branch.work.child.pid));
    branches
}

/// Cells from the placement shared with the summary.
pub(crate) fn cell_order_placed<'a>(placement: &AgentPlacement<'a>) -> Vec<AgentEntry<'a>> {
    let mut cells = Vec::new();
    let mut agents = 0;
    for root in placement.roots() {
        let machine = root.identity.machine;
        let hue = RainbowHue::of_agent(agents);
        agents += 1;
        let mut lines = Vec::new();
        let context = TreeBuild {
            placement,
            heading: machine,
            order: SiblingOrder::Default,
            hues: HueAssignment::DefaultOrder,
        };
        lines_of(
            &context,
            root.identity,
            &mut Vec::new(),
            &mut agents,
            hue,
            &mut lines,
        );
        let launcher = root.row.launched_by.and_then(|pid| {
            placement
                .rows
                .iter()
                .find(|row| row.identity.machine == machine && row.identity.pid == pid)
                .map(|row| row.row.name.as_str())
        });
        cells.push(AgentEntry {
            id: AgentCell {
                machine:         machine.to_string(),
                pid:             root.row.pid,
                process_started: root.row.process_started,
            },
            row: root.row,
            launcher,
            machine,
            hue,
            role_label: placement.role_label(root.identity),
            sort: TreeSort::default(),
            lines,
        });
    }
    cells
}

/// A production-role root or the flat pool for one machine.
#[derive(Clone, Debug)]
pub(crate) enum AgentTile<'a> {
    /// A session with its own tile.
    Agent(AgentEntry<'a>),
    /// Sessions without a production role, with their work below each row.
    Unassigned(UnassignedEntry<'a>),
}

impl AgentTile<'_> {
    /// Stable identity independent of the current sessions in a pool.
    pub(crate) fn key(&self) -> TileKey {
        match self {
            Self::Agent(entry) => TileKey::Agent(entry.id.clone()),
            Self::Unassigned(entry) => TileKey::Unassigned {
                machine: entry.machine.to_owned(),
            },
        }
    }
}

/// A machine's unassigned sessions, each retaining its original hue.
#[derive(Clone, Debug)]
pub(crate) struct UnassignedEntry<'a> {
    /// The machine shared by all roots in this pool.
    pub(crate) machine: &'a str,
    /// Ordering shared by the flat roots and each nested sibling group.
    sort:               TreeSort,
    /// Flat roots followed by the existing depth-first trees below them.
    lines:              Vec<TreeLine<'a>>,
}

/// Classify live tile identities without building or ordering their rows.
pub(crate) fn tile_keys(placement: &AgentPlacement<'_>) -> HashSet<TileKey> {
    placement
        .rows
        .iter()
        .filter(|row| row.placement == Placement::Root)
        .map(|root| match placement.role_label(root.identity) {
            RoleLabel::Unlabeled => TileKey::Unassigned {
                machine: root.identity.machine.to_owned(),
            },
            RoleLabel::Stated(_) | RoleLabel::Inferred(_) => TileKey::Agent(AgentCell {
                machine:         root.identity.machine.to_owned(),
                pid:             root.row.pid,
                process_started: root.row.process_started,
            }),
        })
        .collect()
}

/// Build tiles from a census with hues assigned before grouping or sorting.
#[cfg(test)]
pub(crate) fn tile_order_sorted<'a>(
    placement: &AgentPlacement<'a>,
    sorts: &HashMap<TileKey, TreeSort>,
    now: u64,
) -> Vec<AgentTile<'a>> {
    let agents = cell_order_placed(placement);
    group_tiles(placement, &agents, sorts, now)
}

/// Group already-colored sessions without changing the summary's entries.
pub(crate) fn group_tiles<'a>(
    placement: &AgentPlacement<'a>,
    agents: &[AgentEntry<'a>],
    sorts: &HashMap<TileKey, TreeSort>,
    now: u64,
) -> Vec<AgentTile<'a>> {
    let session_sorts = agents
        .iter()
        .map(|entry| {
            let key = if entry.role_label == RoleLabel::Unlabeled {
                TileKey::Unassigned {
                    machine: entry.machine.to_owned(),
                }
            } else {
                TileKey::Agent(entry.id.clone())
            };
            (
                entry.id.clone(),
                sorts.get(&key).copied().unwrap_or_default(),
            )
        })
        .collect();
    let mut ordered = agents.to_vec();
    sort_cells(placement, &mut ordered, &session_sorts, now);
    let mut tiles = Vec::new();
    let mut pooled_machines = HashSet::new();
    for entry in &ordered {
        match entry.role_label {
            RoleLabel::Stated(_) | RoleLabel::Inferred(_) => {
                tiles.push(AgentTile::Agent(entry.clone()));
            },
            RoleLabel::Unlabeled if pooled_machines.insert(entry.machine) => {
                let mut roots: Vec<_> = ordered
                    .iter()
                    .filter(|root| {
                        root.machine == entry.machine && root.role_label == RoleLabel::Unlabeled
                    })
                    .collect();
                roots.sort_by(|left, right| {
                    unassigned_line(left).compare(&unassigned_line(right), entry.sort, now)
                });
                let lines = roots
                    .into_iter()
                    .flat_map(|root| {
                        iter::once(unassigned_line(root)).chain(root.lines.iter().cloned())
                    })
                    .collect();
                tiles.push(AgentTile::Unassigned(UnassignedEntry {
                    machine: entry.machine,
                    sort: entry.sort,
                    lines,
                }));
            },
            RoleLabel::Unlabeled => {},
        }
    }
    tiles
}

/// A flat root retains its own session values and preassigned hue.
const fn unassigned_line<'a>(entry: &AgentEntry<'a>) -> TreeLine<'a> {
    TreeLine {
        dispatch_label: DispatchLabel::Original,
        machine:        entry.machine,
        location:       SessionLocation::SameMachine,
        row:            entry.row,
        hue:            entry.hue,
        rails:          Vec::new(),
        branch:         Branch::Flat,
        subject:        LineSubject::UnassignedSession,
    }
}

/// Whether this traversal assigns colors or preserves the default traversal's colors.
enum HueAssignment<'p, 'a> {
    /// Assign hues in the existing depth-first census order.
    DefaultOrder,
    /// Borrow the hues already assigned by that traversal.
    Preserve(&'p HashMap<(&'a str, u32), RainbowHue>),
}

impl HueAssignment<'_, '_> {
    /// Reuse a known identity's hue, assigning a default hue for a new identity.
    fn hue(&self, identity: RowIdentity<'_>, ordinal: usize) -> RainbowHue {
        match self {
            Self::DefaultOrder => RainbowHue::of_agent(ordinal),
            Self::Preserve(hues) => hues
                .get(&(identity.machine, identity.pid))
                .copied()
                .unwrap_or_else(|| RainbowHue::of_agent(ordinal)),
        }
    }
}

/// A default traversal needs no clock; selected status values do.
enum SiblingOrder {
    /// The default start, machine and process ordering.
    Default,
    /// The selected column, with a clock for status text such as timers.
    Selected { sort: TreeSort, now: u64 },
}

impl SiblingOrder {
    /// Keep the default tie-break independent of a selected direction.
    fn compare(&self, left: &TreeLine<'_>, right: &TreeLine<'_>) -> Ordering {
        match *self {
            Self::Default => left.order_key().cmp(&right.order_key()),
            Self::Selected { sort, now } => left.compare(right, sort, now),
        }
    }
}

/// Inputs shared while building one tile's hierarchy.
struct TreeBuild<'p, 'a> {
    /// The accepted forest and its dispatch joins.
    placement: &'p AgentPlacement<'a>,
    /// Machine named by the tile heading, for remote suffixes.
    heading:   &'a str,
    /// The default traversal or the selected sibling comparator.
    order:     SiblingOrder,
    /// Assign hues on the first traversal and preserve them on reordering.
    hues:      HueAssignment<'p, 'a>,
}

impl<'a> TreeBuild<'_, 'a> {
    /// The row identity and role used both to compare and draw a sibling.
    fn tree_line(
        &self,
        identity: RowIdentity<'a>,
        row: &'a AgentRow,
        subject: LineSubject<'a>,
        hue: RainbowHue,
    ) -> TreeLine<'a> {
        TreeLine {
            dispatch_label: match subject {
                LineSubject::Agent(_) | LineSubject::UnassignedSession => DispatchLabel::Original,
                LineSubject::ManagedThread(child, _) => self.placement.dispatch_label(child),
                LineSubject::Work(work) => self.placement.dispatch_label(work.child),
            },
            machine: identity.machine,
            location: if identity.machine == self.heading {
                SessionLocation::SameMachine
            } else {
                SessionLocation::AnotherMachine
            },
            row,
            hue,
            rails: Vec::new(),
            branch: Branch::Last,
            subject,
        }
    }
}

/// Rebuild sibling order using hues assigned by the default traversal.
#[cfg(test)]
pub(crate) fn cell_order_sorted<'a>(
    placement: &AgentPlacement<'a>,
    sorts: &HashMap<AgentCell, TreeSort>,
    now: u64,
) -> Vec<AgentEntry<'a>> {
    let mut cells = cell_order_placed(placement);
    sort_cells(placement, &mut cells, sorts, now);
    cells
}

/// Reorder existing trees, preserving every previously assigned name hue.
fn sort_cells<'a>(
    placement: &AgentPlacement<'a>,
    cells: &mut [AgentEntry<'a>],
    sorts: &HashMap<AgentCell, TreeSort>,
    now: u64,
) {
    let hues: HashMap<_, _> = cells
        .iter()
        .flat_map(|entry| {
            iter::once(((entry.machine, entry.row.pid), entry.hue)).chain(
                entry
                    .lines
                    .iter()
                    .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
                    .map(|line| ((line.machine, line.row.pid), line.hue)),
            )
        })
        .collect();
    for entry in cells {
        entry.sort = sorts.get(&entry.id).copied().unwrap_or_default();
        if entry.sort == TreeSort::default() {
            continue;
        }
        let context = TreeBuild {
            placement,
            heading: entry.machine,
            order: SiblingOrder::Selected {
                sort: entry.sort,
                now,
            },
            hues: HueAssignment::Preserve(&hues),
        };
        entry.lines.clear();
        lines_of(
            &context,
            RowIdentity {
                machine: entry.machine,
                pid:     entry.row.pid,
            },
            &mut Vec::new(),
            &mut 0,
            entry.hue,
            &mut entry.lines,
        );
    }
}

/// Merge a member's own work with its listed members and managed threads.
fn lines_of<'a>(
    context: &TreeBuild<'_, 'a>,
    parent: RowIdentity<'a>,
    rails: &mut Vec<Rail>,
    agents: &mut usize,
    parent_hue: RainbowHue,
    lines: &mut Vec<TreeLine<'a>>,
) {
    let placement = context.placement;
    let mut siblings: Vec<_> = placement
        .rows
        .iter()
        .filter(|row| row.placement.is_child_of(parent))
        .map(|row| {
            (
                row.identity,
                row.row,
                LineSubject::Agent(placement.role_label(row.identity)),
                Vec::new(),
            )
        })
        .chain(
            placement
                .threads
                .iter()
                .filter(|thread| thread.manager == parent)
                .map(|thread| {
                    let child = &thread.owner_row.children[thread.child_index];
                    (
                        thread.owner,
                        thread.owner_row,
                        LineSubject::ManagedThread(child, thread.role),
                        Vec::new(),
                    )
                }),
        )
        .collect();
    if let Some(owner) = placement.rows.iter().find(|row| row.identity == parent) {
        siblings.extend(
            unclaimed_work(placement, parent, owner.row)
                .into_iter()
                .map(|branch| {
                    (
                        parent,
                        owner.row,
                        LineSubject::Work(branch.work),
                        branch.branches,
                    )
                }),
        );
    }
    siblings.sort_by(
        |(left_id, left_row, left_subject, _), (right_id, right_row, right_subject, _)| {
            context.order.compare(
                &context.tree_line(*left_id, left_row, *left_subject, parent_hue),
                &context.tree_line(*right_id, right_row, *right_subject, parent_hue),
            )
        },
    );
    let count = siblings.len();
    for (index, (identity, row, subject, branches)) in siblings.into_iter().enumerate() {
        let branch = if index + 1 == count {
            Branch::Last
        } else {
            Branch::Middle
        };
        let hue = if matches!(subject, LineSubject::Agent(_)) {
            let hue = context.hues.hue(identity, *agents);
            *agents += 1;
            hue
        } else {
            parent_hue
        };
        let mut line = context.tree_line(identity, row, subject, hue);
        line.rails.clone_from(rails);
        line.branch = branch;
        lines.push(line);
        rails.push(branch.rail());
        match subject {
            LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                lines_of(context, identity, rails, agents, hue, lines);
            },
            LineSubject::Work(_) => {
                append_work(context, branches, row, identity.machine, rails, hue, lines);
            },
            LineSubject::ManagedThread(_, _) => {},
        }
        rails.pop();
    }
}

/// Order members and work by their start time, machine and process identity.
fn line_order<'a>(
    identity: RowIdentity<'a>,
    row: &AgentRow,
    subject: LineSubject<'_>,
) -> (u64, &'a str, u32) {
    let (started, pid) = match subject {
        LineSubject::Agent(_) | LineSubject::UnassignedSession => (row.started, row.pid),
        LineSubject::ManagedThread(child, _) => (child.started, child.pid.unwrap_or(identity.pid)),
        LineSubject::Work(work) => (work.child.started, work.child.pid.unwrap_or(identity.pid)),
    };
    (started, identity.machine, pid)
}

/// Retain work that is neither a managed thread nor its folded dispatch.
fn unclaimed_work<'a>(
    placement: &AgentPlacement<'a>,
    parent: RowIdentity<'a>,
    row: &'a AgentRow,
) -> Vec<WorkBranch<'a>> {
    let claimed = placement
        .threads
        .iter()
        .filter(|thread| thread.owner == parent)
        .map(|thread| thread.child_index)
        .chain(
            row.children
                .iter()
                .enumerate()
                .filter(|(_, child)| placement.folds_dispatch(child))
                .map(|(index, _)| index),
        )
        .collect();
    work_branches(&row.children, &claimed, 0)
}

/// Append descendants under their nearest retained work ancestor.
fn append_work<'a>(
    context: &TreeBuild<'_, 'a>,
    mut branches: Vec<WorkBranch<'a>>,
    row: &'a AgentRow,
    machine: &'a str,
    rails: &mut Vec<Rail>,
    hue: RainbowHue,
    lines: &mut Vec<TreeLine<'a>>,
) {
    let placement = context.placement;
    let heading = context.heading;
    if let SiblingOrder::Selected { .. } = context.order {
        branches.sort_by(|left, right| {
            let line = |work: Work<'a>| TreeLine {
                dispatch_label: placement.dispatch_label(work.child),
                machine,
                location: SessionLocation::SameMachine,
                row,
                hue,
                rails: Vec::new(),
                branch: Branch::Last,
                subject: LineSubject::Work(work),
            };
            context.order.compare(&line(left.work), &line(right.work))
        });
    }
    let count = branches.len();
    for (index, work_branch) in branches.into_iter().enumerate() {
        let branch = if index + 1 == count {
            Branch::Last
        } else {
            Branch::Middle
        };
        lines.push(TreeLine {
            dispatch_label: placement.dispatch_label(work_branch.work.child),
            machine,
            location: if machine == heading {
                SessionLocation::SameMachine
            } else {
                SessionLocation::AnotherMachine
            },
            row,
            hue,
            rails: rails.clone(),
            branch,
            subject: LineSubject::Work(work_branch.work),
        });
        rails.push(branch.rail());
        append_work(
            context,
            work_branch.branches,
            row,
            machine,
            rails,
            hue,
            lines,
        );
        rails.pop();
    }
}

/// Active full columns when accounts distinguish rows.
const FULL_ACCOUNTS: &[TreeColumn] = &[
    TreeColumn::Session,
    TreeColumn::Role,
    TreeColumn::Status,
    TreeColumn::Account,
    TreeColumn::Branch,
    TreeColumn::Directory,
    TreeColumn::Age,
];
/// Active full columns for one label per program.
const FULL: &[TreeColumn] = &[
    TreeColumn::Session,
    TreeColumn::Role,
    TreeColumn::Status,
    TreeColumn::Branch,
    TreeColumn::Directory,
    TreeColumn::Age,
];
/// Active compressed columns when accounts distinguish rows.
const COMPRESSED_ACCOUNTS: &[TreeColumn] = &[
    TreeColumn::Session,
    TreeColumn::Role,
    TreeColumn::Status,
    TreeColumn::Account,
    TreeColumn::Age,
];
/// Active compressed columns for one label per program.
const COMPRESSED: &[TreeColumn] = &[
    TreeColumn::Session,
    TreeColumn::Role,
    TreeColumn::Status,
    TreeColumn::Age,
];

/// A fitted full tree, or the compressed column set.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CellView {
    Full(FlatColumns),
    Compressed,
}

impl CellView {
    /// Fit every full tree column using the same header and value widths.
    fn tree_at_width(
        lines: &[TreeLine<'_>],
        sort: TreeSort,
        width: u16,
        now: u64,
        account_column: AccountColumn,
        lead: SessionHeaderLead,
    ) -> Self {
        let columns = TreeLayout::Full.columns(account_column);
        let widths = tree_widths(lines, columns, now, sort, lead);
        if !lines.is_empty() && table_width(&widths) > usize::from(width) {
            Self::Compressed
        } else {
            Self::Full(FlatColumns { columns, widths })
        }
    }

    /// Full only when the heading, workplace and all tree values fit.
    fn at_width(
        entry: &AgentEntry<'_>,
        width: u16,
        now: u64,
        account_column: AccountColumn,
    ) -> Self {
        FlatColumns::fitted(entry, width, now, account_column).map_or(Self::Compressed, Self::Full)
    }
    /// Compress when the tile has fewer rows than the full view needs.
    fn fitted(entry: &AgentEntry<'_>, area: Rect, now: u64, account_column: AccountColumn) -> Self {
        let view = Self::at_width(entry, area.width, now, account_column);
        if view_height(entry, area.width, now, &view, account_column) > usize::from(area.height) {
            Self::Compressed
        } else {
            view
        }
    }
}

/// Widths that hold every active full column's header and values whole.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FlatColumns {
    columns: &'static [TreeColumn],
    widths:  Vec<u16>,
}

impl FlatColumns {
    /// Fit the full view without truncating any name.
    fn fitted(
        entry: &AgentEntry<'_>,
        width: u16,
        now: u64,
        account_column: AccountColumn,
    ) -> Option<Self> {
        if [
            header_line(entry.row, entry.machine, now, account_column),
            place_line(entry.row),
        ]
        .iter()
        .any(|line| line.width() > usize::from(width))
        {
            return None;
        }
        match CellView::tree_at_width(
            &entry.lines,
            entry.sort,
            header_width(width),
            now,
            account_column,
            SessionHeaderLead::OverBranch,
        ) {
            CellView::Full(fitted) => Some(fitted),
            CellView::Compressed => None,
        }
    }
}

/// Rows requested for the exact view drawn at this width.
pub(crate) fn height(
    entry: &AgentEntry<'_>,
    width: u16,
    now: u64,
    account_column: AccountColumn,
) -> usize {
    view_height(
        entry,
        width,
        now,
        &CellView::at_width(entry, width, now, account_column),
        account_column,
    )
}

/// Above-tree lines followed immediately by a header and lines, or the empty note.
fn view_height(
    entry: &AgentEntry<'_>,
    width: u16,
    now: u64,
    view: &CellView,
    account_column: AccountColumn,
) -> usize {
    above_tree(
        entry,
        &[],
        width,
        now,
        view,
        account_column,
        &FooterColors::default(),
    )
    .len()
        + if entry.lines.is_empty() {
            usize::from(NOTHING_RUNNING_HEIGHT)
        } else {
            usize::from(TABLE_HEADER_HEIGHT) + entry.lines.len()
        }
}

/// A listed agent's hue, or plain text for an unknown process.
pub(crate) fn name_style(cells: &[AgentEntry<'_>], machine: &str, pid: u32) -> Style {
    cells
        .iter()
        .find_map(|cell| cell.hue_of(machine, pid))
        .map_or_else(
            || Style::default().fg(text_default()),
            |hue| Role::Rainbow(hue).style(),
        )
}

/// Draw one tile with the account decision and footer palette shared by the frame.
pub(crate) fn draw(
    buffer: &mut Buffer,
    area: Rect,
    entry: &AgentEntry<'_>,
    cells: &[AgentEntry<'_>],
    now: u64,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) {
    let view = CellView::fitted(entry, area, now, account_column);
    let above = above_tree(
        entry,
        cells,
        area.width,
        now,
        &view,
        account_column,
        footer_colors,
    );
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
    if entry.lines.is_empty() {
        Paragraph::new(Span::styled(
            NOTHING_RUNNING_NOTE,
            Style::default().fg(text_default()),
        ))
        .render(tree, buffer);
    } else {
        draw_tree(
            buffer,
            tree,
            &entry.lines,
            entry.sort,
            tree_header(
                &entry.lines,
                entry.sort,
                tree,
                now,
                account_column,
                &view,
                SessionHeaderLead::OverBranch,
            ),
            &TreeDraw { now, footer_colors },
        );
    }
}

/// Header and table layout for a flat pool, using the same tree column fitter.
fn unassigned_header(
    entry: &UnassignedEntry<'_>,
    area: Rect,
    now: u64,
    account_column: AccountColumn,
) -> TreeHeader {
    let tree = header_indented(area);
    let view = CellView::tree_at_width(
        &entry.lines,
        entry.sort,
        tree.width,
        now,
        account_column,
        SessionHeaderLead::Flush,
    );
    tree_header(
        &entry.lines,
        entry.sort,
        tree,
        now,
        account_column,
        &view,
        SessionHeaderLead::Flush,
    )
}

/// The pool has one header followed by its flat roots and nested work.
pub(crate) fn unassigned_height(entry: &UnassignedEntry<'_>) -> usize {
    usize::from(TABLE_HEADER_HEIGHT) + entry.lines.len()
}

/// Resolved header cells for clicks in an unassigned tile.
pub(crate) fn unassigned_header_spans(
    entry: &UnassignedEntry<'_>,
    area: Rect,
    now: u64,
    account_column: AccountColumn,
) -> Vec<HeaderSpan<TreeColumn>> {
    unassigned_header(entry, area, now, account_column).spans
}

/// Draw the pool with the shared tree table, palette and sorted header.
pub(crate) fn draw_unassigned(
    buffer: &mut Buffer,
    area: Rect,
    entry: &UnassignedEntry<'_>,
    now: u64,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) {
    draw_tree(
        buffer,
        header_indented(area),
        &entry.lines,
        entry.sort,
        unassigned_header(entry, area, now, account_column),
        &TreeDraw { now, footer_colors },
    );
}

/// Widest header or value in each active column.
fn tree_widths(
    lines: &[TreeLine<'_>],
    columns: &[TreeColumn],
    now: u64,
    sort: TreeSort,
    lead: SessionHeaderLead,
) -> Vec<u16> {
    columns
        .iter()
        .map(|&column| {
            lines
                .iter()
                .map(|line| summary::cell_width(&line.value(column, now)))
                .max()
                .unwrap_or(0)
                .max(summary::cell_width(&column.sorted_header_text(sort, lead)))
        })
        .collect()
}

/// Width of the fitted columns and the spaces between them.
fn table_width(widths: &[u16]) -> usize {
    widths
        .iter()
        .map(|&width| usize::from(width))
        .sum::<usize>()
        + widths.len().saturating_sub(1) * usize::from(TABLE_COLUMN_SPACING)
}

/// Fitted widths and their resolved header cells are one layout decision.
struct TreeHeader {
    /// Session label alignment selected by the tile kind.
    lead:    SessionHeaderLead,
    /// Columns selected by the fitted view.
    columns: &'static [TreeColumn],
    /// Widths shared with the table renderer.
    widths:  Vec<u16>,
    /// Gap between the resolved columns.
    spacing: u16,
    /// Visible header cells for click mapping.
    spans:   Vec<HeaderSpan<TreeColumn>>,
}

/// Resolve columns from the view already fitted by drawing or click mapping.
fn tree_header(
    lines: &[TreeLine<'_>],
    sort: TreeSort,
    tree: Rect,
    now: u64,
    account_column: AccountColumn,
    view: &CellView,
    lead: SessionHeaderLead,
) -> TreeHeader {
    let (columns, widths, spacing) = match view {
        CellView::Full(fitted) => (fitted.columns, fitted.widths.clone(), TABLE_COLUMN_SPACING),
        CellView::Compressed => {
            let columns = TreeLayout::Compressed.columns(account_column);
            let mut widths = tree_widths(lines, columns, now, sort, lead);
            let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
            let minimum_columns = widths[1..]
                .iter()
                .map(|&width| usize::from(width))
                .sum::<usize>()
                + usize::from(summary::cell_width(
                    &columns[0].sorted_header_text(sort, lead),
                ));
            let spacing = TABLE_COLUMN_SPACING.min(
                tree.width
                    .saturating_sub(u16::try_from(minimum_columns).unwrap_or(u16::MAX))
                    / gaps,
            );
            let fixed = widths[1..]
                .iter()
                .map(|&width| usize::from(width))
                .sum::<usize>()
                + usize::from(spacing) * usize::from(gaps);
            widths[0] = tree
                .width
                .saturating_sub(u16::try_from(fixed).unwrap_or(u16::MAX));
            (columns, widths, spacing)
        },
    };
    let rects = Layout::horizontal(widths.iter().copied().map(Constraint::Length))
        .spacing(spacing)
        .flex(Flex::Start)
        .split(tree);
    let spans = if lines.is_empty() || tree.is_empty() {
        Vec::new()
    } else {
        columns
            .iter()
            .zip(rects.iter())
            .map(|(&column, rect)| HeaderSpan {
                column,
                columns: rect.x..rect.right(),
                row: tree.y,
            })
            .collect()
    };
    TreeHeader {
        lead,
        columns,
        widths,
        spacing,
        spans,
    }
}

/// Actual header spans in tile-content coordinates, excluding gaps and data rows.
pub(crate) fn header_spans(
    entry: &AgentEntry<'_>,
    area: Rect,
    now: u64,
    account_column: AccountColumn,
) -> Vec<HeaderSpan<TreeColumn>> {
    let view = CellView::fitted(entry, area, now, account_column);
    let above = above_tree(
        entry,
        &[],
        area.width,
        now,
        &view,
        account_column,
        &FooterColors::default(),
    );
    let above_height = u16::try_from(above.len()).unwrap_or(u16::MAX);
    let tree = header_indented(Rect {
        y: area.y.saturating_add(above_height),
        height: area.height.saturating_sub(above_height),
        ..area
    });
    tree_header(
        &entry.lines,
        entry.sort,
        tree,
        now,
        account_column,
        &view,
        SessionHeaderLead::OverBranch,
    )
    .spans
}

/// Draw both views with one table, preserving fixed columns when names compress.
struct TreeDraw<'a> {
    now:           u64,
    footer_colors: &'a FooterColors,
}

fn draw_tree(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[TreeLine<'_>],
    sort: TreeSort,
    header: TreeHeader,
    frame: &TreeDraw<'_>,
) {
    let TreeDraw { now, footer_colors } = *frame;
    if area.is_empty() {
        return;
    }
    let TreeHeader {
        lead,
        columns,
        widths,
        spacing,
        spans,
    } = header;
    debug_assert!(spans.iter().all(|span| span.row == area.y));
    let text = Style::default().fg(text_default());
    let rows = lines.iter().map(|line| {
        Row::new(columns.iter().enumerate().map(|(index, &column)| {
            if column == TreeColumn::Session {
                let lead = line.lead();
                let remaining = widths[index].saturating_sub(summary::cell_width(&lead));
                let name = summary::truncated(&line.name(), usize::from(remaining));
                let style = line.name_style();
                Cell::from(Line::from(vec![
                    Span::styled(lead, text),
                    Span::styled(name, style),
                ]))
            } else if column == TreeColumn::Account {
                match line.account() {
                    Account::Unrecorded => Cell::from(Span::styled(MISSING_VALUE, text)),
                    Account::Home(home) => {
                        let value = line.value(column, now);
                        let (account, tier) = value.split_at(home.label().len());
                        Cell::from(Line::from(vec![
                            Span::styled(
                                account.to_owned(),
                                footer_colors.account(home.label()).style(text),
                            ),
                            Span::styled(tier.to_owned(), Role::Codex.style()),
                        ]))
                    },
                }
            } else {
                Cell::from(Span::styled(
                    line.value(column, now),
                    match column {
                        TreeColumn::Status => line.status_role().style(),
                        TreeColumn::Role => line.footer_role(footer_colors).style(text),
                        _ => text,
                    },
                ))
            }
        }))
    });
    Table::new(rows, widths.iter().copied().map(Constraint::Length))
        .header(
            Row::new(columns.iter().map(|&column| {
                Cell::from(Span::styled(
                    column.sorted_header_text(sort, lead),
                    Style::default().fg(label_color()),
                ))
            }))
            .height(TABLE_HEADER_HEIGHT),
        )
        .column_spacing(spacing)
        .render(area, buffer);
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
    view: &CellView,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) -> Vec<Line<'static>> {
    let row = entry.row;
    if *view == CellView::Compressed && row.launched_by.is_none() {
        return Vec::new();
    }
    let heading =
        AgentHeading::new(row, entry.machine, now, account_column, footer_colors).fitted(width);
    let mut lines = vec![heading, place_line(row)];
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
fn header_line(
    row: &AgentRow,
    machine: &str,
    now: u64,
    account_column: AccountColumn,
) -> Line<'static> {
    header_line_with_footer_colors(row, machine, now, account_column, &FooterColors::default())
}

/// The header's account label uses the footer; its tier keeps the program style.
fn header_line_with_footer_colors(
    row: &AgentRow,
    machine: &str,
    now: u64,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) -> Line<'static> {
    AgentHeading::new(row, machine, now, account_column, footer_colors).full()
}

/// Identity and activity stay together; the location suffix may be omitted.
struct AgentHeading {
    identity: Line<'static>,
    location: Line<'static>,
}

/// A suffix is retained only when some of its own text can be drawn.
enum HeadingLocation {
    Shown(Line<'static>),
    Cut(Line<'static>),
    Omitted,
}

impl HeadingLocation {
    fn fitted(line: Line<'static>, width: usize) -> Self {
        if line.width() <= width {
            Self::Shown(line)
        } else if width > usize::from(summary::cell_width(&TRUNCATION_MARK.to_string())) {
            Self::Cut(cut_line(
                line.spans,
                u16::try_from(width).unwrap_or(u16::MAX),
            ))
        } else {
            Self::Omitted
        }
    }

    fn append_to(self, mut identity: Line<'static>) -> Line<'static> {
        match self {
            Self::Shown(location) | Self::Cut(location) => {
                identity.spans.push(summary::separator());
                identity.spans.extend(location.spans);
            },
            Self::Omitted => {},
        }
        identity
    }
}

impl AgentHeading {
    fn new(
        row: &AgentRow,
        machine: &str,
        now: u64,
        account_column: AccountColumn,
        footer_colors: &FooterColors,
    ) -> Self {
        let text = Style::default().fg(text_default());
        let separator = summary::separator;
        let mut spans = vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(format!("{PID_LABEL} {}", row.pid), text),
            separator(),
        ];
        spans.extend(summary::agent_label(row, account_column, footer_colors).spans);
        spans.extend([
            separator(),
            Span::styled(
                summary::status_text(row).to_string(),
                summary::status_role(row).style(),
            ),
            separator(),
            Span::styled(age::age_label(now.saturating_sub(row.started)), text),
        ]);
        Self {
            identity: Line::from(spans),
            location: Line::from(vec![
                Span::styled(machine.to_string(), text),
                separator(),
                Span::styled(summary::desktop_text(row).to_string(), text),
            ]),
        }
    }

    fn full(self) -> Line<'static> {
        HeadingLocation::Shown(self.location).append_to(self.identity)
    }

    fn fitted(self, width: u16) -> Line<'static> {
        let remaining =
            usize::from(width).saturating_sub(self.identity.width() + summary::separator().width());
        HeadingLocation::fitted(self.location, remaining).append_to(self.identity)
    }
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
/// `width` cells across: what equal left and right indents leave.
fn header_width(width: u16) -> u16 {
    width.saturating_sub(summary::cell_width(SECTION_HEADER_INDENT).saturating_mul(2))
}

/// The tree area with the heading indent reserved on both sides.
fn header_indented(area: Rect) -> Rect {
    let indent = summary::cell_width(SECTION_HEADER_INDENT);
    Rect {
        x: area.x.saturating_add(indent),
        width: area.width.saturating_sub(indent.saturating_mul(2)),
        ..area
    }
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

/// The same activity colors used for a Claude session's busy and idle states.
const fn thread_status_role(state: AgentActivity) -> Role { state.color_role() }

/// The program color of a retained work line's status.
const fn runs_role(kind: WorkKind) -> Role {
    match kind {
        WorkKind::Shell => Role::Shell,
        WorkKind::Subagent => Role::Claude,
        WorkKind::Thread => Role::Codex,
    }
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
    use crate::census::CodexThreadId;

    #[path = "activity_tests.rs"]
    mod activity_tests;
    #[path = "footer_color_tests.rs"]
    mod footer_color_tests;
    #[path = "sort_tests.rs"]
    mod sort_tests;
    #[path = "tree_tests.rs"]
    mod tree_tests;
    #[path = "unassigned_tests.rs"]
    mod unassigned_tests;
    use crate::census::Agent;
    use crate::census::Census;
    use crate::census::CensusUpdate;
    use crate::census::HierarchyEdge;
    use crate::census::HierarchyEdges;
    use crate::census::HierarchyRole;
    use crate::census::Machine;
    use crate::census::MachineState;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::scan::ScanStage;
    use crate::census::worker_dispatch::WorkerDispatch;
    use crate::census::worker_dispatch::WorkerTask;
    use crate::constants::TRUNCATION_MARK;

    /// Every cell across `machines`, in the order the grid shows them:
    /// machine by machine, one for each top-level agent, holding the
    /// sessions it opened, depth first and oldest first. The agents that
    /// opened sessions come first, then the rest, each oldest first. A
    /// session whose launcher is not listed stands as a top-level agent.
    /// Each agent takes the next hue of the rainbow in that order, a
    /// session as much as the agent titling its cell.
    fn cell_order<'a>(machines: &[Machine<'a>]) -> Vec<AgentEntry<'a>> {
        cell_order_placed(&AgentPlacement::from_machines(machines))
    }

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
    /// header and the tree with every name whole.
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
            session_id: SessionIdentity::Unrecorded,
            agent: Agent::Claude,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
            name: name.to_string(),
            activity: AgentActivity::from(Some("busy")),
            started: NOW - age,
            process_started: NOW - age,
            pid,
            desktop: None,
            directory: format!("~/rust/{name}"),
            branch: None,
            launched_by,
            children: Vec::new(),
        }
    }

    /// A work row, `depth` levels down, started `age`
    /// seconds before [`NOW`].
    fn child(depth: u8, kind: ChildKind, pid: Option<u32>, name: &str, age: u64) -> ChildRow {
        ChildRow {
            worker_task: WorkerTask::Unrecorded,
            dispatch: WorkerDispatch::Plain,
            activity: AgentActivity::Idle,
            thread_id: CodexThreadId::Unrecorded,
            depth,
            kind,
            service_tier: ServiceTier::Unrecorded,
            account: Account::Unrecorded,
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
                machine:         "natedev".to_string(),
                pid:             row.pid,
                process_started: row.process_started,
            },
            row,
            launcher,
            machine: "natedev",
            hue: RainbowHue::Red,
            role_label: RoleLabel::Unlabeled,
            sort: TreeSort::default(),
            lines: {
                let mut lines = Vec::new();
                let placement = AgentPlacement::from_machines(&[]);
                let context = TreeBuild {
                    placement: &placement,
                    heading:   "natedev",
                    order:     SiblingOrder::Default,
                    hues:      HueAssignment::DefaultOrder,
                };
                append_work(
                    &context,
                    work_branches(&row.children, &HashSet::new(), 0),
                    row,
                    "natedev",
                    &mut Vec::new(),
                    RainbowHue::Red,
                    &mut lines,
                );
                lines
            },
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
        draw(
            &mut buffer,
            area,
            entry,
            cells,
            NOW,
            account_column(if cells.is_empty() {
                std::slice::from_ref(entry)
            } else {
                cells
            }),
            &FooterColors::default(),
        );
        buffer
    }

    /// `entry`'s cell among `cells`, drawn `width` cells across into
    /// exactly the rows [`height`] asks for there.
    fn drawn_among(entry: &AgentEntry<'_>, cells: &[AgentEntry<'_>], width: u16) -> Buffer {
        drawn_into(
            entry,
            cells,
            width,
            height(
                entry,
                width,
                NOW,
                account_column(if cells.is_empty() {
                    std::slice::from_ref(entry)
                } else {
                    cells
                }),
            ),
        )
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

    /// Full views retain the process heading, workplace and launcher,
    /// then draw retained work with its role and program status colors.
    #[test]
    fn a_cell_draws_its_header_and_what_its_agent_runs() {
        let trunk = trunk();
        let buffer = drawn(&trunk, Some("boss of bosses"), 180);
        assert_eq!(
            lines(&buffer),
            [
                " pid 3266367 · claude · thinking · 23h · natedev · berth_fix",
                " ~/rust/tool-based-ui-trunk",
                " launched by boss of bosses",
                "    session                                                                role      status   branch  directory  age ▼",
                " ├─ Launch the Phase 1 implementation seat                                 —         command  —       —          12m",
                " │  └─ tool-based-ui-trunk-impl                                            worker    idle     —       —          11m",
                " └─ Review the permission queue                                            subagent  claude   —       —          5m 3s",
                "    └─ cargo nextest run -p hana_video --no-fail-fast -- permission_queue  —         command  —       —          45s",
            ]
        );
        let heading = " pid 3266367 · claude · thinking · 23h · natedev · berth_fix";
        for (index, character) in heading.chars().enumerate() {
            if character == '·' {
                assert_eq!(
                    buffer[(u16::try_from(index).expect("heading column"), 0)].fg,
                    text_default()
                );
            }
        }
        for text in ["pid 3266367", "natedev", "berth_fix"] {
            let (x, y) = find(&buffer, text).expect("heading value");
            for offset in 0..summary::cell_width(text) {
                assert_eq!(buffer[(x + offset, y)].fg, text_default(), "{text}");
            }
        }
        for (text, role) in [
            ("claude", Role::Claude),
            ("thinking", Role::Busy),
            ("idle", Role::Idle),
            ("command", Role::Shell),
        ] {
            let at = find(&buffer, text).expect("styled value");
            assert_eq!(Some(buffer[at].fg), role.style().fg);
        }
        for column in FULL {
            let (x, y) = find(&buffer, column.header()).expect("tree header");
            for offset in 0..summary::cell_width(column.header()) {
                assert_eq!(buffer[(x + offset, y)].fg, label_color());
            }
        }
        for y in 4..buffer.area.height {
            for x in 0..buffer.area.width {
                assert_ne!(buffer[(x, y)].fg, label_color(), "value at {x},{y}");
            }
        }
    }

    /// An agent running nothing says so beneath its heading, and a
    /// launcher that is not listed is named by its pid.
    #[test]
    fn an_agent_running_nothing_says_so() {
        let arrange = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));

        assert_eq!(
            lines(&drawn(&arrange, None, WIDE)),
            [
                " pid 3337048 · claude · thinking · 2h · natedev · —",
                " ~/rust/tool-based-ui-arrange",
                " launched by pid 1579022",
                " nothing running",
            ]
        );
    }

    /// A Codex tier follows the header program; a thread shows activity
    /// in the status column even when the account column is hidden.
    #[test]
    fn a_codex_tier_follows_its_program() {
        let codex = AgentRow {
            agent: Agent::Codex,
            service_tier: ServiceTier::Fast,
            children: vec![ChildRow {
                worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                service_tier: ServiceTier::Standard,
                ..child(0, ChildKind::Thread, None, "trunk mesh", 10 * MINUTE)
            }],
            ..agent(4_039_085, "handler", 12 * MINUTE, None)
        };
        let buffer = drawn(&codex, None, WIDE);
        let at = find(&buffer, "codex fast").expect("the header tier is drawn");
        assert_eq!(Some(buffer[at].fg), Role::Codex.style().fg);
        let at = find(&buffer, "idle").expect("the thread activity is drawn");
        assert_eq!(Some(buffer[at].fg), Role::Idle.style().fg);
        assert!(find(&buffer, "codex --").is_none());
        assert_eq!(find_all(&buffer, "trunk mesh").len(), 1);
    }

    /// A cell draws its full view at the width that holds its header's
    /// line whole, and one cell narrower, where that line would be cut,
    /// its compressed view: for a top-level agent, which the summary
    /// lists, no header and no branch and directory.
    #[test]
    fn a_cell_compresses_where_its_header_would_be_cut() {
        let boss = agent(BOSS, "boss of bosses", 2 * 24 * HOUR, None);
        let line = u16::try_from(header_line(&boss, "natedev", NOW, AccountColumn::Hidden).width())
            .expect("the line should fit a u16");

        let whole = drawn(&boss, None, line);
        let cut = drawn(&boss, None, line - 1);

        assert_eq!(
            lines(&whole),
            [
                " pid 1579022 · claude · thinking · 2d · natedev · —",
                " ~/rust/boss of bosses",
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
            directory: "~/rust/hana_catalyst/crates/hana_video/src/permission_queue/deep/nested/module/source".to_string(),
            children: vec![child(0, ChildKind::Shell, Some(2_424_763), "cargo nextest run", 45)],
            ..agent(ARRANGE, "arrange", 2 * HOUR, Some(BOSS))
        };
        let width = u16::try_from(place_line(&arrange).width()).expect("directory width");
        let whole = drawn(&arrange, None, width);
        let cut = drawn(&arrange, None, width - 1);
        assert_eq!(lines(&whole)[1], format!(" {}", arrange.directory));
        assert!(lines(&cut)[1].ends_with(TRUNCATION_MARK));
        assert!(find(&whole, "directory").is_some());
        assert!(find(&cut, "directory").is_none());
        assert!(find(&cut, "cargo nextest run").is_some());
    }

    /// A tree whose widest name cannot fit switches to compressed columns;
    /// statuses retain their program colors.
    #[test]
    fn a_cell_compresses_where_its_tree_would_cut_a_name() {
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
        let entry = entry(&arrange, None);
        let width = (1..180)
            .find(|&width| {
                matches!(
                    CellView::at_width(&entry, width, NOW, AccountColumn::Hidden),
                    CellView::Full(_)
                )
            })
            .expect("full width");
        let fits = drawn(&arrange, None, width);
        let cut = drawn(&arrange, None, width - 1);
        assert!(find(&fits, "cargo nextest run -p hana_video").is_some());
        assert!(find(&fits, "Review the permission queue").is_some());
        assert_eq!(
            CellView::at_width(&entry, width - 1, NOW, AccountColumn::Hidden),
            CellView::Compressed
        );
        assert!(find(&cut, "directory").is_none());
        assert_eq!(
            Some(cut[find(&cut, "command").expect("shell status")].fg),
            Role::Shell.style().fg
        );
        assert_eq!(
            Some(cut[find(&cut, "claude").expect("subagent status")].fg),
            Role::Claude.style().fg
        );
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
        let labels: HashSet<String> = iter::once(LAUNCHED_BY_LABEL.to_owned())
            .chain(
                FULL.iter()
                    .map(|column| column.sorted_header(TreeSort::default())),
            )
            .collect();
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
        assert_eq!(drawn, labels, "every label is drawn");
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
                let rows = height(entry, width, NOW, AccountColumn::Hidden);

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
        MachineState::Answered(
            vec![
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
                    activity: AgentActivity::from(Some("idle")),
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
                    activity: AgentActivity::from(Some("idle")),
                    directory: "~/scratch/impl".to_string(),
                    ..agent(TRUNK_IMPL, "trunk-impl", 2 * HOUR, Some(TRUNK))
                },
            ],
            crate::census::HierarchyEdges::default().into(),
        )
    }

    /// The cells `natedev` answers with.
    fn natedev_cells(natedev: &MachineState) -> Vec<AgentEntry<'_>> {
        cell_order(&[Machine {
            name:  "natedev",
            state: natedev,
        }])
    }

    #[test]
    fn older_probe_rows_keep_pid_reuse_protection_from_their_start_times() {
        let older_row = |started| {
            serde_json::from_value::<AgentRow>(serde_json::json!({
                "agent": "claude",
                "name": "remote agent",
                "status": "idle",
                "started": started,
                "pid": BOSS,
                "directory": "~",
                "launched_by": null,
                "children": []
            }))
            .expect("an older probe row should parse")
        };
        let first = older_row(NOW - HOUR);
        let reused = older_row(NOW);
        assert_eq!(first.process_started, first.started);
        assert_eq!(reused.process_started, reused.started);
        let first =
            MachineState::Answered(vec![first], crate::census::HierarchyEdges::default().into());
        let reused = MachineState::Answered(
            vec![reused],
            crate::census::HierarchyEdges::default().into(),
        );
        assert_ne!(natedev_cells(&first)[0].id, natedev_cells(&reused)[0].id);
    }

    #[test]
    fn a_resumed_claude_session_keeps_its_cell_key_when_its_age_moves_earlier() {
        let row = agent(BOSS, "resumed", HOUR, None);
        let process_started = row.process_started;
        let classified = MachineState::Answered(
            vec![row.clone()],
            crate::census::HierarchyEdges::default().into(),
        );
        let detailed = MachineState::Answered(
            vec![AgentRow {
                started: process_started - HOUR,
                ..row.clone()
            }],
            crate::census::HierarchyEdges::default().into(),
        );
        let classified_cells = natedev_cells(&classified);
        let detailed_cells = natedev_cells(&detailed);
        assert_ne!(
            classified_cells[0].row.started,
            detailed_cells[0].row.started
        );
        assert_eq!(classified_cells[0].id.process_started, process_started);
        assert_eq!(classified_cells[0].id, detailed_cells[0].id);

        let reused_pid = MachineState::Answered(
            vec![AgentRow {
                process_started: process_started + 1,
                ..row
            }],
            crate::census::HierarchyEdges::default().into(),
        );
        assert_ne!(classified_cells[0].id, natedev_cells(&reused_pid)[0].id);
    }

    /// Where it all fits, a top-level agent's cell hangs the tree of the
    /// sessions it opened from its header and its branch and directory:
    /// a line to each session, giving its status, branch, directory and
    /// age, with `—` for a session outside a repository. Each member's
    /// work nests directly beneath it, without a duplicate member marker.
    #[test]
    fn a_cell_draws_its_sessions_as_a_tree_and_nests_what_they_run() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        assert_eq!(cells.len(), 1);
        let buffer = drawn_among(&cells[0], &cells, 160);
        let rendered = lines(&buffer);
        assert!(rendered[0].contains("pid 1579022"));
        assert_eq!(rendered[1], " main · ~/rust/cargo-liner");
        assert!(rendered[2].contains("session") && rendered[2].contains("directory"));
        for name in [
            "tool-based-ui-trunk",
            "trunk-impl",
            "tool-based-ui-arrange",
            "Run the tests",
            "Review the permission queue",
            "cargo nextest run -p hana_video",
        ] {
            assert_eq!(find_all(&buffer, name).len(), 1, "{name}");
        }
        assert!(find(&buffer, "app-server").is_none());
        for line in &cells[0].lines {
            let (x, y) = find(&buffer, &line.lead()).expect("tree glyphs");
            assert_eq!(buffer[(x, y)].fg, text_default());
        }
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
    /// session's in the tree, in either view, and a
    /// listed launcher's after `launched by`. A name that is no agent's
    /// keeps the default text color.
    #[test]
    fn an_agents_name_takes_its_hue_wherever_it_shows() {
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let hue_of = |cells: &[AgentEntry<'_>], pid| name_style(cells, "natedev", pid).fg;
        for width in [160, 80] {
            let buffer = drawn_among(&cells[0], &cells, width);

            for (name, pid) in [
                ("tool-based-ui-trunk", TRUNK),
                ("trunk-impl", TRUNK_IMPL),
                ("tool-based-ui-arrange", ARRANGE),
            ] {
                let found = find_all(&buffer, name);
                assert_eq!(found.len(), 1, "{name} in the tree at {width}");
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

    /// Too narrow to lie flat, a top-level agent's cell draws no header
    /// and no branch and directory, as the summary lists the agent: a
    /// header row and one line per member or work item. Names truncate,
    /// and statuses retain the colors they take in the full view.
    #[test]
    fn a_compressed_top_level_cell_cuts_each_line_to_its_width() {
        let state = boss_group();
        let cells = natedev_cells(&state);
        let entry = &cells[0];
        let buffer = drawn_among(entry, &cells, NARROW);
        let rendered = lines(&buffer);
        assert!(!rendered[0].contains("pid"));
        assert!(rendered[0].contains("role"));
        assert_eq!(
            rendered.len(),
            entry.lines.len() + usize::from(TABLE_HEADER_HEIGHT)
        );
        assert!(rendered.iter().any(|line| line.contains(TRUNCATION_MARK)));
        assert_eq!(
            Some(buffer[find(&buffer, "thinking").expect("thinking status")].fg),
            Role::Busy.style().fg
        );
        assert_eq!(
            Some(buffer[find(&buffer, "idle").expect("idle status")].fg),
            Role::Idle.style().fg
        );
        assert_eq!(
            Some(buffer[find(&buffer, "command").expect("shell status")].fg),
            Role::Shell.style().fg
        );
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
            matches!(
                CellView::at_width(boss, WIDE, NOW, AccountColumn::Hidden),
                CellView::Full(_)
            ),
            "boss lies flat {WIDE} across"
        );
        let rows = height(boss, WIDE, NOW, AccountColumn::Hidden);
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
    /// wide width. It keeps the role, status and age columns and omits
    /// the branch and directory columns.
    #[test]
    fn a_narrow_cell_compresses_however_many_rows_it_has() {
        let state = boss_group();
        let cells = natedev_cells(&state);
        let boss = &cells[0];
        let buffer = drawn_into(
            boss,
            &cells,
            NARROW,
            height(boss, 160, NOW, AccountColumn::Hidden),
        );
        let rendered = lines(&buffer);
        assert!(find(&buffer, "directory").is_none());
        assert!(!rendered[0].contains("pid"));
        assert!(rendered[0].contains("age"));
        let count = 1 + boss.lines.len();
        assert!(rendered[count..].iter().all(String::is_empty));
        assert!(rendered[..count].iter().all(|line| !line.is_empty()));
    }

    /// A launched session whose launcher is not listed is left out of the
    /// summary, so its compressed view keeps the lines its full view
    /// opens with -- its header, its directory and the agent that
    /// opened it -- above a line to each child.
    #[test]
    fn a_launched_cell_keeps_its_header_when_compressed() {
        let trunk = trunk();
        let entry = entry(&trunk, None);
        let full = lines(&drawn_among(&entry, &[], 180));
        let short = lines(&drawn_into(
            &entry,
            &[],
            180,
            height(&entry, 180, NOW, AccountColumn::Hidden) - 1,
        ));
        assert_eq!(short[..3], full[..3]);
        assert!(short[2].contains(LAUNCHED_BY_LABEL));
        assert!(short[3].contains("session"));
        assert!(!short[3].contains("directory"));
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
                CellView::at_width(entry, NARROW, NOW, AccountColumn::Hidden),
                CellView::Compressed,
                "{name} lies flat {NARROW} across"
            );
            let rows = height(entry, NARROW, NOW, AccountColumn::Hidden);

            let drawn = lines(&drawn_into(entry, &cells, NARROW, rows + 1));

            assert_eq!(
                rows,
                view_height(
                    entry,
                    NARROW,
                    NOW,
                    &CellView::Compressed,
                    AccountColumn::Hidden
                ),
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
    /// compressed view as one line ending at the age when no location
    /// text fits, above its branch and directory and the agent that
    /// opened it, each cut to its one line.
    #[test]
    fn a_narrow_launched_cell_cuts_its_header_to_one_line() {
        let trunk = AgentRow {
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        assert_eq!(
            CellView::at_width(
                &entry(&trunk, Some("boss of bosses")),
                NARROW,
                NOW,
                AccountColumn::Hidden
            ),
            CellView::Compressed
        );

        let buffer = drawn(&trunk, Some("boss of bosses"), NARROW);

        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&buffer)[..3],
            [
                " pid 3266367 · claude · thinking · 23h".to_string(),
                format!(" ~/rust/tool-based-ui-geometry-materi{mark}"),
                " launched by boss of bosses".to_string(),
            ]
        );
        assert!(runs_in(&buffer, label_color()).contains(&LAUNCHED_BY_LABEL.to_string()));
    }

    /// A partial location retains its separator and mark; a mark alone is omitted.
    #[test]
    fn launched_heading_keeps_partial_location_and_omits_a_location_without_text() {
        let trunk = trunk();
        for (width, expected) in [
            (NARROW, " pid 3266367 · claude · thinking · 23h"),
            (42, " pid 3266367 · claude · thinking · 23h"),
            (43, " pid 3266367 · claude · thinking · 23h · n…"),
        ] {
            let buffer = drawn(&trunk, Some("boss of bosses"), width);
            assert_eq!(lines(&buffer)[0], expected, "heading at {width} cells");
        }
    }

    /// A compressed top-level agent begins with its tree header row,
    /// immediately followed by the work lines.
    #[test]
    fn a_compressed_cell_starts_with_its_tree_header() {
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
        let entry = entry(&arrange, None);
        let buffer = drawn_into(&entry, &[], 100, 3);
        let rendered = lines(&buffer);
        assert!(rendered[0].contains("session") && rendered[0].contains("status"));
        assert!(rendered[1].contains("Review the permission queue"));
        assert!(rendered[2].contains("cargo nextest run -p hana_video"));
    }

    /// natedev's answer and the mac's: two top-level agents, the older
    /// opening no session and the other the sessions under it, one under
    /// another, then a session whose launcher is not listed and two rows
    /// naming only each other, then the mac's one agent.
    fn natedev_and_mac() -> (MachineState, MachineState) {
        let natedev = MachineState::Answered(
            vec![
                agent(428_044, "enh/handler", 24 * HOUR, None),
                agent(BOSS, "boss of bosses", 23 * HOUR, None),
                agent(3_266_367, "trunk", 21 * HOUR, Some(BOSS)),
                agent(3_337_048, "arrange", 3 * HOUR, Some(BOSS)),
                agent(3_400_000, "under trunk", 2 * HOUR, Some(3_266_367)),
                agent(3_500_000, "orphan", HOUR, Some(999)),
                agent(3_600_000, "left", 30 * MINUTE, Some(3_700_000)),
                agent(3_700_000, "right", 20 * MINUTE, Some(3_600_000)),
            ],
            crate::census::HierarchyEdges::default().into(),
        );
        let mac = MachineState::Answered(
            vec![agent(12_055, "natemccoy-30", HOUR, None)],
            crate::census::HierarchyEdges::default().into(),
        );
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
                    cell.lines
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
                machine:         "natedev".to_string(),
                pid:             BOSS,
                process_started: NOW - 23 * HOUR,
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
                    cell.lines
                        .iter()
                        .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
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

    /// A timer shell retains its description as its name and draws the
    /// pie and remaining time in its shell-colored status column.
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
                    worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
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
        for width in [160, 65] {
            let buffer = drawn(&seat, Some("boss of bosses"), width);
            let at = find(&buffer, "◔ 3:48").expect("timer status");
            assert_eq!(Some(buffer[at].fg), Role::Shell.style().fg);
            assert!(find(&buffer, "Re-arm the progress timer").is_some());
            assert!(
                runs_in(&buffer, Role::Shell.style().fg.expect("shell color"))
                    .contains(&"command".to_string())
            );
        }
    }
    /// Named management edges for placement fixtures, without running a command.
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

    /// A recorded Claude session, with its own stable cross-machine id.
    fn identified(pid: u32, name: &str, age: u64, launcher: Option<u32>) -> AgentRow {
        AgentRow {
            session_id: SessionIdentity::Claude(name.to_string()),
            ..agent(pid, name, age, launcher)
        }
    }

    /// Unit links across hosts carry running work, and equal pids never share a hue or a child
    /// lookup.
    #[test]
    fn a_cross_machine_unit_brings_its_work_and_uses_its_own_process_identity() {
        let natedev = MachineState::Answered(
            vec![
                identified(1, "production", 3 * HOUR, None),
                AgentRow {
                    children: vec![child(0, ChildKind::Shell, Some(99), "local work", MINUTE)],
                    ..identified(7, "local unit", HOUR, None)
                },
            ],
            links(&[("production", "local unit")]).into(),
        );
        let mac = MachineState::Answered(
            vec![AgentRow {
                children: vec![child(0, ChildKind::Shell, Some(99), "remote work", MINUTE)],
                ..identified(7, "remote unit", 2 * HOUR, None)
            }],
            links(&[("production", "remote unit")]).into(),
        );
        let cells = natedev_and_mac_cells(&natedev, &mac);
        assert_eq!(cells.len(), 1);
        assert_eq!(
            cells[0]
                .lines
                .iter()
                .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
                .map(|session| (session.machine, session.row.name.as_str()))
                .collect::<Vec<_>>(),
            [("mac", "remote unit"), ("natedev", "local unit")]
        );
        let work: Vec<_> = cells[0]
            .lines
            .iter()
            .map(|line| {
                (
                    line.machine,
                    match line.subject {
                        LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                            line.row.name.as_str()
                        },
                        LineSubject::Work(work) => work.child.name.as_str(),
                        LineSubject::ManagedThread(child, _) => child.name.as_str(),
                    },
                    u8::try_from(line.rails.len()).expect("depth"),
                )
            })
            .collect();
        assert_eq!(
            work,
            [
                ("mac", "remote unit", 0),
                ("mac", "remote work", 1),
                ("natedev", "local unit", 0),
                ("natedev", "local work", 1),
            ]
        );
        assert_ne!(
            name_style(&cells, "mac", 7).fg,
            name_style(&cells, "natedev", 7).fg
        );
        let full = drawn_among(&cells[0], &cells, 160);
        let compressed = drawn_into(&cells[0], &cells, 80, 8);
        for buffer in [&full, &compressed] {
            assert!(
                lines(buffer)
                    .iter()
                    .any(|line| line.contains("remote unit · mac"))
            );
            assert!(
                lines(buffer)
                    .iter()
                    .any(|line| line.contains("local unit")
                        && !line.contains("local unit · natedev"))
            );
        }
        for (name, host) in [("remote unit", "mac"), ("local unit", "natedev")] {
            for at in find_all(&full, name) {
                assert_eq!(
                    full[at].fg,
                    name_style(&cells, host, 7)
                        .fg
                        .expect("the member has a hue")
                );
            }
        }
    }

    /// A management override removes stale launch markers and adds the member's work to its
    /// manager.
    #[test]
    fn a_hierarchy_override_moves_the_member_and_work_out_of_its_launcher() {
        let state = MachineState::Answered(
            vec![
                AgentRow {
                    children: vec![child(
                        0,
                        ChildKind::Session(Agent::Claude),
                        Some(3),
                        "worker",
                        HOUR,
                    )],
                    ..identified(1, "launcher", 3 * HOUR, None)
                },
                identified(2, "manager", 2 * HOUR, None),
                AgentRow {
                    children: vec![child(0, ChildKind::Shell, Some(99), "member work", MINUTE)],
                    ..identified(3, "worker", HOUR, Some(1))
                },
            ],
            links(&[("manager", "worker")]).into(),
        );
        let cells = natedev_cells(&state);
        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.row.name.as_str())
                .collect::<Vec<_>>(),
            ["manager", "launcher"]
        );
        assert_eq!(
            cells[0]
                .lines
                .iter()
                .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
                .map(|session| session.row.name.as_str())
                .collect::<Vec<_>>(),
            ["worker"]
        );
        assert_eq!(
            cells[0]
                .lines
                .iter()
                .map(|line| (
                    match line.subject {
                        LineSubject::Agent(_) | LineSubject::UnassignedSession =>
                            line.row.name.as_str(),
                        LineSubject::Work(work) => work.child.name.as_str(),
                        LineSubject::ManagedThread(child, _) => child.name.as_str(),
                    },
                    u8::try_from(line.rails.len()).expect("depth")
                ))
                .collect::<Vec<_>>(),
            [("worker", 0), ("member work", 1)]
        );
        assert!(cells[1].lines.is_empty());
    }

    /// An app server keeps its other work while one identified worker thread moves under its unit.
    #[test]
    fn a_codex_thread_nests_under_its_unit_once_without_moving_its_app_server() {
        let thread = ChildRow {
            worker_task: WorkerTask::Unrecorded,
            thread_id: CodexThreadId::Recorded("thread-id".to_string()),
            ..child(0, ChildKind::Thread, None, "implementation worker", MINUTE)
        };
        let natedev =
            MachineState::Answered(vec![identified(1, "unit", HOUR, None)], links(&[]).into());
        let mac = MachineState::Answered(
            vec![AgentRow {
                agent: Agent::Codex,
                children: vec![
                    thread,
                    child(0, ChildKind::Thread, None, "unmanaged thread", MINUTE),
                ],
                ..agent(1, "app-server", HOUR, None)
            }],
            links(&[("unit", "thread-id")]).into(),
        );
        let cells = natedev_and_mac_cells(&natedev, &mac);
        assert_eq!(cells.len(), 2);
        let session = &cells[0].lines[0];
        assert!(matches!(session.subject, LineSubject::ManagedThread(_, _)));
        assert_eq!(session.name(), "implementation worker · mac");
        assert_eq!(session.started(), NOW - MINUTE);
        assert!(
            cells[0]
                .lines
                .iter()
                .all(|line| !matches!(line.subject, LineSubject::Work(_)))
        );
        assert_eq!(
            cells[1]
                .lines
                .iter()
                .map(TreeLine::name)
                .collect::<Vec<_>>(),
            ["unmanaged thread"]
        );
        let full = lines(&drawn_among(&cells[0], &cells, 160));
        assert_eq!(
            full.iter()
                .filter(|line| line.contains("implementation worker"))
                .count(),
            1
        );
    }

    /// Managed threads show their activity when accounts are hidden in either tree view.
    #[test]
    fn managed_codex_threads_show_activity_in_full_and_compressed_views() {
        let state = MachineState::Answered(
            vec![
                identified(1, "unit", HOUR, None),
                AgentRow {
                    agent: Agent::Codex,
                    children: vec![
                        ChildRow {
                            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                            thread_id: CodexThreadId::Recorded("fast-id".to_string()),
                            service_tier: ServiceTier::Fast,
                            account: Account::Unrecorded,
                            ..child(0, ChildKind::Thread, None, "fast worker", MINUTE)
                        },
                        ChildRow {
                            worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                            thread_id: CodexThreadId::Recorded("standard-id".to_string()),
                            service_tier: ServiceTier::Standard,
                            account: Account::Unrecorded,
                            ..child(0, ChildKind::Thread, None, "standard worker", MINUTE)
                        },
                    ],
                    ..agent(2, "app-server", HOUR, None)
                },
            ],
            links(&[("unit", "fast-id"), ("unit", "standard-id")]).into(),
        );
        let cells = natedev_cells(&state);
        let manager = &cells[0];
        assert!(matches!(
            CellView::at_width(manager, 160, NOW, AccountColumn::Hidden),
            CellView::Full(_)
        ));
        assert_eq!(
            CellView::fitted(manager, area(160, 2), NOW, AccountColumn::Hidden),
            CellView::Compressed
        );
        for buffer in [
            drawn_among(manager, &cells, 160),
            drawn_into(manager, &cells, 160, 3),
        ] {
            let rendered = lines(&buffer);
            for name in ["fast worker", "standard worker"] {
                let worker_lines: Vec<_> =
                    rendered.iter().filter(|line| line.contains(name)).collect();
                assert_eq!(worker_lines.len(), 1, "each managed worker appears once");
                assert!(
                    worker_lines[0].contains("idle"),
                    "{name} shows its activity: {}",
                    worker_lines[0]
                );
            }
        }
    }

    /// Member roles occupy their text-colored column in both tree layouts.
    #[test]
    fn managed_agent_roles_render_in_their_column_in_both_views() {
        let state = MachineState::Answered(
            vec![
                identified(1, "production", 3 * HOUR, None),
                identified(2, "unit", 2 * HOUR, None),
                identified(3, "nested", HOUR, None),
            ],
            HierarchyEdges(vec![
                HierarchyEdge {
                    manager: "production".to_string(),
                    member:  "unit".to_string(),
                    role:    HierarchyRole::Impl,
                },
                HierarchyEdge {
                    manager: "unit".to_string(),
                    member:  "nested".to_string(),
                    role:    HierarchyRole::Showrunner,
                },
            ])
            .into(),
        );
        let cells = natedev_cells(&state);
        assert!(matches!(
            CellView::at_width(&cells[0], 160, NOW, AccountColumn::Hidden),
            CellView::Full(_)
        ));
        assert_eq!(
            CellView::fitted(&cells[0], area(160, 2), NOW, AccountColumn::Hidden),
            CellView::Compressed
        );
        for buffer in [
            drawn_among(&cells[0], &cells, 160),
            drawn_into(&cells[0], &cells, 160, 3),
        ] {
            for (name, role_label, pid) in [
                ("unit", RoleLabel::Stated(HierarchyRole::Impl), 2),
                ("nested", RoleLabel::Stated(HierarchyRole::Showrunner), 3),
            ] {
                let (x, y) = find(&buffer, name).expect("member name");
                assert_eq!(
                    buffer[(x, y)].fg,
                    name_style(&cells, "natedev", pid).fg.expect("member hue")
                );
                let row = &lines(&buffer)[usize::from(y)];
                assert!(row.contains(role_label.column_text()));
                assert!(!row.contains(&role_label.suffix()));
                let role_byte = row.find(role_label.column_text()).expect("role column");
                let role_x = summary::cell_width(&row[..role_byte]);
                assert_eq!(buffer[(role_x, y)].fg, text_default());
            }
        }
    }

    /// A worker discovered on a transport-only mesh server renders under its remote unit.
    #[test]
    fn a_mesh_worker_from_production_classification_renders_without_an_app_server_cell() {
        let (manager, worker) = crate::census::mesh_worker_fixture();
        let cells = natedev_and_mac_cells(&manager, &worker);
        assert_eq!(
            cells.len(),
            1,
            "the transport-only app server titles no cell"
        );
        let manager = &cells[0];
        assert!(matches!(
            CellView::at_width(manager, 160, NOW, AccountColumn::Hidden),
            CellView::Full(_)
        ));
        assert_eq!(
            CellView::fitted(manager, area(160, 1), NOW, AccountColumn::Hidden),
            CellView::Compressed
        );
        for buffer in [
            drawn_among(manager, &cells, 160),
            drawn_into(manager, &cells, 160, 2),
        ] {
            let rendered = lines(&buffer);
            let workers: Vec<_> = rendered
                .iter()
                .filter(|line| line.contains("mesh worker"))
                .collect();
            assert_eq!(workers.len(), 1, "the classified mesh worker appears once");
            assert!(workers[0].contains("mesh worker · mac"));
            let at = find(&buffer, "impl").expect("the managed thread shows its accepted role");
            assert_eq!(buffer[at].fg, text_default());
            assert!(workers[0].contains("idle"));
        }
    }

    /// Missing managers fall back to a listed launcher; missing launchers leave visible roots.
    #[test]
    fn unresolved_managers_preserve_listed_launchers_and_rescue_unlisted_ones() {
        let state = MachineState::Answered(
            vec![
                identified(1, "launcher", 3 * HOUR, None),
                identified(2, "launched", 2 * HOUR, Some(1)),
                identified(3, "orphan", HOUR, Some(99)),
            ],
            links(&[("absent", "launched"), ("absent", "orphan")]).into(),
        );
        let cells = natedev_cells(&state);
        assert_eq!(
            cells
                .iter()
                .map(|cell| cell.row.name.as_str())
                .collect::<Vec<_>>(),
            ["launcher", "orphan"]
        );
        assert_eq!(cells[0].lines[0].row.name, "launched");
    }

    /// Self links, conflicting managers and cycles still place every listed agent exactly once.
    #[test]
    fn conflicting_and_cyclic_edges_form_one_deterministic_forest() {
        let state = MachineState::Answered(
            vec![
                identified(1, "a", 3 * HOUR, None),
                identified(2, "b", 2 * HOUR, None),
                identified(3, "worker", HOUR, None),
            ],
            links(&[
                ("a", "a"),
                ("b", "a"),
                ("a", "b"),
                ("missing", "worker"),
                ("b", "worker"),
                ("a", "worker"),
            ])
            .into(),
        );
        let cells = natedev_cells(&state);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].row.name, "a");
        assert_eq!(
            cells[0]
                .lines
                .iter()
                .map(|session| (session.lead(), session.row.name.as_str()))
                .collect::<Vec<_>>(),
            [("├─ ".to_string(), "b"), ("└─ ".to_string(), "worker")]
        );
        let mut identities: Vec<_> = cells
            .iter()
            .flat_map(|cell| {
                iter::once((cell.machine, cell.row.pid)).chain(
                    cell.lines
                        .iter()
                        .map(|session| (session.machine, session.row.pid)),
                )
            })
            .collect();
        identities.sort_unstable();
        assert_eq!(identities, [("natedev", 1), ("natedev", 2), ("natedev", 3)]);
        let reversed = MachineState::Answered(
            state.rows().to_vec(),
            HierarchyEdges(state.edges().iter().rev().cloned().collect()).into(),
        );
        assert_eq!(
            natedev_cells(&reversed)[0]
                .lines
                .iter()
                .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
                .map(|session| session.row.name.as_str())
                .collect::<Vec<_>>(),
            ["b", "worker"]
        );
    }

    /// Completed links reparent without changing member identity; failure/off and a departed
    /// manager restore roots.
    #[test]
    fn census_answer_transitions_reparent_and_restore_cell_roots() {
        let mut census = crate::census::Census::new("natedev".to_string());
        let configured = ["mac".to_string()];
        let member = identified(7, "unit", HOUR, None);
        let manager = identified(7, "production", 2 * HOUR, None);
        let roots = |census: &Census| {
            cell_order(&census.machines(&configured))
                .iter()
                .map(|cell| {
                    (
                        cell.machine.to_string(),
                        cell.row.name.clone(),
                        cell.lines.len(),
                    )
                })
                .collect::<Vec<_>>()
        };
        census.apply(CensusUpdate::Local {
            rows:      vec![member.clone()],
            hierarchy: links(&[]).into(),
            stage:     ScanStage::Classified,
        });
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![manager.clone()], links(&[]).into()),
        });
        assert_eq!(
            roots(&census),
            [
                ("natedev".to_string(), "unit".to_string(), 0),
                ("mac".to_string(), "production".to_string(), 0)
            ]
        );
        census.apply(CensusUpdate::Local {
            rows:      vec![member.clone()],
            hierarchy: links(&[("production", "unit")]).into(),
            stage:     ScanStage::Classified,
        });
        assert_eq!(
            roots(&census),
            [("mac".to_string(), "production".to_string(), 1)]
        );
        census.apply(CensusUpdate::Local {
            rows:      vec![member.clone()],
            hierarchy: links(&[]).into(),
            stage:     ScanStage::Classified,
        });
        assert_eq!(roots(&census).len(), 2);
        census.apply(CensusUpdate::Local {
            rows:      vec![member],
            hierarchy: links(&[("production", "unit")]).into(),
            stage:     ScanStage::Classified,
        });
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Failed("offline".to_string()),
        });
        assert_eq!(
            roots(&census),
            [("natedev".to_string(), "unit".to_string(), 0)]
        );
        census.apply(CensusUpdate::Remote {
            host:  "mac".to_string(),
            state: MachineState::Answered(vec![manager], links(&[]).into()),
        });
        assert_eq!(
            roots(&census),
            [("mac".to_string(), "production".to_string(), 1)]
        );
    }

    /// A management edge cannot make a launcher its own descendant through the launch fallback.
    #[test]
    fn management_edges_cannot_close_a_cycle_through_a_listed_launcher() {
        let state = MachineState::Answered(
            vec![
                identified(1, "launcher", 2 * HOUR, None),
                identified(2, "launched manager", HOUR, Some(1)),
            ],
            links(&[("launched manager", "launcher")]).into(),
        );
        let cells = natedev_cells(&state);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0].row.name, "launcher");
        assert_eq!(
            cells[0]
                .lines
                .iter()
                .filter(|line| matches!(line.subject, LineSubject::Agent(_)))
                .map(|session| session.row.name.as_str())
                .collect::<Vec<_>>(),
            ["launched manager"]
        );
    }
}
