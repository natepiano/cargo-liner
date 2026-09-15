//! Cumulative invocation measurements and retained compiler ownership.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(target_os = "linux")]
use std::fs;
use std::ops::Add;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
#[cfg(target_os = "linux")]
use std::sync::OnceLock;
use std::time::Duration;
use std::time::Instant;

use sysinfo::Pid;
use sysinfo::Process;

use super::command_text;
use super::process_identity::InvocationId;
use super::process_identity::ProcessIdentities;
use super::process_identity::ProcessIdentity;
#[cfg(test)]
use super::scan;
use super::scan::Census;
use super::scan::CompilerObservation;
use super::scan::ProcessObservation;
use super::scan::WorkingDirectoryObservation;
use crate::birth_stamp;
#[cfg(target_os = "linux")]
use crate::birth_stamp::BirthStamp;
#[cfg(target_os = "linux")]
use crate::birth_stamp::KernelObservation;
use crate::birth_stamp::LifetimeEvidence;
#[cfg(test)]
use crate::birth_stamp::ProcessLifetime;
#[cfg(target_os = "linux")]
use crate::birth_stamp::Verification;
use crate::constants::ARGUMENT_SEPARATOR;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_BOOT_ID_PATH;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_PROC_DIRECTORY;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_COMM_END;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_FILENAME;
#[cfg(target_os = "linux")]
use crate::constants::BIRTH_STAT_START_INDEX;
use crate::constants::CARGO_TARGET_DIR_ENV;
use crate::constants::CARGO_TARGET_DIR_FLAG;
#[cfg(target_os = "linux")]
use crate::constants::CPU_AUXV_CLOCK_TICKS;
#[cfg(target_os = "linux")]
use crate::constants::CPU_AUXV_ENTRY_WORDS;
#[cfg(target_os = "linux")]
use crate::constants::CPU_AUXV_PATH;
use crate::constants::CPU_REPORT_MILLIS;
use crate::constants::CPU_SMOOTHING_SECONDS;
#[cfg(target_os = "linux")]
use crate::constants::CPU_STAT_TIME_FIELDS;
#[cfg(target_os = "linux")]
use crate::constants::CPU_STAT_TIME_INDEX;
use crate::constants::PROCESS_POLL_MILLIS;
use crate::constants::UNAVAILABLE_MEASUREMENT;

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

/// What the census worked out per cargo invocation, once every process
/// under one has been walked up to it.
/// Buckets are disjoint: nested cargo retains its own contribution, never folded into its parent.
pub(super) struct InvocationMeasurements {
    /// Every invocation has an observation, including one compiling nothing.
    pub(super) compilers: HashMap<Pid, CompilerObservation>,
    /// Settled invocation work, unavailable only when its own sample is unestablished.
    pub(super) cpu:       HashMap<Pid, Measurement<f32>>,
}

