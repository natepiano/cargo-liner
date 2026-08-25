//! Locked trunk-update decisions shared by `integrate` and the git hook.

pub(crate) mod install;
pub(crate) mod permit;

use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::str::FromStr;

use serde::Deserialize;
use serde::Serialize;

use crate::alert::Alert;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::config::GateMode;
use crate::edge::IntegrationConstraintProjection;
use crate::edge::IntegrationHold;
use crate::edge::IntegrationReservationFacts;
use crate::edge::MissingReadinessFact;
use crate::git;
use crate::git::GitError;
use crate::ids::CoordinationRunId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::BypassCause;
use crate::ledger::BypassedAction;
use crate::ledger::EditAuthorization;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::FullRefName;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::SkippedDeferral;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::SkippedOrderingEdge;
use crate::ledger::WorktreeContext;
use crate::reconcile;
use crate::reconcile::GateReconciliationError;
use crate::reservation::ReservationLifecycle;

const LOCAL_BRANCH_REFERENCE_PREFIX: &str = "refs/heads/";
const SHA1_OBJECT_ID_CHARACTERS: usize = 40;
const SHA256_OBJECT_ID_CHARACTERS: usize = 64;
const SYMBOLIC_REFERENCE_VALUE_PREFIX: &str = "ref:";

/// Git's reference-transaction phase, converted at the hidden CLI boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceTransactionPhase {
    /// Git is asking hooks to approve the complete proposed transaction.
    Prepared,
    /// Git reports that an already-approved transaction committed.
    Committed,
    /// Git reports that an already-approved transaction aborted.
    Aborted,
}

/// One complete semantic reference transaction, including every proposed update.
#[derive(Clone)]
pub(crate) struct ReferenceTransaction {
    /// The lifecycle point at which git invoked the hook.
    pub(crate) phase: ReferenceTransactionPhase,
    /// Every line supplied on standard input, in git's transaction order.
    entries:          Vec<ReferenceTransactionEntry>,
}

#[derive(Clone)]
enum ReferenceTransactionEntry {
    LocalBranch(ReferenceUpdate),
    OutsideLocalBranchNamespace,
}

/// Whether a parsed transaction names the configured trunk reference.
pub(crate) enum TrunkReferencePresence {
    /// At least one local-branch update names the trunk reference.
    Named,
    /// No local-branch update names the trunk reference.
    NotNamed,
}

/// One parsed old-object, new-object, and full-reference update.
#[derive(Clone)]
pub(crate) struct ReferenceUpdate {
    /// The object currently named by the ref, or git's all-zero absence marker.
    previous:  ReferenceObject,
    /// The object the transaction proposes, or git's all-zero deletion marker.
    proposed:  ReferenceObject,
    /// The full `refs/...` name being updated.
    reference: FullRefName,
}

/// A real git object or the all-zero sentinel used at reference boundaries.
#[derive(Clone)]
enum ReferenceObject {
    Object(GitObjectId),
    Symbolic(FullRefName),
    Absent,
}

enum ReferenceUpdateGateSubject {
    ProposedMainMove(ProposedMainMove),
    NotMainEntry,
    UnsupportedMainUpdate,
}

#[derive(Clone)]
enum PreviousMain {
    Existing(GitObjectId),
    Absent,
}

/// The typed integration request produced from clap's force and reason primitives.
#[derive(Clone)]
pub(crate) enum IntegrationRequest {
    /// Apply normal gate policy.
    EnforceOrdering,
    /// Mint a one-use permit carrying this inseparable non-empty reason.
    ForceOnce(ForcedIntegrationReason),
}

/// A violation identifies the entering reservation and every hold that blocks it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct IntegrationViolation {
    /// Facts needed to identify the reservation, plan, phase, and footprint.
    pub(crate) reservation:           IntegrationReservationFacts,
    /// The exact counterpart facts needed to identify every named blocker.
    pub(crate) blocking_reservations: Vec<IntegrationReservationFacts>,
    /// Every edge and unresolved deferral currently holding this reservation.
    pub(crate) holds:                 Vec<IntegrationHold>,
}

