//! List member dependency pins and Bevy Kana sites.

use std::collections::BTreeSet;

use regex::Regex;
use toml::Value;

use super::constants::BEVY_DEPENDENCY;
use super::constants::DEPENDENCY_SECTIONS;
use super::constants::KANA_DEPENDENCY;
use super::constants::KANA_LEAK_TYPES;
use super::failure::SitesFailure;
use super::item_scan;
use super::ledger::ListedCoordinate;
use super::manifest_inputs::ManifestBlobs;
use super::manifest_inputs::WorkspacePaths;
use super::rust_sources;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct KanaScanner {
    leak: Regex,
}

impl KanaScanner {
    fn new(space: &str) -> Result<Self, SitesFailure> {
        let word = item_scan::class_ranges(WORD);
        let leak = Regex::new(&format!(
            r"^[{space}]*pub[{space}](?:(?:.*[^{word}])?(?:{KANA_LEAK_TYPES})(?:$|[^{word}])|[{space}]*use[{space}]+.*{KANA_DEPENDENCY})"
        ))
        .map_err(SitesFailure::KanaLeakPattern)?;
        Ok(Self { leak })
    }

    fn listed(&self, sources: &RustSources) -> Vec<ListedCoordinate> {
        let candidates = sources.blobs().map(|blob| {
            let sites = blob
                .masked
                .lines()
                .enumerate()
                .filter(|(_, text)| self.leak.is_match(text))
                .map(|(index, _)| {
                    let line = index + 1;
                    RustCoordinate {
                        line,
                        text: blob.display_line(line),
                    }
                })
                .collect();
            (blob.paths.as_slice(), sites)
        });
        rust_sources::listed(candidates)
    }
}

pub(super) fn workspace_deps(
    tree: &SourceTree,
    source: &mut FindingSource<'_>,
    manifests: &mut ManifestBlobs<'_>,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let space = item_scan::class_ranges(SPACE);
    let paths = WorkspacePaths::from(tree);
    let members = paths.members(manifests.read(source, "Cargo.toml")?)?;
    let mut listed = Vec::new();
    for path in members {
        let manifest = manifests.read(source, &path)?;
        for section in DEPENDENCY_SECTIONS {
            let Some(dependencies) = manifest.table(section) else {
                continue;
            };
            for (name, dependency) in dependencies {
                if pins_version(dependency) {
                    listed.push(ListedCoordinate {
                        path: path.clone(),
                        line: manifest.dependency_line(name, &space)?,
                        text: format!(
                            "[{section}] {name} pins a version instead of workspace = true"
                        ),
                    });
                }
            }
        }
    }
    Ok(listed)
}

pub(super) fn bevy_kana_usage(
    tree: &SourceTree,
    source: &mut FindingSource<'_>,
    manifests: &mut ManifestBlobs<'_>,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let space = item_scan::class_ranges(SPACE);
    let paths = WorkspacePaths::from(tree);
    let members = paths.members(manifests.read(source, "Cargo.toml")?)?;
    let manifest_paths = std::iter::once("Cargo.toml").chain(
        members
            .iter()
            .map(String::as_str)
            .filter(|path| *path != "Cargo.toml"),
    );
    let mut listed = Vec::new();
    let mut kana_sources = BTreeSet::new();
    for path in manifest_paths {
        let manifest = manifests.read(source, path)?;
        let Some(dependencies) = manifest.table("dependencies") else {
            continue;
        };
        if dependencies.contains_key(KANA_DEPENDENCY) {
            let directory = path.strip_suffix("Cargo.toml").unwrap_or_default();
            let prefix = format!("{directory}src/");
            if paths.contains(&format!("{prefix}lib.rs")) {
                let _ = kana_sources.insert(prefix);
            }
        } else if dependencies.contains_key(BEVY_DEPENDENCY) {
            listed.push(ListedCoordinate {
                path: path.to_owned(),
                line: manifest.dependency_line(BEVY_DEPENDENCY, &space)?,
                text: "bevy crate without a hana_kana dependency".to_owned(),
            });
        }
    }
    if !kana_sources.is_empty() {
        let sources = RustSources::read(tree, source, |path| {
            path.match_indices('/').any(|(offset, _)| {
                let (prefix, relative) = path.split_at(offset + 1);
                kana_sources.contains(prefix) && rust_sources::rust_path(relative)
            })
        })?;
        listed.extend(KanaScanner::new(&space)?.listed(&sources));
    }
    Ok(listed)
}

fn pins_version(dependency: &Value) -> bool {
    dependency.as_table().map_or_else(
        || dependency.is_str(),
        |table| {
            table.contains_key("version") && table.get("workspace") != Some(&Value::Boolean(true))
        },
    )
}
