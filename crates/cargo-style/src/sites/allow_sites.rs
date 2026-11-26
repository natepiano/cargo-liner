//! List allow attributes without reasons and allow lints with no scope matches.

use regex::Regex;

use super::catalog::Rule;
use super::constants::BOILERPLATE_LINTS;
use super::failure::SitesFailure;
use super::item_scan;
use super::item_scan::BodyClose;
use super::item_scan::FollowingBody;
use super::item_scan::ItemScanner;
use super::item_scan::Position;
use super::joined_source::JoinedSource;
use super::ledger::ListedCoordinate;
use super::rust_sources;
use super::rust_sources::RustCoordinate;
use super::rust_sources::RustSources;
use crate::tip_view::RepositoryTip;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct BoilerplateAllow<'a> {
    position: Position,
    tracked:  Vec<(&'static str, &'static str)>,
    scope:    &'a str,
}

impl<'a> BoilerplateAllow<'a> {
    fn stale(self) -> impl Iterator<Item = RustCoordinate> + 'a {
        let scope = self.scope;
        let line = self.position.line + 1;
        self.tracked
            .into_iter()
            .filter(move |&(_, pattern)| !scope.contains(pattern))
            .map(move |(lint, pattern)| RustCoordinate {
                line,
                text: format!("strip {lint} — 0 matches of `{pattern}` in scope"),
            })
    }
}

struct BoilerplateScanner {
    attributes: Regex,
    items:      ItemScanner,
}

impl BoilerplateScanner {
    fn new() -> Result<Self, SitesFailure> {
        let space = item_scan::class_ranges(SPACE);
        Ok(Self {
            attributes: allow_attributes(&space)?,
            items:      ItemScanner::new()?,
        })
    }

    fn allows<'a>(
        &'a self,
        joined: &'a JoinedSource,
        lines: &'a [&str],
    ) -> impl Iterator<Item = BoilerplateAllow<'a>> {
        let text = joined.text();
        self.attributes
            .captures_iter(text)
            .filter_map(move |matched| {
                let fields = matched.get(1)?.as_str();
                let tracked = fields
                    .split(',')
                    .map(|lint| lint.trim_matches(tokenizer::python_space))
                    .filter_map(|lint| {
                        BOILERPLATE_LINTS
                            .iter()
                            .copied()
                            .find(|(name, _)| *name == lint)
                    })
                    .collect::<Vec<_>>();
                if tracked.is_empty() {
                    return None;
                }
                let attribute = matched.get(0)?;
                let after = joined.position(attribute.end());
                let end = match self.items.body_after(lines, after) {
                    FollowingBody::Present(BodyClose::ClosedAt(close)) => joined.offset(close) + 1,
                    FollowingBody::Present(BodyClose::Unclosed) | FollowingBody::Absent => {
                        text.len()
                    },
                };
                Some(BoilerplateAllow {
                    position: joined.position(attribute.start()),
                    tracked,
                    scope: &text[attribute.end()..end],
                })
            })
    }
}

pub(super) fn allows_without_reason(
    sources: &RustSources,
    tip: &RepositoryTip,
    rule: &Rule,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let space = item_scan::class_ranges(SPACE);
    let word = item_scan::class_ranges(WORD);
    let attributes = allow_attributes(&space)?;
    let reason = Regex::new(&format!(r"(?:^|[^{word}])reason[{space}]*="))
        .map_err(SitesFailure::AllowPattern)?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let joined = JoinedSource::new(&lines);
        let sites = attributes
            .captures_iter(joined.text())
            .filter_map(|matched| {
                if reason.is_match(matched.get(1)?.as_str()) {
                    return None;
                }
                let line = joined.position(matched.get(0)?.start()).line + 1;
                Some(RustCoordinate {
                    line,
                    text: blob.display_line(line),
                })
            })
            .collect();
        (blob.paths.as_slice(), sites)
    });
    Ok(rust_sources::listed(tip, rule, candidates))
}

pub(super) fn test_allow_boilerplate(
    sources: &RustSources,
    tip: &RepositoryTip,
    rule: &Rule,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = BoilerplateScanner::new()?;
    let candidates = sources.blobs().map(|blob| {
        let lines = blob.masked.lines().collect::<Vec<_>>();
        let joined = JoinedSource::new(&lines);
        let sites = scanner
            .allows(&joined, &lines)
            .flat_map(BoilerplateAllow::stale)
            .collect();
        (blob.paths.as_slice(), sites)
    });
    Ok(rust_sources::listed(tip, rule, candidates))
}

fn allow_attributes(space: &str) -> Result<Regex, SitesFailure> {
    Regex::new(&format!(r"#\[allow\(([^)]*?)\)[{space}]*\]")).map_err(SitesFailure::AllowPattern)
}