/// A decision made against one proposed main update at one journal generation.
pub(crate) enum GateDecision {
    /// No newly entering reservation is held.
    Clear {
        /// The exact replay generation validated under the mutation lock.
        generation: crate::ids::ProjectionGeneration,
    },
    /// Observe-only policy found violations but permits the update.
    Observed {
        /// The exact replay generation validated under the mutation lock.
        generation: crate::ids::ProjectionGeneration,
        /// Every violation that enforcing mode would reject.
        violations: Vec<IntegrationViolation>,
    },
    /// Enforcing policy rejects these violations.
    Blocked {
        /// The exact replay generation validated under the mutation lock.
        generation: crate::ids::ProjectionGeneration,
        /// Every violation preventing this update.
        violations: Vec<IntegrationViolation>,
    },
    /// A forced integration journalled a permit for the next matching main update.
    PermitIssued {
        /// The exact replay generation validated under the mutation lock.
        generation:          crate::ids::ProjectionGeneration,
        /// The one-use semantic permit identity.
        permit_id:           ForcedIntegrationPermitId,
        /// The reservation the permit authorizes.
        reservation_id:      ReservationId,
        /// Every hold deliberately skipped.
        skipped_holds:       SkippedIntegrationHoldSet,
        /// Holds on other entering reservations reported under observe-only policy.
        observed_violations: Vec<IntegrationViolation>,
    },
    /// Matching one-use permits were consumed and their skipped holds stayed intact.
    Forced {
        /// The exact replay generation validated under the mutation lock.
        generation: crate::ids::ProjectionGeneration,
    },
}

/// A complete gate result with reconciliation alerts from the same lock hold.
pub(crate) struct GateResult {
    /// The integration decision.
    pub(crate) decision: GateDecision,
    /// Durable alerts produced by the preceding actual-trunk reconciliation.
    pub(crate) alerts:   Vec<Alert>,
}

#[derive(Clone)]
enum GatePurpose {
    Hook {
        phase: ReferenceTransactionPhase,
    },
    Integrate {
        reservation_id: ReservationId,
        request:        IntegrationRequest,
        acting_run:     ActingRun,
    },
}

#[derive(Clone, Copy)]
enum ActingRun {
    Independent(CoordinationRunId),
    ActiveSessionReservationRequired {
        coordination_run_id: CoordinationRunId,
        reservation_id:      ReservationId,
        worktree_id:         WorktreeId,
    },
    ActiveMarkerRequired {
        coordination_run_id: CoordinationRunId,
        worktree_id:         WorktreeId,
    },
}

/// Parse every stdin line into one semantic git reference transaction.
pub(crate) fn parse_reference_transaction(
    phase: ReferenceTransactionPhase,
    input: &str,
) -> Result<ReferenceTransaction, ReferenceTransactionParseError> {
    let entries = input
        .lines()
        .enumerate()
        .map(|(index, line)| parse_reference_update(index + 1, line))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReferenceTransaction { phase, entries })
}

impl ReferenceTransaction {
    /// Classify whether this transaction includes the configured trunk reference.
    pub(crate) fn trunk_reference_presence(
        &self,
        trunk_reference: &FullRefName,
    ) -> TrunkReferencePresence {
        if self.entries.iter().any(|entry| {
            matches!(
                entry,
                ReferenceTransactionEntry::LocalBranch(update)
                    if &update.reference == trunk_reference
            )
        }) {
            TrunkReferencePresence::Named
        } else {
            TrunkReferencePresence::NotNamed
        }
    }
}

fn parse_reference_update(
    line_number: usize,
    line: &str,
) -> Result<ReferenceTransactionEntry, ReferenceTransactionParseError> {
    let fields = line.split_ascii_whitespace().collect::<Vec<_>>();
    let [previous, proposed, reference] = fields.as_slice() else {
        return Err(ReferenceTransactionParseError::FieldCount { line_number });
    };
    if !reference.starts_with(LOCAL_BRANCH_REFERENCE_PREFIX) {
        return Ok(ReferenceTransactionEntry::OutsideLocalBranchNamespace);
    }
    Ok(ReferenceTransactionEntry::LocalBranch(ReferenceUpdate {
        previous:  parse_reference_object(previous).map_err(|()| {
            ReferenceTransactionParseError::InvalidObject {
                line_number,
                value: previous.to_string(),
            }
        })?,
        proposed:  parse_reference_object(proposed).map_err(|()| {
            ReferenceTransactionParseError::InvalidObject {
                line_number,
                value: proposed.to_string(),
            }
        })?,
        reference: reference.parse().map_err(|_| {
            ReferenceTransactionParseError::InvalidReference {
                line_number,
                value: reference.to_string(),
            }
        })?,
    }))
}

