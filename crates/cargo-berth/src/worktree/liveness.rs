//! Typed worktree liveness derived from git's porcelain registry and opaque identity.

use std::error::Error;
#[cfg(unix)]
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
#[cfg(unix)]
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;
use std::str::Utf8Error;
#[cfg(not(unix))]
use std::string::FromUtf8Error;

use serde::Deserialize;
use serde::Serialize;

use super::constants::HEAD_FIELD_PREFIX;
use super::constants::LOCKED_FIELD;
use super::constants::PRUNABLE_FIELD;
use super::constants::WORKTREE_FIELD_PREFIX;
use super::identity;
use super::identity::ValidatedWorktreeOwner;
use crate::git;
use crate::git::GitError;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::RepoInstanceId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::WorktreeContext;
use crate::reservation::Reservation;

/// Whether the worktree that owns a retained reservation can be validated now.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktreeLiveness {
    /// Git and the opaque identity validate the recorded holder.
    Live,
    /// Git deliberately retains a locked registration whose path may be absent.
    Unavailable,
    /// Git still has a prunable registration for a missing worktree path.
    OrphanCandidate,
    /// Git pruned the registration and the recorded path can be recycled.
    Orphaned,
    /// Available evidence could not establish the holder's identity.
    Unknown,
}

/// The holder commit included in the one repository worktree-list observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeHead {
    /// Git reported this full commit for the registered holder.
    Resolved(GitObjectId),
    /// No validated registered holder commit is available.
    Unavailable,
}

/// Whether reconciliation must update the recorded root for an otherwise live holder.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WorktreeRelocation {
    /// The recorded root remains current or the holder is not live.
    Unchanged,
    /// The same opaque holder moved to this canonical root.
    Relocated { current_root: CanonicalWorktreeRoot },
}

/// One liveness conclusion and its possible same-identity relocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct WorktreeLivenessObservation {
    /// The holder state that determines whether scopes and edges remain retained.
    pub(crate) liveness:   WorktreeLiveness,
    /// A root update that is valid only because opaque identity validation succeeded.
    pub(crate) relocation: WorktreeRelocation,
    /// The commit reported for the same registered holder.
    pub(crate) head:       WorktreeHead,
}

/// Parsed registered worktrees from one `git worktree list --porcelain` call.
pub(crate) struct WorktreeRegistry {
    registrations: Vec<WorktreeRegistration>,
}

struct WorktreeRegistration {
    root:  PathBuf,
    state: WorktreeRegistrationState,
    head:  WorktreeHead,
}

#[derive(Clone, Copy)]
enum WorktreeRegistrationState {
    Available,
    Locked,
    Prunable,
}

impl WorktreeRegistry {
    /// Read and parse the repository's registered worktrees once.
    pub(crate) fn read(repository_root: &Path) -> Result<Self, WorktreeRegistryError> {
        let porcelain = git::worktree_list_porcelain(repository_root)?;
        Self::parse(&porcelain)
    }

    /// Classify one recorded holder without treating any absence as abandonment.
    pub(crate) fn classify(
        &self,
        ledger_repository: RepoInstanceId,
        common_git_directory: &Path,
        reservation: &Reservation,
    ) -> WorktreeLivenessObservation {
        if let Some(registration) = self
            .registrations
            .iter()
            .find(|registration| registration.root == reservation.worktree_root().as_ref())
        {
            return match registration.state {
                WorktreeRegistrationState::Locked => {
                    observation(WorktreeLiveness::Unavailable, registration.head.clone())
                },
                WorktreeRegistrationState::Prunable => {
                    observation(WorktreeLiveness::OrphanCandidate, registration.head.clone())
                },
                WorktreeRegistrationState::Available => Self::validate_registration(
                    ledger_repository,
                    common_git_directory,
                    reservation,
                    registration,
                ),
            };
        }

        for registration in self.registrations.iter().filter(|registration| {
            matches!(registration.state, WorktreeRegistrationState::Available)
        }) {
            let validated = Self::validate_registration(
                ledger_repository,
                common_git_directory,
                reservation,
                registration,
            );
            if matches!(validated.liveness, WorktreeLiveness::Live) {
                return validated;
            }
        }
        observation(WorktreeLiveness::Orphaned, WorktreeHead::Unavailable)
    }

    /// Discover contexts whose administrative directories are eligible for marker sweeping.
    pub(crate) fn marker_sweep_contexts(
        &self,
        common_git_directory: &Path,
    ) -> Vec<WorktreeContext> {
        self.registrations
            .iter()
            .filter(|registration| {
                matches!(
                    registration.state,
                    WorktreeRegistrationState::Available | WorktreeRegistrationState::Locked
                )
            })
            .filter_map(|registration| WorktreeContext::discover(&registration.root).ok())
            .filter(|context| context.common_git_directory() == common_git_directory)
            .collect()
    }

