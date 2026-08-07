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
#[expect(
    dead_code,
    reason = "opaque aggregate construction and submission stay separated from UI authority"
)]
mod termination;

pub(crate) use activity::CompileActivity;
pub(crate) use activity::CompileActivityId;
pub(crate) use activity::CompiledCrateIdentity;
pub(crate) use activity::CompilerAttribution;
pub(crate) use activity::CompilerKind;
pub(crate) use activity::UnattributedCompileActivity;
pub(crate) use build_classifier::BuildClassifier;
pub(crate) use classify::CargoSubcommandRecognition;
pub(crate) use constants::MAX_DESCENDANT_WALK_DEPTH;
pub(crate) use execution::BuildClassificationExecutionFailure;
pub(crate) use execution::BuildMonitoringRefreshCycleDemand;
pub(crate) use execution::BuildMonitoringRefreshCycleExecution;
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
pub(crate) use termination::BuildTerminationAuthorizationConstruction;
pub(crate) use termination::BuildTerminationLifecycle;
pub(crate) use termination::BuildTerminationLifecycleRegistry;
pub(crate) use termination::BuildTerminationObservationDemand;
pub(crate) use termination::BuildTerminationObservationExecution;
use termination::BuildTerminationState;
pub(crate) use termination::BuildTerminationSubmission;
pub(crate) use termination::BuildTerminationSubmissionRefusal;
pub(crate) use termination::BuildTerminationTerminalRecord;
pub(crate) use termination::BuildTerminationTransactionId;
pub(crate) use termination::OwnedTerminationSupport;
pub(crate) use termination::ScopeTerminationAuthorization;
pub(crate) use termination::SelectedBuildTerminationAuthorization;
pub(crate) use termination::observe_build_termination_demand;

/// The compile monitor's own classification results and their lifetime.
///
/// It keeps only what is live: the session identities the last stored cycle
/// showed and the latest presentation snapshot. External history is not
/// accumulated — a session that ends disappears with the cycle that stops
/// reporting it.
#[derive(Debug, Default)]
pub(crate) struct BuildMonitor {
    monitor_snapshot:               MonitorSnapshot,
    termination_lifecycle_registry: BuildTerminationLifecycleRegistry,
    termination_state:              BuildTerminationState,
}

impl BuildMonitor {
    /// What the monitor currently has to show.
    pub(crate) const fn monitor_snapshot(&self) -> &MonitorSnapshot { &self.monitor_snapshot }

    /// Lifecycle state retained independently of the replaceable snapshot.
    pub(crate) const fn termination_lifecycle_registry(
        &self,
    ) -> &BuildTerminationLifecycleRegistry {
        &self.termination_lifecycle_registry
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
        self.termination_lifecycle_registry.clear_terminal_entries();
        self.termination_state.clear_current_authorities();
    }

    /// Start an enabled scope with nothing to show yet, dropping whatever the
    /// previous enabled scope left behind.
    pub(crate) fn switch_on(&mut self) {
        self.monitor_snapshot = MonitorSnapshot::Pending;
        self.termination_state.clear_current_authorities();
    }

