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

pub(crate) use activity::CompileActivity;
pub(crate) use activity::CompileActivityId;
pub(crate) use activity::CompiledCrateIdentity;
pub(crate) use activity::CompilerAttribution;
pub(crate) use activity::CompilerKind;
pub(crate) use activity::UnattributedCompileActivity;
pub(crate) use build_classifier::BuildClassifier;
pub(crate) use classify::CargoSubcommandRecognition;
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
pub(crate) use session::BuildProfileLabel;
pub(crate) use session::BuildSessionId;
pub(crate) use session::CargoCommandSelector;
pub(crate) use session::CargoSubcommand;
pub(crate) use session::LiveOwnedRoot;
pub(crate) use session::OwnedRootEvidence;
pub(crate) use session::OwnedRootLifecycle;
pub(crate) use session::SessionScope;
pub(crate) use session::TargetDirectoryEvidence;
pub(crate) use snapshot::BuildSessionActivity;
pub(crate) use snapshot::MonitorDisplay;
pub(crate) use snapshot::MonitorSessionOwnership;
pub(crate) use snapshot::MonitorSessionRow;
pub(crate) use snapshot::MonitorSnapshot;
pub(crate) use snapshot::MonitorStaleness;

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
    pub(crate) const fn monitor_snapshot(&self) -> &MonitorSnapshot { &self.monitor_snapshot }

    /// The session identities the last stored cycle showed within scope.
    #[cfg(test)]
    pub(crate) const fn live_session_ids(&self) -> &BTreeSet<BuildSessionId> {
        &self.live_session_ids
    }

    /// Show `monitor_snapshot`, for a test that needs the pane looking at a
    /// staged cycle rather than at one a live poll produced.
    #[cfg(test)]
    pub(crate) fn show_for_test(&mut self, monitor_snapshot: MonitorSnapshot) {
        self.monitor_snapshot = monitor_snapshot;
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

/// One root Cargo invocation the classification fixture stages, together with
/// the compilers running under it.
#[cfg(test)]
pub(crate) struct ClassifiedRoot {
    /// The pid the root Cargo process is observed under.
    pub(crate) root_pid:      u32,
    /// The pids of the compilers this root spawned, each attributed to this
    /// root's session and to no other.
    pub(crate) compiler_pids: &'static [u32],
}

/// Whether one of the staged roots is the Cargo run Cargo Port launched.
///
/// A column is [`MonitorSessionOwnership::Owned`] only when the classifier
/// attributes its root to the live owned run, so a test that needs the owned
/// column — the one the cursor may cross into captured output from — stages the
/// evidence rather than editing a row after the fact.
#[cfg(test)]
pub(crate) enum FixtureRootOwnership {
    /// Every staged root belongs to some other process on the host.
    AllExternal,
    /// The root observed under `root_pid` is the owned run's Cargo root.
    OwnedRoot {
        root_pid:     u32,
        owned_run_id: crate::tui::OwnedRunId,
    },
}

/// One fresh snapshot holding a session per root in `classified_roots`, built
/// by running the real classifier over an indexed checkout and storing the
/// cycle under the scope that covers it.
///
/// The renderer's own tests need columns that came from classification rather
/// than from hand-assembled rows, and [`MonitorData`](snapshot::MonitorData)
/// owns its rows, so the returned snapshot outlives the fixture's temporary
/// checkout.
#[cfg(test)]
pub(crate) fn classified_monitor_snapshot(
    classified_roots: &[ClassifiedRoot],
) -> Result<MonitorSnapshot, Box<dyn std::error::Error>> {
    classified_monitor_snapshot_with_ownership(classified_roots, &FixtureRootOwnership::AllExternal)
}

/// [`classified_monitor_snapshot`] with one staged root optionally attributed to
/// the owned run.
#[cfg(test)]
pub(crate) fn classified_monitor_snapshot_with_ownership(
    classified_roots: &[ClassifiedRoot],
    fixture_root_ownership: &FixtureRootOwnership,
) -> Result<MonitorSnapshot, Box<dyn std::error::Error>> {
    let mut classification_fixture = classify_tests::ClassificationFixture::new()?;
    let canonical_checkout_root = std::fs::canonicalize(&classification_fixture.checkout_root)?;
    let mut observed_processes = Vec::new();
    let mut owned_root_evidence = OwnedRootEvidence::NoLiveRoot;
    for classified_root in classified_roots {
        let cargo_root = classification_fixture
            .cargo_root_with_pid(classified_root.root_pid, &["cargo", "build"]);
        if let FixtureRootOwnership::OwnedRoot {
            root_pid,
            owned_run_id,
        } = fixture_root_ownership
            && *root_pid == classified_root.root_pid
        {
            owned_root_evidence = OwnedRootEvidence::Root(LiveOwnedRoot::new(
                *owned_run_id,
                cargo_root.identity().clone(),
                canonical_checkout_root.clone(),
                OwnedRootLifecycle::Live,
            ));
        }
        for compiler_pid in classified_root.compiler_pids {
            observed_processes
                .push(classification_fixture.compiler_under(*compiler_pid, &cargo_root));
        }
        observed_processes.push(cargo_root);
    }
    let build_classification =
        classification_fixture.classify_owned(&observed_processes, &owned_root_evidence);

    let mut build_monitor = BuildMonitor::default();
    build_monitor.record_classification(execution::CompletedBuildClassification::new(
        CompileMonitorGeneration::default(),
        BuildScopeKey::for_test(crate::project::AbsolutePath::from(
            canonical_checkout_root.as_path(),
        )),
        owned_root_evidence,
        Box::new(build_classification),
    ));
    Ok(build_monitor.monitor_snapshot().clone())
}
