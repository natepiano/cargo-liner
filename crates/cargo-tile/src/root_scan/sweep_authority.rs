//! Sweep admission, file eligibility, and pair removal accounting.

use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::OnceLock;

use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use rustix::fs::FileType;
use rustix::fs::Mode;
use rustix::fs::OFlags;
use rustix::fs::Stat;
use rustix::fs::fstat;
use rustix::fs::openat;
use rustix::fs::statat;
use rustix::fs::unlinkat;
use rustix::process::geteuid;

use super::RootScan;
use super::inspected_directory;
use super::inspected_directory::Enumeration;
use super::inspected_directory::RegistrationReadPurpose;
use super::inspected_directory::ScanEntry;
use crate::constants::CAPTURE_SWEEP_LIMIT;

/// Permission to sweep one root: the effective uid owns the root, the tree
/// identity is unchanged since it was opened, and the registration directory is
/// reachable. Each file still needs its own independent proof.
pub(super) struct SweepAuthority<'scan> {
    /// Owns the directory handles and the bounded registration sample.
    pub(super) scan: &'scan RootScan,
    /// Effective uid established before considering any file.
    pub(super) uid:  u32,
}

impl SweepAuthority<'_> {
    /// The test adapter changes only the uid compared with descriptor ownership.
    fn prove_file(&self, entry: ScanEntry<'_>) -> SweepFileInspection {
        #[cfg(test)]
        {
            let directory = if entry
                .directory
                .identity
                .same_directory(self.scan.root.identity)
            {
                self.scan.path.clone()
            } else {
                self.scan.registration_path()
            };
            if self
                .scan
                .foreign_files
                .contains(&directory.join(entry.name))
            {
                return SweepFileInspection::inspect(entry, self.uid.wrapping_add(1));
            }
        }
        SweepFileInspection::inspect(entry, self.uid)
    }

    /// Partial inventories sweep their established pairs and count the remainder.
    pub(super) fn sweep(
        self,
        budget: &mut SweepBudget,
        mut registrations: impl FnMut(ScanEntry<'_>) -> SweepDisposition,
    ) -> SweepCounts {
        let incomplete_inventories = std::iter::once(self.scan.registration_outcome())
            .chain(self.scan.sampled_log_outcome())
            .filter(|outcome| !matches!(outcome, Enumeration::Complete))
            .count();
        let mut counts = SweepCounts {
            incomplete_inventories,
            ..SweepCounts::default()
        };
        for entry in self.scan.registration_entries() {
            if budget.remaining() < 2 {
                break;
            }
            if self
                .sweep_pair(entry, budget, &mut registrations, &mut counts)
                .is_err()
            {
                counts.skipped_pair_attempts += 1;
            }
        }
        counts
    }

    /// Prove both files before deleting either; retain the record if its log fails.
    fn sweep_pair(
        &self,
        entry: ScanEntry<'_>,
        budget: &mut SweepBudget,
        registrations: &mut impl FnMut(ScanEntry<'_>) -> SweepDisposition,
        counts: &mut SweepCounts,
    ) -> io::Result<()> {
        let SweepFileInspection::Eligible(registration) = self.prove_file(entry) else {
            return Err(ErrorKind::PermissionDenied.into());
        };
        let entry = ScanEntry {
            directory:         entry.directory,
            name:              entry.name,
            registration_read: RegistrationReadPurpose::SweepRevalidation(&registration),
        };
        let SweepDisposition::Remove(log) = registrations(entry) else {
            return Err(ErrorKind::PermissionDenied.into());
        };
        let log_entry = inspected_directory::named_entry(&self.scan.root, &log)?;
        let log_proof = self.prove_file(log_entry);
        match &log_proof {
            SweepFileInspection::Eligible(_) => {},
            SweepFileInspection::InspectionFailed(error) if error.kind() == ErrorKind::NotFound => {
            },
            SweepFileInspection::Refused | SweepFileInspection::InspectionFailed(_) => {
                return Err(ErrorKind::PermissionDenied.into());
            },
        }
        if registrations(entry) != SweepDisposition::Remove(log.clone())
            || self.scan.revalidate_paths().is_err()
            || !budget.charge_pair()
        {
            return Err(ErrorKind::PermissionDenied.into());
        }
        registration.revalidate(entry)?;
        if let SweepFileInspection::Eligible(file) = log_proof {
            file.unlink(log_entry, counts)?;
        }
        if registrations(entry) != SweepDisposition::Remove(log)
            || self.scan.revalidate_paths().is_err()
        {
            return Err(ErrorKind::PermissionDenied.into());
        }
        registration.unlink(entry, counts)
    }
}

/// Filesystem ownership is independent of this scan's authority to remove data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RootOwner {
    /// The uid read from nofollow metadata or the root's open descriptor.
    Uid(u32),
    /// Opening the root failed before its owner could be inspected.
    Unavailable,
}

