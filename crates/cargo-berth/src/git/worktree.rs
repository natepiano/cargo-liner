//! The native worktree registry and its shared reference snapshot.

use std::ffi::OsStr;
use std::path::Path;
use std::path::PathBuf;

use gix::Repository;
use gix::bstr::BStr;
use gix::bstr::ByteSlice as _;
use gix::refs::Target;
use gix::refs::packed::Buffer;

use super::constants::GIT_LOCAL_BRANCH_REF_PREFIX;
use super::error::GitError;
use super::read;
use super::read::GitReadRequirement;
use super::read::RepositoryRead;

/// Report linked worktrees from one registry scan and one packed-ref snapshot.
pub(super) fn worktrees(root: &Path) -> Result<RepositoryRead<Vec<u8>>, GitError> {
    let repository = match read::open(root)? {
        RepositoryRead::InProcess(repository) => repository,
        RepositoryRead::GitRequired(reason) => return Ok(RepositoryRead::GitRequired(reason)),
    };
    let main = repository
        .main_repo()
        .map_err(|error| read::failure("read main worktree", error))?;
    let main_root = std::fs::canonicalize(main.workdir().unwrap_or_else(|| main.git_dir()))
        .map_err(|error| read::failure("resolve main worktree", error))?;
    // Reuse the packed refs from the replacement-ref check in `read::open`.
    let snapshot = repository
        .refs
        .cached_packed_buffer()
        .map_err(|error| read::failure("read packed worktree refs", error))?;
    let packed = snapshot.as_deref().map(|snapshot| &**snapshot);

    let mut bytes = Vec::new();
    append_worktree_root(&mut bytes, &main_root);
    if main.is_bare() {
        bytes.extend_from_slice(b"bare\0\0");
    } else {
        if let RepositoryRead::GitRequired(reason) =
            append_worktree_head(&mut bytes, &main, packed, b"HEAD".as_bstr())?
        {
            return Ok(RepositoryRead::GitRequired(reason));
        }
        bytes.push(0);
    }

    let mut linked = Vec::new();
    for proxy in main
        .worktrees()
        .map_err(|error| read::failure("read worktree registry", error))?
    {
        let Ok(registration) = LinkedWorktree::read(proxy.git_dir()) else {
            return Ok(RepositoryRead::GitRequired(
                GitReadRequirement::ExtendedWorktreeRegistration,
            ));
        };
        linked.push((registration, proxy));
    }
    linked.sort_by(|(left, _), (right, _)| left.root.cmp(&right.root));
    for (registration, proxy) in linked {
        let id = proxy
            .id()
            .map_err(|error| read::failure("read worktree registration name", error))?;
        let mut head_name = b"worktrees/".to_vec();
        head_name.extend_from_slice(id.as_ref());
        head_name.extend_from_slice(b"/HEAD");
        append_worktree_root(&mut bytes, &registration.root);
        if let RepositoryRead::GitRequired(reason) =
            append_worktree_head(&mut bytes, &main, packed, head_name.as_bstr())?
        {
            return Ok(RepositoryRead::GitRequired(reason));
        }
        if proxy.is_locked() {
            bytes.extend_from_slice(b"locked");
            if let Some(reason) = proxy.lock_reason()
                && !reason.is_empty()
            {
                bytes.push(b' ');
                bytes.extend_from_slice(reason.as_ref());
            }
            bytes.push(0);
        } else if registration.dot_git.symlink_metadata().is_err() {
            bytes.extend_from_slice(b"prunable gitdir file points to non-existent location\0");
        }
        bytes.push(0);
    }
    Ok(RepositoryRead::InProcess(bytes))
}

struct LinkedWorktree {
    root:    PathBuf,
    dot_git: PathBuf,
}

impl LinkedWorktree {
    fn read(administration: &Path) -> Result<Self, GitError> {
        let gitdir = std::fs::read(administration.join("gitdir"))?;
        let path = gix::path::from_bstr(gitdir.trim_ascii_end().as_bstr())
            .map_err(|error| read::failure("read linked worktree path", error))?;
        if path.as_os_str().is_empty() {
            return Err(read::failure("read linked worktree path", "empty gitdir"));
        }
        let root = if path.file_name() == Some(OsStr::new(".git")) {
            path.parent().unwrap_or_else(|| path.as_ref())
        } else {
            path.as_ref()
        };
        if path.is_absolute() {
            return Ok(Self {
                root:    root.to_path_buf(),
                dot_git: path.into_owned(),
            });
        }
        let resolve = |path: &Path| {
            gix::path::realpath(administration.join(path))
                .map_err(|error| read::failure("resolve relative worktree path", error))
        };
        Ok(Self {
            root:    resolve(root)?,
            dot_git: resolve(path.as_ref())?,
        })
    }
}

fn append_worktree_root(output: &mut Vec<u8>, root: &Path) {
    output.extend_from_slice(b"worktree ");
    output.extend_from_slice(root.as_os_str().as_encoded_bytes());
    output.push(0);
}

