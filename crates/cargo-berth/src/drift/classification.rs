//! Classification of observed paths into widening, incursion, and collision effects.

use std::collections::HashSet;

use super::observation::ObservedDriftChanges;
use super::ordering;
use super::report::DriftComparisonMode;
use super::report::DriftEffect;
use super::report::DriftEffectSet;
use super::report::DriftPathAttributionOutcome;
use super::report::DriftReport;
use super::report::ReservationDriftResult;
use super::report::UnattributedDriftPathSet;
use super::selection::DriftWideningSelection;
use super::selection::ResolvedDriftSubjects;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ledger::ClaimSource;
use crate::ledger::CollisionPathSet;
use crate::ledger::ForeignReservationIdSet;
use crate::ledger::IncursionPathSet;
use crate::ledger::JournalOperation;
use crate::ledger::ReservationScopeAdditionSet;
use crate::ledger::WidenCause;
use crate::reservation::DriftBlockingCoverage;
use crate::reservation::IncursionObservation;
use crate::reservation::Reservation;
use crate::reservation::ReservationReplayError;
use crate::reservation::RetainedReservationSet;
use crate::reservation::WidenScopeBinding;
use crate::scope::PathCase;
use crate::scope::ReservationScope;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

enum WideningAttempt {
    NotNeeded,
    Attributed,
}

pub(super) struct PriorClassification {
    foreign_paths: HashSet<(ReservationId, String)>,
}

impl PriorClassification {
    pub(super) fn build(
        reservations: &RetainedReservationSet,
        subject_ids: &[ReservationId],
        changes: &ObservedDriftChanges,
        path_case: PathCase,
    ) -> Result<Self, ReservationReplayError> {
        let mut foreign_paths = HashSet::new();
        for reservation_id in subject_ids {
            let reservation = reservations.reservation(*reservation_id)?;
            changes.visit_paths(*reservation_id, |path| {
                if reservation_covers_path(reservation, path, path_case) {
                    return;
                }
                match blocking_coverage(reservations, reservation, path, path_case) {
                    DriftBlockingCoverage::SameIdentity | DriftBlockingCoverage::Unclaimed => {},
                    DriftBlockingCoverage::Foreign(_) => {
                        foreign_paths.insert((*reservation_id, path.to_string()));
                    },
                }
            });
        }
        Ok(Self { foreign_paths })
    }

    fn was_foreign(&self, reservation_id: ReservationId, path: &ReservationScopePath) -> bool {
        self.foreign_paths
            .contains(&(reservation_id, path.to_string()))
    }
}

#[derive(Default)]
struct DriftEffectBuilder {
    widened_paths:          Vec<ReservationScopePath>,
    incursion_paths:        Vec<ReservationScopePath>,
    incursion_reservations: Vec<ReservationId>,
    collision_paths:        Vec<ReservationScopePath>,
    collision_reservations: Vec<ReservationId>,
}

