//! Shared account discovery and per-root scan observations.

use std::cell::RefCell;
use std::io::ErrorKind;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;

use sysinfo::Users;

use super::capture_diagnostic::CaptureDiagnostic;
use super::capture_diagnostic::PathFailure;
use crate::root_scan;
use crate::root_scan::EffectiveUser;
use crate::root_scan::RootOwner;
use crate::root_scan::SharedCaptureDirectory;
use crate::settings::CaptureAssociation;

/// Display resolution never changes the numeric account identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum AccountName {
    /// The scanner resolved a nonempty account name for the root owner.
    Resolved(String),
    /// A missing passwd entry leaves the numeric owner visible.
    Unavailable,
}

impl AccountName {
    /// Account lookup is a scan observation and never participates in identity comparison.
    pub(crate) fn resolve(owner: RootOwner, users: &Users) -> Self {
        let RootOwner::Uid(uid) = owner else {
            return Self::Unavailable;
        };
        users
            .iter()
            .find(|user| **user.id() == uid)
            .filter(|user| !user.name().is_empty())
            .map_or(Self::Unavailable, |user| {
                Self::Resolved(user.name().to_owned())
            })
    }
}

/// Which account is responsible for removing ended captures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CaptureCleanup {
    /// This reader can prove and remove its own ended registrations.
    Here,
    /// Another account's next shim invocation removes its ended registrations.
    AccountNextRun,
}

/// One account's directory below the shared parent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureRoot {
    /// Absolute pathname; the final component is checked without following links.
    pub(crate) path:    PathBuf,
    /// The numeric directory name must match its owner before it supplies captures.
    pub(crate) uid:     u32,
    /// Cleanup is permitted only in the reader's own uid directory.
    pub(crate) cleanup: CaptureCleanup,
}

impl CaptureRoot {
    fn account(path: PathBuf, uid: u32) -> Self {
        Self {
            path,
            uid,
            cleanup: if root_scan::effective_user() == EffectiveUser::Known(uid) {
                CaptureCleanup::Here
            } else {
                CaptureCleanup::AccountNextRun
            },
        }
    }

    #[cfg(test)]
    pub(super) fn for_test(path: &Path) -> Self {
        let uid = std::fs::metadata(path).map_or_else(
            |_| match root_scan::effective_user() {
                EffectiveUser::Known(uid) => uid,
                EffectiveUser::Unavailable => u32::MAX,
            },
            |metadata| metadata.uid(),
        );
        Self::account(root_scan::canonical_capture_path(path), uid)
    }
}

/// Shared machine discovery and isolated descriptor tests have distinct path semantics.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum CaptureParent {
    /// Numeric account children are discovered beneath this canonical parent.
    Shared(PathBuf),
    /// Unit fixtures already name the account directories whose races they exercise.
    #[cfg(test)]
    IsolatedAccounts,
}

/// One shared parent, with stable account indices across subsequent scans.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CaptureRoots {
    pub(super) parent: CaptureParent,
    accounts:          RefCell<Vec<CaptureRoot>>,
}

impl CaptureRoots {
    /// Tests supply their own parent; production always supplies `CAPTURE_ROOT`.
    pub(crate) fn from_parent(parent: &Path) -> Self {
        let _ = root_scan::prepare_shared_directory(parent);
        Self {
            parent:   CaptureParent::Shared(root_scan::canonical_capture_path(parent)),
            accounts: RefCell::default(),
        }
    }

    /// Refresh discovery every scan so an account's first run appears immediately.
    pub(super) fn discover(&self, users: &Users) -> Vec<AccountCaptureDirectory> {
        let accounts = match &self.parent {
            CaptureParent::Shared(parent) => self.discover_accounts(parent),
            #[cfg(test)]
            CaptureParent::IsolatedAccounts => self.accounts.borrow().clone(),
        };
        accounts
            .into_iter()
            .map(|root| AccountCaptureDirectory::inspect(root, users))
            .collect()
    }

