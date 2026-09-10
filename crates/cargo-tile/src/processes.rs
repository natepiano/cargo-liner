//! Discovery of the `cargo` invocations running on this machine, rolled
//! up into the groups the display is built from.
//!
//! Scanning happens on a background thread and arrives over a channel, so
//! the render loop never pays for it. Each scan is two-phase: a cheap
//! full-system pass reading only pid, name, parent and start time, then a
//! targeted pass reading working directory and argv for the handful of
//! processes that turned out to be cargo. The expensive per-process reads
//! are therefore never spent on the `rustc` and `sccache` processes a
//! build churns through by the hundred.
//!
//! What comes out is not a flat list. One command a developer typed can
//! be a whole tree of cargo processes -- `cargo mend` driving a
//! `cargo nextest` suite that runs `cargo check` per crate -- and a flat
//! list reports that as a dozen unrelated rows. [`CargoGroup`] keeps the
//! tree: the outermost invocation leads, everything running under it
//! follows, and the summary can show one row per command with a count
//! beside it.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::io::ErrorKind;
use std::ops::Add;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use chrono::DateTime;
use chrono::Local;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;
use sysinfo::Users;
use tui_pane::kernel_parent;
use uuid::Uuid;

use crate::birth_stamp;
use crate::birth_stamp::BirthStamp;
use crate::birth_stamp::LifetimeEvidence;
use crate::birth_stamp::ProcessLifetime;
use crate::capture_root::CleanupRefusal;
use crate::capture_root::RootIncarnation;
use crate::capture_root::RootOwner;
use crate::capture_root::SharedCaptureDirectory;
use crate::config::Config;
use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_JSON_FORMAT_PREFIX;
use crate::constants::CARGO_MESSAGE_FORMAT_FLAG;
use crate::constants::CARGO_MESSAGE_FORMAT_JSON_PREFIX;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_QUIET_FLAGS;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
use crate::constants::CARGO_TOOLCHAIN_SELECTOR;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::CPU_REPORT_MILLIS;
use crate::constants::CPU_SMOOTHING_SECONDS;
use crate::constants::FLAG_MARK;
use crate::constants::HOME_ALIAS;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::SCCACHE_BINARY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::SELF_PROCESS_NAME;
use crate::constants::START_TIME_FORMAT;
use crate::constants::SUMMARY_HIDDEN_VALUED_FLAGS;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
use crate::constants::UNAVAILABLE_MEASUREMENT;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;
use crate::progress::Capture;
use crate::progress::CaptureFailure;
use crate::progress::CaptureKey;
use crate::progress::CaptureLookup;
use crate::progress::CaptureRoot;
use crate::progress::CaptureRootIndex;
use crate::progress::CaptureRoots;
use crate::progress::CaptureSelection;
use crate::progress::ConfirmedCapture;
use crate::progress::PathFailure;
use crate::registration::RegistrationCandidate;
use crate::registration::VerifiedRegistration;
use crate::registration::WorkingDirectoryIdentity;
use crate::registration::WriterHome;
use crate::sccache::SccacheServer;

/// Identity of an invocation, independent of its displayed process or row source.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum InvocationId {
    /// The verified registration directly represents this invocation.
    Captured(RunId),
    /// Uncaptured and nested invocations retain their own process lifetime.
    Process(ProcessIdentity),
}

/// Root incarnation and publication generation qualify one captured invocation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RunId {
    /// Startup root position alone cannot detect a replaced directory.
    pub(crate) root:        CaptureRootIndex,
    /// Descriptor identity changes when the directory itself is replaced.
    pub(crate) incarnation: RootIncarnation,
    /// The registration describes the shim, even when cargo supplies the row.
    pub(crate) shim_pid:    u32,
    /// Publication generations separate even identical pid and birth observations.
    pub(crate) generation:  String,
    /// Kernel comparison qualifies the generation without reducing its precision.
    pub(crate) birth:       BirthStamp,
}

/// Process lifetime evidence never substitutes registration comparison seconds.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum ProcessIdentity {
    /// Native kernel precision separates process replacements without a generation.
    Known {
        /// A birth stamp belongs to the process whose kernel entry was read.
        pid:      u32,
        /// Linux start ticks or the full Darwin start timeval, qualified by boot.
        lifetime: ProcessLifetime,
    },
    /// Continuous presence permits row retention without proving a kernel lifetime.
    Unavailable {
        /// Retain the displayed process even when its lifetime cannot be read.
        pid:         u32,
        /// Retired once this pid disappears from a scan, even if the pid returns.
        observation: Uuid,
    },
}

/// Retain unavailable row identities only while their pids remain continuously observed.
#[derive(Default)]
pub(crate) struct ProcessIdentities {
    /// This map is replaced by each scan, so absent pids cannot retain row continuity.
    present: HashMap<Pid, ProcessIdentity>,
}

impl ProcessIdentities {
    /// Kernel evidence controls known lifetimes; presence alone retains unavailable rows.
    pub(crate) fn observe(
        &mut self,
        lifetimes: &HashMap<Pid, LifetimeEvidence>,
    ) -> HashMap<Pid, InvocationId> {
        self.present = lifetimes
            .iter()
            .map(|(&pid, lifetime)| {
                let identity = match (lifetime, self.present.get(&pid)) {
                    (
                        LifetimeEvidence::Unavailable,
                        Some(identity @ ProcessIdentity::Unavailable { .. }),
                    ) => identity.clone(),
                    _ => ProcessIdentity::observed(pid.as_u32(), lifetime.clone()),
                };
                (pid, identity)
            })
            .collect();
        self.present
            .iter()
            .map(|(&pid, identity)| (pid, InvocationId::Process(identity.clone())))
            .collect()
    }
}

impl InvocationId {
    /// Stable synthetic lifetime for fixtures that are not kernel observations.
    #[cfg(test)]
    pub(crate) fn for_test(pid: u32) -> Self {
        Self::Process(ProcessIdentity::Known {
            pid,
            lifetime: crate::birth_stamp::ProcessLifetime::for_test(u64::from(pid)),
        })
    }
}

/// Capture membership supplies progress without granting registration row fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureMembership {
    /// This invocation runs inside the named capture and keeps its own identity.
    Enclosing(RunId),
    /// No enclosing capture supplies this invocation's progress.
    Outside,
}

/// A visible parent is either an invocation, a chain entry, or absent from the view.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VisibleParent {
    /// Family matching uses invocation identity rather than the displayed pid.
    Invocation {
        /// Family continuity follows the invocation across changes of row source.
        id:  InvocationId,
        /// Display the cargo parent pid even when the identity names its shim.
        pid: u32,
    },
    /// A non-cargo ancestor is drawn in the command's ancestry chain.
    Ancestor(u32),
    /// No ancestor is drawn for this invocation.
    None,
}

/// A reading remains distinct from every reason the scanner cannot establish one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Measurement<T> {
    /// The scanner has evidence for this value, including a measured zero.
    Reading(T),
    /// No value may be published while this reason applies.
    Unavailable(MeasurementAbsence),
}

/// Why a measurement cannot currently be published.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MeasurementAbsence {
    /// A rate requires a previous observation of the same process.
    FirstObservation,
    /// The collected sample establishes that a usable reading failed.
    ReadFailed,
    /// The available evidence cannot distinguish a reading from an unread value.
    Unproven,
}

impl<T> Measurement<T> {
    /// Transform a reading without manufacturing a value for an unavailable sample.
    pub(crate) fn map<U>(self, map: impl FnOnce(T) -> U) -> Measurement<U> {
        match self {
            Self::Reading(reading) => Measurement::Reading(map(reading)),
            Self::Unavailable(reason) => Measurement::Unavailable(reason),
        }
    }
}

impl<T: fmt::Display> Display for Measurement<T> {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Reading(reading) => reading.fmt(formatter),
            Self::Unavailable(_) => formatter.write_str(UNAVAILABLE_MEASUREMENT),
        }
    }
}

impl<T: Add<Output = T>> Add for Measurement<T> {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        match (self, other) {
            (Self::Reading(left), Self::Reading(right)) => Self::Reading(left + right),
            (Self::Unavailable(reason), _) | (_, Self::Unavailable(reason)) => {
                Self::Unavailable(reason)
            },
        }
    }
}

/// Compiler absence is an observed idle state; an unknown observation is separate.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CompilerObservation {
    /// The scanner cannot establish which compilers are running.
    Unknown,
    /// The scanner observed no compiler running for this invocation.
    None,
    /// The scanner observed this driver and count.
    Running(Compiler),
}

/// Convert the scanner's optional home at the external API boundary once.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScannerHome<'home> {
    /// Only this prefix may be shortened to the scanner's tilde.
    Known(&'home Path),
    /// No scanner home was observed, so every directory stays absolute.
    Unavailable,
}

impl<'home> From<Option<&'home Path>> for ScannerHome<'home> {
    fn from(home: Option<&'home Path>) -> Self { home.map_or(Self::Unavailable, Self::Known) }
}

/// Missing cwd cannot establish equality or be formatted before metadata merging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorkingDirectoryObservation<'directory> {
    /// The process API supplied its own working directory.
    Observed(&'directory Path),
    /// A direct registration may fill this field before display conversion.
    Unavailable,
}

impl<'directory> From<Option<&'directory Path>> for WorkingDirectoryObservation<'directory> {
    fn from(directory: Option<&'directory Path>) -> Self {
        directory.map_or(Self::Unavailable, Self::Observed)
    }
}

/// Display resolution never changes the numeric account identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccountName {
    /// The scanner resolved a nonempty account name for the root owner.
    Resolved(String),
    /// A missing passwd entry leaves the numeric owner visible.
    Unavailable,
}

/// The verified root owner and its independently resolved display label.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureAccount {
    /// Ownership identity comes from the opened root, never its pathname.
    pub(crate) uid:  u32,
    /// Failure to resolve a label does not change grouping.
    pub(crate) name: AccountName,
}

/// Root incarnation and owner qualify a captured working directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureContext {
    /// Stable account position distinguishes capture directories.
    pub(crate) root:        CaptureRootIndex,
    /// Replacing a directory invalidates its retained grouping identity.
    pub(crate) incarnation: RootIncarnation,
    /// Numeric identity remains separate from its display name.
    pub(crate) account:     CaptureAccount,
}

/// A row's capture context states whether registration fields may describe it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RowProvenance {
    /// No verified capture qualifies this row's own working directory.
    Uncaptured,
    /// This invocation owns its selected registration's metadata.
    Direct(CaptureContext),
    /// A nested invocation shares account and capture, retaining its own fields.
    Enclosing(CaptureContext),
}

impl RowProvenance {
    /// Verification came from this root; missing owner observations grant no qualification.
    fn direct(capture: &Capture, key: &CaptureKey) -> Self {
        let Some(status) = capture.root_status.get(key.root.0) else {
            return Self::Uncaptured;
        };
        let uid = status.root.uid;
        Self::Direct(CaptureContext {
            root:        key.root,
            incarnation: key.incarnation,
            account:     CaptureAccount {
                uid,
                name: status.account.clone(),
            },
        })
    }

    /// Enclosing membership qualifies a group without granting registration fields.
    fn enclosing(capture: &Capture, key: &CaptureKey) -> Self {
        match Self::direct(capture, key) {
            Self::Direct(context) => Self::Enclosing(context),
            provenance => provenance,
        }
    }
}

impl AccountName {
    /// Account lookup is a scan observation and never participates in identity comparison.
    pub(crate) fn resolve(owner: RootOwner, users: &Users) -> Self {
        let RootOwner::Uid(uid) = owner else {
            return Self::Unavailable;
        };
        users
            .iter()
            .find(|user| **user.id() == uid)
            .filter(|user| !user.name().is_empty())
            .map_or(Self::Unavailable, |user| {
                Self::Resolved(user.name().to_owned())
            })
    }
}

/// An unavailable registration timestamp sorts after known starts in ascending order.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum RunStart {
    /// Seconds since the epoch order both process and registration observations.
    Known(u64),
    /// A failed timestamp read cannot prevent a verified invocation's row.
    Unavailable,
}

/// One running `cargo` invocation, preformatted for the table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoProcess {
    /// Row source and displayed pid do not change invocation continuity.
    pub(crate) invocation_id:      InvocationId,
    /// Nested invocations share progress without inheriting registration metadata.
    pub(crate) capture_membership: CaptureMembership,
    /// Account and root qualification do not grant enclosing rows metadata ownership.
    pub(crate) provenance:         RowProvenance,
    /// Working directory display, shortened only when the home prefix is unambiguous.
    pub(crate) path:               String,
    /// Raw absolute directory for grouping; display formatting cannot change membership.
    pub(crate) directory_identity: WorkingDirectoryIdentity,
    /// Process id.
    pub(crate) pid:                u32,
    /// The nearest ancestor the cell draws: the cargo above this one
    /// where there is one, since that is a row of the same table, and
    /// otherwise the step of the chain block the command was started
    /// from. Never the immediate parent, which is the pty and shim the
    /// capture opened and is drawn nowhere.
    ///
    /// The named parent state distinguishes invocation identity from chain display.
    pub(crate) parent:             VisibleParent,
    /// Local wall-clock start time, `hh:mm`.
    pub(crate) start:              String,
    /// The same instant as seconds since the epoch, which is what
    /// orders one invocation against another. The label above it is
    /// only accurate to the minute and turns over at midnight, so it
    /// reads well and sorts badly.
    pub(crate) started:            RunStart,
    /// Elapsed run time, `mm:ss` until an hour and `hh:mm:ss` past it.
    pub(crate) duration:           String,
    /// Share of a core this invocation and everything running under it
    /// are using, as a whole-number percent. `top`'s scale rather than a
    /// share of the machine, so a build across eight cores reads past
    /// 100% instead of flattening to a tenth of one. An unavailable
    /// contributor makes the invocation's whole measurement unavailable.
    pub(crate) cpu:                Measurement<String>,
    /// Compiler processes this invocation currently owns, when observed. On the
    /// invocation leading a group this is the whole group's tally, so
    /// the summary reports the build rather than the driver process.
    pub(crate) compiler:           CompilerObservation,
    /// What the command is doing, when a capture of its output is there
    /// to read it from. Read off the nearest capture at or above the
    /// invocation, so a cargo the enclosing run started -- which the
    /// shim declines to capture a second time -- reports the run it is
    /// inside rather than nothing at all.
    pub(crate) state:              CaptureLookup,
    /// Cargo invocations running under this one. A measured zero names
    /// a plain command; an unavailable count cannot establish that it is idle.
    pub(crate) managed:            Measurement<usize>,
    /// Whether another cargo stands between this invocation and the
    /// lead of its group. False for the lead itself and for the
    /// invocations it started directly.
    ///
    /// What the summary keeps out. A command's own cell lists its whole
    /// tree, which is where the tree is worth reading; gathered into
    /// one table with every other command's, the deeper levels bury the
    /// runs they came from -- one `cargo nextest run` puts a `cargo
    /// mend` in the table for every test it runs.
    pub(crate) nested:             bool,
    /// The command line, split so program and arguments style apart.
    pub(crate) command:            CommandText,
}

/// The compiler driver an invocation is running, and how many at once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Compiler {
    /// Driver name, one of [`COMPILER_PROCESS_NAMES`].
    pub(crate) name:  &'static str,
    /// How many of it are running under this cargo invocation.
    pub(crate) count: usize,
}

/// A command line split into its program and the rest of its arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommandText {
    /// Program name, path stripped.
    pub(crate) program: String,
    /// Remaining arguments, one entry per argv word. Held split rather
    /// than joined because a cell may leave one of them out;
    /// [`CommandText::line`] is what puts them back into a line.
    arguments:          Vec<String>,
}

/// How much of an invocation's command line a cell prints.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SummaryDetail {
    /// The whole line, the way a command's own cell shows it.
    Full,
    /// The manifest path and the summary's noise flags left out, the
    /// way the summary shows it. Every row there already sits under the
    /// working directory heading its group, and cargo is handed the
    /// manifest as an absolute path -- long enough to push the
    /// subcommand off the edge of a narrow cell to repeat what the
    /// header just said.
    Trimmed,
}

impl CommandText {
    /// A command line built from its parts, for the tests elsewhere in
    /// the crate that need an invocation to hand around.
    #[cfg(test)]
    pub(crate) fn of(program: &str, arguments: &[&str]) -> Self {
        Self {
            program:   program.to_string(),
            arguments: arguments.iter().map(|word| (*word).to_string()).collect(),
        }
    }

    /// The cargo subcommand this invocation names: `port` in
    /// `cargo port`, and in `cargo +nightly port` too, the toolchain
    /// selector being no part of it.
    ///
    /// [`command_text`] puts an external subcommand's own name back at
    /// the front of the arguments, so a command that became
    /// `cargo-port` answers this the same as one still spelled
    /// `cargo port`.
    fn subcommand(&self) -> Option<&str> {
        self.arguments
            .iter()
            .map(String::as_str)
            .find(|argument| !argument.starts_with(CARGO_TOOLCHAIN_SELECTOR))
    }

    /// Whether `commands.hidden_when_idle` names this command's
    /// subcommand.
    ///
    /// Half the answer to whether the grid gives the command a cell --
    /// the other half is whether anything is running under it, which
    /// [`crate::roster::TrackedGroup::deserves_a_cell`] puts together
    /// with this.
    pub(crate) fn is_hidden_when_idle(&self, hidden_when_idle: &[String]) -> bool {
        self.subcommand()
            .is_some_and(|subcommand| hidden_when_idle.iter().any(|hidden| hidden == subcommand))
    }

    /// The arguments that still name what runs, with everything the
    /// command was called *with* taken off: `mend` out of `mend
    /// --manifest-path /tmp/x/Cargo.toml --json`, and `nextest run` out
    /// of the whole of `nextest run --workspace --all-features`.
    ///
    /// Keeps the toolchain selector, which is part of what runs rather
    /// than an argument to it -- `+nightly fmt` says something `fmt`
    /// alone does not.
    pub(crate) fn named(&self) -> String {
        self.arguments
            .iter()
            .map(String::as_str)
            .take_while(|word| names_the_command(word))
            .collect::<Vec<&str>>()
            .join(" ")
    }

    /// The arguments as one line, the summary's own flags in or out.
    pub(crate) fn line(&self, detail: SummaryDetail) -> String {
        if detail == SummaryDetail::Full {
            return self.arguments.join(" ");
        }
        let mut kept: Vec<&str> = Vec::with_capacity(self.arguments.len());
        let mut skipping = false;
        let mut handed_over = false;
        for argument in &self.arguments {
            // Everything past a bare `--` belongs to the program cargo
            // runs, which spells its flags however it likes. Nothing
            // there is cargo's to read, so nothing there is dropped.
            if handed_over {
                kept.push(argument);
                continue;
            }
            if argument == ARGUMENT_SEPARATOR {
                handed_over = true;
                kept.push(argument);
                continue;
            }
            // The word after a bare `--color` is the value it takes,
            // and goes wherever the flag goes.
            if std::mem::take(&mut skipping) {
                continue;
            }
            if SUMMARY_HIDDEN_VALUED_FLAGS.contains(&argument.as_str()) {
                skipping = true;
                continue;
            }
            if SUMMARY_HIDDEN_VALUED_FLAGS
                .iter()
                .any(|flag| is_assignment(argument, flag))
            {
                continue;
            }
            kept.push(argument);
        }
        kept.join(" ")
    }
}

/// One whole command line with its arguments taken off, for the steps
/// of a chain, which are held as a line rather than split.
///
/// The first word is the program however it is spelled, path and all --
/// a chain step is often reached by its path, and dropping that would
/// leave a bare `node` or `sh` saying less than the row it heads.
/// Everything after it is kept only while it still names what runs.
pub(crate) fn command_name(line: &str) -> String {
    let mut words = line.split_whitespace();
    let Some(program) = words.next() else {
        return String::new();
    };
    std::iter::once(program)
        .chain(words.take_while(|word| names_the_command(word)))
        .collect::<Vec<&str>>()
        .join(" ")
}

/// Whether a word standing after the program still names the command
/// rather than arguing with it.
///
/// Two answers rule a word out: a leading dash, which is a flag, and a
/// path separator, which is a manifest or a target directory or a
/// binary reached by its path. What survives is the subcommands and the
/// toolchain selector, which is the name of what runs.
fn names_the_command(word: &str) -> bool {
    !word.starts_with(FLAG_MARK) && !word.contains(std::path::MAIN_SEPARATOR)
}

/// Whether an argument is the `--flag=<value>` spelling, which carries
/// the value in the same word instead of the next one.
fn is_assignment(argument: &str, flag: &str) -> bool {
    argument
        .strip_prefix(flag)
        .is_some_and(|rest| rest.starts_with('='))
}

/// One process standing above a command in the process tree.
///
/// What a cell lists to say where the command came from: a shell, an
/// editor, the agent or script that typed it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Ancestor {
    /// Process id.
    pub(crate) pid:            u32,
    /// What the process is, as [`describe`] reads it.
    pub(crate) command:        String,
    /// Whether the process passed a command through rather than
    /// starting it, per [`is_transparent`]. Whether a cell draws one of
    /// these is settled where the chain is drawn rather than here: the
    /// exception is the foot of the chain, and a driver's cell closes
    /// its own chain with the driver, which moves where the foot is.
    pub(crate) passes_through: bool,
}

/// One command and every cargo invocation running under it.
///
/// A plain `cargo build` is a group of one. A command that drives other
/// cargo commands is one group holding all of them, which is what lets
/// the summary carry the command that was typed with a count beside it
/// instead of the fan-out it became.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CargoGroup {
    /// The outermost invocation: the command that was typed, and the row
    /// the summary carries.
    pub(crate) lead:     CargoProcess,
    /// Everything running under [`lead`](Self::lead), newest first.
    /// Empty for a plain command.
    pub(crate) rest:     Vec<CargoProcess>,
    /// What stands above [`lead`](Self::lead), outermost first, ending
    /// at the process that started it. Empty for a command whose
    /// parents cannot be read.
    pub(crate) ancestry: Vec<Ancestor>,
}

impl CargoGroup {
    /// The group's identity, stable for as long as the command runs.
    pub(crate) fn id(&self) -> InvocationId { self.lead.invocation_id.clone() }
}