/// Evidence retained across a refresh to establish identity and monotonic CPU time.
#[derive(Clone)]
pub(super) struct CpuBaseline {
    /// A reused pid must begin a fresh rate observation.
    pub(super) lifetime:    LifetimeEvidence,
    /// A decrease for the same process proves the samples cannot form a valid rate.
    pub(super) accumulated: u64,
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
pub(super) struct InvocationCpuAccounting {
    /// Native lifetimes preserve the first confirmed generation through registration gaps.
    pub(super) owners:       HashMap<ProcessIdentity, InvocationId>,
    /// Accumulated invocation work survives descendant exits between scans.
    pub(super) invocations:  HashMap<InvocationId, InvocationCpuHistory>,
    /// Observed client destinations remain usable while their invocation lives.
    pub(super) targets:      HashMap<InvocationId, HashSet<PathBuf>>,
    /// A compiler lifetime can never donate its accumulated history to another invocation.
    pub(super) cache_owners: HashMap<ProcessIdentity, InvocationId>,
    /// Unavailable lifetime reads retain rows only during continuous pid presence.
    pub(super) identities:   ProcessIdentities,
    /// Bind the pre-refresh counters to the lifetime observed with their last sample.
    pub(super) observed:     HashMap<Pid, LifetimeEvidence>,
    /// Where each invocation's reading has settled, moved on every scan.
    /// Keyed by invocation identity, so a replaced pid cannot inherit old readings.
    pub(super) settled:      HashMap<InvocationId, Measurement<f32>>,
    /// What the table is carrying, taken from
    /// [`settled`](Self::settled) when a reading falls due.
    pub(super) reported:     HashMap<InvocationId, Measurement<f32>>,
    /// A never-published smoother has no previous snapshot to hold.
    publication:             CpuPublication,
}

/// A cumulative subtree counter, retaining completed work on platforms without wait totals.
#[derive(Default)]
pub(super) struct RetainedSubtreeCpuTime {
    /// Linux transfers exited children into their reaper's counter; scans can overlap that
    /// transfer.
    pub(super) total:   Duration,
    /// Darwin cannot read another task's reaped-child time, so retain observed contributions.
    #[cfg(not(target_os = "linux"))]
    pub(super) present: HashMap<ProcessIdentity, Duration>,
}

impl RetainedSubtreeCpuTime {
    /// Account for the current tree once, including work retained after a process exits.
    pub(super) fn update(&mut self, current: &HashMap<ProcessIdentity, Duration>) -> Duration {
        #[cfg(target_os = "linux")]
        {
            self.total = self.total.max(current.values().sum());
        }
        #[cfg(not(target_os = "linux"))]
        {
            for (identity, elapsed) in current {
                let retained = self.present.entry(identity.clone()).or_default();
                *retained = (*retained).max(*elapsed);
            }
            self.total = self.present.values().sum::<Duration>();
        }
        self.total
    }
}

/// A previous invocation-wide counter has a sampling time independent of publication.
#[derive(Default)]
enum InvocationCpuBaseline {
    /// A rate needs two observations of this invocation.
    #[default]
    AwaitingFirstSample,
    /// Accumulated processor time and the instant that counter was sampled.
    Established {
        accumulated: Duration,
        at:          Instant,
    },
}

/// Work owned by one invocation, including compiler trees outside its ancestry.
#[derive(Default)]
pub(super) struct InvocationCpuHistory {
    /// Incomplete tree reads cannot move the invocation's cumulative counter backward.
    pub(super) accumulated: Duration,
    /// Direct descendants transfer their time into the invocation on Linux.
    pub(super) tree:        RetainedSubtreeCpuTime,
    /// Each independently executing compiler keeps its own cumulative tree.
    pub(super) detached:    HashMap<ProcessIdentity, RetainedSubtreeCpuTime>,
    /// Nested cargo time stays excluded after Linux transfers it into the manager's wait totals.
    #[cfg(target_os = "linux")]
    pub(super) nested:      HashMap<InvocationId, Duration>,
    /// Consecutive whole-invocation samples establish the reported rate.
    sample:                 InvocationCpuBaseline,
}

impl InvocationCpuHistory {
    /// New and idle descendants contribute counters, without contributing absence reasons.
    pub(super) fn measure(
        &mut self,
        work: InvocationCpuContributions,
        evidence: Measurement<f32>,
        now: Instant,
    ) -> Measurement<f32> {
        for (identity, current) in work.detached {
            self.detached.entry(identity).or_default().update(&current);
        }
        let tree = self.tree.update(&work.tree);
        #[cfg(target_os = "linux")]
        let tree = {
            for (identity, elapsed) in work.nested {
                let retained = self.nested.entry(identity).or_default();
                *retained = (*retained).max(elapsed);
            }
            tree.saturating_sub(self.nested.values().sum())
        };
        self.accumulated = self.accumulated.max(
            tree + self
                .detached
                .values()
                .map(|counter| counter.total)
                .sum::<Duration>(),
        );
        let accumulated = self.accumulated;
        if let Measurement::Unavailable(reason) = evidence {
            return Measurement::Unavailable(reason);
        }
        let previous = std::mem::replace(
            &mut self.sample,
            InvocationCpuBaseline::Established {
                accumulated,
                at: now,
            },
        );
        match previous {
            InvocationCpuBaseline::AwaitingFirstSample => {
                Measurement::Unavailable(MeasurementAbsence::FirstObservation)
            },
            InvocationCpuBaseline::Established {
                accumulated: previous,
                at,
            } => {
                let elapsed = now.saturating_duration_since(at);
                if elapsed.is_zero() || accumulated < previous {
                    return Measurement::Unavailable(MeasurementAbsence::Unproven);
                }
                Measurement::Reading(
                    accumulated.saturating_sub(previous).as_secs_f32() / elapsed.as_secs_f32()
                        * 100.0,
                )
            },
        }
    }
}

/// The CPU contributions collected for one invocation during one scan, split at cache-server
/// boundaries.
#[derive(Default)]
pub(super) struct InvocationCpuContributions {
    /// Processes owned through cargo ancestry.
    pub(super) tree:     HashMap<ProcessIdentity, Duration>,
    /// Compiler roots and their descendants owned through output-directory evidence.
    pub(super) detached: HashMap<ProcessIdentity, HashMap<ProcessIdentity, Duration>>,
    /// Full nested-cargo counters are excluded even after their process disappears.
    #[cfg(target_os = "linux")]
    pub(super) nested:   HashMap<InvocationId, Duration>,
}

/// Output evidence either singles out a cargo invocation or authorizes no charge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompileOwner {
    /// Exactly one invocation has a matching target directory.
    Unique(Pid),
    /// No invocation has a proven matching destination.
    Unknown,
    /// Several invocation targets contain this destination.
    Ambiguous,
}

