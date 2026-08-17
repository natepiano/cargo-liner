//! What the compile monitor currently has to show, and how fresh that data is.
//!
//! Every data-bearing state carries the instant its evidence was observed, so
//! age is re-derived wherever it is needed — at render time and at the moment a
//! kill is authorized — instead of being implied by which state holds the data.

use std::time::Instant;

use super::activity::CompileActivity;
use super::activity::CompilerKind;
use super::activity::UnattributedCompileActivity;
use super::scope::BuildScopeKey;
use super::session::BuildSession;
use super::session::BuildSessionId;
use super::session::RootCpuActivity;
use crate::tui::OwnedRunId;

/// What one live build session is doing, as this cycle's attributed compile
/// activities name it.
///
/// Every state is backed by an activity the cycle actually observed. A Cargo
/// root with no live compiler child is [`Self::ActiveWithoutCompiler`] rather
/// than a guessed phase: the root anchors the session through the gap, and
/// nothing observed says more than that it is still running.
///
/// There is deliberately no running-target state. A target Cargo launched is not
/// a compile activity: [`CompilerKind`] names only compiler, build-script,
/// linker, and wrapper executables, and
/// `ObservedCandidateRole`
/// classifies every other child of a Cargo root as `NotCandidate`, so this
/// module holds no evidence that would distinguish a running target from any
/// other descendant. `crate::tui::running_targets` owns that observation
/// against its own one-second cadence and joins it at presentation; adding an
/// unbacked variant here would be the guessed phase the paragraph above refuses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildSessionActivity {
    /// A `rustc`, `clippy-driver`, `rustdoc`, or compiler-wrapper child is
    /// running under this session.
    Compiling,
    /// A `build-script-*` executable Cargo compiled is running.
    RunningBuildScript,
    /// A linker is running beneath a compiler or build script.
    Linking,
    /// The Cargo root is live with no compiler, build script, or linker child
    /// this cycle, and the root itself was working or parked as the payload
    /// says.
    ActiveWithoutCompiler(RootCpuActivity),
}

impl BuildSessionActivity {
    /// Decide one session's state from the compile activities already
    /// attributed to it by [`MonitorData::session_row`].
    ///
    /// Compilers outrank the build script and linker children they spawn, so a
    /// session that is compiling reads as compiling even while one crate's
    /// linker is running.
    fn from_attributed_activities(
        compile_activities: &[CompileActivity],
        root_cpu_activity: RootCpuActivity,
    ) -> Self {
        let mut build_session_activity = Self::ActiveWithoutCompiler(root_cpu_activity);
        for compile_activity in compile_activities {
            let observed = match compile_activity.compiler_kind() {
                CompilerKind::Rustc
                | CompilerKind::ClippyDriver
                | CompilerKind::Rustdoc
                | CompilerKind::Wrapper => Self::Compiling,
                CompilerKind::BuildScript => Self::RunningBuildScript,
                CompilerKind::Linker => Self::Linking,
            };
            build_session_activity = build_session_activity.outranked_by(observed);
        }
        build_session_activity
    }

    /// Keep whichever of two observed states the pane should show.
    const fn outranked_by(self, observed: Self) -> Self {
        if observed.precedence() > self.precedence() {
            observed
        } else {
            self
        }
    }

    const fn precedence(self) -> u8 {
        match self {
            Self::ActiveWithoutCompiler(_) => 0,
            Self::Linking => 1,
            Self::RunningBuildScript => 2,
            Self::Compiling => 3,
        }
    }
}

/// Whether Cargo Port launched the run behind a session.
///
/// Only the lifecycle identity is retained. Captured output is never copied
/// into a snapshot; presentation reads it back through the owned run itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorSessionOwnership {
    /// This session is the Cargo Port-owned run's own Cargo process.
    Owned(OwnedRunId),
    /// Nothing Cargo Port launched is behind this session.
    External,
}