fn parse_reference_object(value: &str) -> Result<ReferenceObject, ()> {
    if matches!(
        value.len(),
        SHA1_OBJECT_ID_CHARACTERS | SHA256_OBJECT_ID_CHARACTERS
    ) && value.bytes().all(|byte| byte == b'0')
    {
        Ok(ReferenceObject::Absent)
    } else if let Some(reference) = value.strip_prefix(SYMBOLIC_REFERENCE_VALUE_PREFIX) {
        reference
            .parse()
            .map(ReferenceObject::Symbolic)
            .map_err(|_| ())
    } else {
        value.parse().map(ReferenceObject::Object).map_err(|_| ())
    }
}

/// Evaluate prepared trunk updates and commit their approved permit audits after Git moves the ref.
pub(crate) fn evaluate_reference_transaction(
    invocation_directory: &Path,
    transaction: &ReferenceTransaction,
    trunk_reference: &FullRefName,
) -> Result<Vec<GateResult>, GateError> {
    if transaction.phase == ReferenceTransactionPhase::Aborted {
        return Ok(Vec::new());
    }
    let trunk_updates = transaction
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ReferenceTransactionEntry::LocalBranch(update) => Some(update),
            ReferenceTransactionEntry::OutsideLocalBranchNamespace => None,
        })
        .filter(|update| &update.reference == trunk_reference)
        .collect::<Vec<_>>();
    if trunk_updates.is_empty() {
        return Ok(Vec::new());
    }
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured { .. } => return Ok(Vec::new()),
    };
    let mut results = Vec::new();
    for update in trunk_updates {
        match update.gate_subject() {
            ReferenceUpdateGateSubject::ProposedMainMove(update) => {
                if update.materializes_existing_logical_trunk(
                    worktree_context.repository_root(),
                    &berth_config.trunk,
                ) {
                    continue;
                }
                match transaction.phase {
                    ReferenceTransactionPhase::Prepared => {
                        match evaluate_locked(
                            invocation_directory,
                            &update,
                            &GatePurpose::Hook {
                                phase: ReferenceTransactionPhase::Prepared,
                            },
                        )? {
                            Enrollment::Enrolled(result) => results.push(result),
                            Enrollment::Unconfigured { .. } => return Ok(Vec::new()),
                        }
                    },
                    ReferenceTransactionPhase::Committed => commit_forced_permit_audits(
                        invocation_directory,
                        &worktree_context,
                        &berth_config,
                        &update,
                    )?,
                    ReferenceTransactionPhase::Aborted => {},
                }
            },
            ReferenceUpdateGateSubject::NotMainEntry => {},
            ReferenceUpdateGateSubject::UnsupportedMainUpdate => {
                return Err(GateError::UnsupportedSymbolicTrunkUpdate);
            },
        }
    }
    Ok(results)
}

fn commit_forced_permit_audits(
    invocation_directory: &Path,
    worktree_context: &WorktreeContext,
    berth_config: &BerthConfig,
    update: &ProposedMainMove,
) -> Result<(), GateError> {
    let ledger = Ledger::open(invocation_directory)?;
    let ledger_repository = ledger.repository_identity()?;
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = CoordinationRunId::new();
    let outcome = ledger
        .transact_reconciliation(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let prepared = match reconcile::prepare_gate_reconciliation(
                    state.events(),
                    state.generation(),
                    worktree_context,
                    ledger_repository,
                    berth_config,
                    update.proposed.clone(),
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(error),
                        );
                    },
                };
                let newly_reachable =
                    match newly_reachable_commits(worktree_context.repository_root(), update) {
                        Ok(newly_reachable) => newly_reachable,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Git(error),
                            );
                        },
                    };
                let entering = entering_reservations(prepared.constraints(), &newly_reachable);
                let (_, operations) = match decide(
                    state.events(),
                    prepared.constraints(),
                    &entering,
                    &GatePurpose::Hook {
                        phase: ReferenceTransactionPhase::Committed,
                    },
                    berth_config.gate_mode,
                ) {
                    Ok(decision) => decision,
                    Err(error) => return ReconciliationValidation::Reject(error),
                };
                ReconciliationValidation::Apply {
                    operations,
                    recoverable_operations: Vec::new(),
                    action: (),
                }
            },
            |(), _, _| Ok::<(), Infallible>(()),
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
            LedgerCommittedActionError::Action(error) => match error {},
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended { output: (), .. } => Ok(()),
        LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
    }
}

