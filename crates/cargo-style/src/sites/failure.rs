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
    #[error("cannot compile the module root item pattern")]
    ModuleRootPattern(#[source] regex::Error),
    #[error("cannot compile the Bevy Kana leak pattern")]
    KanaLeakPattern(#[source] regex::Error),
    #[error("cannot compile the manifest dependency pattern")]
    DependencyPattern(#[source] regex::Error),
    #[error("invalid committed workspace member pattern")]
    WorkspacePattern(#[source] regex::Error),
    #[error("manifest is not UTF-8: {0:?}")]
    ManifestEncoding(String),
    #[error("{path:?} is not in the subpath of {root:?}")]
    WorkspaceMemberOutsideRoot { path: PathBuf, root: PathBuf },
    #[error("invalid committed ignore pattern {pattern:?}")]
    IgnorePattern {
        pattern: String,
        #[source]
        source:  regex::Error,
    },
    #[error("cannot parse committed ignore file {path:?}")]
    IgnoreConfig {
        path:   String,
        #[source]
        source: Box<toml::de::Error>,
    },
    #[error("{rule}: candidates kind=struct_fields in {origin} needs field_type_pattern")]
    FieldTypePattern { rule: String, origin: PathBuf },
    #[error("listed site {0} has no committed blob")]
    SiteBlob(String),
    #[error("cannot encode the sites reply")]
    Encode(#[from] serde_json::Error),
    #[error("cannot write the sites reply")]
    Output(#[from] io::Error),
}
