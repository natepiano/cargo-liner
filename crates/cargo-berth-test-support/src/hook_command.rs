//! Hook commands shared by acceptance tests and recorded fixture builders.

use std::error::Error;
use std::path::Path;
use std::process::Output;
use std::process::Stdio;

use crate::TestProcessGroup;
use crate::berth_command;

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

/// Whether the hook process inherits a harness session identity from its environment.
pub enum AmbientHarnessSession<'session> {
    /// The environment names a harness session the hook must not adopt.
    Present(&'session str),
    /// The environment names no harness session.
    Absent,
}

/// Run a hook with the payload on standard input.
///
/// # Errors
/// Returns a process error or a pipe error.
pub fn spawn_hook_verb(
    executable: &Path,
    working_directory: &Path,
    hook_event: &str,
    stdin: &[u8],
    ambient_session: &AmbientHarnessSession<'_>,
) -> TestResult<Output> {
    spawn_hook_verb_for_discovery(
        executable,
        working_directory,
        hook_event,
        stdin,
        ambient_session,
        RepositoryDiscovery::Inherited,
    )
}

/// Run a hook with a repository discovery ceiling.
///
/// # Errors
/// Returns a process error or a pipe error.
pub fn spawn_hook_verb_with_ceiling(
    executable: &Path,
    working_directory: &Path,
    hook_event: &str,
    stdin: &[u8],
    ambient_session: &AmbientHarnessSession<'_>,
    ceiling: &Path,
) -> TestResult<Output> {
    spawn_hook_verb_for_discovery(
        executable,
        working_directory,
        hook_event,
        stdin,
        ambient_session,
        RepositoryDiscovery::Ceiling(ceiling),
    )
}

#[derive(Clone, Copy)]
enum RepositoryDiscovery<'ceiling> {
    Inherited,
    Ceiling(&'ceiling Path),
}

fn spawn_hook_verb_for_discovery(
    executable: &Path,
    working_directory: &Path,
    hook_event: &str,
    stdin: &[u8],
    ambient_session: &AmbientHarnessSession<'_>,
    discovery: RepositoryDiscovery<'_>,
) -> TestResult<Output> {
    let mut command = berth_command(executable);
    command
        .args(["hook", hook_event])
        .current_dir(working_directory)
        .env_remove("CARGO_BERTH_RUN");
    if let RepositoryDiscovery::Ceiling(ceiling) = discovery {
        command.env("GIT_CEILING_DIRECTORIES", ceiling);
    }
    match *ambient_session {
        AmbientHarnessSession::Present(session_id) => {
            command.env("CARGO_BERTH_SESSION_ID", session_id);
        },
        AmbientHarnessSession::Absent => {
            command.env_remove("CARGO_BERTH_SESSION_ID");
        },
    }
    let child = TestProcessGroup::spawn(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    Ok(child.output_with_input(stdin)?)
}
