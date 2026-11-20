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
use crate::constants::FINDING_RENAME_POLICY_FLAG;
use crate::constants::FINDING_REPLY_SCHEMA;
use crate::constants::FINDING_RULES_FLAG;
use crate::finding_anchor::FindingAnchor;
use crate::finding_anchor::NeedsJudgmentReason;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::ledger_read::FindingLookup;
use crate::ledger_read::HeldRename;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerRevision;
use crate::ledger_read::LedgerSnapshot;
use crate::original_site::OriginalSite;
use crate::original_site::SiteProofError;
use crate::original_site::SiteReader;
use crate::rename_admission;
use crate::rename_admission::AdmissionError;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::RenameAdmission;
use crate::rule_snapshot::RuleSnapshot;
use crate::rule_snapshot::RuleSnapshotFailure;

/// The repository, ledger, and ordered IDs requested by `cargo style finding`.
pub(crate) struct FindingRequest {
    repo:   PathBuf,
    ledger: PathBuf,
    scope:  FindingScope,
    ids:    Vec<String>,
    policy: PolicyInput,
}

enum FindingScope {
    Anchor,
    OriginalSite(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingFailure {
    #[error(
        "usage: cargo style finding --repo <path> --ledger <ledger.sqlite> [--rules <rules.json>] [--rename-policy <rename_policy.toml>] <finding id>..."
    )]
    Usage,
    #[error(transparent)]
    Ledger(#[from] LedgerReadFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Rules(#[from] RuleSnapshotFailure),
    #[error(transparent)]
    Site(#[from] SiteProofError),
    #[error("cannot prove rename admission for finding {id} at {path} against {tip}")]
    Rename {
        id:     String,
        path:   String,
        tip:    String,
        source: Box<AdmissionError>,
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
        let mut policy = PolicyInput::NoFlag;
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
                Some(FINDING_RENAME_POLICY_FLAG) if matches!(policy, PolicyInput::NoFlag) => {
                    let path = arguments
                        .next()
                        .map(PathBuf::from)
                        .ok_or(FindingFailure::Usage)?;
                    policy = PolicyInput::from(path);
                },
                Some(id) if !id.is_empty() && !id.starts_with('-') => ids.push(id.to_owned()),
                _ => return Err(FindingFailure::Usage),
            }
        }
        let scope = rules.map_or(FindingScope::Anchor, FindingScope::OriginalSite);
        match (repo, ledger) {
            (Some(repo), Some(ledger)) if !ids.is_empty() => Ok(Self {
                repo,
                ledger,
                scope,
                ids,
                policy,
            }),
            _ => Err(FindingFailure::Usage),
        }
    }

    pub(crate) fn answer(self) -> Result<(), FindingFailure> {
        let proof = match self.scope {
            FindingScope::Anchor => FindingProof::Anchor,
            FindingScope::OriginalSite(path) => {
                FindingProof::WithSite(SiteReader::new(RuleSnapshot::read(&path)?))
            },
        };
        let reply = proof.read(&self.repo, &self.ledger, self.ids, &self.policy)?;
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &reply)?;
        writeln!(stdout)?;
        Ok(())
    }
}

enum FindingProof {
    Anchor,
    WithSite(SiteReader),
}

impl FindingProof {
    fn read(
        self,
        repo: &Path,
        ledger: &Path,
        ids: Vec<String>,
        policy: &PolicyInput,
    ) -> Result<FindingReply, FindingFailure> {
        match self {
            Self::Anchor => {
                let snapshot = LedgerSnapshot::read_renames(ledger, &ids)?;
                let mut source = FindingSource::open(repo)?;
                reply(ids, snapshot, |id, finding, tip| {
                    let anchor = FindingAnchor::prove(&finding.anchor, &mut source)?;
                    let rename_admission =
                        prove_rename(&id, &finding, &anchor, tip, &mut source, policy)?;
                    Ok(FindingAnswer::Known {
                        id,
                        anchor,
                        rename_admission,
                    })
                })
            },
            Self::WithSite(mut reader) => {
                let snapshot = LedgerSnapshot::read_sites(ledger, &ids)?;
                let mut source = FindingSource::open(repo)?;
                reply(ids, snapshot, |id, finding, tip| {
                    let synced = reader.read_synced_tip(&mut source, tip)?;
                    let anchor = match FindingAnchor::prove(&finding.finding.anchor, &mut source) {
                        Err(FindingSourceFailure::GitText {
                            command: "diff", ..
                        }) => FindingAnchor::NeedsJudgment {
                            reason: NeedsJudgmentReason::UnreadableProof,
                        },
                        result => result?,
                    };
                    let original_site = reader.prove(
                        &mut source,
                        &synced,
                        &finding.finding.anchor,
                        &anchor,
                        &finding.site,
                    )?;
                    let rename_admission =
                        prove_rename(&id, &finding.finding, &anchor, tip, &mut source, policy)?;
                    Ok(FindingAnswer::WithSite {
                        id,
                        anchor,
                        original_site,
                        rename_admission,
                    })
                })
            },
        }
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

fn reply<T>(
    ids: Vec<String>,
    snapshot: LedgerSnapshot<T>,
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
        schema: FINDING_REPLY_SCHEMA,
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
        id:               String,
        #[serde(flatten)]
        anchor:           FindingAnchor,
        rename_admission: RenameAdmission,
    },
    WithSite {
        id:               String,
        #[serde(flatten)]
        anchor:           FindingAnchor,
        original_site:    OriginalSite,
        rename_admission: RenameAdmission,
    },
    Unknown {
        id:     String,
        status: UnknownStatus,
    },
}

#[derive(Serialize)]
enum UnknownStatus {
    #[serde(rename = "unknown_finding")]
    Finding,
}
