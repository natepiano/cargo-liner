//! The one value the Output pane is drawn from.
//!
//! Layout, visibility, focus reconciliation, tabbability, the bottom-row action
//! labels, copy availability, hit testing, and rendering all read
//! [`OutputPresentation`], so none of them can disagree about what the pane is
//! currently showing.

use std::time::Instant;

use tui_pane::format_progressive;

use super::constants::MINIMUM_READABLE_COLUMN_WIDTH;
use crate::build_monitor::BuildProfileLabel;
use crate::build_monitor::BuildSessionActivity;
use crate::build_monitor::BuildSessionId;
use crate::build_monitor::BuildTerminationLifecycle;
use crate::build_monitor::BuildTerminationLifecycleRegistry;
use crate::build_monitor::BuildTerminationTerminalRecord;
use crate::build_monitor::CargoCommandSelector;
use crate::build_monitor::CargoSubcommand;
use crate::build_monitor::CargoSubcommandRecognition;
use crate::build_monitor::MonitorDisplay;
use crate::build_monitor::MonitorSessionOwnership;
use crate::build_monitor::MonitorSessionRow;
use crate::build_monitor::MonitorSnapshot;
use crate::build_monitor::MonitorStaleness;
use crate::build_monitor::SessionScope;
use crate::build_monitor::TargetDirectoryEvidence;
use crate::build_monitor::UnattributedCompileActivity;
use crate::tui::OwnedRunId;
use crate::tui::compile_visibility::CompileVisibilityState;
use crate::tui::compile_visibility::MonitorScopeResolution;
use crate::tui::compile_visibility::MonitorWorkspaceIndexReadiness;
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
pub enum MonitorEmptyState {
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

/// One root Cargo invocation, as one stable column.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MonitorColumn<'a> {
    monitor_session_row:         &'a MonitorSessionRow,
    build_termination_lifecycle: BuildTerminationLifecycle,
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
    start_age:               std::time::Duration,
    compiler_child_count:    usize,
    profile:                 String,
    state:                   String,
}

impl SelectedBuildTerminationConfirmationDisplay {
    pub(crate) fn operative_cargo_command(&self) -> &str { &self.operative_cargo_command }

    pub(crate) fn checkout(&self) -> &str { &self.checkout }

    pub(crate) const fn root_pid(&self) -> u32 { self.root_pid }

    pub(crate) const fn start_age(&self) -> std::time::Duration { self.start_age }

    pub(crate) const fn compiler_child_count(&self) -> usize { self.compiler_child_count }

    fn header_fields(&self) -> [String; 4] {
        [
            self.operative_cargo_command.clone(),
            self.checkout.clone(),
            format!(
                "{} · pid {} · {}",
                self.profile,
                self.root_pid,
                format_progressive(self.start_age.as_secs()),
            ),
            self.state.clone(),
        ]
    }
}

/// The Output cursor's selected session represented for the termination
/// confirmation flow. It carries display facts, never signal authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SelectedBuildTerminationDisplayTarget {
    build_session_id:                                BuildSessionId,
    selected_build_termination_confirmation_display: SelectedBuildTerminationConfirmationDisplay,
}

impl SelectedBuildTerminationDisplayTarget {
    pub(crate) const fn build_session_id(&self) -> &BuildSessionId { &self.build_session_id }

    pub(crate) fn into_parts(
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
    pub(super) fn elapsed(self, now: Instant) -> std::time::Duration {
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

    pub(super) fn header_fields(self, now: Instant) -> [String; 4] {
        self.selected_build_termination_confirmation_display(now)
            .header_fields()
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
/// method related it to the project list.
fn checkout_label(monitor_column: MonitorColumn<'_>) -> String {
    let build_session = monitor_column.session_row().build_session();
    match build_session.session_scope() {
        SessionScope::Resolved { root, .. } => root.path().to_string(),
        SessionScope::Unresolved => match build_session.session_target_directory().evidence() {
            TargetDirectoryEvidence::Determined(canonical_target_directory) => {
                format!("writes {}", canonical_target_directory.path())
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
            let activity = match monitor_column.build_session_activity() {
                BuildSessionActivity::Compiling => "compiling",
                BuildSessionActivity::RunningBuildScript => "build script",
                BuildSessionActivity::Linking => "linking",
                BuildSessionActivity::ActiveWithoutCompiler => "active",
            };
            match monitor_column.session_ownership() {
                MonitorSessionOwnership::Owned(_) => format!("{activity} · launched here"),
                MonitorSessionOwnership::External => activity.to_string(),
            }
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
