//! Capture reads and cleanup tied to the directories inspected in one scan.
//!
//! The account directory's final component must be a directory, never a symlink.
//! Ancestor symlinks remain supported, including macOS `/tmp` -> `/private/tmp`.
//! `state` and `pids` are opened separately without following symlinks. Every
//! entry is a sampled or validated basename opened relative to its directory handle.
//!
//! Cleanup proves each regular file through its descriptor: effective ownership,
//! no group or other write bits, and one link. Directory write grants do not
//! authorize or prevent removal; retained directory handles constrain each unlink.

mod inspected_directory;
mod root_history;
mod sweep_authority;

#[cfg(test)]
use std::cell::Cell;
use std::io;
#[cfg(test)]
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

pub(crate) use inspected_directory::Enumeration;
pub(crate) use inspected_directory::ScanEntry;
pub(crate) use inspected_directory::SharedCaptureDirectory;
pub(crate) use inspected_directory::SharedDirectoryState;
pub(crate) use inspected_directory::canonical_capture_path;
pub(crate) use inspected_directory::prepare_shared_directory;
pub(crate) use root_history::RootHistory;
pub(crate) use root_history::RootIncarnation;
use rustix::fs::fstat;
pub(crate) use sweep_authority::EffectiveUser;
pub(crate) use sweep_authority::RootOwner;
pub(crate) use sweep_authority::SweepBudget;
pub(crate) use sweep_authority::SweepDisposition;
pub(crate) use sweep_authority::effective_user;

use self::inspected_directory::InspectedDirectory;
use self::inspected_directory::InspectedDirectoryMetadata;
use self::inspected_directory::Inventory;
use self::inspected_directory::RegistrationAccess;
use self::inspected_directory::RegistrationReadPurpose;
use self::inspected_directory::named_entry;
use self::inspected_directory::open_registration_paths;
use self::inspected_directory::read_tail;
use self::root_history::PreviousRoot;
use self::root_history::RootContinuity;
use self::root_history::TreeIdentity;
use self::sweep_authority::SweepAdmissionRefusal;
use self::sweep_authority::SweepAuthority;
use self::sweep_authority::SweepCounts;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;

/// One bounded registration sample and a lazy legacy-log sample with their handles.
/// No handle or deletion capability is borrowed from a previous scan.
pub(crate) struct RootScan {
    /// Test-only ownership refusal leaves all filesystem observations real.
    #[cfg(test)]
    foreign_files:       Vec<PathBuf>,
    /// Count descriptor log reads, including failures, in scan-deduplication tests.
    #[cfg(test)]
    log_reads:           Cell<usize>,
    /// An opaque value identifies the directory independently of cleanup eligibility.
    incarnation:         RootIncarnation,
    /// Reopened before sweeping so a replacement at this pathname is detected.
    path:                PathBuf,
    /// Ownership comes from fstat of this handle, never pathname metadata.
    root:                InspectedDirectory,
    /// Versioned records bypass enumeration; legacy annotation initializes this sample.
    logs:                OnceLock<Inventory>,
    /// Keep successful traversal handles even when enumeration later fails.
    registration_access: RegistrationAccess,
    /// Unavailable and incomplete registrations never become an empty live set.
    registrations:       Inventory,
    /// Replacement or access recovery disables this scan's cleanup capability.
    continuity:          RootContinuity,
}

impl RootScan {
    /// Reopen the account pathname and sample registrations; legacy logs are lazy.
    pub(crate) fn open(path: &Path, history: &mut RootHistory) -> io::Result<Self> {
        // Removing trailing separators and `.` prevents them bypassing NOFOLLOW
        // on the account directory's final symlink. Ancestors retain OS resolution.
        let path: PathBuf = path.components().collect();
        let root = match InspectedDirectory::open_root(&path) {
            Ok(root) => root,
            Err(error) => {
                history.roots.insert(path, PreviousRoot::Unavailable);
                return Err(error);
            },
        };
        let incarnation = match history.incarnation(&path, &root) {
            Ok(incarnation) => incarnation,
            Err(error) => {
                history.roots.insert(path, PreviousRoot::Unavailable);
                return Err(error);
            },
        };
        let logs = OnceLock::new();
        let (registration_access, registrations) = match open_registration_paths(&root, &path) {
            Ok((state, pids)) => {
                let registrations = Inventory::sample(&pids);
                (RegistrationAccess::Open { state, pids }, registrations)
            },
            Err((path, error)) => (
                RegistrationAccess::Unavailable(path),
                Inventory::failed(error),
            ),
        };
        let identity = TreeIdentity::new(root.identity, &registration_access);
        let continuity = match history
            .roots
            .insert(path.clone(), PreviousRoot::Open(identity))
        {
            None => RootContinuity::Established,
            Some(PreviousRoot::Open(previous)) if previous.same_tree(identity) => {
                RootContinuity::Established
            },
            Some(PreviousRoot::Open(_) | PreviousRoot::Unavailable) => RootContinuity::Changed,
        };
        Ok(Self {
            #[cfg(test)]
            log_reads: std::cell::Cell::new(0),
            #[cfg(test)]
            foreign_files: Vec::new(),
            incarnation,
            path,
            root,
            logs,
            registration_access,
            registrations,
            continuity,
        })
    }