/// Resolve the HEAD and its shared branch through the same ref store.
fn append_worktree_head(
    output: &mut Vec<u8>,
    main: &Repository,
    packed: Option<&Buffer>,
    head_name: &BStr,
) -> Result<RepositoryRead<()>, GitError> {
    let Some(head) = main
        .refs
        .try_find_packed(head_name, packed)
        .map_err(|error| read::failure("read worktree HEAD", error))?
    else {
        return Ok(RepositoryRead::GitRequired(
            GitReadRequirement::ExtendedWorktreeRegistration,
        ));
    };
    let (head_id, attachment) = match head.target {
        Target::Object(id) => (id, b"detached".as_slice().to_vec()),
        Target::Symbolic(branch) => {
            if !branch
                .as_bstr()
                .starts_with(GIT_LOCAL_BRANCH_REF_PREFIX.as_bytes())
            {
                return Ok(RepositoryRead::GitRequired(
                    GitReadRequirement::SymbolicReference,
                ));
            }
            let reference = main
                .refs
                .try_find_packed(&branch, packed)
                .map_err(|error| read::failure("read worktree branch", error))?;
            let head_id = match reference.map(|reference| reference.target) {
                Some(Target::Object(id)) => id,
                Some(Target::Symbolic(_)) => {
                    return Ok(RepositoryRead::GitRequired(
                        GitReadRequirement::SymbolicReference,
                    ));
                },
                None => gix::hash::ObjectId::null(main.object_hash()),
            };
            let mut attachment = b"branch ".to_vec();
            attachment.extend_from_slice(branch.as_bstr());
            (head_id, attachment)
        },
    };
    output.extend_from_slice(b"HEAD ");
    output.extend_from_slice(head_id.to_string().as_bytes());
    output.push(0);
    output.extend_from_slice(&attachment);
    output.push(0);
    Ok(RepositoryRead::InProcess(()))
}

#[cfg(test)]
mod tests {
    use super::worktrees;
    use crate::git::RepositorySnapshot;
    use crate::git::command;
    use crate::git::fixture::FixtureResult;
    use crate::git::fixture::PatchEquivalenceFixture;
    use crate::git::read::RepositoryRead;

    fn in_process(read: RepositoryRead<Vec<u8>>) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        match read {
            RepositoryRead::InProcess(output) => Ok(output),
            RepositoryRead::GitRequired(reason) => Err(format!("Git fallback: {reason:?}").into()),
        }
    }

    #[test]
    fn in_process_worktrees_preserve_linked_detached_and_unborn_heads() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let linked = parent.path().join("linked path");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success());
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        fixture.git(&["checkout", "--orphan", "unborn"])?;
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        Ok(())
    }

    #[test]
    fn locked_worktree_registrations_keep_git_lock_reasons() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let detached = parent.path().join("locked detached");
        let detached = detached.to_str().ok_or("path encoding")?;
        fixture.git(&["worktree", "add", "--detach", detached])?;
        fixture.git(&["worktree", "lock", "--reason", "external disk", detached])?;

        let attached = fixture.root().join("locked-branch");
        let attached = attached.to_str().ok_or("path encoding")?;
        fixture.git(&["worktree", "add", "-b", "locked-branch", attached])?;
        fixture.git(&["pack-refs", "--all"])?;
        fixture.git(&["worktree", "lock", "--reason", "packed branch", attached])?;
        assert_worktree_registration_route(&fixture)?;

        std::fs::write(
            fixture.root().join(".git/worktrees/locked-branch/gitdir"),
            "../../../locked-branch/.git\n",
        )?;
        assert_worktree_registration_route(&fixture)
    }

    #[test]
    fn prunable_worktree_registrations_keep_git_missing_checkout_diagnostics() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let linked = parent.path().join("missing path");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        std::fs::remove_dir_all(&linked)?;
        assert_worktree_registration_route(&fixture)
    }

    fn assert_worktree_registration_route(fixture: &PatchEquivalenceFixture) -> FixtureResult {
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success(), "{expected:?}");
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        assert_eq!(
            RepositorySnapshot::discover(fixture.root())?.worktree_list_porcelain()?,
            expected.stdout
        );
        Ok(())
    }

    #[test]
    fn worktree_listing_resolves_main_checkout_aliases_like_git() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        let alias = parent.path().join("main alias");
        std::os::unix::fs::symlink(fixture.root(), &alias)?;
        let linked = parent.path().join("linked");
        fixture.git(&[
            "worktree",
            "add",
            "--detach",
            linked.to_str().ok_or("path encoding")?,
        ])?;
        for checkout in [&alias, &linked] {
            let expected =
                command::git_output(checkout, ["worktree", "list", "--porcelain", "-z"])?;
            assert!(expected.status.success());
            assert_eq!(in_process(worktrees(checkout)?)?, expected.stdout);
        }
        Ok(())
    }

    #[test]
    fn worktree_listing_orders_paths_independently_of_registration_names() -> FixtureResult {
        let fixture = PatchEquivalenceFixture::new()?;
        let parent = tempfile::tempdir()?;
        for path in [parent.path().join("a/z"), parent.path().join("z/a")] {
            std::fs::create_dir_all(path.parent().ok_or("parent path")?)?;
            fixture.git(&[
                "worktree",
                "add",
                "--detach",
                path.to_str().ok_or("path encoding")?,
            ])?;
        }
        let expected =
            command::git_output(fixture.root(), ["worktree", "list", "--porcelain", "-z"])?;
        assert!(expected.status.success());
        assert_eq!(in_process(worktrees(fixture.root())?)?, expected.stdout);
        Ok(())
    }
}
