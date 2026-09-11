//! Putting the capture shim in front of each toolchain's cargo, and
//! taking it back out.
//!
//! The grid reads a run's progress out of a log, and something has to
//! write that log. Cargo cannot be asked to -- a process's output
//! belongs to the terminal it was started from, and the runs in the grid
//! were started from other terminals. So the shim takes the place of
//! each toolchain's `cargo`, with the real binary kept beside it under
//! [`REAL_CARGO_NAME`], and mirrors what it runs.
//!
//! Standing in front of every cargo invocation on the machine is not
//! something to do quietly. The grid does it when it opens, through
//! [`at_startup`], and says so on screen every time it changes
//! anything; `config.toml` can turn that off, and `cargo tile install`
//! does the same by name. Two properties make it safe to live with: the
//! real binary is only ever moved, never written over or removed, and a
//! `cargo` without [`SHIM_MARKER`] in it is treated as the real one no
//! matter what is beside it -- which is what makes installing twice
//! harmless and repairs the hook after `rustup update` puts a fresh
//! cargo back.
//!
//! Taking the shim out is never automatic. Runs started from other
//! terminals need it whether or not a grid is open.

use std::env;
use std::ffi::CStr;
use std::ffi::CString;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::fs::OpenOptions;
use std::io;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::thread;

use crate::constants::ACCOUNT_EXECUTABLE_MODE;
use crate::constants::ACCOUNT_EXECUTABLE_PREFIX;
use crate::constants::ACCOUNT_GROUPS_GROWTH_FACTOR;
use crate::constants::ACCOUNT_GROUPS_INITIAL_CAPACITY;
use crate::constants::ACCOUNT_GROUPS_MAX_CAPACITY;
use crate::constants::ACCOUNT_HOOK_REPORT_FLAG;
use crate::constants::BINARY_NAME;
use crate::constants::CARGO_NAME;
use crate::constants::REAL_CARGO_NAME;
use crate::constants::RUSTUP_DIRNAME;
use crate::constants::RUSTUP_HOME_ENV;
use crate::constants::SHIM_LOCK_NAME;
use crate::constants::SHIM_LOCK_RECOVERY;
use crate::constants::SHIM_LOCK_RETRY_ATTEMPTS;
use crate::constants::SHIM_LOCK_RETRY_DELAY;
use crate::constants::SHIM_MARKER;
use crate::constants::SHIM_MARKER_SEARCH_BYTES;
use crate::constants::SHIM_MODE;
use crate::constants::SHIM_STAGING_NAME;
use crate::constants::SHIM_VERSION_PREFIX;
use crate::constants::SUPPORTED_REGISTRATION_VERSION;
use crate::constants::TOOLCHAIN_BIN_DIR;
use crate::constants::TOOLCHAINS_DIR;

/// The shim script, compiled in so the binary carries everything it
/// installs and a copy on disk can never drift from it.
const SHIM_SOURCE: &str = include_str!("cargo-capture-shim.sh");

/// The account database identity whose default rustup home is inspected.
#[derive(Debug)]
pub(crate) struct HookAccount {
    /// The system database's display name.
    pub(crate) name: String,
    /// User identity under which the account child runs.
    pub(crate) uid:  u32,
    /// Primary group under which the account child runs.
    pub(crate) gid:  u32,
    /// The home recorded in the account database, independent of HOME.
    pub(crate) home: PathBuf,
}

/// The operation performed by each account child.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookOperation {
    /// Install or refresh capture shims.
    Install,
    /// Restore the saved cargo binaries.
    Uninstall,
    /// Inspect hook state without changing toolchains or capture storage.
    Status,
}

impl HookOperation {
    /// The CLI subcommand passed to the account child.
    pub(crate) const fn subcommand(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Uninstall => "uninstall",
            Self::Status => "status",
        }
    }

    /// Whether every requested removal or inspection completed.
    /// Installation failures remain diagnostics for runner job-start hooks.
    pub(crate) fn completion(self, reports: &[AccountHookReport]) -> io::Result<()> {
        let incomplete = reports
            .iter()
            .filter(|report| matches!(report.outcome, AccountHookOutcome::Incomplete(_)))
            .count();
        if self != Self::Install && incomplete > 0 {
            return Err(io::Error::other(format!(
                "{} incomplete for {incomplete} account(s)",
                self.subcommand()
            )));
        }
        Ok(())
    }
}

/// Whether handling an account's toolchains completed.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum AccountHookOutcome {
    /// Discovery completed and found no cargo binaries.
    NoToolchains,
    /// Every discovered toolchain was handled for the requested operation.
    Completed,
    /// Discovery, credentials, child execution, or a toolchain operation failed.
    Incomplete(String),
}

/// The operation's result for one toolchain, including retained inspection errors.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ToolchainHookOutcome {
    /// Installation's effect on the shim.
    Install(HookOperationOutcome),
    /// Removal's effect on the shim.
    Uninstall(HookOperationOutcome),
    /// The observed hook state.
    Status(HookState),
    /// Inspection failed before a hook state could be established.
    Unreadable(String),
    /// The requested operation failed with this reason.
    Failed(String),
}

impl ToolchainHookOutcome {
    /// Encode the child protocol without allowing names or errors to create extra rows.
    fn protocol(&self) -> String {
        match self {
            Self::Install(outcome) | Self::Uninstall(outcome) => match outcome {
                HookOperationOutcome::Installed => "installed",
                HookOperationOutcome::Refreshed => "refreshed",
                HookOperationOutcome::AlreadyCurrent => "already installed",
                HookOperationOutcome::Removed => "removed",
                HookOperationOutcome::AlreadyAbsent => "absent",
                HookOperationOutcome::Orphaned => "orphaned",
                HookOperationOutcome::DowngradeRefused {
                    installed,
                    supported,
                } => {
                    return format!("downgrade refused\t{installed}\t{supported}");
                },
            }
            .to_owned(),
            Self::Status(state) => match state {
                HookState::Installed => "installed",
                HookState::Absent => "absent",
                HookState::Repairable => "repairable",
                HookState::Orphaned => "orphaned",
            }
            .to_owned(),
            Self::Unreadable(reason) => format!("unreadable\t{}", protocol_field(reason)),
            Self::Failed(reason) => format!("error\t{}", protocol_field(reason)),
        }
    }

    /// Failed inspection and unrepaired mutation outcomes leave handling incomplete.
    pub(crate) const fn is_incomplete(&self) -> bool {
        matches!(
            self,
            Self::Unreadable(_)
                | Self::Failed(_)
                | Self::Install(
                    HookOperationOutcome::Orphaned | HookOperationOutcome::DowngradeRefused { .. }
                )
                | Self::Uninstall(
                    HookOperationOutcome::Orphaned | HookOperationOutcome::DowngradeRefused { .. }
                )
        )
    }
}

/// One named toolchain's result, retained independently of its account summary.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ToolchainHookReport {
    /// The toolchain name from the child's report.
    pub(crate) toolchain: String,
    /// The child's reported operation outcome or inspection failure.
    pub(crate) outcome:   ToolchainHookOutcome,
}

impl ToolchainHookReport {
    /// Decode one line using the operation requested from this child.
    fn parse(operation: HookOperation, line: &str) -> Result<Self, String> {
        let (toolchain, result) = line
            .split_once('\t')
            .filter(|(toolchain, _)| !toolchain.is_empty())
            .ok_or_else(|| format!("invalid hook report: {line}"))?;
        let outcome = match (operation, result) {
            (HookOperation::Install, "installed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Installed)
            },
            (HookOperation::Install, "refreshed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Refreshed)
            },
            (HookOperation::Install, "already installed") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::AlreadyCurrent)
            },
            (HookOperation::Install, "orphaned") => {
                ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned)
            },
            (HookOperation::Uninstall, "removed") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::Removed)
            },
            (HookOperation::Uninstall, "absent") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::AlreadyAbsent)
            },
            (HookOperation::Uninstall, "orphaned") => {
                ToolchainHookOutcome::Uninstall(HookOperationOutcome::Orphaned)
            },
            (HookOperation::Status, "installed") => {
                ToolchainHookOutcome::Status(HookState::Installed)
            },
            (HookOperation::Status, "absent") => ToolchainHookOutcome::Status(HookState::Absent),
            (HookOperation::Status, "repairable") => {
                ToolchainHookOutcome::Status(HookState::Repairable)
            },
            (HookOperation::Status, "orphaned") => {
                ToolchainHookOutcome::Status(HookState::Orphaned)
            },
            (_, result) => match result.split_once('\t') {
                Some(("downgrade refused", versions)) if operation == HookOperation::Install => {
                    versions
                        .split_once('\t')
                        .and_then(|(installed, supported)| {
                            let installed = installed.parse::<u64>().ok()?;
                            let supported = supported.parse::<u64>().ok()?;
                            (installed > supported).then_some(
                                HookOperationOutcome::DowngradeRefused {
                                    installed,
                                    supported,
                                },
                            )
                        })
                        .map_or_else(
                            || ToolchainHookOutcome::Failed(format!("invalid hook report: {line}")),
                            ToolchainHookOutcome::Install,
                        )
                },
                Some(("unreadable", reason)) => ToolchainHookOutcome::Unreadable(reason.to_owned()),
                Some(("error", reason)) => ToolchainHookOutcome::Failed(reason.to_owned()),
                _ => ToolchainHookOutcome::Failed(format!("invalid hook report: {line}")),
            },
        };
        Ok(Self {
            toolchain: toolchain.to_owned(),
            outcome,
        })
    }

    /// One line in the protocol shared by all account children.
    pub(crate) fn protocol(&self) -> String {
        format!(
            "{}\t{}",
            protocol_field(&self.toolchain),
            self.outcome.protocol()
        )
    }
}

impl fmt::Display for ToolchainHookReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: ", self.toolchain)?;
        match &self.outcome {
            ToolchainHookOutcome::Install(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            })
            | ToolchainHookOutcome::Uninstall(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            }) => write!(
                formatter,
                "downgrade refused -- newer shim v{installed} kept; this reader supports v{supported}; upgrade and restart the reader"
            ),
            ToolchainHookOutcome::Install(HookOperationOutcome::Orphaned)
            | ToolchainHookOutcome::Uninstall(HookOperationOutcome::Orphaned)
            | ToolchainHookOutcome::Status(HookState::Orphaned) => formatter
                .write_str("orphaned -- the shim is installed but the real cargo is missing"),
            ToolchainHookOutcome::Unreadable(reason) => write!(formatter, "unreadable: {reason}"),
            ToolchainHookOutcome::Failed(reason) => write!(formatter, "error: {reason}"),
            outcome => formatter.write_str(&outcome.protocol()),
        }
    }
}

