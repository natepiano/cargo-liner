//! How far a build has got, read out of the output the cargo shim
//! captured for it.
//!
//! Cargo already counts the work: while it compiles it draws
//! `Building [====>    ] 149/403: serde, regex` on stderr, and those two
//! numbers are the unit graph's completed and total counts. Nothing here
//! estimates anything -- the display shows cargo's own arithmetic.
//!
//! Reaching it takes a capture, because the invocations in the grid
//! belong to other terminals: [`crate::processes`] finds them by
//! scanning the process table, and a process's stdout is not readable
//! from outside it. The shim installed at `~/.rustup/toolchains/*/bin/cargo`
//! is what closes that gap. It runs each command under a pty, mirrors
//! the output into `<root>/run-<generation>-<pid>.log`, and registers the
//! run as `<root>/state/pids/<pid>.<generation>` for as long as it lives -- the pid
//! in both being the shim's own, which is an ancestor of the cargo
//! process the grid draws.
//!
//! A command that runs tests counts twice over. `cargo nextest run`
//! compiles first, under cargo's own bar, and then works through the
//! tests under a bar of its own -- `Running [ 00:00:03] ███▏ 22/24: 2
//! running, 22 passed` -- which is the same `done/total` pair, with
//! the drawn bar standing between the bracket and the counter rather
//! than in front of it. Both are read, and which of them
//! the reading came from is what the display names the phase by.
//!
//! A runner with nowhere to draw a bar still counts. Nextest draws one
//! only on a terminal, so a run started by a script or an agent -- and
//! that is most of the test runs on this machine -- has none, and the
//! count goes inline into every line it prints instead: `PASS [
//! 1.014s] (11/24) nxprobe t18`. That tally is the same reading in
//! parentheses, and reading it is what keeps those runs from sitting
//! at whatever the build last said for as long as the tests take.
//!
//! So a run reports progress when it was captured and reports none when
//! it was not, and the cell is drawn either way.
//!
//! A log says one other thing worth reading. Cargo locks the build
//! directory, so a second command against the same target waits instead
//! of failing -- it prints `Blocking waiting for file lock on build
//! directory` and then nothing, which from outside looks exactly like a
//! build that has not reached its first unit.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::env;
use std::ffi::OsString;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::birth_stamp;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::capture_root::CleanupRefusal;
use crate::capture_root::Enumeration;
use crate::capture_root::RootHistory;
use crate::capture_root::RootIncarnation;
use crate::capture_root::RootOwner;
use crate::capture_root::RootScan;
use crate::capture_root::SweepBudget;
use crate::capture_root::SweepDisposition;
use crate::constants::BAR_GLYPH_FIRST;
use crate::constants::BAR_GLYPH_LAST;
use crate::constants::BUILD_FINISHED_MARKER;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::CAPTURE_ROOT;
use crate::constants::CAPTURE_ROOT_ENV;
use crate::constants::CAPTURE_ROOT_NOT_ABSOLUTE;
use crate::constants::CONFIG_KEY_CAPTURE_ROOTS;
use crate::constants::LOCK_WAIT_MARKER;
use crate::constants::PHASE_BUILDING;
use crate::constants::PHASE_TESTING;
use crate::constants::PID_SEPARATOR;
use crate::constants::REGISTRATION_SEPARATOR;
use crate::constants::REGISTRATION_TEMP_SUFFIX;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::constants::TALLY_CLOSE;
use crate::constants::TALLY_OPEN;
use crate::constants::TEST_PHASE_MARKER;
use crate::constants::UNIT_COUNTER_LEAD;
use crate::constants::UNIT_COUNTER_SEPARATOR;
use crate::constants::UNIT_COUNTER_TRAILER;
use crate::processes::CaptureDiagnostic;
use crate::processes::RootReadStatus;
use crate::processes::RootStatus;
use crate::registration::Registration;
use crate::registration::RegistrationVerification;
use crate::registration::VerifiedRegistration;

/// Cargo's count of the work in front of it, as its progress bar reports
/// it: units finished out of units planned.
///
/// A unit is one compilation of one crate target, which is what the
/// build is actually made of -- not a package and not a source file. A
/// unit already fresh counts as finished the moment cargo checks it, so
/// an incremental build opens near its total rather than at zero.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Progress {
    /// Units cargo has finished.
    pub(crate) done:  usize,
    /// Units in the build plan.
    pub(crate) total: usize,
}

impl Progress {
    /// How far along, rounded down, so only a finished build reads 100.
    pub(crate) const fn percent(self) -> usize {
        // `total` is never zero: `parse_counter` rejects a counter that
        // would divide by it.
        self.done.saturating_mul(100) / self.total
    }

    /// The same reading in tenths of a percent, rounded down the same
    /// way, so only a finished build reaches 1000.
    pub(crate) const fn percent_tenths(self) -> usize {
        self.done.saturating_mul(1000) / self.total
    }
}

/// Which counter a reading came from, which is the whole of what the
/// numbers themselves say about what the run is doing.
///
/// A command that only ever compiles stays in one of these for its
/// whole life; `cargo nextest run` passes through both, and the two
/// counters are unrelated -- the second opens at nought over the tests
/// collected the moment the first reaches its total.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Phase {
    /// Cargo compiling the units of its build plan.
    Building,
    /// A test runner working through the tests it collected.
    Testing,
}

impl Phase {
    /// The word a working-directory header names this phase with.
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Building => PHASE_BUILDING,
            Self::Testing => PHASE_TESTING,
        }
    }
}

/// What a captured run was doing when its log was last read.
///
/// Cargo takes a lock on the build directory, so a second command in
/// the same target waits rather than fails. It says so once and then
/// prints nothing at all, which from outside is indistinguishable from
/// a build that has not reached its first unit -- the same pid, the
/// same climbing duration, and no reading either way. Reading the wait
/// out of the log is what separates them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RunState {
    /// Getting somewhere, as far along as the counter of the phase it
    /// is in says.
    Working {
        /// Which counter the reading came from.
        phase:    Phase,
        /// What that counter last said.
        progress: Progress,
    },
    /// Waiting for another cargo to give up the build directory.
    Blocked,
}

/// What a gauge can show, preserving the reason it has no counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CounterState {
    /// A phase owns the current numerator and denominator.
    Working {
        /// Counters from different phases must not be combined.
        phase:    Phase,
        /// The current progress in this phase.
        progress: Progress,
    },
    /// The capture reports a build-directory wait.
    Blocked,
    /// Readable output currently supplies no counter, including after Finished.
    NoCurrentProgress,
    /// The selected capture could not be read.
    Unavailable,
    /// No registration supplies a counter for this invocation.
    Unregistered,
}

impl RunState {
    /// Expose counter availability without dropping the blocked state.
    pub(crate) const fn working(self) -> CounterState {
        match self {
            Self::Working { phase, progress } => CounterState::Working { phase, progress },
            Self::Blocked => CounterState::Blocked,
        }
    }
}

/// Whether this invocation has a capture, independently of the latest log contents.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureLookup {
    /// No accepted registration was associated with this invocation in the scan.
    Unregistered,
    /// The registration exists, even when its log has no current progress.
    Registered(CaptureRead),
}

impl CaptureLookup {
    /// Keep every no-counter outcome named until the gauge chooses how to draw it.
    pub(crate) const fn working(&self) -> CounterState {
        match self {
            Self::Registered(CaptureRead::Progress(state)) => state.working(),
            Self::Registered(CaptureRead::NoCurrentProgress) => CounterState::NoCurrentProgress,
            Self::Registered(CaptureRead::Unreadable(_)) => CounterState::Unavailable,
            Self::Unregistered => CounterState::Unregistered,
        }
    }
}

/// Three distinct observations of the log associated with a registration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureRead {
    /// The latest output describes work or a build-directory wait.
    Progress(RunState),
    /// The log was read but has no current counter, including after Finished.
    NoCurrentProgress,
    /// Keep the failure so settings can distinguish access trouble from silence.
    Unreadable(CaptureFailure),
}

impl From<std::io::Result<String>> for CaptureRead {
    fn from(read: std::io::Result<String>) -> Self {
        match read {
            Ok(tail) => parse_state(&tail).map_or(Self::NoCurrentProgress, Self::Progress),
            Err(error) => Self::Unreadable(error.into()),
        }
    }
}

/// A cloneable scan observation retains the actual I/O kind and diagnostic.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureFailure {
    /// Allows later rendering to distinguish denial, absence, and other failures.
    pub(crate) kind:    std::io::ErrorKind,
    /// The error is observed once, without reopening the path during rendering.
    pub(crate) message: String,
}

impl From<std::io::Error> for CaptureFailure {
    fn from(error: std::io::Error) -> Self {
        Self {
            kind:    error.kind(),
            message: error.to_string(),
        }
    }
}

/// Retain the pathname at the failed operation, rather than only its root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PathFailure {
    /// Absolute artifact path, or a kernel interface name for boot observations.
    pub(crate) path:    PathBuf,
    /// Original kind and message survive transport to the display thread.
    pub(crate) failure: CaptureFailure,
}

thread_local! {
    /// The process scanner constructs a fresh `Capture` each pass, so root
    /// identity history belongs to its worker thread rather than that snapshot.
    static ROOT_HISTORY: RefCell<RootHistory> = RefCell::default();
}

/// Stable position in the scanner's root table; it grants no filesystem authority.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CaptureRootIndex(pub(crate) usize);

/// A shim pid belongs to one root even when another root registers the same pid.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct CaptureKey {
    /// Interned at scanner startup, without retaining an owned-root capability.
    pub(crate) root:        CaptureRootIndex,
    /// The shim process named by the registration in this root.
    pub(crate) pid:         u32,
    /// A replaced directory cannot retain any previous association.
    pub(crate) incarnation: RootIncarnation,
    /// Legacy annotation and published generations never alias each other.
    pub(crate) generation:  CaptureGeneration,
    /// Preserve comparison evidence without letting it replace the generation.
    pub(crate) birth:       IdentityEvidence,
}

/// Filename generation classifies publications without asserting verification.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) enum CaptureGeneration {
    /// A pid-only record has no generation or identity proof.
    Legacy,
    /// An opaque, non-repeating publication suffix.
    Published(String),
}

/// Resolve ownership only when the preferred root identifies one capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureSelection {
    /// No retained reading registers this pid in any root.
    Unregistered,
    /// Root precedence and identity evidence leave one possible capture.
    Selected(CaptureKey),
    /// Competing generations forbid ownership and fallback to another root or ancestor.
    Ambiguous,
}

/// Preserve the operator's pathname and its origin after root deduplication.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CaptureRootSource {
    /// Unset and empty environments both select the built-in shim root.
    Default,
    /// A nonempty `CARGO_TILE_ROOT` selects the reader's own shim root.
    Environment {
        /// Kept exactly as supplied, including relative spellings.
        path: PathBuf,
    },
    /// Additional discovery roots are named by entries in `capture.roots`.
    Configuration {
        /// Zero-based position in the original configuration list.
        entry: usize,
        /// Kept exactly as configured, even when another spelling is interned first.
        path:  PathBuf,
    },
}

/// One interned pathname with every source that selected it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureRoot {
    /// Absolute scan path, or a retained validation failure for later reporting.
    pub(crate) path:    Result<PathBuf, CaptureFailure>,
    /// Deduplication never discards the spelling needed by the settings view.
    pub(crate) sources: Vec<CaptureRootSource>,
}

/// Root identity resolved once at scanner startup; access is rechecked each scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureRoots {
    /// Index order gives the reader's own root precedence over additional roots.
    pub(crate) roots: Vec<CaptureRoot>,
}

impl CaptureRoots {
    /// Snapshot the environment and configuration before the first process scan.
    pub(crate) fn resolve(configured: &[PathBuf]) -> Self {
        Self::resolve_environment(configured, env::var_os(CAPTURE_ROOT_ENV).into())
    }

