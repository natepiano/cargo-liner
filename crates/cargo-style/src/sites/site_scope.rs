//! Filter candidate paths by the synced tree, ignores and rule reach.

use toml::Table;
use toml::Value;

use super::catalog::Reach;
use super::failure::SitesFailure;
use super::path_match::PathPattern;
use super::source_tree::SiteText;
use super::source_tree::SourceTree;
use crate::constants::IGNORE_FILES;
use crate::finding_source::FindingSource;
use crate::tip_view;

pub(super) struct SiteScope<'a> {
    tree:    &'a SourceTree,
    ignored: Vec<Ignore>,
}

impl<'a> SiteScope<'a> {
    pub(super) fn read(
        tree: &'a SourceTree,
        source: &mut FindingSource<'_>,
    ) -> Result<Self, SitesFailure> {
        Ok(Self {
            tree,
            ignored: ignored_paths(source, tree)?,
        })
    }

    pub(super) fn includes(&self, path: &str, reach: &Reach) -> bool {
        self.tree.blob(path).is_some() && !is_ignored(path, &self.ignored) && reach.includes(path)
    }

    pub(super) fn rust_paths(&self) -> impl Iterator<Item = &str> {
        self.tree
            .blobs()
            .map(|(path, _)| path.as_str())
            .filter(|path| tip_view::is_rust_path(path) && !is_ignored(path, &self.ignored))
    }
}

struct Ignore {
    prefix: String,
    glob:   PathPattern,
    root:   RootIgnore,
}

impl Ignore {
    fn new(pattern: &str) -> Result<Self, SitesFailure> {
        let compile = |raw| {
            PathPattern::new(raw).map_err(|source| SitesFailure::IgnorePattern {
                pattern: pattern.to_owned(),
                source,
            })
        };
        let root = if let Some(raw) = pattern.strip_prefix("**/") {
            RootIgnore::AlsoMatch(compile(raw)?)
        } else {
            RootIgnore::UseFullPattern
        };
        Ok(Self {
            prefix: pattern.trim_end_matches('/').to_owned(),
            glob: compile(pattern)?,
            root,
        })
    }
}

enum RootIgnore {
    UseFullPattern,
    AlsoMatch(PathPattern),
}

fn ignored_paths(
    source: &mut FindingSource<'_>,
    tree: &SourceTree,
) -> Result<Vec<Ignore>, SitesFailure> {
    let mut patterns = Vec::new();
    for path in IGNORE_FILES {
        if tree.blob(path).is_none() {
            continue;
        }
        let input = tree.read_text(source, path)?;
        let text = match &input {
            SiteText::Utf8(text) => text.text(),
            SiteText::NotUtf8(text) => text,
        };
        let table: Table = text.parse().map_err(|source| SitesFailure::IgnoreConfig {
            path:   path.to_owned(),
            source: Box::new(source),
        })?;
        add_ignores(&table, &mut patterns)?;
    }
    Ok(patterns)
}

fn add_ignores(table: &Table, patterns: &mut Vec<Ignore>) -> Result<(), SitesFailure> {
    for (key, value) in table {
        if matches!(
            key.as_str(),
            "ignore" | "ignored" | "exclude" | "excludes" | "ignored_paths" | "ignore_paths"
        ) {
            for pattern in value
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                patterns.push(Ignore::new(pattern)?);
            }
        }
        if let Some(table) = value.as_table() {
            add_ignores(table, patterns)?;
        }
    }
    Ok(())
}

fn is_ignored(path: &str, patterns: &[Ignore]) -> bool {
    path.rsplit_once('/')
        .is_some_and(|(directory, _)| directory.split('/').any(|part| part == "vendor"))
        || patterns.iter().any(|pattern| {
            tip_view::under(path, &pattern.prefix)
                || pattern.glob.matches(path)
                || matches!(&pattern.root, RootIgnore::AlsoMatch(root) if root.matches(path))
        })
}
