//! Git reference-transaction parsing and the hook's target-update evaluation.

use std::collections::HashSet;
use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::str::FromStr;

use super::audit;
use super::decision;
use super::decision::GatePreparationRejection;
use super::decision::GatePurpose;
use super::decision::GateResult;
use super::error::GateError;
use super::rewrite;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git;
use crate::git::OperationObjectReader;
use crate::git::RepositorySnapshot;
use crate::ids::GitObjectId;
use crate::ledger::FullRefName;
use crate::ledger::GATE_TARGETS_FILE_NAME;
use crate::ledger::IntegrationTarget;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::WorktreeContext;
use crate::reservation::RecordedTarget;
use crate::reservation::Reservation;
use crate::reservation::ReservationLifecycle;

const LOCAL_BRANCH_REFERENCE_PREFIX: &str = "refs/heads/";
pub(crate) const REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT: &str =
    "CARGO_BERTH_REFERENCE_TRANSACTION_ISSUING_DIRECTORY";
const SHA1_OBJECT_ID_CHARACTERS: usize = 40;
const SHA256_OBJECT_ID_CHARACTERS: usize = 64;
const SYMBOLIC_REFERENCE_VALUE_PREFIX: &str = "ref:";

/// Git's reference-transaction phase, converted at the hidden CLI boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceTransactionPhase {
    /// Git is assembling the transaction, one phase ahead of `Prepared`.
    ///
    /// Added in git 2.55, this opens every transaction and carries the same update lines
    /// on standard input. The gate stands aside here and decides at `Prepared` instead,
    /// where the proposed transaction is complete and its references are resolved: an
    /// unborn branch arrives as `HEAD` while preparing and as its concrete ref once
    /// prepared. A non-zero exit does abort the transaction in this phase, so standing
    /// aside is a decision rather than a formality.
    Preparing,
    /// Git is asking hooks to approve the complete proposed transaction.
    Prepared,
    /// Git reports that an already-approved transaction committed.
    Committed,
    /// Git reports that an already-approved transaction aborted.
    Aborted,
    /// A lifecycle word this version of berth does not know.
    ///
    /// The gate permits and stands aside whenever it cannot function, so an unknown phase
    /// is a no-op. Failing to parse it would exit non-zero, which git reads as a rejection
    /// and turns into an aborted ref update — a future phase would brick the repository.
    Unrecognized,
}

/// One complete semantic reference transaction, including every proposed update.
#[derive(Clone)]
pub(crate) struct ReferenceTransaction {
    /// The lifecycle point at which git invoked the hook.
    phase:   ReferenceTransactionPhase,
    /// Every line supplied on standard input, in git's transaction order.
    entries: Vec<ReferenceTransactionEntry>,
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

/// Whether a committed transaction deleted the trunk ref embedded in the managed hook.
pub(crate) enum ManagedTrunkDeletion {
    /// The transaction did not commit a deletion of the embedded trunk ref.
    NotDeleted,
    /// The transaction deleted this embedded ref from its previous object tip.
    Deleted {
        reference:    FullRefName,
        previous_tip: GitObjectId,
    },
}

/// One parsed old-object, new-object, and full-reference update.
#[derive(Clone)]
pub(super) struct ReferenceUpdate {
    /// The object currently named by the ref, or git's all-zero absence marker.
    pub(super) previous:  ReferenceObject,
    /// The object the transaction proposes, or git's all-zero deletion marker.
    pub(super) proposed:  ReferenceObject,
    /// The full `refs/...` name being updated.
    pub(super) reference: FullRefName,
}

/// A real git object or the all-zero sentinel used at reference boundaries.
#[derive(Clone, Eq, PartialEq)]
pub(super) enum ReferenceObject {
    Object(GitObjectId),
    Symbolic(FullRefName),
    Absent,
}

/// Whether an update changes the logical value named by its reference.
enum ReferenceMovement<'transaction> {
    Move(ReferenceMove<'transaction>),
    NoMove,
}

/// A reference update that changes its logical value rather than its storage.
#[derive(Clone, Copy)]
struct ReferenceMove<'transaction>(&'transaction ReferenceUpdate);