/// Why root ownership, access, or continuity could not admit a sweep.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SweepAdmissionRefusal {
    /// This directory belongs to a different account and is correctly read-only.
    ForeignRoot,
    /// The effective uid read failed once; restarting is the only retry.
    EffectiveUserUnavailable,
    /// Directory access or revalidation failed.
    AccessFailure,
}

/// One allowance shared by every cleanup kind across all roots in a single scan.
pub(crate) struct SweepBudget {
    /// Attempts consume allowance even when another process wins the unlink race.
    remaining: usize,
}

impl Default for SweepBudget {
    fn default() -> Self {
        Self {
            remaining: CAPTURE_SWEEP_LIMIT,
        }
    }
}

impl SweepBudget {
    /// Expose the actual production allowance for multi-root budget assertions.
    pub(crate) const fn remaining(&self) -> usize { self.remaining }

    /// Reserve the complete pair before its first removal attempt.
    const fn charge_pair(&mut self) -> bool {
        if self.remaining < 2 {
            return false;
        }
        self.remaining -= 2;
        true
    }
}

/// Identity policy supplies one freshly rechecked registration/log association.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SweepDisposition {
    /// Live, legacy, malformed, or unverifiable records survive, including staging.
    Preserve,
    /// Only this exact log may be removed alongside its ended registration.
    Remove(PathBuf),
}

/// Sweep totals are worker-local observations, never Settings diagnostics.
#[derive(Default, Debug, Eq, PartialEq)]
pub(crate) struct SweepCounts {
    /// Successful file unlinks, excluding names already absent.
    removed_files:          usize,
    /// Candidate pairs whose attempted sweep did not finish.
    skipped_pair_attempts:  usize,
    /// Sampled inventories with truncated or failed enumeration.
    incomplete_inventories: usize,
    /// Roots whose ownership, access, or continuity prevented admission.
    unavailable_roots:      usize,
}

impl SweepCounts {
    /// A refused root attempts no pairs and removes no files.
    pub(super) fn unavailable_root() -> Self {
        Self {
            unavailable_roots: 1,
            ..Self::default()
        }
    }
}

/// Test harnesses observe each count without exposing mutable statistics fields.
#[cfg(test)]
impl SweepCounts {
    /// Successful unlinks observed by the sweep.
    pub const fn removed_files(&self) -> usize { self.removed_files }

    /// Candidate pairs retained after an unsuccessful sweep attempt.
    pub const fn skipped_pair_attempts(&self) -> usize { self.skipped_pair_attempts }

    /// Inventories whose enumeration did not finish successfully.
    pub const fn incomplete_inventories(&self) -> usize { self.incomplete_inventories }

    /// Roots refused before any pair could be attempted.
    pub const fn unavailable_roots(&self) -> usize { self.unavailable_roots }
}

/// Descriptor inspection separates refusal from an unsuccessful filesystem query.
/// Eligibility is an inspected state, not a removal decision: the file is
/// revalidated against its held identity immediately before any unlink.
enum SweepFileInspection {
    /// The open regular file is owned by the sweeping uid, carries no group or
    /// other write bit, and has a single link.
    Eligible(SweepEligibleFile),
    /// Metadata or a symlink contradicts the removal prerequisites.
    Refused,
    /// No descriptor metadata could establish removal authority.
    InspectionFailed(Error),
}

/// A file that passed sweep inspection; its open descriptor pins the inode until
/// its basename is revalidated for removal.
pub(super) struct SweepEligibleFile {
    /// Prevent inode reuse between establishment and unlink.
    pub(super) handle: OwnedFd,
    /// Ownership is checked again on both the held and current entry metadata.
    uid:               u32,
}

impl SweepFileInspection {
    /// NOFOLLOW rejects links and NONBLOCK keeps special files from waiting.
    fn inspect(entry: ScanEntry<'_>, uid: u32) -> Self {
        let handle = match openat(
            &entry.directory.handle,
            entry.name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        ) {
            Ok(handle) => handle,
            Err(rustix::io::Errno::LOOP) => return Self::Refused,
            Err(error) => return Self::InspectionFailed(error.into()),
        };
        match fstat(&handle) {
            Ok(metadata) if file_is_owned(&metadata, uid) => {
                Self::Eligible(SweepEligibleFile { handle, uid })
            },
            Ok(_) => Self::Refused,
            Err(error) => Self::InspectionFailed(error.into()),
        }
    }
}

/// Each predicate describes descriptor metadata supplied by the filesystem API.
fn file_is_owned(metadata: &Stat, uid: u32) -> bool {
    FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
        && metadata.st_uid == uid
        && !Mode::from_raw_mode(metadata.st_mode).intersects(Mode::WGRP | Mode::WOTH)
        && metadata.st_nlink == 1
}

impl SweepEligibleFile {
    /// Compare the pinned inode to the basename immediately before unlinkat.
    fn revalidate(&self, entry: ScanEntry<'_>) -> io::Result<()> {
        let held = fstat(&self.handle)?;
        let current = statat(
            &entry.directory.handle,
            entry.name,
            AtFlags::SYMLINK_NOFOLLOW,
        )?;
        if !file_is_owned(&held, self.uid)
            || !file_is_owned(&current, self.uid)
            || held.st_dev != current.st_dev
            || held.st_ino != current.st_ino
        {
            return Err(ErrorKind::PermissionDenied.into());
        }
        Ok(())
    }

