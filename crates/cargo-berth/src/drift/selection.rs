//! The parsed drift request and the reservations one invocation acts on.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::identity::DriftActingIdentity;
use super::identity::DriftActingRun;
use super::identity::DriftSessionReservation;
use super::report::DriftAttributionCandidateSet;
use super::report::DriftComparisonMode;
use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ids::WorktreeId;
use crate::reservation::Reservation;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;

/// Which working-tree comparison the caller requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftComparisonChoice {
    /// Compare the cheap working-tree observation with the last cache entry.
    CheapDelta,
    /// Compare all active-phase changes with the claim's protected starting commit.
    FullPhaseStart,
}

impl DriftComparisonChoice {
    pub(super) const fn report_mode(self) -> DriftComparisonMode {
        match self {
            Self::CheapDelta => DriftComparisonMode::CheapDelta,
            Self::FullPhaseStart => DriftComparisonMode::FullPhaseStart,
        }
    }
}

/// How a hand-run or hook-run drift command chooses its reservation subjects.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DriftReservationSelection {
    /// Act on the caller-supplied reservation.
    Explicit(ReservationId),
    /// Prefer the session-mapped reservation, otherwise require one active match.
    SessionMappingOrSingleActive,
    /// Report across every local reservation while attributing widening separately.
    EveryActiveForPostCommit {
        /// How an explicit flag or implicit identity selects the widening target.
        widening: PostCommitWideningSelection,
    },
}

impl DriftReservationSelection {
    pub(super) fn resolve(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        if let Self::EveryActiveForPostCommit { widening } = self {
            return widening.resolve_post_commit(reservations, acting_identity);
        }
        let run = match acting_identity.acting_run() {
            DriftActingRun::Identified(run) => run,
            DriftActingRun::Unidentified => return Err(DriftSelectionError::UnidentifiedActingRun),
        };
        let candidates = WireOrderedReservationIds::sorted(
            reservations
                .iter()
                .filter(|reservation| {
                    reservation.is_active_for_coordination_run_and_worktree(run, worktree)
                })
                .map(Reservation::id)
                .collect(),
        );
        match self {
            Self::Explicit(reservation_id) if candidates.as_slice().contains(&reservation_id) => {
                Ok(ResolvedDriftSubjects {
                    reporting:              WireOrderedReservationIds::sorted(vec![reservation_id]),
                    widening:               DriftWideningSelection::Selected(reservation_id),
                    post_write_first_touch: PostWriteFirstTouchRequirement::NotRequired,
                })
            },
            Self::Explicit(reservation_id) => Err(DriftSelectionError::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            }),
            Self::SessionMappingOrSingleActive => {
                let selected = match acting_identity.session_reservation() {
                    DriftSessionReservation::Mapped(reservation_id)
                        if candidates.as_slice().contains(&reservation_id) =>
                    {
                        reservation_id
                    },
                    DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                        match candidates.as_slice() {
                            [reservation_id] => *reservation_id,
                            [] => {
                                return Err(DriftSelectionError::NoActiveReservation {
                                    run,
                                    worktree,
                                });
                            },
                            _ => {
                                return Err(DriftSelectionError::AmbiguousActiveReservations(
                                    candidates,
                                ));
                            },
                        }
                    },
                };
                Ok(ResolvedDriftSubjects {
                    reporting:              WireOrderedReservationIds::sorted(vec![selected]),
                    widening:               DriftWideningSelection::Selected(selected),
                    post_write_first_touch: PostWriteFirstTouchRequirement::NotRequired,
                })
            },
            Self::EveryActiveForPostCommit { .. } => {
                Err(DriftSelectionError::NoPostCommitCandidate)
            },
        }
    }
}

/// How a post-commit invocation chooses its one possible widening target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PostCommitWideningSelection {
    /// Widen only the caller-supplied active reservation.
    Explicit(ReservationId),
    /// Prefer the session mapping, then the only active local candidate.
    SessionMappingOrSingleCandidate,
}

