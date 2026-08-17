//! The one value the Output pane is drawn from.
//!
//! Layout, visibility, focus reconciliation, tabbability, the bottom-row action
//! labels, copy availability, hit testing, and rendering all read
//! [`OutputPresentation`], so none of them can disagree about what the pane is
//! currently showing.

use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;

use tui_pane::format_progressive;

use super::constants::COLUMN_HEADER_HEIGHT;
use super::constants::MINIMUM_READABLE_COLUMN_WIDTH;
use crate::build_monitor::AdditionalBuildExclusion;
use crate::build_monitor::BuildLockContention;
use crate::build_monitor::BuildProfileLabel;
use crate::build_monitor::BuildSessionActivity;
use crate::build_monitor::BuildSessionId;
use crate::build_monitor::BuildTerminationAggregateCompletion;
use crate::build_monitor::BuildTerminationLifecycle;
use crate::build_monitor::BuildTerminationLifecycleRegistry;
use crate::build_monitor::BuildTerminationTerminalRecord;
use crate::build_monitor::BuildTerminationTransactionTargetSet;
use crate::build_monitor::CargoCommandSelector;
use crate::build_monitor::CargoSubcommand;
use crate::build_monitor::CargoSubcommandRecognition;
use crate::build_monitor::MonitorDisplay;
use crate::build_monitor::MonitorSessionOwnership;
use crate::build_monitor::MonitorSessionRow;
use crate::build_monitor::MonitorSnapshot;
use crate::build_monitor::MonitorStaleness;
use crate::build_monitor::RootCpuActivity;
use crate::build_monitor::SessionScope;
use crate::build_monitor::TargetDirectoryEvidence;
use crate::build_monitor::UnattributedCompileActivity;
use crate::project::DisplayPath;
use crate::project::home_relative_path;
use crate::tui::OwnedRunId;
use crate::tui::compile_visibility::CompileVisibilityState;
use crate::tui::compile_visibility::MonitorScopeResolution;
use crate::tui::compile_visibility::MonitorWorkspaceIndexReadiness;
use crate::tui::project_list::ProjectListRowDisplayPathResolution;
use crate::tui::state::OwnedRunCompletionMarker;
use crate::tui::state::OwnedRunOutputStateRef;
use crate::tui::state::OwnedRunOutputTitleRef;
use crate::tui::state::OwnedRunRunningLabelRef;

/// Whether the Output pane occupies the bottom row this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputPaneVisibility {
    /// The diagnostics panes own the bottom row.
    Hidden,
    /// The Output pane is drawn, focusable, and in the tab order.
    Visible,
}

/// Whether the pane currently has anything a copy gesture may read.
///
/// Only Cargo Port's own captured output is copyable: an external column is a
/// live sample of observed processes, not a transcript.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputCopyAvailability {
    /// Nothing on screen is captured output.
    Unavailable,
    /// The owned run's captured output is on screen and may be copied.
    CapturedOutput,
}

/// Why the enabled monitor is showing no build sessions.
///
/// Each state is a different fact about the selected scope, so each renders its
/// own message rather than collapsing into "no sessions".
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorEmptyState {
    /// Enabled with an actionable scope, first cycle not back yet.
    AwaitingFirstCycle,
    /// A cycle came back with no build session in this scope.
    NoBuildSessions,
    /// Nothing observable is left to show for this scope.
    Unavailable,
    /// The workspace index has not yet resolved the selected row.
    PendingIndex(MonitorWorkspaceIndexReadiness),
    /// The selected row is not a Rust checkout, so it covers no roots.
    EmptyNonRust(MonitorWorkspaceIndexReadiness),
    /// More than one checkout claims the selected row's path.
    AmbiguousOwnership(MonitorWorkspaceIndexReadiness),
    /// The selected row's path did not resolve to a canonical checkout.
    UnresolvedPath(MonitorWorkspaceIndexReadiness),
}

impl MonitorEmptyState {
    /// What is drawn in place of columns.
    ///
    /// The four scope-resolution states name the index they were resolved
    /// against: a pending resolution over a retained last-accepted index means
    /// something different to the user than one with no index at all.
    ///
    /// Every part is a `&'static str`, because this is read once per frame in
    /// the render path and an empty monitor is a resting state, not a
    /// transient one.
    pub(super) fn message(self) -> MonitorEmptyStateMessage {
        match self {
            Self::AwaitingFirstCycle => MonitorEmptyStateMessage {
                headline:   "Waiting for the first build scan",
                index_note: MonitorEmptyStateIndexNote::Absent,
            },
            Self::NoBuildSessions => MonitorEmptyStateMessage {
                headline:   "No Cargo build running in this scope",
                index_note: MonitorEmptyStateIndexNote::Absent,
            },
            Self::Unavailable => MonitorEmptyStateMessage {
                headline:   "Build observation unavailable",
                index_note: MonitorEmptyStateIndexNote::Absent,
            },
            Self::PendingIndex(readiness) => MonitorEmptyStateMessage {
                headline:   "Workspace index has not resolved this row yet",
                index_note: MonitorEmptyStateIndexNote::from(readiness),
            },
            Self::EmptyNonRust(readiness) => MonitorEmptyStateMessage {
                headline:   "Selection is not a Rust checkout",
                index_note: MonitorEmptyStateIndexNote::from(readiness),
            },
            Self::AmbiguousOwnership(readiness) => MonitorEmptyStateMessage {
                headline:   "More than one checkout claims this path",
                index_note: MonitorEmptyStateIndexNote::from(readiness),
            },
            Self::UnresolvedPath(readiness) => MonitorEmptyStateMessage {
                headline:   "Path did not resolve to a checkout",
                index_note: MonitorEmptyStateIndexNote::from(readiness),
            },
        }
    }
}