/// Ancestry wins over output evidence when assigning one process's time.
#[derive(Clone, Copy)]
pub(super) enum CpuAssignment {
    /// The nearest cargo receives the ordinary descendant contribution.
    Direct(Pid),
    /// A cache compiler and its children contribute through the compiler's lifetime.
    Detached { owner: Pid, compiler: Pid },
    /// No observed invocation can be charged for this process.
    Unassigned,
}

/// A compiler's first owner is forgotten only when its lifetime cannot return.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CompilerCreditRetention {
    /// A live process or an inconclusive observation preserves exclusive ownership.
    Keep,
    /// Native replacement or kernel-confirmed absence ends that lifetime's credit.
    Retire,
}

/// Command metadata must name a resolvable directory before it can establish ownership.
#[derive(Clone, Copy, Debug)]
pub(super) enum CompilePathAbsence {
    /// The command has no value for the requested path argument.
    Unspecified,
    /// Missing metadata or a failed path resolution prevents comparison.
    Unavailable,
}

/// Repeated evidence for one pid is still one owner; competing pids authorize none.
pub(super) fn compile_owner(owners: impl Iterator<Item = Pid>) -> CompileOwner {
    owners.fold(CompileOwner::Unknown, |owner, pid| match owner {
        CompileOwner::Unknown => CompileOwner::Unique(pid),
        CompileOwner::Unique(previous) if previous == pid => owner,
        CompileOwner::Unique(_) | CompileOwner::Ambiguous => CompileOwner::Ambiguous,
    })
}

/// Recognize split and equals forms without converting path bytes through UTF-8.
fn argument_path(argv: &[OsString], flag: &str) -> Result<PathBuf, CompilePathAbsence> {
    let mut arguments = argv.iter();
    let mut path = Err(CompilePathAbsence::Unspecified);
    while let Some(argument) = arguments.next() {
        if argument == ARGUMENT_SEPARATOR {
            break;
        }
        if argument == flag {
            path = arguments
                .next()
                .map(PathBuf::from)
                .ok_or(CompilePathAbsence::Unavailable);
        } else if let Some(value) = argument
            .as_bytes()
            .strip_prefix(flag.as_bytes())
            .and_then(|suffix| suffix.strip_prefix(b"="))
        {
            path = Ok(PathBuf::from(OsStr::from_bytes(value)));
        }
    }
    path
}

