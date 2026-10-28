//! Process discovery, attribution, and assembly of invocation groups.

use std::borrow::Cow;
use std::cell::Cell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::collections::hash_map::Entry;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
#[cfg(target_os = "linux")]
use std::io::ErrorKind;
use std::ops::Add;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::sync::RwLock;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::SyncSender;
use std::sync::mpsc::TrySendError;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use chrono::DateTime;
use chrono::Local;
use rustix::io::Errno;
use rustix::process::Pid as KernelPid;
use rustix::process::test_kill_process;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessStatus;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;
use tui_pane::CellDepartureReceipt;
use tui_pane::EndedCellDeparture;
use tui_pane::PaintedRowSpan;
use tui_pane::kernel_parent;

use super::command_text;
use super::command_text::CommandText;
use super::command_text::RowAbsence;
use super::command_text::ScannerHome;
#[cfg(test)]
use super::cpu_observation;
#[cfg(test)]
use super::cpu_observation::CpuScanPublication;
use super::direct_capture::DirectAssociation;
use super::direct_capture::DirectCapture;
use super::direct_capture::NearestRegistration;
use super::direct_capture::SelectedProof;
use super::invocation_cpu_accounting;
use super::invocation_cpu_accounting::CensusCadence;
use super::invocation_cpu_accounting::CompileOwner;
use super::invocation_cpu_accounting::CompilePathAbsence;
use super::invocation_cpu_accounting::CompilerCreditRetention;
use super::invocation_cpu_accounting::CpuAssignment;
use super::invocation_cpu_accounting::CpuBaseline;
use super::invocation_cpu_accounting::InvocationCpuAccounting;
use super::invocation_cpu_accounting::InvocationCpuContributions;
use super::invocation_cpu_accounting::InvocationMeasurements;
use super::invocation_cpu_accounting::Measurement;
use super::invocation_cpu_accounting::MeasurementAbsence;
use super::process_identity;
use super::process_identity::CaptureMembership;
use super::process_identity::DispatchedProcess;
use super::process_identity::InvocationId;
use super::process_identity::LifetimeMatch;
use super::process_identity::ProcessIdentities;
use super::process_identity::ProcessIdentity;
use super::process_identity::RunId;
use super::process_identity::TerminalIdentities;
use super::process_identity::VisibleParent;
use crate::birth_stamp;
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::LifetimeEvidence;
#[cfg(test)]
use crate::birth_stamp::Observation;
#[cfg(test)]
use crate::birth_stamp::ProcessLifetime;
use crate::birth_stamp::Verification;
use crate::constants::ARGUMENT_SEPARATOR;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_PROC_DIRECTORY;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_COMM_SEPARATOR;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_FILENAME;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_ZOMBIE_STATE;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_JSON_FORMAT_PREFIX;
use crate::constants::CARGO_MESSAGE_FORMAT_FLAG;
use crate::constants::CARGO_MESSAGE_FORMAT_JSON_PREFIX;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_QUIET_FLAGS;
use crate::constants::CARGO_SUBCOMMAND_PREFIX;
#[cfg(test)]
use crate::constants::CARGO_TARGET_DIR_FLAG;
use crate::constants::CENSUS_ATTRIBUTION_THREAD_NAME;
use crate::constants::CENSUS_CAPTURE_THREAD_NAME;
use crate::constants::CENSUS_DETAIL_READER_COUNT;
use crate::constants::CENSUS_DETAIL_THREAD_PREFIX;
use crate::constants::CENSUS_REFRESH_THREAD_NAME;
use crate::constants::CENSUS_THREAD_NAME;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::DIRECT_CARGO_ARGUMENTS_START;
use crate::constants::MISSING_SCAN_LIMIT;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::RUSTC_BINARY;
use crate::constants::RUSTC_OUT_DIR_FLAG;
use crate::constants::SCCACHE_BINARY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::SHELL_ESCAPED_SINGLE_QUOTE;
use crate::constants::SHELL_QUOTED_ARGUMENT_END;
use crate::constants::SHELL_SINGLE_QUOTE;
use crate::constants::START_TIME_FORMAT;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;
use crate::constants::WRAPPED_CARGO_ARGUMENTS_START;
use crate::probe;
use crate::progress::capture::Capture;
use crate::progress::capture::CaptureKey;
use crate::progress::capture::CaptureSelection;
use crate::progress::capture_read::CaptureLookup;
use crate::progress::capture_roots::CaptureRoots;
use crate::registration::RegistrationCandidate;
use crate::registration::WorkingDirectoryIdentity;
use crate::registration::WriterHome;
use crate::render::Account;
use crate::render::CaptureContext;
use crate::sccache::SccacheServer;
use crate::settings::AssociationSelection;
use crate::settings::CaptureAssociation;
use crate::settings::UnusedCapture;
use crate::settings::UnusedCaptureReason;
use crate::terminal::Scan;

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

/// Disjoint invocation work: its process and descendants assigned to no other invocation.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct InvocationResourceUse {
    /// Core percentage remains numeric until a table formats its final contributors.
    pub(crate) cpu:      Measurement<f32>,
    /// Resident bytes include only this invocation's attributed processes.
    pub(crate) memory:   Measurement<u64>,
    /// Compiler counts retain unavailable observations and driver priority.
    pub(crate) compiler: CompilerObservation,
}

impl InvocationResourceUse {
    /// Missing evidence prevents publishing a partial contribution.
    const fn unavailable(reason: MeasurementAbsence) -> Self {
        Self {
            cpu:      Measurement::Unavailable(reason),
            memory:   Measurement::Unavailable(reason),
            compiler: CompilerObservation::Unknown,
        }
    }

    /// Numeric contributors are formatted only after the table selects its rows.
    pub(crate) fn sum(contributions: impl IntoIterator<Item = Self>) -> Self {
        let mut total = Self {
            cpu:      Measurement::Reading(0.0),
            memory:   Measurement::Reading(0),
            compiler: CompilerObservation::None,
        };
        for contribution in contributions {
            total.cpu = total.cpu + contribution.cpu;
            total.memory = total.memory + contribution.memory;
            total.compiler = sum_compilers([total.compiler, contribution.compiler]);
        }
        total
    }

    /// Another invocation of the same pid can supply a missing observation.
    fn retain_readings(&mut self, other: Self) {
        if matches!(self.cpu, Measurement::Unavailable(_))
            && matches!(other.cpu, Measurement::Reading(_))
        {
            self.cpu = other.cpu;
        }
        if matches!(self.memory, Measurement::Unavailable(_))
            && matches!(other.memory, Measurement::Reading(_))
        {
            self.memory = other.memory;
        }
        if self.compiler == CompilerObservation::Unknown
            && other.compiler != CompilerObservation::Unknown
        {
            self.compiler = other.compiler;
        }
    }

    /// Each attributed bucket is disjoint from every nested cargo invocation.
    fn observed(attributed: &InvocationMeasurements, pid: Pid) -> Self {
        Self {
            cpu:      aggregate(&attributed.cpu, std::iter::once(pid)),
            memory:   aggregate(&attributed.memory, std::iter::once(pid)),
            compiler: attributed
                .compilers
                .get(&pid)
                .cloned()
                .unwrap_or(CompilerObservation::Unknown),
        }
    }
}

/// Each pid contributes once, using any reading supplied by its invocations.
pub(crate) fn sum_resource_use_once_per_pid(
    contributions: impl IntoIterator<Item = (Pid, InvocationResourceUse)>,
) -> InvocationResourceUse {
    let mut indices: HashMap<Pid, usize> = HashMap::new();
    let mut resources: Vec<InvocationResourceUse> = Vec::new();
    for (pid, contribution) in contributions {
        match indices.entry(pid) {
            Entry::Occupied(index) => {
                resources[*index.get()].retain_readings(contribution);
            },
            Entry::Vacant(index) => {
                index.insert(resources.len());
                resources.push(contribution);
            },
        }
    }
    InvocationResourceUse::sum(resources)
}

/// Missing cwd cannot establish equality or be formatted before metadata merging.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorkingDirectoryObservation<'directory> {
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

/// Path spelling alone cannot distinguish a directory from one of its aliases.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DirectoryComparison {
    /// Both observations name the same directory.
    Same,
    /// Readable metadata identifies distinct directories.
    Different,
    /// Missing metadata cannot establish whether different spellings agree.
    Unavailable,
}

impl DirectoryComparison {
    /// Equal observed paths need no extra access; aliases compare filesystem identity.
    /// A relative spelling would resolve against this process's own directory, so it
    /// never establishes identity.
    fn between(left: &Path, right: &Path) -> Self {
        if left == right {
            return Self::Same;
        }
        if !left.is_absolute() || !right.is_absolute() {
            return Self::Unavailable;
        }
        match (fs::metadata(left), fs::metadata(right)) {
            (Ok(left), Ok(right)) if left.is_dir() && right.is_dir() => {
                if left.dev() == right.dev() && left.ino() == right.ino() {
                    Self::Same
                } else {
                    Self::Different
                }
            },
            _ => Self::Unavailable,
        }
    }
}

/// A row's capture context states whether registration fields may describe it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RowProvenance {
    /// No verified capture qualifies this row's own working directory; an observed
    /// process owner still names the account it runs under.
    Uncaptured(ProcessOwner),
    /// This invocation owns its selected registration's metadata.
    Direct(CaptureContext),
    /// A nested invocation shares account and capture, retaining its own fields.
    Enclosing(CaptureContext),
}

impl RowProvenance {
    /// Verification came from this root; missing owner observations grant no qualification,
    /// leaving the row uncaptured under its process `owner`.
    fn direct(capture: &Capture, key: &CaptureKey, owner: ProcessOwner) -> Self {
        let Some(status) = capture.root_status.get(key.root.0) else {
            return Self::Uncaptured(owner);
        };
        let uid = status.root.uid;
        Self::Direct(CaptureContext {
            root:        key.root,
            incarnation: key.incarnation,
            account:     Account {
                uid,
                name: status.account.clone(),
            },
        })
    }

    /// Enclosing membership qualifies a group without granting registration fields.
    fn enclosing(capture: &Capture, key: &CaptureKey, owner: ProcessOwner) -> Self {
        match Self::direct(capture, key, owner) {
            Self::Direct(context) => Self::Enclosing(context),
            provenance => provenance,
        }
    }
}

/// The account an uncaptured invocation's process runs as, named on the scan worker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ProcessOwner {
    /// The process table reported this uid; the account table that named this
    /// scan's roots named it too, so a root of the same uid carries the same label.
    Observed(Account),
    /// An unreadable owner qualifies nothing, so the heading names no account.
    Unavailable,
}

impl ProcessOwner {
    /// Name an observed uid from this scan's account table; rendering performs no lookup.
    fn observe(uid: ProcessField<u32>, capture: &Capture) -> Self {
        match uid {
            ProcessField::Observed(uid) => Self::Observed(Account {
                uid,
                name: capture.account_name(uid),
            }),
            ProcessField::Unavailable => Self::Unavailable,
        }
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

/// One running cargo invocation with numeric resource contributions.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CargoProcess {
    /// Row source and displayed pid do not change invocation continuity.
    pub(crate) invocation_id:        InvocationId,
    /// Nested invocations share progress without inheriting registration metadata.
    pub(crate) capture_membership:   CaptureMembership,
    /// Account and root qualification do not grant enclosing rows metadata ownership.
    pub(crate) provenance:           RowProvenance,
    /// Working directory display, shortened only when the home prefix is unambiguous.
    pub(crate) path:                 String,
    /// Raw absolute directory for grouping; display formatting cannot change membership.
    pub(crate) directory_identity:   WorkingDirectoryIdentity,
    /// Process id.
    pub(crate) pid:                  u32,
    /// The nearest ancestor the cell draws: the cargo above this one
    /// where there is one, since that is a row of the same table, and
    /// otherwise the step of the chain block the command was started
    /// from. Never the immediate parent, which is the pty and shim the
    /// capture opened and is drawn nowhere.
    ///
    /// The named parent state distinguishes invocation identity from chain display.
    pub(crate) parent:               VisibleParent,
    /// Local wall-clock start time, `hh:mm`.
    pub(crate) start:                String,
    /// The same instant as seconds since the epoch, which is what
    /// orders one invocation against another. The label above it is
    /// only accurate to the minute and turns over at midnight, so it
    /// reads well and sorts badly.
    pub(crate) started:              RunStart,
    /// Elapsed run time, `mm:ss` until an hour and `hh:mm:ss` past it.
    pub(crate) duration:             String,
    /// Disjoint work attributed to this invocation, excluding nested cargo buckets.
    pub(crate) resource_use:         InvocationResourceUse,
    /// Numeric total of every assembled invocation below this row, including itself.
    pub(crate) subtree_resource_use: InvocationResourceUse,
    /// What the command is doing, when a capture of its output is there
    /// to read it from. Read off the nearest capture at or above the
    /// invocation, so a cargo the enclosing run started -- which the
    /// shim declines to capture a second time -- reports the run it is
    /// inside rather than nothing at all.
    pub(crate) state:                CaptureLookup,
    /// Cargo invocations running under this one. A measured zero names
    /// a plain command; an unavailable count cannot establish that it is idle.
    pub(crate) managed:              Measurement<usize>,
    /// Whether another cargo stands between this invocation and the
    /// lead of its group. False for the lead itself and for the
    /// invocations it started directly.
    ///
    /// What the summary keeps out. A command's own cell lists its whole
    /// tree, which is where the tree is worth reading; gathered into
    /// one table with every other command's, the deeper levels bury the
    /// runs they came from -- one `cargo nextest run` puts a `cargo
    /// mend` in the table for every test it runs.
    pub(crate) nested:               bool,
    /// The command line, split so program and arguments style apart.
    pub(crate) command:              CommandText,
}

/// The compiler driver an invocation is running, and how many at once.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Compiler {
    /// Driver name, one of [`COMPILER_PROCESS_NAMES`].
    pub(crate) name:  &'static str,
    /// How many of it are running under this cargo invocation.
    pub(crate) count: usize,
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
    pub(crate) command:        CommandText,
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
#[derive(Clone, Debug, PartialEq)]
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
    pub(crate) ancestry: AncestryWalk<Ancestor>,
}

/// Known ancestors retain whether the bounded parent walk reached its end.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AncestryWalk<T> {
    /// No further readable, non-root, unvisited parent remains.
    Ended(Vec<T>),
    /// Another parent remains beyond the walk's limit.
    LimitCut(Vec<T>),
}

impl<T> AncestryWalk<T> {
    /// Known levels, ordered from the outermost ancestor to the nearest.
    pub(crate) fn as_slice(&self) -> &[T] {
        match self {
            Self::Ended(levels) | Self::LimitCut(levels) => levels,
        }
    }

    /// Unavailable process metadata may omit levels without changing why the walk stopped.
    fn filter_map<U>(self, filter: impl FnMut(T) -> Option<U>) -> AncestryWalk<U> {
        match self {
            Self::Ended(levels) => {
                AncestryWalk::Ended(levels.into_iter().filter_map(filter).collect())
            },
            Self::LimitCut(levels) => {
                AncestryWalk::LimitCut(levels.into_iter().filter_map(filter).collect())
            },
        }
    }
}

impl CargoGroup {
    /// The group's identity, stable for as long as the command runs.
    pub(crate) fn id(&self) -> InvocationId { self.lead.invocation_id.clone() }
}

/// `commands.excluded` as the scanner reads it, shared with the app so
/// a list edited in the settings overlay applies from the next scan on
/// without restarting the worker.
#[derive(Clone, Debug)]
pub(crate) struct ExcludedCommands(Arc<RwLock<Vec<String>>>);

impl ExcludedCommands {
    /// Share `subcommands` with the worker.
    pub(crate) fn new(subcommands: Vec<String>) -> Self { Self(Arc::new(RwLock::new(subcommands))) }

    /// Replace the list the next scan leaves out.
    pub(crate) fn replace(&self, subcommands: Vec<String>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = subcommands;
    }

    /// The list as it stands, copied out for one scan.
    pub(crate) fn snapshot(&self) -> Vec<String> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Which cargo processes a scan counts as invocations.
///
/// The executable takes [`CensusScope::default`], every cargo on the
/// machine. PTY tests take `CensusScope::descendants_of` their scenario
/// script, so another session's builds and a parallel test's writers
/// never reach the grid under test. Registrations need no scope: each
/// test already reads its own capture parent.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CensusScope {
    /// The process every counted cargo runs under, at any depth; `None`
    /// counts every cargo.
    root: Option<Pid>,
}

impl CensusScope {
    /// Only the cargo processes running under `root`.
    #[cfg(test)]
    pub(crate) const fn descendants_of(root: Pid) -> Self { Self { root: Some(root) } }

    /// Whether `pid` is counted, walking `parents` up from it.
    ///
    /// Bounded by [`PARENT_WALK_LIMIT`], like every other walk here, so a
    /// reparented chain that loops cannot spin.
    fn contains(self, pid: Pid, parents: &HashMap<Pid, Pid>) -> bool {
        let Some(root) = self.root else {
            return true;
        };
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = parents.get(&current) else {
                return false;
            };
            if parent == root {
                return true;
            }
            current = parent;
        }
        false
    }
}

/// A census worker could not start on the operating system.
#[derive(Debug)]
pub(crate) enum CensusStartError {
    /// The named worker has no thread; the original operating-system error is retained.
    ThreadUnavailable {
        name:   String,
        source: std::io::Error,
    },
}

impl Display for CensusStartError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ThreadUnavailable { name, source } => {
                write!(formatter, "cannot start census thread {name}: {source}")
            },
        }
    }
}

impl std::error::Error for CensusStartError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ThreadUnavailable { source, .. } => Some(source),
        }
    }
}

/// Start the scanner thread and hand back the channel it publishes on.
///
/// The thread ends when the receiver is dropped.
///
/// Each scan reads `excluded` afresh, `cadence` sets the pause between
/// scans and how the `cpu` column follows them, and `scope` sets which
/// cargo processes they count. Parent resolution runs once inside the
/// worker before its scan loop, so a slow filesystem cannot block terminal
/// startup; each scan still opens every root to recheck access and
/// ownership. Keep root resolution on the worker even when it stalls. The
/// resolver and join handle let tests hold resolution and observe repeated
/// scans and shutdown.
///
/// # Errors
///
/// Returns `CensusStartError` if a census worker cannot start.
pub(crate) fn spawn_with_resolver(
    excluded: ExcludedCommands,
    cadence: CensusCadence,
    scope: CensusScope,
    resolve: impl FnOnce() -> CaptureRoots + Send + 'static,
) -> Result<(Receiver<Scan>, JoinHandle<()>), CensusStartError> {
    spawn_resolved_scan_loop(cadence.poll, resolve, move || {
        let mut smoothing = InvocationCpuAccounting::new(cadence);
        let mut history = ScanHistory {
            identities:  ProcessIdentities::default(),
            invocations: Invocations::default(),
            details:     DetailDiscovery::new(|pid| process_details(&[pid]))?,
            refresh:     PublicationStage::new(CENSUS_REFRESH_THREAD_NAME, |()| {
                ProcessDiscovery::refresh()
            })?,
            discovery:   Arc::new(ProcessDiscovery::empty()),
            previous:    Arc::new(ProcessDiscovery::empty()),
            capture:     CaptureDiscovery::Unstarted,
            attribution: PublicationStage::new(
                CENSUS_ATTRIBUTION_THREAD_NAME,
                move |request: AttributionRequest| request.attribute(&mut smoothing),
            )?,
            attributed:  AttributedInvocations::empty(),
        };
        let home = dirs::home_dir();
        Ok(
            move |roots: &CaptureRoots, window: &ScanPublicationWindow| {
                scan(
                    &mut history,
                    window,
                    home.as_deref().into(),
                    &excluded.snapshot(),
                    roots,
                    scope,
                )
            },
        )
    })
}

/// Discovery and publication share one freshness budget, starting before OS reads.
pub(crate) struct ScanPublicationWindow {
    started:        Instant,
    next_discovery: Instant,
}

impl ScanPublicationWindow {
    /// Reserve half the age budget for discovery and half for its publication.
    pub(crate) fn started(started: Instant, budget: Duration) -> Self {
        Self {
            started,
            next_discovery: started + budget / 2,
        }
    }

    /// Completed work consumes the window; an overdue discovery adds no idle time.
    pub(crate) fn pause_after(&self, completed: Instant) -> Duration {
        self.next_discovery.saturating_duration_since(completed)
    }
}

/// The time an answer has already spent in discovery when it is published.
#[derive(Clone, Copy)]
pub(crate) struct PublicationAge {
    published: Instant,
    discovery: Duration,
}

impl PublicationAge {
    /// Carry time spent before publication through the scanner channel.
    pub(crate) fn between(observed: Instant, published: Instant) -> Self {
        Self {
            published,
            discovery: published.saturating_duration_since(observed),
        }
    }

    /// Observation time stays unchanged while the answer waits for rendering.
    pub(crate) fn observed_at(self) -> Instant {
        self.published
            .checked_sub(self.discovery)
            .unwrap_or(self.published)
    }
}

/// A queued read belongs to the native lifetime observed when it was requested.
#[derive(Clone, Eq, Hash, PartialEq)]
struct PendingDetail {
    pid:      Pid,
    identity: InvocationId,
}

/// Publication never waits for a stage: one request owns its worker until completion.
struct PublicationStage<Request, Answer> {
    requests:     SyncSender<Request>,
    replies:      Receiver<Answer>,
    availability: PublicationWorker,
}

/// A blocked operation cannot accumulate stale requests behind it.
enum PublicationWorker {
    Available,
    Running,
}

/// Only a completed answer can replace the last published observation.
enum StageAnswer<Answer> {
    Pending,
    Completed(Answer),
}

impl<Request: Send + 'static, Answer: Send + 'static> PublicationStage<Request, Answer> {
    fn new(
        name: &'static str,
        mut work: impl FnMut(Request) -> Answer + Send + 'static,
    ) -> Result<Self, CensusStartError> {
        let (requests, incoming) = mpsc::sync_channel(1);
        let (completed, replies) = mpsc::channel();
        thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || {
                while let Ok(request) = incoming.recv() {
                    let started = Instant::now();
                    let answer = work(request);
                    probe::scanner_cost(name, started.elapsed());
                    if completed.send(answer).is_err() {
                        return;
                    }
                }
            })
            .map_err(|source| CensusStartError::ThreadUnavailable {
                name: name.to_owned(),
                source,
            })?;
        Ok(Self {
            requests,
            replies,
            availability: PublicationWorker::Available,
        })
    }

    fn receive(&mut self) -> StageAnswer<Answer> {
        match self.replies.try_recv() {
            Ok(answer) => {
                self.availability = PublicationWorker::Available;
                StageAnswer::Completed(answer)
            },
            Err(_) => StageAnswer::Pending,
        }
    }

    fn sample(&mut self, request: impl FnOnce() -> Request) -> StageAnswer<Answer> {
        let answer = self.receive();
        if matches!(self.availability, PublicationWorker::Available)
            && self.requests.try_send(request()).is_ok()
        {
            self.availability = PublicationWorker::Running;
        }
        answer
    }
}

/// A completed process discovery covers absent subjects as well as present pids.
#[derive(Clone)]
enum ProcessDiscoveryEvidence {
    Current(HashSet<Pid>),
    Retained,
}

impl ProcessDiscoveryEvidence {
    fn contains(&self, pid: Pid) -> bool {
        match self {
            Self::Current(processes) => processes.contains(&pid),
            Self::Retained => false,
        }
    }
}

/// Registrations retained during a blocked read are evidence of uncertainty, not a new observation.
#[derive(Clone, Copy)]
enum CaptureFreshness {
    Current,
    Retained,
}

/// The capture worker keeps account indices and descriptors off the publication path.
enum CaptureDiscovery {
    Unstarted,
    Unavailable(CensusStartError),
    Reading {
        stage:  PublicationStage<(), Capture>,
        latest: Capture,
    },
}

impl CaptureDiscovery {
    fn sample(
        &mut self,
        roots: &CaptureRoots,
    ) -> Result<(&mut Capture, CaptureFreshness), &CensusStartError> {
        match self {
            Self::Unstarted => {
                let worker_roots = roots.clone();
                *self = match PublicationStage::new(CENSUS_CAPTURE_THREAD_NAME, move |()| {
                    Capture::take(&worker_roots)
                }) {
                    Ok(stage) => Self::Reading {
                        stage,
                        latest: Capture::default(),
                    },
                    Err(error) => Self::Unavailable(error),
                };
                self.sample(roots)
            },
            Self::Unavailable(error) => Err(error),
            Self::Reading { stage, latest } => {
                let freshness = match stage.sample(|| ()) {
                    StageAnswer::Completed(capture) => {
                        *latest = capture;
                        CaptureFreshness::Current
                    },
                    StageAnswer::Pending => CaptureFreshness::Retained,
                };
                Ok((latest, freshness))
            },
        }
    }
}

/// Reusing a completed snapshot cannot revive a row while its worker is still blocked.
#[derive(Clone)]
enum PublicationEvidence {
    Current,
    Selected {
        processes: ProcessDiscoveryEvidence,
        captures:  CaptureFreshness,
    },
}

/// Only completed discovery can supply current process absence evidence.
#[derive(Clone, Copy)]
enum DiscoveryFreshness {
    Current,
    Retained,
}

impl PublicationEvidence {
    fn selected(
        system: &System,
        freshness: DiscoveryFreshness,
        captures: CaptureFreshness,
    ) -> Self {
        Self::Selected {
            processes: match freshness {
                DiscoveryFreshness::Current => {
                    ProcessDiscoveryEvidence::Current(system.processes().keys().copied().collect())
                },
                DiscoveryFreshness::Retained => ProcessDiscoveryEvidence::Retained,
            },
            captures,
        }
    }

    const fn has_current_capture(&self) -> bool {
        matches!(
            self,
            Self::Current
                | Self::Selected {
                    captures: CaptureFreshness::Current,
                    ..
                }
        )
    }

    fn has_current_process(&self, pid: Pid) -> bool {
        match self {
            Self::Current => true,
            Self::Selected { processes, .. } => processes.contains(pid),
        }
    }

    fn permits(&self, row: &CargoProcess) -> bool {
        match self {
            Self::Current => true,
            Self::Selected {
                processes,
                captures,
            } => {
                processes.contains(Pid::from_u32(row.pid))
                    || (matches!(captures, CaptureFreshness::Current)
                        && matches!(row.invocation_id, InvocationId::Captured(_)))
            },
        }
    }
}

/// Discovery evidence is read on the refresh worker, including native lifetime reads.
struct ProcessDiscovery {
    system:      System,
    baselines:   HashMap<Pid, CpuBaseline>,
    observed_at: Instant,
}

impl ProcessDiscovery {
    fn empty() -> Self {
        Self {
            system:      System::new(),
            baselines:   HashMap::new(),
            observed_at: Instant::now(),
        }
    }

    fn refresh() -> Arc<Self> {
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            process_discovery_refresh_kind(),
        );
        let observed_at = Instant::now();
        let baselines = system
            .processes()
            .iter()
            .map(|(&pid, process)| (pid, CpuBaseline::from(process)))
            .collect();
        Arc::new(Self {
            system,
            baselines,
            observed_at,
        })
    }

    fn records(&self) -> Vec<(&Process, CpuBaseline)> {
        self.system
            .processes()
            .iter()
            .filter_map(|(pid, process)| {
                self.baselines
                    .get(pid)
                    .map(|baseline| (process, baseline.clone()))
            })
            .collect()
    }
}

/// Attribution owns snapshots, so a blocked counter or path read holds no publication borrow.
struct AttributionRequest {
    census:    Census,
    discovery: Arc<ProcessDiscovery>,
    previous:  Arc<ProcessDiscovery>,
    details:   CompletedAttributionDetails<System>,
    at:        Instant,
}

/// A requester has a proved source join or still awaits the retained tree's join.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ForwardingJoin {
    Proved,
    Pending,
}

/// Forwarding aliases count as one requester before compiler ownership is selected.
struct CompilerRequester {
    pid:  Pid,
    join: ForwardingJoin,
}

impl CompilerRequester {
    fn resolve(
        mut pid: Pid,
        census: &Census,
        observations: &ProcessObservations<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
    ) -> Self {
        let mut join = ForwardingJoin::Proved;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some([child]) = children.get(&pid).map(Vec::as_slice) else {
                return Self { pid, join };
            };
            if census.forwards_to_single_child(observations, children, pid) {
                pid = *child;
            } else if Census::is_shim(observations, children, pid) {
                join = ForwardingJoin::Pending;
                pid = *child;
            } else {
                return Self { pid, join };
            }
        }
        Self {
            pid,
            join: ForwardingJoin::Pending,
        }
    }
}

/// Each compiler has one owner, awaits source evidence, or has no matching owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompilerAttribution {
    Owner(Pid),
    PendingMetadata,
    Unassigned,
}

impl CompilerAttribution {
    /// Missing comparison inputs cannot prove either a unique owner or no owner.
    fn resolve(
        pid: Pid,
        census: &Census,
        observations: &ProcessObservations<'_>,
        targets: &HashMap<Pid, HashSet<PathBuf>>,
    ) -> Self {
        let Some(process) = observations.process(pid) else {
            return Self::PendingMetadata;
        };
        let (ProcessField::Observed(_), ProcessField::Observed(uid)) = (process.argv, process.uid)
        else {
            return Self::PendingMetadata;
        };
        let directory =
            match invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG) {
                Ok(directory) => directory,
                Err(CompilePathAbsence::Unavailable) => return Self::PendingMetadata,
                Err(CompilePathAbsence::Unspecified) => return Self::Unassigned,
            };
        let mut requesters = HashMap::new();
        let children = census.cargo_children();
        for owner in census.live_cargo() {
            let Some(cargo) = observations.process(owner) else {
                return Self::PendingMetadata;
            };
            let ProcessField::Observed(owner_uid) = cargo.uid else {
                return Self::PendingMetadata;
            };
            // A shared filesystem path never links work from different users.
            if uid != owner_uid {
                continue;
            }
            let ProcessField::Observed(_) = cargo.argv else {
                return Self::PendingMetadata;
            };
            if matches!(cargo.directory, ProcessField::Unavailable) {
                return Self::PendingMetadata;
            }
            if targets
                .get(&owner)
                .is_some_and(|targets| targets.iter().any(|target| directory.starts_with(target)))
            {
                let requester = CompilerRequester::resolve(owner, census, observations, &children);
                requesters
                    .entry(requester.pid)
                    .and_modify(|join| {
                        if requester.join == ForwardingJoin::Pending {
                            *join = ForwardingJoin::Pending;
                        }
                    })
                    .or_insert(requester.join);
            }
        }
        match invocation_cpu_accounting::compile_owner(requesters.keys().copied()) {
            CompileOwner::Unique(owner) => match requesters[&owner] {
                ForwardingJoin::Proved => Self::Owner(owner),
                ForwardingJoin::Pending => Self::PendingMetadata,
            },
            CompileOwner::Unknown | CompileOwner::Ambiguous => Self::Unassigned,
        }
    }

    /// Native identity and the original credit owner must authorize each selected owner.
    fn bind(
        self,
        compiler: Pid,
        census: &Census,
        accounting: &mut InvocationCpuAccounting,
    ) -> Self {
        let Self::Owner(owner) = self else {
            return self;
        };
        let (Ok(compiler), Ok(process)) =
            (census.cpu_identity(compiler), census.cpu_identity(owner))
        else {
            return Self::PendingMetadata;
        };
        let Some(identity) = accounting.owners.get(&process) else {
            return Self::PendingMetadata;
        };
        if accounting
            .cache_owners
            .entry(compiler)
            .or_insert_with(|| identity.clone())
            == identity
        {
            Self::Owner(owner)
        } else {
            Self::Unassigned
        }
    }
}

/// Only owners with completed current-source metadata enter product attribution.
struct AttributableInvocations {
    pids: HashSet<Pid>,
}

impl AttributableInvocations {
    fn current(census: &Census, observations: &ProcessObservations<'_>) -> Self {
        Self {
            pids: census
                .live_cargo()
                .filter(|pid| observations.process(*pid).is_some())
                .collect(),
        }
    }

    /// Counter fixtures supply their own ownership and metadata evidence.
    #[cfg(test)]
    fn supplied(census: &Census) -> Self {
        Self {
            pids: census.live_cargo().collect(),
        }
    }
}

/// One completed counter pass carries its read time through rate and publication updates.
struct InvocationCpuSamples {
    at:       Instant,
    readings: HashMap<Pid, Measurement<f32>>,
}

struct AttributedInvocations {
    identities:   HashMap<Pid, InvocationId>,
    measurements: InvocationMeasurements,
}

impl AttributionRequest {
    fn attribute(self, smoothing: &mut InvocationCpuAccounting) -> AttributedInvocations {
        let discovery = self.discovery.records();
        let mut records = ProcessObservations::from_discovery(&discovery, &self.previous.baselines);
        for batch in &self.details.0 {
            records.overlay_details(batch);
        }
        let measurements = self.census.attribute(&records, smoothing, self.at);
        AttributedInvocations {
            identities: self.census.identities,
            measurements,
        }
    }
}

impl AttributedInvocations {
    fn empty() -> Self {
        Self {
            identities:   HashMap::new(),
            measurements: InvocationMeasurements {
                #[cfg(test)]
                cpu_scan_publication:              CpuScanPublication::Unobserved,
                compilers:                         HashMap::new(),
                cpu:                               HashMap::new(),
                memory:                            HashMap::new(),
            },
        }
    }

    /// A result for a replaced PID cannot be charged to its new invocation.
    fn for_census(&self, census: &Census) -> InvocationMeasurements {
        let same = |pid: &Pid| {
            self.identities
                .get(pid)
                .is_some_and(|identity| census.identities.get(pid) == Some(identity))
        };
        InvocationMeasurements {
            #[cfg(test)]
            cpu_scan_publication:              self.measurements.cpu_scan_publication,
            compilers:                         self
                .measurements
                .compilers
                .iter()
                .filter(|(pid, _)| same(pid))
                .map(|(pid, value)| (*pid, value.clone()))
                .collect(),
            cpu:                               self
                .measurements
                .cpu
                .iter()
                .filter(|(pid, _)| same(pid))
                .map(|(pid, value)| (*pid, *value))
                .collect(),
            memory:                            self
                .measurements
                .memory
                .iter()
                .filter(|(pid, _)| same(pid))
                .map(|(pid, value)| (*pid, *value))
                .collect(),
        }
    }
}

/// A lifetime has one metadata read in flight, retaining its last completed answer.
enum DetailRead<T> {
    Queued,
    Reading,
    Ready(Arc<T>),
    Refreshing(Arc<T>),
    /// Discovery observed another executable name after the read was scheduled.
    ReadingChanged,
    RefreshingChanged(Arc<T>),
    SourceChanged(Arc<T>),
    ReplacingSource(Arc<T>),
}

impl<T> DetailRead<T> {
    fn answer(&self) -> Option<Arc<T>> {
        match self {
            Self::Queued | Self::Reading | Self::ReadingChanged => None,
            Self::Ready(answer)
            | Self::Refreshing(answer)
            | Self::RefreshingChanged(answer)
            | Self::SourceChanged(answer)
            | Self::ReplacingSource(answer) => Some(Arc::clone(answer)),
        }
    }
}

/// Attribution consumes each completed metadata answer for its current source.
struct CompletedAttributionDetails<T>(Vec<Arc<T>>);

impl<T> CompletedAttributionDetails<T> {
    fn from_reads<'a>(reads: impl Iterator<Item = &'a DetailRead<T>>) -> StageAnswer<Self>
    where
        T: 'a,
    {
        let answers = reads
            .filter_map(|read| match read {
                DetailRead::Ready(answer) | DetailRead::Refreshing(answer) => {
                    Some(Arc::clone(answer))
                },
                DetailRead::Queued
                | DetailRead::Reading
                | DetailRead::ReadingChanged
                | DetailRead::RefreshingChanged(_)
                | DetailRead::SourceChanged(_)
                | DetailRead::ReplacingSource(_) => None,
            })
            .collect::<Vec<_>>();
        if answers.is_empty() {
            StageAnswer::Pending
        } else {
            StageAnswer::Completed(Self(answers))
        }
    }
}

/// Discovery's latest source and the metadata read for one retained native lifetime.
struct LifetimeDetails<T> {
    source: ProcessField<OsString>,
    read:   DetailRead<T>,
}

/// `CENSUS_DETAIL_READER_COUNT` readers share a bounded queue; publishing never waits.
struct DetailDiscovery<T> {
    reads:     HashMap<PendingDetail, LifetimeDetails<T>>,
    pending:   VecDeque<PendingDetail>,
    changes:   VecDeque<PendingDetail>,
    refreshes: VecDeque<PendingDetail>,
    requests:  SyncSender<PendingDetail>,
    replies:   Receiver<(PendingDetail, T)>,
}