/// One account's toolchain reports and completion state.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct AccountHookReport {
    /// The account name used for this report.
    pub(crate) account:    String,
    /// The operation requested from the account child.
    pub(crate) operation:  HookOperation,
    /// Completion across this account's toolchains and child process.
    pub(crate) outcome:    AccountHookOutcome,
    /// Every named toolchain result, in the order reported by the child.
    pub(crate) toolchains: Vec<ToolchainHookReport>,
}

impl AccountHookReport {
    /// Record a failure that prevented the child from reporting any toolchains.
    fn incomplete(account: &HookAccount, operation: HookOperation, reason: String) -> Self {
        Self {
            account: account.name.clone(),
            operation,
            outcome: AccountHookOutcome::Incomplete(reason),
            toolchains: Vec::new(),
        }
    }

    /// Retain every valid toolchain report despite malformed output or a failed exit.
    fn from_output(account: &HookAccount, operation: HookOperation, output: &Output) -> Self {
        let mut toolchains = Vec::new();
        let mut failures = Vec::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            match ToolchainHookReport::parse(operation, line) {
                Ok(report) => {
                    if report.outcome.is_incomplete() {
                        failures.push(report.to_string());
                    }
                    toolchains.push(report);
                },
                Err(error) => failures.push(error),
            }
        }
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let reason = stderr.trim();
            failures.push(if reason.is_empty() {
                format!("account child exited with {}", output.status)
            } else {
                reason.to_owned()
            });
        }
        let outcome = if !failures.is_empty() {
            AccountHookOutcome::Incomplete(failures.join("; "))
        } else if toolchains.is_empty() {
            AccountHookOutcome::NoToolchains
        } else {
            AccountHookOutcome::Completed
        };
        Self {
            account: account.name.clone(),
            operation,
            outcome,
            toolchains,
        }
    }
}

impl fmt::Display for AccountHookReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}: {}: ",
            self.account,
            self.operation.subcommand()
        )?;
        match &self.outcome {
            AccountHookOutcome::NoToolchains => formatter.write_str("no toolchains")?,
            AccountHookOutcome::Completed => formatter.write_str("completed")?,
            AccountHookOutcome::Incomplete(reason) => write!(formatter, "incomplete: {reason}")?,
        }
        for report in &self.toolchains {
            write!(formatter, "\n{}: {report}", self.account)?;
        }
        Ok(())
    }
}

/// Keep child-protocol fields on a single tab-separated line.
fn protocol_field(value: &str) -> String { value.replace(['\t', '\r', '\n'], " ") }

/// One toolchain's cargo, and whatever stands in front of it.
pub(crate) struct Hook {
    /// The toolchain's name, which is what a report names it by.
    name:    String,
    /// The `cargo` the toolchain resolves, which the shim takes the
    /// place of.
    cargo:   PathBuf,
    /// Where the real cargo is kept while the shim holds its name.
    real:    PathBuf,
    /// Where a shim is written before it is renamed over `cargo`.
    staging: PathBuf,
}

/// What the grid found, and did, standing the shim up as it opened.
///
/// Every list holds toolchain names, sorted, so the notice built from
/// it reads the same twice running. A toolchain whose shim was already
/// current is in none of them: it is the ordinary case, and there is
/// nothing to say about it.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Startup {
    /// Toolchains the shim was put in front of just now.
    pub(crate) installed:  Vec<String>,
    /// Toolchains whose shim was out of date -- written by an earlier
    /// cargo-tile -- and now carries this binary's copy.
    pub(crate) refreshed:  Vec<String>,
    /// Newer installed shims kept intact because this reader is older.
    pub(crate) kept_newer: Vec<NewerShim>,
    /// Toolchains whose shim has no real cargo beside it. Left alone:
    /// installing over that would write a shim in front of nothing, and
    /// the only repair is `rustup` putting a cargo back.
    pub(crate) orphaned:   Vec<String>,
    /// Toolchains where installing failed, and the error's text. A
    /// read-only toolchain directory is the usual reason.
    pub(crate) failed:     Vec<(String, String)>,
}

impl Startup {
    /// Whether anything happened that the user should hear about.
    pub(crate) const fn is_quiet(&self) -> bool {
        self.installed.is_empty()
            && self.refreshed.is_empty()
            && self.kept_newer.is_empty()
            && self.orphaned.is_empty()
            && self.failed.is_empty()
    }
}

/// One installed shim that startup cannot replace with this older reader's copy.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct NewerShim {
    /// The toolchain whose shim remains installed.
    pub(crate) toolchain: String,
    /// The version declared by the installed shim.
    pub(crate) installed: u64,
    /// The newest framing version this reader supports.
    pub(crate) supported: u64,
}

/// Stand the shim up in front of every toolchain that lacks one, and
/// bring every installed shim up to date with this binary's copy.
///
/// Orphaned and newer shims are reported and kept unchanged.
/// A toolchain that fails does not stop
/// the others: each is its own file system operation, and one refusing
/// says nothing about the next.
pub(crate) fn at_startup() -> io::Result<Startup> {
    let mut hooks = Vec::new();
    let mut failed = Vec::new();
    for discovery in Hook::all()? {
        match discovery {
            HookDiscovery::Discovered(hook) => hooks.push(hook),
            HookDiscovery::AbsentCargo => {},
            HookDiscovery::InspectionFailed { path, error } => {
                failed.push((path.display().to_string(), error.to_string()));
            },
        }
    }
    let mut startup = stand_up(&hooks);
    startup.failed.extend(failed);
    Ok(startup)
}

/// [`at_startup`] over a known set of hooks, which is what the tests
/// hand in.
fn stand_up(hooks: &[Hook]) -> Startup {
    let mut startup = Startup::default();
    for hook in hooks {
        match hook.ensure() {
            Ok(HookOperationOutcome::Installed) => startup.installed.push(hook.name.clone()),
            Ok(HookOperationOutcome::Refreshed) => startup.refreshed.push(hook.name.clone()),
            Ok(HookOperationOutcome::Orphaned) => startup.orphaned.push(hook.name.clone()),
            Ok(HookOperationOutcome::DowngradeRefused {
                installed,
                supported,
            }) => {
                startup.kept_newer.push(NewerShim {
                    toolchain: hook.name.clone(),
                    installed,
                    supported,
                });
            },
            Ok(
                HookOperationOutcome::Removed
                | HookOperationOutcome::AlreadyAbsent
                | HookOperationOutcome::AlreadyCurrent,
            ) => {},
            Err(error) => startup.failed.push((hook.name.clone(), error.to_string())),
        }
    }
    startup
}

/// What is standing in front of a toolchain's cargo right now.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookState {
    /// The shim holds `cargo` and the real binary is beside it. Runs on
    /// this toolchain are captured.
    Installed,
    /// A real cargo holds its own name. Nothing is captured.
    Absent,
    /// Installation moved the real binary aside but did not write the
    /// shim. Restoring the saved binary repairs the missing `cargo`.
    Repairable,
    /// The shim holds `cargo` but the real binary beside it is gone, so
    /// every invocation fails. Only reachable by deleting the real
    /// binary by hand.
    Orphaned,
}

/// What installing or removing did to one toolchain.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HookOperationOutcome {
    /// The shim now stands where a real cargo did.
    Installed,
    /// The installed shim differed from this binary's copy and was
    /// brought up to date.
    Refreshed,
    /// The installed shim matches this binary's copy and was untouched.
    AlreadyCurrent,
    /// The installed shim is newer than this reader's copy and remains unchanged.
    DowngradeRefused {
        /// The installed shim's declared framing version.
        installed: u64,
        /// The newest framing version this reader supports.
        supported: u64,
    },
    /// The real cargo has its name back.
    Removed,
    /// Nothing to do: no shim was installed.
    AlreadyAbsent,
    /// Nothing done: the shim is there with no real cargo beside it,
    /// which is not a state installing can repair.
    Orphaned,
}

/// Discovery distinguishes a missing cargo from a path that could not be inspected.
pub(crate) enum HookDiscovery {
    /// A toolchain has a cargo or a saved original to operate on.
    Discovered(Hook),
    /// Neither cargo nor the saved original exists in this directory.
    AbsentCargo,
    /// The path could not be inspected, with the original filesystem error.
    InspectionFailed {
        /// Toolchain directory, or its parent when reading an entry failed.
        path:  PathBuf,
        /// Filesystem error retained for the account report.
        error: io::Error,
    },
}

impl Hook {
    /// Yield each completed operation before starting the next toolchain.
    pub(crate) fn reports(
        operation: HookOperation,
    ) -> io::Result<impl Iterator<Item = ToolchainHookReport>> {
        Ok(Self::all()?
            .into_iter()
            .filter_map(move |discovery| match discovery {
                HookDiscovery::Discovered(hook) => Some(ToolchainHookReport {
                    toolchain: hook.name().to_owned(),
                    outcome:   hook.perform(operation),
                }),
                HookDiscovery::AbsentCargo => None,
                HookDiscovery::InspectionFailed { path, error } => Some(ToolchainHookReport {
                    toolchain: path
                        .file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                        .into_owned(),
                    outcome:   ToolchainHookOutcome::Unreadable(format!(
                        "{}: {error}",
                        path.display()
                    )),
                }),
            }))
    }

    /// Every toolchain rustup has installed with a cargo under either
    /// its own name or the saved real binary's name.
    ///
    /// Sorted by name so a report reads the same twice running.
    pub(crate) fn all() -> io::Result<Vec<HookDiscovery>> { Self::in_rustup_home(&rustup_home()?) }

