//! Prove that a held judgment still covers a source site.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::rc::Rc;

use serde::Serialize;
use serde::Serializer;
use serde_json::Error as JsonError;

use crate::enclosing_items::UnreadableItems;
use crate::facts::BlobSyntax;
use crate::finding_anchor::FindingAnchor;
use crate::finding_anchor::NeedsJudgmentReason;
use crate::finding_anchor::ResolvedAnchor;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::finding_source::Utf8Source;
use crate::ledger_read::HeldAnchor;
use crate::ledger_read::OriginalJudgment;
use crate::rule_snapshot::Family;
use crate::rule_snapshot::PinComparison;
use crate::rule_snapshot::RuleFamily;
use crate::rule_snapshot::RuleSnapshot;
use crate::rule_snapshot::RuleSnapshotFailure;
use crate::site_ledger::FindingKind;
use crate::site_ledger::FindingState;
use crate::site_ledger::HolderEvidence;
use crate::site_ledger::InputScope;
use crate::site_ledger::JudgmentHistory;
use crate::site_ledger::PinnedFamily;
use crate::site_ledger::ReadScope;
use crate::site_ledger::SiteEvidence;
use crate::source_lines::LineMapping;
use crate::source_lines::SiteLine;
use crate::source_sites;
use crate::source_sites::SourceSites;
use crate::tip_view;
use crate::tip_view::FingerprintSource;
use crate::tip_view::MissingInput;
use crate::tip_view::RepositoryTip;

pub(crate) enum OriginalSite {
    Unchanged {
        site:   SourceCoordinate,
        anchor: ResolvedAnchor,
    },
    NeedsJudgment(SiteRefusal),
}

#[derive(Serialize)]
pub(crate) struct SourceCoordinate {
    path: String,
    line: usize,
}

#[derive(Debug, Serialize)]
pub(crate) enum SiteRefusal {
    #[serde(rename = "not an open Rust finding")]
    NotOpenRust,
    #[serde(rename = "judgment belongs to an inactive rule generation")]
    InactiveGeneration,
    #[serde(rename = "no original judgment commit")]
    NoOriginalJudgment,
    #[serde(rename = "judgment crossed ancestry or removal boundary")]
    AncestryOrRemoval,
    #[serde(rename = "not a regular source file")]
    NonregularFile,
    #[serde(rename = "pinned rule family unreadable")]
    UnreadableFamily,
    #[serde(rename = "original context inputs changed or missing")]
    ContextChanged,
    #[serde(rename = "source or input proof unreadable")]
    UnreadableProof,
    #[serde(rename = "old source coordinate unreadable")]
    OldCoordinate,
    #[serde(rename = "finding text is not its old source line")]
    WrongText,
    #[serde(rename = "proposed type subject is not located in its original item")]
    MissingSubject,
    #[serde(rename = "enclosing item, owner, imports or original inputs changed")]
    ItemChanged,
    #[serde(rename = "unchanged source site not proved")]
    Unproved,
    #[serde(untagged)]
    Anchor(NeedsJudgmentReason),
}

