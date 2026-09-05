//! Classification of observed paths into widening, incursion, and collision effects.

use super::identity::DriftScopeAcquisition;
use super::identity::DriftWideningAuthorization;
use super::observation::ObservedDriftChanges;
use super::observation::ReservationPhaseHistory;
use super::ordering;
use super::provenance;
use super::provenance::IncursionAttributionBatch;
use super::report::DriftComparisonMode;
use super::report::DriftEffect;
use super::report::DriftEffectSet;
use super::report::DriftPathAttributionOutcome;
use super::report::DriftReport;
use super::report::PostWriteFreePathProtection;
use super::report::ReservationDriftResult;
use super::report::UnattributedDriftPathSet;
use super::selection::DriftWideningSelection;
use super::selection::ResolvedDriftSubjects;
use crate::coordination_identity::IssuingWorktreeRun;
use crate::ids::CommitterTime;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WireOrderedReservationIds;
use crate::ledger::BlockedIncursionPath;
use crate::ledger::BlockedIncursionPathSet;
use crate::ledger::ClaimSource;
use crate::ledger::CollisionPathSet;
use crate::ledger::ForeignReservationIdSet;
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

/// One observed path the ledger's present holders would call foreign.
struct ForeignPathCandidate {
    reservation_id: ReservationId,
    path:           ReservationScopePath,
    /// The path reached the observation only through the subject's phase range.
    committed_only: bool,
}

/// What the pre-lock pass established about each subject's observed paths.
///
/// Two things are settled here and read again under the lock. Which paths already had a
/// foreign holder when the invocation began, which separates an incursion from a
/// collision; and, for the paths a phase committed into such a holder's scope, when each
/// was last committed — read from git once, before the lock, so the locked pass asks the
/// ledger nothing it cannot answer from memory.
pub(super) struct PreLockForeignPathClassification {
    candidates:        Vec<ForeignPathCandidate>,
    committed_history: Option<IncursionAttributionBatch>,
}

impl PreLockForeignPathClassification {
    pub(super) fn build(
        reservations: &RetainedReservationSet,
        subject_ids: &[ReservationId],
        changes: &ObservedDriftChanges,
        path_case: PathCase,
    ) -> Result<Self, ReservationReplayError> {
        let mut candidates = Vec::new();
        for reservation_id in subject_ids {
            let reservation = reservations.reservation(*reservation_id)?;
            changes.visit_paths(*reservation_id, |path| {
                if reservation_covers_path(reservation, path, path_case) {
                    return;
                }
                match blocking_coverage(reservations, reservation, None, path, path_case) {
                    DriftBlockingCoverage::NoForeignStanding | DriftBlockingCoverage::Unclaimed => {
                    },
                    DriftBlockingCoverage::Foreign(_) => {
                        candidates.push(ForeignPathCandidate {
                            reservation_id: *reservation_id,
                            path:           path.clone(),
                            committed_only: changes.committed_only(*reservation_id, path),
                        });
                    },
                }
            });
        }
        Ok(Self {
            candidates,
            committed_history: None,
        })
    }

    /// The paths a phase committed into a present foreign holder's scope, by subject.
    pub(super) fn committed_foreign_paths(&self) -> Vec<(ReservationId, ReservationScopePath)> {
        self.candidates
            .iter()
            .filter(|candidate| candidate.committed_only)
            .map(|candidate| (candidate.reservation_id, candidate.path.clone()))
            .collect()
    }

    /// Attach the commits read for [`Self::committed_foreign_paths`].
    pub(super) fn with_committed_history(
        mut self,
        committed_history: Option<IncursionAttributionBatch>,
    ) -> Self {
        self.committed_history = committed_history;
        self
    }

    pub(super) const fn committed_history(&self) -> Option<&IncursionAttributionBatch> {
        self.committed_history.as_ref()
    }

    /// Which holders block the path, as of the moment the subject wrote it.
    fn blocking_coverage(
        &self,
        reservations: &RetainedReservationSet,
        subject: &Reservation,
        path: &ReservationScopePath,
        path_case: PathCase,
    ) -> DriftBlockingCoverage {
        blocking_coverage(
            reservations,
            subject,
            self.written_at(subject, path),
            path,
            path_case,
        )
    }

    /// When the subject last committed to a path it reached only through its phase range.
    fn written_at(
        &self,
        subject: &Reservation,
        path: &ReservationScopePath,
    ) -> Option<CommitterTime> {
        let batch = self.committed_history.as_ref()?;
        self.candidates
            .iter()
            .find(|candidate| {
                candidate.committed_only
                    && candidate.reservation_id == subject.id()
                    && candidate.path == *path
            })
            .and_then(|_| {
                provenance::latest_commit(batch, subject.phase_start_head().as_ref(), path)
            })
    }

