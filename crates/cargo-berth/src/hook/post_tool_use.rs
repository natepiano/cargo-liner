//! Raw `PostToolUse` payload parsing and post-Bash drift reporting.
//!
//! A Bash call has already completed by the time this event fires, so every
//! answer is notification or stop feedback rather than a permission gate. The
//! engine performs the drift comparison, reads the live incursion board when the
//! drift answer depends on it, and publishes one response object; the harness is
//! never asked to run a second command to complete an answer.

use std::ffi::OsString;
use std::io::Read;
use std::process::ExitCode;

use serde::Deserialize;
use serde_json::Value;

use super::HarnessContinuationStatement;
use super::HarnessSessionIdentityAvailability;
use super::HookWorkingDirectorySelection;
use super::write_context_notice;
use crate::cli::CliOutputFormat;
use crate::coordination_identity::RecoveryCommandLine;
use crate::drift::DriftComparisonChoice;
use crate::drift::DriftRequest;
use crate::drift::DriftReservationSelection;
use crate::drift::PostCommitWideningSelection;
use crate::exit::BerthExit;
use crate::output::EngineAnswerOccasion;
use crate::output::OutputEnvelope;
use crate::output::PostToolUseRendering;
use crate::presentation::unverifiable_incursion_block;
use crate::session::HarnessSessionId;
use crate::verb::board;
use crate::verb::board::BoardDisplayOutcome;
use crate::verb::board::BoardOutputSelection;
use crate::verb::drift;

const HOOK_EVENT_NAME: &str = "PostToolUse";
const INVALID_PAYLOAD_SUMMARY: &str = "cargo-berth rejected an invalid PostToolUse payload.";
const INVALID_PAYLOAD_DETAIL: &str = "STOP: `cargo-berth hook post-tool-use` requires valid JSON, tool_name Bash, a session_id of 1 to 256 characters with no control characters, and a cwd that is a string when it is present. Run `cargo-berth drift --reservation <id> --json` by hand.";
const UNAVAILABLE_WORKING_DIRECTORY_SUMMARY: &str = "cargo-berth could not inspect this Bash call.";
const UNAVAILABLE_WORKING_DIRECTORY_DETAIL: &str =
    "STOP: the hook working directory does not exist or is unavailable.";

/// Serde-only representation of one raw `PostToolUse` payload.
///
/// A payload whose `cwd` or `session_id` is present but not a string lands in
/// [`PostToolUseObservationError::InvalidPayload`] rather than being coerced to an empty
/// string, for the reason the pre-edit boundary states: an empty `cwd` silently observes a
/// different repository and an empty `session_id` silently attributes drift to a different
/// session's reservation.
#[derive(Deserialize)]
struct PostToolUsePayloadBoundary {
    tool_name:  Option<String>,
    cwd:        Option<String>,
    session_id: Option<String>,
}

/// Whether the completed tool call this payload reports is one drift can observe.
///
/// Drift compares a worktree against the commits a shell produced, so a Bash call is
/// the only tool this event has an answer for. A payload naming any other tool, or
/// naming none, is a payload this verb was invoked on by mistake rather than a Bash
/// call with nothing to report, and it is reported as such.
enum PostToolUseObservableToolCall {
    /// The payload reports a completed Bash call.
    BashCall,
    /// The payload reports another tool, or names none, so there is nothing to observe.
    NotABashCall,
}

impl PostToolUseObservableToolCall {
    fn from_boundary(tool_name: Option<&str>) -> Self {
        match tool_name {
            Some("Bash") => Self::BashCall,
            Some(_) | None => Self::NotABashCall,
        }
    }
}

/// The completed Bash call one raw `PostToolUse` payload reports.
struct ObservedBashCall {
    harness_session_id:          HarnessSessionId,
    working_directory_selection: HookWorkingDirectorySelection,
}

/// A raw `PostToolUse` payload could not be converted into an observable Bash call.
enum PostToolUseObservationError {
    /// Stdin did not carry a Bash call this verb can attribute to a harness session.
    InvalidPayload,
    /// The payload named a working directory this process could not enter.
    WorkingDirectoryUnavailable,
}

/// The complete `PostToolUse` answer this verb publishes for one Bash call.
enum PostToolUseAnswer {
    /// The engine considered this Bash call and has nothing to report.
    Silent,
    /// The engine states this summary and detail for the user to read.
    Stated { summary: String, detail: String },
}

/// Whether a board read could establish the current state of the reported incursions.
enum LiveIncursionState {
    /// This board response states which incidents are still outstanding.
    Read(Box<OutputEnvelope>),
    /// No current board read could confirm whether the reported incursions still stand.
    Unverifiable,
}