impl<T: Send + 'static> DetailDiscovery<T> {
    fn new(read: impl Fn(Pid) -> T + Send + Sync + 'static) -> Result<Self, CensusStartError> {
        let (requests, incoming) = mpsc::sync_channel::<PendingDetail>(CENSUS_DETAIL_READER_COUNT);
        let incoming = Arc::new(std::sync::Mutex::new(incoming));
        let read = Arc::new(read);
        let (completed, replies) = mpsc::channel();
        for reader in 0..CENSUS_DETAIL_READER_COUNT {
            let incoming = Arc::clone(&incoming);
            let read = Arc::clone(&read);
            let completed = completed.clone();
            let name = format!("{CENSUS_DETAIL_THREAD_PREFIX}-{reader}");
            thread::Builder::new()
                .name(name.clone())
                .spawn(move || {
                    loop {
                        let request = incoming
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .recv();
                        let Ok(request) = request else {
                            return;
                        };
                        let answer = read(request.pid);
                        if completed.send((request, answer)).is_err() {
                            return;
                        }
                    }
                })
                .map_err(|source| CensusStartError::ThreadUnavailable { name, source })?;
        }
        Ok(Self {
            reads: HashMap::new(),
            pending: VecDeque::new(),
            changes: VecDeque::new(),
            refreshes: VecDeque::new(),
            requests,
            replies,
        })
    }

    fn read(
        &mut self,
        pids: &[Pid],
        identities: &HashMap<Pid, InvocationId>,
        source: impl Fn(Pid) -> ProcessField<OsString>,
    ) -> Vec<Arc<T>> {
        self.reads.retain(|request, _| {
            pids.contains(&request.pid) && identities.get(&request.pid) == Some(&request.identity)
        });
        self.receive();
        self.pending
            .retain(|request| self.reads.contains_key(request));
        self.changes
            .retain(|request| self.reads.contains_key(request));
        self.refreshes
            .retain(|request| self.reads.contains_key(request));
        let mut arrivals = VecDeque::new();
        for pid in pids {
            let Some(identity) = identities.get(pid) else {
                continue;
            };
            let request = PendingDetail {
                pid:      *pid,
                identity: identity.clone(),
            };
            let source = source(*pid);
            match self.reads.entry(request.clone()) {
                Entry::Vacant(entry) => {
                    entry.insert(LifetimeDetails {
                        source,
                        read: DetailRead::Queued,
                    });
                    arrivals.push_back(request);
                },
                Entry::Occupied(mut entry) => {
                    let details = entry.get_mut();
                    if details.source != source {
                        details.source = source;
                        if matches!(details.read, DetailRead::Ready(_)) {
                            self.changes.push_back(request);
                        }
                        details.read = match &details.read {
                            DetailRead::Queued => DetailRead::Queued,
                            DetailRead::Reading | DetailRead::ReadingChanged => {
                                DetailRead::ReadingChanged
                            },
                            DetailRead::Ready(answer) | DetailRead::SourceChanged(answer) => {
                                DetailRead::SourceChanged(Arc::clone(answer))
                            },
                            DetailRead::Refreshing(answer)
                            | DetailRead::RefreshingChanged(answer)
                            | DetailRead::ReplacingSource(answer) => {
                                DetailRead::RefreshingChanged(Arc::clone(answer))
                            },
                        };
                    }
                },
            }
        }
        arrivals.append(&mut self.pending);
        self.pending = arrivals;
        self.dispatch();
        self.reads
            .values()
            .filter_map(|details| details.read.answer())
            .collect()
    }

    /// A source change during a running read requires another read after its reply.
    fn receive(&mut self) {
        while let Ok((request, answer)) = self.replies.try_recv() {
            if let Some(details) = self.reads.get_mut(&request) {
                details.read = match details.read {
                    DetailRead::ReadingChanged | DetailRead::RefreshingChanged(_) => {
                        self.changes.push_back(request);
                        DetailRead::SourceChanged(Arc::new(answer))
                    },
                    _ => {
                        self.refreshes.push_back(request);
                        DetailRead::Ready(Arc::new(answer))
                    },
                };
            }
        }
    }

    /// First reads precede changed sources; unchanged refreshes retain completion order.
    fn dispatch(&mut self) {
        while let Some(request) = self.pending.pop_front() {
            match self.requests.try_send(request.clone()) {
                Ok(()) => {
                    if let Some(details) = self.reads.get_mut(&request) {
                        details.read = DetailRead::Reading;
                    }
                },
                Err(TrySendError::Full(request)) => {
                    self.pending.push_front(request);
                    return;
                },
                Err(TrySendError::Disconnected(_)) => return,
            }
        }
        for queue in [&mut self.changes, &mut self.refreshes] {
            while let Some(request) = queue.pop_front() {
                let Some(details) = self.reads.get_mut(&request) else {
                    continue;
                };
                let (DetailRead::Ready(answer) | DetailRead::SourceChanged(answer)) = &details.read
                else {
                    continue;
                };
                match self.requests.try_send(request.clone()) {
                    Ok(()) => {
                        details.read = if matches!(details.read, DetailRead::SourceChanged(_)) {
                            DetailRead::ReplacingSource(Arc::clone(answer))
                        } else {
                            DetailRead::Refreshing(Arc::clone(answer))
                        };
                    },
                    Err(TrySendError::Full(request)) => {
                        queue.push_front(request);
                        return;
                    },
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        }
    }
}

/// Resolve capture roots once, then publish every scan produced from them.
fn spawn_resolved_scan_loop<Published, Prepare, TakeScan>(
    poll: Duration,
    resolve: impl FnOnce() -> CaptureRoots + Send + 'static,
    prepare: Prepare,
) -> Result<(Receiver<Published>, JoinHandle<()>), CensusStartError>
where
    Published: Send + 'static,
    Prepare: FnOnce() -> Result<TakeScan, CensusStartError>,
    TakeScan: FnMut(&CaptureRoots, &ScanPublicationWindow) -> Published + Send + 'static,
{
    let mut take_scan = prepare()?;
    let (sender, receiver) = mpsc::channel();
    let worker = thread::Builder::new()
        .name(CENSUS_THREAD_NAME.to_owned())
        .spawn(move || {
            let roots = resolve();
            loop {
                let window = ScanPublicationWindow::started(Instant::now(), poll);
                if sender.send(take_scan(&roots, &window)).is_err() {
                    return;
                }
                thread::sleep(window.pause_after(Instant::now()));
            }
        })
        .map_err(|source| CensusStartError::ThreadUnavailable {
            name: CENSUS_THREAD_NAME.to_owned(),
            source,
        })?;
    Ok((receiver, worker))
}

/// One two-phase scan, newest group first.
///
/// `history` retains completed observations and attribution between publications.
/// `window` sets the publication start and the worker polling budget.
fn scan(
    history: &mut ScanHistory,
    window: &ScanPublicationWindow,
    home: ScannerHome<'_>,
    excluded: &[String],
    roots: &CaptureRoots,
    scope: CensusScope,
) -> Scan {
    let ScanHistory {
        identities,
        invocations,
        details,
        refresh,
        discovery: current,
        previous,
        capture,
        attribution,
        attributed,
    } = history;
    let fresh_discovery = match refresh.sample(|| ()) {
        StageAnswer::Completed(discovery) => {
            *previous = Arc::clone(current);
            *current = discovery;
            DiscoveryFreshness::Current
        },
        StageAnswer::Pending => DiscoveryFreshness::Retained,
    };
    let discovery = current.records();
    let mut observations = ProcessObservations::from_discovery(&discovery, &previous.baselines);
    let mut census = Census::take(observations.records.values());
    census.restrict_to(scope);
    if matches!(fresh_discovery, DiscoveryFreshness::Current) {
        identities.retire_missing(&census.lifetimes, process_identity::ProvenProcessEnd::probe);
    }
    invocations.retain_identity_evidence(identities.retained());
    census.identities = identities.observe(&census.lifetimes);
    let mut unavailable_capture = Capture::default();
    let (capture, capture_freshness) = capture
        .sample(roots)
        .unwrap_or((&mut unavailable_capture, CaptureFreshness::Retained));
    census.publication_evidence =
        PublicationEvidence::selected(&current.system, fresh_discovery, capture_freshness);

    // Phase two reads cargo, its ancestors, and compiler drivers. Ancestors name
    // the launcher; compiler arguments associate cache-server work with its requester.
    // Verification precedes row filtering, and registration pids must receive fresh
    // argv even when the cheap snapshot retains a pre-exec process name.
    let mut detailed = census.detailed();
    for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
        for candidate in
            std::iter::once(pid).chain(census.ancestor_pids(pid).as_slice().iter().copied())
        {
            if !detailed.contains(&candidate) {
                detailed.push(candidate);
            }
        }
    }
    let detail_batches = details.read(&detailed, &census.identities, |pid| {
        current
            .system
            .process(pid)
            .map(|process| process.name().to_os_string())
            .into()
    });
    for batch in &detail_batches {
        observations.overlay_details(batch);
    }
    let mut tree = invocations.identify(&mut census, &observations, capture);
    census.prepare(&observations, capture, excluded);
    let attribution_details = CompletedAttributionDetails::from_reads(
        details.reads.values().map(|details| &details.read),
    );
    let attribution_answer = match (fresh_discovery, attribution_details) {
        (DiscoveryFreshness::Current, StageAnswer::Completed(details)) => {
            attribution.sample(|| AttributionRequest {
                census: census.clone(),
                discovery: Arc::clone(current),
                previous: Arc::clone(previous),
                details,
                at: current.observed_at,
            })
        },
        _ => attribution.receive(),
    };
    if let StageAnswer::Completed(answer) = attribution_answer {
        *attributed = answer;
    }
    let measurements = attributed.for_census(&census);
    let mut groups = census.groups(&observations, &measurements, home, capture);
    tree.retain_ancestry(&census, &mut groups);
    census.associate_status(capture, &groups);
    let invocations = invocations.publish(&census, &tree, groups, capture, Instant::now(), |row| {
        InvocationProbe::kernel(ProcessLifetimeSubject::observe(row))
    });
    Scan {
        #[cfg(test)]
        cpu_scan_publication: measurements.cpu_scan_publication,
        invocations,
        age: PublicationAge::between(window.started, Instant::now()),
        sccache: census.sccache(),
        root_status: capture.root_status.clone(),
        shared_directory: capture.shared_directory.clone(),
    }
}

/// Fields read for the full-system pass.
///
/// Tasks are Linux threads. They inherit a process's name and command,
/// so including them would draw one cargo invocation more than once.
fn process_discovery_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cpu()
        .with_memory()
}

/// Fields read for cargo, its ancestors, and compiler ownership evidence.
fn process_detail_refresh_kind() -> ProcessRefreshKind {
    ProcessRefreshKind::nothing()
        .without_tasks()
        .with_cwd(UpdateKind::OnlyIfNotSet)
        .with_cmd(UpdateKind::OnlyIfNotSet)
        .with_exe(UpdateKind::OnlyIfNotSet)
        .with_environ(UpdateKind::OnlyIfNotSet)
        .with_user(UpdateKind::OnlyIfNotSet)
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

/// An independently readable process field never substitutes for another field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ProcessField<T> {
    Observed(T),
    Unavailable,
}

impl<T> From<Option<T>> for ProcessField<T> {
    fn from(value: Option<T>) -> Self { value.map_or(Self::Unavailable, Self::Observed) }
}

/// A PID can disappear between discovery and the detailed OS refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DetailPresence {
    Observed,
    Absent,
}

/// One scan's independent metadata and CPU evidence, borrowed from its OS snapshots.
#[derive(Clone, Debug)]
pub(crate) struct ProcessObservation<'scan> {
    pub(super) pid:         Pid,
    pub(crate) parent:      ProcessField<Pid>,
    pub(crate) name:        ProcessField<&'scan OsStr>,
    pub(super) argv:        ProcessField<&'scan [OsString]>,
    pub(crate) directory:   ProcessField<&'scan Path>,
    pub(super) executable:  ProcessField<&'scan Path>,
    pub(crate) uid:         ProcessField<u32>,
    pub(super) environment: &'scan [OsString],
    pub(super) lifetime:    Cow<'scan, LifetimeEvidence>,
    status:                 NativeProcessStatus,
    pub(super) cpu:         Measurement<f32>,
    pub(super) memory:      u64,
    pub(super) accumulated: u64,
    /// Ordered accounting stores the native read while metadata remains shared.
    pub(super) native_cpu:  Cell<Measurement<Duration>>,
    pub(super) started:     u64,
    pub(super) elapsed:     u64,
    details:                DetailPresence,
}

/// A native birth proves continued execution only while its process is live.
#[derive(Clone, Copy, Debug)]
enum NativeProcessStatus {
    Live,
    Zombie,
    Unavailable,
}

impl From<ProcessStatus> for NativeProcessStatus {
    fn from(status: ProcessStatus) -> Self {
        match status {
            ProcessStatus::Zombie => Self::Zombie,
            ProcessStatus::Unknown(_) => Self::Unavailable,
            _ => Self::Live,
        }
    }
}

impl<'scan> ProcessObservation<'scan> {
    fn from_process(
        process: &'scan Process,
        baseline: &'scan CpuBaseline,
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Self {
        let cpu = Census::measure_cpu(process.pid(), process.cpu_usage(), baseline, previous);
        Self::metadata(process, &baseline.lifetime, cpu)
    }

    fn metadata(
        process: &'scan Process,
        lifetime: &'scan LifetimeEvidence,
        cpu: Measurement<f32>,
    ) -> Self {
        Self {
            pid: process.pid(),
            parent: process
                .parent()
                .or_else(|| kernel_parent(process.pid()))
                .into(),
            name: (!process.name().is_empty()).then(|| process.name()).into(),
            argv: (!process.cmd().is_empty()).then(|| process.cmd()).into(),
            directory: process.cwd().into(),
            executable: process.exe().into(),
            uid: process.user_id().map(|uid| **uid).into(),
            environment: process.environ(),
            cpu,
            memory: process.memory(),
            lifetime: Cow::Borrowed(lifetime),
            status: process.status().into(),
            accumulated: process.accumulated_cpu_time(),
            native_cpu: Cell::new(Measurement::Unavailable(MeasurementAbsence::ReadFailed)),
            started: process.start_time(),
            elapsed: process.run_time(),
            details: DetailPresence::Absent,
        }
    }

    const fn pid(&self) -> Pid { self.pid }
    fn name(&self) -> &OsStr {
        match self.name {
            ProcessField::Observed(name) => name,
            ProcessField::Unavailable => OsStr::new(""),
        }
    }
    pub(super) const fn cmd(&self) -> &[OsString] {
        match self.argv {
            ProcessField::Observed(argv) => argv,
            ProcessField::Unavailable => &[],
        }
    }
    pub(super) const fn cwd(&self) -> WorkingDirectoryObservation<'_> {
        match self.directory {
            ProcessField::Observed(directory) => WorkingDirectoryObservation::Observed(directory),
            ProcessField::Unavailable => WorkingDirectoryObservation::Unavailable,
        }
    }
    const fn exe(&self) -> ProcessField<&Path> { self.executable }
    pub(super) const fn environ(&self) -> &[OsString] { self.environment }
    const fn start_time(&self) -> u64 { self.started }
    const fn run_time(&self) -> u64 { self.elapsed }

    #[cfg(test)]
    pub(crate) fn with_lifetime(mut self, lifetime: LifetimeEvidence) -> Self {
        self.lifetime = Cow::Owned(lifetime);
        self
    }

    /// Discovery knows a PID and its parent while its metadata request is pending.
    #[cfg(test)]
    pub(crate) const fn without_details(mut self) -> Self {
        self.argv = ProcessField::Unavailable;
        self.details = DetailPresence::Absent;
        self
    }

    /// Supply a CPU reading without a kernel refresh.
    #[cfg(test)]
    pub(crate) const fn with_cpu_for_test(mut self, cpu: Measurement<f32>) -> Self {
        self.cpu = cpu;
        self
    }

    #[cfg(test)]
    pub(crate) fn cargo(pid: u32, argv: &'scan [OsString]) -> Self {
        Self {
            pid:         Pid::from_u32(pid),
            parent:      ProcessField::Unavailable,
            name:        ProcessField::Observed(OsStr::new(CARGO_DISPLAY_NAME)),
            argv:        ProcessField::Observed(argv),
            directory:   ProcessField::Observed(Path::new("/work")),
            executable:  ProcessField::Unavailable,
            uid:         ProcessField::Observed(1000),
            environment: &[],
            lifetime:    Cow::Owned(LifetimeEvidence::Available(ProcessLifetime::for_test(
                u64::from(pid),
            ))),
            status:      NativeProcessStatus::Live,
            cpu:         Measurement::Unavailable(MeasurementAbsence::Unproven),
            memory:      0,
            accumulated: 0,
            native_cpu:  Cell::new(Measurement::Unavailable(MeasurementAbsence::ReadFailed)),
            started:     1,
            elapsed:     0,
            details:     DetailPresence::Observed,
        }
    }
}

/// A single PID index holds both discovery and fresh detail observations.
#[derive(Default)]
pub(crate) struct ProcessObservations<'scan> {
    records: HashMap<Pid, ProcessObservation<'scan>>,
}

impl<'scan> ProcessObservations<'scan> {
    fn from_discovery(
        discovery: &'scan [(&'scan Process, CpuBaseline)],
        previous: &HashMap<Pid, CpuBaseline>,
    ) -> Self {
        Self {
            records: discovery
                .iter()
                .map(|(process, baseline)| {
                    (
                        process.pid(),
                        ProcessObservation::from_process(process, baseline, previous),
                    )
                })
                .collect(),
        }
    }

    /// Fresh detail fields replace metadata without rereading discovery parent or lifetime
    /// evidence.
    fn overlay_details(&mut self, details: &'scan System) {
        for (&pid, process) in details.processes() {
            let record = self.records.entry(pid).or_insert_with(|| {
                ProcessObservation::metadata(
                    process,
                    &LifetimeEvidence::Unavailable,
                    Measurement::Unavailable(MeasurementAbsence::Unproven),
                )
            });
            record.details = DetailPresence::Observed;
            record.name = (!process.name().is_empty()).then(|| process.name()).into();
            record.argv = (!process.cmd().is_empty()).then(|| process.cmd()).into();
            record.directory = process.cwd().into();
            record.executable = process.exe().into();
            record.uid = process.user_id().map(|uid| **uid).into();
            record.environment = process.environ();
            record.started = process.start_time();
            record.elapsed = process.run_time();
        }
    }

    fn process(&self, pid: Pid) -> Option<&ProcessObservation<'scan>> {
        self.records
            .get(&pid)
            .filter(|record| record.details == DetailPresence::Observed)
    }
    const fn processes(&self) -> &HashMap<Pid, ProcessObservation<'scan>> { &self.records }

    #[cfg(test)]
    pub(crate) fn new(records: impl IntoIterator<Item = ProcessObservation<'scan>>) -> Self {
        Self {
            records: records
                .into_iter()
                .map(|record| (record.pid, record))
                .collect(),
        }
    }
}

/// A bounded ancestry walk establishes an owner or preserves missing evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CargoAncestry {
    /// An observed parent is a retained cargo invocation.
    Owner(Pid),
    /// The next parent is unavailable; an owner remains unestablished.
    ParentUnavailable,
    /// The walk exhausted its bound; an owner remains unestablished.
    WalkLimitReached,
}

/// The scanner owns both process-tree continuity and cumulative measurements.
struct ScanHistory {
    identities:  ProcessIdentities,
    invocations: Invocations,
    details:     DetailDiscovery<System>,
    refresh:     PublicationStage<(), Arc<ProcessDiscovery>>,
    discovery:   Arc<ProcessDiscovery>,
    previous:    Arc<ProcessDiscovery>,
    capture:     CaptureDiscovery,
    attribution: PublicationStage<AttributionRequest, AttributedInvocations>,
    attributed:  AttributedInvocations,
}

/// Publication position used to retain an unpainted end for one publication.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
struct ScanNumber(u64);

/// Each invocation belongs to exactly one family, including across missing scans.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Membership {
    Lead,
    Member { lead: InvocationId },
}

/// The census has selected one execution for this identity.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ResolvedInvocationId(InvocationId);

impl ResolvedInvocationId {
    pub(crate) const fn id(&self) -> &InvocationId { &self.0 }
}

/// No retained execution resolves this identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct UnresolvedInvocation;

/// The actual allocation that supplied an invocation's paint receipt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum InvocationPaintSite {
    Summary,
    Cell(InvocationId),
}

/// The retained receipt decides whether an ended invocation still has a paint site.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum EndedPresentation {
    Unpresented,
    Presented(InvocationPaintSite),
    Withdrawn,
    Departed,
}

/// The receipt state belongs to the invocation that owned the painted rows.
#[derive(Clone, Default, Debug)]
enum Presentation {
    #[default]
    NotPresented,
    Presented {
        site:      InvocationPaintSite,
        spans:     HashMap<u32, PaintedRowSpan<InvocationId>>,
        departure: CellDepartureReceipt,
    },
    Withdrawn,
    Departed,
}

/// Receipt shared by census retention and each actual row presentation.
#[derive(Clone, Debug)]
pub(crate) struct InvocationPresentation(Arc<Mutex<Presentation>>, ResolvedInvocationId);

impl InvocationPresentation {
    fn new(id: ResolvedInvocationId) -> Self {
        Self(Arc::new(Mutex::new(Presentation::NotPresented)), id)
    }
    #[cfg(test)]
    pub(crate) fn for_test(id: InvocationId) -> Self { Self::new(ResolvedInvocationId(id)) }
    pub(crate) const fn resolved(&self) -> &ResolvedInvocationId { &self.1 }
    pub(crate) fn painted_process(
        &self,
        pid: u32,
        site: InvocationPaintSite,
        span: PaintedRowSpan<InvocationId>,
    ) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match &mut *state {
            Presentation::Presented {
                site: painted_site,
                spans,
                ..
            } => {
                *painted_site = site;
                spans.insert(pid, span);
            },
            Presentation::NotPresented | Presentation::Withdrawn => {
                *state = Presentation::Presented {
                    site,
                    spans: HashMap::from([(pid, span)]),
                    departure: CellDepartureReceipt::default(),
                };
            },
            Presentation::Departed => {},
        }
    }
    fn departed(&self) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Presentation::Departed;
    }
    pub(crate) fn withdraw(&self) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match &*state {
            Presentation::Presented { departure, .. } => departure.withdraw(),
            Presentation::NotPresented | Presentation::Withdrawn => {},
            Presentation::Departed => return,
        }
        *state = Presentation::Withdrawn;
    }
    fn state(&self) -> Presentation {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(&*state, Presentation::Presented { departure, .. } if departure.is_complete()) {
            *state = Presentation::Departed;
        }
        state.clone()
    }
}

/// End evidence freezes this invocation's rows. Only census transitions construct it.
#[derive(Clone, Debug)]
pub(crate) struct EndedInvocation {
    id:           InvocationId,
    rows:         Arc<[CargoProcess]>,
    at:           Instant,
    presentation: InvocationPresentation,
    published:    ScanNumber,
}

impl EndedInvocation {
    pub(crate) fn presentation(&self) -> EndedPresentation {
        match self.presentation.state() {
            Presentation::NotPresented => EndedPresentation::Unpresented,
            Presentation::Presented { site, .. } => EndedPresentation::Presented(site),
            Presentation::Withdrawn => EndedPresentation::Withdrawn,
            Presentation::Departed => EndedPresentation::Departed,
        }
    }
    pub(crate) const fn at(&self) -> Instant { self.at }
    pub(crate) fn withdraw(&self) { self.presentation.withdraw(); }
    /// An ended row can update its paint, but cannot renew a withdrawal.
    pub(crate) fn painted_process(&self, pid: u32, span: PaintedRowSpan<InvocationId>) {
        let mut state = self
            .presentation
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Presentation::Presented { spans, .. } = &mut *state {
            spans.insert(pid, span);
        }
    }
    pub(crate) const fn resolved(&self) -> &ResolvedInvocationId { self.presentation.resolved() }
    fn rows(&self) -> &[CargoProcess] { &self.rows }
    pub(crate) fn departed(&self) { self.presentation.departed(); }
    pub(crate) fn departure(&self, fade: Duration) -> EndedCellDeparture {
        let state = self.presentation.state();
        let departure =
            tui_pane::EndedCellDeparture::new(self.at, tui_pane::CellLifecycleTiming::new(fade));
        match state {
            Presentation::Presented {
                departure: receipt, ..
            } => departure.with_receipt(receipt),
            Presentation::NotPresented | Presentation::Withdrawn | Presentation::Departed => {
                departure
            },
        }
    }
    #[cfg(test)]
    pub(crate) fn fixture(row: CargoProcess, at: Instant) -> Self {
        Self {
            id: row.invocation_id.clone(),
            rows: vec![row.clone()].into(),
            at,
            presentation: InvocationPresentation::for_test(row.invocation_id),
            published: ScanNumber(0),
        }
    }
}

impl PartialEq for EndedInvocation {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.rows == other.rows && self.at == other.at
    }
}

/// A live row carries lifetime evidence for its own process, independent of its owner.
#[derive(Clone)]
struct InvocationProcessRow {
    process:  CargoProcess,
    lifetime: LifetimeEvidence,
}

impl InvocationProcessRow {
    fn observed(process: CargoProcess, census: &Census) -> Self {
        let lifetime = census
            .lifetimes
            .get(&Pid::from_u32(process.pid))
            .cloned()
            .unwrap_or(LifetimeEvidence::Unavailable);
        Self { process, lifetime }
    }

    fn refreshed(&self, process: CargoProcess, census: &Census) -> Self {
        let mut next = Self::observed(process, census);
        if next.process.pid == self.process.pid
            && matches!(next.lifetime, LifetimeEvidence::Unavailable)
        {
            next.lifetime = self.lifetime.clone();
        }
        next
    }
}

/// Only completed discovery observations advance the uncertain-absence bound.
#[derive(Clone, Copy, Default)]
struct MissingObservations(u64);

impl MissingObservations {
    const fn observe(
        self,
        evidence: &PublicationEvidence,
        cargo_readiness: CargoReadiness,
    ) -> Self {
        match (evidence, cargo_readiness) {
            (PublicationEvidence::Current, _)
            | (
                PublicationEvidence::Selected {
                    captures: CaptureFreshness::Current,
                    ..
                },
                CargoReadiness::AwaitingProcess,
            )
            | (
                PublicationEvidence::Selected {
                    processes: ProcessDiscoveryEvidence::Current(_),
                    ..
                },
                CargoReadiness::ObservedProcess,
            ) => Self(self.0.saturating_add(1)),
            (
                PublicationEvidence::Selected {
                    captures: CaptureFreshness::Retained,
                    ..
                },
                CargoReadiness::AwaitingProcess,
            )
            | (
                PublicationEvidence::Selected {
                    processes: ProcessDiscoveryEvidence::Retained,
                    ..
                },
                CargoReadiness::ObservedProcess,
            ) => self,
        }
    }
}

/// The only state machine that owns invocation presence, frozen rows and completion.
enum InvocationState {
    Live {
        row: InvocationProcessRow,
    },
    Missing {
        row:     InvocationProcessRow,
        absence: MissingObservations,
    },
    Ended(EndedInvocation),
}

impl InvocationState {
    fn rows(&self) -> &[CargoProcess] {
        match self {
            Self::Live { row } | Self::Missing { row, .. } => std::slice::from_ref(&row.process),
            Self::Ended(end) => end.rows(),
        }
    }
}

/// Native continuity is evidence owned by the invocation, never another lifecycle.
struct InvocationRecord {
    owner:           InvocationOwner,
    membership:      Membership,
    state:           InvocationState,
    processes:       Vec<(Pid, KnownCommandProcess)>,
    ancestry:        KnownAncestry,
    presentation:    InvocationPresentation,
    cargo_readiness: CargoReadiness,
}

/// One resolved process has one row and one family in a publication.
#[derive(Eq, Hash, PartialEq)]
struct InvocationProcessRowKey {
    invocation: ResolvedInvocationId,
    pid:        Pid,
}

/// Resource values can advance without changing a retained row's lifetime evidence.
struct InvocationProcessResources {
    own:     InvocationResourceUse,
    subtree: InvocationResourceUse,
}

struct FamilyProcessRow<'a> {
    root:    &'a InvocationId,
    process: &'a CargoProcess,
}

/// Key rows before family projection, including rows kept by an ended root.
struct InvocationFamilyRows<'a> {
    entries: HashMap<InvocationProcessRowKey, FamilyProcessRow<'a>>,
}

impl<'a> InvocationFamilyRows<'a> {
    fn of(records: &'a HashMap<InvocationId, InvocationRecord>) -> Self {
        let mut entries = HashMap::new();
        for record in records.values() {
            for row in record.state.rows() {
                // A retained family can hold an older copy of a member's row.
                // The member's own record supplies its receipt, values and root.
                let owner = records.get(&row.invocation_id).unwrap_or(record);
                let process = owner
                    .state
                    .rows()
                    .iter()
                    .find(|current| current.pid == row.pid)
                    .unwrap_or(row);
                let invocation = owner.presentation.resolved();
                let mut root = invocation.id();
                for _ in 0..PARENT_WALK_LIMIT {
                    let Some(parent) = records.get(root) else {
                        break;
                    };
                    let Membership::Member { lead } = &parent.membership else {
                        break;
                    };
                    if lead == invocation.id() || !records.contains_key(lead) {
                        break;
                    }
                    root = lead;
                }
                entries.insert(
                    InvocationProcessRowKey {
                        invocation: invocation.clone(),
                        pid:        Pid::from_u32(row.pid),
                    },
                    FamilyProcessRow { root, process },
                );
            }
        }
        Self { entries }
    }

    fn into_families(self) -> HashMap<InvocationId, Vec<CargoProcess>> {
        let mut families: HashMap<_, Vec<_>> = HashMap::new();
        for row in self.entries.into_values() {
            families
                .entry(row.root.clone())
                .or_default()
                .push(row.process.clone());
        }
        families
    }
}

#[derive(Debug)]
struct UnownedProcessRow;

#[derive(Debug)]
struct ExecutionEndUnproven;

impl InvocationRecord {
    /// Existing ownership permits resource replacement while metadata is pending.
    fn resource_key(
        &self,
        process: &CargoProcess,
    ) -> Result<InvocationProcessRowKey, UnownedProcessRow> {
        let row = match &self.state {
            InvocationState::Live { row } | InvocationState::Missing { row, .. } => row,
            InvocationState::Ended(_) => return Err(UnownedProcessRow),
        };
        if row.process.pid != process.pid {
            return Err(UnownedProcessRow);
        }
        Ok(InvocationProcessRowKey {
            invocation: self.presentation.resolved().clone(),
            pid:        Pid::from_u32(process.pid),
        })
    }

    /// Replace completed resources without supplying new lifetime evidence.
    fn refresh_resources(
        &mut self,
        resources: &mut HashMap<InvocationProcessRowKey, InvocationProcessResources>,
    ) {
        match &mut self.state {
            InvocationState::Live { row } | InvocationState::Missing { row, .. } => {
                let key = InvocationProcessRowKey {
                    invocation: self.presentation.resolved().clone(),
                    pid:        Pid::from_u32(row.process.pid),
                };
                if let Some(resources) = resources.remove(&key) {
                    row.process.resource_use = resources.own;
                    row.process.subtree_resource_use = resources.subtree;
                }
            },
            InvocationState::Ended(_) => {},
        }
    }

    fn end_proof(
        &self,
        evidence: &PublicationEvidence,
        capture: &Capture,
        presence: InvocationProbe,
        scans_missing: u64,
    ) -> Result<ExecutionEndProof, ExecutionEndUnproven> {
        match presence {
            InvocationProbe::Ended(subject) => Ok(ExecutionEndProof::Native(subject)),
            InvocationProbe::Alive => Err(ExecutionEndUnproven),
            InvocationProbe::Uncertain => {
                match (&self.cargo_readiness, &self.owner.identity, evidence) {
                    (
                        CargoReadiness::AwaitingProcess,
                        InvocationId::Captured(run),
                        PublicationEvidence::Current
                        | PublicationEvidence::Selected {
                            captures: CaptureFreshness::Current,
                            ..
                        },
                    ) if !capture.registered_pids().contains(&run.shim_pid) => {
                        Ok(ExecutionEndProof::Registration(run.clone()))
                    },
                    _ if scans_missing >= MISSING_SCAN_LIMIT => Ok(ExecutionEndProof::NotSeen),
                    _ => Err(ExecutionEndUnproven),
                }
            },
        }
    }
}

/// Registration proves an invocation before it identifies the process doing its work.
#[derive(Clone, Copy)]
enum CargoReadiness {
    AwaitingProcess,
    ObservedProcess,
}

impl CargoReadiness {
    fn of(row: &CargoProcess, census: &Census, capture: &Capture) -> Self {
        match capture.row_source(row.pid) {
            DirectAssociation::Direct(_)
                if !census.registered_cargo.contains(&Pid::from_u32(row.pid)) =>
            {
                Self::AwaitingProcess
            },
            DirectAssociation::Direct(_) | DirectAssociation::None => Self::ObservedProcess,
        }
    }
}

/// Exactly one persistent owning map; process-tree projections are scan-local.
#[derive(Default)]
struct Invocations {
    records:  HashMap<InvocationId, InvocationRecord>,
    scan:     ScanNumber,
    joined:   HashMap<InvocationId, InvocationId>,
    terminal: TerminalIdentities,
    bindings: InvocationProcessBindings,
}

/// Each native lifetime keeps the evidence that selected its execution.
#[derive(Clone)]
struct ProcessBindingEvidence {
    origin: InvocationOwner,
}

/// A proven role separates an execution from each process that forwards to it.
#[derive(Clone)]
enum InvocationProcessBinding {
    Executor {
        owner:    InvocationOwner,
        evidence: ProcessBindingEvidence,
    },
    Forwarder {
        owner:    InvocationOwner,
        evidence: ProcessBindingEvidence,
        executor: ProcessIdentity,
    },
    IndependentExecution {
        owner:    InvocationOwner,
        evidence: ProcessBindingEvidence,
        branch:   ProcessIdentity,
    },
}

impl InvocationProcessBinding {
    const fn owner(&self) -> &InvocationOwner {
        match self {
            Self::Executor { owner, .. }
            | Self::Forwarder { owner, .. }
            | Self::IndependentExecution { owner, .. } => owner,
        }
    }
    const fn evidence(&self) -> &ProcessBindingEvidence {
        match self {
            Self::Executor { evidence, .. }
            | Self::Forwarder { evidence, .. }
            | Self::IndependentExecution { evidence, .. } => evidence,
        }
    }
}

/// A proven end cannot become an active binding for the same native subject.
enum ProcessBindingState {
    Active(InvocationProcessBinding),
    Ended(ResolvedInvocationId),
}

/// Only the invocation map owns native lifetime bindings and their terminal facts.
#[derive(Default)]
struct InvocationProcessBindings {
    subjects:      HashMap<ProcessIdentity, ProcessBindingState>,
    registrations: HashMap<RunId, ProcessIdentity>,
}

impl InvocationProcessBindings {
    fn subject(&mut self, native: &InvocationId) -> ProcessIdentity {
        match native {
            InvocationId::Process(identity) => identity.clone(),
            InvocationId::Captured(run) => self
                .registrations
                .entry(run.clone())
                .or_insert_with(|| {
                    ProcessIdentity::observed(run.shim_pid, LifetimeEvidence::Unavailable)
                })
                .clone(),
        }
    }

    fn retain(&mut self, tree: &mut ProcessTreeContinuity) {
        let retained: HashSet<_> = tree
            .nodes
            .values()
            .filter_map(|node| match &node.native {
                InvocationId::Process(subject) => Some(subject.clone()),
                InvocationId::Captured(run) => self.registrations.get(run).cloned(),
            })
            .collect();
        self.subjects
            .retain(|subject, _| retained.contains(subject));
        self.registrations.retain(|run, _| {
            tree.nodes
                .values()
                .any(|node| node.native == InvocationId::Captured(run.clone()))
        });
        for (pid, mut node) in std::mem::take(&mut tree.nodes) {
            let subject = self.subject(&node.native);
            let previous = match &node.invocation.identity {
                InvocationId::Captured(run) => self
                    .registrations
                    .get(run)
                    .and_then(|key| self.subjects.get(key)),
                InvocationId::Process(_) => None,
            };
            let initial = match previous {
                Some(ProcessBindingState::Active(binding)) => binding.clone(),
                Some(ProcessBindingState::Ended(id)) => {
                    self.subjects
                        .insert(subject, ProcessBindingState::Ended(id.clone()));
                    continue;
                },
                None => InvocationProcessBinding::Executor {
                    owner:    node.invocation.clone(),
                    evidence: ProcessBindingEvidence {
                        origin: node.invocation.clone(),
                    },
                },
            };
            if let ProcessBindingState::Active(binding) = self
                .subjects
                .entry(subject)
                .or_insert(ProcessBindingState::Active(initial))
            {
                node.invocation.clone_from(binding.owner());
                tree.nodes.insert(pid, node);
            }
        }
    }

    fn split(&mut self, tree: &mut ProcessTreeContinuity, parent: Pid) {
        let Some(parent_node) = tree.nodes.get(&parent) else {
            return;
        };
        let owner = parent_node.invocation.identity.clone();
        let branch = self.subject(&parent_node.native);
        for node in tree
            .nodes
            .values_mut()
            .filter(|node| node.invocation.identity == owner)
        {
            let subject = self.subject(&node.native);
            if let Some(ProcessBindingState::Active(binding)) = self.subjects.get_mut(&subject) {
                let evidence = binding.evidence().clone();
                node.invocation.clone_from(&evidence.origin);
                *binding = InvocationProcessBinding::IndependentExecution {
                    owner: evidence.origin.clone(),
                    evidence,
                    branch: branch.clone(),
                };
            }
        }
    }

    fn commit(&mut self, tree: &ProcessTreeContinuity) {
        let parents: HashSet<_> = tree
            .nodes
            .iter()
            .filter_map(|(&child, candidate)| {
                let parent = tree.parent_of(child)?;
                (tree.nodes[&parent].invocation.identity == candidate.invocation.identity)
                    .then_some(parent)
            })
            .collect();
        let mut executors = HashMap::new();
        for (&pid, node) in &tree.nodes {
            if !parents.contains(&pid) {
                executors.insert(node.invocation.identity.clone(), self.subject(&node.native));
            }
        }
        let retained: HashSet<_> = tree
            .nodes
            .values()
            .filter_map(|node| match &node.native {
                InvocationId::Process(identity) => Some(identity.clone()),
                InvocationId::Captured(_) => None,
            })
            .collect();
        for state in self.subjects.values() {
            if let ProcessBindingState::Active(InvocationProcessBinding::Forwarder {
                owner,
                executor,
                ..
            }) = state
                && retained.contains(executor)
            {
                executors
                    .entry(owner.identity.clone())
                    .or_insert_with(|| executor.clone());
            }
        }
        for node in tree.nodes.values() {
            let subject = self.subject(&node.native);
            let Some(ProcessBindingState::Active(previous)) = self.subjects.get(&subject) else {
                continue;
            };
            let evidence = previous.evidence().clone();
            let binding = match executors.get(&node.invocation.identity) {
                Some(executor) if executor != &subject => InvocationProcessBinding::Forwarder {
                    owner: node.invocation.clone(),
                    evidence,
                    executor: executor.clone(),
                },
                _ => match previous {
                    InvocationProcessBinding::IndependentExecution { branch, .. } => {
                        InvocationProcessBinding::IndependentExecution {
                            owner: node.invocation.clone(),
                            evidence,
                            branch: branch.clone(),
                        }
                    },
                    _ => InvocationProcessBinding::Executor {
                        owner: node.invocation.clone(),
                        evidence,
                    },
                },
            };
            self.subjects
                .insert(subject, ProcessBindingState::Active(binding));
        }
    }

    fn resolve(
        &mut self,
        node: &KnownCommandProcess,
    ) -> Result<ResolvedInvocationId, UnresolvedInvocation> {
        let subject = self.subject(&node.native);
        match self.subjects.get(&subject) {
            Some(ProcessBindingState::Active(
                InvocationProcessBinding::Executor { owner, .. }
                | InvocationProcessBinding::IndependentExecution { owner, .. },
            )) => Ok(ResolvedInvocationId(owner.identity.clone())),
            Some(
                ProcessBindingState::Active(InvocationProcessBinding::Forwarder { .. })
                | ProcessBindingState::Ended(_),
            )
            | None => Err(UnresolvedInvocation),
        }
    }

    fn end_not_seen(
        &mut self,
        resolved: &ResolvedInvocationId,
        executor: Pid,
        terminal: &mut TerminalIdentities,
    ) {
        for (subject, state) in &self.subjects {
            if let ProcessBindingState::Active(
                InvocationProcessBinding::Executor { owner, .. }
                | InvocationProcessBinding::IndependentExecution { owner, .. },
            ) = state
                && &owner.identity == resolved.id()
            {
                let pid = match subject {
                    ProcessIdentity::Known { pid, .. }
                    | ProcessIdentity::Unavailable { pid, .. } => Pid::from_u32(*pid),
                };
                if pid == executor {
                    terminal.end(pid, &InvocationId::Process(subject.clone()));
                }
            }
        }
        terminal.end(executor, resolved.id());
        self.end(resolved);
    }

    fn end_subject(
        &mut self,
        resolved: &ResolvedInvocationId,
        subject: ProcessLifetimeSubject,
        terminal: &mut TerminalIdentities,
    ) {
        self.end_not_seen(resolved, subject.pid, terminal);
        if let LifetimeEvidence::Available(lifetime) = subject.lifetime {
            self.subjects.insert(
                ProcessIdentity::Known {
                    pid: subject.pid.as_u32(),
                    lifetime,
                },
                ProcessBindingState::Ended(resolved.clone()),
            );
        }
        self.end(resolved);
    }

    fn end_registration(&mut self, resolved: &ResolvedInvocationId, registration: RunId) {
        let identity = self.subject(&InvocationId::Captured(registration));
        self.subjects
            .insert(identity, ProcessBindingState::Ended(resolved.clone()));
        self.end(resolved);
    }

    fn end(&mut self, resolved: &ResolvedInvocationId) {
        for state in self.subjects.values_mut() {
            if matches!(state, ProcessBindingState::Active(binding) if &binding.owner().identity == resolved.id())
            {
                *state = ProcessBindingState::Ended(resolved.clone());
            }
        }
    }
}

/// Publication carries state values; omission never authorizes a departure.
#[derive(Clone, Debug)]
pub(crate) enum PublishedInvocation {
    Live(InvocationPresentation),
    AwaitingCargo(InvocationPresentation),
    Missing(InvocationPresentation),
    Ended(EndedInvocation),
    Joined { into: InvocationId },
}

