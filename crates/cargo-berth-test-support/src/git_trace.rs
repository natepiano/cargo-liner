//! Native Git command traces avoid a shell wrapper for each Git invocation.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use serde_json::Value;

use crate::FixtureCommand;
use crate::constants::GIT_NO_OPTIONAL_LOCKS_ARG;
use crate::constants::GIT_TRACE2_EVENT_ENVIRONMENT;

/// A failure to run a traced command or read, parse, or write its trace.
#[derive(Debug)]
pub enum GitTraceError {
    /// The fixture command failed.
    Command(io::Error),
    /// The trace event file could not be read.
    TraceRead {
        /// The event file that could not be read.
        path:   PathBuf,
        /// The operating system error.
        source: io::Error,
    },
    /// A trace event or its arguments could not be parsed.
    TraceParse {
        /// The event file with invalid data.
        path:   PathBuf,
        /// The JSON parse error.
        source: serde_json::Error,
    },
    /// The command trace could not be written.
    TraceWrite {
        /// The command trace that could not be written.
        path:   PathBuf,
        /// The operating system error.
        source: io::Error,
    },
}

impl Display for GitTraceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Command(source) => write!(formatter, "traced command failed: {source}"),
            Self::TraceRead { path, source } => {
                write!(
                    formatter,
                    "trace read failed for {}: {source}",
                    path.display()
                )
            },
            Self::TraceParse { path, source } => {
                write!(
                    formatter,
                    "trace parse failed for {}: {source}",
                    path.display()
                )
            },
            Self::TraceWrite { path, source } => {
                write!(
                    formatter,
                    "trace write failed for {}: {source}",
                    path.display()
                )
            },
        }
    }
}

impl Error for GitTraceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Command(source)
            | Self::TraceRead { source, .. }
            | Self::TraceWrite { source, .. } => Some(source),
            Self::TraceParse { source, .. } => Some(source),
        }
    }
}

/// Capture Git invocations without a shell wrapper.
///
/// # Errors
/// Returns a command, trace read, trace parse, or trace write error.
pub fn capture_git_trace(
    command: &mut FixtureCommand,
    trace_path: &Path,
) -> Result<Output, GitTraceError> {
    let events_path = trace_path.with_extension("events");
    let output = command
        .env(GIT_TRACE2_EVENT_ENVIRONMENT, &events_path)
        .output()
        .map_err(GitTraceError::Command)?;
    let events = match fs::read_to_string(&events_path) {
        Ok(events) => events,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(GitTraceError::TraceRead {
                path: events_path,
                source,
            });
        },
    };
    let mut commands = String::new();
    for line in events.lines() {
        let parse_error = |source| GitTraceError::TraceParse {
            path: events_path.clone(),
            source,
        };
        let event: Value = serde_json::from_str(line).map_err(parse_error)?;
        if event["event"] != "start" {
            continue;
        }
        let Some(arguments) = event["argv"].as_array() else {
            continue;
        };
        if arguments.get(1).and_then(serde_json::Value::as_str) != Some(GIT_NO_OPTIONAL_LOCKS_ARG) {
            continue;
        }
        let arguments: Vec<String> =
            serde_json::from_value(Value::Array(arguments[2..].to_vec())).map_err(parse_error)?;
        commands.push_str(&arguments.join(" "));
        commands.push('\n');
    }
    fs::write(trace_path, commands).map_err(|source| GitTraceError::TraceWrite {
        path: trace_path.to_path_buf(),
        source,
    })?;
    Ok(output)
}