    /// Exercise foreign-file refusal with one real uid and real descriptor checks.
    #[cfg(test)]
    pub fn mark_file_foreign_for_test(&mut self, path: &Path) -> io::Result<()> {
        let parent = path.parent().ok_or(ErrorKind::InvalidInput)?;
        if parent != self.path && parent != self.registration_path() {
            return Err(ErrorKind::InvalidInput.into());
        }
        self.foreign_files.push(path.to_owned());
        Ok(())
    }

    /// The root object alone identifies an incarnation; access metadata is separate.
    pub(crate) const fn incarnation(&self) -> RootIncarnation { self.incarnation }

    /// Verify actual filesystem calls rather than deduplicated diagnostics.
    #[cfg(test)]
    pub(crate) const fn log_read_count(&self) -> usize { self.log_reads.get() }

    /// Entries cannot carry an arbitrary path into a descriptor-relative read.
    pub(crate) fn log_entries(&self) -> impl Iterator<Item = ScanEntry<'_>> {
        self.logs
            .get_or_init(|| Inventory::sample(&self.root))
            .entries
            .iter()
            .map(|entry| ScanEntry {
                directory:         &self.root,
                name:              entry.name(),
                registration_read: RegistrationReadPurpose::Observation,
            })
    }

    /// Failed traversal yields no entries while keeping its separate outcome.
    pub(crate) fn registration_entries(&self) -> impl Iterator<Item = ScanEntry<'_>> {
        let directory = match &self.registration_access {
            RegistrationAccess::Open { pids, .. } => pids,
            RegistrationAccess::Unavailable(_) => &self.root,
        };
        self.registrations
            .entries
            .iter()
            .map(move |entry| ScanEntry {
                directory,
                name: entry.name(),
                registration_read: RegistrationReadPurpose::Observation,
            })
    }

    /// Tests observe the independently bounded legacy-log sample.
    #[cfg(test)]
    fn log_outcome(&self) -> &Enumeration {
        &self
            .logs
            .get_or_init(|| Inventory::sample(&self.root))
            .outcome
    }

    /// Distinguish an empty live-registration directory from failed access.
    pub(crate) const fn registration_outcome(&self) -> &Enumeration { &self.registrations.outcome }

    /// Open a validated basename even when it was published after enumeration.
    pub(crate) fn read_log(&self, basename: &Path) -> io::Result<String> {
        #[cfg(test)]
        self.log_reads.set(self.log_reads.get() + 1);
        let entry = named_entry(&self.root, basename)?;
        let file = entry.open_regular()?;
        let length = file.metadata()?.len();
        read_tail(file, length)
    }

    /// Only the private owned capability can remove a registration and its log.
    /// The callback rechecks the recorded identity immediately before each unlink.
    pub(crate) fn sweep(
        &self,
        budget: &mut SweepBudget,
        registrations: impl FnMut(ScanEntry<'_>) -> SweepDisposition,
    ) -> SweepCounts {
        self.access(effective_user()).map_or_else(
            |_| SweepCounts::unavailable_root(),
            |owned| owned.sweep(budget, registrations),
        )
    }

    /// The displayed owner comes from the same descriptor that supplies captures.
    pub(crate) const fn owner(&self) -> RootOwner { RootOwner::Uid(self.root.identity.owner) }

    /// Report the precise traversal step, including a failure opening `state` itself.
    pub(crate) fn registration_path(&self) -> PathBuf {
        match &self.registration_access {
            RegistrationAccess::Open { .. } => self.path.join(CAPTURE_LIVE_RUNS_DIR),
            RegistrationAccess::Unavailable(path) => path.clone(),
        }
    }

    /// Artifact diagnostics use the retained absolute root, never a new lookup.
    pub(crate) fn path(&self) -> &Path { &self.path }

    /// Retain sampled legacy enumeration failures without forcing a log inventory.
    pub(crate) fn sampled_log_outcome(&self) -> impl Iterator<Item = &Enumeration> {
        self.logs.get().map(|logs| &logs.outcome).into_iter()
    }

    /// Refusals stay inside the sweep; partial inventories retain usable entries.
    fn access(
        &self,
        effective_user: EffectiveUser,
    ) -> Result<SweepAuthority<'_>, SweepAdmissionRefusal> {
        let uid = self.root.identity.owner;
        if effective_user != EffectiveUser::Known(uid) {
            return Err(if effective_user == EffectiveUser::Unavailable {
                SweepAdmissionRefusal::EffectiveUserUnavailable
            } else {
                SweepAdmissionRefusal::ForeignRoot
            });
        }
        self.revalidate_paths()?;
        let RegistrationAccess::Open { state, pids } = &self.registration_access else {
            return Err(SweepAdmissionRefusal::AccessFailure);
        };
        for directory in [&self.root, state, pids] {
            if directory.identity.owner != uid {
                return Err(SweepAdmissionRefusal::ForeignRoot);
            }
        }
        if self.continuity == RootContinuity::Changed {
            return Err(SweepAdmissionRefusal::AccessFailure);
        }
        Ok(SweepAuthority { scan: self, uid })
    }

    /// Reopened paths and retained handles must still identify the sampled tree.
    fn revalidate_paths(&self) -> Result<(), SweepAdmissionRefusal> {
        let current_root = InspectedDirectory::open_root(&self.path)
            .map_err(|_| SweepAdmissionRefusal::AccessFailure)?;
        let (current_state, current_pids) = open_registration_paths(&current_root, &self.path)
            .map_err(|_| SweepAdmissionRefusal::AccessFailure)?;
        let RegistrationAccess::Open { state, pids } = &self.registration_access else {
            return Err(SweepAdmissionRefusal::AccessFailure);
        };
        for (held, current) in [
            (&self.root, &current_root),
            (state, &current_state),
            (pids, &current_pids),
        ] {
            let metadata = fstat(&held.handle).map_err(|_| SweepAdmissionRefusal::AccessFailure)?;
            if !held.identity.same_directory(current.identity)
                || !InspectedDirectoryMetadata::from(&metadata).same_directory(held.identity)
            {
                return Err(SweepAdmissionRefusal::AccessFailure);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::io;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;

    use tempfile::tempdir;

    use super::EffectiveUser;
    use super::RootHistory;
    use super::RootOwner;
    use super::RootScan;
    use super::inspected_directory;
    use super::inspected_directory::Enumeration;
    use super::sweep_authority;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_STATE_DIR;

    #[test]
    fn directory_write_bits_do_not_change_owner_or_sweep_admission() {
        let root = inspected_directory::capture_root();
        fs::set_permissions(root.path(), fs::Permissions::from_mode(0o720)).expect("shared mode");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("read root");
        let owner = scan.root.identity.owner;
        assert_eq!(scan.owner(), RootOwner::Uid(owner));
        assert!(scan.access(EffectiveUser::Known(owner)).is_ok());
        assert!(matches!(
            scan.access(EffectiveUser::Known(owner.wrapping_add(1))),
            Err(super::SweepAdmissionRefusal::ForeignRoot)
        ));
        assert!(matches!(
            scan.access(EffectiveUser::Unavailable),
            Err(super::SweepAdmissionRefusal::EffectiveUserUnavailable)
        ));
    }

    #[test]
    fn failed_traversal_names_the_exact_directory() {
        let root = inspected_directory::capture_root();
        let pids = root.path().join(CAPTURE_LIVE_RUNS_DIR);
        fs::remove_dir(&pids).expect("remove empty pids");
        fs::write(&pids, "not a directory").expect("replace pids with regular file");
        let scan = RootScan::open(root.path(), &mut RootHistory::default())
            .expect("root remains readable");
        assert_eq!(scan.registration_path(), pids);
        assert!(matches!(
            scan.access(super::effective_user()),
            Err(super::SweepAdmissionRefusal::AccessFailure)
        ));
        fs::remove_file(&pids).expect("remove pids replacement");
        let state = root.path().join(CAPTURE_STATE_DIR);
        fs::remove_dir(&state).expect("remove empty state");
        let scan =
            RootScan::open(root.path(), &mut RootHistory::default()).expect("root still readable");
        assert_eq!(scan.registration_path(), state);
    }

    #[test]
    fn named_log_reads_do_not_require_or_populate_the_inventory() {
        let root = inspected_directory::capture_root();
        let scan = RootScan::open(root.path(), &mut RootHistory::default())
            .expect("open before log publication");
        assert!(scan.logs.get().is_none());
        fs::write(root.path().join("later.log"), "later").expect("publish after scan");
        assert_eq!(
            scan.read_log(Path::new("later.log")).expect("named read"),
            "later"
        );
        assert!(scan.logs.get().is_none());
        for name in [
            "",
            ".",
            "..",
            "../later.log",
            "later.log/",
            "later.log/.",
            "/later.log",
        ] {
            assert_eq!(
                scan.read_log(Path::new(name))
                    .expect_err("invalid basename")
                    .kind(),
                io::ErrorKind::InvalidInput
            );
        }
    }

    /// Final-component symlinks are rejected even with trailing slash or dot.
    #[test]
    fn root_symlinks_are_rejected_but_ancestor_symlinks_are_supported() {
        let parent = tempdir().expect("temporary parent");
        let actual = parent.path().join("actual");
        inspected_directory::create_directories(&actual.join("capture"));
        let alias = parent.path().join("alias");
        symlink(&actual, &alias).expect("ancestor symlink");
        let mut history = RootHistory::default();
        let log = actual.join("capture/log");
        fs::write(&log, "cleanup through a trusted ancestor").expect("root log");
        let scan = RootScan::open(&alias.join("capture"), &mut history).expect("ancestor alias");
        assert!(scan.access(super::effective_user()).is_ok());
        assert_eq!(sweep_authority::sweep_everything(&scan), 0);
        assert!(log.exists());
        let root_link = parent.path().join("capture-link");
        symlink(actual.join("capture"), &root_link).expect("root symlink");
        assert!(RootScan::open(&root_link, &mut history).is_err());
        assert!(RootScan::open(&root_link.join(""), &mut history).is_err());
        assert!(RootScan::open(&root_link.join("."), &mut history).is_err());
    }

    /// Each ancestor must be traversed from its held parent without symlinks.
    #[test]
    fn registration_ancestor_symlinks_cannot_redirect_reads_or_cleanup() {
        for ancestor in [CAPTURE_STATE_DIR, CAPTURE_LIVE_RUNS_DIR] {
            let root = inspected_directory::capture_root();
            let other = inspected_directory::capture_root();
            let path = root.path().join(ancestor);
            fs::remove_dir_all(&path).expect("remove original ancestor");
            symlink(other.path().join(ancestor), &path).expect("redirected ancestor");
            fs::write(root.path().join("log"), "retained").expect("root log");
            let scan =
                RootScan::open(root.path(), &mut RootHistory::default()).expect("readable root");
            assert!(matches!(
                scan.registration_outcome(),
                Enumeration::Failed(_)
            ));
            assert_eq!(sweep_authority::sweep_everything(&scan), 0);
            assert!(root.path().join("log").exists());
        }
    }

    /// Replacing a root during the pass cannot redirect reads or authorize sweep.
    #[test]
    fn root_replacement_after_sampling_preserves_both_directory_trees() {
        let parent = tempdir().expect("parent");
        let root = parent.path().join("capture");
        inspected_directory::create_directories(&root);
        sweep_authority::write_pair(&root);
        fs::write(root.join("log"), "old").expect("old log");
        let scan = RootScan::open(&root, &mut RootHistory::default()).expect("sample old root");
        let moved = parent.path().join("moved");
        fs::rename(&root, &moved).expect("move old root");
        inspected_directory::create_directories(&root);
        fs::write(root.join("log"), "new").expect("replacement log");
        assert_eq!(
            inspected_directory::log_entry(&scan, "log")
                .read_log()
                .expect("held directory read"),
            "old"
        );
        assert_eq!(sweep_authority::sweep_everything(&scan), 0);
        assert_eq!(
            fs::read_to_string(root.join("log")).expect("new log survives"),
            "new"
        );
        assert!(moved.join("log").exists());
    }

    /// Reopening descendants catches replacement even when root identity matches.
    #[test]
    fn replacing_registration_ancestors_after_sampling_disables_cleanup() {
        for ancestor in [CAPTURE_STATE_DIR, CAPTURE_LIVE_RUNS_DIR] {
            let root = inspected_directory::capture_root();
            sweep_authority::write_pair(root.path());
            fs::write(root.path().join("log"), "keep").expect("log");
            let scan =
                RootScan::open(root.path(), &mut RootHistory::default()).expect("initial scan");
            fs::rename(
                root.path().join(ancestor),
                root.path().join("old-directory"),
            )
            .expect("move sampled ancestor");
            inspected_directory::create_directories(root.path());
            assert_eq!(sweep_authority::sweep_everything(&scan), 0);
            assert!(root.path().join("log").exists());
        }
    }
}
