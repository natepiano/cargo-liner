//! Agent tiles draw one tree of members, managed threads and running work.
//! A label-colored header names each column. Full views keep the process
//! heading, workplace and every tree value whole; compressed views reserve
//! the fixed columns and truncate names. Heights count exactly these rows.

use std::cell::RefCell;
use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::iter;
use std::num::NonZeroU64;
use std::ops::Deref;
use std::ops::DerefMut;

use ratatui::buffer::Buffer;
use ratatui::layout::Alignment;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::widgets::Paragraph;
use ratatui::widgets::Widget;
use tui_pane::RowParent;
use tui_pane::SECTION_HEADER_INDENT;
use tui_pane::Viewport;
use tui_pane::label_color;
use tui_pane::text_default;

use crate::app::expansion::ExpansionKey;
use crate::app::expansion::HandlerExpansion;
use crate::app::expansion::HandlerTree;
use crate::app::expansion::HandlerTreeRow;
use crate::app::expansion::RowVisibility;
use crate::app::expansion::TreeMarker;
use crate::census;
use crate::census::Account;
use crate::census::AccountColumn;
use crate::census::Activity;
use crate::census::Agent;
use crate::census::AgentActivity;
use crate::census::AgentPlacement;
use crate::census::AgentRow;
use crate::census::ChildKind;
use crate::census::ChildRow;
use crate::census::CodexThreadId;
use crate::census::HierarchyRole;
#[cfg(test)]
use crate::census::Machine;
use crate::census::Placement;
use crate::census::RoleFallback;
use crate::census::RoleLabel;
use crate::census::RootPresentation;
use crate::census::RowIdentity;
use crate::census::RowRole;
use crate::census::ServiceTier;
use crate::census::ShellTimerObservation;
use crate::census::StaffRowName;
use crate::census::Timer;
use crate::census::WorkIdentity;
#[cfg(test)]
use crate::census::accounts::EMPTY_SESSION_STATES;
use crate::census::accounts::ReportedSessionState;
use crate::census::accounts::SessionState;
use crate::census::accounts::SessionStates;
use crate::census::model::ModelName;
use crate::census::model::ReasoningEffort;
use crate::census::transcript::SubagentRole;
use crate::census::worker_dispatch::DispatchLabel;
use crate::census::worker_dispatch::WorkerDispatch;
use crate::census::worker_dispatch::WorkerRoleInput;
use crate::census::worker_dispatch::WorkerRoleSource;
use crate::census::worker_dispatch::WorkerTask;
use crate::constants::HEADING_SEPARATOR;
use crate::constants::LAUNCHED_BY_LABEL;
use crate::constants::MISSING_VALUE;
use crate::constants::NOTHING_RUNNING_HEIGHT;
use crate::constants::NOTHING_RUNNING_NOTE;
use crate::constants::PID_LABEL;
use crate::constants::PRODUCERS_CELL_TITLE;
use crate::constants::TABLE_COLUMN_SPACING;
use crate::constants::TABLE_HEADER_HEIGHT;
use crate::constants::TIMER_ROLE;
use crate::constants::TREE_ACCOUNT_HEADER;
use crate::constants::TREE_ACTIVITY_HEADER;
use crate::constants::TREE_AGE_HEADER;
use crate::constants::TREE_BRANCH;
use crate::constants::TREE_BRANCH_HEADER;
use crate::constants::TREE_DIRECTORY_HEADER;
use crate::constants::TREE_EFFORT_HEADER;
use crate::constants::TREE_LAST_BRANCH;
use crate::constants::TREE_MODEL_HEADER;
use crate::constants::TREE_RAIL;
use crate::constants::TREE_ROLE_HEADER;
use crate::constants::TREE_SESSION_HEADER;
use crate::constants::TREE_SPACE;
use crate::constants::TREE_STATUS_HEADER;
use crate::constants::TREE_TYPE_HEADER;
use crate::constants::TRUNCATION_MARK;
use crate::constants::UNASSIGNED_CELL_TITLE;
use crate::details::DetailProperty;
use crate::details::RowDetails;
use crate::details::RowDetailsLookup;
use crate::footer_colors::FooterColors;
use crate::footer_colors::FooterTint;
use crate::interaction::RowIdentity as SelectedRow;
use crate::interaction::RowOwner;
use crate::interaction::RowSpan;
use crate::interaction::SurfaceSelection;
use crate::sort;
use crate::sort::HeaderSpan;
use crate::sort::SortColumn;
use crate::sort::SortDirection;
use crate::sort::TreeSort;
use crate::summary;
use crate::summary::age;
use crate::summary::age::AgeLayout;
use crate::theme::Role;
use crate::theme::SessionHue;
use crate::theme::SessionRowInk;
use crate::tiles::AgentCell;
use crate::tiles::TileKey;

/// A top-level agent and the tree of members and work drawn in its tile.
#[derive(Clone, Debug)]
pub(crate) struct AgentEntry<'a> {
    source_state:          ReportedSessionState,
    sizing:                RefCell<TreeSizing>,
    viewport:              RefCell<Viewport>,
    /// Stable grid identity of the root process.
    pub(crate) id:         AgentCell,
    /// The agent named by the tile title.
    pub(crate) row:        &'a AgentRow,
    /// The listed launcher, when its name is known.
    launcher:              Option<&'a str>,
    /// The root's machine heading.
    machine:               &'a str,
    /// The root's name color.
    pub(crate) hue:        SessionHue,
    /// The accepted or inferred root role.
    pub(crate) role_label: RoleLabel,
    /// This tile's ordering, also used to measure its marked header.
    sort:                  TreeSort,
    /// Members and work, in the depth-first order the tile draws.
    lines:                 Vec<TreeLine<'a>>,
}

impl AgentEntry<'_> {
    /// The name color of a listed agent in this tile.
    fn hue_of(&self, machine: &str, pid: u32) -> Option<SessionHue> {
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
    /// The selectable root whose location appears in the tile heading.
    TileRoot(RoleLabel),
    /// A listed agent opened or managed by a member of this tile.
    Agent(RoleLabel),
    /// A top-level session in the flat unassigned pool.
    UnassignedSession,
    /// A Codex thread held by a manager, with its stated responsibility.
    ManagedThread(&'a ChildRow, HierarchyRole, RowIdentity<'a>),
    /// Work not represented by another member line.
    Work(Work<'a>),
}

/// One member or work item, with the rails leading to it in the tree.
#[derive(Clone, Debug)]
struct TreeLine<'a> {
    source_state: ReportedSessionState,
    identity:     SelectedRow,
    content:      TreeLineContent<'a>,
}

impl<'a> TreeLine<'a> {
    fn new(content: TreeLineContent<'a>) -> Self {
        Self {
            source_state: ReportedSessionState::Unavailable,
            identity: content.selection_identity(0),
            content,
        }
    }
}

impl<'a> Deref for TreeLine<'a> {
    type Target = TreeLineContent<'a>;
    fn deref(&self) -> &Self::Target { &self.content }
}

impl DerefMut for TreeLine<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target { &mut self.content }
}

/// Presentation values for one member or work item.
#[derive(Clone, Debug)]
struct TreeLineContent<'a> {
    /// The responsibility of a launcher folded into this thread.
    dispatch_label: DispatchLabel<'a>,
    machine:        &'a str,
    location:       SessionLocation,
    row:            &'a AgentRow,
    hue:            SessionHue,
    rails:          Vec<Rail>,
    branch:         Branch,
    subject:        LineSubject<'a>,
    expansion:      TreeMarker,
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
enum WorkKind<'a> {
    Shell(ShellPurpose),
    Subagent(&'a SubagentRole),
    Thread,
}

/// The shell's role, resolved from its census timer observation.
#[derive(Clone, Copy, Debug)]
enum ShellPurpose {
    Timer(Timer),
    Command,
}

impl From<&ShellTimerObservation> for ShellPurpose {
    fn from(observation: &ShellTimerObservation) -> Self {
        match observation {
            ShellTimerObservation::ObservedCountdown(timer) => Self::Timer(*timer),
            ShellTimerObservation::Untimed => Self::Command,
        }
    }
}

/// A positive countdown resolved at the frame's clock boundary.
#[derive(Clone, Copy, Debug)]
struct PositiveRemainingTime(NonZeroU64);

/// A retained shell command either counts down or continues running.
#[derive(Clone, Copy, Debug)]
enum ShellState {
    Running,
    CountingDown(PositiveRemainingTime),
}

impl ShellState {
    fn resolve(timer: ShellTimerObservation, now: u64) -> Self {
        match timer {
            ShellTimerObservation::Untimed => Self::Running,
            ShellTimerObservation::ObservedCountdown(timer) => {
                NonZeroU64::new(timer.deadline.saturating_sub(now))
                    .map_or(Self::Running, |remaining| {
                        Self::CountingDown(PositiveRemainingTime(remaining))
                    })
            },
        }
    }
}

/// A work row paired with its classified role in the tree.
#[derive(Clone, Copy, Debug)]
struct Work<'a> {
    child:        &'a ChildRow,
    kind:         WorkKind<'a>,
    /// Position in the owning session's census children.
    census_index: usize,
}

impl<'a> Work<'a> {
    const fn account(self, parent: &'a Account) -> &'a Account {
        match (self.kind, &self.child.account) {
            (WorkKind::Thread, _) | (_, Account::Home(_)) => &self.child.account,
            (WorkKind::Shell(_) | WorkKind::Subagent(_), Account::Unrecorded) => parent,
        }
    }
}

/// How a process child participates in the visible work tree.
enum ChildLine<'a> {
    Work(WorkKind<'a>),
    Carrier,
    Session,
}

impl<'a> From<&'a ChildRow> for ChildLine<'a> {
    fn from(child: &'a ChildRow) -> Self {
        match &child.kind {
            ChildKind::Shell(observation) => {
                Self::Work(WorkKind::Shell(ShellPurpose::from(observation)))
            },
            ChildKind::Subagent(role) => Self::Work(WorkKind::Subagent(role)),
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
    Account,
    Role,
    Activity,
    Status,
    Branch,
    Type,
    Model,
    Effort,
    Directory,
    Age,
}

impl TreeColumn {
    /// The complete column set also indexes the shared width measurements.
    const COUNT: usize = FULL_ACCOUNTS.len();

    /// The header and its data use one alignment policy.
    const fn alignment(self) -> Alignment {
        match self {
            Self::Age => Alignment::Right,
            Self::Session
            | Self::Type
            | Self::Role
            | Self::Activity
            | Self::Model
            | Self::Effort
            | Self::Status
            | Self::Account
            | Self::Branch
            | Self::Directory => Alignment::Left,
        }
    }

    /// The label above this column.
    const fn header(self) -> &'static str {
        match self {
            Self::Session => TREE_SESSION_HEADER,
            Self::Type => TREE_TYPE_HEADER,
            Self::Role => TREE_ROLE_HEADER,
            Self::Activity => TREE_ACTIVITY_HEADER,
            Self::Model => TREE_MODEL_HEADER,
            Self::Effort => TREE_EFFORT_HEADER,
            Self::Status => TREE_STATUS_HEADER,
            Self::Account => TREE_ACCOUNT_HEADER,
            Self::Branch => TREE_BRANCH_HEADER,
            Self::Directory => TREE_DIRECTORY_HEADER,
            Self::Age => TREE_AGE_HEADER,
        }
    }

    /// The details property uses the visible column's value.
    const fn detail_property(self) -> DetailProperty {
        match self {
            Self::Account => DetailProperty::Account,
            Self::Session => DetailProperty::Name,
            Self::Role => DetailProperty::Role,
            Self::Activity => DetailProperty::Activity,
            Self::Status => DetailProperty::Status,
            Self::Type => DetailProperty::Type,
            Self::Model => DetailProperty::Model,
            Self::Effort => DetailProperty::Effort,
            Self::Branch => DetailProperty::Branch,
            Self::Directory => DetailProperty::Directory,
            Self::Age => DetailProperty::Age,
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
            Self::Flush => summary::cell_width(TREE_SPACE),
        }
    }
}

/// The visible responsibility shared by role text, footer tint and name style.
enum LineKind<'a> {
    /// A root shown in the flat unassigned pool.
    UnassignedSession,
    /// A listed session with its accepted or inferred hierarchy role.
    Session(RoleLabel),
    /// A Codex worker resolved from hierarchy, roster, then seat tokens.
    Worker(WorkerRoleSource),
    /// A shell keeps its text style and shows its command's purpose.
    Shell(ShellPurpose),
    /// A subagent carries its own semantic responsibility.
    Subagent(&'a SubagentRole),
}

/// Columns with meaningful values in this tile, in the canonical tree order.
/// The session anchor remains available for the empty-tree note.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct TreeColumns([bool; TreeColumn::COUNT]);

impl TreeColumns {
    fn retain(self, columns: &[TreeColumn]) -> Vec<TreeColumn> {
        columns
            .iter()
            .copied()
            .filter(|column| self.0[*column as usize])
            .collect()
    }
}

/// `TreeColumnPopulation::populated` counts recorded values in each column.
#[derive(Clone, Debug)]
struct TreeColumnPopulation {
    populated: [usize; TreeColumn::COUNT],
}

impl TreeColumnPopulation {
    fn measure(lines: &[TreeLine<'_>], now: u64) -> Self {
        Self {
            populated: std::array::from_fn(|index| {
                lines
                    .iter()
                    .filter(|line| {
                        let value = line.value(FULL_ACCOUNTS[index], now);
                        !value.trim().is_empty() && value != MISSING_VALUE
                    })
                    .count()
            }),
        }
    }

    fn columns(&self) -> TreeColumns {
        TreeColumns(std::array::from_fn(|index| {
            FULL_ACCOUNTS[index] != TreeColumn::Age
                && (matches!(
                    FULL_ACCOUNTS[index],
                    TreeColumn::Session
                        | TreeColumn::Role
                        | TreeColumn::Activity
                        | TreeColumn::Status
                ) || self.populated[index] > 0)
        }))
    }
}

/// Populated columns allowed by the account decision, with optional columns
/// removed only in the fixed fitting order.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PermittedTreeColumns(TreeColumns);

impl PermittedTreeColumns {
    fn for_lines(lines: &[TreeLine<'_>], now: u64, account_column: AccountColumn) -> Self {
        Self::from_population(&TreeColumnPopulation::measure(lines, now), account_column)
    }

    fn from_population(population: &TreeColumnPopulation, account_column: AccountColumn) -> Self {
        let mut columns = population.columns();
        if account_column == AccountColumn::Shown {
            columns.0[TreeColumn::Account as usize] = true;
        }
        Self(columns)
    }

    fn columns(self) -> Vec<TreeColumn> { self.0.retain(FULL_ACCOUNTS) }

    const fn remove(&mut self, step: ColumnDrop) { self.0.0[step.column() as usize] = false; }
}

/// `ColumnDrop::ORDER` defines the order for removing optional columns.
#[derive(Clone, Copy)]
enum ColumnDrop {
    Effort,
    Model,
    Directory,
    Type,
    Branch,
    Account,
    Role,
    Activity,
    Status,
}

impl ColumnDrop {
    const ORDER: [Self; 9] = [
        Self::Effort,
        Self::Model,
        Self::Directory,
        Self::Type,
        Self::Branch,
        Self::Account,
        Self::Role,
        Self::Activity,
        Self::Status,
    ];

    const fn column(self) -> TreeColumn {
        match self {
            Self::Effort => TreeColumn::Effort,
            Self::Model => TreeColumn::Model,
            Self::Directory => TreeColumn::Directory,
            Self::Type => TreeColumn::Type,
            Self::Branch => TreeColumn::Branch,
            Self::Account => TreeColumn::Account,
            Self::Role => TreeColumn::Role,
            Self::Activity => TreeColumn::Activity,
            Self::Status => TreeColumn::Status,
        }
    }
}

/// Full headings and complete names, or a compact tree with truncated names.
#[derive(Clone, Copy, Debug)]
enum TreePresentation {
    Full,
    HeightCompressed,
    Compressed,
}

/// Shared widths for tiles in one screen column.
#[derive(Clone, Debug)]
pub(crate) struct TreeMetrics {
    widths:              [u16; TreeColumn::COUNT],
    heading_width:       u16,
    session_label_width: u16,
    session_guide_width: u16,
    session_header_lead: SessionHeaderLead,
    ages:                AgeLayout,
    columns:             PermittedTreeColumns,
    presentation:        TreePresentation,
    alignment:           TreeColumnAlignment,
    /// The activity width that keeps each live word whole and cuts only jobs.
    activity_floor:      u16,
    activity_fit:        ActivityFit,
}

/// How the activity column of a tile fits its width.
#[derive(Clone, Copy, Debug)]
enum ActivityFit {
    /// The column shows each whole activity.
    Whole,
    /// A narrow tile cut its jobs to this width, never below the live word floor.
    Cut(u16),
}

impl ActivityFit {
    /// The drawn width of a column that measures `width`.
    fn width(self, width: u16) -> u16 {
        match self {
            Self::Whole => width,
            Self::Cut(cut) => width.min(cut),
        }
    }

    /// Aligned tiles share the narrower cut, never below the shared `floor`.
    fn merged(self, other: Self, floor: u16) -> Self {
        match (self, other) {
            (Self::Whole, Self::Whole) => Self::Whole,
            (Self::Cut(left), Self::Cut(right)) => Self::Cut(left.min(right).max(floor)),
            (Self::Cut(cut), Self::Whole) | (Self::Whole, Self::Cut(cut)) => {
                Self::Cut(cut.max(floor))
            },
        }
    }
}

/// Measured widths determine demand; fitted widths determine drawing positions.
#[derive(Clone, Copy, Debug)]
enum TreeColumnAlignment {
    Measured,
    Fitted {
        widths:  [u16; TreeColumn::COUNT],
        spacing: u16,
        slots:   PermittedTreeColumns,
    },
}

/// Direct draws measure their source; a prepared grid supplies its screen-column layout.
#[derive(Clone, Debug)]
enum TreeSizing {
    Local,
    Shared(TreeMetrics),
}

impl TreeMetrics {
    fn session_header_width(&self) -> u16 {
        self.session_label_width
            .saturating_add(self.session_header_lead.width())
    }

