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
use crate::ids::ReservationId;
use crate::ledger::ClaimSource;
use crate::ledger::LedgerInitialization;
use crate::ledger::ReservationPurpose;
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
    /// Placeholder facts for an unimplemented release.
    Release(PendingPayload),
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
            exit_code:    BerthExit::LedgerUnreadable,
            reservations: Vec::new(),
            blocked_by:   Vec::new(),
            message:      diagnostic.to_owned(),
            payload:      OutputPayload::NoFacts,
        }
    }
}

impl OutputPayload {
    const fn pending(command_verb: CommandVerb) -> Self {
        let pending = PendingPayload {};
        match command_verb {
            CommandVerb::Board => Self::Board(pending),
            CommandVerb::Init | CommandVerb::Check | CommandVerb::Claim => Self::NoFacts,
            CommandVerb::Release => Self::Release(pending),
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
