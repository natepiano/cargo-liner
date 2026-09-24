//! Identity-keyed capture assembly, selection, and ended-pair cleanup.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;

use super::capture_diagnostic::CaptureDiagnostic;
use super::capture_diagnostic::CaptureFailure;
use super::capture_diagnostic::PathFailure;
use super::capture_read::CaptureLookup;
use super::capture_read::CaptureRead;
use super::capture_roots::AccountCaptureDirectory;
use super::capture_roots::AccountName;
use super::capture_roots::CaptureCleanup;
use super::capture_roots::CaptureParent;
#[cfg(test)]
use super::capture_roots::CaptureRoot;
use super::capture_roots::CaptureRoots;
use super::capture_roots::RootReadStatus;
use super::registered_runs;
use super::registered_runs::RegisteredRun;
use super::registered_runs::RegisteredRuns;
use super::registered_runs::RegistrationName;
use crate::birth_stamp;
use crate::birth_stamp::IdentityEvidence;
use crate::birth_stamp::KernelObservation;
use crate::census::DirectAssociation;
use crate::constants::PID_SEPARATOR;
use crate::constants::RUN_LOG_PREFIX;
use crate::constants::RUN_LOG_SUFFIX;
use crate::registration::Registration;
use crate::registration::RegistrationVerification;
use crate::registration::VerifiedRegistration;
use crate::root_scan;
use crate::root_scan::Enumeration;
use crate::root_scan::RootHistory;
use crate::root_scan::RootIncarnation;
use crate::root_scan::RootOwner;
use crate::root_scan::RootScan;
use crate::root_scan::SharedCaptureDirectory;
use crate::root_scan::SweepBudget;
use crate::root_scan::SweepDisposition;

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
    /// Assigned on first account discovery, without retaining cleanup authority.
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
    Ambiguous(Vec<CaptureKey>),
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

