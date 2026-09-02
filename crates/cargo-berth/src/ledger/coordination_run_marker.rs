//! Content-based retirement of one worktree's coordination-run marker.

use std::fs;
use std::io::ErrorKind;
use std::path::PathBuf;

use super::error::LedgerError;
use crate::ids::CoordinationRunId;

/// A coordination-run marker atomically detached for content-based retirement.
pub(super) struct DetachedCoordinationRunMarker {
    pub(super) administrative_directory: PathBuf,
    pub(super) marker_path:              PathBuf,
    pub(super) retirement_path:          PathBuf,
}

/// Whether a marker was present when retirement atomically detached its pathname.
pub(super) enum CoordinationRunMarkerAtRetirement {
    /// No marker existed at the retirement point.
    AlreadyAbsent,
    /// The exact marker present at the retirement point has a private pathname.
    Detached(DetachedCoordinationRunMarker),
}

/// The content-based decision for one atomically detached marker.
enum DetachedCoordinationRunMarkerDisposition {
    /// The detached marker names the released run.
    Remove,
    /// The detached marker names another run.
    PreserveDifferentRun,
    /// The detached marker does not contain a UUID-v7 run id.
    PreserveMalformed,
}

/// The filesystem result of retiring one coordination-run marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CoordinationRunMarkerRemoval {
    /// The marker named the released run and was removed.
    Removed,
    /// No marker existed when release checked it.
    AlreadyAbsent,
    /// The marker named another run and remains untouched.
    PreservedDifferentRun,
    /// The marker was not a UUID-v7 run id and remains for reconciliation.
    PreservedMalformed,
}

impl DetachedCoordinationRunMarker {
    pub(super) fn retire(
        self,
        released_run_id: CoordinationRunId,
    ) -> Result<CoordinationRunMarkerRemoval, LedgerError> {
        let marker = match fs::read_to_string(&self.retirement_path) {
            Ok(marker) => marker,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        let disposition = marker.trim().parse::<CoordinationRunId>().map_or(
            DetachedCoordinationRunMarkerDisposition::PreserveMalformed,
            |marker_run_id| {
                if marker_run_id == released_run_id {
                    DetachedCoordinationRunMarkerDisposition::Remove
                } else {
                    DetachedCoordinationRunMarkerDisposition::PreserveDifferentRun
                }
            },
        );
        match disposition {
            DetachedCoordinationRunMarkerDisposition::Remove => {
                self.remove()?;
                Ok(CoordinationRunMarkerRemoval::Removed)
            },
            DetachedCoordinationRunMarkerDisposition::PreserveDifferentRun => {
                self.restore()?;
                Ok(CoordinationRunMarkerRemoval::PreservedDifferentRun)
            },
            DetachedCoordinationRunMarkerDisposition::PreserveMalformed => {
                self.restore()?;
                Ok(CoordinationRunMarkerRemoval::PreservedMalformed)
            },
        }
    }

    fn remove(&self) -> Result<(), LedgerError> {
        if fs::metadata(&self.retirement_path)?.is_dir() {
            fs::remove_dir(&self.retirement_path)?;
        } else {
            fs::remove_file(&self.retirement_path)?;
        }
        fs::File::open(&self.administrative_directory)?.sync_all()?;
        Ok(())
    }

    pub(super) fn sweep(
        self,
        active_run_matches: impl Fn(CoordinationRunId) -> bool,
    ) -> Result<(), LedgerError> {
        let retirement_metadata = match fs::metadata(&self.retirement_path) {
            Ok(retirement_metadata) => retirement_metadata,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        if retirement_metadata.is_dir() {
            return self.remove();
        }
        let marker = match fs::read_to_string(&self.retirement_path) {
            Ok(marker) => marker,
            Err(error) => {
                self.restore()?;
                return Err(LedgerError::Io(error));
            },
        };
        match marker.trim().parse::<CoordinationRunId>() {
            Ok(coordination_run_id) if active_run_matches(coordination_run_id) => self.restore(),
            Ok(_) | Err(_) => self.remove(),
        }
    }

    fn restore(&self) -> Result<(), LedgerError> {
        match fs::hard_link(&self.retirement_path, &self.marker_path) {
            Ok(()) => {},
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {},
            Err(error) => return Err(LedgerError::Io(error)),
        }
        self.remove()
    }
}
