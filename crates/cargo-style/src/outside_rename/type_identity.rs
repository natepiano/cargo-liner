//! Compare the resolved types of implemented items.

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::rc::Rc;

use super::ItemAtRevision;
use super::OutsideSearch;
use super::SourceResult;
use super::source_failure;
use crate::declaration_source::DeclarationFile;
use crate::declaration_source::FileModule;
use crate::declaration_source::SourceLocation;
use crate::declarations::DeclaredItem;
use crate::declarations::ImplMembership;
use crate::declarations::ImplementationState;
use crate::declarations::ImplementedPath;
use crate::declarations::ImportPosition;
use crate::tokenizer;
use crate::trait_identity::TraitIdentity;
use crate::trait_identity::TraitPath;
use crate::word_syntax;

pub(super) enum ExternNames {
    Known(HashSet<String>),
    Unresolved,
}

enum TypeArguments {
    Known(Vec<String>),
    Unresolved,
}

impl OutsideSearch<'_, '_> {
    pub(super) fn type_identity(
        &mut self,
        left: ItemAtRevision<'_>,
        right: ItemAtRevision<'_>,
    ) -> SourceResult<TraitIdentity> {
        self.compare_types(
            left.location,
            left.item.implemented_type(),
            right.location,
            right.item.implemented_type(),
        )
    }

    pub(super) fn compare_types(
        &mut self,
        left: SourceLocation<'_>,
        before: &ImplMembership,
        right: SourceLocation<'_>,
        after: &ImplMembership,
    ) -> SourceResult<TraitIdentity> {
        let (before, after) = match (before, after) {
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
            (ImplMembership::Path(before), ImplMembership::Path(after)) => (before, after),
        };
        let first = self.import_path(left, &before.path, &before.position)?;
        let second = self.import_path(right, &after.path, &after.position)?;
        let (TraitPath::Resolved(mut first), TraitPath::Resolved(mut second)) = (first, second)
        else {
            return Ok(TraitIdentity::Uncertain);
        };
        let contexts = [
            left,
            SourceLocation {
                commit: left.commit,
                path:   right.path,
            },
            SourceLocation {
                commit: right.commit,
                path:   left.path,
            },
            right,
        ];
        let before_arguments = self.type_arguments(left, before, &contexts)?;
        let after_arguments = self.type_arguments(right, after, &contexts)?;
        let (TypeArguments::Known(before_arguments), TypeArguments::Known(after_arguments)) =
            (before_arguments, after_arguments)
        else {
            return Ok(TraitIdentity::Uncertain);
        };
        if before_arguments != after_arguments {
            return Ok(TraitIdentity::Different);
        }
        if word_syntax::word(&first, 0) != "crate" || word_syntax::word(&second, 0) != "crate" {
            let first_module = self.type_module(left)?;
            let second_module = self.type_module(right)?;
            let (FileModule::Rooted(first_module), FileModule::Rooted(second_module)) =
                (first_module, second_module)
            else {
                return Ok(TraitIdentity::Uncertain);
            };
            if before.position != ImportPosition::FileLevel
                || after.position != ImportPosition::FileLevel
            {
                return Ok(TraitIdentity::Uncertain);
            }
            if word_syntax::word(&first, 0) != "crate" {
                first = first_module.into_iter().chain(first).collect();
            }
            if word_syntax::word(&second, 0) != "crate" {
                second = second_module.into_iter().chain(second).collect();
            }
        }
        Ok(if first == second {
            TraitIdentity::Same
        } else {
            TraitIdentity::Different
        })
    }

    fn import_path(
        &mut self,
        location: SourceLocation<'_>,
        path: &[String],
        position: &ImportPosition,
    ) -> SourceResult<TraitPath> {
        self.trait_reader
            .import_path(self.source, self.declarations, location, path, position)
            .map_err(source_failure(
                "read imports for the implemented type",
                location.path,
            ))
    }

    fn type_module(&mut self, location: SourceLocation<'_>) -> SourceResult<FileModule> {
        self.declarations
            .file_module(self.source, location)
            .map_err(source_failure(
                "read the implemented type's module",
                location.path,
            ))
    }

