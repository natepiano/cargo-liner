//! The command line, which is the grid unless it says otherwise.
//!
//! Running `cargo-tile` with nothing after it opens the grid, because
//! that is what the tool is for. The subcommands are for the capture
//! shim: the grid stands it up itself as it opens, but taking it out
//! again is never automatic, and a machine with `capture.auto_install`
//! off in `config.toml` needs a way to put it in by name.
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
    /// name. Safe to repeat. The grid does the same as it opens unless
    /// `capture.auto_install` is off in `config.toml`.
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
            Some(Command::Install) => {
                if let Err(error) = install() {
                    eprintln!("{BINARY_NAME}: {error}");
                }
                // Runner job-start hooks must not fail a job when capture
                // setup fails; the diagnostic is the install failure report.
                ExitCode::SUCCESS
            },
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

/// Put the shim in front of every toolchain's cargo, reporting each
/// failure without preventing the remaining toolchains from installing.
fn install() -> io::Result<()> {
    let hooks = Hook::all()?;
    for hook in &hooks {
        match hook.ensure() {
            Ok(change) => println!("{}: {}", hook.name(), describe(change)),
            Err(error) => eprintln!("{BINARY_NAME}: {}: {error}", hook.name()),
        }
    }
    if hooks.is_empty() {
        println!("no rustup toolchains found, so there is no cargo to stand in front of");
    } else {
        println!("\nToolchains with a working shim report progress for new runs. Existing runs");
        println!("cannot be captured -- their output belongs to the terminal that started");
        println!("them -- so they show in the grid without a bar until they are run again.");
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
            HookState::Repairable => {
                "interrupted install -- run cargo-tile install to restore cargo"
            },
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
        Change::Refreshed => "capture shim updated",
        Change::AlreadyCurrent => "capture shim already current, unchanged",
        Change::Removed => "capture shim removed",
        Change::AlreadyAbsent => "no capture shim to remove",
        Change::Orphaned => "broken -- shim installed but the real cargo is missing",
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process;

    use tempfile::tempdir;

    use super::*;
    use crate::constants::CARGO_NAME;
    use crate::constants::REAL_CARGO_NAME;
    use crate::constants::RUSTUP_HOME_ENV;
    use crate::constants::SHIM_STAGING_NAME;
    use crate::constants::TOOLCHAIN_BIN_DIR;
    use crate::constants::TOOLCHAINS_DIR;

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

    /// Child processes isolate rustup discovery from the test runner's
    /// environment and return the same exit code as the binary's main.
    #[test]
    fn a_failed_install_exits_successfully() -> ExitCode {
        if env::var_os("CARGO_TILE_TEST_FAILED_INSTALL").is_some() {
            return Cli::parse_from([BINARY_NAME, "install"]).run();
        }

        let rustup_home = tempdir().expect("temporary rustup home");
        let toolchains = rustup_home.path().join(TOOLCHAINS_DIR);
        let blocked = toolchains.join("blocked").join(TOOLCHAIN_BIN_DIR);
        let working = toolchains.join("working").join(TOOLCHAIN_BIN_DIR);
        fs::create_dir_all(blocked.join(SHIM_STAGING_NAME))
            .expect("block shim staging with a directory");
        fs::create_dir_all(&working).expect("working toolchain directory");
        fs::write(blocked.join(CARGO_NAME), "real cargo").expect("blocked toolchain cargo");
        fs::write(working.join(CARGO_NAME), "real cargo").expect("working toolchain cargo");

        let output = install_in_child(rustup_home.path());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert!(
            stderr.contains(&format!("{BINARY_NAME}: blocked:")),
            "{stderr}"
        );
        assert!(
            String::from_utf8_lossy(&output.stdout).contains("working: capture shim installed")
        );
        assert_eq!(
            fs::read(working.join(REAL_CARGO_NAME)).expect("saved cargo"),
            b"real cargo"
        );
        ExitCode::SUCCESS
    }

    /// Discovery fails before any per-toolchain install can handle errors.
    #[test]
    fn a_toolchain_discovery_failure_exits_successfully() {
        let rustup_home = tempdir().expect("rustup home with no toolchains directory");
        let output = install_in_child(rustup_home.path());
        let stderr = String::from_utf8_lossy(&output.stderr);

        assert_eq!(output.status.code(), Some(0), "{stderr}");
        assert!(stderr.contains(&format!("{BINARY_NAME}:")), "{stderr}");
    }

    /// Re-enter the install test with an isolated environment so the child
    /// returns [`Cli::run`]'s exit code without changing the parent's rustup.
    fn install_in_child(rustup_home: &Path) -> process::Output {
        process::Command::new(env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "cli::tests::a_failed_install_exits_successfully",
                "--nocapture",
            ])
            .env("CARGO_TILE_TEST_FAILED_INSTALL", "1")
            .env(RUSTUP_HOME_ENV, rustup_home)
            .output()
            .expect("install child process")
    }
}
