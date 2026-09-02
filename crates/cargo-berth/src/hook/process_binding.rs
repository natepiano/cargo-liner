//! Binding one hook process to the repository and harness session its payload names.
//!
//! Both decisions are made from the raw payload before any repository work starts, and
//! both are the same for every hook event, so the three event modules share them.

use std::path::PathBuf;

use crate::session;
use crate::session::HarnessSessionId;
use crate::session::HookHarnessSessionSelection;

/// How a hook chooses the directory whose repository owns its answer.
pub(super) enum HookWorkingDirectorySelection {
    /// The payload supplied a non-empty working directory.
    PayloadSupplied(PathBuf),
    /// The payload omitted its working directory, so the process directory applies.
    CurrentProcess,
}

/// Whether a hook can select a disposable harness-session mapping.
pub(super) enum HarnessSessionIdentityAvailability {
    /// The payload supplied a valid bounded session identifier.
    Available(HarnessSessionId),
    /// The payload supplied no identifier, or one unsuitable for durable lookup.
    Unusable,
}

/// A hook could not establish the directory whose repository owns its answer.
pub(super) enum HookWorkingDirectoryResolutionError {
    /// The payload named no directory and this process has no readable current directory.
    CurrentProcessUnavailable,
}

/// A hook could not enter the directory the payload named for its answer.
///
/// Only a payload-supplied directory can fail: a payload that names none leaves this
/// process where the harness launched it, which needs no move and cannot be refused.
pub(super) struct HookWorkingDirectoryUnavailable {
    /// The directory the payload named, in the words the payload named it.
    pub(super) working_directory: PathBuf,
}

impl HookWorkingDirectorySelection {
    pub(super) fn from_boundary(working_directory: Option<String>) -> Self {
        working_directory
            .filter(|working_directory| !working_directory.is_empty())
            .map_or(Self::CurrentProcess, |working_directory| {
                Self::PayloadSupplied(PathBuf::from(working_directory))
            })
    }

    pub(super) fn resolve(&self) -> Result<PathBuf, HookWorkingDirectoryResolutionError> {
        match self {
            Self::PayloadSupplied(working_directory) => Ok(working_directory.clone()),
            Self::CurrentProcess => std::env::current_dir()
                .map_err(|_| HookWorkingDirectoryResolutionError::CurrentProcessUnavailable),
        }
    }

    /// Place this process in the directory whose repository owns the hook's answer.
    ///
    /// The payload's directory is entered exactly as the harness named it, so the kernel
    /// resolves its symlinks and `..` components. Collapsing them textually first would
    /// select a different directory whenever a symlinked ancestor sits left of a `..` —
    /// `/link/../repo` is `/repo` as text and the sibling of `/link`'s target in the
    /// filesystem — and the whole answer would then be about the wrong repository. A
    /// payload that names no directory leaves this process where the harness launched
    /// it, which is already the directory the answer is about.
    ///
    /// A `cwd` present but not a string never reaches here at all: every hook event's
    /// payload boundary rejects it as an invalid payload rather than coercing it away,
    /// because a coerced `cwd` silently observes a different repository.
    pub(super) fn enter_current_process(&self) -> Result<(), HookWorkingDirectoryUnavailable> {
        match self {
            Self::PayloadSupplied(working_directory) => {
                std::env::set_current_dir(working_directory).map_err(|_| {
                    HookWorkingDirectoryUnavailable {
                        working_directory: working_directory.clone(),
                    }
                })
            },
            Self::CurrentProcess => Ok(()),
        }
    }
}

impl HarnessSessionIdentityAvailability {
    pub(super) fn from_boundary(harness_session_id: Option<String>) -> Self {
        harness_session_id
            .filter(|harness_session_id| !harness_session_id.is_empty())
            .map_or(Self::Unusable, |harness_session_id| {
                harness_session_id
                    .parse()
                    .map_or(Self::Unusable, Self::Available)
            })
    }

    /// Bind this process to the payload's session identity, or to no session at all.
    ///
    /// An absent or unusable payload identity must not fall through to an ambient
    /// `CARGO_BERTH_SESSION_ID`. That variable belongs to whichever session launched this
    /// hook process, so adopting it would map the event onto another session's reservation.
    pub(super) fn select_for_current_process(self) {
        session::select_current_process_harness_session(match self {
            Self::Available(harness_session_id) => {
                HookHarnessSessionSelection::Session(harness_session_id)
            },
            Self::Unusable => HookHarnessSessionSelection::NoSession,
        });
    }
}
