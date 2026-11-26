//! Report errors from the sites command.

use std::io;
use std::path::PathBuf;

use super::catalog::CatalogFailure;
use super::ledger::SitesLedgerFailure;
use super::regex_sites::RegexFailure;
use super::source_tree::SourceReadFailure;
use crate::finding_source::FindingSourceFailure;

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
    #[error(transparent)]
    SourceTree(#[from] SourceReadFailure),
    #[error("cannot compile the Rust item scanner")]
    ItemPattern(#[source] regex::Error),
    #[error("cannot compile the Rust trait declaration pattern")]
    TraitDeclarationPattern(#[source] regex::Error),
    #[error("cannot compile the Rust allow patterns")]
    AllowPattern(#[source] regex::Error),
    #[error("cannot compile the Rust observer declaration pattern")]
    ObserverPattern(#[source] regex::Error),
    #[error("cannot compile the Rust literal patterns")]
    LiteralPattern(#[source] regex::Error),
    #[error("{rule}: candidates kind=struct_fields in {origin} needs field_type_pattern")]
    FieldTypePattern { rule: String, origin: PathBuf },
    #[error("listed site {0} has no committed blob")]
    SiteBlob(String),
    #[error("cannot encode the sites reply")]
    Encode(#[from] serde_json::Error),
    #[error("cannot write the sites reply")]
    Output(#[from] io::Error),
}