    fn validate_registration(
        ledger_repository: RepoInstanceId,
        common_git_directory: &Path,
        reservation: &Reservation,
        registration: &WorktreeRegistration,
    ) -> WorktreeLivenessObservation {
        let Ok(candidate) = WorktreeContext::discover(&registration.root) else {
            return observation(WorktreeLiveness::Unknown, WorktreeHead::Unavailable);
        };
        match identity::validate_same_owner(
            ledger_repository,
            reservation.actor().repository,
            common_git_directory,
            reservation.actor().worktree,
            reservation.worktree_root(),
            reservation.worktree_locator(),
            &candidate,
        ) {
            Ok(ValidatedWorktreeOwner::RecordedRoot) => {
                observation(WorktreeLiveness::Live, registration.head.clone())
            },
            Ok(ValidatedWorktreeOwner::Relocated { current_root }) => WorktreeLivenessObservation {
                liveness:   WorktreeLiveness::Live,
                relocation: WorktreeRelocation::Relocated { current_root },
                head:       registration.head.clone(),
            },
            Err(_) => observation(WorktreeLiveness::Unknown, WorktreeHead::Unavailable),
        }
    }

    fn parse(porcelain: &[u8]) -> Result<Self, WorktreeRegistryError> {
        let mut registrations = Vec::new();
        let mut fields = porcelain.split(|byte| *byte == b'\0');
        while let Some(root_field) = fields.next() {
            if root_field.is_empty() {
                continue;
            }
            let root = root_field
                .strip_prefix(WORKTREE_FIELD_PREFIX.as_bytes())
                .ok_or(WorktreeRegistryError::MissingRoot)?;
            #[cfg(unix)]
            let root = PathBuf::from(OsStr::from_bytes(root));
            #[cfg(not(unix))]
            let root = String::from_utf8(root.to_vec())
                .map(PathBuf::from)
                .map_err(WorktreeRegistryError::InvalidPathEncoding)?;
            let mut state = WorktreeRegistrationState::Available;
            let mut head = WorktreeHead::Unavailable;
            for field in fields.by_ref() {
                if field.is_empty() {
                    break;
                }
                if let Some(head_field) = field.strip_prefix(HEAD_FIELD_PREFIX.as_bytes()) {
                    let head_text = std::str::from_utf8(head_field)
                        .map_err(WorktreeRegistryError::InvalidHeadEncoding)?;
                    head = WorktreeHead::Resolved(
                        head_text
                            .parse()
                            .map_err(WorktreeRegistryError::InvalidHead)?,
                    );
                } else if field.starts_with(LOCKED_FIELD.as_bytes()) {
                    state = WorktreeRegistrationState::Locked;
                } else if field.starts_with(PRUNABLE_FIELD.as_bytes())
                    && !matches!(state, WorktreeRegistrationState::Locked)
                {
                    state = WorktreeRegistrationState::Prunable;
                }
            }
            registrations.push(WorktreeRegistration { root, state, head });
        }
        Ok(Self { registrations })
    }
}

const fn observation(
    liveness: WorktreeLiveness,
    head: WorktreeHead,
) -> WorktreeLivenessObservation {
    WorktreeLivenessObservation {
        liveness,
        relocation: WorktreeRelocation::Unchanged,
        head,
    }
}

/// A failure to read or parse git's registered-worktree representation.
#[derive(Debug)]
pub(crate) enum WorktreeRegistryError {
    /// Git could not list registered worktrees.
    Git(GitError),
    /// One porcelain record did not begin with its required worktree root.
    MissingRoot,
    /// A porcelain HEAD field was not UTF-8.
    InvalidHeadEncoding(Utf8Error),
    /// A porcelain HEAD field was not a full git object id.
    InvalidHead(InvalidGitObjectId),
    /// A non-Unix platform could not decode git's worktree root representation.
    #[cfg(not(unix))]
    InvalidPathEncoding(FromUtf8Error),
}

impl Display for WorktreeRegistryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Git(error) => error.fmt(formatter),
            Self::MissingRoot => {
                formatter.write_str("git worktree porcelain omitted a worktree root")
            },
            Self::InvalidHeadEncoding(error) => {
                write!(
                    formatter,
                    "git worktree porcelain returned a non-UTF-8 HEAD: {error}"
                )
            },
            Self::InvalidHead(error) => error.fmt(formatter),
            #[cfg(not(unix))]
            Self::InvalidPathEncoding(error) => {
                write!(formatter, "git returned an invalid worktree path: {error}")
            },
        }
    }
}

impl Error for WorktreeRegistryError {}

impl From<GitError> for WorktreeRegistryError {
    fn from(error: GitError) -> Self { Self::Git(error) }
}