    fn merge(&mut self, other: &Self) {
        self.heading_width = self.heading_width.max(other.heading_width);
        self.session_label_width = self.session_label_width.max(other.session_label_width);
        self.session_guide_width = self.session_guide_width.max(other.session_guide_width);
        if matches!(other.session_header_lead, SessionHeaderLead::OverBranch) {
            self.session_header_lead = SessionHeaderLead::OverBranch;
        }
        for (width, &other) in self.widths.iter_mut().zip(&other.widths) {
            *width = (*width).max(other);
        }
        self.activity_floor = self.activity_floor.max(other.activity_floor);
        self.activity_fit = self
            .activity_fit
            .merged(other.activity_fit, self.activity_floor);
        self.presentation = match (self.presentation, other.presentation) {
            (TreePresentation::Compressed, _) | (_, TreePresentation::Compressed) => {
                TreePresentation::Compressed
            },
            (TreePresentation::HeightCompressed, _) | (_, TreePresentation::HeightCompressed) => {
                TreePresentation::HeightCompressed
            },
            (TreePresentation::Full, TreePresentation::Full) => TreePresentation::Full,
        };
    }

    fn minimum_session_width(&self) -> u16 {
        self.session_label_width
            .saturating_add(SessionHeaderLead::Flush.width())
            .max(self.session_guide_width.saturating_add(1))
    }

    fn set_activity_width(&mut self, width: u16) {
        self.widths[TreeColumn::Activity as usize] = self.activity_fit.width(width);
    }

    fn drop_step(&mut self, step: ColumnDrop, width: u16) {
        if matches!(
            step,
            ColumnDrop::Account | ColumnDrop::Role | ColumnDrop::Activity | ColumnDrop::Status
        ) {
            self.cut_session(width);
        }
        let activity = self.widths[TreeColumn::Activity as usize];
        let overflow = self.occupied_width(1).saturating_sub(usize::from(width));
        if matches!(step, ColumnDrop::Role) && overflow > 0 && activity > self.activity_floor {
            let cut = activity
                .saturating_sub(u16::try_from(overflow).unwrap_or(u16::MAX))
                .max(self.activity_floor);
            self.activity_fit = ActivityFit::Cut(cut);
            self.set_activity_width(activity);
            self.presentation = TreePresentation::Compressed;
        }
        let spacing = match step {
            ColumnDrop::Account | ColumnDrop::Role | ColumnDrop::Activity | ColumnDrop::Status => 1,
            ColumnDrop::Effort
            | ColumnDrop::Model
            | ColumnDrop::Directory
            | ColumnDrop::Type
            | ColumnDrop::Branch => TABLE_COLUMN_SPACING,
        };
        let full_fits = self.occupied_width(spacing) <= usize::from(width);
        if !full_fits || header_width(self.heading_width) > width {
            self.presentation = TreePresentation::Compressed;
        }
        if !full_fits {
            self.columns.remove(step);
        }
    }

    fn occupied_width(&self, spacing: u16) -> usize {
        let (cells, columns) = FULL_ACCOUNTS
            .iter()
            .filter(|column| self.columns.0.0[**column as usize])
            .fold((0usize, 0usize), |(cells, columns), column| {
                (
                    cells + usize::from(self.widths[*column as usize]),
                    columns + 1,
                )
            });
        cells + columns.saturating_sub(1) * usize::from(spacing)
    }

    fn cut_session(&mut self, width: u16) {
        let session = TreeColumn::Session as usize;
        let others = self
            .occupied_width(TABLE_COLUMN_SPACING)
            .saturating_sub(usize::from(self.widths[session]));
        let budget = usize::from(width).saturating_sub(others);
        self.widths[session] = self.widths[session]
            .min(u16::try_from(budget).unwrap_or(u16::MAX))
            .max(self.minimum_session_width());
    }

    fn fit_step(&mut self, step: &GroupFitStep, width: u16) {
        match step {
            GroupFitStep::Drop(column) => self.drop_step(*column, width),
            GroupFitStep::CutSession => self.cut_session(width),
            GroupFitStep::FinalMerge => {},
        }
    }
}

/// A value selected by the presentation that will draw it.
#[derive(Clone, Copy)]
struct PresentationValues<T> {
    full:       T,
    compressed: T,
}

impl<T: Copy> PresentationValues<T> {
    const fn get(&self, presentation: TreePresentation) -> T {
        match presentation {
            TreePresentation::Full => self.full,
            TreePresentation::Compressed | TreePresentation::HeightCompressed => self.compressed,
        }
    }
}

/// Rows reserved above the tree in either fitted presentation.
#[derive(Clone, Copy)]
struct TreeHeadingSpace {
    offset: usize,
    width:  u16,
    rows:   PresentationValues<usize>,
}

/// Complete ages and the two visible row budgets are measured once per tile.
#[derive(Clone)]
pub(crate) struct TreeMeasurement {
    area:            Rect,
    metrics:         TreeMetrics,
    ages:            Vec<u64>,
    role_widths:     PresentationValues<u16>,
    activity_widths: PresentationValues<u16>,
    status_widths:   PresentationValues<u16>,
    role_rows:       Vec<u16>,
    activity_rows:   Vec<u16>,
    status_rows:     Vec<u16>,
    heading:         TreeHeadingSpace,
    role_header:     u16,
    activity_header: u16,
    status_header:   u16,
}

impl TreeMeasurement {
    fn for_entry(
        entry: &AgentEntry<'_>,
        area: Rect,
        now: u64,
        account_column: AccountColumn,
    ) -> Self {
        let heading_width = [
            header_line(entry, account_column).width(),
            place_line(entry.row).width(),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        let full_above = above_tree_rows(entry.row, TreePresentation::Full).len();
        let compressed_above = above_tree_rows(entry.row, TreePresentation::Compressed).len();
        let mut measurement = Self::measure(
            &entry.lines,
            area,
            now,
            entry.sort,
            account_column,
            SessionHeaderLead::OverBranch,
            TreeHeadingSpace {
                offset: entry.viewport.borrow().scroll_offset(),
                width:  u16::try_from(heading_width).unwrap_or(u16::MAX),
                rows:   PresentationValues {
                    full:       full_above,
                    compressed: compressed_above,
                },
            },
        );
        if full_above + usize::from(TABLE_HEADER_HEIGHT) + entry.lines.len()
            > usize::from(area.height)
        {
            measurement.metrics.presentation = TreePresentation::HeightCompressed;
        }
        measurement
    }

    fn measure(
        lines: &[TreeLine<'_>],
        area: Rect,
        now: u64,
        sort: TreeSort,
        account_column: AccountColumn,
        lead: SessionHeaderLead,
        heading: TreeHeadingSpace,
    ) -> Self {
        let measured = tree_widths(lines, FULL_ACCOUNTS, now, sort, lead, &TreeSizing::Local);
        let mut widths = [0; TreeColumn::COUNT];
        for (column, width) in FULL_ACCOUNTS.iter().zip(measured) {
            widths[*column as usize] = width;
        }
        let role_rows: Vec<_> = lines
            .iter()
            .map(|line| summary::cell_width(line.role_text()))
            .collect();
        let activity_rows: Vec<_> = lines
            .iter()
            .map(|line| summary::cell_width(&line.activity_parts(now).text()))
            .collect();
        let status_rows: Vec<_> = lines
            .iter()
            .map(|line| summary::cell_width(line.session_status().text()))
            .collect();
        let visible_widths = |rows: &[u16], column: TreeColumn| {
            let width = |above: usize| {
                let count = usize::from(area.height)
                    .saturating_sub(above + usize::from(TABLE_HEADER_HEIGHT));
                rows.iter()
                    .skip(heading.offset)
                    .take(count)
                    .copied()
                    .max()
                    .unwrap_or(0)
                    .max(summary::cell_width(&column.sorted_header(sort)))
            };
            PresentationValues {
                full:       width(heading.rows.full),
                compressed: width(heading.rows.compressed),
            }
        };
        let role_widths = visible_widths(&role_rows, TreeColumn::Role);
        let activity_widths = visible_widths(&activity_rows, TreeColumn::Activity);
        let activity_floor = lines
            .iter()
            .map(|line| line.activity_parts(now).floor_width())
            .max()
            .unwrap_or(0)
            .max(summary::cell_width(
                &TreeColumn::Activity.sorted_header(sort),
            ));
        let status_widths = visible_widths(&status_rows, TreeColumn::Status);
        widths[TreeColumn::Status as usize] = status_widths.get(TreePresentation::Full);
        widths[TreeColumn::Role as usize] = role_widths.get(TreePresentation::Full);
        widths[TreeColumn::Activity as usize] = activity_widths.get(TreePresentation::Full);
        let ages: Vec<_> = lines
            .iter()
            .map(|line| now.saturating_sub(line.started()))
            .collect();
        let population = TreeColumnPopulation::measure(lines, now);
        let columns = PermittedTreeColumns::from_population(&population, account_column);
        Self {
            area,
            metrics: TreeMetrics {
                widths,
                heading_width: heading.width,
                session_label_width: summary::cell_width(&TreeColumn::Session.sorted_header(sort)),
                session_guide_width: lines
                    .iter()
                    .map(|line| summary::cell_width(&line.lead()))
                    .max()
                    .unwrap_or(0),
                session_header_lead: lead,
                ages: AgeLayout::from_ages(ages.iter().copied()),
                columns,
                presentation: TreePresentation::Full,
                alignment: TreeColumnAlignment::Measured,
                activity_floor,
                activity_fit: ActivityFit::Whole,
            },
            ages,
            role_widths,
            activity_widths,
            status_widths,
            activity_rows,
            role_rows,
            status_rows,
            role_header: summary::cell_width(&TreeColumn::Role.sorted_header(sort)),
            activity_header: summary::cell_width(&TreeColumn::Activity.sorted_header(sort)),
            status_header: summary::cell_width(&TreeColumn::Status.sorted_header(sort)),
            heading,
        }
    }

    /// Reuse source readings at the allocation; no second tree measurement is needed.
    pub(crate) fn allocated(&self, area: Rect, selection: &SurfaceSelection) -> Self {
        let mut allocated = self.clone();
        allocated.area = area;
        // Project navigation with the same operation that installs the painted
        // area after shared fitting. Either height change can move the offset.
        let offset_for = |above: usize| {
            let used = u16::try_from(above)
                .unwrap_or(u16::MAX)
                .saturating_add(TABLE_HEADER_HEIGHT)
                .min(area.height);
            let mut projected = selection.clone();
            projected.set_area(Rect {
                y: area.y.saturating_add(used),
                height: area.height.saturating_sub(used),
                ..area
            });
            projected.viewport().scroll_offset()
        };
        let offsets = PresentationValues {
            full:       offset_for(self.heading.rows.full),
            compressed: offset_for(self.heading.rows.compressed),
        };
        let visible_width = |rows: &[u16], above: usize, offset: usize, header: u16| {
            rows.iter()
                .skip(offset)
                .take(
                    usize::from(area.height)
                        .saturating_sub(above + usize::from(TABLE_HEADER_HEIGHT)),
                )
                .copied()
                .max()
                .unwrap_or(0)
                .max(header)
        };
        let widths = |rows: &[u16], header| PresentationValues {
            full:       visible_width(rows, self.heading.rows.full, offsets.full, header),
            compressed: visible_width(
                rows,
                self.heading.rows.compressed,
                offsets.compressed,
                header,
            ),
        };
        allocated.role_widths = widths(&self.role_rows, self.role_header);
        allocated.activity_widths = widths(&self.activity_rows, self.activity_header);
        allocated.status_widths = widths(&self.status_rows, self.status_header);
        allocated.metrics.widths[TreeColumn::Role as usize] = allocated.role_widths.full;
        allocated.metrics.widths[TreeColumn::Activity as usize] = allocated.activity_widths.full;
        allocated.metrics.widths[TreeColumn::Status as usize] = allocated.status_widths.full;
        allocated.metrics.presentation =
            if self.heading.rows.full + usize::from(TABLE_HEADER_HEIGHT) + self.role_rows.len()
                > usize::from(area.height)
            {
                TreePresentation::HeightCompressed
            } else {
                TreePresentation::Full
            };
        allocated
    }

    /// Read a measured column width for a layout fixture.
    #[cfg(test)]
    pub(crate) const fn column_width(&self, column: TreeColumn) -> u16 {
        self.metrics.widths[column as usize]
    }

    /// The column floor reserves guides, headers and roles; status values use the fitted columns.
    pub(crate) fn minimum_tile_width(&self) -> u16 {
        self.metrics
            .session_header_width()
            .max(self.metrics.session_guide_width.saturating_add(1))
            .saturating_add(self.role_widths.full.max(self.role_widths.compressed))
            .saturating_add(self.status_header)
            .saturating_add(TABLE_COLUMN_SPACING.saturating_mul(2))
            .saturating_add(summary::cell_width(SECTION_HEADER_INDENT).saturating_mul(2))
            .saturating_add(u16::try_from(tui_pane::BLOCK_BORDER_WIDTH).unwrap_or(u16::MAX))
    }
}

impl AgentTile<'_> {
    /// The same initial ordering used by drawing and the first header click.
    pub(crate) fn default_sort(&self) -> TreeSort {
        match self {
            Self::Agent(entry) => default_tree_sort(entry.role_label, RootTile::Own),
            Self::Pooled(entry) => {
                default_tree_sort(RoleLabel::Unlabeled, RootTile::Pooled(entry.pool))
            },
        }
    }

    pub(crate) fn measure(
        &self,
        area: Rect,
        now: u64,
        account_column: AccountColumn,
    ) -> TreeMeasurement {
        match self {
            Self::Agent(entry) => TreeMeasurement::for_entry(entry, area, now, account_column),
            Self::Pooled(entry) => TreeMeasurement::measure(
                &entry.lines,
                area,
                now,
                entry.sort,
                account_column,
                SessionHeaderLead::Flush,
                TreeHeadingSpace {
                    offset: entry.viewport.borrow().scroll_offset(),
                    width:  0,
                    rows:   PresentationValues {
                        full:       0,
                        compressed: 0,
                    },
                },
            ),
        }
    }

    #[cfg(test)]
    pub(crate) fn named_row_identities(&self) -> Vec<(String, SelectedRow)> {
        let lines = match self {
            Self::Agent(entry) => &entry.lines,
            Self::Pooled(entry) => &entry.lines,
        };
        lines
            .iter()
            .map(|line| (line.name(), line.identity.clone()))
            .collect()
    }

    pub(crate) fn row_identities(&self) -> Vec<crate::interaction::RowIdentity> {
        let lines = match self {
            Self::Agent(entry) => &entry.lines,
            Self::Pooled(entry) => &entry.lines,
        };
        lines.iter().map(|line| line.identity.clone()).collect()
    }

    /// Return a `RowSpan` for each visible row.
    pub(crate) fn row_spans(&self) -> Vec<RowSpan> {
        let lines = match self {
            Self::Agent(entry) => &entry.lines,
            Self::Pooled(entry) => &entry.lines,
        };
        (0..lines.len()).map(RowSpan::single).collect()
    }

    pub(crate) fn set_viewport(&self, viewport: &Viewport) {
        let state = match self {
            Self::Agent(entry) => &entry.viewport,
            Self::Pooled(entry) => &entry.viewport,
        };
        *state.borrow_mut() = viewport.clone();
    }

    pub(crate) fn row_area(&self, area: Rect, now: u64, accounts: AccountColumn) -> Rect {
        let above = match self {
            Self::Agent(entry) => above_tree_rows(
                entry.row,
                match CellView::fitted(entry, area, now, accounts) {
                    CellView::Full(_) => TreePresentation::Full,
                    CellView::Compressed => TreePresentation::Compressed,
                },
            )
            .len(),
            Self::Pooled(_) => 0,
        };
        let used = u16::try_from(above)
            .unwrap_or(u16::MAX)
            .saturating_add(TABLE_HEADER_HEIGHT)
            .min(area.height);
        Rect {
            y: area.y.saturating_add(used),
            height: area.height.saturating_sub(used),
            ..area
        }
    }

    pub(crate) fn share_metrics(&self, metrics: TreeMetrics) {
        let sizing = match self {
            Self::Agent(entry) => &entry.sizing,
            Self::Pooled(entry) => &entry.sizing,
        };
        *sizing.borrow_mut() = TreeSizing::Shared(metrics);
    }
}

/// Every fitting pass either drops its named column or merges the final sets.
enum GroupFitStep {
    Drop(ColumnDrop),
    CutSession,
    FinalMerge,
}

impl GroupFitStep {
    const ORDER: [Self; 11] = [
        Self::Drop(ColumnDrop::ORDER[0]),
        Self::Drop(ColumnDrop::ORDER[1]),
        Self::Drop(ColumnDrop::ORDER[2]),
        Self::Drop(ColumnDrop::ORDER[3]),
        Self::Drop(ColumnDrop::ORDER[4]),
        Self::CutSession,
        Self::Drop(ColumnDrop::ORDER[5]),
        Self::Drop(ColumnDrop::ORDER[6]),
        Self::Drop(ColumnDrop::ORDER[7]),
        Self::Drop(ColumnDrop::ORDER[8]),
        Self::FinalMerge,
    ];
}

struct MeasuredTreeGroup {
    x:       u16,
    width:   u16,
    metrics: TreeMetrics,
    members: Vec<usize>,
}

impl MeasuredTreeGroup {
    fn fit_tiles(mut self, measurements: &[TreeMeasurement], step: &GroupFitStep) -> Vec<Self> {
        let group = &mut self;
        group.metrics.ages = AgeLayout::from_ages(
            group
                .members
                .iter()
                .flat_map(|index| measurements[*index].ages.iter().copied()),
        );
        group.metrics.widths[TreeColumn::Age as usize] = group.metrics.widths
            [TreeColumn::Age as usize]
            .max(u16::try_from(group.metrics.ages.width()).unwrap_or(u16::MAX));
        let role_width = |presentation| {
            group
                .members
                .iter()
                .map(|index| {
                    let measurement = &measurements[*index];
                    measurement.role_widths.get(presentation)
                })
                .max()
                .unwrap_or(0)
        };
        group.metrics.widths[TreeColumn::Role as usize] = role_width(group.metrics.presentation);
        let activity_width = |presentation| {
            group
                .members
                .iter()
                .map(|index| {
                    let measurement = &measurements[*index];
                    measurement.activity_widths.get(presentation)
                })
                .max()
                .unwrap_or(0)
        };
        group
            .metrics
            .set_activity_width(activity_width(group.metrics.presentation));
        let status_width = |presentation| {
            group
                .members
                .iter()
                .map(|index| measurements[*index].status_widths.get(presentation))
                .max()
                .unwrap_or(0)
        };
        group.metrics.widths[TreeColumn::Status as usize] =
            status_width(group.metrics.presentation);
        match step {
            GroupFitStep::Drop(_) | GroupFitStep::CutSession => group
                .members
                .iter()
                .map(|&index| {
                    let mut metrics = group.metrics.clone();
                    metrics.fit_step(step, group.width);
                    metrics.widths[TreeColumn::Status as usize] =
                        status_width(metrics.presentation);
                    metrics.widths[TreeColumn::Role as usize] = role_width(metrics.presentation);
                    metrics.set_activity_width(activity_width(metrics.presentation));
                    Self {
                        x: group.x,
                        width: group.width,
                        metrics,
                        members: vec![index],
                    }
                })
                .collect(),
            GroupFitStep::FinalMerge => vec![self],
        }
    }
}

/// Merge equal fitted sets before each fitting step.
/// The array, rather than a convergence condition, bounds the entire pass.
pub(crate) fn shared_tree_metrics(
    measurements: impl IntoIterator<Item = TreeMeasurement>,
) -> Vec<TreeMetrics> {
    let measurements: Vec<_> = measurements.into_iter().collect();
    let mut groups: Vec<_> = measurements
        .iter()
        .enumerate()
        .map(|(index, measurement)| MeasuredTreeGroup {
            x:       measurement.area.x,
            width:   header_width(measurement.area.width),
            metrics: measurement.metrics.clone(),
            members: vec![index],
        })
        .collect();
    for step in GroupFitStep::ORDER {
        let mut merged: HashMap<_, MeasuredTreeGroup> = HashMap::new();
        for group in groups {
            match merged.entry((group.x, group.metrics.columns)) {
                Entry::Vacant(entry) => {
                    entry.insert(group);
                },
                Entry::Occupied(mut entry) => {
                    let shared = entry.get_mut();
                    shared.metrics.merge(&group.metrics);
                    shared.width = shared.width.min(group.width);
                    shared.members.extend(group.members);
                },
            }
        }
        groups = merged
            .into_values()
            .flat_map(|group| group.fit_tiles(&measurements, &step))
            .collect();
    }
    align_screen_columns(&mut groups);
    let mut result: Vec<_> = measurements
        .iter()
        .map(|measurement| measurement.metrics.clone())
        .collect();
    for group in groups {
        for index in group.members {
            result[index] = group.metrics.clone();
        }
    }
    result
}

/// Fit the union of columns at each x and reserve slots for absent columns.
fn align_screen_columns(groups: &mut [MeasuredTreeGroup]) {
    let mut shared: HashMap<u16, (TreeMetrics, u16)> = HashMap::new();
    for group in groups.iter() {
        let (common, width) = shared.entry(group.x).or_insert_with(|| {
            let mut metrics = group.metrics.clone();
            metrics.widths.fill(0);
            metrics.columns.0.0.fill(false);
            (metrics, group.width)
        });
        *width = (*width).min(group.width);
        common.heading_width = common.heading_width.max(group.metrics.heading_width);
        common.session_label_width = common
            .session_label_width
            .max(group.metrics.session_label_width);
        common.session_guide_width = common
            .session_guide_width
            .max(group.metrics.session_guide_width);
        common.activity_floor = common.activity_floor.max(group.metrics.activity_floor);
        common.activity_fit = common
            .activity_fit
            .merged(group.metrics.activity_fit, common.activity_floor);
        if matches!(
            group.metrics.session_header_lead,
            SessionHeaderLead::OverBranch
        ) {
            common.session_header_lead = SessionHeaderLead::OverBranch;
        }
        for (index, width) in group.metrics.widths.iter().enumerate() {
            if group.metrics.columns.0.0[index] {
                common.columns.0.0[index] = true;
                common.widths[index] = common.widths[index].max(*width);
            }
        }
    }
    for (common, width) in shared.values_mut() {
        for step in GroupFitStep::ORDER {
            common.fit_step(&step, *width);
        }
        let columns = common.columns.columns();
        let mut widths: Vec<_> = columns
            .iter()
            .map(|column| common.widths[*column as usize])
            .collect();
        let spacing = fit_tree_names(
            &columns,
            &mut widths,
            common.minimum_session_width(),
            *width,
        );
        fill_tree_width(&columns, &mut widths, *width, spacing);
        let mut slot_widths = common.widths;
        for (&column, &width) in columns.iter().zip(&widths) {
            slot_widths[column as usize] = width;
        }
        common.alignment = TreeColumnAlignment::Fitted {
            widths: slot_widths,
            spacing,
            slots: common.columns,
        };
    }
    for group in groups {
        let (common, _) = &shared[&group.x];
        group.metrics.widths = common.widths;
        group.metrics.session_label_width = common.session_label_width;
        group.metrics.session_guide_width = common.session_guide_width;
        for (present, &permitted) in group
            .metrics
            .columns
            .0
            .0
            .iter_mut()
            .zip(&common.columns.0.0)
        {
            *present &= permitted;
        }
        group.metrics.alignment = common.alignment;
    }
}

impl TreeLineContent<'_> {
    fn selection_identity(&self, ordinal: usize) -> crate::interaction::RowIdentity {
        if let DispatchLabel::Worker(dispatch) = self.dispatch_label {
            return SelectedRow::Seat {
                machine: self.machine.to_owned(),
                seat:    dispatch.selection_key().to_owned(),
                worker:  census::worker_identity(dispatch),
            };
        }
        let child = match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                return SelectedRow::session(self.machine, self.row);
            },
            LineSubject::ManagedThread(child, _, _) => child,
            LineSubject::Work(work) => work.child,
        };
        work_selection_identity(self.machine, self.row, child, self.dispatch_label, ordinal)
    }
}

/// Identify work from recorded data or its census occurrence.
fn work_selection_identity(
    machine: &str,
    row: &AgentRow,
    child: &ChildRow,
    dispatch_label: DispatchLabel<'_>,
    ordinal: usize,
) -> SelectedRow {
    if let DispatchLabel::Worker(dispatch) = dispatch_label {
        return SelectedRow::Seat {
            machine: machine.to_owned(),
            seat:    dispatch.selection_key().to_owned(),
            worker:  census::worker_identity(dispatch),
        };
    }
    if let WorkerDispatch::Worker(dispatch) = &child.dispatch {
        return SelectedRow::Seat {
            machine: machine.to_owned(),
            seat:    dispatch.selection_key().to_owned(),
            worker:  census::worker_identity(dispatch),
        };
    }
    if let WorkIdentity::Recorded(id) = &child.work_id {
        return SelectedRow::Subagent {
            machine: machine.to_owned(),
            owner:   RowOwner {
                pid:     row.pid,
                started: row.process_started,
            },
            id:      id.clone(),
        };
    }
    if let CodexThreadId::Recorded(id) = &child.thread_id {
        return SelectedRow::Thread {
            machine: machine.to_owned(),
            id:      id.clone(),
        };
    }
    if matches!(child.kind, ChildKind::Shell(_))
        && let Some(pid) = child.pid
    {
        return SelectedRow::Shell {
            machine: machine.to_owned(),
            pid,
            started: child.started,
        };
    }
    SelectedRow::LegacyWork {
        machine: machine.to_owned(),
        owner: RowOwner {
            pid:     row.pid,
            started: row.process_started,
        },
        kind: crate::interaction::LegacyWorkKind::from(&child.kind),
        started: child.started,
        ordinal,
    }
}

/// Work identities assigned in census order before tree sorting.
struct CensusWorkIdentities<'a> {
    by_owner: HashMap<RowIdentity<'a>, Vec<SelectedRow>>,
}