    fn discover_accounts(&self, parent: &Path) -> Vec<CaptureRoot> {
        let mut accounts = self.accounts.borrow_mut();
        // Final parent symlinks must not redirect the account inventory.
        if !matches!(
            SharedCaptureDirectory::inspect(parent).state,
            crate::root_scan::SharedDirectoryState::Shared { .. }
                | crate::root_scan::SharedDirectoryState::NotShared { .. }
        ) {
            return Vec::new();
        }
        if let Ok(entries) = std::fs::read_dir(parent) {
            let mut discovered = entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let name = entry.file_name();
                    let name = name.to_str()?;
                    let uid: u32 = name.parse().ok()?;
                    if name != uid.to_string() || !entry.file_type().ok()?.is_dir() {
                        return None;
                    }
                    Some(CaptureRoot::account(parent.join(name), uid))
                })
                .collect::<Vec<_>>();
            discovered.sort_by_key(|root| (root.cleanup != CaptureCleanup::Here, root.uid));
            for root in discovered {
                if !accounts.iter().any(|known| known.path == root.path) {
                    accounts.push(root);
                }
            }
        }
        accounts.clone()
    }

    /// Isolated account scans exercise descriptor and process-identity races directly.
    #[cfg(test)]
    pub(crate) fn for_test(paths: &[&Path]) -> Self {
        Self {
            parent:   CaptureParent::IsolatedAccounts,
            accounts: RefCell::new(
                paths
                    .iter()
                    .map(|path| CaptureRoot::for_test(path))
                    .collect(),
            ),
        }
    }
}

/// One effective root's access, identity and capture observations for this scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AccountCaptureDirectory {
    /// The directory name claims an account; `state` records owner verification.
    pub(crate) root:         CaptureRoot,
    /// Nofollow metadata identifies rejected owners; accepted roots use their descriptor.
    pub(crate) owner:        RootOwner,
    /// Resolved on the scan worker; rendering performs no account lookup.
    pub(crate) account:      AccountName,
    /// Each scan reopens the directory and reports current access.
    pub(crate) state:        RootReadStatus,
    /// Published, verified registrations whose logs were readable in this scan.
    pub(crate) confirmed:    usize,
    /// Failures and retained artifacts remain visible without process-table rows.
    pub(crate) diagnostics:  Vec<CaptureDiagnostic>,
    /// Each association names its process and any competing proof left unused.
    pub(crate) associations: Vec<CaptureAssociation>,
}

/// Access to an account directory is observed again on every scan.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum RootReadStatus {
    /// The root handle opened; diagnostics describe any incomplete contents.
    Readable,
    /// Reopening this absolute path failed in the current scan.
    Unavailable(PathFailure),
    /// A real directory owned by a different uid supplies no captures.
    ForeignOwned { owner: AccountName },
}

impl AccountCaptureDirectory {
    /// Inspect the final component without following links before reading captures.
    pub(crate) fn inspect(root: CaptureRoot, users: &Users) -> Self {
        let mut status = Self {
            account: AccountName::resolve(RootOwner::Uid(root.uid), users),
            root,
            owner: RootOwner::Unavailable,
            state: RootReadStatus::Readable,
            confirmed: 0,
            diagnostics: Vec::new(),
            associations: Vec::new(),
        };
        let metadata = std::fs::symlink_metadata(&status.root.path).and_then(|metadata| {
            if metadata.is_dir() {
                Ok(metadata)
            } else {
                Err(ErrorKind::NotADirectory.into())
            }
        });
        match metadata {
            Ok(metadata) => {
                status.owner = RootOwner::Uid(metadata.uid());
                if metadata.uid() != status.root.uid {
                    status.state = RootReadStatus::ForeignOwned {
                        owner: AccountName::resolve(status.owner, users),
                    };
                }
            },
            Err(error) => {
                let failure = PathFailure {
                    path:    status.root.path.clone(),
                    failure: error.into(),
                };
                status.state = RootReadStatus::Unavailable(failure);
            },
        }
        status
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;

    use super::*;
    use crate::constants::CAPTURE_LIVE_RUNS_DIR;

    #[test]
    fn shared_parent_ignores_nonnumeric_children_and_resolves_ancestor_aliases() {
        let directory = tempdir().unwrap();
        let parent = directory.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let parent = parent.canonicalize().unwrap();
        let alias = directory.path().join("alias");
        symlink(&parent, &alias).unwrap();
        let actual = parent.join("captures");
        let roots = CaptureRoots::from_parent(&alias.join("captures"));
        assert_eq!(roots.parent, CaptureParent::Shared(actual.clone()));
        fs::create_dir_all(actual.join("123").join(CAPTURE_LIVE_RUNS_DIR)).unwrap();
        fs::create_dir(actual.join("not-an-account")).unwrap();
        fs::create_dir(actual.join("0123")).unwrap();
        symlink(actual.join("123"), actual.join("456")).unwrap();
        let accounts = roots.discover(&sysinfo::Users::new());
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].root.uid, 123);
    }
}
