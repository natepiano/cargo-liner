//! Descriptor-relative directory inspection and bounded capture reads.

use std::ffi::OsStr;
use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::io;
use std::io::Error;
use std::io::ErrorKind;
use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
#[cfg(target_os = "linux")]
use std::mem::MaybeUninit;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use rustix::fd::OwnedFd;
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
use rustix::fs::fchmod;
use rustix::fs::fstat;
use rustix::fs::openat;

use super::sweep_authority;
use super::sweep_authority::EffectiveUser;
use super::sweep_authority::RootOwner;
use super::sweep_authority::SweepEligibleFile;
#[cfg(target_os = "linux")]
use crate::constants::CAPTURE_DIRECTORY_BUFFER_BYTES;
#[cfg(test)]
use crate::constants::CAPTURE_DIRECTORY_INCOMPLETE;
use crate::constants::CAPTURE_ENTRY_NAME_BYTES;
use crate::constants::CAPTURE_INVALID_BASENAME;
use crate::constants::CAPTURE_INVENTORY_LIMIT;
use crate::constants::CAPTURE_LIVE_RUNS_DIR;
use crate::constants::CAPTURE_NOT_REGULAR;
use crate::constants::CAPTURE_PIDS_DIR;
use crate::constants::CAPTURE_REGISTRATION_BYTES;
use crate::constants::CAPTURE_REGISTRATION_TOO_LARGE;
use crate::constants::CAPTURE_ROOT;
use crate::constants::CAPTURE_SHARED_MODE;
use crate::constants::CAPTURE_STATE_DIR;
use crate::constants::PERMISSION_BITS;
use crate::constants::RUN_LOG_TAIL_BYTES;
use crate::progress::capture_diagnostic::CaptureFailure;
use crate::registration;
use crate::registration::ParseError;

/// Device, inode, owner and permissions describe the inspected directory.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct InspectedDirectoryMetadata {
    /// Inode numbers alone are not unique across filesystems.
    pub(super) device: Dev,
    /// Detect replacement on the same filesystem.
    pub(super) inode:  u64,
    /// Only the effective owner can receive a deletion capability.
    pub(super) owner:  u32,
    /// Shared-parent repair compares the inspected directory mode.
    mode:              Mode,
}

impl From<&Stat> for InspectedDirectoryMetadata {
    fn from(stat: &Stat) -> Self {
        Self {
            device: stat.st_dev,
            inode:  stat.st_ino,
            owner:  stat.st_uid,
            mode:   Mode::from_raw_mode(stat.st_mode),
        }
    }
}

impl InspectedDirectoryMetadata {
    /// Permission changes do not replace a directory or grant file ownership.
    pub(super) const fn same_directory(self, other: Self) -> bool {
        self.device == other.device && self.inode == other.inode && self.owner == other.owner
    }
}

/// fstat is performed after opening; metadata never chooses a different handle.
pub(super) struct InspectedDirectory {
    /// All later operations stay relative to this directory.
    pub(super) handle:   OwnedFd,
    /// The ownership and identity inspected on this exact descriptor.
    pub(super) identity: InspectedDirectoryMetadata,
}

impl InspectedDirectory {
    /// Trusted ancestor symlinks remain usable; the root itself cannot be one.
    pub(super) fn open_root(path: &Path) -> io::Result<Self> {
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
        let identity = InspectedDirectoryMetadata::from(&fstat(&handle)?);
        Ok(Self { handle, identity })
    }
}

/// The shared parent either admits all accounts or retains why it cannot.
/// `CAPTURE_SHARED_MODE` in the descriptor-typed form `fchmod` takes: the sticky bit plus
/// read, write, and search for everyone. The raw mode type is narrower on macOS than on
/// Linux, so the flags are named rather than converted from the numeric constant.
const SHARED_DIRECTORY_MODE: Mode = Mode::SVTX
    .union(Mode::RWXU)
    .union(Mode::RWXG)
    .union(Mode::RWXO);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SharedDirectoryState {
    /// No parent exists yet.
    Missing,
    /// Sticky world-writable permissions allow every account to create its directory.
    Shared { owner: RootOwner },
    /// Existing permissions need repair by the owner or administrator.
    NotShared { mode: u32, owner: RootOwner },
    /// The parent could not be opened as a real directory.
    Unavailable(CaptureFailure),
}