/// One scan's account of the machine: the cargo commands running, and
/// whether an sccache server is up behind them.
///
/// The two travel together because they are read together. Phase one
/// already names every process to find the compilers under each cargo,
/// and a running server is one more name in that same pass -- which is
/// what makes the answer free, and what keeps the summary's stats read
/// from having to start a server to discover whether one is running.
pub(crate) struct Scan {
    /// The commands running, newest first.
    pub(crate) groups:           Vec<CargoGroup>,
    /// Whether a process named [`SCCACHE_BINARY`] was among them.
    pub(crate) sccache:          SccacheServer,
    /// Settings reads these observations without reopening any capture path.
    pub(crate) root_status:      Vec<AccountCaptureDirectory>,
    pub(crate) shared_directory: SharedCaptureDirectory,
}

/// One effective root's access, identity and capture observations for this scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountCaptureDirectory {
    /// The directory name claims an account; `state` records owner verification.
    pub(crate) root:         CaptureRoot,
    /// Nofollow metadata identifies rejected owners; accepted roots use their descriptor.
    pub(crate) owner:        RootOwner,
    /// Resolved on the scan worker; rendering performs no account lookup.
    pub(crate) account:      AccountName,
    /// Access refusals supplement `state`; only accepted roots permit cleanup.
    pub(crate) cleanup:      Vec<CleanupRefusal>,
    /// Each scan reopens the directory and reports current access.
    pub(crate) state:        RootReadStatus,
    /// Published, verified registrations whose logs were readable in this scan.
    pub(crate) confirmed:    usize,
    /// Failures and retained artifacts remain visible without process-table rows.
    pub(crate) diagnostics:  Vec<CaptureDiagnostic>,
    /// Each association names its process and any competing proof left unused.
    pub(crate) associations: Vec<CaptureAssociation>,
}

/// Access to an account directory is observed again on every scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RootReadStatus {
    /// The root handle opened; diagnostics describe any incomplete contents.
    Readable,
    /// Reopening this absolute path failed in the current scan.
    Unavailable(PathFailure),
    /// A real directory owned by a different uid supplies no captures.
    ForeignOwned { owner: AccountName },
}

impl AccountCaptureDirectory {
    /// Inspect the final component without following links before reading captures.
    pub(crate) fn inspect(root: CaptureRoot, users: &Users) -> Self {
        let mut status = Self {
            account: AccountName::resolve(RootOwner::Uid(root.uid), users),
            root,
            owner: RootOwner::Unavailable,
            cleanup: Vec::new(),
            state: RootReadStatus::Readable,
            confirmed: 0,
            diagnostics: Vec::new(),
            associations: Vec::new(),
        };
        let metadata = std::fs::symlink_metadata(&status.root.path).and_then(|metadata| {
            if metadata.is_dir() {
                Ok(metadata)
            } else {
                Err(ErrorKind::NotADirectory.into())
            }
        });
        match metadata {
            Ok(metadata) => {
                status.owner = RootOwner::Uid(metadata.uid());
                if metadata.uid() != status.root.uid {
                    status.state = RootReadStatus::ForeignOwned {
                        owner: AccountName::resolve(status.owner, users),
                    };
                }
            },
            Err(error) => {
                let failure = PathFailure {
                    path:    status.root.path.clone(),
                    failure: error.into(),
                };
                status.cleanup.push(CleanupRefusal::Access(failure.clone()));
                status.state = RootReadStatus::Unavailable(failure);
            },
        }
        status
    }
}

/// Path-qualified observations supplement the count of verified, readable captures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureDiagnostic {
    /// A short directory inventory can omit runs and prohibits cleanup this scan.
    EnumerationIncomplete(PathBuf),
    /// An inaccessible directory must never read as an empty inventory.
    EnumerationFailed(PathFailure),
    /// An individual registration failed to read; sibling evidence stays available.
    RegistrationUnreadable(PathFailure),
    /// Malformed bytes or mismatched generation cannot establish an association.
    RegistrationInvalid(PathBuf),
    /// A legacy record can annotate a process row but supplies no verifiable identity.
    AnnotationOnly(PathBuf),
    /// Missing identity fields cannot authorize cleanup, even after the pid ends.
    Unverifiable(PathBuf),
    /// The record supplies identity, but this scan could not observe the live process.
    IdentityUnknown(PathBuf),
    /// The cached boot failure prevents checking this record until a restart.
    IdentityBlockedByBoot(PathBuf),
    /// An unpublished artifact stays outside the active capture count.
    Staging(PathBuf),
    /// Retain the exact named log and its I/O failure even without a process row.
    LogUnreadable(PathFailure),
    /// The cached boot read disables verification until a restart retries it.
    BootUnavailable(PathFailure),
}

/// Final selection reaches settings even when no process row can be built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureAssociation {
    /// The displayed process, or the shim when the selection sources no process row.
    pub(crate) pid:       u32,
    /// Retain both successful selection and unresolved competition.
    pub(crate) selection: AssociationSelection,
}

/// Root precedence and verification determine which proof may supply fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AssociationSelection {
    /// One key was selected; other proofs cannot supply metadata.
    Selected {
        /// The exact root, incarnation and generation selected.
        key:    CaptureKey,
        /// Unconfirmed selections permit only log annotation.
        proof:  SelectedProof,
        /// Every confirmed proof left unused has an explicit reason.
        unused: Vec<UnusedCapture>,
    },
    /// Competing publications prevent metadata ownership and ancestor fallback.
    Ambiguous {
        /// Exact identities of the publications that competed in the preferred root.
        candidates: Vec<CaptureKey>,
    },
}

/// Selection and verification are independent facts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SelectedProof {
    /// The selected key has a retained verification proof.
    Confirmed,
    /// The selected reading has no proof and cannot source a row.
    Unconfirmed,
}

/// A retained proof that the selection forbids using.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct UnusedCapture {
    /// Preserve publication identity as well as its operator-facing path.
    pub(crate) key:    CaptureKey,
    /// Settings can name the root without reopening it.
    pub(crate) root:   PathBuf,
    /// Explain why this proof supplied neither another row nor borrowed fields.
    pub(crate) reason: UnusedCaptureReason,
}

/// A preferred root remains authoritative whether or not its key was verified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UnusedCaptureReason {
    /// The selected confirmed publication wins account discovery order.
    RootPrecedence,
    /// The preferred reading is unconfirmed, so another root cannot repair it.
    SelectedUnconfirmed,
}

/// The nearest registered ancestor retains its root for every annotation lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
enum NearestRegistration {
    /// No registration exists along the bounded parent walk.
    Unregistered,
    /// Root precedence selects one registration without mixing its sibling roots.
    Registered(CaptureKey),
    /// Competition stops the ancestry walk and retains the competing identities.
    Ambiguous(Vec<CaptureKey>),
}

/// A verified registration established to directly represent this invocation.
/// Only membership resolution can construct this value; an enclosing capture cannot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirectCapture {
    /// The same identity is used by a process row and a registration row.
    run_id:       RunId,
    /// Metadata and log lookup use the exact selected publication.
    key:          CaptureKey,
    /// The descriptor-bound timestamp survives independently of verification.
    modified:     Result<SystemTime, CaptureFailure>,
    /// Keep the existing verifier's proof, including its permitted directory and argv.
    registration: VerifiedRegistration,
}

impl From<&ConfirmedCapture> for DirectCapture {
    fn from(confirmed: &ConfirmedCapture) -> Self {
        Self {
            run_id:       RunId::verified(&confirmed.key, &confirmed.registration),
            key:          confirmed.key.clone(),
            modified:     confirmed.modified.clone(),
            registration: confirmed.registration.clone(),
        }
    }
}

impl DirectCapture {
    /// Row construction consumes this proof rather than an enclosing membership.
    pub(crate) const fn registration(&self) -> &VerifiedRegistration { &self.registration }

    /// A change of row source cannot change the registered invocation's identity.
    fn invocation_id(&self) -> InvocationId { InvocationId::Captured(self.run_id.clone()) }
}

/// Only the direct arm permits access to verified row metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DirectAssociation {
    /// The registration represents this row's command, rather than an ancestor command.
    Direct(Box<DirectCapture>),
    /// No verified registration directly describes this process.
    None,
}

impl RunId {
    /// Bind the verified generation to the actual directory scanned this time.
    fn verified(key: &CaptureKey, registration: &VerifiedRegistration) -> Self {
        Self {
            root:        key.root,
            incarnation: key.incarnation,
            shim_pid:    registration.pid(),
            generation:  registration.record().generation().to_owned(),
            birth:       registration.birth().clone(),
        }
    }
}

impl ProcessIdentity {
    /// Unavailable evidence receives only a row token, never a claimed lifetime.
    fn observed(pid: u32, evidence: LifetimeEvidence) -> Self {
        match evidence {
            LifetimeEvidence::Available(lifetime) => Self::Known { pid, lifetime },
            LifetimeEvidence::Unavailable => Self::Unavailable {
                pid,
                observation: uuid::Uuid::now_v7(),
            },
        }
    }
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
///
/// The caller only snapshots command exclusions. Parent resolution runs once inside
/// the worker before its scan loop, so a slow filesystem cannot block terminal
/// startup; each scan still opens every root to recheck access and ownership.
/// Keep root resolution on the worker even when it stalls. The resolver and
/// join handle let tests hold resolution and observe repeated scans and shutdown.
pub(crate) fn spawn_with_resolver(
    config: &Config,
    resolve: impl FnOnce() -> CaptureRoots + Send + 'static,
) -> (Receiver<Scan>, JoinHandle<()>) {
    let excluded = config.commands.excluded.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || {
        let roots = resolve();
        let mut system = System::new();
        let mut smoothing = CpuSmoothing::default();
        let home = dirs::home_dir();
        let scanner_home = home.as_deref().into();
        loop {
            if sender
                .send(scan(
                    &mut system,
                    &mut smoothing,
                    Instant::now(),
                    scanner_home,
                    &excluded,
                    &roots,
                ))
                .is_err()
            {
                return;
            }
            thread::sleep(Duration::from_millis(PROCESS_POLL_MILLIS));
        }
    });
    (receiver, worker)
}

/// One two-phase scan, newest group first.
///
/// `smoothing` outlives the scan because a CPU share is settled across
/// scans rather than read out of one, and `now` is what tells it when
/// the table is due a fresh reading.
fn scan(
    system: &mut System,
    smoothing: &mut CpuSmoothing,
    now: Instant,
    home: ScannerHome<'_>,
    excluded: &[String],
    roots: &CaptureRoots,
) -> Scan {
    let previous = system
        .processes()
        .iter()
        .map(|(&pid, process)| {
            (
                pid,
                CpuBaseline {
                    lifetime:    smoothing
                        .observed
                        .get(&pid)
                        .cloned()
                        .unwrap_or(LifetimeEvidence::Unavailable),
                    accumulated: process.accumulated_cpu_time(),
                },
            )
        })
        .collect();
    // Phase one: pid, name, parent and start time for everything. None of
    // the fields this asks for require a per-process read of the argument
    // area, which is what makes it cheap enough to poll continuously.
    // `with_cpu` on the cheap pass rather than the targeted one: the
    // work a cargo command is doing runs in the `rustc` and `sccache`
    // processes under it, and phase two never looks at those. sysinfo
    // reads a share as the delta between two refreshes of the same
    // process. Census keeps a first observation unavailable because it
    // has no preceding sample from which to establish a rate.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        process_discovery_refresh_kind(),
    );

    let mut census = Census::take(system, &previous);
    census.identities = smoothing.identities.observe(&census.lifetimes);

    // Phase two: the costly fields, for the cargo processes and for the
    // handful standing above each of them. The ancestors are read for
    // the same reason the invocations are -- a cell says what launched
    // the command -- and a chain is a few processes long, against the
    // hundreds this pass still skips.
    // Verification precedes row filtering, and registration pids must receive fresh
    // argv even when the cheap snapshot retains a pre-exec process name.
    let mut capture = Capture::take(roots);
    let mut detailed = census.detailed();
    for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
        for candidate in std::iter::once(pid).chain(census.ancestor_pids(pid)) {
            if !detailed.contains(&candidate) {
                detailed.push(candidate);
            }
        }
    }
    let details = process_details(&detailed);
    census.include_registered_processes(&details, &capture);
    census.identify_capture_wrappers(&details, &capture);
    census.collapse_shims(&details);
    census.identify_captures(&capture);
    census.select_rows(&details, &capture, excluded);
    let attributed = census.attribute(smoothing, now);
    let groups = census.groups(&details, &attributed, home, &capture);
    census.associate_status(&mut capture, &groups);
    Scan {
        sccache: census.sccache(),
        groups,
        root_status: capture.root_status,
        shared_directory: capture.shared_directory,
    }
}

/// Fields read for the full-system pass.
///
/// Tasks are Linux threads. They inherit a process's name and command,
/// so including them would draw one cargo invocation more than once.
fn process_discovery_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing().without_tasks().with_cpu()
}

/// Fields read for cargo processes and their ancestors.
fn process_detail_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cwd(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
}

/// sysinfo retains populated metadata even when a requested reread fails.
/// Fresh detail records cannot carry cwd, argv or exe across a pid replacement
/// that its whole-second start time cannot distinguish.
fn process_details(pids: &[Pid]) -> System {
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(pids),
        false,
        process_detail_refresh_kind(),
    );
    system
}

/// What the census worked out per cargo invocation, once every process
/// under one has been walked up to it.
struct Attributed {
    /// Every invocation has an observation, including one compiling nothing.
    compilers: HashMap<Pid, CompilerObservation>,
    /// The settled CPU share each invocation and everything under it
    /// add up to, including unavailable contributors.
    cpu:       HashMap<Pid, Measurement<f32>>,
}

/// Evidence retained across a refresh to establish identity and monotonic CPU time.
#[derive(Clone)]
struct CpuBaseline {
    /// A reused pid must begin a fresh rate observation.
    lifetime:    LifetimeEvidence,
    /// A decrease for the same process proves the samples cannot form a valid rate.
    accumulated: u64,
}

impl From<&Process> for CpuBaseline {
    fn from(process: &Process) -> Self {
        Self {
            lifetime:    birth_stamp::lifetime(process.pid().as_u32()),
            accumulated: process.accumulated_cpu_time(),
        }
    }
}

/// Whether the smoother has ever published a snapshot and when it last did so.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum CpuPublication {
    /// No previous publication exists to retain.
    #[default]
    NeverPublished,
    /// Readings may be held until the reporting interval expires.
    Published(Instant),
}

/// Each cargo invocation's CPU share as the table reports it, carried
/// between scans.
///
/// Two separate things happen here, and they run at different speeds.
/// One scan's sample is a quarter second of a process's life, which for
/// anything that works in bursts says more about where the sample landed
/// than about what the command is doing, so each invocation's reading is
/// carried part way toward its latest sample rather than replaced by it,
/// over the window [`CPU_SMOOTHING_SECONDS`] names. That happens on every
/// scan. What the table is given, though, is held for
/// [`CPU_REPORT_MILLIS`] at a time: a smooth figure redrawn four times a
/// second is still a figure nobody can read.
#[derive(Default)]
struct CpuSmoothing {
    /// Unavailable lifetime reads retain rows only during continuous pid presence.
    identities:  ProcessIdentities,
    /// Bind the pre-refresh counters to the lifetime observed with their last sample.
    observed:    HashMap<Pid, LifetimeEvidence>,
    /// Where each invocation's reading has settled, moved on every scan.
    /// Keyed by invocation identity, so a replaced pid cannot inherit old readings.
    settled:     HashMap<InvocationId, Measurement<f32>>,
    /// What the table is carrying, taken from
    /// [`settled`](Self::settled) when a reading falls due.
    reported:    HashMap<InvocationId, Measurement<f32>>,
    /// A never-published smoother has no previous snapshot to hold.
    publication: CpuPublication,
}

impl CpuSmoothing {
    /// Carry every invocation's reading toward what this scan sampled,
    /// and hand back what the table should show at `now`.
    ///
    /// An unavailable sample immediately replaces any published reading,
    /// even between reporting deadlines. Recovery starts at its own value
    /// because a sample gap cannot contribute to a smoothed rate.
    fn settle(
        &mut self,
        sampled: &HashMap<InvocationId, Measurement<f32>>,
        cargo: &[InvocationId],
        now: Instant,
    ) -> HashMap<InvocationId, Measurement<f32>> {
        self.settled.retain(|pid, _| cargo.contains(pid));
        self.reported.retain(|pid, _| cargo.contains(pid));
        let alpha = smoothing_alpha();
        for pid in cargo {
            let sample = sampled
                .get(pid)
                .copied()
                .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven));
            let settled = self.settled.entry(pid.clone()).or_insert(sample);
            *settled = match (sample, *settled) {
                (Measurement::Reading(sample), Measurement::Reading(previous)) => {
                    Measurement::Reading((sample - previous).mul_add(alpha, previous))
                },
                (sample, _) => sample,
            };
            if matches!(sample, Measurement::Unavailable(_)) {
                self.reported.insert(pid.clone(), sample);
            }
        }
        if self.is_due(now) {
            self.reported.clone_from(&self.settled);
            self.publication = CpuPublication::Published(now);
        } else {
            // An invocation that has only just started has nothing being
            // held for it, and waiting out the rest of somebody else's
            // second would draw it idle. Its opening reading goes
            // straight through.
            for (pid, &settled) in &self.settled {
                let reported = self.reported.entry(pid.clone()).or_insert(settled);
                if matches!(reported, Measurement::Unavailable(_)) {
                    *reported = settled;
                }
            }
        }
        self.reported.clone()
    }

    /// Whether the table is due a fresh reading at `now`.
    fn is_due(&self, now: Instant) -> bool {
        match self.publication {
            CpuPublication::NeverPublished => true,
            CpuPublication::Published(taken) => {
                now.duration_since(taken) >= Duration::from_millis(CPU_REPORT_MILLIS)
            },
        }
    }
}

/// How much of a fresh sample a settled reading takes on.
///
/// Worked out from the scan interval rather than stated, so the window
/// stays the one [`CPU_SMOOTHING_SECONDS`] names however often the scan
/// runs.
fn smoothing_alpha() -> f32 {
    let interval = Duration::from_millis(PROCESS_POLL_MILLIS).as_secs_f32();
    1.0 - (-interval / CPU_SMOOTHING_SECONDS).exp()
}

/// What phase one learned: the parent links, the cargo processes, and the
/// compiler processes waiting to be attributed to one of them.
struct Census {
    /// Registration eligibility is resolved even when no process row can be built.
    registration_eligibility: HashMap<RunId, Result<(), RowAbsence>>,
    /// Invocation identities are established before attribution and row construction.
    identities:               HashMap<Pid, InvocationId>,
    /// Preserve cargo ancestry even after a command is deliberately excluded.
    capture_boundaries:       HashSet<Pid>,
    /// Only observed forwarding of the registered command permits walking through a wrapper.
    capture_wrappers:         HashSet<Pid>,
    /// Observed argv naming a different command forbids direct registration fields.
    capture_mismatches:       HashSet<Pid>,
    /// Native lifetime evidence belongs to this refresh, including unavailable reads.
    lifetimes:                HashMap<Pid, LifetimeEvidence>,
    /// Deliberate exclusion remains distinct from unavailable command metadata.
    eligibility:              HashMap<Pid, Result<(), RowAbsence>>,
    /// Every process's parent, for walking a compiler up to its cargo.
    parents:                  HashMap<Pid, Pid>,
    /// Processes whose own name is `cargo`.
    cargo:                    Vec<Pid>,
    /// Compiler processes paired with which driver they are.
    compilers:                Vec<(Pid, &'static str)>,
    /// Every process contributes a reading or an absence reason to its owner.
    cpu:                      HashMap<Pid, Measurement<f32>>,
}

impl Census {
    /// Classify every process the last refresh saw.
    fn take(system: &System, previous: &HashMap<Pid, CpuBaseline>) -> Self {
        let mut census = Self {
            identities:               HashMap::new(),
            capture_boundaries:       HashSet::new(),
            capture_wrappers:         HashSet::new(),
            capture_mismatches:       HashSet::new(),
            lifetimes:                HashMap::new(),
            eligibility:              HashMap::new(),
            registration_eligibility: HashMap::new(),
            parents:                  HashMap::new(),
            cargo:                    Vec::new(),
            compilers:                Vec::new(),
            cpu:                      HashMap::new(),
        };
        for (&pid, process) in system.processes() {
            if let Some(parent) = process.parent().or_else(|| kernel_parent(pid)) {
                census.parents.insert(pid, parent);
            }
            let baseline = CpuBaseline::from(process);
            census.lifetimes.insert(pid, baseline.lifetime.clone());
            census.cpu.insert(
                pid,
                Self::measure_cpu(pid, process.cpu_usage(), &baseline, previous),
            );
            let name = process.name();
            if is_cargo_name(name) {
                census.cargo.push(pid);
            } else if let Some(driver) = COMPILER_PROCESS_NAMES
                .iter()
                .find(|driver| name == OsStr::new(**driver))
            {
                census.compilers.push((pid, driver));
            }
        }
        census
    }