    /// Freeze the currently displayed authority for one selected session.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "selected authorization is constructed only after Output resolves one session"
        )
    )]
    pub(crate) fn selected_termination_authorization(
        &mut self,
        build_session_id: &BuildSessionId,
    ) -> BuildTerminationAuthorizationConstruction<SelectedBuildTerminationAuthorization> {
        match self.monitor_snapshot.actionability() {
            snapshot::MonitorDataActionability::Actionable(monitor_data) => {
                let Some(monitor_session_row) =
                    monitor_data
                        .session_rows()
                        .iter()
                        .find(|monitor_session_row| {
                            monitor_session_row.build_session_id() == build_session_id
                        })
                else {
                    return BuildTerminationAuthorizationConstruction::SessionNotActionable;
                };
                self.termination_state
                    .selected_authorization(monitor_data.build_scope_key(), monitor_session_row)
            },
            snapshot::MonitorDataActionability::NotActionable => self
                .termination_state
                .selected_authorization_for_inactive_snapshot(),
        }
    }

    /// Freeze the exact current row set only when each live root is actionable.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "scope authorization is constructed from the exact rendered root set"
        )
    )]
    pub(crate) fn scope_termination_authorization(
        &mut self,
    ) -> BuildTerminationAuthorizationConstruction<ScopeTerminationAuthorization> {
        match self.monitor_snapshot.actionability() {
            snapshot::MonitorDataActionability::Actionable(monitor_data) => self
                .termination_state
                .scope_authorization(monitor_data.build_scope_key(), monitor_data.session_rows()),
            snapshot::MonitorDataActionability::NotActionable => self
                .termination_state
                .scope_authorization_for_inactive_snapshot(),
        }
    }

    /// Start one selected-build transaction. The external worker receives its
    /// plan here; actor-issued owned tokens are submitted separately through
    /// [`Self::submit_owned_termination_targets`].
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "retained selected authorization is submitted only after confirmation"
        )
    )]
    pub(crate) fn submit_selected_termination(
        &mut self,
        selected_build_termination_authorization: SelectedBuildTerminationAuthorization,
        process_terminator: &mut crate::process_termination::ProcessTerminator,
        deadline: std::time::Instant,
    ) -> BuildTerminationSubmission {
        match self.monitor_snapshot.actionability() {
            snapshot::MonitorDataActionability::NotActionable => {
                return BuildTerminationSubmission::Refused(
                    BuildTerminationSubmissionRefusal::SnapshotNotActionable,
                );
            },
            snapshot::MonitorDataActionability::Actionable(monitor_data) => {
                match selected_build_termination_authorization.currency_against(monitor_data) {
                    termination::SelectedBuildTerminationAuthorizationCurrency::Current => {},
                    termination::SelectedBuildTerminationAuthorizationCurrency::ScopeChanged => {
                        return BuildTerminationSubmission::Refused(
                            BuildTerminationSubmissionRefusal::SelectedScopeChanged,
                        );
                    },
                    termination::SelectedBuildTerminationAuthorizationCurrency::SessionChanged => {
                        return BuildTerminationSubmission::Refused(
                            BuildTerminationSubmissionRefusal::SelectedSessionChanged,
                        );
                    },
                }
            },
        }
        self.termination_state.submit_selected(
            selected_build_termination_authorization,
            process_terminator,
            deadline,
            &mut self.termination_lifecycle_registry,
        )
    }

    /// Start one exact all-actionable scope transaction.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "retained scope authorization is submitted only after confirmation"
        )
    )]
    pub(crate) fn submit_scope_termination(
        &mut self,
        scope_termination_authorization: ScopeTerminationAuthorization,
        process_terminator: &mut crate::process_termination::ProcessTerminator,
        deadline: std::time::Instant,
    ) -> BuildTerminationSubmission {
        match self.monitor_snapshot.actionability() {
            snapshot::MonitorDataActionability::NotActionable => {
                return BuildTerminationSubmission::Refused(
                    BuildTerminationSubmissionRefusal::SnapshotNotActionable,
                );
            },
            snapshot::MonitorDataActionability::Actionable(monitor_data) => {
                match scope_termination_authorization.currency_against(monitor_data) {
                    termination::ScopeTerminationAuthorizationCurrency::Current => {},
                    termination::ScopeTerminationAuthorizationCurrency::CoveredRootsChanged => {
                        return BuildTerminationSubmission::Refused(
                            BuildTerminationSubmissionRefusal::ScopeRootsChanged,
                        );
                    },
                }
            },
        }
        self.termination_state.submit_scope(
            scope_termination_authorization,
            process_terminator,
            deadline,
            &mut self.termination_lifecycle_registry,
        )
    }

    /// Submit actor tokens without exposing them outside the monitor-owned
    /// transaction boundary.
    #[expect(
        dead_code,
        reason = "owned actor tokens are sent only through this transaction boundary"
    )]
    pub(crate) fn submit_owned_termination_targets(
        &mut self,
        build_termination_transaction_id: BuildTerminationTransactionId,
        submit: impl FnMut(
            crate::tui::OwnedRunTerminationToken,
        ) -> crate::tui::OwnedRunTerminationSubmission,
    ) {
        self.termination_state.submit_owned_targets(
            build_termination_transaction_id,
            submit,
            &mut self.termination_lifecycle_registry,
        );
        self.clear_terminal_termination_if_monitoring_off();
    }

    /// Reconcile one correlated external worker result.
    pub(crate) fn reconcile_external_termination(
        &mut self,
        termination_outcome_summary: &crate::process_termination::TerminationOutcomeSummary,
    ) {
        self.termination_state.reconcile_external_outcome(
            termination_outcome_summary,
            &mut self.termination_lifecycle_registry,
        );
        self.clear_terminal_termination_if_monitoring_off();
    }

    /// Reconcile one FIFO actor outcome after `Inflight` has applied it.
    pub(crate) fn reconcile_owned_termination(
        &mut self,
        owned_run_termination_outcome: crate::tui::OwnedRunTerminationOutcome,
    ) {
        self.termination_state.reconcile_owned_outcome(
            owned_run_termination_outcome,
            &mut self.termination_lifecycle_registry,
        );
        self.clear_terminal_termination_if_monitoring_off();
    }

    /// Reconcile the actor's later child-reap proof for an owned target.
    pub(crate) fn reconcile_owned_termination_finished(
        &mut self,
        owned_run_id: crate::tui::OwnedRunId,
    ) {
        self.termination_state
            .reconcile_owned_finished(owned_run_id, &mut self.termination_lifecycle_registry);
        self.clear_terminal_termination_if_monitoring_off();
    }

    /// End a bounded transaction whose remaining targets did not become
    /// terminal before its deadline.
    pub(crate) fn expire_termination_transaction(&mut self, now: std::time::Instant) {
        self.termination_state
            .expire(now, &mut self.termination_lifecycle_registry);
        self.clear_terminal_termination_if_monitoring_off();
    }

    /// Build one immutable descendant-pass request only for transaction demand.
    pub(crate) fn termination_observation_demand(
        &self,
        process_refresh_consumer_demand: crate::process_observation::ProcessRefreshConsumerDemand,
    ) -> BuildTerminationObservationDemand {
        self.termination_state
            .termination_observation_demand(process_refresh_consumer_demand)
    }

    /// The next time the active transaction needs the shared observer.
    pub(crate) const fn termination_refresh_schedule(
        &self,
    ) -> crate::process_observation::TerminationTransactionRefreshSchedule {
        self.termination_state.termination_refresh_schedule()
    }

    /// Reconcile one sole-observer descendant pass and submit its current leaves.
    pub(crate) fn reconcile_termination_observation(
        &mut self,
        build_termination_observation_execution: BuildTerminationObservationExecution,
        process_terminator: &mut crate::process_termination::ProcessTerminator,
    ) {
        let BuildTerminationObservationExecution::Completed(
            completed_build_termination_observation,
        ) = build_termination_observation_execution
        else {
            return;
        };
        self.termination_state.reconcile_termination_observation(
            completed_build_termination_observation,
            process_terminator,
            &mut self.termination_lifecycle_registry,
        );
        self.clear_terminal_termination_if_monitoring_off();
    }

    fn clear_terminal_termination_if_monitoring_off(&mut self) {
        if matches!(self.monitor_snapshot, MonitorSnapshot::Off) {
            self.termination_lifecycle_registry.clear_terminal_entries();
        }
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
