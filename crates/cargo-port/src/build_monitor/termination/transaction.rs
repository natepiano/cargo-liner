//! One bounded mixed-backend termination transaction and its reconciliation.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::time::Duration;
use std::time::Instant;

use super::super::constants::TERMINATION_DESCENDANT_REFRESH_INTERVAL;
use super::super::scope::BuildScopeKey;
use super::super::session::BuildSessionId;
use super::super::session::SessionScope;
use super::BuildTerminationTransactionCompletion;
use super::authority::BuildTerminationAuthority;
use super::authority::BuildTerminationAuthorizationConstruction;
use super::authority::FrozenBuildTerminationTarget;
use super::authority::OutputBuildSetTerminationAuthorization;
use super::authority::OutputBuildSetTerminationAvailability;
use super::authority::SelectedBuildTerminationAuthorization;
use super::authority::SelectedBuildTerminationAvailability;
use super::lifecycle::BuildTerminationDisplayIdentity;
use super::lifecycle::BuildTerminationLifecycleRegistry;
use super::lifecycle::BuildTerminationTargetResult;
use super::lifecycle::ExternalBuildTerminationResult;
use super::lifecycle::OwnedBuildTerminationDeadline;
use super::lifecycle::OwnedBuildTerminationResult;
use super::lifecycle::OwnedBuildTerminationSubmissionRefusal;
use super::observation::BuildTerminationObservationDemand;
use super::observation::BuildTerminationObservationRequest;
use super::observation::BuildTerminationRootObservationRequest;
use super::observation::BuildTerminationRootPresence;
use super::observation::CompletedBuildTerminationObservation;
use super::observation::NewActionableTerminationDescendant;
use crate::process_observation::identity::ProcessIdentity;
use crate::process_termination::AdmittedTerminationDescendantObservation;
use crate::process_termination::AdmittedTerminationDescendantPresence;
use crate::process_termination::ExternalProcessTerminationCapability;
use crate::process_termination::ProcessTerminator;
use crate::process_termination::TerminationDispatchOutcome;
use crate::process_termination::TerminationError;
use crate::process_termination::TerminationExecutionTarget;
use crate::process_termination::TerminationExecutionTargetRole;
use crate::process_termination::TerminationOutcomeSummary;
use crate::process_termination::TerminationPlanCreation;
use crate::process_termination::TerminationRequestId;
use crate::process_termination::TerminationTargetId;
use crate::process_termination::TerminationTargetResult;
use crate::tui::OwnedProcessGroupSignalOutcome;
use crate::tui::OwnedRunId;
use crate::tui::OwnedRunTerminationOutcome;
use crate::tui::OwnedRunTerminationSubmission;
use crate::tui::OwnedRunTerminationToken;

/// The monotonic owner identity for one build termination transaction.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct BuildTerminationTransactionId(pub(super) NonZeroU64);

/// The user-visible set whose frozen authority owns one transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationTransactionTargetSet {
    /// One root selected through the Output cursor.
    SelectedBuild,
    /// Every live actionable root row the Output snapshot represented.
    OutputBuildSet,
}

/// Whether current Output rows appeared after an output-build-set confirmation
/// froze its immutable target identities.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AdditionalBuildExclusion {
    /// The current display contains no root added after confirmation.
    NoAdditionalBuilds,
    /// Current display rows included this many roots outside frozen authority.
    Excluded { count: usize },
}

/// The fixed grace period available to one submitted build termination.
pub(crate) const BUILD_TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);

/// The expiry instant derived when App submits one frozen termination request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BuildTerminationDeadline(Instant);

impl BuildTerminationDeadline {
    /// Use the shared selected/scope termination timeout from submission time.
    pub(crate) fn from_submission_time(submitted_at: Instant) -> Self {
        Self(submitted_at + BUILD_TERMINATION_TIMEOUT)
    }

    pub(crate) const fn expires_at(self) -> Instant { self.0 }

    fn has_expired(self, now: Instant) -> bool { now >= self.0 }
}

/// Whether a monitor operation completed one transaction that App must present.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationCompletionTransition {
    /// The operation left every active transaction nonterminal.
    NoCompletion,
    /// The operation completed one transaction exactly once.
    Completed(BuildTerminationTransactionCompletion),
}

/// Result of attempting to start one frozen transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationSubmission {
    /// The transaction owns its lifecycle and backend correlations.
    Submitted(BuildTerminationTransactionId),
    /// An earlier transaction is still nonterminal; nothing was enqueued.
    Busy,
    /// The current monitor state no longer permits the retained authorization.
    Refused(BuildTerminationSubmissionRefusal),
    /// A transaction or target identity could not be allocated without reuse.
    IdentityExhausted,
}

/// Why retained authorization was refused before transaction state changed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationSubmissionRefusal {
    /// Monitoring has no current actionable snapshot.
    SnapshotNotActionable,
    /// A selected authorization's exact scope key is no longer current.
    SelectedScopeChanged,
    /// A selected authorization's exact session is no longer current.
    SelectedSessionChanged,
    /// An output-build-set authorization's covered roots changed.
    CoveredScopeRootsChanged,
}

#[derive(Debug)]
pub(crate) struct BuildTerminationTransaction {
    transaction_id:             BuildTerminationTransactionId,
    target_set:                 BuildTerminationTransactionTargetSet,
    additional_build_exclusion: AdditionalBuildExclusion,
    deadline:                   BuildTerminationDeadline,
    pending_targets:            BTreeMap<TerminationTargetId, BuildSessionId>,
    owned_targets:              BTreeMap<OwnedRunId, OwnedPendingTerminationTarget>,
    external_roots:             BTreeMap<TerminationTargetId, FrozenExternalTerminationRoot>,
    admitted_descendants: BTreeMap<TerminationTargetId, AdmittedExternalTerminationDescendant>,
    external_pass_state:        ExternalTerminationPassState,
    terminal_results:           BTreeMap<BuildSessionId, Vec<BuildTerminationTargetResult>>,
}

#[derive(Debug)]
struct FrozenExternalTerminationRoot {
    session_scope:    SessionScope,
    root_identity:    ProcessIdentity,
    capability_state: ExternalTerminationCapabilityState,
}

#[derive(Debug)]
struct AdmittedExternalTerminationDescendant {
    root_target_id:   TerminationTargetId,
    process_identity: ProcessIdentity,
    parent_identity:  ProcessIdentity,
    depth_from_root:  usize,
    capability_state: ExternalTerminationCapabilityState,
}

#[derive(Debug)]
enum ExternalTerminationCapabilityState {
    Ready(ExternalProcessTerminationCapability),
    Submitted,
}

#[derive(Debug)]
enum ExternalTerminationPassState {
    AwaitingObservation {
        not_before: Instant,
    },
    AwaitingWorker {
        termination_request_id: TerminationRequestId,
        target_ids:             BTreeSet<TerminationTargetId>,
    },
    Settled,
}

