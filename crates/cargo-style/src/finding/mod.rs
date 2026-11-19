//! Handle the `finding` command and write its single JSON reply.

mod constants;

use std::ffi::OsString;
use std::io;
use std::io::Error;
use std::io::Write as _;
use std::path::PathBuf;

use serde::Serialize;

use self::constants::FINDING_LEDGER_FLAG;
use self::constants::FINDING_REPO_FLAG;
pub(crate) use self::constants::FINDING_SUBCOMMAND;
use crate::constants::FINDING_REPLY_SCHEMA;
use crate::finding_anchor::FindingAnchor;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::ledger_read::FindingLookup;
use crate::ledger_read::LedgerReadFailure;
use crate::ledger_read::LedgerRevision;
use crate::ledger_read::LedgerSnapshot;

/// The repository, ledger, and ordered IDs requested by `cargo style finding`.
pub(crate) struct FindingRequest {
    repo:   PathBuf,
    ledger: PathBuf,
    ids:    Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum FindingFailure {
    #[error("usage: cargo style finding --repo <path> --ledger <ledger.sqlite> <finding id>...")]
    Usage,
    #[error(transparent)]
    Ledger(#[from] LedgerReadFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
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
        let mut ids = Vec::new();
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some(FINDING_REPO_FLAG) if repo.is_none() => {
                    repo = arguments.next().map(PathBuf::from);
                },
                Some(FINDING_LEDGER_FLAG) if ledger.is_none() => {
                    ledger = arguments.next().map(PathBuf::from);
                },
                Some(id) if !id.is_empty() && !id.starts_with('-') => ids.push(id.to_owned()),
                _ => return Err(FindingFailure::Usage),
            }
        }
        match (repo, ledger) {
            (Some(repo), Some(ledger)) if !ids.is_empty() => Ok(Self { repo, ledger, ids }),
            _ => Err(FindingFailure::Usage),
        }
    }

    pub(crate) fn answer(self) -> Result<(), FindingFailure> {
        let snapshot = LedgerSnapshot::read(&self.ledger, &self.ids)?;
        let mut source = FindingSource::open(&self.repo)?;
        let findings = self
            .ids
            .into_iter()
            .zip(snapshot.findings)
            .map(|(id, lookup)| {
                Ok(match lookup {
                    FindingLookup::Held(finding) => FindingAnswer::Known {
                        id,
                        anchor: FindingAnchor::prove(&finding, &mut source)?,
                    },
                    FindingLookup::Unknown => FindingAnswer::Unknown {
                        id,
                        status: UnknownStatus::Finding,
                    },
                })
            })
            .collect::<Result<_, FindingSourceFailure>>()?;
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
        id:     String,
        #[serde(flatten)]
        anchor: FindingAnchor,
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