    /// Keep the environment boundary separate so resolution tests need no mutation.
    fn resolve_environment(configured: &[PathBuf], environment: CaptureRootEnvironment) -> Self {
        let (path, source) = match environment {
            CaptureRootEnvironment::Default => {
                (Ok(PathBuf::from(CAPTURE_ROOT)), CaptureRootSource::Default)
            },
            CaptureRootEnvironment::Override(path) => (
                std::path::absolute(&path).map_err(CaptureFailure::from),
                CaptureRootSource::Environment { path },
            ),
        };
        let mut roots = Self { roots: Vec::new() };
        roots.intern(path, source);
        for (entry, path) in configured.iter().enumerate() {
            let resolved = if path.is_absolute() {
                Ok(path.clone())
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!(
                        "capture.{CONFIG_KEY_CAPTURE_ROOTS}[{entry}]: {CAPTURE_ROOT_NOT_ABSOLUTE}"
                    ),
                )
                .into())
            };
            roots.intern(
                resolved,
                CaptureRootSource::Configuration {
                    entry,
                    path: path.clone(),
                },
            );
        }
        roots
    }

    /// Resolve ancestor aliases while leaving the final component for `O_NOFOLLOW`.
    fn intern(&mut self, path: Result<PathBuf, CaptureFailure>, source: CaptureRootSource) {
        let path = path.map(|path| {
            let path: PathBuf = path.components().collect();
            match (path.parent(), path.components().next_back()) {
                (Some(parent), Some(component)) => parent.canonicalize().map_or_else(
                    |_| path.clone(),
                    |mut parent| {
                        if component == Component::ParentDir {
                            // Resolve ancestor symlinks before applying the final `..`.
                            parent.pop();
                        } else {
                            parent.push(component);
                        }
                        parent
                    },
                ),
                _ => path,
            }
        });
        if let Some(root) = self
            .roots
            .iter_mut()
            .find(|root| path.is_ok() && root.path == path)
        {
            root.sources.push(source);
        } else {
            self.roots.push(CaptureRoot {
                path,
                sources: vec![source],
            });
        }
    }
}

/// The shim treats an empty environment override just like an unset variable.
enum CaptureRootEnvironment {
    /// Use the built-in write root.
    Default,
    /// Preserve the nonempty environment pathname.
    Override(PathBuf),
}

impl From<Option<OsString>> for CaptureRootEnvironment {
    fn from(value: Option<OsString>) -> Self {
        value
            .filter(|value| !value.is_empty())
            .map_or(Self::Default, |value| Self::Override(PathBuf::from(value)))
    }
}

/// Keep every readable generation separately from evidence completeness.
struct RegisteredRuns {
    /// A pid can retain several ended, unknown, or confirmed generations.
    generations: BTreeMap<u32, Vec<RegisteredRun>>,
    /// A disappearing or unreadable sibling disables cleanup for this scan.
    evidence:    RegistrationEvidence,
    /// Failed reads and records without proof remain visible independently of rows.
    diagnostics: Vec<CaptureDiagnostic>,
}

/// The scan's original record is the comparison target for pending removal.
struct RegisteredRun {
    /// Never infer process identity from this candidate filename.
    name:         PathBuf,
    /// Retain parsed contents for exact log association and final rereading.
    record:       Registration,
    /// Only the Confirmed variant carries a verified registration.
    verification: RegistrationVerification,
    /// Taken from the descriptor that supplied the record bytes.
    modified:     Result<SystemTime, CaptureFailure>,
}

/// Whether this scan read every published registration successfully.
#[derive(Debug, Eq, PartialEq)]
enum RegistrationEvidence {
    /// The bounded inventory and all record reads completed.
    Complete,
    /// Missing or unreadable entries retain sibling readings and prohibit cleanup.
    Incomplete,
}

/// A confirmed registration retains its descriptor-bound display timestamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConfirmedCapture {
    /// Read outcomes and registration metadata must select the same root and pid.
    pub(crate) key:          CaptureKey,
    /// Private verification construction is the only route to this proof.
    pub(crate) registration: VerifiedRegistration,
    /// Timestamp failure does not undo a successful process identity check.
    pub(crate) modified:     Result<SystemTime, CaptureFailure>,
}

/// Filename classification only; generation text never verifies process identity.
enum RegistrationName<'name> {
    /// Older shims publish only their pid.
    Legacy(u32),
    /// A published filename also supplies a candidate log generation.
    Generated {
        /// The shim pid preceding the first separator.
        pid:        u32,
        /// The nonempty suffix, accepted without claiming a birth stamp.
        generation: &'name str,
    },
    /// An unpublished record supplies cleanup evidence without capture membership.
    Staging {
        /// The shim pid is still a candidate until its birth is verified.
        pid:        u32,
        /// Strip only the staging suffix when checking the record's generation.
        generation: &'name str,
    },
    /// Unrelated or malformed names do not establish liveness.
    Unrelated,
}

/// Capture observations from one scan; lookups never reopen a pathname.
#[derive(Default)]
pub(crate) struct Capture {
    /// Absence from this map differs from an accepted but unreadable capture.
    readings:               BTreeMap<CaptureKey, CaptureRead>,
    /// Only identity-confirmed records can supply future registration-sourced rows.
    confirmed:              Vec<ConfirmedCapture>,
    /// One observation per effective root, including missing and invalid paths.
    pub(crate) root_status: Vec<RootStatus>,
}

impl Capture {
    /// Verification reads the current kernel identity independently of process snapshots.
    pub(crate) fn take(roots: &CaptureRoots) -> Self {
        let mut capture = Self::take_roots(roots, &birth_stamp::observe);
        capture.record_boot_verification(birth_stamp::boot_verification());
        capture
    }

    /// Cached boot failures make unknown identities session-long without changing retention.
    fn record_boot_verification(&mut self, verification: Result<(), PathFailure>) {
        let Err(failure) = verification else {
            return;
        };
        for status in &mut self.root_status {
            if matches!(status.state, RootReadStatus::Readable) {
                for diagnostic in &mut status.diagnostics {
                    if let CaptureDiagnostic::IdentityUnknown(path) = diagnostic {
                        *diagnostic = CaptureDiagnostic::IdentityBlockedByBoot(path.clone());
                    }
                }
                status
                    .diagnostics
                    .push(CaptureDiagnostic::BootUnavailable(failure.clone()));
            }
        }
    }

    /// The observer is invoked again immediately before each deletion attempt.
    #[cfg(test)]
    pub(crate) fn take_from(root: &Path, observe: impl Fn(u32) -> KernelObservation) -> Self {
        let roots = CaptureRoots::resolve_environment(
            &[],
            CaptureRootEnvironment::Override(root.to_owned()),
        );
        Self::take_roots(&roots, &observe)
    }

    /// One removal allowance covers every owned root in this pass.
    pub(crate) fn take_roots(
        roots: &CaptureRoots,
        observe: &impl Fn(u32) -> KernelObservation,
    ) -> Self {
        let mut capture = Self::default();
        let mut budget = SweepBudget::default();
        for (index, root) in roots.roots.iter().enumerate() {
            let mut status = RootStatus {
                root:         root.clone(),
                owner:        RootOwner::Unavailable,
                cleanup:      Vec::new(),
                state:        RootReadStatus::Readable,
                confirmed:    0,
                diagnostics:  Vec::new(),
                associations: Vec::new(),
            };
            match &root.path {
                Err(failure) => status.state = RootReadStatus::Invalid(failure.clone()),
                Ok(path) => {
                    match ROOT_HISTORY.with_borrow_mut(|history| RootScan::open(path, history)) {
                        Err(error)
                            if error.kind() == std::io::ErrorKind::NotFound
                                && matches!(
                                    root.sources.as_slice(),
                                    [CaptureRootSource::Default]
                                ) =>
                        {
                            status.state = RootReadStatus::DefaultNotCreated;
                        },
                        Err(error) => {
                            let failure = PathFailure {
                                path:    path.clone(),
                                failure: error.into(),
                            };
                            status.cleanup.push(CleanupRefusal::Access(failure.clone()));
                            status.state = RootReadStatus::Unavailable(failure);
                        },
                        Ok(scan) => {
                            status.owner = scan.owner();
                            capture.scan_root(
                                CaptureRootIndex(index),
                                &scan,
                                observe,
                                &mut budget,
                                &mut status,
                            );
                        },
                    }
                },
            }
            capture.root_status.push(status);
        }
        capture
    }

    /// Versioned records name their log directly. Only legacy display annotations
    /// need the bounded log inventory, and none of those logs can be swept.
    fn scan_root(
        &mut self,
        root: CaptureRootIndex,
        scan: &RootScan,
        observe: &impl Fn(u32) -> KernelObservation,
        budget: &mut SweepBudget,
        status: &mut RootStatus,
    ) {
        let RegisteredRuns {
            generations,
            evidence,
            diagnostics,
        } = registered_runs(scan, observe);
        status.diagnostics = diagnostics;
        let mut logs = BTreeMap::new();
        for (&pid, runs) in &generations {
            for run in runs {
                if matches!(
                    registration_name(&run.name),
                    RegistrationName::Staging { .. }
                ) {
                    continue;
                }
                let (generation, birth) = match &run.record {
                    Registration::Versioned(record) => (
                        CaptureGeneration::Published(record.generation().to_owned()),
                        record.identity().clone(),
                    ),
                    Registration::Legacy(_) => {
                        (CaptureGeneration::Legacy, IdentityEvidence::Unavailable)
                    },
                };
                let key = CaptureKey {
                    root,
                    pid,
                    incarnation: scan.incarnation(),
                    generation,
                    birth,
                };
                match &run.verification {
                    RegistrationVerification::Ended => {},
                    RegistrationVerification::Confirmed(registration) => {
                        self.confirmed.push(ConfirmedCapture {
                            key:          key.clone(),
                            registration: registration.clone(),
                            modified:     run.modified.clone(),
                        });
                        let reading = read_named_log(
                            scan,
                            Path::new(registration.record().log_basename()),
                            &mut logs,
                            &mut status.diagnostics,
                        );
                        if !matches!(reading, CaptureRead::Unreadable(_)) {
                            status.confirmed += 1;
                        }
                        self.readings.insert(key, reading);
                    },
                    RegistrationVerification::Unknown => {
                        let reading = match &run.record {
                            Registration::Versioned(record) => read_named_log(
                                scan,
                                Path::new(record.log_basename()),
                                &mut logs,
                                &mut status.diagnostics,
                            ),
                            Registration::Legacy(_) => {
                                legacy_read(scan, pid, &mut logs, &mut status.diagnostics)
                            },
                        };
                        self.readings.insert(key, reading);
                    },
                }
            }
        }
        for outcome in scan.sampled_log_outcome() {
            enumeration_diagnostic(outcome, scan.path().to_owned(), &mut status.diagnostics);
        }
        status.cleanup = scan.cleanup_refusals();
        if evidence == RegistrationEvidence::Incomplete {
            if matches!(scan.registration_outcome(), Enumeration::Complete) {
                status.cleanup.push(CleanupRefusal::RegistrationIncomplete(
                    scan.registration_path(),
                ));
            }
            return;
        }
        sweep_ended(scan, &generations, observe, budget);
    }

    /// Membership survives empty or unreadable output without claiming current progress.
    pub(crate) fn read(&self, key: &CaptureKey) -> CaptureLookup {
        self.readings
            .get(key)
            .cloned()
            .map_or(CaptureLookup::Unregistered, CaptureLookup::Registered)
    }

    /// Select the preferred root before resolving generations within that root.
    /// The verifier already excludes ended births from `readings`. Every other
    /// published generation still competes: neither generation text nor modification
    /// time proves which same-birth record is live, and unknown proof is not ended.
    pub(crate) fn select(&self, pid: u32) -> CaptureSelection {
        let Some(root) = self
            .readings
            .keys()
            .filter(|key| key.pid == pid)
            .map(|key| key.root)
            .min()
        else {
            return CaptureSelection::Unregistered;
        };
        let mut confirmed = self
            .confirmed
            .iter()
            .filter(|confirmed| confirmed.key.pid == pid && confirmed.key.root == root);
        let mut candidates = self
            .readings
            .keys()
            .filter(|key| key.pid == pid && key.root == root);
        if let Some(confirmed_capture) = confirmed.next() {
            // Separate verification reads can straddle pid reuse. Even different
            // confirmed births do not establish which proof is still current.
            if confirmed.next().is_some()
                || candidates.any(|key| {
                    key != &confirmed_capture.key
                        && matches!(key.generation, CaptureGeneration::Published(_))
                })
            {
                return CaptureSelection::Ambiguous;
            }
            return CaptureSelection::Selected(confirmed_capture.key.clone());
        }
        match (candidates.next(), candidates.next()) {
            (Some(key), None) => CaptureSelection::Selected(key.clone()),
            (Some(_), Some(_)) => CaptureSelection::Ambiguous,
            (None, _) => CaptureSelection::Unregistered,
        }
    }

    /// Inspect retained readings in fixtures; ownership must use `select`.
    #[cfg(test)]
    pub(crate) fn keys(&self, pid: u32) -> impl Iterator<Item = CaptureKey> + '_ {
        let mut keys: Vec<_> = self.readings.keys().filter(|key| key.pid == pid).collect();
        keys.sort_by_key(|key| {
            (
                key.root,
                !self
                    .confirmed
                    .iter()
                    .any(|confirmed| &confirmed.key == *key),
                *key,
            )
        });
        keys.into_iter().cloned()
    }

    /// Consumers receive proofs that only the registration verifier can construct.
    pub(crate) fn confirmed(&self) -> &[ConfirmedCapture] { &self.confirmed }
}

