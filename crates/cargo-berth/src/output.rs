//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response retains the original six-field envelope and adds one
//! typed `payload` field. Consumers can continue reading the original fields,
//! while newer consumers use `payload` instead of scraping `message`.

use serde::Deserialize;
use serde::Serialize;

use crate::config::InitializationState;
use crate::exit::BerthExit;
use crate::ids::ReservationId;
use crate::ledger::LedgerInitialization;

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
    /// Placeholder facts for an unimplemented overlap check.
    Check(PendingPayload),
    /// Placeholder facts for an unimplemented claim.
    Claim(PendingPayload),
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
}

impl OutputPayload {
    const fn pending(command_verb: CommandVerb) -> Self {
        let pending = PendingPayload {};
        match command_verb {
            CommandVerb::Init => Self::NoFacts,
            CommandVerb::Board => Self::Board(pending),
            CommandVerb::Check => Self::Check(pending),
            CommandVerb::Claim => Self::Claim(pending),
            CommandVerb::Release => Self::Release(pending),
            CommandVerb::Sequence => Self::Sequence(pending),
            CommandVerb::Integrate => Self::Integrate(pending),
            CommandVerb::Resolve => Self::Resolve(pending),
            CommandVerb::Renew => Self::Renew(pending),
        }
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