    fn type_arguments(
        &mut self,
        location: SourceLocation<'_>,
        header: &ImplementedPath,
        contexts: &[SourceLocation<'_>],
    ) -> SourceResult<TypeArguments> {
        let arguments = &header.arguments;
        let mut result = Vec::new();
        let mut index = 0;
        while index < arguments.len() {
            let token = word_syntax::word(arguments, index);
            if matches!(token, "<" | ">" | ",") {
                result.push(token.to_owned());
                index += 1;
                continue;
            }
            if !word_syntax::identifier(token) {
                return Ok(TypeArguments::Unresolved);
            }
            let start = index;
            index += 1;
            while index + 1 < arguments.len() && word_syntax::word(arguments, index) == "::" {
                if !word_syntax::identifier(word_syntax::word(arguments, index + 1)) {
                    return Ok(TypeArguments::Unresolved);
                }
                index += 2;
            }
            let parts: Vec<_> = arguments[start..index].iter().step_by(2).cloned().collect();
            let TraitPath::Resolved(rooted) =
                self.import_path(location, &parts, &header.position)?
            else {
                return Ok(TypeArguments::Unresolved);
            };
            if rooted.len() == 1 && word_syntax::word(&rooted, 0) == token && is_primitive(token) {
                result.push(token.to_owned());
                continue;
            }
            let path = if word_syntax::word(&rooted, 0) == "crate" {
                rooted
            } else {
                let TraitPath::Resolved(path) =
                    self.type_argument_path(location, &header.position, token, rooted, contexts)?
                else {
                    return Ok(TypeArguments::Unresolved);
                };
                path
            };
            for (index, segment) in path.into_iter().enumerate() {
                if index > 0 {
                    result.push("::".to_owned());
                }
                result.push(segment);
            }
        }
        Ok(TypeArguments::Known(result))
    }

    fn type_argument_path(
        &mut self,
        location: SourceLocation<'_>,
        position: &ImportPosition,
        spelling: &str,
        rooted: Vec<String>,
        contexts: &[SourceLocation<'_>],
    ) -> SourceResult<TraitPath> {
        let root = word_syntax::word(&rooted, 0);
        for &context in contexts {
            if !matches!(self.extern_names(context)?.as_ref(), ExternNames::Known(names) if !names.contains(spelling) && !names.contains(root))
            {
                return Ok(TraitPath::Unresolved);
            }
        }
        let module = self.type_module(location)?;
        let FileModule::Rooted(module) = module else {
            return Ok(TraitPath::Unresolved);
        };
        if *position != ImportPosition::FileLevel {
            return Ok(TraitPath::Unresolved);
        }
        let file = self.file(location.commit, location.path, "read local type arguments")?;
        let mut locals = file.items().iter().filter(|item| {
            item.name == root
                && matches!(item.keyword(), "mod" | "struct" | "enum" | "union" | "type")
                && *item.import_position() == ImportPosition::FileLevel
        });
        if let Some(local) = locals.next() {
            if locals.next().is_some() || conditional_header(&file, local) {
                return Ok(TraitPath::Unresolved);
            }
            return Ok(TraitPath::Resolved(
                module.into_iter().chain(rooted).collect(),
            ));
        }
        if rooted.len() == 1 {
            Ok(TraitPath::Unresolved)
        } else {
            Ok(TraitPath::Resolved(rooted))
        }
    }

    fn extern_names(&mut self, location: SourceLocation<'_>) -> SourceResult<Rc<ExternNames>> {
        let key = (location.commit.to_owned(), location.path.to_owned());
        if let Some(names) = self.reader.extern_names.get(&key) {
            return Ok(Rc::clone(names));
        }
        let tip = self.source.tip(location.commit).map_err(source_failure(
            "read the tip for extern crates",
            location.path,
        ))?;
        let crate_name = tip.crate_of(location.path);
        let prefix = if crate_name == "." {
            "src".to_owned()
        } else {
            format!("{crate_name}/src")
        };
        let lib = format!("{prefix}/lib.rs");
        let main = format!("{prefix}/main.rs");
        let paths = BTreeSet::from([location.path, lib.as_str(), main.as_str()]);
        let mut names = HashSet::new();
        let mut reading = ExternNames::Known(HashSet::new());
        for path in paths.into_iter().filter(|path| tip.blob(path).is_some()) {
            let file = self.file(location.commit, path, "read extern crate declarations")?;
            match read_extern_names(file.words()) {
                ExternNames::Known(found) => names.extend(found),
                ExternNames::Unresolved => {
                    reading = ExternNames::Unresolved;
                    break;
                },
            }
        }
        if matches!(reading, ExternNames::Known(_)) {
            reading = ExternNames::Known(names);
        }
        let reading = Rc::new(reading);
        self.reader.extern_names.insert(key, Rc::clone(&reading));
        Ok(reading)
    }
}

fn conditional_header(file: &DeclarationFile, item: &DeclaredItem) -> bool {
    let text = file
        .lines()
        .slice(*item.lines().start()..=item.keyword_line());
    let tokens = tokenizer::read_with_comments(&text.chars().collect::<Vec<_>>()).tokens;
    let words: Vec<_> = tokens.into_iter().map(|token| token.value).collect();
    word_syntax::attribute_spans(&words)
        .iter()
        .any(|&(start, _)| matches!(word_syntax::word(&words, start + 2), "cfg" | "cfg_attr"))
}

fn read_extern_names(words: &[String]) -> ExternNames {
    let attribute_ends: HashSet<_> = word_syntax::attribute_spans(words)
        .into_iter()
        .map(|(_, end)| end)
        .collect();
    let mut depth = 0_i64;
    let mut names = HashSet::new();
    for (index, token) in words.iter().enumerate() {
        if token == "extern" && word_syntax::word(words, index + 1) == "crate" {
            let Some(end) =
                (index + 2..words.len()).find(|&cursor| word_syntax::word(words, cursor) == ";")
            else {
                return ExternNames::Unresolved;
            };
            let body = &words[index + 2..end];
            let (name, alias) = match body {
                [name] => (name, name),
                [name, keyword, alias] if keyword == "as" => (name, alias),
                _ => return ExternNames::Unresolved,
            };
            let start =
                index - usize::from(index > 0 && word_syntax::word(words, index - 1) == "pub");
            if depth != 0
                || attribute_ends.contains(&start)
                || (start > 0 && !matches!(word_syntax::word(words, start - 1), ";" | "}"))
                || !word_syntax::identifier(name)
                || !word_syntax::identifier(alias)
            {
                return ExternNames::Unresolved;
            }
            names.extend([name.clone(), alias.clone()]);
        }
        match token.as_str() {
            "{" | "(" | "[" => depth += 1,
            "}" | ")" | "]" => depth -= 1,
            _ => {},
        }
    }
    ExternNames::Known(names)
}

const fn is_primitive(name: &str) -> bool {
    matches!(
        name.as_bytes(),
        b"bool"
            | b"char"
            | b"str"
            | b"u8"
            | b"u16"
            | b"u32"
            | b"u64"
            | b"u128"
            | b"usize"
            | b"i8"
            | b"i16"
            | b"i32"
            | b"i64"
            | b"i128"
            | b"isize"
            | b"f32"
            | b"f64"
    )
}