    /// Retain directory-entry and cargo inspection failures beside discovered toolchains.
    fn in_rustup_home(home: &Path) -> io::Result<Vec<HookDiscovery>> {
        let directory = home.join(TOOLCHAINS_DIR);
        let mut entries = Vec::new();
        let mut failures = Vec::new();
        for entry in fs::read_dir(&directory).map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", directory.display()))
        })? {
            match entry {
                Ok(entry) => entries.push(entry.path()),
                Err(error) => failures.push(HookDiscovery::InspectionFailed {
                    path: directory.clone(),
                    error,
                }),
            }
        }
        entries.sort();
        let mut discoveries: Vec<_> = entries
            .iter()
            .map(|entry| Self::at(entry))
            .filter(|discovery| !matches!(discovery, HookDiscovery::AbsentCargo))
            .collect();
        discoveries.extend(failures);
        Ok(discoveries)
    }

    /// Inspect both cargo paths without folding filesystem failures into absence.
    fn at(toolchain: &Path) -> HookDiscovery {
        let binaries = toolchain.join(TOOLCHAIN_BIN_DIR);
        let cargo = binaries.join(CARGO_NAME);
        let real = binaries.join(REAL_CARGO_NAME);
        match cargo.try_exists().and_then(|cargo_exists| {
            real.try_exists()
                .map(|real_exists| cargo_exists || real_exists)
        }) {
            Ok(false) => HookDiscovery::AbsentCargo,
            Ok(true) => HookDiscovery::Discovered(Self {
                name: toolchain
                    .file_name()
                    .unwrap_or(toolchain.as_os_str())
                    .to_string_lossy()
                    .into_owned(),
                cargo,
                real,
                staging: binaries.join(SHIM_STAGING_NAME),
            }),
            Err(error) => HookDiscovery::InspectionFailed {
                path: toolchain.to_owned(),
                error,
            },
        }
    }

    /// The toolchain this hook belongs to.
    pub(crate) fn name(&self) -> &str { &self.name }

    /// What is standing in front of cargo, retaining metadata and read failures.
    pub(crate) fn state(&self) -> io::Result<HookState> {
        let cargo_exists = self.cargo.try_exists().map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", self.cargo.display()))
        })?;
        let real_exists = self.real.try_exists().map_err(|error| {
            io::Error::new(error.kind(), format!("{}: {error}", self.real.display()))
        })?;
        if !cargo_exists && real_exists {
            return Ok(HookState::Repairable);
        }
        match inspect_shim(&self.cargo)? {
            CargoContents::Original => Ok(HookState::Absent),
            CargoContents::Shim if real_exists => Ok(HookState::Installed),
            CargoContents::Shim => Ok(HookState::Orphaned),
        }
    }

    /// Inspect status directly; mutations inspect state under their toolchain lock.
    fn perform(&self, operation: HookOperation) -> ToolchainHookOutcome {
        match operation {
            HookOperation::Install => self.ensure().map_or_else(
                |error| ToolchainHookOutcome::Failed(error.to_string()),
                ToolchainHookOutcome::Install,
            ),
            HookOperation::Uninstall => self.remove().map_or_else(
                |error| ToolchainHookOutcome::Failed(error.to_string()),
                ToolchainHookOutcome::Uninstall,
            ),
            HookOperation::Status => self.state().map_or_else(
                |error| ToolchainHookOutcome::Unreadable(error.to_string()),
                ToolchainHookOutcome::Status,
            ),
        }
    }

    /// Put the current shim in front of this toolchain's cargo, leaving
    /// an identical installed copy untouched.
    ///
    /// Writing the shim is the last step, so a failure part way through
    /// can leave `cargo` missing. Retrying writes the shim directly and
    /// leaves the saved real binary in place. The per-toolchain lock
    /// covers state inspection and every write, including repair.
    fn install(&self) -> io::Result<HookOperationOutcome> {
        let _installation_lock =
            HookInstallationLock::acquire(self.cargo.with_file_name(SHIM_LOCK_NAME))?;
        match self.state()? {
            HookState::Installed => {
                let contents = fs::read(&self.cargo)?;
                if contents == SHIM_SOURCE.as_bytes() {
                    return Ok(HookOperationOutcome::AlreadyCurrent);
                }
                if let ShimVersion::Versioned(installed) = shim_version(&contents)?
                    && installed > SUPPORTED_REGISTRATION_VERSION
                {
                    return Ok(HookOperationOutcome::DowngradeRefused {
                        installed,
                        supported: SUPPORTED_REGISTRATION_VERSION,
                    });
                }
                self.write_shim()?;
                return Ok(HookOperationOutcome::Refreshed);
            },
            HookState::Orphaned => return Ok(HookOperationOutcome::Orphaned),
            HookState::Repairable => {
                self.write_shim()?;
                return Ok(HookOperationOutcome::Installed);
            },
            HookState::Absent => {},
        }
        // Keep an unmarked first install or the fresh cargo that
        // `rustup update` put back over the shim.
        fs::rename(&self.cargo, &self.real)?;
        self.write_shim()?;
        Ok(HookOperationOutcome::Installed)
    }

    /// Keep startup and the install subcommand on the same repair and
    /// content-comparison path as [`install`](Self::install).
    ///
    /// [`HookOperationOutcome::AlreadyCurrent`] requires reading the shim, but no
    /// shim writes: repeated launches and CI job hooks leave it untouched.
    pub(crate) fn ensure(&self) -> io::Result<HookOperationOutcome> { self.install() }

    /// Write the shim as `cargo`, executable, without ever writing the
    /// file already there in place.
    ///
    /// A shim that is mid-run is still being read by its `sh`, which
    /// waits on the real cargo rather than `exec`ing it. Writing over it
    /// would hand that `sh` a half-written script. So the shim goes in
    /// beside it and is renamed across: the running `sh` keeps the inode
    /// it opened, and the name changes hands in one step.
    fn write_shim(&self) -> io::Result<()> {
        let mut staging = OpenOptions::new()
            .write(true)
            .mode(SHIM_MODE)
            .create(true)
            .truncate(true)
            .open(&self.staging)?;
        staging.write_all(SHIM_SOURCE.as_bytes())?;
        staging.set_permissions(fs::Permissions::from_mode(SHIM_MODE))?;
        fs::rename(&self.staging, &self.cargo)
    }

    /// Give the real cargo its name back.
    pub(crate) fn remove(&self) -> io::Result<HookOperationOutcome> {
        let _installation_lock =
            HookInstallationLock::acquire(self.cargo.with_file_name(SHIM_LOCK_NAME))?;
        match self.state()? {
            HookState::Absent => Ok(HookOperationOutcome::AlreadyAbsent),
            HookState::Orphaned => Ok(HookOperationOutcome::Orphaned),
            HookState::Installed | HookState::Repairable => {
                fs::rename(&self.real, &self.cargo)?;
                Ok(HookOperationOutcome::Removed)
            },
        }
    }
}

/// Exclusive access to one toolchain's install, repair, and removal operations.
/// The lock file's presence owns access until this guard is dropped.
struct HookInstallationLock {
    /// Only the guard that created this path may remove it.
    path: PathBuf,
}

impl HookInstallationLock {
    /// Wait briefly for another installer, then fail without touching
    /// cargo if exclusive creation remains unavailable.
    fn acquire(path: PathBuf) -> io::Result<Self> {
        let mut retries_remaining = SHIM_LOCK_RETRY_ATTEMPTS;
        loop {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self { path }),
                Err(error) if error.kind() == ErrorKind::AlreadyExists && retries_remaining > 0 => {
                    retries_remaining -= 1;
                    thread::sleep(SHIM_LOCK_RETRY_DELAY);
                },
                Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}; {SHIM_LOCK_RECOVERY}", path.display()),
                    ));
                },
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("{}: {error}", path.display()),
                    ));
                },
            }
        }
    }
}

impl Drop for HookInstallationLock {
    fn drop(&mut self) { drop(fs::remove_file(&self.path)); }
}

/// An executable copy accessible to account children until reporting ends.
pub(crate) struct StagedExecutable {
    /// The fresh directory is removed with all its contents when the guard drops.
    directory:  PathBuf,
    /// The executable path passed to each account child.
    executable: PathBuf,
}

impl StagedExecutable {
    /// Keep children independent of permissions on the administrator's home.
    pub(crate) fn path(&self) -> &Path { &self.executable }
}

impl Drop for StagedExecutable {
    fn drop(&mut self) { drop(fs::remove_dir_all(&self.directory)); }
}

/// Stage beneath a trusted parent so other accounts cannot replace the executable.
/// The admin command supplies the root-owned sticky system temporary directory.
pub(crate) fn stage_executable(parent: &Path, source: &Path) -> io::Result<StagedExecutable> {
    let directory = tempfile::Builder::new()
        .prefix(ACCOUNT_EXECUTABLE_PREFIX)
        .permissions(fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE))
        .tempdir_in(parent)?;
    fs::set_permissions(
        directory.path(),
        fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
    )?;
    let executable = directory.path().join(BINARY_NAME);
    fs::copy(source, &executable)?;
    fs::set_permissions(
        &executable,
        fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
    )?;
    Ok(StagedExecutable {
        directory: directory.keep(),
        executable,
    })
}

/// Read the system account database before the CLI starts any worker threads.
/// `getpwent` includes Directory Services accounts on macOS and NSS on Linux.
#[allow(
    unsafe_code,
    reason = "system account enumeration and home directories require libc getpwent; called only by the single-threaded admin command"
)]
pub(crate) fn system_accounts() -> io::Result<Vec<HookAccount>> {
    let mut accounts = Vec::new();
    // SAFETY: the admin CLI calls this before starting threads or other account
    // lookups. Every passwd string is copied before the next getpwent call, and
    // endpwent runs on both success and error before returning owned values.
    unsafe {
        libc::setpwent();
        let result = loop {
            #[cfg(target_os = "linux")]
            let errno = libc::__errno_location();
            #[cfg(target_os = "macos")]
            let errno = libc::__error();
            *errno = 0;
            let entry = libc::getpwent();
            if entry.is_null() {
                break if *errno == 0 {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                };
            }
            let entry = &*entry;
            if entry.pw_name.is_null() || entry.pw_dir.is_null() {
                break Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "account database returned an incomplete account",
                ));
            }
            accounts.push(HookAccount {
                name: CStr::from_ptr(entry.pw_name).to_string_lossy().into_owned(),
                uid:  entry.pw_uid,
                gid:  entry.pw_gid,
                home: PathBuf::from(OsStr::from_bytes(CStr::from_ptr(entry.pw_dir).to_bytes())),
            });
        };
        libc::endpwent();
        result?;
    }
    accounts.sort_by(|left, right| left.name.cmp(&right.name).then(left.uid.cmp(&right.uid)));
    Ok(accounts)
}

