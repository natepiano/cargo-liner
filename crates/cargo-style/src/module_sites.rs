//! Module rename sites, ported from the Python module rename scan.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

use crate::module_scan;
use crate::module_scan::PlainPath;
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::unicode_tables::LINE_BOUNDARIES;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;
use crate::word_syntax;
use crate::word_syntax::InlineModule;

/// The file module within the renamed file's crate, or outside that crate.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum FileModule {
    Crate {
        #[serde(rename = "crate")]
        path: Vec<String>,
    },
    Outside(OutsideCrate),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OutsideCrate {
    OutsideCrate,
}

/// The library name visible from files outside the renamed file's crate.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum CrateLibrary {
    Named { name: String },
    None(NoLibrary),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NoLibrary {
    None,
}

/// One rename with the crate facts supplied by the Python caller.
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ModuleMove {
    old:         String,
    new:         String,
    file_module: FileModule,
    parent:      Vec<String>,
    library:     CrateLibrary,
}

/// A module site query for one blob at one file path.
#[derive(Debug, Deserialize)]
pub(crate) struct ModuleSitesQuery {
    pub(crate) blob:  String,
    pub(crate) path:  String,
    pub(crate) moves: Vec<ModuleMove>,
}

/// Whether a move has sites, or why its paths cannot be read.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum ModuleSiteReading {
    Sites { module_sites: ModuleSites },
    Refused(ModuleSiteRefusal),
}

/// The presence of sites that need the module rename.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ModuleSites {
    Present,
    Absent,
}

impl From<bool> for ModuleSites {
    fn from(present: bool) -> Self { if present { Self::Present } else { Self::Absent } }
}

impl Serialize for ModuleSites {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(matches!(self, Self::Present))
    }
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ModuleSiteRefusal {
    NoRenamedText,
    UncertainModulePath,
}

/// A resolved path or a path that the Python reader cannot resolve.
#[derive(Clone, Debug, PartialEq, Eq)]
enum ModulePath {
    Resolved(Vec<String>),
    Unresolved,
}

/// The source prefix and use keyword for every use tree segment.
struct UseTrees {
    prefixes: BTreeMap<usize, Vec<String>>,
    keywords: BTreeMap<usize, usize>,
}

/// The recognized and uncertain path attributes of module items.
struct PathAttributes {
    keywords: BTreeSet<usize>,
    resolved: BTreeSet<usize>,
    literals: BTreeMap<usize, usize>,
}

struct ModuleBinding {
    scope:   Vec<InlineModule>,
    renamed: BindingTarget,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingTarget {
    Renamed,
    Other,
}

struct GlobImport {
    scope:  Vec<InlineModule>,
    source: ModulePath,
}

/// Bindings that decide whether an unqualified path names the moved module.
struct ScopeBindings {
    bindings:       Vec<ModuleBinding>,
    globs:          Vec<GlobImport>,
    block_bindings: BindingTarget,
}

/// The rename scan for one file and one move.
struct RenameScan<'a> {
    words:  &'a [String],
    moved:  &'a ModuleMove,
    source: &'a str,
}

/// Read the module sites of source characters for every move in order.
#[cfg(test)]
fn read(text: &[char], path: &str, moves: &[ModuleMove]) -> Vec<ModuleSiteReading> {
    read_tokens(text, &tokenizer::tokenize(text), path, moves)
}

/// Read the sites from the request's one token read of the blob.
pub(crate) fn read_tokens(
    text: &[char],
    tokens: &[Token],
    path: &str,
    moves: &[ModuleMove],
) -> Vec<ModuleSiteReading> {
    if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
        return moves
            .iter()
            .map(|_| ModuleSiteReading::Refused(ModuleSiteRefusal::NoRenamedText))
            .collect();
    }
    let words = word_syntax::plain_words(tokens);
    moves
        .iter()
        .map(|moved| {
            let scan = RenameScan {
                words: &words,
                moved,
                source: path,
            };
            match scan.renames() {
                Ok(sites) => {
                    let code_changed = sites.iter().any(|(&index, value)| words[index] != *value);
                    let comments = comment_gaps(text, tokens);
                    let comment_changed = comments
                        .iter()
                        .flatten()
                        .any(|line| renamed_comment(line, &scan) != *line);
                    ModuleSiteReading::Sites {
                        module_sites: ModuleSites::from(code_changed || comment_changed),
                    }
                },
                Err(reason) => ModuleSiteReading::Refused(reason),
            }
        })
        .collect()
}

