//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response retains the original six-field envelope and adds one
//! typed `payload` field. Consumers can continue reading the original fields,
//! while newer consumers use `payload` instead of scraping `message`.

use std::fmt::Write as _;
use std::path::Path;

use serde::Deserialize;
use serde::Serialize;

use crate::alert::Alert;
use crate::answer::OverlapEscalationPayload;
use crate::answer::PermissiveOverlapAnswer;
use crate::board::BoardModel;
use crate::config::InitializationState;
use crate::drift::DriftEffect;
use crate::drift::DriftPathAttributionOutcome;
use crate::drift::DriftReport;
use crate::drift::IncursionCommit;
use crate::drift::IncursionCommitOrigin;
use crate::drift::PostWriteFreePathProtection;
use crate::drift::ReservationDriftResult;
use crate::edge::EdgeDeclarationRejection;
use crate::edge::EdgeHold;
use crate::edge::EdgeReadiness;
use crate::edge::IntegrationHold;
use crate::edge::OrderingEdge;
use crate::edge::UnintegratedPredecessorEvidence;
use crate::exit::BerthExit;
use crate::gate::IntegrationViolation;
use crate::gate::install::ActiveManagedHookInstallation;
use crate::gate::install::ManagedHookActivationOutcome;
use crate::gate::install::ManagedHookInstallation;
use crate::ids::CoordinationRunId;
use crate::ids::EventId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger::ClaimSource;
use crate::ledger::IncursionIncidentId;
use crate::ledger::LedgerError;
use crate::ledger::LedgerInitialization;
use crate::ledger::OrderingDirection;
use crate::ledger::ReservationPurpose;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;
use crate::session::SessionIdentityMappingPublication;
use crate::verb::claim::FirstTouchReservationAcquisition;
use crate::verb::claim::FirstTouchReservationAcquisitionKind;

const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.";
const PROJECTION_REPAIRED_MESSAGE: &str =
    "Rebuilt reservations.json from journal truth without changing the journal.";
const BOARD_READY_MESSAGE: &str =
    "The reservation board was read. Use `cargo-berth board --json` to inspect it.";
#[cfg(test)]
const UNIMPLEMENTED_MESSAGE: &str = "The reservation engine is not implemented.";

/// One response from a `cargo-berth` verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OutputEnvelope {
    /// The verb that produced this response.
    verb:                 CommandVerb,
    /// The response's lifecycle status.
    status:               OutputStatus,
    /// The process exit status for this response.
    pub(crate) exit_code: BerthExit,
    /// Reservations relevant to this response.
    reservations:         Vec<ReservationId>,
    /// Reservations that block this response.
    blocked_by:           Vec<ReservationId>,
    /// A human-readable explanation of this response.
    message:              String,
    /// The verb-keyed facts consumers need without parsing prose.
    payload:              OutputPayload,
}

/// A verb named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CommandVerb {
    /// Initialize the shared ledger.
    Init,
    /// Show the reservation board.
    Board,
    /// Check a proposed path footprint.
    Check,
    /// Claim paths for a reservation.
    Claim,
    /// Compare observed changes with one or more active reservations.
    Drift,
    /// Release a reservation at a checkpoint.
    Release,
    /// Record an ordering relationship.
    Sequence,
    /// Integrate a reservation into trunk.
    Integrate,
    /// Resolve a stuck reservation after inspecting its condition.
    Resolve,
    /// Renew a reservation's explicit activity record.
    Renew,
}

/// Whether the post-commit hook should stay silent or print a warning.
pub(crate) enum PostCommitRendering {
    /// The full comparison found nothing the hook needs to report.
    Silent,
    /// The hook must print this diagnostic while leaving the commit standing.
    Warning(String),
}

/// The status named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum OutputStatus {
    /// The verb parsed, but no engine stands behind it yet.
    Unimplemented,
    /// The headless board was projected from reconciled journal and repository facts.
    BoardReady,
    /// Initialization created or verified the durable coordination resources.
    Initialized,
    /// Explicit repair rebuilt only the disposable journal projection.
    ProjectionRepaired,
    /// Confirmed reinitialization discarded the reviewed journal state.
    Reinitialized,
    /// The journal or its projection could not be safely read or published.
    LedgerUnreadable,
    /// This repository has no berth configuration, so it is not participating in coordination.
    Unconfigured,
    /// The board was handed a terminal and the terminal failed.
    TerminalViewFailed,
    /// An overlap-free edit check may proceed.
    Clear,
    /// A new reservation was appended and published.
    Claimed,
    /// Unreserved changed paths were added to a reservation.
    Widened,
    /// A write entered a foreign edit-blocking reservation.
    Incursion,
    /// A widening gained a foreign blocker before its lock was acquired.
    DriftCollision,
    /// Unclaimed paths require an explicit reservation attribution.
    DriftAttributionRequired,
    /// Repository policy permits no additional live reservations.
    ReservationLimitReached,
    /// Repository policy permits no additional ordering edges.
    OrderingEdgeLimitReached,
    /// One or more foreign reservations overlap the requested paths.
    BlockedByOverlap,
    /// One or more ordering or deferral holds reject integration.
    BlockedByOrdering,
    /// A permissive overlap answer needs a matching reviewed proposal.
    NeedsUserAuthorization,
    /// The caller can correct the request and retry without repairing the ledger.
    InvalidInput,
    /// Another mutation retained the ledger lock through the retry window.
    Contention,
    /// A deferral was converted into one durable ordering edge.
    Sequenced,
    /// The requested directed edge already exists.
    DuplicateOrderingEdge,
    /// The requested directed edge would make the graph cyclic.
    OrderingCycle,
    /// The named reservations have no unresolved deferral to order.
    MissingDeferral,
    /// The reservation now has a protected checkpoint awaiting integration.
    Outstanding,
    /// Current trunk contains the reservation's integration evidence.
    Integrated,
    /// Current trunk no longer contains previously verified evidence.
    TrunkRewritten,
    /// Git could not resolve an object needed to verify integration.
    ObjectUnknown,
    /// A user-confirmed non-integration disposition ended the reservation.
    Released,
    /// A replacement worktree now owns surviving reservation work.
    Recovered,
    /// A still-live reservation recorded recent activity.
    Renewed,
    /// A user disposition answered one outstanding incursion incident.
    IncursionResolved,
}

/// Structured facts and additive alerts returned inside the typed payload field.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct OutputPayload {
    /// The verb-keyed result whose serialized `kind` and `data` layout is stable.
    #[serde(flatten)]
    facts:  OutputFacts,
    /// Durable coordination alerts relevant to this response.
    #[serde(default)]
    alerts: Vec<Alert>,
}

/// Structured facts that correspond to the response's verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
enum OutputFacts {
    /// The operation failed before it could establish any durable facts.
    NoFacts,
    /// Facts returned by `init`.
    Init(InitializationPayload),
    /// Facts returned by `init --repair-projection`.
    ProjectionRepair(ProjectionRepairPayload),
    /// Facts returned by confirmed journal reinitialization.
    Reinitialize(ReinitializationPayload),
    /// Facts returned by the headless reservation board.
    Board(Box<BoardModel>),
    /// Facts returned by `check`.
    Check(CheckPayload),
    /// Facts returned by `claim`.
    Claim(ClaimPayload),
    /// Facts returned by `drift`.
    Drift(DriftReport),
    /// Facts returned by `release`.
    Release(ReleasePayload),
    /// Facts returned by `sequence`.
    Sequence(SequencePayload),
    /// Facts returned by `integrate`.
    Integrate(IntegrationPayload),
    /// Facts returned by a recovery decision.
    Resolve(ResolvePayload),
    /// Facts returned by a renewal.
    Renew(RenewPayload),
}

/// The resources an `init` call created or left intact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct InitializationPayload {
    /// Whether initialization created the journal or found an existing one.
    ledger:        InitializationResource,
    /// Whether initialization created the config or left an existing file intact.
    configuration: InitializationResource,
    /// Whether every registered managed hook is now in force.
    hooks:         Vec<InitializedManagedHook>,
}

/// The activation result for one hook in the managed-hook registry.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct InitializedManagedHook {
    /// The git hook name from the managed-hook registry.
    name:       String,
    /// Whether the hook will run and how initialization reached that state.
    activation: ManagedHookActivation,
}

/// Whether one managed hook will run after initialization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ManagedHookActivation {
    /// The managed hook is installed and executable.
    Active {
        /// Whether this call installed or retained the managed script.
        installation: ActiveHookInstallation,
    },
    /// The managed hook is not in force.
    Inactive {
        /// Why initialization could not activate this hook.
        reason: ManagedHookInactivity,
    },
}

/// How an active managed hook reached its current state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ActiveHookInstallation {
    /// This initialization call created the hook.
    Installed,
    /// This initialization call retained or refreshed the managed hook.
    Current,
}

/// Why a managed hook is not in force after initialization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ManagedHookInactivity {
    /// An unrelated hook still owns the hook name.
    PreservedUnmanaged,
    /// Filesystem or git access prevented hook installation.
    InstallationFailed {
        /// The error returned while installing this hook.
        diagnostic: String,
    },
}

