//! Alert, bypass-audit, incursion and git-cost sections of the reservation board.

use std::collections::HashMap;
use std::collections::HashSet;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::error::BoardError;
use super::rows::BoardReservationSnapshot;
use super::rows::BoardReservationVisibility;
use crate::alert::Alert;
use crate::alert::BranchRefStatus;
use crate::alert::LostEvidenceRecovery;
use crate::alert::LostIntegrationEvidenceStatus;
use crate::alert::ObjectAvailability;
use crate::alert::RecoverabilityVerdict;
use crate::alert::RetentionRefStatus;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::RepositoryReservationEvidence;
use crate::edge::RepositorySnapshot;
use crate::gate::permit;
use crate::ids::EventId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ledger::BypassCause;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::BypassedMergeIdentity;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::FullRefName;
use crate::ledger::IncursionIncidentId;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::SkippedDeferral;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::SkippedOrderingEdge;
use crate::presentation;
use crate::presentation::IncursionResolutionGuidance;
use crate::reconcile::ReconciliationGitCost;
use crate::reservation::IncursionIncidentStatus;
use crate::reservation::ProtectedReservationTip;
use crate::reservation::ReleaseDisposition;
use crate::reservation::ReservationFreshness;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;
use crate::worktree::WorktreeHead;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct AvailableForcedPermit {
    permit_id:      ForcedIntegrationPermitId,
    reservation_id: ReservationId,
    reason:         ForcedIntegrationReason,
    skipped_holds:  SkippedIntegrationHoldSet,
    instruction:    String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum BypassAuditEntry {
    ForcedOrderingEdges {
        permit_id:     ForcedIntegrationPermitId,
        reason:        ForcedIntegrationReason,
        skipped_edges: Vec<SkippedOrderingEdge>,
        occurrence:    BoardBypassTime,
    },
    ForcedUnresolvedDeferrals {
        permit_id:         ForcedIntegrationPermitId,
        reason:            ForcedIntegrationReason,
        skipped_deferrals: Vec<SkippedDeferral>,
        occurrence:        BoardBypassTime,
    },
    ForcedEdgesAndDeferrals {
        permit_id:         ForcedIntegrationPermitId,
        reason:            ForcedIntegrationReason,
        skipped_edges:     Vec<SkippedOrderingEdge>,
        skipped_deferrals: Vec<SkippedDeferral>,
        occurrence:        BoardBypassTime,
    },
    EnvironmentOverride {
        override_name:                  String,
        occurrences:                    Vec<BoardBypassTime>,
        grouped_reference_transactions: u64,
        skipped_holds:                  UnrecordedSkippedHolds,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum BoardBypassTime {
    Known { at: RecordedAt },
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UnrecordedSkippedHolds {
    OverridePrecededLedgerRead,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct OutstandingIncursion {
    pub(super) incident_id:  IncursionIncidentId,
    straying_reservation_id: ReservationId,
    foreign_reservation_ids: Vec<ReservationId>,
    entered_paths:           Vec<ReservationScopePath>,
    /// How many incidents stand outstanding for the straying reservation, this one included.
    ///
    /// A notice naming one incident reads as though answering it ends the matter, and a
    /// backlog accumulated before the dedup landed stays invisible without this.
    outstanding_count:       usize,
    resolution:              IncursionResolutionAction,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
struct IncursionResolutionAction {
    reservation_id: ReservationId,
    incident_id:    IncursionIncidentId,
    flag:           String,
    /// The disposition that clears the reservation's whole outstanding set.
    every_flag:     String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct RecordedIncursionAnswer {
    pub(super) incident_id:  IncursionIncidentId,
    straying_reservation_id: ReservationId,
    foreign_reservation_ids: Vec<ReservationId>,
    entered_paths:           Vec<ReservationScopePath>,
    resolution_event_id:     EventId,
    resolved_at:             RecordedAt,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum BoardAlert {
    LostIntegrationEvidence {
        reservation_id:  ReservationId,
        protected_tip:   ProtectedReservationTip,
        evidence_status: LostIntegrationEvidenceStatus,
        recovery:        LostEvidenceRecovery,
    },
    OrphanedOutstanding {
        reservation_id:       ReservationId,
        protected_tip:        ProtectedReservationTip,
        branch:               BoardBranchRefStatus,
        object_availability:  ObjectAvailability,
        retention_ref:        BoardRetentionRefStatus,
        recoverability:       RecoverabilityVerdict,
        recovery_consequence: OrphanRecoveryConsequence,
        resolution:           OrphanResolutionAction,
    },
    StaleReservation {
        reservation_id: ReservationId,
        freshness:      ReservationFreshness,
        resolution:     StaleReservationResolutionAction,
    },
    UnrecordedBypasses {
        count:            u64,
        occurrence_times: Vec<BoardBypassTime>,
        instruction:      String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum BoardBranchRefStatus {
    Present {
        reference: FullRefName,
        tip:       GitObjectId,
    },
    Missing {
        reference: FullRefName,
    },
    Detached,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(transparent)]
pub(super) struct ReservationRetentionRef(#[schemars(length(min = 1))] String);

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum BoardRetentionRefStatus {
    Present {
        reference: ReservationRetentionRef,
    },
    Missing {
        reference: ReservationRetentionRef,
    },
    Mismatched {
        reference: ReservationRetentionRef,
        actual:    GitObjectId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum OrphanResolutionAction {
    Recover { flag: String },
    RetireOrAbandon { flags: Vec<String> },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum StaleReservationResolutionAction {
    Renew { reservation_id: ReservationId },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OrphanRecoveryConsequence {
    WorkRecoverable,
    CommitsLost,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(super) struct BoardGitCost {
    trunk_resolution_calls:                 u64,
    worktree_list_calls:                    u64,
    reservation_evidence_revalidations:     u64,
    protected_predecessor_ancestry_queries: u64,
    worktree_ahead_behind_computations:     u64,
    orphan_recovery_evidence_queries:       u64,
}

pub(super) fn outstanding_incursion_detail(incursion: &OutstandingIncursion) -> String {
    let entered_paths = incursion
        .entered_paths
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let foreign_reservation_ids = incursion
        .foreign_reservation_ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    presentation::outstanding_board_incursion_block(
        &incursion.straying_reservation_id.to_string(),
        &entered_paths,
        &foreign_reservation_ids,
        &incursion.incident_id.to_string(),
        &IncursionResolutionGuidance {
            outstanding_count: incursion.outstanding_count,
            incident_action:   &incursion.resolution.flag,
            every_action:      &incursion.resolution.every_flag,
        },
    )
}

pub(super) fn board_alert_detail(alert: &BoardAlert) -> String {
    match alert {
        BoardAlert::LostIntegrationEvidence {
            reservation_id,
            protected_tip,
            recovery,
            ..
        } => lost_integration_evidence_detail(*reservation_id, protected_tip, recovery),
        BoardAlert::OrphanedOutstanding {
            reservation_id,
            protected_tip,
            recoverability,
            resolution,
            ..
        } => {
            orphaned_outstanding_detail(*reservation_id, protected_tip, *recoverability, resolution)
        },
        BoardAlert::StaleReservation {
            reservation_id,
            freshness,
            ..
        } => format!(
            "STALE RESERVATION: reservation {reservation_id} has freshness {freshness:?}. Renew it with `cargo-berth renew {reservation_id}` after confirming its work is active."
        ),
        BoardAlert::UnrecordedBypasses {
            count, instruction, ..
        } => format!(
            "UNRECORDED BYPASSES: {count} bypass occurrence(s) still await durable journal audit; {instruction}."
        ),
    }
}

fn lost_integration_evidence_detail(
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
    recovery: &LostEvidenceRecovery,
) -> String {
    match recovery {
        LostEvidenceRecovery::VerifyResolvedTrunk { trunk_oid, .. } => format!(
            "INTEGRATION EVIDENCE LOST: released reservation {reservation_id} remains non-blocking, but trunk {trunk_oid} no longer proves protected tip {protected_tip}. If trunk {trunk_oid} contains the released work, run `cargo-berth resolve {reservation_id} --integrated-as {trunk_oid}`. Otherwise restore the work first. Inspect `cargo-berth board --json`."
        ),
        LostEvidenceRecovery::ResolveTrunkFirst { .. } => format!(
            "INTEGRATION EVIDENCE LOST: released reservation {reservation_id} remains non-blocking, and trunk does not currently resolve to a known object, so protected tip {protected_tip} cannot be proved either way. Resolve trunk first, then rerun. Inspect `cargo-berth board --json`."
        ),
    }
}

fn orphaned_outstanding_detail(
    reservation_id: ReservationId,
    protected_tip: &ProtectedReservationTip,
    recoverability: RecoverabilityVerdict,
    resolution: &OrphanResolutionAction,
) -> String {
    let recoverability = match recoverability {
        RecoverabilityVerdict::RecoverableFromBranch => "recoverable_from_branch",
        RecoverabilityVerdict::RecoverableFromProtectedTip => "recoverable_from_protected_tip",
        RecoverabilityVerdict::CommitUnavailable => "commit_unavailable",
    };
    let recovery_commands = match resolution {
        OrphanResolutionAction::Recover { flag } => {
            vec![reservation_resolution_command(flag, reservation_id)]
        },
        OrphanResolutionAction::RetireOrAbandon { flags } => flags
            .iter()
            .map(|flag| reservation_resolution_command(flag, reservation_id))
            .collect(),
    };
    presentation::orphaned_outstanding_block(
        &reservation_id.to_string(),
        &protected_tip.to_string(),
        recoverability,
        &recovery_commands,
    )
}

fn reservation_resolution_command(flag: &str, reservation_id: ReservationId) -> String {
    flag.strip_prefix("resolve").map_or_else(
        || format!("resolve {reservation_id} {flag}"),
        |arguments| format!("resolve {reservation_id}{arguments}"),
    )
}

pub(super) fn available_forced_permits(
    events: &[JournalEvent],
) -> Result<Vec<AvailableForcedPermit>, BoardError> {
    permit::available_forced_integration_permits(events)
        .map_err(BoardError::ForcedPermitReplay)
        .map(|permits| {
            permits
                .into_iter()
                .map(|permit| AvailableForcedPermit {
                    permit_id:      permit.permit_id,
                    reservation_id: permit.reservation_id,
                    reason:         permit.reason,
                    skipped_holds:  permit.skipped_holds,
                    instruction:    "retrying the integration will consume this permit".to_owned(),
                })
                .collect()
        })
}

pub(super) fn bypass_audit(events: &[JournalEvent]) -> Vec<BypassAuditEntry> {
    let permits = events
        .iter()
        .filter_map(|event| match &event.operation {
            JournalOperation::ForcedIntegrationPermit {
                permit_id,
                skipped_holds,
                ..
            } => Some((*permit_id, skipped_holds.clone())),
            _ => None,
        })
        .collect::<HashMap<_, _>>();
    let mut forced_seen = HashSet::new();
    let mut environment_groups: HashMap<BypassedMergeIdentity, Vec<BoardBypassTime>> =
        HashMap::new();
    let mut audit = Vec::new();
    for event in events {
        let JournalOperation::Bypass {
            cause,
            occurrence_time,
            ..
        } = &event.operation
        else {
            continue;
        };
        let occurrence = bypass_time(event, occurrence_time);
        match cause {
            BypassCause::EnvironmentOverride { bypassed_merge } => {
                environment_groups
                    .entry(bypassed_merge.clone())
                    .or_default()
                    .push(occurrence);
            },
            BypassCause::ForcedIntegration { permit_id, reason }
                if forced_seen.insert(*permit_id) =>
            {
                let Some(skipped_holds) = permits.get(permit_id) else {
                    continue;
                };
                audit.push(match skipped_holds {
                    SkippedIntegrationHoldSet::OrderingEdges { edges } => {
                        BypassAuditEntry::ForcedOrderingEdges {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_edges: edges.clone(),
                            occurrence,
                        }
                    },
                    SkippedIntegrationHoldSet::Deferrals { deferrals } => {
                        BypassAuditEntry::ForcedUnresolvedDeferrals {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_deferrals: deferrals.clone(),
                            occurrence,
                        }
                    },
                    SkippedIntegrationHoldSet::OrderingEdgesAndDeferrals { edges, deferrals } => {
                        BypassAuditEntry::ForcedEdgesAndDeferrals {
                            permit_id: *permit_id,
                            reason: reason.clone(),
                            skipped_edges: edges.clone(),
                            skipped_deferrals: deferrals.clone(),
                            occurrence,
                        }
                    },
                });
            },
            BypassCause::ForcedIntegration { .. } => {},
        }
    }
    audit.extend(environment_groups.into_values().map(|occurrences| {
        BypassAuditEntry::EnvironmentOverride {
            override_name: "CARGO_BERTH_BYPASS=1".to_owned(),
            grouped_reference_transactions: u64::try_from(occurrences.len()).unwrap_or(u64::MAX),
            occurrences,
            skipped_holds: UnrecordedSkippedHolds::OverridePrecededLedgerRead,
        }
    }));
    audit
}

fn bypass_time(event: &JournalEvent, occurrence: &BypassOccurrenceTime) -> BoardBypassTime {
    match occurrence {
        BypassOccurrenceTime::EventRecordedAt => BoardBypassTime::Known {
            at: event.recorded_at().clone(),
        },
        BypassOccurrenceTime::Known { at } => BoardBypassTime::Known { at: at.clone() },
        BypassOccurrenceTime::Unavailable => BoardBypassTime::Unknown,
    }
}

pub(super) fn incursion_sections(
    reservations: &RetainedReservationSet,
) -> (Vec<OutstandingIncursion>, Vec<RecordedIncursionAnswer>) {
    let mut outstanding = Vec::new();
    let mut recorded = Vec::new();
    let mut outstanding_counts: HashMap<ReservationId, usize> = HashMap::new();
    for incident in reservations.outstanding_incursion_incidents() {
        *outstanding_counts
            .entry(incident.reservation_id())
            .or_default() += 1;
    }
    for incident in reservations.incursion_incidents() {
        match incident.status() {
            IncursionIncidentStatus::Outstanding => outstanding.push(OutstandingIncursion {
                incident_id:             incident.id(),
                straying_reservation_id: incident.reservation_id(),
                foreign_reservation_ids: incident.foreign_reservation_ids().as_slice().to_vec(),
                entered_paths:           incident.paths().as_slice().to_vec(),
                outstanding_count:       outstanding_counts
                    .get(&incident.reservation_id())
                    .copied()
                    .unwrap_or(1),
                resolution:              IncursionResolutionAction {
                    reservation_id: incident.reservation_id(),
                    incident_id:    incident.id(),
                    flag:           format!(
                        "resolve {} --incursion {}",
                        incident.reservation_id(),
                        incident.id()
                    ),
                    every_flag:     format!(
                        "resolve {} --every-incursion",
                        incident.reservation_id()
                    ),
                },
            }),
            IncursionIncidentStatus::Resolved {
                resolving_actor: _,
                resolution_event_id,
                resolved_at,
            } => recorded.push(RecordedIncursionAnswer {
                incident_id:             incident.id(),
                straying_reservation_id: incident.reservation_id(),
                foreign_reservation_ids: incident.foreign_reservation_ids().as_slice().to_vec(),
                entered_paths:           incident.paths().as_slice().to_vec(),
                resolution_event_id:     *resolution_event_id,
                resolved_at:             resolved_at.clone(),
            }),
        }
    }
    (outstanding, recorded)
}

pub(super) fn board_alerts(
    alerts: &[Alert],
    reservation_snapshots: &[BoardReservationSnapshot],
    unrecorded_bypasses: &[BypassOccurrenceTime],
) -> Result<Vec<BoardAlert>, BoardError> {
    let mut board_alerts = alerts
        .iter()
        .map(board_alert)
        .collect::<Result<Vec<_>, BoardError>>()?;
    board_alerts.extend(reservation_snapshots.iter().filter_map(
        |snapshot| match &snapshot.freshness {
            ReservationFreshness::Stale { .. }
                if snapshot.visibility != BoardReservationVisibility::ResolvedAudit =>
            {
                Some(BoardAlert::StaleReservation {
                    reservation_id: snapshot.reservation_id,
                    freshness:      snapshot.freshness.clone(),
                    resolution:     StaleReservationResolutionAction::Renew {
                        reservation_id: snapshot.reservation_id,
                    },
                })
            },
            ReservationFreshness::Fresh { .. } | ReservationFreshness::Stale { .. } => None,
        },
    ));
    if !unrecorded_bypasses.is_empty() {
        board_alerts.push(BoardAlert::UnrecordedBypasses {
            count: u64::try_from(unrecorded_bypasses.len()).unwrap_or(u64::MAX),
            occurrence_times: unrecorded_bypasses
                .iter()
                .map(|occurrence| match occurrence {
                    BypassOccurrenceTime::Known { at } => BoardBypassTime::Known { at: at.clone() },
                    BypassOccurrenceTime::EventRecordedAt | BypassOccurrenceTime::Unavailable => {
                        BoardBypassTime::Unknown
                    },
                })
                .collect(),
            instruction: "restore journal write access; the pending marker remains until its audit event is durable"
                .to_owned(),
        });
    }
    Ok(board_alerts)
}

fn board_alert(alert: &Alert) -> Result<BoardAlert, BoardError> {
    match alert {
        Alert::LostIntegrationEvidence(lost_evidence) => Ok(BoardAlert::LostIntegrationEvidence {
            reservation_id:  lost_evidence.reservation_id(),
            protected_tip:   lost_evidence.protected_tip().clone(),
            evidence_status: *lost_evidence.evidence_status(),
            recovery:        lost_evidence.recovery().clone(),
        }),
        Alert::OrphanedOutstanding(orphan) => {
            let recoverability = orphan.recoverability();
            Ok(BoardAlert::OrphanedOutstanding {
                reservation_id: orphan.reservation_id(),
                protected_tip: orphan.protected_tip().clone(),
                branch: board_branch_ref_status(orphan.branch_ref_status())?,
                object_availability: orphan.object_availability(),
                retention_ref: board_retention_ref_status(orphan.retention_ref_status()),
                recoverability,
                recovery_consequence: match recoverability {
                    RecoverabilityVerdict::RecoverableFromBranch
                    | RecoverabilityVerdict::RecoverableFromProtectedTip => {
                        OrphanRecoveryConsequence::WorkRecoverable
                    },
                    RecoverabilityVerdict::CommitUnavailable => {
                        OrphanRecoveryConsequence::CommitsLost
                    },
                },
                resolution: match recoverability {
                    RecoverabilityVerdict::RecoverableFromBranch
                    | RecoverabilityVerdict::RecoverableFromProtectedTip => {
                        OrphanResolutionAction::Recover {
                            flag: "resolve --recovered".to_owned(),
                        }
                    },
                    RecoverabilityVerdict::CommitUnavailable => {
                        OrphanResolutionAction::RetireOrAbandon {
                            flags: vec![
                                "resolve --retire-orphan --why <reason>".to_owned(),
                                "resolve --abandon --why <reason>".to_owned(),
                            ],
                        }
                    },
                },
            })
        },
    }
}

fn board_branch_ref_status(status: &BranchRefStatus) -> Result<BoardBranchRefStatus, BoardError> {
    match status {
        BranchRefStatus::Present { reference, tip } => Ok(BoardBranchRefStatus::Present {
            reference: reference
                .parse()
                .map_err(|_| BoardError::InvalidBranchReference(reference.clone()))?,
            tip:       tip.clone(),
        }),
        BranchRefStatus::Missing { reference } => Ok(BoardBranchRefStatus::Missing {
            reference: reference
                .parse()
                .map_err(|_| BoardError::InvalidBranchReference(reference.clone()))?,
        }),
        BranchRefStatus::Detached => Ok(BoardBranchRefStatus::Detached),
    }
}

fn board_retention_ref_status(status: &RetentionRefStatus) -> BoardRetentionRefStatus {
    match status {
        RetentionRefStatus::Present { reference } => BoardRetentionRefStatus::Present {
            reference: ReservationRetentionRef(reference.clone()),
        },
        RetentionRefStatus::Missing { reference } => BoardRetentionRefStatus::Missing {
            reference: ReservationRetentionRef(reference.clone()),
        },
        RetentionRefStatus::Mismatched { reference, actual } => {
            BoardRetentionRefStatus::Mismatched {
                reference: ReservationRetentionRef(reference.clone()),
                actual:    actual.clone(),
            }
        },
    }
}

pub(super) fn board_git_cost(
    reservations: &RetainedReservationSet,
    constraints: &IntegrationConstraintProjection,
    snapshot: &RepositorySnapshot,
    ahead_behind_computations: u64,
    reconciliation_git_cost: &ReconciliationGitCost,
) -> BoardGitCost {
    let reservation_evidence_revalidations = reservations
        .iter()
        .filter(|reservation| {
            matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Outstanding { .. }
                    | ReservationLifecycle::Released {
                        disposition: ReleaseDisposition::Integrated
                            | ReleaseDisposition::RewrittenIntegration(_),
                    }
            )
        })
        .count();
    let protected_predecessors = constraints
        .ordering_constraints
        .iter()
        .map(|constraint| constraint.predecessor)
        .collect::<HashSet<_>>();
    let protected_predecessor_ancestry_queries = protected_predecessors
        .iter()
        .filter(|predecessor_id| {
            snapshot
                .reservation(**predecessor_id)
                .is_ok_and(|reservation| {
                    matches!(
                        reservation.evidence,
                        RepositoryReservationEvidence::Outstanding { .. }
                            | RepositoryReservationEvidence::Released { .. }
                    ) && constraints.ordering_constraints.iter().any(|constraint| {
                        constraint.predecessor == **predecessor_id
                            && snapshot
                                .reservation(constraint.successor)
                                .is_ok_and(|reservation| {
                                    matches!(reservation.worktree_head, WorktreeHead::Resolved(_))
                                })
                    })
                })
        })
        .count();
    BoardGitCost {
        trunk_resolution_calls:                 reconciliation_git_cost.trunk_resolution_calls,
        worktree_list_calls:                    1,
        reservation_evidence_revalidations:     u64::try_from(reservation_evidence_revalidations)
            .unwrap_or(u64::MAX),
        protected_predecessor_ancestry_queries: u64::try_from(
            protected_predecessor_ancestry_queries,
        )
        .unwrap_or(u64::MAX),
        worktree_ahead_behind_computations:     ahead_behind_computations,
        orphan_recovery_evidence_queries:       reconciliation_git_cost
            .orphan_recovery_evidence_queries,
    }
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::BoardAlert;
    use super::StaleReservationResolutionAction;
    use super::board_alerts;
    use crate::answer::ConflictAuthorization;
    use crate::board::test_support;
    use crate::board::test_support::BoardFixture;
    use crate::board::test_support::FixtureResult;
    use crate::reservation::ReservationFreshness;

    #[test]
    fn stale_reservation_alert_names_the_renew_resolution() -> FixtureResult<()> {
        let fixture = BoardFixture::new()?;
        let actor = fixture.main_actor();
        let reservation = fixture.claim(&actor, "stale.rs", ConflictAuthorization::NoConflict)?;
        let model = fixture.model()?;
        let fresh_row =
            test_support::board_reservation_snapshot(&model, reservation.reservation_id)?.clone();
        assert!(board_alerts(&[], std::slice::from_ref(&fresh_row), &[])?.is_empty());

        let mut stale_row = fresh_row;
        let ReservationFreshness::Fresh { last_activity_at } = stale_row.freshness.clone() else {
            return Err(io::Error::other("new reservation should be fresh").into());
        };
        stale_row.freshness = ReservationFreshness::Stale { last_activity_at };
        let alerts = board_alerts(&[], &[stale_row], &[])?;
        assert!(matches!(
            alerts.as_slice(),
            [BoardAlert::StaleReservation {
                reservation_id,
                resolution: StaleReservationResolutionAction::Renew {
                    reservation_id: action_reservation_id,
                },
                ..
            }] if *reservation_id == reservation.reservation_id
                && *action_reservation_id == reservation.reservation_id
        ));
        Ok(())
    }
}
