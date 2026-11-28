//! Watch the configuration vnodes with kqueue.

use std::borrow::Cow;
use std::fs;
use std::fs::File;
use std::fs::Metadata;
use std::fs::OpenOptions;
use std::io;
use std::io::ErrorKind;
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;

use kqueue::EventData;
use kqueue::EventFilter;
use kqueue::FilterFlag;
use kqueue::Watcher;

struct ConfigRegistration {
    watcher: Watcher,
    vnodes:  ConfigVnodes,
}

struct ConfigVnodes(Vec<OpenVnode>);

struct OpenVnode {
    file:     File,
    identity: VnodeIdentity,
}

#[derive(PartialEq, Eq)]
struct VnodeIdentity {
    device: u64,
    inode:  u64,
}

impl ConfigRegistration {
    fn new(vnodes: ConfigVnodes, stop: &UnixStream) -> io::Result<Self> {
        let mut watcher = Watcher::new()?;
        watcher.add_fd(
            stop.as_raw_fd(),
            EventFilter::EVFILT_READ,
            FilterFlag::empty(),
        )?;
        for vnode in &vnodes.0 {
            watcher.add_file(
                &vnode.file,
                EventFilter::EVFILT_VNODE,
                FilterFlag::NOTE_WRITE
                    | FilterFlag::NOTE_EXTEND
                    | FilterFlag::NOTE_ATTRIB
                    | FilterFlag::NOTE_DELETE
                    | FilterFlag::NOTE_RENAME
                    | FilterFlag::NOTE_REVOKE,
            )?;
        }
        watcher.watch()?;
        Ok(Self { watcher, vnodes })
    }
}

impl ConfigVnodes {
    const MAX_SYMLINK_HOPS: usize = 32;

    fn open(path: &Path) -> io::Result<Self> {
        let mut vnodes = Self(Vec::new());
        let mut target = Cow::Borrowed(path);
        // Keep each symlink target directory registered when the config file is missing.
        for _ in 0..=Self::MAX_SYMLINK_HOPS {
            vnodes.insert(open_watch_directory(&target)?);
            let Ok(link) = fs::read_link(&target) else {
                break;
            };
            target = Cow::Owned(target.with_file_name(link));
        }

        match open_vnode(path) {
            Ok(file) => {
                let metadata = file.metadata()?;
                vnodes.insert(OpenVnode::new(file, &metadata));
            },
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::NotFound | ErrorKind::NotADirectory | ErrorKind::PermissionDenied
                ) => {},
            Err(error) => return Err(error),
        }
        Ok(vnodes)
    }

    fn insert(&mut self, entry: OpenVnode) {
        if !self
            .0
            .iter()
            .any(|current| current.identity == entry.identity)
        {
            self.0.push(entry);
        }
    }

    fn identifies_same_paths(&self, other: &Self) -> bool {
        self.0
            .iter()
            .map(|vnode| &vnode.identity)
            .eq(other.0.iter().map(|vnode| &vnode.identity))
    }
}

impl OpenVnode {
    fn new(file: File, metadata: &Metadata) -> Self {
        Self {
            file,
            identity: VnodeIdentity {
                device: metadata.dev(),
                inode:  metadata.ino(),
            },
        }
    }
}

pub(super) fn watch_changes(
    path: &Path,
    stop: &UnixStream,
    mut publish: impl FnMut(),
) -> io::Result<()> {
    let mut registration: Option<ConfigRegistration> = None;
    loop {
        let vnodes = ConfigVnodes::open(path)?;
        let Some(current) = registration
            .as_ref()
            .filter(|current| current.vnodes.identifies_same_paths(&vnodes))
        else {
            let next = ConfigRegistration::new(vnodes, stop)?;
            if next
                .watcher
                .poll(None)
                .is_some_and(|event| matches!(event.data, EventData::ReadReady(_)))
            {
                return Ok(());
            }
            registration = Some(next);
            // Check the path identities again after kqueue starts watching the open files.
            continue;
        };
        drop(vnodes);
        publish();
        for event in &current.watcher {
            match event.data {
                EventData::ReadReady(_) => return Ok(()),
                EventData::Vnode(_) => break,
                EventData::Error(error) if error.kind() != ErrorKind::Interrupted => {
                    return Err(error);
                },
                _ => {},
            }
        }
    }
}

fn open_watch_directory(path: &Path) -> io::Result<OpenVnode> {
    for ancestor in path.ancestors().skip(1) {
        match open_vnode(ancestor) {
            Ok(file) => {
                let metadata = file.metadata()?;
                if metadata.is_dir() {
                    return Ok(OpenVnode::new(file, &metadata));
                }
            },
            Err(error)
                if matches!(error.kind(), ErrorKind::NotFound | ErrorKind::NotADirectory) => {},
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        ErrorKind::NotFound,
        "no available configuration directory",
    ))
}

fn open_vnode(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_EVTONLY)
        .open(path)
}