/// Reread an ended registration and its kernel evidence before paired removal.
fn sweep_ended(
    scan: &RootScan,
    generations: &BTreeMap<u32, Vec<RegisteredRun>>,
    observe: &impl Fn(u32) -> KernelObservation,
    budget: &mut SweepBudget,
) {
    scan.sweep(budget, |entry| {
        let (RegistrationName::Generated { pid, .. } | RegistrationName::Staging { pid, .. }) =
            registration_name(entry.name())
        else {
            return SweepDisposition::Preserve;
        };
        let Some(run) = generations
            .get(&pid)
            .and_then(|runs| runs.iter().find(|run| run.name == entry.name()))
        else {
            return SweepDisposition::Preserve;
        };
        let Registration::Versioned(record) = &run.record else {
            return SweepDisposition::Preserve;
        };
        if !matches!(run.verification, RegistrationVerification::Ended) {
            return SweepDisposition::Preserve;
        }
        let Ok(current) = entry.read_registration() else {
            return SweepDisposition::Preserve;
        };
        if Registration::parse(&current.bytes).ok().as_ref() != Some(&run.record) {
            return SweepDisposition::Preserve;
        }
        if matches!(
            record.verify_observation(pid, &observe(pid)),
            RegistrationVerification::Ended
        ) {
            SweepDisposition::Remove(PathBuf::from(record.log_basename()))
        } else {
            SweepDisposition::Preserve
        }
    });
}

/// Legacy filenames can annotate an existing process row, but never authorize cleanup.
fn legacy_read(
    scan: &RootScan,
    pid: u32,
    logs: &mut BTreeMap<PathBuf, CaptureRead>,
    diagnostics: &mut Vec<CaptureDiagnostic>,
) -> CaptureRead {
    let log = scan
        .log_entries()
        .filter(|entry| log_pid(entry.name()) == Some(pid))
        .max_by(|left, right| left.name().cmp(right.name()));
    log.map_or(CaptureRead::NoCurrentProgress, |entry| {
        read_named_log(scan, entry.name(), logs, diagnostics)
    })
}

/// A successful identity comparison does not hide a failed read of its named log.
fn read_named_log(
    scan: &RootScan,
    name: &Path,
    logs: &mut BTreeMap<PathBuf, CaptureRead>,
    diagnostics: &mut Vec<CaptureDiagnostic>,
) -> CaptureRead {
    let reading = logs
        .entry(name.to_owned())
        .or_insert_with(|| CaptureRead::from(scan.read_log(name)))
        .clone();
    record_log_failure(scan, name, &reading, diagnostics);
    reading
}

/// Count each failed log path once, even when several registrations name it.
fn record_log_failure(
    scan: &RootScan,
    name: &Path,
    reading: &CaptureRead,
    diagnostics: &mut Vec<CaptureDiagnostic>,
) {
    if let CaptureRead::Unreadable(failure) = reading {
        let path = scan.path().join(name);
        if diagnostics.iter().any(|diagnostic| {
            matches!(diagnostic, CaptureDiagnostic::LogUnreadable(previous) if previous.path == path)
        }) {
            return;
        }
        diagnostics.push(CaptureDiagnostic::LogUnreadable(PathFailure {
            path,
            failure: failure.clone(),
        }));
    }
}

/// Directory enumeration failure is separate from a record that failed after enumeration.
fn enumeration_diagnostic(
    outcome: &Enumeration,
    path: PathBuf,
    diagnostics: &mut Vec<CaptureDiagnostic>,
) {
    match outcome {
        Enumeration::Complete => {},
        Enumeration::Incomplete => diagnostics.push(CaptureDiagnostic::EnumerationIncomplete(path)),
        Enumeration::Failed(error) => {
            diagnostics.push(CaptureDiagnostic::EnumerationFailed(PathFailure {
                path,
                failure: std::io::Error::new(error.kind(), error.to_string()).into(),
            }));
        },
    }
}

/// Parse each registration independently and retain every generation's identity result.
fn registered_runs(scan: &RootScan, observe: &impl Fn(u32) -> KernelObservation) -> RegisteredRuns {
    let mut evidence = match scan.registration_outcome() {
        Enumeration::Complete => RegistrationEvidence::Complete,
        Enumeration::Incomplete | Enumeration::Failed(_) => RegistrationEvidence::Incomplete,
    };
    let mut diagnostics = Vec::new();
    enumeration_diagnostic(
        scan.registration_outcome(),
        scan.registration_path(),
        &mut diagnostics,
    );
    let mut generations: BTreeMap<u32, Vec<RegisteredRun>> = BTreeMap::new();
    let mut entries: Vec<_> = scan.registration_entries().collect();
    entries.sort_by(|left, right| left.name().cmp(right.name()));
    for entry in entries {
        let path = scan.path().join(CAPTURE_LIVE_RUNS_DIR).join(entry.name());
        let pid = match registration_name(entry.name()) {
            RegistrationName::Legacy(pid) | RegistrationName::Generated { pid, .. } => pid,
            RegistrationName::Staging { pid, .. } => {
                diagnostics.push(CaptureDiagnostic::Staging(path.clone()));
                pid
            },
            RegistrationName::Unrelated => continue,
        };
        let observation = match entry.read_registration() {
            Ok(observation) => observation,
            Err(error) => {
                if !matches!(
                    registration_name(entry.name()),
                    RegistrationName::Staging { .. }
                ) {
                    evidence = RegistrationEvidence::Incomplete;
                }
                diagnostics.push(CaptureDiagnostic::RegistrationUnreadable(PathFailure {
                    path,
                    failure: error.into(),
                }));
                continue;
            },
        };
        let Ok(record) = Registration::parse(&observation.bytes) else {
            diagnostics.push(CaptureDiagnostic::RegistrationInvalid(path));
            continue;
        };
        let verification = match (&record, registration_name(entry.name())) {
            (
                Registration::Versioned(record),
                RegistrationName::Generated { generation, .. }
                | RegistrationName::Staging { generation, .. },
            ) if record.generation() == generation => record.verify_observation(pid, &observe(pid)),
            (Registration::Legacy(_), _) => RegistrationVerification::Unknown,
            _ => {
                diagnostics.push(CaptureDiagnostic::RegistrationInvalid(path));
                continue;
            },
        };
        if !matches!(
            registration_name(entry.name()),
            RegistrationName::Staging { .. }
        ) && matches!(verification, RegistrationVerification::Unknown)
        {
            diagnostics.push(match &record {
                Registration::Legacy(_) => CaptureDiagnostic::AnnotationOnly(path),
                Registration::Versioned(record)
                    if matches!(record.identity(), IdentityEvidence::Unavailable) =>
                {
                    CaptureDiagnostic::Unverifiable(path)
                },
                Registration::Versioned(_) => CaptureDiagnostic::IdentityUnknown(path),
            });
        }
        generations.entry(pid).or_default().push(RegisteredRun {
            name: entry.name().to_owned(),
            record,
            verification,
            modified: observation
                .metadata
                .modified()
                .map_err(CaptureFailure::from),
        });
    }
    RegisteredRuns {
        generations,
        evidence,
        diagnostics,
    }
}

/// Accept legacy pids and any nonempty generation suffix without interpreting
/// the suffix as process identity. Staging names supply cleanup candidates only.
fn registration_name(path: &Path) -> RegistrationName<'_> {
    let Some(name) = path.to_str() else {
        return RegistrationName::Unrelated;
    };
    let published = name.strip_suffix(REGISTRATION_TEMP_SUFFIX).unwrap_or(name);
    let (pid, generation) = published
        .split_once(REGISTRATION_SEPARATOR)
        .unwrap_or((published, ""));
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return RegistrationName::Unrelated;
    }
    let Ok(pid) = pid.parse() else {
        return RegistrationName::Unrelated;
    };
    if !generation.is_empty() {
        if published == name {
            RegistrationName::Generated { pid, generation }
        } else {
            RegistrationName::Staging { pid, generation }
        }
    } else if published != name || published.contains(REGISTRATION_SEPARATOR) {
        RegistrationName::Unrelated
    } else {
        RegistrationName::Legacy(pid)
    }
}

/// The shim pid a log file is named for: `run-<generation>-<pid>.log`.
fn log_pid(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix(RUN_LOG_PREFIX)?
        .strip_suffix(RUN_LOG_SUFFIX)?
        .rsplit(PID_SEPARATOR)
        .next()?
        .parse()
        .ok()
}

/// What the end of a log says the run is doing now.
///
/// Neither marker means anything by itself: both are printed once and
/// then stay in the log for as long as the run does. What settles each
/// is what came after it.
///
/// The wait is proof the run waited, never that it still is -- a bar, a
/// `Finished`, or the output of the binary a `cargo run` went on to
/// start each say the lock came free. A run that is still waiting has
/// written the line and then nothing, which is exactly what being
/// blocked looks like from outside.
///
/// A counter is proof the run was building, never that it still is: the
/// bar is left on screen where it stopped, so a build that finished at
/// `1/2` goes on reading 50% for as long as the process lives -- which
/// for a `cargo run` is the whole life of the app it started. Cargo's
/// own `Finished` past the counter is what retires it, and a counter
/// past *that* is a test runner's, which has every right to the column.
fn parse_state(tail: &str) -> Option<RunState> {
    if still_waiting(tail) {
        return Some(RunState::Blocked);
    }
    let (at, state) = last_counter(tail)?;
    tail.rfind(BUILD_FINISHED_MARKER)
        .is_none_or(|over| at > over)
        .then_some(state)
}

/// Whether the log ends on the wait line.
///
/// The line itself is the one line the trailing text is allowed to
/// hold. Cargo redraws its bar over carriage returns rather than
/// newlines, so a redraw that followed the wait counts as the second
/// line here just as a `Finished` would.
fn still_waiting(tail: &str) -> bool {
    tail.rfind(LOCK_WAIT_MARKER)
        .and_then(|at| tail.get(at..))
        .is_some_and(|after| after.trim_end().lines().count() == 1)
}

/// The last counter in `tail` and where it sits, which is the most
/// recent redraw of the bar.
///
/// Last rather than first because a run draws more than one bar: cargo
/// counts downloads before it counts compilations, each nested cargo a
/// command drives counts its own, and a test runner counts the tests
/// once the compiling is over. The one at the end is the one happening
/// now.
///
/// Where it sits is what says whether it still stands: a bar is left
/// on screen where it stopped, so the last redraw of a finished build
/// reads no differently from the last redraw of a running one.
fn last_counter(tail: &str) -> Option<(usize, RunState)> {
    tail.rmatch_indices(UNIT_COUNTER_LEAD)
        .find_map(|(index, lead)| {
            let after = index.saturating_add(lead.len());
            let (counter, progress) = counter_at(tail.get(after..)?)?;
            Some((
                index,
                RunState::Working {
                    phase: counter.phase(tail, index),
                    progress,
                },
            ))
        })
}

/// Which of the two counters a reading came off, which is half of
/// what says whose counter it is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Counter {
    /// Drawn in a bar and closed by a colon, `149/403:`. Cargo draws
    /// one and so does a test runner, so the line it sits on is what
    /// separates them.
    Bar,
    /// A test runner's per-test tally, `(11/24)`. Nothing else writes
    /// one, so the parentheses alone settle it.
    Tally,
}

impl Counter {
    /// Which phase a counter of this kind at `index` belongs to.
    ///
    /// A tally answers for itself. A bar is read off the status word
    /// opening the line it was drawn on -- cargo says `Building`, a
    /// test runner says `Running` -- and only that line is searched: a
    /// log holds the word many times over, cargo saying `Running` of
    /// every test binary a plain `cargo test` starts, and none of those
    /// lines carries a counter.
    fn phase(self, tail: &str, index: usize) -> Phase {
        let Self::Bar = self else {
            return Phase::Testing;
        };
        let opens = tail
            .get(..index)
            .and_then(|ahead| ahead.rfind(['\n', '\r']))
            .map_or(0, |at| at.saturating_add(1));
        if tail
            .get(opens..index)
            .is_some_and(|line| line.contains(TEST_PHASE_MARKER))
        {
            Phase::Testing
        } else {
            Phase::Building
        }
    }
}

