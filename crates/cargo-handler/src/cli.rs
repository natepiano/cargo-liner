//! The command line, which opens the grid and takes nothing else yet.
//!
//! Both spellings reach the same place. `cargo handler` works because
//! cargo runs any `cargo-`-prefixed binary on the path as a subcommand
//! of its own, and [`Cli::parse_arguments`] takes the extra word cargo
//! hands over so that `cargo handler` and `cargo-handler` parse alike.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;

use crate::constants::BINARY_NAME;
use crate::constants::CLI_ABOUT;
use crate::constants::SUBCOMMAND_NAME;
use crate::terminal;

/// `cargo-handler`, as the command line sees it.
///
/// No options yet: parsing is what answers `--help` and `--version`
/// and turns away anything else.
#[derive(Debug, Parser)]
#[command(name = BINARY_NAME, version, about = CLI_ABOUT, long_about = None)]
pub(crate) struct Cli {}

impl Cli {
    /// Read the command line, however this tool was reached.
    pub(crate) fn parse_arguments() -> Self {
        Self::parse_from(without_subcommand_name(env::args_os().collect()))
    }

    /// Do what the command line asked for, which is always to open the
    /// grid.
    pub(crate) fn run(self) -> ExitCode {
        // Destructured so an option added to `Cli` has to be handled here.
        let Self {} = self;
        terminal::run()
    }
}

/// The arguments with cargo's echo of the subcommand name taken out.
///
/// Only the word directly after the binary counts. Further along it is
/// an argument like any other, and a `handler` there belongs to whoever
/// wrote it.
fn without_subcommand_name(mut arguments: Vec<OsString>) -> Vec<OsString> {
    if arguments
        .get(1)
        .is_some_and(|word| word.as_os_str() == OsStr::new(SUBCOMMAND_NAME))
    {
        arguments.remove(1);
    }
    arguments
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::process::Command;

    use super::*;

    /// Whether the command line parses as cargo or a shell would hand
    /// it over.
    fn parses(arguments: &[&str]) -> bool {
        Cli::try_parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .is_ok()
    }

    #[test]
    fn the_binary_on_its_own_parses() {
        assert!(parses(&[BINARY_NAME]));
    }

    /// Cargo runs `cargo handler` by handing this binary its own
    /// subcommand name ahead of everything else, so that has to parse
    /// the same as the bare binary.
    #[test]
    fn reached_as_a_cargo_subcommand_it_still_parses() {
        assert!(parses(&[BINARY_NAME, SUBCOMMAND_NAME]));
    }

    /// Dropping every `handler` rather than only cargo's would eat an
    /// argument the caller meant.
    #[test]
    fn the_subcommand_name_further_along_is_left_where_it_is() {
        let arguments = [BINARY_NAME, "--verbose", SUBCOMMAND_NAME]
            .map(OsString::from)
            .to_vec();

        assert_eq!(without_subcommand_name(arguments.clone()), arguments);
        assert!(!parses(&[BINARY_NAME, "--verbose", SUBCOMMAND_NAME]));
    }

    /// Reject libtest flags through the real command-line parser, in a
    /// child process so clap's exit is the child's.
    #[test]
    fn cli_rejects_unknown_process_arguments() -> io::Result<()> {
        if env::var_os("CARGO_HANDLER_TEST_ARGUMENTS").is_some() {
            // Libtest accepts --exact; cargo-handler must reject it
            // before opening the terminal.
            assert_eq!(Cli::parse_arguments().run(), ExitCode::FAILURE);
            return Ok(());
        }
        let output = Command::new(env::current_exe()?)
            .args([
                "--exact",
                "cli::tests::cli_rejects_unknown_process_arguments",
                "--nocapture",
            ])
            .env("CARGO_HANDLER_TEST_ARGUMENTS", "1")
            .output()?;
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(error.contains("unexpected argument '--exact'"), "{error}");
        Ok(())
    }
}