/// Resolve the account database memberships, including its primary group, before spawning.
#[allow(
    unsafe_code,
    reason = "libc getgrouplist is required to resolve complete account memberships on Linux and macOS"
)]
pub(crate) fn account_groups(name: &str, primary_gid: u32) -> io::Result<Vec<u32>> {
    let name =
        CString::new(name).map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    #[cfg(target_os = "macos")]
    let primary_gid = libc::c_int::try_from(primary_gid)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;
    #[cfg(target_os = "linux")]
    let primary_gid: libc::gid_t = primary_gid;
    let groups = account_groups_with(primary_gid, |groups, count| {
        // SAFETY: name is NUL-terminated and lives through every call. The group
        // element type matches this platform's libc declaration, and count is
        // the allocated slice length on entry to each call.
        unsafe { libc::getgrouplist(name.as_ptr(), primary_gid, groups.as_mut_ptr(), count) }
    })?;
    #[cfg(target_os = "macos")]
    {
        groups
            .into_iter()
            .map(|group| {
                u32::try_from(group).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))
            })
            .collect()
    }
    #[cfg(target_os = "linux")]
    {
        Ok(groups)
    }
}

/// Grow an undersized group buffer within a ceiling, retaining the platform's element type.
fn account_groups_with<Group: Copy>(
    primary_gid: Group,
    mut resolve: impl FnMut(&mut [Group], &mut libc::c_int) -> libc::c_int,
) -> io::Result<Vec<Group>> {
    let mut groups = vec![primary_gid; ACCOUNT_GROUPS_INITIAL_CAPACITY];
    let count = loop {
        let mut count = libc::c_int::try_from(groups.len()).map_err(io::Error::other)?;
        if resolve(&mut groups, &mut count) != -1 {
            break count;
        }
        let needed = usize::try_from(count)
            .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
        if groups.len() == ACCOUNT_GROUPS_MAX_CAPACITY || needed > ACCOUNT_GROUPS_MAX_CAPACITY {
            return Err(io::Error::other(format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )));
        }
        // glibc reports the required size; Darwin can leave the supplied count unchanged.
        let needed = if needed > groups.len() {
            needed
        } else {
            groups
                .len()
                .saturating_mul(ACCOUNT_GROUPS_GROWTH_FACTOR)
                .min(ACCOUNT_GROUPS_MAX_CAPACITY)
        };
        groups.resize(needed, primary_gid);
    };
    let count =
        usize::try_from(count).map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    if count > groups.len() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            format!(
                "getgrouplist returned {count} groups for a buffer of {} entries",
                groups.len()
            ),
        ));
    }
    groups.truncate(count);
    Ok(groups)
}

/// Handle explicit account records through the same staged executable for every operation.
/// Accounts without a default rustup home are omitted.
pub(crate) fn run_account_hooks(
    accounts: &[HookAccount],
    executable: &Path,
    operation: HookOperation,
) -> Vec<AccountHookReport> {
    run_account_hooks_with(accounts, executable, operation, |command, account| {
        if rustix::process::geteuid().is_root() {
            account_credentials(command, account)
        } else {
            command.uid(account.uid).gid(account.gid);
            Ok(())
        }
    })
}

/// Inject only credential preparation so resolver failures can be verified without root.
pub(crate) fn run_account_hooks_with(
    accounts: &[HookAccount],
    executable: &Path,
    operation: HookOperation,
    mut credentials: impl FnMut(&mut Command, &HookAccount) -> io::Result<()>,
) -> Vec<AccountHookReport> {
    let mut reports = Vec::new();
    for account in accounts {
        let home = account.home.join(RUSTUP_DIRNAME);
        if fs::metadata(&home).is_err_and(|error| error.kind() == ErrorKind::NotFound) {
            continue;
        }
        reports.push(run_account_hook(
            account,
            executable,
            operation,
            &mut credentials,
        ));
    }
    reports
}

/// Drop privileges before any toolchain discovery, lock, or operation.
fn run_account_hook(
    account: &HookAccount,
    executable: &Path,
    operation: HookOperation,
    credentials: &mut impl FnMut(&mut Command, &HookAccount) -> io::Result<()>,
) -> AccountHookReport {
    let mut command = Command::new(executable);
    if let Err(error) = credentials(&mut command, account) {
        return AccountHookReport::incomplete(
            account,
            operation,
            format!("could not resolve {}'s groups: {error}", account.name),
        );
    }
    command
        .env("HOME", &account.home)
        .env(RUSTUP_HOME_ENV, account.home.join(RUSTUP_DIRNAME))
        .arg(operation.subcommand())
        .arg(format!("--{ACCOUNT_HOOK_REPORT_FLAG}"))
        .output()
        .map_or_else(
            |error| {
                AccountHookReport::incomplete(
                    account,
                    operation,
                    format!(
                        "could not start the account child as {}: {error}",
                        account.name
                    ),
                )
            },
            |output| AccountHookReport::from_output(account, operation, &output),
        )
}

/// Refuse membership lists exceeding Darwin's runtime setgroups limit.
/// Accepted lists retain every resolved group, including the primary gid.
/// A failed or non-positive sysconf result leaves the resolved membership intact.
#[cfg(any(target_os = "macos", test))]
pub(crate) fn bounded_account_groups(
    primary_gid: u32,
    groups: Vec<u32>,
    limit: libc::c_long,
) -> io::Result<Vec<u32>> {
    let Ok(limit) = usize::try_from(limit) else {
        return Ok(groups);
    };
    if limit == 0 || groups.len() <= limit {
        return Ok(groups);
    }
    Err(io::Error::new(
        ErrorKind::InvalidInput,
        format!(
            "{} resolved groups including primary gid {primary_gid} exceed Darwin's runtime setgroups limit of {limit}; refusing to truncate account memberships and lose group access",
            groups.len()
        ),
    ))
}

/// Apply root's resolved credentials together, before exec, without std clearing groups.
/// `CommandExt::groups` is unstable, so the callback also owns the uid/gid transition.
#[allow(
    unsafe_code,
    reason = "stable Command lacks explicit groups; the child must setgroups before setgid and setuid using only libc credential syscalls"
)]
fn account_credentials(command: &mut Command, account: &HookAccount) -> io::Result<()> {
    let groups = account_groups(&account.name, account.gid)?;
    #[cfg(target_os = "macos")]
    let groups = {
        // SAFETY: sysconf takes the platform selector and has no pointer arguments.
        let limit = unsafe { libc::sysconf(libc::_SC_NGROUPS_MAX) };
        bounded_account_groups(account.gid, groups, limit)?
    };
    #[cfg(target_os = "macos")]
    let count = libc::c_int::try_from(groups.len())
        .map_err(|error| io::Error::new(ErrorKind::InvalidData, error))?;
    #[cfg(target_os = "linux")]
    let count = groups.len();
    let uid = account.uid;
    let gid = account.gid;
    // SAFETY: the parent resolves and allocates groups before fork. The callback
    // uses only credential syscalls and last_os_error, with no allocation or
    // locks; groups owns count gid_t entries for the callback's entire lifetime.
    // No std uid/gid setters precede it, and any failed syscall prevents exec.
    unsafe {
        command.pre_exec(move || {
            if libc::setgroups(count, groups.as_ptr()) == -1
                || libc::setgid(gid) == -1
                || libc::setuid(uid) == -1
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

/// Where rustup keeps its toolchains.
fn rustup_home() -> io::Result<PathBuf> {
    if let Some(home) = env::var_os(RUSTUP_HOME_ENV) {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir()
        .map(|home| home.join(RUSTUP_DIRNAME))
        .ok_or_else(|| io::Error::other("no home directory to find rustup under"))
}

/// What a successful read establishes about the cargo file.
enum CargoContents {
    /// The opening contains the capture shim marker.
    Shim,
    /// The opening belongs to an unmarked cargo binary.
    Original,
}

/// Whether the installed shim declares its framing version.
enum ShimVersion {
    /// Historical shims have no version line and may be upgraded.
    Unversioned,
    /// The header explicitly identifies the shim's framing version.
    Versioned(u64),
}

/// Inspect only the shim header; malformed declarations must not authorize replacement.
fn shim_version(contents: &[u8]) -> io::Result<ShimVersion> {
    let opening = &contents[..contents.len().min(SHIM_MARKER_SEARCH_BYTES)];
    let mut version = ShimVersion::Unversioned;
    for line in String::from_utf8_lossy(opening).split_inclusive('\n') {
        let Some(terminated) = line.strip_suffix('\n') else {
            // Even a partial prefix may begin a declaration cut off by the opening.
            if line.starts_with(SHIM_VERSION_PREFIX) || SHIM_VERSION_PREFIX.starts_with(line) {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "incomplete shim version line",
                ));
            }
            continue;
        };
        let line = terminated.strip_suffix('\r').unwrap_or(terminated);
        let Some(number) = line.strip_prefix(SHIM_VERSION_PREFIX) else {
            continue;
        };
        if matches!(version, ShimVersion::Versioned(_)) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "duplicate shim version line",
            ));
        }
        if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "invalid shim version line",
            ));
        }
        version = ShimVersion::Versioned(number.parse().map_err(|error| {
            io::Error::new(
                ErrorKind::InvalidData,
                format!("invalid shim version: {error}"),
            )
        })?);
    }
    Ok(version)
}

