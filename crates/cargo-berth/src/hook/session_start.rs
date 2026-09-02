//! Raw `SessionStart` payload parsing and engine-decided session reconciliation.

use std::io::Read;
use std::process::ExitCode;

use serde::Deserialize;
use serde_json::Value;

use super::context_notice;
use super::context_notice::HarnessContinuationStatement;
use super::process_binding::HarnessSessionIdentityAvailability;
use super::process_binding::HookWorkingDirectorySelection;
use super::process_binding::HookWorkingDirectoryUnavailable;
use crate::cli::CliOutputFormat;
use crate::output::EngineAnswerOccasion;
use crate::output::OutputEnvelope;
use crate::presentation::EnvelopePresentation;
use crate::presentation::RenderedOutputBlock;
use crate::verb::board;
use crate::verb::board::BoardDisplayOutcome;
use crate::verb::board::BoardOutputSelection;

/// The harness event name this verb answers.
const SESSION_START_EVENT_NAME: &str = "SessionStart";
/// The heading used when a response carries no rendered heading of its own.
const ENGINE_STATED_SUMMARY: &str = "cargo-berth reported on this session in its own words.";
/// The heading used when the payload never reached a reconciliation at all.
const UNRECONCILED_SUMMARY: &str = "cargo-berth could not reconcile this session.";
/// The heading used when the payload could not be read as a `SessionStart` request.
const INVALID_PAYLOAD_SUMMARY: &str = "cargo-berth rejected an invalid SessionStart payload.";
/// The detail used when the payload could not be read as a `SessionStart` request.
const INVALID_PAYLOAD_DETAIL: &str = "SessionStart stdin was not valid JSON, so reconciliation \
                                      did not run. Run `cargo-berth board --json` by hand.";
/// The detail used when the board opened a terminal instead of answering headlessly.
const TERMINAL_INSTEAD_OF_REPORT_DETAIL: &str = "The board opened a terminal view instead of returning a session report. Run \
     `cargo-berth board --json` by hand.";

/// Serde-only representation of one raw harness payload.
#[derive(Deserialize)]
struct SessionStartPayloadBoundary {
    cwd:        Option<String>,
    session_id: Option<String>,
}

/// The typed context one raw `SessionStart` payload supplies for reconciliation.
struct SessionStartReconciliationRequest {
    working_directory_selection:     HookWorkingDirectorySelection,
    harness_session_id_availability: HarnessSessionIdentityAvailability,
}

/// A raw payload could not be converted into the semantic reconciliation request.
enum SessionStartPayloadParseError {
    /// Serde could not read the expected payload object and boundary field types.
    ///
    /// A payload whose `cwd` or `session_id` is present but not a string lands here and
    /// reconciles nothing. The shell hook this verb replaces coerced such a value away and
    /// continued from the process directory; that coercion is deliberately not restored,
    /// for the reason the edit gate beside this one already gives. A coerced `cwd` silently
    /// selects a different repository, and a session report about a different repository is
    /// worse than saying the payload could not be read.
    InvalidPayload,
}

/// What one session-start response has to tell the reader.
enum SessionStartReport {
    /// The engine stated this heading and this complete detail for the reader.
    Stated { summary: String, detail: String },
    /// The engine considered this session and has nothing to raise.
    NothingToRaise,
}

/// Read and execute one raw `SessionStart` reconciliation payload.
///
/// `SessionStart` is advisory: it starts no work and blocks none, so every route ends at
/// exit 0 and the only question this verb answers is what the reader is told.
pub(crate) fn execute() -> ExitCode {
    publish(reconcile_session());
    ExitCode::SUCCESS
}

/// Reconcile the payload's repository and state what this session should read.
fn reconcile_session() -> SessionStartReport {
    let SessionStartReconciliationRequest {
        working_directory_selection,
        harness_session_id_availability,
    } = match read_request() {
        Ok(request) => request,
        Err(SessionStartPayloadParseError::InvalidPayload) => {
            return SessionStartReport::Stated {
                summary: INVALID_PAYLOAD_SUMMARY.to_owned(),
                detail:  INVALID_PAYLOAD_DETAIL.to_owned(),
            };
        },
    };
    if let Err(error) = working_directory_selection.enter_current_process() {
        return SessionStartReport::Stated {
            summary: UNRECONCILED_SUMMARY.to_owned(),
            detail:  unreconciled_detail(&error),
        };
    }
    harness_session_id_availability.select_for_current_process();
    EngineAnswerOccasion::OpeningSession.own_this_process();
    read_board()
}