impl Serialize for OriginalSite {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        #[serde(tag = "status", rename_all = "snake_case")]
        enum SiteReply<'a> {
            Unchanged {
                site:   &'a SourceCoordinate,
                anchor: &'a ResolvedAnchor,
            },
            NeedsJudgment {
                reason: &'a SiteRefusal,
            },
        }
        let reply = match self {
            Self::Unchanged { site, anchor } => SiteReply::Unchanged { site, anchor },
            Self::NeedsJudgment(reason) => SiteReply::NeedsJudgment { reason },
        };
        reply.serialize(serializer)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SiteProofError {
    #[error("the site proof refused the finding")]
    Refused(SiteRefusal),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Rules(#[from] RuleSnapshotFailure),
    #[error(transparent)]
    MissingTipInput(#[from] MissingInput),
    #[error("cannot encode the input fingerprint")]
    Fingerprint(#[from] JsonError),
}

impl From<SiteRefusal> for SiteProofError {
    fn from(reason: SiteRefusal) -> Self { Self::Refused(reason) }
}

impl From<UnreadableItems> for SiteProofError {
    fn from(_: UnreadableItems) -> Self { SiteRefusal::UnreadableProof.into() }
}

pub(crate) struct SiteReader {
    rules: RuleSnapshot,
    sites: HashMap<Option<String>, Result<Rc<SourceSites>, UnreadableItems>>,
}

pub(crate) struct SyncedSiteTip<'a> {
    commit: &'a str,
    view:   Rc<RepositoryTip>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TipRole {
    Synced,
    Historical,
}

impl SiteReader {
    pub(crate) fn new(rules: RuleSnapshot) -> Self {
        Self {
            rules,
            sites: HashMap::new(),
        }
    }

    pub(crate) fn read_synced_tip<'a>(
        source: &mut FindingSource<'_>,
        commit: &'a str,
    ) -> Result<SyncedSiteTip<'a>, SiteProofError> {
        Ok(SyncedSiteTip {
            commit,
            view: read_tip(source, commit, TipRole::Synced)?,
        })
    }

    pub(crate) fn prove(
        &mut self,
        source: &mut FindingSource<'_>,
        synced: &SyncedSiteTip<'_>,
        finding: &HeldAnchor,
        anchor: &FindingAnchor,
        evidence: &SiteEvidence,
    ) -> Result<OriginalSite, SiteProofError> {
        match self.prove_site(source, synced, finding, anchor, evidence) {
            Ok((site, anchor)) => Ok(OriginalSite::Unchanged {
                site,
                anchor: anchor.clone(),
            }),
            Err(SiteProofError::Refused(reason)) => Ok(OriginalSite::NeedsJudgment(reason)),
            Err(SiteProofError::MissingTipInput(_)) => {
                Ok(OriginalSite::NeedsJudgment(SiteRefusal::UnreadableProof))
            },
            Err(error) => Err(error),
        }
    }

    fn prove_site<'a>(
        &mut self,
        source: &mut FindingSource<'_>,
        synced: &SyncedSiteTip<'_>,
        finding: &HeldAnchor,
        anchor: &'a FindingAnchor,
        evidence: &SiteEvidence,
    ) -> Result<(SourceCoordinate, &'a ResolvedAnchor), SiteProofError> {
        let after = &synced.view;
        let tip = synced.commit;
        if !matches!(evidence.state, FindingState::Open) || !tip_view::is_rust_path(&finding.path) {
            return Err(SiteRefusal::NotOpenRust.into());
        }
        if matches!(evidence.holder, HolderEvidence::InactiveGeneration) {
            return Err(SiteRefusal::InactiveGeneration.into());
        }
        let anchor = match anchor {
            FindingAnchor::Anchored(anchor) => anchor,
            FindingAnchor::NeedsJudgment { reason } => {
                return Err(SiteRefusal::Anchor(*reason).into());
            },
        };
        let read = &anchor.commit;
        let removals = active_removals(source, tip, &evidence.history)?;
        let retained = survives_removals(source, read, &removals)?;
        check_cell_history(source, &evidence.history, &removals)?;
        let original = original_commit(source, &evidence.original, read)?;
        if !source.is_ancestor(&original, read)?
            || !source.is_ancestor(read, tip)?
            || !retained
            || !survives_removals(source, &original, &removals)?
        {
            return Err(SiteRefusal::AncestryOrRemoval.into());
        }
        let before = read_tip(source, read, TipRole::Historical)?;
        let path = &finding.path;
        let family = match evidence.family {
            PinnedFamily::Known(family) => RuleFamily::Known(family),
            PinnedFamily::Unspecified => {
                self.rules.current_family(&evidence.rule, &evidence.pin)?
            },
            PinnedFamily::Unreadable => RuleFamily::Unspecified,
        };
        if [&before, after].into_iter().any(|tree| {
            tree.entry(path)
                .is_some_and(|entry| !matches!(entry.mode.as_str(), "100644" | "100755"))
        }) {
            return Err(SiteRefusal::NonregularFile.into());
        }
        let RuleFamily::Known(family) = family else {
            return Err(SiteRefusal::UnreadableFamily.into());
        };
        if !self.context_unchanged(source, tip, after, anchor, evidence, family)? {
            return Err(SiteRefusal::ContextChanged.into());
        }
        let old_blob = before.blob(path);
        let new_blob = after.blob(path);
        let old = self.source_sites(source, old_blob)?;
        let new = self.source_sites(source, new_blob)?;
        old.check_owner(finding.line)?;
        let line = if old_blob.is_some() && old_blob == new_blob {
            finding.line
        } else {
            prove_moved_line(
                source,
                (&anchor.commit, tip),
                finding,
                &old,
                &new,
                &evidence.kind,
                family,
            )?
        };
        Ok((
            SourceCoordinate {
                path: path.clone(),
                line,
            },
            anchor,
        ))
    }

    fn source_sites(
        &mut self,
        source: &mut FindingSource<'_>,
        blob: Option<&str>,
    ) -> Result<Rc<SourceSites>, SiteProofError> {
        let parsed = match self.sites.entry(blob.map(str::to_owned)) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let text = match blob {
                    Some(blob) => match source.source(blob)? {
                        SourceText::Utf8(source) => Rc::clone(source),
                        SourceText::NotUtf8(_) => return Err(SiteRefusal::UnreadableProof.into()),
                    },
                    None => Rc::new(Utf8Source::new(String::new(), BlobSyntax::Unparsed)),
                };
                entry.insert(SourceSites::read(text).map(Rc::new))
            },
        };
        parsed
            .as_ref()
            .map(Rc::clone)
            .map_err(|_| SiteRefusal::UnreadableProof.into())
    }

    fn context_unchanged(
        &self,
        source: &mut FindingSource<'_>,
        tip: &str,
        after: &RepositoryTip,
        anchor: &ResolvedAnchor,
        evidence: &SiteEvidence,
        family: Family,
    ) -> Result<bool, SiteProofError> {
        let contextual = matches!(
            evidence.rule.as_str(),
            "global:rust/no-duplicate-implementations.md"
                | "global:rust/one-mechanism-per-behavior.md"
                | "global:rust/crate-apis-expose-behavior.md"
                | "global:rust/behavior-lives-in-its-most-cohesive-crate.md"
        );
        let HolderEvidence::CoveringRow { row, history } = &evidence.holder else {
            return Ok(matches!(evidence.holder, HolderEvidence::NoCoveringRow)
                && family.is_line_local()
                && !contextual);
        };
        match self
            .rules
            .compares_pins(&evidence.rule, &evidence.pin, &row.blob)?
        {
            PinComparison::Matches => {},
            PinComparison::Changed => return Ok(false),
            PinComparison::InvalidComponent => return Err(SiteRefusal::UnreadableProof.into()),
        }
        if !source.is_ancestor(&row.commit, tip)? {
            return Ok(false);
        }
        let removals = active_removals(source, tip, history)?;
        check_cell_history(source, history, &removals)?;
        if row.verdict.is_removed() || !survives_removals(source, &row.commit, &removals)? {
            return Ok(false);
        }
        let whole = contextual || matches!(row.scope, InputScope::Crate | InputScope::Manifest);
        if family.is_line_local() && !whole {
            return Ok(true);
        }
        let original = read_tip(source, &row.commit, TipRole::Historical)?;
        let scope = row.effective_scope();
        let Ok(mut old) = original.inputs(scope, &row.path, &row.reads) else {
            return Ok(false);
        };
        if scope == row.scope || matches!(row.reads, ReadScope::Explicit(_)) {
            let fingerprint_source = match (&row.reads, scope) {
                (ReadScope::Inferred, InputScope::File) => FingerprintSource::FileBlob(&row.path),
                (ReadScope::Inferred, InputScope::Crate) => {
                    FingerprintSource::CrateTree(original.crate_of(&row.path))
                },
                _ => FingerprintSource::Entries,
            };
            if original.fingerprint(fingerprint_source, &old)? != row.fingerprint {
                return Ok(false);
            }
        }
        let mut new = after.inputs(scope, &row.path, &row.reads)?;
        if !whole {
            old.remove(anchor.path.as_str());
            new.remove(anchor.path.as_str());
        }
        Ok(old == new && source.is_ancestor(&row.commit, &anchor.commit)?)
    }
}

