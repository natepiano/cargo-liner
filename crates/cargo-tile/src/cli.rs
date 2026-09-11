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
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use clap::Args;
use clap::Parser;
use clap::Subcommand;
use rustix::process::geteuid;

use crate::capture_root;
use crate::constants::ACCOUNT_HOOK_REPORT_FLAG;
use crate::constants::BINARY_NAME;
use crate::constants::CAPTURE_ROOT;
use crate::constants::SUBCOMMAND_NAME;
use crate::hook;
use crate::hook::AccountHookOutcome;
use crate::hook::Hook;
use crate::hook::HookOperation;
use crate::hook::HookOperationOutcome;
use crate::hook::HookState;
use crate::hook::ToolchainHookOutcome;
use crate::hook::ToolchainHookReport;
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
    Install(HookArguments),
    /// Take the capture shim back out and give cargo its name back.
    Uninstall(HookArguments),
    /// Report whether the capture shim is installed, toolchain by
    /// toolchain.
    Status(HookArguments),
}

/// Clap's flags selecting whose toolchains to handle and how to report them.
#[derive(Args, Debug, Eq, PartialEq)]
struct HookArguments {
    /// Handle every account's default rustup home (requires sudo).
    #[arg(long)]
    all_accounts:        bool,
    /// Report each toolchain to the administrative parent process.
    #[arg(long = ACCOUNT_HOOK_REPORT_FLAG, hide = true, conflicts_with = "all_accounts")]
    account_hook_report: bool,
}

/// The requested action after converting Clap's optional subcommand and flags.
#[derive(Debug, Eq, PartialEq)]
enum RequestedAction {
    /// Open the live cargo grid.
    Grid,
    /// Handle the caller's toolchains with ordinary CLI output.
    Local(HookOperation),
    /// Handle database accounts through credential-switched children.
    AllAccounts(HookOperation),
    /// Emit the per-toolchain protocol for an administrative parent.
    AccountReport(HookOperation),
}

impl From<Cli> for RequestedAction {
    fn from(cli: Cli) -> Self {
        let (operation, arguments) = match cli.command {
            None => return Self::Grid,
            Some(Command::Install(arguments)) => (HookOperation::Install, arguments),
            Some(Command::Uninstall(arguments)) => (HookOperation::Uninstall, arguments),
            Some(Command::Status(arguments)) => (HookOperation::Status, arguments),
        };
        match arguments {
            HookArguments {
                all_accounts: true, ..
            } => Self::AllAccounts(operation),
            HookArguments {
                account_hook_report: true,
                ..
            } => Self::AccountReport(operation),
            HookArguments {
                all_accounts: false,
                account_hook_report: false,
            } => Self::Local(operation),
        }
    }
}

impl Cli {
    /// Read the command line, however this tool was reached.
    pub(crate) fn parse_arguments() -> Self {
        Self::parse_from(without_subcommand_name(env::args_os().collect()))
    }