    /// Whether a present foreign holder stood on the path when the invocation began.
    fn was_foreign(&self, reservation_id: ReservationId, path: &ReservationScopePath) -> bool {
        self.candidates
            .iter()
            .any(|candidate| candidate.reservation_id == reservation_id && candidate.path == *path)
    }
}

#[derive(Default)]
struct DriftEffectBuilder {
    widened_paths:          Vec<ReservationScopePath>,
    incursions:             Vec<(ReservationScopePath, Vec<ReservationId>)>,
    collision_paths:        Vec<ReservationScopePath>,
    collision_reservations: Vec<ReservationId>,
}

/// Pair every entered path with the holders that block it, in path order.
///
/// Incursion coverage is decided one path at a time, so an observation must carry the
/// holders that actually block each path. A path observed more than once keeps its
/// first holders; a path no holder blocks is not an incursion and is dropped.
fn pair_incursions_with_holders(
    incursions: Vec<(ReservationScopePath, Vec<ReservationId>)>,
) -> Vec<BlockedIncursionPath> {
    let mut blocked_paths: Vec<BlockedIncursionPath> = Vec::new();
    for (path, holders) in incursions {
        if blocked_paths.iter().any(|blocked| blocked.path == path) {
            continue;
        }
        let Ok(holders) = ForeignReservationIdSet::try_from(
            WireOrderedReservationIds::sorted_and_deduplicated(holders).into_vec(),
        ) else {
            continue;
        };
        blocked_paths.push(BlockedIncursionPath { path, holders });
    }
    blocked_paths.sort_by_cached_key(|blocked| blocked.path.to_string());
    blocked_paths
}