impl DriftEffectBuilder {
    fn finish(
        mut self,
        reservations: &RetainedReservationSet,
        reservation: &Reservation,
        path_case: PathCase,
    ) -> (
        Vec<JournalOperation>,
        ReservationDriftResult,
        WideningAttempt,
    ) {
        let reservation_id = reservation.id();
        ordering::normalize_paths(&mut self.widened_paths);
        ordering::normalize_paths(&mut self.incursion_paths);
        ordering::normalize_paths(&mut self.collision_paths);
        ordering::sort_and_deduplicate_reservation_ids(&mut self.incursion_reservations);
        ordering::sort_and_deduplicate_reservation_ids(&mut self.collision_reservations);
        let mut operations = Vec::new();
        let mut effects = Vec::new();
        let widening_attempt = if self.widened_paths.is_empty() {
            WideningAttempt::NotNeeded
        } else {
            WideningAttempt::Attributed
        };
        if let Ok(added_scopes) = ReservationScopeAdditionSet::try_from(
            self.widened_paths
                .into_iter()
                .map(|path| ReservationScope {
                    path,
                    kind: ScopeKind::File,
                })
                .collect::<Vec<_>>(),
        ) {
            match reservations.bind_widened_scopes(reservation, &added_scopes, path_case) {
                WidenScopeBinding::Authorized(authorization) => {
                    operations.push(JournalOperation::Widen {
                        reservation_id,
                        added_scopes: added_scopes.clone(),
                        cause: WidenCause::Drift,
                        authorization,
                        edit_blocking_status: reservation.edit_blocking_status(),
                    });
                    effects.push(DriftEffect::Widened { added_scopes });
                },
                WidenScopeBinding::Blocked(conflicts) => {
                    self.collision_paths.extend(
                        added_scopes
                            .as_slice()
                            .iter()
                            .map(|scope| scope.path.clone()),
                    );
                    self.collision_reservations
                        .extend(conflicts.iter().map(|conflict| conflict.reservation_id));
                    ordering::normalize_paths(&mut self.collision_paths);
                    ordering::sort_and_deduplicate_reservation_ids(
                        &mut self.collision_reservations,
                    );
                },
            }
        }
        if let (Ok(foreign_reservation_ids), Ok(paths)) = (
            ForeignReservationIdSet::try_from(self.incursion_reservations),
            IncursionPathSet::try_from(self.incursion_paths),
        ) {
            let reportable = match reservations.observe_incursion(
                reservation_id,
                &foreign_reservation_ids,
                &paths,
            ) {
                IncursionObservation::AlreadyAnswered => None,
                IncursionObservation::AlreadyOutstanding { incident_id, paths } => {
                    Some((incident_id, paths))
                },
                IncursionObservation::NewlyObserved { incident_id, paths } => {
                    operations.push(JournalOperation::Incursion {
                        incident_id,
                        reservation_id,
                        foreign_reservation_ids: foreign_reservation_ids.clone(),
                        paths: paths.clone(),
                    });
                    Some((incident_id, paths))
                },
            };
            if let Some((incident_id, paths)) = reportable {
                effects.push(DriftEffect::Incursion {
                    incident_id,
                    foreign_reservation_ids,
                    paths,
                });
            }
        }
        if let (Ok(foreign_reservation_ids), Ok(paths)) = (
            ForeignReservationIdSet::try_from(self.collision_reservations),
            CollisionPathSet::try_from(self.collision_paths),
        ) {
            effects.push(DriftEffect::Collision {
                foreign_reservation_ids,
                paths,
            });
        }
        let result = DriftEffectSet::try_from(effects).map_or(
            ReservationDriftResult::Unchanged { reservation_id },
            |effects| ReservationDriftResult::Changed {
                reservation_id,
                effects,
            },
        );
        (operations, result, widening_attempt)
    }
}

pub(super) struct DriftTransactionDecision {
    pub(super) operations: Vec<JournalOperation>,
    pub(super) report:     DriftReport,
}

