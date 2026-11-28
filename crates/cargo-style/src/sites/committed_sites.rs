//! List site coordinates and compute their identities at the synced commit.

use std::cell::OnceCell;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::Connection;
use rusqlite::Error;
use serde::Serialize;
use sha2::Digest;
use sha2::Sha256;

use super::allow_sites;
use super::catalog::Catalog;
use super::catalog::Generator;
use super::catalog::Listing;
use super::catalog::Rule;
use super::constants::HEX_DIGITS;
use super::failure::SitesFailure;
use super::ledger;
use super::ledger::ListedCoordinate;
use super::ledger::ListedHistory;
use super::ledger::ListedMembership;
use super::ledger::SitesLedgerFailure;
use super::literal_sites;
use super::manifest_inputs::ManifestBlobs;
use super::manifest_sites;
use super::module_sites;
use super::observer_sites;
use super::prefilter_sites;
use super::regex_sites;
use super::regex_sites::RegexTool;
use super::rust_sources;
use super::rust_sources::RustSources;
use super::site_scope::SiteScope;
use super::source_tree::SourceTree;
use super::trait_sites;
use super::type_sites;
use crate::finding_source::FindingSource;
use crate::tip_view::RepositoryTip;

pub(super) struct CommittedSites<'a, 'repo> {
    pub(super) repo:    &'a Path,
    pub(super) source:  &'a mut FindingSource<'repo>,
    pub(super) tree:    &'a SourceTree,
    pub(super) catalog: &'a Catalog,
}

impl<'a> CommittedSites<'a, '_> {
    pub(super) fn outcome(
        mut self,
        rule: &Rule,
        history: ListedHistory,
    ) -> Result<SitesOutcome, SitesFailure> {
        let mut manifests = ManifestBlobs::new(self.tree);
        manifests.validate_root(self.source)?;
        let checks_regex_tool = match &rule.listing {
            Listing::Candidates(Generator::Regex(_)) => true,
            Listing::Prefilter(_) => self.catalog.has_regex_generator(),
            _ => false,
        };
        let regex_tool = OnceCell::new();
        if checks_regex_tool {
            let _ = RegexTool::read_once(&regex_tool)?;
        }
        let coordinates = match &rule.listing {
            Listing::NoSites => Vec::new(),
            Listing::Candidates(Generator::Unsupported { .. }) => {
                return Ok(SitesOutcome::Unavailable {
                    reason: UnavailableReason::UnsupportedGenerator,
                });
            },
            Listing::Candidates(generator) => match history {
                ListedHistory::UseLegacy(coordinates) => coordinates,
                ListedHistory::Enumerate => {
                    let scope = self.scope()?;
                    let mut coordinates =
                        self.generate(rule, generator, &scope, &regex_tool, &mut manifests)?;
                    coordinates.sort_unstable();
                    coordinates.dedup();
                    coordinates
                },
            },
            Listing::Prefilter(_) => match history {
                ListedHistory::UseLegacy(coordinates) => coordinates,
                ListedHistory::Enumerate => self.prefilter(rule, &regex_tool, &mut manifests)?,
            },
        };
        let sites = coordinates
            .into_iter()
            .map(|coordinate| site(self.tree, &rule.id, &rule.pin, coordinate))
            .collect::<Result<_, _>>()?;
        Ok(SitesOutcome::Listed { sites })
    }

    pub(super) fn membership(
        mut self,
        connection: &Connection,
        commit: &str,
        tip: &RepositoryTip,
    ) -> Result<SiteMembership, SitesFailure> {
        let regex_tool = OnceCell::new();
        if self.catalog.has_regex_generator() {
            let _ = RegexTool::read_once(&regex_tool)?;
        }
        if matches!(
            ledger::listing_source(connection, self.catalog, commit)
                .map_err(SitesLedgerFailure::from)?,
            ListedMembership::KeepRows
        ) {
            return Ok(SiteMembership::read_legacy(connection, commit)
                .map_err(SitesLedgerFailure::from)?);
        }
        let scope = self.scope()?;
        let mut manifests = ManifestBlobs::new(self.tree);
        let mut membership = self.generator_membership(&scope, &regex_tool, &mut manifests)?;
        if self.catalog.prefilter_rules().next().is_some() {
            let universe = membership.universe(tip.rust_paths());
            prefilter_sites::enumerate(
                self.source,
                self.tree,
                self.catalog,
                &universe,
                |rule, line| {
                    membership.include(&rule.id, line.path.to_owned());
                },
            )?;
        }
        // Python inserts `listed_rules` after it enumerates all sites.
        ledger::validate_listing_keys(self.catalog)?;
        Ok(membership)
    }

    fn scope(&mut self) -> Result<SiteScope<'a>, SitesFailure> {
        self.tree.verify_blobs(self.repo)?;
        SiteScope::read(self.tree, self.source)
    }

    fn prefilter(
        &mut self,
        requested: &Rule,
        regex_tool: &OnceCell<RegexTool>,
        manifests: &mut ManifestBlobs<'_>,
    ) -> Result<Vec<ListedCoordinate>, SitesFailure> {
        let scope = self.scope()?;
        let membership = self.generator_membership(&scope, regex_tool, manifests)?;
        let universe = membership.universe(scope.rust_paths());
        let mut coordinates = Vec::new();
        prefilter_sites::enumerate(
            self.source,
            self.tree,
            self.catalog,
            &universe,
            |rule, line| {
                if rule.id == requested.id {
                    coordinates.push(ListedCoordinate {
                        path: line.path.to_owned(),
                        line: line.number,
                        text: line.text.to_owned(),
                    });
                }
            },
        )?;
        coordinates.sort_unstable();
        coordinates.dedup();
        Ok(coordinates)
    }

    fn generator_membership(
        &mut self,
        scope: &SiteScope<'_>,
        regex_tool: &OnceCell<RegexTool>,
        manifests: &mut ManifestBlobs<'_>,
    ) -> Result<SiteMembership, SitesFailure> {
        let mut membership = SiteMembership::default();
        for (rule, generator) in self.catalog.generator_rules() {
            let coordinates = self.generate(rule, generator, scope, regex_tool, manifests)?;
            membership
                .paths
                .entry(rule.id.clone())
                .or_default()
                .extend(coordinates.into_iter().map(|coordinate| coordinate.path));
        }
        Ok(membership)
    }

    fn generate(
        &mut self,
        rule: &Rule,
        generator: &Generator,
        scope: &SiteScope<'_>,
        regex_tool: &OnceCell<RegexTool>,
        manifests: &mut ManifestBlobs<'_>,
    ) -> Result<Vec<ListedCoordinate>, SitesFailure> {
        let mut coordinates = match generator {
            Generator::Regex(spec) => {
                let tool = RegexTool::read_once(regex_tool)?;
                Ok(regex_sites::enumerate(
                    self.source,
                    self.tree,
                    rule,
                    spec,
                    tool,
                )?)
            },
            Generator::FieldAffixes => self.rust(type_sites::field_affixes),
            Generator::EnumVariantStutter => self.rust(type_sites::enum_variant_stutter),
            Generator::FieldTypeStutter => self.rust(type_sites::field_type_stutter),
            Generator::StructFields(spec) => {
                self.rust(|sources| type_sites::struct_fields(sources, rule, spec))
            },
            Generator::SingleImplTraits => self.rust(trait_sites::single_impl_traits),
            Generator::AllowsWithoutReason => self.rust(allow_sites::allows_without_reason),
            Generator::TestAllowBoilerplate => self.rust(allow_sites::test_allow_boilerplate),
            Generator::ObserverGuards => self.rust(observer_sites::observer_guards),
            Generator::Literals { exemptions } => {
                self.rust(|sources| literal_sites::literals(sources, exemptions))
            },
            Generator::TestLiterals => self.rust(literal_sites::test_literals),
            Generator::ModuleRootItems => {
                let sources = RustSources::read(self.tree, self.source, |path| {
                    rust_sources::rust_path(path) && module_sites::is_module_root(path)
                })?;
                module_sites::module_root_items(&sources)
            },
            Generator::SubmoduleNames => Ok(module_sites::submodule_names(self.tree)),
            Generator::WorkspaceDeps => {
                manifest_sites::workspace_deps(self.tree, self.source, manifests)
            },
            Generator::BevyKanaUsage => {
                manifest_sites::bevy_kana_usage(self.tree, self.source, manifests)
            },
            Generator::Unsupported { kind, origin } => Err(SitesFailure::UnsupportedGenerator {
                rule:   rule.id.clone(),
                kind:   kind.clone(),
                origin: origin.clone(),
            }),
        }?;
        coordinates.retain(|coordinate| scope.includes(&coordinate.path, &rule.reach));
        Ok(coordinates)
    }

    fn rust(
        &mut self,
        generate: impl FnOnce(&RustSources) -> Result<Vec<ListedCoordinate>, SitesFailure>,
    ) -> Result<Vec<ListedCoordinate>, SitesFailure> {
        let sources = RustSources::read(self.tree, self.source, rust_sources::rust_path)?;
        generate(&sources)
    }
}

