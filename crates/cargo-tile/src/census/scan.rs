//! Process discovery, attribution, and assembly of invocation groups.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fs;
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
use rustix::io::Errno;
use rustix::process::Pid as KernelPid;
use rustix::process::test_kill_process;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;
use tui_pane::kernel_parent;

use super::command_text;
use super::command_text::CommandText;
use super::command_text::RowAbsence;
use super::command_text::ScannerHome;
use super::direct_capture::DirectAssociation;
use super::direct_capture::DirectCapture;
use super::direct_capture::NearestRegistration;
use super::direct_capture::SelectedProof;
use super::invocation_cpu_accounting;
use super::invocation_cpu_accounting::CompileOwner;
use super::invocation_cpu_accounting::CompilerCreditRetention;
use super::invocation_cpu_accounting::CpuAssignment;
use super::invocation_cpu_accounting::CpuBaseline;
use super::invocation_cpu_accounting::InvocationCpuAccounting;
use super::invocation_cpu_accounting::InvocationCpuContributions;
use super::invocation_cpu_accounting::InvocationMeasurements;
use super::invocation_cpu_accounting::Measurement;
use super::invocation_cpu_accounting::MeasurementAbsence;
use super::process_identity::CaptureMembership;
use super::process_identity::InvocationId;
#[cfg(test)]
use super::process_identity::ProcessIdentities;
use super::process_identity::ProcessIdentity;
use super::process_identity::RunId;
use super::process_identity::VisibleParent;
#[cfg(any(target_os = "linux", test))]
use crate::birth_stamp;
use crate::birth_stamp::LifetimeEvidence;
#[cfg(test)]
use crate::birth_stamp::ProcessLifetime;
use crate::config::Config;
use crate::constants::ARGUMENT_SEPARATOR;
use crate::constants::CARGO_DISPLAY_NAME;
use crate::constants::CARGO_JSON_FORMAT_PREFIX;
use crate::constants::CARGO_MESSAGE_FORMAT_FLAG;
use crate::constants::CARGO_MESSAGE_FORMAT_JSON_PREFIX;
use crate::constants::CARGO_PROCESS_NAMES;
use crate::constants::CARGO_QUIET_FLAGS;
#[cfg(test)]
use crate::constants::CARGO_TARGET_DIR_FLAG;
use crate::constants::COMPILER_PROCESS_NAMES;
use crate::constants::PARENT_WALK_LIMIT;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::ROOT_PROCESS_PID;
use crate::constants::RUSTC_BINARY;
use crate::constants::RUSTC_OUT_DIR_FLAG;
use crate::constants::SCCACHE_BINARY;
use crate::constants::SECONDS_PER_HOUR;
use crate::constants::SECONDS_PER_MINUTE;
use crate::constants::START_TIME_FORMAT;
use crate::constants::TRANSPARENT_PROCESS_NAMES;
use crate::constants::UNRESOLVED_PATH;
use crate::constants::UNRESOLVED_TIME;
use crate::progress::capture::Capture;
use crate::progress::capture::CaptureKey;
use crate::progress::capture::CaptureSelection;
use crate::progress::capture_read::CaptureLookup;
use crate::progress::capture_roots::CaptureRoots;
use crate::registration::RegistrationCandidate;
use crate::registration::WorkingDirectoryIdentity;
use crate::registration::WriterHome;
use crate::render::CaptureAccount;
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
    /// Whole-number core percentage for the command table. A group lead
    /// carries the group's total; managed rows carry only their own invocation
    /// bucket, including non-cargo descendants attributed to that invocation.
    pub(crate) cpu:                Measurement<String>,
    /// This invocation's CPU bucket plus every assembled cargo descendant,
    /// prepared on the worker for promotion into the summary. Any unavailable
    /// contributor prevents publishing a partial total.
    pub(crate) subtree_cpu:        Measurement<String>,
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
        let mut smoothing = InvocationCpuAccounting::default();
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
    smoothing: &mut InvocationCpuAccounting,
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
    // Read CPU counters on the full-system pass so short-lived descendants
    // can contribute without requiring detailed command metadata. The invocation's
    // own counter must be readable before accumulated work is published.
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        process_discovery_refresh_kind(),
    );

    let discovery: Vec<_> = system
        .processes()
        .values()
        .map(|process| (process, CpuBaseline::from(process)))
        .collect();
    let mut observations = ProcessObservations::from_discovery(&discovery, &previous);
    let mut census = Census::take(observations.records.values());
    census.identities = smoothing.identities.observe(&census.lifetimes);

    // Phase two reads cargo, its ancestors, and compiler drivers. Ancestors name
    // the launcher; compiler arguments associate cache-server work with its requester.
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
    observations.overlay_details(&details);
    census.prepare(&observations, &capture, excluded);
    let attributed = census.attribute(&observations, smoothing, now);
    let groups = census.groups(&observations, &attributed, home, &capture);
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
    pub(super) name:        ProcessField<&'scan OsStr>,
    pub(super) argv:        ProcessField<&'scan [OsString]>,
    pub(crate) directory:   ProcessField<&'scan Path>,
    pub(super) executable:  ProcessField<&'scan Path>,
    pub(crate) uid:         ProcessField<u32>,
    pub(super) environment: &'scan [OsString],
    pub(super) lifetime:    Cow<'scan, LifetimeEvidence>,
    pub(super) cpu:         Measurement<f32>,
    pub(super) accumulated: u64,
    /// Ordered accounting stores the native read while metadata remains shared.
    pub(super) native_cpu:  Cell<Measurement<Duration>>,
    pub(super) started:     u64,
    pub(super) elapsed:     u64,
    details:                DetailPresence,
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
            lifetime: Cow::Borrowed(lifetime),
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
            cpu:         Measurement::Unavailable(MeasurementAbsence::Unproven),
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