impl PartialEq for PublishedInvocation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Live(_), Self::Live(_))
            | (Self::AwaitingCargo(_), Self::AwaitingCargo(_))
            | (Self::Missing(_), Self::Missing(_)) => true,
            (Self::Ended(a), Self::Ended(b)) => a == b,
            (Self::Joined { into: a }, Self::Joined { into: b }) => a == b,
            _ => false,
        }
    }
}

/// Admission evidence is borrowed only from an observed live publication.
#[derive(Clone, Debug)]
pub(crate) struct AdmissibleInvocation {
    presentation: InvocationPresentation,
}

impl AdmissibleInvocation {
    pub(crate) fn painted_process(
        &self,
        pid: u32,
        site: InvocationPaintSite,
        span: PaintedRowSpan<InvocationId>,
    ) {
        self.presentation.painted_process(pid, site, span);
    }
}

#[derive(Debug)]
pub(crate) struct InvocationNotAdmissible;

impl PublishedInvocation {
    pub(crate) fn admissible(&self) -> Result<AdmissibleInvocation, InvocationNotAdmissible> {
        match self {
            Self::Live(presentation) => Ok(AdmissibleInvocation {
                presentation: presentation.clone(),
            }),
            Self::AwaitingCargo(_) | Self::Missing(_) | Self::Ended(_) | Self::Joined { .. } => {
                Err(InvocationNotAdmissible)
            },
        }
    }
}

/// Borrowed live row evidence; ended and missing rows cannot refresh display rows.
#[derive(Clone, Copy)]
pub(crate) struct LiveInvocation<'a> {
    pub(crate) row: &'a CargoProcess,
    publication:    LivePublication<'a>,
}

/// Refreshable rows retain whether the cargo process has been identified.
#[derive(Clone, Copy)]
enum LivePublication<'a> {
    Cargo(&'a InvocationPresentation),
    AwaitingCargo(&'a InvocationPresentation),
}

impl LiveInvocation<'_> {
    pub(crate) fn published(&self) -> PublishedInvocation {
        match self.publication {
            LivePublication::Cargo(presentation) => PublishedInvocation::Live(presentation.clone()),
            LivePublication::AwaitingCargo(presentation) => {
                PublishedInvocation::AwaitingCargo(presentation.clone())
            },
        }
    }
}

/// A Missing publication can replace resources without supplying live-row evidence.
#[derive(Clone, Copy)]
pub(crate) struct RetainedInvocationResources<'a> {
    pub(crate) own:     &'a InvocationResourceUse,
    pub(crate) subtree: &'a InvocationResourceUse,
}

/// Every retained invocation is published once, alongside membership-derived groups.
#[derive(Clone, Default, Debug, PartialEq)]
pub(crate) struct PublishedInvocations {
    groups: Vec<CargoGroup>,
    states: HashMap<InvocationId, PublishedInvocation>,
}

#[derive(Debug)]
pub(crate) struct RetainedResourcesUnavailable;

#[derive(Debug)]
pub(crate) struct LivePublicationUnavailable;

impl PublishedInvocations {
    pub(crate) fn groups(&self) -> &[CargoGroup] { &self.groups }
    pub(crate) fn state(&self, id: &InvocationId) -> Option<&PublishedInvocation> {
        self.states.get(id)
    }

    pub(crate) fn retained_resources<'a>(
        &self,
        row: &'a CargoProcess,
    ) -> Result<RetainedInvocationResources<'a>, RetainedResourcesUnavailable> {
        match self.states.get(&row.invocation_id) {
            Some(PublishedInvocation::Missing(_)) => Ok(RetainedInvocationResources {
                own:     &row.resource_use,
                subtree: &row.subtree_resource_use,
            }),
            Some(
                PublishedInvocation::Live(_)
                | PublishedInvocation::AwaitingCargo(_)
                | PublishedInvocation::Ended(_)
                | PublishedInvocation::Joined { .. },
            )
            | None => Err(RetainedResourcesUnavailable),
        }
    }

    pub(crate) fn live<'a>(
        &'a self,
        row: &'a CargoProcess,
    ) -> Result<LiveInvocation<'a>, LivePublicationUnavailable> {
        match self.states.get(&row.invocation_id) {
            Some(PublishedInvocation::Live(presentation)) => Ok(LiveInvocation {
                row,
                publication: LivePublication::Cargo(presentation),
            }),
            Some(PublishedInvocation::AwaitingCargo(presentation)) => Ok(LiveInvocation {
                row,
                publication: LivePublication::AwaitingCargo(presentation),
            }),
            Some(
                PublishedInvocation::Missing(_)
                | PublishedInvocation::Ended(_)
                | PublishedInvocation::Joined { .. },
            )
            | None => Err(LivePublicationUnavailable),
        }
    }

    /// Synthetic publications share the same row receipts as their previous publication.
    #[cfg(test)]
    pub(crate) fn with_fixture_receipts(mut self, previous: &Self) -> Self {
        for (id, state) in &mut self.states {
            let Some(prior) = previous.states.get(id) else {
                continue;
            };
            let presentation = match prior {
                PublishedInvocation::Live(presentation)
                | PublishedInvocation::AwaitingCargo(presentation)
                | PublishedInvocation::Missing(presentation) => presentation,
                PublishedInvocation::Ended(end) => &end.presentation,
                PublishedInvocation::Joined { .. } => continue,
            };
            match state {
                PublishedInvocation::Live(current)
                | PublishedInvocation::AwaitingCargo(current)
                | PublishedInvocation::Missing(current) => *current = presentation.clone(),
                PublishedInvocation::Ended(current) => {
                    if let PublishedInvocation::Ended(prior_end) = prior {
                        *current = prior_end.clone();
                    } else {
                        current.presentation = presentation.clone();
                    }
                },
                PublishedInvocation::Joined { .. } => {},
            }
        }
        self
    }

    #[cfg(test)]
    pub(crate) fn fixture(
        groups: Vec<CargoGroup>,
        missing: &[InvocationId],
        previous: &[CargoGroup],
        at: Instant,
    ) -> Self {
        let mut states = HashMap::new();
        for row in groups
            .iter()
            .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
        {
            states.insert(
                row.invocation_id.clone(),
                PublishedInvocation::Live(InvocationPresentation::for_test(
                    row.invocation_id.clone(),
                )),
            );
        }
        let mut retained = groups;
        for group in previous {
            for row in std::iter::once(&group.lead).chain(&group.rest) {
                states.entry(row.invocation_id.clone()).or_insert_with(|| {
                    if missing.contains(&row.invocation_id) {
                        PublishedInvocation::Missing(InvocationPresentation::for_test(
                            row.invocation_id.clone(),
                        ))
                    } else {
                        PublishedInvocation::Ended(EndedInvocation {
                            id: row.invocation_id.clone(),
                            rows: vec![row.clone()].into(),
                            at,
                            presentation: InvocationPresentation::for_test(
                                row.invocation_id.clone(),
                            ),
                            published: ScanNumber(0),
                        })
                    }
                });
            }
            let elsewhere: HashSet<_> = retained
                .iter()
                .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
                .map(|row| row.invocation_id.clone())
                .collect();
            if let Some(current) = retained
                .iter_mut()
                .find(|candidate| candidate.id() == group.id())
            {
                current.rest.extend(
                    group
                        .rest
                        .iter()
                        .filter(|row| !elsewhere.contains(&row.invocation_id))
                        .cloned(),
                );
            } else if !elsewhere.contains(&group.id()) {
                let mut held = group.clone();
                held.rest
                    .retain(|row| !elsewhere.contains(&row.invocation_id));
                retained.push(held);
            }
        }
        Self {
            groups: retained,
            states,
        }
    }
}

/// A process lifetime can be checked only against the kernel read for its own pid.
struct ProcessLifetimeSubject {
    pid:                Pid,
    lifetime:           LifetimeEvidence,
    kernel_observation: KernelObservation,
}

impl ProcessLifetimeSubject {
    fn observe(row: &InvocationProcessRow) -> Self {
        let pid = Pid::from_u32(row.process.pid);
        Self {
            pid,
            lifetime: row.lifetime.clone(),
            kernel_observation: birth_stamp::observe(pid.as_u32()),
        }
    }
    fn compare(self, current: &LifetimeEvidence) -> InvocationProbe {
        match NativeLifetimeComparison::between(&self.lifetime, current) {
            NativeLifetimeComparison::Different => InvocationProbe::Ended(self),
            native => {
                let verification = match &self.lifetime {
                    LifetimeEvidence::Available(lifetime) => {
                        lifetime.verify_process(self.pid.as_u32(), &self.kernel_observation)
                    },
                    LifetimeEvidence::Unavailable => Verification::Unknown,
                };
                match verification {
                    Verification::Ended => InvocationProbe::Ended(self),
                    Verification::Confirmed if matches!(native, NativeLifetimeComparison::Same) => {
                        InvocationProbe::Alive
                    },
                    Verification::Confirmed | Verification::Unknown => InvocationProbe::Uncertain,
                }
            },
        }
    }
}

/// An end proof belongs to the resolved execution and names its own subject.
enum ExecutionEndProof {
    Native(ProcessLifetimeSubject),
    Registration(RunId),
    NotSeen,
}

struct ResolvedExecutionEnd {
    execution: ResolvedInvocationId,
    proof:     ExecutionEndProof,
}

/// Kernel end proof includes zombies; inconclusive probes have a bounded Missing state.
enum InvocationProbe {
    Alive,
    Ended(ProcessLifetimeSubject),
    Uncertain,
}

/// Native comparison does not authorize an end without its process subject.
enum NativeLifetimeComparison {
    Same,
    Different,
    Unknown,
}

impl NativeLifetimeComparison {
    fn between(previous: &LifetimeEvidence, current: &LifetimeEvidence) -> Self {
        match (previous, current) {
            (LifetimeEvidence::Available(previous), LifetimeEvidence::Available(current)) => {
                if previous == current {
                    Self::Same
                } else {
                    Self::Different
                }
            },
            _ => Self::Unknown,
        }
    }
}

impl InvocationProbe {
    fn kernel(subject: ProcessLifetimeSubject) -> Self {
        let pid = subject.pid;
        if process_identity::ProvenProcessEnd::probe(pid).is_ok() {
            return Self::Ended(subject);
        }
        #[cfg(target_os = "linux")]
        match fs::read_to_string(
            Path::new(BIRTH_PROC_DIRECTORY)
                .join(pid.to_string())
                .join(BIRTH_STAT_FILENAME),
        ) {
            Ok(stat)
                if stat
                    .rsplit_once(BIRTH_STAT_COMM_SEPARATOR)
                    .is_some_and(|(_, tail)| tail.starts_with(BIRTH_STAT_ZOMBIE_STATE)) =>
            {
                return Self::Ended(subject);
            },
            Err(error) if error.kind() == ErrorKind::NotFound => return Self::Ended(subject),
            Ok(_) | Err(_) => {},
        }
        #[cfg(target_os = "macos")]
        {
            let mut snapshot = System::new();
            snapshot.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::nothing().without_tasks(),
            );
            if snapshot
                .process(pid)
                .is_some_and(|process| process.status() == ProcessStatus::Zombie)
            {
                return Self::Ended(subject);
            }
        }
        subject.compare(&birth_stamp::lifetime(pid.as_u32()))
    }
}

impl Invocations {
    /// Retain terminal facts by their native subjects and bound aliases, not displayed pids.
    fn retain_identity_evidence(&mut self, observations: impl Iterator<Item = InvocationId>) {
        let mut evidence: HashSet<_> = observations.collect();
        for (subject, binding) in &self.bindings.subjects {
            evidence.insert(InvocationId::Process(subject.clone()));
            match binding {
                ProcessBindingState::Active(binding) => {
                    evidence.insert(binding.owner().identity.clone());
                    evidence.insert(binding.evidence().origin.identity.clone());
                },
                ProcessBindingState::Ended(owner) => {
                    evidence.insert(owner.id().clone());
                },
            }
        }
        for (run, subject) in &self.bindings.registrations {
            evidence.insert(InvocationId::Captured(run.clone()));
            evidence.insert(InvocationId::Process(subject.clone()));
        }
        for (id, record) in &self.records {
            evidence.insert(id.clone());
            for (_, process) in &record.processes {
                evidence.insert(process.native.clone());
            }
        }
        self.terminal.retain(&evidence);
        self.joined.retain(|alias, _| evidence.contains(alias));
    }

    fn identify(
        &mut self,
        census: &mut Census,
        observations: &ProcessObservations<'_>,
        capture: &Capture,
    ) -> ProcessTreeContinuity {
        self.scan.0 = self.scan.0.saturating_add(1);
        self.records.retain(|_, record| match &record.state {
            InvocationState::Ended(end) => match end.presentation.state() {
                Presentation::Presented { .. } => true,
                Presentation::NotPresented => end.published >= self.scan,
                Presentation::Withdrawn | Presentation::Departed => false,
            },
            InvocationState::Live { .. } | InvocationState::Missing { .. } => true,
        });
        let mut tree = ProcessTreeContinuity {
            scan: self.scan.0.saturating_sub(1),
            ..ProcessTreeContinuity::default()
        };
        for (pid, identity) in &mut census.identities {
            *identity = self.terminal.subsequent(*pid, identity);
        }
        for (id, record) in &self.records {
            if matches!(record.state, InvocationState::Ended(_)) {
                continue;
            }
            tree.ancestry.insert(id.clone(), record.ancestry.clone());
            for (pid, process) in &record.processes {
                tree.nodes.insert(*pid, process.clone());
            }
        }
        tree.identify(census, observations, capture, &mut self.bindings);
        for (pid, node) in &mut tree.nodes {
            let identity = self.terminal.subsequent(*pid, &node.invocation.identity);
            if identity != node.invocation.identity {
                let owner = InvocationOwner {
                    identity:   identity.clone(),
                    first_scan: tree.scan,
                };
                node.invocation = owner;
            }
            census.identities.insert(*pid, identity);
        }
        self.records.retain(|id, _| !tree.joined.contains_key(id));
        self.joined.extend(tree.joined.clone());
        self.joined.retain(|id, _| !self.records.contains_key(id));
        tree
    }

    /// A newly observed row can restore Missing, but never change a terminal end.
    fn observe_live_row(
        &mut self,
        resolved: ResolvedInvocationId,
        mut row: CargoProcess,
        census: &Census,
        tree: &ProcessTreeContinuity,
        capture: &Capture,
    ) {
        row.invocation_id.clone_from(resolved.id());
        let id = resolved.id().clone();
        let membership = match &row.parent {
            VisibleParent::Invocation { id, .. } => Membership::Member { lead: id.clone() },
            VisibleParent::Ancestor(_) | VisibleParent::None => Membership::Lead,
        };
        if let Some(record) = self.records.get_mut(&id) {
            if let InvocationState::Live { row: previous }
            | InvocationState::Missing { row: previous, .. } = &record.state
            {
                record.cargo_readiness = CargoReadiness::of(&row, census, capture);
                record.state = InvocationState::Live {
                    row: previous.refreshed(row, census),
                };
            }
        } else if let Some(node) = tree
            .nodes
            .values()
            .find(|node| node.invocation.identity == id)
        {
            self.records.insert(
                id,
                InvocationRecord {
                    owner: node.invocation.clone(),
                    membership,
                    state: InvocationState::Live {
                        row: InvocationProcessRow::observed(row.clone(), census),
                    },
                    processes: Vec::new(),
                    ancestry: KnownAncestry {
                        links:     Vec::new(),
                        lifetimes: Vec::new(),
                        walk:      AncestryWalk::Ended(Vec::new()),
                    },
                    presentation: InvocationPresentation::new(resolved),
                    cargo_readiness: CargoReadiness::of(&row, census, capture),
                },
            );
        }
    }

    fn publish(
        &mut self,
        census: &Census,
        tree: &ProcessTreeContinuity,
        groups: Vec<CargoGroup>,
        capture: &Capture,
        now: Instant,
        mut probe: impl FnMut(&InvocationProcessRow) -> InvocationProbe,
    ) -> PublishedInvocations {
        let mut observed = HashSet::new();
        let mut ancestry = HashMap::new();
        let mut resources = HashMap::new();
        for group in groups {
            ancestry.insert(group.id(), group.ancestry.clone());
            for row in std::iter::once(group.lead).chain(group.rest) {
                if let Some(record) = self.records.get(&row.invocation_id)
                    && let Ok(key) = record.resource_key(&row)
                {
                    resources.insert(
                        key,
                        InvocationProcessResources {
                            own:     row.resource_use.clone(),
                            subtree: row.subtree_resource_use.clone(),
                        },
                    );
                }
                let Some(node) = tree.nodes.get(&Pid::from_u32(row.pid)) else {
                    continue;
                };
                let Ok(resolved) = self.bindings.resolve(node) else {
                    continue;
                };
                if census.publication_evidence.permits(&row) {
                    observed.insert(resolved.id().clone());
                    self.observe_live_row(resolved, row, census, tree, capture);
                }
            }
        }
        let mut process_evidence: HashMap<InvocationId, Vec<(Pid, KnownCommandProcess)>> =
            HashMap::new();
        for (pid, node) in &tree.nodes {
            process_evidence
                .entry(node.invocation.identity.clone())
                .or_default()
                .push((*pid, node.clone()));
        }
        for (id, record) in &mut self.records {
            if matches!(record.state, InvocationState::Ended(_)) {
                continue;
            }
            record.refresh_resources(&mut resources);
            if let Some(known) = tree.ancestry.get(id) {
                record.ancestry = known.clone();
            } else if let Some(walk) = ancestry.remove(id) {
                record.ancestry.walk = walk;
            }
            record.processes = process_evidence.remove(id).unwrap_or_default();
            if observed.contains(id) {
                continue;
            }
            let (row, absence) = match &record.state {
                InvocationState::Live { row } => (row.clone(), MissingObservations::default()),
                InvocationState::Missing { row, absence } => (row.clone(), *absence),
                InvocationState::Ended(_) => continue,
            };
            let pid = Pid::from_u32(row.process.pid);
            let absence = absence.observe(&census.publication_evidence, record.cargo_readiness);
            let presence = census.process_presence(&row, &mut probe);
            let proof =
                record.end_proof(&census.publication_evidence, capture, presence, absence.0);
            if let Ok(proof) = proof {
                let ended = ResolvedExecutionEnd {
                    execution: record.presentation.resolved().clone(),
                    proof,
                };
                probe::invocation_ended(id, now);
                match ended.proof {
                    ExecutionEndProof::NotSeen => {
                        self.bindings
                            .end_not_seen(&ended.execution, pid, &mut self.terminal);
                    },
                    ExecutionEndProof::Native(subject) => {
                        self.bindings
                            .end_subject(&ended.execution, subject, &mut self.terminal);
                    },
                    ExecutionEndProof::Registration(registration) => {
                        self.bindings
                            .end_registration(&ended.execution, registration);
                    },
                }
                record.state = InvocationState::Ended(EndedInvocation {
                    id:           record.owner.identity.clone(),
                    rows:         vec![row.process].into(),
                    at:           now,
                    presentation: record.presentation.clone(),
                    published:    self.scan,
                });
            } else {
                record.state = InvocationState::Missing { row, absence };
            }
        }
        self.reconcile_memberships(tree);
        self.publication()
    }

    fn reconcile_memberships(&mut self, tree: &ProcessTreeContinuity) {
        let owners: HashMap<_, _> = self
            .records
            .iter()
            .flat_map(|(id, record)| {
                record
                    .state
                    .rows()
                    .iter()
                    .map(move |row| (Pid::from_u32(row.pid), id.clone()))
            })
            .collect();
        let active: HashSet<_> = self
            .records
            .iter()
            .filter(|(_, record)| !matches!(record.state, InvocationState::Ended(_)))
            .map(|(id, _)| id.clone())
            .collect();
        for (id, record) in &mut self.records {
            if matches!(record.state, InvocationState::Ended(_)) {
                continue;
            }
            if matches!(&record.membership, Membership::Member { lead } if active.contains(lead)) {
                continue;
            }
            let mut membership = Membership::Lead;
            if let Some(row) = record.state.rows().first() {
                let mut walking = Pid::from_u32(row.pid);
                for _ in 0..PARENT_WALK_LIMIT {
                    let Some(parent) = tree.parent_of(walking) else {
                        break;
                    };
                    if let Some(lead) = owners.get(&parent)
                        && lead != id
                        && active.contains(lead)
                    {
                        membership = Membership::Member { lead: lead.clone() };
                        break;
                    }
                    walking = parent;
                }
            }
            record.membership = membership;
        }
    }

    fn publication(&self) -> PublishedInvocations {
        let mut states: HashMap<_, _> = self
            .joined
            .iter()
            .map(|(id, into)| {
                (
                    id.clone(),
                    PublishedInvocation::Joined { into: into.clone() },
                )
            })
            .collect();
        for (id, record) in &self.records {
            states.insert(
                id.clone(),
                match &record.state {
                    InvocationState::Live { .. } => match record.cargo_readiness {
                        CargoReadiness::AwaitingProcess => {
                            PublishedInvocation::AwaitingCargo(record.presentation.clone())
                        },
                        CargoReadiness::ObservedProcess => {
                            PublishedInvocation::Live(record.presentation.clone())
                        },
                    },
                    InvocationState::Missing { .. } => {
                        PublishedInvocation::Missing(record.presentation.clone())
                    },
                    InvocationState::Ended(end) => PublishedInvocation::Ended(end.clone()),
                },
            );
        }
        let mut groups = Vec::new();
        for (id, mut rows) in InvocationFamilyRows::of(&self.records).into_families() {
            let Some(index) = rows.iter().position(|row| row.invocation_id == id) else {
                continue;
            };
            let lead = rows.remove(index);
            rows.sort_by(newest_first);
            groups.push(CargoGroup {
                lead,
                rest: rows,
                ancestry: self.records[&id].ancestry.walk.clone(),
            });
        }
        groups.sort_by(|a, b| newest_first(&a.lead, &b.lead));
        PublishedInvocations { groups, states }
    }
}

/// The last proven process tree owns invocation continuity across scanner refreshes.
#[derive(Default)]
struct ProcessTreeContinuity {
    nodes:    HashMap<Pid, KnownCommandProcess>,
    ancestry: HashMap<InvocationId, KnownAncestry>,
    joined:   HashMap<InvocationId, InvocationId>,
    scan:     u64,
}

#[derive(Clone)]
struct KnownCommandProcess {
    native:     InvocationId,
    invocation: InvocationOwner,
    forwarding: ForwardingHistory,
    argv:       Vec<OsString>,
    directory:  ProcessField<PathBuf>,
    parent:     ParentProcessLink,
}

impl KnownCommandProcess {
    fn adopt_native(&mut self, proof: LifetimeMatch) { self.native = proof.into_native(); }
}

/// Identity and discovery age move as one value across every forwarding edge.
#[derive(Clone)]
struct InvocationOwner {
    identity:   InvocationId,
    first_scan: u64,
}

/// Branching is evidence about this native parent's entire lifetime.
#[derive(Clone, Copy, Default)]
enum ForwardingHistory {
    #[default]
    Unproven,
    Branching,
}

/// A retained edge names the native parent lifetime, never just a reusable PID.
#[derive(Clone)]
enum ParentProcessLink {
    Unreadable,
    Known { pid: Pid, native: InvocationId },
}

#[derive(Clone, Copy)]
struct ForwardingProcess<'a> {
    pid:       Pid,
    argv:      &'a [OsString],
    directory: WorkingDirectoryObservation<'a>,
}

enum Forwarding {
    Independent,
    SingleChild(ForwardingEdge),
}

/// One compatible child and its observed parent establish a forwarding join.
struct ForwardingEdge {
    parent: Pid,
    child:  Pid,
}

/// Both pruning and persistent ownership require exactly one compatible cargo child.
fn classify_forwarding<'a>(
    parent: ForwardingProcess<'a>,
    mut children: impl Iterator<Item = ForwardingProcess<'a>>,
) -> Forwarding {
    let Some(child) = children.next() else {
        return Forwarding::Independent;
    };
    if children.next().is_some() {
        return Forwarding::Independent;
    }
    if matches!(
        process_identity::dispatched_process(parent.argv, child.argv, child.pid),
        DispatchedProcess::Matched { .. }
    ) {
        return Forwarding::SingleChild(ForwardingEdge {
            parent: parent.pid,
            child:  child.pid,
        });
    }
    if child
        .argv
        .first()
        .and_then(|program| Path::new(program).file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            name.starts_with(CARGO_SUBCOMMAND_PREFIX) && !CARGO_PROCESS_NAMES.contains(&name)
        })
    {
        return Forwarding::Independent;
    }
    if observed_shim_match(parent.argv, parent.directory, child.argv, child.directory) {
        Forwarding::SingleChild(ForwardingEdge {
            parent: parent.pid,
            child:  child.pid,
        })
    } else {
        Forwarding::Independent
    }
}

/// Missing metadata cannot contradict the last observed parent links.
#[derive(Clone)]
struct KnownAncestry {
    links:     Vec<(Pid, Pid)>,
    lifetimes: Vec<(Pid, InvocationId)>,
    walk:      AncestryWalk<Ancestor>,
}

/// Current native processes and a matching shim birth prove one captured execution.
struct LiveCaptureBinding {
    owner:     InvocationOwner,
    forwarder: InvocationId,
    executor:  InvocationId,
}

struct UnprovenCaptureBinding;

impl LiveCaptureBinding {
    fn prove(
        tree: &ProcessTreeContinuity,
        census: &Census,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        edge: &ForwardingEdge,
    ) -> Result<Self, UnprovenCaptureBinding> {
        if !census.publication_evidence.has_current_capture()
            || [edge.parent, edge.child].into_iter().any(|pid| {
                !census.publication_evidence.has_current_process(pid)
                    || !records
                        .process(pid)
                        .is_some_and(|process| matches!(process.status, NativeProcessStatus::Live))
            })
        {
            return Err(UnprovenCaptureBinding);
        }
        let confirmed = capture
            .confirmed()
            .iter()
            .find(|confirmed| confirmed.key.pid == edge.parent.as_u32())
            .ok_or(UnprovenCaptureBinding)?;
        let run = RunId::verified(&confirmed.key, &confirmed.registration);
        let birth = census
            .lifetimes
            .get(&edge.parent)
            .ok_or(UnprovenCaptureBinding)?;
        LifetimeMatch::prove(&run, birth).map_err(|_| UnprovenCaptureBinding)?;
        let outer = &tree.nodes[&edge.parent];
        let inner = &tree.nodes[&edge.child];
        if !matches!(inner.invocation.identity, InvocationId::Process(_)) {
            return Err(UnprovenCaptureBinding);
        }
        let identity = match &outer.invocation.identity {
            InvocationId::Captured(_) => outer.invocation.identity.clone(),
            InvocationId::Process(_) => InvocationId::Captured(run),
        };
        Ok(Self {
            owner:     InvocationOwner {
                identity,
                first_scan: outer.invocation.first_scan.min(inner.invocation.first_scan),
            },
            forwarder: outer.invocation.identity.clone(),
            executor:  inner.invocation.identity.clone(),
        })
    }

    fn handovers(self) -> [InvocationHandover; 2] {
        [
            InvocationHandover {
                keep:   self.owner.clone(),
                retire: self.forwarder,
            },
            InvocationHandover {
                keep:   self.owner,
                retire: self.executor,
            },
        ]
    }
}

/// Only a proven forwarding edge or live capture binding can merge invocation owners.
struct InvocationHandover {
    keep:   InvocationOwner,
    retire: InvocationId,
}

impl InvocationHandover {
    fn observed(tree: &ProcessTreeContinuity, edge: &ForwardingEdge) -> Self {
        let outer = &tree.nodes[&edge.parent];
        let inner = &tree.nodes[&edge.child];
        let (keep, retire) = match outer
            .invocation
            .first_scan
            .cmp(&inner.invocation.first_scan)
        {
            Ordering::Less => (outer, inner),
            Ordering::Greater => (inner, outer),
            Ordering::Equal => match (&outer.invocation.identity, &inner.invocation.identity) {
                (InvocationId::Captured(_), _)
                | (InvocationId::Process(_), InvocationId::Process(_)) => (outer, inner),
                (_, InvocationId::Captured(_)) => (inner, outer),
            },
        };
        Self {
            keep:   keep.invocation.clone(),
            retire: retire.invocation.identity.clone(),
        }
    }
}

impl ProcessTreeContinuity {
    fn identify(
        &mut self,
        census: &mut Census,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        bindings: &mut InvocationProcessBindings,
    ) {
        self.scan = self.scan.saturating_add(1);
        census.include_registered_processes(records, capture);
        census.include_dispatched_processes(records);
        self.retain_observed_processes(census, records);
        self.refresh_published_owners(census, capture);
        bindings.retain(self);
        // Row selection must follow the same lifetime-checked edges as ownership.
        // An unavailable parent read cannot make its forwarding shim the lead again.
        for (&pid, record) in records.processes() {
            if matches!(record.parent, ProcessField::Unavailable)
                && let Some(parent) = self.parent_of(pid)
            {
                census.parents.insert(pid, parent);
            }
        }
        let mut children: HashMap<Pid, HashSet<Pid>> = HashMap::new();
        for &child in &census.cargo {
            let mut forwarding_child = child;
            let mut walking = child;
            let mut seen = HashSet::from([child]);
            for _ in 0..PARENT_WALK_LIMIT {
                let Some(parent) = self.parent_of(walking) else {
                    break;
                };
                if !seen.insert(parent) {
                    break;
                }
                let node = &self.nodes[&parent];
                if census.cargo.contains(&parent)
                    || command_text::cargo_split(&node.argv)
                        .is_ok_and(|arguments| arguments.follows_cargo_executable())
                {
                    // Detail reads can reveal pipeline shells that discovery did
                    // not classify as cargo. Keep every edge to the outer shim.
                    children.entry(parent).or_default().insert(forwarding_child);
                    forwarding_child = parent;
                } else {
                    break;
                }
                walking = parent;
            }
        }
        for (&parent, children) in &children {
            if children.len() > 1 {
                bindings.split(self, parent);
                if let Some(node) = self.nodes.get_mut(&parent) {
                    node.forwarding = ForwardingHistory::Branching;
                }
            }
        }
        let mut ordered = census.cargo.clone();
        ordered.sort_by_key(|pid| census.ancestor_pids(*pid).as_slice().len());
        for parent in ordered.into_iter().chain(
            children
                .keys()
                .copied()
                .filter(|pid| !census.cargo.contains(pid)),
        ) {
            let Some(node) = self.nodes.get(&parent) else {
                continue;
            };
            if matches!(node.forwarding, ForwardingHistory::Branching) {
                continue;
            }
            let Some(outer) = self.forwarding_process(parent) else {
                continue;
            };
            let children = children
                .get(&parent)
                .into_iter()
                .flatten()
                .filter_map(|&child| self.forwarding_process(child));
            if let Forwarding::SingleChild(edge) = classify_forwarding(outer, children) {
                if let Ok(binding) =
                    LiveCaptureBinding::prove(self, census, records, capture, &edge)
                {
                    for handover in binding.handovers() {
                        self.unify(&handover);
                    }
                } else {
                    let handover = InvocationHandover::observed(self, &edge);
                    self.unify(&handover);
                }
            }
        }
        for (&pid, identity) in &mut census.identities {
            if let Some(node) = self.nodes.get(&pid) {
                identity.clone_from(&node.invocation.identity);
            }
        }
        bindings.commit(self);
        census.identity_source = InvocationIdentitySource::RetainedTree;
    }

    /// Apply publication metadata without replacing an owner seen in an earlier scan.
    fn refresh_published_owners(&mut self, census: &mut Census, capture: &Capture) {
        // The first observation chooses an owner. A later publication supplies
        // metadata for that owner; it cannot replace an already visible native
        // invocation with a newly captured identity.
        for confirmed in capture.confirmed() {
            let pid = Pid::from_u32(confirmed.key.pid);
            let run = RunId::verified(&confirmed.key, &confirmed.registration);
            let captured = InvocationId::Captured(run.clone());
            if !self.nodes.contains_key(&pid) {
                let owner = InvocationOwner {
                    identity:   captured.clone(),
                    first_scan: self.scan,
                };
                self.nodes.insert(
                    pid,
                    KnownCommandProcess {
                        native:     census
                            .identities
                            .get(&pid)
                            .cloned()
                            .unwrap_or_else(|| captured.clone()),
                        invocation: owner.clone(),
                        forwarding: ForwardingHistory::Unproven,
                        argv:       registration_argv(confirmed.registration.record()),
                        directory:  ProcessField::Observed(
                            confirmed.registration.record().directory().to_owned(),
                        ),
                        parent:     ParentProcessLink::Unreadable,
                    },
                );
                census.identities.insert(pid, captured);
                continue;
            }
            if let Some(node) = self.nodes.get_mut(&pid)
                && census
                    .lifetimes
                    .get(&pid)
                    .is_some_and(|evidence| LifetimeMatch::prove(&run, evidence).is_ok())
                && (node.argv.is_empty()
                    || forwards_capture_command(&node.argv, confirmed.registration.record()))
            {
                // Verified registration evidence fills missing native metadata for
                // this same lifetime, so deadline-limited reads cannot split its
                // forwarding edge into a second invocation.
                if node.argv.is_empty() {
                    node.argv = registration_argv(confirmed.registration.record());
                }
                if matches!(node.directory, ProcessField::Unavailable) {
                    node.directory = ProcessField::Observed(
                        confirmed.registration.record().directory().to_owned(),
                    );
                }
                if node.invocation.first_scan == self.scan {
                    let owner = InvocationOwner {
                        identity:   captured,
                        first_scan: self.scan,
                    };
                    node.invocation = owner;
                }
            }
        }
    }

    /// Refresh readable node evidence while retaining unreadable parent metadata.
    fn retain_observed_processes(&mut self, census: &Census, records: &ProcessObservations<'_>) {
        for (&pid, record) in records.processes() {
            let Some(native) = census.identities.get(&pid) else {
                continue;
            };
            let observed_parent = match record.parent {
                ProcessField::Unavailable => ParentProcessLink::Unreadable,
                ProcessField::Observed(parent) => census
                    .identities
                    .get(&parent)
                    .or_else(|| self.nodes.get(&parent).map(|node| &node.native))
                    .map_or(ParentProcessLink::Unreadable, |native| {
                        ParentProcessLink::Known {
                            pid:    parent,
                            native: native.clone(),
                        }
                    }),
            };
            let node = self.nodes.entry(pid).or_insert_with(|| {
                let owner = InvocationOwner {
                    identity:   native.clone(),
                    first_scan: self.scan,
                };
                KnownCommandProcess {
                    native:     native.clone(),
                    invocation: owner,
                    forwarding: ForwardingHistory::Unproven,
                    argv:       Vec::new(),
                    directory:  ProcessField::Unavailable,
                    parent:     ParentProcessLink::Unreadable,
                }
            });
            if let InvocationId::Captured(run) = &node.native
                && run.shim_pid == pid.as_u32()
                && let Some(evidence) = census.lifetimes.get(&pid)
                && let Ok(proof) = LifetimeMatch::prove(run, evidence)
            {
                node.adopt_native(proof);
            }
            if &node.native != native {
                let owner = InvocationOwner {
                    identity:   native.clone(),
                    first_scan: self.scan,
                };
                *node = KnownCommandProcess {
                    native:     native.clone(),
                    invocation: owner.clone(),
                    forwarding: ForwardingHistory::Unproven,
                    argv:       Vec::new(),
                    directory:  ProcessField::Unavailable,
                    parent:     ParentProcessLink::Unreadable,
                };
            }
            if !record.cmd().is_empty() {
                node.argv = record.cmd().to_vec();
            }
            if let ProcessField::Observed(directory) = record.directory {
                node.directory = ProcessField::Observed(directory.to_owned());
            }
            if matches!(record.parent, ProcessField::Observed(_)) {
                node.parent = observed_parent;
            }
        }
        // A captured parent can acquire native metadata after its child's
        // link was retained. Rebind that link only with the same lifetime
        // proof that permits the parent itself to adopt the native identity.
        let natives: HashMap<_, _> = self
            .nodes
            .iter()
            .map(|(&pid, node)| (pid, node.native.clone()))
            .collect();
        for node in self.nodes.values_mut() {
            if let ParentProcessLink::Known {
                pid,
                native: InvocationId::Captured(run),
            } = &node.parent
                && let Some(evidence) = census.lifetimes.get(pid)
                && let Ok(proof) = LifetimeMatch::prove(run, evidence)
            {
                let native = proof.into_native();
                if natives.get(pid) == Some(&native) {
                    node.parent = ParentProcessLink::Known { pid: *pid, native };
                }
            }
        }
    }

    fn parent_of(&self, child: Pid) -> Option<Pid> {
        let ParentProcessLink::Known { pid, native } = &self.nodes.get(&child)?.parent else {
            return None;
        };
        self.nodes
            .get(pid)
            .filter(|parent| &parent.native == native)
            .map(|_| *pid)
    }

    fn forwarding_process(&self, pid: Pid) -> Option<ForwardingProcess<'_>> {
        let node = self.nodes.get(&pid)?;
        Some(ForwardingProcess {
            pid,
            argv: &node.argv,
            directory: match &node.directory {
                ProcessField::Observed(directory) => {
                    WorkingDirectoryObservation::Observed(directory)
                },
                ProcessField::Unavailable => WorkingDirectoryObservation::Unavailable,
            },
        })
    }

    fn unify(&mut self, handover: &InvocationHandover) {
        if handover.retire != handover.keep.identity {
            self.joined
                .insert(handover.retire.clone(), handover.keep.identity.clone());
        }
        for node in self.nodes.values_mut() {
            if node.invocation.identity == handover.retire
                || node.invocation.identity == handover.keep.identity
            {
                node.invocation.clone_from(&handover.keep);
            }
        }
    }

    fn retain_ancestry(&mut self, census: &Census, groups: &mut [CargoGroup]) {
        for group in groups.iter_mut() {
            let id = group.id();
            if let Some(known) = self.ancestry.get(&id)
                && known.links.iter().all(|(child, parent)| {
                    census
                        .parents
                        .get(child)
                        .is_none_or(|current| current == parent)
                })
                && known.lifetimes.iter().all(|(pid, native)| {
                    self.nodes
                        .get(pid)
                        .is_none_or(|node| &node.native == native)
                })
            {
                let mut levels = group.ancestry.as_slice().to_vec();
                for old in known.walk.as_slice().iter().rev() {
                    if let Some(current) = levels.iter_mut().find(|row| row.pid == old.pid) {
                        if current.command.program_name().is_empty() {
                            *current = old.clone();
                        }
                    } else {
                        levels.insert(0, old.clone());
                    }
                }
                group.ancestry = match group.ancestry {
                    AncestryWalk::Ended(_) => AncestryWalk::Ended(levels),
                    AncestryWalk::LimitCut(_) => AncestryWalk::LimitCut(levels),
                };
            }
            let mut links = Vec::new();
            let mut walking = Pid::from_u32(group.lead.pid);
            for _ in 0..PARENT_WALK_LIMIT {
                let Some(parent) = self.parent_of(walking) else {
                    break;
                };
                if links.contains(&(walking, parent)) {
                    break;
                }
                links.push((walking, parent));
                walking = parent;
            }
            self.ancestry.insert(
                id,
                KnownAncestry {
                    lifetimes: links
                        .iter()
                        .filter_map(|(_, pid)| {
                            self.nodes.get(pid).map(|node| (*pid, node.native.clone()))
                        })
                        .collect(),
                    links,
                    walk: group.ancestry.clone(),
                },
            );
        }
        self.ancestry
            .retain(|id, _| groups.iter().any(|group| &group.id() == id));
    }
}

