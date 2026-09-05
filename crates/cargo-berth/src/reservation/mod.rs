//! Reservation state derived solely from append-only journal events.
//!
//! Each submodule owns one type cluster and the work that belongs to it: the retained set a
//! replay produces and the incursion incidents recorded against it, the per-holder record
//! and the readings taken from it, the foreign holder that blocks a requested scope, how
//! retained reservations partition into the caller's own work and foreign holds, the durable
//! scoped patch verdicts and the schedule that orders them, observed integration evidence,
//! the lifecycle and its transitions, and the faults that stop a replay. This root declares
//! them and re-exports the names the rest of the crate uses.

mod conflict;
mod constants;
mod evidence;
mod lifecycle;
mod partition;
mod record;
mod replay;
mod retention;
mod scoped_patch_evaluation;

pub(crate) use conflict::ReservationConflict;
pub(crate) use evidence::DeferredScopedPatchIntegrationStatus;
pub(crate) use evidence::IntegrationEvidenceObservation;
pub(crate) use evidence::PriorIntegrationStatus;
pub(crate) use evidence::ProtectedReservationTip;
pub(crate) use evidence::ScopedPatchComparisonObservation;
pub(crate) use evidence::current_head;
pub(crate) use evidence::current_trunk;
pub(crate) use evidence::integration_status;
pub(crate) use evidence::observe_integration_status;
pub(crate) use evidence::observe_outstanding_integration_status;
pub(crate) use evidence::outstanding_integration_status;
pub(crate) use evidence::retain_protected_tip;
pub(crate) use lifecycle::AbandonmentReason;
pub(crate) use lifecycle::EditBlockingStatus;
pub(crate) use lifecycle::IntegrationEvidenceStatus;
pub(crate) use lifecycle::IntegrationProof;
pub(crate) use lifecycle::LifecycleTransitionError;
pub(crate) use lifecycle::OrphanRetirementReason;
pub(crate) use lifecycle::ReleaseDisposition;
pub(crate) use lifecycle::ReleaseRevalidationSubject;
pub(crate) use lifecycle::ReservationLifecycle;
pub(crate) use lifecycle::RewrittenIntegrationTrunkCommit;
pub(crate) use partition::AuthorizedEditingIdentity;
pub(crate) use partition::DriftBlockingCoverage;
pub(crate) use partition::WidenScopeBinding;
pub(crate) use record::Reservation;
pub(crate) use record::ReservationEvidenceState;
pub(crate) use record::ReservationFreshness;
pub(crate) use record::ReservationLifecycleSnapshot;
pub(crate) use replay::ReservationReplayError;
pub(crate) use retention::IncursionIncident;
pub(crate) use retention::IncursionIncidentStatus;
pub(crate) use retention::IncursionObservation;
pub(crate) use retention::RetainedReservationSet;
pub(crate) use retention::WorktreeOccupancy;
pub(crate) use scoped_patch_evaluation::DurableScopedPatchComparison;
pub(crate) use scoped_patch_evaluation::IntegrationProofSubjectRevision;
pub(crate) use scoped_patch_evaluation::ScopedPatchEquivalenceVerdict;
pub(crate) use scoped_patch_evaluation::ScopedPatchEvaluationPriority;
pub(crate) use scoped_patch_evaluation::ScopedPatchTargetVerdictAvailability;
pub(crate) use scoped_patch_evaluation::SuccessorScopedPatchEquivalenceVerdict;
pub(crate) use scoped_patch_evaluation::SuccessorScopedPatchTargetVerdictAvailability;
