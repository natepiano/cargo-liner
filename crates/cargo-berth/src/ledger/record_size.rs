//! Separate caller declarations from the repository evidence accompanying them.

use std::collections::BTreeSet;

use serde::Serialize;

use super::constants::MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES;
use super::constants::MAXIMUM_JOURNAL_RECORD_BYTES;
use super::journal::ClaimSource;
use super::journal::ExplicitWidenReason;
use super::journal::ForcedIntegrationReason;
use super::journal::JournalAppendError;
use super::journal::JournalEvent;
use super::journal::JournalOperation;
use super::journal::RecordCeiling;
use super::journal::ReleaseSource;
use super::journal::ReservationPurpose;
use super::journal::ReservationScopeAdditionSet;
use super::journal::ReservationScopeSet;
use super::journal::WidenCause;
use super::target::IntegrationTarget;
use crate::answer::ClaimSequenceBinding;
use crate::answer::ConflictAuthorization;
use crate::edge::OrderingReason;
use crate::reservation::ReleaseDisposition;

/// Only declarations a caller can shorten, without any measured scopes or holder bindings.
///
/// This is an append-time budget projection, never a journal format. The exhaustive conversion
/// decides which input an operation carries; the caller limit cannot accept a journal event.
#[derive(Serialize)]
#[serde(tag = "input", rename_all = "snake_case")]
pub(super) enum CallerJournalInput<'input> {
    /// This operation records only facts the engine obtained from existing state.
    None,
    /// Initial scope, provenance and purpose, plus the explanation of an overlap answer.
    Claim {
        scopes:  &'input ReservationScopeSet,
        source:  &'input ClaimSource,
        purpose: &'input ReservationPurpose,
        reasons: BTreeSet<String>,
    },
    /// Only explicit additions are declarations; a drift's measured additions are not.
    Widen {
        scopes:  &'input ReservationScopeAdditionSet,
        reason:  &'input ExplicitWidenReason,
        reasons: BTreeSet<String>,
    },
    /// An answer repeats one explanation per observed holder on disk, but the caller gave it once.
    Answer {
        scopes:  &'input ReservationScopeSet,
        reasons: BTreeSet<String>,
    },
    /// A refused check or claim still has a caller-declared request, independently of its holders.
    ScopeRequest { scopes: &'input ReservationScopeSet },
    /// The explanation of a newly declared ordering; its covered scopes come from the ledger.
    Ordering { reason: &'input OrderingReason },
    /// The reason for granting a permit; its skipped holds come from the ordering graph.
    ForcedIntegration {
        reason: &'input ForcedIntegrationReason,
    },
    /// The new user explanation, never a superseded disposition copied from the ledger.
    Release { reason: String },
    /// The new integration branch named by a retarget command.
    Retarget { target: &'input IntegrationTarget },
}

impl<'input> From<&'input JournalOperation> for CallerJournalInput<'input> {
    fn from(operation: &'input JournalOperation) -> Self {
        match operation {
            JournalOperation::Claim {
                scopes,
                source:
                    source @ (ClaimSource::Explicit
                    | ClaimSource::FirstTouch
                    | ClaimSource::WorkPlan { .. }),
                purpose,
                authorization,
                ..
            } => Self::Claim {
                scopes,
                source,
                purpose,
                reasons: overlap_reasons(authorization),
            },
            JournalOperation::Widen {
                added_scopes,
                cause: WidenCause::Explicit { reason },
                authorization,
                ..
            } => Self::Widen {
                scopes: added_scopes,
                reason,
                reasons: overlap_reasons(authorization),
            },
            JournalOperation::Answer {
                scopes,
                authorizations,
                ..
            } => Self::Answer {
                scopes,
                reasons: authorizations.iter().flat_map(overlap_reasons).collect(),
            },
            JournalOperation::OverlapRefused { scopes, .. } => Self::ScopeRequest { scopes },
            JournalOperation::ResolveDefer { reason, .. }
            | JournalOperation::Sequence { reason, .. } => Self::Ordering { reason },
            JournalOperation::ForcedIntegrationPermit { reason, .. } => {
                Self::ForcedIntegration { reason }
            },
            JournalOperation::Release {
                disposition,
                source: Some(ReleaseSource::Command),
                ..
            }
            | JournalOperation::ReplaceReleaseDisposition {
                replacement: disposition,
                ..
            } => release_input(disposition),
            JournalOperation::Retarget { target, .. } => Self::Retarget { target },
            JournalOperation::Claim {
                source: ClaimSource::Enrolled | ClaimSource::Cover { .. },
                ..
            }
            | JournalOperation::MergeExtentObserved { .. }
            | JournalOperation::HolderMergeExtentObserved { .. }
            | JournalOperation::Widen {
                cause: WidenCause::Drift,
                ..
            }
            | JournalOperation::Checkpoint { .. }
            | JournalOperation::Resnapshot { .. }
            | JournalOperation::UnrecordedTargetsPinned { .. }
            | JournalOperation::Renew { .. }
            | JournalOperation::Release {
                source: None | Some(ReleaseSource::Reconciliation),
                ..
            }
            | JournalOperation::EvidenceRevalidated { .. }
            | JournalOperation::ScopedPatchEquivalenceChecked { .. }
            | JournalOperation::ScopedPatchComparisonAttempted { .. }
            | JournalOperation::SuccessorScopedPatchEquivalenceChecked { .. }
            | JournalOperation::SuccessorScopedPatchComparisonAttempted { .. }
            | JournalOperation::RetireCyclicOrderingEdge { .. }
            | JournalOperation::Incursion { .. }
            | JournalOperation::ResolveIncursion { .. }
            | JournalOperation::ConsumeForcedIntegrationPermit { .. }
            | JournalOperation::Bypass { .. }
            | JournalOperation::RebindWorktree { .. }
            | JournalOperation::RelocateWorktree { .. } => Self::None,
        }
    }
}

impl CallerJournalInput<'_> {
    /// The 16 KiB check accepts only this projection, never a serialized journal record.
    pub(super) fn validate(&self) -> Result<(), JournalAppendError> {
        let bytes = serde_json::to_vec(self)
            .map_err(JournalAppendError::Serialization)?
            .len();
        if bytes > MAXIMUM_JOURNAL_RECORD_BYTES {
            return Err(JournalAppendError::RecordTooLarge {
                bytes,
                ceiling: RecordCeiling::Declared,
            });
        }
        Ok(())
    }
}

