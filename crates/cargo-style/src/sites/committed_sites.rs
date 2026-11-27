//! List site coordinates and compute their identities at the synced commit.

use std::path::Path;

use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use super::allow_sites;
use super::catalog::Listing;
use super::catalog::RegexSpec;
use super::catalog::Rule;
use super::catalog::UnavailableReason;
use super::constants::HEX_DIGITS;
use super::failure::SitesFailure;
use super::ledger::ListedCoordinate;
use super::ledger::ListedHistory;
use super::literal_sites;
use super::manifest_sites;
use super::module_sites;
use super::observer_sites;
use super::regex_sites;
use super::rust_sources;
use super::rust_sources::RustSources;
use super::site_scope::SiteScope;
use super::source_tree::SourceTree;
use super::trait_sites;
use super::type_sites;
use crate::finding_source::FindingSource;

pub(super) struct CommittedSites<'a, 'repo> {
    pub(super) repo:   &'a Path,
    pub(super) source: &'a mut FindingSource<'repo>,
    pub(super) tree:   &'a SourceTree,
    pub(super) rule:   &'a Rule,
}

impl CommittedSites<'_, '_> {
    pub(super) fn outcome(self, history: ListedHistory) -> Result<SitesOutcome, SitesFailure> {
        match &self.rule.listing {
            Listing::Regex(spec) => self.regex(history, spec),
            Listing::FieldAffixes => self.rust(history, type_sites::field_affixes),
            Listing::EnumVariantStutter => self.rust(history, type_sites::enum_variant_stutter),
            Listing::FieldTypeStutter => self.rust(history, type_sites::field_type_stutter),
            Listing::StructFields(spec) => {
                let rule = self.rule;
                self.rust(history, |sources| {
                    type_sites::struct_fields(sources, rule, spec)
                })
            },
            Listing::SingleImplTraits => self.rust(history, trait_sites::single_impl_traits),
            Listing::AllowsWithoutReason => self.rust(history, allow_sites::allows_without_reason),
            Listing::TestAllowBoilerplate => {
                self.rust(history, allow_sites::test_allow_boilerplate)
            },
            Listing::ObserverGuards => self.rust(history, observer_sites::observer_guards),
            Listing::Literals { exemptions } => self.rust(history, |sources| {
                literal_sites::literals(sources, exemptions)
            }),
            Listing::TestLiterals => self.rust(history, literal_sites::test_literals),
            Listing::ModuleRootItems => self.rust_paths(
                history,
                |path| rust_sources::rust_path(path) && module_sites::is_module_root(path),
                module_sites::module_root_items,
            ),
            Listing::SubmoduleNames => self.list(history, |listing| {
                Ok(module_sites::submodule_names(listing.tree))
            }),
            Listing::WorkspaceDeps => self.list(history, |listing| {
                manifest_sites::workspace_deps(listing.tree, listing.source)
            }),
            Listing::BevyKanaUsage => self.list(history, |listing| {
                manifest_sites::bevy_kana_usage(listing.tree, listing.source)
            }),
            Listing::NoSites => Ok(SitesOutcome::Listed { sites: Vec::new() }),
            Listing::Unavailable(reason) => Ok(SitesOutcome::Unavailable { reason: *reason }),
        }
    }

    fn regex(self, history: ListedHistory, spec: &RegexSpec) -> Result<SitesOutcome, SitesFailure> {
        let tool = regex_sites::RegexTool::read()?;
        self.list(history, |listing| {
            Ok(regex_sites::enumerate(
                listing.repo,
                listing.source,
                listing.tree,
                listing.rule,
                spec,
                &tool,
            )?)
        })
    }

    fn rust(
        self,
        history: ListedHistory,
        generate: impl FnOnce(&RustSources) -> Result<Vec<ListedCoordinate>, SitesFailure>,
    ) -> Result<SitesOutcome, SitesFailure> {
        self.rust_paths(history, rust_sources::rust_path, generate)
    }

    fn rust_paths(
        self,
        history: ListedHistory,
        includes: impl Fn(&str) -> bool,
        generate: impl FnOnce(&RustSources) -> Result<Vec<ListedCoordinate>, SitesFailure>,
    ) -> Result<SitesOutcome, SitesFailure> {
        self.list(history, |listing| {
            let sources = RustSources::read(listing.tree, listing.source, includes)?;
            generate(&sources)
        })
    }

    fn list(
        mut self,
        history: ListedHistory,
        generate: impl FnOnce(&mut Self) -> Result<Vec<ListedCoordinate>, SitesFailure>,
    ) -> Result<SitesOutcome, SitesFailure> {
        let coordinates = match history {
            // Enumerate only the requested rule.
            ListedHistory::Enumerate => {
                self.tree.verify_blobs(self.repo)?;
                let scope = SiteScope::read(self.tree, self.source, &self.rule.reach)?;
                let mut coordinates = generate(&mut self)?;
                coordinates.retain(|coordinate| scope.includes(&coordinate.path));
                coordinates.sort_unstable();
                coordinates.dedup();
                coordinates
            },
            ListedHistory::UseLegacy(coordinates) => coordinates,
        };
        let sites = coordinates
            .into_iter()
            .map(|coordinate| site(self.tree, &self.rule.id, &self.rule.pin, coordinate))
            .collect::<Result<_, _>>()?;
        Ok(SitesOutcome::Listed { sites })
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SitesOutcome {
    Listed { sites: Vec<Site> },
    Unavailable { reason: UnavailableReason },
}

#[derive(Serialize)]
pub(super) struct Site {
    file: String,
    line: usize,
    #[serde(rename = "site_id")]
    id:   String,
}

fn site(
    tree: &SourceTree,
    rule: &str,
    pin: &str,
    coordinate: ListedCoordinate,
) -> Result<Site, SitesFailure> {
    let blob = tree
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