/// An empty monitor message plus row-independent terminal results.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorEmptyPresentation<'a> {
    monitor_empty_state:            MonitorEmptyState,
    termination_lifecycle_registry: &'a BuildTerminationLifecycleRegistry,
}

impl<'a> MonitorEmptyPresentation<'a> {
    const fn new(
        monitor_empty_state: MonitorEmptyState,
        termination_lifecycle_registry: &'a BuildTerminationLifecycleRegistry,
    ) -> Self {
        Self {
            monitor_empty_state,
            termination_lifecycle_registry,
        }
    }

    pub(super) const fn monitor_empty_state(self) -> MonitorEmptyState { self.monitor_empty_state }

    /// Terminal records remain available when no current row survives.
    pub(super) fn terminal_records(
        self,
    ) -> impl Iterator<Item = &'a BuildTerminationTerminalRecord> {
        self.termination_lifecycle_registry.terminal_records()
    }
}

/// The empty-state line, in the parts the renderer draws it from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MonitorEmptyStateMessage {
    headline:   &'static str,
    index_note: MonitorEmptyStateIndexNote,
}

impl MonitorEmptyStateMessage {
    /// Why there are no columns.
    pub(super) const fn headline(self) -> &'static str { self.headline }

    /// Which index that answer was resolved against.
    pub(super) const fn index_note(self) -> MonitorEmptyStateIndexNote { self.index_note }
}

/// How the index behind an empty monitor reads in the message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorEmptyStateIndexNote {
    /// The snapshot alone explains the emptiness, so there is no index to name.
    Absent,
    /// The scope resolution behind the message was read off this index.
    Index(&'static str),
}

impl From<MonitorWorkspaceIndexReadiness> for MonitorEmptyStateIndexNote {
    fn from(monitor_workspace_index_readiness: MonitorWorkspaceIndexReadiness) -> Self {
        match monitor_workspace_index_readiness {
            MonitorWorkspaceIndexReadiness::Current(_) => Self::Index("current index"),
            MonitorWorkspaceIndexReadiness::RetainedLastAccepted(_) => {
                Self::Index("last accepted index")
            },
            MonitorWorkspaceIndexReadiness::Uninitialized => Self::Index("no index yet"),
        }
    }
}

/// Whether the build monitor occupies part of the Output pane this frame.
///
/// Payload-free, so callers outside the pane can read the fact without naming
/// the borrowed monitor model [`MonitorVisibility`] carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputMonitorVisibility {
    /// Compile visibility is off, so nothing monitor-related is drawn.
    Off,
    /// The monitor half is drawn.
    On,
}

/// One output-build-set aggregate derived from terminal records with the same
/// transaction identity. The registry remains the only terminal-result owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OutputBuildSetTerminationAggregateProjection {
    aggregate_completion:       BuildTerminationAggregateCompletion,
    additional_build_exclusion: AdditionalBuildExclusion,
    confirmed_root_count:       usize,
}

impl OutputBuildSetTerminationAggregateProjection {
    pub(super) const fn aggregate_completion(self) -> BuildTerminationAggregateCompletion {
        self.aggregate_completion
    }

    pub(super) const fn additional_build_exclusion(self) -> AdditionalBuildExclusion {
        self.additional_build_exclusion
    }

    pub(super) const fn confirmed_root_count(self) -> usize { self.confirmed_root_count }
}

/// One retained output-build-set transaction, projected from the lifecycle
/// registry as its aggregate followed by the root records that still belong to
/// that transaction.
pub(super) struct OutputBuildSetTerminationResultGroup<'a> {
    aggregate:        OutputBuildSetTerminationAggregateProjection,
    terminal_records: Vec<&'a BuildTerminationTerminalRecord>,
}

impl<'a> OutputBuildSetTerminationResultGroup<'a> {
    pub(super) const fn aggregate(&self) -> OutputBuildSetTerminationAggregateProjection {
        self.aggregate
    }

    pub(super) fn into_terminal_records(self) -> Vec<&'a BuildTerminationTerminalRecord> {
        self.terminal_records
    }
}

/// One root Cargo invocation, as one stable column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MonitorColumn<'a> {
    monitor_session_row:         &'a MonitorSessionRow,
    build_termination_lifecycle: BuildTerminationLifecycle,
}