/// Read a counter off the front of `text`, past a drawn bar where one
/// stands in the way.
///
/// Cargo puts its counter straight after the bracket that closes its
/// bar. A test runner brackets its elapsed time instead and draws the
/// bar after it, so the counter is reached across the blocks the bar is
/// filled with and the blanks it is padded to width with -- and where
/// it has no bar at all, it brackets each test's own duration and puts
/// the count in parentheses beyond that.
///
/// How the two numbers are closed is what tells a counter from anything
/// else that pairs numbers with a slash: a colon for a bar, a
/// parenthesis for a tally, and a pair closed by neither is not a
/// counter at all.
fn counter_at(text: &str) -> Option<(Counter, Progress)> {
    let text = text.trim_start_matches(bar_fill);
    let (counter, text) = text
        .strip_prefix(TALLY_OPEN)
        .map_or((Counter::Bar, text), |inside| (Counter::Tally, inside));
    let (done, text) = leading_number(text.trim_start_matches(bar_fill))?;
    let (total, text) = leading_number(text.strip_prefix(UNIT_COUNTER_SEPARATOR)?)?;
    let closed = match counter {
        Counter::Bar => text.starts_with(UNIT_COUNTER_TRAILER),
        Counter::Tally => text.starts_with(TALLY_CLOSE),
    };
    (total > 0 && closed).then_some((counter, Progress { done, total }))
}

/// Whether `character` is part of a drawn bar rather than of the
/// counter beyond it.
fn bar_fill(character: char) -> bool {
    character == ' ' || (BAR_GLYPH_FIRST..=BAR_GLYPH_LAST).contains(&character)
}

