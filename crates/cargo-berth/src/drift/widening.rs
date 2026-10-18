//! Admission of unheld work into one reservation's drift widening.

use std::borrow::Borrow;

use super::observation::ObservedDriftChanges;
use crate::ids::ReservationScopePath;
use crate::reservation::DriftBlockingCoverage;
use crate::reservation::Reservation;
use crate::reservation::RetainedProtectedTip;
use crate::reservation::RetainedReservationSet;

/// The widening rules for one subject and its `ObservedDriftChanges`.
pub(super) struct WideningAdmission<'a> {
    subject:        &'a Reservation,
    changes:        &'a ObservedDriftChanges,
    comparison_end: ComparisonEnd,
}

impl<'a> WideningAdmission<'a> {
    pub(super) fn new(
        reservations: &RetainedReservationSet,
        subject: &'a Reservation,
        changes: &'a ObservedDriftChanges,
    ) -> Self {
        let comparison_end = if reservations.iter().any(|sibling| {
            sibling.id() != subject.id()
                && sibling
                    .actor()
                    .has_coordination_identity(subject.actor().worktree, subject.actor().run)
                && matches!(
                    sibling.protected_checkpoint(),
                    RetainedProtectedTip::Retained(tip)
                        if changes.compares_to_checkpoint(tip.as_ref())
                )
        }) {
            ComparisonEnd::SiblingCheckpoint
        } else {
            ComparisonEnd::OwnRange
        };
        Self {
            subject,
            changes,
            comparison_end,
        }
    }

    /// Return `WideningPathAdmission::Admitted` only for `DriftBlockingCoverage::Unclaimed`
    /// paths that `ObservedDriftChanges::carries_work`.
    ///
    /// A `committed_only` path also requires `head_commit_introduced` and a comparison
    /// that does not end at a sibling's `protected_checkpoint`. A dirty path, or a commit
    /// the subject makes later, classifies as before: a path B dirties again after A's
    /// checkpoint still widens B.
    pub(super) fn admit(
        &self,
        path: &ReservationScopePath,
        coverage: &DriftBlockingCoverage,
    ) -> WideningPathAdmission {
        if !matches!(coverage, DriftBlockingCoverage::Unclaimed) || !self.changes.carries_work(path)
        {
            return WideningPathAdmission::Excluded;
        }
        if self.changes.committed_only(self.subject.id(), path) {
            match self.comparison_end {
                ComparisonEnd::SiblingCheckpoint => return WideningPathAdmission::Excluded,
                ComparisonEnd::OwnRange if !self.changes.head_commit_introduced(path) => {
                    return WideningPathAdmission::Excluded;
                },
                ComparisonEnd::OwnRange => {},
            }
        }
        WideningPathAdmission::Admitted(UnheldWideningPath(path.clone()))
    }
}

/// Where the full comparison ends relative to the subject's siblings.
enum ComparisonEnd {
    /// The comparison ends at a same-worktree, same-run sibling's checkpoint.
    SiblingCheckpoint,
    /// The comparison ends within the subject's range, outside sibling checkpoints.
    OwnRange,
}

pub(super) enum WideningPathAdmission {
    Excluded,
    Admitted(UnheldWideningPath),
}

/// A `DriftBlockingCoverage::Unclaimed` path not contributed solely by a same-worktree,
/// same-run sibling's `protected_checkpoint`; only `WideningAdmission::admit` issues it.
pub(super) struct UnheldWideningPath(ReservationScopePath);

impl UnheldWideningPath {
    pub(super) fn into_path(self) -> ReservationScopePath { self.0 }
}

impl Borrow<ReservationScopePath> for UnheldWideningPath {
    fn borrow(&self) -> &ReservationScopePath { &self.0 }
}
