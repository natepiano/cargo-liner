//! Child processes for the reader fixtures.

pub(crate) mod process;
#[path = "../../src/state_directory.rs"]
mod state_directory;

use std::path::Path;
use std::path::PathBuf;
use std::process::Output;

use cargo_berth_test_support::FixtureCommand;
use cargo_berth_test_support::fixture_tempdir;
pub(crate) use process::TestResult;
pub(crate) use process::git_input;
pub(crate) use state_directory::STATE_DIRECTORY;

/// The explicit permission to read the Python checkout.
pub(crate) enum ParityCheckout {
    Enabled(PathBuf),
    Disabled,
}

/// Read the checkout setting before a parity test creates its fixtures.
pub(crate) fn parity_checkout() -> ParityCheckout {
    std::env::var_os("STYLE_PARITY_CHECKOUT").map_or_else(
        || {
            eprintln!("Parity test skipped: set STYLE_PARITY_CHECKOUT to the Python checkout.");
            ParityCheckout::Disabled
        },
        |path| ParityCheckout::Enabled(path.into()),
    )
}

/// Keep the parity reader state in a private directory.
pub(crate) fn output_with_input(command: &mut FixtureCommand, input: &[u8]) -> TestResult<Output> {
    let state = fixture_tempdir("cargo-style-parity-state-");
    Ok(process::start(command.env(STATE_DIRECTORY, state.path()))?.output_with_input(input)?)
}

/// Run one Git operation and return its text output.
pub(crate) fn git(repository: &Path, arguments: &[&str]) -> TestResult<String> {
    git_input(repository, arguments, &[])
}
