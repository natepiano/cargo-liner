//! Prove that the child site of a renamed declaration survived.

use std::collections::HashMap;
use std::hash::Hash;
use std::ops::Range;

use crate::declaration_source::DeclarationFile;
use crate::declarations::DeclaredItem;
use crate::declarations::ItemBody;
use crate::same_declaration::Declaration;
use crate::source_sites;
use crate::tokenizer;
use crate::word_syntax;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SiteKind {
    Header,
    Statement,
    NamedChild,
    Unreadable,
}

pub(crate) enum ChildSite {
    OwnerHeader,
    Retained(usize),
    Unproved,
}

enum BindingAt<'a, 'b> {
    Binding(&'b LocalBinding<'a>),
    NoBinding,
    Unreadable,
}

struct LocalBinding<'a> {
    name:    &'a str,
    line:    usize,
    scope:   Vec<usize>,
    closure: BindingClosure<'a>,
}

#[derive(Clone, PartialEq, Eq)]
enum BindingClosure<'a> {
    None,
    Parameter {
        ordinal: usize,
        header:  &'a [String],
        origin:  ClosureOrigin<'a>,
    },
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ClosureOrigin<'a> {
    Initializer(&'a str),
    CallArgument { callee: &'a [String], index: usize },
    Unproved,
}

#[derive(PartialEq, Eq)]
struct BlockSignature<'a> {
    ordinal: usize,
    header:  &'a [String],
}

struct LocalBindings<'a> {
    bindings:   Vec<LocalBinding<'a>>,
    signatures: HashMap<usize, BlockSignature<'a>>,
}

pub(crate) fn prove(
    before: &DeclarationFile,
    owner: &DeclaredItem,
    line: usize,
    after: &DeclarationFile,
    target_declaration: &Declaration,
) -> ChildSite {
    let old_bindings = LocalBindings::read(before, owner);
    let mut targets = after
        .items()
        .iter()
        .filter(|item| target_declaration.matches(item));
    match old_bindings.at(before, line) {
        BindingAt::Unreadable => ChildSite::Unproved,
        BindingAt::Binding(binding) => {
            let Some(target) = targets.next() else {
                return ChildSite::Unproved;
            };
            if targets.next().is_some() {
                return ChildSite::Unproved;
            }
            let new_bindings = LocalBindings::read(after, target);
            let mut children = new_bindings
                .bindings
                .iter()
                .filter(|child| child.name == binding.name);
            let Some(child) = children.next() else {
                return ChildSite::Unproved;
            };
            if children.next().is_none()
                && child.closure == binding.closure
                && child.scope.len() == binding.scope.len()
                && old_bindings.scope(binding).eq(new_bindings.scope(child))
            {
                ChildSite::Retained(child.line)
            } else {
                ChildSite::Unproved
            }
        },
        BindingAt::NoBinding => {
            let kind = site_kind(before, owner, line);
            if kind == SiteKind::Header {
                return ChildSite::OwnerHeader;
            }
            let Some(target) = targets.next() else {
                return ChildSite::Unproved;
            };
            if targets.next().is_some() {
                return ChildSite::Unproved;
            }
            prove_kept_line(before, owner, line, after, target, kind)
        },
    }
}

fn prove_kept_line(
    before: &DeclarationFile,
    old: &DeclaredItem,
    line: usize,
    after: &DeclarationFile,
    item: &DeclaredItem,
    kind: SiteKind,
) -> ChildSite {
    let text = source_sites::trim(before.line_text(line));
    if text.is_empty() {
        return ChildSite::Unproved;
    }
    let source = (old.keyword_line() + 1)..*old.lines().end();
    let target = (item.keyword_line() + 1)..*item.lines().end();
    if !source
        .clone()
        .filter(|&number| source_sites::trim(before.line_text(number)) == text)
        .eq([line])
    {
        return ChildSite::Unproved;
    }
    let mut matches = target
        .clone()
        .filter(|&number| source_sites::trim(after.line_text(number)) == text);
    let Some(target_line) = matches.next() else {
        return ChildSite::Unproved;
    };
    if matches.next().is_some() || site_kind(after, item, target_line) != kind {
        return ChildSite::Unproved;
    }
    if kind == SiteKind::Statement {
        return ChildSite::Retained(target_line);
    }
    if kind != SiteKind::NamedChild
        || target_line - item.keyword_line() != line - old.keyword_line()
    {
        return ChildSite::Unproved;
    }
    let Some((name, tail)) = named_child(text) else {
        return ChildSite::Unproved;
    };
    if tail.is_empty() || tail.contains(';') {
        return ChildSite::Unproved;
    }
    if source
        .filter(|&number| {
            named_child(before.line_text(number)).is_some_and(|(other, _)| other == name)
        })
        .eq([line])
        && target
            .filter(|&number| {
                named_child(after.line_text(number)).is_some_and(|(other, _)| other == name)
            })
            .eq([target_line])
    {
        ChildSite::Retained(target_line)
    } else {
        ChildSite::Unproved
    }
}

