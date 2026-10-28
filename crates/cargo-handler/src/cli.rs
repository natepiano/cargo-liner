//! `Cli::run` calls `terminal::run` to open the grid. With the hidden
//! `Command::Probe` subcommand, it calls `probe::print` to print this
//! machine's agents as JSON for a remote summary to read.
//!
//! `cargo handler` and `cargo-handler` both use `Cli::parse_arguments`.
//! Cargo runs any `cargo-`-prefixed binary on the path as a subcommand.
//! `Cli::parse_arguments` uses `without_subcommand_name` to remove Cargo's
//! extra argument, so both forms parse alike.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use crate::census::probe;
use crate::constants::BINARY_NAME;
use crate::constants::CLI_ABOUT;
use crate::constants::PROBE_COMMAND;
use crate::constants::SUBCOMMAND_NAME;
use crate::terminal;

/// `Cli` defines the `cargo-handler` command line for clap parsing.
///
/// It has no options. Clap handles `--help` and `--version`, accepts the
/// hidden `Command::Probe` subcommand, and rejects anything else.
#[derive(Debug, Parser)]
#[command(name = BINARY_NAME, version, about = CLI_ABOUT, long_about = None)]
pub(crate) struct Cli {
    /// What to run in place of the grid.
    #[command(subcommand)]
    command: Option<Command>,
}

/// The subcommands, all hidden: each is for another cargo-handler to
/// run, not for a person.
#[derive(Debug, Subcommand)]
enum Command {
    /// Print this machine's top-level agents as JSON and exit.
    #[command(name = PROBE_COMMAND, hide = true)]
    Probe,
}

impl Cli {
    /// Read the command line, however this tool was reached.
    pub(crate) fn parse_arguments() -> Self {
        Self::parse_from(without_subcommand_name(env::args_os().collect()))
    }

    /// `Cli::run` calls `terminal::run` to open the grid when no command is set.
    /// `Command::Probe` selects `probe::print` to print the probe.
    pub(crate) fn run(self) -> ExitCode {
        // Destructured so an option added to `Cli` has to be handled here.
        let Self { command } = self;
        match command {
            None => terminal::run(),
            Some(Command::Probe) => probe::print(),
        }
    }
}

/// `without_subcommand_name` removes `arguments[1]` only when it equals
/// `SUBCOMMAND_NAME`, the subcommand name Cargo inserts after the binary.
///
/// Later `handler` arguments belong to the caller, like any other argument,
/// so `without_subcommand_name` preserves them.
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

    use clap::CommandFactory;

    use super::*;

    /// `parses` normalizes Cargo or shell arguments with `without_subcommand_name`
    /// and checks whether `Cli::try_parse_from` accepts them.
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

    /// `Cli::try_parse_from` accepts direct and Cargo-prefixed probe arguments
    /// after `without_subcommand_name` normalization. `--help` omits `super::Command::Probe`.
    #[test]
    fn the_probe_parses_and_stays_out_of_help() {
        assert!(parses(&[BINARY_NAME, PROBE_COMMAND]));
        assert!(parses(&[BINARY_NAME, SUBCOMMAND_NAME, PROBE_COMMAND]));
        let help = Cli::command().render_help().to_string();
        assert!(!help.contains(PROBE_COMMAND), "{help}");
    }

    /// `without_subcommand_name` preserves later `SUBCOMMAND_NAME` arguments after
    /// index-1 normalization. Removing them would discard arguments the caller intended.
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
