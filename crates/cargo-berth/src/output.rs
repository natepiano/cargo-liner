//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response retains the original six-field envelope and adds one
//! typed `payload` field. Consumers can continue reading the original fields,
//! while newer consumers use `payload` instead of scraping `message`.

use serde::Deserialize;
use serde::Serialize;

use crate::config::InitializationState;
use crate::exit::BerthExit;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ledger::ClaimSource;
use crate::ledger::LedgerInitialization;
use crate::ledger::ReservationPurpose;
use crate::reservation::IntegrationEvidenceStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationConflict;
use crate::scope::ReservationScopeSet;

const INITIALIZED_MESSAGE: &str = "Initialized the cargo-berth ledger.";
const UNIMPLEMENTED_MESSAGE: &str = "The reservation engine is not implemented.";

/// One response from a `cargo-berth` verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OutputEnvelope {
    /// The verb that produced this response.
    pub(crate) verb:         CommandVerb,
    /// The response's lifecycle status.
    pub(crate) status:       OutputStatus,
    /// The process exit status for this response.
    pub(crate) exit_code:    BerthExit,
    /// Reservations relevant to this response.
    pub(crate) reservations: Vec<ReservationId>,
    /// Reservations that block this response.
    pub(crate) blocked_by:   Vec<ReservationId>,
    /// A human-readable explanation of this response.
    pub(crate) message:      String,
    /// The verb-keyed facts consumers need without parsing prose.
    pub(crate) payload:      OutputPayload,
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

/// The status named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputStatus {
    /// The verb parsed, but no engine stands behind it yet.
    Unimplemented,
    /// Initialization created or verified the durable coordination resources.
    Initialized,
    /// The journal or its projection could not be safely read or published.
    LedgerUnreadable,
    /// An overlap-free edit check may proceed.
    Clear,
    /// A new reservation was appended and published.
    Claimed,
    /// One or more foreign reservations overlap the requested paths.
    BlockedByOverlap,
    /// The caller can correct the request and retry without repairing the ledger.
    InvalidInput,
    /// Another mutation retained the ledger lock through the retry window.
    Contention,
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
}

/// Structured facts that correspond to the response's verb.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub(crate) enum OutputPayload {
    /// The operation failed before it could establish any durable facts.
    NoFacts,
    /// Facts returned by `init`.
    Init(InitializationPayload),
    /// Placeholder facts for an unimplemented board query.
    Board(PendingPayload),
    /// Facts returned by `check`.
    Check(CheckPayload),
    /// Facts returned by `claim`.
    Claim(ClaimPayload),
    /// Facts returned by `release`.
    Release(ReleasePayload),
    /// Placeholder facts for an unimplemented sequencing operation.
    Sequence(PendingPayload),
    /// Placeholder facts for an unimplemented integration.
    Integrate(PendingPayload),
    /// Placeholder facts for an unimplemented recovery decision.
    Resolve(PendingPayload),
    /// Placeholder facts for an unimplemented renewal.
    Renew(PendingPayload),
}

/// The resources an `init` call created or left intact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct InitializationPayload {
    /// Whether initialization created the journal or found an existing one.
    pub(crate) ledger:        InitializationResource,
    /// Whether initialization created the config or left an existing file intact.
    pub(crate) configuration: InitializationResource,
}

/// The initialization outcome for one durable resource.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum InitializationResource {
    /// This initialization call created the resource.
    Created,
    /// This initialization call retained an existing resource unchanged.
    Existing,
}

/// A deliberately empty typed placeholder for a verb whose engine arrives later.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingPayload {}

