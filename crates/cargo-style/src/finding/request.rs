//! Handle the `finding` command and write its single JSON reply.

use std::ffi::OsString;
use std::io;
use std::io::Error;
use std::io::Write as _;
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

use super::constants::FINDING_LEDGER_FLAG;
use super::constants::FINDING_REPO_FLAG;
use crate::constants::FINDING_APPROVALS_FLAG;
use crate::constants::FINDING_BASE_FLAG;
use crate::constants::FINDING_PROOF_REPLY_SCHEMA;
use crate::constants::FINDING_RENAME_POLICY_FLAG;
use crate::constants::FINDING_REPLY_SCHEMA;
use crate::constants::FINDING_RULES_FLAG;
use crate::declaration_reading::ApprovalFailure;
use crate::declaration_reading::ApprovalSnapshot;
use crate::declaration_reading::DeclarationAnswer;
use crate::declaration_reading::DeclarationFailure;
use crate::declaration_reading::DeclarationInput;
use crate::declaration_reading::DeclarationReader;
use crate::finding_anchor::FindingAnchor;
use crate::finding_anchor::NeedsJudgmentReason;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::footprint::Footprint;
use crate::footprint::FootprintBase;
use crate::footprint::FootprintError;
use crate::footprint::FootprintReader;
use crate::ledger_read::FindingLookup;
use crate::ledger_read::HeldRename;
use crate::ledger_read::HeldSite;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerRevision;
use crate::ledger_read::LedgerSnapshot;
use crate::original_site::OriginalSite;
use crate::original_site::SiteProofError;
use crate::original_site::SiteReader;
use crate::outside_rename::OutsideReader;
use crate::outside_rename::OutsideRename;
use crate::outside_rename::OutsideSourceFailure;
use crate::rename_admission;
use crate::rename_admission::AdmissionError;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::RenameAdmission;
use crate::rename_applied::AppliedError;
use crate::rename_applied::AppliedReader;
use crate::rename_applied::RenameApplied;
use crate::rule_snapshot::RuleSnapshot;
use crate::rule_snapshot::RuleSnapshotFailure;
use crate::same_declaration::RelocationFailure;
use crate::same_declaration::RelocationReader;
use crate::same_declaration::SameDeclaration;

/// The repository, ledger, and ordered IDs requested by `cargo style finding`.
pub(crate) struct FindingRequest {
    repo:   PathBuf,
    ledger: PathBuf,
    scope:  FindingScope,
    ids:    Vec<String>,
    policy: PolicyInput,
    base:   RequestedBase,
}

enum FindingScope {
    Anchor,
    OriginalSite {
        rules:     PathBuf,
        approvals: ApprovalSource,
    },
}

/// Where the user's answers on decision files come from.
enum ApprovalSource {
    File(PathBuf),
    NoFile,
}

enum RequestedBase {
    SyncedOnly,
    Revision(String),
}