    /// Classify sysinfo's sample before a zero can lose its availability meaning.
    ///
    /// A positive previous accumulated counter is required before trusting
    /// sysinfo's rate: without it, the rate can retain an earlier value. A failed
    /// macOS task-info read resets the counter to zero but leaves the rate intact.
    /// Quantized zero counters cannot prove either a reading or a failed read.
    /// Native lifetime evidence separates same-second replacements before counters
    /// are compared. A regression within that lifetime remains unproven.
    /// An unchanged counter cannot prove a positive rate: macOS retains the old one.
    /// Nonzero counters on both samples still support a measured zero rate.
    fn measure_cpu(
        pid: Pid,
        cpu: f32,
        baseline: &CpuBaseline,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Measurement<f32> {
        let LifetimeEvidence::Available(lifetime) = &baseline.lifetime else {
            return Measurement::Unavailable(MeasurementAbsence::Unproven);
        };
        let Some(previous) = previous
            .get(&pid)
            .filter(|previous| matches!(&previous.lifetime, LifetimeEvidence::Available(prior) if prior == lifetime))
        else {
            return Measurement::Unavailable(MeasurementAbsence::FirstObservation);
        };
        if !cpu.is_finite() || cpu < 0.0 {
            return Measurement::Unavailable(MeasurementAbsence::ReadFailed);
        }
        if previous.accumulated == 0
            || baseline.accumulated < previous.accumulated
            || (baseline.accumulated == previous.accumulated && cpu > 0.0)
        {
            return Measurement::Unavailable(MeasurementAbsence::Unproven);
        }
        Measurement::Reading(cpu)
    }

    /// Whether phase one saw an sccache server.
    ///
    /// The server is a process named [`SCCACHE_BINARY`] like any other,
    /// so it lands in `compilers` whether or not a build is running --
    /// which is what lets this answer while the machine is idle, when a
    /// hit rate is exactly what a developer is looking at.
    fn sccache(&self) -> SccacheServer {
        if self
            .compilers
            .iter()
            .any(|&(_, driver)| driver == SCCACHE_BINARY)
        {
            SccacheServer::Running
        } else {
            SccacheServer::Stopped
        }
    }

    /// Count each cargo invocation's compiler descendants, reporting the
    /// highest-priority driver present in [`COMPILER_PROCESS_NAMES`].
    ///
    /// `sccache` outranks `rustc` because when a wrapper is in use every
    /// `rustc` is a child of one, and reporting both would double-count
    /// the same compile.
    fn attribute_compilers(&self) -> HashMap<Pid, CompilerObservation> {
        let mut tallies: HashMap<Pid, HashMap<&'static str, usize>> = HashMap::new();
        for &(pid, driver) in &self.compilers {
            if let Some(owner) = self.owning_cargo(pid) {
                *tallies.entry(owner).or_default().entry(driver).or_default() += 1;
            }
        }
        self.cargo
            .iter()
            .map(|&owner| {
                let compiler = tallies
                    .get(&owner)
                    .and_then(|tally| {
                        COMPILER_PROCESS_NAMES.iter().find_map(|driver| {
                            tally.get(driver).map(|&count| Compiler {
                                name: driver,
                                count,
                            })
                        })
                    })
                    .map_or(CompilerObservation::None, CompilerObservation::Running);
                (owner, compiler)
            })
            .collect()
    }

    /// Everything the census has to say about each cargo invocation.
    ///
    /// The two walks are one call because they run over the same parent
    /// chains and are both spent by the same pass over the groups.
    fn attribute(&self, smoothing: &mut CpuSmoothing, now: Instant) -> Attributed {
        smoothing.observed.clone_from(&self.lifetimes);
        let sampled: HashMap<_, _> = self
            .attribute_cpu()
            .into_iter()
            .filter_map(|(pid, cpu)| {
                self.identities
                    .get(&pid)
                    .map(|identity| (identity.clone(), cpu))
            })
            .collect();
        let identities: Vec<_> = self
            .cargo
            .iter()
            .filter_map(|pid| self.identities.get(pid).cloned())
            .collect();
        let reported = smoothing.settle(&sampled, &identities, now);
        Attributed {
            compilers: self.attribute_compilers(),
            cpu:       self
                .cargo
                .iter()
                .filter_map(|pid| {
                    self.identities
                        .get(pid)
                        .and_then(|identity| reported.get(identity))
                        .map(|cpu| (*pid, *cpu))
                })
                .collect(),
        }
    }

    /// Sum what each cargo invocation is using with what everything
    /// under it is using.
    ///
    /// A cargo process spends next to nothing on its own account: the
    /// work is in the `rustc`, `sccache`, build-script and linker
    /// processes it starts, and a row reporting only the cargo's own
    /// share would read as idle right through a build saturating the
    /// machine. Nesting is settled the way [`attribute_compilers`] settles
    /// it -- a process counts against the nearest cargo above it -- so a
    /// managed command's share lands on that command rather than on its
    /// manager, and [`Self::group`] adds the descendants back in for the
    /// lead.
    ///
    /// [`attribute_compilers`]: Self::attribute_compilers
    fn attribute_cpu(&self) -> HashMap<Pid, Measurement<f32>> {
        let mut tallies: HashMap<Pid, Measurement<f32>> = self
            .cargo
            .iter()
            .map(|&pid| {
                (
                    pid,
                    self.cpu
                        .get(&pid)
                        .copied()
                        .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven)),
                )
            })
            .collect();
        for (&pid, &cpu) in &self.cpu {
            if !self.cargo.contains(&pid)
                && let Some(owner) = self.owning_cargo(pid)
                && let Some(total) = tallies.get_mut(&owner)
            {
                *total = *total + cpu;
            }
        }
        tallies
    }

    /// Drop every cargo that is only a shim in front of another cargo.
    ///
    /// A shim that wraps cargo is itself named `cargo` -- that is the
    /// whole point of a shim -- so one command can present as two
    /// processes. What separates that from a command *managing* other
    /// cargo commands is whether the child is the same command: a shim
    /// hands its line straight on, so the subcommand and the working
    /// directory both match, while `cargo mend` running a `cargo nextest`
    /// suite matches neither. The shim goes and the process doing the
    /// work stays; the manager stays and keeps its children.
    ///
    /// Looping rather than one pass because a shim can stand in front of
    /// a shim, and dropping the outer one is what reveals the next.
    fn collapse_shims(&mut self, system: &System) {
        loop {
            let children = self.cargo_children();
            let shims: Vec<Pid> = self
                .cargo
                .iter()
                .copied()
                .filter(|&pid| Self::is_shim(system, &children, pid))
                .collect();
            if shims.is_empty() {
                return;
            }
            self.cargo.retain(|pid| !shims.contains(pid));
        }
    }

    /// Whether `pid` is a shim in front of the one cargo beneath it.
    fn is_shim(system: &System, children: &HashMap<Pid, Vec<Pid>>, pid: Pid) -> bool {
        let Some(kids) = children.get(&pid) else {
            return false;
        };
        let [child] = kids[..] else {
            return false;
        };
        let (Some(outer), Some(inner)) = (system.process(pid), system.process(child)) else {
            return false;
        };
        observed_shim_match(
            outer.cmd(),
            outer.cwd().into(),
            inner.cmd(),
            inner.cwd().into(),
        )
    }

    /// Each cargo's direct cargo children -- the tree the groups are cut
    /// from, with a process that came out as its own ancestor dropped so
    /// a walk of it cannot loop.
    fn cargo_children(&self) -> HashMap<Pid, Vec<Pid>> {
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for &pid in &self.cargo {
            if let Some(owner) = self.owning_cargo(pid)
                && owner != pid
            {
                children.entry(owner).or_default().push(pid);
            }
        }
        children
    }

    /// Every cargo running under `root`, at any depth.
    fn descendants(children: &HashMap<Pid, Vec<Pid>>, root: Pid) -> Vec<Pid> {
        let mut seen: HashSet<Pid> = HashSet::from([root]);
        let mut queue = vec![root];
        let mut out = Vec::new();
        while let Some(pid) = queue.pop() {
            let Some(kids) = children.get(&pid) else {
                continue;
            };
            for &kid in kids {
                if seen.insert(kid) {
                    out.push(kid);
                    queue.push(kid);
                }
            }
        }
        out
    }

    /// Walk `pid` up its parent chain to the nearest ancestor the cell
    /// actually draws, which is what the `parent` column names.
    ///
    /// Two things are drawn: the cargo invocations, which are the rows
    /// of the table, and the chain block standing over it. The first
    /// ancestor that is one of them is the answer, and it is the only
    /// answer worth writing down -- a pid the screen shows nowhere is a
    /// number the eye cannot pair with anything.
    ///
    /// Which is why the immediate parent is never it. A captured run's
    /// parent is the pty the shim opened, whose parent is the shim,
    /// whose parent is the shell; none of the three is drawn, and the
    /// shell is the first thing above them that is. For an invocation
    /// another cargo started, the same walk stops one step sooner, at
    /// that cargo's own row.
    ///
    /// Bounded by [`PARENT_WALK_LIMIT`], like every other walk here, so
    /// a reparented chain that loops cannot spin.
    fn drawn_parent(&self, pid: Pid, ancestry: &[Ancestor]) -> VisibleParent {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                break;
            };
            if self.cargo.contains(&parent) {
                if let Some(identity) = self.identities.get(&parent) {
                    return VisibleParent::Invocation {
                        id:  identity.clone(),
                        pid: parent.as_u32(),
                    };
                }
            } else if ancestry
                .iter()
                .any(|ancestor| ancestor.pid == parent.as_u32())
            {
                return VisibleParent::Ancestor(parent.as_u32());
            }
            current = parent;
        }
        VisibleParent::None
    }

    /// Walk `pid` up its parent chain to the cargo invocation that owns
    /// it, bounded by [`PARENT_WALK_LIMIT`] so a reparented process whose
    /// chain loops back on itself cannot spin here.
    fn owning_cargo(&self, pid: Pid) -> Option<Pid> {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let parent = *self.parents.get(&current)?;
            if self.cargo.contains(&parent) {
                return Some(parent);
            }
            current = parent;
        }
        None
    }

    /// Every process a scan reads the costly fields for: the cargo
    /// invocations, and everything standing above one of them.
    ///
    /// A superset of what any cell ends up listing. Which ancestors are
    /// the invocation's own plumbing is settled on argv, which is what
    /// this pass is about to read, so the filtering waits for
    /// [`Self::ancestry`] and the extra reads are a handful of
    /// processes.
    fn detailed(&self) -> Vec<Pid> {
        let mut pids = self.cargo.clone();
        for &pid in &self.cargo {
            for ancestor in self.ancestor_pids(pid) {
                if !pids.contains(&ancestor) {
                    pids.push(ancestor);
                }
            }
        }
        pids
    }

    /// Every process standing above `pid`, outermost first.
    ///
    /// The walk stops short of the init process the whole tree roots
    /// at: everything on the machine descends from it, so a row naming
    /// it tells one command from no other. [`PARENT_WALK_LIMIT`] bounds
    /// it the way it bounds the compiler walk, and a pid already on the
    /// chain ends it outright, so a reparented cycle cannot spin here.
    fn ancestor_pids(&self, pid: Pid) -> Vec<Pid> {
        let mut chain = Vec::new();
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                break;
            };
            if parent.as_u32() <= ROOT_PROCESS_PID || chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        chain
    }

    /// What stands above `pid`, as the command's own cell lists it:
    /// outermost first, each entry a pid and what that process is.
    ///
    /// One kind of process is dropped outright: the wrappers belonging
    /// to the invocation itself. A captured run reaches cargo through a
    /// shim and a pty, both running this same cargo command line, and
    /// listing them would answer "what started this" with the machinery
    /// this tool installed to watch it.
    ///
    /// The shells and login processes that merely passed the command
    /// through are marked rather than dropped -- see
    /// [`Ancestor::passes_through`].
    ///
    /// A cargo ancestor that is *not* plumbing cannot reach here:
    /// [`Self::groups`] leads a group with an invocation that has no
    /// cargo above it.
    fn ancestry(&self, system: &System, home: ScannerHome<'_>, pid: Pid) -> Vec<Ancestor> {
        self.ancestor_pids(pid)
            .into_iter()
            .filter_map(|pid| system.process(pid))
            .filter(|process| !names_cargo(process.cmd()))
            .map(|process| Ancestor {
                pid:            process.pid().as_u32(),
                command:        describe(process, home),
                passes_through: is_transparent(process.name()),
            })
            .collect()
    }

    /// What the capture behind `pid` reports, if one is behind it.
    ///
    /// The walk goes upward because the pid a capture is filed under is
    /// the shim's, and the shim is an ancestor of the cargo it started
    /// rather than the cargo itself -- two levels up when the run went
    /// through a pty, one when it did not. The same bound the compiler
    /// walk uses stops a reparented cycle here.
    fn captured_run(&self, capture: &Capture, pid: Pid) -> NearestRegistration {
        let mut walking = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            match capture.select(walking.as_u32()) {
                CaptureSelection::Selected(key) => return NearestRegistration::Registered(key),
                CaptureSelection::Ambiguous(candidates) => {
                    return NearestRegistration::Ambiguous(candidates);
                },
                CaptureSelection::Unregistered => {},
            }
            let Some(parent) = self.parents.get(&walking) else {
                return NearestRegistration::Unregistered;
            };
            walking = *parent;
        }
        NearestRegistration::Unregistered
    }

    /// Report every retained selection, including a shim absent from the process table.
    fn associate_status(&self, capture: &mut Capture, groups: &[CargoGroup]) {
        let mut associations = Vec::new();
        for row in groups
            .iter()
            .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        {
            associations.push((row.pid, self.captured_run(capture, Pid::from_u32(row.pid))));
        }
        for pid in capture.registered_pids() {
            if !associations.iter().any(|(_, nearest)| match nearest {
                NearestRegistration::Registered(key) => key.pid == pid,
                NearestRegistration::Ambiguous(keys) => keys.iter().any(|key| key.pid == pid),
                NearestRegistration::Unregistered => false,
            }) {
                associations.push((pid, self.captured_run(capture, Pid::from_u32(pid))));
            }
        }
        for (pid, nearest) in associations {
            let (root, selection) = match nearest {
                NearestRegistration::Unregistered => continue,
                NearestRegistration::Ambiguous(candidates) => {
                    let Some(key) = candidates.first() else {
                        continue;
                    };
                    (key.root, AssociationSelection::Ambiguous { candidates })
                },
                NearestRegistration::Registered(key) => {
                    let proof = if capture
                        .confirmed()
                        .iter()
                        .any(|confirmed| confirmed.key == key)
                    {
                        SelectedProof::Confirmed
                    } else {
                        SelectedProof::Unconfirmed
                    };
                    let unused = capture
                        .confirmed()
                        .iter()
                        .filter(|confirmed| confirmed.key.pid == key.pid && confirmed.key != key)
                        .filter_map(|confirmed| {
                            let status = capture.root_status.get(confirmed.key.root.0)?;
                            let root = status.root.path.clone();
                            Some(UnusedCapture {
                                key: confirmed.key.clone(),
                                root,
                                reason: match proof {
                                    SelectedProof::Confirmed => UnusedCaptureReason::RootPrecedence,
                                    SelectedProof::Unconfirmed => {
                                        UnusedCaptureReason::SelectedUnconfirmed
                                    },
                                },
                            })
                        })
                        .collect();
                    (
                        key.root,
                        AssociationSelection::Selected { key, proof, unused },
                    )
                },
            };
            if let Some(status) = capture.root_status.get_mut(root.0) {
                status
                    .associations
                    .push(CaptureAssociation { pid, selection });
            }
        }
    }

    /// A registration can become process-readable after exec without a new lifetime.
    fn include_registered_processes(&mut self, system: &System, capture: &Capture) {
        for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
            if system
                .process(pid)
                .is_some_and(|process| names_cargo(process.cmd()))
                && !self.cargo.contains(&pid)
            {
                self.cargo.push(pid);
            }
        }
    }

    /// Record wrappers forwarding the registered command, including the shim's JSON rewrite.
    /// An exec-replaced application no longer carries that command and is a boundary.
    fn identify_capture_wrappers(&mut self, system: &System, capture: &Capture) {
        self.capture_mismatches = system
            .processes()
            .iter()
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        !process.cmd().is_empty()
                            && !forwards_capture_command(
                                process.cmd(),
                                confirmed.registration.record(),
                            )
                    })
                    .map(|_| pid)
            })
            .collect();
        self.capture_wrappers = system
            .processes()
            .iter()
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        forwards_capture_command(process.cmd(), confirmed.registration.record())
                    })
                    .map(|_| pid)
            })
            .collect();
    }

    /// Direct ownership crosses only wrappers observed forwarding this registration.
    /// Any other intervening process establishes enclosing membership.
    fn direct_capture(&self, capture: &Capture, pid: Pid) -> DirectAssociation {
        if self.capture_mismatches.contains(&pid) {
            return DirectAssociation::None;
        }
        let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
            return DirectAssociation::None;
        };
        let Some(confirmed) = capture
            .confirmed()
            .iter()
            .find(|confirmed| confirmed.key == key)
        else {
            return DirectAssociation::None;
        };
        let mut walking = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            if walking.as_u32() == key.pid {
                return DirectAssociation::Direct(Box::new(DirectCapture::from(confirmed)));
            }
            let Some(&parent) = self.parents.get(&walking) else {
                break;
            };
            if self.capture_mismatches.contains(&parent)
                || (parent.as_u32() != key.pid
                    && (!self.capture_wrappers.contains(&parent)
                        || self.capture_boundaries.contains(&parent)
                        || self.cargo.contains(&parent)))
            {
                break;
            }
            walking = parent;
        }
        DirectAssociation::None
    }

    /// Prefer the process doing the work when both the shim and cargo describe one run.
    fn identify_captures(&mut self, capture: &Capture) {
        self.capture_boundaries.extend(self.cargo.iter().copied());
        for &pid in &self.cargo {
            if let DirectAssociation::Direct(direct) = self.direct_capture(capture, pid) {
                self.identities.insert(pid, direct.invocation_id());
            }
        }
        let represented: HashSet<_> = self
            .cargo
            .iter()
            .copied()
            .filter(|pid| {
                self.cargo.iter().any(|child| {
                    child != pid
                        && self
                            .identities
                            .get(child)
                            .is_some_and(|identity| self.identities.get(pid) == Some(identity))
                        && self.ancestor_pids(*child).contains(pid)
                })
            })
            .collect();
        self.cargo.retain(|pid| !represented.contains(pid));
    }

    /// Exclusions prune only row ownership; parents and verified capture liveness remain intact.
    fn select_rows(&mut self, system: &System, capture: &Capture, excluded: &[String]) {
        self.registration_eligibility = capture
            .confirmed()
            .iter()
            .map(|confirmed| {
                let argv = std::iter::once(OsString::from(CARGO_DISPLAY_NAME))
                    .chain(confirmed.registration.record().arguments().iter().cloned())
                    .collect::<Vec<_>>();
                (
                    RunId::verified(&confirmed.key, &confirmed.registration),
                    select_cargo(&argv, excluded).map(|_| ()),
                )
            })
            .collect();
        for &pid in &self.cargo {
            let process = system
                .process(pid)
                .map_or(Err(RowAbsence::Unavailable), |process| {
                    select_cargo(process.cmd(), excluded).map(|_| ())
                });
            let eligibility = match self.direct_capture(capture, pid) {
                DirectAssociation::Direct(direct) => {
                    match self.registration_eligibility.get(&direct.run_id) {
                        Some(Err(RowAbsence::Excluded)) => Err(RowAbsence::Excluded),
                        Some(Ok(())) if process == Err(RowAbsence::Unavailable) => Ok(()),
                        Some(Ok(()) | Err(RowAbsence::Unavailable)) | None => process,
                    }
                },
                DirectAssociation::None => process,
            };
            self.eligibility.insert(pid, eligibility);
        }
        self.cargo
            .retain(|pid| matches!(self.eligibility.get(pid), Some(Ok(()))));
    }

    /// Only a verified direct association may supply directory annotation.
    fn annotate_capture(&self, row: &mut CargoProcess, capture: &Capture, home: ScannerHome<'_>) {
        let pid = Pid::from_u32(row.pid);
        row.provenance = RowProvenance::Uncaptured;
        row.capture_membership = CaptureMembership::Outside;
        if let Some(identity) = self.identities.get(&pid) {
            row.invocation_id.clone_from(identity);
        }
        let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
            row.state = CaptureLookup::Unregistered;
            return;
        };
        row.state = capture.read(&key);
        match self.direct_capture(capture, pid) {
            DirectAssociation::Direct(direct) => {
                row.invocation_id = direct.invocation_id();
                row.provenance = RowProvenance::direct(capture, &key);
                let record = direct.registration().record();
                if matches!(
                    row.directory_identity,
                    WorkingDirectoryIdentity::Absolute(_)
                ) && record.directory_identity() == row.directory_identity
                {
                    row.path = registration_directory(record, home);
                }
            },
            DirectAssociation::None => {
                if let Some(confirmed) = capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                {
                    row.provenance = RowProvenance::enclosing(capture, &key);
                    row.capture_membership = CaptureMembership::Enclosing(RunId::verified(
                        &key,
                        &confirmed.registration,
                    ));
                }
            },
        }
    }

    /// Every group the surviving cargo set forms, newest lead first.
    ///
    /// A lead ties with another when both started inside the same second
    /// -- the start time is whole seconds, since sysinfo reads the
    /// kernel's `pbi_start_tvsec` and drops the microseconds beside it.
    /// Pid breaks the tie in the same direction: macOS hands them out in
    /// order, so within one second the higher pid is the later start.
    fn groups(
        &self,
        system: &System,
        attributed: &Attributed,
        home: ScannerHome<'_>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        let children = self.cargo_children();
        let mut candidates = self.cargo.clone();
        candidates.sort_by_key(|pid| self.ancestor_pids(*pid).len());
        let mut rows: Vec<CargoProcess> = Vec::new();
        for pid in candidates {
            if !rows.iter().any(|row| row.pid == pid.as_u32())
                && let Ok(group) = self.group(system, attributed, home, &children, capture, pid)
            {
                rows.push(group.lead);
                rows.extend(group.rest);
            }
        }
        let process_rows: HashSet<_> = rows.iter().map(|row| row.invocation_id.clone()).collect();
        self.add_registration_rows(&mut rows, capture, home, SystemTime::now());
        let mut groups = self.assemble_groups(system, home, rows);
        for group in &mut groups {
            if process_rows.contains(&group.lead.invocation_id) {
                let members: Vec<_> = std::iter::once(&group.lead)
                    .chain(&group.rest)
                    .map(|row| Pid::from_u32(row.pid))
                    .collect();
                group.lead.cpu =
                    aggregate_cpu(&attributed.cpu, members.iter().copied()).map(cpu_label);
                group.lead.compiler =
                    aggregate_compilers(&attributed.compilers, members.into_iter());
            }
            for row in &mut group.rest {
                if process_rows.contains(&row.invocation_id) {
                    let pid = Pid::from_u32(row.pid);
                    row.cpu = aggregate_cpu(&attributed.cpu, std::iter::once(pid)).map(cpu_label);
                    row.compiler = attributed
                        .compilers
                        .get(&pid)
                        .cloned()
                        .unwrap_or(CompilerObservation::Unknown);
                }
            }
        }
        groups
    }

    /// Enclosing membership never suppresses the invocation represented by a proof.
    fn add_registration_rows(
        &self,
        rows: &mut Vec<CargoProcess>,
        capture: &Capture,
        home: ScannerHome<'_>,
        now: SystemTime,
    ) {
        for pid in capture.registered_pids() {
            let DirectAssociation::Direct(direct) = capture.row_source(pid) else {
                continue;
            };
            if !matches!(
                self.registration_eligibility.get(&direct.run_id),
                Some(Ok(()))
            ) {
                continue;
            }
            // Root selection has already rejected unconfirmed and competing proofs.
            // Only direct representation of this invocation suppresses its source row.
            let represented = rows
                .iter()
                .any(|row| row.invocation_id == direct.invocation_id());
            if !represented && let Ok(row) = registration_row(&direct, capture, home, now) {
                rows.push(row);
            }
        }
    }

    /// Build one ancestry tree after both sources have supplied eligible invocations.
    fn assemble_groups(
        &self,
        system: &System,
        home: ScannerHome<'_>,
        mut rows: Vec<CargoProcess>,
    ) -> Vec<CargoGroup> {
        rows.sort_by(newest_first);
        let mut seen = HashSet::new();
        rows.retain(|row| seen.insert(row.invocation_id.clone()));
        let mut visible = HashMap::new();
        for row in &rows {
            let identity = (row.invocation_id.clone(), row.pid);
            visible.insert(Pid::from_u32(row.pid), identity.clone());
            if let InvocationId::Captured(run) = &row.invocation_id {
                visible.insert(Pid::from_u32(run.shim_pid), identity);
            }
        }
        for row in &mut rows {
            row.parent = self.row_parent(row, &visible);
        }
        let parents: HashMap<_, _> = rows
            .iter()
            .filter_map(|row| match &row.parent {
                VisibleParent::Invocation { id, .. } => {
                    Some((row.invocation_id.clone(), id.clone()))
                },
                VisibleParent::Ancestor(_) | VisibleParent::None => None,
            })
            .collect();
        let mut families: HashMap<InvocationId, Vec<CargoProcess>> = HashMap::new();
        let mut assembled_descendants = HashMap::new();
        for row in rows {
            let mut root = row.invocation_id.clone();
            let mut visited = HashSet::new();
            for _ in 0..PARENT_WALK_LIMIT {
                if !visited.insert(root.clone()) {
                    break;
                }
                let Some(parent) = parents.get(&root) else {
                    break;
                };
                if !visited.contains(parent) {
                    *assembled_descendants.entry(parent.clone()).or_insert(0) += 1;
                }
                root.clone_from(parent);
            }
            families.entry(root).or_default().push(row);
        }
        let mut groups = Vec::new();
        for (id, mut rows) in families {
            for row in &mut rows {
                let managed = assembled_descendants
                    .get(&row.invocation_id)
                    .copied()
                    .unwrap_or_default();
                // Confirmed child rows count regardless of source. Without children,
                // a registration alone still cannot establish a measured zero.
                if managed > 0 || matches!(row.managed, Measurement::Reading(_)) {
                    row.managed = Measurement::Reading(managed);
                }
            }
            let Some(index) = rows.iter().position(|row| row.invocation_id == id) else {
                continue;
            };
            let mut lead = rows.remove(index);
            let ancestry = self.ancestry(system, home, Pid::from_u32(lead.pid));
            lead.parent = ancestry.last().map_or(VisibleParent::None, |ancestor| {
                VisibleParent::Ancestor(ancestor.pid)
            });
            for row in &mut rows {
                row.nested = matches!(&row.parent, VisibleParent::Invocation { id: parent, .. } if parent != &id);
            }
            rows.sort_by(newest_first);
            groups.push(CargoGroup {
                lead,
                rest: rows,
                ancestry,
            });
        }
        groups.sort_by(|left, right| newest_first(&left.lead, &right.lead));
        groups
    }

    /// Match displayed invocation identities, including a registration parent's shim alias.
    fn row_parent(
        &self,
        row: &CargoProcess,
        visible: &HashMap<Pid, (InvocationId, u32)>,
    ) -> VisibleParent {
        let mut walking = Pid::from_u32(row.pid);
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(parent) = self.parents.get(&walking) else {
                break;
            };
            if let Some((id, pid)) = visible.get(parent)
                && id != &row.invocation_id
            {
                return VisibleParent::Invocation {
                    id:  id.clone(),
                    pid: *pid,
                };
            }
            walking = *parent;
        }
        VisibleParent::None
    }

    /// Build the group led by `root`.
    fn group(
        &self,
        system: &System,
        attributed: &Attributed,
        home: ScannerHome<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        capture: &Capture,
        root: Pid,
    ) -> Result<CargoGroup, GroupAbsence> {
        if matches!(self.eligibility.get(&root), Some(Err(RowAbsence::Excluded))) {
            return Err(GroupAbsence::Excluded);
        }
        let managed = Self::descendants(children, root);
        let whole_group = std::iter::once(root).chain(managed.clone());
        let mut lead = row(
            system.process(root).ok_or(GroupAbsence::NoProcess)?,
            self.identities
                .get(&root)
                .ok_or(GroupAbsence::NoIdentity)?
                .clone(),
            attributed
                .compilers
                .get(&root)
                .cloned()
                .unwrap_or(CompilerObservation::Unknown),
            Measurement::Reading(managed.len()),
            home,
            aggregate_cpu(&attributed.cpu, whole_group.clone()),
            &self.direct_capture(capture, root),
        )?;
        // The lead reports the whole group's compilers: what a developer
        // wants off the summary row is how much work the command they
        // typed is doing, and for a manager none of that work is running
        // under the manager's own pid. Its CPU share is the same story
        // told in cores rather than in processes.
        lead.compiler = aggregate_compilers(&attributed.compilers, whole_group);
        self.annotate_capture(&mut lead, capture, home);
        let ancestry = self.ancestry(system, home, root);
        lead.parent = self.drawn_parent(root, &ancestry);

        let mut dated: Vec<(u64, CargoProcess)> = managed
            .into_iter()
            .filter_map(|pid| {
                let process = system.process(pid)?;
                let under = Self::descendants(children, pid).len();
                let mut managed_row = row(
                    process,
                    self.identities.get(&pid)?.clone(),
                    attributed
                        .compilers
                        .get(&pid)
                        .cloned()
                        .unwrap_or(CompilerObservation::Unknown),
                    Measurement::Reading(under),
                    home,
                    aggregate_cpu(&attributed.cpu, std::iter::once(pid)),
                    &self.direct_capture(capture, pid),
                )
                .ok()?;
                // The same read the lead gets. An invocation the lead
                // is driving is captured in its own right where it came
                // through the shim, and where it went round the shim --
                // a cargo the enclosing run started, which the shim
                // declines to capture twice -- the enclosing capture is
                // still its own: the lock it prints about is the one
                // this row is waiting on, mirrored into the log of the
                // run it is inside.
                self.annotate_capture(&mut managed_row, capture, home);
                managed_row.parent = self.drawn_parent(pid, &ancestry);
                managed_row.nested = !children
                    .get(&root)
                    .is_some_and(|started_by_the_lead| started_by_the_lead.contains(&pid));
                Some((process.start_time(), managed_row))
            })
            .collect();
        dated.sort_by(|left, right| right.0.cmp(&left.0).then(right.1.pid.cmp(&left.1.pid)));
        Ok(CargoGroup {
            lead,
            rest: dated.into_iter().map(|(_, process)| process).collect(),
            ancestry,
        })
    }
}

