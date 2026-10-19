//! Repository facts owned by one observation pass, shared by its workers.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::process::Output;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;

use super::command;
use super::constants::GIT_HEAD_REVISION;
use super::constants::GIT_NUL_TERMINATED_ARG;
use super::constants::GIT_PORCELAIN_ARG;
use super::constants::GIT_SYMBOLIC_REF_COMMAND;
use super::constants::GIT_WORKTREE_COMMAND;
use super::constants::GIT_WORKTREE_LIST_ARG;
use super::error::GitError;
use super::object::CommitAvailability;
use super::object::CommitObjectResolution;
use super::object::OperationObjectReader;
use super::refs::HeadAttachment;
use crate::ids::GitObjectId;
use crate::ledger::LedgerError;
use crate::ledger::WorktreeContext;

/// A fact can be initialized once, including an unsuccessful read. Workers see
/// the same failure instead of retrying Git independently.
struct RepositoryFact<T>(OnceLock<Result<T, Arc<GitError>>>);

impl<T> RepositoryFact<T> {
    const fn unread() -> Self { Self(OnceLock::new()) }

    fn read(&self, observe: impl FnOnce() -> Result<T, GitError>) -> Result<&T, GitError> {
        self.0
            .get_or_init(|| observe().map_err(Arc::new))
            .as_ref()
            .map_err(|failure| GitError::SnapshotReadFailed(Arc::clone(failure)))
    }
}

/// One operation's repository paths and first observations. A new pass after a
/// mutation owns a new snapshot; sharing this value never starts a fact twice.
pub(crate) struct RepositorySnapshot {
    context:               WorktreeContext,
    worktrees:             RepositoryFact<Vec<u8>>,
    status:                RepositoryFact<Output>,
    other_checkout_status: Mutex<HashMap<PathBuf, Arc<RepositoryFact<Output>>>>,
    head:                  RepositoryFact<GitObjectId>,
    attachment:            RepositoryFact<HeadAttachment>,
    objects:               OperationObjectReader,
}

impl RepositorySnapshot {
    /// Locate a checkout using its normal or linked administrative layout.
    pub(crate) fn discover(invocation_directory: &Path) -> Result<Self, LedgerError> {
        WorktreeContext::discover(invocation_directory).map(Self::new)
    }

    /// Start a fresh observation from paths the caller already discovered.
    pub(crate) fn new(context: WorktreeContext) -> Self {
        Self {
            objects: OperationObjectReader::new(context.repository_root()),
            context,
            worktrees: RepositoryFact::unread(),
            status: RepositoryFact::unread(),
            other_checkout_status: Mutex::new(HashMap::new()),
            head: RepositoryFact::unread(),
            attachment: RepositoryFact::unread(),
        }
    }

    pub(crate) const fn context(&self) -> &WorktreeContext { &self.context }

    pub(crate) fn repository_root(&self) -> &Path { self.context.repository_root() }

    pub(crate) fn common_directory(&self) -> &Path { self.context.common_git_directory() }

    /// Read the registry in Git's own format, preserving all supported layouts.
    pub(crate) fn worktree_list_porcelain(&self) -> Result<&[u8], GitError> {
        self.worktrees
            .read(|| {
                self.successful_output(
                    GIT_WORKTREE_COMMAND,
                    &[
                        GIT_WORKTREE_COMMAND,
                        GIT_WORKTREE_LIST_ARG,
                        GIT_PORCELAIN_ARG,
                        GIT_NUL_TERMINATED_ARG,
                    ],
                )
                .map(|output| output.stdout)
            })
            .map(Vec::as_slice)
    }

    /// Keep one status result for staged, unstaged and untracked paths.
    pub(crate) fn working_tree_status(&self) -> Result<&Output, GitError> {
        self.status
            .read(|| Self::read_checkout_status(self.repository_root()))
    }

    /// Each holder checkout has one status observation in this operation, even
    /// when several targets or workers ask about it.
    pub(crate) fn working_tree_status_at(&self, checkout: &Path) -> Result<Output, GitError> {
        let checkout = std::fs::canonicalize(checkout)?;
        if checkout == self.repository_root() {
            return self.working_tree_status().cloned();
        }
        let fact = self
            .other_checkout_status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(checkout.clone())
            .or_insert_with(|| Arc::new(RepositoryFact::unread()))
            .clone();
        fact.read(|| Self::read_checkout_status(&checkout)).cloned()
    }

