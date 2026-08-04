//! Conversion of observed processes into build sessions and compile activities.
//!
//! - [`activity`] — compile-activity identity and compiler-to-session attribution
//! - [`build_classifier`] — the mutable caches and the filesystem work around the pure call
//! - [`classify`] — the pure snapshot-to-classification function
//! - [`constants`] — values shared by classification and its caches
//! - [`execution`] — one refresh cycle's classification demand and outcome
//! - [`poll`] — scope narrowing, snapshot storage, and failure aging
//! - [`scope`] — the roots-and-revisions projection of a monitor scope
//! - [`session`] — build-session identity, scope attribution, and session records
//! - [`snapshot`] — what the monitor has to show and how fresh it is

mod activity;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "the timing harness should fail on invalid fixture configuration"
)]
mod benchmarks;
mod build_classifier;
mod classify;
#[cfg(test)]
mod classify_tests;
mod constants;
mod execution;
mod poll;
#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod poll_tests;
mod scope;
mod session;
mod snapshot;

use std::collections::BTreeSet;

pub(crate) use build_classifier::BuildClassifier;
pub(crate) use execution::BuildClassificationExecutionFailure;
pub(crate) use execution::CompileClassificationCancellation;
pub(crate) use execution::CompileClassificationDemand;
pub(crate) use execution::CompileClassificationExecution;
pub(crate) use execution::CompileMonitorGeneration;
/// Named outside `build_monitor` only where a test builds the completion a
/// reconcile refuses or accepts; production reaches it through
/// [`CompileClassificationExecution::Completed`].
#[cfg(test)]
pub(crate) use execution::CompletedBuildClassification;
pub(crate) use scope::BuildScopeActionability;
pub(crate) use scope::BuildScopeKey;
pub(crate) use scope::CoveredScopeRoots;
pub(crate) use scope::LiveTargetDirectoryRevision;
pub(crate) use scope::ScopeRootCoverage;
pub(crate) use session::BuildSessionId;
pub(crate) use session::LiveOwnedRoot;
pub(crate) use session::OwnedRootEvidence;
pub(crate) use session::OwnedRootLifecycle;
pub(crate) use snapshot::MonitorSnapshot;

/// The compile monitor's own classification results and their lifetime.
///
/// It keeps only what is live: the session identities the last stored cycle
/// showed and the latest presentation snapshot. External history is not
/// accumulated — a session that ends disappears with the cycle that stops
/// reporting it.
#[derive(Debug, Default)]
pub(crate) struct BuildMonitor {
    monitor_snapshot: MonitorSnapshot,
    live_session_ids: BTreeSet<BuildSessionId>,
}

impl BuildMonitor {
    /// What the monitor currently has to show.
    #[cfg(test)]
    pub(crate) const fn monitor_snapshot(&self) -> &MonitorSnapshot { &self.monitor_snapshot }

    /// The session identities the last stored cycle showed within scope.
    #[cfg(test)]
    pub(crate) const fn live_session_ids(&self) -> &BTreeSet<BuildSessionId> {
        &self.live_session_ids
    }

    /// Drop everything the monitor was showing because visibility was switched
    /// off, leaving a state a reader can tell apart from enabled-and-waiting.
    pub(crate) fn switch_off(&mut self) {
        self.monitor_snapshot = MonitorSnapshot::Off;
        self.live_session_ids.clear();
    }

    /// Start an enabled scope with nothing to show yet, dropping whatever the
    /// previous enabled scope left behind.
    pub(crate) fn switch_on(&mut self) {
        self.monitor_snapshot = MonitorSnapshot::Pending;
        self.live_session_ids.clear();
    }
}