impl<'transaction> ReferenceMovement<'transaction> {
    fn classify(update: &'transaction ReferenceUpdate, objects: &OperationObjectReader) -> Self {
        if update.previous == update.proposed {
            return Self::NoMove;
        }
        // `ReferenceMovement::classify` ignores packing's new storage entry when the ref already
        // names this tip.
        if let (ReferenceObject::Absent, ReferenceObject::Object(proposed)) =
            (&update.previous, &update.proposed)
            && objects
                .commit(update.reference.as_str())
                .is_ok_and(|current_git_object_id| current_git_object_id == *proposed)
        {
            return Self::NoMove;
        }
        Self::Move(ReferenceMove(update))
    }
}

enum SelectedGateUpdate {
    Evaluate(ProposedTargetMove),
    StandAside(NoGatedUpdate),
}

struct NoGatedUpdate;

#[derive(Clone)]
pub(super) enum PreviousTargetTip {
    Existing(GitObjectId),
    Absent,
}

#[derive(Clone)]
pub(crate) struct ProposedTargetMove {
    pub(crate) target:   IntegrationTarget,
    pub(super) previous: PreviousTargetTip,
    pub(crate) proposed: GitObjectId,
}

/// The checkout that invoked the managed hook, if it reported one.
#[derive(Clone, Copy)]
pub(crate) enum IssuingCheckout<'a> {
    Reported(&'a Path),
    Unreported,
}

/// Parse every stdin line into one semantic git reference transaction.
pub(crate) fn parse_reference_transaction(
    reference_transaction_phase: ReferenceTransactionPhase,
    input: &str,
) -> Result<ReferenceTransaction, ReferenceTransactionParseError> {
    let entries = input
        .lines()
        .enumerate()
        .map(|(index, line)| parse_reference_update(index + 1, line))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ReferenceTransaction {
        phase: reference_transaction_phase,
        entries,
    })
}

impl ReferenceTransaction {
    fn local_branch_updates(&self) -> Vec<&ReferenceUpdate> {
        self.entries
            .iter()
            .filter_map(
                |reference_transaction_entry| match reference_transaction_entry {
                    ReferenceTransactionEntry::LocalBranch(update) => Some(update),
                    ReferenceTransactionEntry::OutsideLocalBranchNamespace => None,
                },
            )
            .collect()
    }

    /// Classify whether this transaction includes the configured trunk reference.
    pub(crate) fn trunk_reference_presence(
        &self,
        trunk_reference: &FullRefName,
    ) -> TrunkReferencePresence {
        if self.entries.iter().any(|reference_transaction_entry| {
            matches!(
                reference_transaction_entry,
                ReferenceTransactionEntry::LocalBranch(update)
                    if &update.reference == trunk_reference
            )
        }) {
            TrunkReferencePresence::Named
        } else {
            TrunkReferencePresence::NotNamed
        }
    }

