//! Read the committed files and crate scopes that a judgment covers.

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;

use regex::Regex;
use serde_json::Error as JsonError;
use toml::Table;
use toml::Value;

use crate::constants::IGNORE_FILES;
use crate::facts;
use crate::finding_source::EntryKind;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::SourceText;
use crate::finding_source::TreeBlobs;
use crate::finding_source::TreeEntries;
use crate::finding_source::TreeEntry;
use crate::site_ledger::InputScope;
use crate::site_ledger::ReadScope;

pub(crate) struct RepositoryTip {
    entries:    Rc<TreeEntries>,
    crates:     Vec<String>,
    rust_files: BTreeMap<String, String>,
}

pub(crate) enum TipReading {
    Read(RepositoryTip),
    Unreadable,
}

type Inputs<'a> = BTreeMap<&'a str, &'a TreeEntry>;

#[derive(Debug, thiserror::Error)]
#[error("missing required input {path:?}")]
pub(crate) struct MissingInput {
    path: String,
}

impl RepositoryTip {
    pub(crate) fn read(
        source: &mut FindingSource<'_>,
        commit: &str,
    ) -> Result<TipReading, FindingSourceFailure> {
        let TreeBlobs::Listed(entries) = source.tree(commit)? else {
            return Ok(TipReading::Unreadable);
        };
        let entries = Rc::clone(entries);
        let mut manifests = BTreeMap::new();
        let mut ignored = Vec::new();
        for (path, entry) in entries
            .iter()
            .filter(|(_, entry)| entry.kind == EntryKind::Blob)
        {
            let manifest = path.rsplit('/').next() == Some("Cargo.toml");
            if !manifest && !IGNORE_FILES.contains(&path.as_str()) {
                continue;
            }
            let content = match source.source(&entry.object)? {
                SourceText::Utf8(source) => source.lines.text(),
                SourceText::NotUtf8(text) => text,
            };
            if manifest {
                manifests.insert(parent(path), content.to_owned());
            } else {
                ignore_patterns(&read_toml(path, content)?, &mut ignored);
            }
        }
        let crates = workspace_members(&manifests)?;
        let ignored: Vec<_> = ignored
            .iter()
            .map(|pattern| IgnorePattern {
                path:    pattern.trim_end_matches('/'),
                matcher: FilePattern::new(pattern),
                at_root: pattern.strip_prefix("**/").map(FilePattern::new),
            })
            .collect();
        let rust_files = entries
            .iter()
            .filter(|(path, entry)| {
                entry.kind == EntryKind::Blob && is_rust_path(path) && !is_ignored(path, &ignored)
            })
            .map(|(path, entry)| (path.clone(), entry.object.clone()))
            .collect();
        Ok(TipReading::Read(Self {
            entries,
            crates,
            rust_files,
        }))
    }

    pub(crate) fn entry(&self, path: &str) -> Option<&TreeEntry> {
        self.entries
            .get(path)
            .filter(|entry| entry.kind != EntryKind::Directory)
    }

    pub(crate) fn blob(&self, path: &str) -> Option<&str> {
        self.rust_files.get(path).map(String::as_str).or_else(|| {
            self.entry(path)
                .filter(|entry| entry.kind == EntryKind::Blob)
                .map(|entry| entry.object.as_str())
        })
    }

    pub(crate) fn crate_of(&self, path: &str) -> &str {
        self.crates
            .iter()
            .find(|directory| under(path, directory))
            .map_or(".", String::as_str)
    }

    pub(crate) fn has_rust_path(&self, path: &str) -> bool { self.rust_files.contains_key(path) }