/// Read the bounded opening completely, retaining open and read errors.
fn inspect_shim(path: &Path) -> io::Result<CargoContents> {
    let mut file = fs::File::open(path)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?
        .take(SHIM_MARKER_SEARCH_BYTES as u64);
    let mut opening = Vec::new();
    file.read_to_end(&mut opening)
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    Ok(if String::from_utf8_lossy(&opening).contains(SHIM_MARKER) {
        CargoContents::Shim
    } else {
        CargoContents::Original
    })
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::process::Command;
    use std::sync::Barrier;
    use std::time::SystemTime;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::constants::HOOK_TEST_DIRECTORY_COLLISION_CARGO;
    use crate::constants::HOOK_TEST_DIRECTORY_COLLISION_LINK;
    use crate::constants::HOOK_TEST_REAL_CARGO;
    use crate::constants::HOOK_TEST_VERSION_ARGUMENT;
    use crate::constants::LOCK_WAIT_MARKER;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::constants::SUBCOMMAND_NAME;

    #[test]
    fn embedded_shim_declares_the_supported_framing_version() {
        assert!(matches!(
            shim_version(SHIM_SOURCE.as_bytes()).unwrap(),
            ShimVersion::Versioned(version) if version == SUPPORTED_REGISTRATION_VERSION
        ));
    }

    #[test]
    fn invalid_or_repeated_version_lines_preserve_installed_bytes() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        for version in [
            "",
            "newer",
            "+4",
            "18446744073709551616",
            "4\n# cargo-tile-shim-version: 3",
        ] {
            let contents = format!("#!/bin/sh\n# {SHIM_MARKER}\n{SHIM_VERSION_PREFIX}{version}\n");
            fs::write(&hook.cargo, &contents).unwrap();
            let written = mark_old_mtime(&hook.cargo);

            assert_eq!(hook.ensure().unwrap_err().kind(), ErrorKind::InvalidData);
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), contents);
            assert_eq!(
                fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
                written
            );
            assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
            assert!(!hook.staging.exists());
        }
    }

    #[test]
    fn version_lines_crossing_the_opening_preserve_installed_files() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        let header = format!("#!/bin/sh\n# {SHIM_MARKER}\n#");
        let future_version = format!("{SUPPORTED_REGISTRATION_VERSION}0");
        for ending in ["\n", "\r\n"] {
            let declaration = format!("{SHIM_VERSION_PREFIX}{future_version}{ending}");
            for visible in 1..=declaration.len() {
                let padding = " ".repeat(SHIM_MARKER_SEARCH_BYTES - header.len() - 1 - visible);
                let contents = format!("{header}{padding}\n{declaration}");
                fs::write(&hook.cargo, &contents).unwrap();
                for path in [&hook.cargo, &hook.real] {
                    fs::set_permissions(path, fs::Permissions::from_mode(0o751)).unwrap();
                    mark_old_mtime(path);
                }
                let preserved = [&hook.cargo, &hook.real].map(|path| {
                    let metadata = fs::metadata(path).unwrap();
                    (
                        fs::read(path).unwrap(),
                        metadata.permissions().mode(),
                        metadata.modified().unwrap(),
                    )
                });

                for install in [Hook::install, Hook::ensure] {
                    if visible == declaration.len() {
                        assert_eq!(
                            install(&hook).unwrap(),
                            HookOperationOutcome::DowngradeRefused {
                                installed: future_version.parse().unwrap(),
                                supported: SUPPORTED_REGISTRATION_VERSION,
                            }
                        );
                    } else {
                        assert_eq!(install(&hook).unwrap_err().kind(), ErrorKind::InvalidData);
                    }
                    for (path, (bytes, mode, modified)) in
                        [&hook.cargo, &hook.real].into_iter().zip(&preserved)
                    {
                        let metadata = fs::metadata(path).unwrap();
                        assert_eq!(&fs::read(path).unwrap(), bytes);
                        assert_eq!(metadata.permissions().mode(), *mode);
                        assert_eq!(metadata.modified().unwrap(), *modified);
                    }
                    assert!(!hook.staging.exists());
                    assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
                }
            }
        }
    }

    #[test]
    fn version_lines_without_a_final_newline_are_invalid() {
        for ending in ["", "\r"] {
            let contents = format!("{SHIM_VERSION_PREFIX}{SUPPORTED_REGISTRATION_VERSION}{ending}");
            assert!(matches!(
                shim_version(contents.as_bytes()),
                Err(error) if error.kind() == ErrorKind::InvalidData
            ));
        }
    }

    #[test]
    fn incomplete_script_lines_do_not_invalidate_complete_version_declarations() {
        let contents = format!(
            "{SHIM_VERSION_PREFIX}{SUPPORTED_REGISTRATION_VERSION}\n{}",
            "exec cargo-real ".repeat(SHIM_MARKER_SEARCH_BYTES)
        );
        assert!(matches!(
            shim_version(contents.as_bytes()).unwrap(),
            ShimVersion::Versioned(version) if version == SUPPORTED_REGISTRATION_VERSION
        ));
        assert!(matches!(
            shim_version(b"#!/bin/sh\nexec cargo-real").unwrap(),
            ShimVersion::Unversioned
        ));
    }

    #[test]
    fn malformed_downgrade_reports_retain_the_toolchain_as_incomplete() {
        for result in ["4", "4\t4", "3\t4", "future\t3", "4\t3\textra"] {
            let report = ToolchainHookReport::parse(
                HookOperation::Install,
                &format!("nightly\tdowngrade refused\t{result}"),
            )
            .unwrap();
            assert_eq!(report.toolchain, "nightly");
            assert!(matches!(report.outcome, ToolchainHookOutcome::Failed(_)));
            assert!(report.outcome.is_incomplete());
        }
    }

    #[test]
    fn credential_groups_at_or_below_the_limit_are_unchanged() {
        let groups = vec![7, 11, 19];
        for limit in [groups.len(), groups.len() + 1] {
            let bounded =
                bounded_account_groups(19, groups.clone(), libc::c_long::try_from(limit).unwrap())
                    .unwrap();
            assert_eq!(bounded, groups);
            assert!(bounded.contains(&19));
        }
    }

    #[test]
    fn over_limit_credential_groups_are_refused_with_both_counts() {
        let groups = vec![7, 11, 19];
        for primary_gid in &groups {
            for limit in 1..groups.len() {
                let error = bounded_account_groups(
                    *primary_gid,
                    groups.clone(),
                    libc::c_long::try_from(limit).unwrap(),
                )
                .unwrap_err();
                assert_eq!(error.kind(), ErrorKind::InvalidInput);
                let reason = error.to_string();
                assert_eq!(
                    reason,
                    format!(
                        "{} resolved groups including primary gid {primary_gid} exceed Darwin's runtime setgroups limit of {limit}; refusing to truncate account memberships and lose group access",
                        groups.len()
                    )
                );
            }
        }
    }

    #[test]
    fn failed_or_non_positive_group_limits_leave_membership_intact() {
        let groups = vec![7, 11, 19];
        for limit in [libc::c_long::MIN, -1, 0] {
            let bounded = bounded_account_groups(19, groups.clone(), limit).unwrap();
            assert_eq!(bounded, groups);
        }
    }

    #[test]
    fn credential_failure_is_incomplete_and_later_accounts_still_run() {
        let fixture = tempdir().unwrap();
        let executable = fixture.path().join(BINARY_NAME);
        fs::write(&executable, "#!/bin/sh\nprintf '%s\\n' started > \"$HOME/child-started\"\ncase $1 in\ninstall|status) printf 'stable\\tinstalled\\n';;\nuninstall) printf 'stable\\tremoved\\n';;\nesac\n").unwrap();
        fs::set_permissions(
            &executable,
            fs::Permissions::from_mode(ACCOUNT_EXECUTABLE_MODE),
        )
        .unwrap();
        let accounts = ["blocked", "later"].map(|name| {
            let home = fixture.path().join(name);
            fs::create_dir_all(home.join(RUSTUP_DIRNAME)).unwrap();
            HookAccount {
                name: name.to_owned(),
                uid: rustix::process::getuid().as_raw(),
                gid: rustix::process::getgid().as_raw(),
                home,
            }
        });
        for operation in [
            HookOperation::Install,
            HookOperation::Uninstall,
            HookOperation::Status,
        ] {
            let mut attempted = Vec::new();
            let reports =
                run_account_hooks_with(&accounts, &executable, operation, |_, account| {
                    attempted.push(account.name.clone());
                    if account.name == "blocked" {
                        Err(io::Error::other("membership resolver unavailable"))
                    } else {
                        Ok(())
                    }
                });
            assert_eq!(attempted, ["blocked", "later"]);
            assert_eq!(reports.len(), accounts.len());
            assert_eq!(
                reports[0].outcome,
                AccountHookOutcome::Incomplete(
                    "could not resolve blocked's groups: membership resolver unavailable"
                        .to_owned()
                )
            );
            assert!(reports[0].toolchains.is_empty());
            assert!(!accounts[0].home.join("child-started").exists());
            assert_eq!(reports[1].outcome, AccountHookOutcome::Completed);
            assert_eq!(reports[1].toolchains.len(), 1);
            assert_eq!(
                fs::read(accounts[1].home.join("child-started")).unwrap(),
                b"started\n"
            );
            let rendered = reports[0].to_string();
            assert!(rendered.contains("blocked: "), "{rendered}");
            assert!(rendered.contains("incomplete: could not resolve blocked's groups: membership resolver unavailable"), "{rendered}");
            assert_eq!(
                operation.completion(&reports).is_ok(),
                operation == HookOperation::Install
            );
        }
    }

    #[test]
    fn discovery_retains_an_uninspectable_toolchain_beside_a_working_one() {
        let (home, _) = toolchain(CARGO_NAME);
        let unreadable = home.path().join(TOOLCHAINS_DIR).join("unreadable");
        std::os::unix::fs::symlink(&unreadable, &unreadable).unwrap();
        let discoveries = Hook::in_rustup_home(home.path()).unwrap();
        assert_eq!(discoveries.len(), 2);
        assert!(
            matches!(&discoveries[0], HookDiscovery::Discovered(hook) if hook.name() == "stable-test")
        );
        assert!(
            matches!(&discoveries[1], HookDiscovery::InspectionFailed { path, error } if path == &unreadable && error.raw_os_error() == Some(libc::ELOOP))
        );
    }

    #[test]
    fn an_unreadable_cargo_retains_its_error_without_changing_the_toolchain() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::remove_file(&hook.cargo).unwrap();
        fs::create_dir(&hook.cargo).unwrap();
        let error = hook.state().unwrap_err().to_string();
        assert_eq!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Unreadable(error)
        );
        assert!(hook.cargo.is_dir());
        assert!(!hook.real.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    #[test]
    fn mutations_acquire_the_lock_before_classifying_an_orphaned_shim() {
        let (_home, hook) = toolchain(SHIM_SOURCE);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let _installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert_eq!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Status(HookState::Orphaned)
        );
        for operation in [HookOperation::Install, HookOperation::Uninstall] {
            let outcome = hook.perform(operation);
            assert!(
                matches!(&outcome, ToolchainHookOutcome::Failed(reason) if reason.contains(&lock_path.display().to_string())),
                "{outcome:?}"
            );
        }
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert!(!hook.real.exists());
        assert!(lock_path.exists());
    }

    #[test]
    fn mutations_acquire_the_lock_before_inspecting_unreadable_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::remove_file(&hook.cargo).unwrap();
        fs::create_dir(&hook.cargo).unwrap();
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let _installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert!(matches!(
            hook.perform(HookOperation::Status),
            ToolchainHookOutcome::Unreadable(_)
        ));
        for operation in [HookOperation::Install, HookOperation::Uninstall] {
            let outcome = hook.perform(operation);
            assert!(
                matches!(&outcome, ToolchainHookOutcome::Failed(reason) if reason.contains(&lock_path.display().to_string())),
                "{outcome:?}"
            );
        }
        assert!(hook.cargo.is_dir());
        assert!(!hook.real.exists());
        assert!(lock_path.exists());
    }

    #[test]
    fn account_groups_resizes_and_returns_every_reported_group() {
        let expected: Vec<u32> = (0..=ACCOUNT_GROUPS_INITIAL_CAPACITY)
            .map(|group| u32::try_from(group).unwrap())
            .collect();
        let mut calls = 0;
        let groups = account_groups_with(expected[0], |groups, count| {
            calls += 1;
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            if calls == 1 {
                assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
                *count = libc::c_int::try_from(expected.len()).unwrap();
                return -1;
            }
            assert_eq!(calls, 2);
            assert_eq!(groups.len(), expected.len());
            groups.copy_from_slice(&expected);
            *count = libc::c_int::try_from(expected.len()).unwrap();
            0
        })
        .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(groups, expected);
    }

    #[test]
    fn account_groups_reports_failure_after_resizing() {
        let needed = ACCOUNT_GROUPS_MAX_CAPACITY;
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            if calls == 1 {
                assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            } else {
                assert_eq!(calls, 2);
                assert_eq!(groups.len(), needed);
            }
            *count = libc::c_int::try_from(needed).unwrap();
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 2);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!("getgrouplist could not resolve groups within the ceiling of {needed} entries")
        );
    }

    #[test]
    fn account_groups_rejects_negative_retry_counts_without_calling_again() {
        let initial_count = libc::c_int::try_from(ACCOUNT_GROUPS_INITIAL_CAPACITY).unwrap();
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(calls, 1);
            assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            assert_eq!(*count, initial_count);
            *count = -1;
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 1);
        assert_eq!(error.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn account_groups_grows_when_failed_counts_supply_no_larger_size() {
        let initial_count = libc::c_int::try_from(ACCOUNT_GROUPS_INITIAL_CAPACITY).unwrap();
        let expected: Vec<u32> = (0..=ACCOUNT_GROUPS_INITIAL_CAPACITY)
            .map(|group| u32::try_from(group).unwrap())
            .collect();
        for reported_count in [0, initial_count - 1, initial_count] {
            let mut calls = 0;
            let groups = account_groups_with(expected[0], |groups, count| {
                calls += 1;
                assert_eq!(usize::try_from(*count).unwrap(), groups.len());
                if calls == 1 {
                    assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
                    *count = reported_count;
                    return -1;
                }
                assert_eq!(calls, 2);
                assert_eq!(
                    groups.len(),
                    ACCOUNT_GROUPS_INITIAL_CAPACITY * ACCOUNT_GROUPS_GROWTH_FACTOR
                );
                groups[..expected.len()].copy_from_slice(&expected);
                *count = libc::c_int::try_from(expected.len()).unwrap();
                0
            })
            .unwrap();

            assert_eq!(calls, 2);
            assert_eq!(groups, expected, "reported count {reported_count}");
        }
    }

    #[test]
    fn account_groups_retries_darwin_overflow_until_every_group_fits() {
        let first_growth = ACCOUNT_GROUPS_INITIAL_CAPACITY * ACCOUNT_GROUPS_GROWTH_FACTOR;
        let expected: Vec<libc::c_int> = (0..=first_growth)
            .map(|group| libc::c_int::try_from(group).unwrap())
            .collect();
        let mut capacities = Vec::new();
        let groups = account_groups_with(expected[0], |groups, count| {
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            capacities.push(groups.len());
            if groups.len() < expected.len() {
                return -1;
            }
            groups[..expected.len()].copy_from_slice(&expected);
            *count = libc::c_int::try_from(expected.len()).unwrap();
            0
        })
        .unwrap();

        assert_eq!(
            capacities,
            [
                ACCOUNT_GROUPS_INITIAL_CAPACITY,
                first_growth,
                first_growth * ACCOUNT_GROUPS_GROWTH_FACTOR,
            ]
        );
        assert_eq!(groups, expected);
    }

    #[test]
    fn account_groups_stops_persistent_overflow_at_the_ceiling() {
        let mut previous_capacity = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            assert_eq!(usize::try_from(*count).unwrap(), groups.len());
            assert!(groups.len() > previous_capacity);
            assert!(groups.len() <= ACCOUNT_GROUPS_MAX_CAPACITY);
            if previous_capacity != 0 {
                assert_eq!(
                    groups.len(),
                    previous_capacity * ACCOUNT_GROUPS_GROWTH_FACTOR
                );
            }
            previous_capacity = groups.len();
            -1
        })
        .unwrap_err();

        assert_eq!(previous_capacity, ACCOUNT_GROUPS_MAX_CAPACITY);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )
        );
    }

    #[test]
    fn account_groups_rejects_required_size_above_the_ceiling_without_retrying() {
        let mut calls = 0;
        let error = account_groups_with(0_u32, |groups, count| {
            calls += 1;
            assert_eq!(calls, 1);
            assert_eq!(groups.len(), ACCOUNT_GROUPS_INITIAL_CAPACITY);
            *count = libc::c_int::try_from(ACCOUNT_GROUPS_MAX_CAPACITY + 1).unwrap();
            -1
        })
        .unwrap_err();

        assert_eq!(calls, 1);
        assert_eq!(error.kind(), ErrorKind::Other);
        assert_eq!(
            error.to_string(),
            format!(
                "getgrouplist could not resolve groups within the ceiling of {ACCOUNT_GROUPS_MAX_CAPACITY} entries"
            )
        );
    }

    /// Enumerate the host database in an isolated process, without installing
    /// anything or sharing libc's enumeration cursor with another test.
    #[test]
    fn system_account_database_includes_the_running_account() {
        if env::var_os("CARGO_TILE_TEST_ACCOUNT_DATABASE").is_some() {
            let accounts = system_accounts().unwrap();
            let uid = rustix::process::getuid().as_raw();
            assert!(accounts.iter().any(|account| account.uid == uid));
            assert!(accounts.iter().all(|account| !account.name.is_empty()));
            return;
        }
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "hook::tests::system_account_database_includes_the_running_account",
                "--nocapture",
            ])
            .env("CARGO_TILE_TEST_ACCOUNT_DATABASE", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Require a discovered hook while preserving the failure detail in test output.
    fn discovered_hook(toolchain: &Path) -> Hook {
        match Hook::at(toolchain) {
            HookDiscovery::Discovered(hook) => hook,
            HookDiscovery::AbsentCargo => panic!("{} has no cargo", toolchain.display()),
            HookDiscovery::InspectionFailed { path, error } => {
                panic!("{}: {error}", path.display())
            },
        }
    }

    /// A rustup home holding one toolchain whose cargo is `contents`.
    fn toolchain(contents: &str) -> (TempDir, Hook) {
        let home = tempdir().unwrap();
        let binaries = home
            .path()
            .join(TOOLCHAINS_DIR)
            .join("stable-test")
            .join(TOOLCHAIN_BIN_DIR);
        fs::create_dir_all(&binaries).unwrap();
        fs::write(binaries.join(CARGO_NAME), contents).unwrap();
        let hook = discovered_hook(&home.path().join(TOOLCHAINS_DIR).join("stable-test"));
        (home, hook)
    }

    #[test]
    fn the_shim_carries_the_marker_the_installer_looks_for() {
        assert!(SHIM_SOURCE.contains(SHIM_MARKER));
    }

    /// The shim runs in front of every cargo invocation on two operating
    /// systems, where the only shell that can be counted on is `sh`.
    #[test]
    fn the_shim_is_posix_sh_and_calls_both_script_implementations() {
        assert!(SHIM_SOURCE.starts_with("#!/bin/sh\n"));
        assert!(SHIM_SOURCE.contains("util-linux"));
        // util-linux exits with its own status without `-e`, which would
        // report every failed build as a success.
        assert!(SHIM_SOURCE.contains("script -q -e -f -c"));
        // Neither implementation flushes the log per write on its own,
        // and the BSD one holds output for thirty seconds at a time --
        // longer than many runs last, and long enough that the single
        // line a blocked run prints reaches the grid after the wait it
        // announced is over.
        assert!(SHIM_SOURCE.contains("script -q -t 0"));
    }

    /// A log is read only while the run writing it is alive, so the run
    /// takes it away as it goes. Unconditionally: what a finished log
    /// happens to hold is no longer a question the shim asks, which is
    /// what freed it from spelling out the reader's markers to answer.
    #[test]
    fn the_shim_retires_its_log_when_its_run_ends() {
        let cleanup = SHIM_SOURCE
            .split_once("cleanup() {")
            .unwrap()
            .1
            .split_once("\n}")
            .unwrap()
            .0;
        assert!(cleanup.lines().any(|line| {
            line.trim_start().starts_with("rm -f ")
                && line.split_whitespace().any(|word| word == r#""$log""#)
        }));
        assert!(SHIM_SOURCE.contains("trap cleanup 0"));
        assert!(
            !SHIM_SOURCE.contains(LOCK_WAIT_MARKER),
            "and no longer carries a copy of a marker the reader owns"
        );
    }

    /// A directory can win the name after setup checks it; POSIX ln
    /// then succeeds inside it, which must still cause uncaptured cargo.
    #[test]
    fn a_directory_arriving_at_publication_preserves_original_cargo() {
        let (home, hook) = toolchain(HOOK_TEST_DIRECTORY_COLLISION_CARGO);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        hook.install().unwrap();
        let observations = home.path().join("observations");
        let tools = home.path().join("tools");
        let parent = home.path().join("capture");
        let root = parent.join(rustix::process::getuid().as_raw().to_string());
        fs::write(
            &hook.cargo,
            SHIM_SOURCE.replace(
                "capture_parent=/tmp/cargo-tile",
                &format!("capture_parent='{}'", parent.display()),
            ),
        )
        .unwrap();
        fs::create_dir(&observations).unwrap();
        fs::create_dir(&tools).unwrap();
        let link = tools.join("ln");
        fs::write(&link, HOOK_TEST_DIRECTORY_COLLISION_LINK).unwrap();
        fs::set_permissions(&link, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        let real_link = Command::new("sh")
            .args(["-c", "command -v ln"])
            .output()
            .unwrap();
        assert!(real_link.status.success());
        let real_link = String::from_utf8(real_link.stdout).unwrap();
        let search_path = env::var_os("PATH").unwrap();
        let search_path =
            env::join_paths(std::iter::once(tools).chain(env::split_paths(&search_path))).unwrap();
        let arguments = ["check", "--quiet", "--message-format=json", "--", "a b", ""];
        let output = Command::new("sh")
            .args(["-c", "umask 0066; exec sh \"$@\"", "publication-test"])
            .arg(&hook.cargo)
            .args(arguments)
            .env("PATH", search_path)
            .env("POSIXLY_CORRECT", "1")
            .env("HOOK_TEST_OBSERVATIONS", &observations)
            .env("HOOK_TEST_REAL_LINK", real_link.trim_end())
            .env_remove("CARGOTILE_NESTED")
            .env_remove("CARGO_TERM_PROGRESS_WHEN")
            .env_remove("CARGO_TERM_PROGRESS_WIDTH")
            .output()
            .unwrap();

        assert_eq!(output.status.code(), Some(37));
        assert_eq!(output.stdout, b"cargo-stdout\n");
        assert_eq!(output.stderr, b"cargo-stderr\n");
        let expected: Vec<u8> = arguments
            .iter()
            .flat_map(|word| word.bytes().chain(std::iter::once(0)))
            .collect();
        assert_eq!(fs::read(observations.join("arguments")).unwrap(), expected);
        let umask = fs::read_to_string(observations.join("umask")).unwrap();
        assert_eq!(u32::from_str_radix(umask.trim(), 8).unwrap(), 0o066);
        assert_eq!(
            fs::read(observations.join("environment")).unwrap(),
            b"unset\0unset\0unset\0"
        );
        let registration = fs::read_to_string(observations.join("registration-path")).unwrap();
        let registration = PathBuf::from(registration);
        assert_eq!(fs::read(registration.join("keep")).unwrap(), b"preserved\n");
        assert_eq!(fs::read_dir(&registration).unwrap().count(), 1);
        assert_eq!(
            fs::read_dir(registration.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        assert_eq!(fs::read_dir(root.join("state")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
    }

    /// Whether the shim's exemption arm names this subcommand. The arm
    /// lists several, so what matters is that this one is among them
    /// rather than what the whole line reads.
    fn shim_passes_through(subcommand: &str) -> bool {
        SHIM_SOURCE.lines().any(|line| {
            line.strip_prefix("    ")
                .and_then(|arm| arm.strip_suffix(')'))
                .is_some_and(|arm| arm.split('|').any(|name| name.trim() == subcommand))
        })
    }

    /// The grid is reachable as `cargo tile`, which puts it in front of
    /// the shim like anything else. Capturing it would run a terminal UI
    /// under `script`, copying every redraw into a log for as long as
    /// the grid stayed open.
    #[test]
    fn the_shim_passes_the_grids_own_subcommand_through() {
        assert!(shim_passes_through(SUBCOMMAND_NAME));
    }

    #[test]
    fn the_shim_passes_the_sibling_terminal_ui_through() {
        assert!(shim_passes_through(SIBLING_SUBCOMMAND_NAME));
    }

    #[test]
    fn a_real_cargo_reads_as_no_shim_installed() {
        let (_home, hook) = toolchain("\u{7f}ELF not a script at all");

        assert_eq!(hook.state().unwrap(), HookState::Absent);
    }

    #[test]
    fn installing_moves_the_real_cargo_aside_and_keeps_every_byte_of_it() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);

        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
        assert!(
            fs::read_to_string(&hook.cargo)
                .unwrap()
                .contains(SHIM_MARKER)
        );
    }

    /// A held lock must stop the install before it moves cargo or
    /// creates a staging file, and a failed contender must leave it held.
    #[test]
    fn installing_with_a_held_lock_leaves_the_toolchain_untouched() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        let installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

        assert_eq!(hook.install().unwrap_err().kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), CARGO_NAME);
        assert!(!hook.real.exists());
        assert!(!hook.staging.exists());
        assert!(lock_path.exists());

        drop(installation_lock);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert!(!lock_path.exists());
    }

    /// A stranded lock must name its path and the operator's recovery
    /// steps once the bounded wait finishes, without deleting the lock.
    #[test]
    fn an_exhausted_installation_lock_reports_how_to_recover() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
        fs::write(&lock_path, CARGO_NAME).unwrap();

        let error = hook.install().unwrap_err();
        let message = error.to_string();

        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(message.contains(&lock_path.display().to_string()));
        assert!(message.contains(SHIM_LOCK_RECOVERY));
        assert!(message.lines().eq([message.as_str()]));
        assert_eq!(fs::read_to_string(&lock_path).unwrap(), CARGO_NAME);
    }

    /// Independent installers starting together must agree which one
    /// saves cargo; the second must inspect the first one's finished shim.
    #[test]
    fn concurrent_installs_preserve_the_original_real_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let competing_hook = discovered_hook(toolchain);
        let installers = [&hook, &competing_hook];
        let start = Barrier::new(installers.len());

        let changes = thread::scope(|scope| {
            let start = &start;
            let installs = installers.map(|hook| {
                scope.spawn(move || {
                    start.wait();
                    hook.install()
                })
            });
            installs.map(|install| install.join().unwrap().unwrap())
        });

        assert!(matches!(
            changes,
            [
                HookOperationOutcome::Installed,
                HookOperationOutcome::AlreadyCurrent
            ] | [
                HookOperationOutcome::AlreadyCurrent,
                HookOperationOutcome::Installed
            ]
        ));
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
        assert!(matches!(
            inspect_shim(&hook.real).unwrap(),
            CargoContents::Original
        ));
        assert!(!hook.staging.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    #[test]
    fn the_installed_shim_is_executable() {
        let (_home, hook) = toolchain("real");
        hook.install().unwrap();

        let mode = fs::metadata(&hook.cargo).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, SHIM_MODE);
    }

    /// Set the mtime far from the current time so even a filesystem with
    /// coarse timestamps exposes an unnecessary shim rewrite.
    fn mark_old_mtime(path: &Path) -> SystemTime {
        fs::File::open(path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH)
            .unwrap();
        fs::metadata(path).unwrap().modified().unwrap()
    }

    /// Installing twice must preserve both the shim's mtime and the
    /// saved real cargo, even though the shim's timestamp is old.
    #[test]
    fn installing_twice_leaves_the_current_shim_and_real_cargo_alone() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);
        hook.install().unwrap();
        let written = mark_old_mtime(&hook.cargo);

        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
    }

    /// Size and mtime can remain unchanged while the installed bytes
    /// differ, so neither can establish that the shim is current.
    #[test]
    fn ensuring_refreshes_changed_contents_with_the_same_size_and_mtime() {
        let (_home, hook) = toolchain(CARGO_NAME);
        hook.install().unwrap();
        let written = mark_old_mtime(&hook.cargo);
        let length = fs::metadata(&hook.cargo).unwrap().len();
        let mut stale = SHIM_SOURCE.as_bytes().to_vec();
        stale.rotate_right(1);
        fs::write(&hook.cargo, &stale).unwrap();
        assert_eq!(mark_old_mtime(&hook.cargo), written);
        assert_eq!(fs::metadata(&hook.cargo).unwrap().len(), length);

        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Refreshed);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
    }

    /// Rediscovery must retain a toolchain whose interrupted install
    /// left the real binary beside the missing `cargo`.
    fn interrupted_toolchain() -> (TempDir, Hook) {
        let (home, hook) = toolchain(HOOK_TEST_REAL_CARGO);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        fs::rename(&hook.cargo, &hook.real).unwrap();
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let hook = discovered_hook(toolchain);
        (home, hook)
    }

    /// A repaired install must execute the saved cargo through the shim
    /// and retain the saved binary's contents.
    fn assert_working_install(hook: &Hook) {
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            HOOK_TEST_REAL_CARGO
        );
        let output = Command::new(&hook.cargo)
            .arg(HOOK_TEST_VERSION_ARGUMENT)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            format!("{HOOK_TEST_VERSION_ARGUMENT}\n").as_bytes()
        );
        assert!(output.stderr.is_empty());
    }

    /// Installation repairs the missing command without using the
    /// saved cargo's marker-bearing contents to classify it as a shim.
    #[test]
    fn installing_repairs_a_toolchain_with_only_the_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();

        assert_eq!(hook.state().unwrap(), HookState::Repairable);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
        let written = mark_old_mtime(&hook.cargo);
        assert_eq!(
            hook.install().unwrap(),
            HookOperationOutcome::AlreadyCurrent
        );
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// The entry point shared by startup and the CLI repairs the same
    /// interrupted state and makes a repeated attempt a no-op.
    #[test]
    fn ensuring_repairs_a_toolchain_with_only_the_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();

        assert_eq!(hook.state().unwrap(), HookState::Repairable);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
        let written = mark_old_mtime(&hook.cargo);
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::AlreadyCurrent);
        assert_eq!(
            fs::metadata(&hook.cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// A failed staging write leaves a discoverable repair state, so a
    /// later attempt can complete after the write obstruction is gone.
    #[test]
    fn a_failed_shim_write_can_be_rediscovered_and_repaired() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::create_dir(&hook.staging).unwrap();

        assert!(hook.install().is_err());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
        assert!(!hook.cargo.exists());
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
        let toolchain = hook.cargo.parent().unwrap().parent().unwrap();
        let hook = discovered_hook(toolchain);
        assert_eq!(hook.state().unwrap(), HookState::Repairable);

        fs::remove_dir(&hook.staging).unwrap();
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(hook.state().unwrap(), HookState::Installed);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(fs::read_to_string(&hook.real).unwrap(), CARGO_NAME);
    }

    /// A repair failure must release its lock while leaving the saved
    /// cargo at its original path for the next attempt.
    #[test]
    fn a_failed_repair_releases_the_lock_and_preserves_saved_cargo() {
        let (_home, hook) = interrupted_toolchain();
        fs::create_dir(&hook.staging).unwrap();

        assert!(hook.install().is_err());
        assert!(!hook.cargo.exists());
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            HOOK_TEST_REAL_CARGO
        );
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());

        fs::remove_dir(&hook.staging).unwrap();
        assert_eq!(hook.ensure().unwrap(), HookOperationOutcome::Installed);
        assert_working_install(&hook);
    }

    /// A remover must leave both installed and repairable toolchains
    /// untouched while another operation owns their installation lock.
    #[test]
    fn removing_with_a_held_lock_leaves_the_toolchain_untouched() {
        for state in [HookState::Installed, HookState::Repairable] {
            let (_home, hook) = interrupted_toolchain();
            if state == HookState::Installed {
                hook.install().unwrap();
            }
            let lock_path = hook.cargo.with_file_name(SHIM_LOCK_NAME);
            let installation_lock = HookInstallationLock::acquire(lock_path.clone()).unwrap();

            assert_eq!(hook.remove().unwrap_err().kind(), ErrorKind::AlreadyExists);
            assert_eq!(hook.state().unwrap(), state);
            assert_eq!(
                fs::read_to_string(&hook.real).unwrap(),
                HOOK_TEST_REAL_CARGO
            );
            if state == HookState::Installed {
                assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
            } else {
                assert!(!hook.cargo.exists());
            }
            assert!(!hook.staging.exists());
            assert!(lock_path.exists());

            drop(installation_lock);
            assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
            assert_eq!(
                fs::read_to_string(&hook.cargo).unwrap(),
                HOOK_TEST_REAL_CARGO
            );
            assert!(!hook.real.exists());
            assert!(!lock_path.exists());
        }
    }

    /// Starting from the interrupted-install state must preserve an
    /// executable cargo whichever competing operation acquires the lock first.
    #[test]
    fn concurrent_install_and_remove_preserve_a_working_cargo() {
        // The restored cargo must classify as Absent, so this fixture
        // omits the marker used by the saved-cargo repair tests.
        let real = HOOK_TEST_REAL_CARGO.replace(SHIM_MARKER, CARGO_NAME);
        let (_home, hook) = toolchain(&real);
        fs::set_permissions(&hook.cargo, fs::Permissions::from_mode(SHIM_MODE)).unwrap();
        fs::rename(&hook.cargo, &hook.real).unwrap();
        let operations = [Hook::install, Hook::remove];
        let start = Barrier::new(operations.len());

        let changes = thread::scope(|scope| {
            let start = &start;
            let hook = &hook;
            let attempts = operations.map(|operation| {
                scope.spawn(move || {
                    start.wait();
                    operation(hook)
                })
            });
            attempts.map(|attempt| attempt.join().unwrap().unwrap())
        });

        assert_eq!(
            changes,
            [
                HookOperationOutcome::Installed,
                HookOperationOutcome::Removed
            ]
        );
        assert!(matches!(
            hook.state().unwrap(),
            HookState::Installed | HookState::Absent
        ));
        if hook.real.exists() {
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
            assert_eq!(fs::read_to_string(&hook.real).unwrap(), real);
        } else {
            assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), real);
        }
        let output = Command::new(&hook.cargo)
            .arg(HOOK_TEST_VERSION_ARGUMENT)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            output.stdout,
            format!("{HOOK_TEST_VERSION_ARGUMENT}\n").as_bytes()
        );
        assert!(output.stderr.is_empty());
        assert!(!hook.staging.exists());
        assert!(!hook.cargo.with_file_name(SHIM_LOCK_NAME).exists());
    }

    /// Uninstalling must also restore a cargo left under its saved name
    /// by a failed install, without installing a shim first.
    #[test]
    fn removing_an_interrupted_install_restores_the_real_cargo() {
        let (_home, hook) = toolchain(CARGO_NAME);
        fs::rename(&hook.cargo, &hook.real).unwrap();

        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
        assert_eq!(hook.state().unwrap(), HookState::Absent);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), CARGO_NAME);
        assert!(!hook.real.exists());
        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::AlreadyAbsent);
    }

    /// What `rustup update` leaves behind: a fresh real cargo back on the
    /// name, with the previous real binary still beside it.
    #[test]
    fn a_toolchain_update_that_overwrote_the_shim_is_repaired_by_installing() {
        let (_home, hook) = toolchain("\u{7f}ELF old cargo");
        hook.install().unwrap();
        fs::write(&hook.cargo, "\u{7f}ELF cargo as rustup just reinstalled it").unwrap();

        assert_eq!(hook.state().unwrap(), HookState::Absent);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Installed);
        assert_eq!(
            fs::read_to_string(&hook.real).unwrap(),
            "\u{7f}ELF cargo as rustup just reinstalled it"
        );
    }

    #[test]
    fn removing_gives_the_real_cargo_its_name_back() {
        let real = "\u{7f}ELF the one and only real cargo";
        let (_home, hook) = toolchain(real);
        hook.install().unwrap();

        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Removed);
        assert_eq!(hook.state().unwrap(), HookState::Absent);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), real);
        assert!(!hook.real.exists());
    }

    #[test]
    fn removing_what_was_never_installed_changes_nothing() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");

        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::AlreadyAbsent);
        assert_eq!(
            fs::read_to_string(&hook.cargo).unwrap(),
            "\u{7f}ELF real cargo"
        );
    }

    #[test]
    fn a_shim_with_no_real_cargo_beside_it_reports_orphaned_and_refuses_removal() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");
        hook.install().unwrap();
        fs::remove_file(&hook.real).unwrap();

        assert_eq!(hook.state().unwrap(), HookState::Orphaned);
        assert_eq!(hook.install().unwrap(), HookOperationOutcome::Orphaned);
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
        assert!(!hook.real.exists());
        assert_eq!(hook.remove().unwrap(), HookOperationOutcome::Orphaned);
    }

    /// The shim is renamed into place, never written there, so a `sh`
    /// that is part way through reading it keeps the file it opened.
    #[test]
    fn writing_the_shim_leaves_nothing_staged_beside_it() {
        let (_home, hook) = toolchain("\u{7f}ELF real cargo");
        hook.install().unwrap();
        hook.install().unwrap();

        assert!(!hook.staging.exists());
        assert_eq!(fs::read_to_string(&hook.cargo).unwrap(), SHIM_SOURCE);
    }

    /// A rustup home holding one toolchain per entry, each with a cargo
    /// holding `contents`, in the order [`Hook::all`] would list them.
    fn toolchains(entries: &[(&str, &str)]) -> (TempDir, Vec<Hook>) {
        let home = tempdir().unwrap();
        for (name, contents) in entries {
            let binaries = home
                .path()
                .join(TOOLCHAINS_DIR)
                .join(name)
                .join(TOOLCHAIN_BIN_DIR);
            fs::create_dir_all(&binaries).unwrap();
            fs::write(binaries.join(CARGO_NAME), contents).unwrap();
        }
        let mut hooks: Vec<Hook> = entries
            .iter()
            .map(|(name, _)| discovered_hook(&home.path().join(TOOLCHAINS_DIR).join(name)))
            .collect();
        hooks.sort_by(|left, right| left.name.cmp(&right.name));
        (home, hooks)
    }

    #[test]
    fn startup_installs_where_nothing_is_and_says_which_toolchains() {
        let (_home, hooks) = toolchains(&[
            ("stable-test", "\u{7f}ELF stable"),
            ("nightly-test", "\u{7f}ELF nightly"),
        ]);

        let startup = stand_up(&hooks);

        assert_eq!(
            startup,
            Startup {
                installed: vec!["nightly-test".to_owned(), "stable-test".to_owned()],
                ..Startup::default()
            }
        );
        assert!(
            hooks
                .iter()
                .all(|hook| hook.state().unwrap() == HookState::Installed)
        );
    }

    /// Startup must include a repaired toolchain in its existing
    /// installed notice so the restored capture is visible.
    #[test]
    fn startup_reports_a_repaired_toolchain_as_installed() {
        let (_home, hook) = interrupted_toolchain();
        let name = hook.name.clone();
        let hooks = [hook];

        let startup = stand_up(&hooks);

        assert_eq!(
            startup,
            Startup {
                installed: vec![name],
                ..Startup::default()
            }
        );
        assert_working_install(&hooks[0]);
    }

    /// Every launch after the first finds this, and it must change
    /// nothing and say nothing.
    #[test]
    fn startup_over_a_current_shim_is_quiet() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        let written = mark_old_mtime(&hooks[0].cargo);

        let startup = stand_up(&hooks);

        assert!(startup.is_quiet());
        assert_eq!(
            fs::metadata(&hooks[0].cargo).unwrap().modified().unwrap(),
            written
        );
    }

    /// A shim an earlier cargo-tile wrote still carries the marker, so
    /// it is installed rather than absent -- and out of date.
    #[test]
    fn startup_brings_a_stale_shim_up_to_this_binarys_copy() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        fs::write(
            &hooks[0].cargo,
            format!("#!/bin/sh\n# {SHIM_MARKER} from an earlier version\n"),
        )
        .unwrap();

        let startup = stand_up(&hooks);

        assert_eq!(startup.refreshed, vec!["stable-test".to_owned()]);
        assert_eq!(fs::read_to_string(&hooks[0].cargo).unwrap(), SHIM_SOURCE);
        assert_eq!(
            fs::read_to_string(&hooks[0].real).unwrap(),
            "\u{7f}ELF stable"
        );
    }

    #[test]
    fn startup_reports_an_orphaned_shim_and_leaves_it_alone() {
        let (_home, hooks) = toolchains(&[("stable-test", "\u{7f}ELF stable")]);
        stand_up(&hooks);
        fs::remove_file(&hooks[0].real).unwrap();

        let startup = stand_up(&hooks);

        assert_eq!(startup.orphaned, vec!["stable-test".to_owned()]);
        assert_eq!(hooks[0].state().unwrap(), HookState::Orphaned);
        assert!(!hooks[0].real.exists());
    }

    /// One toolchain refusing must not stop the shim going in front of
    /// the others.
    #[test]
    fn startup_carries_on_past_a_toolchain_that_cannot_be_written() {
        let (_home, hooks) = toolchains(&[
            ("nightly-test", "\u{7f}ELF nightly"),
            ("stable-test", "\u{7f}ELF stable"),
        ]);
        let locked = hooks[0].cargo.parent().unwrap().to_path_buf();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();

        let startup = stand_up(&hooks);

        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(startup.installed, vec!["stable-test".to_owned()]);
        assert_eq!(startup.failed.len(), 1);
        assert_eq!(startup.failed[0].0, "nightly-test");
        assert_eq!(hooks[0].state().unwrap(), HookState::Absent);
        assert_eq!(
            fs::read_to_string(&hooks[0].cargo).unwrap(),
            "\u{7f}ELF nightly"
        );
    }

    #[test]
    fn a_toolchain_with_no_cargo_in_it_is_not_a_hook() {
        let home = tempdir().unwrap();
        let toolchain = home.path().join(TOOLCHAINS_DIR).join("stable-test");
        fs::create_dir_all(toolchain.join(TOOLCHAIN_BIN_DIR)).unwrap();

        assert!(matches!(Hook::at(&toolchain), HookDiscovery::AbsentCargo));
    }
}
