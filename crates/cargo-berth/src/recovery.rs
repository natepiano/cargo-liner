//! User-confirmed orphan recovery, rewritten integration, retirement, and renewal.

use std::fmt;

use crate::config::BerthConfig;
use crate::config::ConfigError;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::CommittedActionValidation;
use crate::ledger::EditAuthorization;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::ledger::worktree_identity;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;
use crate::output::ResolvePayload;
use crate::reservation::AbandonmentReason;
use crate::reservation::EditBlockingStatus;
use crate::reservation::OrphanRetirementReason;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationLifecycle;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::RewrittenIntegrationTrunkCommit;
use crate::reservation::current_trunk;

/// A parsed recovery request whose variant contains exactly its required evidence.
pub(crate) struct ResolveRequest {
    /// The reservation receiving the recovery decision.
    pub(crate) reservation_id: ReservationId,
    /// The one user-selected recovery decision.
    pub(crate) recovery:       RecoveryRequest,
}

/// The mutually exclusive recovery decisions accepted at the command boundary.
pub(crate) enum RecoveryRequest {
    /// Move surviving work to the invoking replacement worktree.
    Recovered,
    /// Record a verified alternate commit already reachable from trunk.
    IntegratedAs(RewrittenIntegrationTrunkCommit),
    /// Record deliberate user-confirmed abandonment.
    Abandon(AbandonmentReason),
    /// Retire a confirmed orphan while preserving that distinct audit outcome.
    RetireOrphan(OrphanRetirementReason),
}

/// A parsed request to refresh one retained reservation's activity.
#[derive(Clone, Copy)]
pub(crate) struct RenewRequest {
    /// The reservation receiving a renewal fact.
    pub(crate) reservation_id: ReservationId,
}

/// Execute one user-confirmed recovery decision.
pub(crate) fn resolve(resolve_request: ResolveRequest) -> OutputEnvelope {
    let resolved_reservation_id = resolve_request.reservation_id;
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Resolve, &error.to_string());
        },
    };
    let mut reconciliation_report = match crate::reconcile::reconcile(&invocation_directory) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Resolve),
    };
    let output_envelope = match execute_resolution(resolve_request) {
        Ok(resolve_payload) => {
            reconciliation_report
                .alerts
                .retain(|alert| alert.reservation_id() != resolved_reservation_id);
            OutputEnvelope::resolved(resolve_payload)
        },
        Err(error) => error.into_output(CommandVerb::Resolve),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

/// Append one activity renewal without changing scopes, edges, or lifecycle.
pub(crate) fn renew(renew_request: RenewRequest) -> OutputEnvelope {
    let invocation_directory = match std::env::current_dir() {
        Ok(invocation_directory) => invocation_directory,
        Err(error) => {
            return OutputEnvelope::ledger_unreadable(CommandVerb::Renew, &error.to_string());
        },
    };
    let reconciliation_report = match crate::reconcile::reconcile(&invocation_directory) {
        Ok(reconciliation_report) => reconciliation_report,
        Err(error) => return error.into_output(CommandVerb::Renew),
    };
    let output_envelope = match execute_renewal(renew_request) {
        Ok(()) => OutputEnvelope::renewed(renew_request.reservation_id),
        Err(error) => error.into_output(CommandVerb::Renew),
    };
    output_envelope.with_alerts(reconciliation_report.alerts)
}

fn execute_resolution(resolve_request: ResolveRequest) -> Result<ResolvePayload, RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = mutation_run_id(worktree_context.administrative_directory());
    let repository_root = worktree_context.repository_root();
    let berth_config = BerthConfig::read(repository_root)?;
    let current_worktree_root = canonical_root(&worktree_context)?;
    let ledger = Ledger::open(repository_root)?;
    let outcome = ledger
        .transact_with_committed_action(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let recovery_request = match validate_recovery_request(
                    repository_root,
                    &berth_config.trunk,
                    resolve_request.recovery,
                ) {
                    Ok(recovery_request) => recovery_request,
                    Err(error) => return CommittedActionValidation::Reject(error),
                };
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return CommittedActionValidation::Reject(RecoveryRejection::Replay(error));
                    },
                };
                let reservation = match reservations.reservation(resolve_request.reservation_id) {
                    Ok(reservation) => reservation,
                    Err(ReservationReplayError::UnknownReservation(_)) => {
                        return CommittedActionValidation::Reject(
                            RecoveryRejection::UnknownReservation,
                        );
                    },
                    Err(error) => {
                        return CommittedActionValidation::Reject(RecoveryRejection::Replay(error));
                    },
                };
                match recovery_operation(
                    reservation,
                    resolve_request.reservation_id,
                    recovery_request,
                    worktree_identity.id,
                    current_worktree_root,
                    worktree_context.administrative_locator().clone(),
                ) {
                    Ok((operation, resolve_payload, committed_action)) => {
                        CommittedActionValidation::Append {
                            operation: Box::new(operation),
                            action:    RecoveryCommittedAction {
                                committed_action,
                                resolve_payload,
                            },
                        }
                    },
                    Err(error) => CommittedActionValidation::Reject(error),
                }
            },
            |committed_action| committed_action.commit(&worktree_context),
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => RecoveryError::Transaction(error),
            LedgerCommittedActionError::Action(error) => error,
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended(resolve_payload) => Ok(resolve_payload),
        LedgerCommittedActionOutcome::Rejected(RecoveryRejection::Git(error)) => {
            Err(RecoveryError::Git(error))
        },
        LedgerCommittedActionOutcome::Rejected(rejection) => {
            Err(RecoveryError::Rejected(rejection))
        },
    }
}