fn read_tip(
    source: &mut FindingSource<'_>,
    commit: &str,
    role: TipRole,
) -> Result<Rc<RepositoryTip>, SiteProofError> {
    match source.tip(commit) {
        Ok(tip) => Ok(tip),
        Err(FindingSourceFailure::UnreadableTip(_)) if role == TipRole::Synced => {
            Err(FindingSourceFailure::UndecodableSyncedTree(commit.to_owned()).into())
        },
        Err(FindingSourceFailure::UnreadableTip(_)) => Err(SiteRefusal::UnreadableProof.into()),
        Err(error) => Err(error.into()),
    }
}

fn original_commit(
    source: &mut FindingSource<'_>,
    original: &OriginalJudgment,
    anchor: &str,
) -> Result<String, SiteProofError> {
    let candidates = match original {
        OriginalJudgment::Anchor => return Ok(anchor.to_owned()),
        OriginalJudgment::Recorded(commit) => std::slice::from_ref(commit),
        OriginalJudgment::PriorTicket(commits) => commits,
    };
    for commit in candidates {
        if let CommitResolution::Resolved(commit) = source.resolve_commit(commit)? {
            return Ok(commit.clone());
        }
    }
    Err(SiteRefusal::NoOriginalJudgment.into())
}

fn active_removals<'a>(
    source: &mut FindingSource<'_>,
    tip: &str,
    history: &'a [JudgmentHistory],
) -> Result<Vec<&'a str>, FindingSourceFailure> {
    let mut active = Vec::new();
    for row in history.iter().filter(|row| row.verdict.is_removed()) {
        if source.is_ancestor(&row.commit, tip)? {
            active.push(row.commit.as_str());
        }
    }
    Ok(active)
}

