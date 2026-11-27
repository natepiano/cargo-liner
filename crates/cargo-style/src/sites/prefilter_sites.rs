//! Find raw lines that match a rule's `pre_filter` in the shared path universe.

use std::collections::BTreeSet;
use std::rc::Rc;

use super::catalog::Catalog;
use super::catalog::Rule;
use super::failure::SitesFailure;
use super::frontmatter;
use super::ledger::ListedCoordinate;
use super::regex_sites;
use super::regex_sites::PythonSearch;
use super::regex_sites::RegexFailure;
use super::source_tree::SiteText;
use super::source_tree::SourceTree;
use crate::finding_source::FindingSource;
use crate::source_lines::SourceLines;

struct PrefilterFile<'a> {
    path:   &'a str,
    source: Rc<SourceLines>,
}

impl PrefilterFile<'_> {
    fn lines(&self) -> impl Iterator<Item = PrefilterLine<'_>> {
        // SourceLines groups CRLF and splits at lone CR. Removing each ending matches
        // Python splitlines after newline conversion.
        self.source
            .lines()
            .enumerate()
            .map(|(index, line)| PrefilterLine {
                path:   self.path,
                number: index + 1,
                text:   frontmatter::without_line_ending(line),
            })
    }
}

struct PrefilterLine<'a> {
    path:   &'a str,
    number: usize,
    text:   &'a str,
}

struct PrefilterScan<'a> {
    rule:    &'a Rule,
    pattern: &'a str,
    files:   Vec<PrefilterFile<'a>>,
}

impl PrefilterScan<'_> {
    fn lines(&self) -> impl Iterator<Item = PrefilterLine<'_>> {
        self.files.iter().flat_map(PrefilterFile::lines)
    }
}

pub(super) fn enumerate(
    source: &mut FindingSource<'_>,
    tree: &SourceTree,
    catalog: &Catalog,
    requested: &Rule,
    universe: &BTreeSet<String>,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let mut scans = Vec::new();
    let mut read_result = Ok(());
    for (rule, pattern) in catalog.prefilter_rules() {
        match read_files(source, tree, rule, universe) {
            Ok(files) => scans.push(PrefilterScan {
                rule,
                pattern,
                files,
            }),
            Err(error) => {
                // Keep the pattern from the failed read_files call. python_matches
                // compiles it before the request reports the source error.
                scans.push(PrefilterScan {
                    rule,
                    pattern,
                    files: Vec::new(),
                });
                read_result = Err(error);
                break;
            },
        }
    }
    let searches = scans
        .iter()
        .map(|scan| PythonSearch {
            rule:    &scan.rule.id,
            pattern: scan.pattern,
            lines:   scan.lines().map(|line| line.text).collect(),
        })
        .collect::<Vec<_>>();
    let matches = regex_sites::python_matches(&searches).map_err(|error| match error {
        RegexFailure::InvalidPythonPattern { rule, diagnostic } => {
            SitesFailure::PrefilterPattern { rule, diagnostic }
        },
        other => other.into(),
    })?;
    read_result?;
    let mut matches = matches.into_iter();
    let mut coordinates = Vec::new();
    for (scan, search) in scans.iter().zip(searches) {
        let flags = matches.by_ref().take(search.lines.len());
        if scan.rule.id == requested.id {
            coordinates.extend(scan.lines().zip(flags).filter(|(_, matched)| *matched).map(
                |(line, _)| ListedCoordinate {
                    path: line.path.to_owned(),
                    line: line.number,
                    text: line.text.to_owned(),
                },
            ));
        } else {
            let _ = flags.count();
        }
    }
    coordinates.sort_unstable();
    coordinates.dedup();
    Ok(coordinates)
}

fn read_files<'a>(
    source: &mut FindingSource<'_>,
    tree: &SourceTree,
    rule: &Rule,
    universe: &'a BTreeSet<String>,
) -> Result<Vec<PrefilterFile<'a>>, SitesFailure> {
    universe
        .iter()
        .filter(|path| rule.reach.includes(path))
        .map(|path| {
            let SiteText::Utf8(text) = tree.read_text(source, path)? else {
                return Err(SitesFailure::PrefilterEncoding {
                    rule: rule.id.clone(),
                    path: path.clone(),
                });
            };
            Ok(PrefilterFile { path, source: text })
        })
        .collect()
}