/// Path and permissions sampled by the worker for the settings pane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SharedCaptureDirectory {
    pub(crate) path:  PathBuf,
    pub(crate) state: SharedDirectoryState,
}

impl SharedCaptureDirectory {
    /// Inspection performs no mutation and resolves ancestor aliases on both platforms.
    pub(crate) fn inspect(parent: &Path) -> Self {
        let path = canonical_capture_path(parent);
        let state = match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_dir() => {
                let owner = RootOwner::Uid(metadata.uid());
                let mode = metadata.mode() & PERMISSION_BITS;
                if mode == CAPTURE_SHARED_MODE {
                    SharedDirectoryState::Shared { owner }
                } else {
                    SharedDirectoryState::NotShared { mode, owner }
                }
            },
            Ok(_) => {
                SharedDirectoryState::Unavailable(Error::from(ErrorKind::NotADirectory).into())
            },
            Err(error) if error.kind() == ErrorKind::NotFound => SharedDirectoryState::Missing,
            Err(error) => SharedDirectoryState::Unavailable(error.into()),
        };
        Self { path, state }
    }
}

impl Default for SharedCaptureDirectory {
    fn default() -> Self {
        Self {
            path:  PathBuf::from(CAPTURE_ROOT),
            state: SharedDirectoryState::Missing,
        }
    }
}

/// Create the parent; its owner or root repairs permissions through the open handle.
/// Other callers require an existing shared mode and never change its permissions.
pub(crate) fn prepare_shared_directory(parent: &Path) -> io::Result<()> {
    match fs::create_dir(parent) {
        Ok(()) => {},
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
        Err(error) => return Err(error),
    }
    let path = canonical_capture_path(parent);
    let directory = match InspectedDirectory::open_root(&path) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            // An owner can chmod a directory even when its current mode forbids
            // opening it. Only its own real directory permits this recovery;
            // the sticky system parent prevents another account replacing it.
            let metadata = fs::symlink_metadata(&path)?;
            if !metadata.is_dir()
                || sweep_authority::effective_user() != EffectiveUser::Known(metadata.uid())
            {
                return Err(error);
            }
            fs::set_permissions(&path, fs::Permissions::from_mode(CAPTURE_SHARED_MODE))?;
            InspectedDirectory::open_root(&path)?
        },
        Err(error) => return Err(error),
    };
    repair_shared_mode(&directory, sweep_authority::effective_user())
}

/// Ownership authorizes repair; an already shared mode requires no authority.
fn repair_shared_mode(directory: &InspectedDirectory, user: EffectiveUser) -> io::Result<()> {
    if directory.identity.mode == SHARED_DIRECTORY_MODE {
        return Ok(());
    }
    if !matches!(user, EffectiveUser::Known(uid) if uid == 0 || uid == directory.identity.owner) {
        return Err(ErrorKind::PermissionDenied.into());
    }
    fchmod(&directory.handle, SHARED_DIRECTORY_MODE)?;
    Ok(())
}

/// Resolve aliases in ancestors while retaining the final component for NOFOLLOW.
pub(crate) fn canonical_capture_path(path: &Path) -> PathBuf {
    let path: PathBuf = path.components().collect();
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .map_or_else(|_| path.clone(), |parent| parent.join(name)),
        _ => path,
    }
}

/// An empty complete inventory is evidence; a truncated or failed one is not.
#[derive(Debug)]
pub(crate) enum Enumeration {
    /// Every directory entry was visited successfully.
    Complete,
    /// A count or filename limit stopped the inventory before end of directory.
    Incomplete,
    /// Keep the actual access or enumeration failure for the caller to observe.
    Failed(Error),
}

