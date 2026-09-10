//! Capture reads and cleanup tied to the directories inspected in one scan.
//!
//! The configured root's final component must be a directory, never a symlink.
//! Ancestor symlinks remain supported, including macOS `/tmp` -> `/private/tmp`.
//! `state` and `pids` are opened separately without following symlinks. Every
//! entry is a sampled or validated basename opened relative to its directory handle.
//!
//! Cleanup requires the effective user to own every inspected directory, with
//! no group or other write mode bits, on both Linux and macOS.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::File;
use std::fs::Metadata;
use std::io;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
#[cfg(target_os = "linux")]
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use rustix::fd::OwnedFd;
use rustix::fs::AtFlags;
use rustix::fs::CWD;
use rustix::fs::Dev;
#[cfg(not(target_os = "linux"))]
use rustix::fs::Dir;
use rustix::fs::FileType;
use rustix::fs::Mode;
use rustix::fs::OFlags;
#[cfg(target_os = "linux")]
use rustix::fs::RawDir;
use rustix::fs::Stat;
use rustix::fs::fstat;
use rustix::fs::openat;
use rustix::fs::unlinkat;
use sysinfo::Pid;
use sysinfo::Process;
use sysinfo::ProcessRefreshKind;
use sysinfo::ProcessesToUpdate;
use sysinfo::System;
use sysinfo::UpdateKind;

#[cfg(target_os = "linux")]
use crate::constants::CAPTURE_DIRECTORY_BUFFER_BYTES;
use crate::constants::CAPTURE_DIRECTORY_CHANGED;
use crate::constants::CAPTURE_DIRECTORY_INCOMPLETE;
use crate::constants::CAPTURE_ENTRY_NAME_BYTES;
use crate::constants::CAPTURE_INVALID_BASENAME;
use crate::constants::CAPTURE_INVENTORY_LIMIT;
use crate::constants::CAPTURE_NOT_REGULAR;
use crate::constants::CAPTURE_PIDS_DIR;
use crate::constants::CAPTURE_REGISTRATION_BYTES;
use crate::constants::CAPTURE_REGISTRATION_TOO_LARGE;
use crate::constants::CAPTURE_STATE_DIR;
use crate::constants::CAPTURE_SWEEP_LIMIT;
use crate::constants::RUN_LOG_TAIL_BYTES;

/// An empty complete inventory is evidence; a truncated or failed one is not.
#[derive(Debug)]
pub(crate) enum Enumeration {
    /// Every directory entry was visited successfully.
    Complete,
    /// A count or filename limit stopped the inventory before end of directory.
    Incomplete,
    /// Keep the actual access or enumeration failure for the caller to observe.
    Failed(io::Error),
}