impl PostCommitWideningSelection {
    fn resolve_post_commit(
        self,
        reservations: &RetainedReservationSet,
        acting_identity: DriftActingIdentity,
    ) -> Result<ResolvedDriftSubjects, DriftSelectionError> {
        let worktree = acting_identity.worktree();
        // A post-commit observation reports on the whole worktree, so the acting run is
        // deliberately unconstrained here. This asks less than
        // `Reservation::is_active_for_coordination_run_and_worktree`: dropping the run term
        // matches a superset of what that method matches, so the method cannot stand in for
        // this filter. The filter stays inline on purpose — widening the method to cover both
        // would hide which of the two a site actually means.
        let reporting = WireOrderedReservationIds::sorted(
            reservations
                .iter()
                .filter(|reservation| {
                    matches!(reservation.lifecycle(), ReservationLifecycle::Active)
                        && reservation.actor().worktree == worktree
                })
                .map(Reservation::id)
                .collect(),
        );
        let acting_run = acting_identity.acting_run();
        let candidates = WireOrderedReservationIds::sorted(match acting_run {
            DriftActingRun::Identified(run) => reservations
                .iter()
                .filter(|reservation| {
                    reservation.is_active_for_coordination_run_and_worktree(run, worktree)
                })
                .map(Reservation::id)
                .collect(),
            DriftActingRun::Unidentified => Vec::new(),
        });
        let post_write_first_touch = match candidates.as_slice() {
            [] => PostWriteFirstTouchRequirement::Required,
            [_, ..] => PostWriteFirstTouchRequirement::NotRequired,
        };
        let widening = match self {
            Self::Explicit(reservation_id) => {
                let DriftActingRun::Identified(run) = acting_run else {
                    return Err(DriftSelectionError::UnidentifiedActingRun);
                };
                if !candidates.as_slice().contains(&reservation_id) {
                    return Err(DriftSelectionError::ExplicitNotActive {
                        reservation_id,
                        run,
                        worktree,
                    });
                }
                DriftWideningSelection::Selected(reservation_id)
            },
            Self::SessionMappingOrSingleCandidate
                if matches!(acting_run, DriftActingRun::Unidentified) =>
            {
                DriftWideningSelection::NotNeeded
            },
            Self::SessionMappingOrSingleCandidate => match acting_identity.session_reservation() {
                DriftSessionReservation::Mapped(reservation_id)
                    if candidates.as_slice().contains(&reservation_id) =>
                {
                    DriftWideningSelection::Selected(reservation_id)
                },
                DriftSessionReservation::Mapped(_) | DriftSessionReservation::Unavailable => {
                    match candidates.as_slice() {
                        [] => DriftWideningSelection::NotNeeded,
                        [reservation_id] => DriftWideningSelection::Selected(*reservation_id),
                        _ => DriftWideningSelection::Ambiguous(
                            DriftAttributionCandidateSet::try_from(candidates.into_vec())
                                .map_err(|_| DriftSelectionError::NoPostCommitCandidate)?,
                        ),
                    }
                },
            },
        };
        Ok(ResolvedDriftSubjects {
            reporting,
            widening,
            post_write_first_touch,
        })
    }
}

/// A drift request after clap primitives have been converted into domain choices.
#[derive(Clone, Copy)]
pub(crate) struct DriftRequest {
    /// The comparison algorithm selected at the command boundary.
    pub(crate) comparison:  DriftComparisonChoice,
    /// The semantic reservation-selection rule.
    pub(crate) reservation: DriftReservationSelection,
}

pub(super) struct ResolvedDriftSubjects {
    pub(super) reporting:              WireOrderedReservationIds,
    pub(super) widening:               DriftWideningSelection,
    pub(super) post_write_first_touch: PostWriteFirstTouchRequirement,
}

#[derive(Clone, Copy)]
pub(super) enum PostWriteFirstTouchRequirement {
    NotRequired,
    Required,
}

pub(super) enum DriftWideningSelection {
    NotNeeded,
    Selected(ReservationId),
    Ambiguous(DriftAttributionCandidateSet),
}

/// A caller identity could not choose a safe drift subject.
#[derive(Debug)]
pub(super) enum DriftSelectionError {
    UnidentifiedActingRun,
    NoPostCommitCandidate,
    NoActiveReservation {
        run:      CoordinationRunId,
        worktree: WorktreeId,
    },
    AmbiguousActiveReservations(WireOrderedReservationIds),
    ExplicitNotActive {
        reservation_id: ReservationId,
        run:            CoordinationRunId,
        worktree:       WorktreeId,
    },
}

impl Display for DriftSelectionError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnidentifiedActingRun => formatter
                .write_str("drift requires a live session mapping, active coordination-run marker, or CARGO_BERTH_RUN"),
            Self::NoPostCommitCandidate => formatter
                .write_str("post-commit drift found no active reservation candidate"),
            Self::NoActiveReservation { run, worktree } => write!(
                formatter,
                "coordination run {run} has no active reservation in worktree {worktree}"
            ),
            Self::AmbiguousActiveReservations(candidates) => write!(
                formatter,
                "drift is ambiguous; choose one active reservation with --reservation: {}",
                candidates
                    .as_slice()
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::ExplicitNotActive {
                reservation_id,
                run,
                worktree,
            } => write!(
                formatter,
                "reservation {reservation_id} is not active for coordination run {run} in worktree {worktree}"
            ),
        }
    }
}