/// Capture observations from one scan; lookups never reopen a pathname.
#[derive(Default)]
pub(crate) struct Capture {
    /// Absence from this map differs from an accepted but unreadable capture.
    readings:                    BTreeMap<CaptureKey, CaptureRead>,
    /// Only identity-confirmed records can supply future registration-sourced rows.
    confirmed:                   Vec<ConfirmedCapture>,
    /// One observation per effective root, including missing and invalid paths.
    pub(crate) root_status:      Vec<AccountCaptureDirectory>,
    pub(crate) shared_directory: SharedCaptureDirectory,
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
        let roots = CaptureRoots::for_test(&[root]);
        Self::take_roots(&roots, &observe)
    }

    /// One removal allowance covers every owned root in this pass.
    pub(crate) fn take_roots(
        roots: &CaptureRoots,
        observe: &impl Fn(u32) -> KernelObservation,
    ) -> Self {
        let shared_directory = match &roots.parent {
            CaptureParent::Shared(parent) => {
                let _ = root_scan::prepare_shared_directory(parent);
                SharedCaptureDirectory::inspect(parent)
            },
            #[cfg(test)]
            CaptureParent::IsolatedAccounts => SharedCaptureDirectory::default(),
        };
        let mut capture = Self {
            shared_directory,
            ..Self::default()
        };
        let mut budget = SweepBudget::default();
        let users = sysinfo::Users::new_with_refreshed_list();
        for (index, mut status) in roots.discover(&users).into_iter().enumerate() {
            if matches!(status.state, RootReadStatus::ForeignOwned { .. }) {
                capture.root_status.push(status);
                continue;
            }
            let path = &status.root.path;
            match ROOT_HISTORY.with_borrow_mut(|history| RootScan::open(path, history)) {
                Err(error) => {
                    let failure = PathFailure {
                        path:    path.clone(),
                        failure: error.into(),
                    };
                    status.state = RootReadStatus::Unavailable(failure);
                },
                Ok(scan) => {
                    status.state = RootReadStatus::Readable;
                    status.owner = scan.owner();
                    // The descriptor must still belong to the account checked at discovery.
                    if status.owner != RootOwner::Uid(status.root.uid) {
                        status.state = RootReadStatus::ForeignOwned {
                            owner: AccountName::resolve(status.owner, &users),
                        };
                        capture.root_status.push(status);
                        continue;
                    }
                    if let Enumeration::Failed(error) = scan.registration_outcome() {
                        status.state = RootReadStatus::Unavailable(PathFailure {
                            path:    scan.registration_path(),
                            failure: std::io::Error::new(error.kind(), error.to_string()).into(),
                        });
                    }
                    capture.scan_root(
                        CaptureRootIndex(index),
                        &scan,
                        observe,
                        &mut budget,
                        &mut status,
                    );
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
        status: &mut AccountCaptureDirectory,
    ) {
        let RegisteredRuns {
            generations,
            diagnostics,
        } = registered_runs::registered_runs(scan, observe);
        status.diagnostics = diagnostics;
        let mut logs = BTreeMap::new();
        for (&pid, runs) in &generations {
            for run in runs {
                if matches!(
                    registered_runs::registration_name(&run.name),
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
            registered_runs::enumeration_diagnostic(
                outcome,
                scan.path().to_owned(),
                &mut status.diagnostics,
            );
        }
        if status.root.cleanup == CaptureCleanup::Here {
            sweep_ended(scan, &generations, observe, budget);
        }
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
                return CaptureSelection::Ambiguous(
                    self.readings
                        .keys()
                        .filter(|key| {
                            key.pid == pid
                                && key.root == root
                                && matches!(key.generation, CaptureGeneration::Published(_))
                        })
                        .cloned()
                        .collect(),
                );
            }
            return CaptureSelection::Selected(confirmed_capture.key.clone());
        }
        match (candidates.next(), candidates.next()) {
            (Some(key), None) => CaptureSelection::Selected(key.clone()),
            (Some(_), Some(_)) => CaptureSelection::Ambiguous(
                self.readings
                    .keys()
                    .filter(|key| key.pid == pid && key.root == root)
                    .cloned()
                    .collect(),
            ),
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

    /// Retained reading pids support selection reporting even without any process row.
    pub(crate) fn registered_pids(&self) -> BTreeSet<u32> {
        self.readings.keys().map(|key| key.pid).collect()
    }

    /// Only the selected, confirmed publication grants registration row ownership.
    pub(crate) fn row_source(&self, pid: u32) -> DirectAssociation {
        let CaptureSelection::Selected(key) = self.select(pid) else {
            return DirectAssociation::None;
        };
        self.confirmed
            .iter()
            .find(|confirmed| confirmed.key == key)
            .map_or(DirectAssociation::None, |confirmed| {
                DirectAssociation::Direct(Box::new(confirmed.into()))
            })
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
            registered_runs::registration_name(entry.name())
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
        .filter(|entry| log_pid(entry.name()) == LogWriter::Identified(pid))
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

/// Whether a log basename identifies its shim writer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LogWriter {
    /// The expected log framing ends with a representable writer pid.
    Identified(u32),
    /// Missing, unrelated, non-Unicode, or malformed names identify no writer.
    Unidentified,
}

/// The shim pid a log file is named for: `run-<generation>-<pid>.log`.
fn log_pid(path: &Path) -> LogWriter {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return LogWriter::Unidentified;
    };
    let Some(name) = name
        .strip_prefix(RUN_LOG_PREFIX)
        .and_then(|name| name.strip_suffix(RUN_LOG_SUFFIX))
    else {
        return LogWriter::Unidentified;
    };
    name.rsplit(PID_SEPARATOR)
        .next()
        .and_then(|pid| pid.parse().ok())
        .map_or(LogWriter::Unidentified, LogWriter::Identified)
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::*;
    use crate::birth_stamp::IdentityEvidence;
    use crate::birth_stamp::Observation;
    use crate::census::DirectAssociation;
    use crate::census::command_text::CommandText;
    use crate::census::scan::CensusSequence;
    use crate::census::scan::ProcessObservation;
    use crate::census::scan::ProcessObservations;
    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::constants::SUPPORTED_REGISTRATION_VERSION;
    use crate::progress::Progress;
    use crate::progress::capture_read::Phase;
    use crate::progress::capture_read::RunState;
    use crate::progress::capture_roots::AccountName;
    use crate::render::CounterState;
    use crate::root_scan::EffectiveUser;

    #[test]
    fn row_source_requires_the_selected_proof_and_survives_timestamp_or_log_failure() {
        let root = capture_root();
        publish(root.path(), 10, "first", "100", CAPTURED_REDRAW);
        let mut capture = Capture::take_with_observations(root.path(), |_| present("100"));
        capture.confirmed[0].modified =
            Err(std::io::Error::from(ErrorKind::PermissionDenied).into());
        let DirectAssociation::Direct(source) = capture.row_source(10) else {
            panic!("timestamp failure retains proof");
        };
        assert_eq!(source.registration().pid(), 10);
        let log = root.path().join("run-first-10.log");
        fs::remove_file(&log).unwrap();
        fs::create_dir(&log).unwrap();
        let unreadable = Capture::take_with_observations(root.path(), |_| present("100"));
        assert_eq!(unreadable.root_status[0].confirmed, 0);
        assert!(matches!(
            unreadable.row_source(10),
            crate::census::DirectAssociation::Direct(_)
        ));
        let unknown = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        assert!(matches!(
            unknown.row_source(10),
            crate::census::DirectAssociation::None
        ));
        publish(root.path(), 10, "second", "100", CAPTURED_WAIT);
        fs::write(
            root.path().join(CAPTURE_LIVE_RUNS_DIR).join("10"),
            "/writer/project\tcargo build",
        )
        .unwrap();
        let ambiguous = Capture::take_with_observations(root.path(), |_| present("100"));
        assert!(matches!(
            ambiguous.row_source(10),
            crate::census::DirectAssociation::None
        ));
        assert!(
            matches!(ambiguous.select(10), CaptureSelection::Ambiguous(keys) if keys.len() == 2 && keys.iter().all(|key| matches!(key.generation, CaptureGeneration::Published(_))))
        );
        assert!(matches!(
            ambiguous.row_source(11),
            crate::census::DirectAssociation::None
        ));
    }

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
            assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
            for reverse in [false, true] {
                for confirmed in &mut capture.confirmed {
                    let current = confirmed.registration.record().generation() == replacement;
                    confirmed.modified = Ok(SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(u64::from(current != reverse)));
                }
                assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
            }
            capture.confirmed.reverse();
            for confirmed in &mut capture.confirmed {
                confirmed.modified = Err(std::io::Error::from(ErrorKind::PermissionDenied).into());
            }
            assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
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
                assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
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
        assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
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
        assert!(matches!(
            competing.select(10),
            CaptureSelection::Ambiguous(_)
        ));
    }

    #[test]
    fn preferred_root_ambiguity_never_falls_through_to_another_roots_proof() {
        let preferred = capture_root();
        let other = capture_root();
        publish(preferred.path(), 10, "first", "100", CAPTURED_REDRAW);
        publish(preferred.path(), 10, "second", "100", CAPTURED_WAIT);
        publish(other.path(), 10, "third", "100", CAPTURED_TALLY);
        let roots = CaptureRoots::for_test(&[preferred.path(), other.path()]);
        let capture = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(capture.confirmed().len(), 3);
        assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
    }

    #[test]
    fn preferred_unconfirmed_root_blocks_a_later_roots_unique_confirmation() {
        let preferred = capture_root();
        let other = capture_root();
        publish(preferred.path(), 10, "unknown", "", CAPTURED_WAIT);
        publish(other.path(), 10, "confirmed", "100", CAPTURED_REDRAW);
        let roots = CaptureRoots::for_test(&[preferred.path(), other.path()]);
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
        assert_eq!(
            CounterState::from(&reading),
            CounterState::NoCurrentProgress
        );
        assert_eq!(
            CounterState::from(&CaptureLookup::Unregistered),
            CounterState::Unregistered
        );
        assert_eq!(
            CounterState::from(&CaptureLookup::Registered(CaptureRead::Progress(
                RunState::Blocked
            ))),
            CounterState::Blocked
        );
        assert_eq!(
            CounterState::from(&CaptureLookup::Registered(CaptureRead::Unreadable(
                std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
            ))),
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

    /// A tally as nextest writes it where it has no bar to put the
    /// count in, which is every run whose output is not a terminal.
    /// Left-padded to the width of the total, so a run of a thousand
    /// tests opens with three blanks inside the parenthesis.
    const CAPTURED_TALLY: &str = "        PASS [   1.014s] (11/24) nxprobe t18\n";

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
                &mut AccountCaptureDirectory {
                    root:    CaptureRoot::for_test(scan.path()),
                    owner:   scan.owner(),
                    account: AccountName::Unavailable,

                    state:        RootReadStatus::Readable,
                    confirmed:    0,
                    diagnostics:  Vec::new(),
                    associations: Vec::new(),
                },
            );
        }
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
    fn incomplete_registration_reads_remain_read_diagnostics_only() {
        let root = capture_root();
        let path = root.path().canonicalize().unwrap();
        let registration = path.join(CAPTURE_LIVE_RUNS_DIR).join("10.invalid");
        symlink("missing", &registration).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| Observation::Unknown);
        let status = &capture.root_status[0];
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
            failure: std::io::Error::from(ErrorKind::PermissionDenied).into(),
        };

        for _ in 0..2 {
            let mut capture =
                Capture::take_with_observations(root.path(), |_| Observation::Unknown);
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
            IdentityEvidence::Available(stamp) => Observation::Present(stamp),
            IdentityEvidence::Unavailable => Observation::Unknown,
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
    fn shared_parent_discovers_accounts_each_scan_and_ignores_foreign_owned_directories() {
        let parent = tempdir().unwrap();
        let roots = CaptureRoots::from_parent(parent.path());
        let uid = match root_scan::effective_user() {
            EffectiveUser::Known(uid) => uid,
            EffectiveUser::Unavailable => panic!("reader uid"),
        };
        let own = parent.path().join(uid.to_string());
        fs::create_dir_all(own.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        publish(&own, 10, "own", "100", CAPTURED_REDRAW);
        let first = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(pid, present("100"))
        });
        assert_eq!(first.root_status.len(), 1);
        assert_eq!(first.root_status[0].root.cleanup, CaptureCleanup::Here);
        let foreign_uid = uid.checked_add(1).unwrap();
        let foreign = parent.path().join(foreign_uid.to_string());
        fs::create_dir_all(foreign.join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        let stale = publish(&foreign, 20, "stale", "100", "");
        publish(&foreign, 21, "foreign", "100", CAPTURED_TALLY);
        let second = Capture::take_roots(&roots, &|pid| {
            KernelObservation::for_test(
                pid,
                if pid == 20 {
                    Observation::Ended
                } else {
                    present("100")
                },
            )
        });
        assert_eq!(second.root_status.len(), 2);
        assert_eq!(second.root_status[1].root.uid, foreign_uid);
        assert_eq!(
            second.root_status[1].root.cleanup,
            CaptureCleanup::AccountNextRun
        );
        assert!(matches!(
            second.root_status[1].state,
            RootReadStatus::ForeignOwned { .. }
        ));
        assert_eq!(second.root_status[1].owner, RootOwner::Uid(uid));
        assert_eq!(second.root_status[1].confirmed, 0);
        assert_eq!(second.lookup(1, 21), CaptureLookup::Unregistered);
        assert!(stale.0.exists());
        assert!(stale.1.exists());
        assert_eq!(first.confirmed()[0].key, second.confirmed()[0].key);
    }

    #[test]
    fn account_final_symlink_never_becomes_a_scan_capability() {
        let own = capture_root();
        let target = capture_root();
        let (registration, log) = publish(target.path(), 10, "ended", "100", "");
        let alias = own.path().join("alias");
        symlink(target.path(), &alias).unwrap();
        let roots = CaptureRoots::for_test(&[own.path(), &alias]);
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
        let roots = CaptureRoots::for_test(&[first.path(), second.path()]);
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
        let roots = CaptureRoots::for_test(&[first.path(), second.path()]);
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
    fn unsupported_live_registration_survives_repeated_sibling_sweeps() {
        assert_ineligible_registration_survives_sweeps(
            &future_record(),
            &present("100"),
            CaptureDiagnostic::UnsupportedRegistrationVersion {
                path:        PathBuf::new(),
                encountered: SUPPORTED_REGISTRATION_VERSION + 1,
                supported:   SUPPORTED_REGISTRATION_VERSION,
            },
        );
    }

    #[test]
    fn unsupported_ended_registration_survives_repeated_sibling_sweeps() {
        assert_ineligible_registration_survives_sweeps(
            &future_record(),
            &Observation::Ended,
            CaptureDiagnostic::UnsupportedRegistrationVersion {
                path:        PathBuf::new(),
                encountered: SUPPORTED_REGISTRATION_VERSION + 1,
                supported:   SUPPORTED_REGISTRATION_VERSION,
            },
        );
    }

    #[test]
    fn oversized_future_registration_survives_repeated_sibling_sweeps() {
        let mut bytes = future_record();
        bytes.resize(
            usize::try_from(CAPTURE_REGISTRATION_BYTES).unwrap() + 1,
            b'x',
        );
        assert_ineligible_registration_survives_sweeps(
            &bytes,
            &Observation::Ended,
            CaptureDiagnostic::UnsupportedRegistrationVersion {
                path:        PathBuf::new(),
                encountered: SUPPORTED_REGISTRATION_VERSION + 1,
                supported:   SUPPORTED_REGISTRATION_VERSION,
            },
        );
    }

    #[test]
    fn malformed_registration_survives_repeated_sweeps_with_readable_sibling() {
        let bytes = format!("cargo-tile-v{SUPPORTED_REGISTRATION_VERSION}\0partial\0").into_bytes();
        assert_ineligible_registration_survives_sweeps(
            &bytes,
            &present("100"),
            CaptureDiagnostic::RegistrationInvalid(PathBuf::new()),
        );
    }

    fn future_record() -> Vec<u8> {
        let mut bytes =
            format!("cargo-tile-v{}\0", SUPPORTED_REGISTRATION_VERSION + 1).into_bytes();
        bytes.extend_from_slice(b"\xfffuture-layout\0");
        bytes
    }

    fn assert_ineligible_registration_survives_sweeps(
        bytes: &[u8],
        observation: &Observation,
        mut diagnostic: CaptureDiagnostic,
    ) {
        let root = capture_root();
        let (registration, log) = publish(root.path(), 10, "retained", "100", CAPTURED_WAIT);
        fs::write(&registration, bytes).unwrap();
        let registration = registration.canonicalize().unwrap();
        match &mut diagnostic {
            CaptureDiagnostic::UnsupportedRegistrationVersion { path, .. }
            | CaptureDiagnostic::RegistrationInvalid(path) => path.clone_from(&registration),
            _ => panic!("fixture requires a framing diagnostic"),
        }
        let readable = publish(root.path(), 11, "readable", "100", CAPTURED_REDRAW);
        for _ in 0..3 {
            let ended = publish(root.path(), 12, "ended", "100", CAPTURED_WAIT);
            let capture = Capture::take_with_observations(root.path(), |pid| match pid {
                10 => observation.clone(),
                12 => Observation::Ended,
                _ => present("100"),
            });
            assert!(!ended.0.exists());
            assert!(!ended.1.exists());
            assert_eq!(fs::read(&registration).unwrap(), bytes);
            assert_eq!(fs::read(&log).unwrap(), CAPTURED_WAIT.as_bytes());
            assert_eq!(capture.lookup(0, 10), CaptureLookup::Unregistered);
            assert_eq!(capture.row_source(10), DirectAssociation::None);
            assert_eq!(capture.root_status[0].diagnostics, vec![diagnostic.clone()]);
            assert_eq!(capture.confirmed().len(), 1);
            assert_eq!(
                capture.lookup(0, 11),
                CaptureLookup::Registered(CaptureRead::Progress(compiling(149, 403))),
            );
            assert!(readable.0.exists() && readable.1.exists());
            let mut app = crate::app::App::new_for_test().unwrap();
            app.root_status.clone_from(&capture.root_status);
            let settings = crate::settings::rows(&app)
                .rows()
                .iter()
                .map(|row| row.value.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                settings.contains(registration.to_str().unwrap()),
                "{settings}"
            );
            match &diagnostic {
                CaptureDiagnostic::UnsupportedRegistrationVersion {
                    encountered,
                    supported,
                    ..
                } => {
                    assert!(settings.contains("unsupported"), "{settings}");
                    assert!(settings.contains(&format!("v{encountered}")), "{settings}");
                    assert!(settings.contains(&format!("v{supported}")), "{settings}");
                    assert!(
                        settings.contains("upgrade") && settings.contains("restart"),
                        "{settings}"
                    );
                    assert!(!settings.contains("invalid registration"), "{settings}");
                },
                CaptureDiagnostic::RegistrationInvalid(_) => {
                    assert!(settings.contains("invalid registration"), "{settings}");
                    assert!(!settings.contains("unsupported"), "{settings}");
                },
                _ => panic!("fixture requires a framing diagnostic"),
            }
        }
    }

    #[test]
    fn forged_live_process_identity_is_swept_without_removing_either_live_capture() {
        let root = capture_root();
        let first = publish(root.path(), 10, "first", "100", CAPTURED_WAIT);
        let victim = publish(root.path(), 11, "victim", "101", CAPTURED_WAIT);
        let forged = publish(root.path(), 11, "first-forged", "100", CAPTURED_WAIT);
        let capture = Capture::take_with_observations(root.path(), |pid| {
            present(if pid == 10 { "100" } else { "101" })
        });
        assert!(!forged.0.exists() && !forged.1.exists());
        for (pid, pair) in [(10, first), (11, victim)] {
            assert!(pair.0.exists() && pair.1.exists());
            assert_eq!(
                capture.lookup(0, pid),
                CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked)),
            );
            assert!(matches!(
                capture.row_source(pid),
                DirectAssociation::Direct(_)
            ));
        }
        assert_eq!(capture.confirmed().len(), 2);
    }

    #[test]
    fn competing_generation_suppresses_metadata_and_progress_until_removed() {
        assert_competing_generation_recovers("100");
    }

    #[test]
    fn unverifiable_generation_suppresses_metadata_and_progress_until_removed() {
        assert_competing_generation_recovers("");
    }

    fn assert_competing_generation_recovers(competing_birth: &str) {
        let root = capture_root();
        let live = publish(root.path(), 10, "z-live", "100", CAPTURED_WAIT);
        let competing = publish(
            root.path(),
            10,
            "a-competing",
            competing_birth,
            CAPTURED_TALLY,
        );
        let bytes = String::from_utf8(record("a-competing", 10, competing_birth))
            .unwrap()
            .replace("/writer/project", "/competing-directory")
            .replace("\0build\0", "\0test\0");
        fs::write(&competing.0, bytes).unwrap();
        let argv = ["cargo".into(), "build".into()];
        let records = ProcessObservations::new([ProcessObservation::cargo(10, &argv)]);
        let mut sequence = CensusSequence::default();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert!(matches!(capture.select(10), CaptureSelection::Ambiguous(_)));
        assert_eq!(capture.row_source(10), DirectAssociation::None);
        let groups = sequence.sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].rest.is_empty());
        assert_eq!(groups[0].lead.pid, 10);
        assert_eq!(groups[0].lead.command, CommandText::of("cargo", &["build"]));
        assert_eq!(groups[0].lead.path, "/work");
        assert_eq!(groups[0].lead.state, CaptureLookup::Unregistered);
        for path in [&live.0, &live.1, &competing.0, &competing.1] {
            assert!(path.exists());
        }
        fs::remove_file(competing.0).unwrap();
        fs::remove_file(competing.1).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        let groups = sequence.sample_capture(&records, &capture);
        assert_eq!(groups.len(), 1);
        assert!(groups[0].rest.is_empty());
        assert_eq!(groups[0].lead.pid, 10);
        assert_eq!(groups[0].lead.command, CommandText::of("cargo", &["build"]));
        assert_eq!(
            groups[0].lead.state,
            CaptureLookup::Registered(CaptureRead::Progress(RunState::Blocked))
        );
        assert!(live.0.exists() && live.1.exists());
    }

    #[test]
    fn legacy_darwin_boot_mismatch_preserves_live_artifacts_until_the_process_ends() {
        let root = capture_root();
        let (registration, log) = publish(root.path(), 10, "legacy", "100", CAPTURED_REDRAW);
        let bytes = String::from_utf8(record("legacy", 10, "100"))
            .unwrap()
            .replace("\0boot\0", "\0{ sec = 10, usec = 20 }\0");
        fs::write(&registration, &bytes).unwrap();
        let capture = Capture::take_with_observations(root.path(), |_| present("100"));
        assert!(capture.confirmed().is_empty());
        assert!(
            capture.root_status[0]
                .diagnostics
                .contains(&CaptureDiagnostic::IdentityUnknown(
                    registration.canonicalize().unwrap()
                ))
        );
        assert_eq!(fs::read(&registration).unwrap(), bytes.as_bytes());
        assert_eq!(fs::read_to_string(&log).unwrap(), CAPTURED_REDRAW);

        Capture::take_with_observations(root.path(), |_| Observation::Ended);
        assert!(!registration.exists());
        assert!(!log.exists());
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
    fn a_disappearing_registration_preserves_sibling_readings_and_sweeps_proven_pairs() {
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
        assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
        assert!(!stale.0.exists());
        assert!(!stale.1.exists());
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
            assert_eq!(calls.get(), 4);
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
            for changes_after in [1, 2, 3] {
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
                assert_eq!(log.exists(), changes_after < 3);
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
    fn incomplete_registration_inventory_sweeps_any_sampled_proven_pair() {
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
        let sampled = scan
            .registration_entries()
            .any(|entry| entry.name() == Path::new("10.one"));
        let mut budget = SweepBudget::default();
        Capture::default().scan_with_observations(&scan, &|_| Observation::Ended, &mut budget);
        assert_eq!(
            budget.remaining(),
            CAPTURE_SWEEP_LIMIT - usize::from(sampled) * 2
        );
        assert_eq!(pair.0.exists(), !sampled);
        assert_eq!(pair.1.exists(), !sampled);
    }

    #[test]
    fn an_unreadable_registration_preserves_sibling_progress_and_sweeps_proven_pairs() {
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
        let unreadable = root
            .path()
            .canonicalize()
            .unwrap()
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join("12.unreadable");
        assert!(capture.root_status[0].diagnostics.iter().any(|diagnostic| matches!(
            diagnostic, CaptureDiagnostic::RegistrationUnreadable(failure) if failure.path == unreadable
        )));
        assert!(unreadable.is_symlink());
        assert!(!stale.0.exists());
        assert!(!stale.1.exists());
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
    fn a_log_file_is_keyed_by_the_shim_pid_its_name_ends_with() {
        assert_eq!(
            log_pid(Path::new("/tmp/cargo-tile/run-20260820-191029-33395.log")),
            LogWriter::Identified(33395)
        );
        assert_eq!(
            log_pid(Path::new("/tmp/cargo-tile/pane-errors.log")),
            LogWriter::Unidentified
        );
    }
    /// Cargo's closing line as the shim captures it, the profile in the
    /// hyperlink escape cargo wraps it in.
    const CAPTURED_FINISHED: &str = "\u{1b}[1m\u{1b}[92m    Finished\u{1b}[0m \
         \u{1b}]8;;https://doc.rust-lang.org/cargo/reference/profiles.html\u{1b}\\`dev` profile \
         [unoptimized + debuginfo]\u{1b}]8;;\u{1b}\\ target(s) in 1.49s\n";
}