/// Unknown timestamps remain last; invocation identity breaks ties without hash order.
fn newest_first(left: &CargoProcess, right: &CargoProcess) -> std::cmp::Ordering {
    match (left.started, right.started) {
        (RunStart::Known(left), RunStart::Known(right)) => right.cmp(&left),
        (RunStart::Known(_), RunStart::Unavailable) => std::cmp::Ordering::Less,
        (RunStart::Unavailable, RunStart::Known(_)) => std::cmp::Ordering::Greater,
        (RunStart::Unavailable, RunStart::Unavailable) => std::cmp::Ordering::Equal,
    }
    .then_with(|| right.invocation_id.cmp(&left.invocation_id))
}

/// Recognize both argv forwarding and the shim's single-quoted POSIX command string.
/// PTY helpers and their shells carry this command; an application launched by cargo does not.
fn forwards_capture_command(argv: &[OsString], record: &RegistrationCandidate) -> bool {
    if let Ok(arguments) = cargo_split(argv)
        && (argv[arguments.start..] == *record.arguments()
            || forwards_json_capture_arguments(&argv[arguments.start..], record.arguments()))
    {
        return true;
    }
    let mut expected_arguments = Vec::new();
    for argument in record.arguments() {
        expected_arguments.push(b'\'');
        for byte in argument.as_bytes() {
            if *byte == b'\'' {
                expected_arguments.extend_from_slice(b"'\\''");
            } else {
                expected_arguments.push(*byte);
            }
        }
        expected_arguments.extend_from_slice(b"' ");
    }
    argv.iter().any(|argument| {
        argument
            .as_bytes()
            .strip_suffix(expected_arguments.as_slice())
            .and_then(|binary| binary.strip_prefix(b"'"))
            .and_then(|binary| binary.strip_suffix(b"' "))
            .is_some_and(quoted_cargo_binary)
    })
}

/// Match only the shim's complete removal of quiet flags before `--` in JSON mode.
/// All remaining words, including non-UTF-8 bytes and arguments after `--`, stay exact.
fn forwards_json_capture_arguments(argv: &[OsString], registered: &[OsString]) -> bool {
    let separator = registered
        .iter()
        .position(|argument| argument == ARGUMENT_SEPARATOR)
        .unwrap_or(registered.len());
    let (cargo_arguments, passthrough) = registered.split_at(separator);
    if !cargo_arguments.iter().any(|argument| {
        argument
            .as_bytes()
            .starts_with(CARGO_MESSAGE_FORMAT_JSON_PREFIX.as_bytes())
    }) && !cargo_arguments.windows(2).any(|pair| {
        pair[0] == CARGO_MESSAGE_FORMAT_FLAG
            && pair[1]
                .as_bytes()
                .starts_with(CARGO_JSON_FORMAT_PREFIX.as_bytes())
    }) {
        return false;
    }
    cargo_arguments
        .iter()
        .filter(|argument| {
            !CARGO_QUIET_FLAGS
                .iter()
                .any(|quiet| argument.as_os_str() == OsStr::new(quiet))
        })
        .chain(passthrough)
        .eq(argv)
}

/// Inside one quoted executable word, an apostrophe must use the shim's escape sequence.
fn quoted_cargo_binary(binary: &[u8]) -> bool {
    let mut remaining = binary;
    while let Some(index) = remaining.iter().position(|byte| *byte == b'\'') {
        let Some(rest) = remaining[index..].strip_prefix(b"'\\''") else {
            return false;
        };
        remaining = rest;
    }
    Path::new(OsStr::from_bytes(binary))
        .file_name()
        .is_some_and(|name| {
            CARGO_PROCESS_NAMES
                .iter()
                .any(|cargo| name == OsStr::new(cargo))
        })
}

/// Missing command or cwd fields never count as observed equality between wrappers.
fn observed_shim_match(
    outer: &[OsString],
    outer_cwd: WorkingDirectoryObservation<'_>,
    inner: &[OsString],
    inner_cwd: WorkingDirectoryObservation<'_>,
) -> bool {
    matches!((subcommand(outer), subcommand(inner), outer_cwd, inner_cwd),
        (Some(outer), Some(inner), WorkingDirectoryObservation::Observed(outer_cwd), WorkingDirectoryObservation::Observed(inner_cwd))
        if outer == inner && outer_cwd == inner_cwd)
}

/// One compiler tally across a whole group: the highest-priority driver
/// any member is running, totalled over all of them.
fn aggregate_compilers(
    counts: &HashMap<Pid, CompilerObservation>,
    members: impl Iterator<Item = Pid>,
) -> CompilerObservation {
    let mut running = Vec::new();
    for pid in members {
        match counts.get(&pid).unwrap_or(&CompilerObservation::Unknown) {
            CompilerObservation::Unknown => return CompilerObservation::Unknown,
            CompilerObservation::None => {},
            CompilerObservation::Running(compiler) => running.push(compiler),
        }
    }
    let Some(name) = COMPILER_PROCESS_NAMES
        .iter()
        .find(|driver| running.iter().any(|compiler| compiler.name == **driver))
    else {
        return CompilerObservation::None;
    };
    CompilerObservation::Running(Compiler {
        name,
        count: running
            .iter()
            .filter(|compiler| compiler.name == *name)
            .map(|compiler| compiler.count)
            .sum(),
    })
}

/// One CPU share across a whole group; an unavailable or missing member
/// prevents publishing a partial total as a measured value.
pub(crate) fn aggregate_cpu(
    shares: &HashMap<Pid, Measurement<f32>>,
    members: impl Iterator<Item = Pid>,
) -> Measurement<f32> {
    members.fold(Measurement::Reading(0.0), |total, pid| {
        total
            + shares
                .get(&pid)
                .copied()
                .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven))
    })
}

/// Format one cargo process into its table row.
fn row(
    process: &Process,
    invocation_id: InvocationId,
    compiler: CompilerObservation,
    managed: Measurement<usize>,
    home: ScannerHome<'_>,
    cpu: Measurement<f32>,
    direct: &DirectAssociation,
) -> Result<CargoProcess, RowAbsence> {
    let (directory_identity, path, command) =
        row_fields(process.cwd().into(), process.cmd(), direct, home)?;
    let (started, start, duration) = match direct {
        DirectAssociation::Direct(direct) => registration_timing(direct, SystemTime::now()),
        DirectAssociation::None => (
            RunStart::Known(process.start_time()),
            start_label(process.start_time()),
            duration_label(process.run_time()),
        ),
    };
    Ok(CargoProcess {
        invocation_id,
        capture_membership: CaptureMembership::Outside,
        provenance: RowProvenance::Uncaptured,
        path,
        directory_identity,
        pid: process.pid().as_u32(),
        parent: VisibleParent::None,
        start,
        started,
        duration,
        cpu: cpu.map(cpu_label),
        compiler,
        state: CaptureLookup::Unregistered,
        managed,
        nested: false,
        command,
    })
}

/// Resolve each unavailable field independently while observations retain their meaning.
fn row_fields(
    directory: WorkingDirectoryObservation<'_>,
    argv: &[OsString],
    direct: &DirectAssociation,
    home: ScannerHome<'_>,
) -> Result<(WorkingDirectoryIdentity, String, CommandText), RowAbsence> {
    let command = match (command_text(argv, home), direct) {
        (Err(RowAbsence::Unavailable), DirectAssociation::Direct(direct)) => {
            command_text(&registration_argv(direct.registration().record()), home)?
        },
        (Ok(_), DirectAssociation::Direct(direct))
            if cargo_split(argv).is_ok_and(|arguments| {
                forwards_json_capture_arguments(
                    &argv[arguments.start..],
                    direct.registration().record().arguments(),
                )
            }) =>
        {
            // The shim's quiet rewrite changes execution, not the user's command heading.
            command_text(&registration_argv(direct.registration().record()), home)?
        },
        (command, _) => command?,
    };
    let (identity, display) = match (directory, direct) {
        (WorkingDirectoryObservation::Unavailable, DirectAssociation::Direct(direct)) => {
            let record = direct.registration().record();
            (
                record.directory_identity(),
                registration_directory(record, home),
            )
        },
        (WorkingDirectoryObservation::Observed(path), DirectAssociation::Direct(direct))
            if direct.registration().record().directory() == path =>
        {
            (
                path.into(),
                registration_directory(direct.registration().record(), home),
            )
        },
        (WorkingDirectoryObservation::Observed(path), _) => {
            (path.into(), home_relative(path, home))
        },
        (WorkingDirectoryObservation::Unavailable, _) => (
            WorkingDirectoryIdentity::Unavailable,
            UNRESOLVED_PATH.to_owned(),
        ),
    };
    Ok((identity, display, command))
}

/// Preserve the writer's argument boundaries when applying the same command policy.
fn registration_argv(record: &RegistrationCandidate) -> Vec<OsString> {
    std::iter::once(OsString::from(CARGO_DISPLAY_NAME))
        .chain(record.arguments().iter().cloned())
        .collect()
}

/// A captured invocation starts when its registration is published, independently of workers.
fn registration_timing(direct: &DirectCapture, now: SystemTime) -> (RunStart, String, String) {
    let started = direct
        .modified
        .as_ref()
        .map_or(RunStart::Unavailable, |modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .map_or(RunStart::Unavailable, |elapsed| {
                    RunStart::Known(elapsed.as_secs())
                })
        });
    let (start, duration) = match started {
        RunStart::Known(seconds) => (
            start_label(seconds),
            duration_label(
                now.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_sub(seconds),
            ),
        ),
        RunStart::Unavailable => (UNRESOLVED_TIME.to_owned(), UNRESOLVED_TIME.to_owned()),
    };
    (started, start, duration)
}

/// Verified metadata can supply a complete row without a measurable process.
fn registration_row(
    direct: &DirectCapture,
    capture: &Capture,
    home: ScannerHome<'_>,
    now: SystemTime,
) -> Result<CargoProcess, RowAbsence> {
    let record = direct.registration().record();
    let (started, start, duration) = registration_timing(direct, now);
    Ok(CargoProcess {
        invocation_id: direct.invocation_id(),
        capture_membership: CaptureMembership::Outside,
        provenance: RowProvenance::direct(capture, &direct.key),
        path: registration_directory(record, home),
        directory_identity: record.directory_identity(),
        pid: direct.key.pid,
        parent: VisibleParent::None,
        start,
        started,
        duration,
        cpu: Measurement::Unavailable(MeasurementAbsence::Unproven),
        compiler: CompilerObservation::Unknown,
        state: capture.read(&direct.key),
        managed: Measurement::Unavailable(MeasurementAbsence::Unproven),
        nested: false,
        command: command_text(&registration_argv(record), home)?,
    })
}

/// A CPU share as the whole-number percent the table carries.
///
/// Rounded rather than truncated, and never below nought: a quarter of a
/// second of sampling has no meaningful resolution under one percent,
/// and a column of decimals costs width the command line wants.
pub(crate) fn cpu_label(cpu: f32) -> String {
    let percent = cpu.max(0.0);
    format!("{percent:.0}%")
}

/// Whether a process only passed a command through rather than being
/// what launched it, per [`TRANSPARENT_PROCESS_NAMES`].
fn is_transparent(name: &OsStr) -> bool {
    name.to_str()
        .is_some_and(|name| TRANSPARENT_PROCESS_NAMES.contains(&name))
}

/// What an ancestor row calls a process: the command line it is
/// running, the executable behind it when its arguments cannot be read,
/// or the name the kernel reports when neither can.
///
/// macOS lets a process read the argument area of processes its own
/// user owns and of nothing else, so a root-owned ancestor -- a login
/// process, a launch agent -- arrives with an empty argv and falls
/// through to one of the other two.
fn describe(process: &Process, home: ScannerHome<'_>) -> String {
    let line: Vec<String> = process
        .cmd()
        .iter()
        .map(|word| home_relative(Path::new(word), home))
        .collect();
    if !line.is_empty() {
        return line.join(" ");
    }
    process.exe().map_or_else(
        || process.name().to_string_lossy().into_owned(),
        |exe| home_relative(exe, home),
    )
}

/// Render `path` with the home directory collapsed to `~`.
fn home_relative(path: &Path, home: ScannerHome<'_>) -> String {
    let full = path.display().to_string();
    let ScannerHome::Known(home) = home else {
        return full;
    };
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => HOME_ALIAS.to_string(),
        Ok(rest) => format!("{HOME_ALIAS}/{}", rest.display()),
        Err(_) => full,
    }
}

/// A tilde is meaningful only when the writer and scanner agree on its prefix.
fn registration_directory(record: &RegistrationCandidate, scanner_home: ScannerHome<'_>) -> String {
    match record.writer_home() {
        WriterHome::Known(writer_home)
            if scanner_home == ScannerHome::Known(writer_home.as_path()) =>
        {
            home_relative(record.directory(), scanner_home)
        },
        WriterHome::Known(_) | WriterHome::Unavailable => record.directory().display().to_string(),
    }
}

/// Local `hh:mm` for a UNIX timestamp in seconds.
fn start_label(epoch_seconds: u64) -> String {
    let seconds = i64::try_from(epoch_seconds).unwrap_or_default();
    DateTime::from_timestamp(seconds, 0).map_or_else(
        || UNRESOLVED_TIME.to_string(),
        |stamp| {
            stamp
                .with_timezone(&Local)
                .format(START_TIME_FORMAT)
                .to_string()
        },
    )
}

/// `mm:ss`, widening to `hh:mm:ss` once a run passes an hour.
fn duration_label(seconds: u64) -> String {
    let hours = seconds / SECONDS_PER_HOUR;
    let minutes = seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE;
    let remainder = seconds % SECONDS_PER_MINUTE;
    if hours == 0 {
        format!("{minutes:02}:{remainder:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{remainder:02}")
    }
}

/// Split argv into the program's bare name and the rest of the line,
/// retaining whether unavailable metadata or exclusion prevents a cargo row.
///
/// A cargo binary installed under an alias still reads as `cargo`: the
/// name on disk is an artifact of how it was wrapped, not of what the
/// user typed.
///
/// [`RowAbsence::Excluded`] rejects readable argv belonging to another program.
/// [`Census::take`] classifies on [`sysinfo::Process::name`], and
/// macOS does not always let sysinfo read a process's executable: when
/// it cannot, the name reported is the parent's. Every `sccache` a build
/// spawns is a child of cargo, so a whole burst of them can present as
/// cargo at once. Their argv still reads `sccache /path/to/rustc …`,
/// which names no cargo binary, and that is what settles it.
fn command_text(argv: &[OsString], home: ScannerHome<'_>) -> Result<CommandText, RowAbsence> {
    let CargoArguments { start, layout } = cargo_split(argv)?;
    let mut arguments: Vec<String> = argv
        .iter()
        .skip(start)
        .map(|argument| home_relative(Path::new(argument), home))
        .collect();
    // An external subcommand's binary is usually handed its own name
    // back as the first argument -- `cargo-nextest nextest run` -- but
    // a caller invoking the binary directly skips that. Putting it back
    // is what makes both spell the command that was typed.
    if let ArgumentLayout::External(subcommand) = layout
        && arguments.first() != Some(&subcommand)
    {
        arguments.insert(0, subcommand);
    }
    Ok(CommandText {
        program: CARGO_DISPLAY_NAME.to_string(),
        arguments,
    })
}