    /// Report a committed deletion of the trunk ref embedded in the managed hook.
    pub(crate) fn managed_trunk_deletion(
        &self,
        trunk_reference: &FullRefName,
    ) -> ManagedTrunkDeletion {
        if self.phase != ReferenceTransactionPhase::Committed {
            return ManagedTrunkDeletion::NotDeleted;
        }
        self.entries
            .iter()
            .find_map(
                |reference_transaction_entry| match reference_transaction_entry {
                    ReferenceTransactionEntry::LocalBranch(update)
                        if &update.reference == trunk_reference
                            && matches!(&update.proposed, ReferenceObject::Absent) =>
                    {
                        match &update.previous {
                            ReferenceObject::Object(previous_tip) => {
                                Some(ManagedTrunkDeletion::Deleted {
                                    reference:    update.reference.clone(),
                                    previous_tip: previous_tip.clone(),
                                })
                            },
                            ReferenceObject::Symbolic(_) | ReferenceObject::Absent => None,
                        }
                    },
                    ReferenceTransactionEntry::LocalBranch(_)
                    | ReferenceTransactionEntry::OutsideLocalBranchNamespace => None,
                },
            )
            .unwrap_or(ManagedTrunkDeletion::NotDeleted)
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

/// Evaluate prepared target updates and commit approved permit audits after Git moves the ref.
pub(crate) fn evaluate_reference_transaction(
    invocation_directory: &Path,
    issuing_checkout: IssuingCheckout<'_>,
    transaction: &ReferenceTransaction,
    trunk_reference: &FullRefName,
) -> Result<Vec<GateResult>, GateError> {
    // `evaluate_reference_transaction` stands aside before reading any configuration or ledger.
    // Git 2.55 opens every transaction with `preparing`, so the gate would otherwise pay for
    // the whole evaluation twice per ref update to reach a no-op both times.
    if matches!(
        transaction.phase,
        ReferenceTransactionPhase::Preparing
            | ReferenceTransactionPhase::Aborted
            | ReferenceTransactionPhase::Unrecognized
    ) {
        return Ok(Vec::new());
    }
    // `REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT` carries the issuing checkout
    // exported by the managed hook before it changes directory. A hook that did not is not
    // one this binary installed, and there is no fallback to the process's working directory.
    let issuing_directory = match issuing_checkout {
        IssuingCheckout::Reported(directory) => directory,
        IssuingCheckout::Unreported => return Err(GateError::HookReportedNoIssuingDirectory),
    };
    let local_branch_updates = transaction.local_branch_updates();
    if transaction.phase == ReferenceTransactionPhase::Prepared
        && local_branch_updates
            .iter()
            .all(|update| update.previous == update.proposed)
    {
        return Ok(Vec::new());
    }
    // `rewrite::branch_rewrites` asks the cheap ancestry question before discovering the
    // worktree or reading any configuration. An ordinary fast-forward then needs only the
    // target filter before it can stop without opening the ledger.
    let rewrites = if transaction.phase == ReferenceTransactionPhase::Committed {
        rewrite::branch_rewrites(issuing_directory, &local_branch_updates)?
    } else {
        Vec::new()
    };
    if local_branch_updates.is_empty() && rewrites.is_empty() {
        return Ok(Vec::new());
    }
    let worktree_context = WorktreeContext::discover(invocation_directory)?;
    let target_updates = if transaction.phase == ReferenceTransactionPhase::Committed {
        committed_target_candidates(local_branch_updates, &worktree_context, trunk_reference)?
    } else {
        local_branch_updates
    };
    if target_updates.is_empty() && rewrites.is_empty() {
        return Ok(Vec::new());
    }
    let berth_config = match BerthConfig::read(&worktree_context.configuration_lookup())? {
        Enrollment::Enrolled(berth_config) => berth_config,
        Enrollment::Unconfigured { .. } => return Ok(Vec::new()),
    };
    let repository = git::RepositorySnapshot::new(worktree_context);
    let objects = repository.object_reader();
    let movements = target_updates
        .into_iter()
        .filter_map(
            |update| match ReferenceMovement::classify(update, &objects) {
                ReferenceMovement::Move(movement) => Some(movement),
                ReferenceMovement::NoMove => None,
            },
        )
        .collect::<Vec<_>>();
    if transaction.phase == ReferenceTransactionPhase::Prepared {
        return evaluate_prepared_updates(
            &repository,
            &berth_config,
            &movements,
            trunk_reference,
            issuing_directory,
        );
    }

    let worktree_context = repository.context();
    let reservations = read_gate_reservations(worktree_context)?;
    let gated_updates = gated_updates(&movements, reservations.iter(), trunk_reference);
    let rewrite_events = rewrite::capture_branch_rewrites(issuing_directory, &rewrites)?;
    for movement in gated_updates {
        if let SelectedGateUpdate::Evaluate(update) = movement.select_target()? {
            audit::commit_forced_permit_audits(
                worktree_context,
                &berth_config,
                &update,
                issuing_directory,
            )?;
        }
    }
    if !rewrite_events.is_empty() {
        rewrite::reanchor_rewritten_phases(worktree_context, &rewrite_events)?;
    }
    Ok(Vec::new())
}

fn evaluate_prepared_updates(
    repository: &RepositorySnapshot,
    berth_config: &BerthConfig,
    movements: &[ReferenceMove<'_>],
    trunk_reference: &FullRefName,
    issuing_directory: &Path,
) -> Result<Vec<GateResult>, GateError> {
    // A proposed trunk commit always needs selection against the decision's replay.
    // Other transactions can stand aside after a read-only replay, before identity and locking.
    if !movements
        .iter()
        .any(|movement| movement.0.reference == *trunk_reference && movement.proposes_commit())
    {
        let reservations = read_gate_reservations(repository.context())?;
        if let SelectedGateUpdate::StandAside(_) =
            select_prepared_update(movements, reservations.iter(), trunk_reference)?
        {
            return Ok(Vec::new());
        }
    }
    let purpose = GatePurpose::Hook {
        phase:             ReferenceTransactionPhase::Prepared,
        issuing_directory: issuing_directory.to_path_buf(),
    };
    let result = decision::evaluate_updates(repository, berth_config, &purpose, |state, _| {
        let reservations = state.reservations().map_err(|error| {
            GatePreparationRejection::Failed(GateError::Ledger(LedgerError::ReservationReplay(
                error,
            )))
        })?;
        match select_prepared_update(movements, reservations.iter(), trunk_reference)
            .map_err(GatePreparationRejection::Failed)?
        {
            SelectedGateUpdate::Evaluate(update) => Ok(update),
            SelectedGateUpdate::StandAside(reason) => {
                Err(GatePreparationRejection::NoGatedUpdate(reason))
            },
        }
    })?;
    match result {
        Ok(result) => Ok(vec![result]),
        Err(NoGatedUpdate) => Ok(Vec::new()),
    }
}

fn read_gate_reservations(context: &WorktreeContext) -> Result<Vec<Reservation>, GateError> {
    Ledger::open_from_discovered_worktree(context)?
        .read_validated_live_reservations()?
        .map_err(|error| GateError::Ledger(LedgerError::ReservationReplay(error)))
}

fn committed_target_candidates<'transaction>(
    updates: Vec<&'transaction ReferenceUpdate>,
    worktree_context: &WorktreeContext,
    trunk_reference: &FullRefName,
) -> Result<Vec<&'transaction ReferenceUpdate>, GateError> {
    let gate_targets_path = worktree_context
        .ledger_directory()
        .join(GATE_TARGETS_FILE_NAME);
    let gate_targets = match fs::read_to_string(gate_targets_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(GateError::Ledger(LedgerError::Io(error))),
    };
    Ok(updates
        .into_iter()
        .filter(|update| {
            update.reference == *trunk_reference
                || gate_targets
                    .lines()
                    .any(|reference| reference == update.reference.as_str())
        })
        .collect())
}

fn gated_updates<'transaction, 'reservation>(
    movements: &[ReferenceMove<'transaction>],
    reservations: impl Iterator<Item = &'reservation Reservation>,
    trunk_reference: &FullRefName,
) -> Vec<ReferenceMove<'transaction>> {
    let gated_references = reservations
        .filter(|reservation| {
            !matches!(
                reservation.lifecycle(),
                ReservationLifecycle::Released { .. }
            )
        })
        .filter_map(|reservation| match reservation.target() {
            RecordedTarget::Recorded { target, .. } => Some(target.reference()),
            RecordedTarget::Unrecorded => None,
        })
        .chain(std::iter::once(trunk_reference))
        .collect::<HashSet<_>>();
    movements
        .iter()
        .copied()
        .filter(|ReferenceMove(update)| gated_references.contains(&update.reference))
        .collect()
}

fn select_prepared_update<'reservation>(
    movements: &[ReferenceMove<'_>],
    reservations: impl Iterator<Item = &'reservation Reservation>,
    trunk_reference: &FullRefName,
) -> Result<SelectedGateUpdate, GateError> {
    let gated = gated_updates(movements, reservations, trunk_reference);
    match gated.as_slice() {
        [] => Ok(SelectedGateUpdate::StandAside(NoGatedUpdate)),
        [movement] => movement.select_target(),
        _ => Err(GateError::MultipleGatedReferences(
            gated
                .into_iter()
                .map(|ReferenceMove(update)| update.reference.clone())
                .collect(),
        )),
    }
}

impl ReferenceMove<'_> {
    const fn proposes_commit(self) -> bool {
        matches!(
            (&self.0.previous, &self.0.proposed),
            (
                ReferenceObject::Object(_) | ReferenceObject::Absent,
                ReferenceObject::Object(_)
            )
        )
    }

