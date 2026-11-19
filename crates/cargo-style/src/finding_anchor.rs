//! Prove the source coordinate stored for a held finding.

use serde::Serialize;

use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::ledger_read::AnchorEvidence;
use crate::ledger_read::HeldAnchor;
use crate::source_lines::LineContinuity;
use crate::source_lines::SiteLine;
use crate::tokenizer;
use crate::unicode_tables::SPACE;

#[derive(Serialize)]
pub(crate) enum NeedsJudgmentReason {
    #[serde(rename = "last follow commit is missing")]
    LastFollowCommitMissing,
    #[serde(rename = "recorded anchor commit is missing")]
    RecordedAnchorCommitMissing,
    #[serde(rename = "no judging ticket for source anchor")]
    NoJudgingTicket,
    #[serde(rename = "no source anchor matches the stored coordinate")]
    NoMatch,
    #[serde(rename = "source anchor is ambiguous between base and checkpoint")]
    Ambiguous,
}

/// The result of checking a held finding's source history and coordinate.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum FindingAnchor {
    Anchored {
        commit: String,
        path:   String,
        line:   usize,
    },
    NeedsJudgment {
        reason: NeedsJudgmentReason,
    },
}

impl FindingAnchor {
    pub(crate) fn prove(
        finding: &HeldAnchor,
        source: &mut FindingSource<'_>,
    ) -> Result<Self, FindingSourceFailure> {
        match &finding.evidence {
            AnchorEvidence::Followed(revision) => Ok(recorded_anchor(
                finding,
                source.resolve_commit(revision)?,
                NeedsJudgmentReason::LastFollowCommitMissing,
            )),
            AnchorEvidence::Recorded(revision) => Ok(recorded_anchor(
                finding,
                source.resolve_commit(revision)?,
                NeedsJudgmentReason::RecordedAnchorCommitMissing,
            )),
            AnchorEvidence::Judging(revisions) => {
                let mut candidates = Vec::new();
                for revision in revisions.candidates() {
                    if let CommitResolution::Resolved(commit) = source.resolve_commit(revision)?
                        && candidates.last().is_none_or(|prior| prior != commit)
                    {
                        candidates.push(commit.to_owned());
                    }
                }
                prove_legacy_anchor(finding, source, &candidates)
            },
            AnchorEvidence::NoJudgingTicket => Ok(Self::NeedsJudgment {
                reason: NeedsJudgmentReason::NoJudgingTicket,
            }),
        }
    }

    fn at(finding: &HeldAnchor, commit: &str) -> Self {
        Self::Anchored {
            commit: commit.to_owned(),
            path:   finding.path.clone(),
            line:   finding.line,
        }
    }
}

fn recorded_anchor(
    finding: &HeldAnchor,
    commit: &CommitResolution,
    reason: NeedsJudgmentReason,
) -> FindingAnchor {
    match commit {
        CommitResolution::Resolved(commit) => FindingAnchor::at(finding, commit),
        CommitResolution::Unresolved => FindingAnchor::NeedsJudgment { reason },
    }
}

fn prove_legacy_anchor(
    finding: &HeldAnchor,
    source: &mut FindingSource<'_>,
    candidates: &[String],
) -> Result<FindingAnchor, FindingSourceFailure> {
    if let [checkpoint, base] = candidates
        && let Some(blob) = source.blob(base, &finding.path)?.map(str::to_owned)
        && source.blob(checkpoint, &finding.path)? == Some(blob.as_str())
    {
        // Equal blobs give the same coordinate even when HeldAnchor::text describes a site.
        return Ok(FindingAnchor::at(finding, base));
    }

    let mut matches = Vec::new();
    let expected = trim_site_text(&finding.text);
    for commit in candidates {
        if let Some(blob) = source.blob(commit, &finding.path)?.map(str::to_owned)
            && let SiteLine::Text(line) = source.line(&blob, finding.line)?
            && !expected.is_empty()
            && trim_site_text(line) == expected
        {
            matches.push((commit, line.to_owned()));
        }
    }
    Ok(match matches.as_slice() {
        [] => FindingAnchor::NeedsJudgment {
            reason: NeedsJudgmentReason::NoMatch,
        },
        [(commit, _)] => FindingAnchor::at(finding, commit),
        [(checkpoint, after), (base, before), ..] => {
            if source.line_continuity(base, checkpoint, &finding.path, finding.line)?
                == LineContinuity::Unchanged
                && before == after
            {
                FindingAnchor::at(finding, base)
            } else {
                FindingAnchor::NeedsJudgment {
                    reason: NeedsJudgmentReason::Ambiguous,
                }
            }
        },
    })
}

fn trim_site_text(text: &str) -> &str {
    text.trim_matches(|character| tokenizer::in_table(SPACE, character))
}