/// How a column's state row reads against the rest of its header.
///
/// A session stopped behind another session's build-directory lock is making no
/// progress and cannot until that holder finishes, which is the one state the
/// column has to say at a glance rather than by its wording alone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum MonitorStateEmphasis {
    /// Nothing in the state row has to stand out.
    #[default]
    Ordinary,
    /// The session is stopped until the lock holder its row names finishes.
    LockBlocked,
}

/// The rows of one column's header, with how its last row reads.
///
/// The emphasis travels with the rows rather than beside them so a draw site
/// cannot style one column's state row from another column's standing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ColumnHeaderDisplay {
    rows:           [String; COLUMN_HEADER_HEIGHT],
    state_emphasis: MonitorStateEmphasis,
}

impl ColumnHeaderDisplay {
    /// Header rows standing in for a column, for tests that draw one directly.
    #[cfg(test)]
    pub(super) const fn for_test(
        rows: [String; COLUMN_HEADER_HEIGHT],
        state_emphasis: MonitorStateEmphasis,
    ) -> Self {
        Self {
            rows,
            state_emphasis,
        }
    }

    /// The header rows top to bottom, and the emphasis the last of them reads in.
    pub(super) fn into_parts(self) -> ([String; COLUMN_HEADER_HEIGHT], MonitorStateEmphasis) {
        (self.rows, self.state_emphasis)
    }
}

/// Confirmation data derived from a displayed root Cargo session.
///
/// It is intentionally insufficient to authorize anything: the modal owns a
/// separate opaque authorization, while this value only records what the user
/// selected and what the column was showing at that time.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedBuildTerminationConfirmationDisplay {
    operative_cargo_command: String,
    checkout:                String,
    root_pid:                u32,
    start_age:               Duration,
    compiler_child_count:    usize,
    profile:                 String,
    state:                   String,
    state_emphasis:          MonitorStateEmphasis,
}

/// Display data frozen beside opaque authority for all root rows shown in the
/// Output monitor.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OutputBuildSetTerminationConfirmationDisplay {
    selected_row_display_path: DisplayPath,
    target_summaries:          Vec<SelectedBuildTerminationConfirmationDisplay>,
}

impl OutputBuildSetTerminationConfirmationDisplay {
    pub(crate) const fn selected_row_display_path(&self) -> &DisplayPath {
        &self.selected_row_display_path
    }

    pub(crate) fn target_summaries(&self) -> &[SelectedBuildTerminationConfirmationDisplay] {
        &self.target_summaries
    }
}

/// Whether the current selected project-list row can name the frozen output
/// build-set confirmation without fabricating a display value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OutputBuildSetTerminationConfirmationDisplayResolution {
    /// The selected row and exact current Output rows produced display data.
    Ready(OutputBuildSetTerminationConfirmationDisplay),
    /// The selected row vanished before its display identity was frozen.
    SelectedRowUnavailable,
}

impl SelectedBuildTerminationConfirmationDisplay {
    pub(crate) fn operative_cargo_command(&self) -> &str { &self.operative_cargo_command }

    pub(crate) fn checkout(&self) -> &str { &self.checkout }

    pub(crate) const fn root_pid(&self) -> u32 { self.root_pid }

    pub(crate) const fn start_age(&self) -> Duration { self.start_age }

    pub(crate) const fn compiler_child_count(&self) -> usize { self.compiler_child_count }

    fn column_header(&self) -> ColumnHeaderDisplay {
        ColumnHeaderDisplay {
            rows:           [
                self.operative_cargo_command.clone(),
                self.checkout.clone(),
                format!(
                    "{} · pid {} · {}",
                    self.profile,
                    self.root_pid,
                    format_progressive(self.start_age.as_secs()),
                ),
                self.state.clone(),
            ],
            state_emphasis: self.state_emphasis,
        }
    }
}

/// The Output cursor's selected session represented for the termination
/// confirmation flow. It carries display facts, never signal authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectedBuildTerminationDisplayTarget {
    build_session_id:                                BuildSessionId,
    selected_build_termination_confirmation_display: SelectedBuildTerminationConfirmationDisplay,
}

impl SelectedBuildTerminationDisplayTarget {
    pub(in crate::tui) const fn build_session_id(&self) -> &BuildSessionId {
        &self.build_session_id
    }

    pub(in crate::tui) fn into_parts(
        self,
    ) -> (BuildSessionId, SelectedBuildTerminationConfirmationDisplay) {
        (
            self.build_session_id,
            self.selected_build_termination_confirmation_display,
        )
    }
}