    /// Do what the command line asked for.
    pub(crate) fn run(self) -> ExitCode {
        match RequestedAction::from(self) {
            RequestedAction::Grid => terminal::run(),
            RequestedAction::AccountReport(operation) => report(account_hook_report(operation)),
            RequestedAction::AllAccounts(operation) => report(all_accounts(operation)),
            RequestedAction::Local(HookOperation::Install) => {
                if let Err(error) = install() {
                    eprintln!("{BINARY_NAME}: {error}");
                }
                // Runner job-start hooks must not fail a job when capture
                // setup fails; the diagnostic is the install failure report.
                ExitCode::SUCCESS
            },
            RequestedAction::Local(HookOperation::Uninstall) => report(uninstall()),
            RequestedAction::Local(HookOperation::Status) => report(status()),
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
    if let Err(error) = capture_root::prepare_shared_directory(Path::new(CAPTURE_ROOT)) {
        eprintln!("{BINARY_NAME}: {CAPTURE_ROOT}: {error}");
    }
    let toolchains = Hook::reports(HookOperation::Install)?
        .inspect(print_local_report)
        .count();
    if toolchains == 0 {
        println!("no rustup toolchains found, so there is no cargo to stand in front of");
    } else {
        println!("\nToolchains with a working shim report progress for new runs. Existing runs");
        println!("cannot be captured -- their output belongs to the terminal that started");
        println!("them -- so they show in the grid without a bar until they are run again.");
    }
    Ok(())
}

/// Report account-owned operations without shared-directory setup or ordinary CLI prose.
/// Discovery or output failure fails the child; each row carries its operation result.
fn account_hook_report(operation: HookOperation) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    for report in Hook::reports(operation)? {
        writeln!(stdout, "{}", report.protocol())?;
        stdout.flush()?;
    }
    Ok(())
}

/// Run one operation for database accounts after checking administrative privileges.
fn all_accounts(operation: HookOperation) -> io::Result<()> {
    if !geteuid().is_root() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!(
                "run sudo cargo tile {} --all-accounts",
                operation.subcommand()
            ),
        ));
    }
    let accounts = hook::system_accounts()?;
    if operation == HookOperation::Install {
        capture_root::prepare_shared_directory(Path::new(CAPTURE_ROOT))?;
    }
    let parent = Path::new(CAPTURE_ROOT)
        .parent()
        .ok_or_else(|| io::Error::other(format!("{CAPTURE_ROOT} has no parent directory")))?;
    let staged = hook::stage_executable(parent, &env::current_exe()?)?;
    let reports = hook::run_account_hooks(&accounts, staged.path(), operation);
    let mut completed = 0;
    let mut no_toolchains = 0;
    let mut incomplete = 0;
    for report in &reports {
        println!("{report}");
        match &report.outcome {
            AccountHookOutcome::Completed => completed += 1,
            AccountHookOutcome::NoToolchains => no_toolchains += 1,
            AccountHookOutcome::Incomplete(_) => incomplete += 1,
        }
    }
    println!(
        "{} accounts: {completed} completed, {no_toolchains} with no toolchains, {incomplete} incomplete",
        reports.len()
    );
    operation.completion(&reports)
}

/// Attempt every removal and fail the command if any toolchain could not be restored.
fn uninstall() -> io::Result<()> {
    let reports: Vec<_> = Hook::reports(HookOperation::Uninstall)?
        .inspect(print_local_report)
        .collect();
    let failed = reports
        .iter()
        .filter(|report| report.outcome.is_incomplete())
        .count();
    if failed > 0 {
        return Err(io::Error::other(format!(
            "capture shim removal failed for {failed} toolchain(s)"
        )));
    }
    Ok(())
}

/// Report every state without locks, writes, or capture-directory preparation.
fn status() -> io::Result<()> {
    let reports: Vec<_> = Hook::reports(HookOperation::Status)?
        .inspect(print_local_report)
        .collect();
    if reports.iter().any(|report| report.outcome.is_incomplete()) {
        return Err(io::Error::other("capture shim status is incomplete"));
    }
    if reports.is_empty() {
        println!("no rustup toolchains found");
    }
    Ok(())
}

/// Keep ordinary CLI descriptions while account children use the shared protocol.
fn print_local_report(report: &ToolchainHookReport) {
    let name = &report.toolchain;
    let description = match &report.outcome {
        ToolchainHookOutcome::Install(outcome) | ToolchainHookOutcome::Uninstall(outcome) => {
            describe(*outcome)
        },
        ToolchainHookOutcome::Status(state) => match state {
            HookState::Installed => "capturing",
            HookState::Absent => "not installed",
            HookState::Repairable => {
                "interrupted install -- run cargo-tile install to restore cargo"
            },
            HookState::Orphaned => "broken -- shim installed but the real cargo is missing",
        }
        .to_owned(),
        ToolchainHookOutcome::Unreadable(_) | ToolchainHookOutcome::Failed(_) => {
            eprintln!("{BINARY_NAME}: {report}");
            return;
        },
    };
    if matches!(
        report.outcome,
        ToolchainHookOutcome::Uninstall(HookOperationOutcome::Orphaned)
    ) {
        eprintln!("{BINARY_NAME}: {name}: {description}");
    } else {
        println!("{name}: {description}");
    }
}