/// Evaluate one explicit integration request through the same locked decision path as the hook.
pub(crate) fn evaluate_integration(
    invocation_directory: &Path,
    reservation_id: ReservationId,
    request: IntegrationRequest,
    previous: GitObjectId,
    proposed: GitObjectId,
) -> Result<Enrollment<GateResult>, GateError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let acting_run = ActingRun::resolve(&worktree_context);
    let update = ProposedMainMove {
        previous: PreviousMain::Existing(previous),
        proposed,
    };
    let purpose = GatePurpose::Integrate {
        reservation_id,
        request,
        acting_run,
    };
    evaluate_locked(invocation_directory, &update, &purpose)
}

#[derive(Clone)]
struct ProposedMainMove {
    previous: PreviousMain,
    proposed: GitObjectId,
}

impl ProposedMainMove {
    fn materializes_existing_logical_trunk(&self, repository_root: &Path, trunk: &str) -> bool {
        matches!(&self.previous, PreviousMain::Absent)
            && git::branch_object_id(repository_root, trunk)
                .is_ok_and(|current| current == self.proposed)
    }
}

impl ReferenceUpdate {
    fn gate_subject(&self) -> ReferenceUpdateGateSubject {
        match (&self.previous, &self.proposed) {
            (ReferenceObject::Object(previous), ReferenceObject::Object(proposed))
                if previous != proposed =>
            {
                ReferenceUpdateGateSubject::ProposedMainMove(ProposedMainMove {
                    previous: PreviousMain::Existing(previous.clone()),
                    proposed: proposed.clone(),
                })
            },
            (ReferenceObject::Absent, ReferenceObject::Object(proposed)) => {
                ReferenceUpdateGateSubject::ProposedMainMove(ProposedMainMove {
                    previous: PreviousMain::Absent,
                    proposed: proposed.clone(),
                })
            },
            (ReferenceObject::Object(_) | ReferenceObject::Absent, ReferenceObject::Absent)
            | (ReferenceObject::Object(_), ReferenceObject::Object(_)) => {
                ReferenceUpdateGateSubject::NotMainEntry
            },
            (ReferenceObject::Symbolic(previous), ReferenceObject::Symbolic(proposed))
                if previous == proposed =>
            {
                ReferenceUpdateGateSubject::NotMainEntry
            },
            (ReferenceObject::Symbolic(_), _) | (_, ReferenceObject::Symbolic(_)) => {
                ReferenceUpdateGateSubject::UnsupportedMainUpdate
            },
        }
    }
}

fn evaluate_locked(
    invocation_directory: &Path,
    update: &ProposedMainMove,
    purpose: &GatePurpose,
) -> Result<Enrollment<GateResult>, GateError> {
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let berth_config = match BerthConfig::read(worktree_context.repository_root())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured {
            expected_configuration_path,
        } => {
            return Ok(Enrollment::Unconfigured {
                expected_configuration_path,
            });
        },
    };
    let ledger = Ledger::open(worktree_context.repository_root())?;
    let ledger_repository = ledger.repository_identity()?;
    let worktree_identity = ledger::worktree_identity(
        worktree_context.administrative_directory(),
        worktree_context.worktree_kind(),
    )?;
    let coordination_run_id = purpose.coordination_run_id();
    let outcome = ledger
        .transact_reconciliation(
            worktree_identity.id,
            coordination_run_id,
            |state| {
                let prepared = match reconcile::prepare_gate_reconciliation(
                    state.events(),
                    state.generation(),
                    &worktree_context,
                    ledger_repository,
                    &berth_config,
                    update.proposed.clone(),
                ) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(error),
                        );
                    },
                };
                if let GatePurpose::Integrate { acting_run, .. } = purpose
                    && let Err(rejection) = acting_run.validate(prepared.reservations())
                {
                    return ReconciliationValidation::Reject(rejection);
                }
                let newly_reachable =
                    match newly_reachable_commits(worktree_context.repository_root(), update) {
                        Ok(newly_reachable) => newly_reachable,
                        Err(error) => {
                            return ReconciliationValidation::Reject(
                                GateTransactionRejection::Git(error),
                            );
                        },
                    };
                let entering = entering_reservations(prepared.constraints(), &newly_reachable);
                let (decision, operations) = match decide(
                    state.events(),
                    prepared.constraints(),
                    &entering,
                    purpose,
                    berth_config.gate_mode,
                ) {
                    Ok(decision) => decision,
                    Err(error) => return ReconciliationValidation::Reject(error),
                };
                let (operations, action) = prepared.into_action(operations, decision);
                ReconciliationValidation::Apply {
                    operations,
                    recoverable_operations: Vec::new(),
                    action,
                }
            },
            reconcile::GateReconciliationAction::commit,
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
            LedgerCommittedActionError::Action(error) => GateError::Reconciliation(error),
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended {
            output: (report, decision),
            ..
        } => Ok(Enrollment::Enrolled(GateResult {
            decision,
            alerts: report.alerts,
        })),
        LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
    }
}

