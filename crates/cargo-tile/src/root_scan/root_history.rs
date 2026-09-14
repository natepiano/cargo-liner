//! Directory incarnations and scan-to-scan identity continuity.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use rustix::fd::OwnedFd;
use rustix::fs::fstat;
use uuid::Uuid;

use super::inspected_directory::InspectedDirectory;
use super::inspected_directory::InspectedDirectoryMetadata;
use super::inspected_directory::RegistrationAccess;

/// Whether this scan may build on the previous scan's directory identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum RootContinuity {
    /// First successful observation, or the same inspected directory tree.
    Established,
    /// Replaced directories or recovery from failed access prohibit this sweep.
    Changed,
}

/// Cleanup history and directory incarnation have independent evidence and lifetimes.
#[derive(Default)]
pub(crate) struct RootHistory {
    /// An access failure is retained so recovery cannot authorize immediate sweep.
    pub(super) roots: HashMap<PathBuf, PreviousRoot>,
    /// Pins prevent inode reuse from looking like continuity; they grant no cleanup.
    incarnations:     HashMap<PathBuf, RootIncarnationAnchor>,
}

/// Keep the last directory allocated even while its pathname is unavailable.
struct RootIncarnationAnchor {
    /// This handle is used only for object comparison, never for reading or removal.
    handle:      OwnedFd,
    /// Replacement produces a new value even if an earlier inode later returns.
    incarnation: RootIncarnation,
}

impl RootHistory {
    /// Permission and ownership changes cannot change the directory object's identity.
    pub(super) fn incarnation(
        &mut self,
        path: &Path,
        root: &InspectedDirectory,
    ) -> io::Result<RootIncarnation> {
        if let Some(previous) = self.incarnations.get(path) {
            let metadata = fstat(&previous.handle)?;
            if metadata.st_dev == root.identity.device && metadata.st_ino == root.identity.inode {
                return Ok(previous.incarnation);
            }
        }
        let anchor = RootIncarnationAnchor {
            handle:      root.handle.try_clone()?,
            incarnation: RootIncarnation(uuid::Uuid::now_v7()),
        };
        let incarnation = anchor.incarnation;
        self.incarnations.insert(path.to_owned(), anchor);
        Ok(incarnation)
    }
}

/// A successful observation differs from a previously inaccessible pathname.
pub(super) enum PreviousRoot {
    /// Compare all three directory identities on the next observation.
    Open(TreeIdentity),
    /// No previous descriptor can justify cleanup after failed access.
    Unavailable,
}

/// Detect root and registration-ancestor replacement across successive scans.
#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) struct TreeIdentity {
    /// The account directory itself may change without its pathname changing.
    root:          InspectedDirectoryMetadata,
    /// Missing registration ancestors also matter when they become available.
    registrations: RegistrationIdentity,
}

impl TreeIdentity {
    /// Directory permissions do not change the captured tree's object identities.
    pub(super) const fn same_tree(self, other: Self) -> bool {
        self.root.same_directory(other.root)
            && match (self.registrations, other.registrations) {
                (
                    RegistrationIdentity::Open { state, pids },
                    RegistrationIdentity::Open {
                        state: other_state,
                        pids: other_pids,
                    },
                ) => state.same_directory(other_state) && pids.same_directory(other_pids),
                (RegistrationIdentity::Unavailable, RegistrationIdentity::Unavailable) => true,
                _ => false,
            }
    }

    /// Preserve unavailable traversal as a distinct identity state.
    pub(super) const fn new(root: InspectedDirectoryMetadata, access: &RegistrationAccess) -> Self {
        let registrations = match access {
            RegistrationAccess::Open { state, pids } => RegistrationIdentity::Open {
                state: state.identity,
                pids:  pids.identity,
            },
            RegistrationAccess::Unavailable(_) => RegistrationIdentity::Unavailable,
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
        state: InspectedDirectoryMetadata,
        /// The directory whose enumeration determines the live set.
        pids:  InspectedDirectoryMetadata,
    },
    /// Failed traversal is never equivalent to an empty registration directory.
    Unavailable,
}

/// Identity of a directory object, without permissions or removal authority.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RootIncarnation(Uuid);

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::RootContinuity;
    use crate::root_scan::RootHistory;
    use crate::root_scan::RootScan;
    use crate::root_scan::inspected_directory;
    use crate::root_scan::sweep_authority;

    /// A replaced or newly accessible pathname must first establish a new history.
    #[test]
    fn identity_changes_and_access_recovery_disable_one_scan_of_cleanup() {
        let parent = tempdir().expect("parent");
        let root = parent.path().join("capture");
        let mut history = RootHistory::default();
        assert!(RootScan::open(&root, &mut history).is_err());
        inspected_directory::create_directories(&root);
        sweep_authority::write_pair(&root);
        fs::write(root.join("log"), "first").expect("first log");
        let recovered = RootScan::open(&root, &mut history).expect("recovered root");
        assert_eq!(recovered.continuity, RootContinuity::Changed);
        assert_eq!(sweep_authority::sweep_everything(&recovered), 0);
        let established = RootScan::open(&root, &mut history).expect("stable root");
        assert_eq!(established.continuity, RootContinuity::Established);
        fs::rename(&root, parent.path().join("old")).expect("replace root");
        inspected_directory::create_directories(&root);
        sweep_authority::write_pair(&root);
        fs::write(root.join("log"), "new").expect("new log");
        let replaced = RootScan::open(&root, &mut history).expect("replacement root");
        assert_eq!(replaced.continuity, RootContinuity::Changed);
        assert_eq!(sweep_authority::sweep_everything(&replaced), 0);
        assert!(root.join("log").exists());
    }
}