fn execute_renewal(renew_request: RenewRequest) -> Result<(), RecoveryError> {
    let invocation_directory = std::env::current_dir()?;
    let worktree_context = WorktreeContext::discover(&invocation_directory)?;
    let worktree_identity = worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let outcome = ledger.transact(
        worktree_identity.id,
        mutation_run_id(worktree_context.administrative_directory()),
        |state| {
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations,
                Err(error) => {
                    return TransactionValidation::Reject(RecoveryRejection::Replay(error));
                },
            };
            let reservation = match reservations.reservation(renew_request.reservation_id) {
                Ok(reservation) => reservation,
                Err(ReservationReplayError::UnknownReservation(_)) => {
                    return TransactionValidation::Reject(RecoveryRejection::UnknownReservation);
                },
                Err(error) => {
                    return TransactionValidation::Reject(RecoveryRejection::Replay(error));
                },
            };
            if matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            ) {
                return TransactionValidation::Reject(RecoveryRejection::AlreadyResolved);
            }
            TransactionValidation::Append(Box::new(JournalOperation::Renew {
                reservation_id: renew_request.reservation_id,
            }))
        },
    )?;
    match outcome {
        LedgerTransactionOutcome::Appended(_) => Ok(()),
        LedgerTransactionOutcome::Rejected(rejection) => Err(RecoveryError::Rejected(rejection)),
    }
}

fn validate_recovery_request(
    repository_root: &std::path::Path,
    trunk_branch: &str,
    recovery_request: RecoveryRequest,
) -> Result<RecoveryRequest, RecoveryRejection> {
    let RecoveryRequest::IntegratedAs(trunk_commit) = recovery_request else {
        return Ok(recovery_request);
    };
    let trunk_oid = current_trunk(repository_root, trunk_branch).map_err(RecoveryRejection::Git)?;
    match git::reachability(repository_root, trunk_commit.as_ref(), &trunk_oid)
        .map_err(RecoveryRejection::Git)?
    {
        git::Reachability::Ancestor => Ok(RecoveryRequest::IntegratedAs(trunk_commit)),
        git::Reachability::NotAncestor | git::Reachability::ObjectUnknown => {
            Err(RecoveryRejection::UnreachableIntegrationEvidence)
        },
    }
}