impl<'a> CensusWorkIdentities<'a> {
    fn for_placement(placement: &AgentPlacement<'a>) -> Self {
        Self::new(
            placement,
            placement
                .rows
                .iter()
                .map(|row| (row.identity, row.row))
                .chain(
                    placement
                        .threads
                        .iter()
                        .map(|thread| (thread.owner, thread.owner_row)),
                ),
        )
    }

    fn new(
        placement: &AgentPlacement<'a>,
        rows: impl IntoIterator<Item = (RowIdentity<'a>, &'a AgentRow)>,
    ) -> Self {
        let mut by_owner = HashMap::new();
        for (owner, row) in rows {
            by_owner.entry(owner).or_insert_with(|| {
                let mut counts = HashMap::new();
                row.children
                    .iter()
                    .map(|child| {
                        let mut identity = work_selection_identity(
                            owner.machine,
                            row,
                            child,
                            placement.dispatch_label(child),
                            0,
                        );
                        if matches!(identity, SelectedRow::LegacyWork { .. }) {
                            let count = counts.entry(identity.clone()).or_insert(0);
                            if let SelectedRow::LegacyWork { ordinal, .. } = &mut identity {
                                *ordinal = *count;
                            }
                            *count += 1;
                        }
                        identity
                    })
                    .collect()
            });
        }
        Self { by_owner }
    }

    fn work(&self, owner: RowIdentity<'a>, census_index: usize) -> SelectedRow {
        self.by_owner[&owner][census_index].clone()
    }
}

