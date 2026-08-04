//! What the compile monitor currently has to show, and how fresh that data is.
//!
//! Every data-bearing state carries the instant its evidence was observed, so
//! age is re-derived wherever it is needed — at render time and at the moment a
//! kill is authorized — instead of being implied by which state holds the data.

use std::time::Instant;

use super::activity::AttributedSession;
use super::activity::CompileActivity;
use super::activity::CompilerKind;
use super::scope::BuildScopeKey;
use super::session::BuildSession;
use super::session::BuildSessionId;
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
/// [`crate::process_observation::snapshot_builder::ObservedCandidateRole`]
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
    /// this cycle.
    ActiveWithoutCompiler,
}

impl BuildSessionActivity {
    /// Decide one session's state from the compile activities attributed to it.
    ///
    /// Compilers outrank the build script and linker children they spawn, so a
    /// session that is compiling reads as compiling even while one crate's
    /// linker is running.
    fn from_attributed_activities<'a>(
        build_session_id: &BuildSessionId,
        compile_activities: impl Iterator<Item = &'a CompileActivity>,
    ) -> Self {
        let mut build_session_activity = Self::ActiveWithoutCompiler;
        for compile_activity in compile_activities {
            let AttributedSession::Session(attributed_session_id) =
                compile_activity.compiler_attribution().attributed_session()
            else {
                continue;
            };
            if attributed_session_id != build_session_id {
                continue;
            }
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
            Self::ActiveWithoutCompiler => 0,
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

/// One build session as the monitor pane shows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MonitorSessionRow {
    build_session:          BuildSession,
    build_session_activity: BuildSessionActivity,
    session_ownership:      MonitorSessionOwnership,
}

impl MonitorSessionRow {
    /// The immutable session record this row renders.
    #[cfg(test)]
    pub(crate) const fn build_session(&self) -> &BuildSession { &self.build_session }

    /// This session's stable key.
    pub(crate) const fn build_session_id(&self) -> &BuildSessionId {
        self.build_session.build_session_id()
    }

    /// What this session is doing, as attributed activities name it.
    #[cfg(test)]
    pub(crate) const fn build_session_activity(&self) -> BuildSessionActivity {
        self.build_session_activity
    }

    /// Whether Cargo Port owns the run behind this session.
    #[cfg(test)]
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
    build_scope_key: BuildScopeKey,
    session_rows:    Vec<MonitorSessionRow>,
    observed_at:     Instant,
}

impl MonitorData {
    /// Narrow one classification to a scope and pair each surviving session
    /// with its observed state and ownership.
    pub(super) const fn new(
        build_scope_key: BuildScopeKey,
        session_rows: Vec<MonitorSessionRow>,
        observed_at: Instant,
    ) -> Self {
        Self {
            build_scope_key,
            session_rows,
            observed_at,
        }
    }

    /// The scope these rows were classified under.
    pub(crate) const fn build_scope_key(&self) -> &BuildScopeKey { &self.build_scope_key }

    /// Sessions in first-seen order, as classification produced them.
    #[cfg(test)]
    pub(crate) fn session_rows(&self) -> &[MonitorSessionRow] { &self.session_rows }

    /// When the evidence behind these rows was observed.
    #[cfg(test)]
    pub(crate) const fn observed_at(&self) -> Instant { self.observed_at }

    /// Build one session row from a classified session and the cycle's
    /// activities.
    pub(super) fn session_row<'a>(
        build_session: BuildSession,
        compile_activities: impl Iterator<Item = &'a CompileActivity>,
        session_ownership: MonitorSessionOwnership,
    ) -> MonitorSessionRow {
        let build_session_activity = BuildSessionActivity::from_attributed_activities(
            build_session.build_session_id(),
            compile_activities,
        );
        MonitorSessionRow {
            build_session,
            build_session_activity,
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
pub(crate) struct RetainedMonitorData(MonitorData);

impl RetainedMonitorData {
    /// The prior cycle's rows, still current enough to render and act on.
    #[cfg(test)]
    pub(crate) const fn monitor_data(&self) -> &MonitorData { &self.0 }
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
    /// Nothing observable is left to show.
    Unavailable,
}

/// Whether the monitor's current data may authorize a termination.
///
/// Retained data is actionable: the termination path re-resolves a retained
/// identity against the live process snapshot before it signals, so a session
/// that ended meanwhile is simply not found.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MonitorDataActionability<'a> {
    /// These rows may be acted on.
    Actionable(&'a MonitorData),
    /// Nothing shown may be acted on.
    NotActionable,
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

impl MonitorSnapshot {
    /// The rows a termination may act on.
    #[cfg(test)]
    pub(crate) const fn actionability(&self) -> MonitorDataActionability<'_> {
        match self {
            Self::Fresh(monitor_data) => MonitorDataActionability::Actionable(monitor_data),
            Self::PendingWithRetained(retained_monitor_data) => {
                MonitorDataActionability::Actionable(retained_monitor_data.monitor_data())
            },
            Self::Off | Self::Pending | Self::Stale(_) | Self::Unavailable => {
                MonitorDataActionability::NotActionable
            },
        }
    }

    /// When the shown evidence was observed, for age derived at read time.
    #[cfg(test)]
    pub(crate) const fn observation(&self) -> MonitorObservation {
        match self {
            Self::Fresh(monitor_data) | Self::Stale(monitor_data) => {
                MonitorObservation::Observed(monitor_data.observed_at())
            },
            Self::PendingWithRetained(retained_monitor_data) => {
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
                Self::Stale(retained_monitor_data.0)
            },
            Self::Pending | Self::Stale(_) | Self::Unavailable => Self::Unavailable,
            Self::Off => Self::Off,
        }
    }

    /// Keep the prior rows on screen when the new scope covers the same roots,
    /// and show nothing when it does not.
    pub(super) fn superseded_by_scope(self, build_scope_key: &BuildScopeKey) -> Self {
        let retained = match self {
            Self::Fresh(monitor_data) | Self::Stale(monitor_data) => monitor_data,
            Self::PendingWithRetained(retained_monitor_data) => retained_monitor_data.0,
            Self::Off | Self::Pending | Self::Unavailable => return Self::Pending,
        };
        if retained.build_scope_key().covered_scope_roots() == build_scope_key.covered_scope_roots()
        {
            Self::PendingWithRetained(RetainedMonitorData(retained))
        } else {
            Self::Pending
        }
    }
}
