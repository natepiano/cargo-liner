//! What the declaration reading says about one finding, and the site evidence it files with.

use serde::Serialize;
use serde::Serializer;

use super::DeclarationInput;
use crate::declaration_source::StatementRefusal;
use crate::finding_anchor::FindingAnchor;
use crate::ledger_read::JudgingHistory;
use crate::original_site::OriginalSite;
use crate::original_site::SiteRefusal;
use crate::original_site::SourceCoordinate;
use crate::same_declaration::Declaration;

/// Where a finding's declaration is at the synced tip.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum DeclarationOutcome {
    #[serde(rename = "declaration_not_read")]
    NotRead,
    #[serde(rename = "declaration_in_place")]
    InPlace,
    #[serde(rename = "declaration_moved")]
    Moved {
        declaration: Declaration,
        path:        String,
        line:        usize,
    },
    #[serde(rename = "declaration_removed")]
    Removed {
        declaration: Declaration,
        evidence:    OutsideEvidence,
    },
    #[serde(rename = "declared_in_several_files")]
    SeveralFiles {
        declaration: Declaration,
        paths:       Vec<String>,
        evidence:    OutsideEvidence,
    },
    #[serde(rename = "declared_in_another_crate")]
    AnotherCrate {
        declaration: Declaration,
        path:        String,
        evidence:    OutsideEvidence,
    },
    #[serde(rename = "declaration_renamed")]
    Renamed {
        declaration: Declaration,
        renamed:     Declaration,
        path:        String,
        line:        usize,
    },
    RenameAwaitingApproval,
    #[serde(rename = "declaration_gone_from_file")]
    GoneFromFile {
        declaration: Declaration,
        evidence:    OutsideEvidence,
    },
    KeptForOpenBuild {
        ticket: String,
    },
    TraitIdentityUncertain {
        declaration: Declaration,
    },
    OutsideRenameUnresolved {
        declaration: Declaration,
    },
    StatementNotFollowed {
        declaration: Declaration,
        path:        String,
        reason:      StatementRefusal,
    },
    FindingSite,
    ApprovalUnobserved {
        path: String,
    },
}

/// Whether an outcome rests on the outside-rename refusal in the same reply.
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum OutsideEvidence {
    #[serde(rename = "no_outside_evidence")]
    None,
    OutsideRenameNotProven,
}

/// The site evidence that a finding files with after its declaration reading.
pub(super) enum FilingEvidence {
    Site(OriginalSite),
    WithoutHistory { path: String, line: usize },
    Refused(RefusedTransition),
    ApprovalUnobserved,
}

#[derive(Clone, Copy, Serialize)]
pub(super) enum RefusedTransition {
    #[serde(rename = "no source site after proved declaration removal")]
    AfterRemoval,
    #[serde(rename = "existing declaration proof refused this transition")]
    TransitionRefused,
}

impl From<(&DeclarationOutcome, &DeclarationInput<'_>)> for FilingEvidence {
    fn from((outcome, input): (&DeclarationOutcome, &DeclarationInput<'_>)) -> Self {
        match outcome {
            DeclarationOutcome::FindingSite => Self::Site(input.original_site.clone()),
            DeclarationOutcome::Moved { path, line, .. } => Self::Site(match input.anchor {
                FindingAnchor::Anchored(anchor) => OriginalSite::Unchanged {
                    site:   SourceCoordinate::new(path.clone(), *line),
                    anchor: anchor.clone(),
                },
                FindingAnchor::NeedsJudgment { reason } => {
                    OriginalSite::NeedsJudgment(SiteRefusal::Anchor(*reason))
                },
            }),
            DeclarationOutcome::Removed { .. } => Self::Refused(RefusedTransition::AfterRemoval),
            DeclarationOutcome::NotRead
                if matches!(input.held.finding.history.judging(), JudgingHistory::Empty) =>
            {
                let anchor = &input.held.finding.anchor;
                Self::WithoutHistory {
                    path: anchor.path.clone(),
                    line: anchor.line,
                }
            },
            DeclarationOutcome::NotRead | DeclarationOutcome::InPlace => {
                Self::Site(input.original_site.clone())
            },
            DeclarationOutcome::ApprovalUnobserved { .. } => Self::ApprovalUnobserved,
            DeclarationOutcome::SeveralFiles { .. }
            | DeclarationOutcome::AnotherCrate { .. }
            | DeclarationOutcome::Renamed { .. }
            | DeclarationOutcome::RenameAwaitingApproval
            | DeclarationOutcome::GoneFromFile { .. }
            | DeclarationOutcome::KeptForOpenBuild { .. }
            | DeclarationOutcome::TraitIdentityUncertain { .. }
            | DeclarationOutcome::OutsideRenameUnresolved { .. }
            | DeclarationOutcome::StatementNotFollowed { .. } => {
                Self::Refused(RefusedTransition::TransitionRefused)
            },
        }
    }
}

impl Serialize for FilingEvidence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "status", rename_all = "snake_case")]
        enum EvidenceReply<'a> {
            FilingEligibleWithoutHistory {
                path:   &'a str,
                line:   usize,
                reason: &'static str,
            },
            NeedsJudgment {
                reason: RefusedTransition,
            },
            ApprovalUnobserved,
        }
        let reply = match self {
            Self::Site(site) => return site.serialize(serializer),
            Self::WithoutHistory { path, line } => EvidenceReply::FilingEligibleWithoutHistory {
                path,
                line: *line,
                reason: "no judging ticket for source anchor",
            },
            Self::Refused(reason) => EvidenceReply::NeedsJudgment { reason: *reason },
            Self::ApprovalUnobserved => EvidenceReply::ApprovalUnobserved,
        };
        reply.serialize(serializer)
    }
}
