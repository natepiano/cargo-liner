//! Hook response construction and invocation shared by acceptance and reader checks.

use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use std::process::Output;
use std::process::Stdio;

use serde_json::Value;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// The harness event one hook response answers, and the continuation field it states.
///
/// The two events state different response objects, and the difference is a contract:
/// `berth_post_bash.sh` reports `continue`, and `berth_session_start.sh` deliberately
/// reports none, because a session-start response cannot stop anything the harness is
/// already going to do. Naming the event rather than passing its bare string keeps that
/// difference on one type instead of at every comparison.
#[derive(Clone, Copy)]
pub(crate) enum HookResponseEvent {
    /// A `PostToolUse` response, which states that the session continues.
    PostToolUse,
    /// A `SessionStart` response, which states no continuation field at all.
    SessionStart,
}

impl HookResponseEvent {
    const fn name(self) -> &'static str {
        match self {
            Self::PostToolUse => "PostToolUse",
            Self::SessionStart => "SessionStart",
        }
    }
}

/// Whether the hook process inherits a harness session identity from its environment.
pub(crate) enum AmbientHarnessSession<'session> {
    /// The environment names a harness session the hook must not adopt.
    Present(&'session str),
    /// The environment names no harness session.
    Absent,
}

/// The two sentences one hook response puts in front of the reader.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct HookFeedback {
    pub(crate) system_message:     String,
    pub(crate) additional_context: String,
}

/// Run one public hook verb the way the harness runs it: raw payload on standard input.
pub(crate) fn spawn_hook_verb(
    executable: &Path,
    working_directory: &Path,
    hook_event: &str,
    stdin: &[u8],
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    let mut command = Command::new(executable);
    command
        .args(["hook", hook_event])
        .current_dir(working_directory)
        .env_remove("CARGO_BERTH_RUN");
    match *ambient_session {
        AmbientHarnessSession::Present(session_id) => {
            command.env("CARGO_BERTH_SESSION_ID", session_id);
        },
        AmbientHarnessSession::Absent => {
            command.env_remove("CARGO_BERTH_SESSION_ID");
        },
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut piped_stdin = child
        .stdin
        .take()
        .ok_or_else(|| failure("hook stdin should be piped"))?;
    piped_stdin.write_all(stdin)?;
    drop(piped_stdin);
    Ok(child.wait_with_output()?)
}

/// One hook response, read the way a harness reads it.
pub(crate) fn hook_feedback(
    output: &Output,
    event: HookResponseEvent,
    context: &str,
) -> TestResult<HookFeedback> {
    let event_name = event.name();
    if output.status.code() != Some(0) {
        return Err(failure(format!(
            "{context} should exit 0 like the hook it replaces, exited with {:?}: stderr={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    if !output.stderr.is_empty() {
        return Err(failure(format!(
            "{context} should keep its response on stdout: stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let response: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        failure(format!(
            "{context} stdout should be one hook response object: {error}; stdout={}",
            String::from_utf8_lossy(&output.stdout)
        ))
    })?;
    let observed_event = required_string(&response, "/hookSpecificOutput/hookEventName", context)?;
    if observed_event != event_name {
        return Err(failure(format!(
            "{context} should name the {event_name} event, named {observed_event:?}"
        )));
    }
    assert_stated_continuation(&response, event, context)?;
    expected_hook_feedback(&response, context)
}

/// Each event states its own continuation field, and `SessionStart` states none.
///
/// `berth_post_bash.sh` reports `continue`, so a `PostToolUse` response reports it too. The
/// installed `berth_session_start.sh` deliberately reports no continuation field, because a
/// session-start response cannot stop anything the harness is already going to do; moving
/// the two events onto one shared writer must not add the field the installed hook omits.
fn assert_stated_continuation(
    response: &Value,
    event: HookResponseEvent,
    context: &str,
) -> TestResult {
    let stated = response.get("continue");
    match event {
        HookResponseEvent::PostToolUse => {
            if stated == Some(&Value::Bool(true)) {
                return Ok(());
            }
            Err(failure(format!(
                "{context} should report that the session continues: {response}"
            )))
        },
        HookResponseEvent::SessionStart => stated.map_or_else(
            || Ok(()),
            |stated| {
                Err(failure(format!(
                    "{context} states a continuation field the installed hook omits: {stated}"
                )))
            },
        ),
    }
}

/// Read the expected message and context from a frozen hook response object.
pub(crate) fn expected_hook_feedback(response: &Value, context: &str) -> TestResult<HookFeedback> {
    Ok(HookFeedback {
        system_message:     required_string(response, "/systemMessage", context)?.to_owned(),
        additional_context: required_string(
            response,
            "/hookSpecificOutput/additionalContext",
            context,
        )?
        .to_owned(),
    })
}

fn required_string<'value>(
    value: &'value Value,
    pointer: &str,
    context: &str,
) -> TestResult<&'value str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .ok_or_else(|| failure(format!("{context} should carry string {pointer}")))
}

fn failure(message: impl Into<String>) -> Box<dyn Error> {
    std::io::Error::other(message.into()).into()
}
