//! The temporary git repository every scoped-proof and reachability test builds on.
//!
//! The fixture seeds one commit and hands back its object id as the phase start,
//! which is what both the patch-equivalence tests and the reachability tests
//! need. It lives in its own module because those tests sit in different files.

use std::error::Error;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;
use tempfile::tempdir;

use super::error::GitError;
use super::patch;
use super::patch::ScopedPatchComparison;
use super::refs;
use crate::ids::GitObjectId;
use crate::ledger::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

const INITIAL_PRIMARY: &str = "first\nsecond\nthird\n";
const INITIAL_SECONDARY: &str = "secondary\n";
pub(super) const PRIMARY_BACKUP_PATH: &str = "src/primary.rs~backup";
pub(super) const PRIMARY_PATH: &str = "src/primary.rs";
pub(super) const SCRIPT_PATH: &str = "scripts/run.sh";
pub(super) const SECONDARY_PATH: &str = "src/secondary.rs";
pub(super) const UNAVAILABLE_OBJECT_ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

pub(super) type FixtureResult<T = ()> = Result<T, Box<dyn Error>>;

pub(super) struct PatchEquivalenceFixture {
    repository:                  TempDir,
    pub(super) phase_start_head: GitObjectId,
}

impl PatchEquivalenceFixture {
    pub(super) fn new() -> FixtureResult<Self> {
        let repository = tempdir()?;
        run_git(
            repository.path(),
            &["init", "--quiet", "--initial-branch", "main"],
        )?;
        run_git(
            repository.path(),
            &["config", "user.email", "test@example.com"],
        )?;
        run_git(repository.path(), &["config", "user.name", "Test User"])?;
        write_file(repository.path(), PRIMARY_PATH, INITIAL_PRIMARY)?;
        write_file(repository.path(), SECONDARY_PATH, INITIAL_SECONDARY)?;
        write_file(repository.path(), SCRIPT_PATH, "#!/bin/sh\nexit 0\n")?;
        run_git(repository.path(), &["add", "."])?;
        run_git(repository.path(), &["commit", "--quiet", "-m", "initial"])?;
        let phase_start_head = refs::head_object_id(repository.path())?;
        Ok(Self {
            repository,
            phase_start_head,
        })
    }

    pub(super) fn root(&self) -> &Path { self.repository.path() }

    pub(super) fn write(&self, path: &str, contents: &str) -> io::Result<()> {
        write_file(self.root(), path, contents)
    }

    pub(super) fn remove(&self, path: &str) -> io::Result<()> {
        fs::remove_file(self.root().join(path))
    }

    pub(super) fn git(&self, arguments: &[&str]) -> io::Result<()> {
        run_git(self.root(), arguments)
    }

    pub(super) fn commit(&self, message: &str) -> FixtureResult<GitObjectId> {
        self.git(&["add", "--all"])?;
        self.git(&["commit", "--quiet", "-m", message])?;
        Ok(refs::head_object_id(self.root())?)
    }

    pub(super) fn amend(&self, message: &str) -> FixtureResult<GitObjectId> {
        self.git(&["commit", "--quiet", "--amend", "-m", message])?;
        Ok(refs::head_object_id(self.root())?)
    }

    pub(super) fn reset_to_phase_start(&self) -> io::Result<()> {
        self.reset_to(&self.phase_start_head)
    }

    pub(super) fn reset_to(&self, target: &GitObjectId) -> io::Result<()> {
        self.git(&["reset", "--hard", &target.to_string()])
    }

    pub(super) fn set_executable(&self, path: &str) -> io::Result<()> {
        let path = self.root().join(path);
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
    }

    pub(super) fn equivalence(
        &self,
        scopes: &ReservationScopeSet,
        protected_tip: &GitObjectId,
        target: &GitObjectId,
    ) -> Result<ScopedPatchComparison, GitError> {
        patch::scoped_patch_equivalence(
            self.root(),
            &self.phase_start_head,
            scopes,
            protected_tip,
            target,
        )
    }
}

pub(super) fn file_scopes(paths: &[&str]) -> FixtureResult<ReservationScopeSet> {
    scopes(paths, ScopeKind::File)
}

pub(super) fn tree_scopes(paths: &[&str]) -> FixtureResult<ReservationScopeSet> {
    scopes(paths, ScopeKind::Tree)
}

fn scopes(paths: &[&str], scope_kind: ScopeKind) -> FixtureResult<ReservationScopeSet> {
    let scopes = paths
        .iter()
        .map(|path| {
            Ok(ReservationScope {
                path: path.parse()?,
                kind: scope_kind,
            })
        })
        .collect::<FixtureResult<Vec<_>>>()?;
    Ok(ReservationScopeSet::try_from(scopes)?)
}

fn write_file(repository_root: &Path, path: &str, contents: &str) -> io::Result<()> {
    let path = repository_root.join(path);
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("fixture path should have a parent"))?;
    fs::create_dir_all(parent)?;
    fs::write(path, contents)
}

fn run_git(repository_root: &Path, arguments: &[&str]) -> io::Result<()> {
    let output = Command::new("git")
        .arg("--no-optional-locks")
        .args(arguments)
        .current_dir(repository_root)
        .output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "git {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}