/// What phase one learned: the parent links, the cargo processes, and the
/// compiler processes waiting to be attributed to one of them.
pub(super) struct Census {
    /// Registration eligibility is resolved even when no process row can be built.
    registration_eligibility: HashMap<RunId, Result<(), RowAbsence>>,
    /// Invocation identities are established before attribution and row construction.
    pub(super) identities:    HashMap<Pid, InvocationId>,
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
    /// Cargo invocations retained for row construction after selection.
    cargo:                    Vec<Pid>,
    /// Hidden invocations still establish CPU ownership and competing target evidence.
    rowless_cargo:            Vec<Pid>,
    /// Compiler processes paired with which driver they are.
    compilers:                Vec<(Pid, &'static str)>,
    /// Per-process validation is consulted only for an invocation's own counter.
    cpu:                      HashMap<Pid, Measurement<f32>>,
    /// Own accumulated task time is the available counter on Darwin.
    accumulated:              HashMap<Pid, Duration>,
    /// Fixture rows enter group assembly without a process-row measurement.
    #[cfg(test)]
    registration_rows:        Vec<CargoProcess>,
}

impl Census {
    /// Both kernel and synthetic observations use the same discovery-to-row selection.
    fn prepare(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
    ) {
        self.include_registered_processes(records, capture);
        self.identify_capture_wrappers(records, capture);
        self.collapse_shims(records);
        self.identify_captures(capture);
        self.select_rows(records, capture, excluded);
    }