impl<'a> MonitorColumn<'a> {
    /// The exec-sensitive key this column keeps its cursor and selection under.
    pub(super) const fn build_session_id(self) -> &'a BuildSessionId {
        self.monitor_session_row.build_session_id()
    }

    /// The session record the header columns are drawn from.
    pub(super) const fn session_row(self) -> &'a MonitorSessionRow { self.monitor_session_row }

    /// What this session is doing right now.
    pub(super) const fn build_session_activity(self) -> BuildSessionActivity {
        self.monitor_session_row.build_session_activity()
    }

    /// Where this session sits in the queue for its build-directory lock.
    pub(super) const fn build_lock_contention(self) -> BuildLockContention {
        self.monitor_session_row.build_lock_contention()
    }

    /// Whether Cargo Port launched the run behind this column.
    pub(super) const fn session_ownership(self) -> MonitorSessionOwnership {
        self.monitor_session_row.session_ownership()
    }

    /// Transaction lifecycle joined at presentation time, never persisted on
    /// the replaceable monitor row.
    pub(super) const fn build_termination_lifecycle(self) -> BuildTerminationLifecycle {
        self.build_termination_lifecycle
    }

    /// How long this session's root has been running, as of `now`.
    pub(super) fn elapsed(self, now: Instant) -> Duration {
        now.saturating_duration_since(
            self.monitor_session_row
                .build_session()
                .root_observation()
                .first_observed_at(),
        )
    }

    /// Derive the selected-build confirmation display from the same session
    /// record and lifecycle join that the column header consumes.
    pub(super) fn selected_build_termination_display_target(
        self,
        now: Instant,
    ) -> SelectedBuildTerminationDisplayTarget {
        SelectedBuildTerminationDisplayTarget {
            build_session_id:                                self.build_session_id().clone(),
            selected_build_termination_confirmation_display: self
                .selected_build_termination_confirmation_display(now),
        }
    }

    pub(super) fn column_header(self, now: Instant) -> ColumnHeaderDisplay {
        self.selected_build_termination_confirmation_display(now)
            .column_header()
    }

    fn selected_build_termination_confirmation_display(
        self,
        now: Instant,
    ) -> SelectedBuildTerminationConfirmationDisplay {
        let build_session = self.monitor_session_row.build_session();
        let root_observation = build_session.root_observation();
        SelectedBuildTerminationConfirmationDisplay {
            operative_cargo_command: cargo_command_label(self),
            checkout:                checkout_label(self),
            root_pid:                root_observation.root_pid(),
            start_age:               self.elapsed(now),
            compiler_child_count:    self.monitor_session_row.compile_activities().len(),
            profile:                 profile_label(self),
            state:                   state_label(self),
            state_emphasis:          state_emphasis(
                self.build_termination_lifecycle(),
                self.build_lock_contention(),
            ),
        }
    }
}

/// `cargo <subcommand>` followed by the selectors as the user typed them.
fn cargo_command_label(monitor_column: MonitorColumn<'_>) -> String {
    let operative_cargo_command = monitor_column
        .session_row()
        .build_session()
        .operative_cargo_command();
    let subcommand = match operative_cargo_command.subcommand() {
        CargoSubcommand::Named(name) => name.as_str(),
        CargoSubcommand::Absent => "",
    };
    let selectors: Vec<String> = operative_cargo_command
        .selectors()
        .iter()
        .map(selector_label)
        .collect();
    let command = if selectors.is_empty() {
        format!("cargo {subcommand}")
    } else {
        format!("cargo {subcommand} {}", selectors.join(" "))
    };
    let command = command.trim_end();
    match monitor_column
        .session_row()
        .build_session()
        .cargo_subcommand_recognition()
    {
        CargoSubcommandRecognition::Build | CargoSubcommandRecognition::NonBuild => {
            command.to_string()
        },
        CargoSubcommandRecognition::Unrecognized => format!("{command} (alias)"),
    }
}

/// One selector argument, rendered the way it was written.
fn selector_label(cargo_command_selector: &CargoCommandSelector) -> String {
    match cargo_command_selector {
        CargoCommandSelector::Package(name) => format!("-p {name}"),
        CargoCommandSelector::AllPackages => "--workspace".to_string(),
        CargoCommandSelector::Library => "--lib".to_string(),
        CargoCommandSelector::Binary(name) => format!("--bin {name}"),
        CargoCommandSelector::AllBinaries => "--bins".to_string(),
        CargoCommandSelector::Example(name) => format!("--example {name}"),
        CargoCommandSelector::AllExamples => "--examples".to_string(),
        CargoCommandSelector::Test(name) => format!("--test {name}"),
        CargoCommandSelector::AllTests => "--tests".to_string(),
        CargoCommandSelector::Benchmark(name) => format!("--bench {name}"),
        CargoCommandSelector::AllBenchmarks => "--benches".to_string(),
        CargoCommandSelector::AllTargets => "--all-targets".to_string(),
    }
}

/// The checkout this session builds, falling back to where it writes when no
/// method related it to the project list. Paths under the home directory are
/// written `~/…` so the narrow monitor column shows more of the path.
fn checkout_label(monitor_column: MonitorColumn<'_>) -> String {
    let build_session = monitor_column.session_row().build_session();
    match build_session.session_scope() {
        SessionScope::Resolved { root, .. } => home_relative_path(root.path().as_path()),
        SessionScope::Unresolved => match build_session.session_target_directory().evidence() {
            TargetDirectoryEvidence::Determined(canonical_target_directory) => {
                format!(
                    "writes {}",
                    home_relative_path(canonical_target_directory.path().as_path())
                )
            },
            TargetDirectoryEvidence::Unobservable => "checkout unresolved".to_string(),
        },
    }
}

