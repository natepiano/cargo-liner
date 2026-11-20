//! Compare trait membership through the imports and modules of two declarations.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;

use crate::constants::IMPORT_EXPANSION_LIMIT;
use crate::declaration_source::DeclarationSource;
use crate::declaration_source::FileModule;
use crate::declaration_source::SourceLocation;
use crate::declarations::ImplMembership;
use crate::declarations::ImplementationState;
use crate::declarations::ImportPosition;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::word_syntax;

#[derive(Default)]
pub(crate) struct TraitReader {
    imports: HashMap<(String, String), FileImports>,
}

#[derive(Clone, Copy)]
pub(crate) struct TraitSite<'a> {
    pub(crate) location:   SourceLocation<'a>,
    pub(crate) membership: &'a ImplMembership,
}

pub(crate) enum TraitIdentity {
    Same,
    Different,
    Uncertain,
}

pub(crate) enum TraitPath {
    Resolved(Vec<String>),
    Unresolved,
}

enum TraitHome {
    Resolved {
        module:       Vec<String>,
        declarations: usize,
    },
    Unresolved,
}

enum ImportNames {
    Bindings(Vec<(String, Vec<String>)>),
    Blocked(HashSet<String>),
}

enum ImportForm {
    Plain,
    Decorated,
}

enum ImportGlob {
    Wildcard,
    Explicit,
}

struct ScopedImport {
    scope: Vec<usize>,
    names: ImportNames,
    glob:  ImportGlob,
}

struct FileImports {
    uses:   Vec<ScopedImport>,
    blocks: Vec<(usize, usize)>,
}

impl TraitReader {
    pub(crate) fn compare(
        &mut self,
        source: &mut FindingSource<'_>,
        declarations: &mut DeclarationSource,
        left: TraitSite<'_>,
        right: TraitSite<'_>,
    ) -> Result<TraitIdentity, FindingSourceFailure> {
        let (left_header, right_header) = match (left.membership, right.membership) {
            (ImplMembership::State(ImplementationState::Unreadable), _)
            | (_, ImplMembership::State(ImplementationState::Unreadable)) => {
                return Ok(TraitIdentity::Uncertain);
            },
            (
                ImplMembership::State(ImplementationState::None),
                ImplMembership::State(ImplementationState::None),
            ) => return Ok(TraitIdentity::Same),
            (ImplMembership::State(ImplementationState::None), _)
            | (_, ImplMembership::State(ImplementationState::None)) => {
                return Ok(TraitIdentity::Different);
            },
            (ImplMembership::Path(left), ImplMembership::Path(right)) => (left, right),
        };
        let left_path = self.import_path(
            source,
            declarations,
            left.location,
            &left_header.path,
            &left_header.position,
        )?;
        let right_path = self.import_path(
            source,
            declarations,
            right.location,
            &right_header.path,
            &right_header.position,
        )?;
        let (TraitPath::Resolved(left_path), TraitPath::Resolved(right_path)) =
            (left_path, right_path)
        else {
            return Ok(TraitIdentity::Uncertain);
        };
        if left_header.arguments != right_header.arguments || left_path.last() != right_path.last()
        {
            return Ok(TraitIdentity::Different);
        }
        if left_path == right_path {
            return Ok(TraitIdentity::Same);
        }
        let left_home = trait_home(source, declarations, left.location, &left_path)?;
        let right_home = trait_home(source, declarations, right.location, &right_path)?;
        Ok(match (left_home, right_home) {
            (
                TraitHome::Resolved { module: left, .. },
                TraitHome::Resolved { module: right, .. },
            ) if left != right => TraitIdentity::Different,
            (
                TraitHome::Resolved {
                    declarations: 1, ..
                },
                TraitHome::Resolved {
                    declarations: 1, ..
                },
            ) => TraitIdentity::Same,
            _ => TraitIdentity::Uncertain,
        })
    }

    pub(crate) fn import_path(
        &mut self,
        source: &mut FindingSource<'_>,
        declarations: &mut DeclarationSource,
        location: SourceLocation<'_>,
        path: &[String],
        position: &ImportPosition,
    ) -> Result<TraitPath, FindingSourceFailure> {
        let key = (location.commit.to_owned(), location.path.to_owned());
        let imports = match self.imports.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => {
                let file = declarations.file(source, location)?;
                entry.insert(FileImports::read(file.words()))
            },
        };
        let header_blocks: Vec<_> = match position {
            ImportPosition::BlockScoped(index) => imports
                .blocks
                .iter()
                .filter_map(|&(start, end)| (start < *index && *index <= end).then_some(start))
                .collect(),
            ImportPosition::FileLevel => Vec::new(),
        };
        let active: Vec<_> = imports
            .uses
            .iter()
            .filter(|item| header_blocks.starts_with(&item.scope))
            .collect();
        let mut expanded = path.to_vec();
        let mut seen = HashSet::new();
        for _ in 0..IMPORT_EXPANSION_LIMIT {
            let TraitPath::Resolved(relative) =
                relative_path(source, declarations, location, expanded, position)?
            else {
                return Ok(TraitPath::Unresolved);
            };
            let (Some(first), Some(last)) = (relative.first(), relative.last()) else {
                return Ok(TraitPath::Unresolved);
            };
            if active.iter().any(|item| item.blocks(first, last)) {
                return Ok(TraitPath::Unresolved);
            }
            let mut bindings = active
                .iter()
                .flat_map(|item| item.bindings())
                .filter(|(name, _)| name == first);
            let Some((_, imported)) = bindings.next() else {
                return Ok(TraitPath::Resolved(relative));
            };
            if bindings.next().is_some() || !seen.insert(first.clone()) {
                return Ok(TraitPath::Unresolved);
            }
            expanded = imported.iter().chain(&relative[1..]).cloned().collect();
        }
        Ok(TraitPath::Unresolved)
    }
}

