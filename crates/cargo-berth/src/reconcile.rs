//! Shared liveness, evidence, retention-ref, and marker reconciliation.

use std::fmt;
use std::path::Path;

use crate::alert;
use crate::alert::Alert;
use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::WorktreeContext;
use crate::ledger::read_worktree_identity;
use crate::ledger::worktree_identity;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::PriorIntegrationStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseRevalidationSubject;
use crate::reservation::Reservation;
use crate::reservation::ReservationEvidenceState;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::current_trunk;
use crate::reservation::integration_status;
use crate::reservation::outstanding_integration_status;
use crate::reservation::retain_protected_tip;
use crate::worktree::WorktreeRegistry;
use crate::worktree::WorktreeRelocation;
use crate::worktree::liveness::WorktreeRegistryError;

/// Alerts that remain after one complete reconciliation.
pub(crate) struct ReconciliationReport {
    /// Durable alerts derived from retained journal state.
    pub(crate) alerts:   Vec<Alert>,
    /// Integration conclusions appended by this reconciliation.
    pub(crate) evidence: Vec<ReconciledEvidence>,
}

/// One evidence conclusion appended before the requesting stateful verb ran.
pub(crate) struct ReconciledEvidence {
    /// The reservation whose evidence changed.
    pub(crate) reservation_id: ReservationId,
    /// The newly materialized integration result.
    pub(crate) status:         IntegrationEvidenceStatus,
}

struct ReconciliationPlan {
    operations: Vec<JournalOperation>,
    action:     ReconciliationAction,
}

struct ReconciliationAction {
    active_holders:    Vec<ActiveHolder>,
    marker_contexts:   Vec<WorktreeContext>,
    repository_root:   std::path::PathBuf,
    retention_repairs: Vec<RetentionRepair>,
    alert_subjects:    Vec<AlertSubject>,
    evidence:          Vec<ReconciledEvidence>,
}

#[derive(Clone, Copy)]
struct ActiveHolder {
    worktree_id:         WorktreeId,
    coordination_run_id: CoordinationRunId,
}

struct RetentionRepair {
    reservation_id: ReservationId,
    protected_tip:  ProtectedReservationTip,
}

struct AlertSubject {
    reservation:       Reservation,
    worktree_liveness: crate::worktree::WorktreeLiveness,
}

/// Reconcile every retained reservation before a stateful command consumes it.
pub(crate) fn reconcile(
    invocation_directory: &Path,
) -> Result<ReconciliationReport, ReconcileError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let worktree_registry = WorktreeRegistry::read(worktree_context.repository_root())?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let ledger_repository = ledger.repository_identity()?;
    let berth_config = BerthConfig::read(worktree_context.repository_root())?;
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = CoordinationRunId::new();
    let outcome = ledger
        .transact_reconciliation(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => return ReconciliationValidation::Reject(error),
                };
                match build_plan(
                    &reservations,
                    &worktree_registry,
                    ledger_repository,
                    worktree_context.common_git_directory(),
                    worktree_context.repository_root(),
                    &berth_config.trunk,
                ) {
                    Ok(reconciliation_plan) => ReconciliationValidation::Apply {
                        operations: reconciliation_plan.operations,
                        action:     reconciliation_plan.action,
                    },
                    Err(error) => ReconciliationValidation::Reject(error),
                }
            },
            ReconciliationAction::commit,
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => ReconcileError::Transaction(error),
            LedgerCommittedActionError::Action(error) => error,
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended(report) => Ok(report),
        LedgerCommittedActionOutcome::Rejected(error) => Err(ReconcileError::Replay(error)),
    }
}

fn build_plan(
    reservations: &RetainedReservationSet,
    worktree_registry: &WorktreeRegistry,
    ledger_repository: crate::ids::RepoInstanceId,
    common_git_directory: &Path,
    repository_root: &Path,
    trunk_branch: &str,
) -> Result<ReconciliationPlan, ReservationReplayError> {
    let mut operations = Vec::new();
    let mut retention_repairs = Vec::new();
    let mut alert_subjects = Vec::new();
    let mut evidence = Vec::new();
    for reservation in reservations.iter() {
        let observation =
            worktree_registry.classify(ledger_repository, common_git_directory, reservation);
        if let WorktreeRelocation::Relocated { current_root } = observation.relocation {
            operations.push(JournalOperation::RelocateWorktree {
                reservation_id: reservation.id(),
                worktree_id: reservation.actor().worktree,
                previous_root: reservation.worktree_root().clone(),
                current_root,
            });
        }
        alert_subjects.push(AlertSubject {
            reservation:       reservation.clone(),
            worktree_liveness: observation.liveness,
        });
        append_evidence_and_retention(
            repository_root,
            trunk_branch,
            reservation,
            &mut operations,
            &mut retention_repairs,
            &mut evidence,
        )?;
    }
    let active_holders = reservations
        .iter()
        .filter(|reservation| matches!(reservation.lifecycle(), ReservationLifecycle::Active))
        .map(|reservation| ActiveHolder {
            worktree_id:         reservation.actor().worktree,
            coordination_run_id: reservation.actor().run,
        })
        .collect();
    Ok(ReconciliationPlan {
        operations,
        action: ReconciliationAction {
            active_holders,
            marker_contexts: worktree_registry.marker_sweep_contexts(common_git_directory),
            repository_root: repository_root.to_path_buf(),
            retention_repairs,
            alert_subjects,
            evidence,
        },
    })
}