/// The resolved profile, keeping a manifest's custom name.
fn profile_label(monitor_column: MonitorColumn<'_>) -> String {
    match monitor_column
        .session_row()
        .build_session()
        .build_profile()
        .label()
    {
        BuildProfileLabel::Dev => "dev".to_string(),
        BuildProfileLabel::Release => "release".to_string(),
        BuildProfileLabel::Custom(name) => name.clone(),
    }
}

/// The current activity with the transaction lifecycle marker when one exists.
fn state_label(monitor_column: MonitorColumn<'_>) -> String {
    match monitor_column.build_termination_lifecycle() {
        BuildTerminationLifecycle::Terminating => "terminating".to_string(),
        BuildTerminationLifecycle::GoneAfterSignaling => "gone after signal".to_string(),
        BuildTerminationLifecycle::AlreadyGone => "already gone".to_string(),
        BuildTerminationLifecycle::RetryUnavailable => "termination incomplete".to_string(),
        BuildTerminationLifecycle::Observed => {
            let activity = activity_label(
                monitor_column.build_session_activity(),
                monitor_column.build_lock_contention(),
            );
            match monitor_column.session_ownership() {
                MonitorSessionOwnership::Owned(_) => format!("{activity} · launched here"),
                MonitorSessionOwnership::External => activity,
            }
        },
    }
}

/// What the session is doing, followed by its place in the build-directory lock
/// queue when this cycle put it in one.
///
/// A root with no compiler children reads as idle only when it also consumed no
/// CPU: Cargo does its own resolution and fingerprint work before the first
/// `rustc` exists, and that work is not idleness. Which of the two a parked root
/// is waiting on is what the lock queue answers.
pub(super) fn activity_label(
    build_session_activity: BuildSessionActivity,
    build_lock_contention: BuildLockContention,
) -> String {
    let activity = match build_session_activity {
        BuildSessionActivity::Compiling => "compiling",
        BuildSessionActivity::RunningBuildScript => "build script",
        BuildSessionActivity::Linking => "linking",
        BuildSessionActivity::ActiveWithoutCompiler(RootCpuActivity::Parked) => "idle",
        BuildSessionActivity::ActiveWithoutCompiler(
            RootCpuActivity::Working | RootCpuActivity::Unobserved,
        ) => "active",
    };
    match build_lock_contention {
        BuildLockContention::Holding => format!("{activity} · lock holder"),
        BuildLockContention::WaitingBehind { holder_pid } => {
            format!("{activity} · waiting on pid {holder_pid}")
        },
        BuildLockContention::Undetermined => activity.to_string(),
    }
}

/// Whether the state row says the session is stopped behind another session.
///
/// Only a session the monitor is still observing can be waiting: once a
/// termination is under way the row reports what happened to the session, and
/// the lock queue it was in before that no longer describes it.
pub(super) const fn state_emphasis(
    build_termination_lifecycle: BuildTerminationLifecycle,
    build_lock_contention: BuildLockContention,
) -> MonitorStateEmphasis {
    match build_termination_lifecycle {
        BuildTerminationLifecycle::Terminating
        | BuildTerminationLifecycle::GoneAfterSignaling
        | BuildTerminationLifecycle::AlreadyGone
        | BuildTerminationLifecycle::RetryUnavailable => MonitorStateEmphasis::Ordinary,
        BuildTerminationLifecycle::Observed => match build_lock_contention {
            BuildLockContention::WaitingBehind { .. } => MonitorStateEmphasis::LockBlocked,
            BuildLockContention::Holding | BuildLockContention::Undetermined => {
                MonitorStateEmphasis::Ordinary
            },
        },
    }
}

/// The scope's build sessions, the scope-level unattributed section, and
/// whether the whole set carries the staleness marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorColumns<'a> {
    session_rows:                   &'a [MonitorSessionRow],
    termination_lifecycle_registry: &'a BuildTerminationLifecycleRegistry,
    unattributed_activities:        &'a [UnattributedCompileActivity],
    monitor_staleness:              MonitorStaleness,
}