    /// The same retained directory supplies inspection and removal authority.
    fn unlink(&self, entry: ScanEntry<'_>, counts: &mut SweepCounts) -> io::Result<()> {
        self.revalidate(entry)?;
        unlinkat(&entry.directory.handle, entry.name, AtFlags::empty())?;
        counts.removed_files += 1;
        Ok(())
    }
}

/// A missing own-process uid disables cleanup instead of selecting a default uid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EffectiveUser {
    /// Read from this process's effective identity, rather than its real uid.
    Known(u32),
    /// No effective identity is established, so filesystem cleanup is disabled.
    Unavailable,
}

impl EffectiveUser {
    /// Retain the first identity result, including failure, for every later scan.
    fn cached(cache: &OnceLock<Self>, read: impl FnOnce() -> Self) -> Self {
        *cache.get_or_init(read)
    }
}

/// The effective identity is fixed for this process; failed reads stay unavailable.
pub(crate) fn effective_user() -> EffectiveUser {
    /// Share one observation across all roots and scans, including a failed read.
    static EFFECTIVE_USER: OnceLock<EffectiveUser> = OnceLock::new();
    EffectiveUser::cached(&EFFECTIVE_USER, read_effective_user)
}

/// Query the kernel directly without depending on process-table visibility.
fn read_effective_user() -> EffectiveUser { EffectiveUser::Known(geteuid().as_raw()) }