/// Canonical paths compare directory components and resolve relative paths in the owner.
fn process_path(
    process: &ProcessObservation<'_>,
    path: &Path,
) -> Result<PathBuf, CompilePathAbsence> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        match process.cwd() {
            WorkingDirectoryObservation::Observed(directory) => directory.join(path),
            WorkingDirectoryObservation::Unavailable => {
                return Err(CompilePathAbsence::Unavailable);
            },
        }
    };
    path.canonicalize()
        .map_err(|_| CompilePathAbsence::Unavailable)
}

/// Compiler clients and rustc use the same output flag spelling.
pub(super) fn process_argument_path(
    process: &ProcessObservation<'_>,
    flag: &str,
) -> Result<PathBuf, CompilePathAbsence> {
    process_path(process, &argument_path(process.cmd(), flag)?)
}

/// Explicit target paths win over the environment. Defaults and Cargo configuration
/// are learned from observed compiler clients, whose output paths reflect both.
pub(super) fn cargo_target_directory(
    process: &ProcessObservation<'_>,
) -> Result<PathBuf, CompilePathAbsence> {
    let arguments =
        command_text::cargo_split(process.cmd()).map_err(|_| CompilePathAbsence::Unavailable)?;
    let argv = &process.cmd()[arguments.start..];
    match argument_path(argv, CARGO_TARGET_DIR_FLAG) {
        Ok(path) => return process_path(process, &path),
        Err(CompilePathAbsence::Unavailable) => return Err(CompilePathAbsence::Unavailable),
        Err(CompilePathAbsence::Unspecified) => {},
    }
    for variable in process.environ() {
        if let Some(path) = variable
            .as_bytes()
            .strip_prefix(CARGO_TARGET_DIR_ENV.as_bytes())
            .and_then(|suffix| suffix.strip_prefix(b"="))
        {
            return process_path(process, Path::new(OsStr::from_bytes(path)));
        }
    }
    Err(CompilePathAbsence::Unspecified)
}

/// The kernel supplies the stat clock frequency without a subprocess or unsafe call.
#[cfg(target_os = "linux")]
fn linux_clock_ticks() -> Result<u32, MeasurementAbsence> {
    static TICKS: OnceLock<Result<u32, MeasurementAbsence>> = OnceLock::new();
    *TICKS.get_or_init(|| {
        let bytes = fs::read(CPU_AUXV_PATH).map_err(|_| MeasurementAbsence::ReadFailed)?;
        let word = std::mem::size_of::<usize>();
        for entry in bytes.chunks_exact(word * CPU_AUXV_ENTRY_WORDS) {
            let tag = usize::from_ne_bytes(
                entry[..word]
                    .try_into()
                    .map_err(|_| MeasurementAbsence::ReadFailed)?,
            );
            if tag == CPU_AUXV_CLOCK_TICKS {
                let ticks = usize::from_ne_bytes(
                    entry[word..]
                        .try_into()
                        .map_err(|_| MeasurementAbsence::ReadFailed)?,
                );
                return u32::try_from(ticks)
                    .ok()
                    .filter(|&ticks| ticks > 0)
                    .ok_or(MeasurementAbsence::ReadFailed);
            }
        }
        Err(MeasurementAbsence::ReadFailed)
    })
}

/// CPU counters and their native birth counter come from the same stat buffer.
#[cfg(target_os = "linux")]
#[derive(Debug, Eq, PartialEq)]
struct LinuxCpuSample {
    /// Field 22 binds the accumulated ticks to this process lifetime.
    start_ticks:       u64,
    /// Fields 14 through 17 include both own and reaped-child processor time.
    accumulated_ticks: u64,
}

#[cfg(target_os = "linux")]
impl LinuxCpuSample {
    /// The normal kernel comparison rejects a replaced pid or an unverified sample.
    pub(super) fn verify(
        &self,
        pid: u32,
        boot: &str,
        observation: &KernelObservation,
    ) -> Result<(), MeasurementAbsence> {
        let identity = BirthStamp::from_fields(boot, &self.start_ticks.to_string());
        match observation.compare(pid, &identity) {
            Verification::Confirmed => Ok(()),
            Verification::Ended | Verification::Unknown => Err(MeasurementAbsence::Unproven),
        }
    }
}