impl Enumeration {
    /// Preserve the cause when a complete inventory is required for cleanup.
    fn require_complete(&self) -> io::Result<()> {
        match self {
            Self::Complete => Ok(()),
            Self::Incomplete => Err(io::Error::other(CAPTURE_DIRECTORY_INCOMPLETE)),
            Self::Failed(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

/// Whether this scan may build on the previous scan's directory identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RootContinuity {
    /// First successful observation, or the same inspected directory tree.
    Established,
    /// Replaced directories or recovery from failed access prohibit this sweep.
    Changed,
}

/// Registration traversal either holds both ancestors or retains its failure.
enum RegistrationAccess {
    /// Both handles were derived from the inspected root without symlink traversal.
    Open {
        /// Prevent state replacement from being hidden behind an unchanged root.
        state: InspectedDirectory,
        /// Reads and removals resolve registration basenames against this handle.
        pids:  InspectedDirectory,
    },
    /// The registration inventory carries the corresponding access error.
    Unavailable,
}

/// One bounded registration sample and a lazy legacy-log sample with their handles.
/// No handle or deletion capability is borrowed from a previous scan.
pub(crate) struct RootScan {
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
    /// Reopen the configured pathname and sample registrations; legacy logs are lazy.
    pub(crate) fn open(path: &Path, history: &mut RootHistory) -> io::Result<Self> {
        // Removing trailing separators and `.` prevents them bypassing NOFOLLOW
        // on the configured root's final symlink. Ancestors retain OS resolution.
        let path: PathBuf = path.components().collect();
        let root = match InspectedDirectory::open_root(&path) {
            Ok(root) => root,
            Err(error) => {
                history.roots.insert(path, PreviousRoot::Unavailable);
                return Err(error);
            },
        };
        let logs = OnceLock::new();
        let (registration_access, registrations) = match open_registrations(&root) {
            Ok((state, pids)) => {
                let registrations = Inventory::sample(&pids);
                (RegistrationAccess::Open { state, pids }, registrations)
            },
            Err(error) => (RegistrationAccess::Unavailable, Inventory::failed(error)),
        };
        let identity = TreeIdentity::new(root.identity, &registration_access);
        let continuity = match history
            .roots
            .insert(path.clone(), PreviousRoot::Open(identity))
        {
            None => RootContinuity::Established,
            Some(PreviousRoot::Open(previous)) if previous == identity => {
                RootContinuity::Established
            },
            Some(PreviousRoot::Open(_) | PreviousRoot::Unavailable) => RootContinuity::Changed,
        };
        Ok(Self {
            path,
            root,
            logs,
            registration_access,
            registrations,
            continuity,
        })
    }

    /// Entries cannot carry an arbitrary path into a descriptor-relative read.
    pub(crate) fn log_entries(&self) -> impl Iterator<Item = ScanEntry<'_>> {
        self.logs
            .get_or_init(|| Inventory::sample(&self.root))
            .entries
            .iter()
            .map(|entry| ScanEntry {
                directory: &self.root,
                name:      entry.name(),
            })
    }

    /// Failed traversal yields no entries while keeping its separate outcome.
    pub(crate) fn registration_entries(&self) -> impl Iterator<Item = ScanEntry<'_>> {
        let directory = match &self.registration_access {
            RegistrationAccess::Open { pids, .. } => pids,
            RegistrationAccess::Unavailable => &self.root,
        };
        self.registrations
            .entries
            .iter()
            .map(move |entry| ScanEntry {
                directory,
                name: entry.name(),
            })
    }

    /// A bounded root sample must be complete before cleanup is possible.
    #[cfg(test)]
    pub(crate) fn log_outcome(&self) -> &Enumeration {
        &self
            .logs
            .get_or_init(|| Inventory::sample(&self.root))
            .outcome
    }

    /// Distinguish an empty live-registration directory from failed access.
    pub(crate) const fn registration_outcome(&self) -> &Enumeration { &self.registrations.outcome }

    /// Open a validated basename even when it was published after enumeration.
    pub(crate) fn read_log(&self, basename: &Path) -> io::Result<String> {
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
    ) {
        match self.access() {
            RootAccess::Owned(owned) => owned.sweep(budget, registrations),
            RootAccess::Foreign(foreign) => foreign.preserve(),
        }
    }

    /// Ownership, permissions, complete inventories and a fresh pathname check
    /// jointly justify the borrowed cleanup capability.
    fn access(&self) -> RootAccess<'_> {
        let RegistrationAccess::Open { state, pids } = &self.registration_access else {
            return RootAccess::Foreign(ForeignRoot { scan: self });
        };
        let EffectiveUser::Known(effective_uid) = effective_user() else {
            return RootAccess::Foreign(ForeignRoot { scan: self });
        };
        if self.continuity == RootContinuity::Established
            && self
                .logs
                .get()
                .is_none_or(|logs| logs.outcome.require_complete().is_ok())
            && self.registration_outcome().require_complete().is_ok()
            && [&self.root, state, pids]
                .into_iter()
                .all(|directory| directory.identity.exclusive_owner(effective_uid))
            && self.revalidate().is_ok()
        {
            RootAccess::Owned(OwnedRoot { scan: self, pids })
        } else {
            RootAccess::Foreign(ForeignRoot { scan: self })
        }
    }

    /// Fresh traversal catches renamed or replaced directories at the configured
    /// pathname; fstat also catches permission changes on the held handles.
    fn revalidate(&self) -> io::Result<()> {
        let current_root = InspectedDirectory::open_root(&self.path)?;
        let (current_state, current_pids) = open_registrations(&current_root)?;
        let RegistrationAccess::Open { state, pids } = &self.registration_access else {
            return Err(io::Error::other(CAPTURE_DIRECTORY_CHANGED));
        };
        for (held, current) in [
            (&self.root, &current_root),
            (state, &current_state),
            (pids, &current_pids),
        ] {
            if held.identity != current.identity
                || DirectoryIdentity::from(&fstat(&held.handle)?) != held.identity
            {
                return Err(io::Error::other(CAPTURE_DIRECTORY_CHANGED));
            }
        }
        Ok(())
    }
}

/// Persisted by the caller across scans; it retains identities, never handles or
/// cleanup capabilities. Each configured root has independent access history.
#[derive(Default)]
pub(crate) struct RootHistory {
    /// An access failure is retained so recovery cannot authorize immediate sweep.
    roots: HashMap<PathBuf, PreviousRoot>,
}

/// A successful observation differs from a previously inaccessible pathname.
enum PreviousRoot {
    /// Compare all three directory identities on the next observation.
    Open(TreeIdentity),
    /// No previous descriptor can justify cleanup after failed access.
    Unavailable,
}

/// Detect root and registration-ancestor replacement across successive scans.
#[derive(Clone, Copy, Eq, PartialEq)]
struct TreeIdentity {
    /// The configured root itself may change without its pathname changing.
    root:          DirectoryIdentity,
    /// Missing registration ancestors also matter when they become available.
    registrations: RegistrationIdentity,
}