impl Enumeration {
    /// Preserve the cause of an unenumerated portion of the capture tree.
    #[cfg(test)]
    fn require_complete(&self) -> io::Result<()> {
        match self {
            Self::Complete => Ok(()),
            Self::Incomplete => Err(io::Error::other(CAPTURE_DIRECTORY_INCOMPLETE)),
            Self::Failed(error) => Err(io::Error::new(error.kind(), error.to_string())),
        }
    }
}

/// Registration traversal either holds both ancestors or retains its failure.
pub(super) enum RegistrationAccess {
    /// Both handles were derived from the inspected root without symlink traversal.
    Open {
        /// Prevent state replacement from being hidden behind an unchanged root.
        state: InspectedDirectory,
        /// Reads and removals resolve registration basenames against this handle.
        pids:  InspectedDirectory,
    },
    /// Preserve which traversal step failed independently of its inventory.
    Unavailable(PathBuf),
}

/// Keep partial inventories observable while preserving their established entries.
pub(super) struct Inventory {
    /// Names are stored inline; allocation grows only as entries are sampled.
    pub(super) entries: Vec<InventoryEntry>,
    /// Failure details survive even when earlier entries were readable.
    pub(super) outcome: Enumeration,
}

impl Inventory {
    /// Failed traversal has no entries and is never reported as complete.
    pub(super) const fn failed(error: Error) -> Self {
        Self {
            entries: Vec::new(),
            outcome: Enumeration::Failed(error),
        }
    }

    /// Linux getdents fills fixed caller storage without allocating per entry.
    #[cfg(target_os = "linux")]
    pub(super) fn sample(directory: &InspectedDirectory) -> Self {
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
    pub(super) fn sample(directory: &InspectedDirectory) -> Self {
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
pub(super) struct InventoryEntry {
    /// Fixed storage also bounds inventories containing unrelated long filenames.
    pub(super) name: [u8; CAPTURE_ENTRY_NAME_BYTES],
    /// Only these bytes came from the directory enumeration.
    length:          usize,
}

impl InventoryEntry {
    /// Directory APIs supply basenames; callers cannot construct new entries.
    pub(super) fn name(&self) -> &Path { Path::new(OsStr::from_bytes(&self.name[..self.length])) }
}

/// A readable basename borrows the inspected directory that constrains its open.
#[derive(Clone, Copy)]
pub(crate) struct ScanEntry<'scan> {
    /// A replaced pathname cannot redirect the entry's open to another directory.
    pub(super) directory:         &'scan InspectedDirectory,
    /// A sampled or explicitly validated basename carries no pathname prefix.
    pub(super) name:              &'scan Path,
    /// Sweep callbacks read the file whose ownership was proved.
    pub(super) registration_read: RegistrationReadPurpose<'scan>,
}

/// Why a registration is being read: an ordinary scan resolves a basename, and
/// a sweep rereads the inspected file it is about to revalidate and remove.
#[derive(Clone, Copy)]
pub(super) enum RegistrationReadPurpose<'scan> {
    /// An ordinary read with no per-file removal proof.
    Observation,
    /// Record bytes and ownership evidence both come from the eligible file's descriptor.
    SweepRevalidation(&'scan SweepEligibleFile),
}

impl<'scan> ScanEntry<'scan> {
    /// Classify filenames without allocating a joined path for every entry.
    pub(crate) const fn name(&self) -> &'scan Path { self.name }

    /// NONBLOCK prevents FIFO opens from waiting; fstat rejects every non-regular
    /// type before reads. It does not impose a deadline on regular file I/O.
    pub(super) fn open_regular(&self) -> io::Result<File> {
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
    #[cfg(test)]
    pub(super) fn read_log(&self) -> io::Result<String> {
        let file = self.open_regular()?;
        let length = file.metadata()?.len();
        read_tail(file, length)
    }

    /// Read one byte past the record cap to reject oversize supported records.
    /// Newer headers retain their bounded bytes for the version diagnostic.
    pub(crate) fn read_registration(&self) -> io::Result<RegistrationObservation> {
        let file = match self.registration_read {
            RegistrationReadPurpose::Observation => self.open_regular()?,
            RegistrationReadPurpose::SweepRevalidation(file) => {
                File::from(file.handle.try_clone()?)
            },
        };
        read_registration_file(file)
    }
}

/// Contents and metadata come from one descriptor, even after a pathname replacement.
#[derive(Debug)]
pub(crate) struct RegistrationObservation {
    /// Record bytes bounded independently of metadata; newer payloads may be truncated.
    pub(crate) bytes:    Vec<u8>,
    /// Includes the modification time of the same file that supplied the bytes.
    pub(crate) metadata: Metadata,
}

/// Read one already opened registration without consulting its pathname again.
fn read_registration_file(mut file: File) -> io::Result<RegistrationObservation> {
    // Duplicated proof descriptors share an offset; every callback reads from zero.
    file.seek(SeekFrom::Start(0))?;
    let metadata = file.metadata()?;
    let mut bytes = Vec::new();
    file.take(CAPTURE_REGISTRATION_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if matches!(
        registration::check_version(&bytes),
        Err(ParseError::UnsupportedVersion { .. })
    ) {
        return Ok(RegistrationObservation { bytes, metadata });
    }
    if bytes.len() as u64 > CAPTURE_REGISTRATION_BYTES {
        return Err(io::Error::other(CAPTURE_REGISTRATION_TOO_LARGE));
    }
    Ok(RegistrationObservation { bytes, metadata })
}

/// Validate external record names before any descriptor-relative open.
pub(super) fn named_entry<'scan>(
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
            ErrorKind::InvalidInput,
            CAPTURE_INVALID_BASENAME,
        ));
    }
    Ok(ScanEntry {
        directory,
        name: basename,
        registration_read: RegistrationReadPurpose::Observation,
    })
}

