//! Engine-owned harness hook protocols.

pub(crate) mod pre_tool_use;

use std::io::Write;
use std::process::ExitCode;

use serde::Serialize;

use crate::exit::BerthExit;
use crate::output::LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE;
use crate::output::OutputEnvelope;
use crate::presentation::EnvelopePresentation;
use crate::presentation::RenderedOutputBlock;

const BLOCKING_EXIT_CODE: u8 = 2;
const AUTHORIZED_SYSTEM_MESSAGE: &str =
    "cargo-berth authorized this edit and stated the detail below itself.";
const FAIL_OPEN_SYSTEM_MESSAGE: &str =
    "cargo-berth could not establish edit safety and stated the detail below itself.";

/// The stdout object returned when a `PreToolUse` request remains allowed with a notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreToolUseAllowNotice<'detail> {
    system_message:       &'static str,
    hook_specific_output: PreToolUseAllowNoticeDetail<'detail>,
}

/// The event-specific authorization carried by a `PreToolUse` allow notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PreToolUseAllowNoticeDetail<'detail> {
    hook_event_name:            &'static str,
    permission_decision:        &'static str,
    permission_decision_reason: &'detail str,
}

/// Map one engine check answer onto the Claude `PreToolUse` hook protocol.
fn render_pre_tool_use_answer(output_envelope: &OutputEnvelope) -> ExitCode {
    match output_envelope.exit_code() {
        BerthExit::Clear => render_authorized(output_envelope.presentation()),
        BerthExit::LedgerUnreadable => render_fail_open(output_envelope),
        BerthExit::BlockedByOverlap
        | BerthExit::BlockedByOrdering
        | BerthExit::NeedsUserAuthorization
        | BerthExit::UsageError
        | BerthExit::BlockedByContention
        | BerthExit::TerminalViewFailed => render_refusal(output_envelope.presentation()),
    }
}

fn render_authorized(presentation: &EnvelopePresentation) -> ExitCode {
    match presentation {
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_allow_notice(AUTHORIZED_SYSTEM_MESSAGE, &render_blocks(blocks.as_slice()))
        },
        EnvelopePresentation::NothingToShow | EnvelopePresentation::NotProvided => {
            ExitCode::SUCCESS
        },
    }
}

fn render_fail_open(output_envelope: &OutputEnvelope) -> ExitCode {
    match output_envelope.presentation() {
        EnvelopePresentation::NothingToShow => ExitCode::SUCCESS,
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_allow_notice(FAIL_OPEN_SYSTEM_MESSAGE, &render_blocks(blocks.as_slice()))
        },
        EnvelopePresentation::NotProvided => write_allow_notice(
            LEDGER_UNREADABLE_FAIL_OPEN_MESSAGE,
            &output_envelope.render_text(),
        ),
    }
}

fn render_refusal(presentation: &EnvelopePresentation) -> ExitCode {
    match presentation {
        EnvelopePresentation::RenderedBlocks { blocks } => {
            write_stderr_line(&render_blocks(blocks.as_slice()));
        },
        EnvelopePresentation::NothingToShow => refuse_hook_request(
            "the engine returned a blocking check answer marked as deliberate silence",
        ),
        EnvelopePresentation::NotProvided => refuse_hook_request(
            "the engine returned a blocking check answer without a presentation",
        ),
    }
    ExitCode::from(BLOCKING_EXIT_CODE)
}

fn render_blocks(blocks: &[RenderedOutputBlock]) -> String {
    blocks
        .iter()
        .map(|block| format!("{}\n\n{}", block.summary, block.detail))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn write_allow_notice(system_message: &'static str, detail: &str) -> ExitCode {
    let notice = PreToolUseAllowNotice {
        system_message,
        hook_specific_output: PreToolUseAllowNoticeDetail {
            hook_event_name:            "PreToolUse",
            permission_decision:        "allow",
            permission_decision_reason: detail,
        },
    };
    let mut standard_output = std::io::stdout().lock();
    if serde_json::to_writer(&mut standard_output, &notice).is_ok() {
        std::mem::drop(standard_output.write_all(b"\n"));
    }
    ExitCode::SUCCESS
}

fn refuse_hook_request(reason: &str) {
    write_stderr_line(&format!(
        "cargo-berth refused this edit hook request: {reason}"
    ));
}

fn write_stderr_line(detail: &str) {
    let mut standard_error = std::io::stderr().lock();
    std::mem::drop(standard_error.write_all(detail.as_bytes()));
    std::mem::drop(standard_error.write_all(b"\n"));
}