    fn select_target(self) -> Result<SelectedGateUpdate, GateError> {
        let Self(update) = self;
        let target = IntegrationTarget::from_branch_argument(update.reference.as_str())
            .map_err(|reason| LedgerError::InvalidRepositoryTrunk(reason.to_string()))?;
        match (&update.previous, &update.proposed) {
            (ReferenceObject::Object(previous), ReferenceObject::Object(proposed))
                if previous != proposed =>
            {
                Ok(SelectedGateUpdate::Evaluate(ProposedTargetMove {
                    target,
                    previous: PreviousTargetTip::Existing(previous.clone()),
                    proposed: proposed.clone(),
                }))
            },
            (ReferenceObject::Absent, ReferenceObject::Object(proposed)) => {
                Ok(SelectedGateUpdate::Evaluate(ProposedTargetMove {
                    target,
                    previous: PreviousTargetTip::Absent,
                    proposed: proposed.clone(),
                }))
            },
            (ReferenceObject::Object(_) | ReferenceObject::Absent, ReferenceObject::Absent)
            | (ReferenceObject::Object(_), ReferenceObject::Object(_)) => {
                Ok(SelectedGateUpdate::StandAside(NoGatedUpdate))
            },
            (ReferenceObject::Symbolic(previous), ReferenceObject::Symbolic(proposed))
                if previous == proposed =>
            {
                Ok(SelectedGateUpdate::StandAside(NoGatedUpdate))
            },
            (ReferenceObject::Symbolic(_), _) | (_, ReferenceObject::Symbolic(_)) => {
                Err(GateError::UnsupportedSymbolicTargetUpdate(target))
            },
        }
    }
}

