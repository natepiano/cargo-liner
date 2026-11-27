//! List trait declarations by their spelling and project-wide impl count.

use std::collections::BTreeMap;

use regex::Regex;

use super::failure::SitesFailure;
use super::item_scan;
use super::ledger::ListedCoordinate;
use super::rust_sources;
use super::rust_sources::RustBlob;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct TraitDeclaration<'a> {
    name: &'a str,
    line: usize,
}

struct TraitBlob<'a> {
    paths:        &'a [String],
    declarations: Vec<TraitDeclaration<'a>>,
    impl_clauses: Vec<Vec<&'a str>>,
}

impl<'a> TraitBlob<'a> {
    fn scan(blob: &'a RustBlob, declaration: &Regex) -> Self {
        let declarations = blob
            .masked
            .lines()
            .enumerate()
            .filter_map(|(index, line)| {
                let name = declaration.captures(line)?.get(1)?.as_str();
                Some(TraitDeclaration {
                    name,
                    line: index + 1,
                })
            })
            .collect();
        let impl_clauses = blob
            .masked
            .text()
            .split(['{', ';'])
            .filter(|segment| segment.contains("impl") && segment.contains("for"))
            .map(|segment| {
                segment
                    .split(|character| !tokenizer::in_table(WORD, character))
                    .filter(|word| !word.is_empty())
                    .collect()
            })
            .collect();
        Self {
            paths: &blob.paths,
            declarations,
            impl_clauses,
        }
    }

    fn impl_count(&self, name: &str) -> usize {
        self.impl_clauses
            .iter()
            .map(|words| count_matches(words, name))
            .sum()
    }
}

pub(super) fn single_impl_traits(
    sources: &RustSources,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let space = item_scan::class_ranges(SPACE);
    let word = item_scan::class_ranges(WORD);
    let visibility = item_scan::visibility(&space);
    let declaration = Regex::new(&format!(
        r"^[{space}]*{visibility}(?:unsafe[{space}]+)?trait[{space}]+([A-Z][{word}]*)",
    ))
    .map_err(SitesFailure::TraitDeclarationPattern)?;
    let blobs = sources
        .blobs()
        .map(|blob| TraitBlob::scan(blob, &declaration))
        .collect::<Vec<_>>();
    let mut counts = BTreeMap::new();
    for site in blobs.iter().flat_map(|blob| &blob.declarations) {
        let _ = counts
            .entry(site.name)
            .or_insert_with(|| project_count(&blobs, site.name));
    }
    let candidates = blobs.iter().map(|blob| {
        let sites = blob
            .declarations
            .iter()
            .filter_map(|site| {
                let count = counts[site.name];
                (count <= 1).then(|| RustCoordinate {
                    line: site.line,
                    text: format!("trait {}: {count} impl(s) project-wide", site.name),
                })
            })
            .collect();
        (blob.paths, sites)
    });
    Ok(rust_sources::listed(candidates))
}

fn project_count(blobs: &[TraitBlob<'_>], name: &str) -> usize {
    let mut count = 0;
    for blob in blobs {
        count += blob.impl_count(name) * blob.paths.len();
        if count > 1 {
            break;
        }
    }
    count
}

fn count_matches(words: &[&str], name: &str) -> usize {
    let mut remaining = words.iter().copied();
    let mut count = 0;
    // Python finds non-overlapping matches within each clause.
    while remaining.any(|word| word == "impl")
        && remaining.any(|word| word == name)
        && remaining.any(|word| word == "for")
    {
        count += 1;
    }
    count
}