/// Every traversed directory rejects symlinks and closes across exec.
fn directory_flags() -> OFlags {
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

/// Opening ancestors separately reports exactly which directory refused access.
pub(super) fn open_registration_paths(
    root: &InspectedDirectory,
    path: &Path,
) -> Result<(InspectedDirectory, InspectedDirectory), (PathBuf, Error)> {
    let state = root
        .child(CAPTURE_STATE_DIR)
        .map_err(|error| (path.join(CAPTURE_STATE_DIR), error))?;
    let pids = state
        .child(CAPTURE_PIDS_DIR)
        .map_err(|error| (path.join(CAPTURE_LIVE_RUNS_DIR), error))?;
    Ok((state, pids))
}

/// `Read::take` remains authoritative when a file grows after the length sample.
pub(super) fn read_tail(mut file: File, length: u64) -> io::Result<String> {
    file.seek(SeekFrom::Start(length.saturating_sub(RUN_LOG_TAIL_BYTES)))?;
    let mut bytes = Vec::new();
    file.take(RUN_LOG_TAIL_BYTES).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
pub(super) use tests::capture_root;
#[cfg(test)]
pub(super) use tests::create_directories;
#[cfg(test)]
pub(super) use tests::log_entry;
#[cfg(test)]
pub(super) use tests::make_fifo;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::ffi::CString;
    use std::fs;
    use std::fs::File;
    use std::fs::OpenOptions;
    use std::io;
    use std::io::ErrorKind;
    use std::io::Write;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::os::unix::net::UnixListener;
    use std::path::Path;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::Enumeration;
    use super::InspectedDirectory;
    use super::Inventory;
    use super::ScanEntry;
    use crate::constants::CAPTURE_INVENTORY_LIMIT;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;
    use crate::constants::CAPTURE_NOT_REGULAR;
    use crate::constants::CAPTURE_REGISTRATION_BYTES;
    use crate::constants::CAPTURE_REGISTRATION_TOO_LARGE;
    use crate::constants::CAPTURE_STATE_DIR;
    use crate::constants::REGISTRATION_FIELD_SEPARATOR;
    use crate::constants::REGISTRATION_MAGIC;
    use crate::constants::REGISTRATION_MAGIC_PREFIX;
    use crate::constants::REGISTRATION_V2_MAGIC;
    use crate::constants::RUN_LOG_TAIL_BYTES;
    use crate::constants::SUPPORTED_REGISTRATION_VERSION;
    use crate::registration::ParseError;
    use crate::registration::Registration;
    use crate::root_scan::EffectiveUser;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootScan;

    #[test]
    fn shared_parent_owner_repairs_modes_that_prevent_opening_the_directory() {
        let fixture = tempdir().expect("parent fixture");
        let parent = fixture.path().join("capture");
        super::prepare_shared_directory(&parent).expect("create shared parent");
        for mode in [0o000, 0o111, 0o750, 0o1777] {
            fs::set_permissions(&parent, fs::Permissions::from_mode(mode))
                .expect("set parent mode");
            let status = super::SharedCaptureDirectory::inspect(&parent);
            if mode != super::CAPTURE_SHARED_MODE {
                assert!(
                    matches!(status.state, super::SharedDirectoryState::NotShared { mode: observed, .. } if observed == mode)
                );
            }
            super::prepare_shared_directory(&parent).expect("owner repairs parent");
            assert!(matches!(
                super::SharedCaptureDirectory::inspect(&parent).state,
                super::SharedDirectoryState::Shared { .. }
            ));
        }
    }

    #[test]
    fn foreign_parent_requires_shared_mode_unless_the_caller_is_root() {
        let parent = tempdir().expect("shared parent fixture");
        let owner = fs::metadata(parent.path()).expect("parent owner").uid();
        let foreign = owner.checked_add(1).expect("foreign fixture uid");
        fs::set_permissions(parent.path(), fs::Permissions::from_mode(0o755))
            .expect("unshared mode");
        let directory = super::InspectedDirectory::open_root(parent.path()).expect("open parent");
        for user in [EffectiveUser::Known(foreign), EffectiveUser::Unavailable] {
            assert_eq!(
                super::repair_shared_mode(&directory, user)
                    .expect_err("foreign mode must be rejected")
                    .kind(),
                ErrorKind::PermissionDenied
            );
        }
        assert_eq!(
            fs::metadata(parent.path())
                .expect("unchanged parent")
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
        super::repair_shared_mode(&directory, EffectiveUser::Known(0))
            .expect("root may repair foreign parent");
        let shared =
            super::InspectedDirectory::open_root(parent.path()).expect("reopen shared parent");
        super::repair_shared_mode(&shared, EffectiveUser::Known(foreign))
            .expect("foreign shared parent is accepted");
        assert!(matches!(
            super::SharedCaptureDirectory::inspect(parent.path()).state,
            super::SharedDirectoryState::Shared { .. }
        ));
    }

    #[test]
    fn shared_parent_symlink_is_never_repaired_or_used_as_a_directory() {
        let fixture = tempdir().expect("parent fixture");
        let target = fixture.path().join("target");
        fs::create_dir(&target).expect("target directory");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).expect("target mode");
        let parent = fixture.path().join("capture");
        symlink(&target, &parent).expect("parent symlink");
        assert!(super::prepare_shared_directory(&parent).is_err());
        assert!(matches!(
            super::SharedCaptureDirectory::inspect(&parent).state,
            super::SharedDirectoryState::Unavailable(_)
        ));
        assert_eq!(
            fs::metadata(&target)
                .expect("target metadata")
                .permissions()
                .mode()
                & 0o7777,
            0o700
        );
    }

    /// Use one account and explicit directory modes independent of its umask.
    pub fn capture_root() -> TempDir {
        let root = tempdir().expect("temporary capture root");
        create_directories(root.path());
        root
    }

    /// Replacement fixtures use the same independently owned directory layout.
    pub fn create_directories(root: &Path) {
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
    pub fn log_entry<'scan>(scan: &'scan RootScan, name: &str) -> ScanEntry<'scan> {
        scan.log_entries()
            .find(|entry| entry.name() == Path::new(name))
            .expect("sampled log entry")
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

    /// rustix omits `mkfifoat` on Apple targets, which have no such syscall, and
    /// offers no other FIFO constructor there.
    #[allow(
        unsafe_code,
        reason = "creating a FIFO portably requires libc, which rustix does not wrap on Apple"
    )]
    pub fn make_fifo(path: &Path) {
        let raw = CString::new(path.as_os_str().as_bytes()).expect("path without interior NUL");
        // SAFETY: raw is a live NUL-terminated C string that outlives the call, and
        // mkfifo retains no pointer past it. The mode argument is a permission bit
        // set with no pointer content. The result is checked before the path is used.
        let created = unsafe { libc::mkfifo(raw.as_ptr(), 0o600) };
        assert_eq!(created, 0, "create FIFO: {}", io::Error::last_os_error());
    }

    /// NONBLOCK lets FIFO open return without a writer; fstat then rejects it.
    #[test]
    fn fifo_directory_and_socket_entries_are_not_regular_files() {
        let root = capture_root();
        make_fifo(&root.path().join("fifo"));
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
            super::read_tail(file, sampled_length)
                .expect("bounded tail")
                .len(),
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

    #[test]
    fn registration_reads_preserve_newer_version_diagnostics_at_every_size() {
        let root = capture_root();
        let path = root.path().join("record");
        let cap = usize::try_from(CAPTURE_REGISTRATION_BYTES).expect("record cap fits usize");
        let prefix = std::str::from_utf8(REGISTRATION_MAGIC_PREFIX).expect("ASCII magic prefix");
        for encountered in [SUPPORTED_REGISTRATION_VERSION + 1, u64::MAX] {
            for length in [cap, cap + 1, cap * 2] {
                let mut bytes = format!("{prefix}{encountered}").into_bytes();
                bytes.push(REGISTRATION_FIELD_SEPARATOR);
                bytes.resize(length, 0xff);
                fs::write(&path, &bytes).expect("newer registration");
                let scan =
                    RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
                let observation = log_entry(&scan, "record")
                    .read_registration()
                    .expect("newer header remains available");
                assert_eq!(observation.bytes.len(), length.min(cap + 1));
                assert_eq!(
                    Registration::parse(&observation.bytes),
                    Err(ParseError::UnsupportedVersion { encountered })
                );
                assert_eq!(fs::read(&path).expect("registration preserved"), bytes);
            }
        }
    }

    #[test]
    fn registration_reads_reject_oversized_supported_and_malformed_headers() {
        let root = capture_root();
        let path = root.path().join("record");
        let cap = usize::try_from(CAPTURE_REGISTRATION_BYTES).expect("record cap fits usize");
        for magic in [REGISTRATION_V2_MAGIC, REGISTRATION_MAGIC, b"cargo-tile-v4x"] {
            let mut bytes = magic.to_vec();
            bytes.push(REGISTRATION_FIELD_SEPARATOR);
            bytes.resize(cap + 1, b'x');
            fs::write(&path, bytes).expect("oversized registration");
            let scan = RootScan::open(root.path(), &mut RootHistory::default()).expect("scan root");
            assert_eq!(
                log_entry(&scan, "record")
                    .read_registration()
                    .expect_err("oversized registration rejected")
                    .to_string(),
                CAPTURE_REGISTRATION_TOO_LARGE
            );
        }
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
}