impl RenameScan<'_> {
    fn scope_module(&self, scope: &[InlineModule]) -> FileModule {
        match &self.moved.file_module {
            FileModule::Crate { path: module } => FileModule::Crate {
                path: module
                    .iter()
                    .cloned()
                    .chain(scope.iter().map(|module| module.name.clone()))
                    .collect(),
            },
            outside @ FileModule::Outside(_) => outside.clone(),
        }
    }

    fn names_parent(&self, prefix: &[String], module: &FileModule) -> bool {
        if prefix.first().is_some_and(|value| path_anchor(value)) {
            return resolved_module(prefix, module)
                == ModulePath::Resolved(self.moved.parent.clone());
        }
        if prefix.is_empty() || prefix.iter().any(|value| path_anchor(value)) {
            return false;
        }
        match module {
            FileModule::Outside(_) => match &self.moved.library {
                CrateLibrary::Named { name } => {
                    prefix
                        == std::iter::once(name)
                            .chain(&self.moved.parent)
                            .cloned()
                            .collect::<Vec<_>>()
                },
                CrateLibrary::None(_) => false,
            },
            FileModule::Crate { path: module } => {
                module.iter().chain(prefix).eq(self.moved.parent.iter())
            },
        }
    }

    fn qualifier(&self, mut index: usize) -> Vec<String> {
        let mut segments = Vec::new();
        while index >= 2
            && word_syntax::word(self.words, index - 1) == "::"
            && word_syntax::identifier(word_syntax::word(self.words, index - 2))
        {
            segments.insert(0, self.words[index - 2].clone());
            index -= 2;
        }
        if index >= 1 && word_syntax::word(self.words, index - 1) == "::" {
            Vec::new()
        } else {
            segments
        }
    }

    fn mod_line(
        &self,
        index: usize,
        starts: &BTreeSet<usize>,
        modules: &[InlineModule],
        paths: &PathAttributes,
    ) -> bool {
        if index == 0
            || word_syntax::word(self.words, index - 1) != "mod"
            || !starts.contains(&(index - 1))
            || word_syntax::word(self.words, index + 1) != ";"
        {
            return false;
        }
        if paths.keywords.contains(&(index - 1)) {
            return paths.literals.contains_key(&(index - 1));
        }
        match self.scope_module(&word_syntax::enclosing(modules, index)) {
            FileModule::Crate { path: module } => module == self.moved.parent,
            FileModule::Outside(_) => false,
        }
    }

    fn path_attributes(
        &self,
        starts: &BTreeSet<usize>,
        modules: &[InlineModule],
    ) -> PathAttributes {
        let words = self.words;
        let spans: BTreeMap<_, _> = word_syntax::attribute_spans(words).into_iter().collect();
        let attributes = word_syntax::decorated_items(words);
        let keywords: BTreeSet<_> = attributes
            .iter()
            .filter(|(owner, decorated)| {
                word_syntax::word(words, **owner) == "mod"
                    && decorated.iter().any(|value| value == "path")
            })
            .map(|(&owner, _)| owner)
            .collect();
        let mut resolved = BTreeSet::new();
        let mut literals = BTreeMap::new();
        for (start, owner) in word_syntax::attribute_owners(words) {
            let PlainPath::Literal(named) = module_scan::plain_path(words, start, spans[&start])
            else {
                continue;
            };
            let readable = keywords.contains(&owner)
                && starts.contains(&owner)
                && attributes[&owner]
                    .iter()
                    .filter(|value| value.as_str() == "path")
                    .count()
                    == 1
                && word_syntax::enclosing(modules, owner).is_empty();
            if readable && word_syntax::word(words, owner + 2) == ";" {
                resolved.insert(owner);
            }
            if readable
                && named.rsplit('/').next().unwrap_or("") == file_name(&self.moved.old)
                && joined_path(self.source, &named) == self.moved.old
            {
                literals.insert(owner, start + 4);
            }
        }
        PathAttributes {
            keywords,
            resolved,
            literals,
        }
    }

    fn bound_in(
        &self,
        scope: &[InlineModule],
        bindings: &[ModuleBinding],
        globs: &[GlobImport],
    ) -> bool {
        let bound: Vec<_> = bindings
            .iter()
            .filter(|binding| binding.scope == scope)
            .map(|binding| binding.renamed)
            .collect();
        if !bound.is_empty() {
            return bound.iter().all(|&target| target == BindingTarget::Renamed);
        }
        globs.iter().filter(|glob| glob.scope == scope).any(|glob| {
            glob.source == ModulePath::Resolved(self.moved.parent.clone())
                || (0..scope.len()).any(|depth| match self.scope_module(&scope[..depth]) {
                    FileModule::Crate { path: module } => {
                        glob.source == ModulePath::Resolved(module)
                            && self.bound_in(&scope[..depth], bindings, globs)
                    },
                    FileModule::Outside(_) => false,
                })
        })
    }

    fn bindings(
        &self,
        starts: &BTreeSet<usize>,
        modules: &[InlineModule],
        paths: &PathAttributes,
        trees: &UseTrees,
    ) -> ScopeBindings {
        let words = self.words;
        let stem = file_stem(&self.moved.old);
        let mut depths = vec![0_isize];
        for value in words {
            let change = match value.as_str() {
                "{" => 1,
                "}" => -1,
                _ => 0,
            };
            depths.push(depths.last().copied().unwrap_or(0) + change);
        }
        let mut bindings = Vec::new();
        let mut globs = Vec::new();
        let mut block_bindings = BindingTarget::Renamed;
        for (index, value) in words.iter().enumerate().take(words.len().saturating_sub(1)) {
            if index == 0 || (value != "*" && value != stem) {
                continue;
            }
            let anchor = trees.keywords.get(&index).copied().unwrap_or(index - 1);
            let scope = word_syntax::enclosing(modules, anchor);
            if value == "*" {
                if let Some(prefix) = trees.prefixes.get(&index) {
                    globs.push(GlobImport {
                        source: resolved_module(prefix, &self.scope_module(&scope)),
                        scope,
                    });
                }
                continue;
            }
            let mut renamed = self.mod_line(index, starts, modules, paths);
            if let Some(prefix) = trees
                .prefixes
                .get(&index)
                .filter(|_| matches!(word_syntax::word(words, index + 1), ";" | "," | "}"))
            {
                renamed = self.names_parent(prefix, &self.scope_module(&scope));
            } else if !(renamed
                || word_syntax::word(words, index - 1) == "as"
                || (type_item(word_syntax::word(words, index - 1))
                    && starts.contains(&(index - 1)))
                || (index >= 2
                    && word_syntax::word(words, index - 2) == "extern"
                    && word_syntax::word(words, index - 1) == "crate"))
            {
                continue;
            }
            if usize::try_from(depths[anchor]).ok() == Some(scope.len()) {
                bindings.push(ModuleBinding {
                    scope,
                    renamed: if renamed {
                        BindingTarget::Renamed
                    } else {
                        BindingTarget::Other
                    },
                });
            } else {
                block_bindings = BindingTarget::Other;
            }
        }
        ScopeBindings {
            bindings,
            globs,
            block_bindings,
        }
    }

    fn renames(&self) -> Result<BTreeMap<usize, String>, ModuleSiteRefusal> {
        let words = self.words;
        let stem = file_stem(&self.moved.old);
        let starts = word_syntax::code_item_starts(words);
        let modules = word_syntax::inline_modules(words, &starts);
        let paths = self.path_attributes(&starts, &modules);
        if !paths.keywords.is_subset(&paths.resolved) {
            return Err(ModuleSiteRefusal::UncertainModulePath);
        }
        let trees = use_trees(words, &starts);
        let bindings = self.bindings(&starts, &modules, &paths, &trees);
        let mut found = BTreeMap::new();
        for (index, value) in words.iter().enumerate().take(words.len().saturating_sub(1)) {
            if index == 0
                || value != stem
                || word_syntax::word(words, index - 1) == "."
                || (word_syntax::word(words, index + 1) == "::"
                    && word_syntax::word(words, index + 2) == "<")
            {
                continue;
            }
            let scope = word_syntax::enclosing(&modules, index);
            let is_site = match trees.prefixes.get(&index) {
                _ if self.mod_line(index, &starts, &modules, &paths) => true,
                Some(prefix)
                    if !prefix.is_empty() || word_syntax::word(words, index - 1) == "::" =>
                {
                    matches!(
                        word_syntax::word(words, index + 1),
                        "::" | ";" | "," | "}" | "as"
                    ) && self.names_parent(
                        prefix,
                        &self.scope_module(&word_syntax::enclosing(
                            &modules,
                            trees.keywords[&index],
                        )),
                    )
                },
                _ if word_syntax::word(words, index + 1) == "::" => {
                    let prefix = self.qualifier(index);
                    (!prefix.is_empty() && self.names_parent(&prefix, &self.scope_module(&scope)))
                        || (prefix.is_empty()
                            && word_syntax::word(words, index - 1) != "::"
                            && bindings.block_bindings == BindingTarget::Renamed
                            && self.bound_in(&scope, &bindings.bindings, &bindings.globs))
                },
                _ => false,
            };
            if is_site {
                found.insert(index, file_stem(&self.moved.new).to_owned());
            }
        }
        let old_end = format!("{}\"", file_name(&self.moved.old));
        for index in paths.literals.values() {
            let prefix = words[*index]
                .strip_suffix(&old_end)
                .unwrap_or(&words[*index]);
            found.insert(*index, format!("{prefix}{}\"", file_name(&self.moved.new)));
        }
        Ok(found)
    }
}