/// The explicit guarantee reported after rebuilding the disposable projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ProjectionRepairPayload {
    /// The only file this operation rebuilt.
    projection: RepairedProjection,
    /// The journal mutation guarantee of explicit projection repair.
    journal:    ProjectionRepairJournalEffect,
}

/// The exact destructive effect of confirmed ledger reinitialization.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ReinitializationPayload {
    /// The journal bytes discarded after confirmation.
    discarded_bytes:              u64,
    /// The newline-terminated records present before truncation.
    discarded_complete_records:   u64,
    /// Environment bypass markers retained outside the unreadable journal.
    pending_environment_bypasses: u64,
}

/// The disposable projection rebuilt by explicit repair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RepairedProjection {
    /// `reservations.json` was derived again from complete journal facts.
    ReservationsJsonRebuilt,
}

/// Whether explicit projection repair changed journal truth.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ProjectionRepairJournalEffect {
    /// `journal.ndjson` remained byte-identical.
    Unchanged,
}

/// The initialization outcome for one durable resource.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum InitializationResource {
    /// This initialization call created the resource.
    Created,
    /// This initialization call retained an existing resource unchanged.
    Existing,
}

/// Typed outcomes returned by the trunk integration gate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum IntegrationPayload {
    /// The selected reservation entered trunk after a clear decision.
    Integrated {
        /// The reservation whose protected work entered trunk.
        reservation_id: ReservationId,
        /// The main object against which the update was validated.
        previous:       GitObjectId,
        /// The new main object installed by the update.
        proposed:       GitObjectId,
        /// The journal generation validated under the decision lock.
        generation:     ProjectionGeneration,
        /// How gate policy treated the update.
        gate:           IntegratedGateOutcome,
    },
    /// Enforcing policy refused an out-of-order update.
    Blocked {
        /// The reservation the caller asked to integrate.
        reservation_id: ReservationId,
        /// The journal generation validated under the decision lock.
        generation:     ProjectionGeneration,
        /// Every exact hold that prevented integration.
        violations:     Vec<IntegrationViolation>,
    },
    /// Caller identity named a coordination run that no longer owns active work.
    Rejected {
        /// The semantic reason integration could not select active work.
        reason: IntegrationRejectionKind,
    },
}

/// A stable inactive-identity rejection returned by `integrate`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IntegrationRejectionKind {
    /// A harness session mapping no longer identifies its exact active reservation.
    InactiveSessionMapping {
        /// The stale coordination run named by that mapping.
        coordination_run_id: CoordinationRunId,
    },
    /// A marker no longer identifies active work in the invoking worktree.
    InactiveMarkerRun {
        /// The stale coordination run named by that marker.
        coordination_run_id: CoordinationRunId,
    },
}

/// How a successful integration related to current gate policy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum IntegratedGateOutcome {
    /// No integration constraint held the reservation.
    Clear,
    /// Observe-only policy logged holds that enforcing mode would reject.
    Observed {
        /// The holds reported without rejecting the update.
        violations: Vec<IntegrationViolation>,
    },
    /// A one-use permit was minted and consumed by the update.
    Forced {
        /// The durable permit identity.
        permit_id:           ForcedIntegrationPermitId,
        /// The exact holds the user chose to skip.
        skipped_holds:       SkippedIntegrationHoldSet,
        /// Holds on other entering reservations reported by observe-only policy.
        observed_violations: Vec<IntegrationViolation>,
    },
}

/// Typed outcomes returned by `resolve`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ResolvePayload {
    /// A user disposition answered an outstanding incursion incident.
    IncursionResolved {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident answered by the appended disposition.
        incident_id:    IncursionIncidentId,
    },
    /// This invocation appended the requested incursion disposition.
    RecordedNow {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident answered by this invocation.
        incident_id:    IncursionIncidentId,
    },
    /// This worktree coordination run had already appended the disposition.
    AlreadyRecordedBySameCoordinationActor {
        /// The reservation whose drift produced the incident.
        reservation_id: ReservationId,
        /// The incident already answered by this coordination actor.
        incident_id:    IncursionIncidentId,
    },
    /// Another worktree coordination run had already appended the disposition.
    AlreadyRecordedByDifferentCoordinationActor {
        /// The reservation whose drift produced the incident.
        reservation_id:                ReservationId,
        /// The incident already answered by another coordination actor.
        incident_id:                   IncursionIncidentId,
        /// The worktree identity recorded on the resolution event.
        resolving_worktree_id:         WorktreeId,
        /// The coordination run recorded on the resolution event.
        resolving_coordination_run_id: CoordinationRunId,
        /// The journal append that answered the incident.
        resolution_event_id:           EventId,
        /// When the disposition was recorded.
        resolved_at:                   RecordedAt,
    },
    /// A user disposition answered every incident outstanding for one reservation.
    EveryIncursionResolved {
        /// The reservation whose drift produced the incidents.
        reservation_id: ReservationId,
        /// Every incident answered by the appended dispositions.
        incident_ids:   Vec<IncursionIncidentId>,
    },
    /// Surviving work moved to a replacement worktree identity.
    Recovered {
        /// The reservation whose holder changed.
        reservation_id: ReservationId,
        /// The opaque identity of the replacement worktree.
        worktree_id:    WorktreeId,
    },
    /// A user-confirmed terminal disposition resolved the reservation.
    Released {
        /// The reservation that received the disposition.
        reservation_id:              ReservationId,
        /// The recorded disposition or replacement disposition.
        disposition:                 ReleaseDisposition,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
}

/// Typed facts returned by `renew`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RenewPayload {
    /// The reservation whose activity timestamp advanced.
    reservation_id: ReservationId,
}

/// Typed outcomes returned by `claim`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ClaimPayload {
    /// A reservation was appended with this minimal antichain.
    Claimed {
        /// The newly minted reservation identity.
        reservation_id:              ReservationId,
        /// The coordination run that owns the appended reservation.
        coordination_run_id:         CoordinationRunId,
        /// The exact durable footprint.
        scopes:                      ReservationScopeSet,
        /// Whether the worktree marker records `coordination_run_id`.
        marker_publication:          CoordinationRunMarkerPublication,
        /// Whether the harness session mapping reflects this claim.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// Foreign holders prevented the append.
    Blocked {
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
    /// A permissive answer was proposed but has not supplied the current exact token.
    NeedsUserAuthorization {
        /// The conflicts, proposed answer, reason, consequence, and proposal token.
        #[serde(flatten)]
        escalation: Box<OverlapEscalationPayload>,
    },
    /// Repository policy rejected another live reservation.
    ReservationLimitReached {
        /// The configured maximum number of nonterminal reservations.
        maximum: u32,
    },
    /// Repository policy rejected another claim-time ordering edge.
    OrderingEdgeLimitReached {
        /// The configured maximum number of durable ordering edges.
        maximum: u32,
    },
}

/// Typed outcomes returned by `sequence`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SequencePayload {
    /// One durable edge was appended by resolving a prior deferral.
    Sequenced {
        /// The complete replayable edge record.
        edge:      OrderingEdge,
        /// The edge state derived from the preceding repository snapshot.
        readiness: EdgeReadiness,
    },
    /// The locked graph rejected the requested relationship.
    Rejected {
        /// The requested predecessor.
        first:  ReservationId,
        /// The requested successor.
        then:   ReservationId,
        /// The semantic reason no edge was appended.
        reason: SequenceRejectionKind,
    },
}

/// A stable semantic rejection returned by `sequence`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SequenceRejectionKind {
    /// At least one endpoint does not name a retained reservation.
    UnknownEndpoint {
        /// The missing reservation.
        reservation_id: ReservationId,
    },
    /// One reservation was supplied as both endpoints.
    SameEndpoint,
    /// The exact directed edge already exists.
    Duplicate,
    /// The proposed edge would create a directed cycle.
    Cycle,
    /// No unresolved defer answer joins the endpoints.
    MissingDeferral,
    /// Both endpoint directions contain defer answers.
    AmbiguousDeferral,
    /// Repository policy permits no additional ordering edge.
    OrderingEdgeLimitReached {
        /// The configured durable edge maximum.
        maximum: u32,
    },
    /// A harness session mapping no longer identifies its exact active reservation.
    InactiveSessionMapping {
        /// The stale coordination run named by that mapping.
        coordination_run_id: CoordinationRunId,
    },
    /// A marker no longer identifies active work in the invoking worktree.
    InactiveMarkerRun {
        /// The stale coordination run named by that marker.
        coordination_run_id: CoordinationRunId,
    },
}