pub(super) fn classify_locked(
    reservations: &RetainedReservationSet,
    subjects: &ResolvedDriftSubjects,
    changes: &ObservedDriftChanges,
    prior: &PriorClassification,
    path_case: PathCase,
    comparison: DriftComparisonMode,
) -> Result<DriftTransactionDecision, ReservationReplayError> {
    let mut operations = Vec::new();
    let mut results = Vec::new();
    let mut unattributed_paths = Vec::new();
    let mut widening_attempt = WideningAttempt::NotNeeded;
    for reservation_id in &subjects.reporting {
        let reservation = reservations.reservation(*reservation_id)?;
        let mut builder = DriftEffectBuilder::default();
        changes.visit_paths(*reservation_id, |path| {
            if reservation_covers_path(reservation, path, path_case) {
                return;
            }
            match blocking_coverage(reservations, reservation, path, path_case) {
                DriftBlockingCoverage::SameIdentity => {},
                DriftBlockingCoverage::Unclaimed => match &subjects.widening {
                    DriftWideningSelection::Selected(selected) if selected == reservation_id => {
                        builder.widened_paths.push(path.clone());
                    },
                    DriftWideningSelection::Ambiguous(_) => {
                        unattributed_paths.push(path.clone());
                    },
                    DriftWideningSelection::NotNeeded | DriftWideningSelection::Selected(_) => {},
                },
                DriftBlockingCoverage::Foreign(conflicts) => {
                    let blockers = conflicts
                        .iter()
                        .map(|conflict| conflict.reservation_id)
                        .filter(|blocker| match &subjects.widening {
                            DriftWideningSelection::Selected(acting_reservation_id) => {
                                *acting_reservation_id != *blocker
                                    || *acting_reservation_id == *reservation_id
                            },
                            DriftWideningSelection::NotNeeded
                            | DriftWideningSelection::Ambiguous(_) => true,
                        })
                        .collect::<Vec<_>>();
                    if blockers.is_empty()
                        || matches!(reservation.source(), ClaimSource::FirstTouch)
                            && outstanding_incursion_covers(
                                reservations,
                                &subjects.reporting,
                                *reservation_id,
                                path,
                                &blockers,
                            )
                    {
                        return;
                    }
                    if prior.was_foreign(*reservation_id, path) {
                        builder.incursion_paths.push(path.clone());
                        builder.incursion_reservations.extend(blockers);
                    } else {
                        builder.collision_paths.push(path.clone());
                        builder.collision_reservations.extend(blockers);
                    }
                },
            }
        });
        let (mut subject_operations, result, subject_widening_attempt) =
            builder.finish(reservations, reservation, path_case);
        if matches!(subject_widening_attempt, WideningAttempt::Attributed) {
            widening_attempt = WideningAttempt::Attributed;
        }
        operations.append(&mut subject_operations);
        results.push(result);
    }
    ordering::normalize_paths(&mut unattributed_paths);
    let path_attribution = match (&subjects.widening, widening_attempt) {
        (DriftWideningSelection::Selected(reservation_id), WideningAttempt::Attributed) => {
            DriftPathAttributionOutcome::Attributed {
                reservation_id: *reservation_id,
            }
        },
        (DriftWideningSelection::Ambiguous(candidates), _) => {
            UnattributedDriftPathSet::try_from(unattributed_paths).map_or(
                DriftPathAttributionOutcome::NotNeeded,
                |paths| DriftPathAttributionOutcome::Ambiguous {
                    candidates: candidates.clone(),
                    paths,
                },
            )
        },
        (DriftWideningSelection::NotNeeded, _)
        | (DriftWideningSelection::Selected(_), WideningAttempt::NotNeeded) => {
            DriftPathAttributionOutcome::NotNeeded
        },
    };
    Ok(DriftTransactionDecision {
        operations,
        report: DriftReport {
            comparison,
            path_attribution,
            results,
        },
    })
}

fn outstanding_incursion_covers(
    reservations: &RetainedReservationSet,
    reporting: &[ReservationId],
    current_reservation_id: ReservationId,
    path: &ReservationScopePath,
    blockers: &[ReservationId],
) -> bool {
    reservations
        .outstanding_incursion_incidents()
        .any(|incident| {
            incident.reservation_id() != current_reservation_id
                && reporting.contains(&incident.reservation_id())
                && incident.paths().as_slice().contains(path)
                && blockers.iter().all(|blocker| {
                    incident
                        .foreign_reservation_ids()
                        .as_slice()
                        .contains(blocker)
                })
        })
}

fn reservation_covers_path(
    reservation: &Reservation,
    path: &ReservationScopePath,
    path_case: PathCase,
) -> bool {
    let candidate = ReservationScope {
        path: path.clone(),
        kind: ScopeKind::File,
    };
    reservation
        .scopes()
        .as_slice()
        .iter()
        .any(|scope| scope.contains(&candidate, path_case))
}

fn blocking_coverage(
    reservations: &RetainedReservationSet,
    subject: &Reservation,
    path: &ReservationScopePath,
    path_case: PathCase,
) -> DriftBlockingCoverage {
    let Ok(candidate) = ReservationScopeSet::try_from(vec![ReservationScope {
        path: path.clone(),
        kind: ScopeKind::File,
    }]) else {
        return DriftBlockingCoverage::Unclaimed;
    };
    reservations.blocking_coverage_for_drift(&candidate, subject.actor().worktree, path_case)
}
