//! List sites produced from struct fields and enum variants.

use super::catalog::Rule;
use super::catalog::StructFieldsSpec;
use super::constants::UNIT_SUFFIXES;
use super::failure::SitesFailure;
use super::item_scan;
use super::item_scan::FieldDef;
use super::item_scan::ItemScanner;
use super::item_scan::StructDef;
use super::ledger::ListedCoordinate;
use super::regex_sites;
use super::regex_sites::PythonSearch;
use super::rust_sources;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;

pub(super) fn field_affixes(sources: &RustSources) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = ItemScanner::new()?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let sites = scanner.structs(&lines).iter().flat_map(affixes).collect();
        (blob.paths.as_slice(), sites)
    });
    Ok(rust_sources::listed(candidates))
}

fn affixes(structure: &StructDef<'_>) -> Vec<RustCoordinate> {
    if structure.fields.len() < 2 {
        return Vec::new();
    }
    let mut prefixes = structure
        .fields
        .iter()
        .map(|field| affix_word(field.name, Affix::Prefix));
    let mut suffixes = structure
        .fields
        .iter()
        .map(|field| affix_word(field.name, Affix::Suffix));
    let prefix = shared_word(&mut prefixes);
    let suffix = shared_word(&mut suffixes).filter(|word| !UNIT_SUFFIXES.contains(word));
    [("prefix", prefix), ("suffix", suffix)]
        .into_iter()
        .filter_map(|(kind, shared)| {
            let word = shared?;
            let names = structure
                .fields
                .iter()
                .map(|field| field.name)
                .collect::<Vec<_>>()
                .join(", ");
            Some(RustCoordinate {
                line: structure.line,
                text: format!(
                    "struct {}: shared {kind} '{word}' on {names}",
                    structure.name
                ),
            })
        })
        .collect()
}

#[derive(Clone, Copy)]
enum Affix {
    Prefix,
    Suffix,
}

fn affix_word(name: &str, affix: Affix) -> Option<&str> {
    let mut words = name.split('_').filter(|word| !word.is_empty());
    let first = words.next()?;
    let second = words.next()?;
    Some(match affix {
        Affix::Prefix => first,
        Affix::Suffix => words.next_back().unwrap_or(second),
    })
}

fn shared_word<'a>(words: &mut impl Iterator<Item = Option<&'a str>>) -> Option<&'a str> {
    let first = words.next().flatten()?;
    words.all(|word| word == Some(first)).then_some(first)
}

pub(super) fn enum_variant_stutter(
    sources: &RustSources,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = ItemScanner::new()?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let mut sites = Vec::new();
        for enumeration in scanner.enums(&lines) {
            let words = item_scan::camel_words(enumeration.name).collect::<Vec<_>>();
            for variant in enumeration.variants {
                let shared = item_scan::camel_words(variant.name)
                    .filter(|word| words.contains(word))
                    .min();
                if let Some(word) = shared {
                    sites.push(RustCoordinate {
                        line: variant.line,
                        text: format!(
                            "enum {}::{} repeats '{word}'",
                            enumeration.name, variant.name
                        ),
                    });
                }
            }
        }
        (blob.paths.as_slice(), sites)
    });
    Ok(rust_sources::listed(candidates))
}

pub(super) fn field_type_stutter(
    sources: &RustSources,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = ItemScanner::new()?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let mut sites = Vec::new();
        for structure in scanner.structs(&lines) {
            let snake = item_scan::snake_case(structure.name);
            let first = snake.split('_').next().unwrap_or_default();
            for field in structure.fields {
                let named_for_struct = starts_with_word(field.name, &snake)
                    || (!first.is_empty() && starts_with_word(field.name, first));
                if named_for_struct
                    && !scanner
                        .type_idents(field.type_text)
                        .any(|name| item_scan::snake_case(name) == field.name)
                {
                    sites.push(RustCoordinate {
                        line: field.line,
                        text: format!(
                            "struct {}: field `{}` restates the struct name",
                            structure.name, field.name
                        ),
                    });
                }
            }
        }
        (blob.paths.as_slice(), sites)
    });
    Ok(rust_sources::listed(candidates))
}

fn starts_with_word(name: &str, word: &str) -> bool {
    name.strip_prefix(word)
        .is_some_and(|tail| tail.is_empty() || tail.starts_with('_'))
}

pub(super) fn struct_fields(
    sources: &RustSources,
    rule: &Rule,
    spec: &StructFieldsSpec,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    if spec.field_type_pattern.is_empty() {
        return Err(SitesFailure::FieldTypePattern {
            rule:   rule.id.clone(),
            origin: spec.origin.clone(),
        });
    }
    let scanner = ItemScanner::new()?;
    let groups = sources
        .blobs()
        .map(|blob| {
            let lines = blob.masked.lines().collect::<Vec<_>>();
            let fields = scanner
                .structs(&lines)
                .into_iter()
                .filter(|structure| {
                    spec.structs.is_empty()
                        || spec.structs.iter().any(|name| name == structure.name)
                })
                .flat_map(|structure| {
                    structure.fields.into_iter().map(move |field| StructField {
                        structure: structure.name,
                        field,
                    })
                })
                .collect::<Vec<_>>();
            (blob.paths.as_slice(), fields)
        })
        .collect::<Vec<_>>();
    let type_texts = groups
        .iter()
        .flat_map(|(_, fields)| fields.iter().map(|site| site.field.type_text))
        .collect::<Vec<_>>();
    let matches = regex_sites::python_matches(&[PythonSearch {
        rule:    &rule.id,
        pattern: &spec.field_type_pattern,
        lines:   type_texts,
    }])?;
    let mut matches = matches.into_iter();
    let candidates = groups.into_iter().map(|(paths, fields)| {
        let sites = fields
            .into_iter()
            .zip(matches.by_ref())
            .filter(|(_, matched)| *matched)
            .map(|(site, _)| site.coordinate())
            .collect();
        (paths, sites)
    });
    Ok(rust_sources::listed(candidates))
}

struct StructField<'a> {
    structure: &'a str,
    field:     FieldDef<'a>,
}

impl StructField<'_> {
    fn coordinate(self) -> RustCoordinate {
        RustCoordinate {
            line: self.field.line,
            text: format!(
                "struct {}: field `{}: {}`",
                self.structure, self.field.name, self.field.type_text
            ),
        }
    }
}