impl<'a> MonitorColumns<'a> {
    /// The columns in first-seen order.
    pub(super) fn columns(&self) -> impl Iterator<Item = MonitorColumn<'a>> {
        self.session_rows
            .iter()
            .map(|monitor_session_row| MonitorColumn {
                build_termination_lifecycle: self
                    .termination_lifecycle_registry
                    .lifecycle_for(monitor_session_row.build_session_id()),
                monitor_session_row,
            })
    }

    /// How many columns the model holds, on screen or not.
    pub(super) const fn len(&self) -> usize { self.session_rows.len() }

    /// The scope-level activities no single session claims. Always drawn as
    /// observed-only: nothing here may authorize a termination.
    pub(super) const fn unattributed_activities(&self) -> &'a [UnattributedCompileActivity] {
        self.unattributed_activities
    }

    /// Whether the rows carry the visible staleness marker.
    pub(super) const fn monitor_staleness(&self) -> MonitorStaleness { self.monitor_staleness }

    /// Terminal records are read through the registry join, never a TUI store.
    pub(super) fn terminal_records(
        &self,
    ) -> impl Iterator<Item = &'a BuildTerminationTerminalRecord> {
        self.termination_lifecycle_registry.terminal_records()
    }

    /// The contiguous run of columns that fits `width`, chosen so
    /// `selected_index` stays on screen.
    ///
    /// A single column takes the full width with no divider; several split
    /// equally down to [`MINIMUM_READABLE_COLUMN_WIDTH`], and the rest are held
    /// off screen in the model rather than squeezed below it.
    pub(super) fn window(&self, width: u16, selected_index: usize) -> MonitorColumnWindow {
        let fitting = usize::from(width / MINIMUM_READABLE_COLUMN_WIDTH).max(1);
        if self.len() <= fitting {
            return MonitorColumnWindow {
                first: 0,
                count: self.len(),
            };
        }
        let last_first = self.len() - fitting;
        let first = selected_index.saturating_sub(fitting - 1).min(last_first);
        MonitorColumnWindow {
            first,
            count: fitting,
        }
    }
}

/// Group output-build-set terminal records by stable transaction identity.
pub(super) fn output_build_set_termination_result_groups<'a>(
    terminal_records: impl Iterator<Item = &'a BuildTerminationTerminalRecord>,
) -> Vec<OutputBuildSetTerminationResultGroup<'a>> {
    let mut by_transaction = BTreeMap::new();
    for terminal_record in terminal_records {
        if terminal_record.target_set() != BuildTerminationTransactionTargetSet::OutputBuildSet {
            continue;
        }
        by_transaction
            .entry(terminal_record.transaction_id())
            .and_modify(
                |output_build_set_termination_result_group: &mut OutputBuildSetTerminationResultGroup<'_>| {
                    output_build_set_termination_result_group
                        .terminal_records
                        .push(terminal_record);
                },
            )
            .or_insert_with(|| OutputBuildSetTerminationResultGroup {
                aggregate: OutputBuildSetTerminationAggregateProjection {
                    aggregate_completion:       terminal_record.aggregate_completion(),
                    additional_build_exclusion: terminal_record.additional_build_exclusion(),
                    confirmed_root_count:       terminal_record.confirmed_root_count(),
                },
                terminal_records: vec![terminal_record],
            });
    }
    by_transaction.into_values().collect()
}

/// The contiguous run of columns drawn this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MonitorColumnWindow {
    first: usize,
    count: usize,
}

impl MonitorColumnWindow {
    /// The leftmost column drawn this frame.
    #[cfg(test)]
    pub(super) const fn first(&self) -> usize { self.first }

    /// How many columns are drawn this frame.
    #[cfg(test)]
    pub(super) const fn count(&self) -> usize { self.count }

    /// Whether `index` is on screen.
    pub(super) const fn contains(&self, index: usize) -> bool {
        index >= self.first && index < self.first + self.count
    }
}

/// What the monitor half of the pane draws.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MonitorPresentation<'a> {
    /// No column to draw, and the reason the user needs.
    Empty(MonitorEmptyPresentation<'a>),
    /// One column per root Cargo invocation.
    Columns(MonitorColumns<'a>),
}

/// The Cargo Port-owned run's own body: its retained output and lifecycle
/// label, keyed by the run that produced them.
///
/// The producer is the retaining run, never the current lifecycle identity, so
/// run N's output stays attributed to N while run N+1 is queued or starting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedOutputPresentation<'a> {
    producer:          OwnedRunId,
    title:             OwnedRunOutputTitleRef<'a>,
    running_label:     OwnedRunRunningLabelRef<'a>,
    completion_marker: OwnedRunCompletionMarker,
    lines:             &'a [String],
}

impl<'a> OwnedOutputPresentation<'a> {
    /// The run whose output this is.
    pub(super) const fn producer(&self) -> OwnedRunId { self.producer }

    /// The title the retained output was captured under.
    pub(super) const fn title(&self) -> OwnedRunOutputTitleRef<'a> { self.title }

    /// The current run's running label, when one is running.
    pub(super) const fn running_label(&self) -> OwnedRunRunningLabelRef<'a> { self.running_label }

    /// How the producing run ended, drawn as the pin marker. The pin survives a
    /// scope change, so this marker outlives the run it describes.
    pub(super) const fn completion_marker(&self) -> OwnedRunCompletionMarker {
        self.completion_marker
    }

    /// The captured lines, borrowed rather than copied.
    pub(super) const fn lines(&self) -> &'a [String] { self.lines }
}