impl From<EdgeDeclarationRejection> for SequenceRejectionKind {
    fn from(rejection: EdgeDeclarationRejection) -> Self {
        match rejection {
            EdgeDeclarationRejection::UnknownEndpoint(reservation_id) => {
                Self::UnknownEndpoint { reservation_id }
            },
            EdgeDeclarationRejection::SameEndpoint => Self::SameEndpoint,
            EdgeDeclarationRejection::Duplicate => Self::Duplicate,
            EdgeDeclarationRejection::Cycle => Self::Cycle,
            EdgeDeclarationRejection::MissingDeferral => Self::MissingDeferral,
            EdgeDeclarationRejection::AmbiguousDeferral => Self::AmbiguousDeferral,
        }
    }
}

impl SequenceRejectionKind {
    fn blocked_by(&self, first: ReservationId, then: ReservationId) -> Vec<ReservationId> {
        match self {
            Self::Duplicate => vec![first],
            Self::Cycle => vec![then],
            Self::UnknownEndpoint { .. }
            | Self::SameEndpoint
            | Self::MissingDeferral
            | Self::AmbiguousDeferral
            | Self::OrderingEdgeLimitReached { .. }
            | Self::InactiveSessionMapping { .. }
            | Self::InactiveMarkerRun { .. } => Vec::new(),
        }
    }

    fn response(
        &self,
        first: ReservationId,
        then: ReservationId,
    ) -> (OutputStatus, BerthExit, String) {
        match self {
            Self::Duplicate => (
                OutputStatus::DuplicateOrderingEdge,
                BerthExit::BlockedByOrdering,
                format!("Ordering edge {first} before {then} already exists."),
            ),
            Self::Cycle => (
                OutputStatus::OrderingCycle,
                BerthExit::BlockedByOrdering,
                format!("Ordering edge {first} before {then} would create a cycle."),
            ),
            Self::MissingDeferral => (
                OutputStatus::MissingDeferral,
                BerthExit::BlockedByOrdering,
                format!(
                    "Reservations {first} and {then} have no unresolved defer answer to sequence."
                ),
            ),
            Self::OrderingEdgeLimitReached { maximum } => (
                OutputStatus::OrderingEdgeLimitReached,
                BerthExit::BlockedByOrdering,
                format!("The configured maximum of {maximum} ordering edges has been reached."),
            ),
            Self::UnknownEndpoint { reservation_id } => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!("Reservation {reservation_id} does not exist."),
            ),
            Self::SameEndpoint => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                "An ordering edge requires two different reservations.".to_owned(),
            ),
            Self::AmbiguousDeferral => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!("Reservations {first} and {then} recorded deferrals in both directions."),
            ),
            Self::InactiveSessionMapping {
                coordination_run_id,
            } => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!(
                    "Harness session mapping for coordination run {coordination_run_id} no longer names an active reservation."
                ),
            ),
            Self::InactiveMarkerRun {
                coordination_run_id,
            } => (
                OutputStatus::InvalidInput,
                BerthExit::UsageError,
                format!(
                    "Coordination-run marker {coordination_run_id} no longer has an active reservation."
                ),
            ),
        }
    }
}

/// Whether the successful claim also published its worktree run marker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerPublication {
    /// The marker now identifies the run that owns the appended claim.
    Published,
    /// The claim is durable, but the marker could not be updated.
    Unavailable {
        /// The marker publication failure.
        diagnostic: String,
    },
}

/// Typed outcomes returned by `check`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CheckPayload {
    /// No foreign live reservation overlaps the requested paths.
    Clear {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes:      ReservationScopeSet,
        /// The complete first-touch result that permits the edit.
        acquisition: FirstTouchReservationAcquisition,
    },
    /// Foreign holders block one or more requested paths.
    Blocked {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes:    ReservationScopeSet,
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
}

/// Typed state transitions and evidence results returned by `release`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReleasePayload {
    /// An active reservation recorded its first protected checkpoint.
    Checkpointed {
        /// The reservation that changed state.
        reservation_id:              ReservationId,
        /// The fixed commit retained for integration checks.
        protected_tip:               ProtectedReservationTip,
        /// The trunk commit observed at checkpoint.
        trunk_oid:                   GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:                      CoordinationRunMarkerRetirement,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
    /// A rebased outstanding reservation replaced its protected checkpoint.
    Resnapshotted {
        /// The reservation that changed state.
        reservation_id: ReservationId,
        /// The replacement fixed commit.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed with the replacement.
        trunk_oid:      GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A point-in-time git result was appended for hook-safe replay.
    EvidenceRevalidated {
        /// The reservation whose evidence was checked.
        reservation_id: ReservationId,
        /// What current trunk proves.
        evidence:       IntegrationEvidenceStatus,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
    },
    /// A verified or user-confirmed disposition was appended.
    Released {
        /// The reservation that received the disposition.
        reservation_id:              ReservationId,
        /// The retained terminal disposition.
        disposition:                 ReleaseDisposition,
        /// What happened to the worktree coordination-run marker.
        marker:                      CoordinationRunMarkerRetirement,
        /// Whether the harness session mapping retired this reservation.
        session_mapping_publication: SessionIdentityMappingPublication,
    },
}

impl ReleasePayload {
    const fn reservation_id(&self) -> ReservationId {
        match self {
            Self::Checkpointed { reservation_id, .. }
            | Self::Resnapshotted { reservation_id, .. }
            | Self::EvidenceRevalidated { reservation_id, .. }
            | Self::Released { reservation_id, .. } => *reservation_id,
        }
    }

    const fn output_status(&self) -> OutputStatus {
        match self {
            Self::Checkpointed { .. } | Self::Resnapshotted { .. } => OutputStatus::Outstanding,
            Self::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::Integrated { .. } => OutputStatus::Integrated,
                IntegrationEvidenceStatus::NotIntegrated => OutputStatus::Outstanding,
                IntegrationEvidenceStatus::TrunkRewritten => OutputStatus::TrunkRewritten,
                IntegrationEvidenceStatus::ObjectUnknown => OutputStatus::ObjectUnknown,
            },
            Self::Released { disposition, .. } => match disposition {
                ReleaseDisposition::Integrated | ReleaseDisposition::RewrittenIntegration(_) => {
                    OutputStatus::Integrated
                },
                ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
                    OutputStatus::Released
                },
            },
        }
    }
}

/// The ordinary-release decision for the worktree coordination-run marker.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum CoordinationRunMarkerRetirement {
    /// The marker still named this run and was removed.
    Removed,
    /// No marker existed when release checked it.
    AlreadyAbsent,
    /// Another active reservation from this run still needs the marker.
    PreservedForActiveReservation,
    /// A newer run owns the marker.
    PreservedDifferentRun,
    /// The stateful check ran outside the reservation's holder worktree.
    PreservedDifferentWorktree,
    /// A malformed marker remains for phase-5 reconciliation.
    PreservedMalformed,
    /// The release fact is durable, but marker access failed.
    Unavailable {
        /// The marker filesystem diagnostic.
        diagnostic: String,
    },
}

