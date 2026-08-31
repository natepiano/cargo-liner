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

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::constants::HEAD_FIELD_PREFIX;
use super::constants::LOCKED_FIELD;
use super::constants::PRUNABLE_FIELD;
use super::constants::WORKTREE_FIELD_PREFIX;
use super::identity;
use super::identity::RecordedWorktreeOwner;
use super::identity::RegisteredWorktreeBacklink;
use super::identity::RegisteredWorktreeIdentity;
use super::identity::RegisteredWorktreeOwnerObservation;
use super::identity::ValidatedWorktreeOwner;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::InvalidGitObjectId;
use crate::ids::RepoInstanceId;
use crate::ids::WorktreeId;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::LedgerError;
use crate::ledger::RegisteredWorktreeAvailability;
use crate::ledger::WorktreeContext;
use crate::reservation::Reservation;

/// Whether the worktree that owns a retained reservation can be validated now.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
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
    root:     PathBuf,
    state:    WorktreeRegistrationState,
    head:     WorktreeHead,
    location: RegisteredWorktreeLocation,
}

enum RegisteredWorktreeLocation {
    Discovered {
        context:  WorktreeContext,
        identity: RegisteredWorktreeIdentity,
        backlink: RegisteredWorktreeBacklink,
    },
    Unavailable,
}

/// One worktree context and identity observed for marker retention in this pass.
pub(crate) struct WorktreeMarkerSweepContext {
    context:  WorktreeContext,
    identity: RegisteredWorktreeIdentity,
}

impl WorktreeMarkerSweepContext {
    /// Sweep a marker against the active holders from the same reconciliation snapshot.
    pub(crate) fn sweep_coordination_run_marker(
        &self,
        is_active: impl Fn(WorktreeId, CoordinationRunId) -> bool,
    ) -> Result<(), LedgerError> {
        self.context
            .sweep_coordination_run_marker(|coordination_run_id| match self.identity {
                RegisteredWorktreeIdentity::Resolved(worktree_id) => {
                    is_active(worktree_id, coordination_run_id)
                },
                RegisteredWorktreeIdentity::Unavailable => false,
            })
    }
}

#[derive(Clone, Copy)]
enum WorktreeRegistrationState {
    Available,
    Locked,
    Prunable,
}

impl WorktreeRegistry {
    /// Read and parse the repository's registered worktrees once.
    pub(crate) fn read(worktree_context: &WorktreeContext) -> Result<Self, WorktreeRegistryError> {
        let porcelain = git::worktree_list_porcelain(worktree_context.repository_root())?;
        let mut registry = Self::parse(&porcelain)?;
        for registration in &mut registry.registrations {
            if matches!(registration.state, WorktreeRegistrationState::Prunable) {
                continue;
            }
            registration.location = registered_worktree_location(
                &registration.root,
                worktree_context.common_git_directory(),
            )?;
        }
        Ok(registry)
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
    ) -> Vec<WorktreeMarkerSweepContext> {
        self.registrations
            .iter()
            .filter(|registration| {
                matches!(
                    registration.state,
                    WorktreeRegistrationState::Available | WorktreeRegistrationState::Locked
                )
            })
            .filter_map(|registration| match &registration.location {
                RegisteredWorktreeLocation::Discovered {
                    context, identity, ..
                } if context.common_git_directory() == common_git_directory => {
                    Some(WorktreeMarkerSweepContext {
                        context:  context.clone(),
                        identity: *identity,
                    })
                },
                RegisteredWorktreeLocation::Discovered { .. }
                | RegisteredWorktreeLocation::Unavailable => None,
            })
            .collect()
    }