impl TreeLine<'_> {
    /// The name, with its machine when it differs from the heading.
    fn name(&self) -> String {
        let name = self.session_name();
        match self.location {
            SessionLocation::SameMachine => name,
            SessionLocation::AnotherMachine => format!("{name}{HEADING_SEPARATOR}{}", self.machine),
        }
    }

    /// The name without its machine.
    fn session_name(&self) -> String {
        let original = match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                &self.row.name
            },
            LineSubject::ManagedThread(child, _, _) => &child.name,
            LineSubject::Work(work) => &work.child.name,
        };
        let staff_name = match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                match &self.row.staff_names.own {
                    StaffRowName::Named(name) => Some(name),
                    StaffRowName::Unlisted => None,
                }
            },
            LineSubject::ManagedThread(child, _, _) => match &child.thread_id {
                CodexThreadId::Recorded(thread) => self.row.staff_names.threads.get(thread),
                CodexThreadId::Unrecorded => None,
            },
            LineSubject::Work(work) => match &work.child.thread_id {
                CodexThreadId::Recorded(thread) => self.row.staff_names.threads.get(thread),
                CodexThreadId::Unrecorded => None,
            },
        };
        staff_name.map_or_else(
            || match self.dispatch_label {
                DispatchLabel::Original => match self.subject {
                    LineSubject::ManagedThread(child, role, _) => child.worker_task.label(
                        WorkerRoleSource::resolve(WorkerRoleInput::Hierarchy(role)),
                        original,
                    ),
                    LineSubject::Work(work) if matches!(work.kind, WorkKind::Thread) => {
                        work.child.worker_task.label(
                            WorkerRoleSource::resolve(WorkerRoleInput::Dispatch(
                                self.dispatch_label,
                            )),
                            original,
                        )
                    },
                    _ => original.clone(),
                },
                DispatchLabel::Worker(dispatch) => dispatch.label(),
            },
            Clone::clone,
        )
    }

    /// Tree rails and the branch preceding this name.
    fn lead(&self) -> String {
        let ending = match (self.branch, self.expansion) {
            (Branch::Flat, TreeMarker::Open) => "▼ ",
            (Branch::Flat, TreeMarker::Closed) => "▶ ",
            (Branch::Flat, TreeMarker::Leaf) => "  ",
            (Branch::Middle, TreeMarker::Open) => "├─▼ ",
            (Branch::Middle, TreeMarker::Closed) => "├─▶ ",
            (Branch::Last, TreeMarker::Open) => "└─▼ ",
            (Branch::Last, TreeMarker::Closed) => "└─▶ ",
            _ => self.branch.glyph(),
        };
        self.rails
            .iter()
            .map(|&rail| rail.glyph())
            .chain(iter::once(ending))
            .collect()
    }

    /// The start of this subject, rather than its carrier process.
    const fn started(&self) -> u64 {
        match self.content.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                self.content.row.started
            },
            LineSubject::ManagedThread(child, _, _) => child.started,
            LineSubject::Work(work) => work.child.started,
        }
    }

    /// Work without its own recorded account uses the owning session's account.
    const fn account(&self) -> &Account {
        match self.content.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                &self.content.row.account
            },
            LineSubject::ManagedThread(child, _, _) => &child.account,
            LineSubject::Work(work) => work.account(&self.content.row.account),
        }
    }

    /// The tier follows the account rather than a thread's activity.
    const fn service_tier(&self) -> ServiceTier {
        match self.content.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                self.content.row.service_tier
            },
            LineSubject::ManagedThread(child, _, _)
            | LineSubject::Work(Work {
                child,
                kind: WorkKind::Thread,
                ..
            }) => child.service_tier,
            LineSubject::Work(_) => ServiceTier::Unrecorded,
        }
    }

    /// The program whose account this line names.
    const fn program(&self) -> Agent {
        match self.content.subject {
            LineSubject::TileRoot(_)
            | LineSubject::Agent(_)
            | LineSubject::UnassignedSession
            | LineSubject::Work(Work {
                kind: WorkKind::Shell(_) | WorkKind::Subagent(_),
                ..
            }) => self.content.row.agent,
            LineSubject::ManagedThread(_, _, _)
            | LineSubject::Work(Work {
                kind: WorkKind::Thread,
                ..
            }) => Agent::Codex,
        }
    }

    /// Classify a visible row once for all of its presentation decisions.
    fn kind(&self) -> LineKind<'_> {
        match self.subject {
            LineSubject::UnassignedSession => {
                if self.row.role_fallback == RoleFallback::Review {
                    LineKind::Session(RoleLabel::ReviewFallback)
                } else {
                    LineKind::UnassignedSession
                }
            },
            LineSubject::TileRoot(role) | LineSubject::Agent(role) => LineKind::Session(role),
            LineSubject::ManagedThread(
                ChildRow {
                    worker_task: WorkerTask::Reviewer { .. },
                    ..
                },
                _,
                _,
            )
            | LineSubject::Work(Work {
                child:
                    ChildRow {
                        worker_task: WorkerTask::Reviewer { .. },
                        ..
                    },
                kind: WorkKind::Thread,
                ..
            }) => LineKind::Worker(WorkerRoleSource::resolve(WorkerRoleInput::ReviewerLease)),
            LineSubject::ManagedThread(_, role, _) => {
                LineKind::Worker(WorkerRoleSource::resolve(WorkerRoleInput::Hierarchy(role)))
            },
            LineSubject::Work(Work {
                kind: WorkKind::Thread,
                ..
            }) => LineKind::Worker(WorkerRoleSource::resolve(WorkerRoleInput::Dispatch(
                self.dispatch_label,
            ))),
            LineSubject::Work(Work {
                kind: WorkKind::Shell(purpose),
                ..
            }) => LineKind::Shell(purpose),
            LineSubject::Work(Work {
                kind: WorkKind::Subagent(role),
                ..
            }) => LineKind::Subagent(role),
        }
    }

    /// Worker seats show their responsibility, or the generic worker label.
    fn role_text(&self) -> &str {
        match self.kind() {
            LineKind::Worker(_) | LineKind::Subagent(_) => RowRole::Worker.text(),
            LineKind::UnassignedSession | LineKind::Shell(_) => "",
            LineKind::Session(role) => role.column_text(),
        }
    }

    /// The census activity of this line's agent.
    fn live_activity(&self) -> AgentActivity {
        match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                self.conversation_activity(self.row.activity)
            },
            LineSubject::ManagedThread(child, _, _)
            | LineSubject::Work(Work {
                child,
                kind: WorkKind::Thread,
                ..
            }) => self.conversation_activity(child.activity),
            LineSubject::Work(Work { child, .. }) => child.activity,
        }
    }

    /// The accounts report gives `source_state` and can read the session record after the
    /// census scan, so it can see a new turn first. A `SessionState::Busy` conversation whose
    /// scanned activity names no work is `Activity::Thinking`.
    const fn conversation_activity(&self, scanned: AgentActivity) -> AgentActivity {
        match (self.source_state, scanned) {
            (
                ReportedSessionState::Reported(SessionState::Busy),
                AgentActivity::Idle | AgentActivity::Unreported,
            ) => AgentActivity::Active(Activity::Thinking),
            _ => scanned,
        }
    }

    /// The job, and the live word when the row is busy and the word adds to the job.
    fn activity_parts(&self, now: u64) -> ActivityParts {
        let parts = self.job_and_live_word(now);
        match self.session_status() {
            ReportedSessionState::Reported(SessionState::Busy) => parts,
            _ => ActivityParts {
                live: None,
                ..parts
            },
        }
    }

    /// The job, and the live word when the word adds to the job.
    fn job_and_live_word(&self, now: u64) -> ActivityParts {
        let job = self.activity_text(now);
        let live = match self.live_activity() {
            active @ AgentActivity::Active(activity) => (active.text() != job).then_some(activity),
            AgentActivity::Idle | AgentActivity::Unreported => None,
        };
        ActivityParts { job, live }
    }

    fn activity_text(&self, now: u64) -> String {
        let activity = self.live_activity();
        if activity == AgentActivity::Active(Activity::Compacting)
            && !matches!(self.kind(), LineKind::Shell(_))
        {
            return "compacting".to_owned();
        }
        let text = match self.kind() {
            LineKind::Worker(source) => source.activity_text(),
            LineKind::UnassignedSession | LineKind::Shell(ShellPurpose::Command) => "",
            LineKind::Session(role) => role.activity_text(),
            LineKind::Subagent(role) => role.text(),
            LineKind::Shell(ShellPurpose::Timer(timer)) => {
                return match ShellState::resolve(
                    ShellTimerObservation::ObservedCountdown(timer),
                    now,
                ) {
                    ShellState::Running => TIMER_ROLE.to_owned(),
                    ShellState::CountingDown(PositiveRemainingTime(remaining)) => {
                        format!("{TIMER_ROLE} {}", time_left_label(remaining.get()))
                    },
                };
            },
        };
        text.to_owned()
    }

    fn reported_state(&self, states: &SessionStates) -> ReportedSessionState {
        match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                states.for_row(self.row)
            },
            LineSubject::ManagedThread(child, _, _) | LineSubject::Work(Work { child, .. }) => {
                match &child.thread_id {
                    CodexThreadId::Recorded(id) => states.get(id),
                    CodexThreadId::Unrecorded => {
                        ReportedSessionState::Reported(if child.pid.is_some() {
                            SessionState::Idle
                        } else {
                            SessionState::Exited
                        })
                    },
                }
            },
        }
    }

    fn session_status(&self) -> ReportedSessionState {
        match self.subject {
            LineSubject::Work(Work {
                kind: WorkKind::Shell(_) | WorkKind::Subagent(_),
                ..
            }) => ReportedSessionState::Reported(SessionState::Busy),
            _ => self.source_state,
        }
    }

    /// Every Codex worker inherits its parent session's hue.
    fn name_style(&self) -> Style {
        match self.kind() {
            LineKind::Shell(_) | LineKind::Subagent(_) => Style::default().fg(text_default()),
            LineKind::UnassignedSession | LineKind::Session(_) | LineKind::Worker(_) => {
                Role::Session(self.hue).style()
            },
        }
    }

    /// Compare one selected value, preserving the default order on equal values.
    fn compare(&self, other: &Self, sort: TreeSort, now: u64) -> Ordering {
        let primary = match sort.column {
            // A larger age is an earlier start.
            TreeColumn::Age => other.started().cmp(&self.started()),
            TreeColumn::Session => sort::compare_names(&self.name(), &other.name()),
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
            TreeColumn::Activity => self.activity_parts(now).text(),
            TreeColumn::Model => self.model().display_text().to_owned(),
            TreeColumn::Effort => self.effort().display_text().to_owned(),
            TreeColumn::Type => match self.subject {
                LineSubject::Work(Work {
                    kind: WorkKind::Shell(_),
                    ..
                }) => "shell",
                LineSubject::Work(Work {
                    kind: WorkKind::Subagent(_),
                    ..
                }) => "subagent",
                LineSubject::TileRoot(_)
                | LineSubject::Agent(_)
                | LineSubject::UnassignedSession
                | LineSubject::ManagedThread(_, _, _)
                | LineSubject::Work(Work {
                    kind: WorkKind::Thread,
                    ..
                }) => "",
            }
            .to_owned(),
            TreeColumn::Status => self.session_status().text().to_owned(),
            TreeColumn::Account => match self.account() {
                Account::Unrecorded => MISSING_VALUE.to_string(),
                Account::Home(home) => self
                    .service_tier()
                    .mark(home.label().to_owned())
                    .into_owned(),
            },
            TreeColumn::Branch | TreeColumn::Directory
                if matches!(
                    self.subject,
                    LineSubject::TileRoot(_) | LineSubject::Work(_)
                ) =>
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

    /// Use the conversation's model without inheriting its owner's metadata.
    fn model(&self) -> &ModelName {
        match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                &self.row.model
            },
            LineSubject::ManagedThread(child, _, _) | LineSubject::Work(Work { child, .. }) => {
                &child.model
            },
        }
    }

    /// Use the conversation's effort without inheriting its owner's metadata.
    fn effort(&self) -> ReasoningEffort {
        match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                self.row.effort
            },
            LineSubject::ManagedThread(child, _, _) | LineSubject::Work(Work { child, .. }) => {
                child.effort
            },
        }
    }

    /// Text written into a tree cell, with the shared age width.
    fn cell_text(&self, column: TreeColumn, now: u64, ages: &AgeLayout) -> String {
        match column {
            TreeColumn::Age => ages.format(now.saturating_sub(self.started())),
            TreeColumn::Session
            | TreeColumn::Type
            | TreeColumn::Role
            | TreeColumn::Activity
            | TreeColumn::Model
            | TreeColumn::Effort
            | TreeColumn::Status
            | TreeColumn::Account
            | TreeColumn::Branch
            | TreeColumn::Directory => self.value(column, now),
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

impl TreeLine<'_> {
    /// A timer job shows its countdown ink; other jobs use the footer role ink.
    fn job_style(&self, now: u64, colors: &FooterColors) -> Style {
        match self.subject {
            LineSubject::Work(Work {
                kind: WorkKind::Shell(ShellPurpose::Timer(timer)),
                ..
            }) => match ShellState::resolve(ShellTimerObservation::ObservedCountdown(timer), now) {
                ShellState::Running => Role::Busy.style(),
                ShellState::CountingDown(_) => Role::Shell.style(),
            },
            _ => self.footer_role(colors).style(SessionRowInk::Text.style()),
        }
    }

    /// Only management responsibilities use the footer's role colors.
    fn footer_role(&self, colors: &FooterColors) -> FooterTint {
        match self.kind() {
            LineKind::Session(RoleLabel::Stated(role) | RoleLabel::Inferred(role))
            | LineKind::Worker(WorkerRoleSource::Hierarchy(role)) => colors.hierarchy_role(role),
            LineKind::UnassignedSession
            | LineKind::Session(RoleLabel::Unlabeled | RoleLabel::ReviewFallback)
            | LineKind::Worker(_)
            | LineKind::Shell(_) => FooterTint::Plain,
            LineKind::Subagent(role) => role.ink(colors),
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
        match ChildLine::from(child) {
            ChildLine::Session => {},
            ChildLine::Carrier => branches.extend(descendants()),
            ChildLine::Work(kind) => {
                let descendants = descendants();
                if claimed.contains(&(offset + index)) {
                    branches.extend(descendants);
                } else {
                    branches.push(WorkBranch {
                        work:     Work {
                            child,
                            kind,
                            census_index: offset + index,
                        },
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
    let identities = CensusWorkIdentities::for_placement(placement);
    let mut cells = Vec::new();
    let mut descendants = 0;
    for (ordinal, root) in placement.roots().into_iter().enumerate() {
        let machine = root.identity.machine;
        let hue = SessionHue::of_agent(ordinal);
        let mut lines = Vec::new();
        let context = TreeBuild {
            placement,
            identities: &identities,
            heading: machine,
            order: SiblingOrder::Default,
            hues: HueAssignment::DefaultOrder,
        };
        lines_of(
            &context,
            root.identity,
            &mut Vec::new(),
            &mut descendants,
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
            source_state: placement.session_states.for_row(root.row),
            sizing: RefCell::new(TreeSizing::Local),
            viewport: RefCell::new(tui_pane::Viewport::new()),
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

/// The tile cells of one session, as its tile line draws them.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SessionCells {
    /// The machine the session runs on.
    pub(crate) machine:  String,
    /// The name cell, without the machine.
    pub(crate) name:     String,
    /// The state the status cell draws.
    pub(crate) status:   ReportedSessionState,
    /// The activity cell text.
    pub(crate) activity: String,
}

/// Each session's first tile line, by session id. Shell and subagent work has no session.
pub(crate) fn session_cells(
    placement: &AgentPlacement<'_>,
    now: u64,
) -> BTreeMap<String, SessionCells> {
    let mut cells = BTreeMap::new();
    for entry in cell_order_placed(placement) {
        let root = match root_tile(placement, root_identity(&entry)) {
            RootTile::Own => tile_root_line(&entry),
            RootTile::Pooled(_) => pooled_line(&entry),
        };
        for line in iter::once(&root).chain(&entry.lines) {
            let thread = match line.subject {
                LineSubject::TileRoot(_)
                | LineSubject::Agent(_)
                | LineSubject::UnassignedSession => None,
                LineSubject::ManagedThread(child, _, _)
                | LineSubject::Work(Work {
                    child,
                    kind: WorkKind::Thread,
                    ..
                }) => Some(&child.thread_id),
                LineSubject::Work(Work {
                    kind: WorkKind::Shell(_) | WorkKind::Subagent(_),
                    ..
                }) => continue,
            };
            let ids: Vec<&str> = match thread {
                None => line.row.session_id.ids().collect(),
                Some(CodexThreadId::Recorded(id)) => vec![id.as_str()],
                Some(CodexThreadId::Unrecorded) => Vec::new(),
            };
            for id in ids {
                cells.entry(id.to_owned()).or_insert_with(|| SessionCells {
                    machine:  line.machine.to_owned(),
                    name:     line.session_name(),
                    status:   line.session_status(),
                    activity: line.value(TreeColumn::Activity, now),
                });
            }
        }
    }
    cells
}

/// A production-role root or one pool of flat roots.
#[derive(Clone, Debug)]
pub(crate) enum AgentTile<'a> {
    /// A session with its own tile.
    Agent(AgentEntry<'a>),
    /// Flat roots that share one tile, with their work below each row.
    Pooled(PooledEntry<'a>),
}

impl AgentTile<'_> {
    /// Count the full tree before expansion removes hidden rows.
    pub(crate) const fn full_tree_rows(&self) -> usize {
        match self {
            Self::Agent(entry) => entry.lines.len().saturating_add(1),
            Self::Pooled(entry) => entry.lines.len(),
        }
    }

    /// Stable identity independent of the current sessions in a pool.
    pub(crate) fn key(&self) -> TileKey {
        match self {
            Self::Agent(entry) => TileKey::Agent(entry.id.clone()),
            Self::Pooled(entry) => entry.pool.key(),
        }
    }
}

/// The shared tile a flat root draws in.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TilePool<'a> {
    /// The producer sessions of every machine.
    Producers,
    /// The roots of one machine without a visible hierarchy.
    Unassigned {
        /// The machine shared by all roots in this pool.
        machine: &'a str,
    },
}

impl TilePool<'_> {
    /// The grid identity of this pool.
    fn key(self) -> TileKey {
        match self {
            Self::Producers => TileKey::Producers,
            Self::Unassigned { machine } => TileKey::Unassigned {
                machine: machine.to_owned(),
            },
        }
    }

    /// The tile title: `producers`, or `unassigned` with its machine.
    pub(crate) fn title(self) -> String {
        match self {
            Self::Producers => PRODUCERS_CELL_TITLE.to_owned(),
            Self::Unassigned { machine } => {
                format!("{UNASSIGNED_CELL_TITLE}{HEADING_SEPARATOR}{machine}")
            },
        }
    }
}

/// Where a root session draws: its own tile, or a pool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootTile<'a> {
    /// The root titles its own tile.
    Own,
    /// The root is one flat row of a pool.
    Pooled(TilePool<'a>),
}

/// Staff producers share one tile; other roots follow their visible hierarchy.
fn root_tile<'a>(placement: &AgentPlacement<'a>, identity: RowIdentity<'a>) -> RootTile<'a> {
    if placement.role_label(identity) == RoleLabel::Stated(HierarchyRole::Producer) {
        return RootTile::Pooled(TilePool::Producers);
    }
    match placement.root_presentation(identity) {
        RootPresentation::Hierarchy => RootTile::Own,
        RootPresentation::Unassigned => RootTile::Pooled(TilePool::Unassigned {
            machine: identity.machine,
        }),
    }
}

/// The identity of a tile entry's root session.
const fn root_identity<'a>(entry: &AgentEntry<'a>) -> RowIdentity<'a> {
    RowIdentity {
        machine: entry.machine,
        pid:     entry.row.pid,
    }
}

/// A pool and a showrunner tile list their sessions by name.
const POOL_SORT: TreeSort = TreeSort {
    column:    TreeColumn::Session,
    direction: SortDirection::Ascending,
};

/// Default ordering belongs to a tile's semantic role, independent of saved selections.
fn default_tree_sort(role: RoleLabel, root: RootTile<'_>) -> TreeSort {
    match (root, role) {
        (RootTile::Pooled(_), _)
        | (
            RootTile::Own,
            RoleLabel::Stated(HierarchyRole::Showrunner)
            | RoleLabel::Inferred(HierarchyRole::Showrunner),
        ) => POOL_SORT,
        (RootTile::Own, _) => TreeSort::default(),
    }
}

/// One pool's sessions, each retaining its original hue.
#[derive(Clone, Debug)]
pub(crate) struct PooledEntry<'a> {
    sizing:          RefCell<TreeSizing>,
    viewport:        RefCell<Viewport>,
    /// The pool these roots share.
    pub(crate) pool: TilePool<'a>,
    /// Ordering shared by the flat roots and each nested sibling group.
    sort:            TreeSort,
    /// Flat roots followed by the existing depth-first trees below them.
    lines:           Vec<TreeLine<'a>>,
}

/// Classify live tile identities without building or ordering their rows.
pub(crate) fn tile_keys(placement: &AgentPlacement<'_>) -> HashSet<TileKey> {
    placement
        .rows
        .iter()
        .filter(|row| row.placement == Placement::Root)
        .map(|root| match root_tile(placement, root.identity) {
            RootTile::Pooled(pool) => pool.key(),
            RootTile::Own => TileKey::Agent(AgentCell {
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
///
/// The producers tile draws before the first unassigned tile.
pub(crate) fn group_tiles<'a>(
    placement: &AgentPlacement<'a>,
    agents: &[AgentEntry<'a>],
    sorts: &HashMap<TileKey, TreeSort>,
    now: u64,
) -> Vec<AgentTile<'a>> {
    let session_sorts = agents
        .iter()
        .map(|entry| {
            let root = root_tile(placement, root_identity(entry));
            let key = match root {
                RootTile::Own => TileKey::Agent(entry.id.clone()),
                RootTile::Pooled(pool) => pool.key(),
            };
            (
                entry.id.clone(),
                sorts
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| default_tree_sort(entry.role_label, root)),
            )
        })
        .collect();
    let mut ordered = agents.to_vec();
    sort_cells(placement, &mut ordered, &session_sorts, now);
    let mut tiles = Vec::new();
    let mut drawn_pools = HashSet::new();
    for entry in &ordered {
        match root_tile(placement, root_identity(entry)) {
            RootTile::Own => tiles.push(AgentTile::Agent(entry.clone())),
            RootTile::Pooled(pool) => {
                for pool in [TilePool::Producers, pool] {
                    if drawn_pools.insert(pool) {
                        tiles.extend(pooled_tile(placement, &ordered, pool, sorts, now));
                    }
                }
            },
        }
    }
    tiles
}

/// One pool's flat roots, each followed by its own tree; `None` when the pool is empty.
fn pooled_tile<'a>(
    placement: &AgentPlacement<'a>,
    ordered: &[AgentEntry<'a>],
    pool: TilePool<'a>,
    sorts: &HashMap<TileKey, TreeSort>,
    now: u64,
) -> Option<AgentTile<'a>> {
    let roots: Vec<_> = ordered
        .iter()
        .filter(|root| root_tile(placement, root_identity(root)) == RootTile::Pooled(pool))
        .collect();
    let first = roots.first()?;
    let location = if roots.iter().all(|root| root.machine == first.machine) {
        SessionLocation::SameMachine
    } else {
        SessionLocation::AnotherMachine
    };
    let sort = sorts.get(&pool.key()).copied().unwrap_or(POOL_SORT);
    let mut rows: Vec<_> = roots
        .into_iter()
        .map(|root| {
            let mut line = pooled_line(root);
            line.location = location;
            (line, root)
        })
        .collect();
    rows.sort_by(|(left, _), (right, _)| left.compare(right, sort, now));
    let lines = rows
        .into_iter()
        .flat_map(|(line, root)| iter::once(line).chain(root.lines.iter().cloned()))
        .collect();
    Some(AgentTile::Pooled(PooledEntry {
        sizing: RefCell::new(TreeSizing::Local),
        viewport: RefCell::new(tui_pane::Viewport::new()),
        pool,
        sort,
        lines,
    }))
}

/// A flat root retains its own session values and preassigned hue.
fn pooled_line<'a>(entry: &AgentEntry<'a>) -> TreeLine<'a> {
    let mut line = TreeLine::new(TreeLineContent {
        dispatch_label: DispatchLabel::Original,
        machine:        entry.machine,
        location:       SessionLocation::SameMachine,
        row:            entry.row,
        hue:            entry.hue,
        rails:          Vec::new(),
        branch:         Branch::Flat,
        subject:        match entry.role_label {
            RoleLabel::Unlabeled => LineSubject::UnassignedSession,
            role => LineSubject::Agent(role),
        },
        expansion:      TreeMarker::Leaf,
    });
    line.source_state = entry.source_state;
    line
}

/// A tile root keeps its accepted role and uses the heading's location.
fn tile_root_line<'a>(entry: &AgentEntry<'a>) -> TreeLine<'a> {
    let mut root = pooled_line(entry);
    root.subject = LineSubject::TileRoot(entry.role_label);
    root
}

/// Whether this traversal assigns colors or preserves the default traversal's colors.
enum HueAssignment<'p, 'a> {
    /// Assign hues in the existing depth-first census order.
    DefaultOrder,
    /// Borrow the hues already assigned by that traversal.
    Preserve(&'p HashMap<(&'a str, u32), SessionHue>),
}

impl HueAssignment<'_, '_> {
    /// Reuse a known identity's hue, assigning a default hue for a new identity.
    fn hue(&self, identity: RowIdentity<'_>, ordinal: usize) -> SessionHue {
        match self {
            Self::DefaultOrder => SessionHue::of_agent(ordinal),
            Self::Preserve(hues) => hues
                .get(&(identity.machine, identity.pid))
                .copied()
                .unwrap_or_else(|| SessionHue::of_agent(ordinal)),
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
    placement:  &'p AgentPlacement<'a>,
    /// Census identities shared by every sorted traversal.
    identities: &'p CensusWorkIdentities<'a>,
    /// Machine named by the tile heading, for remote suffixes.
    heading:    &'a str,
    /// The default traversal or the selected sibling comparator.
    order:      SiblingOrder,
    /// Assign hues on the first traversal and preserve them on reordering.
    hues:       HueAssignment<'p, 'a>,
}

impl<'a> TreeBuild<'_, 'a> {
    /// The row identity and role used both to compare and draw a sibling.
    fn tree_line(
        &self,
        identity: RowIdentity<'a>,
        row: &'a AgentRow,
        subject: LineSubject<'a>,
        hue: SessionHue,
        selection: SelectedRow,
    ) -> TreeLine<'a> {
        let content = TreeLineContent {
            dispatch_label: match subject {
                LineSubject::TileRoot(_)
                | LineSubject::Agent(_)
                | LineSubject::UnassignedSession => self.placement.row_dispatch_label(identity),
                LineSubject::ManagedThread(child, _, _) => self.placement.dispatch_label(child),
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
            expansion: TreeMarker::Leaf,
        };
        let identity = if matches!(content.dispatch_label, DispatchLabel::Worker(_)) {
            content.selection_identity(0)
        } else {
            selection
        };
        let mut line = TreeLine {
            identity,
            content,
            source_state: ReportedSessionState::Unavailable,
        };
        line.source_state = line.reported_state(self.placement.session_states);
        line
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
    let identities = CensusWorkIdentities::for_placement(placement);
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
            identities: &identities,
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
    parent_hue: SessionHue,
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
                SelectedRow::session(row.identity.machine, row.row),
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
                        LineSubject::ManagedThread(child, thread.role, thread.manager),
                        Vec::new(),
                        context.identities.work(thread.owner, thread.child_index),
                    )
                }),
        )
        .collect();
    siblings.extend(
        session_work_branches(placement, parent)
            .into_iter()
            .map(|work| {
                (
                    work.owner,
                    work.row,
                    LineSubject::Work(work.branch.work),
                    work.branch.branches,
                    context
                        .identities
                        .work(work.owner, work.branch.work.census_index),
                )
            }),
    );
    siblings.sort_by(
        |(left_id, left_row, left_subject, _, left_selection),
         (right_id, right_row, right_subject, _, right_selection)| {
            context.order.compare(
                &context.tree_line(
                    *left_id,
                    left_row,
                    *left_subject,
                    parent_hue,
                    left_selection.clone(),
                ),
                &context.tree_line(
                    *right_id,
                    right_row,
                    *right_subject,
                    parent_hue,
                    right_selection.clone(),
                ),
            )
        },
    );
    let count = siblings.len();
    for (index, (identity, row, subject, branches, selection)) in siblings.into_iter().enumerate() {
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
        let mut line = context.tree_line(identity, row, subject, hue, selection);
        line.rails.clone_from(rails);
        line.branch = branch;
        lines.push(line);
        rails.push(branch.rail());
        match subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                lines_of(context, identity, rails, agents, hue, lines);
            },
            LineSubject::Work(_) => {
                append_work(context, branches, row, identity.machine, rails, hue, lines);
            },
            LineSubject::ManagedThread(_, _, _) => {},
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
        LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
            (row.started, row.pid)
        },
        LineSubject::ManagedThread(child, _, _) => {
            (child.started, child.pid.unwrap_or(identity.pid))
        },
        LineSubject::Work(work) => (work.child.started, work.child.pid.unwrap_or(identity.pid)),
    };
    (started, identity.machine, pid)
}

