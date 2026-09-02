//! Git reference-transaction parsing and the hook's trunk-update evaluation.

use std::convert::Infallible;
use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use super::audit::commit_forced_permit_audits;
use super::decision::GatePurpose;
use super::decision::GateResult;
use super::decision::evaluate_locked;
use super::error::GateError;
use super::rewrite::branch_rewrites;
use super::rewrite::reanchor_rewritten_phases;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::git;
use crate::ids::GitObjectId;
use crate::ledger::FullRefName;
use crate::ledger::WorktreeContext;

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

/// Whether the managed hook preserved the checkout that issued this transaction.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReferenceTransactionIssuingDirectory {
    /// A current managed hook exported its issuing checkout before changing directory.
    CapturedByManagedHook(PathBuf),
    /// A managed hook installed before issuing-directory capture exported no checkout.
    MissingFromLegacyHook,
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
#[derive(Clone)]
pub(super) enum ReferenceObject {
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
pub(super) enum PreviousMain {
    Existing(GitObjectId),
    Absent,
}

#[derive(Clone)]
pub(super) struct ProposedMainMove {
    pub(super) previous: PreviousMain,
    pub(super) proposed: GitObjectId,
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
            .find_map(|entry| match entry {
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
            })
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

/// Evaluate prepared trunk updates and commit their approved permit audits after Git moves the ref.
pub(crate) fn evaluate_reference_transaction(
    invocation_directory: &Path,
    issuing_directory: &ReferenceTransactionIssuingDirectory,
    transaction: &ReferenceTransaction,
    trunk_reference: &FullRefName,
) -> Result<Vec<GateResult>, GateError> {
    // Stand aside before reading any configuration or ledger. Git 2.55 opens every
    // transaction with `preparing`, so the gate would otherwise pay for the whole
    // evaluation twice per ref update to reach a no-op both times.
    if matches!(
        transaction.phase,
        ReferenceTransactionPhase::Preparing
            | ReferenceTransactionPhase::Aborted
            | ReferenceTransactionPhase::Unrecognized
    ) {
        return Ok(Vec::new());
    }
    let local_branch_updates = transaction
        .entries
        .iter()
        .filter_map(|entry| match entry {
            ReferenceTransactionEntry::LocalBranch(update) => Some(update),
            ReferenceTransactionEntry::OutsideLocalBranchNamespace => None,
        })
        .collect::<Vec<_>>();
    let trunk_updates = local_branch_updates
        .iter()
        .copied()
        .filter(|update| &update.reference == trunk_reference)
        .collect::<Vec<_>>();
    // Ask the cheap ancestry question before discovering the worktree or reading any
    // configuration. Every ordinary commit reaches here, and every ordinary commit is a
    // fast-forward, so the common case pays one `merge-base --is-ancestor` and stops.
    let rewrites = match transaction.phase {
        ReferenceTransactionPhase::Committed => {
            branch_rewrites(invocation_directory, &local_branch_updates)?
        },
        ReferenceTransactionPhase::Prepared
        | ReferenceTransactionPhase::Preparing
        | ReferenceTransactionPhase::Aborted
        | ReferenceTransactionPhase::Unrecognized => Vec::new(),
    };
    if trunk_updates.is_empty() && rewrites.is_empty() {
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
                                phase:             ReferenceTransactionPhase::Prepared,
                                issuing_directory: issuing_directory.clone(),
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
                        issuing_directory,
                    )?,
                    ReferenceTransactionPhase::Preparing
                    | ReferenceTransactionPhase::Aborted
                    | ReferenceTransactionPhase::Unrecognized => {},
                }
            },
            ReferenceUpdateGateSubject::NotMainEntry => {},
            ReferenceUpdateGateSubject::UnsupportedMainUpdate => {
                return Err(GateError::UnsupportedSymbolicTrunkUpdate);
            },
        }
    }
    if !rewrites.is_empty() {
        reanchor_rewritten_phases(invocation_directory, &worktree_context, &rewrites)?;
    }
    Ok(results)
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