fn path_anchor(value: &str) -> bool { matches!(value, "crate" | "self" | "super") }

fn type_item(value: &str) -> bool {
    matches!(
        value,
        "mod" | "struct" | "enum" | "union" | "trait" | "type"
    )
}

fn file_name(path: &str) -> &str { path.trim_end_matches('/').rsplit('/').next().unwrap_or("") }

fn file_stem(path: &str) -> &str {
    let name = file_name(path);
    name.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map_or(name, |(stem, _)| stem)
}

/// Python POSIX join and normalization, independent of the host platform.
fn joined_path(source: &str, named: &str) -> String {
    let parent = source.rsplit_once('/').map_or("", |(parent, _)| parent);
    let joined = if named.starts_with('/') || parent.is_empty() {
        named.to_owned()
    } else {
        format!("{parent}/{named}")
    };
    let mut segments = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {},
            ".." if segments.last().is_some_and(|&last| last != "..") => {
                segments.pop();
            },
            ".." if joined.starts_with('/') => {},
            _ => segments.push(segment),
        }
    }
    let prefix = match joined.as_str() {
        value if value.starts_with("//") && !value.starts_with("///") => "//",
        value if value.starts_with('/') => "/",
        _ => "",
    };
    let normalized = format!("{prefix}{}", segments.join("/"));
    if normalized.is_empty() {
        ".".to_owned()
    } else {
        normalized
    }
}