/// A work branch retains the census owner that supplies its row metadata.
struct SessionWorkBranch<'a> {
    owner:  RowIdentity<'a>,
    row:    &'a AgentRow,
    branch: WorkBranch<'a>,
}

/// Combine a session's own work with its paired dispatch work.
fn session_work_branches<'a>(
    placement: &AgentPlacement<'a>,
    parent: RowIdentity<'a>,
) -> Vec<SessionWorkBranch<'a>> {
    let mut branches = Vec::new();
    if let Some(owner) = placement.rows.iter().find(|row| row.identity == parent) {
        branches.extend(
            unclaimed_work(placement, parent, owner.row)
                .into_iter()
                .map(|branch| SessionWorkBranch {
                    owner: parent,
                    row: owner.row,
                    branch,
                }),
        );
    }
    let dispatches = placement.session_dispatch_work();
    for work in dispatches.iter().filter(|work| work.worker == parent) {
        let mut claimed = work_claims(placement, work.owner, work.row);
        claimed.extend(
            dispatches
                .iter()
                .filter(|other| {
                    other.owner == work.owner
                        && other.descendants != work.descendants
                        && work.descendants.contains(&other.descendants.start)
                })
                .flat_map(|other| other.descendants.clone()),
        );
        branches.extend(
            work_branches(
                &work.row.children[work.descendants.clone()],
                &claimed,
                work.descendants.start,
            )
            .into_iter()
            .map(|branch| SessionWorkBranch {
                owner: work.owner,
                row: work.row,
                branch,
            }),
        );
    }
    branches
}

/// Identify managed threads and paired shells in their original census row.
fn work_claims(
    placement: &AgentPlacement<'_>,
    parent: RowIdentity<'_>,
    row: &AgentRow,
) -> HashSet<usize> {
    placement
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
        .collect()
}

/// Retain work that is neither a managed thread nor its folded dispatch.
fn unclaimed_work<'a>(
    placement: &AgentPlacement<'a>,
    parent: RowIdentity<'a>,
    row: &'a AgentRow,
) -> Vec<WorkBranch<'a>> {
    let mut claimed = work_claims(placement, parent, row);
    claimed.extend(
        placement
            .session_dispatch_work()
            .into_iter()
            .filter(|work| work.owner == parent)
            .flat_map(|work| work.descendants),
    );
    work_branches(&row.children, &claimed, 0)
}

/// Append descendants under their nearest retained work ancestor.
fn append_work<'a>(
    context: &TreeBuild<'_, 'a>,
    mut branches: Vec<WorkBranch<'a>>,
    row: &'a AgentRow,
    machine: &'a str,
    rails: &mut Vec<Rail>,
    hue: SessionHue,
    lines: &mut Vec<TreeLine<'a>>,
) {
    let owner = RowIdentity {
        machine,
        pid: row.pid,
    };
    let line = |work: Work<'a>| {
        context.tree_line(
            owner,
            row,
            LineSubject::Work(work),
            hue,
            context.identities.work(owner, work.census_index),
        )
    };
    if let SiblingOrder::Selected { .. } = context.order {
        branches.sort_by(|left, right| context.order.compare(&line(left.work), &line(right.work)));
    }
    let count = branches.len();
    for (index, work_branch) in branches.into_iter().enumerate() {
        let branch = if index + 1 == count {
            Branch::Last
        } else {
            Branch::Middle
        };
        let mut work_line = line(work_branch.work);
        work_line.rails.clone_from(rails);
        work_line.branch = branch;
        lines.push(work_line);
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
    TreeColumn::Account,
    TreeColumn::Role,
    TreeColumn::Activity,
    TreeColumn::Status,
    TreeColumn::Branch,
    TreeColumn::Type,
    TreeColumn::Model,
    TreeColumn::Effort,
    TreeColumn::Directory,
    TreeColumn::Age,
];
/// A fitted full tree, or the compressed column set.
#[derive(Clone, Debug, Eq, PartialEq)]
enum CellView {
    Full(FittedTreeColumns),
    Compressed,
}

impl CellView {
    /// Request rows before the allocation compresses a tree that fits this width.
    fn requested_at_width(
        entry: &AgentEntry<'_>,
        width: u16,
        now: u64,
        account_column: AccountColumn,
    ) -> Self {
        let sizing = entry.sizing.borrow();
        if let TreeSizing::Shared(metrics) = &*sizing
            && matches!(metrics.presentation, TreePresentation::HeightCompressed)
            && metrics.heading_width <= width
        {
            return Self::tree_at_width(
                &entry.lines,
                entry.sort,
                header_width(width),
                now,
                account_column,
                SessionHeaderLead::OverBranch,
                &sizing,
            );
        }
        Self::at_width(entry, width, now, account_column)
    }

    /// Fit every full tree column using the same header and value widths.
    fn tree_at_width(
        lines: &[TreeLine<'_>],
        sort: TreeSort,
        width: u16,
        now: u64,
        account_column: AccountColumn,
        lead: SessionHeaderLead,
        sizing: &TreeSizing,
    ) -> Self {
        let columns = match sizing {
            TreeSizing::Local => {
                PermittedTreeColumns::for_lines(lines, now, account_column).columns()
            },
            TreeSizing::Shared(metrics) => metrics.columns.columns(),
        };
        let widths = tree_widths(lines, &columns, now, sort, lead, sizing);
        if !lines.is_empty() && table_width(&widths) > usize::from(width) {
            Self::Compressed
        } else {
            Self::Full(FittedTreeColumns { columns, widths })
        }
    }

    /// Full only when the heading, workplace and all tree values fit.
    fn at_width(
        entry: &AgentEntry<'_>,
        width: u16,
        now: u64,
        account_column: AccountColumn,
    ) -> Self {
        match FittedTreeColumns::fitted(entry, width, now, account_column) {
            TreeColumnFit::Fitted(columns) => Self::Full(columns),
            TreeColumnFit::TooNarrow => Self::Compressed,
        }
    }
    /// Compress when the tile has fewer rows than the full view needs.
    fn fitted(entry: &AgentEntry<'_>, area: Rect, now: u64, account_column: AccountColumn) -> Self {
        let view = Self::at_width(entry, area.width, now, account_column);
        if view_height(entry, area.width, &view, account_column) > usize::from(area.height) {
            Self::Compressed
        } else {
            view
        }
    }
}

/// Widths that hold every active full column's header and values whole.
#[derive(Clone, Debug, Eq, PartialEq)]
struct FittedTreeColumns {
    columns: Vec<TreeColumn>,
    widths:  Vec<u16>,
}

/// A full column fit, or a request for the compressed presentation.
enum TreeColumnFit {
    Fitted(FittedTreeColumns),
    TooNarrow,
}

impl FittedTreeColumns {
    /// Fit the full view without truncating any name.
    fn fitted(
        entry: &AgentEntry<'_>,
        width: u16,
        now: u64,
        account_column: AccountColumn,
    ) -> TreeColumnFit {
        let heading_fits = match &*entry.sizing.borrow() {
            TreeSizing::Shared(metrics) => {
                matches!(metrics.presentation, TreePresentation::Full)
                    && metrics.heading_width <= width
            },
            TreeSizing::Local => [header_line(entry, account_column), place_line(entry.row)]
                .iter()
                .all(|line| line.width() <= usize::from(width)),
        };
        if !heading_fits {
            return TreeColumnFit::TooNarrow;
        }
        match CellView::tree_at_width(
            &entry.lines,
            entry.sort,
            header_width(width),
            now,
            account_column,
            SessionHeaderLead::OverBranch,
            &entry.sizing.borrow(),
        ) {
            CellView::Full(fitted) => TreeColumnFit::Fitted(fitted),
            CellView::Compressed => TreeColumnFit::TooNarrow,
        }
    }
}

/// Rows requested at this width before allocation-dependent height compression.
pub(crate) fn height(
    entry: &AgentEntry<'_>,
    width: u16,
    now: u64,
    account_column: AccountColumn,
) -> usize {
    view_height(
        entry,
        width,
        &CellView::requested_at_width(entry, width, now, account_column),
        account_column,
    )
}

/// Above-tree lines followed immediately by a header and lines, or the empty note.
fn view_height(
    entry: &AgentEntry<'_>,
    width: u16,
    view: &CellView,
    account_column: AccountColumn,
) -> usize {
    above_tree(
        entry,
        &[],
        width,
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
            |hue| Role::Session(hue).style(),
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
    let shared;
    let entry = if matches!(*entry.sizing.borrow(), TreeSizing::Local) && cells.len() > 1 {
        let entries = iter::once(entry).chain(cells.iter().filter(|cell| cell.id != entry.id));
        let measurements = shared_tree_metrics(
            entries.map(|cell| TreeMeasurement::for_entry(cell, area, now, account_column)),
        );
        shared = entry.clone();
        *shared.sizing.borrow_mut() = TreeSizing::Shared(measurements[0].clone());
        &shared
    } else {
        entry
    };
    let view = CellView::fitted(entry, area, now, account_column);
    let above = above_tree(
        entry,
        cells,
        area.width,
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
                TreeHeaderLayout {
                    view:   &view,
                    lead:   SessionHeaderLead::OverBranch,
                    sizing: &entry.sizing.borrow(),
                },
            ),
            &TreeDraw {
                now,
                footer_colors,
                offset: entry.viewport.borrow().scroll_offset(),
            },
        );
    }
}

/// Header and table layout for a flat pool, using the same tree column fitter.
fn pooled_header(
    entry: &PooledEntry<'_>,
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
        &entry.sizing.borrow(),
    );
    tree_header(
        &entry.lines,
        entry.sort,
        tree,
        now,
        account_column,
        TreeHeaderLayout {
            view:   &view,
            lead:   SessionHeaderLead::Flush,
            sizing: &entry.sizing.borrow(),
        },
    )
}

/// The pool has one header followed by its flat roots and nested work.
pub(crate) fn pooled_height(entry: &PooledEntry<'_>) -> usize {
    usize::from(TABLE_HEADER_HEIGHT) + entry.lines.len()
}

/// Resolved header cells for clicks in a pooled tile.
pub(crate) fn pooled_header_spans(
    entry: &PooledEntry<'_>,
    area: Rect,
    now: u64,
    account_column: AccountColumn,
) -> Vec<HeaderSpan<TreeColumn>> {
    pooled_header(entry, area, now, account_column).spans
}

/// Draw the pool with the shared tree table, palette and sorted header.
pub(crate) fn draw_pooled(
    buffer: &mut Buffer,
    area: Rect,
    entry: &PooledEntry<'_>,
    now: u64,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) {
    draw_tree(
        buffer,
        header_indented(area),
        &entry.lines,
        entry.sort,
        pooled_header(entry, area, now, account_column),
        &TreeDraw {
            now,
            footer_colors,
            offset: entry.viewport.borrow().scroll_offset(),
        },
    );
}