/// Parse the four CPU counters and start ticks after the final comm delimiter.
#[cfg(target_os = "linux")]
fn linux_cpu_ticks(stat: &[u8]) -> Result<LinuxCpuSample, MeasurementAbsence> {
    let end = stat
        .iter()
        .rposition(|byte| *byte == BIRTH_STAT_COMM_END)
        .ok_or(MeasurementAbsence::ReadFailed)?;
    let mut fields = stat[end + 1..]
        .split(u8::is_ascii_whitespace)
        .filter(|field| !field.is_empty())
        .skip(CPU_STAT_TIME_INDEX);
    let mut ticks = 0_u64;
    for _ in 0..CPU_STAT_TIME_FIELDS {
        let field = fields.next().ok_or(MeasurementAbsence::ReadFailed)?;
        let value = std::str::from_utf8(field)
            .map_err(|_| MeasurementAbsence::ReadFailed)?
            .parse::<u64>()
            .map_err(|_| MeasurementAbsence::ReadFailed)?;
        ticks = ticks
            .checked_add(value)
            .ok_or(MeasurementAbsence::ReadFailed)?;
    }
    let field = fields
        .nth(BIRTH_STAT_START_INDEX - CPU_STAT_TIME_INDEX - CPU_STAT_TIME_FIELDS)
        .ok_or(MeasurementAbsence::ReadFailed)?;
    let start_ticks = std::str::from_utf8(field)
        .map_err(|_| MeasurementAbsence::ReadFailed)?
        .parse::<u64>()
        .map_err(|_| MeasurementAbsence::ReadFailed)?;
    Ok(LinuxCpuSample {
        start_ticks,
        accumulated_ticks: ticks,
    })
}

/// Read live task time plus the time transferred by wait into its child counters.
#[cfg(target_os = "linux")]
pub(super) fn linux_cpu_time(pid: Pid) -> Result<Duration, MeasurementAbsence> {
    static BOOT: OnceLock<Result<String, MeasurementAbsence>> = OnceLock::new();
    let path = Path::new(BIRTH_PROC_DIRECTORY)
        .join(pid.to_string())
        .join(BIRTH_STAT_FILENAME);
    let stat = fs::read(path).map_err(|_| MeasurementAbsence::ReadFailed)?;
    let sample = linux_cpu_ticks(&stat)?;
    let boot = BOOT
        .get_or_init(|| {
            fs::read_to_string(BIRTH_BOOT_ID_PATH).map_err(|_| MeasurementAbsence::ReadFailed)
        })
        .as_ref()
        .map_err(|reason| *reason)?;
    sample.verify(pid.as_u32(), boot, &birth_stamp::observe(pid.as_u32()))?;
    Ok(Duration::from_secs(sample.accumulated_ticks) / linux_clock_ticks()?)
}

impl InvocationCpuAccounting {
    /// Only recovery for a proven process lifetime may rename accumulated CPU ownership.
    /// A different confirmed generation starts fresh even when pid and birth still match.
    pub(super) fn identify_owners(&mut self, census: &Census) {
        let mut present = HashSet::new();
        for pid in census.live_cargo() {
            let (Ok(process), Some(identity)) =
                (census.cpu_identity(pid), census.identities.get(&pid))
            else {
                continue;
            };
            present.insert(process.clone());
            let owner = self
                .owners
                .entry(process)
                .or_insert_with(|| identity.clone());
            if let InvocationId::Captured(_) = identity {
                let previous = std::mem::replace(owner, identity.clone());
                if matches!(previous, InvocationId::Process(_)) {
                    self.reidentify_owner(&previous, identity);
                }
            }
        }
        self.owners.retain(|process, _| present.contains(process));
    }

