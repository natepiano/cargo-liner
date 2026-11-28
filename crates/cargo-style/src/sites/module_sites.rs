//! List top-level items and module names at the synced tip.

use std::fmt::Write as _;
use std::path::Path;

use regex::Regex;

use super::constants::ANTI_PATTERN_MODULE_STEMS;
use super::constants::MODULE_ROOT_ITEM_KINDS;
use super::constants::MODULE_ROOT_PREVIEW_LIMIT;
use super::constants::PYTHON_INPUT_ROOT;
use super::failure::SitesFailure;
use super::item_scan;
use super::ledger::ListedCoordinate;
use super::rust_sources;
use super::rust_sources::RustSources;
use super::source_tree::SourceTree;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

struct ModuleRootScanner {
    item: Regex,
}

impl ModuleRootScanner {
    fn new() -> Result<Self, SitesFailure> {
        let space = item_scan::class_ranges(SPACE);
        let visibility = item_scan::visibility(&space);
        let item = Regex::new(&format!(
            r"^[{space}]*{visibility}(?:unsafe[{space}]+)?({MODULE_ROOT_ITEM_KINDS})"
        ))
        .map_err(SitesFailure::ModuleRootPattern)?;
        Ok(Self { item })
    }

    fn items<'a>(&self, lines: impl Iterator<Item = &'a str>) -> Vec<RootItem<'a>> {
        let mut depth = 0_isize;
        let mut items = Vec::new();
        for (index, text) in lines.enumerate() {
            if depth == 0
                && let Some(item) = self.item(text, index + 1)
            {
                items.push(item);
            }
            for byte in text.bytes() {
                depth += isize::from(byte == b'{') - isize::from(byte == b'}');
            }
        }
        items
    }

    fn item<'a>(&self, text: &'a str, line: usize) -> Option<RootItem<'a>> {
        let matched = self.item.captures(text)?.get(1)?;
        let kind = matched.as_str();
        let tail = &text[matched.end()..];
        let follows_word = tail.starts_with(is_word);
        if follows_word != (kind == "macro_rules!") {
            return None;
        }
        let tail = tail.trim_start_matches(tokenizer::python_space);
        let name = &tail[..tail
            .find(|character| !is_word(character))
            .unwrap_or(tail.len())];
        Some(RootItem { line, kind, name })
    }
}

struct RootItem<'a> {
    line: usize,
    kind: &'a str,
    name: &'a str,
}

impl RootItem<'_> {
    fn label(&self) -> String {
        if self.name.is_empty() {
            self.kind.to_owned()
        } else {
            format!("{} {}", self.kind, self.name)
        }
    }
}

pub(super) fn is_module_root(path: &str) -> bool {
    matches!(
        path.rsplit('/').next(),
        Some("mod.rs" | "lib.rs" | "main.rs")
    )
}

pub(super) fn module_root_items(
    sources: &RustSources,
) -> Result<Vec<ListedCoordinate>, SitesFailure> {
    let scanner = ModuleRootScanner::new()?;
    let mut listed = Vec::new();
    for blob in sources.blobs() {
        let items = scanner.items(blob.masked.lines());
        listed.extend(
            blob.paths
                .iter()
                .filter_map(|path| root_coordinate(path, &items)),
        );
    }
    Ok(listed)
}

pub(super) fn submodule_names(tree: &SourceTree) -> Vec<ListedCoordinate> {
    let root_name = Path::new(PYTHON_INPUT_ROOT)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    tree.blobs()
        .filter(|(path, _)| rust_sources::rust_path(path))
        .filter_map(|(path, _)| {
            let file = Path::new(path);
            let stem = file.file_stem()?.to_str()?;
            let parent = file
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .unwrap_or(root_name);
            ModuleNameFinding::classify(stem, parent).coordinate(path)
        })
        .collect()
}

fn root_coordinate(path: &str, items: &[RootItem<'_>]) -> Option<ListedCoordinate> {
    let main = path.rsplit('/').next() == Some("main.rs");
    let mut items = items
        .iter()
        .filter(|item| !(main && item.kind == "fn" && item.name == "main"));
    let first = items.next()?;
    let labels = std::iter::once(first)
        .chain(items.by_ref().take(MODULE_ROOT_PREVIEW_LIMIT - 1))
        .map(RootItem::label)
        .collect::<Vec<_>>();
    let remaining = items.count();
    let count = labels.len() + remaining;
    let mut preview = labels.join(", ");
    if remaining > 0 {
        let _ = write!(preview, ", … (+{remaining})");
    }
    Some(ListedCoordinate {
        path: path.to_owned(),
        line: first.line,
        text: format!("{count} top-level item(s) beyond mod/use: {preview}"),
    })
}

enum ModuleNameFinding<'a> {
    AntiPatternStem(&'a str),
    AntiPatternDirectory(&'a str),
    ParentStutter { stem: &'a str, parent: &'a str },
    Accepted,
}

impl<'a> ModuleNameFinding<'a> {
    fn classify(stem: &'a str, parent: &'a str) -> Self {
        if ANTI_PATTERN_MODULE_STEMS.contains(&stem) {
            Self::AntiPatternStem(stem)
        } else if stem == "mod" && ANTI_PATTERN_MODULE_STEMS.contains(&parent) {
            Self::AntiPatternDirectory(parent)
        } else if !matches!(stem, "mod" | "lib" | "main")
            && !matches!(parent, "src" | "")
            && stem
                .strip_prefix(parent)
                .is_some_and(|suffix| suffix.starts_with('_'))
        {
            Self::ParentStutter { stem, parent }
        } else {
            Self::Accepted
        }
    }

    fn coordinate(self, path: &str) -> Option<ListedCoordinate> {
        let text = match self {
            Self::AntiPatternStem(stem) => format!("anti-pattern module name `{stem}.rs`"),
            Self::AntiPatternDirectory(parent) => {
                format!("anti-pattern module directory `{parent}/`")
            },
            Self::ParentStutter { stem, parent } => {
                format!("`{parent}/{stem}.rs` stutters the parent module name")
            },
            Self::Accepted => return None,
        };
        Some(ListedCoordinate {
            path: path.to_owned(),
            line: 1,
            text,
        })
    }
}

fn is_word(character: char) -> bool { tokenizer::in_table(WORD, character) }