    fn read_checkout_status(checkout: &Path) -> Result<Output, GitError> {
        let output = command::command_output(command::git_command(checkout)?.args([
            "status",
            "--porcelain",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ]))?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(GitError::CommandFailed {
                command: "status",
                stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }

    pub(crate) fn head_object_id(&self) -> Result<&GitObjectId, GitError> {
        self.head.read(|| self.objects.commit(GIT_HEAD_REVISION))
    }

    pub(crate) fn head_attachment(&self) -> Result<&HeadAttachment, GitError> {
        self.attachment.read(|| self.read_head_attachment())
    }

    /// Resolve HEAD attachment once with Git's symbolic-ref and backend semantics.
    fn read_head_attachment(&self) -> Result<HeadAttachment, GitError> {
        let output = command::git_output(
            self.repository_root(),
            [GIT_SYMBOLIC_REF_COMMAND, "--quiet", GIT_HEAD_REVISION],
        )?;
        if output.status.success() {
            let reference = String::from_utf8(output.stdout).map_err(GitError::InvalidOutput)?;
            return reference
                .trim()
                .parse()
                .map(|full_ref| HeadAttachment::Branch { full_ref })
                .map_err(|_| GitError::InvalidReferenceName { reference });
        }
        if output.status.code() == Some(1) {
            return Ok(HeadAttachment::Detached);
        }
        Err(GitError::CommandFailed {
            command: "symbolic-ref",
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }

    pub(super) fn resolve_commit_objects(
        &self,
        expressions: &[String],
    ) -> Result<Vec<CommitObjectResolution>, GitError> {
        self.objects.resolve(expressions)
    }

    pub(super) fn commit_availability(
        &self,
        object_ids: &[GitObjectId],
    ) -> Result<Vec<CommitAvailability>, GitError> {
        self.objects.commit_availability(object_ids)
    }

    /// Retain this operation's object child through a prepared journal action.
    pub(crate) fn object_reader(&self) -> OperationObjectReader { self.objects.clone() }

    fn successful_output(
        &self,
        command: &'static str,
        arguments: &[&str],
    ) -> Result<Output, GitError> {
        let output =
            command::command_output(command::git_command(self.repository_root())?.args(arguments))?;
        if output.status.success() {
            Ok(output)
        } else {
            Err(GitError::CommandFailed {
                command,
                stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RepositorySnapshot;
    use crate::git::HeadAttachment;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PatchEquivalenceFixture;

    #[test]
    fn repository_snapshot_shares_holder_status_between_workers_and_path_aliases() -> FixtureResult
    {
        let fixture = PatchEquivalenceFixture::new()?;
        let directories = tempfile::tempdir()?;
        let linked = directories.path().join("linked checkout");
        fixture.git(&[
            "worktree",
            "add",
            "-b",
            "holder-status",
            linked.to_str().ok_or("linked path is not UTF-8")?,
        ])?;
        let snapshot = RepositorySnapshot::discover(fixture.root())?;
        let original = snapshot.working_tree_status_at(&linked)?.stdout;
        std::fs::write(linked.join("pending.txt"), "holder work")?;
        let alias = linked.join("..").join("linked checkout");
        std::thread::scope(|scope| -> FixtureResult {
            let readers = (0..4)
                .map(|_| scope.spawn(|| snapshot.working_tree_status_at(&alias)))
                .collect::<Vec<_>>();
            for reader in readers {
                let output = reader.join().map_err(|_| "status worker panicked")??;
                assert_eq!(output.stdout, original);
            }
            Ok(())
        })?;
        let fresh = RepositorySnapshot::new(snapshot.context().clone());
        assert_ne!(fresh.working_tree_status_at(&linked)?.stdout, original);
        assert_eq!(
            snapshot.working_tree_status_at(fixture.root())?.stdout,
            snapshot.working_tree_status()?.stdout
        );
        Ok(())
    }

    #[test]
    fn repository_snapshot_keeps_facts_until_a_fresh_pass_observes_changes() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let snapshot = RepositorySnapshot::discover(fixture.root())?;
        let head = snapshot.head_object_id()?.clone();
        let status = snapshot.working_tree_status()?.stdout.clone();
        let worktrees = snapshot.worktree_list_porcelain()?.to_vec();
        assert!(matches!(
            snapshot.head_attachment()?,
            HeadAttachment::Branch { .. }
        ));

        fixture.git(&["commit", "--allow-empty", "-m", "new observation"])?;
        fixture.git(&["checkout", "--detach"])?;
        std::fs::write(fixture.root().join("new-untracked.txt"), "pending work")?;

        assert_eq!(snapshot.head_object_id()?, &head);
        assert_eq!(snapshot.working_tree_status()?.stdout, status);
        assert_eq!(snapshot.worktree_list_porcelain()?, worktrees);
        assert!(matches!(
            snapshot.head_attachment()?,
            HeadAttachment::Branch { .. }
        ));

        let fresh = RepositorySnapshot::new(snapshot.context().clone());
        assert_ne!(fresh.head_object_id()?, &head);
        assert_ne!(fresh.working_tree_status()?.stdout, status);
        assert_ne!(fresh.worktree_list_porcelain()?, worktrees);
        assert!(matches!(fresh.head_attachment()?, HeadAttachment::Detached));
        assert_eq!(fresh.common_directory(), snapshot.common_directory());
        Ok(())
    }

    #[test]
    fn repository_snapshot_locates_linked_and_separate_git_directories_from_nested_paths()
    -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let directories = tempfile::tempdir()?;
        let linked = directories.path().join("linked checkout");
        fixture.git(&[
            "worktree",
            "add",
            "-b",
            "snapshot-linked",
            linked.to_str().ok_or("linked checkout path is not UTF-8")?,
        ])?;
        let nested = linked.join("nested/directory");
        std::fs::create_dir_all(&nested)?;
        let snapshot = RepositorySnapshot::discover(&nested)?;
        assert_eq!(snapshot.repository_root(), std::fs::canonicalize(&linked)?);
        assert_eq!(
            snapshot.common_directory(),
            std::fs::canonicalize(fixture.root().join(".git"))?
        );
        assert_eq!(snapshot.head_object_id()?, &fixture.phase_start_head);

        let separate = directories.path().join("separate checkout");
        let administrative = directories.path().join("administrative directory");
        fixture.git(&[
            "init",
            "--separate-git-dir",
            administrative
                .to_str()
                .ok_or("administrative path is not UTF-8")?,
            separate
                .to_str()
                .ok_or("separate checkout path is not UTF-8")?,
        ])?;
        let nested = separate.join("nested/directory");
        std::fs::create_dir_all(&nested)?;
        let snapshot = RepositorySnapshot::discover(&nested)?;
        assert_eq!(
            snapshot.repository_root(),
            std::fs::canonicalize(&separate)?
        );
        assert_eq!(
            snapshot.common_directory(),
            std::fs::canonicalize(&administrative)?
        );
        Ok(())
    }
}
