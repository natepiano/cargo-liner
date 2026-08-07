//! Build-session termination lifecycle and terminal facts retained across snapshots.

use std::collections::BTreeMap;

use super::super::session::BuildSessionId;
use super::super::session::SessionScope;
use super::super::snapshot::MonitorSessionOwnership;
use super::super::snapshot::MonitorSessionRow;
use super::transaction::BuildTerminationTransactionId;
use crate::process_termination::TerminationExecutionTargetRole;
use crate::process_termination::TerminationTargetId;
use crate::process_termination::TerminationTargetResult;
use crate::tui::OwnedRunId;

/// The state the Output presentation joins to a replaceable monitor row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationLifecycle {
    /// No submitted termination transaction currently changes this session.
    Observed,
    /// A frozen target is awaiting a backend outcome, owned reap, or deadline.
    Terminating,
    /// At least one signal was accepted and every target was later observed gone.
    GoneAfterSignaling,
    /// Every authorized target was absent before signal delivery.
    AlreadyGone,
    /// The transaction ended without proving every target gone.
    RetryUnavailable,
}

/// Frozen identity needed to present a terminated session after its row disappears.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildTerminationDisplayIdentity {
    session_id:        BuildSessionId,
    session_scope:     SessionScope,
    root_pid:          u32,
    session_ownership: MonitorSessionOwnership,
}

impl BuildTerminationDisplayIdentity {
    pub(super) fn from_monitor_session_row(monitor_session_row: &MonitorSessionRow) -> Self {
        Self {
            session_id:        monitor_session_row.build_session_id().clone(),
            session_scope:     monitor_session_row.build_session().session_scope().clone(),
            root_pid:          monitor_session_row
                .build_session()
                .root_observation()
                .root_pid(),
            session_ownership: monitor_session_row.session_ownership(),
        }
    }

    pub(crate) const fn build_session_id(&self) -> &BuildSessionId { &self.session_id }

    pub(crate) const fn session_scope(&self) -> &SessionScope { &self.session_scope }

    pub(crate) const fn root_pid(&self) -> u32 { self.root_pid }

    pub(crate) const fn session_ownership(&self) -> MonitorSessionOwnership {
        self.session_ownership
    }

    #[cfg(test)]
    pub(crate) const fn for_test(
        session_id: BuildSessionId,
        session_scope: SessionScope,
        root_pid: u32,
        session_ownership: MonitorSessionOwnership,
    ) -> Self {
        Self {
            session_id,
            session_scope,
            root_pid,
            session_ownership,
        }
    }

    fn replacement_by(&self, observed: &Self) -> BuildTerminationRecordReplacement {
        if self.session_id == observed.session_id
            || self
                .session_scope
                .shares_resolved_root(&observed.session_scope)
        {
            BuildTerminationRecordReplacement::Replaced
        } else {
            BuildTerminationRecordReplacement::Unrelated
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildTerminationRecordReplacement {
    Replaced,
    Unrelated,
}

/// Why an owned actor request failed before it produced an actor outcome.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedBuildTerminationSubmissionRefusal {
    /// The actor accepted a different run identity than the frozen target.
    RunCorrelationMismatch,
    /// The actor already had its single pending termination request.
    RequestAlreadyPending,
    /// The actor refused the frozen run-bound token.
    TokenRefused,
    /// The actor command endpoint was unavailable.
    ActorUnavailable,
}

/// Which owned wait was still incomplete when the transaction deadline arrived.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedBuildTerminationDeadline {
    /// The actor token had not yet been submitted.
    AwaitingSubmission,
    /// The actor had accepted the token but had not reported signal admission.
    AwaitingSignalOutcome,
    /// The actor sent the signal but had not yet reaped the child.
    ReapUnconfirmedAfterSignal,
}

/// The terminal fact established for one Cargo Port-owned build target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OwnedBuildTerminationResult {
    /// The actor sent the signal and its later `Finished` event proved reap.
    ReapedAfterSignal { owned_run_id: OwnedRunId },
    /// The child had already been reaped when the actor handled the request.
    AlreadyReaped { owned_run_id: OwnedRunId },
    /// Strong identity revalidation no longer matched the authorized process.
    IdentityNoLongerCurrent { owned_run_id: OwnedRunId },
    /// The actor could not deliver the graceful process-group signal.
    SignalFailed { owned_run_id: OwnedRunId },
    /// The actor worker refused the token after receiving it.
    ActorRefused { owned_run_id: OwnedRunId },
    /// Submission failed before the actor could produce an outcome.
    SubmissionRefused {
        owned_run_id: OwnedRunId,
        refusal:      OwnedBuildTerminationSubmissionRefusal,
    },
    /// The bounded transaction expired while waiting on the named actor phase.
    DeadlineExpired {
        owned_run_id: OwnedRunId,
        waiting_for:  OwnedBuildTerminationDeadline,
    },
}