impl Catalog {
    pub(crate) fn membership(
        &self,
        repo: &Path,
        connection: &Connection,
        commit: &str,
        source: &mut FindingSource<'_>,
        tip: &RepositoryTip,
    ) -> Result<SiteMembership, SitesFailure> {
        let tree = SourceTree::read(source, commit)?;
        CommittedSites {
            repo,
            source,
            tree: &tree,
            catalog: self,
        }
        .membership(connection, commit, tip)
    }
}

/// Paths that have a listed site at the synced tip, by rule.
#[derive(Default)]
pub(crate) struct SiteMembership {
    paths: BTreeMap<String, BTreeSet<String>>,
}

impl SiteMembership {
    pub(crate) fn paths(&self, rule: &str) -> impl Iterator<Item = &str> {
        self.paths
            .get(rule)
            .into_iter()
            .flatten()
            .map(String::as_str)
    }

    fn include(&mut self, rule: &str, path: String) {
        if let Some(paths) = self.paths.get_mut(rule) {
            paths.insert(path);
        } else {
            self.paths.insert(rule.to_owned(), BTreeSet::from([path]));
        }
    }

    fn universe<'p>(&self, rust_paths: impl Iterator<Item = &'p str>) -> BTreeSet<String> {
        rust_paths
            .map(str::to_owned)
            .chain(self.paths.values().flatten().cloned())
            .collect()
    }

    fn read_legacy(connection: &Connection, commit: &str) -> Result<Self, Error> {
        let mut statement =
            connection.prepare("SELECT DISTINCT rule,path FROM listed_sites WHERE tip=?1")?;
        let mut membership = Self::default();
        let mut rows = statement.query([commit])?;
        while let Some(row) = rows.next()? {
            membership
                .paths
                .entry(row.get(0)?)
                .or_default()
                .insert(row.get(1)?);
        }
        Ok(membership)
    }
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum SitesOutcome {
    Listed { sites: Vec<Site> },
    Unavailable { reason: UnavailableReason },
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum UnavailableReason {
    UnsupportedGenerator,
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