    fn validate_registration(
        ledger_repository: RepoInstanceId,
        common_git_directory: &Path,
        reservation: &Reservation,
        registration: &WorktreeRegistration,
    ) -> WorktreeLivenessObservation {
        let RegisteredWorktreeLocation::Discovered {
            context: candidate,
            identity: candidate_identity,
            backlink: candidate_backlink,
        } = &registration.location
        else {
            return observation(WorktreeLiveness::Unknown, WorktreeHead::Unavailable);
        };
        match identity::validate_same_owner(
            ledger_repository,
            common_git_directory,
            RecordedWorktreeOwner {
                repository: reservation.actor().repository,
                worktree:   reservation.actor().worktree,
                root:       reservation.worktree_root(),
                locator:    reservation.worktree_locator(),
            },
            RegisteredWorktreeOwnerObservation {
                context:  candidate,
                identity: *candidate_identity,
                backlink: *candidate_backlink,
            },
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
            registrations.push(WorktreeRegistration {
                root,
                state,
                head,
                location: RegisteredWorktreeLocation::Unavailable,
            });
        }
        Ok(Self { registrations })
    }
}

fn registered_worktree_location(
    root: &Path,
    common_git_directory: &Path,
) -> Result<RegisteredWorktreeLocation, WorktreeRegistryError> {
    match WorktreeContext::from_registered_root(root, common_git_directory).map_err(|error| {
        WorktreeRegistryError::InvalidRegisteredWorktreeContext {
            root: root.to_path_buf(),
            error,
        }
    })? {
        RegisteredWorktreeAvailability::Readable(context) => {
            let identity = RegisteredWorktreeIdentity::observe(&context);
            let backlink = RegisteredWorktreeBacklink::observe(&context);
            Ok(RegisteredWorktreeLocation::Discovered {
                context,
                identity,
                backlink,
            })
        },
        RegisteredWorktreeAvailability::Unavailable => Ok(RegisteredWorktreeLocation::Unavailable),
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
    /// The isolated registry observer stopped before returning a result.
    ObservationWorkerPanicked,
    /// A readable registered root produced an invalid administrative context.
    InvalidRegisteredWorktreeContext { root: PathBuf, error: LedgerError },
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
            Self::ObservationWorkerPanicked => {
                formatter.write_str("the worktree registry observer stopped unexpectedly")
            },
            Self::InvalidRegisteredWorktreeContext { root, error } => {
                write!(
                    formatter,
                    "registered worktree {} produced an invalid context: {error}",
                    root.display()
                )
            },
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

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::RegisteredWorktreeLocation;
    use super::registered_worktree_location;
    use crate::ids::WorktreeKind;
    use crate::ledger::WorktreeContext;

    #[test]
    fn registered_separate_git_directory_main_worktree_is_readable_as_main() {
        let temporary_directory = tempdir().expect("temporary repository parent should exist");
        let repository_root = temporary_directory.path().join("worktree");
        let administrative_directory = temporary_directory.path().join("administrative.git");
        run_git(
            temporary_directory.path(),
            &[
                "init",
                "--quiet",
                "--initial-branch=main",
                "--separate-git-dir",
                administrative_directory
                    .to_str()
                    .expect("administrative path should be UTF-8"),
                repository_root
                    .to_str()
                    .expect("worktree path should be UTF-8"),
            ],
        );
        run_git(&repository_root, &["config", "user.name", "Berth Test"]);
        run_git(
            &repository_root,
            &["config", "user.email", "berth@example.invalid"],
        );
        fs::write(
            repository_root.join("README.md"),
            "separate git directory\n",
        )
        .expect("fixture file should write");
        run_git(&repository_root, &["add", "README.md"]);
        run_git(&repository_root, &["commit", "--quiet", "-m", "initial"]);
        let worktree_context =
            WorktreeContext::discover(&repository_root).expect("main worktree should be readable");

        let location =
            registered_worktree_location(&repository_root, worktree_context.common_git_directory())
                .expect("registered main worktree should be readable");

        assert!(matches!(
            location,
            RegisteredWorktreeLocation::Discovered { context, .. }
                if context.worktree_kind() == WorktreeKind::Main
                    && context.repository_root()
                        == fs::canonicalize(repository_root)
                            .expect("repository root should canonicalize")
        ));
    }

    fn run_git(repository_root: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .args(arguments)
            .current_dir(repository_root)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git failed: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