/// One external target's correlated terminal fact from one bounded pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExternalBuildTerminationResult {
    semantic_target_id: TerminationTargetId,
    role:               TerminationExecutionTargetRole,
    result:             TerminationTargetResult,
}

impl ExternalBuildTerminationResult {
    pub(super) const fn new(
        semantic_target_id: TerminationTargetId,
        role: TerminationExecutionTargetRole,
        result: TerminationTargetResult,
    ) -> Self {
        Self {
            semantic_target_id,
            role,
            result,
        }
    }

    pub(crate) const fn semantic_target_id(&self) -> TerminationTargetId { self.semantic_target_id }

    pub(crate) const fn role(&self) -> TerminationExecutionTargetRole { self.role }

    pub(crate) const fn result(&self) -> &TerminationTargetResult { &self.result }
}

/// One terminal target fact retained for presentation and retry decisions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationTargetResult {
    /// A Cargo Port-owned actor established this result.
    Owned(OwnedBuildTerminationResult),
    /// The external worker or sole observer established this result.
    External(ExternalBuildTerminationResult),
    /// Transaction correlation ended without a backend fact for this session.
    TransactionResultUnavailable,
}

/// Aggregate outcome for one build session in a completed transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationSessionCompletion {
    /// Every target was already absent before any signal was delivered.
    AlreadyGone,
    /// Every target was observed gone and at least one signal was delivered.
    GoneAfterSignaling,
    /// At least one target ended without a confirmed disappearance.
    RetryUnavailable,
    /// The transaction deadline ended at least one target wait.
    DeadlineExpired,
}

impl BuildTerminationSessionCompletion {
    const fn lifecycle(self) -> BuildTerminationLifecycle {
        match self {
            Self::AlreadyGone => BuildTerminationLifecycle::AlreadyGone,
            Self::GoneAfterSignaling => BuildTerminationLifecycle::GoneAfterSignaling,
            Self::RetryUnavailable | Self::DeadlineExpired => {
                BuildTerminationLifecycle::RetryUnavailable
            },
        }
    }
}

/// Aggregate outcome across every session frozen into one transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BuildTerminationAggregateCompletion {
    /// Every frozen target was observed gone.
    AllTargetsGone,
    /// At least one target ended without a confirmed disappearance.
    RetryUnavailable,
    /// The transaction deadline ended at least one target wait.
    DeadlineExpired,
}

/// Persistent row-independent result for one completed build session.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BuildTerminationTerminalRecord {
    transaction_id:       BuildTerminationTransactionId,
    display_identity:     BuildTerminationDisplayIdentity,
    session_completion:   BuildTerminationSessionCompletion,
    aggregate_completion: BuildTerminationAggregateCompletion,
    target_results:       Vec<BuildTerminationTargetResult>,
}

impl BuildTerminationTerminalRecord {
    pub(crate) const fn transaction_id(&self) -> BuildTerminationTransactionId {
        self.transaction_id
    }

    pub(crate) const fn display_identity(&self) -> &BuildTerminationDisplayIdentity {
        &self.display_identity
    }

    pub(crate) const fn session_completion(&self) -> BuildTerminationSessionCompletion {
        self.session_completion
    }

    pub(crate) const fn aggregate_completion(&self) -> BuildTerminationAggregateCompletion {
        self.aggregate_completion
    }

    pub(crate) fn target_results(&self) -> &[BuildTerminationTargetResult] { &self.target_results }
}