fn recovery_operation(
    reservation: &crate::reservation::Reservation,
    reservation_id: ReservationId,
    recovery_request: RecoveryRequest,
    current_worktree_id: crate::ids::WorktreeId,
    current_worktree_root: CanonicalWorktreeRoot,
    current_worktree_administrative_locator: crate::ledger::WorktreeAdministrativeLocator,
) -> Result<(JournalOperation, ResolvePayload, RecoveryAction), RecoveryRejection> {
    match recovery_request {
        RecoveryRequest::Recovered => {
            if reservation.actor().worktree == current_worktree_id {
                return Err(RecoveryRejection::SameWorktreeRecovery);
            }
            let committed_action = match reservation.lifecycle() {
                ReservationLifecycle::Active => {
                    RecoveryAction::PublishMarker(reservation.actor().run)
                },
                ReservationLifecycle::Outstanding { .. } => RecoveryAction::None,
                ReservationLifecycle::Released { .. } => {
                    return Err(RecoveryRejection::AlreadyResolved);
                },
            };
            Ok((
                JournalOperation::RebindWorktree {
                    reservation_id,
                    previous_worktree_id: reservation.actor().worktree,
                    current_worktree_id,
                    current_worktree_root,
                    current_worktree_administrative_locator,
                },
                ResolvePayload::Recovered {
                    reservation_id,
                    worktree_id: current_worktree_id,
                },
                committed_action,
            ))
        },
        RecoveryRequest::IntegratedAs(trunk_commit) => {
            let disposition = ReleaseDisposition::RewrittenIntegration(trunk_commit);
            let operation = match reservation.lifecycle() {
                ReservationLifecycle::Outstanding { .. } => JournalOperation::Release {
                    reservation_id,
                    disposition: disposition.clone(),
                },
                ReservationLifecycle::Released {
                    disposition: superseded,
                } if reservation.edit_blocking_status() == EditBlockingStatus::Blocking => {
                    JournalOperation::ReplaceReleaseDisposition {
                        reservation_id,
                        superseded: superseded.clone(),
                        replacement: disposition.clone(),
                    }
                },
                ReservationLifecycle::Released { .. } => {
                    return Err(RecoveryRejection::AlreadyResolved);
                },
                ReservationLifecycle::Active => {
                    return Err(RecoveryRejection::CheckpointRequired);
                },
            };
            Ok((
                operation,
                ResolvePayload::Released {
                    reservation_id,
                    disposition,
                },
                RecoveryAction::None,
            ))
        },
        RecoveryRequest::Abandon(reason) => disposition_operation(
            reservation,
            reservation_id,
            ReleaseDisposition::Abandoned(reason),
        ),
        RecoveryRequest::RetireOrphan(reason) => disposition_operation(
            reservation,
            reservation_id,
            ReleaseDisposition::RetiredOrphan(reason),
        ),
    }
}

fn disposition_operation(
    reservation: &crate::reservation::Reservation,
    reservation_id: ReservationId,
    disposition: ReleaseDisposition,
) -> Result<(JournalOperation, ResolvePayload, RecoveryAction), RecoveryRejection> {
    match reservation.lifecycle() {
        ReservationLifecycle::Active | ReservationLifecycle::Outstanding { .. } => Ok((
            JournalOperation::Release {
                reservation_id,
                disposition: disposition.clone(),
            },
            ResolvePayload::Released {
                reservation_id,
                disposition,
            },
            RecoveryAction::None,
        )),
        ReservationLifecycle::Released { .. } => Err(RecoveryRejection::AlreadyResolved),
    }
}