fn read_request() -> Result<SessionStartReconciliationRequest, SessionStartPayloadParseError> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|_| SessionStartPayloadParseError::InvalidPayload)?;
    let value = serde_json::from_str::<Value>(&input)
        .map_err(|_| SessionStartPayloadParseError::InvalidPayload)?;
    SessionStartReconciliationRequest::from_value(&value)
}

/// Read the complete board and take the account the engine stated for it.
fn read_board() -> SessionStartReport {
    match board::execute(BoardOutputSelection::CompleteBoard, CliOutputFormat::Json) {
        BoardDisplayOutcome::HeadlessResponse(output_envelope)
        | BoardDisplayOutcome::TerminalDidNotOpen(output_envelope)
        | BoardDisplayOutcome::TerminalFailedAfterOpening(output_envelope)
        | BoardDisplayOutcome::FactsUnavailable(output_envelope) => {
            SessionStartReport::from_board_response(&output_envelope)
        },
        BoardDisplayOutcome::TerminalRestored => SessionStartReport::Stated {
            summary: UNRECONCILED_SUMMARY.to_owned(),
            detail:  TERMINAL_INSTEAD_OF_REPORT_DETAIL.to_owned(),
        },
    }
}

/// Name the directory this session start could not reconcile in.
fn unreconciled_detail(error: &HookWorkingDirectoryUnavailable) -> String {
    format!(
        "Hook working directory {} could not be entered. Run `cargo-berth board --json` by hand.",
        error.working_directory.display()
    )
}

impl SessionStartReconciliationRequest {
    fn from_value(value: &Value) -> Result<Self, SessionStartPayloadParseError> {
        let boundary = serde_json::from_value::<SessionStartPayloadBoundary>(value.clone())
            .map_err(|_| SessionStartPayloadParseError::InvalidPayload)?;
        Ok(Self {
            working_directory_selection:     HookWorkingDirectorySelection::from_boundary(
                boundary.cwd,
            ),
            harness_session_id_availability: HarnessSessionIdentityAvailability::from_boundary(
                boundary.session_id,
            ),
        })
    }
}

impl SessionStartReport {
    /// Take one board response's own account of the session, without classifying it.
    ///
    /// The engine that decided the response already decided both what is worth raising and
    /// how to say it, so its rendered blocks are published as they are and their leading
    /// heading is this session's heading. `NothingToShow` is the engine stating there is
    /// nothing to raise, which is a different answer from having supplied no report at all:
    /// a response carrying no presentation still carries the engine's own message, and
    /// publishing that beats reporting that the engine's output could not be read.
    fn from_board_response(output_envelope: &OutputEnvelope) -> Self {
        match output_envelope.presentation() {
            EnvelopePresentation::RenderedBlocks { blocks } => Self::from_blocks(blocks.as_slice()),
            EnvelopePresentation::NothingToShow => Self::NothingToRaise,
            EnvelopePresentation::NotProvided => Self::Stated {
                summary: ENGINE_STATED_SUMMARY.to_owned(),
                detail:  output_envelope.render_text(),
            },
        }
    }

    fn from_blocks(blocks: &[RenderedOutputBlock]) -> Self {
        match blocks {
            [] => Self::NothingToRaise,
            [leading_block, ..] => Self::Stated {
                summary: leading_block.summary.clone(),
                detail:  context_notice::render_blocks(blocks),
            },
        }
    }
}

/// Publish one session report on standard output, or publish nothing at all.
///
/// `berth_session_start.sh` states no continuation field, because a session-start
/// response cannot stop anything and the harness already continues by default. The
/// shared writer is told to omit it rather than being given a second copy of itself.
fn publish(report: SessionStartReport) {
    match report {
        SessionStartReport::NothingToRaise => {},
        SessionStartReport::Stated { summary, detail } => {
            context_notice::write_context_notice(
                SESSION_START_EVENT_NAME,
                &HarnessContinuationStatement::Omitted,
                &summary,
                &detail,
            );
        },
    }
}