#[derive(Debug, Eq, PartialEq)]
enum BuildTerminationLifecycleEntry {
    Terminating {
        transaction_id:   BuildTerminationTransactionId,
        display_identity: BuildTerminationDisplayIdentity,
    },
    Terminal(BuildTerminationTerminalRecord),
}

/// Lifecycle and terminal result owner independent of the replaceable snapshot.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct BuildTerminationLifecycleRegistry {
    entries: BTreeMap<BuildSessionId, BuildTerminationLifecycleEntry>,
}

impl BuildTerminationLifecycleRegistry {
    /// The retained lifecycle for one session, or the ordinary observed state.
    pub(crate) fn lifecycle_for(
        &self,
        build_session_id: &BuildSessionId,
    ) -> BuildTerminationLifecycle {
        match self.entries.get(build_session_id) {
            Some(BuildTerminationLifecycleEntry::Terminating { .. }) => {
                BuildTerminationLifecycle::Terminating
            },
            Some(BuildTerminationLifecycleEntry::Terminal(record)) => {
                record.session_completion.lifecycle()
            },
            None => BuildTerminationLifecycle::Observed,
        }
    }

    /// Completed records available to presentation without a current monitor row.
    pub(crate) fn terminal_records(&self) -> impl Iterator<Item = &BuildTerminationTerminalRecord> {
        self.entries.values().filter_map(|entry| match entry {
            BuildTerminationLifecycleEntry::Terminating { .. } => None,
            BuildTerminationLifecycleEntry::Terminal(record) => Some(record),
        })
    }

    pub(super) fn mark_terminating(
        &mut self,
        transaction_id: BuildTerminationTransactionId,
        display_identity: BuildTerminationDisplayIdentity,
    ) {
        self.entries.insert(
            display_identity.build_session_id().clone(),
            BuildTerminationLifecycleEntry::Terminating {
                transaction_id,
                display_identity,
            },
        );
    }

    /// Stage an active lifecycle at the presentation boundary for focused tests.
    #[cfg(test)]
    pub(crate) fn mark_terminating_for_test(&mut self, build_session_id: &BuildSessionId) {
        self.mark_terminating(
            BuildTerminationTransactionId(std::num::NonZeroU64::MIN),
            BuildTerminationDisplayIdentity::for_test(
                build_session_id.clone(),
                SessionScope::Unresolved,
                0,
                MonitorSessionOwnership::External,
            ),
        );
    }

    /// Stage one external terminal record for presentation-boundary tests.
    #[cfg(test)]
    pub(crate) fn record_external_terminal_for_test(
        &mut self,
        monitor_session_row: &MonitorSessionRow,
        termination_target_result: TerminationTargetResult,
    ) {
        let transaction_id = BuildTerminationTransactionId(std::num::NonZeroU64::MIN);
        let build_session_id = monitor_session_row.build_session_id().clone();
        self.mark_terminating(
            transaction_id,
            BuildTerminationDisplayIdentity::from_monitor_session_row(monitor_session_row),
        );
        self.complete_transaction(
            transaction_id,
            BTreeMap::from([(
                build_session_id,
                vec![BuildTerminationTargetResult::External(
                    ExternalBuildTerminationResult::new(
                        TerminationTargetId::for_test(std::num::NonZeroU64::MIN),
                        TerminationExecutionTargetRole::FrozenRoot,
                        termination_target_result,
                    ),
                )],
            )]),
        );
    }