fn relative_path(
    source: &mut FindingSource<'_>,
    declarations: &mut DeclarationSource,
    location: SourceLocation<'_>,
    path: Vec<String>,
    position: &ImportPosition,
) -> Result<TraitPath, FindingSourceFailure> {
    let first = word_syntax::word(&path, 0);
    if !matches!(first, "self" | "super") {
        return Ok(TraitPath::Resolved(path));
    }
    if matches!(position, ImportPosition::BlockScoped(_)) {
        return Ok(TraitPath::Unresolved);
    }
    let FileModule::Rooted(mut module) = declarations.file_module(source, location)? else {
        return Ok(TraitPath::Unresolved);
    };
    if first == "self" {
        module.extend(path.into_iter().skip(1));
    } else {
        let supers = path.iter().take_while(|part| *part == "super").count();
        if supers >= module.len() {
            return Ok(TraitPath::Unresolved);
        }
        module.truncate(module.len() - supers);
        module.extend(path.into_iter().skip(supers));
    }
    Ok(TraitPath::Resolved(module))
}

fn trait_home(
    source: &mut FindingSource<'_>,
    declarations: &mut DeclarationSource,
    location: SourceLocation<'_>,
    path: &[String],
) -> Result<TraitHome, FindingSourceFailure> {
    let Some(name) = path.last() else {
        return Ok(TraitHome::Unresolved);
    };
    if matches!(word_syntax::word(path, 0), "self" | "super") {
        return Ok(TraitHome::Unresolved);
    }
    let tip = source.tip(location.commit)?;
    let crate_name = tip.crate_of(location.path);
    let mut homes = Vec::new();
    let files = source.files_naming(location.commit, name)?;
    for candidate in files
        .iter()
        .filter(|path| tip.has_rust_path(path) && tip.crate_of(path) == crate_name)
    {
        let candidate = SourceLocation {
            path: candidate,
            ..location
        };
        let file = declarations.file(source, candidate)?;
        let count = file
            .items()
            .iter()
            .filter(|item| item.keyword() == "trait" && item.name == *name && item.is_module_item())
            .count();
        if count > 0 {
            homes.push((declarations.file_module(source, candidate)?, count));
        }
    }
    let at_source = declarations.file_module(source, location)?;
    let count = homes.iter().map(|(_, count)| count).sum();
    let choices = if path.len() == 1 {
        let local: Vec<_> = homes.iter().filter(|(module, _)| {
            matches!((module, &at_source), (FileModule::Rooted(at_home), FileModule::Rooted(at_source)) if at_home == at_source)
        }).collect();
        if local.is_empty() {
            homes.iter().collect()
        } else {
            local
        }
    } else {
        let mut expected = path[..path.len() - 1].to_vec();
        if path[0] != "crate" {
            expected.insert(0, "crate".to_owned());
        }
        homes
            .iter()
            .filter(
                |(module, _)| matches!(module, FileModule::Rooted(at_home) if *at_home == expected),
            )
            .collect()
    };
    Ok(match choices.as_slice() {
        [(FileModule::Rooted(module), 1)] => TraitHome::Resolved {
            module:       module.clone(),
            declarations: count,
        },
        _ => TraitHome::Unresolved,
    })
}