/// Typed outcomes returned by `claim`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ClaimPayload {
    /// A reservation was appended with this minimal antichain.
    Claimed {
        /// The newly minted reservation identity.
        reservation_id:      ReservationId,
        /// The coordination run that owns the appended reservation.
        coordination_run_id: CoordinationRunId,
        /// The exact durable footprint.
        scopes:              ReservationScopeSet,
        /// Whether the worktree marker records `coordination_run_id`.
        marker_publication:  CoordinationRunMarkerPublication,
    },
    /// Foreign holders prevented the append.
    Blocked {
        /// Every holder whose live scopes intersected the request.
        conflicts: Vec<ReservationConflict>,
    },
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
pub(crate) enum CheckPayload {
    /// No foreign live reservation overlaps the requested paths.
    Clear {
        /// The minimal exact-file antichain evaluated by the hook.
        scopes: ReservationScopeSet,
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
        reservation_id: ReservationId,
        /// The fixed commit retained for integration checks.
        protected_tip:  ProtectedReservationTip,
        /// The trunk commit observed at checkpoint.
        trunk_oid:      GitObjectId,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
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
        reservation_id: ReservationId,
        /// The retained terminal disposition.
        disposition:    ReleaseDisposition,
        /// What happened to the worktree coordination-run marker.
        marker:         CoordinationRunMarkerRetirement,
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
    pub(crate) fn unimplemented(command_verb: CommandVerb) -> Self {
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

    /// Build the successful response for completed initialization.
    pub(crate) fn initialized(initialization: LedgerInitialization) -> Self {
        Self {
            verb:         CommandVerb::Init,
            status:       OutputStatus::Initialized,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      INITIALIZED_MESSAGE.to_owned(),
            payload:      OutputPayload::Init(InitializationPayload {
                ledger:        initialization.ledger.into(),
                configuration: initialization.configuration.into(),
            }),
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
            payload:      OutputPayload::NoFacts,
        }
    }

    /// Build the successful result for one appended claim.
    pub(crate) fn claimed(
        reservation_id: ReservationId,
        coordination_run_id: CoordinationRunId,
        scopes: ReservationScopeSet,
        marker_publication: CoordinationRunMarkerPublication,
    ) -> Self {
        let scope_count = scopes.as_slice().len();
        let message = match &marker_publication {
            CoordinationRunMarkerPublication::Published => {
                format!("Claimed {scope_count} reservation scope(s) as {reservation_id}.")
            },
            CoordinationRunMarkerPublication::Unavailable { diagnostic } => format!(
                "Claimed {scope_count} reservation scope(s) as {reservation_id}, but the coordination-run marker could not be published: {diagnostic}. Restore coordination run {coordination_run_id} through the process environment before subsequent commands."
            ),
        };
        Self {
            verb: CommandVerb::Claim,
            status: OutputStatus::Claimed,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::Claim(ClaimPayload::Claimed {
                reservation_id,
                coordination_run_id,
                scopes,
                marker_publication,
            }),
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
            payload: OutputPayload::Claim(ClaimPayload::Blocked { conflicts }),
        }
    }

    /// Build a successful mutation-free edit check.
    pub(crate) fn clear_check(scopes: ReservationScopeSet) -> Self {
        Self {
            verb:         CommandVerb::Check,
            status:       OutputStatus::Clear,
            exit_code:    BerthExit::Clear,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      "No foreign reservation overlaps the requested paths.".to_owned(),
            payload:      OutputPayload::Check(CheckPayload::Clear { scopes }),
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
            payload: OutputPayload::Check(CheckPayload::Blocked { scopes, conflicts }),
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
            payload:      OutputPayload::NoFacts,
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
            payload:      OutputPayload::NoFacts,
        }
    }

    /// Build a successful release lifecycle or evidence response.
    pub(crate) fn released(release_payload: ReleasePayload) -> Self {
        let reservation_id = release_payload.reservation_id();
        let status = release_payload.output_status();
        let message = match &release_payload {
            ReleasePayload::Checkpointed { protected_tip, .. } => format!(
                "Reservation {reservation_id} is outstanding at protected tip {protected_tip}."
            ),
            ReleasePayload::Resnapshotted { protected_tip, .. } => {
                format!("Reservation {reservation_id} now retains protected tip {protected_tip}.")
            },
            ReleasePayload::EvidenceRevalidated { evidence, .. } => match evidence {
                IntegrationEvidenceStatus::NotIntegrated => format!(
                    "Reservation {reservation_id} remains outstanding; its protected tip is not in trunk."
                ),
                IntegrationEvidenceStatus::Integrated { trunk_oid } => format!(
                    "Reservation {reservation_id} has integration evidence in trunk commit {trunk_oid}."
                ),
                IntegrationEvidenceStatus::TrunkRewritten => format!(
                    "Reservation {reservation_id} is blocking again because trunk no longer contains its verified evidence."
                ),
                IntegrationEvidenceStatus::ObjectUnknown => format!(
                    "Reservation {reservation_id} is blocking because git could not resolve its integration evidence."
                ),
            },
            ReleasePayload::Released { disposition, .. } => {
                format!("Reservation {reservation_id} recorded disposition {disposition:?}.")
            },
        };
        Self {
            verb: CommandVerb::Release,
            status,
            exit_code: BerthExit::Clear,
            reservations: vec![reservation_id],
            blocked_by: Vec::new(),
            message,
            payload: OutputPayload::Release(release_payload),
        }
    }
}

impl OutputPayload {
    const fn pending(command_verb: CommandVerb) -> Self {
        let pending = PendingPayload {};
        match command_verb {
            CommandVerb::Board => Self::Board(pending),
            CommandVerb::Init | CommandVerb::Check | CommandVerb::Claim | CommandVerb::Release => {
                Self::NoFacts
            },
            CommandVerb::Sequence => Self::Sequence(pending),
            CommandVerb::Integrate => Self::Integrate(pending),
            CommandVerb::Resolve => Self::Resolve(pending),
            CommandVerb::Renew => Self::Renew(pending),
        }
    }
}

fn blocked_message(conflicts: &[ReservationConflict]) -> String {
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

fn source_description(claim_source: &ClaimSource) -> String {
    match claim_source {
        ClaimSource::WorkPlan { plan, phase } => format!("plan {plan}, phase {phase}"),
        ClaimSource::Explicit => "explicit claim".to_owned(),
    }
}

fn purpose_description(reservation_purpose: &ReservationPurpose) -> String {
    match reservation_purpose {
        ReservationPurpose::Explained(explanation) => explanation.to_string(),
        ReservationPurpose::NotProvidedByCaller => "no reason provided by caller".to_owned(),
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

#[cfg(test)]
mod tests {
    use super::CommandVerb;
    use super::OutputEnvelope;
    use super::OutputStatus;

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
    fn init_has_a_non_placeholder_status() {
        let output_envelope = OutputEnvelope::initialized(crate::ledger::LedgerInitialization {
            ledger:        crate::config::InitializationState::Created,
            configuration: crate::config::InitializationState::Existing,
        });

        assert_eq!(output_envelope.status, OutputStatus::Initialized);
        assert_eq!(output_envelope.exit_code, crate::exit::BerthExit::Clear);
    }

    #[test]
    fn failed_init_has_no_initialization_facts() {
        let output_envelope = OutputEnvelope::ledger_unreadable(CommandVerb::Init, "bad journal");

        assert_eq!(output_envelope.payload, super::OutputPayload::NoFacts);
        assert!(
            serde_json::to_string(&output_envelope.payload).is_ok_and(|payload| !payload
                .contains("ledger")
                && !payload.contains("configuration"))
        );
    }
}
