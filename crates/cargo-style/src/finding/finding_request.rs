//! Handle the `finding` command and write its single JSON reply.

use std::ffi::OsString;
use std::io;
use std::io::Error;
use std::io::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use super::constants::FINDING_LEDGER_FLAG;
use super::constants::FINDING_REPO_FLAG;
use crate::constants::FINDING_BASE_FLAG;
use crate::constants::FINDING_REPLY_SCHEMA;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::CommitResolution;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::footprint::Footprint;
use crate::footprint::FootprintBase;
use crate::footprint::FootprintError;
use crate::footprint::FootprintReader;
use crate::ledger_read::FindingLookup;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerRevision;
use crate::ledger_read::LedgerSnapshot;

/// The repository, ledger, and ordered IDs requested by `cargo style finding`.
pub(crate) struct FindingRequest {
    repo:   PathBuf,
    ledger: PathBuf,
    base:   RequestedBase,
    ids:    Vec<String>,
}

enum RequestedBase {
    SyncedOnly,
    Revision(String),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingFailure {
    #[error(
        "usage: cargo style finding --repo <path> --ledger <ledger.sqlite> [--base <commit>] <finding id>..."
    )]
    Usage,
    #[error("cannot resolve finding base {0:?} to a commit")]
    UnresolvedBase(String),
    #[error(transparent)]
    Ledger(#[from] LedgerReadFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Footprint(#[from] FootprintError),
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
        match (repo, ledger) {
            (Some(repo), Some(ledger)) if !ids.is_empty() => Ok(Self {
                repo,
                ledger,
                base,
                ids,
            }),
            _ => Err(FindingFailure::Usage),
        }
    }

    pub(crate) fn answer(self) -> Result<(), FindingFailure> {
        let snapshot = LedgerSnapshot::read(&self.ledger, &self.ids)?;
        let mut source = FindingSource::open(&self.repo)?;
        let base = match self.base {
            RequestedBase::SyncedOnly => FootprintBase::SyncedOnly,
            RequestedBase::Revision(revision) => match source.resolve_commit(&revision)? {
                CommitResolution::Resolved(commit) => FootprintBase::WithBase(commit.clone()),
                CommitResolution::Unresolved => {
                    return Err(FindingFailure::UnresolvedBase(revision));
                },
            },
        };
        let mut footprints = FootprintReader::new(&self.repo, snapshot.synced_tip.clone(), base)?;
        let findings = self
            .ids
            .into_iter()
            .zip(snapshot.findings)
            .map(|(id, lookup)| {
                Ok(match lookup {
                    FindingLookup::Held(finding) => FindingAnswer::Known {
                        id,
                        anchor: FindingAnchor::prove(&finding.anchor, &mut source)?,
                        footprint: footprints.read(&finding, &mut source)?,
                    },
                    FindingLookup::Unknown => FindingAnswer::Unknown {
                        id,
                        status: UnknownStatus::Finding,
                    },
                })
            })
            .collect::<Result<_, FindingFailure>>()?;
        let reply = FindingReply {
            schema: FINDING_REPLY_SCHEMA,
            observed: ObservedLedger {
                synced_tip:      snapshot.synced_tip,
                ledger_revision: snapshot.ledger_revision,
            },
            findings,
        };
        let mut stdout = io::stdout().lock();
        serde_json::to_writer(&mut stdout, &reply)?;
        writeln!(stdout)?;
        Ok(())
    }
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
        id:        String,
        #[serde(flatten)]
        anchor:    FindingAnchor,
        footprint: Footprint,
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