impl FileImports {
    fn read(words: &[String]) -> Self {
        let attributes: HashMap<_, _> = word_syntax::attribute_spans(words)
            .into_iter()
            .map(|(start, end)| (end, start))
            .collect();
        let openings: HashMap<_, _> = word_syntax::delimiter_ends(words)
            .into_iter()
            .map(|(start, end)| (end, start))
            .collect();
        let mut semicolon = words.len();
        let mut semicolons = vec![words.len(); words.len()];
        for (index, word) in words.iter().enumerate().rev() {
            semicolons[index] = semicolon;
            if word == ";" {
                semicolon = index;
            }
        }
        let mut open = Vec::new();
        let mut blocks = Vec::new();
        let mut scopes = HashMap::new();
        for (index, word) in words.iter().enumerate() {
            match word.as_str() {
                "{" => open.push(index),
                "}" => {
                    if let Some(start) = open.pop() {
                        blocks.push((start, index));
                    }
                },
                "use" => {
                    scopes.insert(index, open.clone());
                },
                _ => {},
            }
        }
        blocks.extend(open.into_iter().map(|start| (start, words.len())));
        blocks.sort_unstable();
        let uses = words
            .iter()
            .enumerate()
            .filter(|(_, word)| *word == "use")
            .map(|(index, _)| {
                let start = import_start(words, index, &openings, &attributes);
                let end = semicolons[index];
                // The first use word can occur inside an attribute.
                let first = (start..index)
                    .find(|&word| words[word] == "use")
                    .unwrap_or(index);
                let body = words
                    .get(first + 1..end.min(words.len().saturating_sub(1)))
                    .unwrap_or(&[]);
                let form = if start == index {
                    ImportForm::Plain
                } else {
                    ImportForm::Decorated
                };
                ScopedImport::read(body, &words[index + 1..end], &scopes[&first], form)
            })
            .collect();
        Self { uses, blocks }
    }
}

fn import_start(
    words: &[String],
    index: usize,
    openings: &HashMap<usize, usize>,
    attributes: &HashMap<usize, usize>,
) -> usize {
    let mut start = index;
    if index > 0 && words[index - 1] == "pub" {
        start = index - 1;
    } else if index > 0
        && words[index - 1] == ")"
        && let Some(&begin) = openings.get(&(index - 1))
        && begin > 0
        && words[begin - 1] == "pub"
    {
        start = begin - 1;
    }
    while let Some(&attribute) = attributes.get(&start) {
        start = attribute;
    }
    start
}

impl ScopedImport {
    fn read(body: &[String], paths: &[String], scope: &[usize], form: ImportForm) -> Self {
        let bindings = body.iter().position(|word| word == "as").map_or_else(
            || {
                use_paths(paths, &[])
                    .into_iter()
                    .filter_map(|path| path.last().cloned().map(|name| (name, path)))
                    .collect()
            },
            |alias| {
                let mut paths = use_paths(&body[..alias], &[]);
                if paths.len() == 1
                    && body.len() == alias + 2
                    && word_syntax::identifier(&body[alias + 1])
                {
                    vec![(body[alias + 1].clone(), paths.remove(0))]
                } else {
                    Vec::new()
                }
            },
        );
        let names = match (bindings.as_slice(), form, scope) {
            ([], _, _) => ImportNames::Blocked(
                body.iter()
                    .filter(|word| word_syntax::identifier(word))
                    .cloned()
                    .collect(),
            ),
            (_, ImportForm::Plain, []) => ImportNames::Bindings(bindings),
            _ => ImportNames::Blocked(bindings.into_iter().map(|(name, _)| name).collect()),
        };
        let glob = if body.iter().any(|word| word == "*") {
            ImportGlob::Wildcard
        } else {
            ImportGlob::Explicit
        };
        Self {
            scope: scope.to_vec(),
            names,
            glob,
        }
    }

    fn blocks(&self, first: &str, last: &str) -> bool {
        matches!(self.glob, ImportGlob::Wildcard)
            || matches!(&self.names, ImportNames::Blocked(names) if names.contains(first) || names.contains(last))
    }

    fn bindings(&self) -> &[(String, Vec<String>)] {
        match &self.names {
            ImportNames::Bindings(bindings) => bindings,
            ImportNames::Blocked(_) => &[],
        }
    }
}

fn use_paths(words: &[String], prefix: &[String]) -> Vec<Vec<String>> {
    if let Some(begin) = words.iter().position(|word| word == "{") {
        let end = word_syntax::closing(words, begin);
        if end != words.len() - 1 || (begin > 0 && words[begin - 1] != "::") {
            return Vec::new();
        }
        let stem = &words[..begin.saturating_sub(1)];
        if stem
            .iter()
            .any(|word| word != "::" && !word_syntax::identifier(word))
        {
            return Vec::new();
        }
        let head: Vec<_> = prefix
            .iter()
            .chain(stem.iter().filter(|word| *word != "::"))
            .cloned()
            .collect();
        let mut result = Vec::new();
        let mut start = begin + 1;
        let mut depth = 0_i32;
        for (index, word) in words.iter().enumerate().take(end + 1).skip(start) {
            depth += i32::from(word == "{") - i32::from(word == "}");
            if (word == "," && depth == 0) || index == end {
                if start < index {
                    let paths = use_paths(&words[start..index], &head);
                    if paths.is_empty() {
                        return Vec::new();
                    }
                    result.extend(paths);
                }
                start = index + 1;
            }
        }
        result
    } else if words.len() % 2 == 1
        && words.iter().enumerate().all(|(index, word)| {
            if index % 2 == 0 {
                word_syntax::identifier(word)
            } else {
                word == "::"
            }
        })
    {
        vec![
            prefix
                .iter()
                .chain(words.iter().step_by(2))
                .cloned()
                .collect(),
        ]
    } else {
        Vec::new()
    }
}
