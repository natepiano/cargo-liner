//! List observers with an early return near the opening brace.

use regex::Regex;

use super::constants::OBSERVER_BODY_WINDOW;
use super::failure::SitesFailure;
use super::item_scan;
use super::joined_source::JoinedSource;
use super::ledger::ListedCoordinate;
use super::rust_sources;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct ObserverScanner {
    declaration: Regex,
}

impl ObserverScanner {
    fn new() -> Result<Self, SitesFailure> {
        let space = item_scan::class_ranges(SPACE);
        let word = item_scan::class_ranges(WORD);
        let declaration = Regex::new(&format!(
            r"^fn[{space}]+([{word}]+)[{space}]*(?:<[^>]*>)?[{space}]*\("
        ))
        .map_err(SitesFailure::ObserverPattern)?;
        Ok(Self { declaration })
    }

    fn scan(&self, lines: &[&str]) -> Vec<RustCoordinate> {
        let joined = JoinedSource::new(lines);
        let text = joined.text();
        let mut sites = Vec::new();
        let mut previous_end = 0;
        for (start, _) in text.match_indices("fn") {
            if start < previous_end
                || text[..start].ends_with(|character| tokenizer::in_table(WORD, character))
            {
                continue;
            }
            let Some(declaration) = self.declaration.captures(&text[start..]) else {
                continue;
            };
            let Some(whole) = declaration.get(0) else {
                continue;
            };
            previous_end = start + whole.end();
            let parameters = &text[previous_end..];
            let Some(params_end) = parameters.find(')') else {
                continue;
            };
            let params = &parameters[..params_end];
            if !params.contains("On<") && !params.contains("Trigger<") {
                continue;
            }
            let Some(brace) = parameters[params_end..].find('{') else {
                continue;
            };
            let body = joined.position(previous_end + params_end + brace);
            if lines
                .iter()
                .skip(body.line)
                .take(OBSERVER_BODY_WINDOW)
                .any(|line| line.contains("return"))
            {
                sites.push(RustCoordinate {
                    line: joined.position(start).line + 1,
                    text: format!("observer fn {} opens with an early return", &declaration[1]),
                });
            }
        }
        sites
    }
}

pub(super) fn observer_guards(
    sources: &RustSources,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = ObserverScanner::new()?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        (blob.paths.as_slice(), scanner.scan(&lines))
    });
    Ok(rust_sources::listed(candidates))
}
