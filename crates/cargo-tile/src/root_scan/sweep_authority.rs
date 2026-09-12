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
use super::inspected_directory::Enumeration;
use super::inspected_directory::RegistrationReadPurpose;
use super::inspected_directory::ScanEntry;
use super::inspected_directory::named_entry;
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
        let log_entry = named_entry(&self.scan.root, &log)?;
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
    pub(crate) const fn removed_files(&self) -> usize { self.removed_files }

    /// Candidate pairs retained after an unsuccessful sweep attempt.
    pub(crate) const fn skipped_pair_attempts(&self) -> usize { self.skipped_pair_attempts }

    /// Inventories whose enumeration did not finish successfully.
    pub(crate) const fn incomplete_inventories(&self) -> usize { self.incomplete_inventories }

    /// Roots refused before any pair could be attempted.
    pub(crate) const fn unavailable_roots(&self) -> usize { self.unavailable_roots }
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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
pub(super) mod tests {
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
    use crate::root_scan::EffectiveUser;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootScan;
    use crate::root_scan::SweepBudget;
    use crate::root_scan::SweepDisposition;
    use crate::root_scan::effective_user;
    use crate::root_scan::inspected_directory::Enumeration;
    use crate::root_scan::inspected_directory::tests::capture_root;
    use crate::root_scan::inspected_directory::tests::log_entry;
    use crate::root_scan::inspected_directory::tests::make_fifo;

    #[test]
    fn partial_enumeration_sweeps_established_pairs_and_counts_the_remainder() {
        for outcome in [
            Enumeration::Incomplete,
            Enumeration::Failed(io::Error::from(ErrorKind::PermissionDenied)),
        ] {
            let root = capture_root();
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
        let root = capture_root();
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
    pub(in super::super) fn sweep_everything(scan: &RootScan) -> usize {
        let mut budget = SweepBudget::default();
        scan.sweep(&mut budget, |_| {
            SweepDisposition::Remove(Path::new("log").to_owned())
        });
        CAPTURE_SWEEP_LIMIT - budget.remaining()
    }

    #[test]
    fn a_pair_that_does_not_fit_the_remaining_allowance_stays_whole() {
        let root = capture_root();
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
        let root = capture_root();
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
        let first = effective_user();
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
        let known = effective_user();
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
                let root = capture_root();
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
    pub(in super::super) fn write_pair(root: &Path) {
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
            let root = capture_root();
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
                let root = capture_root();
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
            let root = capture_root();
            write_pair(root.path());
            fs::hard_link(root.path().join(name), root.path().join("alias")).expect("second link");
            assert_retained_pair(&root);
            assert!(root.path().join("alias").exists());
        }
    }

    #[test]
    fn foreign_file_ownership_is_refused_with_one_fixture_account() {
        for name in ["log", "state/pids/record"] {
            let root = capture_root();
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
            let root = capture_root();
            write_pair(root.path());
            let candidate = root.path().join(name);
            fs::remove_file(&candidate).expect("remove regular file");
            fs::create_dir(&candidate).expect("directory candidate");
            assert_retained_pair(&root);
            fs::remove_dir(&candidate).expect("remove directory candidate");
            make_fifo(&candidate);
            assert_retained_pair(&root);
        }
    }

    #[test]
    fn file_permissions_changed_after_proof_are_rechecked_before_unlink() {
        for name in ["log", "state/pids/record"] {
            let root = capture_root();
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
        let root = capture_root();
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
        let root = capture_root();
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
            let root = capture_root();
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
        let root = capture_root();
        let path = root.path().join("log");
        fs::write(&path, "bounded read").expect("root log");
        let registration = root.path().join(CAPTURE_LIVE_RUNS_DIR).join("123");
        fs::write(&registration, "stale registration").expect("registration");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        assert_eq!(
            log_entry(&scan, "log").read_log().expect("read owned log"),
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
        let root = capture_root();
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