fn entering_reservations(
    constraints: &IntegrationConstraintProjection,
    newly_reachable: &[GitObjectId],
) -> Vec<ReservationId> {
    constraints
        .reservations
        .iter()
        .filter(|reservation| {
            !matches!(reservation.lifecycle, ReservationLifecycle::Released { .. })
        })
        .filter_map(|reservation| match &reservation.subject {
            crate::edge::IntegrationSubject::Commit { object_id }
                if newly_reachable.contains(object_id) =>
            {
                Some(reservation.reservation_id)
            },
            crate::edge::IntegrationSubject::WorktreeHeadUnavailable => {
                Some(reservation.reservation_id)
            },
            crate::edge::IntegrationSubject::Commit { .. }
            | crate::edge::IntegrationSubject::NotApplicable => None,
        })
        .collect()
}

fn decide(
    events: &[crate::ledger::JournalEvent],
    constraints: &IntegrationConstraintProjection,
    entering: &[ReservationId],
    purpose: &GatePurpose,
    gate_mode: GateMode,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    let mut violations = Vec::new();
    for reservation_id in entering {
        let holds = constraints
            .holds_for(*reservation_id)
            .cloned()
            .collect::<Vec<_>>();
        if holds.is_empty() {
            continue;
        }
        let reservation = constraints
            .reservation(*reservation_id)
            .map_err(GateTransactionRejection::MissingConstraintFact)?
            .clone();
        let blocking_reservations = blocking_reservations(constraints, *reservation_id, &holds)?;
        violations.push(IntegrationViolation {
            reservation,
            blocking_reservations,
            holds,
        });
    }
    match purpose {
        GatePurpose::Hook { phase } => decide_hook(
            events,
            constraints.generation,
            &violations,
            gate_mode,
            *phase,
        ),
        GatePurpose::Integrate {
            reservation_id,
            request,
            ..
        } => decide_integration(
            constraints.generation,
            *reservation_id,
            request,
            entering,
            violations,
            gate_mode,
        ),
    }
}

fn newly_reachable_commits(
    repository_root: &Path,
    update: &ProposedMainMove,
) -> Result<Vec<GitObjectId>, GitError> {
    match &update.previous {
        PreviousMain::Existing(previous) => {
            git::newly_reachable_commits(repository_root, previous, &update.proposed)
        },
        PreviousMain::Absent => git::reachable_commits(repository_root, &update.proposed),
    }
}

fn blocking_reservations(
    constraints: &IntegrationConstraintProjection,
    subject: ReservationId,
    holds: &[IntegrationHold],
) -> Result<Vec<IntegrationReservationFacts>, GateTransactionRejection> {
    let mut blocker_ids = holds
        .iter()
        .map(|hold| match hold {
            IntegrationHold::OrderingEdge { predecessor, .. } => *predecessor,
            IntegrationHold::DeferredOverlap {
                deferred, blocker, ..
            } if *deferred == subject => *blocker,
            IntegrationHold::DeferredOverlap { deferred, .. } => *deferred,
        })
        .collect::<Vec<_>>();
    blocker_ids.sort_by_key(ToString::to_string);
    blocker_ids.dedup();
    blocker_ids
        .into_iter()
        .map(|blocker_id| {
            constraints
                .reservation(blocker_id)
                .cloned()
                .map_err(GateTransactionRejection::MissingConstraintFact)
        })
        .collect()
}