    pub(crate) fn crate_rust_paths<'a>(
        &'a self,
        crate_name: &'a str,
    ) -> impl Iterator<Item = &'a str> {
        self.entries
            .iter()
            .filter(move |(path, entry)| {
                entry.kind == EntryKind::Blob
                    && is_rust_path(path)
                    && self.crate_of(path) == crate_name
            })
            .map(|(path, _)| path.as_str())
    }

    pub(crate) fn inputs<'a>(
        &'a self,
        scope: InputScope,
        path: &str,
        reads: &ReadScope,
    ) -> Result<Inputs<'a>, MissingInput> {
        if let ReadScope::Explicit(paths) = reads {
            return std::iter::once(path)
                .chain(paths.iter().map(String::as_str))
                .map(|name| {
                    self.entries
                        .get_key_value(name)
                        .filter(|(_, entry)| entry.kind == EntryKind::Blob)
                        .map(|(name, entry)| (name.as_str(), entry))
                        .ok_or_else(|| MissingInput {
                            path: name.to_owned(),
                        })
                })
                .collect();
        }
        if matches!(scope, InputScope::Module | InputScope::File) && self.blob(path).is_none() {
            return Err(MissingInput {
                path: path.to_owned(),
            });
        }
        let crate_dir = self.crate_of(path);
        let directory = parent(path);
        let parent_module = format!("{directory}.rs");
        Ok(self
            .entries
            .iter()
            .filter(|(name, entry)| {
                entry.kind != EntryKind::Directory
                    && match scope {
                        InputScope::File => *name == path,
                        InputScope::Module => {
                            is_rust_path(name)
                                && (parent(name) == directory || *name == &parent_module)
                        },
                        InputScope::Manifest => {
                            *name == "Cargo.toml"
                                || (name.ends_with("/Cargo.toml")
                                    && self.crates.iter().any(|member| member == parent(name)))
                        },
                        InputScope::Crate if crate_dir == "." => self.crate_of(name) == ".",
                        InputScope::Crate => under(name, crate_dir),
                    }
            })
            .map(|(name, entry)| (name.as_str(), entry))
            .collect())
    }

    pub(crate) fn fingerprint(
        &self,
        scope: FingerprintSource<'_>,
        inputs: &Inputs<'_>,
    ) -> Result<String, JsonError> {
        match scope {
            FingerprintSource::FileBlob(path) => return Ok(inputs[path].object.clone()),
            FingerprintSource::CrateTree(directory) => {
                if directory != "."
                    && let Some(entry) = self.entries.get(directory)
                {
                    return Ok(entry.object.clone());
                }
            },
            FingerprintSource::Entries => {},
        }
        let pairs: Vec<_> = inputs
            .iter()
            .filter(|(_, entry)| entry.kind == EntryKind::Blob)
            .map(|(path, entry)| (*path, entry.object.as_str()))
            .collect();
        facts::python_json_digest(&pairs)
    }
}

#[derive(Clone, Copy)]
pub(crate) enum FingerprintSource<'a> {
    FileBlob(&'a str),
    CrateTree(&'a str),
    Entries,
}

pub(crate) fn is_rust_path(path: &str) -> bool { path.as_bytes().ends_with(b".rs") }

pub(crate) fn under(path: &str, directory: &str) -> bool {
    directory == "."
        || path == directory
        || path
            .strip_prefix(directory.trim_end_matches('/'))
            .is_some_and(|part| part.starts_with('/'))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/')
        .map_or(".", |(directory, _)| directory)
}

fn table<'a>(key: &str, parent: &'a Table) -> Option<&'a Table> {
    parent.get(key).and_then(Value::as_table)
}

fn strings(value: &Value) -> impl Iterator<Item = &str> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
}

fn ignore_patterns(table: &Table, patterns: &mut Vec<String>) {
    for (key, value) in table {
        if matches!(
            key.as_str(),
            "ignore" | "ignored" | "exclude" | "excludes" | "ignored_paths" | "ignore_paths"
        ) {
            patterns.extend(strings(value).map(str::to_owned));
        }
        if let Some(nested) = value.as_table() {
            ignore_patterns(nested, patterns);
        }
    }
}

struct IgnorePattern<'a> {
    path:    &'a str,
    matcher: FilePattern<'a>,
    at_root: Option<FilePattern<'a>>,
}