fn check_cell_history(
    source: &mut FindingSource<'_>,
    history: &[JudgmentHistory],
    removals: &[&str],
) -> Result<(), FindingSourceFailure> {
    if removals.is_empty() {
        return Ok(());
    }
    for row in history {
        if !row.verdict.is_removed() || !removals.contains(&row.commit.as_str()) {
            let _ = survives_removals(source, &row.commit, removals)?;
        }
    }
    Ok(())
}

fn survives_removals(
    source: &mut FindingSource<'_>,
    commit: &str,
    removals: &[&str],
) -> Result<bool, FindingSourceFailure> {
    for removal in removals {
        if !source.is_ancestor(removal, commit)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn prove_moved_line(
    source: &mut FindingSource<'_>,
    (read, tip): (&str, &str),
    finding: &HeldAnchor,
    old: &SourceSites,
    new: &SourceSites,
    kind: &FindingKind,
    family: Family,
) -> Result<usize, SiteProofError> {
    let SiteLine::Text(line) = old.line(finding.line) else {
        return Err(SiteRefusal::OldCoordinate.into());
    };
    let text = source_sites::trim(line);
    if text.is_empty() || text != source_sites::trim(&finding.text) {
        return Err(SiteRefusal::WrongText.into());
    }
    if let FindingKind::Type { subject } = kind
        && !old.locates_subject(finding.line, subject)
    {
        return Err(SiteRefusal::MissingSubject.into());
    }
    let mapping = source.mapped_line(read, tip, &finding.path, finding.line);
    let mapping = match mapping {
        Err(FindingSourceFailure::GitText {
            command: "diff", ..
        }) => {
            return Err(SiteRefusal::UnreadableProof.into());
        },
        result => result?,
    };
    let LineMapping::Mapped(mapped) = mapping else {
        return Err(SiteRefusal::Unproved.into());
    };
    if !matches!(new.line(mapped), SiteLine::Text(text) if text == line)
        || !old.same_owner(finding.line, new, mapped)?
    {
        return Err(SiteRefusal::Unproved.into());
    }
    if !family.is_line_local() && !old.same_item(finding.line, new, mapped) {
        return Err(SiteRefusal::ItemChanged.into());
    }
    Ok(mapped)
}