/// Group construction preserves the reason a candidate cannot lead a tile.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GroupAbsence {
    /// The detailed process snapshot contains no such pid.
    NoProcess,
    /// The complete census supplied no invocation identity.
    NoIdentity,
    /// Required process fields remain unavailable after direct registration merging.
    Unavailable,
    /// Observed arguments or configured policy deliberately reject this command.
    Excluded,
}

impl From<RowAbsence> for GroupAbsence {
    fn from(absence: RowAbsence) -> Self {
        match absence {
            RowAbsence::Unavailable => Self::Unavailable,
            RowAbsence::Excluded => Self::Excluded,
        }
    }
}

/// Why a process-table candidate cannot become a cargo row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RowAbsence {
    /// The external process API could not supply argv.
    Unavailable,
    /// The available arguments identify another program or an excluded command.
    Excluded,
}

/// The two layouts carry different rules for rebuilding the displayed command.
#[derive(Debug, Eq, PartialEq)]
enum ArgumentLayout {
    /// Arguments immediately follow a cargo executable within argv.
    Cargo,
    /// The executable itself supplies the cargo subcommand name.
    External(String),
}

/// A recognized cargo argv retains where arguments begin and their interpretation.
#[derive(Debug, Eq, PartialEq)]
struct CargoArguments {
    /// Skip wrappers and the cargo executable when displaying arguments.
    start:  usize,
    /// External subcommands may need their name inserted into the display.
    layout: ArgumentLayout,
}

/// Unavailable argv differs from a readable argv that deliberately excludes a row.
fn cargo_split(argv: &[OsString]) -> Result<CargoArguments, RowAbsence> {
    if let Some(start) = cargo_argv_start(argv) {
        return Ok(CargoArguments {
            start:  start + 1,
            layout: ArgumentLayout::Cargo,
        });
    }
    let program = argv.first().ok_or(RowAbsence::Unavailable)?;
    let subcommand = external_subcommand(program).ok_or(RowAbsence::Excluded)?;
    Ok(CargoArguments {
        start:  1,
        layout: ArgumentLayout::External(subcommand),
    })
}

/// User exclusions and unavailable process metadata retain separate reasons.
fn select_cargo(argv: &[OsString], excluded: &[String]) -> Result<CargoArguments, RowAbsence> {
    let arguments = cargo_split(argv)?;
    if is_excluded(argv, excluded) {
        return Err(RowAbsence::Excluded);
    }
    Ok(arguments)
}

/// Whether an argv belongs to a cargo invocation at all.
///
/// [`Census::take`] classifies on the process's own name, and a process
/// can wear one without being one -- see [`command_text`] -- so this is
/// what settles it.
fn names_cargo(argv: &[OsString]) -> bool { cargo_split(argv).is_ok() }

/// The subcommand an argv names: the first word past the cargo binary
/// that is neither a flag nor a `+toolchain` selector.
fn subcommand(argv: &[OsString]) -> Option<String> {
    let CargoArguments { start, layout } = cargo_split(argv).ok()?;
    if let ArgumentLayout::External(subcommand) = layout {
        return Some(subcommand);
    }
    argv.iter()
        .skip(start)
        .map(|argument| argument.to_string_lossy().into_owned())
        .find(|argument| !argument.starts_with('-') && !argument.starts_with('+'))
}

/// Whether `commands.excluded` names the subcommand an argv carries.
///
/// Keyed on the subcommand rather than the binary so one entry covers
/// both spellings of the same command: `cargo berth claim` run through
/// cargo and `cargo-berth berth claim` run as its own binary answer
/// [`subcommand`] the same.
fn is_excluded(argv: &[OsString], excluded: &[String]) -> bool {
    subcommand(argv).is_some_and(|subcommand| excluded.contains(&subcommand))
}

/// Whether a process's own name is one a cargo invocation wears.
///
/// Three spellings reach here: `cargo` itself, the name a shim's
/// wrapped binary was renamed to, and `cargo-<subcommand>` for every
/// tool installed as an external subcommand. This binary is the one
/// `cargo-` name left out -- cargo-tile watching the builds is not one
/// of the builds.
fn is_cargo_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    name != SELF_PROCESS_NAME
        && (CARGO_PROCESS_NAMES.contains(&name) || name.starts_with(CARGO_SUBCOMMAND_PREFIX))
}

/// The subcommand an external cargo tool's binary name carries, or
/// `None` when the name is not one.
fn external_subcommand(argument: &OsString) -> Option<String> {
    let name = base_name(argument);
    if name == SELF_PROCESS_NAME || CARGO_PROCESS_NAMES.contains(&name.as_str()) {
        return None;
    }
    Some(name.strip_prefix(CARGO_SUBCOMMAND_PREFIX)?.to_string())
}

/// Where the cargo binary sits in argv, or `None` when none of it names
/// one.
///
/// A shim caught before it hands off still has its interpreter at
/// argv\[0\] — `zsh /path/to/cargo check …`. Starting at the cargo binary
/// instead renders that identically to the same command a moment later,
/// once the real cargo is running it.
fn cargo_argv_start(argv: &[OsString]) -> Option<usize> { argv.iter().position(is_cargo_binary) }

/// Whether an argv entry names a cargo binary, under any of the names one
/// gets installed as.
fn is_cargo_binary(argument: &OsString) -> bool {
    CARGO_PROCESS_NAMES.contains(&base_name(argument).as_str())
}