impl TreeIdentity {
    /// Preserve unavailable traversal as a distinct identity state.
    const fn new(root: DirectoryIdentity, access: &RegistrationAccess) -> Self {
        let registrations = match access {
            RegistrationAccess::Open { state, pids } => RegistrationIdentity::Open {
                state: state.identity,
                pids:  pids.identity,
            },
            RegistrationAccess::Unavailable => RegistrationIdentity::Unavailable,
        };
        Self {
            root,
            registrations,
        }
    }
}

/// Ancestor identities prevent a new live set authorizing deletion of old logs.
#[derive(Clone, Copy, Eq, PartialEq)]
enum RegistrationIdentity {
    /// Both ancestors were inspectable in this observation.
    Open {
        /// State ownership alone does not establish ownership of pids.
        state: DirectoryIdentity,
        /// The directory whose enumeration determines the live set.
        pids:  DirectoryIdentity,
    },
    /// Failed traversal is never equivalent to an empty registration directory.
    Unavailable,
}

/// Device, inode, owner and permissions describe the inspected directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct DirectoryIdentity {
    /// Inode numbers alone are not unique across filesystems.
    device: Dev,
    /// Detect replacement on the same filesystem.
    inode:  u64,
    /// Only the effective owner can receive a deletion capability.
    owner:  u32,
    /// A different group or permission mask also invalidates a prior observation.
    group:  u32,
    /// No non-owner may have directory write permission.
    mode:   Mode,
}

impl From<&Stat> for DirectoryIdentity {
    fn from(stat: &Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode:  stat.st_ino,
            owner:  stat.st_uid,
            group:  stat.st_gid,
            mode:   Mode::from_raw_mode(stat.st_mode),
        }
    }
}

impl DirectoryIdentity {
    /// Require the effective owner and forbid group or other directory writes.
    /// Linux POSIX ACL write masks appear in group mode bits. On macOS, an ACL
    /// can grant another account write access without changing those bits; this
    /// mode-based check does not detect that residual exposure.
    fn exclusive_owner(self, effective_uid: u32) -> bool {
        self.owner == effective_uid && !self.mode.intersects(Mode::WGRP | Mode::WOTH)
    }
}

/// fstat is performed after opening; metadata never chooses a different handle.
struct InspectedDirectory {
    /// All later operations stay relative to this directory.
    handle:   OwnedFd,
    /// The ownership and identity inspected on this exact descriptor.
    identity: DirectoryIdentity,
}

impl InspectedDirectory {
    /// Trusted ancestor symlinks remain usable; the root itself cannot be one.
    fn open_root(path: &Path) -> io::Result<Self> {
        Self::inspect(openat(CWD, path, directory_flags(), Mode::empty())?)
    }

    /// State and pids must each be a real child directory of their held parent.
    fn child(&self, name: &str) -> io::Result<Self> {
        Self::inspect(openat(
            &self.handle,
            name,
            directory_flags(),
            Mode::empty(),
        )?)
    }

    /// Successful fstat is required before the descriptor can authorize access.
    fn inspect(handle: OwnedFd) -> io::Result<Self> {
        let identity = DirectoryIdentity::from(&fstat(&handle)?);
        Ok(Self { handle, identity })
    }
}

/// Keep partial inventories observable while forbidding their use for cleanup.
struct Inventory {
    /// Names are stored inline; allocation grows only as entries are sampled.
    entries: Vec<InventoryEntry>,
    /// Failure details survive even when earlier entries were readable.
    outcome: Enumeration,
}

impl Inventory {
    /// Failed traversal has no entries and is never reported as complete.
    const fn failed(error: io::Error) -> Self {
        Self {
            entries: Vec::new(),
            outcome: Enumeration::Failed(error),
        }
    }

    /// Linux getdents fills fixed caller storage without allocating per entry.
    #[cfg(target_os = "linux")]
    fn sample(directory: &InspectedDirectory) -> Self {
        let mut inventory = Self {
            entries: Vec::new(),
            outcome: Enumeration::Complete,
        };
        let mut buffer = [MaybeUninit::uninit(); CAPTURE_DIRECTORY_BUFFER_BYTES];
        let mut entries = RawDir::new(&directory.handle, &mut buffer);
        while let Some(entry) = entries.next() {
            match entry {
                Ok(entry) => inventory.push(entry.file_name().to_bytes()),
                Err(error) => inventory.outcome = Enumeration::Failed(error.into()),
            }
            if !matches!(inventory.outcome, Enumeration::Complete) {
                break;
            }
        }
        inventory
    }