#[derive(Clone, Copy, Debug)]
enum ExternalTerminationPassSubmissionFailure {
    RequestIdentitiesExhausted,
    WorkerUnavailable,
    RequestCorrelationMismatch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalPassDispatch {
    AwaitingWorker,
    ReconcileTerminalState,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExternalPendingTarget {
    Resolved {
        pid:  u32,
        role: TerminationExecutionTargetRole,
    },
    CorrelationUnavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct OwnedPendingTerminationTarget {
    termination_target_id:       TerminationTargetId,
    owned_run_termination_token: OwnedRunTerminationToken,
    progress:                    OwnedBuildTerminationProgress,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnedBuildTerminationProgress {
    ReadyToSubmit,
    SignalOutcomePending,
    ReapingAfterSignal,
}

impl BuildTerminationTransaction {
    fn is_terminal(&self) -> bool { self.pending_targets.is_empty() }

    fn has_pending_external_targets(&self) -> bool {
        self.external_roots
            .keys()
            .chain(self.admitted_descendants.keys())
            .any(|termination_target_id| self.pending_targets.contains_key(termination_target_id))
    }
}

#[derive(Debug, Default)]
enum ActiveBuildTerminationTransaction {
    #[default]
    Idle,
    Active(Box<BuildTerminationTransaction>),
}

/// Owns active transaction state and current snapshot-derived authority.
#[derive(Debug)]
pub(crate) struct BuildTerminationState {
    next_transaction_id: NonZeroU64,
    next_target_id:      NonZeroU64,
    current_authorities: BTreeMap<BuildSessionId, BuildTerminationAuthority>,
    active_transaction:  ActiveBuildTerminationTransaction,
}

impl Default for BuildTerminationState {
    fn default() -> Self {
        Self {
            next_transaction_id: NonZeroU64::MIN,
            next_target_id:      NonZeroU64::MIN,
            current_authorities: BTreeMap::new(),
            active_transaction:  ActiveBuildTerminationTransaction::Idle,
        }
    }
}

impl BuildTerminationState {
    pub(super) const fn lifecycle_transaction_is_active(&self) -> bool {
        matches!(
            self.active_transaction,
            ActiveBuildTerminationTransaction::Active(_)
        )
    }

    pub(in crate::build_monitor) fn replace_current_authorities(
        &mut self,
        current_authorities: BTreeMap<BuildSessionId, BuildTerminationAuthority>,
    ) {
        if !self.lifecycle_transaction_is_active() {
            self.current_authorities = current_authorities;
        }
    }

    pub(in crate::build_monitor) fn clear_current_authorities(&mut self) {
        self.current_authorities.clear();
    }

    /// Read the selected session's authority state without consuming it.
    pub(in crate::build_monitor) fn selected_termination_availability(
        &self,
        build_session_id: &BuildSessionId,
    ) -> SelectedBuildTerminationAvailability {
        if self.lifecycle_transaction_is_active() {
            return SelectedBuildTerminationAvailability::Busy;
        }
        if self.current_authorities.contains_key(build_session_id) {
            SelectedBuildTerminationAvailability::Available
        } else {
            SelectedBuildTerminationAvailability::SessionNotActionable
        }
    }

    /// Read the selected session's state when no actionable snapshot exists.
    pub(in crate::build_monitor) const fn selected_termination_availability_for_inactive_snapshot(
        &self,
    ) -> SelectedBuildTerminationAvailability {
        if self.lifecycle_transaction_is_active() {
            SelectedBuildTerminationAvailability::Busy
        } else {
            SelectedBuildTerminationAvailability::SnapshotNotActionable
        }
    }

    /// Read exact Output-build-set actionability without taking any authority.
    pub(in crate::build_monitor) fn output_build_set_termination_availability(
        &self,
        monitor_session_rows: &[super::super::snapshot::MonitorSessionRow],
    ) -> OutputBuildSetTerminationAvailability {
        if self.lifecycle_transaction_is_active() {
            return OutputBuildSetTerminationAvailability::Busy;
        }
        if monitor_session_rows.is_empty()
            || monitor_session_rows.iter().any(|monitor_session_row| {
                !self
                    .current_authorities
                    .contains_key(monitor_session_row.build_session_id())
            })
        {
            OutputBuildSetTerminationAvailability::BuildSetNotFullyActionable
        } else {
            OutputBuildSetTerminationAvailability::Available
        }
    }

    /// Read output-build-set actionability when the snapshot is unavailable.
    pub(in crate::build_monitor) const fn output_build_set_termination_availability_for_inactive_snapshot(
        &self,
    ) -> OutputBuildSetTerminationAvailability {
        if self.lifecycle_transaction_is_active() {
            OutputBuildSetTerminationAvailability::Busy
        } else {
            OutputBuildSetTerminationAvailability::SnapshotNotActionable
        }
    }

    pub(in crate::build_monitor) fn selected_authorization(
        &mut self,
        build_scope_key: &BuildScopeKey,
        monitor_session_row: &super::super::snapshot::MonitorSessionRow,
    ) -> BuildTerminationAuthorizationConstruction<SelectedBuildTerminationAuthorization> {
        if self.lifecycle_transaction_is_active() {
            return BuildTerminationAuthorizationConstruction::Busy;
        }
        let build_session_id = monitor_session_row.build_session_id();
        let Some(build_termination_authority) = self.current_authorities.remove(build_session_id)
        else {
            return BuildTerminationAuthorizationConstruction::SessionNotActionable;
        };
        BuildTerminationAuthorizationConstruction::Authorized(
            SelectedBuildTerminationAuthorization {
                session_id:       build_session_id.clone(),
                scope_key:        build_scope_key.clone(),
                authority:        build_termination_authority,
                display_identity: BuildTerminationDisplayIdentity::from_monitor_session_row(
                    monitor_session_row,
                ),
            },
        )
    }

    pub(in crate::build_monitor) const fn selected_authorization_for_inactive_snapshot(
        &self,
    ) -> BuildTerminationAuthorizationConstruction<SelectedBuildTerminationAuthorization> {
        if self.lifecycle_transaction_is_active() {
            BuildTerminationAuthorizationConstruction::Busy
        } else {
            BuildTerminationAuthorizationConstruction::SnapshotNotActionable
        }
    }

    pub(in crate::build_monitor) fn output_build_set_authorization(
        &mut self,
        build_scope_key: &BuildScopeKey,
        monitor_session_rows: &[super::super::snapshot::MonitorSessionRow],
    ) -> BuildTerminationAuthorizationConstruction<OutputBuildSetTerminationAuthorization> {
        if self.lifecycle_transaction_is_active() {
            return BuildTerminationAuthorizationConstruction::Busy;
        }
        if monitor_session_rows.is_empty()
            || monitor_session_rows.iter().any(|monitor_session_row| {
                !self
                    .current_authorities
                    .contains_key(monitor_session_row.build_session_id())
            })
        {
            return BuildTerminationAuthorizationConstruction::BuildSetNotFullyActionable;
        }
        let mut targets = Vec::with_capacity(monitor_session_rows.len());
        for monitor_session_row in monitor_session_rows {
            let build_session_id = monitor_session_row.build_session_id();
            let Some(build_termination_authority) =
                self.current_authorities.remove(build_session_id)
            else {
                return BuildTerminationAuthorizationConstruction::BuildSetNotFullyActionable;
            };
            targets.push(FrozenBuildTerminationTarget {
                session_id:       build_session_id.clone(),
                authority:        build_termination_authority,
                display_identity: BuildTerminationDisplayIdentity::from_monitor_session_row(
                    monitor_session_row,
                ),
            });
        }
        BuildTerminationAuthorizationConstruction::Authorized(
            OutputBuildSetTerminationAuthorization {
                scope_key: build_scope_key.clone(),
                targets,
            },
        )
    }

    pub(in crate::build_monitor) const fn output_build_set_authorization_for_inactive_snapshot(
        &self,
    ) -> BuildTerminationAuthorizationConstruction<OutputBuildSetTerminationAuthorization> {
        if self.lifecycle_transaction_is_active() {
            BuildTerminationAuthorizationConstruction::Busy
        } else {
            BuildTerminationAuthorizationConstruction::SnapshotNotActionable
        }
    }

    pub(in crate::build_monitor) fn submit_selected(
        &mut self,
        selected_build_termination_authorization: SelectedBuildTerminationAuthorization,
        process_terminator: &mut ProcessTerminator,
        build_termination_deadline: BuildTerminationDeadline,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationSubmission {
        let SelectedBuildTerminationAuthorization {
            session_id,
            scope_key: _,
            authority,
            display_identity,
        } = selected_build_termination_authorization;
        self.submit_targets(
            vec![FrozenBuildTerminationTarget {
                session_id,
                authority,
                display_identity,
            }],
            BuildTerminationTransactionTargetSet::SelectedBuild,
            AdditionalBuildExclusion::NoAdditionalBuilds,
            process_terminator,
            build_termination_deadline,
            lifecycle_registry,
        )
    }

    pub(in crate::build_monitor) fn submit_output_build_set(
        &mut self,
        output_build_set_termination_authorization: OutputBuildSetTerminationAuthorization,
        additional_build_exclusion: AdditionalBuildExclusion,
        process_terminator: &mut ProcessTerminator,
        build_termination_deadline: BuildTerminationDeadline,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationSubmission {
        self.submit_targets(
            output_build_set_termination_authorization.targets,
            BuildTerminationTransactionTargetSet::OutputBuildSet,
            additional_build_exclusion,
            process_terminator,
            build_termination_deadline,
            lifecycle_registry,
        )
    }

    /// Submit a fixed output-build-set fixture without later-row exclusions.
    #[cfg(test)]
    fn submit_output_build_set_for_test(
        &mut self,
        output_build_set_termination_authorization: OutputBuildSetTerminationAuthorization,
        process_terminator: &mut ProcessTerminator,
        build_termination_deadline: BuildTerminationDeadline,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationSubmission {
        self.submit_output_build_set(
            output_build_set_termination_authorization,
            AdditionalBuildExclusion::NoAdditionalBuilds,
            process_terminator,
            build_termination_deadline,
            lifecycle_registry,
        )
    }

    fn submit_targets(
        &mut self,
        frozen_targets: Vec<FrozenBuildTerminationTarget>,
        build_termination_transaction_target_set: BuildTerminationTransactionTargetSet,
        additional_build_exclusion: AdditionalBuildExclusion,
        _: &mut ProcessTerminator,
        build_termination_deadline: BuildTerminationDeadline,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationSubmission {
        if self.lifecycle_transaction_is_active() {
            return BuildTerminationSubmission::Busy;
        }
        let Some(next_transaction_id) = self.next_transaction_id.checked_add(1) else {
            return BuildTerminationSubmission::IdentityExhausted;
        };
        let build_termination_transaction_id =
            BuildTerminationTransactionId(self.next_transaction_id);
        let mut following_target_id = self.next_target_id;
        let mut allocated_target_ids = Vec::with_capacity(frozen_targets.len());
        for _ in &frozen_targets {
            let Some(next_target_id) = following_target_id.checked_add(1) else {
                return BuildTerminationSubmission::IdentityExhausted;
            };
            allocated_target_ids.push(TerminationTargetId::from_non_zero(following_target_id));
            following_target_id = next_target_id;
        }
        let mut pending_targets = BTreeMap::new();
        let mut owned_targets = BTreeMap::new();
        let mut external_roots = BTreeMap::new();
        for (frozen_build_termination_target, termination_target_id) in
            frozen_targets.into_iter().zip(allocated_target_ids)
        {
            lifecycle_registry.mark_terminating(
                build_termination_transaction_id,
                frozen_build_termination_target.display_identity,
            );
            pending_targets.insert(
                termination_target_id,
                frozen_build_termination_target.session_id.clone(),
            );
            match frozen_build_termination_target.authority {
                BuildTerminationAuthority::Owned(owned_build_termination_authority) => {
                    owned_targets.insert(
                        owned_build_termination_authority.owned_run_id,
                        OwnedPendingTerminationTarget {
                            termination_target_id,
                            owned_run_termination_token: owned_build_termination_authority
                                .owned_run_termination_token,
                            progress: OwnedBuildTerminationProgress::ReadyToSubmit,
                        },
                    );
                },
                BuildTerminationAuthority::External(external_build_termination_authority) => {
                    external_roots.insert(
                        termination_target_id,
                        FrozenExternalTerminationRoot {
                            session_scope:    external_build_termination_authority.session_scope,
                            root_identity:    external_build_termination_authority.root_identity,
                            capability_state: ExternalTerminationCapabilityState::Ready(
                                external_build_termination_authority
                                    .external_process_termination_capability,
                            ),
                        },
                    );
                },
            }
        }
        self.next_transaction_id = next_transaction_id;
        self.next_target_id = following_target_id;
        let external_pass_state = if external_roots.is_empty() {
            ExternalTerminationPassState::Settled
        } else {
            ExternalTerminationPassState::AwaitingObservation {
                not_before: Instant::now(),
            }
        };
        self.active_transaction =
            ActiveBuildTerminationTransaction::Active(Box::new(BuildTerminationTransaction {
                transaction_id: build_termination_transaction_id,
                target_set: build_termination_transaction_target_set,
                additional_build_exclusion,
                deadline: build_termination_deadline,
                pending_targets,
                owned_targets,
                external_roots,
                admitted_descendants: BTreeMap::new(),
                external_pass_state,
                terminal_results: BTreeMap::new(),
            }));
        BuildTerminationSubmission::Submitted(build_termination_transaction_id)
    }

    /// Submit the actor-issued tokens for the active transaction without
    /// exposing them to UI callers.
    pub(in crate::build_monitor) fn submit_owned_targets(
        &mut self,
        transaction_id: BuildTerminationTransactionId,
        mut submit: impl FnMut(OwnedRunTerminationToken) -> OwnedRunTerminationSubmission,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if build_termination_transaction.transaction_id != transaction_id {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        let owned_targets = build_termination_transaction.owned_targets.clone();
        for (owned_run_id, owned_pending_termination_target) in owned_targets {
            if owned_pending_termination_target.progress
                != OwnedBuildTerminationProgress::ReadyToSubmit
            {
                continue;
            }
            match submit(owned_pending_termination_target.owned_run_termination_token) {
                OwnedRunTerminationSubmission::Submitted(submitted_owned_run_id)
                    if submitted_owned_run_id == owned_run_id =>
                {
                    if let Some(pending_target) = build_termination_transaction
                        .owned_targets
                        .get_mut(&owned_run_id)
                    {
                        pending_target.progress =
                            OwnedBuildTerminationProgress::SignalOutcomePending;
                    }
                },
                OwnedRunTerminationSubmission::Submitted(_) => complete_owned_target(
                    build_termination_transaction,
                    owned_run_id,
                    OwnedBuildTerminationResult::SubmissionRefused {
                        owned_run_id,
                        refusal: OwnedBuildTerminationSubmissionRefusal::RunCorrelationMismatch,
                    },
                ),
                OwnedRunTerminationSubmission::RequestAlreadyPending => complete_owned_target(
                    build_termination_transaction,
                    owned_run_id,
                    OwnedBuildTerminationResult::SubmissionRefused {
                        owned_run_id,
                        refusal: OwnedBuildTerminationSubmissionRefusal::RequestAlreadyPending,
                    },
                ),
                OwnedRunTerminationSubmission::TokenRefused => complete_owned_target(
                    build_termination_transaction,
                    owned_run_id,
                    OwnedBuildTerminationResult::SubmissionRefused {
                        owned_run_id,
                        refusal: OwnedBuildTerminationSubmissionRefusal::TokenRefused,
                    },
                ),
                OwnedRunTerminationSubmission::ActorUnavailable => complete_owned_target(
                    build_termination_transaction,
                    owned_run_id,
                    OwnedBuildTerminationResult::SubmissionRefused {
                        owned_run_id,
                        refusal: OwnedBuildTerminationSubmissionRefusal::ActorUnavailable,
                    },
                ),
            }
        }
        self.complete_terminal_transaction(lifecycle_registry)
    }

    pub(in crate::build_monitor) fn termination_observation_demand(
        &self,
        process_refresh_consumer_demand: crate::process_observation::ProcessRefreshConsumerDemand,
    ) -> BuildTerminationObservationDemand {
        if !process_refresh_consumer_demand.includes_termination_transaction() {
            return BuildTerminationObservationDemand::NotRequested;
        }
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &self.active_transaction
        else {
            return BuildTerminationObservationDemand::NotRequested;
        };
        if !matches!(
            build_termination_transaction.external_pass_state,
            ExternalTerminationPassState::AwaitingObservation { .. }
        ) {
            return BuildTerminationObservationDemand::NotRequested;
        }
        BuildTerminationObservationDemand::Requested(BuildTerminationObservationRequest {
            transaction_id:       build_termination_transaction.transaction_id,
            frozen_roots:         build_termination_transaction
                .external_roots
                .iter()
                .map(
                    |(semantic_target_id, root)| BuildTerminationRootObservationRequest {
                        semantic_target_id: *semantic_target_id,
                        session_scope:      root.session_scope.clone(),
                        root_identity:      root.root_identity.clone(),
                    },
                )
                .collect(),
            admitted_descendants: build_termination_transaction
                .admitted_descendants
                .iter()
                .map(|(semantic_target_id, descendant)| {
                    AdmittedTerminationDescendantObservation::new(
                        *semantic_target_id,
                        descendant.process_identity.clone(),
                    )
                })
                .collect(),
        })
    }

    pub(in crate::build_monitor) const fn termination_refresh_schedule(
        &self,
    ) -> crate::process_observation::TerminationTransactionRefreshSchedule {
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &self.active_transaction
        else {
            return crate::process_observation::TerminationTransactionRefreshSchedule::NotScheduled;
        };
        match build_termination_transaction.external_pass_state {
            ExternalTerminationPassState::AwaitingObservation { not_before } => {
                crate::process_observation::TerminationTransactionRefreshSchedule::At(not_before)
            },
            ExternalTerminationPassState::AwaitingWorker { .. }
            | ExternalTerminationPassState::Settled => {
                crate::process_observation::TerminationTransactionRefreshSchedule::NotScheduled
            },
        }
    }

    pub(in crate::build_monitor) fn reconcile_termination_observation(
        &mut self,
        completed_build_termination_observation: CompletedBuildTerminationObservation,
        process_terminator: &mut ProcessTerminator,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let next_target_id = &mut self.next_target_id;
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if build_termination_transaction.transaction_id
            != completed_build_termination_observation.transaction_id
            || !matches!(
                build_termination_transaction.external_pass_state,
                ExternalTerminationPassState::AwaitingObservation { .. }
            )
        {
            return BuildTerminationCompletionTransition::NoCompletion;
        }

        reconcile_observed_roots(
            build_termination_transaction,
            &completed_build_termination_observation.root_presence,
        );
        reconcile_observed_descendants(
            build_termination_transaction,
            &completed_build_termination_observation.descendant_presence,
        );
        admit_new_descendants(
            next_target_id,
            build_termination_transaction,
            completed_build_termination_observation.admitted_descendants,
        );

        let (execution_targets, submitted_target_ids) = next_external_execution_targets(
            build_termination_transaction,
            &completed_build_termination_observation.root_presence,
        );

        if dispatch_external_pass(
            build_termination_transaction,
            process_terminator,
            execution_targets,
            submitted_target_ids,
        ) == ExternalPassDispatch::ReconcileTerminalState
        {
            return self.complete_terminal_transaction(lifecycle_registry);
        }
        BuildTerminationCompletionTransition::NoCompletion
    }

    pub(in crate::build_monitor) fn reconcile_external_outcome(
        &mut self,
        termination_outcome_summary: &TerminationOutcomeSummary,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        let termination_request_id = termination_outcome_summary.termination_request_id();
        let ExternalTerminationPassState::AwaitingWorker {
            termination_request_id: expected_request_id,
            target_ids: expected_target_ids,
        } = &build_termination_transaction.external_pass_state
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if *expected_request_id != termination_request_id {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        let expected_target_ids = expected_target_ids.clone();
        for termination_target_outcome in termination_outcome_summary.target_outcomes() {
            let termination_target_id = termination_target_outcome.semantic_target_id();
            if !expected_target_ids.contains(&termination_target_id) {
                continue;
            }
            let Some(build_session_id) = build_termination_transaction
                .pending_targets
                .get(&termination_target_id)
                .cloned()
            else {
                continue;
            };
            record_external_target_result(
                build_termination_transaction,
                build_session_id,
                termination_target_id,
                termination_target_outcome.role(),
                termination_target_outcome.result().clone(),
            );
            if termination_target_is_gone(termination_target_outcome.result()) {
                build_termination_transaction
                    .pending_targets
                    .remove(&termination_target_id);
                build_termination_transaction
                    .admitted_descendants
                    .remove(&termination_target_id);
            }
        }
        build_termination_transaction.external_pass_state =
            if build_termination_transaction.has_pending_external_targets() {
                ExternalTerminationPassState::AwaitingObservation {
                    not_before: Instant::now() + TERMINATION_DESCENDANT_REFRESH_INTERVAL,
                }
            } else {
                ExternalTerminationPassState::Settled
            };
        self.complete_terminal_transaction(lifecycle_registry)
    }

    pub(in crate::build_monitor) fn reconcile_owned_outcome(
        &mut self,
        owned_run_termination_outcome: OwnedRunTerminationOutcome,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let owned_run_id = match owned_run_termination_outcome {
            OwnedRunTerminationOutcome::Honored { owned_run_id, .. }
            | OwnedRunTerminationOutcome::Refused { owned_run_id } => owned_run_id,
        };
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        let Some(owned_pending_termination_target) = build_termination_transaction
            .owned_targets
            .get(&owned_run_id)
            .copied()
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if owned_pending_termination_target.progress
            != OwnedBuildTerminationProgress::SignalOutcomePending
        {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        let owned_build_termination_result = match owned_run_termination_outcome {
            OwnedRunTerminationOutcome::Honored {
                signal: OwnedProcessGroupSignalOutcome::Sent,
                ..
            } => {
                if let Some(pending_target) = build_termination_transaction
                    .owned_targets
                    .get_mut(&owned_run_id)
                {
                    pending_target.progress = OwnedBuildTerminationProgress::ReapingAfterSignal;
                }
                return BuildTerminationCompletionTransition::NoCompletion;
            },
            OwnedRunTerminationOutcome::Honored {
                signal: OwnedProcessGroupSignalOutcome::ProcessAlreadyReaped,
                ..
            } => OwnedBuildTerminationResult::AlreadyReaped { owned_run_id },
            OwnedRunTerminationOutcome::Honored {
                signal: OwnedProcessGroupSignalOutcome::IdentityNoLongerCurrent,
                ..
            } => OwnedBuildTerminationResult::IdentityNoLongerCurrent { owned_run_id },
            OwnedRunTerminationOutcome::Honored {
                signal: OwnedProcessGroupSignalOutcome::SignalFailed,
                ..
            } => OwnedBuildTerminationResult::SignalFailed { owned_run_id },
            OwnedRunTerminationOutcome::Refused { .. } => {
                OwnedBuildTerminationResult::ActorRefused { owned_run_id }
            },
        };
        complete_owned_target(
            build_termination_transaction,
            owned_run_id,
            owned_build_termination_result,
        );
        self.complete_terminal_transaction(lifecycle_registry)
    }

    pub(in crate::build_monitor) fn reconcile_owned_finished(
        &mut self,
        owned_run_id: OwnedRunId,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        let Some(owned_pending_termination_target) = build_termination_transaction
            .owned_targets
            .get(&owned_run_id)
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if owned_pending_termination_target.progress
            != OwnedBuildTerminationProgress::ReapingAfterSignal
        {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        complete_owned_target(
            build_termination_transaction,
            owned_run_id,
            OwnedBuildTerminationResult::ReapedAfterSignal { owned_run_id },
        );
        self.complete_terminal_transaction(lifecycle_registry)
    }

    pub(in crate::build_monitor) fn expire(
        &mut self,
        now: Instant,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            &mut self.active_transaction
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        if !build_termination_transaction.deadline.has_expired(now) {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        let expired_targets: Vec<_> = build_termination_transaction
            .pending_targets
            .iter()
            .map(|(termination_target_id, build_session_id)| {
                (*termination_target_id, build_session_id.clone())
            })
            .collect();
        for (termination_target_id, build_session_id) in expired_targets {
            if let Some((owned_run_id, owned_pending_termination_target)) =
                build_termination_transaction
                    .owned_targets
                    .iter()
                    .find(|(_, target)| target.termination_target_id == termination_target_id)
            {
                let waiting_for = match owned_pending_termination_target.progress {
                    OwnedBuildTerminationProgress::ReadyToSubmit => {
                        OwnedBuildTerminationDeadline::AwaitingSubmission
                    },
                    OwnedBuildTerminationProgress::SignalOutcomePending => {
                        OwnedBuildTerminationDeadline::AwaitingSignalOutcome
                    },
                    OwnedBuildTerminationProgress::ReapingAfterSignal => {
                        OwnedBuildTerminationDeadline::ReapUnconfirmedAfterSignal
                    },
                };
                build_termination_transaction
                    .terminal_results
                    .entry(build_session_id)
                    .or_default()
                    .push(BuildTerminationTargetResult::Owned(
                        OwnedBuildTerminationResult::DeadlineExpired {
                            owned_run_id: *owned_run_id,
                            waiting_for,
                        },
                    ));
                continue;
            }
            let ExternalPendingTarget::Resolved { pid, role } =
                external_target_pid_and_role(build_termination_transaction, termination_target_id)
            else {
                build_termination_transaction
                    .terminal_results
                    .entry(build_session_id)
                    .or_default()
                    .push(BuildTerminationTargetResult::TransactionResultUnavailable);
                continue;
            };
            record_external_target_result(
                build_termination_transaction,
                build_session_id,
                termination_target_id,
                role,
                TerminationTargetResult::Refused(TerminationError::DeadlineExpired { pid }),
            );
        }
        build_termination_transaction.pending_targets.clear();
        self.complete_terminal_transaction(lifecycle_registry)
    }

    fn complete_terminal_transaction(
        &mut self,
        lifecycle_registry: &mut BuildTerminationLifecycleRegistry,
    ) -> BuildTerminationCompletionTransition {
        if !matches!(
            &self.active_transaction,
            ActiveBuildTerminationTransaction::Active(build_termination_transaction)
                if build_termination_transaction.is_terminal()
        ) {
            return BuildTerminationCompletionTransition::NoCompletion;
        }
        let ActiveBuildTerminationTransaction::Active(build_termination_transaction) =
            std::mem::replace(
                &mut self.active_transaction,
                ActiveBuildTerminationTransaction::Idle,
            )
        else {
            return BuildTerminationCompletionTransition::NoCompletion;
        };
        BuildTerminationCompletionTransition::Completed(
            lifecycle_registry.complete_transaction_with_target_set(
                build_termination_transaction.transaction_id,
                build_termination_transaction.target_set,
                build_termination_transaction.additional_build_exclusion,
                build_termination_transaction.terminal_results,
            ),
        )
    }
}

fn complete_owned_target(
    build_termination_transaction: &mut BuildTerminationTransaction,
    owned_run_id: OwnedRunId,
    owned_build_termination_result: OwnedBuildTerminationResult,
) {
    let Some(owned_pending_termination_target) = build_termination_transaction
        .owned_targets
        .remove(&owned_run_id)
    else {
        return;
    };
    let Some(build_session_id) = build_termination_transaction
        .pending_targets
        .remove(&owned_pending_termination_target.termination_target_id)
    else {
        return;
    };
    build_termination_transaction
        .terminal_results
        .entry(build_session_id)
        .or_default()
        .push(BuildTerminationTargetResult::Owned(
            owned_build_termination_result,
        ));
}

fn record_external_target_result(
    build_termination_transaction: &mut BuildTerminationTransaction,
    build_session_id: BuildSessionId,
    termination_target_id: TerminationTargetId,
    termination_execution_target_role: TerminationExecutionTargetRole,
    termination_target_result: TerminationTargetResult,
) {
    build_termination_transaction
        .terminal_results
        .entry(build_session_id)
        .or_default()
        .push(BuildTerminationTargetResult::External(
            ExternalBuildTerminationResult::new(
                termination_target_id,
                termination_execution_target_role,
                termination_target_result,
            ),
        ));
}

fn external_target_pid_and_role(
    build_termination_transaction: &BuildTerminationTransaction,
    termination_target_id: TerminationTargetId,
) -> ExternalPendingTarget {
    if let Some(root) = build_termination_transaction
        .external_roots
        .get(&termination_target_id)
    {
        return ExternalPendingTarget::Resolved {
            pid:  root.root_identity.pid(),
            role: TerminationExecutionTargetRole::FrozenRoot,
        };
    }
    if let Some(descendant) = build_termination_transaction
        .admitted_descendants
        .get(&termination_target_id)
    {
        return ExternalPendingTarget::Resolved {
            pid:  descendant.process_identity.pid(),
            role: TerminationExecutionTargetRole::AdmittedDescendant {
                depth_from_root: descendant.depth_from_root,
            },
        };
    }
    ExternalPendingTarget::CorrelationUnavailable
}

fn reconcile_observed_roots(
    build_termination_transaction: &mut BuildTerminationTransaction,
    root_presence: &BTreeMap<TerminationTargetId, BuildTerminationRootPresence>,
) {
    for (root_target_id, presence) in root_presence {
        let Some(root) = build_termination_transaction
            .external_roots
            .get_mut(root_target_id)
        else {
            continue;
        };
        match presence {
            BuildTerminationRootPresence::LiveInFrozenScope => {},
            BuildTerminationRootPresence::Gone => {
                if let Some(build_session_id) = build_termination_transaction
                    .pending_targets
                    .remove(root_target_id)
                {
                    let result =
                        observed_gone_result(root.root_identity.pid(), &root.capability_state);
                    build_termination_transaction
                        .terminal_results
                        .entry(build_session_id)
                        .or_default()
                        .push(BuildTerminationTargetResult::External(
                            ExternalBuildTerminationResult::new(
                                *root_target_id,
                                TerminationExecutionTargetRole::FrozenRoot,
                                result,
                            ),
                        ));
                }
            },
            BuildTerminationRootPresence::ScopeDiverged => {
                if matches!(
                    root.capability_state,
                    ExternalTerminationCapabilityState::Ready(_)
                ) {
                    root.capability_state = ExternalTerminationCapabilityState::Submitted;
                    if let Some(build_session_id) = build_termination_transaction
                        .pending_targets
                        .get(root_target_id)
                        .cloned()
                    {
                        build_termination_transaction
                            .terminal_results
                            .entry(build_session_id)
                            .or_default()
                            .push(BuildTerminationTargetResult::External(
                                ExternalBuildTerminationResult::new(
                                    *root_target_id,
                                    TerminationExecutionTargetRole::FrozenRoot,
                                    TerminationTargetResult::Refused(
                                        TerminationError::FrozenScopeDiverged {
                                            pid: root.root_identity.pid(),
                                        },
                                    ),
                                ),
                            ));
                    }
                }
            },
        }
    }
}

fn reconcile_observed_descendants(
    build_termination_transaction: &mut BuildTerminationTransaction,
    descendant_presence: &BTreeMap<TerminationTargetId, AdmittedTerminationDescendantPresence>,
) {
    let gone_descendant_ids: Vec<_> = descendant_presence
        .iter()
        .filter_map(|(termination_target_id, presence)| {
            (*presence == AdmittedTerminationDescendantPresence::Gone)
                .then_some(*termination_target_id)
        })
        .collect();
    for termination_target_id in gone_descendant_ids {
        let Some(descendant) = build_termination_transaction
            .admitted_descendants
            .remove(&termination_target_id)
        else {
            continue;
        };
        let Some(build_session_id) = build_termination_transaction
            .pending_targets
            .remove(&termination_target_id)
        else {
            continue;
        };
        build_termination_transaction
            .terminal_results
            .entry(build_session_id)
            .or_default()
            .push(BuildTerminationTargetResult::External(
                ExternalBuildTerminationResult::new(
                    termination_target_id,
                    TerminationExecutionTargetRole::AdmittedDescendant {
                        depth_from_root: descendant.depth_from_root,
                    },
                    observed_gone_result(
                        descendant.process_identity.pid(),
                        &descendant.capability_state,
                    ),
                ),
            ));
    }
}

fn admit_new_descendants(
    next_target_id: &mut NonZeroU64,
    build_termination_transaction: &mut BuildTerminationTransaction,
    admitted_descendants: Vec<NewActionableTerminationDescendant>,
) {
    for admitted_descendant in admitted_descendants {
        if build_termination_transaction
            .admitted_descendants
            .values()
            .any(|existing| existing.process_identity == admitted_descendant.process_identity)
        {
            continue;
        }
        let Some(build_session_id) = build_termination_transaction
            .pending_targets
            .get(&admitted_descendant.root_target_id)
            .cloned()
        else {
            continue;
        };
        let Some(following_target_id) = next_target_id.checked_add(1) else {
            continue;
        };
        let termination_target_id = TerminationTargetId::from_non_zero(*next_target_id);
        *next_target_id = following_target_id;
        build_termination_transaction
            .pending_targets
            .insert(termination_target_id, build_session_id);
        build_termination_transaction.admitted_descendants.insert(
            termination_target_id,
            AdmittedExternalTerminationDescendant {
                root_target_id:   admitted_descendant.root_target_id,
                process_identity: admitted_descendant.process_identity,
                parent_identity:  admitted_descendant.parent_identity,
                depth_from_root:  admitted_descendant.depth_from_root,
                capability_state: ExternalTerminationCapabilityState::Ready(
                    admitted_descendant.capability,
                ),
            },
        );
    }
}

fn next_external_execution_targets(
    build_termination_transaction: &mut BuildTerminationTransaction,
    root_presence: &BTreeMap<TerminationTargetId, BuildTerminationRootPresence>,
) -> (
    Vec<TerminationExecutionTarget>,
    BTreeSet<TerminationTargetId>,
) {
    let live_descendant_ids: BTreeSet<_> = build_termination_transaction
        .admitted_descendants
        .keys()
        .copied()
        .filter(|termination_target_id| {
            build_termination_transaction
                .pending_targets
                .contains_key(termination_target_id)
        })
        .collect();
    let leaf_ids: Vec<_> = live_descendant_ids
        .iter()
        .copied()
        .filter(|candidate_id| {
            let candidate = &build_termination_transaction.admitted_descendants[candidate_id];
            matches!(
                candidate.capability_state,
                ExternalTerminationCapabilityState::Ready(_)
            ) && !live_descendant_ids.iter().any(|other_id| {
                other_id != candidate_id
                    && build_termination_transaction.admitted_descendants[other_id].parent_identity
                        == candidate.process_identity
            })
        })
        .collect();
    let mut execution_targets = Vec::new();
    let mut submitted_target_ids = BTreeSet::new();
    for termination_target_id in leaf_ids {
        let Some(descendant) = build_termination_transaction
            .admitted_descendants
            .get_mut(&termination_target_id)
        else {
            continue;
        };
        let ExternalTerminationCapabilityState::Ready(capability) = std::mem::replace(
            &mut descendant.capability_state,
            ExternalTerminationCapabilityState::Submitted,
        ) else {
            continue;
        };
        execution_targets.push(TerminationExecutionTarget::admitted_descendant(
            termination_target_id,
            descendant.depth_from_root,
            capability,
        ));
        submitted_target_ids.insert(termination_target_id);
    }

    for (root_target_id, root) in &mut build_termination_transaction.external_roots {
        let has_live_descendant = live_descendant_ids.iter().any(|termination_target_id| {
            build_termination_transaction.admitted_descendants[termination_target_id].root_target_id
                == *root_target_id
        });
        if has_live_descendant
            || root_presence.get(root_target_id)
                != Some(&BuildTerminationRootPresence::LiveInFrozenScope)
        {
            continue;
        }
        let ExternalTerminationCapabilityState::Ready(capability) = std::mem::replace(
            &mut root.capability_state,
            ExternalTerminationCapabilityState::Submitted,
        ) else {
            continue;
        };
        execution_targets.push(TerminationExecutionTarget::new(*root_target_id, capability));
        submitted_target_ids.insert(*root_target_id);
    }
    (execution_targets, submitted_target_ids)
}

fn dispatch_external_pass(
    build_termination_transaction: &mut BuildTerminationTransaction,
    process_terminator: &mut ProcessTerminator,
    execution_targets: Vec<TerminationExecutionTarget>,
    submitted_target_ids: BTreeSet<TerminationTargetId>,
) -> ExternalPassDispatch {
    if execution_targets.is_empty() {
        build_termination_transaction.external_pass_state =
            if build_termination_transaction.has_pending_external_targets() {
                ExternalTerminationPassState::AwaitingObservation {
                    not_before: Instant::now() + TERMINATION_DESCENDANT_REFRESH_INTERVAL,
                }
            } else {
                ExternalTerminationPassState::Settled
            };
        return ExternalPassDispatch::ReconcileTerminalState;
    }
    let termination_execution_plan = match process_terminator.plan_bounded_termination(
        execution_targets,
        build_termination_transaction.deadline.expires_at(),
    ) {
        TerminationPlanCreation::Planned(termination_execution_plan) => termination_execution_plan,
        TerminationPlanCreation::RequestIdsExhausted => {
            mark_external_worker_failure(
                build_termination_transaction,
                &submitted_target_ids,
                ExternalTerminationPassSubmissionFailure::RequestIdentitiesExhausted,
            );
            return ExternalPassDispatch::ReconcileTerminalState;
        },
    };
    let termination_request_id = termination_execution_plan.termination_request_id();
    match process_terminator.request_termination(termination_execution_plan) {
        TerminationDispatchOutcome::Dispatched(dispatched_request_id)
            if dispatched_request_id == termination_request_id =>
        {
            build_termination_transaction.external_pass_state =
                ExternalTerminationPassState::AwaitingWorker {
                    termination_request_id,
                    target_ids: submitted_target_ids,
                };
            ExternalPassDispatch::AwaitingWorker
        },
        TerminationDispatchOutcome::Dispatched(_) => {
            mark_external_worker_failure(
                build_termination_transaction,
                &submitted_target_ids,
                ExternalTerminationPassSubmissionFailure::RequestCorrelationMismatch,
            );
            ExternalPassDispatch::ReconcileTerminalState
        },
        TerminationDispatchOutcome::WorkerUnavailable => {
            mark_external_worker_failure(
                build_termination_transaction,
                &submitted_target_ids,
                ExternalTerminationPassSubmissionFailure::WorkerUnavailable,
            );
            ExternalPassDispatch::ReconcileTerminalState
        },
    }
}

const fn termination_target_is_gone(termination_target_result: &TerminationTargetResult) -> bool {
    match termination_target_result {
        TerminationTargetResult::AlreadyGone { .. } => true,
        #[cfg(any(target_os = "linux", test))]
        TerminationTargetResult::GoneAfterSignaling { .. } => true,
        TerminationTargetResult::Refused(_) => false,
        #[cfg(any(target_os = "linux", test))]
        TerminationTargetResult::Survived { .. }
        | TerminationTargetResult::SignaledButUnconfirmed { .. } => false,
    }
}

const fn observed_gone_result(
    pid: u32,
    external_termination_capability_state: &ExternalTerminationCapabilityState,
) -> TerminationTargetResult {
    if matches!(
        external_termination_capability_state,
        ExternalTerminationCapabilityState::Submitted
    ) {
        #[cfg(any(target_os = "linux", test))]
        {
            return TerminationTargetResult::GoneAfterSignaling { pid };
        }
    }
    TerminationTargetResult::AlreadyGone { pid }
}

fn mark_external_worker_failure(
    build_termination_transaction: &mut BuildTerminationTransaction,
    submitted_target_ids: &BTreeSet<TerminationTargetId>,
    failure: ExternalTerminationPassSubmissionFailure,
) {
    for termination_target_id in submitted_target_ids {
        let Some(build_session_id) = build_termination_transaction
            .pending_targets
            .get(termination_target_id)
            .cloned()
        else {
            continue;
        };
        let ExternalPendingTarget::Resolved { pid, role } =
            external_target_pid_and_role(build_termination_transaction, *termination_target_id)
        else {
            build_termination_transaction
                .terminal_results
                .entry(build_session_id)
                .or_default()
                .push(BuildTerminationTargetResult::TransactionResultUnavailable);
            continue;
        };
        record_external_target_result(
            build_termination_transaction,
            build_session_id,
            *termination_target_id,
            role,
            TerminationTargetResult::Refused(match failure {
                ExternalTerminationPassSubmissionFailure::RequestIdentitiesExhausted => {
                    TerminationError::RequestIdentitiesExhausted { pid }
                },
                ExternalTerminationPassSubmissionFailure::WorkerUnavailable => {
                    TerminationError::TerminationWorkerUnavailable { pid }
                },
                ExternalTerminationPassSubmissionFailure::RequestCorrelationMismatch => {
                    TerminationError::TerminationRequestCorrelationMismatch { pid }
                },
            }),
        );
    }
    build_termination_transaction.external_pass_state =
        ExternalTerminationPassState::AwaitingObservation {
            not_before: Instant::now() + TERMINATION_DESCENDANT_REFRESH_INTERVAL,
        };
}

#[cfg(test)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::Duration;

    use super::super::authority::ExternalBuildTerminationAuthority;
    use super::super::authority::OwnedBuildTerminationAuthority;
    use super::super::lifecycle::BuildTerminationAggregateCompletion;
    use super::super::lifecycle::BuildTerminationLifecycle;
    use super::super::lifecycle::BuildTerminationSessionCompletion;
    use super::super::observation::BuildTerminationObservationExecution;
    use super::super::observation::observe_build_termination_demand;
    use super::*;
    use crate::build_monitor::BuildScopeActionability;
    use crate::build_monitor::ClassifiedRoot;
    use crate::build_monitor::FixtureRootOwnership;
    use crate::build_monitor::LiveTargetDirectoryRevision;
    use crate::build_monitor::MonitorSessionOwnership;
    use crate::build_monitor::MonitorSnapshot;
    use crate::build_monitor::classified_monitor_snapshot_with_ownership;
    use crate::build_monitor::session::ScopeAttribution;
    use crate::build_monitor::snapshot::MonitorData;
    use crate::process_observation::ProcessObserver;
    use crate::process_observation::ProcessRefreshConsumerDemand;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;
    use crate::process_observation::snapshot_builder::ObservedProcess;
    use crate::process_observation::snapshot_builder::snapshot_of;
    use crate::process_termination::TerminationResultPoll;
    use crate::project::AbsolutePath;
    use crate::project::AcceptedCargoMetadataRevision;
    use crate::project::CanonicalCheckoutRoot;
    use crate::project::ProjectListRevision;

    fn build_session_id_for(pid: u32) -> BuildSessionId {
        BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(pid, 12),
            "/usr/bin/cargo",
        ))
    }

    fn resolved_scope(root: &str) -> SessionScope {
        SessionScope::Resolved {
            method: ScopeAttribution::WorkingDirectoryManifest,
            root:   CanonicalCheckoutRoot::for_test(AbsolutePath::from(PathBuf::from(root))),
        }
    }

    fn completed_observation(
        execution: BuildTerminationObservationExecution,
    ) -> CompletedBuildTerminationObservation {
        match execution {
            BuildTerminationObservationExecution::Completed(completed) => completed,
            BuildTerminationObservationExecution::NotRequested => {
                panic!("test requested a transaction observation")
            },
        }
    }

    fn await_external_outcome(process_terminator: &ProcessTerminator) -> TerminationOutcomeSummary {
        let deadline = Instant::now() + BUILD_TERMINATION_TIMEOUT;
        loop {
            match process_terminator.poll_outcome() {
                TerminationResultPoll::Completed(summary) => return summary,
                TerminationResultPoll::NoCompletedRequest if Instant::now() < deadline => {
                    std::thread::yield_now();
                },
                TerminationResultPoll::NoCompletedRequest => {
                    panic!("termination worker should finish before the test deadline")
                },
                TerminationResultPoll::WorkerUnavailable => {
                    panic!("termination worker should remain available")
                },
            }
        }
    }

    fn owned_output_build_set_authorization(
        build_session_id: BuildSessionId,
        owned_run_id: OwnedRunId,
        root: &str,
        pid: u32,
    ) -> OutputBuildSetTerminationAuthorization {
        OutputBuildSetTerminationAuthorization {
            scope_key: BuildScopeKey::for_test(AbsolutePath::from(PathBuf::from(root))),
            targets:   vec![FrozenBuildTerminationTarget {
                session_id:       build_session_id.clone(),
                authority:        BuildTerminationAuthority::Owned(
                    OwnedBuildTerminationAuthority {
                        owned_run_id,
                        owned_run_termination_token: OwnedRunTerminationToken::for_test(
                            owned_run_id,
                        ),
                    },
                ),
                display_identity: BuildTerminationDisplayIdentity::for_test(
                    build_session_id,
                    resolved_scope(root),
                    pid,
                    MonitorSessionOwnership::Owned(owned_run_id),
                ),
            }],
        }
    }

    fn owned_output_build_set_target(
        build_session_id: BuildSessionId,
        owned_run_id: OwnedRunId,
        root: &str,
        pid: u32,
    ) -> FrozenBuildTerminationTarget {
        FrozenBuildTerminationTarget {
            session_id:       build_session_id.clone(),
            authority:        BuildTerminationAuthority::Owned(OwnedBuildTerminationAuthority {
                owned_run_id,
                owned_run_termination_token: OwnedRunTerminationToken::for_test(owned_run_id),
            }),
            display_identity: BuildTerminationDisplayIdentity::for_test(
                build_session_id,
                resolved_scope(root),
                pid,
                MonitorSessionOwnership::Owned(owned_run_id),
            ),
        }
    }

    fn started_owned_transaction(
        build_termination_deadline: BuildTerminationDeadline,
    ) -> (
        BuildTerminationState,
        BuildTerminationLifecycleRegistry,
        BuildTerminationTransactionId,
        BuildSessionId,
        OwnedRunId,
    ) {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let build_session_id = build_session_id_for(4_200_101);
        let authorization = owned_output_build_set_authorization(
            build_session_id.clone(),
            owned_run_id,
            "/workspace",
            4_200_101,
        );
        let mut termination_state = BuildTerminationState::default();
        let mut lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        let mut process_terminator = ProcessTerminator::start();
        let BuildTerminationSubmission::Submitted(transaction_id) = termination_state
            .submit_output_build_set_for_test(
                authorization,
                &mut process_terminator,
                build_termination_deadline,
                &mut lifecycle_registry,
            )
        else {
            panic!("owned transaction should start");
        };
        termination_state.submit_owned_targets(
            transaction_id,
            |_| OwnedRunTerminationSubmission::Submitted(owned_run_id),
            &mut lifecycle_registry,
        );
        (
            termination_state,
            lifecycle_registry,
            transaction_id,
            build_session_id,
            owned_run_id,
        )
    }

    fn monitor_with_owned_authority(
        root_pid: u32,
        owned_run_id: OwnedRunId,
    ) -> (crate::build_monitor::BuildMonitor, BuildSessionId) {
        let monitor_snapshot = match classified_monitor_snapshot_with_ownership(
            &[ClassifiedRoot {
                root_pid,
                compiler_pids: &[],
            }],
            &FixtureRootOwnership::OwnedRoot {
                root_pid,
                owned_run_id,
            },
        ) {
            Ok(monitor_snapshot) => monitor_snapshot,
            Err(error) => panic!("classification fixture should succeed: {error}"),
        };
        let MonitorSnapshot::Fresh(monitor_data) = &monitor_snapshot else {
            panic!("classification fixture should produce a fresh snapshot");
        };
        let build_session_id = monitor_data.session_rows()[0].build_session_id().clone();
        let mut build_monitor = crate::build_monitor::BuildMonitor::default();
        build_monitor.show_for_test(monitor_snapshot);
        build_monitor
            .termination_state
            .replace_current_authorities(BTreeMap::from([(
                build_session_id.clone(),
                BuildTerminationAuthority::Owned(OwnedBuildTerminationAuthority {
                    owned_run_id,
                    owned_run_termination_token: OwnedRunTerminationToken::for_test(owned_run_id),
                }),
            )]));
        (build_monitor, build_session_id)
    }

    fn revision_changed_snapshot(monitor_snapshot: &MonitorSnapshot) -> MonitorSnapshot {
        let MonitorSnapshot::Fresh(monitor_data) = monitor_snapshot else {
            panic!("test snapshot should be fresh");
        };
        let mut project_list_revision = ProjectListRevision::default();
        project_list_revision.advance();
        let build_scope_key = BuildScopeKey::from_covered_scope_roots(
            monitor_data.build_scope_key().covered_scope_roots().clone(),
            AcceptedCargoMetadataRevision::default(),
            project_list_revision,
            LiveTargetDirectoryRevision::default(),
        );
        MonitorSnapshot::Fresh(MonitorData::new(
            build_scope_key,
            monitor_data.session_rows().to_vec(),
            monitor_data.unattributed_activities().to_vec(),
            monitor_data.observed_at(),
        ))
    }

    fn session_changed_snapshot(monitor_snapshot: &MonitorSnapshot) -> MonitorSnapshot {
        let MonitorSnapshot::Fresh(monitor_data) = monitor_snapshot else {
            panic!("test snapshot should be fresh");
        };
        let replacement_snapshot = match classified_monitor_snapshot_with_ownership(
            &[ClassifiedRoot {
                root_pid:      4_200_202,
                compiler_pids: &[],
            }],
            &FixtureRootOwnership::AllExternal,
        ) {
            Ok(monitor_snapshot) => monitor_snapshot,
            Err(error) => panic!("replacement fixture should succeed: {error}"),
        };
        let MonitorSnapshot::Fresh(replacement_data) = replacement_snapshot else {
            panic!("replacement fixture should produce a fresh snapshot");
        };
        MonitorSnapshot::Fresh(MonitorData::new(
            monitor_data.build_scope_key().clone(),
            replacement_data.session_rows().to_vec(),
            Vec::new(),
            monitor_data.observed_at(),
        ))
    }

    #[test]
    fn selected_availability_reads_authority_without_consuming_it() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut build_monitor, build_session_id) =
            monitor_with_owned_authority(4_200_111, owned_run_id);

        assert_eq!(
            build_monitor.selected_termination_availability(&build_session_id),
            SelectedBuildTerminationAvailability::Available
        );
        assert!(matches!(
            build_monitor.selected_termination_authorization(&build_session_id),
            BuildTerminationAuthorizationConstruction::Authorized(_)
        ));
    }

    #[test]
    fn output_build_set_availability_reads_authority_without_consuming_it() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut build_monitor, _) = monitor_with_owned_authority(4_200_115, owned_run_id);

        assert_eq!(
            build_monitor.output_build_set_termination_availability(),
            OutputBuildSetTerminationAvailability::Available
        );
        assert!(matches!(
            build_monitor.output_build_set_termination_authorization(),
            BuildTerminationAuthorizationConstruction::Authorized(_)
        ));
    }

    #[test]
    fn output_build_set_availability_refuses_an_observed_only_root() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut build_monitor, _) = monitor_with_owned_authority(4_200_116, owned_run_id);
        build_monitor.termination_state.clear_current_authorities();

        assert_eq!(
            build_monitor.output_build_set_termination_availability(),
            OutputBuildSetTerminationAvailability::BuildSetNotFullyActionable
        );
    }

    #[test]
    fn output_build_set_submits_each_confirmed_owned_root_once() {
        let first_owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let Some(second_owned_run_identity) = NonZeroU64::new(2) else {
            panic!("two is a nonzero owned-run identity");
        };
        let second_owned_run_id = OwnedRunId::for_test(second_owned_run_identity);
        let first_session_id = build_session_id_for(4_200_117);
        let second_session_id = build_session_id_for(4_200_118);
        let authorization = OutputBuildSetTerminationAuthorization {
            scope_key: BuildScopeKey::for_test(AbsolutePath::from(PathBuf::from(
                "/selected-checkout",
            ))),
            targets:   vec![
                owned_output_build_set_target(
                    first_session_id,
                    first_owned_run_id,
                    "/selected-checkout",
                    4_200_117,
                ),
                owned_output_build_set_target(
                    second_session_id,
                    second_owned_run_id,
                    "/shown-outside-selected-checkout",
                    4_200_118,
                ),
            ],
        };
        let mut termination_state = BuildTerminationState::default();
        let mut lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        let mut process_terminator = ProcessTerminator::start();
        let BuildTerminationSubmission::Submitted(transaction_id) = termination_state
            .submit_output_build_set_for_test(
                authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
                &mut lifecycle_registry,
            )
        else {
            panic!("every confirmed owned root should form one transaction");
        };

        let submitted_run_ids = [first_owned_run_id, second_owned_run_id];
        let mut submitted_count = 0;
        assert_eq!(
            termination_state.submit_owned_targets(
                transaction_id,
                |_| {
                    let submitted_run_id = submitted_run_ids[submitted_count];
                    submitted_count += 1;
                    OwnedRunTerminationSubmission::Submitted(submitted_run_id)
                },
                &mut lifecycle_registry,
            ),
            BuildTerminationCompletionTransition::NoCompletion
        );
        assert_eq!(
            submitted_count, 2,
            "one output-build-set acceptance fans every frozen owned token out immediately"
        );
        assert!(termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            termination_state.submit_owned_targets(
                transaction_id,
                |_| OwnedRunTerminationSubmission::ActorUnavailable,
                &mut lifecycle_registry,
            ),
            BuildTerminationCompletionTransition::NoCompletion,
            "the transaction never reconstructs or resubmits accepted authority"
        );
    }

    #[test]
    fn selected_and_output_build_set_submission_share_the_five_second_semantic_deadline() {
        assert_eq!(BUILD_TERMINATION_TIMEOUT, Duration::from_secs(5));
        let submitted_at = Instant::now();
        let build_termination_deadline =
            BuildTerminationDeadline::from_submission_time(submitted_at);
        assert_eq!(
            build_termination_deadline
                .expires_at()
                .duration_since(submitted_at),
            BUILD_TERMINATION_TIMEOUT
        );

        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let mut process_terminator = ProcessTerminator::start();
        let (mut selected_monitor, selected_session_id) =
            monitor_with_owned_authority(4_200_112, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(selected_authorization) =
            selected_monitor.selected_termination_authorization(&selected_session_id)
        else {
            panic!("selected fixture should authorize");
        };
        assert!(matches!(
            selected_monitor.submit_selected_termination(
                selected_authorization,
                &mut process_terminator,
                build_termination_deadline,
            ),
            BuildTerminationSubmission::Submitted(_)
        ));
        let ActiveBuildTerminationTransaction::Active(selected_transaction) =
            &selected_monitor.termination_state.active_transaction
        else {
            panic!("selected submission should activate a transaction");
        };
        assert_eq!(selected_transaction.deadline, build_termination_deadline);

        let output_build_set_submitted_at = Instant::now();
        let output_build_set_deadline =
            BuildTerminationDeadline::from_submission_time(output_build_set_submitted_at);
        let (mut output_build_set_monitor, _) =
            monitor_with_owned_authority(4_200_113, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(output_build_set_authorization) =
            output_build_set_monitor.output_build_set_termination_authorization()
        else {
            panic!("output-build-set fixture should authorize");
        };
        assert!(matches!(
            output_build_set_monitor.submit_output_build_set_termination(
                output_build_set_authorization,
                &mut process_terminator,
                output_build_set_deadline,
            ),
            BuildTerminationSubmission::Submitted(_)
        ));
        let ActiveBuildTerminationTransaction::Active(output_build_set_transaction) =
            &output_build_set_monitor
                .termination_state
                .active_transaction
        else {
            panic!("output-build-set submission should activate a transaction");
        };
        assert_eq!(
            output_build_set_transaction.deadline,
            output_build_set_deadline
        );
        assert_eq!(
            output_build_set_deadline
                .expires_at()
                .duration_since(output_build_set_submitted_at),
            BUILD_TERMINATION_TIMEOUT
        );
    }

    #[test]
    fn synchronous_owned_submission_refusal_emits_one_completion_transition() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let build_session_id = build_session_id_for(4_200_114);
        let mut termination_state = BuildTerminationState::default();
        let mut lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        let mut process_terminator = ProcessTerminator::start();
        let BuildTerminationSubmission::Submitted(transaction_id) = termination_state
            .submit_output_build_set_for_test(
                owned_output_build_set_authorization(
                    build_session_id.clone(),
                    owned_run_id,
                    "/workspace",
                    4_200_114,
                ),
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
                &mut lifecycle_registry,
            )
        else {
            panic!("owned fixture should start a transaction");
        };

        let completion_transition = termination_state.submit_owned_targets(
            transaction_id,
            |_| OwnedRunTerminationSubmission::ActorUnavailable,
            &mut lifecycle_registry,
        );
        assert!(matches!(
            completion_transition,
            BuildTerminationCompletionTransition::Completed(
                BuildTerminationTransactionCompletion { .. }
            )
        ));
        assert_eq!(
            termination_state.submit_owned_targets(
                transaction_id,
                |_| OwnedRunTerminationSubmission::ActorUnavailable,
                &mut lifecycle_registry,
            ),
            BuildTerminationCompletionTransition::NoCompletion
        );
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::RetryUnavailable
        );
    }

    #[test]
    fn selected_delayed_confirmation_requires_exact_scope_and_session() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut current_monitor, current_session_id) =
            monitor_with_owned_authority(4_200_201, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(current_authorization) =
            current_monitor.selected_termination_authorization(&current_session_id)
        else {
            panic!("current selected session should authorize");
        };
        let mut process_terminator = ProcessTerminator::start();
        assert!(matches!(
            current_monitor.submit_selected_termination(
                current_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Submitted(_)
        ));

        let (mut revision_monitor, revision_session_id) =
            monitor_with_owned_authority(4_200_201, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(revision_authorization) =
            revision_monitor.selected_termination_authorization(&revision_session_id)
        else {
            panic!("selected session should authorize before revision churn");
        };
        let changed_snapshot = revision_changed_snapshot(revision_monitor.monitor_snapshot());
        let MonitorSnapshot::Fresh(changed_data) = changed_snapshot else {
            panic!("revision helper should keep a fresh snapshot");
        };
        revision_monitor.replace_scope(&BuildScopeActionability::Actionable(
            changed_data.build_scope_key().clone(),
        ));
        assert_eq!(
            revision_monitor.submit_selected_termination(
                revision_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Refused(
                BuildTerminationSubmissionRefusal::SelectedScopeChanged
            )
        );
        assert!(
            !revision_monitor
                .termination_state
                .lifecycle_transaction_is_active()
        );
        assert_eq!(
            revision_monitor
                .termination_lifecycle_registry()
                .lifecycle_for(&revision_session_id),
            BuildTerminationLifecycle::Observed
        );

        let (mut session_monitor, session_id) =
            monitor_with_owned_authority(4_200_201, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(session_authorization) =
            session_monitor.selected_termination_authorization(&session_id)
        else {
            panic!("selected session should authorize before replacement");
        };
        let changed_snapshot = session_changed_snapshot(session_monitor.monitor_snapshot());
        session_monitor.show_for_test(changed_snapshot);
        assert_eq!(
            session_monitor.submit_selected_termination(
                session_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Refused(
                BuildTerminationSubmissionRefusal::SelectedSessionChanged
            )
        );
        assert!(
            !session_monitor
                .termination_state
                .lifecycle_transaction_is_active()
        );
    }

    #[test]
    fn output_build_set_delayed_confirmation_accepts_equal_roots_and_refuses_incompatible_state() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut revision_monitor, _) = monitor_with_owned_authority(4_200_203, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(revision_authorization) =
            revision_monitor.output_build_set_termination_authorization()
        else {
            panic!("current output build set should authorize");
        };
        let changed_snapshot = revision_changed_snapshot(revision_monitor.monitor_snapshot());
        let MonitorSnapshot::Fresh(changed_data) = changed_snapshot else {
            panic!("revision helper should keep a fresh snapshot");
        };
        revision_monitor.replace_scope(&BuildScopeActionability::Actionable(
            changed_data.build_scope_key().clone(),
        ));
        let mut process_terminator = ProcessTerminator::start();
        assert!(matches!(
            revision_monitor.submit_output_build_set_termination(
                revision_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Submitted(_)
        ));

        let (mut incompatible_monitor, incompatible_session_id) =
            monitor_with_owned_authority(4_200_203, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(incompatible_authorization) =
            incompatible_monitor.output_build_set_termination_authorization()
        else {
            panic!("current output build set should authorize before root replacement");
        };
        let MonitorSnapshot::Fresh(monitor_data) = incompatible_monitor.monitor_snapshot() else {
            panic!("test monitor should be fresh");
        };
        incompatible_monitor.show_for_test(MonitorSnapshot::Fresh(MonitorData::new(
            BuildScopeKey::for_test(AbsolutePath::from(PathBuf::from("/incompatible"))),
            monitor_data.session_rows().to_vec(),
            Vec::new(),
            monitor_data.observed_at(),
        )));
        assert_eq!(
            incompatible_monitor.submit_output_build_set_termination(
                incompatible_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Refused(
                BuildTerminationSubmissionRefusal::CoveredScopeRootsChanged
            )
        );
        assert_eq!(
            incompatible_monitor
                .termination_lifecycle_registry()
                .lifecycle_for(&incompatible_session_id),
            BuildTerminationLifecycle::Observed
        );

        let (mut off_monitor, off_session_id) =
            monitor_with_owned_authority(4_200_203, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(off_authorization) =
            off_monitor.output_build_set_termination_authorization()
        else {
            panic!("current output build set should authorize before monitoring switches off");
        };
        off_monitor.switch_off();
        assert_eq!(
            off_monitor.submit_output_build_set_termination(
                off_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Refused(
                BuildTerminationSubmissionRefusal::SnapshotNotActionable
            )
        );
        assert_eq!(
            off_monitor
                .termination_lifecycle_registry()
                .lifecycle_for(&off_session_id),
            BuildTerminationLifecycle::Observed
        );
    }

    #[test]
    fn output_build_set_excludes_a_newly_shown_root_from_frozen_authority() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let (mut build_monitor, _) = monitor_with_owned_authority(4_200_206, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(authorization) =
            build_monitor.output_build_set_termination_authorization()
        else {
            panic!("the original shown root should authorize one output-build-set transaction");
        };
        let MonitorSnapshot::Fresh(current_data) = build_monitor.monitor_snapshot() else {
            panic!("the test monitor should remain fresh before a new root appears");
        };
        let added_snapshot = match classified_monitor_snapshot_with_ownership(
            &[ClassifiedRoot {
                root_pid:      4_200_207,
                compiler_pids: &[],
            }],
            &FixtureRootOwnership::AllExternal,
        ) {
            Ok(snapshot) => snapshot,
            Err(error) => panic!("the new shown root should classify: {error}"),
        };
        let MonitorSnapshot::Fresh(added_data) = &added_snapshot else {
            panic!("the added root fixture should produce fresh monitor data");
        };
        let mut shown_rows = current_data.session_rows().to_vec();
        shown_rows.extend_from_slice(added_data.session_rows());
        let expanded_snapshot = MonitorSnapshot::Fresh(MonitorData::new(
            current_data.build_scope_key().clone(),
            shown_rows,
            current_data.unattributed_activities().to_vec(),
            current_data.observed_at(),
        ));
        build_monitor.show_for_test(expanded_snapshot);

        let mut process_terminator = ProcessTerminator::start();
        let BuildTerminationSubmission::Submitted(transaction_id) = build_monitor
            .submit_output_build_set_termination(
                authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            )
        else {
            panic!("a new shown root must not replace or subset frozen authority");
        };
        let ActiveBuildTerminationTransaction::Active(transaction) =
            &build_monitor.termination_state.active_transaction
        else {
            panic!("the original authority should own one active transaction");
        };
        assert_eq!(
            transaction.additional_build_exclusion,
            AdditionalBuildExclusion::Excluded { count: 1 }
        );
        let mut owned_submission_count = 0;
        assert_eq!(
            build_monitor.submit_owned_termination_targets(transaction_id, |_| {
                owned_submission_count += 1;
                OwnedRunTerminationSubmission::Submitted(owned_run_id)
            }),
            BuildTerminationCompletionTransition::NoCompletion
        );
        assert_eq!(
            owned_submission_count, 1,
            "only the original owned root receives a termination token"
        );
    }

    #[test]
    fn terminal_records_follow_scope_replacement_and_monitoring_eviction_rules() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);

        let (mut equal_root_monitor, _) = monitor_with_owned_authority(4_200_204, owned_run_id);
        let MonitorSnapshot::Fresh(equal_root_data) = equal_root_monitor.monitor_snapshot() else {
            panic!("test monitor should be fresh");
        };
        let terminal_row = equal_root_data.session_rows()[0].clone();
        equal_root_monitor
            .termination_lifecycle_registry
            .record_external_terminal_for_test(
                &terminal_row,
                TerminationTargetResult::AlreadyGone { pid: 4_200_204 },
            );
        let MonitorSnapshot::Fresh(revision_data) =
            revision_changed_snapshot(equal_root_monitor.monitor_snapshot())
        else {
            panic!("revision helper should keep a fresh snapshot");
        };
        equal_root_monitor.replace_scope(&BuildScopeActionability::Actionable(
            revision_data.build_scope_key().clone(),
        ));
        assert_eq!(
            equal_root_monitor
                .termination_lifecycle_registry()
                .terminal_records()
                .count(),
            1
        );

        let (mut incompatible_monitor, _) = monitor_with_owned_authority(4_200_205, owned_run_id);
        let MonitorSnapshot::Fresh(incompatible_data) = incompatible_monitor.monitor_snapshot()
        else {
            panic!("test monitor should be fresh");
        };
        let terminal_row = incompatible_data.session_rows()[0].clone();
        incompatible_monitor
            .termination_lifecycle_registry
            .record_external_terminal_for_test(
                &terminal_row,
                TerminationTargetResult::AlreadyGone { pid: 4_200_205 },
            );
        incompatible_monitor.replace_scope(&BuildScopeActionability::Actionable(
            BuildScopeKey::for_test(AbsolutePath::from(PathBuf::from("/incompatible"))),
        ));
        assert_eq!(
            incompatible_monitor
                .termination_lifecycle_registry()
                .terminal_records()
                .count(),
            0
        );

        let (mut off_monitor, _) = monitor_with_owned_authority(4_200_206, owned_run_id);
        let MonitorSnapshot::Fresh(off_data) = off_monitor.monitor_snapshot() else {
            panic!("test monitor should be fresh");
        };
        let terminal_row = off_data.session_rows()[0].clone();
        off_monitor
            .termination_lifecycle_registry
            .record_external_terminal_for_test(
                &terminal_row,
                TerminationTargetResult::AlreadyGone { pid: 4_200_206 },
            );
        off_monitor.switch_off();
        assert_eq!(
            off_monitor
                .termination_lifecycle_registry()
                .terminal_records()
                .count(),
            0
        );

        let (mut active_monitor, active_session_id) =
            monitor_with_owned_authority(4_200_207, owned_run_id);
        let BuildTerminationAuthorizationConstruction::Authorized(active_authorization) =
            active_monitor.output_build_set_termination_authorization()
        else {
            panic!("active-state fixture should authorize");
        };
        let mut process_terminator = ProcessTerminator::start();
        assert!(matches!(
            active_monitor.submit_output_build_set_termination(
                active_authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
            ),
            BuildTerminationSubmission::Submitted(_)
        ));
        active_monitor.switch_off();
        assert!(
            active_monitor
                .termination_state
                .lifecycle_transaction_is_active()
        );
        assert_eq!(
            active_monitor
                .termination_lifecycle_registry()
                .lifecycle_for(&active_session_id),
            BuildTerminationLifecycle::Terminating
        );
    }

    #[test]
    fn owned_signal_outcome_waits_for_matching_finished_event() {
        let deadline = BuildTerminationDeadline::from_submission_time(Instant::now());
        let (mut termination_state, mut lifecycle_registry, _, build_session_id, owned_run_id) =
            started_owned_transaction(deadline);
        termination_state.reconcile_owned_outcome(
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            },
            &mut lifecycle_registry,
        );
        assert!(termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::Terminating
        );
        let Some(other_run_identity) = NonZeroU64::new(2) else {
            panic!("two is nonzero");
        };
        termination_state.reconcile_owned_finished(
            OwnedRunId::for_test(other_run_identity),
            &mut lifecycle_registry,
        );
        assert!(termination_state.lifecycle_transaction_is_active());

        termination_state.reconcile_owned_finished(owned_run_id, &mut lifecycle_registry);
        assert!(!termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::GoneAfterSignaling
        );
        let records: Vec<_> = lifecycle_registry.terminal_records().collect();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].session_completion(),
            BuildTerminationSessionCompletion::GoneAfterSignaling
        );
        assert_eq!(
            records[0].aggregate_completion(),
            BuildTerminationAggregateCompletion::AllTargetsGone
        );
        assert!(matches!(
            records[0].target_results(),
            [BuildTerminationTargetResult::Owned(
                OwnedBuildTerminationResult::ReapedAfterSignal { .. }
            )]
        ));
    }

    #[test]
    fn owned_reap_deadline_remains_authoritative() {
        let deadline = BuildTerminationDeadline::from_submission_time(Instant::now());
        let (mut termination_state, mut lifecycle_registry, _, build_session_id, owned_run_id) =
            started_owned_transaction(deadline);
        termination_state.reconcile_owned_outcome(
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            },
            &mut lifecycle_registry,
        );
        termination_state.expire(deadline.expires_at(), &mut lifecycle_registry);
        assert!(!termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::RetryUnavailable
        );
        let records: Vec<_> = lifecycle_registry.terminal_records().collect();
        assert_eq!(
            records[0].aggregate_completion(),
            BuildTerminationAggregateCompletion::DeadlineExpired
        );
        assert!(matches!(
            records[0].target_results(),
            [BuildTerminationTargetResult::Owned(
                OwnedBuildTerminationResult::DeadlineExpired {
                    waiting_for: OwnedBuildTerminationDeadline::ReapUnconfirmedAfterSignal,
                    ..
                }
            )]
        ));

        termination_state.reconcile_owned_finished(owned_run_id, &mut lifecycle_registry);
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::RetryUnavailable
        );
    }

    #[test]
    fn owned_process_already_reaped_completes_as_already_gone() {
        let (mut termination_state, mut lifecycle_registry, _, build_session_id, owned_run_id) =
            started_owned_transaction(BuildTerminationDeadline::from_submission_time(
                Instant::now(),
            ));
        termination_state.reconcile_owned_outcome(
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::ProcessAlreadyReaped,
            },
            &mut lifecycle_registry,
        );

        assert!(!termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::AlreadyGone
        );
        let records: Vec<_> = lifecycle_registry.terminal_records().collect();
        assert!(matches!(
            records[0].target_results(),
            [BuildTerminationTargetResult::Owned(
                OwnedBuildTerminationResult::AlreadyReaped { .. }
            )]
        ));
    }

    #[test]
    fn submission_identity_exhaustion_does_not_start_or_mark_a_transaction() {
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let build_session_id = build_session_id_for(4_200_301);
        let mut process_terminator = ProcessTerminator::start();
        let mut transaction_identity_exhausted = BuildTerminationState {
            next_transaction_id: NonZeroU64::MAX,
            ..Default::default()
        };
        let mut target_identity_exhausted = BuildTerminationState {
            next_target_id: NonZeroU64::MAX,
            ..Default::default()
        };

        for termination_state in [
            &mut transaction_identity_exhausted,
            &mut target_identity_exhausted,
        ] {
            let mut lifecycle_registry = BuildTerminationLifecycleRegistry::default();
            let submission = termination_state.submit_output_build_set_for_test(
                owned_output_build_set_authorization(
                    build_session_id.clone(),
                    owned_run_id,
                    "/workspace",
                    4_200_301,
                ),
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
                &mut lifecycle_registry,
            );
            assert_eq!(submission, BuildTerminationSubmission::IdentityExhausted);
            assert!(!termination_state.lifecycle_transaction_is_active());
            assert_eq!(
                lifecycle_registry.lifecycle_for(&build_session_id),
                BuildTerminationLifecycle::Observed
            );
        }
    }

    #[test]
    fn mixed_owned_and_external_targets_complete_under_original_session_correlation() {
        let root = ObservedProcess::new(4_200_001, 1, "root", "/usr/bin/cargo", &["cargo"])
            .with_cwd(Path::new("/workspace/project"));
        let root_incarnation = root.incarnation().clone();
        let external_session_id = BuildSessionId::for_test(root_incarnation.clone());
        let owned_session_id = build_session_id_for(4_200_002);
        let owned_run_id = OwnedRunId::for_test(NonZeroU64::MIN);
        let scope_key = BuildScopeKey::for_test(AbsolutePath::from(PathBuf::from("/workspace")));
        let authorization = OutputBuildSetTerminationAuthorization {
            scope_key,
            targets: vec![
                FrozenBuildTerminationTarget {
                    session_id:       external_session_id.clone(),
                    authority:        BuildTerminationAuthority::External(
                        ExternalBuildTerminationAuthority {
                            session_scope:                           resolved_scope("/workspace"),
                            root_identity:                           root.identity().clone(),
                            external_process_termination_capability:
                                ExternalProcessTerminationCapability::actionable_for_test(
                                    root_incarnation,
                                ),
                        },
                    ),
                    display_identity: BuildTerminationDisplayIdentity::for_test(
                        external_session_id.clone(),
                        resolved_scope("/workspace"),
                        root.identity().pid(),
                        MonitorSessionOwnership::External,
                    ),
                },
                FrozenBuildTerminationTarget {
                    session_id:       owned_session_id.clone(),
                    authority:        BuildTerminationAuthority::Owned(
                        OwnedBuildTerminationAuthority {
                            owned_run_id,
                            owned_run_termination_token: OwnedRunTerminationToken::for_test(
                                owned_run_id,
                            ),
                        },
                    ),
                    display_identity: BuildTerminationDisplayIdentity::for_test(
                        owned_session_id.clone(),
                        resolved_scope("/workspace"),
                        4_200_002,
                        MonitorSessionOwnership::Owned(owned_run_id),
                    ),
                },
            ],
        };
        let mut termination_state = BuildTerminationState::default();
        let mut lifecycle_registry = BuildTerminationLifecycleRegistry::default();
        let mut process_terminator = ProcessTerminator::start();
        let BuildTerminationSubmission::Submitted(transaction_id) = termination_state
            .submit_output_build_set_for_test(
                authorization,
                &mut process_terminator,
                BuildTerminationDeadline::from_submission_time(Instant::now()),
                &mut lifecycle_registry,
            )
        else {
            panic!("mixed transaction should start");
        };
        termination_state.submit_owned_targets(
            transaction_id,
            |_| OwnedRunTerminationSubmission::Submitted(owned_run_id),
            &mut lifecycle_registry,
        );

        let snapshot = snapshot_of(std::slice::from_ref(&root));
        let demand = termination_state
            .termination_observation_demand(ProcessRefreshConsumerDemand::TerminationTransaction);
        let observation = completed_observation(observe_build_termination_demand(
            &ProcessObserver::default(),
            &snapshot,
            demand,
        ));
        termination_state.reconcile_termination_observation(
            observation,
            &mut process_terminator,
            &mut lifecycle_registry,
        );
        let external_outcome = await_external_outcome(&process_terminator);
        termination_state.reconcile_external_outcome(&external_outcome, &mut lifecycle_registry);
        assert!(termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&external_session_id),
            BuildTerminationLifecycle::Terminating
        );

        termination_state.reconcile_owned_outcome(
            OwnedRunTerminationOutcome::Refused { owned_run_id },
            &mut lifecycle_registry,
        );
        assert!(!termination_state.lifecycle_transaction_is_active());
        assert_eq!(
            lifecycle_registry.lifecycle_for(&external_session_id),
            BuildTerminationLifecycle::GoneAfterSignaling
        );
        assert_eq!(
            lifecycle_registry.lifecycle_for(&owned_session_id),
            BuildTerminationLifecycle::RetryUnavailable
        );
    }
}
