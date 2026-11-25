//! List a rule's sites at the ledger's synced commit.

mod catalog;
mod constants;
mod frontmatter;
mod ledger;
mod path_match;
mod regex_sites;

use std::ffi::OsString;
use std::io;
use std::path::PathBuf;

use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use self::catalog::Catalog;
use self::catalog::CatalogFailure;
use self::catalog::Listing;
use self::constants::HEX_DIGITS;
use self::constants::SITES_REPLY_SCHEMA;
pub(crate) use self::constants::SITES_SUBCOMMAND;
use self::ledger::ListedCoordinate;
use self::ledger::ListedHistory;
use self::ledger::SitesLedger;
use self::ledger::SitesLedgerFailure;
use self::regex_sites::RegexFailure;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::tip_view::RepositoryTip;

pub(crate) struct SitesRequest {
    repo:     PathBuf,
    ledger:   PathBuf,
    checkout: CheckoutSource,
    rule:     String,
}

enum CheckoutSource {
    Repository,
    Explicit(PathBuf),
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SitesFailure {
    #[error("usage: cargo style sites --repo PATH --ledger PATH [--checkout PATH] RULE")]
    Arguments,
    #[error(transparent)]
    Catalog(#[from] CatalogFailure),
    #[error(transparent)]
    Ledger(#[from] SitesLedgerFailure),
    #[error(transparent)]
    Source(#[from] FindingSourceFailure),
    #[error(transparent)]
    Regex(#[from] RegexFailure),
    #[error("listed site {0} has no committed blob")]
    SiteBlob(String),
    #[error("cannot encode the sites reply")]
    Encode(#[from] serde_json::Error),
    #[error("cannot write the sites reply")]
    Output(#[from] io::Error),
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
                Some("--checkout") => &mut checkout,
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
        let checkout = match &self.checkout {
            CheckoutSource::Repository => &self.repo,
            CheckoutSource::Explicit(path) => path,
        };
        let catalog = Catalog::read(checkout)?;
        let rule = catalog.rule(&self.rule)?;
        let mut source = FindingSource::open(&self.repo)?;
        let ledger = SitesLedger::read(&self.ledger, &catalog, rule, &mut source)?;
        let commit = &ledger.synced_tip;
        let tip = source.tip(commit)?;
        let outcome = match &rule.listing {
            Listing::Regex(spec) => {
                let tool = regex_sites::RegexTool::read()?;
                let coordinates = match ledger.history {
                    // Enumerate only this rule. Python `_refresh_candidate_sites` enumerates every
                    // tier B and D rule and fails when one of them fails.
                    ListedHistory::Enumerate => regex_sites::enumerate(
                        &self.repo,
                        &mut source,
                        &tip,
                        commit,
                        rule,
                        spec,
                        &tool,
                    )?,
                    ListedHistory::UseLegacy(coordinates) => coordinates,
                };
                SitesOutcome::Listed {
                    sites: coordinates
                        .into_iter()
                        .map(|coordinate| site(&tip, &rule.id, &rule.pin, coordinate))
                        .collect::<Result<_, _>>()?,
                }
            },
            Listing::NoSites => SitesOutcome::Listed { sites: Vec::new() },
            Listing::Unavailable(reason) => SitesOutcome::Unavailable { reason: *reason },
        };
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

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum SitesOutcome {
    Listed { sites: Vec<Site> },
    Unavailable { reason: UnavailableReason },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum UnavailableReason {
    UnsupportedGenerator,
    PrefilterRequiresSharedMembership,
}

#[derive(Serialize)]
struct Site {
    file: String,
    line: usize,
    #[serde(rename = "site_id")]
    id:   String,
}

fn site(
    tip: &RepositoryTip,
    rule: &str,
    pin: &str,
    coordinate: ListedCoordinate,
) -> Result<Site, SitesFailure> {
    let blob = tip
        .blob(&coordinate.path)
        .ok_or_else(|| SitesFailure::SiteBlob(coordinate.path.clone()))?;
    let identity = serde_json::to_vec(&(
        rule,
        pin,
        &coordinate.path,
        blob,
        coordinate.line,
        &coordinate.text,
    ))?;
    Ok(Site {
        file: coordinate.path,
        line: coordinate.line,
        id:   hexadecimal(&Sha256::digest(identity)),
    })
}

fn hexadecimal(bytes: &[u8]) -> String {
    bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0x0f])
        .map(|nibble| char::from(HEX_DIGITS[usize::from(nibble)]))
        .collect()
}