/// Where a session sits in the queue for its Cargo build-directory lock.
///
/// Cargo holds an exclusive lock on the build directory for a whole build, so
/// concurrent runs against one target directory serialize. Sessions sharing a
/// determined target directory are therefore a queue, and the one with compiler
/// children is the one building. This is read off the classification alone: no
/// lock file is opened and no lock state is queried, so every state below is a
/// statement about the observed sessions rather than about the lock itself.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BuildLockContention {
    /// This session has compiler children while other sessions writing to the
    /// same target directory have none.
    Holding,
    /// Another session writing to this session's target directory is the only
    /// one with compiler children.
    WaitingBehind { holder_pid: u32 },
    /// No queue was established: the target directory is unobservable, no other
    /// session writes to it, or the sessions that do leave no single one with
    /// compiler children.
    #[default]
    Undetermined,
}

/// One build session as the monitor pane shows it.
///
/// The attributed activities travel with the row rather than being looked back
/// up from the classification: the classification is dropped once the cycle is
/// recorded, and the pane draws one selectable row per activity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MonitorSessionRow {
    build_session:          BuildSession,
    build_session_activity: BuildSessionActivity,
    build_lock_contention:  BuildLockContention,
    compile_activities:     Vec<CompileActivity>,
    session_ownership:      MonitorSessionOwnership,
}

impl MonitorSessionRow {
    /// The immutable session record this row renders.
    pub(crate) const fn build_session(&self) -> &BuildSession { &self.build_session }

    /// This session's stable key.
    pub(crate) const fn build_session_id(&self) -> &BuildSessionId {
        self.build_session.build_session_id()
    }

    /// What this session is doing, as attributed activities name it.
    pub(crate) const fn build_session_activity(&self) -> BuildSessionActivity {
        self.build_session_activity
    }

    /// Where this session sits in the queue for its build-directory lock.
    pub(crate) const fn build_lock_contention(&self) -> BuildLockContention {
        self.build_lock_contention
    }

    /// The activities this cycle attributed to this session, in classification
    /// order, one selectable pane row each.
    pub(crate) fn compile_activities(&self) -> &[CompileActivity] { &self.compile_activities }

    /// Whether Cargo Port owns the run behind this session.
    pub(crate) const fn session_ownership(&self) -> MonitorSessionOwnership {
        self.session_ownership
    }
}

/// One classified cycle narrowed to the scope it was classified under.
///
/// The rows are already scope-filtered: this is the one value the pane renders
/// and the one value a scope-wide termination acts on, so the two cannot name
/// different sets.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MonitorData {
    build_scope_key:         BuildScopeKey,
    session_rows:            Vec<MonitorSessionRow>,
    unattributed_activities: Vec<UnattributedCompileActivity>,
    observed_at:             Instant,
}

impl MonitorData {
    /// Narrow one classification to a scope and pair each surviving session
    /// with its observed state and ownership.
    pub(super) const fn new(
        build_scope_key: BuildScopeKey,
        session_rows: Vec<MonitorSessionRow>,
        unattributed_activities: Vec<UnattributedCompileActivity>,
        observed_at: Instant,
    ) -> Self {
        Self {
            build_scope_key,
            session_rows,
            unattributed_activities,
            observed_at,
        }
    }

    /// The scope these rows were classified under.
    pub(super) const fn build_scope_key(&self) -> &BuildScopeKey { &self.build_scope_key }

    /// Sessions in first-seen order, as classification produced them.
    pub(crate) fn session_rows(&self) -> &[MonitorSessionRow] { &self.session_rows }

    /// Ambiguous and unattributed activities this scope covers, drawn once in
    /// the scope-level section rather than under a session they may not belong
    /// to. Narrowed by the same filter as [`Self::session_rows`], so the pane
    /// never re-derives the set.
    pub(crate) fn unattributed_activities(&self) -> &[UnattributedCompileActivity] {
        &self.unattributed_activities
    }