fn append_evidence_and_retention(
    repository_root: &Path,
    trunk_branch: &str,
    reservation: &Reservation,
    operations: &mut Vec<JournalOperation>,
    retention_repairs: &mut Vec<RetentionRepair>,
    evidence_conclusions: &mut Vec<ReconciledEvidence>,
) -> Result<(), ReservationReplayError> {
    let evidence_state = reservation.evidence_state()?;
    let (protected_tip, evidence) = match evidence_state {
        ReservationEvidenceState::Active { .. }
        | ReservationEvidenceState::ReleasedWithoutCheckpoint { .. } => return Ok(()),
        ReservationEvidenceState::Outstanding {
            protected_tip,
            trunk_snapshot,
            integration_status: materialized,
        } => {
            let evidence = current_trunk(repository_root, trunk_branch).map_or(
                IntegrationEvidenceStatus::ObjectUnknown,
                |current_trunk_oid| {
                    if matches!(materialized, IntegrationEvidenceStatus::Integrated { .. }) {
                        integration_status(
                            repository_root,
                            &protected_tip,
                            &current_trunk_oid,
                            PriorIntegrationStatus::Proven,
                        )
                    } else {
                        outstanding_integration_status(
                            repository_root,
                            &protected_tip,
                            &trunk_snapshot,
                            &current_trunk_oid,
                        )
                    }
                    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown)
                },
            );
            (protected_tip, Some((materialized, evidence)))
        },
        ReservationEvidenceState::Released {
            protected_tip,
            disposition,
            integration_status: materialized,
            ..
        } => {
            let revalidation_tip = match disposition.revalidation_subject() {
                ReleaseRevalidationSubject::ProtectedTip => protected_tip.clone(),
                ReleaseRevalidationSubject::RewrittenIntegration(trunk_commit) => {
                    ProtectedReservationTip::from(trunk_commit.as_ref().clone())
                },
                ReleaseRevalidationSubject::None => {
                    retention_repairs.push(RetentionRepair {
                        reservation_id: reservation.id(),
                        protected_tip,
                    });
                    return Ok(());
                },
            };
            let evidence = current_trunk(repository_root, trunk_branch).map_or(
                IntegrationEvidenceStatus::ObjectUnknown,
                |current_trunk_oid| {
                    integration_status(
                        repository_root,
                        &revalidation_tip,
                        &current_trunk_oid,
                        PriorIntegrationStatus::Proven,
                    )
                    .unwrap_or(IntegrationEvidenceStatus::ObjectUnknown)
                },
            );
            (protected_tip, Some((materialized, evidence)))
        },
    };
    retention_repairs.push(RetentionRepair {
        reservation_id: reservation.id(),
        protected_tip,
    });
    if let Some((materialized, evidence)) = evidence {
        let edit_blocking_status = evidence.edit_blocking_status();
        if materialized != evidence || reservation.edit_blocking_status() != edit_blocking_status {
            operations.push(JournalOperation::EvidenceRevalidated {
                reservation_id: reservation.id(),
                status: evidence.clone(),
                edit_blocking_status,
            });
            evidence_conclusions.push(ReconciledEvidence {
                reservation_id: reservation.id(),
                status:         evidence,
            });
        }
    }
    Ok(())
}

impl ReconciliationAction {
    fn commit(self) -> Result<ReconciliationReport, ReconcileError> {
        for retention_repair in self.retention_repairs {
            if crate::git::commit_is_available(
                &self.repository_root,
                retention_repair.protected_tip.as_ref(),
            )? {
                retain_protected_tip(
                    &self.repository_root,
                    retention_repair.reservation_id,
                    &retention_repair.protected_tip,
                )?;
            }
        }
        for marker_context in self.marker_contexts {
            let marker_worktree_id =
                read_worktree_identity(marker_context.administrative_directory());
            marker_context.sweep_coordination_run_marker(|coordination_run_id| {
                marker_worktree_id.is_ok_and(|worktree_id| {
                    self.active_holders.iter().any(|active_holder| {
                        active_holder.worktree_id == worktree_id
                            && active_holder.coordination_run_id == coordination_run_id
                    })
                })
            })?;
        }
        let mut alerts = Vec::new();
        for alert_subject in self.alert_subjects {
            alerts.extend(alert::for_orphaned_outstanding(
                &self.repository_root,
                &alert_subject.reservation,
                alert_subject.worktree_liveness,
            )?);
        }
        Ok(ReconciliationReport {
            alerts,
            evidence: self.evidence,
        })
    }
}

/// A reconciliation failure classified for command-boundary exit behavior.
#[derive(Debug)]
pub(crate) enum ReconcileError {
    Config(ConfigError),
    Git(GitError),
    Ledger(LedgerError),
    Replay(ReservationReplayError),
    Transaction(LedgerTransactionError),
    WorktreeRegistry(WorktreeRegistryError),
}

impl ReconcileError {
    /// Convert a failed prerequisite into the requesting verb's public response.
    pub(crate) fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
        match self {
            Self::Transaction(LedgerTransactionError::LockContention) => {
                OutputEnvelope::contention(
                    command_verb,
                    &LedgerTransactionError::LockContention.to_string(),
                )
            },
            Self::Transaction(LedgerTransactionError::CorrectableInput(error)) => {
                OutputEnvelope::invalid_input(command_verb, &error.to_string())
            },
            Self::Config(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::Git(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Ledger(error)
            | Self::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::Replay(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::WorktreeRegistry(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
        }
    }
}

impl fmt::Display for ReconcileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Git(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Replay(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::WorktreeRegistry(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ReconcileError {}

impl From<ConfigError> for ReconcileError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<GitError> for ReconcileError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<LedgerError> for ReconcileError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<WorktreeRegistryError> for ReconcileError {
    fn from(error: WorktreeRegistryError) -> Self { Self::WorktreeRegistry(error) }
}