fn decide_hook(
    events: &[crate::ledger::JournalEvent],
    generation: crate::ids::ProjectionGeneration,
    violations: &[IntegrationViolation],
    gate_mode: GateMode,
    phase: ReferenceTransactionPhase,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    if violations.is_empty() {
        return Ok((GateDecision::Clear { generation }, Vec::new()));
    }
    let permits = permit::available_forced_integration_permits(events)
        .map_err(GateTransactionRejection::PermitReplay)?;
    let mut covered = Vec::new();
    let mut uncovered = Vec::new();
    for violation in violations {
        let covering_permit = permits.iter().find(|permit| {
            permit.reservation_id == violation.reservation.reservation_id
                && skipped_set_covers(&permit.skipped_holds, &violation.holds)
        });
        if let Some(permit) = covering_permit {
            covered.push(permit);
        } else {
            uncovered.push(violation.clone());
        }
    }
    if gate_mode.enforces() && !uncovered.is_empty() {
        return Ok((
            GateDecision::Blocked {
                generation,
                violations: uncovered,
            },
            Vec::new(),
        ));
    }
    let operations = match phase {
        ReferenceTransactionPhase::Committed => covered
            .into_iter()
            .flat_map(|permit| {
                [
                    JournalOperation::ConsumeForcedIntegrationPermit {
                        permit_id:      permit.permit_id,
                        reservation_id: permit.reservation_id,
                    },
                    JournalOperation::Bypass {
                        action:          BypassedAction::Integration,
                        cause:           BypassCause::ForcedIntegration {
                            permit_id: permit.permit_id,
                            reason:    permit.reason.clone(),
                        },
                        occurrence_time: crate::ledger::BypassOccurrenceTime::EventRecordedAt,
                        recording:       crate::ledger::BypassRecording::Direct,
                    },
                ]
            })
            .collect(),
        ReferenceTransactionPhase::Prepared | ReferenceTransactionPhase::Aborted => Vec::new(),
    };
    if uncovered.is_empty() {
        Ok((GateDecision::Forced { generation }, operations))
    } else {
        Ok((
            GateDecision::Observed {
                generation,
                violations: uncovered,
            },
            operations,
        ))
    }
}

fn decide_integration(
    generation: crate::ids::ProjectionGeneration,
    reservation_id: ReservationId,
    request: &IntegrationRequest,
    entering: &[ReservationId],
    violations: Vec<IntegrationViolation>,
    gate_mode: GateMode,
) -> Result<(GateDecision, Vec<JournalOperation>), GateTransactionRejection> {
    if !entering.contains(&reservation_id) {
        return Err(GateTransactionRejection::ReservationNotEntering(
            reservation_id,
        ));
    }
    match request {
        IntegrationRequest::ForceOnce(reason) => {
            let Some(violation) = violations
                .iter()
                .find(|violation| violation.reservation.reservation_id == reservation_id)
            else {
                return Err(GateTransactionRejection::NoHoldToForce(reservation_id));
            };
            let observed_violations = violations
                .iter()
                .filter(|violation| violation.reservation.reservation_id != reservation_id)
                .cloned()
                .collect::<Vec<_>>();
            if gate_mode.enforces() && !observed_violations.is_empty() {
                return Ok((
                    GateDecision::Blocked {
                        generation,
                        violations,
                    },
                    Vec::new(),
                ));
            }
            let skipped_holds = skipped_holds(&violation.holds)?;
            let permit_id = ForcedIntegrationPermitId::new();
            Ok((
                GateDecision::PermitIssued {
                    generation,
                    permit_id,
                    reservation_id,
                    skipped_holds: skipped_holds.clone(),
                    observed_violations,
                },
                vec![JournalOperation::ForcedIntegrationPermit {
                    permit_id,
                    reservation_id,
                    reason: reason.clone(),
                    skipped_holds,
                }],
            ))
        },
        IntegrationRequest::EnforceOrdering if !violations.is_empty() && gate_mode.enforces() => {
            Ok((
                GateDecision::Blocked {
                    generation,
                    violations,
                },
                Vec::new(),
            ))
        },
        IntegrationRequest::EnforceOrdering if !violations.is_empty() => Ok((
            GateDecision::Observed {
                generation,
                violations,
            },
            Vec::new(),
        )),
        IntegrationRequest::EnforceOrdering => Ok((GateDecision::Clear { generation }, Vec::new())),
    }
}

fn skipped_holds(
    holds: &[IntegrationHold],
) -> Result<SkippedIntegrationHoldSet, GateTransactionRejection> {
    let mut edges = Vec::new();
    let mut deferrals = Vec::new();
    for hold in holds {
        match hold {
            IntegrationHold::OrderingEdge {
                edge_id,
                predecessor,
                ..
            } => edges.push(SkippedOrderingEdge {
                edge_id:     *edge_id,
                predecessor: *predecessor,
            }),
            IntegrationHold::DeferredOverlap {
                declaration_event_id,
                deferred,
                blocker,
                ..
            } => deferrals.push(SkippedDeferral {
                declaration_event_id: *declaration_event_id,
                deferred:             *deferred,
                blocker:              *blocker,
            }),
        }
    }
    SkippedIntegrationHoldSet::new(edges, deferrals)
        .map_err(|_| GateTransactionRejection::MissingSkippedHold)
}

