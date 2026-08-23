//! The frozen JSON output contract for `cargo-berth`.
//!
//! Every JSON response is one object with this exact envelope:
//!
//! ```json
//! {
//!   "verb": "<verb>",
//!   "status": "<status>",
//!   "exit_code": 0,
//!   "reservations": [],
//!   "blocked_by": [],
//!   "message": "<message>"
//! }
//! ```
//!
//! The exit-code values are `0` clear, `1` blocked by overlap, `2` blocked by
//! an unsatisfied ordering edge, `3` needs user authorization, `4` ledger
//! unreadable (edit paths fail open and `integrate` fails closed), and `5`
//! usage error.
//!
//! | `exit_code` | Meaning |
//! | --- | --- |
//! | `0` | Clear to proceed. |
//! | `1` | Blocked by a reservation overlap. |
//! | `2` | Blocked by an unsatisfied ordering edge. |
//! | `3` | Needs user authorization. |
//! | `4` | Ledger unreadable; edit paths fail open and `integrate` fails closed. |
//! | `5` | Usage error. |

use serde::Deserialize;
use serde::Serialize;

use crate::exit::BerthExit;
use crate::ids::ReservationId;

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
}

impl CommandVerb {
    /// Return this verb's fixed JSON string value.
    const fn json_name(self) -> &'static str {
        match self {
            Self::Init => "init",
            Self::Board => "board",
            Self::Check => "check",
            Self::Claim => "claim",
            Self::Release => "release",
            Self::Sequence => "sequence",
            Self::Integrate => "integrate",
        }
    }
}

/// The status named in a JSON response.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OutputStatus {
    /// The verb parsed, but no engine stands behind it yet.
    Unimplemented,
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
        }
    }

    /// Render the unimplemented response without a fallible serializer.
    ///
    /// Every value here is fixed or drawn from a closed enum: one status, the
    /// `BerthExit::Clear` code, empty reservation lists, a constant message,
    /// and a verb name from [`CommandVerb::json_name`]. Direct formatting
    /// therefore cannot fail, which is what lets an exit status of zero mean
    /// a complete envelope reached stdout.
    pub(crate) fn unimplemented_json(command_verb: CommandVerb) -> String {
        format!(
            concat!(
                "{{\"verb\":\"{}\",\"status\":\"unimplemented\",\"exit_code\":{},",
                "\"reservations\":[],\"blocked_by\":[],\"message\":\"{}\"}}"
            ),
            command_verb.json_name(),
            BerthExit::Clear.code(),
            UNIMPLEMENTED_MESSAGE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::CommandVerb;
    use super::OutputEnvelope;

    const EXPECTED_JSON: &str = "{\"verb\":\"board\",\"status\":\"unimplemented\",\"exit_code\":0,\"reservations\":[],\"blocked_by\":[],\"message\":\"The reservation engine is not implemented.\"}";

    #[test]
    fn envelope_round_trips_with_the_published_field_names() {
        let output_envelope = OutputEnvelope::unimplemented(CommandVerb::Board);
        let serialized_envelope = serde_json::to_string(&output_envelope);

        assert_eq!(
            OutputEnvelope::unimplemented_json(CommandVerb::Board),
            EXPECTED_JSON
        );
        assert!(
            serialized_envelope
                .as_ref()
                .is_ok_and(|serialized_envelope| serialized_envelope == EXPECTED_JSON)
        );
        assert!(
            serialized_envelope
                .and_then(|serialized_envelope| {
                    serde_json::from_str::<OutputEnvelope>(&serialized_envelope)
                })
                .is_ok_and(|round_tripped| round_tripped == output_envelope)
        );
    }

    /// The direct formatter and the derived serializer are two independent
    /// encoders of one frozen envelope. Comparing every verb is what keeps
    /// [`CommandVerb::json_name`] agreeing with `rename_all`.
    #[test]
    fn every_verb_renders_the_same_bytes_through_both_encoders() {
        for command_verb in [
            CommandVerb::Init,
            CommandVerb::Board,
            CommandVerb::Check,
            CommandVerb::Claim,
            CommandVerb::Release,
            CommandVerb::Sequence,
            CommandVerb::Integrate,
        ] {
            let serialized_envelope =
                serde_json::to_string(&OutputEnvelope::unimplemented(command_verb));

            assert!(serialized_envelope.is_ok_and(|serialized_envelope| {
                serialized_envelope == OutputEnvelope::unimplemented_json(command_verb)
            }));
        }
    }

    #[test]
    fn envelope_rejects_an_unknown_exit_code() {
        const UNKNOWN_EXIT_CODE: &str = "{\"verb\":\"board\",\"status\":\"unimplemented\",\"exit_code\":6,\"reservations\":[],\"blocked_by\":[],\"message\":\"The reservation engine is not implemented.\"}";

        assert!(serde_json::from_str::<OutputEnvelope>(UNKNOWN_EXIT_CODE).is_err());
    }
}