/// The source prefix and keyword for each segment and alias of a use tree.
fn use_trees(words: &[String], starts: &BTreeSet<usize>) -> UseTrees {
    let mut prefixes = BTreeMap::new();
    let mut keywords = BTreeMap::new();
    for (keyword, value) in words.iter().enumerate() {
        if value != "use" || !starts.contains(&keyword) {
            continue;
        }
        let mut groups = vec![Vec::<String>::new()];
        let mut path = Vec::new();
        let mut cursor = keyword + 1;
        while cursor < words.len() && word_syntax::word(words, cursor) != ";" {
            match word_syntax::word(words, cursor) {
                "{" => {
                    groups.push(
                        groups
                            .last()
                            .into_iter()
                            .flatten()
                            .chain(&path)
                            .cloned()
                            .collect(),
                    );
                    path.clear();
                },
                "}" | "," => {
                    if word_syntax::word(words, cursor) == "}" && groups.len() > 1 {
                        groups.pop();
                    }
                    path.clear();
                },
                "as" => {
                    cursor += 1;
                    keywords.insert(cursor, keyword);
                },
                "::" => {},
                _ => {
                    prefixes.insert(
                        cursor,
                        groups
                            .last()
                            .into_iter()
                            .flatten()
                            .chain(&path)
                            .cloned()
                            .collect(),
                    );
                    keywords.insert(cursor, keyword);
                    path.push(words[cursor].clone());
                },
            }
            cursor += 1;
        }
    }
    UseTrees { prefixes, keywords }
}

