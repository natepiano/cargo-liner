//! The one stdout object every hook event publishes when work continues.
//!
//! The three events differ only in whether stating continuation means anything for
//! them, so the object, its writer, and the rendering that fills it live here rather
//! than in any one event's module.

use std::io::Write;

use serde::Serialize;

use crate::presentation::RenderedOutputBlock;

/// Whether one hook event's response object states that the harness continues.
pub(super) enum HarnessContinuationStatement {
    /// The event can stop the harness, so its response says outright that it does not.
    Stated,
    /// The event can stop nothing, so its response omits a field whose only value is
    /// the default the harness already applies.
    Omitted,
}

/// The stdout object a hook returns when it publishes context and lets work continue.
///
/// The continuation field is serde-only and carries the absent case for one reason: a
/// `SessionStart` response omits it, and which fields the object carries is what the harness
/// reads. [`HarnessContinuationStatement`] is what callers name it by.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HookContextNotice<'notice> {
    #[serde(rename = "continue", skip_serializing_if = "Option::is_none")]
    harness_continues:    Option<bool>,
    system_message:       &'notice str,
    hook_specific_output: HookContextNoticeDetail<'notice>,
}

/// The event-specific context carried by one hook context notice.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HookContextNoticeDetail<'notice> {
    hook_event_name:    &'static str,
    additional_context: &'notice str,
}

/// Publish one hook response that states context to read and lets the harness continue.
///
/// Every event this engine answers publishes the same object; they differ only in
/// whether stating continuation means anything for that event, so that is the one thing
/// the caller chooses.
pub(super) fn write_context_notice(
    hook_event_name: &'static str,
    continuation: &HarnessContinuationStatement,
    system_message: &str,
    additional_context: &str,
) {
    let notice = HookContextNotice {
        harness_continues: match continuation {
            HarnessContinuationStatement::Stated => Some(true),
            HarnessContinuationStatement::Omitted => None,
        },
        system_message,
        hook_specific_output: HookContextNoticeDetail {
            hook_event_name,
            additional_context,
        },
    };
    let mut standard_output = std::io::stdout().lock();
    if serde_json::to_writer(&mut standard_output, &notice).is_ok() {
        std::mem::drop(standard_output.write_all(b"\n"));
    }
}

pub(super) fn render_blocks(blocks: &[RenderedOutputBlock]) -> String {
    blocks
        .iter()
        .map(|block| format!("{}\n\n{}", block.summary, block.detail))
        .collect::<Vec<_>>()
        .join("\n\n")
}