impl Error for DriftSelectionError {}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use serde_json::Value;
    use serde_json::json;

    use super::DriftReservationSelection;
    use super::PostCommitWideningSelection;
    use super::PostWriteFirstTouchRequirement;
    use crate::coordination_identity::PresentedCoordinationRun;
    use crate::drift::identity::DriftActingIdentity;
    use crate::ids::CoordinationRunId;
    use crate::ids::ReservationId;
    use crate::ids::WorktreeId;
    use crate::ledger::JournalEvent;
    use crate::reservation::RetainedReservationSet;
    use crate::reservation::WorktreeOccupancy;

    const HOLDER_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1e";
    const RESERVATION_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1f";
    const SECOND_RUN_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a20";
    const WORKTREE_ID: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1d";

    /// An empty post-commit reporting set is the same fact as "this worktree has no incumbent".
    ///
    /// `execute::execute_inner` has one branch that returns before the lock which decides
    /// refusal --- the post-write first touch taken when nothing is reported --- and states
    /// `DriftScopeAcquisition::Permitted` without asking the occupancy question. It may do that
    /// only because reaching it rules the incumbent out: post-commit selection drops the acting
    /// run from its filter and matches every `Active` reservation in the worktree, and occupancy
    /// requires an `Active` holder. So the two are the same test of the same worktree, and this
    /// pins them moving together --- an incumbent puts a reservation in `reporting`, and an
    /// empty `reporting` leaves the occupancy question nothing to find.
    ///
    /// Re-adding a run term to that filter would break the implication silently: the branch
    /// would keep compiling and start reporting `Permitted` for a run the rule refuses.
    #[test]
    fn an_empty_post_commit_reporting_set_means_this_worktree_has_no_incumbent() {
        let reservation_id = RESERVATION_ID
            .parse::<ReservationId>()
            .expect("reservation id should parse");
        let holder_run_id = HOLDER_RUN_ID
            .parse::<CoordinationRunId>()
            .expect("holder run id should parse");
        let second_run_id = SECOND_RUN_ID
            .parse::<CoordinationRunId>()
            .expect("second run id should parse");
        let worktree = WORKTREE_ID
            .parse::<WorktreeId>()
            .expect("worktree id should parse");
        let second_run = PresentedCoordinationRun::from_run_argument(second_run_id);
        let post_commit = DriftReservationSelection::EveryActiveForPostCommit {
            widening: PostCommitWideningSelection::SessionMappingOrSingleCandidate,
        };
        let acting = DriftActingIdentity::Run {
            run: second_run_id,
            worktree,
        };

        let occupied = RetainedReservationSet::replay(&[presented_claim()])
            .expect("the claim fixture should replay");
        let subjects = post_commit
            .resolve(&occupied, acting)
            .expect("post-commit selection should resolve against an occupied worktree");
        assert_eq!(subjects.reporting.as_slice(), [reservation_id]);
        assert!(matches!(
            subjects.post_write_first_touch,
            PostWriteFirstTouchRequirement::Required
        ));
        assert!(
            matches!(
                occupied.worktree_occupancy(worktree, second_run),
                WorktreeOccupancy::Incumbent(incumbent)
                    if incumbent.actor().run == holder_run_id
            ),
            "the claim's run should occupy the worktree against a second run"
        );

        let vacant = RetainedReservationSet::default();
        let subjects = post_commit
            .resolve(&vacant, acting)
            .expect("post-commit selection should resolve against a vacant worktree");
        assert!(subjects.reporting.as_slice().is_empty());
        assert!(matches!(
            subjects.post_write_first_touch,
            PostWriteFirstTouchRequirement::Required
        ));
        assert!(matches!(
            vacant.worktree_occupancy(worktree, second_run),
            WorktreeOccupancy::NoIncumbent
        ));
    }

    /// One `Active` claim in `WORKTREE_ID`, recorded under a presented identity.
    fn presented_claim() -> JournalEvent {
        let event: Value = json!({
            "schema_version": 2,
            "event_id": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1b",
            "actor": {
                "repository": "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a1c",
                "worktree": WORKTREE_ID,
                "run": HOLDER_RUN_ID,
            },
            "at": "2026-08-23T17:34:54.123Z",
            "projection_generation": 1,
            "op": "claim",
            "reservation_id": RESERVATION_ID,
            "scopes": [{"path": "src", "kind": "tree"}],
            "source": {"kind": "explicit"},
            "purpose": {"kind": "not_provided_by_caller"},
            "trunk_at_claim": "1111111111111111111111111111111111111111",
            "head_snapshot": {
                "kind": "branch",
                "full_ref": "refs/heads/phase",
                "head": "2222222222222222222222222222222222222222",
            },
            "phase_start_head": "1111111111111111111111111111111111111111",
            "worktree_root": "/repo",
            "worktree_administrative_locator": ".",
            "authorization": {"kind": "no_conflict"},
            "coordination_identity_provenance": "presented",
        });
        serde_json::from_value(event).expect("the claim fixture should deserialize")
    }
}
