//! Shared-ledger location, initialization, identity storage, and transactions.

mod authorization;
mod constants;
mod coordination_run_marker;
mod error;
mod handle;
mod identity;
mod journal;
mod lock;
mod path;
mod projection;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod test_support;
mod worktree_context;

pub(crate) use authorization::EditAuthorization;
pub(crate) use authorization::ResolvedEditAuthorization;
pub(crate) use constants::HARNESS_SESSION_ENVIRONMENT;
pub(crate) use constants::MUTATING_VERB_CONTENTION_TOLERANCE;
pub(crate) use coordination_run_marker::CoordinationRunMarkerRemoval;
pub(crate) use error::LedgerCommittedActionError;
pub(crate) use error::LedgerError;
pub(crate) use error::LedgerTransactionError;
pub(crate) use handle::CommittedActionValidation;
pub(crate) use handle::Ledger;
pub(crate) use handle::LedgerCommittedActionOutcome;
pub(crate) use handle::LedgerInitialization;
pub(crate) use handle::LedgerTransactionOutcome;
pub(crate) use handle::ReconciliationValidation;
pub(crate) use handle::RecoverableReconciliationAppendFailures;
pub(crate) use handle::ReplayedLedgerState;
pub(crate) use handle::TransactionValidation;
pub(crate) use identity::read_worktree_identity;
pub(crate) use identity::resolve_identity;
pub(crate) use identity::worktree_identity;
pub(crate) use journal::BypassCause;
pub(crate) use journal::BypassOccurrenceTime;
pub(crate) use journal::BypassRecording;
pub(crate) use journal::BypassedAction;
pub(crate) use journal::BypassedMergeIdentity;
pub(crate) use journal::CanonicalWorktreeRoot;
pub(crate) use journal::ClaimHeadCommit;
pub(crate) use journal::ClaimHeadSnapshot;
pub(crate) use journal::ClaimSource;
pub(crate) use journal::CollisionPathSet;
pub(crate) use journal::ForcedIntegrationReason;
pub(crate) use journal::ForeignReservationIdSet;
pub(crate) use journal::FullRefName;
pub(crate) use journal::IncursionIncidentId;
pub(crate) use journal::IncursionPathSet;
pub(crate) use journal::JournalActor;
pub(crate) use journal::JournalEvent;
pub(crate) use journal::JournalOperation;
pub(crate) use journal::NonEmptyReservationPurpose;
pub(crate) use journal::OrderingDirection;
pub(crate) use journal::PendingBypassMarkerId;
pub(crate) use journal::ProtectedPhaseStartHead;
pub(crate) use journal::ReservationPurpose;
pub(crate) use journal::ReservationScope;
pub(crate) use journal::ReservationScopeAdditionSet;
pub(crate) use journal::ReservationScopeSet;
pub(crate) use journal::ReservationSnapshot;
pub(crate) use journal::ScopeKind;
pub(crate) use journal::SkippedDeferral;
pub(crate) use journal::SkippedIntegrationHoldSet;
pub(crate) use journal::SkippedOrderingEdge;
#[cfg(test)]
pub(crate) use journal::TrunkCommitAtClaim;
pub(crate) use journal::TrunkObservationAtClaim;
pub(crate) use journal::WidenCause;
pub(crate) use journal::WorkPlanReference;
pub(crate) use journal::WorktreeAdministrativeLocator;
pub(crate) use path::AncestorCanonicalizationError;
pub(crate) use path::canonicalize_through_nearest_existing_ancestor;
pub(crate) use path::normalize_absolute_path;
pub(crate) use worktree_context::RegisteredWorktreeAvailability;
pub(crate) use worktree_context::WorktreeContext;
