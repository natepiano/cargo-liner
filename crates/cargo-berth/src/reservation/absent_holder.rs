//! Verified integration of recorded work after its holder identity is absent.

use serde::Deserialize;
use serde::Serialize;

use super::MergeExtent;
use super::Reservation;
use super::RewrittenIntegrationTrunkCommit;
use super::merge_extent::RetainedMergeEvidence;
use crate::git::Reachability;
use crate::ids::GitObjectId;
use crate::worktree::HolderIdentityPresence;

/// The recorded facts that permit an integration without a checkpoint.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct AbsentHolderIntegration {
    /// The clean head in the last successful merge extent observation.
    pub(crate) work_head: GitObjectId,
    /// The trunk commit verified to contain that head.
    pub(crate) target:    RewrittenIntegrationTrunkCommit,
    /// The reason this recovery can omit the holder's checkpoint.
    reason:               AbsentHolderIntegrationReason,
}

/// The journaled integration retained independently of later merge observations.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) enum AbsentHolderIntegrationRecord {
    /// No checkpoint-free integration was recorded.
    #[default]
    NotRecorded,
    /// The journal verified the work head and its trunk witness.
    Verified(AbsentHolderIntegration),
}

/// The registry and ancestry facts needed for checkpoint-free integration.
pub(crate) struct AbsentHolderIntegrationChecks {
    /// The registry result for the previous holder identity.
    pub(crate) holder_presence:        HolderIdentityPresence,
    /// Whether the supplied witness contains the recorded work head.
    pub(crate) work_head_at_target:    Reachability,
    /// Whether the supplied witness contains any remaining claim branch tip.
    pub(crate) claim_branch_at_target: Reachability,
    /// Whether the repository trunk contains the supplied witness.
    pub(crate) target_at_trunk:        Reachability,
}

/// A refusal when the facts do not prove checkpoint-free integration.
pub(crate) struct AbsentHolderIntegrationRefusal;

impl AbsentHolderIntegration {
    /// Construct evidence only when absence and every required ancestry check succeed.
    pub(crate) fn verify(
        work_head: GitObjectId,
        target: RewrittenIntegrationTrunkCommit,
        checks: &AbsentHolderIntegrationChecks,
    ) -> Result<Self, AbsentHolderIntegrationRefusal> {
        if !matches!(checks.holder_presence, HolderIdentityPresence::Absent)
            || !matches!(checks.work_head_at_target, Reachability::Ancestor)
            || !matches!(checks.claim_branch_at_target, Reachability::Ancestor)
            || !matches!(checks.target_at_trunk, Reachability::Ancestor)
        {
            return Err(AbsentHolderIntegrationRefusal);
        }
        Ok(Self {
            work_head,
            target,
            reason: AbsentHolderIntegrationReason::HolderAbsentAndHeadIntegrated,
        })
    }
}

/// The verification that replaces a checkpoint for an absent holder.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum AbsentHolderIntegrationReason {
    /// The registry proves absence and the trunk commit contains the recorded work head.
    HolderAbsentAndHeadIntegrated,
}

/// Whether the journal identifies a clean work head after the claim.
pub(crate) enum RecordedWorkHead<'reservation> {
    /// A successful merge extent observation recorded this clean head.
    Clean(&'reservation GitObjectId),
    /// No clean head is recorded, or the last observation includes uncommitted work.
    Unverified,
}

impl Reservation {
    /// Read the last successful observation instead of the immutable claim head.
    pub(crate) const fn recorded_work_head(&self) -> RecordedWorkHead<'_> {
        let key = match self.merge_extent() {
            MergeExtent::Empty { key }
            | MergeExtent::Protected { key, .. }
            | MergeExtent::Unavailable {
                retained_evidence:
                    RetainedMergeEvidence::Empty { key } | RetainedMergeEvidence::Protected { key, .. },
                ..
            } => key,
            MergeExtent::NotDerived { .. }
            | MergeExtent::Unavailable {
                retained_evidence: RetainedMergeEvidence::NotDerived { .. },
                ..
            } => return RecordedWorkHead::Unverified,
        };
        if key.working_tree.tracked_paths.is_empty() && key.working_tree.untracked_paths.is_empty()
        {
            RecordedWorkHead::Clean(&key.head)
        } else {
            RecordedWorkHead::Unverified
        }
    }
}