/// Gather blocked paths into one observation per distinct set of blocking holders.
///
/// Each observation becomes its own incident, and an incident is answered as one, so
/// paths held by different reservations are kept apart: answering the overlap with one
/// holder must not silently answer an overlap with another. Groups keep the order their
/// first path arrived in, which is path order.
fn group_blocked_paths_by_holders(
    blocked_paths: Vec<BlockedIncursionPath>,
) -> Vec<BlockedIncursionPathSet> {
    let mut groups: Vec<Vec<BlockedIncursionPath>> = Vec::new();
    for blocked in blocked_paths {
        match groups.iter_mut().find(|group| {
            group
                .first()
                .is_some_and(|first| first.holders == blocked.holders)
        }) {
            Some(group) => group.push(blocked),
            None => groups.push(vec![blocked]),
        }
    }
    groups
        .into_iter()
        .filter_map(|group| BlockedIncursionPathSet::try_from(group).ok())
        .collect()
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
        ordering::normalize_paths(&mut self.collision_paths);
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
                },
            }
        }
        for blocked_paths in
            group_blocked_paths_by_holders(pair_incursions_with_holders(self.incursions))
        {
            let reportable = match reservations.observe_incursion(reservation_id, &blocked_paths) {
                IncursionObservation::AlreadyAnswered => None,
                IncursionObservation::AlreadyOutstanding {
                    incident_id,
                    blocked_paths,
                } => Some((incident_id, blocked_paths)),
                IncursionObservation::NewlyObserved {
                    incident_id,
                    blocked_paths,
                } => {
                    operations.push(JournalOperation::Incursion {
                        incident_id,
                        reservation_id,
                        blocked_paths: blocked_paths.clone().into(),
                    });
                    Some((incident_id, blocked_paths))
                },
            };
            if let Some((incident_id, blocked_paths)) = reportable {
                effects.push(DriftEffect::Incursion {
                    incident_id,
                    foreign_reservation_ids: blocked_paths.holders(),
                    paths: blocked_paths.paths(),
                    commits: Vec::new(),
                });
            }
        }
        if let (Ok(foreign_reservation_ids), Ok(paths)) = (
            ForeignReservationIdSet::try_from(
                WireOrderedReservationIds::sorted_and_deduplicated(self.collision_reservations)
                    .into_vec(),
            ),
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
    prior: &PreLockForeignPathClassification,
    path_case: PathCase,
    comparison: DriftComparisonMode,
    widening_authorization: DriftWideningAuthorization,
) -> Result<DriftTransactionDecision, ReservationReplayError> {
    let mut operations = Vec::new();
    let mut results = Vec::new();
    let mut unattributed_paths = Vec::new();
    let mut widening_attempt = WideningAttempt::NotNeeded;
    for reservation_id in subjects.reporting.as_slice() {
        if let ReservationPhaseHistory::PhaseStartObjectUnknown(phase_start) =
            changes.reservation_phase_history(*reservation_id)
        {
            results.push(ReservationDriftResult::PhaseStartObjectUnknown {
                reservation_id: *reservation_id,
                phase_start:    phase_start.clone(),
            });
            continue;
        }
        let reservation = reservations.reservation(*reservation_id)?;
        let mut builder = DriftEffectBuilder::default();
        changes.visit_paths(*reservation_id, |path| {
            if reservation_covers_path(reservation, path, path_case) {
                return;
            }
            match prior.blocking_coverage(reservations, reservation, path, path_case) {
                DriftBlockingCoverage::NoForeignStanding => {},
                DriftBlockingCoverage::Unclaimed => {
                    if !changes.carries_work(path) {
                        return;
                    }
                    match widening_authorization {
                        DriftWideningAuthorization::WithheldFromRefusedRun => {},
                        DriftWideningAuthorization::Permitted => match &subjects.widening {
                            DriftWideningSelection::Selected(selected)
                                if selected == reservation_id =>
                            {
                                builder.widened_paths.push(path.clone());
                            },
                            DriftWideningSelection::Ambiguous(_) => {
                                unattributed_paths.push(path.clone());
                            },
                            DriftWideningSelection::NotNeeded
                            | DriftWideningSelection::Selected(_) => {},
                        },
                    }
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
                                subjects.reporting.as_slice(),
                                *reservation_id,
                                path,
                                &blockers,
                            )
                    {
                        return;
                    }
                    if prior.was_foreign(*reservation_id, path) {
                        builder.incursions.push((path.clone(), blockers));
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
    let path_attribution =
        attribute_paths(&subjects.widening, widening_attempt, unattributed_paths);
    Ok(DriftTransactionDecision {
        operations,
        report: DriftReport {
            comparison,
            path_attribution,
            results,
            scope_acquisition: DriftScopeAcquisition::Permitted,
        },
    })
}

/// What a refused second run's observed writes entered in the worktree it stands in.
pub(super) enum RefusedRunPathEntry {
    /// No holder protects any path this observation attributed to the refused run.
    NoHolderEntered,
    /// The refused run's observed writes entered paths these holders already protect.
    HoldersEntered(DriftPathAttributionOutcome),
}

/// Attribute a refused run's observed writes to every holder whose scopes they entered.
///
/// The refusal withholds acquisition, so this asks the acquisition question's *other* half
/// without taking anything: which holders would have blocked a first touch of these paths.
/// The predicate is [`RetainedReservationSet::conflicts_for_first_touch`], the same one
/// [`crate::verb::claim::acquire_first_touch`] asks under its own lock, so the reported shape
/// matches an unrefused post-write detection exactly — minus the reservation, which is what
/// was refused.
///
/// The incumbent occupying this worktree is a holder like any other here. Subject
/// classification cannot report that entry: the incumbent is its own subject, and a
/// reservation never enters its own scopes.
///
/// Only paths this invocation can attribute to the refused run are asked about, which is what
/// [`ObservedDriftChanges::acting_run_attributable_paths`] names. A reporting subject's
/// committed range runs from that subject's own phase start, so the incumbent's earlier
/// commits sit inside it the moment the refused run commits onto the same branch: asking about
/// that range reported the incumbent's own writes back to the refused run as an incursion it
/// had committed, and stopped it on them.
pub(super) fn attribute_refused_run_entry(
    reservations: &RetainedReservationSet,
    changes: &ObservedDriftChanges,
    refused: IssuingWorktreeRun,
    path_case: PathCase,
) -> RefusedRunPathEntry {
    let observed_paths = changes
        .acting_run_attributable_paths()
        .into_iter()
        .filter(|path| changes.carries_work(path))
        .collect::<Vec<_>>();
    let Ok(candidate) = ReservationScopeSet::try_from(
        observed_paths
            .iter()
            .map(|path| ReservationScope {
                path: path.clone(),
                kind: ScopeKind::File,
            })
            .collect::<Vec<_>>(),
    ) else {
        return RefusedRunPathEntry::NoHolderEntered;
    };
    let conflicts = reservations.conflicts_for_first_touch(
        &candidate,
        refused.coordination_run_id,
        refused.worktree_id,
        path_case,
    );
    let entered = observed_paths
        .into_iter()
        .filter(|path| {
            let entered_scope = ReservationScope {
                path: path.clone(),
                kind: ScopeKind::File,
            };
            conflicts.iter().any(|conflict| {
                conflict
                    .overlapping_scopes
                    .as_slice()
                    .iter()
                    .any(|held| held.overlaps(&entered_scope, path_case))
            })
        })
        .collect::<Vec<_>>();
    UnattributedDriftPathSet::try_from(entered).map_or(
        RefusedRunPathEntry::NoHolderEntered,
        |paths| {
            RefusedRunPathEntry::HoldersEntered(DriftPathAttributionOutcome::IncursionDetected {
                paths,
                conflicts,
                protection: PostWriteFreePathProtection::NotAcquired,
            })
        },
    )
}

/// Name who the widening belongs to, or why nobody could be named.
fn attribute_paths(
    widening: &DriftWideningSelection,
    attempt: WideningAttempt,
    unattributed_paths: Vec<ReservationScopePath>,
) -> DriftPathAttributionOutcome {
    match (widening, attempt) {
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
    }
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
                && incident
                    .blocked_paths()
                    .holders_of(path)
                    .is_some_and(|recorded| {
                        blockers
                            .iter()
                            .all(|blocker| recorded.as_slice().contains(blocker))
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

/// Which holders block one observed path, as of the moment the subject wrote it.
///
/// The ledger answers for the present. A path that reached the observation only through
/// a commit was written when that commit was made — `written_at` — and a holder whose
/// claim postdates that commit was not entered by it: the claim that "entered" it did not
/// exist yet. Asking the present instead let any new claim over a long-lived shared file
/// manufacture an incursion against every reservation that had ever committed to it, and
/// stop the operator on an overlap that never happened. A path the working tree still
/// holds open was written just now, so it arrives with no `written_at` and the present is
/// the right moment.
///
/// A holder's `claimed_at` stands in for when it began covering the path. A scope widened
/// onto later is dated by the claim, earlier than it should be, so such a holder is kept
/// rather than dropped: the error is on the reporting side.
fn blocking_coverage(
    reservations: &RetainedReservationSet,
    subject: &Reservation,
    written_at: Option<CommitterTime>,
    path: &ReservationScopePath,
    path_case: PathCase,
) -> DriftBlockingCoverage {
    let Ok(candidate) = ReservationScopeSet::try_from(vec![ReservationScope {
        path: path.clone(),
        kind: ScopeKind::File,
    }]) else {
        return DriftBlockingCoverage::Unclaimed;
    };
    let coverage = reservations.blocking_coverage_for_drift(
        &candidate,
        subject.actor().worktree,
        subject.actor().run,
        path_case,
    );
    let DriftBlockingCoverage::Foreign(conflicts) = coverage else {
        return coverage;
    };
    let Some(committed_at) = written_at else {
        return DriftBlockingCoverage::Foreign(conflicts);
    };
    let in_force_when_written = conflicts
        .into_iter()
        .filter(|conflict| !conflict.claimed_at().follows(committed_at))
        .collect::<Vec<_>>();
    if in_force_when_written.is_empty() {
        DriftBlockingCoverage::NoForeignStanding
    } else {
        DriftBlockingCoverage::Foreign(in_force_when_written)
    }
}

#[cfg(test)]
mod tests {
    use super::pair_incursions_with_holders;
    use crate::ids::ReservationId;

    const FIRST_HOLDER: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a22";
    const SECOND_HOLDER: &str = "01900a1b-2c3d-7e4f-8a5b-6c7d8e9f0a24";

    /// Every entered path keeps the holders that block it, and only those.
    ///
    /// Reporting paths under the union of their holders forced one incident to carry
    /// holders no single path exhibited, which stopped an answered path from matching
    /// its own answer as soon as an unrelated path added a holder.
    #[test]
    fn each_path_keeps_its_own_holders_in_path_order() -> Result<(), Box<dyn std::error::Error>> {
        let first = FIRST_HOLDER.parse::<ReservationId>()?;
        let second = SECOND_HOLDER.parse::<ReservationId>()?;
        let blocked = pair_incursions_with_holders(vec![
            ("src/other.rs".parse()?, vec![second, first, second]),
            ("src/lib.rs".parse()?, vec![first]),
        ]);

        let reported = blocked
            .iter()
            .map(|blocked| {
                (
                    blocked.path.to_string(),
                    blocked
                        .holders
                        .as_slice()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            reported,
            vec![
                ("src/lib.rs".to_owned(), vec![FIRST_HOLDER.to_owned()]),
                (
                    "src/other.rs".to_owned(),
                    vec![FIRST_HOLDER.to_owned(), SECOND_HOLDER.to_owned()]
                ),
            ],
            "paths are ordered regardless of entry order, and holders are ordered and distinct"
        );
        Ok(())
    }

    /// A path with no blocking holder is not an incursion, and a repeated path is one.
    #[test]
    fn unblocked_and_repeated_paths_collapse() -> Result<(), Box<dyn std::error::Error>> {
        let first = FIRST_HOLDER.parse::<ReservationId>()?;
        let second = SECOND_HOLDER.parse::<ReservationId>()?;
        let blocked = pair_incursions_with_holders(vec![
            ("src/lib.rs".parse()?, vec![first]),
            ("src/lib.rs".parse()?, vec![second]),
            ("src/free.rs".parse()?, Vec::new()),
        ]);

        assert_eq!(blocked.len(), 1);
        assert_eq!(blocked[0].path.to_string(), "src/lib.rs");
        assert_eq!(blocked[0].holders.as_slice(), [first]);
        Ok(())
    }
}