    /// Classify every process the last refresh saw.
    fn take<'scan>(records: impl IntoIterator<Item = &'scan ProcessObservation<'scan>>) -> Self {
        let mut census = Self {
            identities:                     HashMap::new(),
            capture_boundaries:             HashSet::new(),
            capture_wrappers:               HashSet::new(),
            capture_mismatches:             HashSet::new(),
            lifetimes:                      HashMap::new(),
            eligibility:                    HashMap::new(),
            registration_eligibility:       HashMap::new(),
            parents:                        HashMap::new(),
            cargo:                          Vec::new(),
            rowless_cargo:                  Vec::new(),
            compilers:                      Vec::new(),
            cpu:                            HashMap::new(),
            accumulated:                    HashMap::new(),
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
            census
                .accumulated
                .insert(pid, Duration::from_millis(process.accumulated));
            census.cpu.insert(pid, process.cpu);
            let name = process.name();
            if command_text::is_cargo_name(name) {
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

    /// Everything the census has to say about each cargo invocation.
    ///
    /// The two walks are one call because they run over the same parent
    /// chains and are both spent by the same pass over the groups.
    fn attribute(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
    ) -> InvocationMeasurements {
        self.attribute_with(system, smoothing, now, |pid| {
            let measurement = self.process_cpu_time(pid);
            system.records.get(&pid).map_or(measurement, |record| {
                record.native_cpu.set(measurement);
                record.native_cpu.get()
            })
        })
    }

    /// Kernel and fixture counters share invocation smoothing and published PID readings.
    fn attribute_with(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> InvocationMeasurements {
        smoothing.observed.clone_from(&self.lifetimes);
        let sampled: HashMap<_, _> = self
            .attribute_cpu_with(system, smoothing, now, read)
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
        InvocationMeasurements {
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

    /// Measure accumulated invocation work between scans, validating only its own counter.
    /// Direct descendants take precedence over output-based cache attribution.
    /// Counter reads share the same ordered accounting pass for kernel and fixture samples.
    fn attribute_cpu_with(
        &self,
        system: &ProcessObservations<'_>,
        smoothing: &mut InvocationCpuAccounting,
        now: Instant,
        read: impl FnMut(Pid) -> Measurement<Duration>,
    ) -> HashMap<Pid, Measurement<f32>> {
        smoothing.identify_owners(self);
        let identities: HashSet<_> = smoothing.owners.values().cloned().collect();
        smoothing
            .invocations
            .retain(|identity, _| identities.contains(identity));
        smoothing
            .targets
            .retain(|identity, _| identities.contains(identity));
        let targets = self.observe_targets(system, &smoothing.owners, &mut smoothing.targets);
        smoothing.cache_owners.retain(|identity, _| {
            self.compiler_credit_retention(identity) == CompilerCreditRetention::Keep
        });
        let detached = self
            .detached_compilers(system, &targets)
            .into_iter()
            .filter(|&(compiler, owner)| {
                let (Ok(compiler), Ok(process)) =
                    (self.cpu_identity(compiler), self.cpu_identity(owner))
                else {
                    return false;
                };
                let Some(owner) = smoothing.owners.get(&process) else {
                    return false;
                };
                smoothing
                    .cache_owners
                    .entry(compiler)
                    .or_insert_with(|| owner.clone())
                    == owner
            })
            .collect::<HashMap<_, _>>();
        let mut work: HashMap<Pid, InvocationCpuContributions> = self
            .live_cargo()
            .map(|pid| (pid, InvocationCpuContributions::default()))
            .collect();
        let mut evidence = self.cpu.clone();
        self.collect_cpu_work(&detached, &mut work, &mut evidence, read);
        #[cfg(target_os = "linux")]
        self.exclude_nested_cpu(&mut work, smoothing);
        work.into_iter()
            .filter_map(|(pid, work)| {
                let process = self.cpu_identity(pid).ok()?;
                let identity = smoothing.owners.get(&process)?;
                let evidence = evidence
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven));
                let sample = smoothing
                    .invocations
                    .entry(identity.clone())
                    .or_default()
                    .measure(work, evidence, now);
                Some((pid, sample))
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
                    || (evidence.get(&owner)
                        == Some(&Measurement::Unavailable(
                            MeasurementAbsence::FirstObservation,
                        ))
                        && !elapsed.is_zero()))
            {
                // Linux's lifetime-checked stat read proves even an idle owner's
                // zero counter independently of sysinfo's unavailable rate.
                // Darwin's zero may still be a failed task-info read.
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
        system: &ProcessObservations<'_>,
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
            if let Some(process) = system.process(pid)
                && let Ok(directory) = invocation_cpu_accounting::cargo_target_directory(process)
            {
                targets.entry(pid).or_default().insert(directory);
            }
        }
        for &(pid, _) in &self.compilers {
            if let Some(owner) = self.cpu_owners(pid).next()
                && let Some(process) = system.process(pid)
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
        system: &ProcessObservations<'_>,
        targets: &HashMap<Pid, HashSet<PathBuf>>,
    ) -> HashMap<Pid, Pid> {
        self.compilers
            .iter()
            .filter_map(|&(pid, driver)| {
                if driver != RUSTC_BINARY
                    || self.cpu_owners(pid).next().is_some()
                    || !self
                        .cpu_ancestry(pid)
                        .iter()
                        .any(|ancestor| self.compilers.contains(&(*ancestor, SCCACHE_BINARY)))
                {
                    return None;
                }
                let process = system.process(pid)?;
                let directory =
                    invocation_cpu_accounting::process_argument_path(process, RUSTC_OUT_DIR_FLAG)
                        .ok()?;
                let owners = self.live_cargo().filter(|owner| {
                    let Some(cargo) = system.process(*owner) else {
                        return false;
                    };
                    // A shared filesystem path alone never links work from different users.
                    if process.uid == ProcessField::Unavailable || process.uid != cargo.uid {
                        return false;
                    }
                    targets.get(owner).is_some_and(|targets| {
                        targets.iter().any(|target| directory.starts_with(target))
                    })
                });
                match invocation_cpu_accounting::compile_owner(owners) {
                    CompileOwner::Unique(owner) => Some((pid, owner)),
                    CompileOwner::Unknown | CompileOwner::Ambiguous => None,
                }
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
    fn collapse_shims(&mut self, system: &ProcessObservations<'_>) {
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
    fn is_shim(
        system: &ProcessObservations<'_>,
        children: &HashMap<Pid, Vec<Pid>>,
        pid: Pid,
    ) -> bool {
        let Some(kids) = children.get(&pid) else {
            return false;
        };
        let [child] = kids[..] else {
            return false;
        };
        let (Some(outer), Some(inner)) = (system.process(pid), system.process(child)) else {
            return false;
        };
        observed_shim_match(outer.cmd(), outer.cwd(), inner.cmd(), inner.cwd())
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
    fn ancestry(
        &self,
        system: &ProcessObservations<'_>,
        home: ScannerHome<'_>,
        pid: Pid,
    ) -> Vec<Ancestor> {
        self.ancestor_pids(pid)
            .into_iter()
            .filter_map(|pid| system.process(pid))
            .filter(|process| !command_text::names_cargo(process.cmd()))
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
    fn include_registered_processes(
        &mut self,
        system: &ProcessObservations<'_>,
        capture: &Capture,
    ) {
        for pid in capture.registered_pids().into_iter().map(Pid::from_u32) {
            if system
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
    fn identify_capture_wrappers(&mut self, system: &ProcessObservations<'_>, capture: &Capture) {
        self.capture_mismatches = system
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
                        !forwards_capture_command(process.cmd(), confirmed.registration.record())
                    })
                    .map(|_| pid)
            })
            .collect();
        self.capture_wrappers = system
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
    fn select_rows(
        &mut self,
        system: &ProcessObservations<'_>,
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
            let process = system
                .process(pid)
                .map_or(Err(RowAbsence::ArgvUnavailable), |process| {
                    command_text::select_cargo(process.cmd(), excluded).map(|_| ())
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
                system.process(*pid).is_none_or(|process| {
                    process.cmd().is_empty() || command_text::names_cargo(process.cmd())
                })
            })
            .collect();
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
        system: &ProcessObservations<'_>,
        attributed: &InvocationMeasurements,
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
        #[cfg(test)]
        rows.extend_from_slice(&self.registration_rows);
        let mut groups = self.assemble_groups(system, home, rows);
        for group in &mut groups {
            let members: Vec<_> = std::iter::once(&group.lead).chain(&group.rest).collect();
            let totals = subtree_cpu(&attributed.cpu, &members, &process_rows);
            if process_rows.contains(&group.lead.invocation_id) {
                let pids = members.iter().map(|row| Pid::from_u32(row.pid));
                let proven: HashSet<_> = members
                    .iter()
                    .filter(|row| process_rows.contains(&row.invocation_id))
                    .map(|row| row.pid)
                    .collect();
                let cpu = if members.iter().all(|row| proven.contains(&row.pid)) {
                    aggregate_cpu(&attributed.cpu, pids.clone()).map(cpu_label)
                } else {
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                };
                let compiler = aggregate_compilers(&attributed.compilers, pids);
                group.lead.cpu = cpu;
                group.lead.compiler = compiler;
            }
            for (row, total) in std::iter::once(&mut group.lead)
                .chain(&mut group.rest)
                .zip(totals)
            {
                row.subtree_cpu = total.map(cpu_label);
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
        system: &ProcessObservations<'_>,
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
        system: &ProcessObservations<'_>,
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
    if let Ok(arguments) = command_text::cargo_split(argv)
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
    matches!((command_text::subcommand(outer), command_text::subcommand(inner), outer_cwd, inner_cwd),
        (Ok(outer), Ok(inner), WorkingDirectoryObservation::Observed(outer_cwd), WorkingDirectoryObservation::Observed(inner_cwd))
        if outer == inner && DirectoryComparison::between(outer_cwd, inner_cwd) == DirectoryComparison::Same)
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
    let mut seen = HashSet::new();
    members
        .filter(|pid| seen.insert(*pid))
        .fold(Measurement::Reading(0.0), |total, pid| {
            total
                + shares
                    .get(&pid)
                    .copied()
                    .unwrap_or(Measurement::Unavailable(MeasurementAbsence::Unproven))
        })
}

/// Sum each pid bucket once per subtree while retaining unavailable identities.
/// The membership order also indexes the returned totals, independently of row age.
fn subtree_cpu(
    shares: &HashMap<Pid, Measurement<f32>>,
    members: &[&CargoProcess],
    process_rows: &HashSet<InvocationId>,
) -> Vec<Measurement<f32>> {
    let indices: HashMap<_, _> = members
        .iter()
        .enumerate()
        .map(|(index, row)| (&row.invocation_id, index))
        .collect();
    let mut totals: Vec<_> = members
        .iter()
        .map(|row| {
            if process_rows.contains(&row.invocation_id) {
                aggregate_cpu(shares, std::iter::once(Pid::from_u32(row.pid)))
            } else {
                Measurement::Unavailable(MeasurementAbsence::Unproven)
            }
        })
        .collect();
    let mut remaining_children = vec![0_usize; members.len()];
    let mut contributors: Vec<HashSet<Pid>> = members
        .iter()
        .map(|row| {
            if process_rows.contains(&row.invocation_id) {
                HashSet::from([Pid::from_u32(row.pid)])
            } else {
                HashSet::new()
            }
        })
        .collect();
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
            let additional: Vec<_> = contributors[index]
                .difference(&contributors[parent])
                .copied()
                .collect();
            totals[parent] = totals[parent]
                + match totals[index] {
                    Measurement::Unavailable(reason) => Measurement::Unavailable(reason),
                    Measurement::Reading(_) => aggregate_cpu(shares, additional.iter().copied()),
                };
            contributors[parent].extend(additional);
            remaining_children[parent] -= 1;
            if remaining_children[parent] == 0 {
                ready.push(parent);
            }
        }
    }
    // A cyclic parent chain cannot establish a complete subtree measurement.
    for (total, children) in totals.iter_mut().zip(remaining_children) {
        if children > 0 {
            *total = Measurement::Unavailable(MeasurementAbsence::Unproven);
        }
    }
    totals
}

/// Format one cargo process into its table row.
fn row(
    process: &ProcessObservation<'_>,
    invocation_id: InvocationId,
    compiler: CompilerObservation,
    managed: Measurement<usize>,
    home: ScannerHome<'_>,
    cpu: Measurement<f32>,
    direct: &DirectAssociation,
) -> Result<CargoProcess, RowAbsence> {
    let (directory_identity, path, command) =
        row_fields(process.cwd(), process.cmd(), direct, home)?;
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
        subtree_cpu: Measurement::Unavailable(MeasurementAbsence::Unproven),
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
        provenance: RowProvenance::direct(capture, &direct.key),
        path: registration_directory(record, home),
        directory_identity: record.directory_identity(),
        pid: direct.key.pid,
        parent: VisibleParent::None,
        start,
        started,
        duration,
        cpu: Measurement::Unavailable(MeasurementAbsence::Unproven),
        subtree_cpu: Measurement::Unavailable(MeasurementAbsence::Unproven),
        compiler: CompilerObservation::Unknown,
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
fn describe(process: &ProcessObservation<'_>, home: ScannerHome<'_>) -> String {
    let line: Vec<String> = process
        .cmd()
        .iter()
        .map(|word| command_text::home_relative(Path::new(word), home))
        .collect();
    if !line.is_empty() {
        return line.join(" ");
    }
    match process.exe() {
        ProcessField::Observed(exe) => command_text::home_relative(exe, home),
        ProcessField::Unavailable => process.name().to_string_lossy().into_owned(),
    }
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

/// Successive snapshots retain invocation identity and cumulative CPU history.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CensusSequence {
    smoothing:                    InvocationCpuAccounting,
    previous:                     HashMap<Pid, CpuBaseline>,
    pub(super) registration_rows: Vec<CargoProcess>,
}

#[cfg(test)]
impl CensusSequence {
    /// Supplied rates exercise selection and assembly without invoking OS accounting.
    pub(super) fn sample(&mut self, records: &ProcessObservations<'_>) -> Vec<CargoGroup> {
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

    fn sample_attributed(
        &mut self,
        records: &ProcessObservations<'_>,
        capture: &Capture,
        excluded: &[String],
        home: ScannerHome<'_>,
        cpu: HashMap<Pid, Measurement<f32>>,
    ) -> Vec<CargoGroup> {
        let mut census = Census::take(records.records.values());
        census.identities = self.smoothing.identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        census.prepare(records, capture, excluded);
        let attributed = InvocationMeasurements {
            compilers: census.attribute_compilers(),
            cpu,
        };
        census.groups(records, &attributed, home, capture)
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
        census.identities = self.smoothing.identities.observe(&census.lifetimes);
        census.registration_rows.clone_from(&self.registration_rows);
        census.prepare(records, capture, excluded);
        let attributed = census.attribute_with(records, &mut self.smoothing, now, |pid| {
            records.process(pid).map_or(
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                |record| record.native_cpu.get(),
            )
        });
        census.groups(records, &attributed, ScannerHome::Unavailable, capture)
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
                cpu:       HashMap::new(),
                compilers: HashMap::new(),
            },
            ScannerHome::Known(Path::new("/writer")),
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

    use super::*;
    use crate::app::App;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::KernelObservation;
    use crate::birth_stamp::Observation;
    use crate::constants::CAPTURE_ASSOCIATION_AMBIGUOUS;
    use crate::constants::CAPTURE_ASSOCIATION_CONFIRMED;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::COMPILER_COLUMN;
    use crate::constants::CPU_COLUMN;
    use crate::constants::MANAGED_COLUMN;
    use crate::constants::PID_SEPARATOR;
    use crate::constants::RUN_LOG_PREFIX;
    use crate::constants::RUN_LOG_SUFFIX;
    use crate::constants::TABLE_CELL;
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
            compilers: HashMap::new(),
            cpu:       HashMap::new(),
        }
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
            row.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            row.managed,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(row.compiler, CompilerObservation::Unknown);
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
        roster.observe(first, now);
        let ids = roster.tiled_ids(&[]);
        roster.observe(second, now + invocation_cpu_accounting::poll());
        assert_eq!(roster.tiled_ids(&[]), ids);
        assert_eq!(roster.groups().len(), 1);
        for mut census in [census_of(&[]), census_of(&[(pid.as_u32(), 10)])] {
            census.cargo.push(pid);
            assert!(
                CensusSequence::assemble(&mut census, &system, &capture, &["build".into()])
                    .is_empty()
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
        assert_eq!(row.provenance, RowProvenance::Uncaptured);
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
            recovered_settings.contains(CAPTURE_ASSOCIATION_CONFIRMED),
            "{recovered_settings}"
        );
        assert!(
            !recovered_settings.contains(CAPTURE_ASSOCIATION_AMBIGUOUS),
            "{recovered_settings}"
        );
        assert!(
            recovered_settings.contains("10.first"),
            "{recovered_settings}"
        );
        assert!(
            !recovered_settings.contains("10.second"),
            "{recovered_settings}"
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
        assert!(rejected.associations.is_empty());
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
            assert!(groups.is_empty());
            assert!(capture.confirmed().is_empty());
        } else {
            assert_eq!(groups.len(), 1);
            assert!(groups[0].rest.is_empty());
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
            .rows
            .into_iter()
            .map(|row| row.value)
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
        assert!(settings.contains(": 10.generation)"), "{settings}");
        assert!(!settings.contains("(10.generation; "), "{settings}");
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
        assert_eq!(rows[0].provenance, RowProvenance::Uncaptured);
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
        assert!(
            CensusSequence::assemble(
                &mut census_of(&[]),
                &ProcessObservations::default(),
                capture,
                &[],
            )
            .is_empty()
        );
    }

    fn account_settings_text(app: &App) -> String {
        crate::settings::rows(app)
            .rows
            .into_iter()
            .find(|row| row.label == "account 1")
            .expect("scanned account reaches Settings")
            .value
    }

    /// A scanned registration renders one qualified row with unavailable process measurements.
    fn assert_registration_display(groups: &[CargoGroup], capture: &Capture) -> String {
        assert_eq!(groups.len(), 1);
        assert!(groups[0].rest.is_empty());
        let row = &groups[0].lead;
        assert_eq!(row.pid, 10);
        assert_eq!(row.command, CommandText::of("cargo", &["build"]));
        assert_eq!(row.path, "~/project");
        assert_eq!(
            row.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(row.compiler, CompilerObservation::Unknown);
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
        for column in [CPU_COLUMN, COMPILER_COLUMN, MANAGED_COLUMN] {
            let start = header
                .find(TABLE_HEADERS[column])
                .expect("measurement column");
            let end = TABLE_HEADERS[column + 1..]
                .iter()
                .filter_map(|label| header.find(label))
                .min()
                .unwrap_or(rendered.len());
            assert_eq!(
                rendered[start..end].trim(),
                UNAVAILABLE_MEASUREMENT,
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
        assert!(groups[0].rest.is_empty());
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
        roster.observe(vec![group.clone()], Instant::now());
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
            assert!(scan[0].rest.is_empty());
            assert_eq!(scan[0].id(), identity);
            assert_eq!(scan[0].lead.pid, pid);
            assert_eq!(
                scan[0].lead.state,
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
            if index == 1 {
                assert_eq!(scan[0].lead.cpu, Measurement::Reading("42%".into()));
                assert_eq!(scan[0].lead.managed, Measurement::Reading(0));
                assert_eq!(scan[0].lead.compiler, CompilerObservation::None);
            } else {
                assert_eq!(
                    scan[0].lead.cpu,
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                );
                assert_eq!(
                    scan[0].lead.managed,
                    Measurement::Unavailable(MeasurementAbsence::Unproven)
                );
                assert_eq!(scan[0].lead.compiler, CompilerObservation::Unknown);
            }
            roster.observe(scan.clone(), Instant::now());
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
        assert!(census.cargo.is_empty());
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
            groups[0].lead.cpu,
            Measurement::Unavailable(MeasurementAbsence::Unproven)
        );
        assert_eq!(groups[0].lead.subtree_cpu, groups[0].lead.cpu);
        let unobserved = CensusSequence::default().sample(&system);
        assert_eq!(unobserved[0].lead.cpu, groups[0].lead.cpu);
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
            let command = format!(
                "{} {}",
                row.command.program,
                row.command.line(crate::render::SummaryDetail::Full)
            );
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
        assert!(groups[0].rest.is_empty());
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
        let settings = account_settings_text(&app);
        assert!(
            settings.contains("capture association: pid 11 via registration 10"),
            "{settings}"
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
        assert!(census.cargo.is_empty());
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
            rowless_cargo:            Vec::new(),
            compilers:                Vec::new(),
            cpu:                      HashMap::new(),
            accumulated:              HashMap::new(),
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
            subtree_cpu:        Measurement::Reading("0%".to_owned()),
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
        let mut smoothing = InvocationCpuAccounting::default();
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
            aggregate_cpu(&shares, members).map(cpu_label).to_string(),
            "391%"
        );
    }

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
            HashMap::from([(compiler, requester)])
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
                census.detached_compilers(&system, &targets).is_empty(),
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
            samples[0][0].lead.cpu,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert_eq!(samples[1][0].lead.cpu, Measurement::Reading("20%".into()));
        assert_eq!(samples[0][0].id(), samples[1][0].id());
        assert_ne!(samples[1][0].id(), samples[2][0].id());
        assert_eq!(
            samples[2][0].lead.cpu,
            Measurement::Unavailable(MeasurementAbsence::FirstObservation)
        );
        assert!(
            sequence
                .sample_counters(
                    &mut ProcessObservations::default(),
                    now + Duration::from_secs(3)
                )
                .is_empty()
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