fn release_input(disposition: &ReleaseDisposition) -> CallerJournalInput<'_> {
    match disposition {
        ReleaseDisposition::Abandoned(reason) => CallerJournalInput::Release {
            reason: reason.to_string(),
        },
        ReleaseDisposition::RetiredOrphan(reason) => CallerJournalInput::Release {
            reason: reason.to_string(),
        },
        ReleaseDisposition::Integrated
        | ReleaseDisposition::RewrittenIntegration(_)
        | ReleaseDisposition::UnchangedScopes(_) => CallerJournalInput::None,
    }
}

fn overlap_reasons(authorization: &ConflictAuthorization) -> BTreeSet<String> {
    match authorization {
        ConflictAuthorization::Sequence { reason, .. }
        | ConflictAuthorization::SequencePerHolder { reason, .. }
        | ConflictAuthorization::Defer { reason, .. }
        | ConflictAuthorization::Override { reason, .. } => BTreeSet::from([reason.to_string()]),
        ConflictAuthorization::SequenceWithExistingEdges { holders, .. } => holders
            .iter()
            .filter_map(|binding| match binding {
                ClaimSequenceBinding::NewEdge { reason, .. } => Some(reason.to_string()),
                ClaimSequenceBinding::ExistingEdge { .. } => None,
            })
            .collect(),
        ConflictAuthorization::NoConflict
        | ConflictAuthorization::Enrollment { .. }
        | ConflictAuthorization::ExistingAnswersCoverEveryOverlap { .. } => BTreeSet::new(),
    }
}

/// The complete encoded journal fact, including all engine-derived evidence and its envelope.
///
/// It has its own budget and cannot be passed to the caller-input check. Its bytes and replay
/// format are identical to existing journal records; only admission at append time changes.
pub(super) struct DerivedJournalRecord(Vec<u8>);

impl DerivedJournalRecord {
    pub(super) fn encode(event: &JournalEvent) -> Result<Self, JournalAppendError> {
        let mut bytes = serde_json::to_vec(event).map_err(JournalAppendError::Serialization)?;
        bytes.push(b'\n');
        if bytes.len() > MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES {
            return Err(JournalAppendError::RecordTooLarge {
                bytes:   bytes.len(),
                ceiling: RecordCeiling::Derived,
            });
        }
        Ok(Self(bytes))
    }

    pub(super) fn into_bytes(self) -> Vec<u8> { self.0 }
}