impl ObservedBashCall {
    fn from_value(value: &Value) -> Result<Self, PostToolUseObservationError> {
        let boundary = serde_json::from_value::<PostToolUsePayloadBoundary>(value.clone())
            .map_err(|_| PostToolUseObservationError::InvalidPayload)?;
        let PostToolUseObservableToolCall::BashCall =
            PostToolUseObservableToolCall::from_boundary(boundary.tool_name.as_deref())
        else {
            return Err(PostToolUseObservationError::InvalidPayload);
        };
        let HarnessSessionIdentityAvailability::Available(harness_session_id) =
            HarnessSessionIdentityAvailability::from_boundary(boundary.session_id)
        else {
            return Err(PostToolUseObservationError::InvalidPayload);
        };
        Ok(Self {
            harness_session_id,
            working_directory_selection: HookWorkingDirectorySelection::from_boundary(boundary.cwd),
        })
    }

    /// Bind this process to the repository, harness session and occasion of the Bash call.
    ///
    /// This verb is the only route that reaches a completed Bash call, and it binds all
    /// three before it reports anything, so every response it produces names the Bash call
    /// it was taken after.
    fn enter_current_process(self) -> Result<(), PostToolUseObservationError> {
        self.working_directory_selection
            .enter_current_process()
            .map_err(|_| PostToolUseObservationError::WorkingDirectoryUnavailable)?;
        HarnessSessionIdentityAvailability::Available(self.harness_session_id)
            .select_for_current_process();
        EngineAnswerOccasion::CompletedBashCall.own_this_process();
        Ok(())
    }
}

impl PostToolUseObservationError {
    /// State a rejected payload in the same response object a drift answer uses.
    ///
    /// The Bash call already ran, so a payload this verb cannot read is still reported
    /// rather than swallowed: the user is owed the fact that no drift check covered it.
    fn answer(&self) -> PostToolUseAnswer {
        let (summary, detail) = match self {
            Self::InvalidPayload => (INVALID_PAYLOAD_SUMMARY, INVALID_PAYLOAD_DETAIL),
            Self::WorkingDirectoryUnavailable => (
                UNAVAILABLE_WORKING_DIRECTORY_SUMMARY,
                UNAVAILABLE_WORKING_DIRECTORY_DETAIL,
            ),
        };
        PostToolUseAnswer::Stated {
            summary: summary.to_owned(),
            detail:  detail.to_owned(),
        }
    }
}

impl PostToolUseAnswer {
    /// State one engine rendering, treating an unresolved live-board decision as unverifiable.
    ///
    /// A rendering taken against a live board has already decided what the response
    /// carries, so a second live-board request cannot be satisfied and is exactly the
    /// case the unverifiable notice describes.
    fn from_rendering(rendering: PostToolUseRendering) -> Self {
        match rendering {
            PostToolUseRendering::NoFeedback => Self::Silent,
            PostToolUseRendering::Feedback { summary, detail } => Self::Stated { summary, detail },
            PostToolUseRendering::FeedbackDecidedByLiveIncursionState => Self::unverifiable(),
        }
    }

    fn unverifiable() -> Self {
        let block = unverifiable_incursion_block();
        Self::Stated {
            summary: block.summary,
            detail:  block.detail,
        }
    }

    fn publish(&self) {
        match self {
            Self::Silent => {},
            Self::Stated { summary, detail } => {
                write_context_notice(
                    HOOK_EVENT_NAME,
                    &HarnessContinuationStatement::Stated,
                    summary,
                    detail,
                );
            },
        }
    }
}

/// Read and answer one raw `PostToolUse` payload for a completed Bash call.
///
/// The Bash call is already done, so nothing this verb reports can block it. Every
/// answer therefore leaves the process status successful and speaks through the
/// response object alone.
pub(crate) fn execute() -> ExitCode {
    let answer = match read_observed_bash_call().and_then(ObservedBashCall::enter_current_process) {
        Ok(()) => answer_for(&drift::execute(
            post_tool_use_drift_request(),
            &drift_recovery(),
        )),
        Err(error) => error.answer(),
    };
    answer.publish();
    ExitCode::SUCCESS
}

fn read_observed_bash_call() -> Result<ObservedBashCall, PostToolUseObservationError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| PostToolUseObservationError::InvalidPayload)?;
    let value = serde_json::from_str::<Value>(&input)
        .map_err(|_| PostToolUseObservationError::InvalidPayload)?;
    ObservedBashCall::from_value(&value)
}

/// Decide what the response carries, reading the live board only when drift needs it.
fn answer_for(drift_envelope: &OutputEnvelope) -> PostToolUseAnswer {
    match drift_envelope.post_tool_use_rendering() {
        PostToolUseRendering::NoFeedback => PostToolUseAnswer::Silent,
        PostToolUseRendering::Feedback { summary, detail } => {
            PostToolUseAnswer::Stated { summary, detail }
        },
        PostToolUseRendering::FeedbackDecidedByLiveIncursionState => match live_incursion_state() {
            LiveIncursionState::Read(board_envelope) => PostToolUseAnswer::from_rendering(
                drift_envelope.post_tool_use_rendering_with_live_board(&board_envelope),
            ),
            LiveIncursionState::Unverifiable => PostToolUseAnswer::unverifiable(),
        },
    }
}