    /// rustix supplies a safe readdir wrapper on macOS; retained names still use
    /// fixed inline storage and the same count and error rules as Linux.
    #[cfg(not(target_os = "linux"))]
    fn sample(directory: &InspectedDirectory) -> Self {
        let entries = match Dir::read_from(&directory.handle) {
            Ok(entries) => entries,
            Err(error) => return Self::failed(error.into()),
        };
        let mut inventory = Self {
            entries: Vec::new(),
            outcome: Enumeration::Complete,
        };
        for entry in entries {
            match entry {
                Ok(entry) => inventory.push(entry.file_name().to_bytes()),
                Err(error) => inventory.outcome = Enumeration::Failed(error.into()),
            }
            if !matches!(inventory.outcome, Enumeration::Complete) {
                break;
            }
        }
        inventory
    }

    /// Reaching the limit is explicitly incomplete, including at exact capacity.
    fn push(&mut self, name: &[u8]) {
        if name == b"." || name == b".." {
            return;
        }
        if name.len() > CAPTURE_ENTRY_NAME_BYTES {
            self.outcome = Enumeration::Incomplete;
            return;
        }
        let mut entry = InventoryEntry {
            name:   [0; CAPTURE_ENTRY_NAME_BYTES],
            length: name.len(),
        };
        entry.name[..name.len()].copy_from_slice(name);
        self.entries.push(entry);
        if self.entries.len() >= CAPTURE_INVENTORY_LIMIT {
            self.outcome = Enumeration::Incomplete;
        }
    }
}

/// A sampled basename has no pathname prefix and owns no per-entry allocation.
struct InventoryEntry {
    /// Fixed storage also bounds inventories containing unrelated long filenames.
    name:   [u8; CAPTURE_ENTRY_NAME_BYTES],
    /// Only these bytes came from the directory enumeration.
    length: usize,
}

impl InventoryEntry {
    /// Directory APIs supply basenames; callers cannot construct new entries.
    fn name(&self) -> &Path { Path::new(OsStr::from_bytes(&self.name[..self.length])) }
}

/// A readable basename borrows the inspected directory that constrains its open.
#[derive(Clone, Copy)]
pub(crate) struct ScanEntry<'scan> {
    /// A replaced pathname cannot redirect the entry's open to another directory.
    directory: &'scan InspectedDirectory,
    /// A sampled or explicitly validated basename carries no pathname prefix.
    name:      &'scan Path,
}

impl<'scan> ScanEntry<'scan> {
    /// Classify filenames without allocating a joined path for every entry.
    pub(crate) const fn name(&self) -> &'scan Path { self.name }

    /// NONBLOCK prevents FIFO opens from waiting; fstat rejects every non-regular
    /// type before reads. It does not impose a deadline on regular file I/O.
    fn open_regular(&self) -> io::Result<File> {
        let handle = openat(
            &self.directory.handle,
            self.name(),
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        if FileType::from_raw_mode(fstat(&handle)?.st_mode) != FileType::RegularFile {
            return Err(io::Error::other(CAPTURE_NOT_REGULAR));
        }
        Ok(File::from(handle))
    }

    /// Metadata chooses only the starting position; `Read::take` enforces the byte
    /// bound even when the file grows after that metadata was inspected.
    pub(crate) fn read_log(&self) -> io::Result<String> {
        let file = self.open_regular()?;
        let length = file.metadata()?.len();
        read_tail(file, length)
    }

    /// Read one byte past the record cap so oversize input is rejected rather
    /// than accepted as a complete truncated registration.
    pub(crate) fn read_registration(&self) -> io::Result<RegistrationObservation> {
        read_registration_file(self.open_regular()?)
    }
}

/// Contents and metadata come from one descriptor, even after a pathname replacement.
#[derive(Debug)]
pub(crate) struct RegistrationObservation {
    /// NUL-framed record bytes bounded independently of metadata.
    pub(crate) bytes:    Vec<u8>,
    /// Includes the modification time of the same file that supplied the bytes.
    pub(crate) metadata: Metadata,
}

/// Read one already opened registration without consulting its pathname again.
fn read_registration_file(file: File) -> io::Result<RegistrationObservation> {
    let metadata = file.metadata()?;
    let mut bytes = Vec::new();
    file.take(CAPTURE_REGISTRATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > CAPTURE_REGISTRATION_BYTES {
        return Err(io::Error::other(CAPTURE_REGISTRATION_TOO_LARGE));
    }
    Ok(RegistrationObservation { bytes, metadata })
}

/// Validate external record names before any descriptor-relative open.
fn named_entry<'scan>(
    directory: &'scan InspectedDirectory,
    basename: &'scan Path,
) -> io::Result<ScanEntry<'scan>> {
    if basename.as_os_str().as_bytes().contains(&b'/')
        || !matches!(
            basename.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            CAPTURE_INVALID_BASENAME,
        ));
    }
    Ok(ScanEntry {
        directory,
        name: basename,
    })
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