fn is_ignored(path: &str, patterns: &[IgnorePattern<'_>]) -> bool {
    parent(path).split('/').any(|part| part == "vendor")
        || patterns.iter().any(|pattern| {
            under(path, pattern.path)
                || pattern.matcher.matches(path)
                || pattern
                    .at_root
                    .as_ref()
                    .is_some_and(|pattern| pattern.matches(path))
        })
}

fn read_toml(path: &str, text: &str) -> Result<Table, FindingSourceFailure> {
    text.parse()
        .map_err(|error| FindingSourceFailure::InvalidTipToml {
            path:   path.to_owned(),
            source: Box::new(error),
        })
}

fn workspace_members(
    manifests: &BTreeMap<&str, String>,
) -> Result<Vec<String>, FindingSourceFailure> {
    let root = read_toml("Cargo.toml", manifests.get(".").map_or("", String::as_str))?;
    let workspace = table("workspace", &root);
    let patterns = workspace
        .and_then(|workspace| workspace.get("members"))
        .into_iter()
        .flat_map(strings)
        .collect::<Vec<_>>();
    let exclusions = workspace
        .and_then(|workspace| workspace.get("exclude"))
        .into_iter()
        .flat_map(strings)
        .collect::<Vec<_>>();
    let inherited = workspace.and_then(|workspace| table("dependencies", workspace));
    let mut pending: Vec<String> = manifests
        .keys()
        .filter(|&&directory| {
            directory == "."
                || patterns
                    .iter()
                    .any(|pattern| workspace_matches(directory, pattern))
        })
        .map(|&directory| directory.to_owned())
        .collect();
    let mut visited = HashSet::new();
    let mut members = Vec::new();
    while let Some(directory) = pending.pop() {
        if !visited.insert(directory.clone())
            || ancestors(&directory).any(|path| {
                exclusions
                    .iter()
                    .any(|pattern| workspace_matches(path, pattern))
            })
        {
            continue;
        }
        let manifest = if directory == "." {
            root.clone()
        } else {
            read_toml(
                &format!("{directory}/Cargo.toml"),
                &manifests[directory.as_str()],
            )?
        };
        if table("package", &manifest)
            .and_then(|package| package.get("name"))
            .and_then(Value::as_str)
            .is_none()
        {
            continue;
        }
        let targets = table("target", &manifest)
            .into_iter()
            .flat_map(Table::values)
            .filter_map(Value::as_table);
        for owner in std::iter::once(&manifest).chain(targets) {
            add_dependencies(owner, &directory, inherited, manifests, &mut pending);
        }
        members.push(directory);
    }
    members.sort_by_key(|member| (Reverse(path_parts(member).count()), member.clone()));
    Ok(members)
}

fn add_dependencies(
    owner: &Table,
    directory: &str,
    inherited: Option<&Table>,
    manifests: &BTreeMap<&str, String>,
    pending: &mut Vec<String>,
) {
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        for (name, dependency) in table(kind, owner).into_iter().flatten() {
            let Some(dependency) = dependency.as_table() else {
                continue;
            };
            let (dependency, base) =
                if dependency.get("workspace").and_then(Value::as_bool) == Some(true) {
                    (inherited.and_then(|parent| table(name, parent)), ".")
                } else {
                    (Some(dependency), directory)
                };
            if let Some(path) = dependency
                .and_then(|fields| fields.get("path"))
                .and_then(Value::as_str)
                && !path.starts_with('/')
                && let Some(path) = normalized_dependency(base, path)
                && manifests.contains_key(path.as_str())
            {
                pending.push(path);
            }
        }
    }
}

fn normalized_dependency(base: &str, path: &str) -> Option<String> {
    let mut parts = Vec::new();
    for part in path_parts(base).chain(path_parts(path)) {
        if part == ".." {
            parts.pop()?;
        } else {
            parts.push(part);
        }
    }
    Some(if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    })
}

fn ancestors(path: &str) -> impl Iterator<Item = &str> {
    Path::new(path)
        .ancestors()
        .map(|path| path.to_str().filter(|path| !path.is_empty()).unwrap_or("."))
}

/// The parts of a `/` path, without empty and `.` parts.
pub(crate) fn path_parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|part| !matches!(*part, "" | "."))
}