fn skipped_set_covers(
    skipped_holds: &SkippedIntegrationHoldSet,
    holds: &[IntegrationHold],
) -> bool {
    let edge_ids = holds
        .iter()
        .filter_map(|hold| match hold {
            IntegrationHold::OrderingEdge { edge_id, .. } => Some(*edge_id),
            IntegrationHold::DeferredOverlap { .. } => None,
        })
        .collect::<Vec<_>>();
    let deferral_event_ids = holds
        .iter()
        .filter_map(|hold| match hold {
            IntegrationHold::DeferredOverlap {
                declaration_event_id,
                ..
            } => Some(*declaration_event_id),
            IntegrationHold::OrderingEdge { .. } => None,
        })
        .collect::<Vec<_>>();
    skipped_holds.covers(&edge_ids, &deferral_event_ids)
}

impl GatePurpose {
    fn coordination_run_id(&self) -> CoordinationRunId {
        match self {
            Self::Hook { .. } => CoordinationRunId::new(),
            Self::Integrate { acting_run, .. } => acting_run.coordination_run_id(),
        }
    }
}

impl ActingRun {
    fn resolve(worktree_context: &WorktreeContext) -> Self {
        match EditAuthorization::resolve(
            worktree_context.administrative_directory(),
            &worktree_context.ledger_directory(),
        ) {
            EditAuthorization::Session {
                coordination_run_id,
                reservation_id,
                worktree_id,
            } => Self::ActiveSessionReservationRequired {
                coordination_run_id,
                reservation_id,
                worktree_id,
            },
            EditAuthorization::Environment(coordination_run_id) => {
                Self::Independent(coordination_run_id)
            },
            EditAuthorization::Marker {
                coordination_run_id,
                worktree_id,
            } => Self::ActiveMarkerRequired {
                coordination_run_id,
                worktree_id,
            },
            EditAuthorization::Unidentified => Self::Independent(CoordinationRunId::new()),
        }
    }

    const fn coordination_run_id(self) -> CoordinationRunId {
        match self {
            Self::Independent(coordination_run_id)
            | Self::ActiveSessionReservationRequired {
                coordination_run_id,
                ..
            }
            | Self::ActiveMarkerRequired {
                coordination_run_id,
                ..
            } => coordination_run_id,
        }
    }

    fn validate(
        self,
        reservations: &crate::reservation::RetainedReservationSet,
    ) -> Result<(), GateTransactionRejection> {
        if let Self::ActiveSessionReservationRequired {
            coordination_run_id,
            reservation_id,
            worktree_id,
        } = self
        {
            return if reservations.iter().any(|reservation| {
                reservation.id() == reservation_id
                    && reservation.actor().run == coordination_run_id
                    && reservation.actor().worktree == worktree_id
                    && matches!(reservation.lifecycle(), ReservationLifecycle::Active)
            }) {
                Ok(())
            } else {
                Err(GateTransactionRejection::InactiveSessionMapping(
                    coordination_run_id,
                ))
            };
        }
        let Self::ActiveMarkerRequired {
            coordination_run_id,
            worktree_id,
        } = self
        else {
            return Ok(());
        };
        if reservations.iter().any(|reservation| {
            reservation.actor().run == coordination_run_id
                && reservation.actor().worktree == worktree_id
                && matches!(reservation.lifecycle(), ReservationLifecycle::Active)
        }) {
            Ok(())
        } else {
            Err(GateTransactionRejection::InactiveMarkerRun(
                coordination_run_id,
            ))
        }
    }
}

impl FromStr for ReferenceTransactionPhase {
    type Err = InvalidReferenceTransactionPhase;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "committed" => Ok(Self::Committed),
            "aborted" => Ok(Self::Aborted),
            _ => Err(InvalidReferenceTransactionPhase(value.to_owned())),
        }
    }
}

/// Git supplied an unsupported reference-transaction lifecycle word.
#[derive(Debug)]
pub(crate) struct InvalidReferenceTransactionPhase(String);

impl Display for InvalidReferenceTransactionPhase {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid reference-transaction phase: {}", self.0)
    }
}

impl Error for InvalidReferenceTransactionPhase {}