impl OutputEnvelope {
    /// Build the response for a verb that has no engine behind it yet.
    #[cfg(test)]
    fn unimplemented(command_verb: CommandVerb) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::Unimplemented,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      UNIMPLEMENTED_MESSAGE.to_owned(),
            payload:      OutputPayload::pending(command_verb),
        }
    }

    /// Build a successful headless board response without requiring a terminal.
    pub(crate) fn board(board: BoardModel) -> Self {
        let reservations = board.reservation_ids();
        Self {
            verb: CommandVerb::Board,
            status: OutputStatus::BoardReady,
            exit_code: BerthExit::Clear,
            reservations,
            blocked_by: Vec::new(),
            message: BOARD_READY_MESSAGE.to_owned(),
            payload: OutputPayload::from_facts(OutputFacts::Board(Box::new(board))),
        }
    }

    /// Build a successful board response after the terminal view could not open.
    pub(crate) fn board_with_terminal_view_opening_failure(
        board: BoardModel,
        diagnostic: &str,
    ) -> Self {
        let mut output_envelope = Self::board(board);
        output_envelope
            .message
            .push_str("\nThe terminal view could not open: ");
        output_envelope.message.push_str(diagnostic);
        output_envelope
            .message
            .push_str(". Run `cargo-berth board --json` instead.");
        output_envelope
    }

    /// Build an internal-failure response after the terminal board was visible.
    pub(crate) fn terminal_view_failed_after_board_opened(diagnostic: &str) -> Self {
        Self {
            verb:         CommandVerb::Board,
            status:       OutputStatus::TerminalViewFailed,
            exit_code:    BerthExit::TerminalViewFailed,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!(
                "The terminal view failed after it opened: {diagnostic}. Run `cargo-berth board --json` instead."
            ),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build the successful response for completed initialization.
    pub(crate) fn initialized(
        initialization: LedgerInitialization,
        hook_installations: &[ManagedHookInstallation],
    ) -> Self {
        let hooks = hook_installations
            .iter()
            .map(InitializedManagedHook::from)
            .collect::<Vec<_>>();
        let message = initialization_message(&hooks);
        Self {
            verb: CommandVerb::Init,
            status: OutputStatus::Initialized,
            exit_code: BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Init(InitializationPayload {
                ledger: initialization.ledger.into(),
                configuration: initialization.configuration.into(),
                hooks,
            })),
        }
    }

    /// Build the successful response for an explicit projection-only repair.
    pub(crate) fn projection_repaired() -> Self {
        Self {
            verb:         CommandVerb::Init,
            status:       OutputStatus::ProjectionRepaired,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      PROJECTION_REPAIRED_MESSAGE.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::ProjectionRepair(
                ProjectionRepairPayload {
                    projection: RepairedProjection::ReservationsJsonRebuilt,
                    journal:    ProjectionRepairJournalEffect::Unchanged,
                },
            )),
        }
    }

    /// Build a successful trunk update after its locked gate decision.
    pub(crate) fn integrated(integration_payload: IntegrationPayload) -> Self {
        let IntegrationPayload::Integrated {
            reservation_id,
            gate,
            ..
        } = &integration_payload
        else {
            return Self::invalid_input(
                CommandVerb::Integrate,
                "an integrated response requires an integrated payload",
            );
        };
        let policy = match gate {
            IntegratedGateOutcome::Clear => "the ordering gate was clear",
            IntegratedGateOutcome::Observed { .. } => {
                "observe-only policy reported an ordering hold"
            },
            IntegratedGateOutcome::Forced { permit_id, .. } => {
                return Self {
                    verb:         CommandVerb::Integrate,
                    status:       OutputStatus::Integrated,
                    exit_code:    BerthExit::Clear,
                    reservations: vec![*reservation_id],
                    blocked_by:   Vec::new(),
                    message:      format!(
                        "Integrated reservation {reservation_id} using one-use permit {permit_id}."
                    ),
                    payload:      OutputPayload::from_facts(OutputFacts::Integrate(
                        integration_payload,
                    )),
                };
            },
        };
        Self {
            verb:         CommandVerb::Integrate,
            status:       OutputStatus::Integrated,
            exit_code:    BerthExit::Clear,
            reservations: vec![*reservation_id],
            blocked_by:   Vec::new(),
            message:      format!("Integrated reservation {reservation_id}; {policy}."),
            payload:      OutputPayload::from_facts(OutputFacts::Integrate(integration_payload)),
        }
    }

    /// Build an enforcing gate denial with complete reservation and recovery context.
    pub(crate) fn integration_blocked(
        reservation_id: ReservationId,
        generation: ProjectionGeneration,
        violations: Vec<IntegrationViolation>,
    ) -> Self {
        let blocked_by = integration_blockers(&violations);
        let message = integration_blocked_message(reservation_id, &violations);
        Self {
            verb: CommandVerb::Integrate,
            status: OutputStatus::BlockedByOrdering,
            exit_code: BerthExit::BlockedByOrdering,
            reservations: vec![reservation_id],
            blocked_by,
            message,
            payload: OutputPayload::from_facts(OutputFacts::Integrate(
                IntegrationPayload::Blocked {
                    reservation_id,
                    generation,
                    violations,
                },
            )),
        }
    }

    /// Build the result of confirmed journal reinitialization.
    pub(crate) fn reinitialized(
        discarded_bytes: u64,
        discarded_complete_records: u64,
        pending_environment_bypasses: u64,
    ) -> Self {
        Self {
            verb:         CommandVerb::Init,
            status:       OutputStatus::Reinitialized,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!(
                "Reinitialized cargo-berth after confirmed order review; discarded {discarded_bytes} journal bytes across {discarded_complete_records} complete record(s). {pending_environment_bypasses} environment bypass marker(s) remain reportable."
            ),
            payload:      OutputPayload::from_facts(OutputFacts::Reinitialize(
                ReinitializationPayload {
                    discarded_bytes,
                    discarded_complete_records,
                    pending_environment_bypasses,
                },
            )),
        }
    }

    /// Build a ledger-unreadable response without adding a new process outcome.
    pub(crate) fn ledger_unreadable(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::LedgerUnreadable,
            exit_code:    BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!("The reservation ledger could not be read: {diagnostic}"),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a response for a repository that is not participating in coordination.
    pub(crate) fn unconfigured(
        command_verb: CommandVerb,
        expected_configuration_path: &Path,
    ) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::Unconfigured,
            exit_code:    BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!(
                "this repository has no cargo-berth configuration at {}; run `cargo-berth init` to create it",
                expected_configuration_path.display()
            ),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Convert a ledger failure into the requesting verb's public response.
    pub(crate) fn ledger_error(command_verb: CommandVerb, error: &LedgerError) -> Self {
        Self::ledger_unreadable(command_verb, &error.to_string())
    }

    /// Build the successful result for one appended claim.
    pub(crate) fn claimed(
        reservation_id: ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes: ReservationScopeSet,
        marker_publication: CoordinationRunMarkerPublication,
        session_mapping_publication: SessionIdentityMappingPublication,
    ) -> Self {
        let scope_count = scopes.as_slice().len();
        let message = match (&marker_publication, &session_mapping_publication) {
            (
                CoordinationRunMarkerPublication::Published,
                SessionIdentityMappingPublication::Published,
            ) => {
                format!("Claimed {scope_count} reservation scope(s) as {reservation_id}.")
            },
            (
                CoordinationRunMarkerPublication::Unavailable { diagnostic },
                SessionIdentityMappingPublication::Published,
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the coordination-run marker could not be published: {diagnostic}. Restore coordination run {coordination_run_id} through the process environment before subsequent commands."
            ),
            (
                CoordinationRunMarkerPublication::Published,
                SessionIdentityMappingPublication::Unavailable { diagnostic },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the harness session mapping could not be published: {diagnostic}. Later session-keyed drift checks may require an explicit coordination run and reservation."
            ),
            (
                CoordinationRunMarkerPublication::Unavailable {
                    diagnostic: marker_diagnostic,
                },
                SessionIdentityMappingPublication::Unavailable {
                    diagnostic: session_diagnostic,
                },
            ) => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but neither fallback identity publication completed. Coordination-run marker: {marker_diagnostic}. Harness session mapping: {session_diagnostic}. Restore coordination run {coordination_run_id} through the process environment and name reservation {reservation_id} explicitly for later drift checks."
            ),
        };
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::Claimed,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
                session_mapping_publication,
            })),
        }
    }

    /// Build a complete drift result with status and process outcome in agreement.
    pub(crate) fn drift(report: DriftReport) -> Self {
        let has_incursion = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Incursion { .. })
            })
        });
        let has_collision = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Collision { .. })
            })
        });
        let has_widen = report.results.iter().any(|result| {
            result_has_effect(result, |effect| {
                matches!(effect, DriftEffect::Widened { .. })
            })
        });
        let has_unknown_phase_start = report.results.iter().any(|result| {
            matches!(
                result,
                ReservationDriftResult::PhaseStartObjectUnknown { .. }
            )
        });
        let status = if has_incursion
            || matches!(
                &report.path_attribution,
                DriftPathAttributionOutcome::IncursionDetected { .. }
            ) {
            OutputStatus::Incursion
        } else if has_collision {
            OutputStatus::DriftCollision
        } else if has_widen
            || matches!(
                &report.path_attribution,
                DriftPathAttributionOutcome::FirstTouchReserved { .. }
            )
        {
            OutputStatus::Widened
        } else if matches!(
            &report.path_attribution,
            DriftPathAttributionOutcome::Ambiguous { .. }
                | DriftPathAttributionOutcome::CoordinationRunRequired { .. }
        ) {
            OutputStatus::DriftAttributionRequired
        } else if has_unknown_phase_start {
            OutputStatus::ObjectUnknown
        } else {
            OutputStatus::Clear
        };
        let exit_code = if report.has_blocking_effect() {
            BerthExit::BlockedByOverlap
        } else {
            BerthExit::Clear
        };
        let reservations = report.reservation_ids();
        let blocked_by = report.blocking_reservation_ids();
        let message = drift_message(&report);
        Self {
            verb: CommandVerb::Drift,
            status,
            exit_code,
            reservations,
            blocked_by,
            message,
            payload: OutputPayload::from_facts(OutputFacts::Drift(report)),
        }
    }

    /// Build a typed rejection when no additional live reservation is permitted.
    pub(crate) fn reservation_limit_reached(maximum: u32) -> Self {
        Self {
            verb:         CommandVerb::Claim,
            status:       OutputStatus::ReservationLimitReached,
            exit_code:    BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!(
                "The configured maximum of {maximum} live reservations has been reached."
            ),
            payload:      OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::ReservationLimitReached { maximum },
            )),
        }
    }

    /// Build a typed claim rejection when no additional ordering edge is permitted.
    pub(crate) fn claim_ordering_edge_limit_reached(maximum: u32) -> Self {
        Self {
            verb:         CommandVerb::Claim,
            status:       OutputStatus::OrderingEdgeLimitReached,
            exit_code:    BerthExit::BlockedByOrdering,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      format!(
                "The configured maximum of {maximum} ordering edges has been reached."
            ),
            payload:      OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::OrderingEdgeLimitReached { maximum },
            )),
        }
    }

    /// Build the successful response for a deferral converted into an ordering edge.
    pub(crate) fn sequenced(edge: OrderingEdge, readiness: EdgeReadiness) -> Self {
        let edge_id = edge.edge_id;
        let before = edge.before;
        let after = edge.after;
        Self {
            verb:         CommandVerb::Sequence,
            status:       OutputStatus::Sequenced,
            exit_code:    BerthExit::Clear,
            reservations: vec![before, after],
            blocked_by:   if readiness.holds_successor() {
                vec![before]
            } else {
                Vec::new()
            },
            message:      format!("Recorded ordering edge {edge_id}: {before} before {after}."),
            payload:      OutputPayload::from_facts(OutputFacts::Sequence(
                SequencePayload::Sequenced { edge, readiness },
            )),
        }
    }

    /// Build a locked semantic rejection for a requested deferral resolution.
    pub(crate) fn sequence_rejected(
        first: ReservationId,
        then: ReservationId,
        reason: SequenceRejectionKind,
    ) -> Self {
        let (status, exit_code, message) = reason.response(first, then);
        let blocked_by = reason.blocked_by(first, then);
        Self {
            verb: CommandVerb::Sequence,
            status,
            exit_code,
            reservations: vec![first, then],
            blocked_by,
            message,
            payload: OutputPayload::from_facts(OutputFacts::Sequence(SequencePayload::Rejected {
                first,
                then,
                reason,
            })),
        }
    }

    /// Build an integration rejection that retains the inactive identity source.
    pub(crate) fn integration_rejected(
        reservation_id: ReservationId,
        reason: IntegrationRejectionKind,
        diagnostic: &str,
    ) -> Self {
        Self {
            verb:         CommandVerb::Integrate,
            status:       OutputStatus::InvalidInput,
            exit_code:    BerthExit::UsageError,
            reservations: vec![reservation_id],
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::Integrate(
                IntegrationPayload::Rejected { reason },
            )),
        }
    }

    /// Build a claim rejection that names every foreign holder.
    pub(crate) fn blocked_claim(conflicts: Vec<ReservationConflict>) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message: blocked_message(&conflicts),
            payload: OutputPayload::from_facts(OutputFacts::Claim(ClaimPayload::Blocked {
                conflicts,
            })),
        }
    }

    /// Build a claim response that requires a second invocation with the current token.
    pub(crate) fn claim_authorization_required(escalation: OverlapEscalationPayload) -> Self {
        let blocked_by = escalation
            .conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        let mut message = format!(
            "User authorization is required before this overlap can be recorded: {}. Review every holder, shared scope, plan, phase, direction, and reason in the payload, then rerun this claim with --proposal '{}'.",
            escalation.consequence, escalation.proposal_token
        );
        let direction = overlap_direction_description(&escalation.answer);
        let holder_material = escalation
            .conflicts
            .iter()
            .map(|conflict| {
                let shared_scopes = conflict
                    .overlapping_scopes
                    .as_slice()
                    .iter()
                    .map(|scope| {
                        let kind = match scope.kind {
                            ScopeKind::File => "file",
                            ScopeKind::Tree => "tree",
                        };
                        format!("{kind}:{}", scope.path)
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "Holder {}: {}; shared scopes: {}; direction: {}; reason: {}; consequence: {}.",
                    conflict.reservation_id,
                    source_description(&conflict.source),
                    shared_scopes,
                    direction,
                    escalation.authorization_reason,
                    escalation.consequence,
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !holder_material.is_empty() {
            message.push('\n');
            message.push_str(&holder_material);
        }
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::NeedsUserAuthorization,
            exit_code: BerthExit::NeedsUserAuthorization,
            reservations: Vec::new(),
            blocked_by,
            message,
            payload: OutputPayload::from_facts(OutputFacts::Claim(
                ClaimPayload::NeedsUserAuthorization {
                    escalation: Box::new(escalation),
                },
            )),
        }
    }

    /// Build a successful edit check whose locked transaction established protection.
    pub(crate) fn clear_check(
        scopes: ReservationScopeSet,
        acquisition: FirstTouchReservationAcquisition,
    ) -> Self {
        let message = match acquisition.kind {
            FirstTouchReservationAcquisitionKind::Appended => {
                "No foreign reservation overlaps the requested paths; a first-touch reservation was acquired."
            },
            FirstTouchReservationAcquisitionKind::Widened => {
                "No foreign reservation overlaps the requested paths; the acting run's first-touch reservation was widened."
            },
            FirstTouchReservationAcquisitionKind::AlreadyHeld => {
                "No foreign reservation overlaps the requested paths; the acting run already holds them."
            },
        };
        let reservation_id = acquisition.reservation_id;
        Self {
            verb:         CommandVerb::Check,
            status:       OutputStatus::Clear,
            exit_code:    BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by:   Vec::new(),
            message:      message.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Clear {
                scopes,
                acquisition,
            })),
        }
    }

    /// Build a blocked mutation-free edit check.
    pub(crate) fn blocked_check(
        scopes: ReservationScopeSet,
        conflicts: Vec<ReservationConflict>,
    ) -> Self {
        let blocked_by = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect();
        Self {
            verb: CommandVerb::Check,
            status: OutputStatus::BlockedByOverlap,
            exit_code: BerthExit::BlockedByOverlap,
            reservations: Vec::new(),
            blocked_by,
            message: blocked_message(&conflicts),
            payload: OutputPayload::from_facts(OutputFacts::Check(CheckPayload::Blocked {
                scopes,
                conflicts,
            })),
        }
    }

    /// Build a caller-correctable request rejection.
    pub(crate) fn invalid_input(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::InvalidInput,
            exit_code:    BerthExit::UsageError,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Convert a drift envelope into the commit hook's silent-or-warning behavior.
    pub(crate) fn post_commit_rendering(&self) -> PostCommitRendering {
        match self.status {
            OutputStatus::Clear | OutputStatus::Unconfigured => PostCommitRendering::Silent,
            OutputStatus::Widened | OutputStatus::Incursion | OutputStatus::DriftCollision => {
                PostCommitRendering::Warning(self.message.clone())
            },
            OutputStatus::LedgerUnreadable => PostCommitRendering::Warning(format!(
                "cargo-berth could not check this commit's drift because the ledger was unreadable. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
            OutputStatus::Contention => PostCommitRendering::Warning(format!(
                "cargo-berth could not check this commit's drift because the ledger lock deadline was exhausted. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
            _ => PostCommitRendering::Warning(format!(
                "cargo-berth could not complete the post-commit drift check. {} Run `cargo-berth drift --full` by hand; this commit remains in place.",
                self.message
            )),
        }
    }

    /// Build a bounded lock-contention result with retry guidance.
    pub(crate) fn contention(command_verb: CommandVerb, diagnostic: &str) -> Self {
        Self {
            verb:         command_verb,
            status:       OutputStatus::Contention,
            exit_code:    BerthExit::BlockedByContention,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::from_facts(OutputFacts::NoFacts),
        }
    }

    /// Build a successful release lifecycle or evidence response.
    pub(crate) fn released(release_payload: ReleasePayload) -> Self {
        let reservation_id = release_payload.reservation_id();
        let status = release_payload.output_status();
        let message = match &release_payload {
            ReleasePayload::Checkpointed {
                protected_tip,
                session_mapping_publication,
                ..
            } => message_with_session_mapping_publication(
                format!(
                    "Reservation {reservation_id} is outstanding at protected tip {protected_tip}."
                ),
                session_mapping_publication,
            ),
            ReleasePayload::Resnapshotted { protected_tip, .. } => {
                format!("Reservation {reservation_id} now retains protected tip {protected_tip}.")
            },
            ReleasePayload::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::NotIntegrated => format!(
                    "Reservation {reservation_id} remains outstanding; its protected tip is not in trunk."
                ),
                IntegrationEvidenceStatus::Integrated { trunk_oid, .. } => format!(
                    "Reservation {reservation_id} has integration evidence in trunk commit {trunk_oid}."
                ),
                IntegrationEvidenceStatus::TrunkRewritten => format!(
                    "Reservation {reservation_id} is blocking again because trunk no longer contains its verified evidence."
                ),
                IntegrationEvidenceStatus::ObjectUnknown => format!(
                    "Reservation {reservation_id} is blocking because git could not resolve its integration evidence."
                ),
            },
            ReleasePayload::Released {
                disposition,
                session_mapping_publication,
                ..
            } => message_with_session_mapping_publication(
                format!("Reservation {reservation_id} recorded disposition {disposition:?}."),
                session_mapping_publication,
            ),
        };
        Self {
            verb: CommandVerb::Release,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Release(release_payload)),
        }
    }

    /// Build a successful recovery response.
    pub(crate) fn resolved(resolve_payload: ResolvePayload) -> Self {
        let (reservation_id, status, message) = match &resolve_payload {
            ResolvePayload::IncursionResolved {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!("Incursion incident {incident_id} is resolved."),
            ),
            ResolvePayload::RecordedNow {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!("Incursion incident {incident_id} was recorded as resolved."),
            ),
            ResolvePayload::AlreadyRecordedBySameCoordinationActor {
                reservation_id,
                incident_id,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!(
                    "Incursion incident {incident_id} was already resolved by this worktree coordination run."
                ),
            ),
            ResolvePayload::AlreadyRecordedByDifferentCoordinationActor {
                reservation_id,
                incident_id,
                resolving_worktree_id,
                resolving_coordination_run_id,
                resolution_event_id,
                resolved_at,
            } => {
                return Self::incursion_resolution_recorded_by_different_actor(
                    *reservation_id,
                    *incident_id,
                    *resolving_worktree_id,
                    *resolving_coordination_run_id,
                    *resolution_event_id,
                    resolved_at.clone(),
                );
            },
            ResolvePayload::EveryIncursionResolved {
                reservation_id,
                incident_ids,
            } => (
                *reservation_id,
                OutputStatus::IncursionResolved,
                format!(
                    "Every incursion incident outstanding for reservation {reservation_id} is resolved: {}.",
                    incident_ids
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            ResolvePayload::Recovered {
                reservation_id,
                worktree_id,
            } => (
                *reservation_id,
                OutputStatus::Recovered,
                format!("Reservation {reservation_id} is recovered in worktree {worktree_id}."),
            ),
            ResolvePayload::Released {
                reservation_id,
                disposition,
                session_mapping_publication,
            } => (
                *reservation_id,
                match disposition {
                    ReleaseDisposition::Integrated
                    | ReleaseDisposition::RewrittenIntegration(_) => OutputStatus::Integrated,
                    ReleaseDisposition::Abandoned(_) | ReleaseDisposition::RetiredOrphan(_) => {
                        OutputStatus::Released
                    },
                },
                message_with_session_mapping_publication(
                    format!("Reservation {reservation_id} recorded disposition {disposition:?}."),
                    session_mapping_publication,
                ),
            ),
        };
        Self {
            verb: CommandVerb::Resolve,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::from_facts(OutputFacts::Resolve(resolve_payload)),
        }
    }

    /// Build a typed rejection for an incident resolved by another coordination actor.
    pub(crate) fn incursion_resolution_recorded_by_different_actor(
        reservation_id: ReservationId,
        incident_id: IncursionIncidentId,
        resolving_worktree_id: WorktreeId,
        resolving_coordination_run_id: CoordinationRunId,
        resolution_event_id: EventId,
        resolved_at: RecordedAt,
    ) -> Self {
        Self {
            verb:         CommandVerb::Resolve,
            status:       OutputStatus::InvalidInput,
            exit_code:    BerthExit::UsageError,
            reservations: vec![reservation_id],
            blocked_by:   Vec::new(),
            message:      format!(
                "Incursion incident {incident_id} was already resolved by worktree {resolving_worktree_id} in coordination run {resolving_coordination_run_id}, event {resolution_event_id} at {resolved_at}."
            ),
            payload:      OutputPayload::from_facts(OutputFacts::Resolve(
                ResolvePayload::AlreadyRecordedByDifferentCoordinationActor {
                    reservation_id,
                    incident_id,
                    resolving_worktree_id,
                    resolving_coordination_run_id,
                    resolution_event_id,
                    resolved_at,
                },
            )),
        }
    }

    /// Build a successful activity-renewal response.
    pub(crate) fn renewed(reservation_id: ReservationId) -> Self {
        Self {
            verb:         CommandVerb::Renew,
            status:       OutputStatus::Renewed,
            exit_code:    BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by:   Vec::new(),
            message:      format!("Reservation {reservation_id} activity was renewed."),
            payload:      OutputPayload::from_facts(OutputFacts::Renew(RenewPayload {
                reservation_id,
            })),
        }
    }

    /// Attach alerts derived by the reconciliation that preceded this command.
    pub(crate) fn with_alerts(mut self, alerts: Vec<Alert>) -> Self {
        self.payload.alerts = alerts;
        self
    }

    /// Render the primary result followed by every durable alert as its own line.
    pub(crate) fn render_text(&self) -> String {
        let mut rendered = self.message.clone();
        if let OutputFacts::Board(board) = &self.payload.facts {
            for marker_name in board.recovered_bypass_marker_names() {
                let _ = write!(
                    rendered,
                    "\nRecovered bypass marker {marker_name}: a bypass recorded earlier while the journal was unwritable has now been filed in the journal."
                );
            }
        }
        for alert in &self.payload.alerts {
            rendered.push('\n');
            rendered.push_str(&alert.to_string());
        }
        rendered
    }
}

impl OutputPayload {
    const fn from_facts(facts: OutputFacts) -> Self {
        Self {
            facts,
            alerts: Vec::new(),
        }
    }

    #[cfg(test)]
    const fn pending(command_verb: CommandVerb) -> Self {
        let facts = match command_verb {
            CommandVerb::Board
            | CommandVerb::Init
            | CommandVerb::Check
            | CommandVerb::Claim
            | CommandVerb::Drift
            | CommandVerb::Release
            | CommandVerb::Sequence
            | CommandVerb::Resolve
            | CommandVerb::Renew
            | CommandVerb::Integrate => OutputFacts::NoFacts,
        };
        Self::from_facts(facts)
    }
}

fn blocked_message(conflicts: &[ReservationConflict]) -> String {
    let mut message = overlap_holder_description(conflicts);
    if let Some(disposition) = first_touch_disposition_description(conflicts) {
        message.push(' ');
        message.push_str(&disposition);
    }
    message
}

/// Name the verbs that clear a first-touch holder, which no other message reaches.
fn first_touch_disposition_description(conflicts: &[ReservationConflict]) -> Option<String> {
    let first_touch_holders = conflicts
        .iter()
        .filter(|conflict| matches!(conflict.source, ClaimSource::FirstTouch))
        .map(|conflict| conflict.reservation_id.to_string())
        .collect::<Vec<_>>();
    match first_touch_holders.as_slice() {
        [] => None,
        [reservation_id] => Some(format!(
            "Reservation {reservation_id} came from a first-touch edit, so its holder clears it with cargo-berth release {reservation_id} once the work is on trunk, cargo-berth resolve {reservation_id} --integrated-as <TRUNK_OID> after that release when git cannot prove the integration, or cargo-berth resolve {reservation_id} --abandon --why <WHY> when the work is discarded."
        )),
        [_, _, ..] => Some(format!(
            "Reservations {} came from first-touch edits, so a holder clears one with cargo-berth release <RESERVATION_ID> once the work is on trunk, cargo-berth resolve <RESERVATION_ID> --integrated-as <TRUNK_OID> after that release when git cannot prove the integration, or cargo-berth resolve <RESERVATION_ID> --abandon --why <WHY> when the work is discarded.",
            first_touch_holders.join(", ")
        )),
    }
}

fn overlap_holder_description(conflicts: &[ReservationConflict]) -> String {
    match conflicts {
        [] => {
            "A foreign reservation overlaps the requested paths; reduce the requested scopes or coordinate with the holder, then retry."
                .to_owned()
        },
        [conflict] => {
            format!(
                "Reservation {} on {} ({}, {}) holds overlapping paths for {}; reduce the requested scopes or coordinate with the holder, then retry.",
                conflict.reservation_id,
                conflict.holder_branch(),
                source_description(&conflict.source),
                purpose_description(&conflict.purpose),
                conflict.holder_run_id,
            )
        },
        [_, _, ..] => {
            let holder_count = conflicts.len();
            let holders = conflicts
                .iter()
                .map(|conflict| {
                    format!(
                        "reservation {} on {} ({}, {}) for coordination run {}",
                        conflict.reservation_id,
                        conflict.holder_branch(),
                        source_description(&conflict.source),
                        purpose_description(&conflict.purpose),
                        conflict.holder_run_id,
                    )
                })
                .collect::<Vec<_>>()
                .join("; ");
            format!(
                "{holder_count} reservations hold overlapping paths: {holders}; reduce the requested scopes or coordinate with the holders, then retry.",
            )
        },
    }
}

fn message_with_session_mapping_publication(
    message: String,
    publication: &SessionIdentityMappingPublication,
) -> String {
    match publication {
        SessionIdentityMappingPublication::Published => message,
        SessionIdentityMappingPublication::Unavailable { diagnostic } => format!(
            "{message} The harness session mapping could not be published: {diagnostic}. Name the coordination run and reservation explicitly for later drift checks."
        ),
    }
}

fn result_has_effect(
    result: &ReservationDriftResult,
    matches_effect: impl Fn(&DriftEffect) -> bool,
) -> bool {
    match result {
        ReservationDriftResult::Unchanged { .. }
        | ReservationDriftResult::PhaseStartObjectUnknown { .. } => false,
        ReservationDriftResult::Changed { effects, .. } => {
            effects.as_slice().iter().any(matches_effect)
        },
    }
}

/// The abbreviated object-name length a reader can paste into a git command.
const SHORT_OBJECT_ID_CHARACTERS: usize = 8;

/// Name the commits behind an incursion's entered paths, or nothing when it has none.
///
/// A path that arrived on a commit and a path just written read identically otherwise,
/// so the reader cannot tell a false incursion from a real one without rebuilding the
/// phase range by hand.
fn render_incursion_commits(commits: &[IncursionCommit]) -> String {
    if commits.is_empty() {
        return String::new();
    }
    let rendered = commits
        .iter()
        .map(|commit| {
            format!(
                "{} \"{}\" ({}) covering {}",
                commit
                    .commit
                    .to_string()
                    .chars()
                    .take(SHORT_OBJECT_ID_CHARACTERS)
                    .collect::<String>(),
                commit.subject,
                match commit.origin {
                    IncursionCommitOrigin::PhaseAuthored => "this phase authored it",
                    IncursionCommitOrigin::AlreadyOnTrunk =>
                        "already on trunk, so this phase received it",
                    IncursionCommitOrigin::Unknown => "origin undetermined",
                },
                commit
                    .paths
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(" Committed by {rendered}.")
}

fn drift_message(report: &DriftReport) -> String {
    if !report.has_reportable_effect() {
        return if report.results.is_empty() {
            "No active reservation in this worktree required a drift check.".to_owned()
        } else {
            "No changed path fell outside the selected reservation coverage.".to_owned()
        };
    }
    let mut message = drift_path_attribution_message(&report.path_attribution);
    for result in &report.results {
        let (reservation_id, effects) = match result {
            ReservationDriftResult::Unchanged { .. } => continue,
            ReservationDriftResult::PhaseStartObjectUnknown {
                reservation_id,
                phase_start,
            } => {
                if !message.is_empty() {
                    message.push(' ');
                }
                let _ = write!(
                    message,
                    "Reservation {reservation_id} could not be compared because git could not read phase-start object {phase_start}. Restore that object before using this drift result."
                );
                continue;
            },
            ReservationDriftResult::Changed {
                reservation_id,
                effects,
            } => (reservation_id, effects),
        };
        for effect in effects.as_slice() {
            if !message.is_empty() {
                message.push(' ');
            }
            match effect {
                DriftEffect::Widened { added_scopes } => {
                    let rendered = added_scopes
                        .as_slice()
                        .iter()
                        .map(|scope| format!("file:{}", scope.path))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let _ = write!(
                        message,
                        "Widened reservation {reservation_id} to cover {rendered}."
                    );
                },
                DriftEffect::Incursion {
                    incident_id,
                    foreign_reservation_ids,
                    paths,
                    commits,
                } => {
                    let _ = write!(
                        message,
                        "Incursion {incident_id}: reservation {reservation_id} entered {} held by foreign reservation(s) {}.{} Stop and resolve the overlap with `resolve {reservation_id} --incursion {incident_id}` before making more changes. If no coordination run was identified before first-touch attribution, CARGO_BERTH_RUN can select an existing run for later invocations.",
                        paths
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        foreign_reservation_ids
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        render_incursion_commits(commits)
                    );
                },
                DriftEffect::Collision {
                    foreign_reservation_ids,
                    paths,
                } => {
                    let _ = write!(
                        message,
                        "Reservation {reservation_id} could not widen to {} because foreign reservation(s) {} acquired an edit-blocking overlap. Stop and resolve the collision.",
                        paths
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        foreign_reservation_ids
                            .as_slice()
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                },
            }
        }
    }
    message
}

fn drift_path_attribution_message(attribution: &DriftPathAttributionOutcome) -> String {
    match attribution {
        DriftPathAttributionOutcome::NotNeeded | DriftPathAttributionOutcome::Attributed { .. } => {
            String::new()
        },
        DriftPathAttributionOutcome::FirstTouchReserved { acquisition, .. } => {
            format!(
                "First-touch reservation {} now protects the changed paths.",
                acquisition.reservation_id
            )
        },
        DriftPathAttributionOutcome::IncursionDetected {
            paths,
            conflicts,
            protection,
        } => {
            let incursion = format!(
                "Post-write detection found changed paths {} inside foreign reservations {}. The write already happened; stop and resolve the incursion before making more changes.",
                paths
                    .as_slice()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
                conflicts
                    .iter()
                    .map(|conflict| conflict.reservation_id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            match protection {
                PostWriteFreePathProtection::NotAcquired => incursion,
                PostWriteFreePathProtection::Acquired {
                    acquisition,
                    scopes,
                } => format!(
                    "{incursion} First-touch reservation {} now protects the free paths {}. If no coordination run was identified before this observation, one was started; CARGO_BERTH_RUN can select an existing run for later invocations.",
                    acquisition.reservation_id,
                    scopes
                        .as_slice()
                        .iter()
                        .map(|scope| scope.path.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        },
        DriftPathAttributionOutcome::Ambiguous { candidates, paths } => format!(
            "Changed paths {} were not widened because attribution is ambiguous among reservations {}. Run drift --reservation <id> with one listed reservation.",
            paths
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            candidates
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        DriftPathAttributionOutcome::CoordinationRunRequired { paths } => format!(
            "Changed paths {} were not widened because no coordination run was identified. Set CARGO_BERTH_RUN to the run that owns the target reservation, then run drift --reservation <id>.",
            paths
                .as_slice()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn integration_blockers(violations: &[IntegrationViolation]) -> Vec<ReservationId> {
    let mut blockers = violations
        .iter()
        .flat_map(|violation| violation.blocking_reservations.iter())
        .map(|reservation| reservation.reservation_id)
        .collect::<Vec<_>>();
    blockers.sort_by_key(ToString::to_string);
    blockers.dedup();
    blockers
}

fn integration_blocked_message(
    reservation_id: ReservationId,
    violations: &[IntegrationViolation],
) -> String {
    let mut message = format!(
        "Reservation {reservation_id} cannot enter main while its integration order is held."
    );
    for violation in violations {
        let _ = write!(
            message,
            "\nEntering reservation {}: {}; purpose: {}; protected paths: {}.",
            violation.reservation.reservation_id,
            source_description(&violation.reservation.source),
            purpose_description(&violation.reservation.purpose),
            render_scopes(&violation.reservation.scopes),
        );
        for blocker in &violation.blocking_reservations {
            let _ = write!(
                message,
                "\nBlocking reservation {}: {}; purpose: {}; protected paths: {}.",
                blocker.reservation_id,
                source_description(&blocker.source),
                purpose_description(&blocker.purpose),
                render_scopes(&blocker.scopes),
            );
        }
        for hold in &violation.holds {
            message.push('\n');
            message.push_str(&integration_hold_message(
                violation.reservation.reservation_id,
                hold,
            ));
        }
    }
    let _ = write!(
        message,
        "\nTo deliberately proceed once: cargo-berth integrate {reservation_id} --force --why \"<reason>\". Last resort: CARGO_BERTH_BYPASS=1 <git command>."
    );
    message
}

fn integration_hold_message(subject: ReservationId, hold: &IntegrationHold) -> String {
    match hold {
        IntegrationHold::OrderingEdge {
            edge_id,
            predecessor,
            scopes,
            reason,
            readiness,
            ..
        } => {
            let recovery = match readiness {
                EdgeReadiness::Holding {
                    hold: EdgeHold::AwaitingPredecessorCheckpoint,
                } => format!("run cargo-berth release {predecessor} after checkpointing it"),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::NotIntegrated,
                        },
                } => format!("run cargo-berth integrate {predecessor}"),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::TrunkRewritten,
                        },
                } => format!(
                    "re-record verified evidence with cargo-berth resolve {predecessor} --integrated-as <trunk-oid>"
                ),
                EdgeReadiness::Holding {
                    hold:
                        EdgeHold::PredecessorNotOnTrunk {
                            evidence: UnintegratedPredecessorEvidence::ObjectUnknown,
                        },
                } => "repair the unresolvable git object, then rerun the integration".to_owned(),
                EdgeReadiness::Holding {
                    hold: EdgeHold::AwaitingSuccessorIncorporation,
                } => "rebase this worktree onto current main so it incorporates the predecessor"
                    .to_owned(),
                EdgeReadiness::Cancelled | EdgeReadiness::Fulfilled => {
                    "rerun the gate because this edge is no longer holding".to_owned()
                },
            };
            format!(
                "Ordering edge {edge_id} waits on reservation {predecessor}; covered paths: {}; recorded reason: {reason}; recovery: {recovery}.",
                render_scopes(scopes),
            )
        },
        IntegrationHold::DeferredOverlap {
            deferred,
            blocker,
            scopes,
            reason,
            ..
        } => {
            let counterpart = if *deferred == subject {
                *blocker
            } else {
                *deferred
            };
            format!(
                "Unresolved deferral with reservation {counterpart}; covered paths: {}; recorded reason: {reason}; recovery: cargo-berth sequence {counterpart} {subject} --why \"{}\".",
                render_scopes(scopes),
                shell_double_quoted(&reason.to_string()),
            )
        },
    }
}

fn render_scopes(scopes: &ReservationScopeSet) -> String {
    scopes
        .as_slice()
        .iter()
        .map(|scope| {
            let kind = match scope.kind {
                ScopeKind::File => "file",
                ScopeKind::Tree => "tree",
            };
            format!("{kind}:{}", scope.path)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn shell_double_quoted(value: &str) -> String { value.replace('\\', "\\\\").replace('"', "\\\"") }

fn source_description(claim_source: &ClaimSource) -> String {
    match claim_source {
        ClaimSource::WorkPlan { plan, phase } => format!("plan {plan}, phase {phase}"),
        ClaimSource::FirstTouch => "first-touch edit".to_owned(),
        ClaimSource::Explicit => "explicit claim".to_owned(),
    }
}

fn purpose_description(reservation_purpose: &ReservationPurpose) -> String {
    match reservation_purpose {
        ReservationPurpose::Explained(explanation) => explanation.to_string(),
        ReservationPurpose::NotProvidedByCaller => "no reason provided by caller".to_owned(),
    }
}

fn overlap_direction_description(answer: &PermissiveOverlapAnswer) -> String {
    match answer {
        PermissiveOverlapAnswer::Sequence { blocker, direction } => match direction {
            OrderingDirection::RequesterBeforeHolder => {
                format!("requester before holder {blocker}")
            },
            OrderingDirection::HolderBeforeRequester => {
                format!("holder {blocker} before requester")
            },
        },
        PermissiveOverlapAnswer::Defer { blocker } => {
            format!("none declared; deferred with holder {blocker}")
        },
        PermissiveOverlapAnswer::Override { blocker } => {
            format!("none declared; overridden with holder {blocker}")
        },
    }
}

impl From<InitializationState> for InitializationResource {
    fn from(initialization_state: InitializationState) -> Self {
        match initialization_state {
            InitializationState::Created => Self::Created,
            InitializationState::Existing => Self::Existing,
        }
    }
}

impl From<&ManagedHookInstallation> for InitializedManagedHook {
    fn from(installation: &ManagedHookInstallation) -> Self {
        Self {
            name:       installation.name().to_owned(),
            activation: ManagedHookActivation::from(installation.activation()),
        }
    }
}

impl From<&ManagedHookActivationOutcome> for ManagedHookActivation {
    fn from(activation: &ManagedHookActivationOutcome) -> Self {
        match activation {
            ManagedHookActivationOutcome::Active { installation } => Self::Active {
                installation: ActiveHookInstallation::from(*installation),
            },
            ManagedHookActivationOutcome::Inactive { reason } => Self::Inactive {
                reason: ManagedHookInactivity::from(reason),
            },
        }
    }
}

impl From<ActiveManagedHookInstallation> for ActiveHookInstallation {
    fn from(installation: ActiveManagedHookInstallation) -> Self {
        match installation {
            ActiveManagedHookInstallation::Installed => Self::Installed,
            ActiveManagedHookInstallation::Current => Self::Current,
        }
    }
}

impl From<&crate::gate::install::ManagedHookInactivity> for ManagedHookInactivity {
    fn from(reason: &crate::gate::install::ManagedHookInactivity) -> Self {
        match reason {
            crate::gate::install::ManagedHookInactivity::PreservedUnmanaged => {
                Self::PreservedUnmanaged
            },
            crate::gate::install::ManagedHookInactivity::InstallationFailed { diagnostic } => {
                Self::InstallationFailed {
                    diagnostic: diagnostic.clone(),
                }
            },
        }
    }
}

fn initialization_message(hooks: &[InitializedManagedHook]) -> String {
    let mut message = INITIALIZED_MESSAGE.to_owned();
    for hook in hooks {
        match &hook.activation {
            ManagedHookActivation::Active { .. } => {},
            ManagedHookActivation::Inactive {
                reason: ManagedHookInactivity::PreservedUnmanaged,
            } => {
                let _ = write!(
                    message,
                    " Hook '{}' is occupied by an unmanaged hook, so cargo-berth protection for that hook is not active. Incorporate the existing hook in a wrapper or move it aside, then rerun cargo berth init.",
                    hook.name
                );
            },
            ManagedHookActivation::Inactive {
                reason: ManagedHookInactivity::InstallationFailed { diagnostic },
            } => {
                let _ = write!(
                    message,
                    " Hook '{}' is not active because cargo-berth could not install it: {diagnostic}. Resolve the reported hook installation error, then rerun cargo berth init.",
                    hook.name
                );
            },
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::CommandVerb;
    use super::OutputEnvelope;
    use super::OutputStatus;
    use super::PostCommitRendering;
    use crate::config::ConfigError;
    use crate::config::InitializationState;
    use crate::ledger::LedgerError;
    use crate::ledger::LedgerInitialization;

    #[test]
    fn envelope_round_trips_with_its_additive_payload_field() {
        let output_envelope = OutputEnvelope::unimplemented(CommandVerb::Board);
        let serialized_envelope = serde_json::to_string(&output_envelope);

        assert!(
            serialized_envelope
                .as_ref()
                .is_ok_and(|serialized_envelope| serialized_envelope.contains("\"payload\""))
        );
        assert!(
            serialized_envelope
                .and_then(
                    |serialized_envelope| serde_json::from_str::<OutputEnvelope>(
                        &serialized_envelope
                    )
                )
                .is_ok_and(|round_tripped| round_tripped == output_envelope)
        );
    }

    #[test]
    fn drift_verb_uses_its_frozen_serde_spelling() {
        assert!(
            serde_json::to_string(&CommandVerb::Drift)
                .is_ok_and(|serialized| serialized == "\"drift\"")
        );
        assert_eq!(
            serde_json::from_str::<CommandVerb>("\"drift\"").ok(),
            Some(CommandVerb::Drift)
        );
    }

    #[test]
    fn init_has_a_non_placeholder_status() {
        let output_envelope = OutputEnvelope::initialized(
            LedgerInitialization {
                ledger:        InitializationState::Created,
                configuration: InitializationState::Existing,
            },
            &[],
        );

        assert_eq!(output_envelope.status, OutputStatus::Initialized);
        assert_eq!(output_envelope.exit_code, crate::exit::BerthExit::Clear);
    }

    #[test]
    fn failed_init_has_no_initialization_facts() {
        let output_envelope = OutputEnvelope::ledger_unreadable(CommandVerb::Init, "bad journal");

        assert_eq!(output_envelope.payload.facts, super::OutputFacts::NoFacts);
        assert!(
            serde_json::to_string(&output_envelope.payload).is_ok_and(|payload| !payload
                .contains("ledger")
                && !payload.contains("configuration"))
        );
    }

    #[test]
    fn unconfigured_and_unreadable_configuration_have_distinct_statuses_at_exit_four() {
        let expected_configuration_path = PathBuf::from(".claude/config/berth.toml");
        let malformed = LedgerError::Config(ConfigError::UnknownKey("porthole".to_owned()));

        let unconfigured =
            OutputEnvelope::unconfigured(CommandVerb::Drift, &expected_configuration_path);
        let ledger_unreadable = OutputEnvelope::ledger_error(CommandVerb::Drift, &malformed);

        assert_eq!(unconfigured.status, OutputStatus::Unconfigured);
        assert_eq!(ledger_unreadable.status, OutputStatus::LedgerUnreadable);
        assert_eq!(
            unconfigured.exit_code,
            crate::exit::BerthExit::LedgerUnreadable
        );
        assert_eq!(
            ledger_unreadable.exit_code,
            crate::exit::BerthExit::LedgerUnreadable
        );
        assert!(
            unconfigured
                .message
                .contains(&expected_configuration_path.display().to_string())
        );
    }

    #[test]
    fn ledger_error_keeps_its_prefix_for_a_malformed_configuration() {
        let malformed = LedgerError::Config(ConfigError::UnknownKey("porthole".to_owned()));

        let unreadable = OutputEnvelope::ledger_error(CommandVerb::Init, &malformed);

        assert_eq!(unreadable.status, OutputStatus::LedgerUnreadable);
        assert!(unreadable.message.ends_with(&malformed.to_string()));
        assert!(
            unreadable
                .message
                .contains("ledger configuration failed: unknown berth configuration key: porthole")
        );
    }

    #[test]
    fn unconfigured_post_commit_rendering_is_silent() {
        let output_envelope = OutputEnvelope::unconfigured(
            CommandVerb::Drift,
            &PathBuf::from(".claude/config/berth.toml"),
        );

        assert!(matches!(
            output_envelope.post_commit_rendering(),
            PostCommitRendering::Silent
        ));
    }

    #[test]
    fn terminal_view_failure_has_its_own_status_and_exit_code() {
        let output_envelope =
            OutputEnvelope::terminal_view_failed_after_board_opened("terminal disconnected");

        assert_eq!(output_envelope.status, OutputStatus::TerminalViewFailed);
        assert_eq!(
            output_envelope.exit_code,
            crate::exit::BerthExit::TerminalViewFailed
        );
        assert_eq!(output_envelope.payload.facts, super::OutputFacts::NoFacts);
        assert!(output_envelope.message.contains("terminal disconnected"));
        assert!(output_envelope.message.contains("cargo-berth board --json"));
    }
}