impl RequestedBase {
    fn reader<'a>(
        self,
        repo: &'a Path,
        tip: &str,
        source: &mut FindingSource<'_>,
    ) -> Result<FootprintReader<'a>, FindingFailure> {
        let base = match self {
            Self::SyncedOnly => FootprintBase::SyncedOnly,
            Self::Revision(revision) => match source.resolve_commit(&revision)? {
                CommitResolution::Resolved(commit) => FootprintBase::WithBase(commit.clone()),
                CommitResolution::Unresolved => {
                    return Err(FindingFailure::UnresolvedBase(revision));
                },
            },
        };
        Ok(FootprintReader::new(repo, tip.to_owned(), base)?)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingFailure {
    #[error(
        "usage: cargo style finding --repo <path> --ledger <ledger.sqlite> [--rules <rules.json> [--approvals <approvals.json>]] [--rename-policy <rename_policy.toml>] [--base <commit>] <finding id>..."
    )]
    Usage,
    #[error("cannot resolve finding base {0:?} to a commit")]
    UnresolvedBase(String),
    #[error(transparent)]
    Ledger(#[from] LedgerReadFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Rules(#[from] RuleSnapshotFailure),
    #[error(transparent)]
    Site(#[from] SiteProofError),
    #[error(transparent)]
    Approvals(#[from] ApprovalFailure),
    #[error("cannot read declaration for {path} from {commit} to {synced_tip}")]
    Declaration {
        path:       String,
        commit:     String,
        synced_tip: String,
        source:     Box<DeclarationFailure>,
    },
    #[error(transparent)]
    Built(Box<DeclarationFailure>),
    #[error("cannot prove declaration relocation for {path} from {commit} to {synced_tip}")]
    Relocation {
        path:       String,
        commit:     String,
        synced_tip: String,
        source:     Box<RelocationFailure>,
    },
    #[error("cannot prove outside rename for {path} from {commit} to {synced_tip}")]
    OutsideRename {
        path:       String,
        commit:     String,
        synced_tip: String,
        source:     Box<OutsideSourceFailure>,
    },
    #[error(transparent)]
    Footprint(#[from] FootprintError),
    #[error("cannot prove rename admission for finding {id} at {path} against {tip}")]
    Rename {
        id:     String,
        path:   String,
        tip:    String,
        source: Box<AdmissionError>,
    },
    #[error("cannot prove rename applied for finding {id} at {path} against {tip}")]
    Applied {
        id:     String,
        path:   String,
        tip:    String,
        source: Box<AppliedError>,
    },
    #[error("cannot write the finding reply")]
    Reply(#[from] serde_json::Error),
    #[error("cannot finish the finding reply")]
    Write(#[from] Error),
}

impl FindingRequest {
    pub(crate) fn parse(
        mut arguments: impl Iterator<Item = OsString>,
    ) -> Result<Self, FindingFailure> {
        let mut repo = None;
        let mut ledger = None;
        let mut rules = None;
        let mut approvals = None;
        let mut policy = PolicyInput::ErrorOnMissingFlag;
        let mut base = RequestedBase::SyncedOnly;
        let mut ids = Vec::new();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some(FINDING_REPO_FLAG) if repo.is_none() => {
                    repo = arguments.next().map(PathBuf::from);
                },
                Some(FINDING_LEDGER_FLAG) if ledger.is_none() => {
                    ledger = arguments.next().map(PathBuf::from);
                },
                Some(FINDING_RULES_FLAG) if rules.is_none() => {
                    rules = Some(
                        arguments
                            .next()
                            .map(PathBuf::from)
                            .ok_or(FindingFailure::Usage)?,
                    );
                },
                Some(FINDING_APPROVALS_FLAG) if approvals.is_none() => {
                    approvals = Some(
                        arguments
                            .next()
                            .map(PathBuf::from)
                            .ok_or(FindingFailure::Usage)?,
                    );
                },
                Some(FINDING_RENAME_POLICY_FLAG)
                    if matches!(policy, PolicyInput::ErrorOnMissingFlag) =>
                {
                    let path = arguments
                        .next()
                        .map(PathBuf::from)
                        .ok_or(FindingFailure::Usage)?;
                    policy = PolicyInput::from(path);
                },
                Some(FINDING_BASE_FLAG) if matches!(base, RequestedBase::SyncedOnly) => {
                    let revision = arguments
                        .next()
                        .and_then(|argument| argument.into_string().ok())
                        .filter(|revision| !revision.is_empty() && !revision.starts_with('-'))
                        .ok_or(FindingFailure::Usage)?;
                    base = RequestedBase::Revision(revision);
                },
                Some(id) if !id.is_empty() && !id.starts_with('-') => ids.push(id.to_owned()),
                _ => return Err(FindingFailure::Usage),
            }
        }
        let scope = match (rules, approvals) {
            (None, None) => FindingScope::Anchor,
            (None, Some(_)) => return Err(FindingFailure::Usage),
            (Some(rules), approvals) => FindingScope::OriginalSite {
                rules,
                approvals: approvals.map_or(ApprovalSource::NoFile, ApprovalSource::File),
            },
        };
        match (repo, ledger) {
            (Some(repo), Some(ledger)) if !ids.is_empty() => Ok(Self {
                repo,
                ledger,
                scope,
                ids,
                policy,
                base,
            }),
            _ => Err(FindingFailure::Usage),
        }
    }

    pub(crate) fn answer(self) -> Result<(), FindingFailure> {
        let proof = match self.scope {
            FindingScope::Anchor => FindingProof::Anchor,
            FindingScope::OriginalSite { rules, approvals } => {
                let approvals = match approvals {
                    ApprovalSource::File(path) => ApprovalSnapshot::read(&path)?,
                    ApprovalSource::NoFile => ApprovalSnapshot::default(),
                };
                FindingProof::WithSite(SiteReader::new(RuleSnapshot::read(&rules)?), approvals)
            },
        };
        let reply = proof.read(&self.repo, &self.ledger, self.ids, &self.policy, self.base)?;
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &reply)?;
        writeln!(stdout)?;
        Ok(())
    }
}

enum FindingProof {
    Anchor,
    WithSite(SiteReader, ApprovalSnapshot),
}

impl FindingProof {
    fn read(
        self,
        repo: &Path,
        ledger: &Path,
        ids: Vec<String>,
        policy: &PolicyInput,
        base: RequestedBase,
    ) -> Result<FindingReply, FindingFailure> {
        match self {
            Self::Anchor => {
                let snapshot = LedgerSnapshot::read_renames(ledger, &ids)?;
                let mut source = FindingSource::open(repo)?;
                let mut edits =
                    FindingEditReader::new(base, repo, &snapshot.synced_tip, &mut source, policy)?;
                reply(FINDING_REPLY_SCHEMA, ids, snapshot, |id, finding, tip| {
                    let anchor = FindingAnchor::prove(&finding.anchor, &mut source)?;
                    let edits = edits.prove(&id, &finding, &anchor, tip, &mut source)?;
                    Ok(FindingAnswer::Known { id, anchor, edits })
                })
            },
            Self::WithSite(reader, approvals) => {
                let mut snapshot = LedgerSnapshot::read_sites(ledger, &ids)?;
                let built = std::mem::take(&mut snapshot.beside);
                let mut source = FindingSource::open(repo)?;
                let policy = match policy {
                    PolicyInput::ErrorOnMissingFlag => &PolicyInput::RefuseMissingFlag,
                    policy => policy,
                };
                let mut edits =
                    FindingEditReader::new(base, repo, &snapshot.synced_tip, &mut source, policy)?;
                let mut rules = RuleFindingReader {
                    site:         reader,
                    relocation:   RelocationReader::default(),
                    outside:      OutsideReader::default(),
                    declarations: DeclarationReader::new(approvals, built),
                };
                reply(
                    FINDING_PROOF_REPLY_SCHEMA,
                    ids,
                    snapshot,
                    |id, finding, tip| {
                        let (anchor, site) = rules.prove(&finding, tip, &mut source)?;
                        let edits =
                            edits.prove(&id, &finding.finding, &anchor, tip, &mut source)?;
                        Ok(FindingAnswer::WithSite {
                            id,
                            anchor,
                            site: Box::new(site),
                            edits,
                        })
                    },
                )
            },
        }
    }
}

struct RuleFindingReader {
    site:         SiteReader,
    relocation:   RelocationReader,
    outside:      OutsideReader,
    declarations: DeclarationReader,
}

impl RuleFindingReader {
    fn prove(
        &mut self,
        held: &HeldSite,
        tip: &str,
        source: &mut FindingSource<'_>,
    ) -> Result<(FindingAnchor, RuleFindingAnswers), FindingFailure> {
        let synced = SiteReader::read_synced_tip(source, tip)?;
        let finding = &held.finding.anchor;
        let anchor = match FindingAnchor::prove(finding, source) {
            Err(FindingSourceFailure::GitText {
                command: "diff", ..
            }) => FindingAnchor::NeedsJudgment {
                reason: NeedsJudgmentReason::UnreadableProof,
            },
            result => result?,
        };
        let original_site = self
            .site
            .prove(source, &synced, finding, &anchor, &held.site)?;
        let read = match &anchor {
            FindingAnchor::Anchored(anchor) => anchor.commit.as_str(),
            FindingAnchor::NeedsJudgment { .. } => tip,
        };
        let same_declaration = self
            .relocation
            .prove(source, tip, finding, &anchor, &held.relocation)
            .map_err(|source| FindingFailure::Relocation {
                path:       finding.path.clone(),
                commit:     read.to_owned(),
                synced_tip: tip.to_owned(),
                source:     Box::new(source),
            })?;
        let input = DeclarationInput {
            held,
            anchor: &anchor,
            original_site: &original_site,
            same: &same_declaration,
        };
        let (outside_rename, declaration) = self
            .declarations
            .read(source, &mut self.relocation, &mut self.outside, tip, &input)
            .map_err(|failure| match failure {
                DeclarationFailure::Built { .. } => FindingFailure::Built(Box::new(failure)),
                DeclarationFailure::Outside(source) => FindingFailure::OutsideRename {
                    path:       source.path.clone(),
                    commit:     read.to_owned(),
                    synced_tip: tip.to_owned(),
                    source:     Box::new(source),
                },
                DeclarationFailure::Source(_) | DeclarationFailure::Applied(_) => {
                    FindingFailure::Declaration {
                        path:       finding.path.clone(),
                        commit:     read.to_owned(),
                        synced_tip: tip.to_owned(),
                        source:     Box::new(failure),
                    }
                },
            })?;
        let answers = RuleFindingAnswers {
            original_site,
            same_declaration: Box::new(same_declaration),
            outside_rename,
            declaration,
        };
        Ok((anchor, answers))
    }
}

struct FindingEditReader<'a> {
    footprints: FootprintReader<'a>,
    applied:    AppliedReader,
    policy:     &'a PolicyInput,
}

impl<'a> FindingEditReader<'a> {
    fn new(
        base: RequestedBase,
        repo: &'a Path,
        tip: &str,
        source: &mut FindingSource<'_>,
        policy: &'a PolicyInput,
    ) -> Result<Self, FindingFailure> {
        Ok(Self {
            footprints: base.reader(repo, tip, source)?,
            applied: AppliedReader::default(),
            policy,
        })
    }

    fn prove(
        &mut self,
        id: &str,
        finding: &HeldRename,
        anchor: &FindingAnchor,
        tip: &str,
        source: &mut FindingSource<'_>,
    ) -> Result<FindingEditAnswers, FindingFailure> {
        Ok(FindingEditAnswers {
            rename_admission: prove_rename(id, finding, anchor, tip, source, self.policy)?,
            rename_applied:   prove_applied(
                &mut self.applied,
                id,
                finding,
                tip,
                source,
                self.policy,
            )?,
            footprint:        self.footprints.read(finding, source)?,
        })
    }
}

fn prove_rename(
    id: &str,
    finding: &HeldRename,
    anchor: &FindingAnchor,
    tip: &str,
    source: &mut FindingSource<'_>,
    policy: &PolicyInput,
) -> Result<RenameAdmission, FindingFailure> {
    rename_admission::admit(id, finding, anchor, tip, source, policy).map_err(|source| {
        FindingFailure::Rename {
            id:     id.to_owned(),
            path:   finding.anchor.path.clone(),
            tip:    tip.to_owned(),
            source: Box::new(source),
        }
    })
}

fn prove_applied(
    reader: &mut AppliedReader,
    id: &str,
    finding: &HeldRename,
    tip: &str,
    source: &mut FindingSource<'_>,
    policy: &PolicyInput,
) -> Result<RenameApplied, FindingFailure> {
    reader
        .check(finding, tip, source, policy)
        .map_err(|source| FindingFailure::Applied {
            id:     id.to_owned(),
            path:   finding.anchor.path.clone(),
            tip:    tip.to_owned(),
            source: Box::new(source),
        })
}

fn reply<T, B>(
    schema: u32,
    ids: Vec<String>,
    snapshot: LedgerSnapshot<T, B>,
    mut prove: impl FnMut(String, T, &str) -> Result<FindingAnswer, FindingFailure>,
) -> Result<FindingReply, FindingFailure> {
    let findings = ids
        .into_iter()
        .zip(snapshot.findings)
        .map(|(id, lookup)| match lookup {
            FindingLookup::Held(finding) => prove(id, finding, &snapshot.synced_tip),
            FindingLookup::Unknown => Ok(FindingAnswer::Unknown {
                id,
                status: UnknownStatus::Finding,
            }),
        })
        .collect::<Result<_, _>>()?;
    Ok(FindingReply {
        schema,
        observed: ObservedLedger {
            synced_tip:      snapshot.synced_tip,
            ledger_revision: snapshot.ledger_revision,
        },
        findings,
    })
}

#[derive(Serialize)]
struct FindingReply {
    schema:   u32,
    observed: ObservedLedger,
    findings: Vec<FindingAnswer>,
}

#[derive(Serialize)]
struct ObservedLedger {
    synced_tip:      String,
    ledger_revision: LedgerRevision,
}

#[derive(Serialize)]
#[serde(untagged)]
enum FindingAnswer {
    Known {
        id:     String,
        #[serde(flatten)]
        anchor: FindingAnchor,
        #[serde(flatten)]
        edits:  FindingEditAnswers,
    },
    WithSite {
        id:     String,
        #[serde(flatten)]
        anchor: FindingAnchor,
        #[serde(flatten)]
        site:   Box<RuleFindingAnswers>,
        #[serde(flatten)]
        edits:  FindingEditAnswers,
    },
    Unknown {
        id:     String,
        status: UnknownStatus,
    },
}

#[derive(Serialize)]
struct RuleFindingAnswers {
    original_site:    OriginalSite,
    same_declaration: Box<SameDeclaration>,
    outside_rename:   OutsideRename,
    #[serde(flatten)]
    declaration:      DeclarationAnswer,
}

#[derive(Serialize)]
struct FindingEditAnswers {
    rename_admission: RenameAdmission,
    rename_applied:   RenameApplied,
    footprint:        Footprint,
}

#[derive(Serialize)]
enum UnknownStatus {
    #[serde(rename = "unknown_finding")]
    Finding,
}