/// Resolve the anchored paths accepted by the Python module reader.
fn resolved_module(prefix: &[String], module: &FileModule) -> ModulePath {
    let FileModule::Crate { path: module } = module else {
        return ModulePath::Unresolved;
    };
    let Some(first) = prefix.first().filter(|value| path_anchor(value)) else {
        return ModulePath::Unresolved;
    };
    let mut base = if first == "crate" {
        Vec::new()
    } else {
        module.clone()
    };
    let mut rest = &prefix[1..];
    if first == "super" {
        rest = prefix;
        while rest.first().is_some_and(|value| value == "super") {
            if base.pop().is_none() {
                return ModulePath::Unresolved;
            }
            rest = &rest[1..];
        }
    }
    if rest.iter().any(|value| path_anchor(value)) {
        return ModulePath::Unresolved;
    }
    base.extend_from_slice(rest);
    ModulePath::Resolved(base)
}

/// Nonempty Python-stripped comment lines between each token pair.
fn comment_gaps(text: &[char], tokens: &[Token]) -> Vec<Vec<String>> {
    let ends = std::iter::once(0).chain(
        tokens
            .iter()
            .map(|token| token.start + token.value.chars().count()),
    );
    let starts = tokens
        .iter()
        .map(|token| token.start)
        .chain(std::iter::once(text.len()));
    ends.zip(starts)
        .map(|(end, start)| {
            let mut lines = Vec::new();
            let mut cursor = end;
            let mut line_start = end;
            while cursor < start {
                if tokenizer::in_table(LINE_BOUNDARIES, text[cursor]) {
                    push_comment(&text[line_start..cursor], &mut lines);
                    cursor += 1;
                    if text[cursor - 1] == '\r' && text.get(cursor) == Some(&'\n') && cursor < start
                    {
                        cursor += 1;
                    }
                    line_start = cursor;
                } else {
                    cursor += 1;
                }
            }
            push_comment(&text[line_start..start], &mut lines);
            lines
        })
        .collect()
}

fn push_comment(line: &[char], lines: &mut Vec<String>) {
    let first = line
        .iter()
        .position(|&value| !tokenizer::in_table(SPACE, value));
    let last = line
        .iter()
        .rposition(|&value| !tokenizer::in_table(SPACE, value));
    if let (Some(first), Some(last)) = (first, last) {
        lines.push(line[first..=last].iter().collect());
    }
}

/// Rename only the crate paths matched by the Python comment regular expression.
fn renamed_comment(line: &str, scan: &RenameScan<'_>) -> String {
    let text: Vec<char> = line.chars().collect();
    let mut output = String::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let is_start = text.get(cursor..cursor + 5) == Some(&['c', 'r', 'a', 't', 'e'])
            && (cursor == 0
                || !(tokenizer::in_table(WORD, text[cursor - 1])
                    || matches!(text[cursor - 1], ':' | '$')));
        if !is_start {
            output.push(text[cursor]);
            cursor += 1;
            continue;
        }
        let mut segments = vec!["crate".to_owned()];
        let mut end = cursor + 5;
        while text.get(end..end + 2) == Some(&[':', ':'])
            && text
                .get(end + 2)
                .is_some_and(|value| value.is_ascii_alphabetic() || *value == '_')
        {
            let start = end + 2;
            end = start + 1;
            while text
                .get(end)
                .is_some_and(|value| value.is_ascii_alphanumeric() || *value == '_')
            {
                end += 1;
            }
            segments.push(text[start..end].iter().collect());
        }
        if segments.len() == 1 {
            output.push(text[cursor]);
            cursor += 1;
            continue;
        }
        let renamed: Vec<_> = segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                if segment == file_stem(&scan.moved.old)
                    && scan.names_parent(&segments[..index], &scan.moved.file_module)
                {
                    file_stem(&scan.moved.new).to_owned()
                } else {
                    segment.clone()
                }
            })
            .collect();
        output.push_str(&renamed.join("::"));
        cursor = end;
    }
    output
}

