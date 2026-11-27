//! Parse a sites request and write its reply.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::path::PathBuf;

use serde::Serialize;

use super::catalog::Catalog;
use super::committed_sites::CommittedSites;
use super::committed_sites::SitesOutcome;
use super::constants::SITES_REPLY_SCHEMA;
use super::failure::SitesFailure;
use super::ledger::SitesLedger;
use super::source_tree::SourceTree;
use crate::constants::CHECKOUT_FLAG;
use crate::finding_source::FindingSource;

pub(crate) struct SitesRequest {
    repo:     PathBuf,
    ledger:   PathBuf,
    checkout: CheckoutSource,
    rule:     String,
}

pub(crate) enum CheckoutSource {
    Repository,
    Explicit(PathBuf),
}

impl CheckoutSource {
    pub(crate) fn path<'a>(&'a self, repository: &'a Path) -> &'a Path {
        match self {
            Self::Repository => repository,
            Self::Explicit(path) => path,
        }
    }
}

impl SitesRequest {
    pub(crate) fn parse(
        mut arguments: impl Iterator<Item = OsString>,
    ) -> Result<Self, SitesFailure> {
        let mut repo = None;
        let mut ledger = None;
        let mut checkout = None;
        let mut rule = None;
        while let Some(argument) = arguments.next() {
            let slot = match argument.to_str() {
                Some("--repo") => &mut repo,
                Some("--ledger") => &mut ledger,
                Some(CHECKOUT_FLAG) => &mut checkout,
                Some(text) if !text.is_empty() && !text.starts_with('-') && rule.is_none() => {
                    rule = Some(text.to_owned());
                    continue;
                },
                _ => return Err(SitesFailure::Arguments),
            };
            if slot.is_some() {
                return Err(SitesFailure::Arguments);
            }
            *slot = Some(arguments.next().ok_or(SitesFailure::Arguments)?.into());
        }
        Ok(Self {
            repo:     repo.ok_or(SitesFailure::Arguments)?,
            ledger:   ledger.ok_or(SitesFailure::Arguments)?,
            checkout: checkout.map_or(CheckoutSource::Repository, CheckoutSource::Explicit),
            rule:     rule.ok_or(SitesFailure::Arguments)?,
        })
    }

    pub(crate) fn answer(self) -> Result<(), SitesFailure> {
        let catalog = Catalog::read(self.checkout.path(&self.repo))?;
        let rule = catalog.rule(&self.rule)?;
        let mut source = FindingSource::open(&self.repo)?;
        let ledger = SitesLedger::read(&self.ledger, &catalog, rule, &mut source)?;
        let commit = &ledger.synced_tip;
        let tree = SourceTree::read(&mut source, commit)?;
        let outcome = CommittedSites {
            repo:    &self.repo,
            source:  &mut source,
            tree:    &tree,
            catalog: &catalog,
        }
        .outcome(rule, ledger.history)?;
        let reply = SitesReply {
            schema: SITES_REPLY_SCHEMA,
            rule: &rule.id,
            observed: SitesObservation {
                synced_tip:      commit,
                rule_pin:        &rule.pin,
                ledger_revision: LedgerRevision::Absent,
            },
            outcome,
        };
        let mut line = serde_json::to_vec(&reply)?;
        line.push(b'\n');
        io::Write::write_all(&mut io::stdout().lock(), &line)?;
        Ok(())
    }
}

#[derive(Serialize)]
struct SitesReply<'a> {
    schema:   u32,
    rule:     &'a str,
    observed: SitesObservation<'a>,
    #[serde(flatten)]
    outcome:  SitesOutcome,
}

#[derive(Serialize)]
struct SitesObservation<'a> {
    synced_tip:      &'a str,
    rule_pin:        &'a str,
    ledger_revision: LedgerRevision,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum LedgerRevision {
    Absent,
}