fn workspace_matches(directory: &str, pattern: &str) -> bool {
    fn matches(parts: &[&str], patterns: &[&str]) -> bool {
        match (parts, patterns) {
            (_, []) => parts.is_empty(),
            (_, ["**", rest @ ..]) => {
                matches(parts, rest)
                    || parts
                        .split_first()
                        .is_some_and(|(_, tail)| matches(tail, patterns))
            },
            ([part, tail @ ..], [pattern, rest @ ..]) => {
                FilePattern::new(pattern).matches(part) && matches(tail, rest)
            },
            _ => false,
        }
    }
    if pattern.starts_with('/') {
        return false;
    }
    matches(
        &path_parts(directory).collect::<Vec<_>>(),
        &path_parts(pattern).collect::<Vec<_>>(),
    )
}

enum FilePattern<'a> {
    Literal(&'a str),
    Glob(Regex),
    Invalid,
}

impl<'a> FilePattern<'a> {
    fn new(pattern: &'a str) -> Self {
        if !pattern.contains(['*', '?', '[']) {
            return Self::Literal(pattern);
        }
        Regex::new(&glob_regex(pattern)).map_or(Self::Invalid, Self::Glob)
    }

    fn matches(&self, name: &str) -> bool {
        match self {
            Self::Literal(literal) => name == *literal,
            Self::Glob(glob) => glob.is_match(name),
            Self::Invalid => false,
        }
    }
}

fn glob_regex(pattern: &str) -> String {
    let words: Vec<_> = pattern.chars().collect();
    let mut regex = String::from("(?s)\\A");
    let mut cursor = 0;
    while let Some(&word) = words.get(cursor) {
        cursor += 1;
        match word {
            '*' if cursor > 1 && words[cursor - 2] == '*' => {},
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            '[' if let Some(end) = class_end(&words, cursor) => {
                write_class(&words[cursor..end], &mut regex);
                cursor = end + 1;
            },
            _ => regex.push_str(&regex::escape(&word.to_string())),
        }
    }
    regex.push_str("\\z");
    regex
}

fn class_end(chars: &[char], mut cursor: usize) -> Option<usize> {
    if chars.get(cursor) == Some(&'!') {
        cursor += 1;
    }
    if chars.get(cursor) == Some(&']') {
        cursor += 1;
    }
    (cursor..chars.len()).find(|&index| chars[index] == ']')
}

fn write_class(chars: &[char], regex: &mut String) {
    let chunks = class_chunks(chars);
    let mut parts = chunks.iter();
    let first = parts.next().map_or(&[][..], Vec::as_slice);
    if first.is_empty() {
        regex.push_str(r"[^\s\S]");
        return;
    }
    if first == ['!'] && parts.as_slice().is_empty() {
        regex.push('.');
        return;
    }
    regex.push('[');
    if let Some(members) = first.strip_prefix(&['!']) {
        regex.push('^');
        write_class_members(members, regex);
    } else {
        write_class_members(first, regex);
    }
    for members in parts {
        regex.push('-');
        write_class_members(members, regex);
    }
    regex.push(']');
}

fn class_chunks(chars: &[char]) -> Vec<Vec<char>> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut next = 1 + usize::from(chars.first() == Some(&'!'));
    while let Some(range) = (next..chars.len()).find(|&index| chars[index] == '-') {
        chunks.push(chars[start..range].to_vec());
        start = range + 1;
        next = range + 3;
    }
    if start < chars.len() {
        chunks.push(chars[start..].to_vec());
    } else if let Some(last) = chunks.last_mut() {
        last.push('-');
    }
    // Python removes reversed ranges from right to left and joins the remaining members.
    for index in (1..chunks.len()).rev() {
        if chunks[index - 1].last() > chunks[index].first() {
            let right = chunks.remove(index);
            let left = &mut chunks[index - 1];
            left.pop();
            left.extend_from_slice(&right[1..]);
        }
    }
    chunks
}

fn write_class_members(members: &[char], regex: &mut String) {
    for &character in members {
        if matches!(character, '\\' | '[' | ']' | '^' | '&' | '~' | '|' | '-') {
            regex.push('\\');
        }
        regex.push(character);
    }
}
