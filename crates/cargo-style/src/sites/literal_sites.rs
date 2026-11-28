//! List literals that match the production and test rules.

use regex::Regex;

use super::constants::FIXED_SPELLINGS;
use super::constants::FORMAT_MACROS;
use super::constants::IDENTITY_NUMERICS;
use super::constants::INCLUDE_MACROS;
use super::constants::LITERAL_EXEMPTIONS;
use super::constants::MIN_NUMERIC_LITERAL_DIGITS;
use super::constants::MIN_STRING_LITERAL_CHARACTERS;
use super::failure::SitesFailure;
use super::item_scan;
use super::item_scan::ItemScanner;
use super::ledger::ListedCoordinate;
use super::mask::LiteralKind;
use super::path_match;
use super::rust_sources;
use super::rust_sources::RustBlob;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;
use super::test_spans;
use super::test_spans::TestSpans;
use crate::tokenizer;
use crate::unicode_tables::DECIMAL;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct LiteralScanner {
    numeric:       Regex,
    constant:      Regex,
    format_macro:  Regex,
    include_macro: Regex,
    comparison:    Regex,
    items:         ItemScanner,
}

impl LiteralScanner {
    fn new() -> Result<Self, SitesFailure> {
        let space = item_scan::class_ranges(SPACE);
        let word = item_scan::class_ranges(WORD);
        let digit = item_scan::class_ranges(DECIMAL);
        let visibility = item_scan::visibility(&space);
        let boundary = format!(r"(?:^|[^{word}])");
        let compile = |pattern: String| Regex::new(&pattern).map_err(SitesFailure::LiteralPattern);
        Ok(Self {
            numeric:       compile(format!(
                r"^[{digit}][{digit}_]*(?:\.[{digit}][{digit}_]*)?(?:[eE][+-]?[{digit}]+)?"
            ))?,
            constant:      compile(format!(r"^[{space}]*{visibility}const[{space}]"))?,
            format_macro:  compile(format!(r"{boundary}(?:{FORMAT_MACROS})!"))?,
            include_macro: compile(format!(r"{boundary}(?:{INCLUDE_MACROS})![{space}]*\("))?,
            comparison:    compile(format!(r"{boundary}assert|[=!<>]=|matches!"))?,
            items:         ItemScanner::new()?,
        })
    }

    fn production(&self, blob: &RustBlob) -> Vec<RustCoordinate> {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let spans = TestSpans::inline(&self.items, &lines);
        let mut sites = self.numeric_sites(blob, &lines, |line, _| !spans.contains(line));
        sites.extend(blob.masked.literals().iter().filter_map(|literal| {
            if literal.kind != LiteralKind::String
                || literal
                    .text
                    .chars()
                    .nth(MIN_STRING_LITERAL_CHARACTERS - 1)
                    .is_none()
                || spans.contains(literal.line)
                || FIXED_SPELLINGS.contains(&literal.text.as_str())
            {
                return None;
            }
            let line = lines.get(literal.line - 1).copied().unwrap_or_default();
            let quote = line
                .char_indices()
                .nth(literal.column)
                .map_or(line.len(), |(offset, _)| offset);
            let prefix = &line[..quote];
            if self.excluded_line(line)
                || self.format_macro.is_match(prefix)
                || self.include_macro.is_match(prefix)
            {
                return None;
            }
            Some(RustCoordinate {
                line: literal.line,
                text: blob.display_line(literal.line),
            })
        }));
        sites
    }

    fn tests(&self, blob: &RustBlob, lines: &[&str], spans: &TestSpans) -> Vec<RustCoordinate> {
        self.numeric_sites(blob, lines, |line, text| {
            spans.contains(line) && self.comparison.is_match(text)
        })
    }

    fn numeric_sites(
        &self,
        blob: &RustBlob,
        lines: &[&str],
        mut in_scope: impl FnMut(usize, &str) -> bool,
    ) -> Vec<RustCoordinate> {
        lines
            .iter()
            .enumerate()
            .filter_map(|(index, &text)| {
                let line = index + 1;
                if !in_scope(line, text) || self.excluded_line(text) || !self.has_number(text) {
                    return None;
                }
                Some(RustCoordinate {
                    line,
                    text: blob.display_line(line),
                })
            })
            .collect()
    }

    fn excluded_line(&self, text: &str) -> bool {
        self.constant.is_match(text)
            || text
                .trim_start_matches(tokenizer::python_space)
                .starts_with('#')
    }

    fn has_number(&self, line: &str) -> bool {
        let mut previous_end = 0;
        for (start, character) in line.char_indices() {
            if start < previous_end
                || !is_decimal(character)
                || line[..start]
                    .ends_with(|character| character == '.' || tokenizer::in_table(WORD, character))
            {
                continue;
            }
            let Some(matched) = self.numeric.find(&line[start..]) else {
                continue;
            };
            previous_end = start + matched.end();
            if significant_numeric(matched.as_str()) && !array_dimension(line, start) {
                return true;
            }
        }
        false
    }
}

pub(super) fn literals(
    sources: &RustSources,
    exemptions: &[String],
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let exemptions = exemptions
        .iter()
        .cloned()
        .chain(LITERAL_EXEMPTIONS.into_iter().map(str::to_owned))
        .collect::<Vec<_>>();
    let scanner = LiteralScanner::new()?;
    let candidates = sources.blobs().filter_map(|blob| {
        let paths = blob
            .paths
            .iter()
            .filter(|path| {
                !test_spans::is_test_file(path) && !path_match::is_exempt(path, &exemptions)
            })
            .collect::<Vec<_>>();
        (!paths.is_empty()).then(|| (paths, scanner.production(blob)))
    });
    Ok(rust_sources::listed(candidates))
}

pub(super) fn test_literals(sources: &RustSources) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = LiteralScanner::new()?;
    let candidates = sources.blobs().flat_map(|blob| {
        let (test_paths, other_paths): (Vec<_>, Vec<_>) = blob
            .paths
            .iter()
            .partition(|path| test_spans::is_test_file(path));
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let test_sites = if test_paths.is_empty() {
            Vec::new()
        } else {
            scanner.tests(blob, &lines, &TestSpans::WholeFile)
        };
        let other_sites = if other_paths.is_empty() {
            Vec::new()
        } else {
            let spans = TestSpans::inline(&scanner.items, &lines);
            scanner.tests(blob, &lines, &spans)
        };
        [(test_paths, test_sites), (other_paths, other_sites)]
    });
    Ok(rust_sources::listed(candidates))
}

fn is_decimal(character: char) -> bool { tokenizer::in_table(DECIMAL, character) }

fn significant_numeric(token: &str) -> bool {
    let digits = token.replace('_', "");
    !IDENTITY_NUMERICS.contains(&digits.as_str())
        && digits
            .chars()
            .filter(|&character| is_decimal(character))
            .nth(MIN_NUMERIC_LITERAL_DIGITS - 1)
            .is_some()
}

fn array_dimension(line: &str, start: usize) -> bool {
    let (before, after) = line.split_at(start);
    before
        .trim_end_matches(tokenizer::python_space)
        .ends_with(';')
        && after
            .trim_start_matches(|character| character == '_' || is_decimal(character))
            .trim_start_matches(tokenizer::python_space)
            .starts_with(']')
}
