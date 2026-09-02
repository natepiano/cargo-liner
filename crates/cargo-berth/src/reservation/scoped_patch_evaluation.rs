//! Durable scoped patch verdicts and the schedule that decides what to compare next.
//!
//! A scoped patch comparison is expensive, so its verdict is retained against the immutable
//! inputs that produced it: the proof subject revision and the target commit. When either
//! changes the retained verdict no longer applies. Alongside the verdicts sits a bounded
//! round-robin schedule recording which subject-and-target pairs have already been attempted,
//! so an unvisited target sorts ahead of a retried transient failure.

use std::collections::VecDeque;

use serde::Deserialize;
use serde::Serialize;

use super::constants::SCOPED_PATCH_TARGET_RETENTION_LIMIT;
use super::constants::SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT;
use crate::ids::GitObjectId;
use crate::ids::ProjectionGeneration;

/// The version of the baseline, protected content, and scopes used by a scoped proof.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub(crate) struct IntegrationProofSubjectRevision(pub(super) u64);

impl IntegrationProofSubjectRevision {
    pub(super) const INITIAL: Self = Self(1);
}

declare_wire_enum! {
    /// A definitive content verdict produced by scoped patch equivalence.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum ScopedPatchEquivalenceVerdict {
        /// The target contains the protected scoped change.
        Integrated => "integrated";
        /// The target does not contain an outstanding protected scoped change.
        NotIntegrated => "not_integrated";
        /// The target no longer contains a previously integrated scoped change.
        TrunkRewritten => "trunk_rewritten";
    }
}

/// An immutable scoped patch result that can be reused under a later integration context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DurableScopedPatchComparison {
    /// The target contains the protected scoped change.
    Equivalent,
    /// The target does not contain the protected scoped change.
    Different,
}

impl From<ScopedPatchEquivalenceVerdict> for DurableScopedPatchComparison {
    fn from(verdict: ScopedPatchEquivalenceVerdict) -> Self {
        match verdict {
            ScopedPatchEquivalenceVerdict::Integrated => Self::Equivalent,
            ScopedPatchEquivalenceVerdict::NotIntegrated
            | ScopedPatchEquivalenceVerdict::TrunkRewritten => Self::Different,
        }
    }
}

/// One definitive scoped patch verdict retained for an immutable target.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedScopedPatchTargetVerdict {
    subject: IntegrationProofSubjectRevision,
    target:  GitObjectId,
    verdict: ScopedPatchEquivalenceVerdict,
}

/// Durable scoped patch verdicts retained for the most recently recorded reconciliation targets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedScopedPatchTargetVerdicts {
    entries: VecDeque<RetainedScopedPatchTargetVerdict>,
}

/// Whether a retained scoped patch verdict applies to one requested subject and target.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScopedPatchTargetVerdictAvailability {
    /// The stored subject and target match the request.
    Hit(DurableScopedPatchComparison),
    /// No stored verdict applies to the request.
    Miss,
}

declare_wire_enum! {
    /// A definitive successor-incorporation verdict produced by scoped patch equivalence.
    #[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub(crate) enum SuccessorScopedPatchEquivalenceVerdict {
        /// The successor head contains the predecessor's protected scoped change.
        Equivalent => "equivalent";
        /// The successor head does not contain the predecessor's protected scoped change.
        Different => "different";
    }
}

/// One definitive successor-incorporation verdict retained for an immutable head.
#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedSuccessorScopedPatchTargetVerdict {
    subject:        IntegrationProofSubjectRevision,
    successor_head: GitObjectId,
    verdict:        SuccessorScopedPatchEquivalenceVerdict,
}

/// Durable scoped patch verdicts retained for recently observed successor heads.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct RetainedSuccessorScopedPatchTargetVerdicts {
    entries: VecDeque<RetainedSuccessorScopedPatchTargetVerdict>,
}

/// Whether a retained successor-incorporation verdict applies to one proof subject and head.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SuccessorScopedPatchTargetVerdictAvailability {
    /// The stored proof subject and successor head match the request.
    Hit(SuccessorScopedPatchEquivalenceVerdict),
    /// No stored successor-incorporation verdict applies to the request.
    Miss,
}

/// The scheduling order for scoped comparisons without a retained verdict at one trunk target.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ScopedPatchEvaluationPriority {
    /// This proof subject has not been compared with the target.
    NotAttempted,
    /// This generation last compared the proof subject with the target.
    LastAttemptedAt(ProjectionGeneration),
}

/// One comparison attempt retained for target-specific round-robin scheduling.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ScopedPatchComparisonAttempt {
    subject:    IntegrationProofSubjectRevision,
    target:     GitObjectId,
    generation: ProjectionGeneration,
}

/// The bounded evaluation schedule for the most recently recorded reconciliation targets.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ScopedPatchTargetEvaluationSchedule {
    entries: VecDeque<ScopedPatchComparisonAttempt>,
}

impl ScopedPatchTargetEvaluationSchedule {
    pub(super) fn priority(
        &self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        for attempt in &self.entries {
            if attempt.subject == subject && attempt.target == *target {
                return ScopedPatchEvaluationPriority::LastAttemptedAt(attempt.generation);
            }
        }
        ScopedPatchEvaluationPriority::NotAttempted
    }

