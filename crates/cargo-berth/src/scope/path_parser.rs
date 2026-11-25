//! Parse reservation paths from commands and literal edit targets.

use std::ffi::OsString;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs::Metadata;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use crate::ids::InvalidReservationScopePath;
use crate::ids::ReservationScopePath;
use crate::ledger::LedgerError;
use crate::ledger::ReservationScope;
use crate::ledger::ScopeKind;
use crate::ledger::WorktreeContext;
use crate::session;

const FILE_SCOPE_PREFIX: &str = "file:";
const TREE_SCOPE_PREFIX: &str = "tree:";

/// Keep the worktree root after the first argument that needs a filesystem check.
pub(super) enum ScopePathParser {
    Literal,
    CommandLine,
    CommandLineInWorktree(PathBuf),
}

impl ScopePathParser {
    pub(super) fn parse(
        &mut self,
        path: PathBuf,
        default_kind: ScopeKind,
    ) -> Result<ReservationScope, DeclaredReservationScopeError> {
        let path = path
            .into_os_string()
            .into_string()
            .map_err(DeclaredReservationScopeError::NonUtf8)?;
        let (kind, path) = path.strip_prefix(FILE_SCOPE_PREFIX).map_or_else(
            || {
                path.strip_prefix(TREE_SCOPE_PREFIX)
                    .map_or((default_kind, path.as_str()), |path| {
                        (ScopeKind::Tree, path)
                    })
            },
            |path| (ScopeKind::File, path),
        );
        let path = path
            .parse()
            .map_err(DeclaredReservationScopeError::InvalidPath)?;
        self.validate_argument(&path)?;
        Ok(ReservationScope { path, kind })
    }

    fn validate_argument(
        &mut self,
        path: &ReservationScopePath,
    ) -> Result<(), DeclaredReservationScopeError> {
        if matches!(self, Self::Literal) || !path.as_str().chars().any(char::is_whitespace) {
            return Ok(());
        }
        if matches!(self, Self::CommandLine) {
            let directory = session::command_directory()
                .map_err(DeclaredReservationScopeError::CommandDirectory)?;
            let worktree = WorktreeContext::discover(&directory)
                .map_err(DeclaredReservationScopeError::CommandWorktree)?;
            *self = Self::CommandLineInWorktree(worktree.repository_root().to_path_buf());
        }
        if let Self::CommandLineInWorktree(root) = self {
            validate_whitespace_path(root, path)?;
        }
        Ok(())
    }
}

fn validate_whitespace_path(
    root: &Path,
    path: &ReservationScopePath,
) -> Result<(), DeclaredReservationScopeError> {
    let full_path = root.join(path.as_str());
    if path_metadata(&full_path)?.is_some_and(|entry| entry.is_file() || entry.is_dir()) {
        return Ok(());
    }
    for part in path.as_str().split_whitespace() {
        if let Ok(part) = part.parse::<ReservationScopePath>()
            && path_metadata(&root.join(part.as_str()))?
                .is_some_and(|entry| entry.is_file() || entry.is_dir())
        {
            return Err(DeclaredReservationScopeError::ExistingPathPart {
                path: path.clone(),
                part,
            });
        }
    }
    if let Some(parent) = full_path.parent()
        && path_metadata(parent)?.is_some_and(|entry| entry.is_dir())
    {
        return Ok(());
    }
    Err(DeclaredReservationScopeError::MissingParent(path.clone()))
}

fn path_metadata(path: &Path) -> Result<Option<Metadata>, DeclaredReservationScopeError> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
            ) =>
        {
            Ok(None)
        },
        Err(error) => Err(DeclaredReservationScopeError::InspectPath {
            path: path.to_path_buf(),
            error,
        }),
    }
}

/// A failure while parsing one declared scope.
#[derive(Debug)]
pub(crate) enum DeclaredReservationScopeError {
    NonUtf8(OsString),
    InvalidPath(InvalidReservationScopePath),
    CommandDirectory(io::Error),
    CommandWorktree(LedgerError),
    InspectPath {
        path:  PathBuf,
        error: io::Error,
    },
    ExistingPathPart {
        path: ReservationScopePath,
        part: ReservationScopePath,
    },
    MissingParent(ReservationScopePath),
}

impl Display for DeclaredReservationScopeError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonUtf8(path) => write!(
                formatter,
                "reservation paths must be UTF-8; correct this path and retry: {}",
                path.to_string_lossy()
            ),
            Self::InvalidPath(error) => write!(
                formatter,
                "{error}; use a normalized repository-relative path and retry"
            ),
            Self::CommandDirectory(error) => write!(
                formatter,
                "scope_command_directory_unavailable: Cannot read the command directory ({error})."
            ),
            Self::CommandWorktree(error) => write!(
                formatter,
                "scope_worktree_unavailable: Cannot find the command worktree ({error})."
            ),
            Self::InspectPath { path, error } => write!(
                formatter,
                "scope_path_unreadable: Cannot inspect {} ({error}).",
                path.display()
            ),
            Self::ExistingPathPart { path, part } => write!(
                formatter,
                "path_contains_whitespace: {:?} does not exist and includes the existing path {:?}. Pass each reservation path as a separate argument.",
                path.as_str(),
                part.as_str()
            ),
            Self::MissingParent(path) => write!(
                formatter,
                "path_contains_whitespace: The parent folder of {:?} does not exist. Pass each reservation path as a separate argument.",
                path.as_str()
            ),
        }
    }
}

impl std::error::Error for DeclaredReservationScopeError {}