/// Read the board this process must consult before it can state incursion feedback.
fn live_incursion_state() -> LiveIncursionState {
    let board_envelope =
        match board::execute(BoardOutputSelection::CompleteBoard, CliOutputFormat::Json) {
            BoardDisplayOutcome::HeadlessResponse(board_envelope)
            | BoardDisplayOutcome::TerminalDidNotOpen(board_envelope)
            | BoardDisplayOutcome::TerminalFailedAfterOpening(board_envelope)
            | BoardDisplayOutcome::FactsUnavailable(board_envelope) => board_envelope,
            BoardDisplayOutcome::TerminalRestored => return LiveIncursionState::Unverifiable,
        };
    if matches!(board_envelope.exit_code(), BerthExit::Clear) {
        LiveIncursionState::Read(Box::new(board_envelope))
    } else {
        LiveIncursionState::Unverifiable
    }
}

/// The drift comparison this event performs for every reservation the worktree holds.
const fn post_tool_use_drift_request() -> DriftRequest {
    DriftRequest {
        comparison:  DriftComparisonChoice::CheapDelta,
        reservation: DriftReservationSelection::EveryActiveForPostCommit {
            widening: PostCommitWideningSelection::SessionMappingOrSingleCandidate,
        },
    }
}

/// The command a coordination-identity rejection tells the user to rerun.
///
/// This process reads its payload from standard input, so its own argv is not a command
/// anyone can rerun by hand. The drift comparison it performs is.
fn drift_recovery() -> RecoveryCommandLine {
    RecoveryCommandLine::try_from(vec![
        OsString::from("cargo-berth"),
        OsString::from("drift"),
        OsString::from("--json"),
    ])
    .unwrap_or_else(|_| RecoveryCommandLine::current_process())
}

#[cfg(test)]
mod tests {
    use super::ObservedBashCall;
    use super::PostToolUseObservationError;
    use crate::hook::HookWorkingDirectorySelection;
    use crate::session::HarnessSessionId;

    #[test]
    fn multibyte_harness_session_id_uses_character_limit() {
        let accepted_session_id = "é".repeat(HarnessSessionId::MAXIMUM_CHARACTERS);
        let accepted_payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": accepted_session_id,
        });
        let expected_harness_session_id = accepted_session_id.parse::<HarnessSessionId>();

        assert!(matches!(
            (
                ObservedBashCall::from_value(&accepted_payload),
                expected_harness_session_id,
            ),
            (
                Ok(ObservedBashCall {
                    harness_session_id,
                    ..
                }),
                Ok(expected_harness_session_id),
            ) if harness_session_id == expected_harness_session_id
        ));

        let rejected_session_id = "é".repeat(HarnessSessionId::MAXIMUM_CHARACTERS + 1);
        let rejected_payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": rejected_session_id,
        });

        assert!(matches!(
            ObservedBashCall::from_value(&rejected_payload),
            Err(PostToolUseObservationError::InvalidPayload)
        ));
    }

    #[test]
    fn overlong_harness_session_id_is_an_invalid_payload() {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": "a".repeat(HarnessSessionId::MAXIMUM_CHARACTERS + 1),
        });

        assert!(matches!(
            ObservedBashCall::from_value(&payload),
            Err(PostToolUseObservationError::InvalidPayload)
        ));
    }

    #[test]
    fn control_character_harness_session_id_is_an_invalid_payload() {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": "session\u{0000}",
        });

        assert!(matches!(
            ObservedBashCall::from_value(&payload),
            Err(PostToolUseObservationError::InvalidPayload)
        ));
    }

    #[test]
    fn a_payload_without_a_bash_call_is_an_invalid_payload() {
        let payload = serde_json::json!({
            "tool_name": "Edit",
            "session_id": "post-tool-use-session",
        });

        assert!(matches!(
            ObservedBashCall::from_value(&payload),
            Err(PostToolUseObservationError::InvalidPayload)
        ));
    }

    #[test]
    fn an_empty_working_directory_selects_the_process_directory() {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": "post-tool-use-session",
            "cwd": "",
        });

        assert!(matches!(
            ObservedBashCall::from_value(&payload),
            Ok(ObservedBashCall {
                working_directory_selection: HookWorkingDirectorySelection::CurrentProcess,
                ..
            })
        ));
    }

    #[test]
    fn a_non_string_working_directory_is_an_invalid_payload() {
        let payload = serde_json::json!({
            "tool_name": "Bash",
            "session_id": "post-tool-use-session",
            "cwd": 7,
        });

        assert!(matches!(
            ObservedBashCall::from_value(&payload),
            Err(PostToolUseObservationError::InvalidPayload)
        ));
    }
}