/// A proven forwarding edge transfers one invocation to its working process.
struct ForwardedInvocation {
    source:         Pid,
    representative: Pid,
    identity:       InvocationId,
}

impl ForwardedInvocation {
    fn carry(self, identities: &mut HashMap<Pid, InvocationId>) {
        identities.insert(self.representative, self.identity);
    }
}

/// A native snapshot has not yet established persistent invocation ownership.
#[derive(Clone)]
enum InvocationIdentitySource {
    NativeSnapshot,
    RetainedTree,
}

/// What phase one learned: parent links and processes awaiting attribution.
#[derive(Clone)]
pub(super) struct Census {
    /// Only retained owners can supersede a verified registration's run identity.
    identity_source:          InvocationIdentitySource,
    publication_evidence:     PublicationEvidence,
    /// Registration eligibility is resolved even when no process row can be built.
    registration_eligibility: HashMap<RunId, Result<(), RowAbsence>>,
    /// Invocation identities are established before attribution and row construction.
    pub(super) identities:    HashMap<Pid, InvocationId>,
    /// Preserve cargo ancestry even after a command is deliberately excluded.
    capture_boundaries:       HashSet<Pid>,
    /// Only observed forwarding of the registered command permits walking through a wrapper.
    capture_wrappers:         HashSet<Pid>,
    /// Current argv proves cargo itself has replaced the registered process.
    registered_cargo:         HashSet<Pid>,
    /// Observed argv naming a different command forbids direct registration fields.
    capture_mismatches:       HashSet<Pid>,
    /// Native lifetime evidence belongs to this refresh, including unavailable reads.
    lifetimes:                HashMap<Pid, LifetimeEvidence>,
    /// Native status comes from the same refresh as the process lifetime.
    process_statuses:         HashMap<Pid, NativeProcessStatus>,
    /// Deliberate exclusion remains distinct from unavailable command metadata.
    eligibility:              HashMap<Pid, Result<(), RowAbsence>>,
    /// Every process's parent, for walking a compiler up to its cargo.
    parents:                  HashMap<Pid, Pid>,
    /// Cargo invocations retained for row construction after selection.
    cargo:                    Vec<Pid>,
    /// Runs carried by hidden drivers to the process that represents their command.
    dispatched_runs:          HashMap<Pid, usize>,
    /// Hidden invocations still establish CPU ownership and competing target evidence.
    rowless_cargo:            Vec<Pid>,
    /// Compiler processes paired with which driver they are.
    compilers:                Vec<(Pid, &'static str)>,
    /// Per-process validation is consulted only for an invocation's own counter.
    cpu:                      HashMap<Pid, Measurement<f32>>,
    /// Resident bytes for every process in the discovery refresh.
    memory:                   HashMap<Pid, u64>,
    /// Own accumulated task time is the available counter on Darwin.
    accumulated:              HashMap<Pid, Duration>,
    /// Owners of retained invocations, named from this scan's account table.
    owners:                   HashMap<Pid, ProcessOwner>,
    /// Fixture rows enter group assembly without a process-row measurement.
    #[cfg(test)]
    registration_rows:        Vec<CargoProcess>,
}

impl Census {
    /// A current live native process supplies its own lifetime proof.
    fn process_presence(
        &self,
        row: &InvocationProcessRow,
        probe: &mut impl FnMut(&InvocationProcessRow) -> InvocationProbe,
    ) -> InvocationProbe {
        let pid = Pid::from_u32(row.process.pid);
        match self
            .lifetimes
            .get(&pid)
            .map(|current| NativeLifetimeComparison::between(&row.lifetime, current))
        {
            Some(NativeLifetimeComparison::Different) => {
                InvocationProbe::Ended(ProcessLifetimeSubject::observe(row))
            },
            Some(NativeLifetimeComparison::Same)
                if self.publication_evidence.has_current_process(pid)
                    && matches!(
                        self.process_statuses.get(&pid),
                        Some(NativeProcessStatus::Live)
                    ) =>
            {
                InvocationProbe::Alive
            },
            Some(NativeLifetimeComparison::Same | NativeLifetimeComparison::Unknown) | None => {
                probe(row)
            },
        }
    }

    /// Both kernel and synthetic observations use the same discovery-to-row selection.
    fn prepare(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) {
        if matches!(
            self.identity_source,
            InvocationIdentitySource::NativeSnapshot
        ) {
            self.include_registered_processes(records, capture);
            self.include_dispatched_processes(records);
        }
        self.registered_cargo = capture
            .registered_pids()
            .into_iter()
            .map(Pid::from_u32)
            .filter(|pid| {
                records.process(*pid).is_some_and(|process| {
                    command_text::cargo_split(process.cmd()).is_ok_and(|arguments| {
                        arguments.start == DIRECT_CARGO_ARGUMENTS_START
                            && arguments.follows_cargo_executable()
                    })
                })
            })
            .collect();
        self.identify_capture_wrappers(records, capture);
        self.collapse_shims(records);
        self.identify_captures(capture);
        self.select_rows(records, capture, excluded);
        self.owners = self
            .cargo
            .iter()
            .filter_map(|&pid| {
                let process = records.process(pid)?;
                Some((pid, ProcessOwner::observe(process.uid, capture)))
            })
            .collect();
    }

    /// A cargo-launched script keeps its dispatched pid despite an interpreter's native name.
    fn include_dispatched_processes(&mut self, records: &ProcessObservations<'_>) {
        let dispatched: Vec<_> = records
            .processes()
            .iter()
            .filter_map(|(&pid, process)| {
                let parent = self.parents.get(&pid)?;
                if !self.cargo.contains(parent) {
                    return None;
                }
                let shim = records.process(*parent)?;
                match Self::dispatch(shim, process) {
                    DispatchedProcess::Matched { pid, .. } => Some(pid),
                    DispatchedProcess::NotObserved => None,
                }
            })
            .collect();
        for pid in dispatched {
            if !self.cargo.contains(&pid) {
                self.cargo.push(pid);
            }
        }
    }

    /// Exact forwarded arguments identify a dispatched child independently of its cwd.
    fn dispatch(
        shim: &ProcessObservation<'_>,
        process: &ProcessObservation<'_>,
    ) -> DispatchedProcess {
        process_identity::dispatched_process(shim.cmd(), process.cmd(), process.pid())
    }