#[cfg(test)]
pub(super) use tests::sweep_everything;
#[cfg(test)]
pub(super) use tests::write_pair;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::io;
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::sync::OnceLock;

    use tempfile::TempDir;

    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::root_scan;
    use crate::root_scan::EffectiveUser;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootScan;
    use crate::root_scan::SweepBudget;
    use crate::root_scan::SweepDisposition;
    use crate::root_scan::inspected_directory;
    use crate::root_scan::inspected_directory::Enumeration;

    #[test]
    fn partial_enumeration_sweeps_established_pairs_and_counts_the_remainder() {
        for outcome in [
            Enumeration::Incomplete,
            Enumeration::Failed(io::Error::from(ErrorKind::PermissionDenied)),
        ] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let mut scan =
                RootScan::open(root.path(), &mut RootHistory::default()).expect("open root");
            scan.registrations.outcome = outcome;
            let counts = scan.sweep(&mut SweepBudget::default(), |_| {
                SweepDisposition::Remove("log".into())
            });
            assert_eq!(counts.removed_files(), 2);
            assert_eq!(counts.skipped_pair_attempts(), 0);
            assert_eq!(counts.incomplete_inventories(), 1);
            assert_eq!(counts.unavailable_roots(), 0);
            assert!(!root.path().join("log").exists());
        }
    }

    #[test]
    fn a_refused_root_counts_once_without_callbacks_or_budget_consumption() {
        let root = inspected_directory::capture_root();
        write_pair(root.path());
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let registrations = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::rename(&registrations, root.path().join("old-pids"))
            .expect("replace registration directory");
        fs::create_dir(&registrations).expect("new registration directory");
        let calls = Cell::new(0);
        let mut budget = SweepBudget::default();
        let remaining = budget.remaining();
        let counts = scan.sweep(&mut budget, |_| {
            calls.set(calls.get() + 1);
            SweepDisposition::Remove("log".into())
        });
        assert_eq!(counts.removed_files(), 0);
        assert_eq!(counts.skipped_pair_attempts(), 0);
        assert_eq!(counts.incomplete_inventories(), 0);
        assert_eq!(counts.unavailable_roots(), 1);
        assert_eq!(calls.get(), 0);
        assert_eq!(budget.remaining(), remaining);
        assert!(root.path().join("log").exists());
    }

    /// Attempt every artifact through the production capability dispatcher.
    pub fn sweep_everything(scan: &RootScan) -> usize {
        let mut budget = SweepBudget::default();
        scan.sweep(&mut budget, |_| {
            SweepDisposition::Remove(Path::new("log").to_owned())
        });
        CAPTURE_SWEEP_LIMIT - budget.remaining()
    }

    #[test]
    fn a_pair_that_does_not_fit_the_remaining_allowance_stays_whole() {
        let root = inspected_directory::capture_root();
        let registration = root
            .path()
            .join(CAPTURE_LIVE_RUNS_DIR)
            .join("10.generation");
        fs::write(&registration, "record").expect("record");
        let log = root.path().join("log");
        fs::write(&log, "output").expect("log");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan");
        let mut budget = SweepBudget { remaining: 1 };
        scan.sweep(&mut budget, |_| {
            SweepDisposition::Remove(Path::new("log").to_owned())
        });
        assert_eq!(budget.remaining(), 1);
        assert!(registration.exists());
        assert!(log.exists());
        budget.remaining = 2;
        scan.sweep(&mut budget, |_| {
            SweepDisposition::Remove(Path::new("log").to_owned())
        });
        assert_eq!(budget.remaining(), 0);
        assert!(!registration.exists());
        assert!(!log.exists());
    }

    /// Log enumeration bounds do not block directly named registration pairs.
    #[test]
    fn incomplete_log_inventory_still_sweeps_directly_named_pairs() {
        let root = inspected_directory::capture_root();
        for number in 0..CAPTURE_INVENTORY_LIMIT {
            fs::write(root.path().join(format!("unrelated-{number}")), "")
                .expect("unrelated entry");
        }
        write_pair(root.path());
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("bounded scan");
        assert!(matches!(scan.log_outcome(), Enumeration::Incomplete));
        assert_eq!(scan.log_entries().count(), CAPTURE_INVENTORY_LIMIT);
        assert_eq!(sweep_everything(&scan), 2);
        assert!(!root.path().join("log").exists());
    }

    /// Later roots and scans reuse the successful first identity observation.
    #[test]
    fn a_known_effective_user_is_read_only_once() {
        let cache = OnceLock::new();
        let reads = Cell::new(0);
        let first = root_scan::effective_user();
        assert!(matches!(first, EffectiveUser::Known(_)));
        for result in [first, EffectiveUser::Unavailable] {
            assert_eq!(
                EffectiveUser::cached(&cache, || {
                    reads.set(reads.get() + 1);
                    result
                }),
                first
            );
        }
        assert_eq!(reads.get(), 1);
    }

    /// A failed first read never becomes a later authorization to sweep.
    #[test]
    fn an_unavailable_effective_user_is_never_retried() {
        let cache = OnceLock::new();
        let reads = Cell::new(0);
        let known = root_scan::effective_user();
        assert!(matches!(known, EffectiveUser::Known(_)));
        for result in [EffectiveUser::Unavailable, known] {
            assert_eq!(
                EffectiveUser::cached(&cache, || {
                    reads.set(reads.get() + 1);
                    result
                }),
                EffectiveUser::Unavailable
            );
        }
        assert_eq!(reads.get(), 1);
    }

    /// Each file supplies its own ownership proof regardless of directory writers.
    #[test]
    fn directories_permitting_other_writers_are_swept_of_owned_leftovers() {
        for ancestor in ["", CAPTURE_STATE_DIR, CAPTURE_LIVE_RUNS_DIR] {
            for mode in [0o720, 0o702] {
                let root = inspected_directory::capture_root();
                write_pair(root.path());
                fs::set_permissions(root.path().join(ancestor), fs::Permissions::from_mode(mode))
                    .expect("shared directory");
                let scan =
                    RootScan::open(root.path(), &mut RootHistory::default()).expect("read root");
                assert_eq!(sweep_everything(&scan), 2);
                assert!(!root.path().join("log").exists());
                assert!(
                    !root
                        .path()
                        .join(CAPTURE_LIVE_RUNS_DIR)
                        .join("record")
                        .exists()
                );
            }
        }
    }

    /// Fixtures own regular files independently of the test runner's umask.
    pub fn write_pair(root: &Path) {
        for path in [
            root.join("log"),
            root.join(CAPTURE_LIVE_RUNS_DIR).join("record"),
        ] {
            fs::write(&path, "ended capture").expect("capture file");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("owned file");
        }
    }

    #[test]
    fn symlink_candidates_are_retained_and_counted() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let candidate = root.path().join(name);
            fs::rename(&candidate, root.path().join("target")).expect("move target");
            symlink(root.path().join("target"), &candidate).expect("symlink candidate");
            assert_retained_pair(&root);
            assert!(candidate.is_symlink());
            assert!(root.path().join("target").exists());
        }
    }

    #[test]
    fn group_or_other_writable_candidates_are_retained_and_counted() {
        for name in ["log", "state/pids/record"] {
            for mode in [0o620, 0o602] {
                let root = inspected_directory::capture_root();
                write_pair(root.path());
                fs::set_permissions(root.path().join(name), fs::Permissions::from_mode(mode))
                    .expect("writable candidate");
                assert_retained_pair(&root);
            }
        }
    }

    #[test]
    fn candidates_with_a_second_hard_link_are_retained_and_counted() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            fs::hard_link(root.path().join(name), root.path().join("alias")).expect("second link");
            assert_retained_pair(&root);
            assert!(root.path().join("alias").exists());
        }
    }

    #[test]
    fn foreign_file_ownership_is_refused_with_one_fixture_account() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let mut scan =
                RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
            scan.mark_file_foreign_for_test(&root.path().join(name))
                .expect("foreign comparison");
            let counts = scan.sweep(&mut SweepBudget::default(), |_| {
                SweepDisposition::Remove("log".into())
            });
            assert_eq!(counts.removed_files(), 0);
            assert_eq!(counts.skipped_pair_attempts(), 1);
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            assert!(root.path().join(name).exists());
        }
    }

    fn assert_retained_pair(root: &TempDir) {
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let counts = scan.sweep(&mut SweepBudget::default(), |_| {
            SweepDisposition::Remove("log".into())
        });
        assert_eq!(counts.removed_files(), 0);
        assert_eq!(counts.skipped_pair_attempts(), 1);
        assert_eq!(counts.incomplete_inventories(), 0);
        assert_eq!(counts.unavailable_roots(), 0);
        assert!(root.path().join("log").symlink_metadata().is_ok());
        assert!(
            root.path()
                .join(CAPTURE_LIVE_RUNS_DIR)
                .join("record")
                .symlink_metadata()
                .is_ok()
        );
    }

    #[test]
    fn nonregular_candidates_are_retained_and_counted() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let candidate = root.path().join(name);
            fs::remove_file(&candidate).expect("remove regular file");
            fs::create_dir(&candidate).expect("directory candidate");
            assert_retained_pair(&root);
            fs::remove_dir(&candidate).expect("remove directory candidate");
            inspected_directory::make_fifo(&candidate);
            assert_retained_pair(&root);
        }
    }

    #[test]
    fn file_permissions_changed_after_proof_are_rechecked_before_unlink() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let candidate = root.path().join(name);
            let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
            let mut calls = 0;
            let counts = scan.sweep(&mut SweepBudget::default(), |_| {
                calls += 1;
                if calls == 2 {
                    fs::set_permissions(&candidate, fs::Permissions::from_mode(0o620))
                        .expect("revoke file proof");
                }
                SweepDisposition::Remove("log".into())
            });
            assert_eq!(counts.removed_files(), 0);
            assert_eq!(counts.skipped_pair_attempts(), 1);
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            assert!(candidate.exists());
        }
    }

    #[test]
    fn sweep_callbacks_read_the_proved_registration_after_basename_replacement() {
        let root = inspected_directory::capture_root();
        write_pair(root.path());
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("record");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let mut calls = 0;
        let counts = scan.sweep(&mut SweepBudget::default(), |entry| {
            calls += 1;
            if calls == 1 {
                fs::rename(&registration, root.path().join("original"))
                    .expect("move proved record");
                fs::write(&registration, "replacement").expect("replace basename");
            }
            let observed = entry.read_registration().expect("read proved descriptor");
            assert_eq!(observed.bytes, b"ended capture");
            SweepDisposition::Remove("log".into())
        });
        assert_eq!(calls, 2);
        assert_eq!(counts.removed_files(), 0);
        assert_eq!(counts.skipped_pair_attempts(), 1);
        assert_eq!(counts.incomplete_inventories(), 0);
        assert_eq!(counts.unavailable_roots(), 0);
        assert_eq!(
            fs::read_to_string(registration).expect("replacement survives"),
            "replacement"
        );
        assert!(root.path().join("log").exists());
    }

    #[test]
    fn registration_swapped_after_log_unlink_survives_the_final_identity_check() {
        let root = inspected_directory::capture_root();
        write_pair(root.path());
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("record");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let counts = scan.sweep(&mut SweepBudget::default(), |_| {
            if !root.path().join("log").exists() {
                fs::rename(&registration, root.path().join("original"))
                    .expect("move original record");
                fs::write(&registration, "replacement").expect("replace record");
            }
            SweepDisposition::Remove("log".into())
        });
        assert_eq!(counts.removed_files(), 1);
        assert_eq!(counts.skipped_pair_attempts(), 1);
        assert_eq!(counts.incomplete_inventories(), 0);
        assert_eq!(counts.unavailable_roots(), 0);
        assert_eq!(
            fs::read_to_string(registration).expect("replacement survives"),
            "replacement"
        );
    }

    #[test]
    fn identity_recheck_before_unlink_refuses_a_swapped_entry() {
        for name in ["log", "state/pids/record"] {
            let root = inspected_directory::capture_root();
            write_pair(root.path());
            let candidate = root.path().join(name);
            let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
            let mut calls = 0;
            let counts = scan.sweep(&mut SweepBudget::default(), |_| {
                calls += 1;
                if calls == 2 {
                    fs::rename(&candidate, root.path().join("original")).expect("move proved file");
                    fs::write(&candidate, "replacement").expect("swap entry");
                }
                SweepDisposition::Remove("log".into())
            });
            assert_eq!(counts.removed_files(), 0);
            assert_eq!(counts.skipped_pair_attempts(), 1);
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            assert_eq!(
                fs::read_to_string(candidate).expect("replacement survives"),
                "replacement"
            );
            assert!(root.path().join("original").exists());
        }
    }

    /// Linux and macOS use the same effective-owner and directory-mode policy.
    #[test]
    fn owned_cleanup_follows_the_explicit_platform_permission_policy() {
        let root = inspected_directory::capture_root();
        let path = root.path().join("log");
        fs::write(&path, "bounded read").expect("root log");
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("123");
        fs::write(&registration, "stale registration").expect("registration");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        assert_eq!(
            inspected_directory::log_entry(&scan, "log")
                .read_log()
                .expect("read owned log"),
            "bounded read"
        );
        let attempts = sweep_everything(&scan);
        // One full pair reserves its allowance before the first unlink.
        assert_eq!(attempts, 2);
        assert!(!path.exists());
        assert!(!registration.exists());
    }

    /// A new directory writer cannot revoke the descriptor proof of an owned file.
    #[test]
    fn changing_directory_permissions_after_sampling_allows_owned_cleanup() {
        let root = inspected_directory::capture_root();
        write_pair(root.path());
        let mut history = RootHistory::default();
        let scan = RootScan::open(root.path(), &mut history).expect("initial scan");
        fs::set_permissions(
            root.path().join(CAPTURE_LIVE_RUNS_DIR),
            fs::Permissions::from_mode(0o720),
        )
        .expect("shared directory");
        assert_eq!(sweep_everything(&scan), 2);
        write_pair(root.path());
        let next = RootScan::open(root.path(), &mut history).expect("next scan");
        assert_eq!(sweep_everything(&next), 2);
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod acl_tests {
    use std::fs;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;

    use rustix::fs::Mode;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::constants::PERMISSION_BITS;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootOwner;
    use crate::root_scan::RootScan;
    use crate::root_scan::SweepBudget;
    use crate::root_scan::SweepDisposition;

    /// Exercise the account root and both registration ancestors independently.
    const CAPTURE_ACL_TEST_DIRECTORIES: [&str; 3] = ["", CAPTURE_STATE_DIR, CAPTURE_LIVE_RUNS_DIR];
    /// Establish directory ownership independently of the invoking account's umask.
    const CAPTURE_ACL_TEST_DIRECTORY_MODE: u32 = 0o700;
    /// Candidate file fixtures must satisfy the per-file mode prerequisite initially.
    const CAPTURE_ACL_TEST_FILE_MODE: u32 = 0o600;
    /// Native chmod spellings independently exercise every write-class permission;
    /// deriving this list from the production bit mask would hide missing mask bits.
    const CAPTURE_ACL_TEST_WRITE_PERMISSIONS: [&str; 8] = [
        "add_file",
        "add_subdirectory",
        "delete",
        "delete_child",
        "writeattr",
        "writeextattr",
        "writesecurity",
        "chown",
    ];

    /// One owner, one removable pair, and three directories with known permissions.
    struct CaptureRoot {
        directory: TempDir,
    }

    impl CaptureRoot {
        fn new() -> Self {
            let directory = tempdir().expect("temporary capture root");
            fs::create_dir_all(directory.path().join(CAPTURE_LIVE_RUNS_DIR))
                .expect("registration directories");
            let root = Self { directory };
            for relative in CAPTURE_ACL_TEST_DIRECTORIES {
                let path = root.path(relative);
                chmod(&path, &["-N"]);
                fs::set_permissions(
                    path,
                    fs::Permissions::from_mode(CAPTURE_ACL_TEST_DIRECTORY_MODE),
                )
                .expect("owner-only mode bits");
            }
            fs::write(root.log(), "retained log").expect("fixture log");
            fs::write(root.registration(), "sampled registration").expect("fixture registration");
            for path in [root.log(), root.registration()] {
                fs::set_permissions(path, fs::Permissions::from_mode(CAPTURE_ACL_TEST_FILE_MODE))
                    .expect("owner-only file mode bits");
            }
            root
        }

        fn path(&self, relative: &str) -> PathBuf {
            if relative.is_empty() {
                self.directory.path().to_owned()
            } else {
                self.directory.path().join(relative)
            }
        }

        fn log(&self) -> PathBuf { self.path("log") }

        fn registration(&self) -> PathBuf { self.path(CAPTURE_LIVE_RUNS_DIR).join("record") }

        fn scan(&self) -> RootScan {
            RootScan::open(self.directory.path(), &mut RootHistory::default())
                .expect("open capture directories")
        }

        fn assert_owner(&self, scan: &RootScan) {
            let owner = fs::metadata(self.directory.path())
                .expect("fixture root metadata")
                .uid();
            assert_eq!(scan.owner(), RootOwner::Uid(owner));
        }

        fn assert_retained(&self) {
            assert_eq!(fs::read(self.log()).expect("retained log"), b"retained log");
            assert_eq!(
                fs::read(self.registration()).expect("retained registration"),
                b"sampled registration"
            );
        }

        fn assert_sweeps(&self, scan: &RootScan) {
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            let counts = scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(counts.removed_files(), 2);
            assert_eq!(counts.skipped_pair_attempts(), 0);
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            self.assert_removed();
        }

        fn assert_removed(&self) {
            assert!(!self.log().try_exists().expect("log existence"));
            assert!(
                !self
                    .registration()
                    .try_exists()
                    .expect("registration existence")
            );
        }

        fn assert_skips(&self, scan: &RootScan) {
            self.assert_owner(scan);
            let mut budget = SweepBudget::default();
            let counts = scan.sweep(&mut budget, |_| {
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert_eq!(counts.removed_files(), 0);
            assert!(
                counts.skipped_pair_attempts() > 0,
                "unproved candidates are counted"
            );
            assert_eq!(counts.incomplete_inventories(), 0);
            assert_eq!(counts.unavailable_roots(), 0);
            self.assert_retained();
        }
    }

    #[test]
    fn every_non_owner_write_permission_on_root_allows_owned_file_cleanup() {
        assert_write_permissions_sweep("");
    }

    #[test]
    fn every_non_owner_write_permission_on_state_allows_owned_file_cleanup() {
        assert_write_permissions_sweep(CAPTURE_STATE_DIR);
    }

    #[test]
    fn every_non_owner_write_permission_on_pids_allows_owned_file_cleanup() {
        assert_write_permissions_sweep(CAPTURE_LIVE_RUNS_DIR);
    }

    #[test]
    fn non_owner_write_entry_after_read_only_entry_allows_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            chmod(&path, &["+a#", "0", "group:everyone allow list"]);
            chmod(&path, &["+a#", "1", "group:everyone allow add_file"]);
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn non_owner_write_added_after_open_allows_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            grant_write(&path, "add_file");
            root.assert_sweeps(&scan);
        }
    }

    #[test]
    fn non_owner_write_added_by_first_sweep_callback_allows_both_unlinks() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                if callbacks == 1 {
                    grant_write(&path, "add_file");
                }
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert!(callbacks >= 2, "identity is rechecked before each unlink");
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            root.assert_removed();
        }
    }

    #[test]
    fn non_owner_write_added_before_registration_unlink_allows_both_unlinks() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            let path = root.path(relative);
            let mut callbacks = 0;
            let mut grants_after_log_removal = 0;
            let mut budget = SweepBudget::default();
            scan.sweep(&mut budget, |_| {
                callbacks += 1;
                if !root
                    .log()
                    .try_exists()
                    .expect("log existence during callback")
                {
                    grants_after_log_removal += 1;
                    grant_write(&path, "add_file");
                }
                SweepDisposition::Remove(PathBuf::from("log"))
            });
            assert!(callbacks >= 2);
            assert!(grants_after_log_removal > 0);
            assert_eq!(budget.remaining(), CAPTURE_SWEEP_LIMIT - 2);
            root.assert_removed();
            root.assert_owner(&scan);
        }
    }

    #[test]
    fn directories_without_acls_still_sweep() {
        let root = CaptureRoot::new();
        root.assert_sweeps(&root.scan());
    }

    #[test]
    fn directories_with_explicitly_emptied_acls_still_sweep() {
        let root = CaptureRoot::new();
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let path = root.path(relative);
            grant_write(&path, "add_file");
            // Removing the sole entry writes a zero-entry ACL; -N removes the ACL itself.
            chmod(&path, &["-a#", "0"]);
        }
        root.assert_sweeps(&root.scan());
    }

    #[test]
    fn read_only_non_owner_acl_on_each_directory_still_sweeps() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            chmod(
                &root.path(relative),
                &[
                    "+a",
                    "group:everyone allow list,search,readattr,readextattr,readsecurity",
                ],
            );
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn read_only_non_owner_acl_added_after_open_still_sweeps() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let scan = root.scan();
            chmod(
                &root.path(relative),
                &["+a", "group:everyone allow list,search"],
            );
            root.assert_sweeps(&scan);
        }
    }

    #[test]
    fn owner_write_acl_on_each_directory_still_sweeps() {
        let output = Command::new("/usr/bin/id")
            .arg("-un")
            .output()
            .expect("effective owner name");
        assert!(output.status.success(), "{output:?}");
        let owner = String::from_utf8(output.stdout).expect("owner name is UTF-8");
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            let entry = format!(
                "user:{}:allow:{}",
                owner.trim(),
                CAPTURE_ACL_TEST_WRITE_PERMISSIONS.join(",")
            );
            chmod(&root.path(relative), &["+a", &entry]);
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn non_owner_deny_entry_is_not_a_write_grant() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            chmod(
                &root.path(relative),
                &["+a", "group:everyone deny add_file"],
            );
            root.assert_sweeps(&root.scan());
        }
    }

    #[test]
    fn foreign_registration_in_acl_writable_directory_preserves_pair() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            grant_write(&root.path(relative), "add_file");
            let mut scan = root.scan();
            // Change only the uid comparison; file metadata and directory ownership stay real.
            scan.mark_file_foreign_for_test(&root.registration())
                .expect("inject foreign registration owner");
            root.assert_skips(&scan);
        }
    }

    #[test]
    fn foreign_log_in_acl_writable_directory_preserves_pair() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            let root = CaptureRoot::new();
            grant_write(&root.path(relative), "add_file");
            let mut scan = root.scan();
            // Change only the uid comparison; file metadata and directory ownership stay real.
            scan.mark_file_foreign_for_test(&root.log())
                .expect("inject foreign log owner");
            root.assert_skips(&scan);
        }
    }

    #[test]
    fn writable_directory_mode_bits_allow_owned_file_cleanup() {
        for relative in CAPTURE_ACL_TEST_DIRECTORIES {
            for write in [Mode::WGRP, Mode::WOTH] {
                let root = CaptureRoot::new();
                fs::set_permissions(
                    root.path(relative),
                    fs::Permissions::from_mode(
                        CAPTURE_ACL_TEST_DIRECTORY_MODE | u32::from(write.bits()),
                    ),
                )
                .expect("directory permits non-owner writes");
                root.assert_sweeps(&root.scan());
            }
        }
    }

    #[test]
    fn writable_file_mode_bits_preserve_pair_in_acl_writable_directory() {
        for write in [Mode::WGRP, Mode::WOTH] {
            for candidate in [CaptureRoot::log, CaptureRoot::registration] {
                let root = CaptureRoot::new();
                grant_write(&root.path(""), "add_file");
                fs::set_permissions(
                    candidate(&root),
                    fs::Permissions::from_mode(
                        CAPTURE_ACL_TEST_FILE_MODE | u32::from(write.bits()),
                    ),
                )
                .expect("candidate permits non-owner writes");
                root.assert_skips(&root.scan());
            }
        }
    }

    #[test]
    fn multiply_linked_file_preserves_pair_in_acl_writable_directory() {
        for candidate in [CaptureRoot::log, CaptureRoot::registration] {
            let root = CaptureRoot::new();
            grant_write(&root.path(""), "add_file");
            let alias = root.path("alias");
            fs::hard_link(candidate(&root), &alias).expect("second link to candidate");
            let contents = fs::read(&alias).expect("linked contents before sweep");
            root.assert_skips(&root.scan());
            assert_eq!(
                fs::read(alias).expect("linked contents after sweep"),
                contents
            );
        }
    }

    #[test]
    fn symlinked_file_preserves_target_in_acl_writable_directory() {
        for candidate in [CaptureRoot::log, CaptureRoot::registration] {
            let root = CaptureRoot::new();
            grant_write(&root.path(""), "add_file");
            let path = candidate(&root);
            let target = root.path("target");
            fs::rename(&path, &target).expect("move candidate to symlink target");
            symlink(&target, &path).expect("candidate symlink");
            let contents = fs::read(&target).expect("target contents before sweep");
            root.assert_skips(&root.scan());
            assert!(
                fs::symlink_metadata(path)
                    .expect("retained symlink")
                    .is_symlink()
            );
            assert_eq!(
                fs::read(target).expect("target contents after sweep"),
                contents
            );
        }
    }

    #[test]
    fn registration_replaced_before_its_unlink_survives_in_acl_writable_directory() {
        let root = CaptureRoot::new();
        grant_write(&root.path(CAPTURE_LIVE_RUNS_DIR), "add_file");
        let scan = root.scan();
        let mut callbacks = 0;
        let mut budget = SweepBudget::default();
        scan.sweep(&mut budget, |_| {
            callbacks += 1;
            if !root
                .log()
                .try_exists()
                .expect("log existence during callback")
            {
                fs::rename(root.registration(), root.path("held-record"))
                    .expect("retain sampled inode separately");
                fs::write(root.registration(), "replacement registration")
                    .expect("replace registration before unlink");
                fs::set_permissions(
                    root.registration(),
                    fs::Permissions::from_mode(CAPTURE_ACL_TEST_FILE_MODE),
                )
                .expect("replacement independently satisfies file mode rule");
            }
            SweepDisposition::Remove(PathBuf::from("log"))
        });
        assert!(callbacks >= 2);
        assert!(
            !root
                .log()
                .try_exists()
                .expect("log removed before replacement")
        );
        assert_eq!(
            fs::read(root.registration()).expect("replacement registration survives"),
            b"replacement registration"
        );
        assert_eq!(
            fs::read(root.path("held-record")).expect("sampled inode survives"),
            b"sampled registration"
        );
    }

    fn assert_write_permissions_sweep(relative: &str) {
        for permission in CAPTURE_ACL_TEST_WRITE_PERMISSIONS {
            let root = CaptureRoot::new();
            let path = root.path(relative);
            grant_write(&path, permission);
            root.assert_sweeps(&root.scan());
        }
    }

    fn grant_write(path: &Path, permission: &str) {
        chmod(path, &["+a", &format!("group:everyone allow {permission}")]);
        assert_eq!(
            fs::metadata(path).expect("ACL directory metadata").mode() & PERMISSION_BITS,
            CAPTURE_ACL_TEST_DIRECTORY_MODE,
            "the ACL fixture must not rely on group or other write mode bits: {} ({permission})",
            path.display()
        );
    }

    fn chmod(path: &Path, arguments: &[&str]) {
        let output = Command::new("/bin/chmod")
            .args(arguments)
            .arg(path)
            .output()
            .expect("run native ACL fixture command");
        assert!(
            output.status.success(),
            "chmod {arguments:?} {}: {output:?}",
            path.display()
        );
    }
}