fn canonical_root(
    worktree_context: &WorktreeContext,
) -> Result<CanonicalWorktreeRoot, RecoveryError> {
    worktree_context
        .repository_root()
        .to_str()
        .ok_or(RecoveryError::NonUtf8WorktreeRoot)?
        .parse()
        .map_err(|_| RecoveryError::InvalidCanonicalWorktreeRoot)
}

fn mutation_run_id(administrative_directory: &std::path::Path) -> CoordinationRunId {
    match EditAuthorization::resolve(administrative_directory) {
        EditAuthorization::Environment(coordination_run_id)
        | EditAuthorization::Marker {
            coordination_run_id,
            ..
        } => coordination_run_id,
        EditAuthorization::Unidentified => CoordinationRunId::new(),
    }
}

struct RecoveryCommittedAction {
    committed_action: RecoveryAction,
    resolve_payload:  ResolvePayload,
}

enum RecoveryAction {
    None,
    PublishMarker(CoordinationRunId),
}

impl RecoveryCommittedAction {
    fn commit(self, worktree_context: &WorktreeContext) -> Result<ResolvePayload, RecoveryError> {
        match self.committed_action {
            RecoveryAction::None => {},
            RecoveryAction::PublishMarker(coordination_run_id) => {
                worktree_context.publish_coordination_run_marker(coordination_run_id)?;
            },
        }
        Ok(self.resolve_payload)
    }
}

#[derive(Debug)]
enum RecoveryRejection {
    UnknownReservation,
    Replay(ReservationReplayError),
    CheckpointRequired,
    AlreadyResolved,
    SameWorktreeRecovery,
    UnreachableIntegrationEvidence,
    Git(GitError),
}

#[derive(Debug)]
enum RecoveryError {
    Io(std::io::Error),
    Config(ConfigError),
    Git(GitError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    Rejected(RecoveryRejection),
    NonUtf8WorktreeRoot,
    InvalidCanonicalWorktreeRoot,
}

impl RecoveryError {
    fn into_output(self, command_verb: CommandVerb) -> OutputEnvelope {
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
            Self::Rejected(rejection) => {
                OutputEnvelope::invalid_input(command_verb, &rejection.to_string())
            },
            Self::Io(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Config(error) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::Git(error) => OutputEnvelope::ledger_unreadable(command_verb, &error.to_string()),
            Self::Ledger(error)
            | Self::Transaction(LedgerTransactionError::LedgerUnreadable(error)) => {
                OutputEnvelope::ledger_unreadable(command_verb, &error.to_string())
            },
            Self::NonUtf8WorktreeRoot => OutputEnvelope::ledger_unreadable(
                command_verb,
                "the replacement worktree root is not UTF-8",
            ),
            Self::InvalidCanonicalWorktreeRoot => OutputEnvelope::ledger_unreadable(
                command_verb,
                "the replacement worktree root is not canonical",
            ),
        }
    }
}

impl fmt::Display for RecoveryRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownReservation => formatter.write_str("the reservation does not exist"),
            Self::Replay(error) => error.fmt(formatter),
            Self::CheckpointRequired => {
                formatter.write_str("the reservation must have a protected checkpoint first")
            },
            Self::AlreadyResolved => formatter.write_str("the reservation is already resolved"),
            Self::SameWorktreeRecovery => formatter
                .write_str("--recovered requires a replacement worktree with a new identity"),
            Self::UnreachableIntegrationEvidence => formatter.write_str(
                "the --integrated-as commit must resolve in this repository and be reachable from trunk",
            ),
            Self::Git(error) => error.fmt(formatter),
        }
    }
}

impl From<std::io::Error> for RecoveryError {
    fn from(error: std::io::Error) -> Self { Self::Io(error) }
}

impl From<ConfigError> for RecoveryError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<GitError> for RecoveryError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}

impl From<LedgerError> for RecoveryError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}

impl From<LedgerTransactionError> for RecoveryError {
    fn from(error: LedgerTransactionError) -> Self { Self::Transaction(error) }
}