/// One internal dispatch distinguishes cleanup authority from read-only access.
enum RootAccess<'scan> {
    /// This scan proved both ownership and complete directory enumeration.
    Owned(OwnedRoot<'scan>),
    /// Foreign, writable-by-others, incomplete or changed roots retain no authority.
    Foreign(ForeignRoot<'scan>),
}

/// Private and non-clonable: no caller can retain authority beyond this scan.
struct OwnedRoot<'scan> {
    /// Owns the handles and complete inventories that justify deletion.
    scan: &'scan RootScan,
    /// Available only after both registration ancestors passed inspection.
    pids: &'scan InspectedDirectory,
}

impl OwnedRoot<'_> {
    /// Retained entries never consume allowance; each attempted pair reserves two.
    fn sweep(
        self,
        budget: &mut SweepBudget,
        mut registrations: impl FnMut(ScanEntry<'_>) -> SweepDisposition,
    ) {
        for entry in self.scan.registration_entries() {
            if budget.remaining() < 2 {
                return;
            }
            let SweepDisposition::Remove(log) = registrations(entry) else {
                continue;
            };
            if self.scan.revalidate().is_err()
                || named_entry(&self.scan.root, &log).is_err()
                || !budget.charge_pair()
            {
                continue;
            }
            // A failed log removal must leave its only deletion evidence intact.
            match unlinkat(&self.scan.root.handle, &log, AtFlags::empty()) {
                Ok(()) | Err(rustix::io::Errno::NOENT) => {},
                Err(_) => continue,
            }
            // Publication never reuses a generation. Re-reading also protects
            // older records replaced before this final confirmation.
            if registrations(entry) == SweepDisposition::Remove(log)
                && self.scan.revalidate().is_ok()
            {
                let _ = unlinkat(&self.pids.handle, entry.name(), AtFlags::empty());
            }
        }
    }
}

/// Readable observations never imply cleanup authority over foreign data.
struct ForeignRoot<'scan> {
    /// Shares this scan's immutable observations without acquiring removal rights.
    scan: &'scan RootScan,
}

impl ForeignRoot<'_> {
    /// Consume the read-only capability without invoking either cleanup callback.
    const fn preserve(self) { let _ = self.scan; }
}

/// Every traversed directory rejects symlinks and closes across exec.
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Opening both ancestors separately prevents intermediate symlink redirection.
fn open_registrations(
    root: &InspectedDirectory,
) -> io::Result<(InspectedDirectory, InspectedDirectory)> {
    let state = root.child(CAPTURE_STATE_DIR)?;
    let pids = state.child(CAPTURE_PIDS_DIR)?;
    Ok((state, pids))
}

/// A missing own-process uid disables cleanup instead of selecting a default uid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EffectiveUser {
    /// Read from this process's effective identity, rather than its real uid.
    Known(u32),
    /// The safe platform process API could not establish the effective owner.
    Unavailable,
}

impl EffectiveUser {
    /// Retain the first identity result, including failure, for every later scan.
    fn cached(cache: &OnceLock<Self>, read: impl FnOnce() -> Self) -> Self {
        *cache.get_or_init(read)
    }
}

/// The effective identity is fixed for this process; failed reads stay unavailable.
fn effective_user() -> EffectiveUser {
    /// Share one observation across all roots and scans, including a failed read.
    static EFFECTIVE_USER: OnceLock<EffectiveUser> = OnceLock::new();
    EffectiveUser::cached(&EFFECTIVE_USER, read_effective_user)
}

/// Refresh only our process through the existing safe sysinfo dependency. rustix
/// fs has no geteuid, and this avoids relying on unrelated process feature flags.
fn read_effective_user() -> EffectiveUser {
    let mut system = System::new();
    let pid = Pid::from_u32(std::process::id());
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_user(UpdateKind::Always),
    );
    system
        .process(pid)
        .and_then(Process::effective_user_id)
        .map_or(EffectiveUser::Unavailable, |uid| {
            EffectiveUser::Known(**uid)
        })
}