impl FromStr for ReferenceTransactionPhase {
    type Err = Infallible;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "committed" => Ok(Self::Committed),
            "aborted" => Ok(Self::Aborted),
            "preparing" => Ok(Self::Preparing),
            _ => Ok(Self::Unrecognized),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::ReferenceTransactionPhase;

    #[test]
    fn an_unknown_reference_transaction_phase_parses_instead_of_aborting_the_update() {
        for known in ["prepared", "committed", "aborted", "preparing"] {
            assert_ne!(
                known.parse::<ReferenceTransactionPhase>(),
                Ok(ReferenceTransactionPhase::Unrecognized)
            );
        }
        assert_eq!(
            "a-phase-git-has-not-invented-yet".parse::<ReferenceTransactionPhase>(),
            Ok(ReferenceTransactionPhase::Unrecognized),
            "a phase berth cannot parse exits non-zero, which git turns into an aborted update"
        );
    }

    /// Git 2.55 opens every transaction with this phase, and rejecting there aborts it.
    #[test]
    fn the_preparing_phase_is_named_rather_than_merely_tolerated() {
        assert_eq!(
            "preparing".parse::<ReferenceTransactionPhase>(),
            Ok(ReferenceTransactionPhase::Preparing),
            "berth decides at prepared, where the transaction is complete and refs resolved"
        );
    }
}
