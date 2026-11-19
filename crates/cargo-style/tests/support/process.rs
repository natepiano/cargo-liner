//! Git and child process operations for reader tests.

use std::error::Error;
use std::io;
use std::path::Path;
use std::process::Stdio;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::TestProcessGroup;
use cargo_berth_test_support::fixture_git_command;

/// A fixture result with its original error.
pub(crate) type TestResult<T> = Result<T, Box<dyn Error>>;

/// Start a child with pipes for the request and reply.
pub(crate) fn start(command: &mut FixtureCommand) -> io::Result<TestProcessGroup> {
    TestProcessGroup::spawn(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )
}

/// Run one Git operation with fixture bytes on stdin.
pub(crate) fn git_input(repository: &Path, arguments: &[&str], input: &[u8]) -> TestResult<String> {
    let output = start(
        fixture_git_command()
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .arg("-C")
            .arg(repository)
            .args(arguments),
    )?
    .output_with_input(input)?;
    assert!(
        output.status.success(),
        "Git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