#[cfg(test)]
mod tests {
    use super::CrateLibrary;
    use super::FileModule;
    use super::ModuleMove;
    use super::ModuleSiteReading;
    use super::ModuleSiteRefusal;
    use super::NoLibrary;
    use super::OutsideCrate;
    use super::read;

    fn moved() -> ModuleMove {
        ModuleMove {
            old:         "src/old.rs".to_owned(),
            new:         "src/new.rs".to_owned(),
            file_module: FileModule::Crate { path: Vec::new() },
            parent:      Vec::new(),
            library:     CrateLibrary::None(NoLibrary::None),
        }
    }

    #[test]
    fn module_sites_follow_bindings_paths_attributes_and_comments() {
        for (source, expected) in [
            ("mod old; fn f() { old::f(); }", true),
            ("use crate::{old as alias, old::*};", true),
            ("mod nested { use super::old; old::f(); }", true),
            ("mod nested { struct old; old::f(); }", false),
            ("// crate::old::Thing\nfn f() {}", true),
            ("// $crate::old wordcrate::old :crate::old\n", false),
            ("#[path = \"./old.rs\"] mod other;", true),
            ("make! { mod old; old::f(); }", false),
            ("use crate::*; fn f() { old::f(); }", true),
            ("mod r#old; r#old::f();", true),
            ("const TEXT: &str = \"crate::old\";", false),
            ("fn f() { let x = value.old::f(); }", false),
        ] {
            let reading = read(
                &source.chars().collect::<Vec<_>>(),
                "src/lib.rs",
                &[moved()],
            );
            assert_eq!(
                reading,
                [ModuleSiteReading::Sites {
                    module_sites: expected.into(),
                }],
                "{source}"
            );
        }
    }

    #[test]
    fn unreadable_tokens_and_uncertain_paths_keep_distinct_readings() {
        for (source, expected) in [
            ("mod old; `", ModuleSiteRefusal::NoRenamedText),
            ("mod old; /* unfinished", ModuleSiteRefusal::NoRenamedText),
            (
                "#[path = r\"old.rs\"] mod old;",
                ModuleSiteRefusal::UncertainModulePath,
            ),
            (
                "mod inner { #[path = \"old.rs\"] mod old; }",
                ModuleSiteRefusal::UncertainModulePath,
            ),
            (
                "#[cfg_attr(test, path = \"old.rs\")] mod old;",
                ModuleSiteRefusal::UncertainModulePath,
            ),
        ] {
            assert_eq!(
                read(
                    &source.chars().collect::<Vec<_>>(),
                    "src/lib.rs",
                    &[moved()]
                ),
                [ModuleSiteReading::Refused(expected)],
                "{source}"
            );
        }
    }

    #[test]
    fn outside_crate_paths_require_a_library_name() {
        for (library, expected) in [
            (CrateLibrary::None(NoLibrary::None), false),
            (
                CrateLibrary::Named {
                    name: "example".to_owned(),
                },
                true,
            ),
        ] {
            let mut moved = moved();
            moved.file_module = FileModule::Outside(OutsideCrate::OutsideCrate);
            moved.library = library;
            assert_eq!(
                read(
                    &"example::old::f();".chars().collect::<Vec<_>>(),
                    "src/lib.rs",
                    &[moved]
                ),
                [ModuleSiteReading::Sites {
                    module_sites: expected.into(),
                }]
            );
        }
    }
}
