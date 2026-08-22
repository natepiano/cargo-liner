//! The command line, which is the grid unless it says otherwise.
//!
//! Running `cargo-tile` with nothing after it opens the grid, because
//! that is what the tool is for. The subcommands exist for the one thing
//! the grid cannot do for itself: putting the capture shim in front of
//! cargo, which rewrites files in every rustup toolchain and so is only
//! ever done when it is asked for by name.
//!
//! Both spellings reach the same place. `cargo tile` works because cargo
//! runs any `cargo-`-prefixed binary on the path as a subcommand of its
//! own, and [`Cli::parse_arguments`] takes the extra word cargo hands
//! over so that `cargo tile install` and `cargo-tile install` parse
//! alike.

use std::env;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io;
use std::process::ExitCode;

use clap::Parser;
use clap::Subcommand;

use crate::constants::BINARY_NAME;
use crate::constants::SUBCOMMAND_NAME;
use crate::hook::Change;
use crate::hook::Hook;
use crate::hook::HookState;
use crate::terminal;

/// `cargo-tile`, as the command line sees it.
#[derive(Debug, Parser)]
#[command(name = BINARY_NAME, version, about = "Watch the cargo runs on this machine")]
pub(crate) struct Cli {
    /// What to do instead of opening the grid.
    #[command(subcommand)]
    command: Option<Command>,
}

/// The things `cargo-tile` does other than open the grid.
#[derive(Debug, Eq, PartialEq, Subcommand)]
enum Command {
    /// Put the capture shim in front of cargo, so runs report progress.
    ///
    /// Each toolchain's real cargo is moved aside and the shim takes its
    /// name. Safe to repeat: it is also how the hook is repaired after
    /// `rustup update` replaces it.
    Install,
    /// Take the capture shim back out and give cargo its name back.
    Uninstall,
    /// Report whether the capture shim is installed, toolchain by
    /// toolchain.
    Status,
}

impl Cli {
    /// Read the command line, however this tool was reached.
    pub(crate) fn parse_arguments() -> Self {
        Self::parse_from(without_subcommand_name(env::args_os().collect()))
    }

    /// Do what the command line asked for.
    pub(crate) fn run(self) -> ExitCode {
        match self.command {
            None => terminal::run(),
            Some(Command::Install) => report(install()),
            Some(Command::Uninstall) => report(uninstall()),
            Some(Command::Status) => report(status()),
        }
    }
}

/// The arguments with cargo's echo of the subcommand name taken out.
///
/// Only the word directly after the binary counts. Further along it is
/// an argument like any other, and a `tile` there belongs to whoever
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

/// Print what went wrong, if anything, and turn it into an exit code.
fn report(outcome: io::Result<()>) -> ExitCode {
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{BINARY_NAME}: {error}");
            ExitCode::FAILURE
        },
    }
}

/// Put the shim in front of every toolchain's cargo.
fn install() -> io::Result<()> {
    let hooks = Hook::all()?;
    for hook in &hooks {
        let change = hook.install()?;
        println!("{}: {}", hook.name(), describe(change));
    }
    if hooks.is_empty() {
        println!("no rustup toolchains found, so there is no cargo to stand in front of");
    } else {
        println!("\nRuns started from now on report progress. Ones already going cannot be");
        println!("captured -- a running process's output belongs to the terminal that started");
        println!("it -- so they show in the grid without a bar until they are run again.");
    }
    Ok(())
}

/// Take the shim back out of every toolchain.
fn uninstall() -> io::Result<()> {
    for hook in &Hook::all()? {
        let change = hook.remove()?;
        println!("{}: {}", hook.name(), describe(change));
    }
    Ok(())
}

/// Report what stands in front of each toolchain's cargo.
fn status() -> io::Result<()> {
    for hook in &Hook::all()? {
        let state = match hook.state() {
            HookState::Installed => "capturing",
            HookState::Absent => "not installed",
            HookState::Orphaned => "broken -- shim installed but the real cargo is missing",
        };
        println!("{}: {state}", hook.name());
    }
    Ok(())
}

/// What one toolchain's outcome reads as.
const fn describe(change: Change) -> &'static str {
    match change {
        Change::Installed => "capture shim installed",
        Change::Refreshed => "capture shim already installed, rewritten",
        Change::Removed => "capture shim removed",
        Change::AlreadyAbsent => "no capture shim to remove",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The command line as cargo would hand it over, or as a shell would.
    fn parse(arguments: &[&str]) -> Option<Command> {
        Cli::parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .command
    }

    #[test]
    fn the_binary_on_its_own_opens_the_grid() {
        assert!(parse(&[BINARY_NAME]).is_none());
    }

    /// Cargo runs `cargo tile` by handing this binary its own subcommand
    /// name ahead of everything else, so the grid has to open either way.
    #[test]
    fn reached_as_a_cargo_subcommand_the_grid_still_opens() {
        assert!(parse(&[BINARY_NAME, SUBCOMMAND_NAME]).is_none());
    }

    #[test]
    fn both_spellings_of_a_subcommand_ask_for_the_same_thing() {
        assert_eq!(
            parse(&[BINARY_NAME, "install"]),
            parse(&[BINARY_NAME, SUBCOMMAND_NAME, "install"])
        );
    }

    /// Dropping every `tile` rather than only cargo's would eat an
    /// argument the caller meant.
    #[test]
    fn the_subcommand_name_further_along_is_left_where_it_is() {
        let arguments = ["cargo-tile", "install", "tile"]
            .map(OsString::from)
            .to_vec();

        assert_eq!(without_subcommand_name(arguments.clone()), arguments);
    }
}