    pub(super) fn complete_transaction(
        &mut self,
        transaction_id: BuildTerminationTransactionId,
        mut target_results_by_session: BTreeMap<BuildSessionId, Vec<BuildTerminationTargetResult>>,
    ) {
        let completing_sessions: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(build_session_id, entry)| match entry {
                BuildTerminationLifecycleEntry::Terminating {
                    transaction_id: active_transaction_id,
                    ..
                } if *active_transaction_id == transaction_id => Some(build_session_id.clone()),
                BuildTerminationLifecycleEntry::Terminating { .. }
                | BuildTerminationLifecycleEntry::Terminal(_) => None,
            })
            .collect();
        for build_session_id in &completing_sessions {
            target_results_by_session
                .entry(build_session_id.clone())
                .or_insert_with(|| {
                    vec![BuildTerminationTargetResult::TransactionResultUnavailable]
                });
        }
        let aggregate_completion = aggregate_completion(
            target_results_by_session
                .values()
                .flat_map(|target_results| target_results.iter()),
        );
        for build_session_id in completing_sessions {
            let Some(BuildTerminationLifecycleEntry::Terminating {
                display_identity, ..
            }) = self.entries.remove(&build_session_id)
            else {
                continue;
            };
            let target_results = target_results_by_session
                .remove(&build_session_id)
                .unwrap_or_else(|| {
                    vec![BuildTerminationTargetResult::TransactionResultUnavailable]
                });
            let session_completion = session_completion(&target_results);
            self.entries.insert(
                build_session_id,
                BuildTerminationLifecycleEntry::Terminal(BuildTerminationTerminalRecord {
                    transaction_id,
                    display_identity,
                    session_completion,
                    aggregate_completion,
                    target_results,
                }),
            );
        }
    }

    pub(in crate::build_monitor) fn record_fresh_observations<'a>(
        &mut self,
        monitor_session_rows: impl IntoIterator<Item = &'a MonitorSessionRow>,
    ) {
        let observed: Vec<_> = monitor_session_rows
            .into_iter()
            .map(BuildTerminationDisplayIdentity::from_monitor_session_row)
            .collect();
        self.record_fresh_display_identities(&observed);
    }

    fn record_fresh_display_identities(&mut self, observed: &[BuildTerminationDisplayIdentity]) {
        self.entries.retain(|_, entry| match entry {
            BuildTerminationLifecycleEntry::Terminating { .. } => true,
            BuildTerminationLifecycleEntry::Terminal(record) => observed.iter().all(|current| {
                record.display_identity.replacement_by(current)
                    == BuildTerminationRecordReplacement::Unrelated
            }),
        });
    }

    #[cfg(test)]
    fn record_fresh_display_identities_for_test(
        &mut self,
        observed: &[BuildTerminationDisplayIdentity],
    ) {
        self.record_fresh_display_identities(observed);
    }

    pub(in crate::build_monitor) fn clear_terminal_entries(&mut self) {
        self.entries
            .retain(|_, entry| matches!(entry, BuildTerminationLifecycleEntry::Terminating { .. }));
    }
}

fn session_completion(
    target_results: &[BuildTerminationTargetResult],
) -> BuildTerminationSessionCompletion {
    let mut session_completion = BuildTerminationSessionCompletion::AlreadyGone;
    for target_result in target_results {
        match target_completion(target_result) {
            BuildTerminationTargetCompletion::DeadlineExpired => {
                return BuildTerminationSessionCompletion::DeadlineExpired;
            },
            BuildTerminationTargetCompletion::RetryUnavailable => {
                session_completion = BuildTerminationSessionCompletion::RetryUnavailable;
            },
            BuildTerminationTargetCompletion::GoneAfterSignaling
                if session_completion == BuildTerminationSessionCompletion::AlreadyGone =>
            {
                session_completion = BuildTerminationSessionCompletion::GoneAfterSignaling;
            },
            BuildTerminationTargetCompletion::GoneAfterSignaling
            | BuildTerminationTargetCompletion::AlreadyGone => {},
        }
    }
    session_completion
}