/// Widest header or value in each active column.
fn tree_widths(
    lines: &[TreeLine<'_>],
    columns: &[TreeColumn],
    now: u64,
    sort: TreeSort,
    lead: SessionHeaderLead,
    sizing: &TreeSizing,
) -> Vec<u16> {
    let ages = AgeLayout::from_ages(lines.iter().map(|line| now.saturating_sub(line.started())));
    columns
        .iter()
        .map(|&column| {
            if matches!(
                column,
                TreeColumn::Session | TreeColumn::Role | TreeColumn::Activity | TreeColumn::Status
            ) && let TreeSizing::Shared(metrics) = sizing
            {
                return metrics.widths[column as usize];
            }
            lines
                .iter()
                .map(|line| summary::cell_width(&line.cell_text(column, now, &ages)))
                .max()
                .unwrap_or(0)
                .max(summary::cell_width(&column.sorted_header_text(sort, lead)))
                .max(match sizing {
                    TreeSizing::Local => 0,
                    TreeSizing::Shared(metrics) => metrics.widths[column as usize],
                })
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

/// Give the table's unused width to the directory, or the session when absent.
fn fill_tree_width(columns: &[TreeColumn], widths: &mut [u16], width: u16, spacing: u16) {
    let used = widths.iter().copied().map(usize::from).sum::<usize>()
        + columns.len().saturating_sub(1) * usize::from(spacing);
    let slack = width.saturating_sub(u16::try_from(used).unwrap_or(u16::MAX));
    let expanding = columns
        .iter()
        .position(|column| *column == TreeColumn::Directory)
        .or_else(|| {
            columns
                .iter()
                .position(|column| *column == TreeColumn::Session)
        })
        .unwrap_or(0);
    widths[expanding] = widths[expanding].saturating_add(slack);
}

/// Fitted widths and their resolved header cells are one layout decision.
struct TreeHeader {
    ages:    AgeLayout,
    /// Session label alignment selected by the tile kind.
    lead:    SessionHeaderLead,
    /// Columns selected by the fitted view.
    columns: Vec<TreeColumn>,
    /// Columns with headers and values in this tile.
    visible: PermittedTreeColumns,
    /// Widths shared with the table renderer.
    widths:  Vec<u16>,
    /// Gap between the resolved columns.
    spacing: u16,
    /// Visible header cells for click mapping.
    spans:   Vec<HeaderSpan<TreeColumn>>,
}

/// The fitted presentation and shared measurements used by drawing and hit testing.
#[derive(Clone, Copy)]
struct TreeHeaderLayout<'a> {
    view:   &'a CellView,
    lead:   SessionHeaderLead,
    sizing: &'a TreeSizing,
}

/// Measure and fit a tree when no shared group metrics were prepared.
fn local_tree_metrics(
    lines: &[TreeLine<'_>],
    sort: TreeSort,
    tree: Rect,
    now: u64,
    account_column: AccountColumn,
    view: &CellView,
    lead: SessionHeaderLead,
) -> TreeMetrics {
    let mut measurement = TreeMeasurement::measure(
        lines,
        tree,
        now,
        sort,
        account_column,
        lead,
        TreeHeadingSpace {
            offset: 0,
            width:  0,
            rows:   PresentationValues {
                full:       0,
                compressed: 0,
            },
        },
    );
    if matches!(view, CellView::Compressed) {
        measurement.metrics.presentation = TreePresentation::Compressed;
    }
    measurement.metrics.widths[TreeColumn::Age as usize] = measurement.metrics.widths
        [TreeColumn::Age as usize]
        .max(u16::try_from(measurement.metrics.ages.width()).unwrap_or(u16::MAX));
    for step in GroupFitStep::ORDER {
        let activity = measurement
            .activity_widths
            .get(measurement.metrics.presentation);
        measurement.metrics.set_activity_width(activity);
        measurement.metrics.widths[TreeColumn::Role as usize] = measurement
            .role_widths
            .get(measurement.metrics.presentation);
        measurement.metrics.widths[TreeColumn::Status as usize] = measurement
            .status_widths
            .get(measurement.metrics.presentation);
        measurement.metrics.fit_step(&step, tree.width);
    }
    let activity = measurement
        .activity_widths
        .get(TreePresentation::Compressed);
    measurement.metrics.set_activity_width(activity);
    measurement.metrics.widths[TreeColumn::Role as usize] =
        measurement.role_widths.get(TreePresentation::Compressed);
    measurement.metrics.widths[TreeColumn::Status as usize] =
        measurement.status_widths.get(TreePresentation::Compressed);
    measurement.metrics
}

/// Keep the session header and tree guides before reducing the final gaps.
fn fit_tree_names(
    columns: &[TreeColumn],
    widths: &mut [u16],
    minimum_session: u16,
    width: u16,
) -> u16 {
    let gaps = u16::try_from(columns.len().saturating_sub(1)).unwrap_or(u16::MAX);
    let session = columns
        .iter()
        .position(|column| *column == TreeColumn::Session)
        .unwrap_or(0);
    let fixed = widths
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != session)
        .map(|(_, width)| usize::from(*width))
        .sum::<usize>();
    let fixed = u16::try_from(fixed).unwrap_or(u16::MAX);
    let spacing = TABLE_COLUMN_SPACING
        .min(width.saturating_sub(minimum_session.saturating_add(fixed)) / gaps.max(1));
    let budget = width.saturating_sub(fixed.saturating_add(spacing.saturating_mul(gaps)));
    widths[session] = widths[session].min(budget).max(minimum_session);
    spacing
}

/// Resolve columns from the view already fitted by drawing or click mapping.
fn tree_header(
    lines: &[TreeLine<'_>],
    sort: TreeSort,
    tree: Rect,
    now: u64,
    account_column: AccountColumn,
    layout: TreeHeaderLayout<'_>,
) -> TreeHeader {
    let TreeHeaderLayout { view, lead, sizing } = layout;
    let local_metrics;
    let metrics = match sizing {
        TreeSizing::Shared(metrics) => metrics,
        TreeSizing::Local => {
            local_metrics = local_tree_metrics(lines, sort, tree, now, account_column, view, lead);
            &local_metrics
        },
    };
    let columns = match metrics.alignment {
        TreeColumnAlignment::Measured => metrics.columns.columns(),
        TreeColumnAlignment::Fitted { slots, .. } => slots.columns(),
    };
    let mut widths: Vec<_> = columns
        .iter()
        .map(|column| match metrics.alignment {
            TreeColumnAlignment::Measured => metrics.widths[*column as usize],
            TreeColumnAlignment::Fitted { widths, .. } => widths[*column as usize],
        })
        .collect();
    let session = columns
        .iter()
        .position(|column| *column == TreeColumn::Session)
        .unwrap_or(0);
    let spacing = match metrics.alignment {
        TreeColumnAlignment::Fitted { spacing, .. } => spacing,
        TreeColumnAlignment::Measured => fit_tree_names(
            &columns,
            &mut widths,
            metrics.minimum_session_width(),
            tree.width,
        ),
    };
    // The header can use the guide cells when its indented label does not fit.
    let lead = if widths[session] < metrics.session_header_width() {
        SessionHeaderLead::Flush
    } else {
        lead
    };
    // Resolve the group's age slots before giving any remaining width to its
    // directory, or to its session column when the directory is absent.
    let ages = match sizing {
        TreeSizing::Local => {
            AgeLayout::from_ages(lines.iter().map(|line| now.saturating_sub(line.started())))
        },
        TreeSizing::Shared(metrics) => metrics.ages.clone(),
    };
    if let Some(index) = columns.iter().position(|column| *column == TreeColumn::Age) {
        widths[index] = u16::try_from(ages.width())
            .unwrap_or(u16::MAX)
            .max(summary::cell_width(
                &TreeColumn::Age.sorted_header_text(sort, lead),
            ))
            .max(metrics.widths[TreeColumn::Age as usize]);
    }
    if matches!(sizing, TreeSizing::Local) {
        fill_tree_width(&columns, &mut widths, tree.width, spacing);
    }
    let mut x = tree.x;
    let rects: Vec<_> = widths
        .iter()
        .map(|&width| {
            let rect = Rect::new(x, tree.y, width, tree.height).intersection(tree);
            x = x.saturating_add(width).saturating_add(spacing);
            rect
        })
        .collect();
    let spans = if lines.is_empty() || tree.is_empty() {
        Vec::new()
    } else {
        sort::column_spans(&columns, &rects, tree.y)
            .into_iter()
            .filter(|span| metrics.columns.0.0[span.column as usize])
            .collect()
    };
    TreeHeader {
        ages,
        lead,
        columns,
        visible: metrics.columns,
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
        TreeHeaderLayout {
            view:   &view,
            lead:   SessionHeaderLead::OverBranch,
            sizing: &entry.sizing.borrow(),
        },
    )
    .spans
}

/// Draw both views with one table, preserving fixed columns when names compress.
struct TreeDraw<'a> {
    offset:        usize,
    now:           u64,
    footer_colors: &'a FooterColors,
}

/// One fitted tree field, preserving styles and never truncating a number or status.
fn tree_cell_line(
    line: &TreeLine<'_>,
    column: TreeColumn,
    width: u16,
    ages: &AgeLayout,
    frame: &TreeDraw<'_>,
) -> Line<'static> {
    let now = frame.now;
    let footer_colors = frame.footer_colors;
    if column == TreeColumn::Session {
        let lead = line.lead();
        let remaining = width.saturating_sub(summary::cell_width(&lead));
        let mut value = tree_value_line(
            line,
            column,
            summary::truncated(&line.name(), usize::from(remaining)),
            now,
            footer_colors,
        );
        value
            .spans
            .insert(0, Span::styled(lead, SessionRowInk::Text.style()));
        value
    } else if column == TreeColumn::Activity {
        activity_line(
            &line.activity_parts(now),
            usize::from(width),
            line.job_style(now, footer_colors),
        )
    } else {
        let value = match column {
            TreeColumn::Role | TreeColumn::Branch | TreeColumn::Directory => {
                summary::truncated(&line.cell_text(column, now, ages), usize::from(width))
            },
            _ => line.cell_text(column, now, ages),
        };
        tree_value_line(line, column, value, now, footer_colors)
    }
}

/// Tile cells and details values share the same ink rules.
fn tree_value_line(
    line: &TreeLine<'_>,
    column: TreeColumn,
    value: String,
    now: u64,
    colors: &FooterColors,
) -> Line<'static> {
    let text = SessionRowInk::Text.style();
    if column == TreeColumn::Account {
        return match line.account() {
            Account::Unrecorded => Line::styled(value, text),
            Account::Home(home) => {
                let (account, tier) = value.split_at(home.label().len());
                Line::from(vec![
                    Span::styled(account.to_owned(), colors.account(home.label()).style(text)),
                    Span::styled(tier.to_owned(), Role::Codex.style()),
                ])
            },
        };
    }
    if column == TreeColumn::Activity && value != MISSING_VALUE {
        return activity_line(
            &line.job_and_live_word(now),
            usize::MAX,
            line.job_style(now, colors),
        );
    }
    let style = match column {
        TreeColumn::Session => line.name_style(),
        TreeColumn::Status => line.session_status().color_role().style(),
        TreeColumn::Role => line.footer_role(colors).style(text),
        TreeColumn::Activity => line.job_style(now, colors),
        _ => text,
    };
    Line::styled(value, style)
}

/// An activity value: the job in its ink, then the live word that `parts` holds.
/// A narrow cell cuts the job first and keeps the live word whole.
fn activity_line(parts: &ActivityParts, width: usize, job_style: Style) -> Line<'static> {
    let Some(live) = parts.live else {
        return Line::styled(summary::truncated(&parts.job, width), job_style);
    };
    let word = AgentActivity::Active(live).text();
    let word_style = match live {
        Activity::Waiting => Role::Waiting.style(),
        _ => job_style,
    };
    let job_width = width.saturating_sub(usize::from(
        summary::cell_width(word).saturating_add(summary::cell_width(HEADING_SEPARATOR)),
    ));
    if parts.job.is_empty() || job_width == 0 {
        return Line::styled(summary::truncated(word, width), word_style);
    }
    Line::from(vec![
        Span::styled(summary::truncated(&parts.job, job_width), job_style),
        Span::styled(HEADING_SEPARATOR, job_style),
        Span::styled(word, word_style),
    ])
}

/// One activity cell's text: the job, and the live word of a busy row.
struct ActivityParts {
    job:  String,
    live: Option<Activity>,
}

impl ActivityParts {
    /// The narrowest width that keeps the live word whole: a busy row's job can be cut.
    fn floor_width(&self) -> u16 {
        self.live.map_or_else(
            || summary::cell_width(&self.job),
            |live| summary::cell_width(AgentActivity::Active(live).text()),
        )
    }

    /// The joined text that the tile, the details view and `census --json` show.
    fn text(&self) -> String {
        match self.live.map(|live| AgentActivity::Active(live).text()) {
            None => self.job.clone(),
            Some(word) if self.job.is_empty() => word.to_owned(),
            Some(word) => format!("{}{HEADING_SEPARATOR}{word}", self.job),
        }
    }
}

fn draw_tree(
    buffer: &mut Buffer,
    area: Rect,
    lines: &[TreeLine<'_>],
    sort: TreeSort,
    header: TreeHeader,
    frame: &TreeDraw<'_>,
) {
    let offset = frame.offset;
    if area.is_empty() {
        return;
    }
    let TreeHeader {
        ages,
        lead,
        columns,
        visible,
        widths,
        spacing,
        spans,
    } = header;
    debug_assert!(spans.iter().all(|span| span.row == area.y));
    let mut x = area.x;
    for (index, &column) in columns.iter().enumerate() {
        let width = widths[index];
        let column_area = Rect::new(x, area.y, width, area.height).intersection(area);
        x = x.saturating_add(width).saturating_add(spacing);
        if column_area.is_empty() || !visible.0.0[column as usize] {
            continue;
        }
        let heading =
            summary::truncated(&column.sorted_header_text(sort, lead), usize::from(width));
        Paragraph::new(
            Line::styled(heading, Style::default().fg(label_color())).alignment(column.alignment()),
        )
        .render(
            Rect {
                height: TABLE_HEADER_HEIGHT.min(column_area.height),
                ..column_area
            },
            buffer,
        );
        for (row, line) in lines
            .iter()
            .skip(offset)
            .take(usize::from(area.height.saturating_sub(TABLE_HEADER_HEIGHT)))
            .enumerate()
        {
            let value = tree_cell_line(line, column, width, &ages, frame);
            Paragraph::new(value.alignment(column.alignment())).render(
                Rect::new(
                    column_area.x,
                    area.y
                        .saturating_add(TABLE_HEADER_HEIGHT)
                        .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
                    column_area.width,
                    1,
                ),
                buffer,
            );
        }
    }
}
/// The decoration rows shared by slice measurement and painting.
enum TreeHeadingRow {
    Heading,
    Place,
    Launcher(u32),
}

fn above_tree_rows(row: &AgentRow, presentation: TreePresentation) -> Vec<TreeHeadingRow> {
    match (presentation, row.launched_by) {
        (TreePresentation::Compressed | TreePresentation::HeightCompressed, None) => Vec::new(),
        (_, None) => vec![TreeHeadingRow::Heading, TreeHeadingRow::Place],
        (_, Some(launcher)) => vec![
            TreeHeadingRow::Heading,
            TreeHeadingRow::Place,
            TreeHeadingRow::Launcher(launcher),
        ],
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
    view: &CellView,
    account_column: AccountColumn,
    footer_colors: &FooterColors,
) -> Vec<Line<'static>> {
    let row = entry.row;
    let presentation = match view {
        CellView::Full(_) => TreePresentation::Full,
        CellView::Compressed => TreePresentation::Compressed,
    };
    above_tree_rows(row, presentation)
        .into_iter()
        .map(|row_kind| {
            let line = match row_kind {
                TreeHeadingRow::Heading => {
                    AgentHeading::new(entry, account_column, footer_colors).fitted(width)
                },
                TreeHeadingRow::Place => place_line(row),
                TreeHeadingRow::Launcher(launcher) => {
                    let name = entry
                        .launcher
                        .map_or_else(|| format!("{PID_LABEL} {launcher}"), str::to_string);
                    Line::from(vec![
                        Span::raw(SECTION_HEADER_INDENT),
                        Span::styled(
                            format!("{LAUNCHED_BY_LABEL} "),
                            SessionRowInk::LauncherLabel.style(),
                        ),
                        Span::styled(name, name_style(cells, entry.machine, launcher)),
                    ])
                },
            };
            cut_line(line.spans, width)
        })
        .collect()
}

/// The heading shows identity, reported status, machine, and desktop.
fn header_line(entry: &AgentEntry<'_>, account_column: AccountColumn) -> Line<'static> {
    AgentHeading::new(entry, account_column, &FooterColors::default()).full()
}

/// Identity and reported status stay together; the location suffix can be omitted.
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
        entry: &AgentEntry<'_>,
        account_column: AccountColumn,
        footer_colors: &FooterColors,
    ) -> Self {
        let row = entry.row;
        let text = Style::default().fg(text_default());
        let separator = summary::separator;
        let mut spans = vec![
            Span::raw(SECTION_HEADER_INDENT),
            Span::styled(format!("{PID_LABEL} {}", row.pid), text),
            separator(),
        ];
        spans.extend(summary::agent_label(row, account_column, footer_colors).spans);
        if let ReportedSessionState::Reported(_) = entry.source_state {
            spans.push(separator());
            spans.push(Span::styled(
                entry.source_state.text(),
                entry.source_state.color_role().style(),
            ));
        }
        Self {
            identity: Line::from(spans),
            location: Line::from(vec![
                Span::styled(entry.machine.to_string(), text),
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
    summary::truncated_line(Line::from(spans), usize::from(width))
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

impl AgentTile<'_> {
    /// Apply expansion after sorting and color assignment.
    pub(crate) fn apply_expansion(&mut self, expansion: &HandlerExpansion) {
        let level = expansion.level(&self.key());
        let lines = match self {
            Self::Agent(entry) => {
                entry.lines.insert(0, tile_root_line(entry));
                &mut entry.lines
            },
            Self::Pooled(entry) => &mut entry.lines,
        };
        let tree = expansion_tree(lines).with_level(level);
        let mut visibility = tree.visibility(expansion);
        lines.retain_mut(|line| match visibility.next() {
            Some(RowVisibility::Visible(marker)) => {
                line.expansion = marker;
                true
            },
            _ => false,
        });
    }

    /// Describe the complete sorted forest before visibility filtering.
    pub(crate) fn expansion_tree(&self) -> HandlerTree {
        match self {
            Self::Agent(entry) => {
                let mut lines = vec![tile_root_line(entry)];
                lines.extend(entry.lines.iter().cloned());
                expansion_tree(&lines)
            },
            Self::Pooled(entry) => expansion_tree(&entry.lines),
        }
    }
}

fn expansion_tree(lines: &[TreeLine<'_>]) -> HandlerTree {
    let mut ancestors = Vec::new();
    let mut rows = Vec::new();
    for line in lines {
        let depth = if matches!(line.branch, Branch::Flat) {
            0
        } else {
            line.rails.len() + 1
        };
        ancestors.truncate(depth);
        let key = match line.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                ExpansionKey::process(line.machine, line.row.pid)
            },
            LineSubject::ManagedThread(child, _, _) | LineSubject::Work(Work { child, .. }) => {
                child.pid.map_or_else(
                    || ExpansionKey::Work(line.identity.clone()),
                    |pid| ExpansionKey::process(line.machine, pid),
                )
            },
        };
        rows.push(HandlerTreeRow {
            identity: line.identity.clone(),
            key,
            depth,
            parent: ancestors
                .last()
                .copied()
                .map_or(RowParent::Root, RowParent::Parent),
        });
        ancestors.push(rows.len() - 1);
    }
    crate::app::expansion::HandlerTree::new(rows)
}

/// Build complete trees for census pruning.
pub(crate) fn tile_order_for_expansion<'a>(placement: &AgentPlacement<'a>) -> Vec<AgentTile<'a>> {
    let agents = cell_order_placed(placement);
    group_tiles(placement, &agents, &HashMap::new(), 0)
}

/// Resolve details through the same tree identities that selection uses.
#[cfg(test)]
pub(crate) fn row_details(
    placement: &AgentPlacement<'_>,
    selected: &SelectedRow,
    now: u64,
) -> RowDetailsLookup {
    row_details_with_inks(placement, selected, now, &FooterColors::default())
}

/// Retain the tile inks with the selected row's values.
pub(crate) fn row_details_with_inks(
    placement: &AgentPlacement<'_>,
    selected: &SelectedRow,
    now: u64,
    colors: &FooterColors,
) -> RowDetailsLookup {
    let entries = cell_order_placed(placement);
    for entry in &entries {
        let root = tile_root_line(entry);
        for line in iter::once(&root).chain(entry.lines.iter()) {
            if &line.identity != selected {
                continue;
            }
            let mut details = line.details(now);
            line.hierarchy_details(placement, &mut details.properties);
            details.properties.retain(|(property, value)| {
                matches!(
                    property,
                    DetailProperty::Effort | DetailProperty::Activity | DetailProperty::Status
                ) || (!value.is_empty() && value != MISSING_VALUE)
            });
            details.properties.sort_by_key(|(property, _)| *property);
            for column in FULL_ACCOUNTS {
                let property = column.detail_property();
                if property == DetailProperty::Age {
                    continue;
                }
                if let Some((_, value)) =
                    details.properties.iter().find(|(key, _)| *key == property)
                {
                    details.value_lines.insert(
                        property,
                        tree_value_line(line, *column, value.clone(), now, colors),
                    );
                }
            }
            return RowDetailsLookup::Found(details);
        }
    }
    RowDetailsLookup::Unavailable
}

impl TreeLine<'_> {
    /// Read hierarchy names from the accepted placement chain.
    fn hierarchy_details(
        &self,
        placement: &AgentPlacement<'_>,
        properties: &mut Vec<(DetailProperty, String)>,
    ) {
        if matches!(
            self.subject,
            LineSubject::Work(Work {
                kind: WorkKind::Shell(_),
                ..
            })
        ) {
            return;
        }
        let mut named = Vec::new();
        if matches!(
            self.subject,
            LineSubject::ManagedThread(_, _, _) | LineSubject::Work(_)
        ) {
            named.push((DetailProperty::Worker, self.name()));
        }
        let mut identity = match self.subject {
            LineSubject::ManagedThread(_, _, manager) => manager,
            LineSubject::TileRoot(_)
            | LineSubject::Agent(_)
            | LineSubject::UnassignedSession
            | LineSubject::Work(_) => RowIdentity {
                machine: self.machine,
                pid:     self.row.pid,
            },
        };
        while let Some(agent) = placement
            .rows
            .iter()
            .find(|agent| agent.identity == identity)
        {
            let role = match agent.placement {
                Placement::Managed { role, .. } => RowRole::from_hierarchy(role),
                Placement::Root | Placement::Launched(_) => match placement.role_label(identity) {
                    RoleLabel::Stated(role) | RoleLabel::Inferred(role) => {
                        RowRole::from_hierarchy(role)
                    },
                    RoleLabel::Unlabeled | RoleLabel::ReviewFallback => RowRole::Unlabeled,
                },
            };
            let property = match role {
                RowRole::Showrunner => Some(DetailProperty::Showrunner),
                RowRole::Unit => Some(DetailProperty::Unit),
                RowRole::Worker => Some(DetailProperty::Worker),
                RowRole::Unlabeled | RowRole::Producer => None,
            };
            if let Some(property) = property
                && !named.iter().any(|(existing, _)| *existing == property)
            {
                let name = if identity
                    == (RowIdentity {
                        machine: self.machine,
                        pid:     self.row.pid,
                    })
                    && named.is_empty()
                {
                    self.name()
                } else {
                    agent.row.name.clone()
                };
                named.push((property, name));
            }
            match agent.placement {
                Placement::Managed { manager, .. } => identity = manager,
                Placement::Root | Placement::Launched(_) => break,
            }
        }
        if !named.is_empty() {
            properties.retain(|(property, _)| *property != DetailProperty::Name);
            properties.extend(named);
        }
    }

    /// The job and the live word of any status, or the missing mark when the row has none.
    fn details_activity(&self, now: u64) -> String {
        let text = self.job_and_live_word(now).text();
        if text.is_empty() {
            MISSING_VALUE.to_owned()
        } else {
            text
        }
    }

    fn details(&self, now: u64) -> RowDetails {
        let mut properties = vec![
            (
                DetailProperty::Account,
                self.value(TreeColumn::Account, now),
            ),
            (DetailProperty::Name, self.name()),
            (DetailProperty::Role, self.role_text().to_owned()),
            (DetailProperty::Activity, self.details_activity(now)),
            (
                DetailProperty::Status,
                self.session_status().text().to_owned(),
            ),
        ];
        for column in [
            TreeColumn::Type,
            TreeColumn::Model,
            TreeColumn::Effort,
            TreeColumn::Branch,
            TreeColumn::Directory,
        ] {
            if matches!(column, TreeColumn::Branch | TreeColumn::Directory)
                && matches!(
                    self.subject,
                    LineSubject::Work(_) | LineSubject::ManagedThread(_, _, _)
                )
            {
                continue;
            }
            let value = self.value(column, now);
            if column == TreeColumn::Effort || (!value.is_empty() && value != MISSING_VALUE) {
                properties.push((
                    column.detail_property(),
                    if value.is_empty() {
                        MISSING_VALUE.to_owned()
                    } else {
                        value
                    },
                ));
            }
        }
        let pid = match self.subject {
            LineSubject::Work(work) => work.child.pid,
            LineSubject::ManagedThread(child, _, _) => child.pid,
            _ => Some(self.row.pid),
        };
        if let Some(pid) = pid {
            properties.push((DetailProperty::Pid, pid.to_string()));
        }
        match self.subject {
            LineSubject::TileRoot(_) | LineSubject::Agent(_) | LineSubject::UnassignedSession => {
                if !self.row.directory.is_empty()
                    && !properties
                        .iter()
                        .any(|(property, _)| *property == DetailProperty::Directory)
                {
                    properties.push((DetailProperty::Directory, self.row.directory.clone()));
                }
                if let Some(branch) = &self.row.branch
                    && !properties
                        .iter()
                        .any(|(property, _)| *property == DetailProperty::Branch)
                {
                    properties.push((DetailProperty::Branch, branch.clone()));
                }
                if let Some(desktop) = &self.row.desktop {
                    properties.push((DetailProperty::Desktop, desktop.clone()));
                }
            },
            LineSubject::Work(_) | LineSubject::ManagedThread(_, _, _) => {},
        }
        properties.push((
            DetailProperty::Age,
            age::long_age_label(now.saturating_sub(self.started())),
        ));
        RowDetails {
            properties,
            value_lines: BTreeMap::new(),
        }
    }
}