/// Everything the Output pane shows this frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputPresentation<'a> {
    /// Nothing to show: the diagnostics panes own the bottom row.
    Hidden,
    /// Monitoring is off and the owned run's captured output is on screen.
    OwnedOnly(OwnedOutputPresentation<'a>),
    /// Monitoring is on with no owned output retained.
    Monitor(MonitorPresentation<'a>),
    /// Monitoring is on and an owned run's output is pinned beside it. The pin
    /// survives a scope change, so it can name a run outside the monitored
    /// scope.
    MonitorWithOwned {
        monitor: MonitorPresentation<'a>,
        owned:   OwnedOutputPresentation<'a>,
    },
}

impl<'a> OutputPresentation<'a> {
    /// Derive what the pane shows from the two inputs the renderer takes.
    ///
    /// The snapshot alone cannot tell a first cycle apart from an unresolvable
    /// scope — [`MonitorSnapshot::Pending`] covers both — so the scope
    /// resolution is read alongside it.
    pub fn derive(
        compile_visibility_state: &'a CompileVisibilityState,
        monitor_snapshot: &'a MonitorSnapshot,
        termination_lifecycle_registry: &'a BuildTerminationLifecycleRegistry,
        owned_run_output_state: OwnedRunOutputStateRef<'a>,
        owned_run_running_label: OwnedRunRunningLabelRef<'a>,
        owned_run_completion_marker: OwnedRunCompletionMarker,
    ) -> Self {
        let owned = match owned_run_output_state {
            OwnedRunOutputStateRef::Retained {
                producer,
                title,
                lines,
            } if !lines.is_empty() => Some(OwnedOutputPresentation {
                producer,
                title,
                running_label: owned_run_running_label,
                completion_marker: owned_run_completion_marker,
                lines,
            }),
            // No producer, or a producer that has emitted nothing yet: either
            // way there is nothing to draw, and the pane stays off the bottom
            // row until there is.
            OwnedRunOutputStateRef::Absent | OwnedRunOutputStateRef::Retained { .. } => None,
        };
        let monitor_visibility = monitor_presentation(
            compile_visibility_state,
            monitor_snapshot,
            termination_lifecycle_registry,
        );
        match (monitor_visibility, owned) {
            (MonitorVisibility::Off, None) => Self::Hidden,
            (MonitorVisibility::Off, Some(owned)) => Self::OwnedOnly(owned),
            (MonitorVisibility::On(monitor), None) => Self::Monitor(monitor),
            (MonitorVisibility::On(monitor), Some(owned)) => {
                Self::MonitorWithOwned { monitor, owned }
            },
        }
    }

    /// Whether the pane is drawn, focusable, and in the tab order.
    pub const fn pane_visibility(&self) -> OutputPaneVisibility {
        match self {
            Self::Hidden => OutputPaneVisibility::Hidden,
            Self::OwnedOnly(_) | Self::Monitor(_) | Self::MonitorWithOwned { .. } => {
                OutputPaneVisibility::Visible
            },
        }
    }

    /// Whether the build monitor occupies part of the pane this frame.
    ///
    /// The payload-free companion to [`MonitorVisibility`], for callers outside
    /// the pane that need only the fact and cannot name the borrowed model.
    pub const fn monitor_visibility(&self) -> OutputMonitorVisibility {
        match self {
            Self::Hidden | Self::OwnedOnly(_) => OutputMonitorVisibility::Off,
            Self::Monitor(_) | Self::MonitorWithOwned { .. } => OutputMonitorVisibility::On,
        }
    }

    /// Whether a copy gesture has captured output to read.
    pub const fn copy_availability(&self) -> OutputCopyAvailability {
        match self {
            Self::Hidden | Self::Monitor(_) => OutputCopyAvailability::Unavailable,
            Self::OwnedOnly(_) | Self::MonitorWithOwned { .. } => {
                OutputCopyAvailability::CapturedOutput
            },
        }
    }

    /// Freeze the selected-row identity and every exact current monitor-root
    /// summary separately from opaque termination authority.
    pub(crate) fn output_build_set_termination_confirmation_display_resolution(
        &self,
        project_list_row_display_path_resolution: ProjectListRowDisplayPathResolution,
        now: Instant,
    ) -> OutputBuildSetTerminationConfirmationDisplayResolution {
        let ProjectListRowDisplayPathResolution::Resolved(selected_row_display_path) =
            project_list_row_display_path_resolution
        else {
            return OutputBuildSetTerminationConfirmationDisplayResolution::SelectedRowUnavailable;
        };
        let target_summaries = match self.monitor() {
            MonitorVisibility::Off | MonitorVisibility::On(MonitorPresentation::Empty(_)) => {
                Vec::new()
            },
            MonitorVisibility::On(MonitorPresentation::Columns(monitor_columns)) => monitor_columns
                .columns()
                .map(|monitor_column| {
                    monitor_column.selected_build_termination_confirmation_display(now)
                })
                .collect(),
        };
        OutputBuildSetTerminationConfirmationDisplayResolution::Ready(
            OutputBuildSetTerminationConfirmationDisplay {
                selected_row_display_path,
                target_summaries,
            },
        )
    }

    /// Whether the owned body is on screen this frame.
    pub(super) const fn owned_output(&self) -> OwnedOutputVisibility<'a> {
        match self {
            Self::Hidden | Self::Monitor(_) => OwnedOutputVisibility::Absent,
            Self::OwnedOnly(owned) | Self::MonitorWithOwned { owned, .. } => {
                OwnedOutputVisibility::OnScreen(*owned)
            },
        }
    }

    /// Whether the monitor half is on screen this frame.
    pub(super) const fn monitor(&self) -> MonitorVisibility<'a> {
        match self {
            Self::Hidden | Self::OwnedOnly(_) => MonitorVisibility::Off,
            Self::Monitor(monitor) | Self::MonitorWithOwned { monitor, .. } => {
                MonitorVisibility::On(*monitor)
            },
        }
    }

    /// The captured lines a copy or visual selection reads, empty when the pane
    /// is showing only external columns.
    pub(super) const fn captured_lines(&self) -> &'a [String] {
        match self.owned_output() {
            OwnedOutputVisibility::Absent => &[],
            OwnedOutputVisibility::OnScreen(owned) => owned.lines(),
        }
    }
}