fn aggregate_completion<'a>(
    target_results: impl IntoIterator<Item = &'a BuildTerminationTargetResult>,
) -> BuildTerminationAggregateCompletion {
    let mut aggregate_completion = BuildTerminationAggregateCompletion::AllTargetsGone;
    for target_result in target_results {
        match target_completion(target_result) {
            BuildTerminationTargetCompletion::DeadlineExpired => {
                return BuildTerminationAggregateCompletion::DeadlineExpired;
            },
            BuildTerminationTargetCompletion::RetryUnavailable => {
                aggregate_completion = BuildTerminationAggregateCompletion::RetryUnavailable;
            },
            BuildTerminationTargetCompletion::AlreadyGone
            | BuildTerminationTargetCompletion::GoneAfterSignaling => {},
        }
    }
    aggregate_completion
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildTerminationTargetCompletion {
    AlreadyGone,
    GoneAfterSignaling,
    RetryUnavailable,
    DeadlineExpired,
}

const fn target_completion(
    target_result: &BuildTerminationTargetResult,
) -> BuildTerminationTargetCompletion {
    match target_result {
        BuildTerminationTargetResult::Owned(OwnedBuildTerminationResult::AlreadyReaped {
            ..
        }) => BuildTerminationTargetCompletion::AlreadyGone,
        BuildTerminationTargetResult::Owned(OwnedBuildTerminationResult::ReapedAfterSignal {
            ..
        }) => BuildTerminationTargetCompletion::GoneAfterSignaling,
        BuildTerminationTargetResult::Owned(
            OwnedBuildTerminationResult::IdentityNoLongerCurrent { .. }
            | OwnedBuildTerminationResult::SignalFailed { .. }
            | OwnedBuildTerminationResult::ActorRefused { .. }
            | OwnedBuildTerminationResult::SubmissionRefused { .. },
        )
        | BuildTerminationTargetResult::TransactionResultUnavailable => {
            BuildTerminationTargetCompletion::RetryUnavailable
        },
        BuildTerminationTargetResult::Owned(OwnedBuildTerminationResult::DeadlineExpired {
            ..
        }) => BuildTerminationTargetCompletion::DeadlineExpired,
        BuildTerminationTargetResult::External(external) => match external.result() {
            TerminationTargetResult::AlreadyGone { .. } => {
                BuildTerminationTargetCompletion::AlreadyGone
            },
            #[cfg(any(target_os = "linux", test))]
            TerminationTargetResult::GoneAfterSignaling { .. } => {
                BuildTerminationTargetCompletion::GoneAfterSignaling
            },
            TerminationTargetResult::Refused(
                crate::process_termination::TerminationError::DeadlineExpired { .. },
            ) => BuildTerminationTargetCompletion::DeadlineExpired,
            TerminationTargetResult::Refused(_) => {
                BuildTerminationTargetCompletion::RetryUnavailable
            },
            #[cfg(any(target_os = "linux", test))]
            TerminationTargetResult::Survived { .. }
            | TerminationTargetResult::SignaledButUnconfirmed { .. } => {
                BuildTerminationTargetCompletion::RetryUnavailable
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::build_monitor::session::ScopeAttribution;
    use crate::process_observation::identity::ProcessIdentity;
    use crate::process_observation::identity::ProcessIncarnation;
    use crate::process_termination::TerminationError;
    use crate::project::AbsolutePath;
    use crate::project::CanonicalCheckoutRoot;

    fn build_session_id_for(pid: u32) -> BuildSessionId {
        BuildSessionId::for_test(ProcessIncarnation::for_test(
            ProcessIdentity::for_test(pid, 12),
            "/usr/bin/cargo",
        ))
    }

    fn display_identity(pid: u32, root: &str) -> BuildTerminationDisplayIdentity {
        BuildTerminationDisplayIdentity::for_test(
            build_session_id_for(pid),
            SessionScope::Resolved {
                method: ScopeAttribution::WorkingDirectoryManifest,
                root:   CanonicalCheckoutRoot::for_test(AbsolutePath::from(PathBuf::from(root))),
            },
            pid,
            MonitorSessionOwnership::External,
        )
    }

    #[test]
    fn terminal_record_keeps_identity_and_detailed_external_results() {
        let display_identity = display_identity(4_242, "/workspace");
        let build_session_id = display_identity.build_session_id().clone();
        let transaction_id = BuildTerminationTransactionId(std::num::NonZeroU64::MIN);
        let target_id = TerminationTargetId::for_test(std::num::NonZeroU64::MIN);
        let mut registry = BuildTerminationLifecycleRegistry::default();
        registry.mark_terminating(transaction_id, display_identity);
        registry.complete_transaction(
            transaction_id,
            BTreeMap::from([(
                build_session_id.clone(),
                vec![BuildTerminationTargetResult::External(
                    ExternalBuildTerminationResult::new(
                        target_id,
                        TerminationExecutionTargetRole::FrozenRoot,
                        TerminationTargetResult::SignaledButUnconfirmed { pid: 4_242 },
                    ),
                )],
            )]),
        );

        let records: Vec<_> = registry.terminal_records().collect();
        assert_eq!(records.len(), 1);
        let record = records[0];
        assert_eq!(record.display_identity().root_pid(), 4_242);
        assert_eq!(
            record.session_completion(),
            BuildTerminationSessionCompletion::RetryUnavailable
        );
        assert_eq!(record.target_results().len(), 1);
        assert_eq!(
            registry.lifecycle_for(&build_session_id),
            BuildTerminationLifecycle::RetryUnavailable
        );

        registry.record_fresh_display_identities_for_test(&[]);
        assert_eq!(registry.terminal_records().count(), 1);
    }

    #[test]
    fn replacement_build_evicts_terminal_record_but_not_active_transaction() {
        let completed_identity = display_identity(4_242, "/workspace");
        let active_identity = display_identity(4_243, "/other");
        let replacement_identity = display_identity(4_244, "/workspace");
        let transaction_id = BuildTerminationTransactionId(std::num::NonZeroU64::MIN);
        let Some(next_identity) = std::num::NonZeroU64::new(2) else {
            return;
        };
        let active_transaction_id = BuildTerminationTransactionId(next_identity);
        let active_session_id = active_identity.build_session_id().clone();
        let mut registry = BuildTerminationLifecycleRegistry::default();
        registry.mark_terminating(transaction_id, completed_identity);
        registry.complete_transaction(transaction_id, BTreeMap::new());
        registry.mark_terminating(active_transaction_id, active_identity);

        registry.record_fresh_display_identities_for_test(&[replacement_identity]);

        assert_eq!(registry.terminal_records().count(), 0);
        assert_eq!(
            registry.lifecycle_for(&active_session_id),
            BuildTerminationLifecycle::Terminating
        );
    }

    #[test]
    fn external_terminal_records_preserve_each_semantic_result() {
        let cases = [
            (
                TerminationTargetResult::AlreadyGone { pid: 4_250 },
                BuildTerminationSessionCompletion::AlreadyGone,
            ),
            (
                TerminationTargetResult::GoneAfterSignaling { pid: 4_250 },
                BuildTerminationSessionCompletion::GoneAfterSignaling,
            ),
            (
                TerminationTargetResult::Survived { pid: 4_250 },
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::SignaledButUnconfirmed { pid: 4_250 },
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(TerminationError::HostRejectedSignal {
                    pid: 4_250,
                }),
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(
                    TerminationError::ProcessRevalidationUnavailable { pid: 4_250 },
                ),
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(TerminationError::ProcessImageReplaced {
                    pid: 4_250,
                }),
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(TerminationError::RequestIdentitiesExhausted {
                    pid: 4_250,
                }),
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(TerminationError::TerminationWorkerUnavailable {
                    pid: 4_250,
                }),
                BuildTerminationSessionCompletion::RetryUnavailable,
            ),
            (
                TerminationTargetResult::Refused(TerminationError::DeadlineExpired { pid: 4_250 }),
                BuildTerminationSessionCompletion::DeadlineExpired,
            ),
        ];
        for (termination_target_result, expected_completion) in cases {
            let display_identity = display_identity(4_250, "/workspace");
            let build_session_id = display_identity.build_session_id().clone();
            let transaction_id = BuildTerminationTransactionId(std::num::NonZeroU64::MIN);
            let mut registry = BuildTerminationLifecycleRegistry::default();
            registry.mark_terminating(transaction_id, display_identity);
            registry.complete_transaction(
                transaction_id,
                BTreeMap::from([(
                    build_session_id,
                    vec![BuildTerminationTargetResult::External(
                        ExternalBuildTerminationResult::new(
                            TerminationTargetId::for_test(std::num::NonZeroU64::MIN),
                            TerminationExecutionTargetRole::FrozenRoot,
                            termination_target_result.clone(),
                        ),
                    )],
                )]),
            );

            let records: Vec<_> = registry.terminal_records().collect();
            assert_eq!(records[0].session_completion(), expected_completion);
            let external_results: Vec<_> = records[0]
                .target_results()
                .iter()
                .filter_map(|target_result| match target_result {
                    BuildTerminationTargetResult::External(external_result) => {
                        Some(external_result)
                    },
                    BuildTerminationTargetResult::Owned(_)
                    | BuildTerminationTargetResult::TransactionResultUnavailable => None,
                })
                .collect();
            assert_eq!(external_results.len(), 1);
            assert_eq!(external_results[0].result(), &termination_target_result);
        }
    }
}