    /// Move the original baseline and compiler credits together without adding prior time.
    fn reidentify_owner(&mut self, previous: &InvocationId, recovered: &InvocationId) {
        if let Some(cpu) = self.invocations.remove(previous) {
            self.invocations.entry(recovered.clone()).or_insert(cpu);
        }
        if let Some(targets) = self.targets.remove(previous) {
            self.targets
                .entry(recovered.clone())
                .or_default()
                .extend(targets);
        }
        for owner in self.cache_owners.values_mut() {
            if owner == previous {
                owner.clone_from(recovered);
            }
        }
        #[cfg(target_os = "linux")]
        for cpu in self.invocations.values_mut() {
            if let Some(elapsed) = cpu.nested.remove(previous) {
                let retained = cpu.nested.entry(recovered.clone()).or_default();
                *retained = (*retained).max(elapsed);
            }
        }
    }

    /// Carry every invocation's reading toward what this scan sampled,
    /// and hand back what the table should show at `now`.
    ///
    /// An unavailable sample immediately replaces any published reading,
    /// even between reporting deadlines. Recovery starts at its own value
    /// because a sample gap cannot contribute to a smoothed rate.
    pub(super) fn settle(
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
    pub(super) fn is_due(&self, now: Instant) -> bool {
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

#[cfg(test)]
pub(super) use tests::cpu_process_identity;
#[cfg(test)]
pub(super) use tests::cpu_work;
#[cfg(test)]
pub(super) use tests::poll;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use crate::birth_stamp::IdentityEvidence;
    #[cfg(target_os = "linux")]
    use crate::birth_stamp::KernelObservation;
    #[cfg(target_os = "linux")]
    use crate::birth_stamp::Observation;

    #[test]
    fn new_and_idle_descendants_keep_the_invocations_cpu_measurable() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        cpu.measure(cpu_work(&[(1, 10)]), Measurement::Reading(0.0), now);
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 10), (2, 100), (3, 0)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(1)
            ),
            Measurement::Reading(10.0),
        );
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 10), (2, 100), (4, 0)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(2)
            ),
            Measurement::Reading(0.0),
        );
    }

    pub fn cpu_work(counters: &[(u32, u64)]) -> InvocationCpuContributions {
        InvocationCpuContributions {
            tree: counters
                .iter()
                .map(|&(pid, elapsed)| (cpu_process_identity(pid), Duration::from_millis(elapsed)))
                .collect(),
            ..InvocationCpuContributions::default()
        }
    }

    pub fn cpu_process_identity(pid: u32) -> ProcessIdentity {
        ProcessIdentity::Known {
            pid,
            lifetime: ProcessLifetime::for_test(u64::from(pid)),
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reaped_descendants_transfer_time_without_repeating_it() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        cpu.measure(
            cpu_work(&[(1, 10), (2, 100)]),
            Measurement::Reading(0.0),
            now,
        );
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 110)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(1)
            ),
            Measurement::Reading(0.0)
        );
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 210)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(2)
            ),
            Measurement::Reading(10.0)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reaped_nested_cargo_keeps_its_prior_cpu_credit() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        let mut first = cpu_work(&[(1, 10), (2, 100)]);
        first
            .nested
            .insert(InvocationId::for_test(2), Duration::from_millis(100));
        cpu.measure(first, Measurement::Reading(0.0), now);
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 110)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(1)
            ),
            Measurement::Reading(0.0)
        );
    }

    #[test]
    fn detached_compiler_observation_gaps_do_not_repeat_its_counter() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        let compiler = ProcessIdentity::Known {
            pid:      2,
            lifetime: ProcessLifetime::for_test(2),
        };
        let mut first = cpu_work(&[(1, 10)]);
        first
            .detached
            .insert(compiler.clone(), cpu_work(&[(2, 100)]).tree);
        cpu.measure(first, Measurement::Reading(0.0), now);
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 10)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(1)
            ),
            Measurement::Reading(0.0)
        );
        let mut recovered = cpu_work(&[(1, 10)]);
        recovered
            .detached
            .insert(compiler, cpu_work(&[(2, 200)]).tree);
        assert_eq!(
            cpu.measure(
                recovered,
                Measurement::Reading(0.0),
                now + Duration::from_secs(2)
            ),
            Measurement::Reading(10.0)
        );
    }

    #[test]
    fn compiler_time_cannot_replace_the_invocations_own_counter_evidence() {
        for reason in [
            MeasurementAbsence::FirstObservation,
            MeasurementAbsence::ReadFailed,
            MeasurementAbsence::Unproven,
        ] {
            let mut cpu = InvocationCpuHistory::default();
            let now = Instant::now();
            cpu.measure(cpu_work(&[(1, 10)]), Measurement::Reading(0.0), now);
            assert_eq!(
                cpu.measure(
                    cpu_work(&[(1, 10), (2, 100)]),
                    Measurement::Unavailable(reason),
                    now + Duration::from_secs(1)
                ),
                Measurement::Unavailable(reason)
            );
        }
    }

    #[test]
    fn failed_own_counter_preserves_the_full_elapsed_recovery_interval() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        cpu.measure(cpu_work(&[(1, 100)]), Measurement::Reading(0.0), now);
        assert_eq!(
            cpu.measure(
                InvocationCpuContributions::default(),
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                now + Duration::from_secs(1)
            ),
            Measurement::Unavailable(MeasurementAbsence::ReadFailed),
        );
        assert_eq!(
            cpu.measure(
                cpu_work(&[(1, 300)]),
                Measurement::Reading(0.0),
                now + Duration::from_secs(2)
            ),
            Measurement::Reading(10.0),
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn partial_tree_reads_do_not_restart_the_invocation_counter() {
        let mut cpu = InvocationCpuHistory::default();
        let now = Instant::now();
        let nested = InvocationId::for_test(2);
        let mut first = cpu_work(&[(1, 10), (2, 100)]);
        first
            .nested
            .insert(nested.clone(), Duration::from_millis(100));
        cpu.measure(first, Measurement::Reading(0.0), now);
        let mut incomplete = cpu_work(&[(2, 200)]);
        incomplete
            .nested
            .insert(nested.clone(), Duration::from_millis(200));
        assert_eq!(
            cpu.measure(
                incomplete,
                Measurement::Unavailable(MeasurementAbsence::ReadFailed),
                now + Duration::from_secs(1)
            ),
            Measurement::Unavailable(MeasurementAbsence::ReadFailed)
        );
        let mut recovered = cpu_work(&[(1, 10), (2, 200)]);
        recovered.nested.insert(nested, Duration::from_millis(200));
        assert_eq!(
            cpu.measure(
                recovered,
                Measurement::Reading(0.0),
                now + Duration::from_secs(2)
            ),
            Measurement::Reading(0.0)
        );
    }

    #[test]
    fn competing_compile_owners_authorize_no_charge() {
        assert_eq!(
            compile_owner([Pid::from_u32(1), Pid::from_u32(1)].into_iter()),
            CompileOwner::Unique(Pid::from_u32(1))
        );
        assert_eq!(
            compile_owner([Pid::from_u32(1), Pid::from_u32(2)].into_iter()),
            CompileOwner::Ambiguous
        );
        assert_eq!(compile_owner(std::iter::empty()), CompileOwner::Unknown);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_cpu_stat_includes_reaped_time_and_ignores_comm_delimiters() {
        let mut stat = b"123 (space ) (\n\xff)".to_vec();
        for _ in 0..CPU_STAT_TIME_INDEX {
            stat.extend_from_slice(b" 0");
        }
        stat.extend_from_slice(b" 11 13 17 19");
        for _ in CPU_STAT_TIME_INDEX + CPU_STAT_TIME_FIELDS..BIRTH_STAT_START_INDEX {
            stat.extend_from_slice(b" 0");
        }
        assert_eq!(linux_cpu_ticks(&stat), Err(MeasurementAbsence::ReadFailed));
        stat.extend_from_slice(b" 123456789");
        assert_eq!(
            linux_cpu_ticks(&stat),
            Ok(LinuxCpuSample {
                start_ticks:       123_456_789,
                accumulated_ticks: 60,
            })
        );
        assert_eq!(
            linux_cpu_ticks(b"1 (short) S"),
            Err(MeasurementAbsence::ReadFailed)
        );
        assert!(linux_clock_ticks().is_ok());
        assert!(linux_cpu_time(Pid::from_u32(std::process::id())).is_ok());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cpu_stat_birth_rejects_pid_reuse_and_unbound_observations() {
        let sample = LinuxCpuSample {
            start_ticks:       11,
            accumulated_ticks: 60,
        };
        let IdentityEvidence::Available(birth) = BirthStamp::from_fields("boot", "11") else {
            panic!("fixture birth must parse");
        };
        let matching = KernelObservation::for_test(123, Observation::Present(birth));
        assert_eq!(sample.verify(123, "boot", &matching), Ok(()));
        assert_eq!(
            sample.verify(124, "boot", &matching),
            Err(MeasurementAbsence::Unproven)
        );
        let IdentityEvidence::Available(replacement) = BirthStamp::from_fields("boot", "12") else {
            panic!("replacement birth must parse");
        };
        let replaced = KernelObservation::for_test(123, Observation::Present(replacement));
        assert_eq!(
            sample.verify(123, "boot", &replaced),
            Err(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            sample.verify(123, "other-boot", &matching),
            Err(MeasurementAbsence::Unproven)
        );
        assert_eq!(
            sample.verify(
                123,
                "boot",
                &KernelObservation::for_test(123, Observation::Unknown)
            ),
            Err(MeasurementAbsence::Unproven)
        );
    }

    /// A clock the settling tests step forward by hand.
    fn start() -> Instant { Instant::now() }

    /// One scan's worth of the poll interval.
    pub fn poll() -> Duration { Duration::from_millis(PROCESS_POLL_MILLIS) }

    /// One invocation's reading once `sampled` has been folded in at
    /// `now`.
    fn settle_one(smoothing: &mut InvocationCpuAccounting, sampled: f32, now: Instant) -> f32 {
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
        smoothing: &mut InvocationCpuAccounting,
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
        let opening = settle_one(&mut InvocationCpuAccounting::default(), 400.0, start());

        assert!(
            (opening - 400.0).abs() < f32::EPSILON,
            "opened at {opening} rather than at its own sample"
        );
    }

    /// A burst lands as a step toward itself, not as the whole of it:
    /// one scan of a command that works in bursts is mostly artefact.
    #[test]
    fn a_sample_that_jumps_is_taken_a_step_at_a_time() {
        let mut smoothing = InvocationCpuAccounting::default();
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
        let mut smoothing = InvocationCpuAccounting::default();
        let now = start();
        settle_one(&mut smoothing, 0.0, now);

        // Four windows of the climb, by which point a reading settled
        // this way stands within two percent of what it is climbing to.
        let over = Duration::from_secs_f32(CPU_SMOOTHING_SECONDS * 4.0);

        assert_eq!(
            scan::cpu_label(settle_over(&mut smoothing, 100.0, now, over)),
            "98%"
        );
    }

    /// The reading behind the column moves on every scan; the column
    /// itself is only allowed to say something new once a second, so a
    /// smooth figure is not redrawn faster than it can be read.
    #[test]
    fn the_table_holds_a_reading_for_the_whole_report_interval() {
        let mut smoothing = InvocationCpuAccounting::default();
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
        let mut smoothing = InvocationCpuAccounting::default();
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
        let mut smoothing = InvocationCpuAccounting::default();
        let now = start();
        settle_one(&mut smoothing, 400.0, now);
        smoothing.settle(&HashMap::new(), &[], now + poll());

        assert!(smoothing.settled.is_empty());
        assert!(smoothing.reported.is_empty());
    }

    #[test]
    fn a_never_published_smoother_is_distinct_from_one_holding_a_reading() {
        let mut smoothing = InvocationCpuAccounting::default();
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
            let mut smoothing = InvocationCpuAccounting::default();
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
        let mut smoothing = InvocationCpuAccounting::default();
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
        let mut smoothing = InvocationCpuAccounting::default();
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
}