/// Whether Cargo Port's own captured output occupies part of the pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OwnedOutputVisibility<'a> {
    /// No run has retained output with lines to draw.
    Absent,
    /// This run's retained output is drawn, in its own column or pinned beside
    /// the monitor.
    OnScreen(OwnedOutputPresentation<'a>),
}

/// Whether the build monitor occupies part of the pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MonitorVisibility<'a> {
    /// Compile visibility is off, so nothing monitor-related is drawn.
    Off,
    /// The monitor half is drawn, with columns or with the reason it has none.
    On(MonitorPresentation<'a>),
}

/// Whether the monitor half is drawn, and what it shows.
///
/// [`MonitorSnapshot::Off`] and a disabled [`CompileVisibilityState`] are the
/// same fact reached from two directions; either one means the monitor draws
/// nothing at all.
fn monitor_presentation<'a>(
    compile_visibility_state: &'a CompileVisibilityState,
    monitor_snapshot: &'a MonitorSnapshot,
    termination_lifecycle_registry: &'a BuildTerminationLifecycleRegistry,
) -> MonitorVisibility<'a> {
    let CompileVisibilityState::On(active_monitor_state) = compile_visibility_state else {
        return MonitorVisibility::Off;
    };
    let monitor_empty_state = match monitor_snapshot.monitor_display() {
        MonitorDisplay::SwitchedOff => return MonitorVisibility::Off,
        MonitorDisplay::Rows {
            monitor_data,
            monitor_staleness,
        } => {
            if monitor_data.session_rows().is_empty()
                && monitor_data.unattributed_activities().is_empty()
            {
                return MonitorVisibility::On(MonitorPresentation::Empty(
                    MonitorEmptyPresentation::new(
                        MonitorEmptyState::NoBuildSessions,
                        termination_lifecycle_registry,
                    ),
                ));
            }
            return MonitorVisibility::On(MonitorPresentation::Columns(MonitorColumns {
                session_rows: monitor_data.session_rows(),
                termination_lifecycle_registry,
                unattributed_activities: monitor_data.unattributed_activities(),
                monitor_staleness,
            }));
        },
        MonitorDisplay::AwaitingFirstCycle => scope_resolution_empty_state(
            active_monitor_state.monitor_scope_resolution(),
            MonitorEmptyState::AwaitingFirstCycle,
        ),
        MonitorDisplay::Unavailable => scope_resolution_empty_state(
            active_monitor_state.monitor_scope_resolution(),
            MonitorEmptyState::Unavailable,
        ),
    };
    MonitorVisibility::On(MonitorPresentation::Empty(MonitorEmptyPresentation::new(
        monitor_empty_state,
        termination_lifecycle_registry,
    )))
}

/// The empty state to draw: the one a non-actionable scope resolution names, or
/// `snapshot_explains` when the scope is actionable and only the snapshot has
/// anything to say.
const fn scope_resolution_empty_state(
    monitor_scope_resolution: &MonitorScopeResolution,
    snapshot_explains: MonitorEmptyState,
) -> MonitorEmptyState {
    match monitor_scope_resolution {
        MonitorScopeResolution::Ready(_) => snapshot_explains,
        MonitorScopeResolution::PendingIndex(revision) => {
            MonitorEmptyState::PendingIndex(revision.monitor_workspace_index_readiness())
        },
        MonitorScopeResolution::EmptyNonRust(revision) => {
            MonitorEmptyState::EmptyNonRust(revision.monitor_workspace_index_readiness())
        },
        MonitorScopeResolution::AmbiguousOwnership(revision) => {
            MonitorEmptyState::AmbiguousOwnership(revision.monitor_workspace_index_readiness())
        },
        MonitorScopeResolution::UnresolvedPath(revision) => {
            MonitorEmptyState::UnresolvedPath(revision.monitor_workspace_index_readiness())
        },
    }
}
