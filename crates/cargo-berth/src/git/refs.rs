//! Reservation retention references.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;

use super::GitError;
use super::command;
use super::constants::GIT_DELETE_REF_ARG;
use super::constants::GIT_UPDATE_REF_COMMAND;
use super::constants::RESERVATION_RETENTION_REF_PREFIX;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;

/// The full private git ref that retains one reservation's protected tip.
pub(super) struct ReservationRetentionRef(String);

impl ReservationRetentionRef {
    fn for_reservation(reservation_id: ReservationId) -> Self {
        Self(format!(
            "{RESERVATION_RETENTION_REF_PREFIX}{reservation_id}"
        ))
    }
}

/// Return the serialized retention ref for one reservation.
pub(super) fn name(reservation_id: ReservationId) -> String {
    ReservationRetentionRef::for_reservation(reservation_id).to_string()
}

impl Display for ReservationRetentionRef {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) }
}

/// Create or update the retention ref for a protected tip.
pub(super) fn write(
    repository_root: &Path,
    reservation_id: ReservationId,
    protected_tip: &GitObjectId,
) -> Result<(), GitError> {
    let retention_ref = name(reservation_id);
    let protected_tip = protected_tip.to_string();
    let output = command::git_output(
        repository_root,
        [GIT_UPDATE_REF_COMMAND, &retention_ref, &protected_tip],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: GIT_UPDATE_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}

/// Delete a reservation retention ref.
pub(super) fn delete(
    repository_root: &Path,
    reservation_id: ReservationId,
) -> Result<(), GitError> {
    let retention_ref = name(reservation_id);
    let output = command::git_output(
        repository_root,
        [GIT_UPDATE_REF_COMMAND, GIT_DELETE_REF_ARG, &retention_ref],
    )?;
    if output.status.success() {
        Ok(())
    } else {
        Err(GitError::CommandFailed {
            command: GIT_UPDATE_REF_COMMAND,
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        })
    }
}
