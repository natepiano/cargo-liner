//! Child processes for the reader fixtures.

use std::error::Error;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::process::Stdio;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::TestProcessGroup;
use cargo_berth_test_support::fixture_git_command;

/// A fixture result with its original error.
pub(crate) type TestResult<T> = Result<T, Box<dyn Error>>;

/// The explicit permission to read the Python checkout.
pub(crate) enum ParityCheckout {
    Enabled(PathBuf),
    Disabled,
}

/// Read the checkout setting before a parity test creates its fixtures.
pub(crate) fn parity_checkout() -> ParityCheckout {
    std::env::var_os("CARGO_STYLE_PARITY_CHECKOUT").map_or_else(
        || {
            eprintln!(
                "Parity test skipped: set CARGO_STYLE_PARITY_CHECKOUT to the Python checkout."
            );
            ParityCheckout::Disabled
        },
        |path| ParityCheckout::Enabled(path.into()),
    )
}

/// Supply input and collect output through the shared process owner.
pub(crate) fn output_with_input(command: &mut FixtureCommand, input: &[u8]) -> TestResult<Output> {
    Ok(TestProcessGroup::spawn(
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?
    .output_with_input(input)?)
}

/// Run one Git operation and return its text output.
pub(crate) fn git(repository: &Path, arguments: &[&str]) -> TestResult<String> {
    git_input(repository, arguments, &[])
}

/// Run one Git operation with fixture bytes on stdin.
pub(crate) fn git_input(repository: &Path, arguments: &[&str], input: &[u8]) -> TestResult<String> {
    let output = output_with_input(
        fixture_git_command()
            .arg("-C")
            .arg(repository)
            .args(arguments),
        input,
    )?;
    assert!(
        output.status.success(),
        "Git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}