/// `Read::take` remains authoritative when a file grows after the length sample.
fn read_tail(mut file: File, length: u64) -> io::Result<String> {
    file.seek(SeekFrom::Start(length.saturating_sub(RUN_LOG_TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.take(RUN_LOG_TAIL_BYTES).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::cell::Cell;
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::OnceLock;

    use rustix::fs::CWD;
    use rustix::fs::Mode;
    use rustix::fs::mkfifoat;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::EffectiveUser;
    use super::Enumeration;
    use super::InspectedDirectory;
    use super::Inventory;
    use super::RootAccess;
    use super::RootContinuity;
    use super::RootHistory;
    use super::RootScan;
    use super::ScanEntry;
    use super::SweepBudget;
    use super::SweepDisposition;
    use super::effective_user;
    use super::read_tail;
    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_NOT_REGULAR;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::CAPTURE_REGISTRATION_TOO_LARGE;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::CAPTURE_SWEEP_LIMIT;
    use crate::constants::RUN_LOG_TAIL_BYTES;

    /// Use one account and explicit directory modes independent of its umask.
    fn capture_root() -> TempDir {
        let root = tempdir().expect("temporary capture root");
        create_directories(root.path());
        root
    }

    /// Replacement fixtures use the same independently owned directory layout.
    fn create_directories(root: &Path) {
        fs::create_dir_all(root.join(CAPTURE_LIVE_RUNS_DIR)).expect("registration directories");
        for path in [
            root.to_owned(),
            root.join(CAPTURE_STATE_DIR),
            root.join(CAPTURE_LIVE_RUNS_DIR),
        ] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .expect("private directory");
        }
    }

    /// Find a sampled entry without constructing a public read or remove target.
    fn log_entry<'scan>(scan: &'scan RootScan, name: &str) -> ScanEntry<'scan> {
        scan.log_entries()
            .find(|entry| entry.name() == Path::new(name))
            .expect("sampled log entry")
    }

    /// Attempt every artifact through the production capability dispatcher.
    fn sweep_everything(scan: &RootScan) -> usize {
        let mut budget = SweepBudget::default();
        scan.sweep(&mut budget, |_| {
            SweepDisposition::Remove(Path::new("log").to_owned())
        });
        CAPTURE_SWEEP_LIMIT - budget.remaining()
    }

    /// Metadata is read from the open descriptor even after its name is replaced.
    #[test]
    fn registration_bytes_and_timestamp_always_come_from_the_same_descriptor() {
        let root = capture_root();
        let path = root.path().join("record");
        fs::write(&path, "original").expect("original record");
        let file = File::open(&path).expect("original descriptor");
        let original_time = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(123);
        file.set_times(std::fs::FileTimes::new().set_modified(original_time))
            .expect("original timestamp");
        fs::rename(&path, root.path().join("moved")).expect("move opened record");
        fs::write(&path, "replacement").expect("replacement record");
        let observed = super::read_registration_file(file).expect("read held descriptor");
        assert_eq!(observed.bytes, b"original");
        assert_eq!(
            observed.metadata.modified().expect("descriptor timestamp"),
            original_time
        );
        assert_ne!(
            fs::metadata(&path)
                .expect("replacement metadata")
                .modified()
                .expect("replacement timestamp"),
            original_time
        );
    }

    #[test]
    fn named_log_reads_do_not_require_or_populate_the_inventory() {
        let root = capture_root();
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

    /// Final-component symlinks are rejected even with trailing slash or dot.
    #[test]
    fn root_symlinks_are_rejected_but_ancestor_symlinks_are_supported() {
        let parent = tempdir().expect("temporary parent");
        let actual = parent.path().join("actual");
        create_directories(&actual.join("capture"));
        let alias = parent.path().join("alias");
        symlink(&actual, &alias).expect("ancestor symlink");
        let mut history = RootHistory::default();
        let log = actual.join("capture/log");
        fs::write(&log, "cleanup through a trusted ancestor").expect("root log");
        let scan = RootScan::open(&alias.join("capture"), &mut history).expect("ancestor alias");
        assert!(matches!(scan.access(), RootAccess::Owned(_)));
        assert_eq!(sweep_everything(&scan), 0);
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
            let root = capture_root();
            let other = capture_root();
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
            assert_eq!(sweep_everything(&scan), 0);
            assert!(root.path().join("log").exists());
        }
    }

    /// Symlink targets, including an endless device, are never opened.
    #[test]
    fn symlink_entries_are_rejected_before_reading() {
        let root = capture_root();
        fs::write(root.path().join("regular"), "secret").expect("regular target");
        symlink("regular", root.path().join("link")).expect("relative file symlink");
        symlink("/dev/zero", root.path().join("zero")).expect("device symlink");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        for name in ["link", "zero"] {
            let entry = log_entry(&scan, name);
            assert!(entry.read_log().is_err());
            assert!(entry.read_registration().is_err());
        }
    }

    /// NONBLOCK lets FIFO open return without a writer; fstat then rejects it.
    #[test]
    fn fifo_directory_and_socket_entries_are_not_regular_files() {
        let root = capture_root();
        mkfifoat(CWD, root.path().join("fifo"), Mode::RUSR | Mode::WUSR).expect("create FIFO");
        fs::create_dir(root.path().join("directory")).expect("create directory");
        let socket = UnixListener::bind(root.path().join("socket")).expect("create socket");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        for name in ["fifo", "directory"] {
            let error = log_entry(&scan, name)
                .read_log()
                .expect_err("special file rejected");
            assert_eq!(error.to_string(), CAPTURE_NOT_REGULAR);
            assert!(log_entry(&scan, name).read_registration().is_err());
        }
        assert!(log_entry(&scan, "socket").read_log().is_err());
        drop(socket);
    }

    /// A basename replaced after enumeration is still opened with NOFOLLOW.
    #[test]
    fn an_entry_replaced_by_a_symlink_after_sampling_is_rejected() {
        let root = capture_root();
        let path = root.path().join("log");
        fs::write(&path, "before").expect("original log");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        fs::remove_file(&path).expect("remove original entry");
        symlink("/dev/zero", &path).expect("replace entry with symlink");
        assert!(log_entry(&scan, "log").read_log().is_err());
    }

    /// Large regular logs return only their tail, retaining lossy UTF-8 behavior.
    #[test]
    fn log_reads_keep_the_tail_cap_and_lossy_utf8() {
        let root = capture_root();
        let cap = usize::try_from(RUN_LOG_TAIL_BYTES).expect("tail cap fits usize");
        let mut bytes = vec![b'x'; cap * 2];
        bytes.push(0xff);
        fs::write(root.path().join("log"), bytes).expect("long invalid UTF-8 log");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let tail = log_entry(&scan, "log")
            .read_log()
            .expect("bounded lossy tail");
        assert_eq!(tail.chars().count(), cap);
        assert!(tail.ends_with('\u{fffd}'));
    }

    /// Growth after the metadata sample cannot bypass the independent take cap.
    #[test]
    fn log_growth_after_the_length_sample_is_still_bounded() {
        let root = capture_root();
        let path = root.path().join("growing");
        fs::write(&path, "").expect("initially empty log");
        let file = File::open(&path).expect("read handle");
        let sampled_length = file.metadata().expect("length before append").len();
        let cap = usize::try_from(RUN_LOG_TAIL_BYTES).expect("tail cap fits usize");
        OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append handle")
            .write_all(&vec![b'x'; cap * 2])
            .expect("grow after metadata");
        assert_eq!(
            read_tail(file, sampled_length).expect("bounded tail").len(),
            cap
        );
    }

    /// Exactly capped records are readable; a single extra byte is not.
    #[test]
    fn registration_reads_detect_oversize_instead_of_accepting_a_prefix() {
        let root = capture_root();
        let cap = usize::try_from(CAPTURE_REGISTRATION_BYTES).expect("record cap fits usize");
        fs::write(root.path().join("record"), vec![b'x'; cap]).expect("record at cap");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
        let entry = log_entry(&scan, "record");
        assert_eq!(
            entry
                .read_registration()
                .expect("exact cap allowed")
                .bytes
                .len(),
            cap
        );
        OpenOptions::new()
            .append(true)
            .open(root.path().join("record"))
            .expect("append handle")
            .write_all(b"x")
            .expect("one extra byte");
        assert_eq!(
            entry
                .read_registration()
                .expect_err("oversize record rejected")
                .to_string(),
            CAPTURE_REGISTRATION_TOO_LARGE
        );
    }

    /// A real getdents/readdir failure survives instead of becoming empty success.
    #[test]
    fn enumeration_errors_preserve_their_io_cause() {
        let root = capture_root();
        let path = root.path().join("regular");
        fs::write(&path, "").expect("ordinary file");
        let directory = InspectedDirectory::inspect(File::open(path).expect("file handle").into())
            .expect("fstat file for failing enumeration");
        let inventory = Inventory::sample(&directory);
        assert!(inventory.entries.is_empty());
        assert!(matches!(inventory.outcome, Enumeration::Failed(_)));
        assert_eq!(
            inventory
                .outcome
                .require_complete()
                .expect_err("enumeration error retained")
                .kind(),
            io::ErrorKind::NotADirectory
        );
    }

    /// Empty directories need no heap storage for their retained entry names.
    #[test]
    fn empty_inventories_do_not_allocate_entry_storage() {
        let root = tempdir().expect("empty directory");
        let directory = InspectedDirectory::open_root(root.path()).expect("inspected root");
        let inventory = Inventory::sample(&directory);
        assert!(matches!(inventory.outcome, Enumeration::Complete));
        assert!(inventory.entries.is_empty());
        assert_eq!(inventory.entries.capacity(), 0);
    }

    /// A small directory allocates for its sample rather than the inventory cap.
    #[test]
    fn small_inventories_do_not_reserve_the_whole_bound() {
        let root = capture_root();
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("small scan");
        assert!(matches!(scan.log_outcome(), Enumeration::Complete));
        assert_eq!(scan.log_entries().count(), 1);
        assert!(
            scan.logs.get().expect("sampled logs").entries.capacity() < CAPTURE_INVENTORY_LIMIT
        );
        assert_eq!(scan.registrations.entries.capacity(), 0);
    }

    /// Every unrelated entry counts toward the memory bound and disables sweep.
    #[test]
    fn large_unrelated_inventory_is_explicitly_incomplete_and_cannot_sweep() {
        let root = capture_root();
        for number in 0..CAPTURE_INVENTORY_LIMIT {
            fs::write(root.path().join(format!("unrelated-{number}")), "")
                .expect("unrelated entry");
        }
        fs::write(root.path().join("log"), "keep").expect("log beside unrelated entries");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("bounded scan");
        assert!(matches!(scan.log_outcome(), Enumeration::Incomplete));
        assert_eq!(scan.log_entries().count(), CAPTURE_INVENTORY_LIMIT);
        assert_eq!(sweep_everything(&scan), 0);
        assert!(root.path().join("log").exists());
    }

    /// Ownership of all ancestors matters, independent of the test user's uid.
    #[test]
    fn ownership_requires_the_effective_uid_and_no_nonowner_write_bits() {
        let root = capture_root();
        let directory = InspectedDirectory::open_root(root.path()).expect("inspected root");
        let identity = directory.identity;
        assert!(identity.exclusive_owner(identity.owner));
        assert!(!identity.exclusive_owner(identity.owner.wrapping_add(1)));
        for mode in [Mode::WGRP, Mode::WOTH] {
            let mut writable = identity;
            writable.mode |= mode;
            assert!(!writable.exclusive_owner(identity.owner));
        }
        assert!(matches!(effective_user(), EffectiveUser::Known(uid) if uid == identity.owner));
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

    /// The root owner cannot sweep an ancestor another account can write.
    #[test]
    fn every_group_or_other_writable_ancestor_disables_cleanup() {
        for ancestor in ["", CAPTURE_STATE_DIR, CAPTURE_LIVE_RUNS_DIR] {
            for mode in [0o720, 0o702] {
                let root = capture_root();
                fs::write(root.path().join("log"), "keep").expect("root log");
                fs::set_permissions(root.path().join(ancestor), fs::Permissions::from_mode(mode))
                    .expect("nonowner write permission");
                let scan =
                    RootScan::open(root.path(), &mut RootHistory::default()).expect("read root");
                assert!(matches!(scan.access(), RootAccess::Foreign(_)));
                assert_eq!(sweep_everything(&scan), 0);
                assert!(root.path().join("log").exists());
            }
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

    /// Replacing a root during the pass cannot redirect reads or authorize sweep.
    #[test]
    fn root_replacement_after_sampling_preserves_both_directory_trees() {
        let parent = tempdir().expect("parent");
        let root = parent.path().join("capture");
        create_directories(&root);
        fs::write(root.join("log"), "old").expect("old log");
        let scan = RootScan::open(&root, &mut RootHistory::default()).expect("sample old root");
        let moved = parent.path().join("moved");
        fs::rename(&root, &moved).expect("move old root");
        create_directories(&root);
        fs::write(root.join("log"), "new").expect("replacement log");
        assert_eq!(
            log_entry(&scan, "log")
                .read_log()
                .expect("held directory read"),
            "old"
        );
        assert_eq!(sweep_everything(&scan), 0);
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
            let root = capture_root();
            fs::write(root.path().join("log"), "keep").expect("log");
            let scan =
                RootScan::open(root.path(), &mut RootHistory::default()).expect("initial scan");
            fs::rename(
                root.path().join(ancestor),
                root.path().join("old-directory"),
            )
            .expect("move sampled ancestor");
            create_directories(root.path());
            assert_eq!(sweep_everything(&scan), 0);
            assert!(root.path().join("log").exists());
        }
    }

    /// Permission changes between sample and sweep revoke the capability.
    #[test]
    fn changing_permissions_after_sampling_disables_cleanup() {
        let root = capture_root();
        fs::write(root.path().join("log"), "keep").expect("log");
        let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("initial scan");
        fs::set_permissions(
            root.path().join(CAPTURE_LIVE_RUNS_DIR),
            fs::Permissions::from_mode(0o720),
        )
        .expect("make registration directory group-writable");
        assert_eq!(sweep_everything(&scan), 0);
        assert!(root.path().join("log").exists());
    }

    /// A replaced or newly accessible pathname must first establish a new history.
    #[test]
    fn identity_changes_and_access_recovery_disable_one_scan_of_cleanup() {
        let parent = tempdir().expect("parent");
        let root = parent.path().join("capture");
        let mut history = RootHistory::default();
        assert!(RootScan::open(&root, &mut history).is_err());
        create_directories(&root);
        fs::write(root.join("log"), "first").expect("first log");
        let recovered = RootScan::open(&root, &mut history).expect("recovered root");
        assert_eq!(recovered.continuity, RootContinuity::Changed);
        assert_eq!(sweep_everything(&recovered), 0);
        let established = RootScan::open(&root, &mut history).expect("stable root");
        assert_eq!(established.continuity, RootContinuity::Established);
        fs::rename(&root, parent.path().join("old")).expect("replace root");
        create_directories(&root);
        fs::write(root.join("log"), "new").expect("new log");
        let replaced = RootScan::open(&root, &mut history).expect("replacement root");
        assert_eq!(replaced.continuity, RootContinuity::Changed);
        assert_eq!(sweep_everything(&replaced), 0);
        assert!(root.join("log").exists());
    }
}