/// What one toolchain's mutation outcome reads as.
fn describe(outcome: HookOperationOutcome) -> String {
    match outcome {
        HookOperationOutcome::Installed => "capture shim installed",
        HookOperationOutcome::Refreshed => "capture shim updated",
        HookOperationOutcome::AlreadyCurrent => "capture shim already current, unchanged",
        HookOperationOutcome::DowngradeRefused { installed, supported } => {
            return format!(
                "downgrade refused -- newer shim v{installed} kept; this reader supports v{supported}; upgrade and restart the reader"
            );
        },
        HookOperationOutcome::Removed => "capture shim removed",
        HookOperationOutcome::AlreadyAbsent => "no capture shim to remove",
        HookOperationOutcome::Orphaned => "broken -- shim installed but the real cargo is missing",
    }.to_owned()
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
    use std::process::Output;

    use tempfile::tempdir;

    use super::*;
    use crate::constants::CARGO_NAME;
    use crate::constants::REAL_CARGO_NAME;
    use crate::constants::RUSTUP_HOME_ENV;
    use crate::constants::SHIM_STAGING_NAME;
    use crate::constants::TOOLCHAIN_BIN_DIR;
    use crate::constants::TOOLCHAINS_DIR;
    use crate::hook::AccountHookReport;

    /// The command line as cargo would hand it over, or as a shell would.
    fn parse(arguments: &[&str]) -> RequestedAction {
        Cli::parse_from(without_subcommand_name(
            arguments.iter().map(OsString::from).collect(),
        ))
        .into()
    }

    #[test]
    fn the_binary_on_its_own_opens_the_grid() {
        assert_eq!(parse(&[BINARY_NAME]), RequestedAction::Grid);
    }

    /// Cargo runs `cargo tile` by handing this binary its own subcommand
    /// name ahead of everything else, so the grid has to open either way.
    #[test]
    fn reached_as_a_cargo_subcommand_the_grid_still_opens() {
        assert_eq!(
            parse(&[BINARY_NAME, SUBCOMMAND_NAME]),
            RequestedAction::Grid
        );
    }

    #[test]
    fn both_spellings_of_a_subcommand_ask_for_the_same_thing() {
        assert_eq!(
            parse(&[BINARY_NAME, "install"]),
            parse(&[BINARY_NAME, SUBCOMMAND_NAME, "install"])
        );
    }

    #[test]
    fn every_operation_has_explicit_local_admin_and_child_actions() {
        for operation in [
            HookOperation::Install,
            HookOperation::Uninstall,
            HookOperation::Status,
        ] {
            assert_eq!(
                parse(&[BINARY_NAME, operation.subcommand()]),
                RequestedAction::Local(operation)
            );
            assert_eq!(
                parse(&[BINARY_NAME, operation.subcommand(), "--all-accounts"]),
                RequestedAction::AllAccounts(operation)
            );
            let flag = format!("--{ACCOUNT_HOOK_REPORT_FLAG}");
            assert_eq!(
                parse(&[BINARY_NAME, operation.subcommand(), &flag]),
                RequestedAction::AccountReport(operation)
            );
            assert!(
                Cli::try_parse_from([BINARY_NAME, operation.subcommand(), "--all-accounts", &flag])
                    .is_err()
            );
        }
    }

    #[test]
    fn an_account_credential_failure_makes_uninstall_exit_unsuccessfully() {
        let reports = [AccountHookReport {
            account:    "runner".to_owned(),
            operation:  HookOperation::Uninstall,
            outcome:    AccountHookOutcome::Incomplete(
                "could not resolve runner's groups: membership resolver unavailable".to_owned(),
            ),
            toolchains: Vec::new(),
        }];
        assert_eq!(
            report(HookOperation::Uninstall.completion(&reports)),
            ExitCode::FAILURE
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
    fn install_in_child(rustup_home: &Path) -> Output {
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