/// An argv entry's trailing path component.
fn base_name(argument: &OsString) -> String {
    PathBuf::from(argument)
        .file_name()
        .unwrap_or(argument.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::ErrorKind;
    use std::io::Write;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::process::Child;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::COORDINATION_SUBCOMMAND_NAME;
    use crate::constants::DEFAULT_EXCLUDED;
    use crate::constants::DEFAULT_HIDDEN_WHEN_IDLE;
    use crate::constants::PID_SEPARATOR;
    use crate::constants::RUN_LOG_PREFIX;
    use crate::constants::RUN_LOG_SUFFIX;
    use crate::constants::SIBLING_SUBCOMMAND_NAME;
    use crate::constants::TABLE_CELL;
    use crate::progress::CaptureRead;
    use crate::progress::CaptureRootIndex;
    use crate::progress::RunState;
    use crate::registration::Registration;
    use crate::roster::FamilyHead;
    use crate::tiles::TileDemands;

    /// Reap the metadata fixture even if an assertion fails before its exec transition.
    struct MetadataProcess {
        /// The test owns stdin, lifetime, and cleanup of the sampled process.
        child: Child,
    }

    impl Drop for MetadataProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    /// Keep a cargo-shaped argv alive until the owning fixture is dropped.
    fn cargo_process(command: &str) -> MetadataProcess {
        MetadataProcess {
            child: std::process::Command::new("sh")
                .args(["-c", "read -r release", "cargo", command])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .spawn()
                .expect("cargo command fixture"),
        }
    }

    /// Measurements are supplied explicitly by tests that need process evidence.
    fn no_measurements() -> Attributed {
        Attributed {
            compilers: HashMap::new(),
            cpu:       HashMap::new(),
        }
    }

    /// Run production selection and assembly against a controlled process snapshot.
    fn fixture_groups(
        census: &mut Census,
        system: &System,
        capture: &Capture,
        excluded: &[String],
    ) -> Vec<CargoGroup> {
        census.identify_capture_wrappers(system, capture);
        census.identify_captures(capture);
        census.select_rows(system, capture, excluded);
        census.groups(
            system,
            &no_measurements(),
            ScannerHome::Known(Path::new("/writer")),
            capture,
        )
    }

    #[test]
    fn confirmed_registration_sources_directory_command_time_and_unknown_measurements() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[]);
        let groups = fixture_groups(&mut census, &System::new(), &capture, &[]);
        assert_eq!(groups.len(), 1);
        let row = &groups[0].lead;
        assert_eq!(row.pid, 10);
        assert_eq!(row.path, "~/project");
        assert_eq!(row.command, CommandText::of("cargo", &["build"]));
        assert_eq!(
            row.directory_identity,
            WorkingDirectoryIdentity::Absolute("/writer/project".into())
        );
        assert!(matches!(row.started, RunStart::Known(_)));
        assert!(matches!(row.provenance, RowProvenance::Direct(_)));
        assert_eq!(
            row.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            row.managed,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(row.compiler, CompilerObservation::Unknown);
    }

    /// Missing account records change the label without discarding verified ownership.
    #[test]
    fn unresolved_account_lookup_preserves_fallback_row_owner() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let mut capture = verified_capture(root.path());
        let users = Users::new();
        let status = capture.root_status.first_mut().expect("observed root");
        let RootOwner::Uid(uid) = status.owner else {
            panic!("verified root has an observed owner");
        };
        status.account = AccountName::resolve(status.owner, &users);
        assert_eq!(status.account, AccountName::Unavailable);
        assert_eq!(
            AccountName::resolve(RootOwner::Unavailable, &users),
            AccountName::Unavailable
        );
        let mut census = census_of(&[]);
        let groups = fixture_groups(&mut census, &System::new(), &capture, &[]);
        assert_eq!(groups.len(), 1);
        let RowProvenance::Direct(context) = &groups[0].lead.provenance else {
            panic!("missing account name does not remove verified provenance");
        };
        assert_eq!(context.account.uid, uid);
        assert_eq!(context.account.name, AccountName::Unavailable);
    }

    #[test]
    fn registration_timestamp_failure_preserves_row_and_orders_it_after_known_starts() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let DirectAssociation::Direct(mut direct) = capture.row_source(10) else {
            panic!("verified source");
        };
        direct.modified = Err(std::io::Error::from(ErrorKind::PermissionDenied).into());
        let unknown = registration_row(
            &direct,
            &capture,
            ScannerHome::Unavailable,
            SystemTime::now(),
        )
        .expect("row survives missing mtime");
        assert_eq!(unknown.started, RunStart::Unavailable);
        assert_eq!(unknown.start, UNRESOLVED_TIME);
        assert_eq!(unknown.duration, UNRESOLVED_TIME);
        let mut known = directory_row();
        known.started = RunStart::Known(1);
        let mut ordered = vec![unknown.clone(), known.clone()];
        ordered.sort_by(newest_first);
        assert_eq!(ordered, [known, unknown]);
        assert!(RunStart::Known(u64::MAX) < RunStart::Unavailable);
    }

    #[test]
    fn direct_registration_fills_cwd_and_argv_independently_before_formatting() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let direct = capture.row_source(10);
        let observed = [OsString::from("cargo"), OsString::from("test")];
        let (identity, path, command) = row_fields(
            WorkingDirectoryObservation::Unavailable,
            &observed,
            &direct,
            ScannerHome::Known(Path::new("/writer")),
        )
        .expect("merge directory");
        assert_eq!(
            identity,
            WorkingDirectoryIdentity::Absolute("/writer/project".into())
        );
        assert_eq!(path, "~/project");
        assert_eq!(command, CommandText::of("cargo", &["test"]));
        let (identity, path, command) = row_fields(
            WorkingDirectoryObservation::Observed(Path::new("/own")),
            &[],
            &direct,
            ScannerHome::Unavailable,
        )
        .expect("merge argv");
        assert_eq!(identity, WorkingDirectoryIdentity::Absolute("/own".into()));
        assert_eq!(path, "/own");
        assert_eq!(command, CommandText::of("cargo", &["build"]));
        assert_eq!(
            row_fields(
                WorkingDirectoryObservation::Unavailable,
                &[OsString::from("application")],
                &direct,
                ScannerHome::Unavailable
            ),
            Err(RowAbsence::Excluded)
        );
    }

    #[test]
    fn process_with_unavailable_cwd_keeps_pid_and_measurements_after_merge() {
        let fixture = cargo_process("build");
        let pid = Pid::from_u32(fixture.child.id());
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            ProcessRefreshKind::nothing()
                .without_tasks()
                .with_cmd(UpdateKind::Always),
        );
        let process = system.process(pid).expect("live process");
        assert!(process.cwd().is_none());
        let row = row(
            process,
            InvocationId::for_test(pid.as_u32()),
            CompilerObservation::None,
            Measurement::Reading(3),
            ScannerHome::Known(Path::new("/writer")),
            Measurement::Reading(42.0),
            &capture.row_source(10),
        )
        .expect("merged process row");
        assert_eq!(row.pid, pid.as_u32());
        assert_eq!(row.path, "~/project");
        assert_eq!(row.cpu, Measurement::Reading("42%".into()));
        assert_eq!(row.managed, Measurement::Reading(3));
        assert_eq!(row.compiler, CompilerObservation::None);
    }

    #[test]
    fn registration_and_process_scans_keep_one_invocation_and_exclusions_apply_to_both() {
        let fixture = cargo_process("build");
        let pid = Pid::from_u32(fixture.child.id());
        let system = process_details(&[pid]);
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let modified = UNIX_EPOCH + Duration::from_secs(1234);
        fs::File::open(
            root.path()
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join("10.generation"),
        )
        .expect("registration descriptor")
        .set_times(fs::FileTimes::new().set_modified(modified))
        .expect("distinct invocation start");
        let capture = verified_capture(root.path());
        let mut absent = census_of(&[]);
        let first = fixture_groups(&mut absent, &System::new(), &capture, &[]);
        let mut present = census_of(&[(pid.as_u32(), 10)]);
        present.cargo.push(pid);
        let second = fixture_groups(&mut present, &system, &capture, &[]);
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].id(), second[0].id());
        assert_eq!(first[0].lead.started, RunStart::Known(1234));
        assert_eq!(second[0].lead.started, first[0].lead.started);
        assert_eq!(second[0].lead.start, first[0].lead.start);
        assert_eq!(first[0].lead.pid, 10);
        assert_eq!(second[0].lead.pid, pid.as_u32());
        let mut roster = crate::roster::Roster::new();
        let now = Instant::now();
        roster.observe(first, now);
        let ids = roster.tiled_ids(&[]);
        roster.observe(second, now + poll());
        assert_eq!(roster.tiled_ids(&[]), ids);
        assert_eq!(roster.groups().len(), 1);
        for mut census in [census_of(&[]), census_of(&[(pid.as_u32(), 10)])] {
            census.cargo.push(pid);
            assert!(fixture_groups(&mut census, &system, &capture, &["build".into()]).is_empty());
        }
    }

    #[test]
    fn unknown_legacy_and_ambiguous_publications_never_source_rows_and_ambiguity_recovers() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "first",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let unknown = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert!(fixture_groups(&mut census_of(&[]), &System::new(), &unknown, &[]).is_empty());
        assert!(matches!(unknown.row_source(10), DirectAssociation::None));
        write_versioned_capture(root.path(), 10, "second", "/other", "/writer", "test", "");
        let mut competing = verified_capture(root.path());
        let census = census_of(&[]);
        assert!(fixture_groups(&mut census_of(&[]), &System::new(), &competing, &[]).is_empty());
        census.associate_status(&mut competing, &[]);
        let associations = &competing.root_status[0].associations;
        assert_eq!(associations.len(), 1);
        assert!(
            matches!(&associations[0].selection, AssociationSelection::Ambiguous { candidates } if candidates.len() == 2)
        );
        let mut row = directory_row();
        census.annotate_capture(&mut row, &competing, ScannerHome::Unavailable);
        assert_eq!(row.provenance, RowProvenance::Uncaptured);
        assert_eq!(row.state, CaptureLookup::Unregistered);
        fs::remove_file(root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10.second"))
            .expect("remove competing publication");
        let recovered = verified_capture(root.path());
        assert_eq!(
            fixture_groups(&mut census_of(&[]), &System::new(), &recovered, &[]).len(),
            1
        );
        let legacy_root = capture_root(&[(10, "")]);
        let legacy = verified_capture(legacy_root.path());
        assert!(fixture_groups(&mut census_of(&[]), &System::new(), &legacy, &[]).is_empty());
    }

    #[test]
    fn two_confirmed_roots_select_one_fallback_and_report_the_unused_proof() {
        let first = tempdir().expect("first root");
        let second = tempdir().expect("second root");
        write_versioned_capture(
            first.path(),
            10,
            "first",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        write_versioned_capture(second.path(), 10, "second", "/other", "/writer", "test", "");
        let roots = resolved_test_roots(&[first.path(), second.path()]);
        let stamp = directory_record("/writer").identity().clone();
        let IdentityEvidence::Available(stamp) = stamp else {
            panic!("birth");
        };
        let mut capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        let mut census = census_of(&[]);
        let groups = fixture_groups(&mut census, &System::new(), &capture, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.path, "~/project");
        assert_eq!(groups[0].lead.command, CommandText::of("cargo", &["build"]));
        census.associate_status(&mut capture, &groups);
        assert!(matches!(&capture.root_status[0].associations[0].selection,
            AssociationSelection::Selected { key, proof: SelectedProof::Confirmed, unused }
            if key.root == CaptureRootIndex(0) && unused.len() == 1 && unused[0].key.root == CaptureRootIndex(1) && unused[0].reason == UnusedCaptureReason::RootPrecedence));
    }

    #[test]
    fn selected_unconfirmed_root_never_borrows_another_roots_metadata_or_adds_a_row() {
        let first = capture_root(&[(10, "")]);
        let second = tempdir().expect("second root");
        write_versioned_capture(second.path(), 10, "second", "/other", "/writer", "test", "");
        let roots = resolved_test_roots(&[first.path(), second.path()]);
        let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone()
        else {
            panic!("birth");
        };
        let mut capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        let mut census = census_of(&[]);
        census.select_rows(&System::new(), &capture, &[]);
        let mut row = directory_row();
        census.annotate_capture(&mut row, &capture, ScannerHome::Unavailable);
        let mut rows = vec![row.clone()];
        census.add_registration_rows(
            &mut rows,
            &capture,
            ScannerHome::Unavailable,
            SystemTime::now(),
        );
        assert_eq!(rows, [row]);
        assert_eq!(rows[0].provenance, RowProvenance::Uncaptured);
        assert_eq!(rows[0].command, CommandText::of("cargo", &["build"]));
        let groups = census.assemble_groups(&System::new(), ScannerHome::Unavailable, rows);
        census.associate_status(&mut capture, &groups);
        assert!(matches!(&capture.root_status[0].associations[0].selection,
            AssociationSelection::Selected { proof: SelectedProof::Unconfirmed, unused, .. }
            if unused.len() == 1 && unused[0].reason == UnusedCaptureReason::SelectedUnconfirmed));
    }

    #[test]
    fn unreadable_log_does_not_remove_verified_registration_row_or_change_active_count() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let log = root.path().join("run-generation-10.log");
        fs::remove_file(&log).expect("remove log");
        fs::create_dir(&log).expect("unreadable log");
        let capture = verified_capture(root.path());
        assert_eq!(capture.root_status[0].confirmed, 0);
        assert!(
            capture.root_status[0]
                .diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::LogUnreadable(_)))
        );
        let groups = fixture_groups(&mut census_of(&[]), &System::new(), &capture, &[]);
        assert_eq!(groups.len(), 1);
        assert!(matches!(
            groups[0].lead.state,
            CaptureLookup::Registered(CaptureRead::Unreadable(_))
        ));
        assert_eq!(capture.root_status[0].confirmed, 0);
    }

    #[test]
    fn enclosing_descendant_does_not_suppress_fallback_and_both_sources_assemble_one_tree() {
        let child = cargo_process("test");
        let child_pid = Pid::from_u32(child.child.id());
        let parent = cargo_process("build");
        let parent_pid = Pid::from_u32(parent.child.id());
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let child_only = process_details(&[child_pid]);
        let both = process_details(&[parent_pid, child_pid]);
        let mut census = census_of(&[
            (parent_pid.as_u32(), 10),
            (child_pid.as_u32(), parent_pid.as_u32()),
        ]);
        census.cargo = vec![child_pid];
        let first = fixture_groups(&mut census, &child_only, &capture, &[]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].rest.len(), 1);
        assert!(matches!(
            first[0].rest[0].capture_membership,
            CaptureMembership::Enclosing(_)
        ));
        assert!(matches!(
            first[0].rest[0].provenance,
            RowProvenance::Enclosing(_)
        ));
        assert_eq!(
            first[0].rest[0].command,
            CommandText::of("cargo", &["test"])
        );
        assert_eq!(
            first[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  first[0].id(),
                pid: 10,
            }
        );
        let mut present = census_of(&[
            (parent_pid.as_u32(), 10),
            (child_pid.as_u32(), parent_pid.as_u32()),
        ]);
        present.cargo = vec![parent_pid, child_pid];
        let second = fixture_groups(&mut present, &both, &capture, &[]);
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].rest.len(), 1);
        assert_eq!(second[0].id(), first[0].id());
        assert_eq!(second[0].lead.pid, parent_pid.as_u32());
        assert_eq!(
            second[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  first[0].id(),
                pid: parent_pid.as_u32(),
            }
        );
        let mut missing_parent = census_of(&[
            (parent_pid.as_u32(), 10),
            (child_pid.as_u32(), parent_pid.as_u32()),
        ]);
        missing_parent.cargo = vec![parent_pid, child_pid];
        let partial = fixture_groups(&mut missing_parent, &child_only, &capture, &[]);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].id(), first[0].id());
        assert_eq!(partial[0].rest.len(), 1);
        assert_eq!(
            partial[0].rest[0].invocation_id,
            first[0].rest[0].invocation_id
        );
        assert_family_source_transitions(&[first.clone(), second, partial, first]);
    }

    /// Source transitions retain family assignments and the focused tile together.
    fn assert_family_source_transitions(scans: &[Vec<CargoGroup>]) {
        let mut roster = crate::roster::Roster::new();
        let mut grid = crate::tiles::TileGrid::new();
        grid.set_layout(ratatui::layout::Rect::new(0, 0, 120, 40), 1);
        let mut expected_family = FamilyHead::NoChildren;
        for (index, scan) in scans.iter().enumerate() {
            roster.observe(scan.clone(), Instant::now());
            assert_eq!(roster.groups().len(), 1);
            let tracked = &roster.groups()[0];
            assert_eq!(tracked.rows().count(), 2);
            assert!(tracked.rows().all(|row| !row.is_ended()));
            if index == 0 {
                expected_family = tracked.lead.family();
            }
            assert_eq!(tracked.lead.family(), expected_family);
            assert!(matches!(
                expected_family,
                crate::roster::FamilyHead::Heads(_)
            ));
            let demands = TileDemands {
                summary: 3,
                groups:  vec![crate::tiles::TileDemand {
                    id:   tracked.id.clone(),
                    rows: 3,
                }],
            };
            grid.sync(&demands, 1);
            if index == 0 {
                grid.focus_cell(TABLE_CELL + 1);
            }
            assert!(
                grid.placements(ratatui::layout::Rect::new(0, 0, 120, 40), 1)
                    .iter()
                    .any(|placement| placement.content
                        == crate::tiles::TileContent::Group(tracked.id.clone())
                        && placement.frame.is_focused())
            );
        }
    }

    #[test]
    fn enclosing_membership_never_suppresses_fallback_even_at_the_registered_pid() {
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(10, 1)]);
        census.capture_mismatches.insert(Pid::from_u32(10));
        census.select_rows(&System::new(), &capture, &[]);
        let mut process = directory_row();
        process.command = CommandText::of("cargo", &["test"]);
        census.annotate_capture(&mut process, &capture, ScannerHome::Unavailable);
        assert!(matches!(
            process.capture_membership,
            CaptureMembership::Enclosing(_)
        ));
        let mut rows = vec![process];
        census.add_registration_rows(
            &mut rows,
            &capture,
            ScannerHome::Unavailable,
            SystemTime::now(),
        );
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].invocation_id, rows[1].invocation_id);
        assert_eq!(rows[0].command, CommandText::of("cargo", &["test"]));
        assert_eq!(rows[1].command, CommandText::of("cargo", &["build"]));
    }

    #[test]
    fn current_registration_argv_is_reconsidered_when_the_census_name_was_not_cargo() {
        let fixture = cargo_process("build");
        let pid = Pid::from_u32(fixture.child.id());
        let system = process_details(&[pid]);
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            pid.as_u32(),
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(pid.as_u32(), 1)]);
        assert!(census.cargo.is_empty());
        census.include_registered_processes(&system, &capture);
        assert_eq!(census.cargo, [pid]);
        let groups = fixture_groups(&mut census, &system, &capture, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, pid.as_u32());
        assert_eq!(groups[0].lead.managed, Measurement::Reading(0));
        assert!(matches!(
            groups[0].lead.invocation_id,
            InvocationId::Captured(_)
        ));
    }

    #[test]
    fn assembled_managed_counts_include_registration_children_and_nested_parents() {
        let root = tempdir().expect("root");
        for pid in [20, 30] {
            write_versioned_capture(
                root.path(),
                pid,
                "generation",
                "/writer/project",
                "/writer",
                "build",
                "",
            );
        }
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(20, 10), (30, 20)]);
        census.select_rows(&System::new(), &capture, &[]);
        let mut rows = vec![directory_row()];
        census.add_registration_rows(
            &mut rows,
            &capture,
            ScannerHome::Unavailable,
            SystemTime::now(),
        );
        let groups = census.assemble_groups(&System::new(), ScannerHome::Unavailable, rows);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 10);
        assert_eq!(groups[0].lead.managed, Measurement::Reading(2));
        assert_eq!(groups[0].rest.len(), 2);
        let parent = groups[0]
            .rest
            .iter()
            .find(|row| row.pid == 20)
            .expect("parent");
        assert_eq!(parent.managed, Measurement::Reading(1));
        let child = groups[0]
            .rest
            .iter()
            .find(|row| row.pid == 30)
            .expect("child");
        assert_eq!(
            child.managed,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            child.parent,
            VisibleParent::Invocation {
                id:  parent.invocation_id.clone(),
                pid: parent.pid,
            }
        );
    }

    #[test]
    fn group_absence_distinguishes_process_identity_and_deliberate_exclusion() {
        let fixture = cargo_process("build");
        let pid = Pid::from_u32(fixture.child.id());
        let system = process_details(&[pid]);
        let mut census = census_of(&[]);
        let children = HashMap::new();
        let capture = Capture::default();
        assert_eq!(
            census.group(
                &System::new(),
                &no_measurements(),
                ScannerHome::Unavailable,
                &children,
                &capture,
                pid
            ),
            Err(GroupAbsence::NoProcess)
        );
        assert_eq!(
            census.group(
                &system,
                &no_measurements(),
                ScannerHome::Unavailable,
                &children,
                &capture,
                pid
            ),
            Err(GroupAbsence::NoIdentity)
        );
        census.eligibility.insert(pid, Err(RowAbsence::Excluded));
        assert_eq!(
            census.group(
                &system,
                &no_measurements(),
                ScannerHome::Unavailable,
                &children,
                &capture,
                pid
            ),
            Err(GroupAbsence::Excluded)
        );
    }

    #[test]
    fn fresh_details_replace_cached_command_directory_and_executable() {
        let root = tempdir().expect("metadata directories");
        let before = root.path().join("before");
        let after = root.path().join("after");
        fs::create_dir(&before).expect("initial directory");
        fs::create_dir(&after).expect("replacement directory");
        let mut fixture = MetadataProcess {
            child: std::process::Command::new("sh")
                .args([
                    "-c",
                    "printf 'ready\\n'; read -r release; cd \"$1\" || exit 1; exec sleep 60",
                    "metadata",
                ])
                .arg(&after)
                .current_dir(&before)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("metadata fixture"),
        };
        let mut ready = String::new();
        BufReader::new(fixture.child.stdout.take().expect("ready pipe"))
            .read_line(&mut ready)
            .expect("fixture handshake");
        assert_eq!(ready, "ready\n");
        let pid = Pid::from_u32(fixture.child.id());
        let mut cached = process_details(&[pid]);
        let original = cached.process(pid).expect("initial process");
        let command = original.cmd().to_vec();
        let executable = original.exe().expect("initial executable").to_path_buf();
        assert_eq!(original.cwd(), Some(before.as_path()));
        fixture
            .child
            .stdin
            .take()
            .expect("release pipe")
            .write_all(b"\n")
            .expect("release exec");
        let deadline = Instant::now() + WORKER_REPLY_TIMEOUT;
        loop {
            let refreshed = process_details(&[pid]);
            if let Some(process) = refreshed.process(pid)
                && process.cwd() == Some(after.as_path())
                && process.exe().is_some_and(|exe| exe != executable)
            {
                assert_ne!(process.cmd(), command);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "fixture must exec with new metadata"
            );
            thread::sleep(poll());
        }
        cached.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            false,
            process_detail_refresh_kind(),
        );
        let retained = cached.process(pid).expect("cached process");
        assert_eq!(retained.cmd(), command);
        assert_eq!(retained.cwd(), Some(before.as_path()));
        assert_eq!(retained.exe(), Some(executable.as_path()));
        drop(fixture);
        assert!(process_details(&[pid]).process(pid).is_none());
    }

    #[test]
    fn shim_detection_requires_observed_cwds_and_subcommands() {
        let argv = [OsString::from("cargo"), OsString::from("build")];
        let cwd = Path::new("/work");
        assert!(!observed_shim_match(
            &argv,
            WorkingDirectoryObservation::Unavailable,
            &argv,
            WorkingDirectoryObservation::Unavailable
        ));
        assert!(!observed_shim_match(
            &argv,
            WorkingDirectoryObservation::Observed(cwd),
            &argv,
            WorkingDirectoryObservation::Unavailable
        ));
        assert!(!observed_shim_match(
            &[],
            WorkingDirectoryObservation::Observed(cwd),
            &[],
            WorkingDirectoryObservation::Observed(cwd)
        ));
        assert!(observed_shim_match(
            &argv,
            WorkingDirectoryObservation::Observed(cwd),
            &argv,
            WorkingDirectoryObservation::Observed(cwd)
        ));
        assert!(!observed_shim_match(
            &argv,
            WorkingDirectoryObservation::Observed(cwd),
            &argv,
            WorkingDirectoryObservation::Observed(Path::new("/other"))
        ));
    }

    /// A confirmed record supplies a real proof without using host process allocation.
    fn verified_capture(root: &Path) -> Capture {
        let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone()
        else {
            return Capture::default();
        };
        Capture::take_from(root, |pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        })
    }

    #[test]
    fn direct_registration_and_process_share_identity_while_nested_invocations_do_not() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(11, 10), (12, 11)]);
        census.cargo = vec![Pid::from_u32(11), Pid::from_u32(12)];
        census.identify_captures(&capture);
        let direct = match census.direct_capture(&capture, Pid::from_u32(11)) {
            DirectAssociation::Direct(direct) => Ok(direct),
            DirectAssociation::None => Err("first cargo must directly represent the registration"),
        }
        .expect("direct proof");
        let mut process = directory_row();
        process.pid = 11;
        census.annotate_capture(&mut process, &capture, ScannerHome::Unavailable);
        assert_eq!(process.invocation_id, direct.invocation_id());
        assert_eq!(process.capture_membership, CaptureMembership::Outside);
        let mut nested = directory_row();
        nested.pid = 12;
        nested.path = "/nested".to_owned();
        nested.command = CommandText::of("cargo", &["test"]);
        census.annotate_capture(&mut nested, &capture, ScannerHome::Unavailable);
        assert_ne!(process.invocation_id, nested.invocation_id);
        assert_eq!(
            nested.capture_membership,
            CaptureMembership::Enclosing(direct.run_id)
        );
        assert_eq!(nested.path, "/nested");
        assert_eq!(nested.command, CommandText::of("cargo", &["test"]));
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(12)),
            DirectAssociation::None
        );
    }

    #[test]
    fn unverified_nearest_registration_cannot_supply_direct_row_metadata() {
        let root = capture_root(&[(10, "")]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let census = census_of(&[(11, 10)]);
        assert!(matches!(
            census.captured_run(&capture, Pid::from_u32(11)),
            NearestRegistration::Registered(_)
        ));
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(11)),
            DirectAssociation::None
        );
    }

    #[test]
    fn identical_pid_and_birth_with_different_generations_never_share_cpu_history() {
        let root = tempdir().expect("capture root");
        for generation in ["first", "second"] {
            write_versioned_capture(
                root.path(),
                10,
                generation,
                "/writer/project",
                "/writer",
                "build",
                "",
            );
        }
        let capture = verified_capture(root.path());
        let identities: Vec<_> = capture
            .confirmed()
            .iter()
            .map(|confirmed| {
                InvocationId::Captured(RunId::verified(&confirmed.key, &confirmed.registration))
            })
            .collect();
        assert_eq!(identities.len(), 2);
        assert_ne!(identities[0], identities[1]);
        let mut smoothing = CpuSmoothing::default();
        let now = Instant::now();
        smoothing.settle(
            &HashMap::from([(identities[0].clone(), Measurement::Reading(400.0))]),
            &identities[..1],
            now,
        );
        let reported = smoothing.settle(
            &HashMap::from([(
                identities[1].clone(),
                Measurement::Unavailable(MeasurementAbsence::FirstObservation),
            )]),
            &identities[1..],
            now + poll(),
        );
        assert_eq!(
            reported[&identities[1]],
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert!(!smoothing.settled.contains_key(&identities[0]));
        assert!(!smoothing.reported.contains_key(&identities[0]));
    }

    #[test]
    fn unavailable_lifetimes_retain_present_rows_without_claiming_cpu_continuity() {
        let pid = Pid::from_u32(10);
        let lifetimes = HashMap::from([(pid, LifetimeEvidence::Unavailable)]);
        let mut identities = ProcessIdentities::default();
        let first = identities.observe(&lifetimes);
        assert_eq!(first, identities.observe(&lifetimes));
        assert!(matches!(
            first[&pid],
            InvocationId::Process(ProcessIdentity::Unavailable { .. })
        ));
        assert!(identities.observe(&HashMap::new()).is_empty());
        assert_ne!(first, identities.observe(&lifetimes));
        let previous = HashMap::from([(
            Pid::from_u32(10),
            CpuBaseline {
                lifetime:    LifetimeEvidence::Unavailable,
                accumulated: 10,
            },
        )]);
        assert_eq!(
            Census::measure_cpu(
                Pid::from_u32(10),
                50.0,
                &CpuBaseline {
                    lifetime:    LifetimeEvidence::Unavailable,
                    accumulated: 20,
                },
                &previous
            ),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn exec_replaced_application_separates_nested_cargo_from_the_enclosing_run() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "run",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(11, 10), (12, 11), (13, 11)]);
        census.cargo = vec![Pid::from_u32(12), Pid::from_u32(13)];
        census.identify_captures(&capture);
        let mut nested = Vec::new();
        for pid in [12, 13] {
            assert_eq!(
                census.direct_capture(&capture, Pid::from_u32(pid)),
                DirectAssociation::None
            );
            let mut row = directory_row();
            row.pid = pid;
            census.annotate_capture(&mut row, &capture, ScannerHome::Unavailable);
            assert!(matches!(
                row.capture_membership,
                CaptureMembership::Enclosing(_)
            ));
            nested.push(row);
        }
        assert_ne!(nested[0].invocation_id, nested[1].invocation_id);
        census.select_rows(&System::new(), &capture, &["run".to_owned()]);
        for pid in [12, 13] {
            assert_eq!(
                census.eligibility[&Pid::from_u32(pid)],
                Err(RowAbsence::Unavailable)
            );
        }
    }

    #[test]
    fn direct_capture_crosses_only_observed_command_forwarders() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(11, 10), (12, 11), (13, 12)]);
        census.cargo = vec![Pid::from_u32(13)];
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(13)),
            DirectAssociation::None
        );
        census
            .capture_wrappers
            .extend([Pid::from_u32(11), Pid::from_u32(12)]);
        assert!(matches!(
            census.direct_capture(&capture, Pid::from_u32(13)),
            DirectAssociation::Direct(_)
        ));
        census.capture_wrappers.remove(&Pid::from_u32(12));
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(13)),
            DirectAssociation::None
        );
    }

    #[test]
    fn capture_forwarding_matches_exact_registered_arguments_in_both_pty_forms() {
        let record = directory_record("/writer");
        for argv in [
            vec![
                "script",
                "-q",
                "-t",
                "0",
                "/capture/run-generation-10.log",
                "/toolchain/cargo-tile-real",
                "build",
            ],
            vec![
                "script",
                "-q",
                "-e",
                "-f",
                "-c",
                "'/toolchain/cargo-tile-real' 'build' ",
                "/capture/run-generation-10.log",
            ],
            vec!["sh", "-c", "'/toolchain/cargo-tile-real' 'build' "],
            vec![
                "sh",
                "-c",
                "'/writer'\\''s toolchain/cargo-tile-real' 'build' ",
            ],
        ] {
            assert!(forwards_capture_command(
                &argv.into_iter().map(OsString::from).collect::<Vec<_>>(),
                &record
            ));
        }
        for argv in [
            vec!["sh", "/writer/application"],
            vec!["cargo-tile-real", "test"],
            vec!["sh", "-c", "'/toolchain/cargo-tile-real' 'test' "],
            vec!["sh", "-c", "'printf' '/toolchain/cargo-tile-real' 'build' "],
        ] {
            assert!(!forwards_capture_command(
                &argv.into_iter().map(OsString::from).collect::<Vec<_>>(),
                &record
            ));
        }
    }

    #[test]
    fn capture_forwarding_preserves_non_utf8_paths_and_arguments() {
        let mut bytes = directory_record_bytes("/writer");
        bytes.insert(bytes.len() - 1, 0xff);
        let record = match Registration::parse(&bytes).expect("byte-preserving registration") {
            Registration::Versioned(record) => Ok(record),
            Registration::Legacy(_) => Err("expected v2"),
        }
        .expect("versioned record");
        let argv = [
            OsString::from("sh"),
            OsString::from("-c"),
            OsString::from_vec(b"'/toolchain\xff/cargo-tile-real' 'build\xff' ".to_vec()),
        ];
        assert!(forwards_capture_command(&argv, &record));
    }

    #[test]
    fn json_capture_forwarding_removes_all_quiet_flags_only_before_the_separator() {
        for format in [
            vec!["--message-format=json"],
            vec!["--message-format", "json-diagnostic-rendered-ansi"],
        ] {
            let registered: Vec<OsString> = ["check", "--quiet", "-q", "--quiet"]
                .into_iter()
                .chain(format.iter().copied())
                .chain(["--", "--quiet", "-q"])
                .map(OsString::from)
                .chain([OsString::from_vec(b"package\xff".to_vec())])
                .collect();
            let rewritten: Vec<OsString> = std::iter::once("check")
                .chain(format.iter().copied())
                .chain(["--", "--quiet", "-q"])
                .map(OsString::from)
                .chain([OsString::from_vec(b"package\xff".to_vec())])
                .collect();
            assert!(forwards_json_capture_arguments(&rewritten, &registered));
            let mut partial = rewritten.clone();
            partial.insert(1, OsString::from("-q"));
            assert!(!forwards_json_capture_arguments(&partial, &registered));
            let mut changed = rewritten.clone();
            changed[0] = OsString::from("build");
            assert!(!forwards_json_capture_arguments(&changed, &registered));
            let mut changed = rewritten.clone();
            changed.pop();
            changed.push(OsString::from("package"));
            assert!(!forwards_json_capture_arguments(&changed, &registered));
            let mut changed = rewritten;
            changed.remove(changed.len() - 2);
            assert!(!forwards_json_capture_arguments(&changed, &registered));
        }
    }

    #[test]
    fn json_capture_forwarding_rejects_non_json_and_post_separator_formats() {
        for registered in [
            vec!["check", "--quiet"],
            vec!["check", "--quiet", "--message-format=human"],
            vec!["check", "--quiet", "--", "--message-format=json"],
            vec!["check", "--quiet", "--message-format", "--", "json"],
        ] {
            let registered: Vec<_> = registered.into_iter().map(OsString::from).collect();
            let mut rewritten = registered.clone();
            rewritten.remove(1);
            assert!(!forwards_json_capture_arguments(&rewritten, &registered));
        }
    }

    #[test]
    fn same_second_replacement_with_a_higher_counter_begins_a_new_sample() {
        let pid = Pid::from_u32(10);
        let before = LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(100_001));
        let after = LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(100_002));
        assert_ne!(
            ProcessIdentity::observed(10, before.clone()),
            ProcessIdentity::observed(10, after.clone())
        );
        let previous = HashMap::from([(
            pid,
            CpuBaseline {
                lifetime:    before,
                accumulated: 10,
            },
        )]);
        assert_eq!(
            Census::measure_cpu(
                pid,
                80.0,
                &CpuBaseline {
                    lifetime:    after,
                    accumulated: 50,
                },
                &previous
            ),
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
    }

    #[test]
    fn excluded_registration_stays_live_and_preserves_nested_membership_boundaries() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "live",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[(11, 10), (12, 11)]);
        census.cargo = vec![Pid::from_u32(11), Pid::from_u32(12)];
        census.identify_captures(&capture);
        let parents = census.parents.clone();
        census.select_rows(&System::new(), &capture, &["build".to_owned()]);
        assert!(census.cargo.is_empty());
        assert_eq!(
            census.eligibility[&Pid::from_u32(11)],
            Err(RowAbsence::Excluded)
        );
        assert_eq!(
            census.eligibility[&Pid::from_u32(12)],
            Err(RowAbsence::Unavailable)
        );
        assert_eq!(census.parents, parents);
        assert_eq!(
            census.direct_capture(&capture, Pid::from_u32(12)),
            DirectAssociation::None
        );
        assert_eq!(verified_capture(root.path()).confirmed().len(), 1);
        assert!(root.path().join("state/pids/10.live").exists());
        assert!(root.path().join("run-live-10.log").exists());
    }

    /// Bound failed worker handshakes without imposing a startup speed threshold.
    const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    #[test]
    fn spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans() {
        let config = Config::default();
        let parent = tempdir().expect("isolated capture parent");
        let caller = thread::current().id();
        let resolutions = Arc::new(AtomicUsize::new(0));
        let worker_resolutions = Arc::clone(&resolutions);
        let (started, resolution_started) = mpsc::channel();
        let (release, resolution_release) = mpsc::channel();
        let (scans, worker) = spawn_with_resolver(&config, move || {
            assert_ne!(thread::current().id(), caller);
            worker_resolutions.fetch_add(1, Ordering::SeqCst);
            started.send(()).expect("startup observer is alive");
            resolution_release
                .recv_timeout(WORKER_REPLY_TIMEOUT)
                .expect("spawn must return before resolution is released");
            // No capture root is scanned; this test owns only worker scheduling.
            CaptureRoots::from_parent(parent.path())
        });

        resolution_started
            .recv_timeout(WORKER_REPLY_TIMEOUT)
            .expect("worker must reach root resolution");
        assert!(matches!(scans.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).expect("release worker root resolution");
        for _ in 0..2 {
            scans
                .recv_timeout(WORKER_REPLY_TIMEOUT)
                .expect("worker must publish successive scans");
        }
        drop(scans);
        worker
            .join()
            .expect("scanner must exit after receiver drop");
        assert_eq!(resolutions.load(Ordering::SeqCst), 1);
    }

    /// A capture directory holding one live run's log per entry.
    fn capture_root(runs: &[(u32, &str)]) -> TempDir {
        let root = tempdir().expect("temp dir must be created");
        let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("marker dir must be created");
        for (pid, output) in runs {
            let name =
                format!("{RUN_LOG_PREFIX}20260824-084300{PID_SEPARATOR}{pid}{RUN_LOG_SUFFIX}");
            fs::write(root.path().join(name), output).expect("run log must be written");
            fs::write(
                markers.join(pid.to_string()),
                "/writer/project\tcargo build",
            )
            .expect("live marker must be written");
        }
        root
    }

    /// A census that knows nothing but who each process's parent is,
    /// which is all the capture walk reads.
    fn census_of(parents: &[(u32, u32)]) -> Census {
        Census {
            identities:               parents
                .iter()
                .flat_map(|&pair| <[u32; 2]>::from(pair))
                .map(|pid| (Pid::from_u32(pid), InvocationId::for_test(pid)))
                .collect(),
            capture_boundaries:       HashSet::new(),
            capture_wrappers:         HashSet::new(),
            capture_mismatches:       HashSet::new(),
            lifetimes:                HashMap::new(),
            eligibility:              HashMap::new(),
            registration_eligibility: HashMap::new(),
            parents:                  parents
                .iter()
                .map(|&(child, parent)| (Pid::from_u32(child), Pid::from_u32(parent)))
                .collect(),
            cargo:                    Vec::new(),
            compilers:                Vec::new(),
            cpu:                      HashMap::new(),
        }
    }

    /// Keep the same wire fields for parser and capture-annotation fixtures.
    fn directory_record_bytes(home: &str) -> Vec<u8> {
        [
            "cargo-tile-v2",
            "generation",
            "boot",
            "100",
            "run-generation-10.log",
            "/writer/project",
            home,
            "1",
            "build",
            "",
        ]
        .join("\0")
        .into_bytes()
    }

    /// A candidate record can provide text without becoming a verified row source.
    fn directory_record(home: &str) -> RegistrationCandidate {
        match Registration::parse(&directory_record_bytes(home))
            .expect("well-formed directory record")
        {
            Registration::Versioned(record) => Ok(record),
            Registration::Legacy(_) => Err("expected v2 record"),
        }
        .expect("fixture writes a versioned record")
    }

    /// A process-table row whose absolute cwd is independent of display shortening.
    fn directory_row() -> CargoProcess {
        CargoProcess {
            invocation_id:      InvocationId::for_test(10),
            capture_membership: CaptureMembership::Outside,
            provenance:         RowProvenance::Uncaptured,
            path:               "~/project".to_owned(),
            directory_identity: WorkingDirectoryIdentity::Absolute("/writer/project".into()),
            pid:                10,
            parent:             VisibleParent::None,
            start:              "10:00".to_owned(),
            started:            RunStart::Known(0),
            duration:           "00:01".to_owned(),
            cpu:                Measurement::Reading("0%".to_owned()),
            compiler:           CompilerObservation::None,
            state:              CaptureLookup::Unregistered,
            managed:            Measurement::Reading(0),
            nested:             false,
            command:            CommandText::of("cargo", &["build"]),
        }
    }

    /// Exercise the annotation path when asserting the nearest ancestor's reading.
    fn captured_state(census: &Census, capture: &Capture, pid: u32) -> CaptureLookup {
        let mut row = directory_row();
        row.pid = pid;
        census.annotate_capture(&mut row, capture, ScannerHome::Unavailable);
        row.state
    }

    /// Resolve fixture paths through production code without scanning the user's root.
    fn resolved_test_roots(paths: &[&Path]) -> CaptureRoots { CaptureRoots::for_test(paths) }

    /// Different roots may name different logs and commands for the same shim pid.
    fn write_versioned_capture(
        root: &Path,
        pid: u32,
        generation: &str,
        directory: &str,
        home: &str,
        command: &str,
        output: &str,
    ) {
        let markers = root.join(CAPTURE_LIVE_RUNS_DIR);
        fs::create_dir_all(&markers).expect("registration directory");
        let log = format!("run-{generation}-{pid}.log");
        let record = [
            "cargo-tile-v2",
            generation,
            "boot",
            "100",
            &log,
            directory,
            home,
            "1",
            command,
            "",
        ]
        .join("\0");
        fs::write(markers.join(format!("{pid}.{generation}")), record)
            .expect("versioned registration");
        fs::write(root.join(log), output).expect("capture output");
    }

    #[test]
    fn capture_annotation_uses_one_root_for_progress_directory_and_command() {
        let first = tempdir().expect("first root");
        let second = tempdir().expect("second root");
        write_versioned_capture(
            first.path(),
            10,
            "first",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        write_versioned_capture(
            second.path(),
            10,
            "second",
            "/runner/project",
            "/runner",
            "test",
            "    Blocking waiting for file lock on build directory",
        );
        let record = directory_record("/writer");
        let stamp = match record.identity() {
            IdentityEvidence::Available(stamp) => Ok(stamp),
            IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
        }
        .expect("fixture supplies a complete birth");
        for (paths, directory, command, expected_path, expected_state) in [
            (
                [first.path(), second.path()],
                "/writer/project",
                "build",
                "~/project",
                CaptureLookup::Registered(CaptureRead::NoCurrentProgress),
            ),
            (
                [second.path(), first.path()],
                "/runner/project",
                "test",
                "/runner/project",
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
            ),
        ] {
            let roots = resolved_test_roots(&paths);
            let capture = Capture::take_roots(&roots, &|pid| {
                KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
            });
            let census = census_of(&[]);
            let key = capture.keys(10).next().expect("preferred root key");
            assert_eq!(capture.keys(10).count(), 2);
            assert_eq!(capture.confirmed().len(), 2);
            assert_eq!(
                census.captured_run(&capture, Pid::from_u32(10)),
                NearestRegistration::Registered(key)
            );

            let mut row = directory_row();
            row.path = "process directory".to_owned();
            row.directory_identity = WorkingDirectoryIdentity::Absolute(directory.into());
            row.command = CommandText::of("cargo", &[command]);
            census.annotate_capture(&mut row, &capture, ScannerHome::Known(Path::new("/writer")));

            assert_eq!(row.path, expected_path);
            assert_eq!(
                row.directory_identity,
                WorkingDirectoryIdentity::Absolute(directory.into())
            );
            assert_eq!(row.command, CommandText::of("cargo", &[command]));
            assert_eq!(row.state, expected_state);
        }
    }

    #[test]
    fn associated_root_names_proofs_suppressed_by_precedence_or_nearer_ancestry() {
        for (preferred_index, confirmed_pid) in [(0, 10), (1, 20)] {
            let preferred = capture_root(&[(10, "")]);
            let confirmed = tempdir().expect("confirmed root");
            write_versioned_capture(
                confirmed.path(),
                confirmed_pid,
                "verified",
                "/writer/project",
                "/writer",
                "build",
                "",
            );
            let record = directory_record("/writer");
            let stamp = match record.identity() {
                IdentityEvidence::Available(stamp) => Ok(stamp),
                IdentityEvidence::Unavailable => Err("fixture has no identity"),
            }
            .expect("complete fixture birth");
            let paths = if preferred_index == 0 {
                [preferred.path(), confirmed.path()]
            } else {
                [confirmed.path(), preferred.path()]
            };
            let roots = resolved_test_roots(&paths);
            let mut capture = Capture::take_roots(&roots, &|pid| {
                KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
            });
            let groups = vec![CargoGroup {
                lead:     directory_row(),
                rest:     Vec::new(),
                ancestry: Vec::new(),
            }];
            census_of(&[(10, 20)]).associate_status(&mut capture, &groups);
            let associations = &capture.root_status[preferred_index].associations;
            assert_eq!(associations.len(), 1);
            assert_eq!(associations[0].pid, 10);
            let AssociationSelection::Selected { key, proof, unused } = &associations[0].selection
            else {
                panic!("preferred root selects its reading");
            };
            assert_eq!(key.pid, 10);
            assert_eq!(*proof, SelectedProof::Unconfirmed);
            if confirmed_pid == 10 {
                assert_eq!(unused.len(), 1);
                assert_eq!(unused[0].reason, UnusedCaptureReason::SelectedUnconfirmed);
                assert_eq!(
                    unused[0].root,
                    confirmed.path().canonicalize().expect("root")
                );
            } else {
                assert_eq!(
                    capture.root_status[1 - preferred_index].associations.len(),
                    1
                );
            }
        }
    }

    #[test]
    fn an_unconfirmed_root_does_not_borrow_another_roots_confirmed_metadata() {
        let first = capture_root(&[(10, "")]);
        let second = tempdir().expect("confirmed root");
        write_versioned_capture(
            second.path(),
            10,
            "second",
            "/writer/project",
            "/custom",
            "test",
            "    Blocking waiting for file lock on build directory",
        );
        let record = directory_record("/writer");
        let stamp = match record.identity() {
            IdentityEvidence::Available(stamp) => Ok(stamp),
            IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
        }
        .expect("fixture supplies a complete birth");
        let roots = resolved_test_roots(&[first.path(), second.path()]);
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        assert_eq!(capture.keys(10).count(), 2);
        assert_eq!(capture.confirmed().len(), 1);
        assert_eq!(capture.confirmed()[0].key.root, CaptureRootIndex(1));
        let mut row = directory_row();
        row.path = "process directory".to_owned();
        census_of(&[]).annotate_capture(
            &mut row,
            &capture,
            ScannerHome::Known(Path::new("/writer")),
        );

        assert_eq!(row.path, "process directory");
        assert_eq!(
            row.directory_identity,
            WorkingDirectoryIdentity::Absolute("/writer/project".into())
        );
        assert_eq!(row.command, CommandText::of("cargo", &["build"]));
        assert_eq!(
            row.state,
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
    }

    #[test]
    fn a_nearer_capture_in_an_additional_root_precedes_the_own_root_ancestor() {
        let first = capture_root(&[(20, "    Blocking waiting for file lock on build directory")]);
        let second = capture_root(&[(10, "")]);
        let roots = resolved_test_roots(&[first.path(), second.path()]);
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let census = census_of(&[(11, 10), (10, 20)]);
        assert_eq!(
            census.captured_run(&capture, Pid::from_u32(11)),
            NearestRegistration::Registered(capture.keys(10).next().expect("nearest key")),
        );
        assert_eq!(
            captured_state(&census, &capture, 11),
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
    }

    #[test]
    fn repeated_scans_reuse_roots_resolved_before_an_ancestor_alias_changes() {
        let directory = tempdir().expect("fixture directory");
        let original = directory.path().join("original");
        let replacement = directory.path().join("replacement");
        let alias = directory.path().join("alias");
        let root = original.join("capture");
        let other = replacement.join("capture");
        let pid = std::process::id();
        write_versioned_capture(
            &root,
            pid,
            "first",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        write_versioned_capture(
            &other,
            pid,
            "other",
            "/runner/project",
            "/runner",
            "test",
            "",
        );
        symlink(&original, &alias).expect("original ancestor alias");
        let roots = resolved_test_roots(&[&alias.join("capture")]);
        let mut system = System::new();
        let mut smoothing = CpuSmoothing::default();
        scan(
            &mut system,
            &mut smoothing,
            Instant::now(),
            ScannerHome::Unavailable,
            &[],
            &roots,
        );
        assert!(
            !root
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join(format!("{pid}.first"))
                .exists()
        );

        fs::remove_file(&alias).expect("remove original ancestor alias");
        symlink(&replacement, &alias).expect("retarget ancestor alias");
        write_versioned_capture(
            &root,
            pid,
            "next",
            "/writer/project",
            "/writer",
            "build",
            "",
        );
        scan(
            &mut system,
            &mut smoothing,
            Instant::now(),
            ScannerHome::Unavailable,
            &[],
            &roots,
        );

        assert!(
            !root
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join(format!("{pid}.next"))
                .exists()
        );
        assert!(
            other
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join(format!("{pid}.other"))
                .exists()
        );
        assert_ne!(roots, resolved_test_roots(&[&alias.join("capture")]));
    }

    #[test]
    fn capture_annotation_changes_only_display_when_writer_home_differs() {
        for (home, display) in [("/writer", "~/project"), ("/custom", "/writer/project")] {
            let root = tempdir().expect("capture root");
            let markers = root.path().join(CAPTURE_LIVE_RUNS_DIR);
            fs::create_dir_all(&markers).expect("registration directory");
            fs::write(markers.join("10.generation"), directory_record_bytes(home))
                .expect("versioned registration");
            let record = directory_record(home);
            let stamp = match record.identity() {
                IdentityEvidence::Available(stamp) => Ok(stamp),
                IdentityEvidence::Unavailable => Err("fixture identity unavailable"),
            }
            .expect("fixture supplies a complete birth");
            let capture = Capture::take_from(root.path(), |pid| {
                KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
            });
            assert_eq!(capture.confirmed().len(), 1);
            let mut row = directory_row();
            let identity = row.directory_identity.clone();

            census_of(&[]).annotate_capture(
                &mut row,
                &capture,
                ScannerHome::Known(Path::new("/writer")),
            );

            assert_eq!(row.path, display);
            assert_eq!(row.directory_identity, identity);

            row.directory_identity = WorkingDirectoryIdentity::Absolute("/other/project".into());
            row.path = "~/project".to_owned();
            census_of(&[]).annotate_capture(
                &mut row,
                &capture,
                ScannerHome::Known(Path::new("/writer")),
            );
            assert_eq!(row.path, "~/project");
            assert_eq!(
                row.directory_identity,
                WorkingDirectoryIdentity::Absolute("/other/project".into())
            );
        }
    }

    #[test]
    fn registration_directory_shortens_only_the_same_writer_and_scanner_home() {
        let record = directory_record("/writer");
        assert_eq!(record.directory(), Path::new("/writer/project"));
        assert_eq!(
            registration_directory(&record, ScannerHome::Known(Path::new("/writer"))),
            "~/project"
        );
        assert_eq!(
            registration_directory(&record, ScannerHome::Known(Path::new("/other"))),
            "/writer/project"
        );
        assert_eq!(
            registration_directory(&record, ScannerHome::Unavailable),
            "/writer/project"
        );
        assert_eq!(
            registration_directory(
                &directory_record(""),
                ScannerHome::Known(Path::new("/writer"))
            ),
            "/writer/project"
        );
    }

    #[test]
    fn a_custom_writer_home_does_not_use_the_scanners_tilde_prefix() {
        assert_eq!(
            registration_directory(
                &directory_record("/custom"),
                ScannerHome::Known(Path::new("/writer"))
            ),
            "/writer/project"
        );
    }

    #[test]
    fn nearest_empty_capture_does_not_inherit_an_enclosing_progress_reading() {
        let root = capture_root(&[
            (10, ""),
            (20, "    Blocking waiting for file lock on build directory"),
        ]);
        let census = census_of(&[(11, 10), (10, 20)]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_eq!(
            captured_state(&census, &capture, 11),
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
    }

    #[test]
    fn nearest_unreadable_capture_does_not_inherit_an_enclosing_progress_reading() {
        let root = capture_root(&[
            (10, ""),
            (20, "    Blocking waiting for file lock on build directory"),
        ]);
        let log = root.path().join(format!(
            "{RUN_LOG_PREFIX}20260824-084300{PID_SEPARATOR}10{RUN_LOG_SUFFIX}"
        ));
        fs::remove_file(&log).expect("remove unreadable log placeholder");
        fs::create_dir(&log).expect("a directory cannot supply a log tail");
        let census = census_of(&[(11, 10), (10, 20)]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert!(matches!(
            captured_state(&census, &capture, 11),
            CaptureLookup::Registered(CaptureRead::Unreadable(_))
        ));
    }

    #[test]
    fn unavailable_argv_and_deliberately_excluded_rows_have_different_outcomes() {
        assert_eq!(cargo_split(&[]), Err(RowAbsence::Unavailable));
        let excluded = vec![OsString::from("cargo"), OsString::from("build")];
        assert_eq!(
            select_cargo(&excluded, &[String::from("build")]),
            Err(RowAbsence::Excluded)
        );
        assert!(matches!(
            cargo_split(&excluded),
            Ok(CargoArguments {
                layout: ArgumentLayout::Cargo,
                ..
            })
        ));
        assert!(
            matches!(cargo_split(&[OsString::from("cargo-nextest"), OsString::from("run")]), Ok(CargoArguments { layout: ArgumentLayout::External(name), .. }) if name == "nextest")
        );
    }

    #[test]
    fn scanner_process_refreshes_exclude_tasks() {
        assert!(!process_discovery_refresh_kind().tasks());
        assert!(!process_detail_refresh_kind().tasks());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scanner_discovery_excludes_linux_tasks() -> std::io::Result<()> {
        let process_pid = std::process::id();
        let task_pids = fs::read_dir(format!("/proc/{process_pid}/task"))?
            .filter_map(Result::ok)
            .filter_map(|entry| entry.file_name().to_string_lossy().parse::<u32>().ok())
            .filter(|pid| *pid != process_pid)
            .map(Pid::from_u32)
            .collect::<HashSet<_>>();
        let mut system = System::new();

        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_discovery_refresh_kind(),
        );

        let discovered_pids = system.processes().keys().copied().collect::<HashSet<_>>();
        assert!(task_pids.is_disjoint(&discovered_pids));
        assert!(
            system
                .process(Pid::from_u32(process_pid))
                .is_some_and(|process| process.tasks().is_none())
        );
        Ok(())
    }

    /// A cell names what launched the command, so the walk has to
    /// reach past the shell to whatever started that.
    #[test]
    fn the_chain_above_a_command_reads_outermost_first() {
        // cargo (64432) under a shell (12445) under a login (12444)
        // under the editor that opened it (6218), which launchd owns.
        let census = census_of(&[(64432, 12445), (12445, 12444), (12444, 6218), (6218, 1)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![
                Pid::from_u32(6218),
                Pid::from_u32(12444),
                Pid::from_u32(12445),
            ],
        );
    }

    /// Every command on the machine descends from the init process, so
    /// a row naming it tells one command from no other.
    #[test]
    fn the_walk_stops_short_of_the_process_the_tree_roots_at() {
        let census = census_of(&[(64432, 12445), (12445, 1)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![Pid::from_u32(12445)],
        );
    }

    /// A reparented chain that comes back round on itself must end the
    /// walk rather than spin it.
    #[test]
    fn a_chain_that_loops_ends_where_it_repeats() {
        let census = census_of(&[(64432, 900), (900, 901), (901, 900)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            vec![Pid::from_u32(901), Pid::from_u32(900)],
        );
    }

    /// A shell or login process passed a command through rather than
    /// starting it, and is marked so the cell can decide whether to
    /// draw it.
    #[test]
    fn a_shell_and_a_login_are_marked_as_passing_through() {
        for name in ["zsh", "bash", "sh", "login"] {
            assert!(is_transparent(OsStr::new(name)), "{name}");
        }
    }

    #[test]
    fn what_started_a_command_is_never_marked() {
        for name in ["zed", "iTerm2", "node", "cargo-mend"] {
            assert!(!is_transparent(OsStr::new(name)), "{name}");
        }
    }

    /// A nested cargo waits on the build-directory lock in its own
    /// right, and nothing above it can say which invocation is the one
    /// waiting -- so the row has to carry it.
    #[test]
    fn an_invocation_under_the_lead_reports_its_own_wait() {
        // `cargo doc` (76847) under a shim (76846) the lead (64432)
        // started, which is how a manager's nested cargo is captured.
        let root = capture_root(&[(
            76846,
            "    Blocking waiting for file lock on build directory",
        )]);
        let census = census_of(&[(76847, 76846), (76846, 64432)]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });

        assert_eq!(
            captured_state(&census, &capture, 76847),
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
        );
    }

    /// A cargo the enclosing run started has no capture of its own --
    /// the shim declines to open a second one inside a run it is
    /// already capturing. The enclosing one is still its own reading:
    /// the wait it prints about is mirrored into that log because it is
    /// the process doing the waiting.
    #[test]
    fn a_nested_invocation_reads_the_run_it_is_inside() {
        let root = capture_root(&[(
            64431,
            "    Blocking waiting for file lock on build directory",
        )]);
        let census = census_of(&[(76847, 64432), (64432, 64431)]);
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });

        assert_eq!(
            captured_state(&census, &capture, 76847),
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
        );
        assert_eq!(
            captured_state(&census, &capture, 64432),
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
            "the lead still reads its own shim",
        );
    }

    /// The list as it reaches [`CommandText::is_hidden_when_idle`] once
    /// the config has turned it into owned strings.
    fn hidden_when_idle() -> Vec<String> {
        DEFAULT_HIDDEN_WHEN_IDLE
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    #[test]
    fn the_subcommand_is_the_first_argument_past_a_toolchain_selector() {
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", "build"]).subcommand(),
            Some("build")
        );
        assert_eq!(
            CommandText::of(CARGO_DISPLAY_NAME, &["build"]).subcommand(),
            Some("build")
        );
    }

    #[test]
    fn a_subcommand_on_the_list_is_recognised_past_a_toolchain_selector() {
        let selected = CommandText::of(CARGO_DISPLAY_NAME, &["+nightly", SIBLING_SUBCOMMAND_NAME]);
        assert!(selected.is_hidden_when_idle(&hidden_when_idle()));
    }

    #[test]
    fn a_subcommand_off_the_list_is_not_hidden() {
        let building = CommandText::of(CARGO_DISPLAY_NAME, &["build"]);
        assert!(!building.is_hidden_when_idle(&hidden_when_idle()));
    }

    #[test]
    fn duration_stays_minutes_and_seconds_under_an_hour() {
        assert_eq!(duration_label(57), "00:57");
        assert_eq!(duration_label(3599), "59:59");
    }

    #[test]
    fn duration_widens_to_hours_once_pathological() {
        assert_eq!(duration_label(3600), "01:00:00");
        assert_eq!(duration_label(45_296), "12:34:56");
    }

    #[test]
    fn home_prefix_collapses_to_tilde() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/Users/someone/rust/project");
        assert_eq!(
            home_relative(&path, ScannerHome::Known(&home)),
            "~/rust/project"
        );
    }

    #[test]
    fn home_itself_renders_as_bare_tilde() {
        let home = PathBuf::from("/Users/someone");
        assert_eq!(home_relative(&home, ScannerHome::Known(&home)), "~");
    }

    #[test]
    fn path_outside_home_is_left_alone() {
        let home = PathBuf::from("/Users/someone");
        let path = PathBuf::from("/opt/build");
        assert_eq!(
            home_relative(&path, ScannerHome::Known(&home)),
            "/opt/build"
        );
    }

    #[test]
    fn command_splits_program_from_arguments() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("build"),
            OsString::from("--release"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.line(SummaryDetail::Full), "build --release");
    }

    #[test]
    fn a_shim_caught_before_handoff_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/bin/zsh"),
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo"),
            OsString::from("check"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.program, "cargo");
        assert_eq!(text.line(SummaryDetail::Full), "check --all-targets");
    }

    #[test]
    fn a_wrapped_cargo_still_reads_as_cargo() {
        let argv = vec![
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/cargo-tile-real"),
            OsString::from("build"),
        ];
        assert_eq!(
            command_text(&argv, ScannerHome::Unavailable)
                .expect("argv names a cargo binary")
                .program,
            "cargo"
        );
    }

    /// The list as it reaches [`is_excluded`] once the config has
    /// turned it into owned strings.
    fn excluded() -> Vec<String> {
        DEFAULT_EXCLUDED
            .iter()
            .map(|subcommand| (*subcommand).to_string())
            .collect()
    }

    #[test]
    fn an_excluded_subcommand_run_through_cargo_is_dropped() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("claim"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    /// The same command reached as its own binary, which is how a hook
    /// with the path already resolved runs it. Keying the list on the
    /// subcommand rather than the binary is what makes one entry cover
    /// both.
    #[test]
    fn an_excluded_subcommand_run_as_its_own_binary_is_dropped() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo-berth"),
            OsString::from("claim"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    #[test]
    fn an_excluded_subcommand_is_dropped_past_a_toolchain_selector() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("+nightly"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("drift"),
        ];
        assert!(is_excluded(&argv, &excluded()));
    }

    #[test]
    fn a_subcommand_off_the_list_is_kept() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("build"),
            OsString::from("--release"),
        ];
        assert!(!is_excluded(&argv, &excluded()));
    }

    /// An emptied list is the setting turned off, not a list that
    /// matches everything.
    #[test]
    fn an_empty_list_excludes_nothing() {
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from(COORDINATION_SUBCOMMAND_NAME),
            OsString::from("renew"),
        ];
        assert!(!is_excluded(&argv, &[]));
    }

    /// macOS can report an `sccache` with the name of the cargo that
    /// spawned it, which is how one reaches [`command_text`] at all.
    #[test]
    fn a_compiler_wrapper_wearing_cargos_name_is_not_a_cargo_command() {
        let argv = vec![
            OsString::from("sccache"),
            OsString::from("/Users/someone/.rustup/toolchains/stable/bin/rustc"),
            OsString::from("--crate-name"),
            OsString::from("bevy_transform"),
        ];
        assert!(command_text(&argv, ScannerHome::Unavailable).is_err());
    }

    #[test]
    fn the_summary_drops_the_manifest_path_and_the_flag_naming_it() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path"),
            OsString::from("/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "check --all-targets");
    }

    #[test]
    fn the_summary_drops_a_manifest_path_written_as_one_word() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path=/opt/project/Cargo.toml"),
            OsString::from("--all-targets"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "check --all-targets");
    }

    /// What is being built is what the row is there to say: which
    /// member of a workspace, and how much of it.
    #[test]
    fn the_summary_keeps_what_names_the_work() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("mend"),
            OsString::from("--all-targets"),
            OsString::from("-p"),
            OsString::from("hana_clerestory"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(
            text.line(SummaryDetail::Trimmed),
            "mend --all-targets -p hana_clerestory"
        );
    }

    /// A rendering flag says how the caller wanted the output, which is
    /// the caller's business rather than the run's -- in either
    /// spelling, and wherever in the line it falls.
    #[test]
    fn the_summary_drops_the_rendering_flags() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("--color=auto"),
            OsString::from("test"),
            OsString::from("--no-run"),
            OsString::from("--message-format"),
            OsString::from("json-render-diagnostics"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(text.line(SummaryDetail::Trimmed), "test --no-run");
    }

    /// Past a bare `--` the arguments are the other program's. It
    /// spells its flags however it likes, and none of them are cargo's
    /// to drop -- a `--color` there is the other program's setting.
    #[test]
    fn the_summary_keeps_everything_handed_to_another_program() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("clippy"),
            OsString::from("--color"),
            OsString::from("never"),
            OsString::from("--"),
            OsString::from("-D"),
            OsString::from("warnings"),
            OsString::from("--color"),
            OsString::from("always"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(
            text.line(SummaryDetail::Trimmed),
            "clippy -- -D warnings --color always"
        );
    }

    /// A command's own cell shows the line as it was typed, however
    /// much of it the summary leaves out.
    #[test]
    fn a_cell_of_its_own_keeps_the_whole_line() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("build"),
            OsString::from("--bin"),
            OsString::from("hana"),
            OsString::from("--message-format=json"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(
            text.line(SummaryDetail::Full),
            "build --bin hana --message-format=json"
        );
        assert_eq!(text.line(SummaryDetail::Trimmed), "build --bin hana");
    }

    /// The flag is matched whole: an argument that merely starts the
    /// same way names something else and stays.
    #[test]
    fn an_argument_that_only_starts_like_the_manifest_flag_stays() {
        let argv = vec![
            OsString::from("cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path-of-record"),
        ];
        let text =
            command_text(&argv, ScannerHome::Unavailable).expect("argv names a cargo binary");
        assert_eq!(
            text.line(SummaryDetail::Trimmed),
            "check --manifest-path-of-record"
        );
    }

    #[test]
    fn arguments_collapse_the_home_prefix() {
        let home = PathBuf::from("/Users/someone");
        let argv = vec![
            OsString::from("/Users/someone/.cargo/bin/cargo"),
            OsString::from("check"),
            OsString::from("--manifest-path"),
            OsString::from("/Users/someone/rust/project/Cargo.toml"),
        ];
        let text =
            command_text(&argv, ScannerHome::Known(&home)).expect("argv names a cargo binary");
        assert_eq!(
            text.line(SummaryDetail::Full),
            "check --manifest-path ~/rust/project/Cargo.toml"
        );
    }

    /// The scale is `top`'s, so a build across several cores reads past
    /// 100% rather than being folded back into a share of the machine.
    #[test]
    fn a_cpu_share_reads_as_a_whole_number_of_percent() {
        assert_eq!(cpu_label(0.0), "0%");
        assert_eq!(cpu_label(12.4), "12%");
        assert_eq!(cpu_label(12.6), "13%");
        assert_eq!(cpu_label(783.2), "783%");
    }

    /// A share the platform could only report as a rounding artefact
    /// still has to read as idle rather than as a negative percent.
    #[test]
    fn a_share_below_nought_reads_as_nought() {
        assert_eq!(cpu_label(-0.4), "0%");
    }

    /// Every group member contributes explicitly, including measured zero.
    #[test]
    fn a_group_adds_up_the_shares_of_everything_under_it() {
        let shares = HashMap::from([
            (Pid::from(1), Measurement::Reading(90.4)),
            (Pid::from(2), Measurement::Reading(300.2)),
            (Pid::from(3), Measurement::Reading(0.0)),
        ]);
        let members = [1, 2, 3].into_iter().map(Pid::from);

        assert_eq!(
            aggregate_cpu(&shares, members).map(cpu_label).to_string(),
            "391%"
        );
    }

    /// Missing entries cannot silently subtract a contributor from the total.
    #[test]
    fn a_group_with_a_missing_share_is_unavailable() {
        assert_eq!(
            aggregate_cpu(&HashMap::new(), std::iter::once(Pid::from(1))),
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        );
    }

    #[test]
    fn a_group_of_measured_zero_shares_reads_as_idle() {
        let pid = Pid::from(1);
        let shares = HashMap::from([(pid, Measurement::Reading(0.0))]);
        assert_eq!(
            aggregate_cpu(&shares, std::iter::once(pid))
                .map(cpu_label)
                .to_string(),
            "0%",
        );
    }

    #[test]
    fn an_unknown_cpu_contributor_makes_the_group_total_unknown() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let shares = HashMap::from([
                (Pid::from(1), Measurement::Reading(100.0)),
                (Pid::from(2), Measurement::Unavailable(reason)),
            ]);
            for members in [[Pid::from(1), Pid::from(2)], [Pid::from(2), Pid::from(1)]] {
                assert_eq!(
                    aggregate_cpu(&shares, members.into_iter()),
                    Measurement::Unavailable(reason)
                );
            }
        }
    }

    #[test]
    fn an_unknown_descendant_invalidates_its_owning_cargos_cpu() {
        let owner = Pid::from(1);
        let descendant = Pid::from(2);
        let mut census = census_of(&[(2, 1)]);
        census.cargo.push(owner);
        census.cpu = HashMap::from([
            (owner, Measurement::Reading(10.0)),
            (
                descendant,
                Measurement::Unavailable(MeasurementAbsence::Unproven),
            ),
        ]);
        assert_eq!(
            census.attribute_cpu()[&owner],
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn unknown_compiler_contributors_invalidate_the_group_tally() {
        let first = Pid::from(1);
        let second = Pid::from(2);
        for unknown in [
            HashMap::new(),
            HashMap::from([(second, CompilerObservation::Unknown)]),
        ] {
            let mut counts = unknown;
            counts.insert(
                first,
                CompilerObservation::Running(Compiler {
                    name:  "rustc",
                    count: 2,
                }),
            );
            assert_eq!(
                aggregate_compilers(&counts, [first, second].into_iter()),
                CompilerObservation::Unknown
            );
        }
    }

    #[test]
    fn compiler_totals_preserve_measured_absence_and_driver_priority() {
        let first = Pid::from(1);
        let second = Pid::from(2);
        let third = Pid::from(3);
        let mut counts = HashMap::from([
            (first, CompilerObservation::None),
            (second, CompilerObservation::None),
        ]);
        assert_eq!(
            aggregate_compilers(&counts, [first, second].into_iter()),
            CompilerObservation::None
        );
        counts.insert(
            first,
            CompilerObservation::Running(Compiler {
                name:  "rustc",
                count: 9,
            }),
        );
        counts.insert(
            second,
            CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 2,
            }),
        );
        counts.insert(
            third,
            CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 3,
            }),
        );
        assert_eq!(
            aggregate_compilers(&counts, [first, second, third].into_iter()),
            CompilerObservation::Running(Compiler {
                name:  "sccache",
                count: 5,
            })
        );
    }

    /// Same process identity for CPU boundary fixtures; only the counter changes.
    fn cpu_baseline(accumulated: u64) -> CpuBaseline {
        CpuBaseline {
            lifetime: LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(1)),
            accumulated,
        }
    }

    #[test]
    fn collection_names_the_first_observation_before_publishing_a_rate() {
        assert_eq!(
            Census::measure_cpu(Pid::from(1), 0.0, &cpu_baseline(10), &HashMap::new()),
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
    }

    #[test]
    fn collection_keeps_a_regressing_counter_unproven() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        for cpu in [0.0, 50.0] {
            assert_eq!(
                Census::measure_cpu(pid, cpu, &cpu_baseline(0), &previous),
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            );
        }
    }

    #[test]
    fn collection_keeps_consecutive_failed_reads_and_recovery_unproven() {
        let pid = Pid::from(1);
        let mut previous = HashMap::from([(pid, cpu_baseline(10))]);
        // Failed task-info reads keep the old rate. The first successful read
        // after them also keeps it because sysinfo's previous counter was zero.
        for accumulated in [0, 0, 0, 20] {
            let baseline = cpu_baseline(accumulated);
            assert_eq!(
                Census::measure_cpu(pid, 0.4, &baseline, &previous),
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            );
            previous.insert(pid, baseline);
        }
        assert_eq!(
            Census::measure_cpu(pid, 0.5, &cpu_baseline(30), &previous),
            Measurement::Reading(0.5)
        );
    }

    #[test]
    fn collection_waits_for_a_positive_baseline_after_an_initial_failed_read() {
        let pid = Pid::from(1);
        let mut previous = HashMap::new();
        let initial = cpu_baseline(0);
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &initial, &previous),
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        previous.insert(pid, initial);
        let recovered = cpu_baseline(20);
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &recovered, &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        previous.insert(pid, recovered.clone());
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &recovered, &previous),
            Measurement::Reading(0.0)
        );
    }

    #[test]
    fn collection_preserves_a_measured_zero_with_nonzero_accumulated_time() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        let measured = Census::measure_cpu(pid, 0.0, &cpu_baseline(10), &previous);
        assert_eq!(measured, Measurement::Reading(0.0));
        assert_eq!(measured.map(cpu_label).to_string(), "0%");
    }

    #[test]
    fn collection_keeps_a_retained_rate_unproven_when_cpu_time_is_unchanged() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        assert_eq!(
            Census::measure_cpu(pid, 80.0, &cpu_baseline(10), &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn readable_quantized_zero_is_unproven_instead_of_failed() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(0))]);
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &cpu_baseline(0), &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        // A nonzero rate cannot establish a fresh computation from a zero baseline.
        assert_eq!(
            Census::measure_cpu(pid, 0.5, &cpu_baseline(0), &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn collection_rejects_invalid_rates() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        for cpu in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0] {
            assert_eq!(
                Census::measure_cpu(pid, cpu, &cpu_baseline(10), &previous),
                Measurement::Unavailable(MeasurementAbsence::ReadFailed)
            );
        }
    }

    #[test]
    fn a_reused_pid_starts_a_new_cpu_observation() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        let replacement = CpuBaseline {
            lifetime:    LifetimeEvidence::Available(birth_stamp::ProcessLifetime::for_test(2)),
            accumulated: 0,
        };
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &replacement, &previous),
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
    }

    #[test]
    fn a_counter_regression_with_unchanged_native_lifetime_remains_unproven() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        let replacement = cpu_baseline(5);
        assert_eq!(
            Census::measure_cpu(pid, 0.0, &replacement, &previous),
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn census_keeps_first_observations_instead_of_dropping_zero_samples() {
        let pid = Pid::from_u32(std::process::id());
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            process_discovery_refresh_kind(),
        );
        let census = Census::take(&system, &HashMap::new());
        assert_eq!(
            census.cpu.get(&pid),
            Some(&Measurement::Unavailable(
                MeasurementAbsence::FirstObservation
            ))
        );
    }

    /// A clock the settling tests step forward by hand.
    fn start() -> Instant { Instant::now() }

    /// One scan's worth of the poll interval.
    fn poll() -> Duration { Duration::from_millis(PROCESS_POLL_MILLIS) }

    /// One invocation's reading once `sampled` has been folded in at
    /// `now`.
    fn settle_one(smoothing: &mut CpuSmoothing, sampled: f32, now: Instant) -> f32 {
        let pid = InvocationId::for_test(1);
        match smoothing.settle(
            &HashMap::from([(pid.clone(), Measurement::Reading(sampled))]),
            std::slice::from_ref(&pid),
            now,
        )[&pid]
        {
            Measurement::Reading(reading) => Ok(reading),
            Measurement::Unavailable(reason) => Err(reason),
        }
        .expect("supplied reading must remain available")
    }

    /// One invocation settled at `sampled` for `over`, reporting where
    /// the table's reading stood at the end of it.
    fn settle_over(
        smoothing: &mut CpuSmoothing,
        sampled: f32,
        from: Instant,
        over: Duration,
    ) -> f32 {
        let mut elapsed = Duration::ZERO;
        let mut reading = 0.0;
        while elapsed < over {
            elapsed += poll();
            reading = settle_one(smoothing, sampled, from + elapsed);
        }
        reading
    }

    /// A command that starts busy is reported busy rather than drawn
    /// climbing to what its first sample already said.
    #[test]
    fn the_first_sample_of_an_invocation_is_taken_whole() {
        let opening = settle_one(&mut CpuSmoothing::default(), 400.0, start());

        assert!(
            (opening - 400.0).abs() < f32::EPSILON,
            "opened at {opening} rather than at its own sample"
        );
    }

    /// A burst lands as a step toward itself, not as the whole of it:
    /// one scan of a command that works in bursts is mostly artefact.
    #[test]
    fn a_sample_that_jumps_is_taken_a_step_at_a_time() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);

        // Past the report interval so what comes back is this scan's
        // settled reading rather than the one being held.
        let stepped = settle_one(
            &mut smoothing,
            100.0,
            now + Duration::from_millis(CPU_REPORT_MILLIS),
        );

        assert!(stepped > 0.0, "the burst moved the reading");
        assert!(stepped < 100.0, "but not the whole way to it: {stepped}");
    }

    /// Held long enough, a steady share is what the column settles on --
    /// the smoothing is a delay, not a ceiling.
    #[test]
    fn a_share_held_steady_is_arrived_at() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);

        // Four windows of the climb, by which point a reading settled
        // this way stands within two percent of what it is climbing to.
        let over = Duration::from_secs_f32(CPU_SMOOTHING_SECONDS * 4.0);

        assert_eq!(
            cpu_label(settle_over(&mut smoothing, 100.0, now, over)),
            "98%"
        );
    }

    /// The reading behind the column moves on every scan; the column
    /// itself is only allowed to say something new once a second, so a
    /// smooth figure is not redrawn faster than it can be read.
    #[test]
    fn the_table_holds_a_reading_for_the_whole_report_interval() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);
        let held = settle_one(&mut smoothing, 100.0, now + poll());

        assert!(
            held.abs() < f32::EPSILON,
            "the opening reading was still being held, not {held}"
        );

        let refreshed = settle_one(
            &mut smoothing,
            100.0,
            now + Duration::from_millis(CPU_REPORT_MILLIS),
        );

        assert!(refreshed > 0.0, "the second brought the climb through");
    }

    /// A command that starts partway through somebody else's second is
    /// reported straight away rather than drawn idle until it ends.
    #[test]
    fn an_invocation_that_arrives_mid_second_reports_at_once() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        let (running, arriving) = (InvocationId::for_test(1), InvocationId::for_test(2));
        smoothing.settle(
            &HashMap::from([(running.clone(), Measurement::Reading(10.0))]),
            std::slice::from_ref(&running),
            now,
        );

        let reported = smoothing.settle(
            &HashMap::from([
                (running.clone(), Measurement::Reading(10.0)),
                (arriving.clone(), Measurement::Reading(400.0)),
            ]),
            &[running, arriving.clone()],
            now + poll(),
        );

        assert_eq!(
            reported.get(&arriving).copied(),
            Some(Measurement::Reading(400.0))
        );
    }

    /// An invocation the scan no longer carries takes its history with
    /// it, so a pid handed out again opens fresh.
    #[test]
    fn an_invocation_that_ends_is_let_go_of() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        settle_one(&mut smoothing, 400.0, now);
        smoothing.settle(&HashMap::new(), &[], now + poll());

        assert!(smoothing.settled.is_empty());
        assert!(smoothing.reported.is_empty());
    }

    #[test]
    fn a_never_published_smoother_is_distinct_from_one_holding_a_reading() {
        let mut smoothing = CpuSmoothing::default();
        assert_eq!(smoothing.publication, CpuPublication::NeverPublished);
        let now = start();
        settle_one(&mut smoothing, 40.0, now);
        assert_eq!(smoothing.publication, CpuPublication::Published(now));
        assert_eq!(
            smoothing.reported[&InvocationId::for_test(1)],
            Measurement::Reading(40.0)
        );
    }

    #[test]
    fn unavailable_cpu_replaces_a_stale_reading_before_its_publication_deadline() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let mut smoothing = CpuSmoothing::default();
            let now = start();
            let pid = InvocationId::for_test(1);
            settle_one(&mut smoothing, 400.0, now);
            assert!(!smoothing.is_due(now + poll()));
            let reported = smoothing.settle(
                &HashMap::from([(pid.clone(), Measurement::Unavailable(reason))]),
                std::slice::from_ref(&pid),
                now + poll(),
            );
            assert_eq!(reported[&pid], Measurement::Unavailable(reason));
            assert_eq!(smoothing.settled[&pid], Measurement::Unavailable(reason));
            assert_eq!(smoothing.publication, CpuPublication::Published(now));
            let later = smoothing.settle(
                &HashMap::from([(pid.clone(), Measurement::Unavailable(reason))]),
                std::slice::from_ref(&pid),
                now + Duration::from_millis(CPU_REPORT_MILLIS),
            );
            assert_eq!(later[&pid], Measurement::Unavailable(reason));
        }
    }

    #[test]
    fn a_missing_cpu_sample_never_becomes_a_measured_zero() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        let pid = InvocationId::for_test(1);
        settle_one(&mut smoothing, 400.0, now);
        let reported = smoothing.settle(&HashMap::new(), std::slice::from_ref(&pid), now + poll());
        assert_eq!(
            reported[&pid],
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
    }

    #[test]
    fn cpu_recovery_starts_fresh_without_the_value_from_before_the_gap() {
        let mut smoothing = CpuSmoothing::default();
        let now = start();
        let pid = InvocationId::for_test(1);
        settle_one(&mut smoothing, 400.0, now);
        smoothing.settle(
            &HashMap::from([(
                pid.clone(),
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            )]),
            std::slice::from_ref(&pid),
            now + poll(),
        );
        let recovered = settle_one(&mut smoothing, 20.0, now + poll() * 2);
        assert!((recovered - 20.0).abs() < f32::EPSILON);
    }

    /// The short display keeps the words that name what runs and stops
    /// at the first argument. A manifest path is what makes these rows
    /// unreadable -- every one of a test suite's cases carries a
    /// different temporary directory, and none of them says anything the
    /// row's own pid does not.
    #[test]
    fn a_named_command_stops_at_its_first_argument() {
        let mend = CommandText::of(
            "cargo",
            &[
                "mend",
                "--manifest-path",
                "/var/folders/T/x/Cargo.toml",
                "--json",
            ],
        );

        assert_eq!(mend.named(), "mend");
        assert!(
            mend.line(SummaryDetail::Full).contains("--json"),
            "and the long line still carries every one of them",
        );
    }

    /// A subcommand of a subcommand is still the name of what runs, so
    /// `nextest run` keeps both words -- and the toolchain selector
    /// keeps its place ahead of them, `+nightly fmt` saying something
    /// `fmt` alone does not.
    #[test]
    fn a_named_command_keeps_its_subcommands_and_its_toolchain() {
        assert_eq!(
            CommandText::of("cargo", &["nextest", "run", "--workspace"]).named(),
            "nextest run",
        );
        assert_eq!(
            CommandText::of("cargo", &["+nightly", "fmt", "--all"]).named(),
            "+nightly fmt",
        );
    }

    /// A chain step is held as one line rather than split, and its
    /// program is often reached by its path -- so the first word stands
    /// however it is spelled, and only what follows is read for
    /// arguments. A bare `node` would say less than the row it heads.
    #[test]
    fn a_named_chain_step_keeps_the_path_it_was_reached_by() {
        assert_eq!(
            command_name("~/.claude/local/claude --setting on --setting off"),
            "~/.claude/local/claude",
        );
        assert_eq!(command_name("zsh -c cargo nextest run"), "zsh");
        assert_eq!(command_name("zed"), "zed");
        assert_eq!(command_name(""), "");
    }
}