/// Git's reference-transaction input could not be converted into semantic updates.
#[derive(Debug)]
pub(crate) enum ReferenceTransactionParseError {
    /// A line did not have exactly old object, new object, and full ref name.
    FieldCount { line_number: usize },
    /// An old or new object was neither a full id nor an all-zero sentinel.
    InvalidObject {
        line_number: usize,
        value:       String,
    },
    /// A full reference name failed validation.
    InvalidReference {
        line_number: usize,
        value:       String,
    },
}

impl Display for ReferenceTransactionParseError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::FieldCount { line_number } => write!(
                formatter,
                "reference-transaction line {line_number} must have exactly three fields"
            ),
            Self::InvalidObject { line_number, value } => write!(
                formatter,
                "reference-transaction line {line_number} has invalid object id {value:?}"
            ),
            Self::InvalidReference { line_number, value } => write!(
                formatter,
                "reference-transaction line {line_number} has invalid ref name {value:?}"
            ),
        }
    }
}

impl Error for ReferenceTransactionParseError {}

#[derive(Debug)]
enum GateTransactionRejection {
    Reconciliation(GateReconciliationError),
    Git(GitError),
    PermitReplay(permit::ForcedIntegrationPermitReplayError),
    InactiveSessionMapping(CoordinationRunId),
    InactiveMarkerRun(CoordinationRunId),
    ReservationNotEntering(ReservationId),
    NoHoldToForce(ReservationId),
    MissingSkippedHold,
    MissingConstraintFact(MissingReadinessFact),
}

/// A gate decision failed before it could establish safe integration facts.
#[derive(Debug)]
pub(crate) enum GateError {
    Config(crate::config::ConfigError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    Reconciliation(reconcile::ReconcileError),
    Planning(GateReconciliationError),
    Git(GitError),
    PermitReplay(permit::ForcedIntegrationPermitReplayError),
    InactiveSessionMapping(CoordinationRunId),
    InactiveMarkerRun(CoordinationRunId),
    ReservationNotEntering(ReservationId),
    NoHoldToForce(ReservationId),
    MissingSkippedHold,
    MissingConstraintFact(MissingReadinessFact),
    UnsupportedSymbolicTrunkUpdate,
}

impl Display for GateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::Reconciliation(error) => error.fmt(formatter),
            Self::Planning(error) => error.fmt(formatter),
            Self::Git(error) => error.fmt(formatter),
            Self::PermitReplay(error) => error.fmt(formatter),
            Self::InactiveSessionMapping(run) => write!(
                formatter,
                "harness session mapping for coordination run {run} no longer names an active reservation in this worktree"
            ),
            Self::InactiveMarkerRun(run) => write!(
                formatter,
                "coordination-run marker {run} no longer has an active reservation in this worktree"
            ),
            Self::ReservationNotEntering(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is not newly reachable in the proposed main update"
            ),
            Self::NoHoldToForce(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has no integration hold to force"
            ),
            Self::MissingSkippedHold => {
                formatter.write_str("a forced integration found no hold to record")
            },
            Self::MissingConstraintFact(error) => error.fmt(formatter),
            Self::UnsupportedSymbolicTrunkUpdate => formatter.write_str(
                "the configured trunk received a symbolic-ref update instead of a commit update",
            ),
        }
    }
}

impl Error for GateError {}

impl From<GateTransactionRejection> for GateError {
    fn from(rejection: GateTransactionRejection) -> Self {
        match rejection {
            GateTransactionRejection::Reconciliation(error) => Self::Planning(error),
            GateTransactionRejection::Git(error) => Self::Git(error),
            GateTransactionRejection::PermitReplay(error) => Self::PermitReplay(error),
            GateTransactionRejection::InactiveSessionMapping(run) => {
                Self::InactiveSessionMapping(run)
            },
            GateTransactionRejection::InactiveMarkerRun(run) => Self::InactiveMarkerRun(run),
            GateTransactionRejection::ReservationNotEntering(reservation_id) => {
                Self::ReservationNotEntering(reservation_id)
            },
            GateTransactionRejection::NoHoldToForce(reservation_id) => {
                Self::NoHoldToForce(reservation_id)
            },
            GateTransactionRejection::MissingSkippedHold => Self::MissingSkippedHold,
            GateTransactionRejection::MissingConstraintFact(error) => {
                Self::MissingConstraintFact(error)
            },
        }
    }
}

impl From<crate::config::ConfigError> for GateError {
    fn from(error: crate::config::ConfigError) -> Self { Self::Config(error) }
}

impl From<LedgerError> for GateError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}