    /// When the evidence behind these rows was observed.
    #[cfg(test)]
    pub(super) const fn observed_at(&self) -> Instant { self.observed_at }

    /// Build one session row from a classified session and the activities this
    /// cycle attributed to it.
    pub(super) fn session_row(
        build_session: BuildSession,
        compile_activities: Vec<CompileActivity>,
        session_ownership: MonitorSessionOwnership,
        build_lock_contention: BuildLockContention,
    ) -> MonitorSessionRow {
        let build_session_activity = BuildSessionActivity::from_attributed_activities(
            &compile_activities,
            build_session.root_observation().root_cpu_activity(),
        );
        MonitorSessionRow {
            build_session,
            build_session_activity,
            build_lock_contention,
            compile_activities,
            session_ownership,
        }
    }
}

/// Monitor data kept on screen while the current generation has produced none.
///
/// This is simultaneously "no snapshot for this generation yet" and "prior data
/// still on screen", which is why it is its own value rather than a payload
/// bolted onto pending or stale.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RetainedMonitorData {
    displayed_data:          MonitorData,
    current_build_scope_key: BuildScopeKey,
}

impl RetainedMonitorData {
    const fn new(displayed_data: MonitorData, current_build_scope_key: BuildScopeKey) -> Self {
        Self {
            displayed_data,
            current_build_scope_key,
        }
    }

    /// The prior cycle's rows, still current enough to render and act on.
    const fn monitor_data(&self) -> &MonitorData { &self.displayed_data }

    /// The current scope key that retained rows may authorize against.
    const fn current_build_scope_key(&self) -> &BuildScopeKey { &self.current_build_scope_key }
}

/// What the compile monitor has to show right now.
///
/// `App` holds one [`BuildMonitor`](super::BuildMonitor) whether or not
/// visibility is enabled, so enablement is a state here rather than something a
/// reader has to infer from an absent snapshot.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum MonitorSnapshot {
    /// Visibility is switched off; the monitor owns no data and is owed none.
    #[default]
    Off,
    /// Enabled, with no data for this scope and nothing worth retaining.
    Pending,
    /// Enabled, with no data for the current generation, still showing the
    /// prior generation's rows because the scope covers the same roots.
    PendingWithRetained(RetainedMonitorData),
    /// Data classified under the current generation.
    Fresh(MonitorData),
    /// Data that matched the current generation and then aged past one refresh
    /// interval without a replacement.
    Stale(MonitorData),
    /// Rows that had already aged to [`Self::Stale`] when the scope moved to a
    /// new generation over the same roots. They stay on screen with the
    /// staleness marker and stay non-actionable: a scope replacement is not
    /// evidence that a failed cycle's data became live again.
    StaleWithRetained(RetainedMonitorData),
    /// Nothing observable is left to show.
    Unavailable,
}

/// Whether the monitor's current data may authorize a termination.
///
/// Retained data is actionable: the termination path re-resolves a retained
/// identity against the live process snapshot before it signals, so a session
/// that ended meanwhile is simply not found. Data that a failed cycle already
/// aged is not, however it was retained afterwards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorDataActionability<'a> {
    /// These rows may be acted on.
    Actionable(ActionableMonitorData<'a>),
    /// Nothing shown may be acted on.
    NotActionable,
}

/// Current scope identity paired with the rows that remain actionable in it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ActionableMonitorData<'a> {
    build_scope_key: &'a BuildScopeKey,
    session_rows:    &'a [MonitorSessionRow],
}

impl<'a> ActionableMonitorData<'a> {
    pub(super) const fn build_scope_key(self) -> &'a BuildScopeKey { self.build_scope_key }

    pub(super) const fn session_rows(self) -> &'a [MonitorSessionRow] { self.session_rows }
}