    /// Canonical external-tool arguments leave process identity and all sampled fields intact.
    fn invocation_arguments<'records>(
        &self,
        records: &'records ProcessObservations<'_>,
        process: &'records ProcessObservation<'_>,
    ) -> &'records [OsString] {
        if let Some(parent) = self.parents.get(&process.pid())
            && let Some(shim) = records.process(*parent)
            && let DispatchedProcess::Matched {
                arguments_start, ..
            } = Self::dispatch(shim, process)
        {
            return &process.cmd()[arguments_start..];
        }
        process.cmd()
    }

    /// Classify every process the last refresh saw.
    fn take<'scan>(records: impl IntoIterator<Item = &'scan ProcessObservation<'scan>>) -> Self {
        let mut census = Self {
            identity_source:                InvocationIdentitySource::NativeSnapshot,
            publication_evidence:           PublicationEvidence::Current,
            identities:                     HashMap::new(),
            capture_boundaries:             HashSet::new(),
            capture_wrappers:               HashSet::new(),
            registered_cargo:               HashSet::new(),
            capture_mismatches:             HashSet::new(),
            lifetimes:                      HashMap::new(),
            process_statuses:               HashMap::new(),
            eligibility:                    HashMap::new(),
            registration_eligibility:       HashMap::new(),
            parents:                        HashMap::new(),
            cargo:                          Vec::new(),
            rowless_cargo:                  Vec::new(),
            dispatched_runs:                HashMap::new(),
            compilers:                      Vec::new(),
            cpu:                            HashMap::new(),
            memory:                         HashMap::new(),
            accumulated:                    HashMap::new(),
            owners:                         HashMap::new(),
            #[cfg(test)]
            registration_rows:              Vec::new(),
        };
        for process in records {
            let pid = process.pid;
            if let ProcessField::Observed(parent) = process.parent {
                census.parents.insert(pid, parent);
            }
            census
                .lifetimes
                .insert(pid, process.lifetime.as_ref().clone());
            census.process_statuses.insert(pid, process.status);
            census
                .accumulated
                .insert(pid, Duration::from_millis(process.accumulated));
            census.cpu.insert(pid, process.cpu);
            census.memory.insert(pid, process.memory);
            let name = process.name();
            if command_text::is_cargo_name(name)
                || (is_transparent(name)
                    && command_text::cargo_split(process.cmd()).is_ok_and(|arguments| {
                        arguments.start == WRAPPED_CARGO_ARGUMENTS_START
                            && arguments.follows_cargo_executable()
                    }))
            {
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

    /// Drop each cargo `scope` leaves out before anything reads
    /// [`Self::cargo`], so it owns no row, compiler or CPU and stands in a
    /// parent chain like any other process.
    fn restrict_to(&mut self, scope: CensusScope) {
        let parents = &self.parents;
        self.cargo.retain(|&pid| scope.contains(pid, parents));
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
        if previous.accumulated == 0 || baseline.accumulated < previous.accumulated {
            return Measurement::Unavailable(MeasurementAbsence::Unproven);
        }
        if baseline.accumulated == previous.accumulated && cpu > 0.0 {
            return Measurement::Unavailable(MeasurementAbsence::RetainedRate);
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
            if let CargoAncestry::Owner(owner) = self.owning_cargo(pid) {
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

    /// Kernel counter reads and supplied samples use the same clocked accounting path.
    fn attribute(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
    ) -> InvocationMeasurements {
        self.attribute_with_clock(
            observations,
            smoothing,
            |pid| {
                let measurement = self.process_cpu_time(pid);
                observations
                    .records
                    .get(&pid)
                    .map_or(measurement, |record| {
                        record.native_cpu.set(measurement);
                        record.native_cpu.get()
                    })
            },
            || {
                if cfg!(target_os = "linux") {
                    Instant::now()
                } else {
                    now
                }
            },
        )
    }

    /// Supplied counters carry the supplied read time through the product accounting path.
    #[cfg(test)]
    fn attribute_with(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> InvocationMeasurements {
        self.attribute_selected_with_clock(
            observations,
            smoothing,
            &AttributableInvocations::supplied(self),
            read,
            || now,
        )
    }

    /// Kernel and fixture counters share invocation smoothing and published PID readings.
    fn attribute_with_clock(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        read: impl FnMut(Pid) -> Measurement<Duration>,
        clock: impl FnOnce() -> Instant,
    ) -> InvocationMeasurements {
        self.attribute_selected_with_clock(
            observations,
            smoothing,
            &AttributableInvocations::current(self, observations),
            read,
            clock,
        )
    }

    fn attribute_selected_with_clock(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        attributable: &AttributableInvocations,
        read: impl FnMut(Pid) -> Measurement<Duration>,
        clock: impl FnOnce() -> Instant,
    ) -> InvocationMeasurements {
        smoothing.observed.clone_from(&self.lifetimes);
        let samples = self.sample_cpu_with(observations, smoothing, attributable, read, clock);
        let now = samples.at;
        let cpu_by_identity: HashMap<_, _> = samples
            .readings
            .into_iter()
            .filter_map(|(pid, cpu)| {
                self.identities
                    .get(&pid)
                    .map(|identity| (identity.clone(), cpu))
            })
            .collect();
        let owners: Vec<_> = self
            .cargo
            .iter()
            .copied()
            .filter(|pid| attributable.pids.contains(pid))
            .collect();
        let identities: Vec<_> = owners
            .iter()
            .filter_map(|pid| self.identities.get(pid).cloned())
            .collect();
        let memory_sampled: HashMap<_, _> = self
            .attribute_memory(&smoothing.detached)
            .into_iter()
            .filter_map(|(pid, memory)| {
                self.identities
                    .get(&pid)
                    .map(|identity| (identity.clone(), memory))
            })
            .collect();
        let memory_report = smoothing.memory_report(now);
        let reported = smoothing.settle(&cpu_by_identity, &identities, now);
        let reported_memory = smoothing.report_memory(&memory_sampled, &identities, memory_report);
        InvocationMeasurements {
            #[cfg(test)]
            cpu_scan_publication:              smoothing.observe_cpu(
                &cpu_by_identity,
                &self.identities,
                now,
            ),
            compilers:                         self
                .attribute_compilers()
                .into_iter()
                .filter(|(pid, _)| owners.contains(pid))
                .collect(),
            cpu:                               owners
                .iter()
                .filter_map(|pid| {
                    self.identities
                        .get(pid)
                        .and_then(|identity| reported.get(identity))
                        .map(|cpu| (*pid, *cpu))
                })
                .collect(),
            memory:                            owners
                .iter()
                .filter_map(|pid| {
                    self.identities
                        .get(pid)
                        .and_then(|identity| reported_memory.get(identity))
                        .map(|memory| (*pid, *memory))
                })
                .collect(),
        }
    }

    /// Measure accumulated invocation work between scans, validating only its own counter.
    /// Direct descendants take precedence over output-based cache attribution.
    /// Counter reads share the same ordered accounting pass for kernel and fixture samples.
    #[cfg(test)]
    fn attribute_cpu_with(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> HashMap<Pid, Measurement<f32>> {
        self.sample_cpu_with(
            observations,
            smoothing,
            &AttributableInvocations::supplied(self),
            read,
            || now,
        )
        .readings
    }

    fn sample_cpu_with(
        &self,
        observations: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        attributable: &AttributableInvocations,
        read: impl FnMut(Pid) -> Measurement<Duration>,
        clock: impl FnOnce() -> Instant,
    ) -> InvocationCpuSamples {
        smoothing.identify_owners(self);
        let identities: HashSet<_> = smoothing.owners.values().cloned().collect();
        smoothing
            .invocations
            .retain(|identity, _| identities.contains(identity));
        smoothing
            .targets
            .retain(|identity, _| identities.contains(identity));
        let targets = self.observe_targets(observations, &smoothing.owners, &mut smoothing.targets);
        smoothing.cache_owners.retain(|identity, _| {
            self.compiler_credit_retention(identity) == CompilerCreditRetention::Keep
        });
        let compiler_attributions: HashMap<_, _> = self
            .detached_compilers(observations, &targets)
            .into_iter()
            .map(|(compiler, attribution)| (compiler, attribution.bind(compiler, self, smoothing)))
            .collect();
        smoothing.detached = compiler_attributions
            .iter()
            .filter_map(|(&compiler, attribution)| match attribution {
                CompilerAttribution::Owner(owner) => Some((compiler, *owner)),
                CompilerAttribution::PendingMetadata | CompilerAttribution::Unassigned => None,
            })
            .collect();
        #[cfg(test)]
        for pid in self.detailed() {
            if (observations.process(pid).is_none()
                || compiler_attributions.get(&pid) == Some(&CompilerAttribution::PendingMetadata))
                && let Ok(identity) = self.cpu_identity(pid)
            {
                cpu_observation::pending_metadata(&identity, self.accumulated[&pid]);
            }
        }
        let mut work: HashMap<Pid, InvocationCpuContributions> = self
            .live_cargo()
            .filter(|pid| attributable.pids.contains(pid))
            .map(|pid| (pid, InvocationCpuContributions::default()))
            .collect();
        let mut evidence = self.cpu.clone();
        self.collect_cpu_work(&smoothing.detached, &mut work, &mut evidence, read);
        let now = clock();
        #[cfg(target_os = "linux")]
        self.exclude_nested_cpu(&mut work, smoothing);
        let readings = work
            .into_iter()
            .filter_map(|(pid, work)| {
                let process = self.cpu_identity(pid).ok()?;
                let identity = smoothing.owners.get(&process)?;
                let evidence = evidence
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven));
                #[cfg(test)]
                cpu_observation::work(pid, identity, &work, evidence);
                let sample = smoothing
                    .invocations
                    .entry(identity.clone())
                    .or_default()
                    .measure(work, evidence, now);
                Some((pid, sample))
            })
            .collect();
        InvocationCpuSamples { at: now, readings }
    }

    /// Resident bytes per row-holding cargo: its own process and every process assigned to it.
    fn attribute_memory(&self, detached: &HashMap<Pid, Pid>) -> HashMap<Pid, Measurement<u64>> {
        let mut buckets = HashMap::<Pid, u64>::new();
        for (&pid, &bytes) in &self.memory {
            let owner = match self.cpu_assignment(pid, detached) {
                CpuAssignment::Direct(owner) | CpuAssignment::Detached { owner, .. } => owner,
                CpuAssignment::Unassigned => continue,
            };
            let bucket = buckets.entry(owner).or_default();
            *bucket = bucket.saturating_add(bytes);
        }
        self.cargo
            .iter()
            .map(|&pid| {
                let memory = match self.memory.get(&pid) {
                    Some(&bytes) if bytes > 0 => {
                        Measurement::Reading(buckets.get(&pid).copied().unwrap_or_default())
                    },
                    _ => Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                };
                (pid, memory)
            })
            .collect()
    }

    /// Row eligibility never removes a live invocation from CPU ownership.
    pub(super) fn live_cargo(&self) -> impl Iterator<Item = Pid> + '_ {
        self.cargo.iter().chain(&self.rowless_cargo).copied()
    }

    /// CPU accounting includes the root process and orders ancestors nearest first.
    fn cpu_ancestry(&self, pid: Pid) -> Vec<Pid> {
        let mut chain = vec![pid];
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            if current.as_u32() <= ROOT_PROCESS_PID {
                break;
            }
            let Some(&parent) = self.parents.get(&current) else {
                break;
            };
            if chain.contains(&parent) {
                break;
            }
            chain.push(parent);
            if parent.as_u32() <= ROOT_PROCESS_PID {
                break;
            }
            current = parent;
        }
        chain
    }

    /// Each live cargo in the process chain can own work, even without a displayed row.
    fn cpu_owners(&self, pid: Pid) -> impl Iterator<Item = Pid> + '_ {
        self.cpu_ancestry(pid)
            .into_iter()
            .filter(|ancestor| self.live_cargo().any(|cargo| cargo == *ancestor))
    }

    /// Ancestry and detached compiler roots establish one assignment per process.
    fn cpu_assignment(&self, pid: Pid, detached: &HashMap<Pid, Pid>) -> CpuAssignment {
        if let Some(owner) = self.cpu_owners(pid).next() {
            return CpuAssignment::Direct(owner);
        }
        self.cpu_ancestry(pid)
            .into_iter()
            .find_map(|compiler| {
                detached
                    .get(&compiler)
                    .map(|&owner| CpuAssignment::Detached { owner, compiler })
            })
            .unwrap_or(CpuAssignment::Unassigned)
    }

    /// Read parents before children so a concurrent wait cannot repeat a live counter.
    fn collect_cpu_work(
        &self,
        detached: &HashMap<Pid, Pid>,
        work: &mut HashMap<Pid, InvocationCpuContributions>,
        evidence: &mut HashMap<Pid, Measurement<f32>>,
        mut read: impl FnMut(Pid) -> Measurement<Duration>,
    ) {
        // Parents are read before children: a wait completed during this pass may
        // defer time until the next sample, but cannot charge both child and reaper.
        let mut pids: Vec<_> = self.accumulated.keys().copied().collect();
        pids.sort_by_key(|pid| self.cpu_ancestry(*pid).len());
        for pid in pids {
            let assignment = self.cpu_assignment(pid, detached);
            let owner = match assignment {
                CpuAssignment::Direct(owner) | CpuAssignment::Detached { owner, .. } => owner,
                CpuAssignment::Unassigned => continue,
            };
            let Ok(identity) = self.cpu_identity(pid) else {
                continue;
            };
            let elapsed = match read(pid) {
                Measurement::Reading(elapsed) => elapsed,
                Measurement::Unavailable(reason) => {
                    if pid == owner {
                        evidence.insert(owner, Measurement::Unavailable(reason));
                    }
                    continue;
                },
            };
            if pid == owner
                && (cfg!(target_os = "linux")
                    || (matches!(
                        evidence.get(&owner),
                        Some(Measurement::Unavailable(
                            MeasurementAbsence::FirstObservation | MeasurementAbsence::RetainedRate
                        ))
                    ) && !elapsed.is_zero()))
            {
                // Linux's lifetime-checked stat read proves even an idle owner's
                // zero counter independently of sysinfo's unavailable rate.
                // Darwin's zero may still be a failed task-info read, while a
                // nonzero counter is a sound read whatever rate sysinfo kept:
                // the invocation's rate comes from counters, never that rate.
                // InvocationCpuHistory still reports FirstObservation for the first sample.
                evidence.insert(owner, Measurement::Reading(0.0));
            }
            match assignment {
                CpuAssignment::Detached { compiler, .. } => {
                    if let Ok(root) = self.cpu_identity(compiler)
                        && let Some(work) = work.get_mut(&owner)
                    {
                        work.detached
                            .entry(root)
                            .or_default()
                            .insert(identity, elapsed);
                    }
                },
                CpuAssignment::Direct(_) => {
                    #[cfg(target_os = "linux")]
                    for ancestor in self.cpu_owners(owner) {
                        if let Some(work) = work.get_mut(&ancestor) {
                            work.tree.insert(identity.clone(), elapsed);
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    if let Some(work) = work.get_mut(&owner) {
                        work.tree.insert(identity, elapsed);
                    }
                },
                CpuAssignment::Unassigned => {},
            }
        }
    }

    /// Keep the manager's full tree and the nested invocations' prior credits separate.
    #[cfg(target_os = "linux")]
    fn exclude_nested_cpu(
        &self,
        work: &mut HashMap<Pid, InvocationCpuContributions>,
        smoothing: &InvocationCpuAccounting,
    ) {
        let totals: HashMap<_, _> = work
            .iter()
            .map(|(&pid, work)| {
                let retained = self
                    .cpu_identity(pid)
                    .ok()
                    .and_then(|process| smoothing.owners.get(&process))
                    .and_then(|identity| smoothing.invocations.get(identity))
                    .map_or(Duration::ZERO, |cpu| cpu.tree.total);
                (pid, retained.max(work.tree.values().sum()))
            })
            .collect();
        for (&pid, &total) in &totals {
            if let Some(parent) = self.cpu_owners(pid).nth(1)
                && let Some(work) = work.get_mut(&parent)
                && let Ok(process) = self.cpu_identity(pid)
                && let Some(identity) = smoothing.owners.get(&process)
            {
                work.nested.insert(identity.clone(), total);
            }
        }
    }

    /// Missing census metadata cannot release a still-live compiler's original owner.
    fn compiler_credit_retention(&self, identity: &ProcessIdentity) -> CompilerCreditRetention {
        let pid = match identity {
            ProcessIdentity::Known { pid, lifetime } => {
                if let Some(LifetimeEvidence::Available(current)) =
                    self.lifetimes.get(&Pid::from_u32(*pid))
                {
                    return if current == lifetime {
                        CompilerCreditRetention::Keep
                    } else {
                        CompilerCreditRetention::Retire
                    };
                }
                *pid
            },
            ProcessIdentity::Unavailable { pid, .. } => *pid,
        };
        let Some(pid) = i32::try_from(pid).ok().and_then(KernelPid::from_raw) else {
            return CompilerCreditRetention::Keep;
        };
        // Signal zero checks existence without delivering a signal. Permission and
        // inspection failures preserve the credit; only ESRCH proves process absence.
        if test_kill_process(pid) == Err(Errno::SRCH) {
            CompilerCreditRetention::Retire
        } else {
            CompilerCreditRetention::Keep
        }
    }

    /// Lifetime-qualified keys cannot transfer CPU time across pid reuse.
    pub(super) fn cpu_identity(&self, pid: Pid) -> Result<ProcessIdentity, MeasurementAbsence> {
        match self.lifetimes.get(&pid) {
            Some(LifetimeEvidence::Available(lifetime)) => Ok(ProcessIdentity::Known {
                pid:      pid.as_u32(),
                lifetime: lifetime.clone(),
            }),
            _ => Err(MeasurementAbsence::Unproven),
        }
    }

    /// Linux includes completed children; other platforms supply observed task time.
    fn process_cpu_time(&self, pid: Pid) -> Measurement<Duration> {
        #[cfg(target_os = "linux")]
        {
            let result = invocation_cpu_accounting::linux_cpu_time(pid).and_then(|elapsed| {
                if self.lifetimes.get(&pid) == Some(&birth_stamp::lifetime(pid.as_u32())) {
                    Ok(elapsed)
                } else {
                    Err(MeasurementAbsence::Unproven)
                }
            });
            match result {
                Ok(elapsed) => Measurement::Reading(elapsed),
                Err(reason) => Measurement::Unavailable(reason),
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.accumulated.get(&pid).copied().map_or(
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                Measurement::Reading,
            )
        }
    }

    /// Current targets can establish competition without authorizing a counter read.
    /// Only proven process lifetimes retain destinations across scans.
    fn observe_targets(
        &self,
        observations: &ProcessObservations<'_>,
        owners: &HashMap<ProcessIdentity, InvocationId>,
        retained: &mut HashMap<InvocationId, HashSet<PathBuf>>,
    ) -> HashMap<Pid, HashSet<PathBuf>> {
        let mut targets: HashMap<_, _> = self
            .live_cargo()
            .map(|pid| {
                let directories = self
                    .cpu_identity(pid)
                    .ok()
                    .and_then(|process| owners.get(&process))
                    .and_then(|identity| retained.get(identity))
                    .cloned()
                    .unwrap_or_default();
                (pid, directories)
            })
            .collect();
        for pid in self.live_cargo() {
            if let Some(process) = observations.process(pid)
                && let Ok(directory) = invocation_cpu_accounting::cargo_target_directory(process)
            {
                targets.entry(pid).or_default().insert(directory);
            }
        }
        for &(pid, _) in &self.compilers {
            if let Some(owner) = self.cpu_owners(pid).next()
                && let Some(process) = observations.process(pid)
                && let Ok(directory) =
                    invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG)
            {
                targets.entry(owner).or_default().insert(directory);
            }
        }
        for (&pid, directories) in &targets {
            if let Ok(process) = self.cpu_identity(pid)
                && let Some(identity) = owners.get(&process)
            {
                retained.insert(identity.clone(), directories.clone());
            }
        }
        targets
    }

    /// Only a detached rustc beneath sccache may use output-directory ownership.
    fn detached_compilers(
        &self,
        observations: &ProcessObservations<'_>,
        targets: &HashMap<Pid, HashSet<PathBuf>>,
    ) -> HashMap<Pid, CompilerAttribution> {
        self.compilers
            .iter()
            .filter(|(_, driver)| *driver == RUSTC_BINARY)
            .map(|&(pid, _)| {
                let attribution = if self.cpu_owners(pid).next().is_some()
                    || !self
                        .cpu_ancestry(pid)
                        .iter()
                        .any(|ancestor| self.compilers.contains(&(*ancestor, SCCACHE_BINARY)))
                {
                    CompilerAttribution::Unassigned
                } else {
                    CompilerAttribution::resolve(pid, self, observations, targets)
                };
                (pid, attribution)
            })
            .collect()
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
    fn collapse_shims(&mut self, observations: &ProcessObservations<'_>) {
        // Runs are counted only after cargo-named forwarding aliases have gone.
        // Interpreted drivers admitted for dispatch naming keep their reading;
        // the aliases themselves must never become runs in that reading.
        loop {
            let children = self.cargo_children();
            let aliases: Vec<_> = self
                .cargo
                .iter()
                .copied()
                .filter(|pid| {
                    let Some(outer) = observations.process(*pid) else {
                        return false;
                    };
                    if !command_text::is_cargo_name(outer.name())
                        || !self.forwards_to_single_child(observations, &children, *pid)
                    {
                        return false;
                    }
                    let child = children[pid][0];
                    let Some(inner) = observations.process(child) else {
                        return false;
                    };
                    observed_shim_match(outer.cmd(), outer.cwd(), inner.cmd(), inner.cwd())
                })
                .collect();
            if aliases.is_empty() {
                break;
            }
            self.carry_forwarded_invocations(&aliases, &children);
            self.cargo.retain(|pid| !aliases.contains(pid));
        }
        loop {
            let children = self.cargo_children();
            let shims: Vec<Pid> = self
                .cargo
                .iter()
                .copied()
                .filter(|&pid| self.forwards_to_single_child(observations, &children, pid))
                .collect();
            if shims.is_empty() {
                return;
            }
            self.carry_forwarded_invocations(&shims, &children);
            for &shim in &shims {
                let child = children[&shim][0];
                let Some(outer) = observations.process(shim) else {
                    continue;
                };
                let mut representative = child;
                while shims.contains(&representative) {
                    representative = children[&representative][0];
                }
                let Some(inner) = observations.process(representative) else {
                    continue;
                };
                // Preserve the runs of an external dispatch chain. A shim in front of
                // a built-in cargo command carries no runs of its own to preserve.
                if !matches!(
                    Self::dispatch(outer, inner),
                    DispatchedProcess::Matched { .. }
                ) && !self.dispatched_runs.contains_key(&shim)
                {
                    continue;
                }
                let runs = Self::descendants(&children, shim)
                    .len()
                    .max(self.dispatched_runs.get(&shim).copied().unwrap_or_default());
                self.dispatched_runs
                    .entry(representative)
                    .and_modify(|carried| *carried = (*carried).max(runs))
                    .or_insert(runs);
            }
            self.cargo.retain(|pid| !shims.contains(pid));
        }
    }

    /// Outer forwarding lifetimes remain the command identity through nested shims.
    fn carry_forwarded_invocations(&mut self, sources: &[Pid], children: &HashMap<Pid, Vec<Pid>>) {
        let mut forwarded = sources
            .iter()
            .filter_map(|shim| {
                Some(ForwardedInvocation {
                    source:         *shim,
                    representative: children[shim][0],
                    identity:       self.identities.get(shim)?.clone(),
                })
            })
            .collect::<Vec<_>>();
        forwarded.sort_by_key(|handover| self.ancestor_pids(handover.source).as_slice().len());
        for mut invocation in forwarded {
            if let Some(identity) = self.identities.get(&invocation.source) {
                invocation.identity.clone_from(identity);
            }
            invocation.carry(&mut self.identities);
        }
    }

    /// The retained tree owns forwarding; census pruning consumes that same proof.
    fn forwards_to_single_child(
        &self,
        observations: &ProcessObservations<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        pid: Pid,
    ) -> bool {
        match self.identity_source {
            InvocationIdentitySource::RetainedTree => {
                let Some([child]) = children.get(&pid).map(Vec::as_slice) else {
                    return false;
                };
                matches!((self.identities.get(&pid), self.identities.get(child)),
                    (Some(parent), Some(child)) if parent == child)
            },
            InvocationIdentitySource::NativeSnapshot => Self::is_shim(observations, children, pid),
        }
    }

    /// Whether `pid` is a shim in front of the one cargo beneath it.
    fn is_shim(
        observations: &ProcessObservations<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        pid: Pid,
    ) -> bool {
        let Some(outer) = observations.process(pid) else {
            return false;
        };
        let process = |pid| {
            observations.process(pid).map(|record| ForwardingProcess {
                pid,
                argv: record.cmd(),
                directory: record.cwd(),
            })
        };
        matches!(
            classify_forwarding(
                ForwardingProcess {
                    pid,
                    argv: outer.cmd(),
                    directory: outer.cwd()
                },
                children
                    .get(&pid)
                    .into_iter()
                    .flatten()
                    .filter_map(|&pid| process(pid)),
            ),
            Forwarding::SingleChild(_)
        )
    }

    /// Each cargo's direct cargo children -- the tree the groups are cut
    /// from, with a process that came out as its own ancestor dropped so
    /// a walk of it cannot loop.
    fn cargo_children(&self) -> HashMap<Pid, Vec<Pid>> {
        let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
        for &pid in &self.cargo {
            if let CargoAncestry::Owner(owner) = self.owning_cargo(pid)
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
    fn owning_cargo(&self, pid: Pid) -> CargoAncestry {
        let mut current = pid;
        for _ in 0..PARENT_WALK_LIMIT {
            let Some(&parent) = self.parents.get(&current) else {
                return CargoAncestry::ParentUnavailable;
            };
            if self.cargo.contains(&parent) {
                return CargoAncestry::Owner(parent);
            }
            current = parent;
        }
        CargoAncestry::WalkLimitReached
    }

    /// Read detailed metadata for cargo, its ancestors, and compiler drivers.
    ///
    /// A superset of what any cell ends up listing. Which ancestors are
    /// the invocation's own plumbing is settled on argv, which is what
    /// this pass is about to read, so the filtering waits for
    /// [`Self::ancestry`] and the extra reads are a handful of
    /// processes.
    fn detailed(&self) -> Vec<Pid> {
        let mut pids = self.cargo.clone();
        pids.extend(self.compilers.iter().map(|&(pid, _)| pid));
        for (&pid, parent) in &self.parents {
            if self.cargo.contains(parent) && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
        for &pid in &self.cargo {
            for &ancestor in self.ancestor_pids(pid).as_slice() {
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
    fn ancestor_pids(&self, pid: Pid) -> AncestryWalk<Pid> {
        let mut chain = Vec::new();
        let mut current = pid;
        while let Some(&parent) = self.parents.get(&current) {
            if parent.as_u32() <= ROOT_PROCESS_PID || chain.contains(&parent) {
                break;
            }
            if chain.len() == PARENT_WALK_LIMIT {
                chain.reverse();
                return AncestryWalk::LimitCut(chain);
            }
            chain.push(parent);
            current = parent;
        }
        chain.reverse();
        AncestryWalk::Ended(chain)
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
    fn ancestry(
        &self,
        observations: &ProcessObservations<'_>,
        home: ScannerHome<'_>,
        pid: Pid,
    ) -> AncestryWalk<Ancestor> {
        self.ancestor_pids(pid).filter_map(|pid| {
            let process = observations.process(pid)?;
            if command_text::names_cargo(self.invocation_arguments(observations, process)) {
                return None;
            }
            Some(Ancestor {
                pid:            process.pid().as_u32(),
                command:        describe(process, home),
                passes_through: is_transparent(process.name()),
            })
        })
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
    fn include_registered_processes(
        &mut self,
        observations: &ProcessObservations<'_>,
        capture: &Capture,
    ) {
        for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
            if observations
                .process(pid)
                .is_some_and(|process| command_text::names_cargo(process.cmd()))
                && !self.cargo.contains(&pid)
            {
                self.cargo.push(pid);
            }
        }
    }

    /// Record wrappers forwarding the registered command, including the shim's JSON rewrite.
    /// An exec-replaced application no longer carries that command and is a boundary.
    fn identify_capture_wrappers(
        &mut self,
        observations: &ProcessObservations<'_>,
        capture: &Capture,
    ) {
        self.capture_mismatches = observations
            .processes()
            .iter()
            .filter(|(_, process)| {
                process.details == DetailPresence::Observed && !process.cmd().is_empty()
            })
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        !forwards_capture_command(
                            self.invocation_arguments(observations, process),
                            confirmed.registration.record(),
                        )
                    })
                    .map(|_| pid)
            })
            .collect();
        self.capture_wrappers = observations
            .processes()
            .iter()
            .filter(|(_, process)| {
                process.details == DetailPresence::Observed && !process.cmd().is_empty()
            })
            .filter_map(|(&pid, process)| {
                let NearestRegistration::Registered(key) = self.captured_run(capture, pid) else {
                    return None;
                };
                capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                    .filter(|confirmed| {
                        forwards_capture_command(
                            self.invocation_arguments(observations, process),
                            confirmed.registration.record(),
                        )
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

    /// Publication enriches the tree's owner; it never rekeys an observed command.
    /// A registration-only command has no native owner and keeps its verified run ID.
    fn registered_identity(&self, direct: &DirectCapture) -> InvocationId {
        let shim = Pid::from_u32(direct.run_id.shim_pid);
        if matches!(self.identity_source, InvocationIdentitySource::RetainedTree)
            && !self.capture_mismatches.contains(&shim)
            && let Some(identity) = self.identities.get(&shim)
        {
            return identity.clone();
        }
        direct.invocation_id()
    }

    /// Prefer the process doing the work when both the shim and cargo describe one run.
    fn identify_captures(&mut self, capture: &Capture) {
        self.capture_boundaries.extend(self.cargo.iter().copied());
        for &pid in &self.cargo {
            if let DirectAssociation::Direct(direct) = self.direct_capture(capture, pid) {
                self.identities
                    .insert(pid, self.registered_identity(&direct));
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
                        && self.ancestor_pids(*child).as_slice().contains(pid)
                })
            })
            .collect();
        self.cargo.retain(|pid| !represented.contains(pid));
    }

    /// Exclusions prune only row ownership; parents and verified capture liveness remain intact.
    fn select_rows(
        &mut self,
        observations: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) {
        self.registration_eligibility = capture
            .confirmed()
            .iter()
            .map(|confirmed| {
                let argv = std::iter::once(OsString::from(CARGO_DISPLAY_NAME))
                    .chain(confirmed.registration.record().arguments().iter().cloned())
                    .collect::<Vec<_>>();
                (
                    RunId::verified(&confirmed.key, &confirmed.registration),
                    command_text::select_cargo(&argv, excluded).map(|_| ()),
                )
            })
            .collect();
        for &pid in &self.cargo {
            let process =
                observations
                    .process(pid)
                    .map_or(Err(RowAbsence::ArgvUnavailable), |process| {
                        command_text::select_cargo(
                            self.invocation_arguments(observations, process),
                            excluded,
                        )
                        .map(|_| ())
                    });
            let eligibility = match self.direct_capture(capture, pid) {
                DirectAssociation::Direct(direct) => {
                    match self.registration_eligibility.get(&direct.run_id) {
                        Some(Err(
                            absence @ (RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded),
                        )) => Err(*absence),
                        Some(Ok(())) if process == Err(RowAbsence::ArgvUnavailable) => Ok(()),
                        Some(Ok(()) | Err(RowAbsence::ArgvUnavailable)) | None => process,
                    }
                },
                DirectAssociation::None => process,
            };
            self.eligibility.insert(pid, eligibility);
        }
        self.rowless_cargo = self
            .cargo
            .iter()
            .copied()
            .filter(|pid| !matches!(self.eligibility.get(pid), Some(Ok(()))))
            .filter(|pid| {
                observations.process(*pid).is_none_or(|process| {
                    process.cmd().is_empty()
                        || command_text::names_cargo(
                            self.invocation_arguments(observations, process),
                        )
                })
            })
            .collect();
        self.cargo
            .retain(|pid| matches!(self.eligibility.get(pid), Some(Ok(()))));
    }

    /// Only a verified direct association may supply directory annotation.
    /// Without a capture, the process owner names the row's account.
    fn annotate_capture(&self, row: &mut CargoProcess, capture: &Capture, home: ScannerHome<'_>) {
        let pid = Pid::from_u32(row.pid);
        let owner = self
            .owners
            .get(&pid)
            .cloned()
            .unwrap_or(ProcessOwner::Unavailable);
        row.provenance = RowProvenance::Uncaptured(owner.clone());
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
                row.invocation_id = self.registered_identity(&direct);
                row.provenance = RowProvenance::direct(capture, &key, owner);
                let record = direct.registration().record();
                if let WorkingDirectoryIdentity::Absolute(path) = &row.directory_identity
                    && DirectoryComparison::between(record.directory(), path)
                        == DirectoryComparison::Same
                {
                    row.directory_identity = record.directory_identity();
                    row.path = registration_directory(record, home);
                }
            },
            DirectAssociation::None => {
                if let Some(confirmed) = capture
                    .confirmed()
                    .iter()
                    .find(|confirmed| confirmed.key == key)
                {
                    row.provenance = RowProvenance::enclosing(capture, &key, owner);
                    row.capture_membership = CaptureMembership::Enclosing(RunId::verified(
                        &key,
                        &confirmed.registration,
                    ));
                } else if pid.as_u32() != key.pid
                    && (self.capture_boundaries.contains(&Pid::from_u32(key.pid))
                        || self
                            .ancestor_pids(pid)
                            .as_slice()
                            .iter()
                            .rev()
                            .take_while(|ancestor| ancestor.as_u32() != key.pid)
                            .any(|ancestor| self.capture_boundaries.contains(ancestor)))
                {
                    row.capture_membership = CaptureMembership::EnclosingUnconfirmed;
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
        observations: &ProcessObservations<'_>,
        attributed: &InvocationMeasurements,
        home: ScannerHome<'_>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        let children = self.cargo_children();
        let mut candidates = self.cargo.clone();
        candidates.sort_by_key(|pid| self.ancestor_pids(*pid).as_slice().len());
        let mut rows: Vec<CargoProcess> = Vec::new();
        for pid in candidates {
            if !rows.iter().any(|row| row.pid == pid.as_u32())
                && let Ok(group) =
                    self.group(observations, attributed, home, &children, capture, pid)
            {
                rows.push(group.lead);
                rows.extend(group.rest);
            }
        }
        let process_rows: HashSet<_> = rows.iter().map(|row| row.invocation_id.clone()).collect();
        self.add_registration_rows(&mut rows, capture, home, SystemTime::now());
        #[cfg(test)]
        rows.extend_from_slice(&self.registration_rows);
        let mut groups = self.assemble_groups(observations, home, rows);
        for group in &mut groups {
            for row in std::iter::once(&mut group.lead).chain(&mut group.rest) {
                if let Some(&runs) = self.dispatched_runs.get(&Pid::from_u32(row.pid)) {
                    row.managed = Measurement::Reading(match row.managed {
                        Measurement::Reading(managed) => managed.max(runs),
                        Measurement::Unavailable(_) => runs,
                    });
                }
                row.resource_use = if process_rows.contains(&row.invocation_id) {
                    InvocationResourceUse::observed(attributed, Pid::from_u32(row.pid))
                } else {
                    InvocationResourceUse::unavailable(MeasurementAbsence::Unproven)
                };
            }
            let members: Vec<_> = std::iter::once(&group.lead).chain(&group.rest).collect();
            let totals = subtree_resource_totals(&members);
            for (row, total) in std::iter::once(&mut group.lead)
                .chain(&mut group.rest)
                .zip(totals)
            {
                row.subtree_resource_use = total;
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
            let identity = self.registered_identity(&direct);
            let represented = rows.iter().any(|row| row.invocation_id == identity);
            if !represented && let Ok(mut row) = registration_row(&direct, capture, home, now) {
                row.invocation_id = identity;
                rows.push(row);
            }
        }
    }

    /// Build one ancestry tree after both sources have supplied eligible invocations.
    fn assemble_groups(
        &self,
        observations: &ProcessObservations<'_>,
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
            let ancestry = self.ancestry(observations, home, Pid::from_u32(lead.pid));
            lead.parent = ancestry
                .as_slice()
                .last()
                .map_or(VisibleParent::None, |ancestor| {
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
        observations: &ProcessObservations<'_>,
        attributed: &InvocationMeasurements,
        home: ScannerHome<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        capture: &Capture,
        root: Pid,
    ) -> Result<CargoGroup, GroupAbsence> {
        if matches!(
            self.eligibility.get(&root),
            Some(Err(RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded))
        ) {
            return Err(GroupAbsence::Excluded);
        }
        let managed = Self::descendants(children, root);
        let process = observations.process(root).ok_or(GroupAbsence::NoProcess)?;
        let mut lead = row(
            process,
            self.invocation_arguments(observations, process),
            self.identities
                .get(&root)
                .ok_or(GroupAbsence::NoIdentity)?
                .clone(),
            Measurement::Reading(managed.len()),
            home,
            InvocationResourceUse::observed(attributed, root),
            &self.direct_capture(capture, root),
        )?;
        self.annotate_capture(&mut lead, capture, home);
        let ancestry = self.ancestry(observations, home, root);
        lead.parent = self.drawn_parent(root, ancestry.as_slice());

        let mut dated: Vec<(u64, CargoProcess)> = managed
            .into_iter()
            .filter_map(|pid| {
                let process = observations.process(pid)?;
                let under = Self::descendants(children, pid).len();
                let mut managed_row = row(
                    process,
                    self.invocation_arguments(observations, process),
                    self.identities.get(&pid)?.clone(),
                    Measurement::Reading(under),
                    home,
                    InvocationResourceUse::observed(attributed, pid),
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
                managed_row.parent = self.drawn_parent(pid, ancestry.as_slice());
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
fn newest_first(left: &CargoProcess, right: &CargoProcess) -> Ordering {
    match (left.started, right.started) {
        (RunStart::Known(left), RunStart::Known(right)) => right.cmp(&left),
        (RunStart::Known(_), RunStart::Unavailable) => Ordering::Less,
        (RunStart::Unavailable, RunStart::Known(_)) => Ordering::Greater,
        (RunStart::Unavailable, RunStart::Unavailable) => Ordering::Equal,
    }
    .then_with(|| right.invocation_id.cmp(&left.invocation_id))
}

/// Recognize both argv forwarding and the shim's single-quoted POSIX command string.
/// PTY helpers and their shells carry this command; an application launched by cargo does not.
fn forwards_capture_command(argv: &[OsString], record: &RegistrationCandidate) -> bool {
    if let Ok(arguments) = command_text::cargo_split(argv)
        && (argv[arguments.start..] == *record.arguments()
            || forwards_json_capture_arguments(&argv[arguments.start..], record.arguments()))
    {
        return true;
    }
    let mut expected_arguments = Vec::new();
    for argument in record.arguments() {
        expected_arguments.push(SHELL_SINGLE_QUOTE);
        for byte in argument.as_bytes() {
            if *byte == SHELL_SINGLE_QUOTE {
                expected_arguments.extend_from_slice(SHELL_ESCAPED_SINGLE_QUOTE);
            } else {
                expected_arguments.push(*byte);
            }
        }
        expected_arguments.extend_from_slice(SHELL_QUOTED_ARGUMENT_END);
    }
    argv.iter().any(|argument| {
        argument
            .as_bytes()
            .strip_suffix(expected_arguments.as_slice())
            .and_then(|binary| binary.strip_prefix(&[SHELL_SINGLE_QUOTE]))
            .and_then(|binary| binary.strip_suffix(SHELL_QUOTED_ARGUMENT_END))
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
    while let Some(index) = remaining
        .iter()
        .position(|byte| *byte == SHELL_SINGLE_QUOTE)
    {
        let Some(rest) = remaining[index..].strip_prefix(SHELL_ESCAPED_SINGLE_QUOTE) else {
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
    matches!((command_text::subcommand(outer), command_text::subcommand(inner), outer_cwd, inner_cwd),
        (Ok(outer), Ok(inner), WorkingDirectoryObservation::Observed(outer_cwd), WorkingDirectoryObservation::Observed(inner_cwd))
        if outer == inner && DirectoryComparison::between(outer_cwd, inner_cwd) == DirectoryComparison::Same)
}

/// One compiler tally across a whole group: the highest-priority driver
/// any member is running, totalled over all of them.
#[cfg(test)]
fn aggregate_compilers(
    counts: &HashMap<Pid, CompilerObservation>,
    members: impl Iterator<Item = Pid>,
) -> CompilerObservation {
    sum_compilers(members.map(|pid| {
        counts
            .get(&pid)
            .cloned()
            .unwrap_or(CompilerObservation::Unknown)
    }))
}

/// Keep the highest-priority observed driver without publishing a partial count.
fn sum_compilers(counts: impl IntoIterator<Item = CompilerObservation>) -> CompilerObservation {
    let mut running = Vec::new();
    for observation in counts {
        match observation {
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

/// One total across a whole group; an unavailable or missing member
/// prevents publishing a partial total as a measured value.
pub(crate) fn aggregate<T: Copy + Default + Add<Output = T>>(
    measurements: &HashMap<Pid, Measurement<T>>,
    members: impl Iterator<Item = Pid>,
) -> Measurement<T> {
    let mut seen = HashSet::new();
    members.filter(|pid| seen.insert(*pid)).fold(
        Measurement::Reading(T::default()),
        |total, pid| {
            total
                + measurements
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven))
        },
    )
}

/// Each pid contributes once to every ancestor's numeric subtree total.
pub(crate) fn subtree_resource_totals(members: &[&CargoProcess]) -> Vec<InvocationResourceUse> {
    let indices: HashMap<_, _> = members
        .iter()
        .enumerate()
        .map(|(index, row)| (&row.invocation_id, index))
        .collect();
    let mut contributions: Vec<_> = members
        .iter()
        .map(|row| vec![(Pid::from_u32(row.pid), row.resource_use.clone())])
        .collect();
    let mut remaining_children = vec![0_usize; members.len()];
    for row in members {
        if let VisibleParent::Invocation { id, .. } = &row.parent
            && let Some(&parent) = indices.get(id)
        {
            remaining_children[parent] += 1;
        }
    }
    let mut ready: Vec<_> = remaining_children
        .iter()
        .enumerate()
        .filter_map(|(index, &children)| (children == 0).then_some(index))
        .collect();
    while let Some(index) = ready.pop() {
        if let VisibleParent::Invocation { id, .. } = &members[index].parent
            && let Some(&parent) = indices.get(id)
        {
            let descendants = contributions[index].clone();
            contributions[parent].extend(descendants);
            remaining_children[parent] -= 1;
            if remaining_children[parent] == 0 {
                ready.push(parent);
            }
        }
    }
    contributions
        .into_iter()
        .zip(remaining_children)
        .map(|(resources, children)| {
            if children > 0 {
                InvocationResourceUse::unavailable(MeasurementAbsence::Unproven)
            } else {
                sum_resource_use_once_per_pid(resources)
            }
        })
        .collect()
}

/// Format one cargo process into its table row.
fn row(
    process: &ProcessObservation<'_>,
    arguments: &[OsString],
    invocation_id: InvocationId,
    managed: Measurement<usize>,
    home: ScannerHome<'_>,
    resource_use: InvocationResourceUse,
    direct: &DirectAssociation,
) -> Result<CargoProcess, RowAbsence> {
    let (directory_identity, path, command) = row_fields(process.cwd(), arguments, direct, home)?;
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
        provenance: RowProvenance::Uncaptured(ProcessOwner::Unavailable),
        path,
        directory_identity,
        pid: process.pid().as_u32(),
        parent: VisibleParent::None,
        start,
        started,
        duration,
        resource_use,
        subtree_resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
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
    let command = match (command_text::command_text(argv, home), direct) {
        (Err(RowAbsence::ArgvUnavailable), DirectAssociation::Direct(direct)) => {
            command_text::command_text(&registration_argv(direct.registration().record()), home)?
        },
        (Ok(_), DirectAssociation::Direct(direct))
            if command_text::cargo_split(argv).is_ok_and(|arguments| {
                forwards_json_capture_arguments(
                    &argv[arguments.start..],
                    direct.registration().record().arguments(),
                )
            }) =>
        {
            // The shim's quiet rewrite changes execution, not the user's command heading.
            command_text::command_text(&registration_argv(direct.registration().record()), home)?
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
            if DirectoryComparison::between(direct.registration().record().directory(), path)
                == DirectoryComparison::Same =>
        {
            (
                direct.registration().record().directory_identity(),
                registration_directory(direct.registration().record(), home),
            )
        },
        (WorkingDirectoryObservation::Observed(path), _) => {
            (path.into(), command_text::home_relative(path, home))
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
        // No process is observed here, so there is no owner to fall back on.
        provenance: RowProvenance::direct(capture, &direct.key, ProcessOwner::Unavailable),
        path: registration_directory(record, home),
        directory_identity: record.directory_identity(),
        pid: direct.key.pid,
        parent: VisibleParent::None,
        start,
        started,
        duration,
        resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
        subtree_resource_use: InvocationResourceUse::unavailable(MeasurementAbsence::Unproven),
        state: capture.read(&direct.key),
        managed: Measurement::Unavailable(MeasurementAbsence::Unproven),
        nested: false,
        command: command_text::command_text(&registration_argv(record), home)?,
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
    Path::new(name)
        .file_name()
        .and_then(OsStr::to_str)
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
fn describe(process: &ProcessObservation<'_>, home: ScannerHome<'_>) -> CommandText {
    let mut words = process
        .cmd()
        .iter()
        .map(|word| command_text::home_relative(Path::new(word), home));
    if let Some(program) = words.next() {
        return CommandText::new(program, words.collect());
    }
    let program = match process.exe() {
        ProcessField::Observed(exe) => command_text::home_relative(exe, home),
        ProcessField::Unavailable => process.name().to_string_lossy().into_owned(),
    };
    CommandText::new(program, Vec::new())
}

/// A tilde is meaningful only when the writer and scanner agree on its prefix.
fn registration_directory(record: &RegistrationCandidate, scanner_home: ScannerHome<'_>) -> String {
    match record.writer_home() {
        WriterHome::Known(writer_home)
            if scanner_home == ScannerHome::Known(writer_home.as_path()) =>
        {
            command_text::home_relative(record.directory(), scanner_home)
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
            RowAbsence::ArgvUnavailable => Self::Unavailable,
            RowAbsence::ProgramRejected | RowAbsence::PolicyExcluded => Self::Excluded,
        }
    }
}

/// Select fresh rows or retain the prior publication in a fixture.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum FixturePublication {
    Current,
    Retained,
}

/// Successive snapshots retain invocation identity and cumulative CPU history.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CensusSequence {
    invocations:                  Invocations,
    smoothing:                    InvocationCpuAccounting,
    fixture_identities:           ProcessIdentities,
    previous:                     HashMap<Pid, CpuBaseline>,
    pub(super) registration_rows: Vec<CargoProcess>,
    published:                    PublishedInvocations,
}

#[cfg(test)]
impl CensusSequence {
    /// Retained invocation count for departure retirement regressions.
    pub(crate) fn retained_invocations_for_test(&self) -> usize { self.invocations.records.len() }

    /// Explicit publication for fixtures exercising scanner continuity.
    pub(crate) fn publication(&self) -> &PublishedInvocations { &self.published }

    /// Compatibility for older synthetic omission fixtures.
    pub(crate) fn missing_invocations(&self) -> Vec<InvocationId> {
        self.published
            .states
            .iter()
            .filter_map(|(id, state)| {
                matches!(state, PublishedInvocation::Missing(_)).then_some(id.clone())
            })
            .collect()
    }

    /// Exercise explicit publication with inconclusive kernel reads for synthetic pids.
    pub(crate) fn sample_published(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        now: Instant,
    ) {
        self.publish_fixture(records, capture, now, |_| InvocationProbe::Uncertain);
    }

    /// Use selected publications and native reads for each process subject.
    pub(crate) fn sample_with_native_evidence(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        now: Instant,
        publication: FixturePublication,
        mut read: impl FnMut(u32) -> Observation,
    ) {
        let evidence = match publication {
            FixturePublication::Current => PublicationEvidence::Current,
            FixturePublication::Retained => PublicationEvidence::Selected {
                processes: ProcessDiscoveryEvidence::Retained,
                captures:  CaptureFreshness::Retained,
            },
        };
        self.publish_fixture_with_evidence(records, capture, now, evidence, |row| {
            let observation = read(row.process.pid);
            let current = match &observation {
                Observation::Present(stamp) => LifetimeEvidence::Available(
                    ProcessLifetime::matching_registration_for_test(stamp),
                ),
                Observation::Ended | Observation::Unknown => LifetimeEvidence::Unavailable,
            };
            let subject = ProcessLifetimeSubject {
                pid:                Pid::from_u32(row.process.pid),
                lifetime:           row.lifetime.clone(),
                kernel_observation: KernelObservation::for_test(
                    row.process.pid,
                    observation.clone(),
                ),
            };
            match observation {
                Observation::Ended => InvocationProbe::Ended(subject),
                Observation::Present(_) | Observation::Unknown => subject.compare(&current),
            }
        });
    }

    /// Exercise the production absence probe with a real process or confirmed exit.
    pub(crate) fn sample_kernel(&mut self, records: &ProcessObservations<'_>, now: Instant) {
        self.publish_fixture(records, &Capture::default(), now, |row| {
            InvocationProbe::kernel(ProcessLifetimeSubject::observe(row))
        });
    }

    fn publish_fixture(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        now: Instant,
        probe: impl FnMut(&InvocationProcessRow) -> InvocationProbe,
    ) {
        self.publish_fixture_with_evidence(
            records,
            capture,
            now,
            PublicationEvidence::Current,
            probe,
        );
    }

    fn publish_fixture_with_evidence(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        now: Instant,
        evidence: PublicationEvidence,
        probe: impl FnMut(&InvocationProcessRow) -> InvocationProbe,
    ) {
        let mut census = Census::take(records.records.values());
        census.publication_evidence = evidence;
        census.identities = self.fixture_identities.observe(&census.lifetimes);
        let mut tree = self.invocations.identify(&mut census, records, capture);
        census.prepare(records, capture, &[]);
        let attributed = InvocationMeasurements {
            cpu_scan_publication: CpuScanPublication::Unobserved,
            cpu:                  records
                .records
                .iter()
                .map(|(&pid, record)| (pid, record.cpu))
                .collect(),
            memory:               census.attribute_memory(&HashMap::new()),
            compilers:            census.attribute_compilers(),
        };
        let mut groups = census.groups(records, &attributed, ScannerHome::Unavailable, capture);
        tree.retain_ancestry(&census, &mut groups);
        self.published = self
            .invocations
            .publish(&census, &tree, groups, capture, now, probe);
    }

    /// Supplied rates exercise selection and assembly without invoking OS accounting.
    pub(crate) fn sample(&mut self, records: &ProcessObservations<'_>) -> Vec<CargoGroup> {
        self.sample_with_capture(records, &Capture::default(), &[], ScannerHome::Unavailable)
    }

    /// App scenarios feed verified captures through the production row-selection path.
    pub(crate) fn sample_capture(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
    ) -> Vec<CargoGroup> {
        self.sample_with_capture(records, capture, &[], ScannerHome::Unavailable)
    }

    fn sample_with_capture(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
        home: ScannerHome<'_>,
    ) -> Vec<CargoGroup> {
        self.sample_attributed(
            records,
            capture,
            excluded,
            home,
            records
                .records
                .iter()
                .map(|(&pid, record)| (pid, record.cpu))
                .collect(),
        )
    }

    /// An omitted share stays absent until production row and subtree assembly handle it.
    pub(super) fn sample_cpu(
        &mut self,
        records: &ProcessObservations<'_>,
        shares: &[(u32, Measurement<f32>)],
    ) -> Vec<CargoGroup> {
        self.sample_attributed(
            records,
            &Capture::default(),
            &[],
            ScannerHome::Unavailable,
            shares
                .iter()
                .map(|&(pid, cpu)| (Pid::from_u32(pid), cpu))
                .collect(),
        )
    }

    /// Resident bytes per cargo when `detached` names each compiler root's owner.
    pub(super) fn attribute_memory(
        records: &ProcessObservations<'_>,
        detached: &HashMap<Pid, Pid>,
    ) -> HashMap<Pid, Measurement<u64>> {
        Census::take(records.records.values()).attribute_memory(detached)
    }

    fn sample_attributed(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
        home: ScannerHome<'_>,
        cpu: HashMap<Pid, Measurement<f32>>,
    ) -> Vec<CargoGroup> {
        let mut census = Census::take(records.records.values());
        census.identities = self.fixture_identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        let mut tree = self.invocations.identify(&mut census, records, capture);
        census.prepare(records, capture, excluded);
        let attributed = InvocationMeasurements {
            cpu_scan_publication: CpuScanPublication::Unobserved,
            compilers: census.attribute_compilers(),
            cpu,
            memory: census.attribute_memory(&HashMap::new()),
        };
        let mut groups = census.groups(records, &attributed, home, capture);
        tree.retain_ancestry(&census, &mut groups);
        self.published = self.invocations.publish(
            &census,
            &tree,
            groups.clone(),
            capture,
            Instant::now(),
            |_| InvocationProbe::Uncertain,
        );
        groups
    }

    /// Each fresh snapshot goes through own-counter validation and invocation accounting.
    pub(super) fn sample_counters(
        &mut self,
        records: &mut ProcessObservations<'_>,
        now: Instant,
    ) -> Vec<CargoGroup> {
        self.sample_counters_with_capture(records, now, &Capture::default(), &[])
    }

    /// Retained CPU accounting sees the same captures and exclusions as row selection.
    fn sample_counters_with_capture(
        &mut self,
        records: &mut ProcessObservations<'_>,
        now: Instant,
        capture: &Capture,
        excluded: &[String],
    ) -> Vec<CargoGroup> {
        for (&pid, record) in &mut records.records {
            let baseline = CpuBaseline {
                lifetime:    record.lifetime.as_ref().clone(),
                accumulated: record.accumulated,
            };
            record.cpu = Census::measure_cpu(pid, 0.0, &baseline, &self.previous);
            self.previous.insert(pid, baseline);
        }
        self.previous
            .retain(|pid, _| records.records.contains_key(pid));
        let mut census = Census::take(records.records.values());
        census.identities = self.fixture_identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        let mut tree = self.invocations.identify(&mut census, records, capture);
        census.prepare(records, capture, excluded);
        let attributed = census.attribute_with(records, &mut self.smoothing, now, |pid| {
            records.process(pid).map_or(
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                |record| record.native_cpu.get(),
            )
        });
        let mut groups = census.groups(records, &attributed, ScannerHome::Unavailable, capture);
        tree.retain_ancestry(&census, &mut groups);
        self.published = self.invocations.publish(
            &census,
            &tree,
            groups.clone(),
            capture,
            Instant::now(),
            |_| InvocationProbe::Uncertain,
        );
        groups
    }

    /// Capture tests can retain deliberately partial ancestry while sharing row assembly.
    fn assemble(
        census: &mut Census,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) -> Vec<CargoGroup> {
        census.prepare(records, capture, excluded);
        census.groups(
            records,
            &InvocationMeasurements {
                #[cfg(test)]
                cpu_scan_publication:              CpuScanPublication::Unobserved,
                cpu:                               HashMap::new(),
                memory:                            HashMap::new(),
                compilers:                         HashMap::new(),
            },
            ScannerHome::Known(Path::new(tests::FIXTURE_WRITER_HOME)),
            capture,
        )
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
#[path = "."]
mod tests {
    #[path = "scan_cpu_scenario_tests.rs"]
    mod scan_cpu_scenario_tests;

    use std::fs;
    use std::io::BufRead;
    use std::io::BufReader;
    use std::io::ErrorKind;
    use std::io::Write;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::process::CommandExt;
    use std::process::Child;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    use sysinfo::Users;
    use tempfile::TempDir;
    use tempfile::tempdir;
    use tui_pane::TABLE_CELL;
    use tui_pane::TileGrowth;

    use super::process_identity::ProcessEndUnproven;
    use super::*;
    use crate::app::App;
    use crate::birth_stamp::BirthStamp;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::census::cpu_observation::CpuScanPublication;
    use crate::constants::CAPTURE_ASSOCIATION_AMBIGUOUS;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::COMPILER_COLUMN;
    use crate::constants::CPU_COLUMN;
    use crate::constants::MANAGED_COLUMN;
    use crate::constants::MEMORY_COLUMN;
    use crate::constants::PID_SEPARATOR;
    use crate::constants::RUN_LOG_PREFIX;
    use crate::constants::RUN_LOG_SUFFIX;
    use crate::constants::TABLE_HEADERS;
    use crate::constants::UNAVAILABLE_MEASUREMENT;
    use crate::progress::capture::CaptureRootIndex;
    use crate::progress::capture_diagnostic::CaptureDiagnostic;
    use crate::progress::capture_read::CaptureRead;
    use crate::progress::capture_read::RunState;
    use crate::progress::capture_roots::AccountName;
    use crate::registration::Registration;
    use crate::root_scan::EffectiveUser;
    use crate::root_scan::RootOwner;
    use crate::roster::FamilyHead;
    use crate::tiles::TileContent;

    pub(super) const FIXTURE_WRITER_HOME: &str = "/writer";

    #[test]
    fn two_absence_cycles_never_revive_either_ended_identity() {
        let arguments = ["cargo", "check"].map(OsString::from);
        let epoch = Instant::now() + Duration::from_secs(60);
        let mut sequence = CensusSequence::default();
        let mut ended = Vec::new();
        for cycle in 0..=2 {
            let now = epoch + Duration::from_secs(cycle * 2);
            sequence.sample_published(
                &ProcessObservations::new([ProcessObservation::cargo(4100, &arguments)]),
                &Capture::default(),
                now,
            );
            let identity = sequence.publication().groups()[0].id();
            assert!(
                !ended.contains(&identity),
                "cycle {cycle} revived an ended identity: {identity:?}"
            );
            ended.push(identity);
            for scan in 1..=4 {
                sequence.sample_published(
                    &ProcessObservations::default(),
                    &Capture::default(),
                    now + Duration::from_millis(scan * crate::constants::PROCESS_POLL_MILLIS),
                );
            }
            assert_eq!(sequence.retained_invocations_for_test(), 0);
        }
    }

    #[test]
    fn native_identity_retirement_requires_its_own_absence_proof() {
        let pid = Pid::from_u32(4100);
        let lifetimes = HashMap::from([(pid, LifetimeEvidence::Unavailable)]);
        let mut identities = ProcessIdentities::default();
        let first = identities.observe(&lifetimes);
        identities.retire_missing(&HashMap::new(), |_| Err(ProcessEndUnproven::Alive));
        assert_eq!(identities.observe(&lifetimes), first);
        identities.retire_missing(&HashMap::new(), |subject| {
            Ok(process_identity::ProvenProcessEnd::for_test(subject))
        });
        assert_ne!(identities.observe(&lifetimes), first);
    }

    #[test]
    fn terminal_aliases_expire_only_after_their_retained_evidence() {
        let pid = Pid::from_u32(4100);
        let alias = InvocationId::for_test(4100);
        let mut invocations = Invocations::default();
        invocations.terminal.end(pid, &alias);
        let replacement = invocations.terminal.subsequent(pid, &alias);
        assert_ne!(replacement, alias);
        invocations.retain_identity_evidence(std::iter::once(alias.clone()));
        assert_eq!(invocations.terminal.subsequent(pid, &alias), replacement);
        invocations.retain_identity_evidence(std::iter::empty());
        assert_eq!(invocations.terminal.subsequent(pid, &alias), alias);
    }

    #[test]
    fn a_reused_pid_does_not_retain_its_old_process_binding() {
        let argv = ["cargo", "check"].map(OsString::from);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        for lifetime in 1..=4 {
            let cargo = ProcessObservation::cargo(4100, &argv).with_lifetime(
                LifetimeEvidence::Available(ProcessLifetime::for_test(lifetime)),
            );
            sequence.sample_published(
                &ProcessObservations::new([cargo]),
                &Capture::default(),
                now + Duration::from_millis(lifetime),
            );
            assert!(
                sequence.invocations.bindings.subjects.len() <= 2,
                "only the current lifetime and its just-ended predecessor remain"
            );
            assert_eq!(
                sequence
                    .invocations
                    .bindings
                    .subjects
                    .values()
                    .filter(|state| { matches!(state, ProcessBindingState::Active(_)) })
                    .count(),
                1,
                "only the current lifetime remains active"
            );
        }
    }

    #[test]
    fn delayed_attribution_uses_counter_read_times() {
        let argv = ["cargo", "check"].map(OsString::from);
        let pid = Pid::from_u32(4100);
        let observations = ProcessObservations::new([ProcessObservation::cargo(4100, &argv)]);
        let mut census = Census::take(observations.records.values());
        census.identities.insert(pid, InvocationId::for_test(4100));
        census.cpu.insert(pid, Measurement::Reading(0.0));
        let mut smoothing = InvocationCpuAccounting::new(CensusCadence {
            poll:      Duration::from_millis(50),
            report:    Duration::ZERO,
            smoothing: Duration::ZERO,
        });
        let origin = Instant::now();
        let actual_read_time = Cell::new(origin);
        // Discovery is 100 ms apart; the counter reads are 400 ms apart.
        let samples = [(0, 100, 100), (100, 500, 500)];
        let mut result = AttributedInvocations::empty().measurements;
        for (discovery_ms, read_ms, counter_ms) in samples {
            actual_read_time.set(origin + Duration::from_millis(discovery_ms));
            result = census.attribute_with_clock(
                &observations,
                &mut smoothing,
                |_| {
                    actual_read_time.set(origin + Duration::from_millis(read_ms));
                    Measurement::Reading(Duration::from_millis(counter_ms))
                },
                || actual_read_time.get(),
            );
        }
        // This oracle uses the supplied read events, never the accounting timestamp.
        let elapsed = Duration::from_millis(samples[1].1 - samples[0].1);
        let work = Duration::from_millis(samples[1].2 - samples[0].2);
        let expected = 100.0 * work.as_secs_f32() / elapsed.as_secs_f32();
        assert_eq!(
            result.cpu[&pid],
            Measurement::Reading(expected),
            "CPU uses the interval between counter reads"
        );
    }

    #[test]
    fn a_blocked_detail_read_keeps_other_processes_attributed() {
        let argv = ["cargo", "check"].map(OsString::from);
        let first = DetailRead::Ready(Arc::new(4100));
        let blocked = DetailRead::Reading;
        let third = DetailRead::Refreshing(Arc::new(4300));
        let StageAnswer::Completed(details) =
            CompletedAttributionDetails::from_reads([&first, &blocked, &third].into_iter())
        else {
            panic!("two completed process reads remain available for attribution");
        };
        let mut observations = ProcessObservations::new([4100, 4200, 4300].map(|pid| {
            let mut process = ProcessObservation::cargo(pid, &argv);
            process.details = DetailPresence::Absent;
            process
        }));
        for pid in &details.0 {
            let process = ProcessObservation::cargo(**pid, &argv);
            observations.records.insert(process.pid, process);
        }
        let mut census = Census::take(observations.records.values());
        for pid in [4100, 4200, 4300].map(Pid::from_u32) {
            census
                .identities
                .insert(pid, InvocationId::for_test(pid.as_u32()));
            census.cpu.insert(pid, Measurement::Reading(0.0));
        }
        let mut smoothing = InvocationCpuAccounting::new(CensusCadence {
            poll:      Duration::from_millis(50),
            report:    Duration::ZERO,
            smoothing: Duration::ZERO,
        });
        let now = Instant::now();
        census.attribute_with_clock(
            &observations,
            &mut smoothing,
            |_| Measurement::Reading(Duration::from_millis(100)),
            || now,
        );
        let readings = census.attribute_with_clock(
            &observations,
            &mut smoothing,
            |_| Measurement::Reading(Duration::from_millis(150)),
            || now + Duration::from_millis(100),
        );
        assert_eq!(readings.cpu.len(), 2, "two completed processes receive CPU");
        for pid in [4100, 4300].map(Pid::from_u32) {
            assert_eq!(readings.cpu[&pid], Measurement::Reading(50.0));
        }
        assert!(
            !readings.cpu.contains_key(&Pid::from_u32(4200)),
            "the blocked process has no completed attribution"
        );
    }

    #[test]
    fn a_non_cargo_ancestor_keeps_forwarding_executions_separate() {
        let argv = ["cargo", "check"].map(OsString::from);
        let make_argv = ["make", "check"].map(OsString::from);
        let outer = ProcessObservation::cargo(4100, &argv);
        let mut make = ProcessObservation::cargo(4101, &make_argv);
        make.name = ProcessField::Observed(OsStr::new("make"));
        make.parent = ProcessField::Observed(Pid::from_u32(4100));
        let mut inner = ProcessObservation::cargo(4102, &argv);
        inner.parent = ProcessField::Observed(Pid::from_u32(4101));
        let mut sequence = CensusSequence::default();
        sequence.sample_published(
            &ProcessObservations::new([outer, make, inner]),
            &Capture::default(),
            Instant::now(),
        );
        let rows: HashMap<_, _> = sequence
            .publication()
            .groups()
            .iter()
            .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
            .map(|row| (row.pid, row.invocation_id.clone()))
            .collect();
        assert_ne!(
            rows[&4100], rows[&4102],
            "make separates the two cargo execution owners"
        );
    }

    #[test]
    fn attribution_requires_completed_details_for_each_current_source() {
        let ready = DetailRead::Ready(Arc::new(1));
        let refreshing = DetailRead::Refreshing(Arc::new(2));
        let StageAnswer::Completed(answers) =
            CompletedAttributionDetails::from_reads([&ready, &refreshing].into_iter())
        else {
            panic!("completed current detail answers must allow attribution");
        };
        assert_eq!(
            answers.0.iter().map(|answer| **answer).collect::<Vec<_>>(),
            [1, 2]
        );
        for pending in [
            DetailRead::Queued,
            DetailRead::Reading,
            DetailRead::ReadingChanged,
            DetailRead::SourceChanged(Arc::new(3)),
            DetailRead::RefreshingChanged(Arc::new(4)),
            DetailRead::ReplacingSource(Arc::new(5)),
        ] {
            let StageAnswer::Completed(answers) =
                CompletedAttributionDetails::from_reads([&ready, &pending].into_iter())
            else {
                panic!("the completed current source supplies attribution");
            };
            assert_eq!(
                answers.0.iter().map(|answer| **answer).collect::<Vec<_>>(),
                [1],
                "only the completed current source supplies attribution"
            );
            assert!(
                matches!(
                    CompletedAttributionDetails::from_reads([&pending].into_iter()),
                    StageAnswer::Pending
                ),
                "a pending source waits for its completed metadata"
            );
        }
    }

    #[test]
    fn retained_discovery_publishes_completed_cpu_without_freshening_lifetime() {
        let argv = ["cargo", "check"].map(OsString::from);
        let mut first = ProcessObservation::cargo(4100, &argv);
        first.cpu = Measurement::Reading(41.0);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(&ProcessObservations::new([first]), &Capture::default(), now);
        let owner = sequence.publication().groups()[0].id();
        let mut replacement = ProcessObservation::cargo(4100, &argv);
        replacement.cpu = Measurement::Reading(36.610_973);
        sequence.sample_with_native_evidence(
            &ProcessObservations::new([replacement]),
            &Capture::default(),
            now + Duration::from_millis(50),
            FixturePublication::Retained,
            |_| Observation::Unknown,
        );
        let publication = sequence.publication();
        assert!(
            matches!(
                publication.state(&owner),
                Some(PublishedInvocation::Missing(_))
            ),
            "new CPU does not turn retained discovery into current lifetime evidence"
        );
        assert_eq!(
            publication.groups()[0].lead.resource_use.cpu,
            Measurement::Reading(36.610_973)
        );
        assert_eq!(
            publication.groups()[0].lead.subtree_resource_use.cpu,
            Measurement::Reading(36.610_973)
        );
        let row = &publication.groups()[0].lead;
        let resources = publication
            .retained_resources(row)
            .expect("Missing resource publication");
        assert_eq!(resources.own.cpu, Measurement::Reading(36.610_973));
        assert_eq!(resources.subtree.cpu, Measurement::Reading(36.610_973));
        assert!(
            publication.live(row).is_err(),
            "retained resources supply no live-row evidence"
        );
        assert!(
            publication
                .state(&owner)
                .expect("Missing state")
                .admissible()
                .is_err()
        );
    }

    #[test]
    fn retained_cpu_reaches_its_owned_row_when_metadata_resolution_is_pending() {
        let argv = ["cargo", "check"].map(OsString::from);
        let records = ProcessObservations::new([ProcessObservation::cargo(4100, &argv)]);
        let mut sequence = CensusSequence::default();
        let capture = Capture::default();
        let now = Instant::now();
        sequence.sample_published(&records, &capture, now);
        let mut group = sequence.publication().groups()[0].clone();
        let owner = group.id();
        let pid = Pid::from_u32(4100);
        group.lead.resource_use.cpu = Measurement::Reading(36.610_973);
        group.lead.subtree_resource_use.cpu = Measurement::Reading(36.610_973);
        let mut census = Census::take(records.records.values());
        census.publication_evidence = PublicationEvidence::Selected {
            processes: ProcessDiscoveryEvidence::Retained,
            captures:  CaptureFreshness::Retained,
        };
        census.identities = sequence.fixture_identities.observe(&census.lifetimes);
        let tree = sequence
            .invocations
            .identify(&mut census, &records, &capture);
        let node = tree.nodes.get(&pid).expect("retained process");
        let subject = sequence.invocations.bindings.subject(&node.native);
        sequence.invocations.bindings.subjects.insert(
            subject,
            ProcessBindingState::Active(InvocationProcessBinding::Forwarder {
                owner:    node.invocation.clone(),
                evidence: ProcessBindingEvidence {
                    origin: node.invocation.clone(),
                },
                executor: ProcessIdentity::observed(4200, LifetimeEvidence::Unavailable),
            }),
        );
        assert!(
            sequence.invocations.bindings.resolve(node).is_err(),
            "pending executor metadata cannot grant admission"
        );
        let publication = sequence.invocations.publish(
            &census,
            &tree,
            vec![group],
            &capture,
            now + Duration::from_millis(50),
            |_| InvocationProbe::Uncertain,
        );
        assert!(matches!(
            publication.state(&owner),
            Some(PublishedInvocation::Missing(_))
        ));
        let row = &publication.groups()[0].lead;
        assert_eq!(row.resource_use.cpu, Measurement::Reading(36.610_973));
        assert_eq!(
            row.subtree_resource_use.cpu,
            Measurement::Reading(36.610_973)
        );
        assert!(
            publication
                .state(&owner)
                .expect("retained state")
                .admissible()
                .is_err()
        );
    }

    #[test]
    fn a_reused_attribution_keeps_its_measurement_stamp() {
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let records = ProcessObservations::new([ProcessObservation::cargo(4100, &argv)]);
        let mut census = Census::take(records.records.values());
        let pid = Pid::from_u32(4100);
        let identity = InvocationId::for_test(4100);
        census.identities.insert(pid, identity.clone());
        let mut first = AttributedInvocations::empty();
        first.identities.insert(pid, identity);
        first
            .measurements
            .cpu
            .insert(pid, Measurement::Reading(27.0));
        first.measurements.cpu_scan_publication = CpuScanPublication::Observed(12);

        // A later discovery reuses the completed attribution, including its scan.
        let reused = first.for_census(&census);
        assert_eq!(reused.cpu.get(&pid), Some(&Measurement::Reading(27.0)));
        assert!(matches!(
            reused.cpu_scan_publication,
            CpuScanPublication::Observed(12)
        ));
        let empty = AttributedInvocations::empty().for_census(&census);
        assert!(empty.cpu.is_empty());
        assert!(matches!(
            empty.cpu_scan_publication,
            CpuScanPublication::Unobserved
        ));

        let mut replacement = AttributedInvocations::empty();
        replacement.identities = first.identities.clone();
        replacement
            .measurements
            .cpu
            .insert(pid, Measurement::Reading(43.0));
        replacement.measurements.cpu_scan_publication = CpuScanPublication::Observed(33);
        let current = replacement.for_census(&census);
        assert_eq!(current.cpu.get(&pid), Some(&Measurement::Reading(43.0)));
        assert!(matches!(
            current.cpu_scan_publication,
            CpuScanPublication::Observed(33)
        ));

        census.identities.insert(pid, InvocationId::for_test(5100));
        let replaced = first.for_census(&census);
        assert!(
            replaced.cpu.is_empty(),
            "a replaced lifetime cannot inherit CPU"
        );
        assert!(matches!(
            replaced.cpu_scan_publication,
            CpuScanPublication::Observed(12)
        ));
    }

    #[test]
    fn a_resolved_process_row_has_one_family_after_roots_overlap() {
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(
            &ProcessObservations::new(
                [4100, 5100, 6100].map(|pid| ProcessObservation::cargo(pid, &argv)),
            ),
            &Capture::default(),
            now,
        );
        let rows: HashMap<_, _> = sequence
            .publication()
            .groups()
            .iter()
            .map(|group| (group.lead.pid, group.lead.clone()))
            .collect();
        let first = rows[&4100].invocation_id.clone();
        let second = rows[&5100].invocation_id.clone();
        let member = rows[&6100].invocation_id.clone();
        sequence
            .invocations
            .records
            .get_mut(&member)
            .expect("member record")
            .membership = Membership::Member {
            lead: second.clone(),
        };
        // Two retained roots contain the member's last row. Its own record now
        // assigns the process to the second family.
        for root in [&first, &second] {
            let record = sequence
                .invocations
                .records
                .get_mut(root)
                .expect("root record");
            let mut frozen = record.state.rows().to_vec();
            frozen.push(rows[&6100].clone());
            record.state = InvocationState::Ended(EndedInvocation {
                id:           root.clone(),
                rows:         frozen.into(),
                at:           now,
                presentation: record.presentation.clone(),
                published:    sequence.invocations.scan,
            });
        }
        let published = sequence.invocations.publication();
        for pid in [4100, 5100, 6100] {
            let owners: Vec<_> = published
                .groups()
                .iter()
                .filter(|group| {
                    std::iter::once(&group.lead)
                        .chain(&group.rest)
                        .any(|row| row.pid == pid)
                })
                .map(CargoGroup::id)
                .collect();
            assert_eq!(owners.len(), 1, "pid {pid} has more than one family");
            if pid == 6100 {
                assert_eq!(
                    owners.as_slice(),
                    std::slice::from_ref(&second),
                    "the canonical record selects the family"
                );
                assert_eq!(
                    published
                        .groups()
                        .iter()
                        .flat_map(|group| { std::iter::once(&group.lead).chain(&group.rest) })
                        .filter(|row| row.pid == pid)
                        .count(),
                    1,
                    "one row per resolved process"
                );
            }
        }
    }

    #[test]
    fn a_registered_pipeline_resolves_each_forwarder_before_admission() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            4100,
            "execution",
            "/work",
            "/writer",
            "check",
            "",
        );
        let capture = verified_capture(root.path());
        let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", "100") else {
            panic!("fixture birth");
        };
        let argv = ["cargo", "check"].map(OsString::from);
        let shell = ["/bin/sh", "/capture/cargo", "check"].map(OsString::from);
        let mut shim = ProcessObservation::cargo(4100, &shell).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(&birth)),
        );
        shim.name = ProcessField::Observed(OsStr::new("sh"));
        let mut pipeline = ProcessObservation::cargo(4101, &shell);
        pipeline.name = ProcessField::Observed(OsStr::new("sh"));
        pipeline.parent = ProcessField::Observed(Pid::from_u32(4100));
        let mut cargo = ProcessObservation::cargo(4102, &argv);
        cargo.parent = ProcessField::Observed(Pid::from_u32(4101));
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(&ProcessObservations::new([shim.clone()]), &capture, now);
        let owner = sequence.publication().groups()[0].id();
        let discovery = ProcessObservations::new([
            shim.clone().without_details(),
            pipeline.clone().without_details(),
            cargo.clone().without_details(),
        ]);
        let details = ProcessObservations::new([shim, pipeline, cargo]);
        // Production classifies discovery before the asynchronous detail overlay.
        let mut census = Census::take(discovery.records.values());
        census.identities = sequence.fixture_identities.observe(&census.lifetimes);
        let mut tree = sequence
            .invocations
            .identify(&mut census, &details, &capture);
        census.prepare(&details, &capture, &[]);
        let measurements = InvocationMeasurements {
            cpu_scan_publication: CpuScanPublication::Unobserved,
            cpu:                  HashMap::new(),
            memory:               HashMap::new(),
            compilers:            HashMap::new(),
        };
        let mut groups = census.groups(&details, &measurements, ScannerHome::Unavailable, &capture);
        tree.retain_ancestry(&census, &mut groups);
        let published = sequence.invocations.publish(
            &census,
            &tree,
            groups,
            &capture,
            now + Duration::from_millis(50),
            |_| InvocationProbe::Uncertain,
        );
        assert_eq!(published.groups().len(), 1, "one pipeline is one execution");
        let group = &published.groups()[0];
        assert_eq!(
            group.id(),
            owner,
            "the registered execution keeps its owner"
        );
        assert_eq!(group.lead.pid, 4102, "cargo supplies the row");
        assert!(group.rest.is_empty(), "pipeline shells do not request rows");
        assert!(
            matches!(published.state(&owner), Some(PublishedInvocation::Live(_))),
            "completed native details grant admission"
        );
    }

    #[test]
    fn a_late_registered_forwarder_keeps_its_live_parent_in_one_execution() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(root.path(), 10, "outer", "/work", "/writer", "check", "");
        let capture = verified_capture(root.path());
        let IdentityEvidence::Available(stamp) = BirthStamp::from_fields("boot", "100") else {
            panic!("fixture birth");
        };
        let argv = ["cargo", "check"].map(OsString::from);
        let shell = ["/bin/sh", "/capture/cargo", "check"].map(OsString::from);
        let make = |pid| {
            ProcessObservation::cargo(pid, &argv).with_lifetime(LifetimeEvidence::Available(
                ProcessLifetime::matching_registration_for_test(&stamp),
            ))
        };
        let parent = make(10);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(&ProcessObservations::new([parent.clone()]), &capture, now);
        let owner = sequence.publication().groups()[0].id();
        write_versioned_capture(root.path(), 20, "inner", "/work", "/writer", "check", "");
        let capture = verified_capture(root.path());
        let mut forwarder = make(20);
        forwarder.argv = ProcessField::Observed(&shell);
        forwarder.name = ProcessField::Observed(OsStr::new("sh"));
        forwarder.parent = ProcessField::Observed(Pid::from_u32(10));
        let mut executor = make(30);
        executor.parent = ProcessField::Observed(Pid::from_u32(20));
        let records = ProcessObservations::new([parent, forwarder, executor]);
        for millis in [2840, 3500, 5000, 6000] {
            sequence.sample_published(&records, &capture, now + Duration::from_millis(millis));
            let groups = sequence.publication().groups();
            let rows: Vec<_> = groups
                .iter()
                .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
                .map(|row| row.pid)
                .collect();
            assert_eq!(
                rows,
                [30],
                "a live forwarding parent returned after registration at {millis}ms"
            );
            assert_eq!(groups[0].id(), owner, "the first owner stays assigned");
        }
    }

    #[test]
    fn a_live_capture_binding_retires_its_unavailable_process_alias_before_the_end() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            4100,
            "execution",
            "/work",
            "/writer",
            "check",
            "",
        );
        let capture = verified_capture(root.path());
        let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", "100") else {
            panic!("fixture birth");
        };
        let argv = ["cargo", "check"].map(OsString::from);
        let mut cargo =
            ProcessObservation::cargo(4102, &argv).with_lifetime(LifetimeEvidence::Unavailable);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(
            &ProcessObservations::new([cargo.clone()]),
            &Capture::default(),
            now,
        );
        let process = sequence.publication().groups()[0].id();
        let shim = ProcessObservation::cargo(4100, &argv).with_lifetime(
            LifetimeEvidence::Available(ProcessLifetime::matching_registration_for_test(&birth)),
        );
        cargo.parent = ProcessField::Observed(Pid::from_u32(4100));
        let records = ProcessObservations::new([shim, cargo]);
        for millis in [2840, 3500, 5000] {
            sequence.sample_published(&records, &capture, now + Duration::from_millis(millis));
            let published = sequence.publication();
            assert_eq!(
                published.groups().len(),
                1,
                "one live execution at {millis}ms"
            );
            assert!(
                matches!(published.groups()[0].id(), InvocationId::Captured(_)),
                "the capture owns its current cargo process at {millis}ms"
            );
            assert_eq!(published.groups()[0].lead.pid, 4102);
            assert_eq!(published.groups()[0].rest, [] as [CargoProcess; 0]);
            assert!(
                matches!(
                    published.state(&process),
                    Some(PublishedInvocation::Joined { .. })
                ),
                "the process alias joins before its end at {millis}ms"
            );
        }
        let captured = sequence.publication().groups()[0].id();
        let ended_capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Ended)
        });
        sequence.sample_with_native_evidence(
            &ProcessObservations::default(),
            &ended_capture,
            now + Duration::from_millis(6000),
            FixturePublication::Current,
            |_| Observation::Ended,
        );
        assert!(matches!(
            sequence.publication().state(&captured),
            Some(PublishedInvocation::Ended(_))
        ));
        assert!(matches!(
            sequence.publication().state(&process),
            Some(PublishedInvocation::Joined { .. })
        ));
        assert_eq!(
            sequence
                .publication()
                .states
                .values()
                .filter(|state| matches!(state, PublishedInvocation::Ended(_)))
                .count(),
            1,
            "the resolved execution emits one end"
        );
    }

    #[test]
    fn retained_discovery_does_not_advance_the_missing_observation_limit() {
        let argv = ["cargo", "check"].map(OsString::from);
        let records = ProcessObservations::new([ProcessObservation::cargo(4100, &argv)]);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(&records, &Capture::default(), now);
        let owner = sequence.publication().groups()[0].id();
        for publication in 1..=MISSING_SCAN_LIMIT + 2 {
            sequence.sample_with_native_evidence(
                &ProcessObservations::default(),
                &Capture::default(),
                now + Duration::from_millis(publication),
                FixturePublication::Retained,
                |_| Observation::Unknown,
            );
            assert!(
                matches!(
                    sequence.publication().state(&owner),
                    Some(PublishedInvocation::Missing(_))
                ),
                "retained publication {publication} is not fresh absence evidence"
            );
        }
        for observation in 1..=MISSING_SCAN_LIMIT {
            sequence.sample_with_native_evidence(
                &ProcessObservations::default(),
                &Capture::default(),
                now + Duration::from_millis(10 + observation),
                FixturePublication::Current,
                |_| Observation::Unknown,
            );
            let state = sequence.publication().state(&owner);
            if observation == MISSING_SCAN_LIMIT {
                assert!(matches!(state, Some(PublishedInvocation::Ended(_))));
            } else {
                assert!(matches!(state, Some(PublishedInvocation::Missing(_))));
            }
        }
    }

    #[test]
    fn a_current_native_lifetime_does_not_repeat_an_end_probe() {
        let argv = ["cargo", "check"].map(OsString::from);
        let records = ProcessObservations::new([ProcessObservation::cargo(10, &argv)]);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        let capture = Capture::default();
        sequence.sample_published(&records, &capture, now);
        let owner = sequence.publication().groups()[0].id();
        let mut census = Census::take(records.records.values());
        census.identities = sequence.fixture_identities.observe(&census.lifetimes);
        let tree = sequence
            .invocations
            .identify(&mut census, &records, &capture);
        let mut reads = 0;
        // Row selection can omit a process whose native discovery is current.
        sequence
            .invocations
            .publish(&census, &tree, Vec::new(), &capture, now, |_| {
                reads += 1;
                InvocationProbe::Alive
            });
        assert_eq!(
            reads, 0,
            "a current matching birth already proves the same native process"
        );
        census.publication_evidence = PublicationEvidence::Selected {
            processes: ProcessDiscoveryEvidence::Retained,
            captures:  CaptureFreshness::Retained,
        };
        let published =
            sequence
                .invocations
                .publish(&census, &tree, Vec::new(), &capture, now, |_| {
                    reads += 1;
                    InvocationProbe::Alive
                });
        assert_eq!(
            reads, 1,
            "a retained snapshot still needs a current native probe"
        );
        assert!(matches!(
            published.state(&owner),
            Some(PublishedInvocation::Missing(_))
        ));
        census.publication_evidence = PublicationEvidence::Current;
        census
            .process_statuses
            .insert(Pid::from_u32(10), NativeProcessStatus::Zombie);
        let published =
            sequence
                .invocations
                .publish(&census, &tree, Vec::new(), &capture, now, |row| {
                    reads += 1;
                    InvocationProbe::Ended(ProcessLifetimeSubject::observe(row))
                });
        assert_eq!(
            reads, 2,
            "a zombie's matching birth cannot prove continued execution"
        );
        assert!(matches!(
            published.state(&owner),
            Some(PublishedInvocation::Ended(_))
        ));
    }

    #[test]
    fn a_process_binding_leaves_with_its_process_tree_node() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "execution",
            "/work",
            "/writer",
            "check",
            "",
        );
        let capture = verified_capture(root.path());
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let mut sequence = CensusSequence::default();
        sequence.sample_published(
            &ProcessObservations::new([ProcessObservation::cargo(20, &argv)]),
            &capture,
            Instant::now(),
        );
        assert!(
            !sequence.invocations.bindings.registrations.is_empty(),
            "capture binding exists"
        );
        let mut tree = ProcessTreeContinuity::default();
        for record in sequence.invocations.records.values() {
            tree.nodes.extend(record.processes.iter().cloned());
        }
        assert!(
            tree.nodes.remove(&Pid::from_u32(10)).is_some(),
            "registered node exists"
        );
        sequence.invocations.bindings.retain(&mut tree);
        assert!(
            sequence.invocations.bindings.registrations.is_empty(),
            "departed registrations release their binding"
        );
        assert_eq!(
            sequence.invocations.bindings.subjects.len(),
            1,
            "only the other execution keeps its subject binding"
        );
        assert!(
            tree.nodes.contains_key(&Pid::from_u32(20)),
            "the other execution stays bound"
        );
    }

    #[test]
    fn a_forwarded_process_is_not_ended_by_its_owners_birth() {
        let pid = Pid::from_u32(std::process::id());
        let lifetime = birth_stamp::lifetime(pid.as_u32());
        assert!(matches!(lifetime, LifetimeEvidence::Available(_)));
        let owner = InvocationId::Process(ProcessIdentity::Known {
            pid:      pid.as_u32().saturating_add(1),
            lifetime: ProcessLifetime::for_test(1),
        });
        let argv = [OsString::from("cargo"), OsString::from("build")];
        let observation = ProcessObservation::cargo(pid.as_u32(), &argv).with_lifetime(lifetime);
        let census = Census::take([&observation]);
        let mut process = directory_row();
        process.pid = pid.as_u32();
        process.invocation_id = owner;
        let row = InvocationProcessRow::observed(process, &census);
        assert!(matches!(
            InvocationProbe::kernel(ProcessLifetimeSubject::observe(&row)),
            InvocationProbe::Alive
        ));
    }

    #[test]
    fn an_unread_executor_does_not_turn_its_forwarder_into_a_second_execution() {
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let parent = ProcessObservation::cargo(4100, &argv);
        let mut child = ProcessObservation::cargo(4102, &argv);
        child.parent = ProcessField::Observed(Pid::from_u32(4100));
        let now = Instant::now();
        let mut sequence = CensusSequence::default();
        sequence.sample_published(
            &ProcessObservations::new([parent.clone(), child.clone()]),
            &Capture::default(),
            now,
        );
        let owner = sequence.publication().groups()[0].id();
        assert_eq!(sequence.publication().groups()[0].lead.pid, 4102);
        sequence.sample_published(
            &ProcessObservations::new([parent.clone()]),
            &Capture::default(),
            now,
        );
        assert_eq!(sequence.publication().groups().len(), 1);
        assert_eq!(
            sequence.publication().groups()[0].lead.pid,
            4102,
            "an unread executor keeps its own row"
        );
        sequence.sample_published(
            &ProcessObservations::new([parent, child]),
            &Capture::default(),
            now,
        );
        assert_eq!(sequence.publication().groups()[0].id(), owner);
        assert_eq!(sequence.publication().groups().len(), 1);
    }

    #[test]
    fn a_missing_registration_cannot_end_its_verified_cargo_executor() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "execution",
            "/work",
            "/writer",
            "check",
            "",
        );
        let capture = verified_capture(root.path());
        let stamp = match BirthStamp::from_fields("boot", "100") {
            IdentityEvidence::Available(stamp) => stamp,
            IdentityEvidence::Unavailable => panic!("test fields must identify a lifetime"),
        };
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let records = ProcessObservations::new([ProcessObservation::cargo(10, &argv)
            .with_lifetime(LifetimeEvidence::Available(
                ProcessLifetime::matching_registration_for_test(&stamp),
            ))]);
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        sequence.sample_published(&records, &capture, now);
        let owner = sequence.publication().groups()[0].id();
        for _ in 0..4 {
            sequence.sample_with_native_evidence(
                &records,
                &Capture::default(),
                now,
                FixturePublication::Retained,
                |_| Observation::Present(stamp.clone()),
            );
            assert!(
                matches!(
                    sequence.publication().state(&owner),
                    Some(PublishedInvocation::Live(_) | PublishedInvocation::Missing(_))
                ),
                "a living cargo executor remains the same execution without its registration"
            );
        }
    }

    #[test]
    fn a_proven_execution_end_retires_every_forwarding_subject() {
        let argv = [OsString::from("cargo"), OsString::from("check")];
        let parent = ProcessObservation::cargo(4100, &argv);
        let mut child = ProcessObservation::cargo(4102, &argv);
        child.parent = ProcessField::Observed(Pid::from_u32(4100));
        let now = Instant::now();
        let mut sequence = CensusSequence::default();
        sequence.sample_published(
            &ProcessObservations::new([parent.clone(), child]),
            &Capture::default(),
            now,
        );
        let owner = sequence.publication().groups()[0].id();
        sequence.publish_fixture(
            &ProcessObservations::new([]),
            &Capture::default(),
            now,
            |row| {
                InvocationProbe::Ended(ProcessLifetimeSubject {
                    pid:                Pid::from_u32(row.process.pid),
                    lifetime:           row.lifetime.clone(),
                    kernel_observation: KernelObservation::for_test(
                        row.process.pid,
                        Observation::Ended,
                    ),
                })
            },
        );
        assert!(matches!(
            sequence.publication().state(&owner),
            Some(PublishedInvocation::Ended(_))
        ));
        sequence.sample_published(
            &ProcessObservations::new([parent]),
            &Capture::default(),
            now,
        );
        assert!(
            sequence.publication().groups().is_empty(),
            "an ended forwarder cannot acquire another owner"
        );
    }

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

    /// Owned text supports borrowed records without a process or leaked allocation.
    struct CargoFixture {
        pid:       u32,
        argv:      Vec<OsString>,
        directory: PathBuf,
    }

    impl CargoFixture {
        fn observation(&self) -> ProcessObservation<'_> {
            let mut record = ProcessObservation::cargo(self.pid, &self.argv);
            record.directory = ProcessField::Observed(&self.directory);
            record
        }
    }

    fn cargo_process(command: &str) -> CargoFixture {
        CargoFixture {
            pid:       if command == "test" { 101 } else { 100 },
            argv:      vec![OsString::from(CARGO_DISPLAY_NAME), OsString::from(command)],
            directory: std::env::current_dir().expect("test directory"),
        }
    }

    /// Measurements are supplied explicitly by tests that need process evidence.
    fn no_measurements() -> InvocationMeasurements {
        InvocationMeasurements {
            cpu_scan_publication: CpuScanPublication::Unobserved,
            compilers:            HashMap::new(),
            cpu:                  HashMap::new(),
            memory:               HashMap::new(),
        }
    }

    /// A blocked OS stage has one request and never owns the publication thread.
    #[test]
    fn blocked_refresh_capture_and_attribution_leave_publication_available()
    -> Result<(), Box<dyn std::error::Error>> {
        for name in [
            CENSUS_REFRESH_THREAD_NAME,
            CENSUS_CAPTURE_THREAD_NAME,
            CENSUS_ATTRIBUTION_THREAD_NAME,
        ] {
            let (entered, started) = mpsc::channel();
            let (release, gate) = mpsc::channel();
            let mut stage = PublicationStage::new(name, move |request: u8| {
                entered.send(()).expect("worker starts");
                gate.recv().expect("worker released");
                request
            })?;
            assert!(matches!(stage.sample(|| 1), StageAnswer::Pending));
            started
                .recv_timeout(Duration::from_millis(100))
                .expect("worker entered blocking read");
            let before = Instant::now();
            for _ in 0..8 {
                assert!(matches!(
                    stage.sample(|| panic!("blocked worker acquired a second request")),
                    StageAnswer::Pending
                ));
            }
            assert!(
                before.elapsed() < Duration::from_millis(50),
                "publication waited for {name}"
            );
            release.send(()).expect("finish blocked read");
        }
        Ok(())
    }

    #[test]
    fn an_installed_shim_pipeline_keeps_the_driver_and_child_pids() {
        let shim = [
            OsString::from("/bin/sh"),
            "/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo".into(),
            "p20bdrv".into(),
        ];
        let driver = [
            OsString::from("/bin/sh"),
            "/tools/drvbin/cargo-p20bdrv".into(),
            "p20bdrv".into(),
        ];
        let nextest = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let tee = [
            OsString::from("tee"),
            "-a".into(),
            "/capture/run.log".into(),
        ];
        let records = ProcessObservations::new(
            [
                (2_928_730, 1, "sh", shim.as_slice()),
                (2_929_160, 2_928_730, "sh", shim.as_slice()),
                (2_929_161, 2_929_160, "sh", shim.as_slice()),
                (2_929_162, 2_929_161, "cargo-p20bdrv", driver.as_slice()),
                (2_929_286, 2_929_162, "cargo-nextest", nextest.as_slice()),
                (2_929_163, 2_929_160, "tee", tee.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 2_929_162);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 2_929_286);
        assert_eq!(
            groups[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  groups[0].lead.invocation_id.clone(),
                pid: 2_929_162,
            }
        );
    }

    #[test]
    fn a_rustup_shim_dispatched_nextest_counts_only_its_managed_run() {
        let shim = [
            "/bin/sh",
            "/home/u/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin/cargo",
            "nextest",
            "run",
            "--locked",
        ]
        .map(OsString::from);
        let nextest = ["cargo-nextest", "nextest", "run", "--locked"].map(OsString::from);
        let test = ["cargo", "test", "--no-run"].map(OsString::from);
        let records = ProcessObservations::new(
            [
                (3_214_500, 1, "cargo", shim.as_slice()),
                (3_215_101, 3_214_500, "cargo", shim.as_slice()),
                (3_215_102, 3_215_101, "cargo", shim.as_slice()),
                (3_215_108, 3_215_102, "cargo-nextest", nextest.as_slice()),
                (3_262_399, 3_215_108, "cargo", test.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 3_215_108);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 3_262_399);
        assert_eq!(groups[0].lead.managed, Measurement::Reading(1));
    }

    #[test]
    fn a_dispatched_nextest_keeps_its_hidden_ci_drivers_runs() {
        let shim = [
            OsString::from("/bin/sh"),
            "/tools/bin/cargo".into(),
            "nextest".into(),
            "run".into(),
            "--profile".into(),
            "ci".into(),
        ];
        let nextest = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
            "--profile".into(),
            "ci".into(),
        ];
        let records = ProcessObservations::new(
            [
                (172_043, 170_718, "sh", shim.as_slice()),
                (172_044, 172_043, "sh", shim.as_slice()),
                (172_047, 172_044, "cargo-nextest", nextest.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record
            }),
        );
        let discovered = Census::take(records.processes().values());
        assert_eq!(
            discovered.cargo.len(),
            3,
            "both interpreted drivers must enter discovery"
        );
        let children = discovered.cargo_children();
        assert_eq!(
            Census::descendants(&children, Pid::from_u32(172_043)).len(),
            2
        );
        assert_eq!(
            Census::descendants(&children, Pid::from_u32(172_044)).len(),
            1
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 172_047);
        assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
        assert_eq!(groups[0].lead.managed, Measurement::Reading(2));
    }

    #[test]
    fn a_driver_row_carries_the_dispatched_process_pid() {
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [OsString::from("/tools/cargo-p20driver"), "p20driver".into()];
        let child = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let records = ProcessObservations::new(
            [
                (100, 1, "cargo", shim.as_slice()),
                (200, 100, "cargo-p20driver", driver.as_slice()),
                (300, 200, "cargo-nextest", child.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                if pid == 200 {
                    record.directory = ProcessField::Unavailable;
                }
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 200);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 300);
        assert_eq!(
            groups[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  groups[0].lead.invocation_id.clone(),
                pid: 200,
            }
        );
    }

    #[test]
    fn an_interpreted_driver_carries_its_dispatched_pid_and_capture_identity() {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            100,
            "driver-generation",
            "/work",
            "/writer",
            "p20driver",
            "",
        );
        let capture = verified_capture(root.path());
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [
            OsString::from("python3"),
            "/tools/cargo-p20driver".into(),
            "p20driver".into(),
        ];
        let child = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let records = ProcessObservations::new(
            [
                (100, 1, "cargo", shim.as_slice()),
                (200, 100, "python3", driver.as_slice()),
                (300, 200, "cargo-nextest", child.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record
            }),
        );
        let mut records = records;
        let confirmed = capture.confirmed();
        let run = RunId::verified(&confirmed[0].key, &confirmed[0].registration);
        records
            .records
            .get_mut(&Pid::from_u32(100))
            .expect("observed shim")
            .lifetime = Cow::Owned(LifetimeEvidence::Available(
            ProcessLifetime::matching_registration_for_test(&run.birth),
        ));
        let groups = CensusSequence::default().sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 200);
        assert!(
            matches!(&groups[0].lead.invocation_id, InvocationId::Captured(run) if run.shim_pid == 100)
        );
        assert_eq!(
            groups[0].lead.command,
            CommandText::of("cargo", &["p20driver"])
        );
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 300);
        assert_eq!(
            groups[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  groups[0].lead.invocation_id.clone(),
                pid: 200,
            }
        );
        assert!(matches!(
            groups[0].rest[0].capture_membership,
            CaptureMembership::Enclosing(_)
        ));
    }

    #[test]
    fn a_driver_that_changes_directory_keeps_its_dispatched_pid() {
        let root = tempdir().expect("directories");
        let parent_directory = root.path().join("parent");
        let child_directory = root.path().join("child");
        fs::create_dir(&parent_directory).expect("parent directory");
        fs::create_dir(&child_directory).expect("child directory");
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [OsString::from("/tools/cargo-p20driver"), "p20driver".into()];
        let nextest = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let mut parent = ProcessObservation::cargo(100, &shim);
        parent.directory = ProcessField::Observed(&parent_directory);
        let mut child = ProcessObservation::cargo(200, &driver);
        child.name = ProcessField::Observed(OsStr::new("cargo-p20driver"));
        child.directory = ProcessField::Observed(&child_directory);
        child.parent = ProcessField::Observed(Pid::from_u32(100));
        let mut nested = ProcessObservation::cargo(300, &nextest);
        nested.name = ProcessField::Observed(OsStr::new("cargo-nextest"));
        nested.parent = ProcessField::Observed(Pid::from_u32(200));
        let groups =
            CensusSequence::default().sample(&ProcessObservations::new([parent, child, nested]));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 200);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 300);
        assert_eq!(
            groups[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  groups[0].lead.invocation_id.clone(),
                pid: 200,
            }
        );
    }

    #[test]
    fn an_interpreter_with_flags_keeps_the_driver_and_child_pids() {
        let shim = [OsString::from("cargo"), "p20driver".into()];
        let driver = [
            OsString::from("python3"),
            "-u".into(),
            "/tools/cargo-p20driver".into(),
            "p20driver".into(),
        ];
        let nextest = [
            OsString::from("cargo-nextest"),
            "nextest".into(),
            "run".into(),
        ];
        let records = ProcessObservations::new(
            [
                (100, 1, "cargo", shim.as_slice()),
                (200, 100, "python3", driver.as_slice()),
                (300, 200, "cargo-nextest", nextest.as_slice()),
            ]
            .map(|(pid, parent, name, argv)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 200);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 300);
        assert_eq!(
            groups[0].rest[0].parent,
            VisibleParent::Invocation {
                id:  groups[0].lead.invocation_id.clone(),
                pid: 200,
            }
        );
    }

    #[test]
    fn a_shared_pid_contributes_once_to_its_parent_subtree() {
        let mut parent = directory_row();
        parent.resource_use = InvocationResourceUse {
            cpu:      Measurement::Reading(10.0),
            memory:   Measurement::Reading(100),
            compiler: CompilerObservation::None,
        };
        let mut unproven = directory_row();
        unproven.invocation_id = InvocationId::for_test(21);
        unproven.pid = 20;
        unproven.parent = VisibleParent::Invocation {
            id:  parent.invocation_id.clone(),
            pid: parent.pid,
        };
        unproven.resource_use = InvocationResourceUse::unavailable(MeasurementAbsence::Unproven);
        let mut known = unproven.clone();
        known.invocation_id = InvocationId::for_test(20);
        known.resource_use = InvocationResourceUse {
            cpu:      Measurement::Reading(20.0),
            memory:   Measurement::Reading(200),
            compiler: CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 2,
            }),
        };
        let mut also_known = known.clone();
        also_known.invocation_id = InvocationId::for_test(22);
        for children in [
            [&unproven, &known],
            [&known, &unproven],
            [&known, &also_known],
        ] {
            let totals = subtree_resource_totals(&[&parent, children[0], children[1]]);
            assert_eq!(totals[0].cpu, Measurement::Reading(30.0));
            assert_eq!(totals[0].memory, Measurement::Reading(300));
            assert_eq!(totals[0].compiler, known.resource_use.compiler);
        }
    }

    #[test]
    fn different_external_arguments_keep_both_invocations() {
        let shim = [
            OsString::from("cargo"),
            "p20driver".into(),
            "--jobs".into(),
            "2".into(),
        ];
        let driver = [
            OsString::from("/tools/cargo-p20driver"),
            "p20driver".into(),
            "--jobs".into(),
            "4".into(),
        ];
        let parent = ProcessObservation::cargo(100, &shim);
        let mut child = ProcessObservation::cargo(200, &driver);
        child.name = ProcessField::Observed(OsStr::new("cargo-p20driver"));
        child.parent = ProcessField::Observed(Pid::from_u32(100));
        let groups = CensusSequence::default().sample(&ProcessObservations::new([parent, child]));
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, 100);
        assert_eq!(groups[0].rest.len(), 1);
        assert_eq!(groups[0].rest[0].pid, 200);
    }

    #[test]
    fn each_invocation_keeps_its_own_resource_use_including_the_lead() {
        let parent = [OsString::from("cargo"), "build".into()];
        let child = [OsString::from("cargo"), "check".into()];
        let compiler = [OsString::from("sccache"), "rustc".into()];
        let records = ProcessObservations::new(
            [
                (100, 1, "cargo", parent.as_slice(), 10.25, 100),
                (200, 100, "cargo", child.as_slice(), 89.75, 200),
                (300, 200, "sccache", compiler.as_slice(), 0.0, 300),
            ]
            .map(|(pid, parent, name, argv, cpu, memory)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.name = ProcessField::Observed(OsStr::new(name));
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record.cpu = Measurement::Reading(cpu);
                record.memory = memory;
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        let lead = &groups[0].lead;
        assert_eq!(lead.resource_use.cpu, Measurement::Reading(10.25));
        assert_eq!(lead.resource_use.memory, Measurement::Reading(100));
        assert_eq!(lead.resource_use.compiler, CompilerObservation::None);
        let child = &groups[0].rest[0];
        assert_eq!(child.resource_use.cpu, Measurement::Reading(89.75));
        assert_eq!(child.resource_use.memory, Measurement::Reading(500));
        assert_eq!(
            child.resource_use.compiler,
            CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 1,
            })
        );
        assert_eq!(lead.subtree_resource_use.cpu, Measurement::Reading(100.0));
        assert_eq!(lead.subtree_resource_use.memory, Measurement::Reading(600));
        assert_eq!(
            lead.subtree_resource_use.compiler,
            child.resource_use.compiler
        );
    }

    #[test]
    fn an_unavailable_contributor_keeps_the_total_unavailable_and_the_lead_known() {
        let parent = [OsString::from("cargo"), "build".into()];
        let child = [OsString::from("cargo"), "check".into()];
        let records = ProcessObservations::new(
            [
                (100, 1, parent.as_slice(), Measurement::Reading(12.5)),
                (
                    200,
                    100,
                    child.as_slice(),
                    Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                ),
            ]
            .map(|(pid, parent, argv, cpu)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record.cpu = cpu;
                record
            }),
        );
        let groups = CensusSequence::default().sample(&records);
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].lead.subtree_resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
        assert_eq!(groups[0].lead.resource_use.cpu, Measurement::Reading(12.5));
    }

    #[test]
    fn resource_use_sums_numeric_contributions_before_rounding() {
        let total = InvocationResourceUse::sum([
            InvocationResourceUse {
                cpu:      Measurement::Reading(0.375),
                memory:   Measurement::Reading(100),
                compiler: CompilerObservation::Running(Compiler {
                    name:  SCCACHE_BINARY,
                    count: 2,
                }),
            },
            InvocationResourceUse {
                cpu:      Measurement::Reading(0.375),
                memory:   Measurement::Reading(200),
                compiler: CompilerObservation::Running(Compiler {
                    name:  SCCACHE_BINARY,
                    count: 3,
                }),
            },
        ]);
        assert_eq!(total.cpu, Measurement::Reading(0.75));
        assert_eq!(
            total.cpu.map(cpu_label),
            Measurement::Reading("1%".to_owned())
        );
        assert_eq!(total.memory, Measurement::Reading(300));
        assert_eq!(
            total.compiler,
            CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 5,
            })
        );
    }

    #[test]
    fn shared_pid_readings_replace_only_their_missing_measurements() {
        let first = InvocationResourceUse {
            cpu:      Measurement::Reading(10.0),
            memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
            compiler: CompilerObservation::Unknown,
        };
        let second = InvocationResourceUse {
            cpu:      Measurement::Unavailable(MeasurementAbsence::ReadFailed),
            memory:   Measurement::Reading(200),
            compiler: CompilerObservation::Running(Compiler {
                name:  SCCACHE_BINARY,
                count: 2,
            }),
        };
        for readings in [
            [first.clone(), second.clone()],
            [second.clone(), first.clone()],
        ] {
            let total =
                sum_resource_use_once_per_pid(readings.map(|reading| (Pid::from_u32(10), reading)));
            assert_eq!(total.cpu, first.cpu);
            assert_eq!(total.memory, second.memory);
            assert_eq!(total.compiler, second.compiler);
        }
        let total = sum_resource_use_once_per_pid([
            (Pid::from_u32(10), first),
            (Pid::from_u32(20), second),
        ]);
        assert_eq!(
            total.cpu,
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
        assert_eq!(
            total.memory,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(total.compiler, CompilerObservation::Unknown);
    }

    #[test]
    fn resource_use_sums_keep_each_unavailable_measurement() {
        let total = InvocationResourceUse::sum([
            InvocationResourceUse {
                cpu:      Measurement::Reading(10.0),
                memory:   Measurement::Reading(100),
                compiler: CompilerObservation::None,
            },
            InvocationResourceUse {
                cpu:      Measurement::Unavailable(MeasurementAbsence::FirstObservation),
                memory:   Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                compiler: CompilerObservation::Unknown,
            },
        ]);
        assert_eq!(
            total.cpu,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert_eq!(
            total.memory,
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
        assert_eq!(total.compiler, CompilerObservation::Unknown);
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
            "    Blocking waiting for file lock on build directory",
        );
        let capture = verified_capture(root.path());
        let mut census = census_of(&[]);
        let groups =
            CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
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
            row.resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            row.managed,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(row.resource_use.compiler, CompilerObservation::Unknown);
        assert_eq!(
            row.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
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
        let groups =
            CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
        assert_eq!(groups.len(), 1);
        let RowProvenance::Direct(context) = &groups[0].lead.provenance else {
            panic!("missing account name does not remove verified provenance");
        };
        assert_eq!(context.account.uid, uid);
        assert_eq!(context.account.name, AccountName::Unavailable);
    }

    /// The worker names an uncaptured row's owner from the table that named its roots.
    #[test]
    fn an_uncaptured_row_names_its_process_owner_on_the_worker() {
        let root = tempdir().expect("root");
        let capture = verified_capture(root.path());
        let EffectiveUser::Known(uid) = crate::root_scan::effective_user() else {
            panic!("fixture owner");
        };
        assert_eq!(capture.root_status[0].root.uid, uid);
        let fixture = cargo_process("build");
        for (observed, expected) in [
            (
                ProcessField::Observed(uid),
                ProcessOwner::Observed(Account {
                    uid,
                    name: capture.root_status[0].account.clone(),
                }),
            ),
            (ProcessField::Unavailable, ProcessOwner::Unavailable),
        ] {
            let mut record = fixture.observation();
            record.uid = observed;
            let records = ProcessObservations::new([record]);
            let groups = CensusSequence::default().sample_capture(&records, &capture);
            assert_eq!(groups.len(), 1);
            assert_eq!(
                groups[0].lead.provenance,
                RowProvenance::Uncaptured(expected)
            );
        }
        assert_eq!(
            capture.account_name(uid),
            AccountName::resolve(RootOwner::Uid(uid), &Users::new_with_refreshed_list())
        );
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
            Err(RowAbsence::ProgramRejected)
        );
    }

    #[test]
    fn process_with_unavailable_cwd_keeps_pid_and_measurements_after_merge() {
        let fixture = cargo_process("build");
        let pid = Pid::from_u32(fixture.pid);
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
        let mut record = fixture.observation();
        record.directory = ProcessField::Unavailable;
        let system = ProcessObservations::new([record]);
        let process = system.process(pid).expect("live process");
        assert_eq!(process.cwd(), WorkingDirectoryObservation::Unavailable);
        let row = row(
            process,
            process.cmd(),
            InvocationId::for_test(pid.as_u32()),
            Measurement::Reading(3),
            ScannerHome::Known(Path::new("/writer")),
            InvocationResourceUse {
                cpu:      Measurement::Reading(42.0),
                memory:   Measurement::Reading(1_024),
                compiler: CompilerObservation::None,
            },
            &capture.row_source(10),
        )
        .expect("merged process row");
        assert_eq!(row.pid, pid.as_u32());
        assert_eq!(row.path, "~/project");
        assert_eq!(row.resource_use.cpu, Measurement::Reading(42.0));
        assert_eq!(row.resource_use.memory, Measurement::Reading(1_024));
        assert_eq!(row.managed, Measurement::Reading(3));
        assert_eq!(row.resource_use.compiler, CompilerObservation::None);
    }

    #[test]
    fn registration_and_process_scans_keep_one_invocation_and_exclusions_apply_to_both() {
        let mut fixture = cargo_process("build");
        fixture.directory = PathBuf::from("/writer/project");
        let pid = Pid::from_u32(fixture.pid);
        let mut record = fixture.observation();
        record.parent = ProcessField::Observed(Pid::from_u32(10));
        record.cpu = Measurement::Reading(42.0);
        let system = ProcessObservations::new([record]);
        let process = system.process(pid).unwrap_or_else(|| {
            panic!("ready fixture pid {pid} is absent from the detailed process snapshot");
        });
        assert_eq!(
            command_text::subcommand(process.cmd()).as_deref(),
            Ok("build"),
            "fixture pid {pid}: argv={:?}, cwd={:?}",
            process.cmd(),
            process.cwd(),
        );
        let directory = PathBuf::from("/writer/project");
        assert_eq!(
            process.cwd(),
            WorkingDirectoryObservation::Observed(directory.as_path()),
            "fixture pid {pid}: argv={:?}, cwd={:?}",
            process.cmd(),
            process.cwd(),
        );
        let root = tempdir().expect("root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "    Blocking waiting for file lock on build directory",
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
        let mut sequence = CensusSequence::default();
        let home = ScannerHome::Known(Path::new("/writer"));
        let first =
            sequence.sample_with_capture(&ProcessObservations::default(), &capture, &[], home);
        let second = sequence.sample_with_capture(&system, &capture, &[], home);
        let third =
            sequence.sample_with_capture(&ProcessObservations::default(), &capture, &[], home);
        assert_single_source_transitions(
            &[first.clone(), second.clone(), third],
            &[10, pid.as_u32(), 10],
        );
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
        roster.observe_invocations(&roster.fixture_with_ended_absences(first, now));
        let ids = roster.tiled_ids(&[]);
        roster.observe_invocations(
            &roster.fixture_with_ended_absences(second, now + invocation_cpu_accounting::poll()),
        );
        assert_eq!(roster.tiled_ids(&[]), ids);
        assert_eq!(roster.groups().len(), 1);
        for mut census in [census_of(&[]), census_of(&[(pid.as_u32(), 10)])] {
            census.cargo.push(pid);
            assert_eq!(
                CensusSequence::assemble(&mut census, &system, &capture, &["build".into()]),
                [] as [CargoGroup; 0]
            );
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
            "    Blocking waiting for file lock on build directory",
        );
        let unknown = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_no_registration_rows(&unknown);
        assert!(matches!(unknown.row_source(10), DirectAssociation::None));
        write_versioned_capture(root.path(), 10, "second", "/other", "/writer", "test", "");
        let mut competing = verified_capture(root.path());
        let census = census_of(&[]);
        assert_no_registration_rows(&competing);
        census.associate_status(&mut competing, &[]);
        let associations = &competing.root_status[0].associations;
        assert_eq!(associations.len(), 1);
        assert!(
            matches!(&associations[0].selection, AssociationSelection::Ambiguous { candidates } if candidates.len() == 2)
        );
        let mut app = crate::app::App::new_for_test().expect("quiet settings app");
        app.root_status.clone_from(&competing.root_status);
        let ambiguous = account_settings_text(&app);
        assert!(
            ambiguous.contains(CAPTURE_ASSOCIATION_AMBIGUOUS),
            "{ambiguous}"
        );
        assert!(ambiguous.contains("10.first"), "{ambiguous}");
        assert!(ambiguous.contains("10.second"), "{ambiguous}");
        let mut row = directory_row();
        census.annotate_capture(&mut row, &competing, ScannerHome::Unavailable);
        assert_eq!(
            row.provenance,
            RowProvenance::Uncaptured(ProcessOwner::Unavailable)
        );
        assert_eq!(row.state, CaptureLookup::Unregistered);
        fs::remove_file(root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10.second"))
            .expect("remove competing publication");
        let mut recovered = verified_capture(root.path());
        let mut census = census_of(&[]);
        let groups = CensusSequence::assemble(
            &mut census,
            &ProcessObservations::default(),
            &recovered,
            &[],
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups[0].lead.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        let rendered = assert_registration_display(&groups, &recovered);
        assert_eq!(rendered.matches("blocked").count(), 1, "{rendered}");
        census.associate_status(&mut recovered, &groups);
        assert!(
            matches!(&recovered.root_status[0].associations[0].selection,
            AssociationSelection::Selected { proof: SelectedProof::Confirmed, unused, .. } if unused.is_empty())
        );
        app.root_status.clone_from(&recovered.root_status);
        let recovered_settings = account_settings_text(&app);
        assert!(
            !recovered_settings.contains('\n'),
            "a confirmed selection standing alone adds no line: {recovered_settings}"
        );
        assert_ne!(recovered_settings, ambiguous);
        let legacy_root = capture_root(&[(10, "")]);
        let legacy = verified_capture(legacy_root.path());
        assert_no_registration_rows(&legacy);
    }

    #[test]
    fn foreign_owned_duplicate_cannot_add_or_relabel_a_process_row() {
        let argv = [OsString::from("cargo"), "build".into()];
        let mut process = ProcessObservation::cargo(11, &argv);
        process.parent = ProcessField::Observed(Pid::from_u32(10));
        process.directory = ProcessField::Observed(Path::new("/writer/project"));
        assert_foreign_owned_duplicate(&ProcessObservations::new([process]), "100", &[11]);
    }

    #[test]
    fn foreign_owned_duplicate_cannot_add_or_relabel_a_registration_row() {
        assert_foreign_owned_duplicate(&ProcessObservations::default(), "100", &[10]);
    }

    #[test]
    fn foreign_owned_duplicate_cannot_confirm_an_unknown_selected_registration() {
        assert_foreign_owned_duplicate(&ProcessObservations::default(), "", &[]);
    }

    fn assert_foreign_owned_duplicate(
        records: &ProcessObservations<'_>,
        birth: &str,
        expected_pids: &[u32],
    ) {
        let parent = tempdir().expect("shared capture parent");
        let roots = CaptureRoots::from_parent(parent.path());
        let EffectiveUser::Known(uid) = crate::root_scan::effective_user() else {
            panic!("fixture owner");
        };
        let users = Users::new_with_refreshed_list();
        let foreign_uid = (0..=u32::MAX)
            .rev()
            .find(|candidate| {
                *candidate != uid && users.iter().all(|user| **user.id() != *candidate)
            })
            .expect("unassigned uid");
        let own = parent.path().join(uid.to_string());
        let foreign = parent.path().join(foreign_uid.to_string());
        write_versioned_capture(
            &own,
            10,
            "generation",
            "/writer/project",
            "/writer",
            "build",
            "Blocking waiting for file lock on build directory\n",
        );
        write_versioned_capture(
            &foreign,
            10,
            "generation",
            "/unused-directory",
            "/writer",
            "test",
            "PASS [0.010s] (7/13) unused\n",
        );
        let publication = own.join("state/pids/10.generation");
        if birth.is_empty() {
            let bytes = String::from_utf8(directory_record_bytes("/writer"))
                .expect("fixture UTF-8")
                .replace("\0boot\x00100\0", "\0boot\0\0");
            fs::write(&publication, bytes).expect("unverifiable selected proof");
        }
        let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone()
        else {
            panic!("fixture birth");
        };
        let mut capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Present(stamp.clone()))
        });
        assert_eq!(capture.root_status.len(), 2);
        let rejected = &capture.root_status[1];
        assert_eq!(rejected.owner, RootOwner::Uid(uid));
        assert_eq!(rejected.root.uid, foreign_uid);
        assert!(matches!(
            rejected.state,
            crate::progress::capture_roots::RootReadStatus::ForeignOwned { .. }
        ));
        assert_eq!(rejected.confirmed, 0);
        assert_eq!(rejected.associations, [] as [CaptureAssociation; 0]);
        let mut census = Census::take(records.records.values());
        let groups = CensusSequence::assemble(&mut census, records, &capture, &[]);
        assert_eq!(
            groups
                .iter()
                .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
                .map(|row| row.pid)
                .collect::<Vec<_>>(),
            expected_pids
        );
        if birth.is_empty() {
            assert_eq!(groups, [] as [CargoGroup; 0]);
            assert_eq!(capture.confirmed(), []);
        } else {
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
            let row = &groups[0].lead;
            assert_eq!(row.command, CommandText::of("cargo", &["build"]));
            assert_eq!(row.path, "~/project");
            assert_eq!(
                row.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
            let RowProvenance::Direct(context) = &row.provenance else {
                panic!("selected row keeps its accepted-root provenance");
            };
            assert_eq!(context.root, CaptureRootIndex(0));
            assert_eq!(context.account.uid, uid);
            assert_eq!(context.account.name, capture.root_status[0].account);
            if records.records.is_empty() {
                assert_registration_display(&groups, &capture);
            } else {
                assert_single_captured_group_display(&groups, &capture);
            }
        }
        census.associate_status(&mut capture, &groups);
        assert_eq!(capture.root_status[0].associations.len(), 1);
        assert!(matches!(&capture.root_status[0].associations[0].selection,
            AssociationSelection::Selected { key, unused, .. }
                if key.pid == 10 && key.root == CaptureRootIndex(0) && unused.is_empty()));
        assert!(publication.exists());
        assert!(own.join("run-generation-10.log").exists());
        assert!(foreign.join("state/pids/10.generation").exists());
        assert_ignored_duplicate_settings(&capture, &foreign, uid, foreign_uid);
    }

    fn assert_ignored_duplicate_settings(
        capture: &Capture,
        foreign: &Path,
        uid: u32,
        foreign_uid: u32,
    ) {
        let mut app = crate::app::App::new_for_test().expect("settings app");
        app.root_status.clone_from(&capture.root_status);
        let settings = crate::settings::rows(&app)
            .rows()
            .iter()
            .map(|row| row.value.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let owner = match &capture.root_status[0].account {
            AccountName::Resolved(name) => name.clone(),
            AccountName::Unavailable => uid.to_string(),
        };
        let ignored = format!(
            "{}: owned by {owner}, not by {foreign_uid} — ignored",
            foreign.canonicalize().expect("foreign path").display()
        );
        assert!(settings.contains(&ignored), "{settings}");
        assert!(!settings.contains("unused directory"), "{settings}");
        assert!(!settings.contains("/unused-directory"), "{settings}");
        // The own root's publication is the selection, and the ignored
        // duplicate is not listed as a proof that went unused.
        assert!(
            capture.root_status[0]
                .associations
                .iter()
                .any(|association| matches!(
                    &association.selection,
                    AssociationSelection::Selected { key, unused, .. }
                        if key.pid == 10 && unused.is_empty()
                )),
            "{:?}",
            capture.root_status[0].associations
        );
        assert!(!settings.contains("went unused"), "{settings}");
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
        let groups =
            CensusSequence::assemble(&mut census, &ProcessObservations::default(), &capture, &[]);
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
        census.select_rows(&ProcessObservations::default(), &capture, &[]);
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
        assert_eq!(
            rows[0].provenance,
            RowProvenance::Uncaptured(ProcessOwner::Unavailable)
        );
        assert_eq!(rows[0].command, CommandText::of("cargo", &["build"]));
        let groups = census.assemble_groups(
            &ProcessObservations::default(),
            ScannerHome::Unavailable,
            rows,
        );
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
        let groups = CensusSequence::assemble(
            &mut census_of(&[]),
            &ProcessObservations::default(),
            &capture,
            &[],
        );
        assert_eq!(groups.len(), 1);
        assert!(matches!(
            groups[0].lead.state,
            CaptureLookup::Registered(CaptureRead::Unreadable(_))
        ));
        assert_registration_display(&groups, &capture);
        assert_eq!(capture.root_status[0].confirmed, 0);
    }

    fn assert_no_registration_rows(capture: &Capture) {
        assert_eq!(
            CensusSequence::assemble(
                &mut census_of(&[]),
                &ProcessObservations::default(),
                capture,
                &[],
            ),
            [] as [CargoGroup; 0]
        );
    }

    fn account_settings_text(app: &App) -> String {
        crate::settings::rows(app)
            .rows()
            .iter()
            .find(|row| row.label == crate::settings::account_label(&app.root_status[0]))
            .expect("scanned account reaches Settings")
            .value
            .clone()
    }

    /// A scanned registration renders one qualified row with unavailable process measurements.
    fn assert_registration_display(groups: &[CargoGroup], capture: &Capture) -> String {
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
        let row = &groups[0].lead;
        assert_eq!(row.pid, 10);
        assert_eq!(row.command, CommandText::of("cargo", &["build"]));
        assert_eq!(row.path, "~/project");
        assert_eq!(
            row.resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            row.resource_use.memory,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            row.subtree_resource_use.memory,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(row.resource_use.compiler, CompilerObservation::Unknown);
        assert_eq!(
            row.managed,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        let lines = assert_single_captured_group_display(groups, capture);
        let text = lines.join("\n");
        let header = lines
            .iter()
            .find(|line| line.contains(TABLE_HEADERS[CPU_COLUMN]))
            .expect("table header");
        let rendered = lines
            .iter()
            .find(|line| line.contains("cargo build"))
            .expect("displayed registration row");
        assert_eq!(
            rendered
                .split_whitespace()
                .filter(|word| *word == "10")
                .count(),
            1,
            "{text}"
        );
        for column in [CPU_COLUMN, MEMORY_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
            assert_eq!(
                crate::census::measurement_cell(header, rendered, TABLE_HEADERS[column], &text),
                "",
                "{text}"
            );
        }
        text
    }

    fn assert_single_captured_group_display(
        groups: &[CargoGroup],
        capture: &Capture,
    ) -> Vec<String> {
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
        let lines = render_group_lines(&groups[0]);
        let text = lines.join("\n");
        let account = match &capture.root_status[0].account {
            AccountName::Resolved(name) => name.clone(),
            AccountName::Unavailable => capture.root_status[0].root.uid.to_string(),
        };
        assert_eq!(
            text.matches(&format!("[{account}] ~/project")).count(),
            1,
            "{text}"
        );
        assert_eq!(text.matches("~/project").count(), 1, "{text}");
        assert_eq!(text.matches("cargo build").count(), 1, "{text}");
        lines
    }

    fn render_group_lines(group: &CargoGroup) -> Vec<String> {
        let mut roster = crate::roster::Roster::new();
        roster.observe_invocations(
            &roster.fixture_with_ended_absences(vec![group.clone()], Instant::now()),
        );
        assert_eq!(roster.groups().len(), 1);
        assert_eq!(roster.groups()[0].rows().count(), 1 + group.rest.len());
        assert_eq!(roster.tiled_ids(&[]), [group.id()]);
        let area = ratatui::layout::Rect::new(0, 0, 160, 24);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        crate::render::draw_cell_for_test(
            &mut buffer,
            &roster,
            &TileContent::Group(group.id()),
            area,
            4,
        );
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buffer[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn enclosing_descendant_does_not_suppress_fallback_and_both_sources_assemble_one_tree() {
        let child = cargo_process("test");
        let child_pid = Pid::from_u32(child.pid);
        let parent = cargo_process("build");
        let parent_pid = Pid::from_u32(parent.pid);
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
        let child_only = ProcessObservations::new([child.observation()]);
        let both = ProcessObservations::new([parent.observation(), child.observation()]);
        let mut census = census_of(&[
            (parent_pid.as_u32(), 10),
            (child_pid.as_u32(), parent_pid.as_u32()),
        ]);
        census.cargo = vec![child_pid];
        let first = CensusSequence::assemble(&mut census, &child_only, &capture, &[]);
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
        let second = CensusSequence::assemble(&mut present, &both, &capture, &[]);
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
        let partial = CensusSequence::assemble(&mut missing_parent, &child_only, &capture, &[]);
        assert_eq!(partial.len(), 1);
        assert_eq!(partial[0].id(), first[0].id());
        assert_eq!(partial[0].rest.len(), 1);
        assert_eq!(
            partial[0].rest[0].invocation_id,
            first[0].rest[0].invocation_id
        );
        assert_family_source_transitions(&[first.clone(), second, partial, first]);
    }

    /// Completed samples preserve one row and one heading through both source switches.
    fn assert_single_source_transitions(scans: &[Vec<CargoGroup>], pids: &[u32]) {
        let mut roster = crate::roster::Roster::new();
        let identity = scans[0][0].id();
        for (index, (scan, &pid)) in scans.iter().zip(pids).enumerate() {
            assert_eq!(scan.len(), 1);
            assert_eq!(scan[0].rest, [] as [CargoProcess; 0]);
            assert_eq!(scan[0].id(), identity);
            assert_eq!(scan[0].lead.pid, pid);
            assert_eq!(
                scan[0].lead.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
            if index == 1 {
                assert_eq!(scan[0].lead.resource_use.cpu, Measurement::Reading(42.0));
                assert_eq!(scan[0].lead.managed, Measurement::Reading(0));
                assert_eq!(
                    scan[0].lead.resource_use.compiler,
                    CompilerObservation::None
                );
            } else {
                assert_eq!(
                    scan[0].lead.resource_use.cpu,
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                );
                assert_eq!(
                    scan[0].lead.managed,
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                );
                assert_eq!(
                    scan[0].lead.resource_use.compiler,
                    CompilerObservation::Unknown
                );
            }
            roster.observe_invocations(
                &roster.fixture_with_ended_absences(scan.clone(), Instant::now()),
            );
            assert_eq!(roster.groups().len(), 1);
            assert_eq!(roster.groups()[0].rows().count(), 1);
            let area = ratatui::layout::Rect::new(0, 0, 160, 12);
            let mut buffer = ratatui::buffer::Buffer::empty(area);
            crate::render::draw_cell_for_test(
                &mut buffer,
                &roster,
                &TileContent::Group(identity.clone()),
                area,
                4,
            );
            let text: String = buffer
                .content()
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect();
            assert_eq!(text.matches("~/project").count(), 1, "{text}");
            assert_eq!(text.matches("cargo build").count(), 1, "{text}");
            assert_eq!(text.matches("blocked").count(), 1, "{text}");
            assert_eq!(
                text.split_whitespace()
                    .filter(|word| *word == pid.to_string())
                    .count(),
                1,
                "{text}"
            );
        }
    }

    /// Source transitions retain family assignments and the focused tile together.
    fn assert_family_source_transitions(scans: &[Vec<CargoGroup>]) {
        let mut roster = crate::roster::Roster::new();
        let mut grid = crate::tiles::TileGrid::new();
        let growth = TileGrowth {
            initial_rows:  1,
            fill:          tui_pane::TileFill::default(),
            widen_summary: false,
        };
        grid.set_layout(ratatui::layout::Rect::new(0, 0, 120, 40), growth);
        let mut expected_family = FamilyHead::NoChildren;
        for (index, scan) in scans.iter().enumerate() {
            roster.observe_invocations(
                &roster.fixture_with_ended_absences(scan.clone(), Instant::now()),
            );
            let visible = roster.visible(&[]);
            assert_eq!(visible.groups().len(), 1);
            let tracked = &visible.groups()[0];
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
            let area = ratatui::layout::Rect::new(0, 0, 120, 40);
            crate::render::draw_grid_for_test(&mut grid, &roster, area, growth);
            grid.settle_for_test();
            crate::render::draw_grid_for_test(&mut grid, &roster, area, growth);
            if index == 0 {
                grid.focus_cell(TABLE_CELL + 1);
            }
            assert!(
                grid.placements(ratatui::layout::Rect::new(0, 0, 120, 40), growth)
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
        census.select_rows(&ProcessObservations::default(), &capture, &[]);
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
        let pid = Pid::from_u32(fixture.pid);
        let system = ProcessObservations::new([fixture.observation()]);
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
        assert_eq!(census.cargo, [] as [Pid; 0]);
        census.include_registered_processes(&system, &capture);
        assert_eq!(census.cargo, [pid]);
        let groups = CensusSequence::assemble(&mut census, &system, &capture, &[]);
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
        census.select_rows(&ProcessObservations::default(), &capture, &[]);
        let mut rows = vec![directory_row()];
        census.add_registration_rows(
            &mut rows,
            &capture,
            ScannerHome::Unavailable,
            SystemTime::now(),
        );
        let groups = census.assemble_groups(
            &ProcessObservations::default(),
            ScannerHome::Unavailable,
            rows,
        );
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
        let pid = Pid::from_u32(fixture.pid);
        let system = ProcessObservations::new([fixture.observation()]);
        // An unavailable CPU bucket does not remove an otherwise displayable group.
        let groups = CensusSequence::default().sample(&system);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].lead.pid, pid.as_u32());
        assert_eq!(
            groups[0].lead.resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            groups[0].lead.subtree_resource_use.cpu,
            groups[0].lead.resource_use.cpu
        );
        let unobserved = CensusSequence::default().sample(&system);
        assert_eq!(
            unobserved[0].lead.resource_use.cpu,
            groups[0].lead.resource_use.cpu
        );
        let mut census = census_of(&[]);
        let children = HashMap::new();
        let capture = Capture::default();
        assert_eq!(
            census.group(
                &ProcessObservations::default(),
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
        census
            .eligibility
            .insert(pid, Err(RowAbsence::PolicyExcluded));
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
    fn discovery_lifetime_and_parent_survive_fresh_detail_overlay() {
        let pid = Pid::from_u32(std::process::id());
        let discovery_system = process_details(&[pid]);
        let process = discovery_system
            .process(pid)
            .expect("current process discovery");
        let discovery = [(process, CpuBaseline::from(process))];
        let mut observations = ProcessObservations::from_discovery(&discovery, &HashMap::new());
        assert!(
            observations.process(pid).is_none(),
            "discovery alone is not a detail observation"
        );
        let original_parent = ProcessField::Observed(Pid::from_u32(42));
        let record = observations.records.get_mut(&pid).expect("discovered PID");
        record.parent = original_parent;
        record.argv = ProcessField::Unavailable;
        record.directory = ProcessField::Unavailable;
        record.executable = ProcessField::Unavailable;
        let accumulated = record.accumulated;
        let cpu = record.cpu;
        assert!(std::ptr::eq(
            record.lifetime.as_ref(),
            &raw const discovery[0].1.lifetime
        ));
        let census = Census::take(observations.records.values());
        assert_eq!(census.parents.get(&pid), Some(&Pid::from_u32(42)));
        assert_eq!(census.lifetimes.get(&pid), Some(&discovery[0].1.lifetime));

        let details = process_details(&[pid]);
        let process = details.process(pid).expect("fresh process details");
        observations.overlay_details(&details);
        assert_eq!(observations.records.len(), 1);
        let record = observations
            .process(pid)
            .expect("detail overlay retains the PID");
        assert_eq!(record.parent, original_parent);
        assert!(std::ptr::eq(
            record.lifetime.as_ref(),
            &raw const discovery[0].1.lifetime
        ));
        assert_eq!(record.accumulated, accumulated);
        assert_eq!(record.cpu, cpu);
        assert!(std::ptr::eq(record.cmd(), process.cmd()));
        assert!(std::ptr::eq(record.environ(), process.environ()));
        assert_eq!(record.cwd(), process.cwd().into());
        assert_eq!(record.exe(), process.exe().into());
    }

    #[test]
    fn wrapper_classification_requires_fresh_nonempty_arguments() {
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
        let forwarding = [OsString::from("cargo"), OsString::from("build")];
        let mismatched = [OsString::from("application")];
        let records = ProcessObservations::new(
            [
                (100, forwarding.as_slice(), DetailPresence::Absent),
                (101, mismatched.as_slice(), DetailPresence::Absent),
                (102, forwarding.as_slice(), DetailPresence::Observed),
                (103, mismatched.as_slice(), DetailPresence::Observed),
                (104, &[], DetailPresence::Observed),
            ]
            .map(|(pid, argv, details)| {
                let mut record = ProcessObservation::cargo(pid, argv);
                record.parent = ProcessField::Observed(Pid::from_u32(10));
                record.details = details;
                record
            }),
        );
        let mut census = Census::take(records.records.values());
        census.identify_capture_wrappers(&records, &capture);
        assert_eq!(census.capture_wrappers, HashSet::from([Pid::from_u32(102)]));
        assert_eq!(
            census.capture_mismatches,
            HashSet::from([Pid::from_u32(103)])
        );
    }

    #[test]
    fn late_details_borrow_metadata_without_inventing_discovery_evidence() {
        let pid = Pid::from_u32(std::process::id());
        let details = process_details(&[pid]);
        let process = details.process(pid).expect("current process details");
        let census = Census::take([]);
        let mut observations = ProcessObservations::from_discovery(&[], &HashMap::new());
        observations.overlay_details(&details);
        let record = observations.process(pid).expect("late detail PID retained");
        assert!(std::ptr::eq(record.cmd(), process.cmd()));
        assert!(std::ptr::eq(record.environ(), process.environ()));
        assert!(std::ptr::eq(record.name(), process.name()));
        assert_eq!(record.cwd(), process.cwd().into());
        assert_eq!(record.exe(), process.exe().into());
        assert_eq!(record.lifetime.as_ref(), &LifetimeEvidence::Unavailable);
        assert_eq!(
            record.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert!(census.lifetimes.is_empty());
    }

    #[test]
    fn observation_targets_prefer_arguments_and_resolve_environment_against_cwd() {
        let root = tempdir().expect("target root");
        let explicit = root.path().join("explicit");
        let environment = root.path().join("environment");
        fs::create_dir(&explicit).expect("explicit target");
        fs::create_dir(&environment).expect("environment target");
        let argv = [
            OsString::from("cargo"),
            "build".into(),
            CARGO_TARGET_DIR_FLAG.into(),
            "explicit".into(),
        ];
        let variables = [OsString::from("CARGO_TARGET_DIR=environment")];
        let mut record = ProcessObservation::cargo(100, &argv);
        record.directory = ProcessField::Observed(root.path());
        record.environment = &variables;
        assert_eq!(
            invocation_cpu_accounting::cargo_target_directory(&record).expect("argument target"),
            explicit.canonicalize().expect("explicit path")
        );
        record.argv = ProcessField::Observed(&argv[..2]);
        assert_eq!(
            invocation_cpu_accounting::cargo_target_directory(&record).expect("environment target"),
            environment.canonicalize().expect("environment path")
        );
        record.directory = ProcessField::Unavailable;
        assert!(invocation_cpu_accounting::cargo_target_directory(&record).is_err());
        record.argv = ProcessField::Unavailable;
        assert!(invocation_cpu_accounting::cargo_target_directory(&record).is_err());
    }

    #[test]
    fn metadata_exec_child() {
        let Some(directory) = std::env::var_os("CARGO_TILE_METADATA_EXEC_DIRECTORY") else {
            return;
        };
        println!("metadata ready");
        std::io::stdout().flush().expect("readiness");
        let mut release = String::new();
        std::io::stdin()
            .read_line(&mut release)
            .expect("exec release");
        let error = std::process::Command::new("sleep")
            .arg("60")
            .current_dir(directory)
            .exec();
        panic!("exec sleep: {error}");
    }

    #[test]
    fn fresh_details_replace_cached_command_directory_and_executable() {
        let root = tempdir().expect("metadata directories");
        let before = root.path().join("before");
        let after = root.path().join("after");
        fs::create_dir(&before).expect("initial directory");
        fs::create_dir(&after).expect("replacement directory");
        let before = before.canonicalize().expect("physical initial directory");
        let after = after
            .canonicalize()
            .expect("physical replacement directory");
        let mut fixture = MetadataProcess {
            child: std::process::Command::new(std::env::current_exe().expect("test executable"))
                .args([
                    "--exact",
                    "census::scan::tests::metadata_exec_child",
                    "--nocapture",
                ])
                .env("CARGO_TILE_METADATA_EXEC_DIRECTORY", &after)
                .current_dir(&before)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .expect("metadata fixture"),
        };
        let mut reader = BufReader::new(fixture.child.stdout.take().expect("ready pipe"));
        loop {
            let mut ready = String::new();
            assert_ne!(reader.read_line(&mut ready).expect("fixture handshake"), 0);
            if ready.trim() == "metadata ready" {
                break;
            }
        }
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
            thread::sleep(invocation_cpu_accounting::poll());
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

    #[test]
    fn directory_aliases_preserve_wrapper_and_registration_associations() {
        let root = tempdir().expect("capture root");
        let real = root.path().join("real");
        fs::create_dir(&real).expect("working directory");
        let alias = root.path().join("alias");
        symlink(&real, &alias).expect("working directory alias");
        let argv = [OsString::from("cargo"), OsString::from("build")];
        assert!(observed_shim_match(
            &argv,
            WorkingDirectoryObservation::Observed(&real),
            &argv,
            WorkingDirectoryObservation::Observed(&alias),
        ));
        assert_eq!(
            DirectoryComparison::between(&real, root.path()),
            DirectoryComparison::Different
        );
        assert_eq!(
            DirectoryComparison::between(&real, &root.path().join("missing")),
            DirectoryComparison::Unavailable
        );
        assert_eq!(
            DirectoryComparison::between(Path::new("~/real"), &real),
            DirectoryComparison::Unavailable
        );

        write_versioned_capture(
            root.path(),
            10,
            "generation",
            real.to_str().expect("fixture path"),
            root.path().to_str().expect("fixture home"),
            "build",
            "",
        );
        let capture = verified_capture(root.path());
        let home = ScannerHome::Known(root.path());
        let (identity, path, _) = row_fields(
            WorkingDirectoryObservation::Observed(&alias),
            &argv,
            &capture.row_source(10),
            home,
        )
        .expect("alias agrees with verified registration");
        assert_eq!(identity, WorkingDirectoryIdentity::Absolute(real.clone()));
        assert_eq!(path, "~/real");

        let mut census = census_of(&[]);
        census.identify_captures(&capture);
        let mut row = directory_row();
        row.directory_identity = WorkingDirectoryIdentity::Absolute(alias);
        census.annotate_capture(&mut row, &capture, home);
        assert_eq!(
            row.directory_identity,
            WorkingDirectoryIdentity::Absolute(real)
        );
        assert_eq!(row.path, "~/real");
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
    fn an_unconfirmed_driver_capture_belongs_only_to_its_enclosing_invocation() {
        for (program, arguments) in [
            ("cargo-mend", vec!["cargo-mend", "mend"]),
            ("cargo-nextest", vec!["cargo-nextest", "nextest", "run"]),
        ] {
            let root = capture_root(&[(10, "Building [=>] 998/1000: checking\r")]);
            let capture = Capture::take_from(root.path(), |pid| {
                KernelObservation::for_test(pid, Observation::Unknown)
            });
            let shell = [OsString::from("bash")];
            let lead = arguments
                .into_iter()
                .map(OsString::from)
                .collect::<Vec<_>>();
            let child = ["cargo", "check"].map(OsString::from);
            let records = ProcessObservations::new(
                [
                    (9, 1, "bash", shell.as_slice()),
                    (10, 9, program, lead.as_slice()),
                    (11, 10, "cargo", child.as_slice()),
                ]
                .map(|(pid, parent, name, argv)| {
                    let mut process = ProcessObservation::cargo(pid, argv);
                    process.name = ProcessField::Observed(OsStr::new(name));
                    process.parent = ProcessField::Observed(Pid::from_u32(parent));
                    process.uid = ProcessField::Observed(2000);
                    process.directory = ProcessField::Observed(Path::new("/runner/project"));
                    process
                }),
            );
            let mut census = Census::take(records.processes().values());
            census.prepare(&records, &capture, &[]);
            let NearestRegistration::Registered(lead_key) =
                census.captured_run(&capture, Pid::from_u32(10))
            else {
                panic!("driver capture");
            };
            assert_eq!(lead_key.pid, 10);
            assert_eq!(
                census.captured_run(&capture, Pid::from_u32(11)),
                NearestRegistration::Registered(lead_key)
            );
            assert_eq!(
                census.direct_capture(&capture, Pid::from_u32(10)),
                DirectAssociation::None
            );
            assert_eq!(
                census.direct_capture(&capture, Pid::from_u32(11)),
                DirectAssociation::None
            );
            let groups = CensusSequence::default().sample_capture(&records, &capture);
            assert_eq!(groups.len(), 1);
            assert_eq!(
                groups[0].lead.capture_membership,
                CaptureMembership::Outside
            );
            assert_eq!(
                groups[0].rest[0].capture_membership,
                CaptureMembership::EnclosingUnconfirmed
            );
        }
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
        let mut smoothing = InvocationCpuAccounting::default();
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
            now + invocation_cpu_accounting::poll(),
        );
        assert_eq!(
            reported[&identities[1]],
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert!(!smoothing.settled.contains_key(&identities[0]));
        assert!(!smoothing.reported.contains_key(&identities[0]));

        let pid = Pid::from_u32(10);
        let compiler = invocation_cpu_accounting::cpu_process_identity(30);
        let mut census = census_of(&[]);
        census.cargo.push(pid);
        census.lifetimes.insert(
            pid,
            LifetimeEvidence::Available(ProcessLifetime::for_test(10)),
        );
        census.lifetimes.insert(
            Pid::from_u32(30),
            LifetimeEvidence::Available(ProcessLifetime::for_test(30)),
        );
        census.accumulated.insert(pid, Duration::ZERO);
        census.cpu.insert(pid, Measurement::Reading(0.0));
        census.identities.insert(pid, identities[0].clone());
        let first =
            census.attribute_cpu_with(&ProcessObservations::default(), &mut smoothing, now, |_| {
                Measurement::Reading(Duration::from_millis(100))
            });
        assert_eq!(
            first[&pid],
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        smoothing
            .cache_owners
            .insert(compiler.clone(), identities[0].clone());
        smoothing
            .invocations
            .get_mut(&identities[0])
            .expect("first generation counter")
            .detached
            .entry(compiler.clone())
            .or_default()
            .update(&invocation_cpu_accounting::cpu_work(&[(30, 400)]).tree);
        census.identities.insert(pid, identities[1].clone());
        let replaced = census.attribute_cpu_with(
            &ProcessObservations::default(),
            &mut smoothing,
            now + Duration::from_secs(1),
            |_| Measurement::Reading(Duration::from_millis(200)),
        );
        assert_eq!(
            replaced[&pid],
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert!(!smoothing.invocations.contains_key(&identities[0]));
        assert_eq!(
            smoothing.invocations[&identities[1]].accumulated,
            Duration::from_millis(200)
        );
        assert!(smoothing.invocations[&identities[1]].detached.is_empty());
        assert_eq!(smoothing.cache_owners[&compiler], identities[0]);
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
        identities.finish(&process_identity::ProvenProcessEnd::for_test(pid));
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
        census.select_rows(
            &ProcessObservations::default(),
            &capture,
            &["run".to_owned()],
        );
        for pid in [12, 13] {
            assert_eq!(
                census.eligibility[&Pid::from_u32(pid)],
                Err(RowAbsence::ArgvUnavailable)
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
    fn nested_selected_rows_keep_commands_pids_directories_and_enclosing_progress() {
        assert_nested_selected_rows("build", &[]);
    }

    #[test]
    fn exec_nested_selected_rows_keep_commands_pids_directories_and_enclosing_progress() {
        assert_nested_selected_rows("run", &[]);
    }

    #[test]
    fn exec_excluded_nested_rows_keep_progress_and_writes_continue_after_sweep() {
        assert_nested_selected_rows("run", &["run".into()]);
    }

    fn assert_nested_selected_rows(command: &str, excluded: &[String]) {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "enclosing",
            "/writer/project",
            "/writer",
            command,
            "Blocking waiting for file lock on build directory\n",
        );
        write_versioned_capture(
            root.path(),
            20,
            "ended",
            "/writer/project",
            "/writer",
            "build",
            "ended\n",
        );
        let enclosing = root.path().join("state/pids/10.enclosing");
        let log = root.path().join("run-enclosing-10.log");
        let mut writer = fs::OpenOptions::new()
            .append(true)
            .open(&log)
            .expect("live writer descriptor");
        let IdentityEvidence::Available(stamp) = directory_record("/writer").identity().clone()
        else {
            panic!("fixture birth");
        };
        let capture = Capture::take_from(root.path(), |pid| {
            KernelObservation::for_test(
                pid,
                if pid == 20 {
                    Observation::Ended
                } else {
                    Observation::Present(stamp.clone())
                },
            )
        });
        assert!(!root.path().join("state/pids/20.ended").exists());
        assert!(!root.path().join("run-ended-20.log").exists());
        assert!(enclosing.exists() && log.exists());
        let outer_argv = if command == "build" {
            vec!["cargo".into(), command.into()]
        } else {
            vec!["/writer/application".into()]
        };
        let check_argv = ["cargo".into(), "check".into(), "probe-nested-check".into()];
        let test_argv = ["cargo".into(), "test".into(), "probe-nested-test".into()];
        let ancestor_argv = [
            "cargo".into(),
            "test".into(),
            "probe-common-ancestor".into(),
        ];
        let mut ancestor = ProcessObservation::cargo(9, &ancestor_argv);
        ancestor.directory = ProcessField::Observed(Path::new("/writer/ancestor-directory"));
        let shim_argv = ["cargo".into(), command.into()];
        let mut shim = ProcessObservation::cargo(10, &shim_argv);
        shim.parent = ProcessField::Observed(Pid::from_u32(9));
        shim.directory = ProcessField::Observed(Path::new("/writer/project"));
        let mut outer = ProcessObservation::cargo(11, &outer_argv);
        outer.parent = ProcessField::Observed(Pid::from_u32(10));
        outer.directory = ProcessField::Observed(Path::new("/writer/project"));
        if command == "run" {
            outer.name = ProcessField::Observed(OsStr::new("application"));
        }
        let nested = [(12, &check_argv), (13, &test_argv)].map(|(pid, argv)| {
            let mut record = ProcessObservation::cargo(pid, argv);
            record.parent = ProcessField::Observed(Pid::from_u32(11));
            record.directory = ProcessField::Observed(Path::new("/writer/nested-directory"));
            record
        });
        let records =
            ProcessObservations::new([ancestor, shim, outer, nested[0].clone(), nested[1].clone()]);
        let mut sequence = CensusSequence::default();
        let groups = sequence.sample_with_capture(
            &records,
            &capture,
            excluded,
            ScannerHome::Known(Path::new("/writer")),
        );
        assert_nested_rows(&groups, command, excluded);
        writeln!(writer, "writer remains captured after reader scan")
            .expect("continued capture write");
        assert!(
            fs::read_to_string(&log)
                .expect("retained log")
                .contains("writer remains captured after reader scan")
        );
        assert!(enclosing.exists());
        assert_eq!(verified_capture(root.path()).confirmed().len(), 1);
    }

    fn assert_nested_rows(groups: &[CargoGroup], command: &str, excluded: &[String]) {
        assert_eq!(groups.len(), 1, "nested invocations share one group");
        let group = &groups[0];
        assert_eq!(group.lead.pid, 9);
        assert_eq!(
            group.lead.command,
            CommandText::of("cargo", &["test", "probe-common-ancestor"])
        );
        let rows: Vec<_> = group.rest.iter().collect();
        assert_eq!(rows.len(), if excluded.is_empty() { 3 } else { 2 });
        for (pid, command, marker) in [
            (12, "check", "probe-nested-check"),
            (13, "test", "probe-nested-test"),
        ] {
            let matching: Vec<_> = rows.iter().filter(|row| row.pid == pid).collect();
            assert_eq!(matching.len(), 1);
            let row = matching[0];
            assert_eq!(row.command, CommandText::of("cargo", &[command, marker]));
            assert_eq!(row.path, "~/nested-directory");
            assert_eq!(
                row.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
            assert!(matches!(
                row.capture_membership,
                CaptureMembership::Enclosing(_)
            ));
        }
        assert_ne!(
            rows.iter()
                .find(|row| row.pid == 12)
                .expect("check")
                .invocation_id,
            rows.iter()
                .find(|row| row.pid == 13)
                .expect("test")
                .invocation_id
        );
        if excluded.is_empty() {
            assert_eq!(
                rows.iter()
                    .filter(|row| row.command == CommandText::of("cargo", &[command]))
                    .count(),
                1
            );
        } else {
            assert!(
                rows.iter()
                    .all(|row| row.command != CommandText::of("cargo", &[command]))
            );
        }
        let lines = render_group_lines(group);
        let text = lines.join("\n");
        for row in std::iter::once(&group.lead).chain(&group.rest) {
            let command = format!("{} {}", row.command.program, row.command.line());
            let matching: Vec<_> = lines
                .iter()
                .filter(|line| line.contains(&command))
                .collect();
            assert_eq!(matching.len(), 1, "{text}");
            assert_eq!(
                matching[0]
                    .split_whitespace()
                    .filter(|word| *word == row.pid.to_string())
                    .count(),
                1,
                "{text}"
            );
        }
        assert_eq!(text.matches("probe-common-ancestor").count(), 1, "{text}");
        assert_eq!(text.matches("probe-nested-check").count(), 1, "{text}");
        assert_eq!(text.matches("probe-nested-test").count(), 1, "{text}");
        assert_eq!(text.matches("blocked").count(), rows.len(), "{text}");
        if !excluded.is_empty() {
            assert!(!text.contains(&format!("cargo {command}")), "{text}");
        }
    }

    #[test]
    fn quiet_json_short_selects_one_direct_row_with_original_arguments() {
        assert_accepted_rewrite_rows(
            &[
                "check",
                "probe-json",
                "-q",
                "--message-format=json",
                "-q",
                "--",
                "--quiet",
                "-q",
            ],
            &[
                "check",
                "probe-json",
                "--message-format=json",
                "--",
                "--quiet",
                "-q",
            ],
        );
    }

    #[test]
    fn quiet_json_separate_selects_one_direct_row_with_original_arguments() {
        assert_accepted_rewrite_rows(
            &[
                "check",
                "probe-json",
                "-q",
                "--message-format",
                "json",
                "-q",
                "--",
                "--quiet",
                "-q",
            ],
            &[
                "check",
                "probe-json",
                "--message-format",
                "json",
                "--",
                "--quiet",
                "-q",
            ],
        );
    }

    fn assert_accepted_rewrite_rows(registered: &[&str], executed: &[&str]) {
        let (root, records_argv) = rewrite_capture(registered, executed);
        let mut record = ProcessObservation::cargo(11, &records_argv);
        record.parent = ProcessField::Observed(Pid::from_u32(10));
        record.directory = ProcessField::Observed(Path::new("/writer/project"));
        let records = ProcessObservations::new([record]);
        let mut capture = verified_capture(root.path());
        let mut census = Census::take(records.records.values());
        let groups = CensusSequence::assemble(&mut census, &records, &capture, &[]);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].rest, [] as [CargoProcess; 0]);
        let row = &groups[0].lead;
        assert_eq!(row.pid, 11);
        assert_eq!(row.command, CommandText::of("cargo", registered));
        assert_eq!(
            row.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        assert!(matches!(
            census.direct_capture(&capture, Pid::from_u32(11)),
            DirectAssociation::Direct(_)
        ));
        census.associate_status(&mut capture, &groups);
        let mut app = crate::app::App::new_for_test().expect("settings app");
        app.root_status.clone_from(&capture.root_status);
        assert!(
            capture.root_status[0]
                .associations
                .iter()
                .any(|association| {
                    association.pid == 11
                        && matches!(&association.selection,
                        AssociationSelection::Selected { key, proof: SelectedProof::Confirmed, .. }
                            if key.pid == 10)
                }),
            "{:?}",
            capture.root_status[0].associations
        );
        assert!(root.path().join("state/pids/10.generation").exists());
        assert!(root.path().join("run-generation-10.log").exists());
    }

    #[test]
    fn rejected_non_json_quiet_rewrites_keep_process_and_registration_rows() {
        for quiet in ["--quiet", "-q"] {
            assert_rejected_rewrite_rows(
                &["check", "probe-mismatch", quiet],
                &["check", "probe-mismatch"],
            );
        }
    }

    #[test]
    fn rejected_post_separator_quiet_rewrites_keep_process_and_registration_rows() {
        for quiet in ["--quiet", "-q"] {
            assert_rejected_rewrite_rows(
                &[
                    "check",
                    "probe-mismatch",
                    quiet,
                    "--message-format=json",
                    "--",
                    quiet,
                ],
                &["check", "probe-mismatch", "--message-format=json", "--"],
            );
        }
    }

    #[test]
    fn rejected_unrelated_rewrite_keeps_process_and_registration_rows() {
        assert_rejected_rewrite_rows(
            &[
                "check",
                "probe-mismatch",
                "--quiet",
                "--message-format=json",
                "--workspace",
            ],
            &[
                "check",
                "probe-mismatch",
                "--message-format=json",
                "--release",
            ],
        );
    }

    fn assert_rejected_rewrite_rows(registered: &[&str], executed: &[&str]) {
        let (root, records_argv) = rewrite_capture(registered, executed);
        let mut record = ProcessObservation::cargo(11, &records_argv);
        record.parent = ProcessField::Observed(Pid::from_u32(10));
        record.directory = ProcessField::Observed(Path::new("/writer/project"));
        let records = ProcessObservations::new([record]);
        let capture = verified_capture(root.path());
        let groups = CensusSequence::default().sample_capture(&records, &capture);
        let rows: Vec<_> = groups
            .iter()
            .flat_map(|group| std::iter::once(&group.lead).chain(&group.rest))
            .collect();
        assert_eq!(rows.len(), 2);
        for (pid, arguments) in [(10, registered), (11, executed)] {
            let matching: Vec<_> = rows.iter().filter(|row| row.pid == pid).collect();
            assert_eq!(matching.len(), 1);
            assert_eq!(matching[0].command, CommandText::of("cargo", arguments));
        }
        assert_ne!(rows[0].invocation_id, rows[1].invocation_id);
        assert!(root.path().join("state/pids/10.generation").exists());
        assert!(root.path().join("run-generation-10.log").exists());
    }

    fn rewrite_capture(registered: &[&str], executed: &[&str]) -> (TempDir, Vec<OsString>) {
        let root = tempdir().expect("capture root");
        write_versioned_capture(
            root.path(),
            10,
            "generation",
            "/writer/project",
            "/writer",
            "check",
            "Blocking waiting for file lock on build directory\n",
        );
        let bytes = directory_record_bytes("/writer");
        let fields = bytes.split(|byte| *byte == 0).take(7).collect::<Vec<_>>();
        let mut bytes = fields.join(&0);
        bytes.push(0);
        bytes.extend_from_slice(registered.len().to_string().as_bytes());
        bytes.push(0);
        for argument in registered {
            bytes.extend_from_slice(argument.as_bytes());
            bytes.push(0);
        }
        fs::write(root.path().join("state/pids/10.generation"), bytes)
            .expect("registered arguments");
        (
            root,
            std::iter::once("cargo")
                .chain(executed.iter().copied())
                .map(OsString::from)
                .collect(),
        )
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
        census.select_rows(
            &ProcessObservations::default(),
            &capture,
            &["build".to_owned()],
        );
        assert_eq!(census.cargo, [] as [Pid; 0]);
        assert_eq!(
            census.eligibility[&Pid::from_u32(11)],
            Err(RowAbsence::PolicyExcluded)
        );
        assert_eq!(
            census.eligibility[&Pid::from_u32(12)],
            Err(RowAbsence::ArgvUnavailable)
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

    /// A new command's first native metadata read cannot wait behind refresh traffic.
    #[test]
    fn new_command_metadata_precedes_the_existing_read_backlog() {
        let (requests, incoming) = mpsc::sync_channel(4);
        let (_, replies) = mpsc::channel();
        let mut details = DetailDiscovery::<Pid> {
            reads: HashMap::new(),
            pending: VecDeque::new(),
            changes: VecDeque::new(),
            refreshes: VecDeque::new(),
            requests,
            replies,
        };
        let mut pids: Vec<_> = (4100..4112).map(Pid::from_u32).collect();
        let mut identities: HashMap<_, _> = pids
            .iter()
            .map(|pid| (*pid, InvocationId::for_test(pid.as_u32())))
            .collect();
        details.read(&pids, &identities, |_| ProcessField::Unavailable);
        let occupied: Vec<_> = incoming.try_iter().collect();
        assert_eq!(occupied.len(), 4);
        let arrivals: Vec<_> = (4200..4203).map(Pid::from_u32).collect();
        pids.extend(&arrivals);
        identities.extend(
            arrivals
                .iter()
                .map(|pid| (*pid, InvocationId::for_test(pid.as_u32()))),
        );
        details.read(&pids, &identities, |_| ProcessField::Unavailable);
        let scheduled: Vec<_> = incoming.try_iter().map(|request| request.pid).collect();
        assert_eq!(&scheduled[..arrivals.len()], arrivals.as_slice());
        assert!(
            scheduled
                .iter()
                .all(|pid| occupied.iter().all(|request| request.pid != *pid)),
            "in-flight reads were queued twice"
        );
    }

    /// A fast completed read cannot monopolize refreshes while another lifetime waits.
    #[test]
    fn metadata_refreshes_visit_every_completed_lifetime() {
        let (requests, incoming) = mpsc::sync_channel(4);
        let (completed, replies) = mpsc::channel();
        let mut details = DetailDiscovery::<Pid> {
            reads: HashMap::new(),
            pending: VecDeque::new(),
            changes: VecDeque::new(),
            refreshes: VecDeque::new(),
            requests,
            replies,
        };
        let pids: Vec<_> = (4100..4108).map(Pid::from_u32).collect();
        let identities = pids
            .iter()
            .map(|pid| (*pid, InvocationId::for_test(pid.as_u32())))
            .collect();
        for _ in 0..2 {
            details.read(&pids, &identities, |_| ProcessField::Unavailable);
            for request in incoming.try_iter() {
                let pid = request.pid;
                assert!(completed.send((request, pid)).is_ok());
            }
        }
        let mut refreshed = HashSet::new();
        for _ in 0..2 {
            details.read(&pids, &identities, |_| ProcessField::Unavailable);
            for request in incoming.try_iter() {
                refreshed.insert(request.pid);
                let pid = request.pid;
                assert!(completed.send((request, pid)).is_ok());
            }
        }
        assert_eq!(
            refreshed.len(),
            pids.len(),
            "refresh traffic starves a lifetime"
        );
    }

    /// Changed native sources precede unchanged refreshes, after new-job first reads.
    #[test]
    fn a_changed_source_rereads_before_unchanged_lifetimes() {
        let (requests, incoming) = mpsc::sync_channel(4);
        let (completed, replies) = mpsc::channel();
        let mut details = DetailDiscovery::<Pid> {
            reads: HashMap::new(),
            pending: VecDeque::new(),
            changes: VecDeque::new(),
            refreshes: VecDeque::new(),
            requests,
            replies,
        };
        let mut pids: Vec<_> = (4100..4108).map(Pid::from_u32).collect();
        let mut identities: HashMap<_, _> = pids
            .iter()
            .map(|pid| (*pid, InvocationId::for_test(pid.as_u32())))
            .collect();
        for _ in 0..2 {
            details.read(&pids, &identities, |_| {
                ProcessField::Observed(OsString::from("cargo"))
            });
            for request in incoming.try_iter() {
                let pid = request.pid;
                completed
                    .send((request, pid))
                    .expect("completed first read");
            }
        }
        let changed = pids[7];
        let arrival = Pid::from_u32(4200);
        pids.push(arrival);
        identities.insert(arrival, InvocationId::for_test(arrival.as_u32()));
        details.read(&pids, &identities, |pid| {
            ProcessField::Observed(OsString::from(if pid == changed { "sh" } else { "cargo" }))
        });
        let scheduled: Vec<_> = incoming.try_iter().map(|request| request.pid).collect();
        assert_eq!(scheduled[0], arrival, "new jobs keep first-read priority");
        assert_eq!(
            scheduled[1], changed,
            "a source change waits behind unchanged refreshes"
        );

        // Another source change while its read runs must retain one in-flight read.
        let answers = details.read(&pids, &identities, |_| {
            ProcessField::Observed(OsString::from("cargo"))
        });
        assert!(answers.iter().any(|answer| **answer == changed));
        assert!(incoming.try_iter().all(|request| request.pid != changed));
        completed
            .send((
                PendingDetail {
                    pid:      changed,
                    identity: identities[&changed].clone(),
                },
                changed,
            ))
            .expect("completed old source read");
        details.read(&pids, &identities, |_| {
            ProcessField::Observed(OsString::from("cargo"))
        });
        assert_eq!(
            incoming.try_recv().expect("changed source reread").pid,
            changed
        );
    }

    /// Blocked readers occupy their workers without holding later reads or publication.
    #[test]
    fn a_blocked_metadata_read_does_not_hold_the_next_read()
    -> Result<(), Box<dyn std::error::Error>> {
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let reading = Arc::clone(&gate);
        let (answers, worker) = spawn_resolved_scan_loop(
            Duration::from_millis(40),
            || resolved_test_roots(&[]),
            move || {
                let mut details = DetailDiscovery::new(move |pid| {
                    if pid != Pid::from_u32(4101) {
                        let (lock, changed) = &*reading;
                        let mut released = lock.lock().expect("gate");
                        while !*released {
                            released = changed.wait(released).expect("release");
                        }
                        drop(released);
                    }
                    pid
                })?;
                Ok(move |_: &CaptureRoots, _: &ScanPublicationWindow| {
                    let pids: Vec<_> = (4100..4106).map(Pid::from_u32).collect();
                    let identities = pids
                        .iter()
                        .map(|pid| (*pid, InvocationId::for_test(pid.as_u32())))
                        .collect();
                    details
                        .read(&pids, &identities, |_| ProcessField::Unavailable)
                        .iter()
                        .map(|pid| **pid)
                        .collect::<Vec<_>>()
                })
            },
        )?;
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut saw_fast = false;
        let mut published = 0;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            let Ok(answer) = answers.recv_timeout(left) else {
                break;
            };
            published += 1;
            saw_fast |= answer.contains(&Pid::from_u32(4101));
            if saw_fast && published >= 3 {
                break;
            }
        }
        *gate.0.lock().expect("gate") = true;
        gate.1.notify_all();
        drop(answers);
        worker.join().expect("scanner shutdown");
        assert!(saw_fast, "one blocked read cannot serialize the fast read");
        assert!(
            published >= 3,
            "all four occupied workers cannot hold publication"
        );
        Ok(())
    }

    /// Bound failed worker handshakes without imposing a startup speed threshold.
    const WORKER_REPLY_TIMEOUT: Duration = Duration::from_secs(10);

    /// A blocked metadata read cannot hold completed discoveries past publication.
    #[test]
    fn slow_discovery_publishes_its_age_and_resumes_pending_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let (release, released) = mpsc::channel();
        let (answers, worker) = spawn_resolved_scan_loop(
            Duration::from_millis(80),
            || resolved_test_roots(&[]),
            move || {
                let blocked = std::sync::atomic::AtomicBool::new(false);
                let released = std::sync::Mutex::new(released);
                let mut details = DetailDiscovery::new(move |pid| {
                    if pid == Pid::from_u32(4101) && !blocked.swap(true, Ordering::Relaxed) {
                        released
                            .lock()
                            .expect("release receiver")
                            .recv()
                            .expect("release blocked metadata");
                    }
                    pid
                })?;
                Ok(move |_: &CaptureRoots, window: &ScanPublicationWindow| {
                    let pids: Vec<_> = (4100..4106).map(Pid::from_u32).collect();
                    let identities = pids
                        .iter()
                        .map(|pid| (*pid, InvocationId::for_test(pid.as_u32())))
                        .collect();
                    let completed = details
                        .read(&pids, &identities, |_| ProcessField::Unavailable)
                        .iter()
                        .map(|pid| **pid)
                        .collect::<Vec<_>>();
                    (
                        completed,
                        PublicationAge::between(window.started, Instant::now()),
                    )
                })
            },
        )?;
        let first = answers.recv_timeout(Duration::from_millis(250));
        release.send(()).expect("resume unfinished discovery");
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut second = answers.recv_timeout(Duration::from_millis(250));
        while second
            .as_ref()
            .is_ok_and(|answer| !answer.0.contains(&Pid::from_u32(4101)))
        {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            second = answers.recv_timeout(left);
        }
        drop(answers);
        worker.join().expect("scanner shutdown");
        let first = first.expect("publish before blocked metadata returns");
        let second = second.expect("resumed discovery");
        assert!(!first.0.contains(&Pid::from_u32(4101)));
        assert!(first.1.discovery < Duration::from_millis(250));
        assert!(second.1.discovery < Duration::from_millis(250));
        assert!(first.1.published >= first.1.observed_at());
        assert!(
            second.0.contains(&Pid::from_u32(4101)),
            "pending discovery resumes"
        );
        Ok(())
    }

    #[test]
    fn spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans()
    -> Result<(), Box<dyn std::error::Error>> {
        let parent = tempdir().expect("isolated capture parent");
        let capture = parent.path().join("capture");
        fs::create_dir(&capture).expect("capture directory");
        let caller = thread::current().id();
        let resolutions = Arc::new(AtomicUsize::new(0));
        let worker_resolutions = Arc::clone(&resolutions);
        let (started, resolution_started) = mpsc::channel();
        let (release, resolution_release) = mpsc::channel();
        let worker_capture = capture.clone();
        let (scans, worker) = spawn_resolved_scan_loop(
            CensusCadence::for_test().poll,
            move || {
                assert_ne!(thread::current().id(), caller);
                worker_resolutions.fetch_add(1, Ordering::SeqCst);
                started.send(()).expect("startup observer is alive");
                resolution_release
                    .recv_timeout(WORKER_REPLY_TIMEOUT)
                    .expect("spawn must return before resolution is released");
                resolved_test_roots(&[&worker_capture])
            },
            move || {
                let mut generation = ["first", "second"].into_iter();
                Ok(move |roots: &CaptureRoots, _: &ScanPublicationWindow| {
                    let generation = generation.next().unwrap_or("later");
                    write_versioned_capture(
                        &capture,
                        4_000_000,
                        generation,
                        "/writer/project",
                        "/writer",
                        "build",
                        "",
                    );
                    Capture::take_roots(roots, &|pid| {
                        KernelObservation::for_test(pid, Observation::Ended)
                    });
                    assert!(
                        !capture
                            .join(CAPTURE_LIVE_RUNS_DIR)
                            .join(format!("4000000.{generation}"))
                            .exists()
                    );
                    generation
                })
            },
        )?;

        resolution_started
            .recv_timeout(WORKER_REPLY_TIMEOUT)
            .expect("worker must reach root resolution");
        assert!(matches!(scans.try_recv(), Err(mpsc::TryRecvError::Empty)));
        release.send(()).expect("release worker root resolution");
        assert_eq!(
            scans
                .recv_timeout(WORKER_REPLY_TIMEOUT)
                .expect("worker publishes its first scan"),
            "first"
        );
        assert_eq!(
            scans
                .recv_timeout(WORKER_REPLY_TIMEOUT)
                .expect("worker publishes its second scan"),
            "second"
        );
        drop(scans);
        worker
            .join()
            .expect("scanner must exit after receiver drop");
        assert_eq!(resolutions.load(Ordering::SeqCst), 1);
        Ok(())
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
            identity_source:          InvocationIdentitySource::NativeSnapshot,
            publication_evidence:     PublicationEvidence::Current,
            identities:               parents
                .iter()
                .flat_map(|&pair| <[u32; 2]>::from(pair))
                .map(|pid| (Pid::from_u32(pid), InvocationId::for_test(pid)))
                .collect(),
            capture_boundaries:       HashSet::new(),
            capture_wrappers:         HashSet::new(),
            registered_cargo:         HashSet::new(),
            capture_mismatches:       HashSet::new(),
            lifetimes:                HashMap::new(),
            process_statuses:         HashMap::new(),
            eligibility:              HashMap::new(),
            registration_eligibility: HashMap::new(),
            parents:                  parents
                .iter()
                .map(|&(child, parent)| (Pid::from_u32(child), Pid::from_u32(parent)))
                .collect(),
            cargo:                    Vec::new(),
            rowless_cargo:            Vec::new(),
            dispatched_runs:          HashMap::new(),
            compilers:                Vec::new(),
            cpu:                      HashMap::new(),
            memory:                   HashMap::new(),
            accumulated:              HashMap::new(),
            owners:                   HashMap::new(),
            registration_rows:        Vec::new(),
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
            invocation_id:        InvocationId::for_test(10),
            capture_membership:   CaptureMembership::Outside,
            provenance:           RowProvenance::Uncaptured(ProcessOwner::Unavailable),
            path:                 "~/project".to_owned(),
            directory_identity:   WorkingDirectoryIdentity::Absolute("/writer/project".into()),
            pid:                  10,
            parent:               VisibleParent::None,
            start:                "10:00".to_owned(),
            started:              RunStart::Known(0),
            duration:             "00:01".to_owned(),
            resource_use:         InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            subtree_resource_use: InvocationResourceUse {
                cpu:      Measurement::Reading(0.0),
                memory:   Measurement::Unavailable(MeasurementAbsence::Unproven),
                compiler: CompilerObservation::None,
            },
            state:                CaptureLookup::Unregistered,
            managed:              Measurement::Reading(0),
            nested:               false,
            command:              CommandText::of("cargo", &["build"]),
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
                ancestry: AncestryWalk::Ended(Vec::new()),
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
        Capture::take_roots(&roots, &|observed_pid| {
            KernelObservation::for_test(observed_pid, Observation::Ended)
        });
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
        Capture::take_roots(&roots, &|observed_pid| {
            KernelObservation::for_test(observed_pid, Observation::Ended)
        });

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
            AncestryWalk::Ended(vec![
                Pid::from_u32(6218),
                Pid::from_u32(12444),
                Pid::from_u32(12445),
            ]),
        );
    }

    /// Every command on the machine descends from the init process, so
    /// a row naming it tells one command from no other.
    #[test]
    fn the_walk_stops_short_of_the_process_the_tree_roots_at() {
        let census = census_of(&[(64432, 12445), (12445, 1)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            AncestryWalk::Ended(vec![Pid::from_u32(12445)]),
        );
    }

    /// A reparented chain that comes back round on itself must end the
    /// walk rather than spin it.
    #[test]
    fn a_chain_that_loops_ends_where_it_repeats() {
        let census = census_of(&[(64432, 900), (900, 901), (901, 900)]);

        assert_eq!(
            census.ancestor_pids(Pid::from_u32(64432)),
            AncestryWalk::Ended(vec![Pid::from_u32(901), Pid::from_u32(900)]),
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
    fn nu_is_a_transparent_launcher() {
        assert!(is_transparent(OsStr::new("nu")));
        assert!(is_transparent(OsStr::new("/nix/store/shell/bin/nu")));
    }

    #[test]
    fn a_parent_walk_reports_a_cut_only_when_an_unvisited_parent_remains() {
        for count in [
            0,
            PARENT_WALK_LIMIT - 1,
            PARENT_WALK_LIMIT,
            PARENT_WALK_LIMIT + 1,
        ] {
            let parents: Vec<_> = (0..count)
                .map(|index| {
                    let offset = u32::try_from(index).expect("fixture fits a pid");
                    (100 + offset, 101 + offset)
                })
                .chain(std::iter::once((
                    100 + u32::try_from(count).expect("fixture fits a pid"),
                    ROOT_PROCESS_PID,
                )))
                .collect();
            let census = census_of(&parents);
            let walk = census.ancestor_pids(Pid::from_u32(100));
            assert_eq!(walk.as_slice().len(), count.min(PARENT_WALK_LIMIT));
            let expected: Vec<_> = (0..count.min(PARENT_WALK_LIMIT))
                .rev()
                .map(|index| Pid::from_u32(101 + u32::try_from(index).expect("fixture fits a pid")))
                .collect();
            assert_eq!(walk.as_slice(), expected);
            assert_eq!(
                matches!(walk, AncestryWalk::LimitCut(_)),
                count > PARENT_WALK_LIMIT
            );
        }
        let mut parents: Vec<_> = (0..PARENT_WALK_LIMIT)
            .map(|index| {
                let offset = u32::try_from(index).expect("fixture fits a pid");
                (100 + offset, 101 + offset)
            })
            .collect();
        parents.push((
            100 + u32::try_from(PARENT_WALK_LIMIT).expect("fixture fits a pid"),
            101,
        ));
        assert!(matches!(
            census_of(&parents).ancestor_pids(Pid::from_u32(100)),
            AncestryWalk::Ended(_)
        ));
        parents.pop();
        assert!(matches!(
            census_of(&parents).ancestor_pids(Pid::from_u32(100)),
            AncestryWalk::Ended(_)
        ));
    }

    #[test]
    fn unreadable_and_cargo_ancestors_do_not_erase_a_walk_limit_cut() {
        let parents: Vec<_> = (0..=PARENT_WALK_LIMIT)
            .map(|index| {
                let offset = u32::try_from(index).expect("fixture fits a pid");
                (100 + offset, 101 + offset)
            })
            .collect();
        let census = census_of(&parents);
        let runner_argv = [
            OsString::from("/nix/store/runner/bin/Runner.Worker"),
            "spawnclient".into(),
        ];
        let cargo_argv = [OsString::from("cargo"), "check".into()];
        let observations = ProcessObservations::new([
            ProcessObservation::cargo(101, &runner_argv),
            ProcessObservation::cargo(102, &cargo_argv),
        ]);
        let ancestry = census.ancestry(&observations, ScannerHome::Unavailable, Pid::from_u32(100));
        assert!(matches!(ancestry, AncestryWalk::LimitCut(_)));
        assert_eq!(ancestry.as_slice().len(), 1);
        assert_eq!(ancestry.as_slice()[0].pid, 101);
        assert!(
            matches!(census.ancestry(&ProcessObservations::default(), ScannerHome::Unavailable, Pid::from_u32(100)), AncestryWalk::LimitCut(levels) if levels.is_empty())
        );
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

    /// The list as it reaches [`CommandText::is_hidden`] once
    /// the config has turned it into owned strings.

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
            aggregate(&shares, members).map(cpu_label).to_string(),
            "391%"
        );
    }

    #[test]
    fn a_group_with_a_missing_share_is_unavailable() {
        assert_eq!(
            aggregate(
                &HashMap::<Pid, Measurement<f32>>::new(),
                std::iter::once(Pid::from(1)),
            ),
            Measurement::Unavailable(MeasurementAbsence::Unproven),
        );
    }

    #[test]
    fn a_group_of_measured_zero_shares_reads_as_idle() {
        let pid = Pid::from(1);
        let shares = HashMap::from([(pid, Measurement::Reading(0.0))]);
        assert_eq!(
            aggregate(&shares, std::iter::once(pid))
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
                    aggregate(&shares, members.into_iter()),
                    Measurement::Unavailable(reason)
                );
            }
        }
    }

    #[test]
    fn a_live_compiler_keeps_its_first_owner_after_owner_and_census_disappear() {
        let pid = std::process::id();
        let LifetimeEvidence::Available(lifetime) = birth_stamp::lifetime(pid) else {
            panic!("current process must have a native lifetime");
        };
        let compiler = ProcessIdentity::Known { pid, lifetime };
        let owner = InvocationId::for_test(1);
        let mut smoothing = InvocationCpuAccounting::default();
        smoothing
            .cache_owners
            .insert(compiler.clone(), owner.clone());
        let mut census = census_of(&[]);
        census.attribute(
            &ProcessObservations::default(),
            &mut smoothing,
            Instant::now(),
        );
        assert_eq!(smoothing.cache_owners.get(&compiler), Some(&owner));
        census
            .lifetimes
            .insert(Pid::from_u32(pid), LifetimeEvidence::Unavailable);
        census.attribute(
            &ProcessObservations::default(),
            &mut smoothing,
            Instant::now(),
        );
        assert_eq!(smoothing.cache_owners.get(&compiler), Some(&owner));
    }

    #[test]
    fn native_replacement_retires_the_prior_compilers_owner() {
        let mut census = census_of(&[]);
        let compiler = ProcessIdentity::Known {
            pid:      123,
            lifetime: ProcessLifetime::for_test(1),
        };
        let mut smoothing = InvocationCpuAccounting::default();
        smoothing
            .cache_owners
            .insert(compiler.clone(), InvocationId::for_test(1));
        census.lifetimes.insert(
            Pid::from_u32(123),
            LifetimeEvidence::Available(ProcessLifetime::for_test(2)),
        );
        census.attribute(
            &ProcessObservations::default(),
            &mut smoothing,
            Instant::now(),
        );
        assert!(!smoothing.cache_owners.contains_key(&compiler));
    }

    #[test]
    fn root_cargo_is_read_before_the_compiler_it_can_reap() {
        let root = Pid::from_u32(ROOT_PROCESS_PID);
        let child = Pid::from_u32(ROOT_PROCESS_PID + 1);
        let mut census = census_of(&[(child.as_u32(), root.as_u32()), (root.as_u32(), 0)]);
        census.cargo.push(root);
        for pid in [root, child] {
            census.lifetimes.insert(
                pid,
                LifetimeEvidence::Available(ProcessLifetime::for_test(u64::from(pid.as_u32()))),
            );
            census.accumulated.insert(pid, Duration::ZERO);
        }
        let mut order = Vec::new();
        let mut work = HashMap::from([(root, InvocationCpuContributions::default())]);
        census.collect_cpu_work(&HashMap::new(), &mut work, &mut HashMap::new(), |pid| {
            // Reading the child first lets a subsequent wait repeat its time in root.
            let elapsed = if pid == child || order.contains(&child) {
                100
            } else {
                0
            };
            order.push(pid);
            Measurement::Reading(Duration::from_millis(elapsed))
        });
        assert_eq!(order, [root, child]);
        assert_eq!(
            work[&root].tree.values().sum::<Duration>(),
            Duration::from_millis(100)
        );
    }

    /// An idle owner keeps the rate macOS last computed for it, yet its sound
    /// counter still lets its invocation publish a reading.
    #[test]
    fn an_owner_with_a_retained_rate_still_reads_its_invocation() {
        let owner = Pid::from_u32(ROOT_PROCESS_PID + 1);
        let mut census = census_of(&[(owner.as_u32(), ROOT_PROCESS_PID)]);
        census.cargo.push(owner);
        census.lifetimes.insert(
            owner,
            LifetimeEvidence::Available(ProcessLifetime::for_test(u64::from(owner.as_u32()))),
        );
        census.accumulated.insert(owner, Duration::from_millis(10));
        let mut work = HashMap::from([(owner, InvocationCpuContributions::default())]);
        let mut evidence = HashMap::from([(
            owner,
            Measurement::Unavailable(MeasurementAbsence::RetainedRate),
        )]);
        census.collect_cpu_work(&HashMap::new(), &mut work, &mut evidence, |_| {
            Measurement::Reading(Duration::from_millis(10))
        });
        assert_eq!(evidence[&owner], Measurement::Reading(0.0));
    }

    #[test]
    fn proven_registration_identity_changes_preserve_the_same_process_cpu_baseline() {
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
        let pid = Pid::from_u32(20);
        let mut census = census_of(&[(pid.as_u32(), 10)]);
        census.cargo.push(pid);
        census.lifetimes.insert(
            pid,
            LifetimeEvidence::Available(ProcessLifetime::for_test(20)),
        );
        census.accumulated.insert(pid, Duration::ZERO);
        census.cpu.insert(
            pid,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation),
        );
        let mut smoothing = InvocationCpuAccounting::default();
        let now = Instant::now();
        let first =
            census.attribute_cpu_with(&ProcessObservations::default(), &mut smoothing, now, |_| {
                Measurement::Reading(Duration::from_millis(100))
            });
        assert_eq!(
            first[&pid],
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        census.cpu.insert(pid, Measurement::Reading(0.0));
        census.identify_captures(&capture);
        assert!(matches!(census.identities[&pid], InvocationId::Captured(_)));
        let recovered = census.attribute_cpu_with(
            &ProcessObservations::default(),
            &mut smoothing,
            now + Duration::from_secs(1),
            |_| Measurement::Reading(Duration::from_millis(200)),
        );
        assert_eq!(recovered[&pid], Measurement::Reading(10.0));
        census
            .identities
            .insert(pid, InvocationId::for_test(pid.as_u32()));
        let unconfirmed = census.attribute_cpu_with(
            &ProcessObservations::default(),
            &mut smoothing,
            now + Duration::from_secs(2),
            |_| Measurement::Reading(Duration::from_millis(300)),
        );
        assert_eq!(unconfirmed[&pid], Measurement::Reading(10.0));
        assert_eq!(smoothing.invocations.len(), 1);
    }

    #[test]
    fn a_late_cpu_scan_keeps_the_configured_smoothing_window() {
        let pid = Pid::from_u32(100);
        let mut census = census_of(&[(pid.as_u32(), ROOT_PROCESS_PID)]);
        census.cargo.push(pid);
        census.lifetimes.insert(
            pid,
            LifetimeEvidence::Available(ProcessLifetime::for_test(100)),
        );
        census.identities.insert(pid, InvocationId::for_test(100));
        census.accumulated.insert(pid, Duration::ZERO);
        census.cpu.insert(pid, Measurement::Reading(0.0));
        let mut accounting = InvocationCpuAccounting::new(CensusCadence::for_test());
        let now = Instant::now();
        for (elapsed, accumulated) in [(0, 0), (50, 50), (1050, 110)] {
            let measured = census.attribute_with(
                &ProcessObservations::default(),
                &mut accounting,
                now + Duration::from_millis(elapsed),
                |_| Measurement::Reading(Duration::from_millis(accumulated)),
            );
            if elapsed == 1050 {
                let Measurement::Reading(cpu) = measured.cpu[&pid] else {
                    panic!("the same live process has three CPU counter samples");
                };
                assert!(
                    (cpu - 6.0).abs() < 0.01,
                    "one second exceeds ten test smoothing windows: {cpu}%"
                );
            }
        }
    }

    #[test]
    fn non_cargo_argv_does_not_hide_a_detached_compiler_beneath_a_cargo_name() {
        let root = tempdir().expect("compiler metadata fixture");
        let [requester, server, worker, compiler] = [100, 101, 102, 103].map(Pid::from_u32);
        let arguments = [
            vec![
                OsString::from(CARGO_DISPLAY_NAME),
                "build".into(),
                CARGO_TARGET_DIR_FLAG.into(),
                root.path().as_os_str().into(),
            ],
            vec![SCCACHE_BINARY.into()],
            vec!["cache-worker".into()],
            vec![
                RUSTC_BINARY.into(),
                RUSTC_OUT_DIR_FLAG.into(),
                root.path().as_os_str().into(),
            ],
        ];
        let mut system = ProcessObservations::new(
            [
                (requester, ROOT_PROCESS_PID, CARGO_DISPLAY_NAME),
                (server, ROOT_PROCESS_PID, SCCACHE_BINARY),
                (worker, server.as_u32(), CARGO_DISPLAY_NAME),
                (compiler, worker.as_u32(), RUSTC_BINARY),
            ]
            .into_iter()
            .zip(&arguments)
            .map(|((pid, parent, name), argv)| {
                let mut record = ProcessObservation::cargo(pid.as_u32(), argv);
                record.parent = ProcessField::Observed(Pid::from_u32(parent));
                record.name = ProcessField::Observed(OsStr::new(name));
                record
            }),
        );
        let mut census = Census::take(system.records.values());
        // Native name and argv disagree; ancestry crosses the intermediate worker.
        assert!(
            census.cargo.contains(&worker),
            "native cargo name must enter discovery"
        );
        let process = system.process(worker).expect("worker metadata");
        assert_eq!(
            command_text::select_cargo(process.cmd(), &[]),
            Err(RowAbsence::ProgramRejected)
        );
        census.identities = ProcessIdentities::default().observe(&census.lifetimes);
        census.select_rows(&system, &Capture::default(), &[]);
        assert_eq!(census.live_cargo().collect::<Vec<_>>(), [requester]);
        let targets = census.observe_targets(&system, &HashMap::new(), &mut HashMap::new());
        assert_eq!(
            census.detached_compilers(&system, &targets),
            HashMap::from([(compiler, CompilerAttribution::Owner(requester))])
        );

        census.cpu.insert(requester, Measurement::Reading(0.0));
        let mut smoothing = InvocationCpuAccounting::default();
        let now = Instant::now();
        for (offset, milliseconds, expected) in [
            (
                0,
                100,
                Measurement::Unavailable(MeasurementAbsence::FirstObservation),
            ),
            (1, 200, Measurement::Reading(10.0)),
        ] {
            let measured = census.attribute_cpu_with(
                &system,
                &mut smoothing,
                now + Duration::from_secs(offset),
                |pid| {
                    Measurement::Reading(if pid == compiler {
                        Duration::from_millis(milliseconds)
                    } else {
                        Duration::ZERO
                    })
                },
            );
            assert_eq!(measured[&requester], expected);
        }
        for uid in [ProcessField::Unavailable, ProcessField::Observed(1001)] {
            system
                .records
                .get_mut(&compiler)
                .expect("compiler observation")
                .uid = uid;
            assert!(
                !matches!(
                    census.detached_compilers(&system, &targets)[&compiler],
                    CompilerAttribution::Owner(_)
                ),
                "unobserved or different users cannot share target ownership"
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
    fn collection_names_a_retained_rate_when_cpu_time_is_unchanged() {
        let pid = Pid::from(1);
        let previous = HashMap::from([(pid, cpu_baseline(10))]);
        let measured = Census::measure_cpu(pid, 80.0, &cpu_baseline(10), &previous);
        assert_eq!(
            measured,
            Measurement::Unavailable(MeasurementAbsence::RetainedRate)
        );
        assert_eq!(measured.map(cpu_label).to_string(), UNAVAILABLE_MEASUREMENT);
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
    fn sequence_retains_cpu_and_identity_but_replaces_each_process_snapshot() {
        let argv = [OsString::from("cargo"), OsString::from("build")];
        let mut sequence = CensusSequence::default();
        let now = Instant::now();
        let snapshots = [(10, 100), (10, 300), (20, 500)];
        let samples: Vec<_> = snapshots
            .into_iter()
            .enumerate()
            .map(|(index, (birth, millis))| {
                let mut record = ProcessObservation::cargo(100, &argv);
                record.lifetime = Cow::Owned(LifetimeEvidence::Available(
                    ProcessLifetime::for_test(birth),
                ));
                record.accumulated = millis;
                record
                    .native_cpu
                    .set(Measurement::Reading(Duration::from_millis(millis)));
                sequence.sample_counters(
                    &mut ProcessObservations::new([record]),
                    now + Duration::from_secs(u64::try_from(index).expect("sample index")),
                )
            })
            .collect();
        assert!(
            samples
                .iter()
                .all(|groups| groups.len() == 1 && groups[0].rest.is_empty())
        );
        assert_eq!(
            samples[0][0].lead.resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert_eq!(
            samples[1][0].lead.resource_use.cpu,
            Measurement::Reading(20.0)
        );
        assert_eq!(samples[0][0].id(), samples[1][0].id());
        assert_ne!(samples[1][0].id(), samples[2][0].id());
        assert_eq!(
            samples[2][0].lead.resource_use.cpu,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert_eq!(
            sequence.sample_counters(
                &mut ProcessObservations::default(),
                now + Duration::from_secs(3)
            ),
            [] as [CargoGroup; 0]
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
        let discovery: Vec<_> = system
            .processes()
            .values()
            .map(|process| (process, CpuBaseline::from(process)))
            .collect();
        let observations = ProcessObservations::from_discovery(&discovery, &HashMap::new());
        let census = Census::take(observations.records.values());
        assert_eq!(
            census.cpu.get(&pid),
            Some(&Measurement::Unavailable(
                MeasurementAbsence::FirstObservation
            ))
        );
    }

    #[test]
    fn cargo_ancestry_preserves_unavailable_parent_evidence() {
        let census = census_of(&[(2, 1)]);
        assert_eq!(
            census.owning_cargo(Pid::from_u32(2)),
            CargoAncestry::ParentUnavailable
        );
    }

    #[test]
    fn cargo_ancestry_preserves_a_reached_walk_limit() {
        let census = census_of(&[(2, 3), (3, 2)]);
        assert_eq!(
            census.owning_cargo(Pid::from_u32(2)),
            CargoAncestry::WalkLimitReached
        );
    }

    #[test]
    fn cargo_ancestry_identifies_an_observed_owner() {
        let mut census = census_of(&[(3, 2), (2, 1)]);
        census.cargo.push(Pid::from_u32(1));
        assert_eq!(
            census.owning_cargo(Pid::from_u32(3)),
            CargoAncestry::Owner(Pid::from_u32(1))
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
    /// The scale is `top`'s, so a build across several cores reads past
    /// 100% rather than being folded back into a share of the machine.
    #[test]
    fn a_cpu_share_reads_as_a_whole_number_of_percent() {
        assert_eq!(cpu_label(0.0), "0%");
        assert_eq!(cpu_label(12.4), "12%");
        assert_eq!(cpu_label(12.6), "13%");
        assert_eq!(cpu_label(783.2), "783%");
    }
}
