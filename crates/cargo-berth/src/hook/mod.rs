//! Engine-owned harness hook protocols.
//!
//! Every harness hook event the engine answers reads one raw JSON payload from
//! standard input, binds this process to the repository and harness session that
//! payload names, and publishes the response object the harness expects. The
//! pieces that are the same for every event live here; each verb's module holds
//! only the wire shape and decisions that belong to its own event.

pub(crate) mod post_tool_use;
pub(crate) mod pre_tool_use;
pub(crate) mod session_start;

use std::io::Write;
use std::path::PathBuf;

use serde::Serialize;

use crate::presentation::RenderedOutputBlock;
use crate::session;
use crate::session::HarnessSessionId;
use crate::session::HookHarnessSessionSelection;

const BLOCKING_EXIT_CODE: u8 = 2;

/// How a hook chooses the directory whose repository owns its answer.
enum HookWorkingDirectorySelection {
    /// The payload supplied a non-empty working directory.
    PayloadSupplied(PathBuf),
    /// The payload omitted its working directory, so the process directory applies.
    CurrentProcess,
}

/// Whether a hook can select a disposable harness-session mapping.
enum HarnessSessionIdentityAvailability {
    /// The payload supplied a valid bounded session identifier.
    Available(HarnessSessionId),
    /// The payload supplied no identifier, or one unsuitable for durable lookup.
    Unusable,
}

/// A hook could not establish the directory whose repository owns its answer.
enum HookWorkingDirectoryResolutionError {
    /// The payload named no directory and this process has no readable current directory.
    CurrentProcessUnavailable,
}

/// A hook could not enter the directory the payload named for its answer.
///
/// Only a payload-supplied directory can fail: a payload that names none leaves this
/// process where the harness launched it, which needs no move and cannot be refused.
struct HookWorkingDirectoryUnavailable {
    /// The directory the payload named, in the words the payload named it.
    working_directory: PathBuf,
}

/// Whether one hook event's response object states that the harness continues.
enum HarnessContinuationStatement {
    /// The event can stop the harness, so its response says outright that it does not.
    Stated,
    /// The event can stop nothing, so its response omits a field whose only value is
    /// the default the harness already applies.
    Omitted,
}

impl HookWorkingDirectorySelection {
    fn from_boundary(working_directory: Option<String>) -> Self {
        working_directory
            .filter(|working_directory| !working_directory.is_empty())
            .map_or(Self::CurrentProcess, |working_directory| {
                Self::PayloadSupplied(PathBuf::from(working_directory))
            })
    }

    fn resolve(&self) -> Result<PathBuf, HookWorkingDirectoryResolutionError> {
        match self {
            Self::PayloadSupplied(working_directory) => Ok(working_directory.clone()),
            Self::CurrentProcess => std::env::current_dir()
                .map_err(|_| HookWorkingDirectoryResolutionError::CurrentProcessUnavailable),
        }
    }

    /// Place this process in the directory whose repository owns the hook's answer.
    ///
    /// The payload's directory is entered exactly as the harness named it, so the kernel
    /// resolves its symlinks and `..` components. Collapsing them textually first would
    /// select a different directory whenever a symlinked ancestor sits left of a `..` —
    /// `/link/../repo` is `/repo` as text and the sibling of `/link`'s target in the
    /// filesystem — and the whole answer would then be about the wrong repository. A
    /// payload that names no directory leaves this process where the harness launched
    /// it, which is already the directory the answer is about.
    ///
    /// A `cwd` present but not a string never reaches here at all: every route into this
    /// selection, the `--post-tool-use-payload` route included, rejects it as an invalid
    /// payload rather than coercing it away, because a coerced `cwd` silently observes a
    /// different repository.
    fn enter_current_process(&self) -> Result<(), HookWorkingDirectoryUnavailable> {
        match self {
            Self::PayloadSupplied(working_directory) => {
                std::env::set_current_dir(working_directory).map_err(|_| {
                    HookWorkingDirectoryUnavailable {
                        working_directory: working_directory.clone(),
                    }
                })
            },
            Self::CurrentProcess => Ok(()),
        }
    }
}

impl HarnessSessionIdentityAvailability {
    fn from_boundary(harness_session_id: Option<String>) -> Self {
        harness_session_id
            .filter(|harness_session_id| !harness_session_id.is_empty())
            .map_or(Self::Unusable, |harness_session_id| {
                harness_session_id
                    .parse()
                    .map_or(Self::Unusable, Self::Available)
            })
    }

    /// Bind this process to the payload's session identity, or to no session at all.
    ///
    /// An absent or unusable payload identity must not fall through to an ambient
    /// `CARGO_BERTH_SESSION_ID`. That variable belongs to whichever session launched this
    /// hook process, so adopting it would map the event onto another session's reservation.
    fn select_for_current_process(self) {
        session::select_current_process_harness_session(match self {
            Self::Available(harness_session_id) => {
                HookHarnessSessionSelection::Session(harness_session_id)
            },
            Self::Unusable => HookHarnessSessionSelection::NoSession,
        });
    }
}

/// The stdout object a hook returns when it publishes context and lets work continue.
///
/// The continuation field is serde-only and carries the absent case for one reason: a
/// `SessionStart` response omits it, and the shape of the object is what the harness
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
fn write_context_notice(
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

fn render_blocks(blocks: &[RenderedOutputBlock]) -> String {
    blocks
        .iter()
        .map(|block| format!("{}\n\n{}", block.summary, block.detail))
        .collect::<Vec<_>>()
        .join("\n\n")
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