fn named_child(line: &str) -> Option<(&str, &str)> {
    let text = source_sites::trim(line);
    let after_visibility = text.strip_prefix("pub").and_then(|rest| {
        let rest = if let Some(scope) = rest.strip_prefix('(') {
            scope.split_once(')')?.1
        } else {
            rest
        };
        let name = rest.trim_start_matches(tokenizer::python_space);
        (name.len() < rest.len()).then_some(name)
    });
    after_visibility
        .and_then(child_name_and_tail)
        .or_else(|| child_name_and_tail(text))
}

fn child_name_and_tail(text: &str) -> Option<(&str, &str)> {
    if !text.starts_with(|character: char| character.is_ascii_alphabetic() || character == '_') {
        return None;
    }
    let end = text
        .find(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .unwrap_or(text.len());
    let (name, tail) = text.split_at(end);
    Some((
        name,
        tail.trim_start_matches(tokenizer::python_space)
            .strip_prefix(':')?,
    ))
}

fn site_kind(file: &DeclarationFile, item: &DeclaredItem, line: usize) -> SiteKind {
    if !item.lines().contains(&line) {
        return SiteKind::Unreadable;
    }
    if line <= item.keyword_line() {
        return SiteKind::Header;
    }
    let ItemBody::Braced(body) = file.body(item) else {
        return SiteKind::Unreadable;
    };
    if body.close >= file.words().len() {
        return SiteKind::Unreadable;
    }
    if line <= file.word_line(body.open) {
        SiteKind::NamedChild
    } else if line >= file.word_line(body.close) {
        SiteKind::Unreadable
    } else if item.keyword() == "fn" {
        SiteKind::Statement
    } else {
        SiteKind::NamedChild
    }
}

impl<'a> LocalBindings<'a> {
    fn read(file: &'a DeclarationFile, owner: &DeclaredItem) -> Self {
        let mut result = Self {
            bindings:   Vec::new(),
            signatures: HashMap::new(),
        };
        let ItemBody::Braced(body) = file.body(owner) else {
            return result;
        };
        let words = file.words();
        let mut blocks = Vec::new();
        let mut block_orders = HashMap::new();
        let mut closures = HashMap::new();
        let mut origins = HashMap::new();
        let mut next_closure = body.open;
        let end = body.close.min(words.len());
        for index in body.open..end {
            let token = word_syntax::word(words, index);
            if token == "{" {
                if let Some(&parent) = blocks.last() {
                    let ordinal = take_ordinal(&mut block_orders, parent);
                    let mut start = index;
                    while start > parent + 1
                        && !matches!(word_syntax::word(words, start - 1), ";" | "{" | "}")
                    {
                        start -= 1;
                    }
                    result.signatures.insert(
                        index,
                        BlockSignature {
                            ordinal,
                            header: &words[start..index],
                        },
                    );
                }
                blocks.push(index);
            } else if token == "}" {
                let _ = blocks.pop();
            }
            if token == "let" {
                let name = index + 1 + usize::from(word_syntax::word(words, index + 1) == "mut");
                if word_syntax::identifier(word_syntax::word(words, name))
                    && matches!(word_syntax::word(words, name + 1), "=" | ":")
                {
                    result.bindings.push(LocalBinding {
                        name:    word_syntax::word(words, name),
                        line:    file.word_line(name),
                        scope:   blocks.clone(),
                        closure: BindingClosure::None,
                    });
                }
            }
            if matches!(token, "|" | "||")
                && index >= next_closure
                && index > 0
                && matches!(
                    word_syntax::word(words, index - 1),
                    "move" | "=" | "(" | ","
                )
                && let Some(&block) = blocks.last()
            {
                let ordinal = take_ordinal(&mut closures, blocks.clone());
                let closure_end = if token == "|" {
                    (index + 1..end)
                        .find(|&cursor| word_syntax::word(words, cursor) == "|")
                        .unwrap_or(index)
                } else {
                    index
                };
                let start = index - usize::from(word_syntax::word(words, index - 1) == "move");
                let origin = closure_context(words, block, start);
                let _ = take_ordinal(&mut origins, (blocks.clone(), origin.clone()));
                let closure = BindingClosure::Parameter {
                    ordinal,
                    header: &words[start..=closure_end],
                    origin,
                };
                result.add_closure_parameters(file, index + 1..closure_end, &blocks, &closure);
                next_closure = closure_end + 1;
            }
        }
        for binding in &mut result.bindings {
            if let BindingClosure::Parameter { origin, .. } = &mut binding.closure
                && origins.get(&(binding.scope.clone(), origin.clone())) != Some(&1)
            {
                *origin = ClosureOrigin::Unproved;
            }
        }
        result
    }

    fn add_closure_parameters(
        &mut self,
        file: &'a DeclarationFile,
        parameters: Range<usize>,
        scope: &[usize],
        closure: &BindingClosure<'a>,
    ) {
        let words = file.words();
        if parameters.clone().any(|cursor| {
            let token = word_syntax::word(words, cursor);
            token != "," && !word_syntax::identifier(token)
        }) {
            return;
        }
        for cursor in parameters.filter(|&cursor| word_syntax::word(words, cursor) != ",") {
            self.bindings.push(LocalBinding {
                name:    word_syntax::word(words, cursor),
                line:    file.word_line(cursor),
                scope:   scope.to_vec(),
                closure: closure.clone(),
            });
        }
    }

    fn at<'b>(&'b self, file: &DeclarationFile, line: usize) -> BindingAt<'a, 'b> {
        let mut bindings = self.bindings.iter().filter(|binding| binding.line == line);
        if let Some(binding) = bindings.next() {
            if bindings.next().is_some()
                || matches!(
                    binding.closure,
                    BindingClosure::Parameter {
                        origin: ClosureOrigin::Unproved,
                        ..
                    }
                )
            {
                return BindingAt::Unreadable;
            }
            return BindingAt::Binding(binding);
        }
        let text: Vec<_> = file.line_text(line).chars().collect();
        if tokenizer::read_with_comments(&text)
            .tokens
            .iter()
            .any(|token| matches!(token.value.as_str(), "let" | "|"))
        {
            BindingAt::Unreadable
        } else {
            BindingAt::NoBinding
        }
    }

    fn scope(&self, binding: &LocalBinding<'a>) -> impl Iterator<Item = &BlockSignature<'a>> {
        binding
            .scope
            .iter()
            .skip(1)
            .filter_map(move |index| self.signatures.get(index))
    }
}

fn take_ordinal<K: Eq + Hash>(orders: &mut HashMap<K, usize>, key: K) -> usize {
    let next = orders.entry(key).or_default();
    let ordinal = *next;
    *next += 1;
    ordinal
}

fn closure_context(words: &[String], block: usize, start: usize) -> ClosureOrigin<'_> {
    let mut stack = Vec::new();
    let mut statement = block + 1;
    for cursor in block + 1..start {
        match word_syntax::word(words, cursor) {
            "(" | "[" | "{" => stack.push(cursor),
            close @ (")" | "]" | "}") => {
                if stack
                    .pop()
                    .is_none_or(|opening| !paired(word_syntax::word(words, opening), close))
                {
                    return ClosureOrigin::Unproved;
                }
                if close == "}" && stack.is_empty() && word_syntax::word(words, statement) != "let"
                {
                    statement = cursor + 1;
                }
            },
            ";" if stack.is_empty() => statement = cursor + 1,
            _ => {},
        }
    }
    let prefix = &words[statement..start];
    if word_syntax::word(prefix, 0) == "let" {
        let name = 1 + usize::from(word_syntax::word(prefix, 1) == "mut");
        if !word_syntax::identifier(word_syntax::word(prefix, name)) || name + 1 >= prefix.len() {
            return ClosureOrigin::Unproved;
        }
        match word_syntax::word(prefix, name + 1) {
            "=" => return ClosureOrigin::Initializer(word_syntax::word(prefix, name)),
            ":" => {
                let Some(assignment) = type_assignment(prefix, name + 2) else {
                    return ClosureOrigin::Unproved;
                };
                if word_syntax::word(prefix, assignment) == "=" {
                    return ClosureOrigin::Initializer(word_syntax::word(prefix, name));
                }
            },
            _ => {},
        }
    }
    let Some(&opening) = stack.last() else {
        return ClosureOrigin::Unproved;
    };
    if opening == 0
        || word_syntax::word(words, opening) != "("
        || !word_syntax::identifier(word_syntax::word(words, opening - 1))
        || !matches!(word_syntax::word(words, start - 1), "(" | ",")
    {
        return ClosureOrigin::Unproved;
    }
    let mut callee = opening - 1;
    while callee >= 2
        && matches!(word_syntax::word(words, callee - 1), "::" | ".")
        && word_syntax::identifier(word_syntax::word(words, callee - 2))
    {
        callee -= 2;
    }
    let mut index = 0;
    let mut cursor = opening + 1;
    while cursor < start {
        match word_syntax::word(words, cursor) {
            "(" | "[" | "{" => cursor = word_syntax::closing(words, cursor),
            "," => index += 1,
            _ => {},
        }
        cursor += 1;
    }
    ClosureOrigin::CallArgument {
        callee: &words[callee..opening],
        index,
    }
}

fn type_assignment(prefix: &[String], start: usize) -> Option<usize> {
    let mut brackets = Vec::new();
    let mut cursor = start;
    while cursor < prefix.len() {
        match word_syntax::word(prefix, cursor) {
            "=" if brackets.is_empty() => break,
            opening @ ("(" | "[" | "{" | "<") => brackets.push(opening),
            close @ (")" | "]" | "}" | ">") if !paired(brackets.pop()?, close) => return None,
            _ => {},
        }
        cursor += 1;
    }
    (brackets.is_empty() && cursor != start).then_some(cursor)
}

const fn paired(open: &str, close: &str) -> bool {
    matches!(
        (open.as_bytes(), close.as_bytes()),
        (b"(", b")") | (b"[", b"]") | (b"{", b"}") | (b"<", b">")
    )
}