/// Whether the monitor holds an observation instant to derive age from.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorObservation {
    /// The evidence behind the shown rows was observed at this instant.
    Observed(Instant),
    /// Nothing is shown, so there is no age to derive.
    NoObservation,
}

/// Whether the rows the pane draws carry the visible staleness marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorStaleness {
    /// The rows are the monitor's current answer for this scope.
    Live,
    /// A cycle failed after these rows were classified; they are shown with the
    /// staleness marker and cannot authorize a termination.
    Stale,
}

/// What the monitor pane draws for the snapshot it holds.
///
/// Every [`MonitorSnapshot`] variant maps here, so the pane never has to decide
/// what an unhandled state should look like. [`Self::AwaitingFirstCycle`] is the
/// enabled-but-empty message and is distinct from [`Self::SwitchedOff`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorDisplay<'a> {
    /// Visibility is switched off; the monitor draws nothing at all.
    SwitchedOff,
    /// Enabled with no rows for this scope yet.
    AwaitingFirstCycle,
    /// Rows to draw, and whether they carry the staleness marker.
    Rows {
        monitor_data:      &'a MonitorData,
        monitor_staleness: MonitorStaleness,
    },
    /// Enabled with nothing observable left to show, and no rows to retain.
    Unavailable,
}

impl MonitorSnapshot {
    /// The rows a termination may act on.
    pub(crate) fn actionability(&self) -> MonitorDataActionability<'_> {
        match self {
            Self::Fresh(monitor_data) => {
                MonitorDataActionability::Actionable(ActionableMonitorData {
                    build_scope_key: monitor_data.build_scope_key(),
                    session_rows:    monitor_data.session_rows(),
                })
            },
            Self::PendingWithRetained(retained_monitor_data) => {
                MonitorDataActionability::Actionable(ActionableMonitorData {
                    build_scope_key: retained_monitor_data.current_build_scope_key(),
                    session_rows:    retained_monitor_data.monitor_data().session_rows(),
                })
            },
            Self::Off
            | Self::Pending
            | Self::Stale(_)
            | Self::StaleWithRetained(_)
            | Self::Unavailable => MonitorDataActionability::NotActionable,
        }
    }

    /// What the pane draws for this snapshot.
    pub(crate) const fn monitor_display(&self) -> MonitorDisplay<'_> {
        match self {
            Self::Off => MonitorDisplay::SwitchedOff,
            Self::Pending => MonitorDisplay::AwaitingFirstCycle,
            Self::Fresh(monitor_data) => MonitorDisplay::Rows {
                monitor_data,
                monitor_staleness: MonitorStaleness::Live,
            },
            Self::PendingWithRetained(retained_monitor_data) => MonitorDisplay::Rows {
                monitor_data:      retained_monitor_data.monitor_data(),
                monitor_staleness: MonitorStaleness::Live,
            },
            Self::Stale(monitor_data) => MonitorDisplay::Rows {
                monitor_data,
                monitor_staleness: MonitorStaleness::Stale,
            },
            Self::StaleWithRetained(retained_monitor_data) => MonitorDisplay::Rows {
                monitor_data:      retained_monitor_data.monitor_data(),
                monitor_staleness: MonitorStaleness::Stale,
            },
            Self::Unavailable => MonitorDisplay::Unavailable,
        }
    }

    /// When the shown evidence was observed, for age derived at read time.
    #[cfg(test)]
    pub(crate) const fn observation(&self) -> MonitorObservation {
        match self {
            Self::Fresh(monitor_data) | Self::Stale(monitor_data) => {
                MonitorObservation::Observed(monitor_data.observed_at())
            },
            Self::PendingWithRetained(retained_monitor_data)
            | Self::StaleWithRetained(retained_monitor_data) => {
                MonitorObservation::Observed(retained_monitor_data.monitor_data().observed_at())
            },
            Self::Off | Self::Pending | Self::Unavailable => MonitorObservation::NoObservation,
        }
    }

    /// Move whatever is shown one step further from live, for a cycle that
    /// produced no classification at all.
    pub(super) fn aged(self) -> Self {
        match self {
            Self::Fresh(monitor_data) => Self::Stale(monitor_data),
            Self::PendingWithRetained(retained_monitor_data) => {
                Self::Stale(retained_monitor_data.displayed_data)
            },
            Self::Pending | Self::Stale(_) | Self::StaleWithRetained(_) | Self::Unavailable => {
                Self::Unavailable
            },
            Self::Off => Self::Off,
        }
    }

    /// Keep the prior rows on screen when the new scope covers the same roots,
    /// and show nothing when it does not.
    ///
    /// Staleness survives the replacement. Rows a failed cycle already aged stay
    /// stale and non-actionable, because moving the cursor between two rows of
    /// one workspace observes nothing about the processes those rows describe.
    pub(super) fn superseded_by_scope(self, build_scope_key: &BuildScopeKey) -> Self {
        let (retained, monitor_staleness) = match self {
            Self::Fresh(monitor_data) => (monitor_data, MonitorStaleness::Live),
            Self::PendingWithRetained(retained_monitor_data) => {
                (retained_monitor_data.displayed_data, MonitorStaleness::Live)
            },
            Self::Stale(monitor_data) => (monitor_data, MonitorStaleness::Stale),
            Self::StaleWithRetained(retained_monitor_data) => (
                retained_monitor_data.displayed_data,
                MonitorStaleness::Stale,
            ),
            Self::Off | Self::Pending | Self::Unavailable => return Self::Pending,
        };
        if retained.build_scope_key().covered_scope_roots() != build_scope_key.covered_scope_roots()
        {
            return Self::Pending;
        }
        let retained_monitor_data = RetainedMonitorData::new(retained, build_scope_key.clone());
        match monitor_staleness {
            MonitorStaleness::Live => Self::PendingWithRetained(retained_monitor_data),
            MonitorStaleness::Stale => Self::StaleWithRetained(retained_monitor_data),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::time::Instant;

    use super::MonitorData;
    use super::MonitorDataActionability;
    use super::MonitorDisplay;
    use super::MonitorSnapshot;
    use super::MonitorStaleness;
    use super::RetainedMonitorData;
    use crate::build_monitor::BuildScopeKey;
    use crate::project::AbsolutePath;

    fn monitor_data() -> MonitorData {
        MonitorData::new(
            BuildScopeKey::for_test(AbsolutePath::from(Path::new("/"))),
            Vec::new(),
            Vec::new(),
            Instant::now(),
        )
    }

    fn retained_monitor_data(monitor_data: MonitorData) -> RetainedMonitorData {
        let build_scope_key = monitor_data.build_scope_key().clone();
        RetainedMonitorData::new(monitor_data, build_scope_key)
    }

    #[test]
    fn actionability_agrees_with_visible_staleness_for_every_snapshot_variant() {
        let monitor_data = monitor_data();
        let snapshots = [
            MonitorSnapshot::Off,
            MonitorSnapshot::Pending,
            MonitorSnapshot::PendingWithRetained(retained_monitor_data(monitor_data.clone())),
            MonitorSnapshot::Fresh(monitor_data.clone()),
            MonitorSnapshot::Stale(monitor_data.clone()),
            MonitorSnapshot::StaleWithRetained(retained_monitor_data(monitor_data)),
            MonitorSnapshot::Unavailable,
        ];

        for monitor_snapshot in snapshots {
            let visible_rows_are_live = matches!(
                monitor_snapshot.monitor_display(),
                MonitorDisplay::Rows {
                    monitor_staleness: MonitorStaleness::Live,
                    ..
                }
            );
            let snapshot_is_actionable = matches!(
                monitor_snapshot.actionability(),
                MonitorDataActionability::Actionable(_)
            );
            assert_eq!(snapshot_is_actionable, visible_rows_are_live);
        }
    }
}