/// The digits `text` opens with, and what follows them.
fn leading_number(text: &str) -> Option<(usize, &str)> {
    let end = text
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(text.len());
    Some((text.get(..end)?.parse().ok()?, text.get(end..)?))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::birth_stamp::Observation;
    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;

    #[test]
    fn aliased_legacy_and_versioned_registrations_read_one_log_once_per_scan() {
        let root = capture_root();
        publish(root.path(), 10, "same", "100", CAPTURED_REDRAW);
        fs::write(
            root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10"),
            "/writer/project\tcargo build",
        )
        .unwrap();
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        let mut capture = Capture::default();
        capture.scan_with_observations(&scan, &|_| present("100"), &mut SweepBudget::default());
        assert_eq!(scan.log_read_count(), 1);
        assert_eq!(capture.keys(10).count(), 2);
        for key in capture.keys(10) {
            assert_eq!(
                capture.read(&key),
                CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
            );
        }
        fs::remove_file(root.path().join("run-same-10.log")).unwrap();
        fs::create_dir(root.path().join("run-same-10.log")).unwrap();
        let unreadable = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        let mut capture = Capture::default();
        capture.scan_with_observations(
            &unreadable,
            &|_| present("100"),
            &mut SweepBudget::default(),
        );
        assert_eq!(unreadable.log_read_count(), 1);
        for key in capture.keys(10) {
            assert!(matches!(
                capture.read(&key),
                CaptureLookup::Registered(CaptureRead::Unreadable(_))
            ));
        }
    }

    #[test]
    fn same_pid_birth_and_root_keep_both_generation_logs_and_metadata() {
        let root = capture_root();
        publish(root.path(), 10, "first", "100", CAPTURED_REDRAW);
        let (second, _) = publish(root.path(), 10, "second", "100", CAPTURED_WAIT);
        let second_record = String::from_utf8(fs::read(&second).unwrap())
            .unwrap()
            .replace("/writer/project", "/second/project");
        fs::write(second, second_record).unwrap();
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        let mut capture = Capture::default();
        capture.scan_with_observations(&scan, &|_| present("100"), &mut SweepBudget::default());
        assert_eq!(scan.log_read_count(), 2);
        assert_eq!(capture.keys(10).count(), 2);
        assert_eq!(capture.confirmed().len(), 2);
        for confirmed in capture.confirmed() {
            let generation = confirmed.registration.record().generation();
            assert_eq!(
                confirmed.registration.record().directory(),
                Path::new(if generation == "first" {
                    "/writer/project"
                } else {
                    "/second/project"
                })
            );
            assert_eq!(
                confirmed.key.generation,
                CaptureGeneration::Published(generation.to_owned())
            );
            let expected = if generation == "first" {
                compiling(149, 403)
            } else {
                RunState::Blocked
            };
            assert_eq!(
                capture.read(&confirmed.key),
                CaptureLookup::Registered(CaptureRead::Progress(expected))
            );
        }
        assert_ne!(capture.confirmed()[0].key, capture.confirmed()[1].key);
    }

    #[test]
    fn competing_confirmed_generations_never_choose_by_name_or_modification_time() {
        for (stale, replacement) in [("a-stale", "z-current"), ("z-stale", "a-current")] {
            let root = capture_root();
            publish(root.path(), 10, stale, "100", CAPTURED_REDRAW);
            publish(root.path(), 10, replacement, "100", CAPTURED_WAIT);
            let mut capture = Capture::take_with_observations(root.path(), |_| present("100"));
            assert_eq!(capture.confirmed().len(), 2);
            assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
            for reverse in [false, true] {
                for confirmed in &mut capture.confirmed {
                    let current = confirmed.registration.record().generation() == replacement;
                    confirmed.modified = Ok(SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(u64::from(current != reverse)));
                }
                assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
            }
            capture.confirmed.reverse();
            for confirmed in &mut capture.confirmed {
                confirmed.modified =
                    Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied).into());
            }
            assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
        }
    }

    #[test]
    fn ended_birth_evidence_selects_the_live_generation_in_either_name_order() {
        for (stale, live) in [("a-stale", "z-live"), ("z-stale", "a-live")] {
            let root = capture_root();
            publish(root.path(), 10, stale, "100", CAPTURED_REDRAW);
            publish(root.path(), 10, live, "101", CAPTURED_WAIT);
            let capture = Capture::take_with_observations(root.path(), |_| present("101"));
            let confirmed = &capture.confirmed()[0];
            assert_eq!(confirmed.registration.record().generation(), live);
            assert_eq!(
                capture.select(10),
                CaptureSelection::Selected(confirmed.key.clone())
            );
            assert_eq!(
                capture.read(&confirmed.key),
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
            );
        }
    }

    #[test]
    fn a_failed_sibling_verification_cannot_resolve_generation_ownership() {
        for sibling_birth in ["100", "101", ""] {
            for (live, sibling) in [("a-live", "z-sibling"), ("z-live", "a-sibling")] {
                let root = capture_root();
                publish(root.path(), 10, live, "100", CAPTURED_REDRAW);
                publish(root.path(), 10, sibling, sibling_birth, CAPTURED_WAIT);
                let observations = std::cell::Cell::new(0);
                let capture = Capture::take_with_observations(root.path(), |_| {
                    let first = observations.get() == 0;
                    observations.set(observations.get() + 1);
                    if first == (live < sibling) {
                        present("100")
                    } else {
                        Observation::Unknown
                    }
                });
                assert_eq!(capture.confirmed().len(), 1);
                assert_eq!(capture.keys(10).count(), 2);
                assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
            }
        }
    }

    #[test]
    fn confirmation_across_two_births_does_not_order_the_live_generation() {
        let root = capture_root();
        publish(root.path(), 10, "a-first", "100", CAPTURED_REDRAW);
        publish(root.path(), 10, "z-second", "101", CAPTURED_WAIT);
        let observations = std::cell::Cell::new(0);
        let capture = Capture::take_with_observations(root.path(), |_| {
            let first = observations.get() == 0;
            observations.set(observations.get() + 1);
            present(if first { "100" } else { "101" })
        });
        assert_eq!(capture.confirmed().len(), 2);
        assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
    }

    #[test]
    fn a_unique_confirmed_generation_takes_precedence_over_legacy_annotation() {
        let root = capture_root();
        publish(root.path(), 10, "live", "100", CAPTURED_REDRAW);
        fs::write(
            root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10"),
            "/writer/project\tcargo build",
        )
        .unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(capture.keys(10).count(), 2);
        assert_eq!(
            capture.select(10),
            CaptureSelection::Selected(capture.confirmed()[0].key.clone())
        );
    }

    #[test]
    fn unconfirmed_annotations_require_one_generation_in_the_preferred_root() {
        let root = capture_root();
        assert_eq!(
            Capture::take_with_observations(root.path(), |_| Observation::Unknown).select(10),
            CaptureSelection::Unregistered
        );
        publish(root.path(), 10, "first", "100", CAPTURED_REDRAW);
        let unique = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        assert_eq!(
            unique.select(10),
            CaptureSelection::Selected(unique.keys(10).next().unwrap())
        );
        publish(root.path(), 10, "second", "100", CAPTURED_WAIT);
        let competing = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        assert_eq!(competing.select(10), CaptureSelection::Ambiguous);
    }

    #[test]
    fn preferred_root_ambiguity_never_falls_through_to_another_roots_proof() {
        let preferred = capture_root();
        let other = capture_root();
        publish(preferred.path(), 10, "first", "100", CAPTURED_REDRAW);
        publish(preferred.path(), 10, "second", "100", CAPTURED_WAIT);
        publish(other.path(), 10, "third", "100", CAPTURED_TALLY);
        let roots = CaptureRoots::resolve_environment(
            &[other.path().to_owned()],
            CaptureRootEnvironment::Override(preferred.path().to_owned()),
        );
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(capture.confirmed().len(), 3);
        assert_eq!(capture.select(10), CaptureSelection::Ambiguous);
    }

    #[test]
    fn preferred_unconfirmed_root_blocks_a_later_roots_unique_confirmation() {
        let preferred = capture_root();
        let other = capture_root();
        publish(preferred.path(), 10, "unknown", "", CAPTURED_WAIT);
        publish(other.path(), 10, "confirmed", "100", CAPTURED_REDRAW);
        let roots = CaptureRoots::resolve_environment(
            &[other.path().to_owned()],
            CaptureRootEnvironment::Override(preferred.path().to_owned()),
        );
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(capture.confirmed().len(), 1);
        let preferred_key = capture.keys(10).next().unwrap();
        assert_eq!(preferred_key.root, CaptureRootIndex(0));
        assert_eq!(
            capture.select(10),
            CaptureSelection::Selected(preferred_key)
        );
    }

    #[test]
    fn permission_changes_preserve_keys_but_replaced_roots_invalidate_them() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("capture");
        fs::create_dir_all(root.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        publish(&root, 10, "same", "100", CAPTURED_REDRAW);
        let first = Capture::take_with_observations(&root, |_| present("100"));
        let key = first.keys(10).next().unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();
        let changed_permissions = Capture::take_with_observations(&root, |_| present("100"));
        assert_eq!(key, changed_permissions.confirmed()[0].key);
        assert_eq!(first.read(&key), changed_permissions.read(&key));
        fs::rename(&root, parent.path().join("previous")).unwrap();
        fs::create_dir_all(root.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        publish(&root, 10, "same", "100", CAPTURED_REDRAW);
        let replacement = Capture::take_with_observations(&root, |_| present("100"));
        assert_ne!(key, replacement.confirmed()[0].key);
        assert_eq!(replacement.read(&key), CaptureLookup::Unregistered);
        fs::rename(&root, parent.path().join("second")).unwrap();
        fs::rename(parent.path().join("previous"), &root).unwrap();
        let restored_object = Capture::take_with_observations(&root, |_| present("100"));
        assert_ne!(key, restored_object.confirmed()[0].key);
        assert_ne!(
            replacement.confirmed()[0].key,
            restored_object.confirmed()[0].key
        );
    }

    #[test]
    fn finished_application_retains_registration_without_a_current_counter() {
        let root = capture_root();
        publish(
            root.path(),
            10,
            "running",
            "100",
            &format!("{CAPTURED_REDRAW}\n{CAPTURED_FINISHED}application is alive\n"),
        );
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        let reading = capture.lookup(0, 10);
        assert_eq!(
            reading,
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
        assert_eq!(reading.working(), CounterState::NoCurrentProgress);
        assert_eq!(
            CaptureLookup::Unregistered.working(),
            CounterState::Unregistered
        );
        assert_eq!(
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)).working(),
            CounterState::Blocked
        );
        assert_eq!(
            CaptureLookup::Registered(CaptureRead::Unreadable(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
            ))
            .working(),
            CounterState::Unavailable
        );
    }

    /// The line cargo prints when another cargo holds the build
    /// directory, as it comes out of a real 1.96 run.
    const CAPTURED_WAIT: &str = "    Blocking waiting for file lock on build directory\n";

    /// The state a run reports while it is compiling `done` of `total`.
    fn compiling(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Building,
            progress: Progress { done, total },
        }
    }

    /// The state a run reports while it works through `done` of `total`
    /// tests.
    fn testing(done: usize, total: usize) -> RunState {
        RunState::Working {
            phase:    Phase::Testing,
            progress: Progress { done, total },
        }
    }

    /// A redraw of cargo's bar as the shim captures it, colour codes and
    /// carriage return included.
    const CAPTURED_REDRAW: &str = "\u{1b}[1m\u{1b}[92m    Building\u{1b}[0m \
         [========>                ] 149/403: globset, regex-automata\r";

    /// A redraw of nextest's bar as the shim captures it: the elapsed
    /// time bracketed, the drawn bar after it, and the counter past
    /// that.
    const CAPTURED_TEST_REDRAW: &str = "\u{1b}[32;1m     Running\u{1b}[0m \
         [ 00:00:01] \u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{258b}      \
         12/24: \u{1b}[1m2\u{1b}[0m running, \u{1b}[1m12\u{1b}[0m passed\r\n";

    /// A tally as nextest writes it where it has no bar to put the
    /// count in, which is every run whose output is not a terminal.
    /// Left-padded to the width of the total, so a run of a thousand
    /// tests opens with three blanks inside the parenthesis.
    const CAPTURED_TALLY: &str = "        PASS [   1.014s] (11/24) nxprobe t18\n";

    /// The line nextest prints under its bar for each test in flight,
    /// which is what stands between the bar and the end of the log.
    const CAPTURED_TEST_ROW: &str =
        "             [ 00:00:00] \u{1b}[35;1mnxprobe\u{1b}[0m \u{1b}[34;1mt18\u{1b}[0m\r\n";

    impl Capture {
        /// Tests observe missing registrations through the same identity-keyed lookup.
        fn lookup(&self, root: usize, pid: u32) -> CaptureLookup {
            self.keys(pid)
                .find(|key| key.root == CaptureRootIndex(root))
                .map_or(CaptureLookup::Unregistered, |key| self.read(&key))
        }

        /// Bind each injected race observation without enabling production injection.
        fn take_with_observations(root: &Path, observe: impl Fn(u32) -> Observation) -> Self {
            Self::take_from(root, |pid| KernelObservation::for_test(pid, observe(pid)))
        }

        /// Keep directory replacement and shared-budget tests on the production sweep.
        fn scan_with_observations(
            &mut self,
            scan: &RootScan,
            observe: &impl Fn(u32) -> Observation,
            budget: &mut SweepBudget,
        ) {
            self.scan_root(
                CaptureRootIndex(0),
                scan,
                &|pid| KernelObservation::for_test(pid, observe(pid)),
                budget,
                &mut RootStatus {
                    root:         CaptureRoot {
                        path:    Ok(scan.path().to_owned()),
                        sources: vec![CaptureRootSource::Default],
                    },
                    owner:        scan.owner(),
                    cleanup:      Vec::new(),
                    state:        RootReadStatus::Readable,
                    confirmed:    0,
                    diagnostics:  Vec::new(),
                    associations: Vec::new(),
                },
            );
        }
    }

    #[test]
    fn missing_default_only_root_stays_quiet_and_recovers_next_scan() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("unused-default");
        let roots = CaptureRoots {
            roots: vec![CaptureRoot {
                path:    Ok(root.clone()),
                sources: vec![CaptureRootSource::Default],
            }],
        };
        let first = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_eq!(first.root_status.len(), 1);
        let status = &first.root_status[0];
        assert_eq!(status.root, roots.roots[0]);
        assert_eq!(status.state, RootReadStatus::DefaultNotCreated);
        assert!(status.cleanup.is_empty());
        assert!(status.diagnostics.is_empty());
        assert_eq!(status.confirmed, 0);

        fs::create_dir_all(root.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let second = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_eq!(second.root_status.len(), 1);
        assert_eq!(second.root_status[0].state, RootReadStatus::Readable);
    }

    #[test]
    fn explicitly_named_missing_roots_keep_access_failures() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("explicit-missing");
        let environment = CaptureRootSource::Environment { path: root.clone() };
        let configuration = CaptureRootSource::Configuration {
            entry: 0,
            path:  root.clone(),
        };
        for sources in [
            vec![environment.clone()],
            vec![configuration.clone()],
            vec![CaptureRootSource::Default, environment],
            vec![CaptureRootSource::Default, configuration],
        ] {
            let roots = CaptureRoots {
                roots: vec![CaptureRoot {
                    path: Ok(root.clone()),
                    sources,
                }],
            };
            let capture = Capture::take_roots(&roots, &|pid| {
                KernelObservation::for_test(pid, Observation::Unknown)
            });
            assert_eq!(capture.root_status.len(), 1);
            let status = &capture.root_status[0];
            assert!(matches!(
                &status.state,
                RootReadStatus::Unavailable(failure)
                    if failure.path == root
                        && failure.failure.kind == std::io::ErrorKind::NotFound
            ));
            assert!(matches!(
                status.cleanup.as_slice(),
                [CleanupRefusal::Access(failure)]
                    if status.state == RootReadStatus::Unavailable(failure.clone())
            ));
        }
    }

    #[test]
    fn default_only_root_keeps_failures_other_than_missing() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("default-file");
        fs::write(&root, "not a directory").unwrap();
        let roots = CaptureRoots {
            roots: vec![CaptureRoot {
                path:    Ok(root.clone()),
                sources: vec![CaptureRootSource::Default],
            }],
        };
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        let status = &capture.root_status[0];
        assert!(matches!(
            &status.state,
            RootReadStatus::Unavailable(failure)
                if failure.path == root
                    && failure.failure.kind != std::io::ErrorKind::NotFound
        ));
        assert!(matches!(
            status.cleanup.as_slice(),
            [CleanupRefusal::Access(failure)]
                if status.state == RootReadStatus::Unavailable(failure.clone())
        ));
    }

    #[test]
    fn missing_root_recovers_without_resolving_again() {
        let parent = tempdir().unwrap();
        let root = parent.path().join("later");
        let roots = CaptureRoots {
            roots: vec![CaptureRoot {
                path:    Ok(root.clone()),
                sources: vec![CaptureRootSource::Configuration {
                    entry: 0,
                    path:  root.clone(),
                }],
            }],
        };
        let first = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert!(
            matches!(&first.root_status[0].state, RootReadStatus::Unavailable(failure) if failure.path == root && failure.failure.kind == std::io::ErrorKind::NotFound)
        );
        fs::create_dir_all(root.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let second = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Unknown)
        });
        assert_eq!(second.root_status[0].state, RootReadStatus::Readable);
        assert!(second.root_status[0].diagnostics.is_empty());
        assert!(first.readings.is_empty() && second.readings.is_empty());
    }

    #[test]
    fn mixed_root_counts_readable_confirmed_publications_and_retains_other_artifacts() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        publish(&path, 10, "good", "100", CAPTURED_REDRAW);
        let legacy = path.join(CAPTURE_LIVE_RUNS_DIR).join("11");
        fs::write(&legacy, "/writer/project\tcargo build").unwrap();
        let (_, unreadable) = publish(&path, 12, "denied", "100", "");
        fs::remove_file(&unreadable).unwrap();
        symlink("missing-target", &unreadable).unwrap();
        let (staging, _) = stage(&path, 13, "pending", "100");
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        let status = &capture.root_status[0];
        assert_eq!(capture.confirmed.len(), 2);
        assert_eq!(status.confirmed, 1);
        assert!(
            status
                .diagnostics
                .contains(&CaptureDiagnostic::AnnotationOnly(legacy))
        );
        assert!(
            status
                .diagnostics
                .contains(&CaptureDiagnostic::Staging(staging))
        );
        assert!(status.diagnostics.iter().any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::LogUnreadable(failure) if failure.path == unreadable)));
    }

    #[test]
    fn unreadable_log_without_process_rows_recovers_on_next_scan() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let (_, log) = publish(&path, 10, "live", "100", "");
        fs::remove_file(&log).unwrap();
        symlink("unreadable", &log).unwrap();
        let first = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(first.root_status[0].confirmed, 0);
        assert!(first.root_status[0].diagnostics.iter().any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::LogUnreadable(failure) if failure.path == log)));
        fs::remove_file(&log).unwrap();
        fs::write(&log, CAPTURED_REDRAW).unwrap();
        let second = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(second.root_status[0].confirmed, 1);
        assert!(second.root_status[0].diagnostics.is_empty());
    }

    #[test]
    fn legacy_and_versioned_records_report_one_shared_unreadable_log() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let (registration, log) = publish(&path, 42, "live", "100", "");
        let legacy = path.join(CAPTURE_LIVE_RUNS_DIR).join("42");
        fs::write(&legacy, "/writer/project\tcargo build").unwrap();
        fs::remove_file(&log).unwrap();
        symlink("unreadable", &log).unwrap();

        let capture = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        let diagnostics = &capture.root_status[0].diagnostics;
        let failures: Vec<_> = diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                CaptureDiagnostic::LogUnreadable(failure) => Some(failure),
                _ => None,
            })
            .collect();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].path, log);
        assert!(diagnostics.contains(&CaptureDiagnostic::AnnotationOnly(legacy.clone())));
        assert!(diagnostics.contains(&CaptureDiagnostic::IdentityUnknown(registration.clone())));
        assert!(matches!(
            capture.lookup(0, 42),
            CaptureLookup::Registered(CaptureRead::Unreadable(_))
        ));
        assert!(registration.exists() && legacy.exists() && log.is_symlink());
    }

    #[test]
    fn distinct_unreadable_logs_for_one_pid_keep_separate_diagnostics() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let logs: Vec<_> = ["first", "second"]
            .into_iter()
            .map(|generation| {
                let (_, log) = publish(&path, 42, generation, "100", "");
                fs::remove_file(&log).unwrap();
                symlink("unreadable", &log).unwrap();
                log
            })
            .collect();
        let capture = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        let failures: Vec<_> = capture.root_status[0]
            .diagnostics
            .iter()
            .filter_map(|diagnostic| match diagnostic {
                CaptureDiagnostic::LogUnreadable(failure) => Some(failure.path.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(failures, logs);
    }

    #[test]
    fn incomplete_registration_reads_disable_cleanup_after_complete_enumeration() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let registration = path.join(CAPTURE_LIVE_RUNS_DIR).join("10.invalid");
        symlink("missing", &registration).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        let status = &capture.root_status[0];
        assert!(
            status
                .cleanup
                .contains(&CleanupRefusal::RegistrationIncomplete(
                    path.join(CAPTURE_LIVE_RUNS_DIR)
                ))
        );
        assert!(status.diagnostics.iter().any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::RegistrationUnreadable(failure) if failure.path == registration)));
        assert!(!status.diagnostics.iter().any(|diagnostic| matches!(
            diagnostic,
            CaptureDiagnostic::EnumerationIncomplete(_) | CaptureDiagnostic::EnumerationFailed(_)
        )));
    }

    #[test]
    fn unreadable_staging_retains_itself_without_disabling_published_pair_cleanup() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let staging = path.join(CAPTURE_LIVE_RUNS_DIR).join("11.pending.tmp");
        symlink("unreadable", &staging).unwrap();
        let (registration, log) = publish(&path, 10, "ended", "100", "");
        let capture = Capture::take_with_observations(root.path(), |_| Observation::Ended);
        assert!(staging.is_symlink());
        assert!(!registration.exists());
        assert!(!log.exists());
        assert!(capture.root_status[0].cleanup.is_empty());
        assert!(
            capture.root_status[0]
                .diagnostics
                .contains(&CaptureDiagnostic::Staging(staging.clone()))
        );
        assert!(capture.root_status[0].diagnostics.iter().any(|diagnostic| matches!(diagnostic, CaptureDiagnostic::RegistrationUnreadable(failure) if failure.path == staging)));
    }

    #[test]
    fn permanent_missing_identity_and_temporary_kernel_uncertainty_are_distinct() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let (permanent, _) = publish(&path, 10, "permanent", "", "");
        let (temporary, _) = publish(&path, 11, "temporary", "100", "");
        let mut capture = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        capture.record_boot_verification(Ok(()));
        assert_eq!(capture.root_status[0].confirmed, 0);
        assert!(
            capture.root_status[0]
                .diagnostics
                .contains(&CaptureDiagnostic::Unverifiable(permanent))
        );
        assert!(
            capture.root_status[0]
                .diagnostics
                .contains(&CaptureDiagnostic::IdentityUnknown(temporary))
        );
    }

    #[test]
    fn cached_boot_failure_removes_next_scan_retry_from_retained_records() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let (registration, log) = publish(&path, 10, "live", "100", CAPTURED_REDRAW);
        let (unverifiable, _) = publish(&path, 11, "no-identity", "", "");
        symlink(
            "unreadable",
            path.join(CAPTURE_LIVE_RUNS_DIR).join("12.denied"),
        )
        .unwrap();
        let boot_failure = PathFailure {
            path:    path.join("kernel-boot"),
            failure: std::io::Error::from(std::io::ErrorKind::PermissionDenied).into(),
        };

        for _ in 0..2 {
            let mut capture =
                Capture::take_with_observations(root.path(), |_| Observation::Unknown);
            let cleanup = capture.root_status[0].cleanup.clone();
            assert!(!cleanup.is_empty());
            let reading = capture.lookup(0, 10);
            capture.record_boot_verification(Err(boot_failure.clone()));

            let status = &capture.root_status[0];
            assert!(
                status
                    .diagnostics
                    .contains(&CaptureDiagnostic::IdentityBlockedByBoot(
                        registration.clone()
                    ))
            );
            assert!(
                status
                    .diagnostics
                    .contains(&CaptureDiagnostic::BootUnavailable(boot_failure.clone()))
            );
            assert!(
                status
                    .diagnostics
                    .contains(&CaptureDiagnostic::Unverifiable(unverifiable.clone()))
            );
            assert!(
                !status.diagnostics.iter().any(|diagnostic| {
                    matches!(diagnostic, CaptureDiagnostic::IdentityUnknown(_))
                })
            );
            assert_eq!(status.cleanup, cleanup);
            assert_eq!(capture.lookup(0, 10), reading);
            assert_eq!(status.confirmed, 0);
            assert!(capture.confirmed().is_empty());
            assert!(registration.exists() && log.exists() && unverifiable.exists());
        }
    }

    /// The fixture owns every path and does not change process-global environment.
    fn capture_root() -> TempDir {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        for path in [
            root.path().to_owned(),
            root.path().join("state"),
            root.path().join(CAPTURE_LIVE_RUNS_DIR),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        root
    }

    /// Keep the writer format visible in fixtures instead of accepting empty marker files.
    fn record(generation: &str, pid: u32, birth: &str) -> Vec<u8> {
        let log = format!("run-{generation}-{pid}.log");
        [
            "cargo-tile-v2",
            generation,
            "boot",
            birth,
            &log,
            "/writer/project",
            "/writer",
            "1",
            "build",
            "",
        ]
        .join("\0")
        .into_bytes()
    }

    /// Publish one registration and the exact log named in its body.
    fn publish(
        root: &Path,
        pid: u32,
        generation: &str,
        birth: &str,
        output: &str,
    ) -> (PathBuf, PathBuf) {
        let registration = root
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join(format!("{pid}.{generation}"));
        let log = root.join(format!("run-{generation}-{pid}.log"));
        fs::write(&registration, record(generation, pid, birth)).unwrap();
        fs::write(&log, output).unwrap();
        (registration, log)
    }

    /// Leave the complete record at the shim's pre-publication staging basename.
    fn stage(root: &Path, pid: u32, generation: &str, birth: &str) -> (PathBuf, PathBuf) {
        let (registration, log) = publish(root, pid, generation, birth, CAPTURED_REDRAW);
        let staging = registration.with_file_name(format!("{pid}.{generation}.tmp"));
        fs::rename(registration, &staging).unwrap();
        (staging, log)
    }

    /// Inject complete kernel evidence without depending on host pid allocation.
    fn present(birth: &str) -> Observation {
        match crate::birth_stamp::BirthStamp::from_fields("boot", birth) {
            crate::birth_stamp::IdentityEvidence::Available(stamp) => Observation::Present(stamp),
            crate::birth_stamp::IdentityEvidence::Unavailable => Observation::Unknown,
        }
    }

    /// Resolve one fixture's root and pid without inventing generation evidence.
    fn capture_key(capture: &Capture, root: usize, pid: u32) -> CaptureKey {
        capture
            .keys(pid)
            .find(|key| key.root == CaptureRootIndex(root))
            .unwrap()
    }

    #[test]
    fn unset_root_environment_selects_default_source() {
        let roots = CaptureRoots::resolve_environment(&[], None.into());
        assert_eq!(roots.roots.len(), 1);
        assert_eq!(roots.roots[0].sources, [CaptureRootSource::Default]);
        let path = roots.roots[0].path.as_ref().unwrap();
        assert_eq!(path.file_name(), Path::new(CAPTURE_ROOT).file_name());
        assert_eq!(
            path.parent().unwrap(),
            Path::new(CAPTURE_ROOT)
                .parent()
                .unwrap()
                .canonicalize()
                .unwrap()
        );
    }

    #[test]
    fn empty_root_environment_selects_default_source() {
        assert_eq!(
            CaptureRoots::resolve_environment(&[], Some(OsString::new()).into()),
            CaptureRoots::resolve_environment(&[], None.into()),
        );
    }

    #[test]
    fn nonempty_root_environment_selects_environment_source() {
        let root = capture_root();
        let path = root.path().to_path_buf();
        let roots =
            CaptureRoots::resolve_environment(&[], Some(path.clone().into_os_string()).into());
        assert_eq!(roots.roots.len(), 1);
        assert_eq!(
            roots.roots[0].path.as_ref().unwrap(),
            &path.canonicalize().unwrap()
        );
        assert_eq!(
            roots.roots[0].sources,
            [CaptureRootSource::Environment { path }]
        );
    }

    #[test]
    fn config_without_roots_keeps_exactly_the_readers_own_root() {
        let config: crate::config::Config =
            toml::from_str("[capture]\nauto_install = false\n").unwrap();
        let root = capture_root();
        let roots = CaptureRoots::resolve_environment(
            &config.capture.roots,
            CaptureRootEnvironment::Override(root.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 1);
        assert_eq!(
            roots.roots[0].path.as_ref().unwrap(),
            &root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn missing_configured_root_retains_its_source_and_path() {
        let own = capture_root();
        let configured = own.path().join("not-created");
        let roots = CaptureRoots::resolve_environment(
            std::slice::from_ref(&configured),
            CaptureRootEnvironment::Override(own.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 2);
        assert_eq!(
            roots.roots[1].path.as_ref().unwrap(),
            &own.path().canonicalize().unwrap().join("not-created")
        );
        assert_eq!(
            roots.roots[1].sources,
            [CaptureRootSource::Configuration {
                entry: 0,
                path:  configured,
            }]
        );
    }

    #[test]
    fn relative_configured_root_retains_a_validation_failure() {
        let own = capture_root();
        let configured = PathBuf::from("runner/cargo-tile");
        let roots = CaptureRoots::resolve_environment(
            std::slice::from_ref(&configured),
            CaptureRootEnvironment::Override(own.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 2);
        assert_eq!(
            roots.roots[1].path.as_ref().unwrap_err().kind,
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            roots.roots[1].sources,
            [CaptureRootSource::Configuration {
                entry: 0,
                path:  configured,
            }]
        );
    }

    #[test]
    fn deduplication_preserves_every_configured_spelling() {
        let own = capture_root();
        let parent = tempdir().unwrap();
        let real = parent.path().join("real");
        let alias = parent.path().join("alias");
        fs::create_dir(&real).unwrap();
        symlink(&real, &alias).unwrap();
        let configured = [alias.join("capture"), real.join("capture")];
        let roots = CaptureRoots::resolve_environment(
            &configured,
            CaptureRootEnvironment::Override(own.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 2);
        assert_eq!(
            roots.roots[1].path.as_ref().unwrap(),
            &real.canonicalize().unwrap().join("capture")
        );
        assert_eq!(
            roots.roots[1].sources,
            [
                CaptureRootSource::Configuration {
                    entry: 0,
                    path:  configured[0].clone(),
                },
                CaptureRootSource::Configuration {
                    entry: 1,
                    path:  configured[1].clone(),
                },
            ]
        );
    }

    #[test]
    fn trailing_parent_deduplicates_a_populated_root_and_preserves_both_sources() {
        let root = capture_root();
        publish(root.path(), 10, "live", "100", CAPTURED_REDRAW);
        let configured = root.path().join("state/..");
        let roots = CaptureRoots::resolve_environment(
            std::slice::from_ref(&configured),
            CaptureRootEnvironment::Override(root.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 1);
        assert_eq!(
            roots.roots[0].path.as_ref().unwrap(),
            &root.path().canonicalize().unwrap()
        );
        assert_eq!(
            roots.roots[0].sources,
            [
                CaptureRootSource::Environment {
                    path: root.path().to_owned(),
                },
                CaptureRootSource::Configuration {
                    entry: 0,
                    path:  configured,
                },
            ]
        );
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(capture.readings.len(), 1);
        assert_eq!(capture.confirmed().len(), 1);
        assert_eq!(capture.confirmed()[0].key, capture_key(&capture, 0, 10));
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
    }

    #[test]
    fn trailing_parents_resolve_ancestor_symlinks_before_deduplication() {
        let root = capture_root();
        let parent = tempdir().unwrap();
        let alias = parent.path().join("alias");
        symlink(root.path().join(CAPTURE_LIVE_RUNS_DIR), &alias).unwrap();
        let roots = CaptureRoots::resolve_environment(
            &[alias.join("../..")],
            CaptureRootEnvironment::Override(root.path().to_owned()),
        );
        assert_eq!(roots.roots.len(), 1);
        assert_eq!(
            roots.roots[0].path.as_ref().unwrap(),
            &root.path().canonicalize().unwrap()
        );
    }

    #[test]
    fn unresolved_ancestors_retain_configured_roots_including_trailing_parents() {
        let own = capture_root();
        for suffix in [
            "not-created/capture",
            "not-created/..",
            "not-created/state/../..",
        ] {
            let configured = own.path().join(suffix);
            let roots = CaptureRoots::resolve_environment(
                std::slice::from_ref(&configured),
                CaptureRootEnvironment::Override(own.path().to_owned()),
            );
            assert_eq!(roots.roots.len(), 2);
            assert_eq!(roots.roots[1].path.as_ref().unwrap(), &configured);
            assert_eq!(
                roots.roots[1].sources,
                [CaptureRootSource::Configuration {
                    entry: 0,
                    path:  configured,
                }]
            );
        }
    }

    #[test]
    fn configured_final_symlink_never_becomes_a_scan_capability() {
        let own = capture_root();
        let target = capture_root();
        let (registration, log) = publish(target.path(), 10, "ended", "100", "");
        let alias = own.path().join("alias");
        symlink(target.path(), &alias).unwrap();
        let roots = CaptureRoots::resolve_environment(
            &[alias],
            CaptureRootEnvironment::Override(own.path().to_owned()),
        );
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Ended)
        });
        assert!(capture.readings.is_empty());
        assert!(registration.exists());
        assert!(log.exists());
    }

    #[test]
    fn same_pid_in_two_roots_keeps_both_readings_and_confirmations() {
        let first = capture_root();
        let second = capture_root();
        publish(first.path(), 10, "first", "100", CAPTURED_REDRAW);
        let (registration, _) = publish(second.path(), 10, "second", "100", CAPTURED_TALLY);
        let record = String::from_utf8(record("second", 10, "100"))
            .unwrap()
            .replace("/writer/project", "/runner/worktree");
        fs::write(registration, record).unwrap();
        let roots = CaptureRoots::resolve_environment(
            &[second.path().to_owned()],
            CaptureRootEnvironment::Override(first.path().to_owned()),
        );
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(capture.readings.len(), 2);
        assert_eq!(capture.confirmed().len(), 2);
        assert_eq!(
            capture.keys(10).collect::<Vec<_>>(),
            [capture_key(&capture, 0, 10), capture_key(&capture, 1, 10)]
        );
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert_eq!(
            capture.lookup(1, 10),
            CaptureLookup::Registered(CaptureRead::Progress(testing(11, 24)))
        );
        for (index, expected) in [
            (0, ("run-first-10.log", Path::new("/writer/project"))),
            (1, ("run-second-10.log", Path::new("/runner/worktree"))),
        ] {
            let confirmed = capture
                .confirmed()
                .iter()
                .find(|confirmed| confirmed.key == capture_key(&capture, index, 10))
                .unwrap();
            assert_eq!(confirmed.registration.record().log_basename(), expected.0);
            assert_eq!(
                confirmed.registration.record().directory_identity(),
                crate::registration::WorkingDirectoryIdentity::Absolute(expected.1.to_owned())
            );
        }
    }

    #[test]
    fn taking_two_resolved_roots_shares_one_removal_allowance() {
        let first = capture_root();
        let second = capture_root();
        let pairs = CAPTURE_SWEEP_LIMIT / 2;
        for index in 0..pairs {
            publish(first.path(), 10, &format!("first-{index}"), "100", "");
            publish(second.path(), 20, &format!("second-{index}"), "100", "");
        }
        let roots = CaptureRoots::resolve_environment(
            &[second.path().to_owned()],
            CaptureRootEnvironment::Override(first.path().to_owned()),
        );
        Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, Observation::Ended)
        });
        assert_eq!(
            fs::read_dir(first.path().join(CAPTURE_LIVE_RUNS_DIR))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            fs::read_dir(second.path().join(CAPTURE_LIVE_RUNS_DIR))
                .unwrap()
                .count(),
            pairs
        );
        assert_eq!(fs::read_dir(second.path()).unwrap().count(), pairs + 1);
    }

    #[test]
    fn each_lookup_and_read_outcome_remains_distinguishable() {
        let root = capture_root();
        publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
        publish(root.path(), 11, "two", "100", "");
        let (_, missing) = publish(root.path(), 12, "three", "100", "");
        fs::remove_file(missing).unwrap();
        publish(
            root.path(),
            13,
            "four",
            "100",
            &format!(
                "{CAPTURED_REDRAW}\n    Finished dev profile target(s) in 0.1s\napplication still running"
            ),
        );
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(capture.lookup(0, 9), CaptureLookup::Unregistered);
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert_eq!(
            capture.lookup(0, 11),
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
        assert!(matches!(
            capture.lookup(0, 12),
            CaptureLookup::Registered(CaptureRead::Unreadable(CaptureFailure {
                kind: std::io::ErrorKind::NotFound,
                ..
            }))
        ));
        assert_eq!(
            capture.lookup(0, 13),
            CaptureLookup::Registered(CaptureRead::NoCurrentProgress)
        );
        assert_eq!(capture.confirmed().len(), 4);
    }

    #[test]
    fn an_ended_registration_and_its_exact_log_are_removed_together() {
        let root = capture_root();
        let (registration, log) = publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
        let orphan = root.path().join("run-unregistered-10.log");
        fs::write(&orphan, CAPTURED_REDRAW).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("101"));
        assert_eq!(capture.lookup(0, 10), CaptureLookup::Unregistered);
        assert!(capture.confirmed().is_empty());
        assert!(!registration.exists());
        assert!(!log.exists());
        assert!(orphan.exists());
    }

    #[test]
    fn unknown_and_unnormalized_identity_never_authorize_deletion() {
        for (birth, observation) in [
            ("", Observation::Ended),
            ("Thu Sep 10 01:00:00 2026", Observation::Ended),
            ("100", Observation::Unknown),
        ] {
            let root = capture_root();
            let (registration, log) = publish(root.path(), 10, "one", birth, CAPTURED_REDRAW);
            let capture = Capture::take_with_observations(root.path(), |_| observation.clone());
            assert!(registration.exists());
            assert!(log.exists());
            assert!(capture.confirmed().is_empty());
        }
    }

    #[test]
    fn legacy_records_annotate_but_never_supply_verified_rows() {
        let root = capture_root();
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10");
        fs::write(&registration, "/writer/project\tcargo build").unwrap();
        let log = root.path().join("run-legacy-10.log");
        fs::write(&log, CAPTURED_REDRAW).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| Observation::Ended);
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert!(capture.confirmed().is_empty());
        assert!(registration.exists());
        assert!(log.exists());
    }

    #[test]
    fn malformed_records_do_not_discard_readable_siblings() {
        let root = capture_root();
        publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
        let (malformed, _) = publish(root.path(), 11, "two", "100", "");
        fs::write(malformed, b"cargo-tile-v2\0truncated").unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert_eq!(capture.lookup(0, 11), CaptureLookup::Unregistered);
    }

    #[test]
    fn a_disappearing_registration_preserves_sibling_readings_and_disables_cleanup() {
        let root = capture_root();
        for pid in 10..13 {
            publish(root.path(), pid, "one", "100", CAPTURED_REDRAW);
        }
        let stale = publish(root.path(), 13, "stale", "99", "");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        fs::remove_file(root.path().join(CAPTURE_LIVE_RUNS_DIR).join("11.one")).unwrap();
        let mut capture = Capture::default();
        let mut budget = SweepBudget::default();
        capture.scan_with_observations(&scan, &|_| present("100"), &mut budget);
        for pid in [10, 12] {
            assert_eq!(
                capture.lookup(0, pid),
                CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
            );
        }
        assert_eq!(capture.lookup(0, 11), CaptureLookup::Unregistered);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
        assert!(stale.0.exists());
        assert!(stale.1.exists());
    }

    #[test]
    fn named_log_published_after_enumeration_is_read_and_never_swept() {
        let root = capture_root();
        let (_, log) = publish(root.path(), 10, "one", "100", "");
        fs::remove_file(&log).unwrap();
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        assert!(
            !scan
                .log_entries()
                .any(|entry| entry.name() == log.file_name().unwrap())
        );
        let mut capture = Capture::default();
        capture.scan_with_observations(
            &scan,
            &|_| {
                fs::write(&log, CAPTURED_REDRAW).unwrap();
                present("100")
            },
            &mut SweepBudget::default(),
        );
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert!(log.exists());
    }

    #[test]
    fn a_missing_named_log_never_borrows_an_older_generations_output() {
        let root = capture_root();
        let (_, log) = publish(root.path(), 10, "new", "100", "");
        fs::remove_file(&log).unwrap();
        fs::write(root.path().join("run-older-10.log"), CAPTURED_REDRAW).unwrap();
        assert!(matches!(
            Capture::take_with_observations(root.path(), |_| present("100")).lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Unreadable(_))
        ));
        fs::write(&log, CAPTURED_TALLY).unwrap();
        assert_eq!(
            Capture::take_with_observations(root.path(), |_| present("100")).lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(testing(11, 24)))
        );
    }

    #[test]
    fn a_capture_keeps_its_reading_without_reopening_a_replaced_log() {
        let root = capture_root();
        let (_, log) = publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        fs::write(&log, CAPTURED_TALLY).unwrap();
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert_eq!(
            Capture::take_with_observations(root.path(), |_| present("100")).lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(testing(11, 24)))
        );
    }

    #[test]
    fn fresh_confirmation_or_uncertainty_overrules_an_earlier_ended_reading() {
        for fresh in [present("100"), Observation::Unknown] {
            let root = capture_root();
            let (registration, log) = publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
            let calls = std::cell::Cell::new(0);
            Capture::take_with_observations(root.path(), |_| {
                let count = calls.get();
                calls.set(count + 1);
                if count == 0 {
                    Observation::Ended
                } else {
                    fresh.clone()
                }
            });
            assert!(registration.exists());
            assert!(log.exists());
            assert_eq!(calls.get(), 2);
        }
    }

    #[test]
    fn publication_after_process_snapshot_is_verified_live() {
        for fresh in [present("100"), Observation::Unknown] {
            let root = capture_root();
            // No pid snapshot is an argument to Capture: this run can have started after it.
            let (registration, log) = publish(root.path(), 10, "new", "100", CAPTURED_REDRAW);
            Capture::take_with_observations(root.path(), |_| fresh.clone());
            assert!(registration.exists());
            assert!(log.exists());
        }
    }

    #[test]
    fn replacement_identity_preserves_both_republished_artifacts() {
        let root = capture_root();
        let (registration, log) = publish(root.path(), 10, "reused", "100", CAPTURED_REDRAW);
        Capture::take_with_observations(root.path(), |_| {
            fs::remove_file(&registration).unwrap();
            fs::remove_file(&log).unwrap();
            publish(root.path(), 10, "reused", "101", CAPTURED_TALLY);
            Observation::Ended
        });
        assert_eq!(
            fs::read(&registration).unwrap(),
            record("reused", 10, "101")
        );
        assert_eq!(fs::read_to_string(&log).unwrap(), CAPTURED_TALLY);
    }

    #[test]
    fn distinct_generation_survives_publication_after_final_identity_read() {
        let root = capture_root();
        let (old_registration, old_log) = publish(root.path(), 10, "old-uuid", "100", "");
        let calls = std::cell::Cell::new(0);
        Capture::take_with_observations(root.path(), |_| {
            let count = calls.get();
            calls.set(count + 1);
            if count == 1 {
                fs::remove_file(&old_registration).unwrap();
                fs::remove_file(&old_log).unwrap();
                publish(root.path(), 10, "new-uuid", "101", CAPTURED_TALLY);
            }
            Observation::Ended
        });
        assert!(
            root.path()
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join("10.new-uuid")
                .exists()
        );
        assert_eq!(
            fs::read_to_string(root.path().join("run-new-uuid-10.log")).unwrap(),
            CAPTURED_TALLY
        );
    }

    #[test]
    fn failed_log_unlink_retains_the_registration() {
        let root = capture_root();
        let (registration, log) = publish(root.path(), 10, "one", "100", "");
        fs::remove_file(&log).unwrap();
        fs::create_dir(&log).unwrap();
        Capture::take_with_observations(root.path(), |_| Observation::Ended);
        assert!(registration.exists());
        assert!(log.is_dir());
    }

    #[test]
    fn staging_inspections_leave_allowance_for_pairs_and_later_roots() {
        let first = capture_root();
        let second = capture_root();
        for index in 0..CAPTURE_SWEEP_LIMIT {
            fs::write(
                first
                    .path()
                    .join(CAPTURE_LIVE_RUNS_DIR)
                    .join(format!("{index}.retained.tmp")),
                "unpublished",
            )
            .unwrap();
        }
        for pass in 0..2 {
            let generation = format!("ended-{pass}");
            let pair = publish(first.path(), 10, &generation, "100", "");
            let next_pair = publish(second.path(), 20, &generation, "100", "");
            let mut budget = SweepBudget::default();
            for root in [first.path(), second.path()] {
                let scan = RootScan::open(root, &mut RootHistory::default()).unwrap();
                Capture::default().scan_with_observations(
                    &scan,
                    &|_| Observation::Ended,
                    &mut budget,
                );
            }
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 4);
            for path in [pair.0, pair.1, next_pair.0, next_pair.1] {
                assert!(!path.exists());
            }
        }
        assert_eq!(
            fs::read_dir(first.path().join(CAPTURE_LIVE_RUNS_DIR))
                .unwrap()
                .count(),
            CAPTURE_SWEEP_LIMIT
        );
    }

    #[test]
    fn complete_ended_staging_records_and_their_exact_logs_are_removed() {
        for observation in [Observation::Ended, present("101")] {
            let root = capture_root();
            let (staging, log) = stage(root.path(), 10, "one", "100");
            let unrelated_log = root.path().join("run-unregistered-10.log");
            fs::write(&unrelated_log, CAPTURED_TALLY).unwrap();
            let calls = std::cell::Cell::new(0);
            let capture = Capture::take_with_observations(root.path(), |pid| {
                assert_eq!(pid, 10);
                calls.set(calls.get() + 1);
                observation.clone()
            });
            assert_eq!(calls.get(), 3);
            assert_eq!(capture.lookup(0, 10), CaptureLookup::Unregistered);
            assert!(capture.confirmed().is_empty());
            assert!(!staging.exists());
            assert!(!log.exists());
            assert!(unrelated_log.exists());
        }
    }

    #[test]
    fn ended_staging_is_removed_even_if_setup_never_created_its_log() {
        let root = capture_root();
        let (staging, log) = stage(root.path(), 10, "one", "100");
        fs::remove_file(&log).unwrap();
        Capture::take_with_observations(root.path(), |_| Observation::Ended);
        assert!(!staging.exists());
        assert!(!log.exists());
    }

    #[test]
    fn retained_staging_never_supplies_capture_membership_or_spends_allowance() {
        for (birth, observation) in [
            ("100", present("100")),
            ("100", Observation::Unknown),
            ("", Observation::Ended),
            ("Thu Sep 10 01:00:00 2026", Observation::Ended),
        ] {
            let root = capture_root();
            let (staging, log) = stage(root.path(), 10, "one", birth);
            let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
            let mut budget = SweepBudget::default();
            let mut capture = Capture::default();
            let calls = std::cell::Cell::new(0);
            capture.scan_with_observations(
                &scan,
                &|_| {
                    calls.set(calls.get() + 1);
                    observation.clone()
                },
                &mut budget,
            );
            assert_eq!(calls.get(), 1);
            assert!(staging.exists());
            assert!(log.exists());
            assert_eq!(capture.lookup(0, 10), CaptureLookup::Unregistered);
            assert!(capture.confirmed().is_empty());
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
        }
    }

    #[test]
    fn staging_cleanup_requires_fresh_ended_evidence_before_each_unlink() {
        for fresh in [present("100"), Observation::Unknown] {
            for changes_after in [1, 2] {
                let root = capture_root();
                let (staging, log) = stage(root.path(), 10, "one", "100");
                let calls = std::cell::Cell::new(0);
                Capture::take_with_observations(root.path(), |_| {
                    let count = calls.get();
                    calls.set(count + 1);
                    if count < changes_after {
                        Observation::Ended
                    } else {
                        fresh.clone()
                    }
                });
                assert!(staging.exists());
                assert_eq!(log.exists(), changes_after == 1);
                assert_eq!(calls.get(), changes_after + 1);
            }
        }
    }

    #[test]
    fn a_replaced_staging_record_is_reread_before_any_unlink() {
        let root = capture_root();
        let (staging, log) = stage(root.path(), 10, "one", "100");
        let replacement = record("one", 10, "101");
        let calls = std::cell::Cell::new(0);
        Capture::take_with_observations(root.path(), |_| {
            calls.set(calls.get() + 1);
            fs::write(&staging, &replacement).unwrap();
            Observation::Ended
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(fs::read(&staging).unwrap(), replacement);
        assert!(log.exists());
    }

    #[test]
    fn malformed_or_mismatched_staging_records_never_authorize_cleanup() {
        for contents in [
            b"cargo-tile-v2\0truncated".to_vec(),
            record("different-generation", 10, "100"),
        ] {
            let root = capture_root();
            let (staging, log) = stage(root.path(), 10, "one", "100");
            fs::write(&staging, &contents).unwrap();
            let calls = std::cell::Cell::new(0);
            Capture::take_with_observations(root.path(), |_| {
                calls.set(calls.get() + 1);
                Observation::Ended
            });
            assert_eq!(calls.get(), 0);
            assert_eq!(fs::read(&staging).unwrap(), contents);
            assert!(log.exists());
        }
    }

    #[test]
    fn unknown_staging_inspections_leave_allowance_for_ended_pairs_across_roots() {
        let first = capture_root();
        let second = capture_root();
        for index in 0..CAPTURE_SWEEP_LIMIT {
            stage(first.path(), 10, &format!("retained-{index}"), "100");
        }
        let pair = stage(first.path(), 11, "ended", "100");
        let next_pair = publish(second.path(), 20, "ended", "100", "");
        let mut budget = SweepBudget::default();
        let calls = std::cell::Cell::new(0);
        for root in [first.path(), second.path()] {
            let scan = RootScan::open(root, &mut RootHistory::default()).unwrap();
            Capture::default().scan_with_observations(
                &scan,
                &|pid| {
                    if pid == 10 {
                        calls.set(calls.get() + 1);
                        Observation::Unknown
                    } else {
                        Observation::Ended
                    }
                },
                &mut budget,
            );
        }
        assert_eq!(calls.get(), CAPTURE_SWEEP_LIMIT);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 4);
        for path in [pair.0, pair.1, next_pair.0, next_pair.1] {
            assert!(!path.exists());
        }
        assert_eq!(
            fs::read_dir(first.path().join(CAPTURE_LIVE_RUNS_DIR))
                .unwrap()
                .count(),
            CAPTURE_SWEEP_LIMIT
        );
    }

    #[test]
    fn multiple_generations_keep_their_own_identity_and_log_associations() {
        let root = capture_root();
        let stale = publish(root.path(), 10, "stale", "99", CAPTURED_REDRAW);
        let live = publish(root.path(), 10, "live", "100", CAPTURED_TALLY);
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(testing(11, 24)))
        );
        assert_eq!(capture.confirmed().len(), 1);
        assert!(!stale.0.exists());
        assert!(!stale.1.exists());
        assert!(live.0.exists());
        assert!(live.1.exists());
    }

    #[test]
    fn incomplete_registration_inventory_preserves_all_artifacts() {
        let root = capture_root();
        let pair = publish(root.path(), 10, "one", "100", "");
        for index in 0..CAPTURE_INVENTORY_LIMIT {
            fs::write(
                root.path()
                    .join(CAPTURE_LIVE_RUNS_DIR)
                    .join(format!("unrelated-{index}")),
                "",
            )
            .unwrap();
        }
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).unwrap();
        assert!(matches!(
            scan.registration_outcome(),
            Enumeration::Incomplete
        ));
        let mut budget = SweepBudget::default();
        Capture::default().scan_with_observations(&scan, &|_| Observation::Ended, &mut budget);
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT);
        assert!(pair.0.exists());
        assert!(pair.1.exists());
    }

    #[test]
    fn an_unreadable_registration_disables_cleanup_but_not_sibling_progress() {
        let root = capture_root();
        publish(root.path(), 10, "one", "100", CAPTURED_REDRAW);
        let stale = publish(root.path(), 11, "stale", "99", "");
        symlink(
            "/dev/zero",
            root.path()
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join("12.unreadable"),
        )
        .unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(
            capture.lookup(0, 10),
            CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403)))
        );
        assert!(stale.0.exists());
        assert!(stale.1.exists());
    }

    #[test]
    fn unknown_roots_and_missing_registration_directories_never_sweep_logs() {
        let root = capture_root();
        let log = root.path().join("run-orphan-10.log");
        fs::write(&log, "").unwrap();
        fs::remove_dir(root.path().join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        assert_eq!(
            Capture::take_with_observations(root.path(), |_| Observation::Ended).lookup(0, 10),
            CaptureLookup::Unregistered
        );
        assert!(log.exists());
        assert_eq!(
            Capture::take_with_observations(&root.path().join("absent"), |_| Observation::Ended)
                .lookup(0, 10),
            CaptureLookup::Unregistered
        );
    }

    #[test]
    fn a_captured_redraw_reports_cargos_own_counts() {
        assert_eq!(parse_state(CAPTURED_REDRAW), Some(compiling(149, 403)));
    }

    #[test]
    fn the_last_redraw_in_the_tail_is_the_one_reported() {
        let tail = format!("{CAPTURED_REDRAW}Building [=>] 7/9: serde\r");
        assert_eq!(parse_state(&tail), Some(compiling(7, 9)));
    }

    #[test]
    fn a_download_counter_reports_the_phase_that_is_running() {
        assert_eq!(
            parse_state("Downloading [==>    ] 12/40: serde, regex\r"),
            Some(compiling(12, 40))
        );
    }

    #[test]
    fn a_test_runners_tally_is_not_a_counter() {
        assert_eq!(parse_state("PASS [   0.012s] 12/345 crate::suite"), None);
    }

    #[test]
    fn a_test_runners_own_bar_reports_the_tests_it_has_got_through() {
        assert_eq!(parse_state(CAPTURED_TEST_REDRAW), Some(testing(12, 24)));
    }

    /// The bar is redrawn above the tests in flight, so the counter is
    /// never the last thing in the log while the run is going.
    #[test]
    fn the_rows_drawn_under_a_test_bar_do_not_hide_its_counter() {
        let tail = format!("{CAPTURED_TEST_REDRAW}{CAPTURED_TEST_ROW}{CAPTURED_TEST_ROW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    /// What `cargo nextest run` does from end to end: cargo's units
    /// first, then the tests. Each phase is reported while it is the
    /// one running.
    #[test]
    fn a_test_run_reports_building_and_then_testing() {
        assert_eq!(parse_state(CAPTURED_REDRAW), Some(compiling(149, 403)));

        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    /// A run with no terminal under it -- a script, an agent, a CI job
    /// -- gets no bar from nextest at all, and the count it would have
    /// drawn there goes into every line it prints instead.
    #[test]
    fn a_tally_reports_the_tests_where_there_was_no_bar_to_draw() {
        assert_eq!(parse_state(CAPTURED_TALLY), Some(testing(11, 24)));
    }

    /// The first numbers of a run are padded out to the width of the
    /// total, blanks inside the parenthesis rather than in front of it.
    #[test]
    fn a_padded_tally_is_read_the_same_as_a_full_one() {
        assert_eq!(
            parse_state("        PASS [   1.022s] (  1/240) nxprobe t1\n"),
            Some(testing(1, 240))
        );
    }

    /// The build is what a run without a terminal reports until the
    /// tests start, cargo drawing the bar it is asked for either way.
    #[test]
    fn a_tally_after_a_build_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_TALLY}");

        assert_eq!(parse_state(&tail), Some(testing(11, 24)));
    }

    #[test]
    fn output_with_no_bar_in_it_reports_nothing() {
        assert_eq!(parse_state("   Compiling serde v1.0.0\n"), None);
    }

    #[test]
    fn a_counter_over_zero_units_is_rejected_rather_than_divided_by() {
        assert_eq!(parse_state("Building [ ] 0/0: \r"), None);
    }

    #[test]
    fn percent_rounds_down_so_only_a_finished_build_reads_full() {
        assert_eq!(
            Progress {
                done:  402,
                total: 403,
            }
            .percent(),
            99
        );
        assert_eq!(
            Progress {
                done:  403,
                total: 403,
            }
            .percent(),
            100
        );
    }

    #[test]
    fn a_run_waiting_on_the_build_directory_reports_that_it_is_blocked() {
        assert_eq!(parse_state(CAPTURED_WAIT), Some(RunState::Blocked));
    }

    /// Cargo counts its downloads before it reaches for the lock, so a
    /// blocked run has usually drawn a bar already. The bar is stale and
    /// the wait is not.
    #[test]
    fn a_wait_after_a_bar_is_what_the_run_is_doing() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_WAIT}");

        assert_eq!(parse_state(&tail), Some(RunState::Blocked));
    }

    /// The wait line is printed once and stays in the log, so a run that
    /// got its lock and started building must not still read as blocked.
    #[test]
    fn a_bar_after_a_wait_means_the_lock_came_free() {
        let tail = format!("{CAPTURED_WAIT}{CAPTURED_REDRAW}");

        assert_eq!(parse_state(&tail), Some(compiling(149, 403)));
    }

    /// A run with nothing to compile draws no bar at all, so there is no
    /// counter to weigh the wait against -- and what follows it is the
    /// output of the binary the run went on to start. That output is
    /// proof enough the lock came free.
    #[test]
    fn output_after_a_wait_means_the_lock_came_free_even_with_no_bar() {
        let tail = format!(
            "{CAPTURED_WAIT}    Finished `dev` profile [unoptimized + debuginfo] target(s) in \
             3.19s\n     Running `/rust/bevy_brp/target/debug/examples/extras_plugin`\nINFO \
             bevy_winit::system: Creating new window\n"
        );

        assert_eq!(parse_state(&tail), None);
    }

    /// Cargo takes the package cache under the same wording as the build
    /// directory and gives it straight back, so every command run beside
    /// another says this. It is not a wait anyone can see.
    #[test]
    fn a_wait_on_the_package_cache_is_not_a_state_worth_showing() {
        let tail = "    Blocking waiting for file lock on package cache\n";

        assert_eq!(parse_state(tail), None);
    }

    /// Cargo's closing line as the shim captures it, the profile in the
    /// hyperlink escape cargo wraps it in.
    const CAPTURED_FINISHED: &str = "\u{1b}[1m\u{1b}[92m    Finished\u{1b}[0m \
         \u{1b}]8;;https://doc.rust-lang.org/cargo/reference/profiles.html\u{1b}\\`dev` profile \
         [unoptimized + debuginfo]\u{1b}]8;;\u{1b}\\ target(s) in 1.49s\n";

    /// The bar is left on screen where it stopped, so the last redraw of
    /// a build that finished at `1/2` reads no differently from one still
    /// working through its second unit. A `cargo run` then lives on as
    /// the app it started, reporting 50% for hours.
    #[test]
    fn a_counter_a_finished_build_left_behind_is_not_a_reading() {
        let tail = format!("{CAPTURED_REDRAW}\n{CAPTURED_FINISHED}");

        assert_eq!(parse_state(&tail), None);
    }

    /// A test runner counts its own tests once the compiling is over, so
    /// a counter past cargo's `Finished` is the run's own and stands.
    #[test]
    fn a_counter_after_a_finished_build_is_the_test_runners() {
        let tail = format!("{CAPTURED_FINISHED}{CAPTURED_TEST_REDRAW}");

        assert_eq!(parse_state(&tail), Some(testing(12, 24)));
    }

    #[test]
    fn a_blocked_state_has_no_reading_to_draw() {
        assert_eq!(RunState::Blocked.working(), CounterState::Blocked);
    }

    #[test]
    fn a_log_file_is_keyed_by_the_shim_pid_its_name_ends_with() {
        assert_eq!(
            log_pid(Path::new("/tmp/cargo-tile/run-20260820-191029-33395.log")),
            Some(33395)
        );
        assert_eq!(log_pid(Path::new("/tmp/cargo-tile/pane-errors.log")), None);
    }
}