    pub(super) fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        generation: ProjectionGeneration,
    ) {
        self.entries
            .retain(|attempt| attempt.subject != subject || attempt.target != *target);
        if self.entries.len() == SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(ScopedPatchComparisonAttempt {
            subject,
            target: target.clone(),
            generation,
        });
    }
}

/// Attempt generations for recent successor heads under the current proof subject.
///
/// The retention limit matches the retained successor verdict limit, so an unvisited retained
/// head sorts ahead of retried transient failures. Recording a new proof subject removes every
/// superseded subject before applying that limit.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct SuccessorScopedPatchTargetEvaluationSchedule {
    entries: VecDeque<ScopedPatchComparisonAttempt>,
}

impl SuccessorScopedPatchTargetEvaluationSchedule {
    pub(super) fn priority(
        &self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
    ) -> ScopedPatchEvaluationPriority {
        for attempt in &self.entries {
            if attempt.subject == subject && attempt.target == *successor_head {
                return ScopedPatchEvaluationPriority::LastAttemptedAt(attempt.generation);
            }
        }
        ScopedPatchEvaluationPriority::NotAttempted
    }

    pub(super) fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        generation: ProjectionGeneration,
    ) {
        self.entries
            .retain(|attempt| attempt.subject == subject && attempt.target != *successor_head);
        if self.entries.len() == SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(ScopedPatchComparisonAttempt {
            subject,
            target: successor_head.clone(),
            generation,
        });
    }
}

impl RetainedScopedPatchTargetVerdicts {
    /// Look up a verdict only when both immutable proof inputs match.
    pub(crate) fn lookup(
        &self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
    ) -> ScopedPatchTargetVerdictAvailability {
        for entry in &self.entries {
            if entry.subject == subject && entry.target == *target {
                return ScopedPatchTargetVerdictAvailability::Hit(entry.verdict.into());
            }
        }
        ScopedPatchTargetVerdictAvailability::Miss
    }

    pub(super) fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        target: &GitObjectId,
        verdict: ScopedPatchEquivalenceVerdict,
    ) {
        self.entries
            .retain(|entry| entry.subject != subject || entry.target != *target);
        if self.entries.len() == SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries.push_back(RetainedScopedPatchTargetVerdict {
            subject,
            target: target.clone(),
            verdict,
        });
    }
}

impl RetainedSuccessorScopedPatchTargetVerdicts {
    /// Look up a verdict only when both immutable successor-proof inputs match.
    pub(crate) fn lookup(
        &self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
    ) -> SuccessorScopedPatchTargetVerdictAvailability {
        for entry in &self.entries {
            if entry.subject == subject && entry.successor_head == *successor_head {
                return SuccessorScopedPatchTargetVerdictAvailability::Hit(entry.verdict);
            }
        }
        SuccessorScopedPatchTargetVerdictAvailability::Miss
    }

    pub(super) fn record(
        &mut self,
        subject: IntegrationProofSubjectRevision,
        successor_head: &GitObjectId,
        verdict: SuccessorScopedPatchEquivalenceVerdict,
    ) {
        self.entries
            .retain(|entry| entry.subject != subject || entry.successor_head != *successor_head);
        if self.entries.len() == SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT {
            std::mem::drop(self.entries.pop_front());
        }
        self.entries
            .push_back(RetainedSuccessorScopedPatchTargetVerdict {
                subject,
                successor_head: successor_head.clone(),
                verdict,
            });
    }
}

#[cfg(test)]
mod tests {
    use super::IntegrationProofSubjectRevision;
    use super::SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT;
    use super::ScopedPatchEvaluationPriority;
    use super::SuccessorScopedPatchTargetEvaluationSchedule;
    use crate::ids::GitObjectId;
    use crate::ids::ProjectionGeneration;

    const TRUNK_OID: &str = "1111111111111111111111111111111111111111";

    #[test]
    fn successor_scoped_patch_schedule_retains_only_bounded_current_subject_heads()
    -> Result<(), Box<dyn std::error::Error>> {
        let superseded_subject = IntegrationProofSubjectRevision::INITIAL;
        let current_subject = IntegrationProofSubjectRevision(2);
        let superseded_head = TRUNK_OID.parse::<GitObjectId>()?;
        let generation = ProjectionGeneration::from(3);
        let mut evaluation_schedule = SuccessorScopedPatchTargetEvaluationSchedule::default();
        evaluation_schedule.record(superseded_subject, &superseded_head, generation);

        for successor_number in 1..=SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT + 1 {
            let successor_head = format!("{successor_number:040x}").parse::<GitObjectId>()?;
            evaluation_schedule.record(current_subject, &successor_head, generation);
        }

        let evicted_head = format!("{:040x}", 1).parse::<GitObjectId>()?;
        let oldest_retained_head = format!("{:040x}", 2).parse::<GitObjectId>()?;
        assert_eq!(
            evaluation_schedule.entries.len(),
            SUCCESSOR_SCOPED_PATCH_TARGET_RETENTION_LIMIT
        );
        assert_eq!(
            evaluation_schedule.priority(superseded_subject, &superseded_head),
            ScopedPatchEvaluationPriority::NotAttempted
        );
        assert_eq!(
            evaluation_schedule.priority(current_subject, &evicted_head),
            ScopedPatchEvaluationPriority::NotAttempted
        );
        assert_eq!(
            evaluation_schedule.priority(current_subject, &oldest_retained_head),
            ScopedPatchEvaluationPriority::LastAttemptedAt(generation)
        );
        Ok(())
    }
}