/// Read a fixture property through the production details builder.
#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "the fixture row and property must exist"
)]
pub(crate) fn detail_property_for_test(
    machines: &[Machine<'_>],
    name: &str,
    property: &str,
    now: u64,
) -> String {
    let placement = AgentPlacement::from_machines(machines, &EMPTY_SESSION_STATES);
    let entries = cell_order_placed(&placement);
    let line = entries
        .iter()
        .flat_map(|entry| iter::once(tile_root_line(entry)).chain(entry.lines.iter().cloned()))
        .find(|line| line.name() == name)
        .expect("fixture row");
    line.details(now).value(property)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::error::Error;

    use ratatui::style::Color;

    use super::*;
    use crate::census::Account;
    use crate::census::CodexThreadId;
    use crate::census::StaffRowNames;

    #[path = "activity_tests.rs"]
    mod activity_tests;
    #[path = "column_visibility_tests.rs"]
    mod column_visibility_tests;
    #[path = "design_ink_tests.rs"]
    mod design_ink_tests;
    #[path = "footer_color_tests.rs"]
    mod footer_color_tests;
    #[path = "producers_tests.rs"]
    mod producers_tests;
    #[path = "repair_tests.rs"]
    mod repair_tests;
    #[path = "sort_tests.rs"]
    mod sort_tests;
    #[path = "subagent_row_tests.rs"]
    mod subagent_row_tests;
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
    use crate::census::RoleFallback;
    use crate::census::ServiceTier;
    use crate::census::SessionIdentity;
    use crate::census::WorkIdentity;
    use crate::census::model::ModelName;
    use crate::census::model::ReasoningEffort;
    use crate::census::scan::ScanStage;
    use crate::census::transcript::SubagentRole;
    use crate::census::worker_dispatch::WorkerDispatch;
    use crate::census::worker_dispatch::WorkerTask;
    use crate::constants::TRUNCATION_MARK;

    #[test]
    fn dispatched_workers_draw_staff_names_before_dispatch_labels() {
        let mut owner: AgentRow =
            serde_json::from_str(include_str!("census/fixtures/restarted_unit_codex.json"))
                .expect("dispatched worker");
        let fallback = lines(&drawn(&owner, None, 160)).join("\n");
        assert!(fallback.contains("impl · compile crates"), "{fallback}");
        owner.children[0].name = "staff worker".into();
        owner
            .staff_names
            .threads
            .insert("unit-worker-thread".into(), "staff worker".into());
        let named = lines(&drawn(&owner, None, 160)).join("\n");
        assert!(named.contains("staff worker"), "{named}");
        assert!(!named.contains("impl · compile crates"), "{named}");
    }

    #[test]
    fn a_narrow_activity_cell_cuts_the_job_first_and_keeps_the_live_word_whole() {
        let mut owner: AgentRow =
            serde_json::from_str(include_str!("census/fixtures/restarted_unit_codex.json"))
                .expect("dispatched worker");
        owner.children[0].activity = AgentActivity::Active(Activity::Waiting);
        let mut cell = entry(&owner, None);
        cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Busy);
        let line = &cell.lines[0];
        let colors = FooterColors::default();
        let parts = line.activity_parts(NOW);
        let job_style = line.job_style(NOW, &colors);
        let text = |width: usize| activity_line(&parts, width, job_style).to_string();
        assert!(!parts.job.is_empty(), "the fixture worker has a job");
        let whole = parts.text();
        let whole_width = usize::from(summary::cell_width(&whole));
        assert_eq!(whole, format!("{}{HEADING_SEPARATOR}waiting", parts.job));
        assert_eq!(text(whole_width), whole);
        let cut = text(whole_width - 1);
        assert!(
            cut.ends_with(&format!("{TRUNCATION_MARK}{HEADING_SEPARATOR}waiting")),
            "{cut}"
        );
        assert_eq!(usize::from(summary::cell_width(&cut)), whole_width - 1);
        assert_eq!(text("waiting".len()), "waiting");
        let narrow = activity_line(&parts, whole_width, job_style);
        let word = narrow.spans.last().expect("live word span");
        assert_eq!(word.content, "waiting");
        assert_eq!(word.style, Role::Waiting.style());
    }

    #[test]
    fn a_busy_conversation_with_no_scanned_turn_shows_thinking_and_work_lines_do_not() {
        let mut owner: AgentRow =
            serde_json::from_str(include_str!("census/fixtures/restarted_unit_codex.json"))
                .expect("dispatched worker");
        owner.children[0].activity = AgentActivity::Idle;
        let mut cell = entry(&owner, None);
        cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Busy);
        assert_eq!(
            cell.lines[0].activity_parts(NOW).live,
            Some(Activity::Thinking)
        );
        let mut owner_with_subagent = owner.clone();
        owner_with_subagent.children = vec![
            serde_json::from_value(serde_json::json!(
                {"depth":0,"kind":"subagent","name":"helper","started":110}
            ))
            .expect("subagent child"),
        ];
        owner_with_subagent.children[0].activity = AgentActivity::Idle;
        let cell = entry(&owner_with_subagent, None);
        let subagent = cell
            .lines
            .iter()
            .find(|line| line.name() == "helper")
            .expect("subagent line");
        assert_eq!(subagent.activity_parts(NOW).live, None);
    }

    /// Every cell across `machines`, in the order the grid shows them:
    /// machine by machine, one for each top-level agent, holding the
    /// sessions it opened, depth first and oldest first. The agents that
    /// opened sessions come first, then the rest, each oldest first. A
    /// session whose launcher is not listed stands as a top-level agent.
    /// Each agent takes the next hue of the rainbow in that order, a
    /// session as much as the agent titling its cell.
    fn cell_order<'a>(machines: &[Machine<'a>]) -> Vec<AgentEntry<'a>> {
        cell_order_placed(&AgentPlacement::from_machines(
            machines,
            &EMPTY_SESSION_STATES,
        ))
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
            model: ModelName::Unrecorded,
            effort: ReasoningEffort::Unrecorded,
            staff_names: StaffRowNames::default(),
            role_fallback: RoleFallback::None,
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
            model: ModelName::Unrecorded,
            effort: ReasoningEffort::Unrecorded,
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
            work_id: WorkIdentity::Legacy,
        }
    }

    /// tool-based-ui-trunk, a session boss opened, on the `berth_fix`
    /// desktop: a Codex app server that has left its shell, a shell
    /// running a Codex app server with a thread, a subagent running a
    /// shell whose command is its name, and the session it opened,
    /// tool-based-ui-arrange.
    fn trunk() -> AgentRow {
        AgentRow {
            role_fallback: RoleFallback::None,
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
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
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
                    ChildKind::Subagent(SubagentRole::Unrecorded),
                    None,
                    "Review the permission queue",
                    5 * MINUTE + 3,
                ),
                child(
                    1,
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
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
            source_state: ReportedSessionState::Unavailable,
            sizing: RefCell::new(TreeSizing::Local),
            viewport: RefCell::new(tui_pane::Viewport::new()),
            id: AgentCell {
                machine:         "natedev".to_string(),
                pid:             row.pid,
                process_started: row.process_started,
            },
            row,
            launcher,
            machine: "natedev",
            hue: SessionHue::First,
            role_label: RoleLabel::Unlabeled,
            sort: TreeSort::default(),
            lines: {
                let mut lines = Vec::new();
                let placement = AgentPlacement::from_machines(&[], &EMPTY_SESSION_STATES);
                let identities = CensusWorkIdentities::new(
                    &placement,
                    [(
                        RowIdentity {
                            machine: "natedev",
                            pid:     row.pid,
                        },
                        row,
                    )],
                );
                let context = TreeBuild {
                    placement:  &placement,
                    identities: &identities,
                    heading:    "natedev",
                    order:      SiblingOrder::Default,
                    hues:       HueAssignment::DefaultOrder,
                };
                append_work(
                    &context,
                    work_branches(&row.children, &HashSet::new(), 0),
                    row,
                    "natedev",
                    &mut Vec::new(),
                    SessionHue::First,
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

    fn detail_value(entry: &AgentEntry<'_>, name: &str, property: &str, now: u64) -> String {
        let root = tile_root_line(entry);
        let line = iter::once(&root)
            .chain(entry.lines.iter())
            .find(|line| line.name() == name)
            .expect("the fixture row has details");
        line.details(now).value(property)
    }

    /// Full views retain the process heading, workplace and launcher,
    /// then draw retained work with its role and program status colors.
    #[test]
    fn a_cell_draws_its_header_and_what_its_agent_runs() {
        let mut trunk = trunk();
        trunk.children[3].pid = Some(2_372_721);
        for source_state in [
            ReportedSessionState::Reported(SessionState::Busy),
            ReportedSessionState::Reported(SessionState::Idle),
            ReportedSessionState::Reported(SessionState::Paused),
            ReportedSessionState::Reported(SessionState::Exited),
            ReportedSessionState::Unavailable,
        ] {
            let mut cell = entry(&trunk, Some("boss of bosses"));
            cell.source_state = source_state;
            let buffer = drawn_among(&cell, &[], 180);
            let heading = match source_state {
                ReportedSessionState::Reported(state) => format!(
                    " pid 3266367 · claude · {} · natedev · berth_fix",
                    state.text()
                ),
                ReportedSessionState::Unavailable => {
                    " pid 3266367 · claude · natedev · berth_fix".to_owned()
                },
            };
            assert_eq!(
                lines(&buffer),
                [
                    heading.as_str(),
                    " ~/rust/tool-based-ui-trunk",
                    " launched by boss of bosses",
                    "     session                                                                                                                                     role    activity  status  type",
                    " ├── Launch the Phase 1 implementation seat                                                                                                                        busy    shell",
                    " │ └── tool-based-ui-trunk-impl                                                                                                                  worker            idle",
                    " └── Review the permission queue                                                                                                                 worker            busy    subagent",
                    "   └── cargo nextest run -p hana_video --no-fail-fast -- permission_queue                                                                                          busy    shell",
                ]
            );
            if let ReportedSessionState::Reported(state) = source_state {
                let (x, y) = find(&buffer, state.text()).expect("reported heading status");
                assert_eq!(y, 0);
                for offset in 0..summary::cell_width(state.text()) {
                    assert_eq!(
                        Some(buffer[(x + offset, y)].fg),
                        source_state.color_role().style().fg
                    );
                }
            }
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
                ("idle", Role::Idle),
                ("busy", Role::Busy),
            ] {
                let at = find(&buffer, text).expect("styled value");
                assert_eq!(Some(buffer[at].fg), role.style().fg);
            }
            for column in [
                TreeColumn::Session,
                TreeColumn::Type,
                TreeColumn::Role,
                TreeColumn::Activity,
                TreeColumn::Status,
            ] {
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
    }

    /// An agent running nothing says so beneath its heading, and a
    /// launcher that is not listed is named by its pid.
    #[test]
    fn an_agent_running_nothing_says_so() {
        let arrange = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));

        let mut cell = entry(&arrange, None);
        cell.source_state = ReportedSessionState::Reported(SessionState::Idle);
        assert_eq!(
            lines(&drawn_among(&cell, &[], WIDE)),
            [
                " pid 3337048 · claude · idle · natedev · —",
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
            role_fallback: RoleFallback::None,
            agent: Agent::Codex,
            service_tier: ServiceTier::Fast,
            children: vec![ChildRow {
                worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                service_tier: ServiceTier::Standard,
                ..child(0, ChildKind::Thread, None, "trunk mesh", 10 * MINUTE)
            }],
            ..agent(4_039_085, "handler", 12 * MINUTE, None)
        };
        let mut cell = entry(&codex, None);
        cell.lines[0].source_state = ReportedSessionState::Reported(SessionState::Idle);
        let buffer = drawn_among(&cell, &[], WIDE);
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
        let mut cell = entry(&boss, None);
        cell.source_state = ReportedSessionState::Reported(SessionState::Idle);
        let line = u16::try_from(header_line(&cell, AccountColumn::Hidden).width())
            .expect("the line should fit a u16");

        let whole = drawn_among(&cell, &[], line);
        let cut = drawn_among(&cell, &[], line - 1);

        assert_eq!(
            lines(&whole),
            [
                " pid 1579022 · claude · idle · natedev · —",
                " ~/rust/boss of bosses",
                " nothing running",
            ]
        );
        assert_eq!(lines(&cut), [" nothing running"]);
    }

    /// A cell switches to its compressed view when the workplace heading
    /// no longer fits. Shell-only trees omit their empty directory column
    /// in both views while the heading is truncated only when necessary.
    #[test]
    fn a_cell_compresses_where_its_directory_would_be_broken() {
        let arrange = AgentRow {
            role_fallback: RoleFallback::None,
            directory: "~/rust/hana_catalyst/crates/hana_video/src/permission_queue/deep/nested/module/source".to_string(),
            children: vec![child(0, ChildKind::Shell(crate::census::ShellTimerObservation::Untimed), Some(2_424_763), "cargo nextest run", 45)],
            ..agent(ARRANGE, "arrange", 2 * HOUR, Some(BOSS))
        };
        let width = u16::try_from(place_line(&arrange).width()).expect("directory width");
        let whole = drawn(&arrange, None, width);
        let cut = drawn(&arrange, None, width - 1);
        assert_eq!(lines(&whole)[1], format!(" {}", arrange.directory));
        assert!(lines(&cut)[1].ends_with(TRUNCATION_MARK));
        let entry = entry(&arrange, None);
        assert!(matches!(
            CellView::at_width(&entry, width, NOW, AccountColumn::Hidden),
            CellView::Full(_)
        ));
        assert_eq!(
            CellView::at_width(&entry, width - 1, NOW, AccountColumn::Hidden),
            CellView::Compressed
        );
        assert!(find(&whole, "directory").is_none());
        assert!(find(&cut, "directory").is_none());
        assert!(find(&cut, "cargo nextest run").is_some());
    }

    /// A tree whose widest name cannot fit switches to compressed columns;
    /// statuses retain their activity and shell colors.
    #[test]
    fn a_cell_compresses_where_its_tree_would_cut_a_name() {
        let arrange = AgentRow {
            role_fallback: RoleFallback::None,
            children: vec![
                child(
                    0,
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                    Some(2_424_763),
                    "cargo nextest run -p hana_video",
                    45,
                ),
                child(
                    0,
                    ChildKind::Subagent(SubagentRole::Unrecorded),
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
            Some(cut[find(&cut, "busy").expect("shell status")].fg),
            Role::Busy.style().fg
        );
        assert_eq!(
            detail_value(&entry, "Review the permission queue", "activity", NOW),
            MISSING_VALUE
        );
        let subagent_y = find(&cut, "Review the permission").expect("subagent row").1;
        let status_x = find(&cut, "status").expect("status header").0;
        assert_eq!(
            (status_x..status_x + 4)
                .map(|x| cut[(x, subagent_y)].symbol())
                .collect::<String>(),
            "busy"
        );
        assert_eq!(Some(cut[(status_x, subagent_y)].fg), Role::Busy.style().fg);
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
            role_fallback: RoleFallback::None,
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let idle = agent(ARRANGE, "tool-based-ui-arrange", 2 * HOUR, Some(BOSS));
        let natedev = boss_group();
        let group = natedev_cells(&natedev);
        let entries = [
            entry(&trunk, Some("boss of bosses")),
            entry(&idle, Some("boss of bosses")),
            group[0].clone(),
        ];
        let labels: HashSet<String> = iter::once(LAUNCHED_BY_LABEL.to_owned())
            .chain(entries.iter().flat_map(|entry| {
                PermittedTreeColumns::for_lines(&entry.lines, NOW, AccountColumn::Hidden)
                    .columns()
                    .into_iter()
                    .map(|column| column.sorted_header(TreeSort::default()))
            }))
            .collect();
        let mut drawn = HashSet::new();
        for entry in &entries {
            for width in [NARROW, 220] {
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
            role_fallback: RoleFallback::None,
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
                    role_fallback: crate::census::RoleFallback::None,
                    branch: Some("main".to_string()),
                    directory: "~/rust/cargo-liner".to_string(),
                    children: vec![
                        child(
                            0,
                            ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
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
                    role_fallback: crate::census::RoleFallback::None,
                    branch: Some("enh/trunk".to_string()),
                    directory: "~/rust/ui-trunk".to_string(),
                    children: vec![
                        child(
                            0,
                            ChildKind::Subagent(SubagentRole::Unrecorded),
                            None,
                            "Review the permission queue",
                            5 * MINUTE + 3,
                        ),
                        child(
                            1,
                            ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                            Some(2_424_763),
                            "cargo nextest run -p hana_video",
                            45,
                        ),
                        session(TRUNK_IMPL, "trunk-impl", 2 * HOUR),
                    ],
                    ..agent(TRUNK, "tool-based-ui-trunk", 21 * HOUR, Some(BOSS))
                },
                AgentRow {
                    role_fallback: crate::census::RoleFallback::None,
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
                    role_fallback: crate::census::RoleFallback::None,
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
                role_fallback: crate::census::RoleFallback::None,
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
                role_fallback: crate::census::RoleFallback::None,
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
        let mut cells = natedev_cells(&state);
        for line in &mut cells[0].lines {
            line.source_state = ReportedSessionState::Reported(SessionState::Busy);
        }
        cells[0]
            .lines
            .iter_mut()
            .find(|line| matches!(line.subject, LineSubject::Agent(_)))
            .expect("idle thread")
            .source_state = ReportedSessionState::Reported(SessionState::Idle);
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
            Some(buffer[find(&buffer, "busy").expect("busy source status")].fg),
            Role::Busy.style().fg
        );
        assert_eq!(
            Some(buffer[find(&buffer, "idle").expect("idle status")].fg),
            Role::Idle.style().fg
        );
        assert_eq!(
            Some(buffer[find(&buffer, "busy").expect("shell status")].fg),
            Role::Busy.style().fg
        );
    }

    /// A wide cell, where every part of its full view lies flat, asks
    /// for exactly the rows of that view: given more, it draws its
    /// one-line header and its branch and directory, and its last line
    /// of text on the last of the rows [`height`] asked for. One row
    /// short of them, it draws its compressed view.
    #[test]
    fn a_wide_cell_given_its_height_draws_its_full_view() {
        let width = 220;
        let natedev = boss_group();
        let cells = natedev_cells(&natedev);
        let boss = &cells[0];
        assert!(
            matches!(
                CellView::at_width(boss, width, NOW, AccountColumn::Hidden),
                CellView::Full(_)
            ),
            "boss lies flat {width} across"
        );
        let rows = height(boss, width, NOW, AccountColumn::Hidden);
        let header = format!("{SECTION_HEADER_INDENT}{PID_LABEL} {BOSS}");
        let workplace = workplace(boss.row);

        let full = lines(&drawn_into(boss, &cells, width, rows + 1));
        let short = lines(&drawn_into(boss, &cells, width, rows - 1));

        assert_eq!(
            full.iter()
                .rposition(|line| !line.is_empty())
                .map(|last| last + 1),
            Some(rows),
            "{full:#?}"
        );
        assert_eq!(full[..rows], lines(&drawn_among(boss, &cells, width)));
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

    /// A narrow cell keeps complete role and status columns.
    /// Age stays in the details view at each width.
    #[test]
    fn a_narrow_cell_compresses_however_many_rows_it_has() {
        let state = boss_group();
        let cells = natedev_cells(&state);
        let boss = &cells[0];
        let buffer = drawn_into(
            boss,
            &cells,
            33,
            height(boss, 160, NOW, AccountColumn::Hidden),
        );
        let rendered = lines(&buffer);
        assert!(find(&buffer, "directory").is_none());
        assert!(!rendered[0].contains("pid"));
        assert!(!rendered[0].contains("age"), "{rendered:?}");
        assert!(rendered[0].contains("session"), "{rendered:?}");
        assert!(rendered[0].contains("activity"), "{rendered:?}");
        assert!(rendered[0].contains("status"), "{rendered:?}");
        assert_eq!(
            detail_value(boss, "tool-based-ui-trunk", "activity", NOW),
            "thinking"
        );
        let restored = lines(&drawn_into(
            boss,
            &cells,
            35,
            usize::from(buffer.area.height),
        ));
        assert!(!restored[0].contains("age"), "{restored:?}");
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
            role_fallback: RoleFallback::None,
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
                view_height(entry, NARROW, &CellView::Compressed, AccountColumn::Hidden),
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
    /// The compressed view retains the reported status when the location does not fit.
    #[test]
    fn a_narrow_launched_cell_cuts_its_header_to_one_line() {
        let trunk = AgentRow {
            role_fallback: RoleFallback::None,
            directory: LONG_DIRECTORY.to_string(),
            ..trunk()
        };
        let mut cell = entry(&trunk, Some("boss of bosses"));
        cell.source_state = ReportedSessionState::Reported(SessionState::Busy);
        assert_eq!(
            CellView::at_width(&cell, NARROW + 2, NOW, AccountColumn::Hidden),
            CellView::Compressed
        );

        let buffer = drawn_among(&cell, &[], NARROW + 2);

        let mark = TRUNCATION_MARK;
        assert_eq!(
            lines(&buffer)[..3],
            [
                " pid 3266367 · claude · busy · natedev…".to_string(),
                format!(" ~/rust/tool-based-ui-geometry-material{mark}"),
                " launched by boss of bosses".to_string(),
            ]
        );
        assert!(runs_in(&buffer, label_color()).contains(&LAUNCHED_BY_LABEL.to_string()));
    }

    /// A partial location retains its separator and mark; a mark alone is omitted.
    #[test]
    fn launched_heading_keeps_partial_location_and_omits_a_location_without_text() {
        let trunk = trunk();
        for (source_state, width, expected) in [
            (
                ReportedSessionState::Reported(SessionState::Busy),
                33,
                " pid 3266367 · claude · busy · n…",
            ),
            (
                ReportedSessionState::Reported(SessionState::Busy),
                32,
                " pid 3266367 · claude · busy",
            ),
            (
                ReportedSessionState::Reported(SessionState::Busy),
                31,
                " pid 3266367 · claude · busy",
            ),
            (
                ReportedSessionState::Unavailable,
                26,
                " pid 3266367 · claude · n…",
            ),
            (
                ReportedSessionState::Unavailable,
                25,
                " pid 3266367 · claude",
            ),
            (
                ReportedSessionState::Unavailable,
                24,
                " pid 3266367 · claude",
            ),
        ] {
            let mut cell = entry(&trunk, Some("boss of bosses"));
            cell.source_state = source_state;
            let buffer = drawn_among(&cell, &[], width);
            assert_eq!(lines(&buffer)[0], expected, "heading at {width} cells");
        }
    }

    /// A compressed top-level agent begins with its tree header row,
    /// immediately followed by the work lines.
    #[test]
    fn a_compressed_cell_starts_with_its_tree_header() {
        let arrange = AgentRow {
            role_fallback: RoleFallback::None,
            children: vec![
                child(
                    0,
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                    Some(2_424_763),
                    "cargo nextest run -p hana_video",
                    45,
                ),
                child(
                    0,
                    ChildKind::Subagent(SubagentRole::Unrecorded),
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
                        hung("├── ", "trunk"),
                        hung("│ └── ", "under trunk"),
                        hung("└── ", "arrange"),
                    ]
                ),
                ("natedev", "enh/handler", None, vec![]),
                ("natedev", "orphan", None, vec![]),
                (
                    "natedev",
                    "left",
                    Some("right"),
                    vec![hung("└── ", "right")]
                ),
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

    /// Root and descendant sequences each continue across machines without
    /// a subtree consuming the next root's session color.
    #[test]
    fn each_agent_takes_the_next_hue_of_the_rainbow() {
        let (natedev, mac) = natedev_and_mac();

        let hues: Vec<(&str, SessionHue)> = natedev_and_mac_cells(&natedev, &mac)
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
                ("boss of bosses", SessionHue::First),
                ("trunk", SessionHue::First),
                ("under trunk", SessionHue::Second),
                ("arrange", SessionHue::Third),
                ("enh/handler", SessionHue::Second),
                ("orphan", SessionHue::Third),
                ("left", SessionHue::Fourth),
                ("right", SessionHue::Fourth),
                ("natemccoy-30", SessionHue::Fifth),
            ]
        );
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
        assert!(matches!(
            ShellState::resolve(
                crate::census::ShellTimerObservation::ObservedCountdown(past),
                NOW
            ),
            ShellState::Running
        ));
    }

    /// A timer shell retains its description and draws its remaining time
    /// in the shell-colored status column.
    #[test]
    fn a_timer_row_shows_its_time_left_in_shell_ink() {
        let seat = AgentRow {
            role_fallback: RoleFallback::None,
            children: vec![
                child(
                    0,
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                    Some(2_500_000),
                    "Run lint then the full suite",
                    4 * MINUTE,
                ),
                ChildRow {
                    worker_task: crate::census::worker_dispatch::WorkerTask::Unrecorded,
                    kind: ChildKind::Shell(
                        crate::census::ShellTimerObservation::ObservedCountdown(Timer {
                            started:  NOW - 92,
                            deadline: NOW + 228,
                        }),
                    ),
                    ..child(
                        0,
                        ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
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
            let at = find(&buffer, "3:48").expect("timer status");
            assert_eq!(Some(buffer[at].fg), Role::Shell.style().fg);
            assert!(find(&buffer, "Re-arm the progress timer").is_some());
            assert!(
                runs_in(&buffer, Role::Busy.style().fg.expect("running color"))
                    .contains(&"busy".to_string())
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
            role_fallback: RoleFallback::None,
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
                    role_fallback: crate::census::RoleFallback::None,
                    children: vec![child(
                        0,
                        ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                        Some(99),
                        "local work",
                        MINUTE,
                    )],
                    ..identified(7, "local unit", HOUR, None)
                },
            ],
            links(&[("production", "local unit")]).into(),
        );
        let mac = MachineState::Answered(
            vec![AgentRow {
                role_fallback: crate::census::RoleFallback::None,
                children: vec![child(
                    0,
                    ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                    Some(99),
                    "remote work",
                    MINUTE,
                )],
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
                        LineSubject::TileRoot(_)
                        | LineSubject::Agent(_)
                        | LineSubject::UnassignedSession => line.row.name.as_str(),
                        LineSubject::Work(work) => work.child.name.as_str(),
                        LineSubject::ManagedThread(child, _, _) => child.name.as_str(),
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
                    role_fallback: crate::census::RoleFallback::None,
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
                    role_fallback: crate::census::RoleFallback::None,
                    children: vec![child(
                        0,
                        ChildKind::Shell(crate::census::ShellTimerObservation::Untimed),
                        Some(99),
                        "member work",
                        MINUTE,
                    )],
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
                        LineSubject::TileRoot(_)
                        | LineSubject::Agent(_)
                        | LineSubject::UnassignedSession => line.row.name.as_str(),
                        LineSubject::Work(work) => work.child.name.as_str(),
                        LineSubject::ManagedThread(child, _, _) => child.name.as_str(),
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
                role_fallback: crate::census::RoleFallback::None,
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
        assert!(matches!(
            session.subject,
            LineSubject::ManagedThread(_, _, _)
        ));
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
                    role_fallback: crate::census::RoleFallback::None,
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
        let mut cells = natedev_cells(&state);
        for line in &mut cells[0].lines {
            line.source_state = ReportedSessionState::Reported(SessionState::Idle);
        }
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
    fn a_mesh_worker_from_production_classification_renders_without_an_app_server_cell()
    -> Result<(), Box<dyn Error>> {
        let (manager, worker) = crate::census::mesh_worker_fixture();
        let mut cells = natedev_and_mac_cells(&manager, &worker);
        cells[0].lines[0].source_state = ReportedSessionState::Reported(SessionState::Idle);
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
        for include_showrunner in [false, true] {
            let (manager_state, _) = crate::census::mesh_worker_fixture();
            let mut rows = manager_state.rows().to_vec();
            let mut edges = HierarchyEdges::default();
            if include_showrunner {
                rows.push(identified(11, "showrunner", 2 * HOUR, None));
                edges.0.push(HierarchyEdge {
                    manager: "showrunner".into(),
                    member:  "unit".into(),
                    role:    HierarchyRole::Unit,
                });
            }
            let managers = MachineState::Answered(rows, edges.into());
            let machines = [
                Machine {
                    name:  "natedev",
                    state: &managers,
                },
                Machine {
                    name:  "mac",
                    state: &worker,
                },
            ];
            let placement = AgentPlacement::from_machines(&machines, &EMPTY_SESSION_STATES);
            let selected = &cells[0].lines[0].identity;
            let RowDetailsLookup::Found(details) = row_details(&placement, selected, NOW) else {
                return Err("mesh details".into());
            };
            assert_eq!(details.value("unit"), "unit");
            assert_eq!(details.value("worker"), "mesh worker · mac");
            if include_showrunner {
                assert_eq!(details.value("showrunner"), "showrunner");
            }
        }
        Ok(())
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
            [("├── ".to_string(), "b"), ("└── ".to_string(), "worker")]
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
