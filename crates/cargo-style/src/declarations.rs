//! The Python declaration reader, with the same word spans and ownership rules.

use std::collections::BTreeMap;

use serde::Serialize;
use serde::Serializer;

#[cfg(test)]
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::word_syntax;

/// The direct owner of a declaration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DeclarationOwner {
    Module,
    ImplOrTrait(String),
}

/// The import scope of a declaration or its owning impl header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ImportPosition {
    FileLevel,
    BlockScoped(usize),
}

/// An impl has no target of this kind, or its target cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ImplementationState {
    None,
    Unreadable,
}

/// A complete impl path and its generic arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
struct ImplementedPath {
    path:      Vec<String>,
    arguments: Vec<String>,
    position:  ImportPosition,
}

/// The trait or type relationship of a direct impl member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(untagged)]
enum ImplMembership {
    State(ImplementationState),
    Path(ImplementedPath),
}

impl ImplMembership {
    const fn none() -> Self { Self::State(ImplementationState::None) }

    const fn unreadable() -> Self { Self::State(ImplementationState::Unreadable) }

    const fn at(mut self, position: ImportPosition) -> Self {
        if let Self::Path(path) = &mut self {
            path.position = position;
        }
        self
    }
}

/// One named declaration and its source lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct DeclaredItem {
    keyword:           String,
    pub(crate) name:   String,
    owner:             DeclarationOwner,
    keyword_line:      usize,
    first_line:        usize,
    last_line:         usize,
    implemented_trait: ImplMembership,
    import_position:   ImportPosition,
    implemented_type:  ImplMembership,
}

/// The word indexes of a complete or incomplete braced body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct BodyWords {
    pub(crate) open:  usize,
    pub(crate) close: usize,
}

/// A declaration ends at a semicolon or has a braced body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ItemBody {
    NoBody,
    Braced(BodyWords),
}

impl Serialize for ItemBody {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::NoBody => serializer.serialize_str("none"),
            Self::Braced(body) => body.serialize(serializer),
        }
    }
}

/// The keyword, end, and body word indexes of one item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ItemSpan {
    keyword:         usize,
    end:             usize,
    pub(crate) body: ItemBody,
}

/// A file's declarations and the tokens used to read their bodies.
#[derive(Debug, Serialize)]
pub(crate) struct DeclaredFile {
    pub(crate) items:  Vec<DeclaredItem>,
    pub(crate) spans:  Vec<ItemSpan>,
    others:            Vec<ItemSpan>,
    #[serde(skip)]
    pub(crate) tokens: Vec<Token>,
}

/// A readable impl owner, or an owner whose members Python omits.
#[derive(Clone, Debug)]
enum ImplOwner {
    Readable(String),
    Unreadable,
}

struct ImplHeader {
    owner:             ImplOwner,
    implemented_trait: ImplMembership,
    implemented_type:  ImplMembership,
}

struct OwningBody {
    body:   BodyWords,
    header: ImplHeader,
}

/// The path before generic arguments or a where clause.
fn type_path(words: &[String]) -> Vec<String> {
    let finish = words
        .iter()
        .position(|value| matches!(value.as_str(), "<" | "where"))
        .unwrap_or(words.len());
    let path = &words[..finish];
    if path.len() % 2 != 1
        || !path
            .iter()
            .enumerate()
            .all(|(index, value)| match index % 2 {
                0 => word_syntax::identifier(value),
                _ => value == "::",
            })
    {
        return Vec::new();
    }
    path.iter().step_by(2).cloned().collect()
}

/// Read a full trait or type path without dropping trailing words.
fn implemented_path(words: &[String]) -> ImplMembership {
    let path = type_path(words);
    if path.is_empty() {
        return ImplMembership::unreadable();
    }
    let start = path.len() * 2 - 1;
    let readable = |arguments| {
        ImplMembership::Path(ImplementedPath {
            path: path.clone(),
            arguments,
            position: ImportPosition::FileLevel,
        })
    };
    if start == words.len() {
        return readable(Vec::new());
    }
    if word_syntax::word(words, start) != "<" || words.last().is_none_or(|value| value != ">") {
        return ImplMembership::unreadable();
    }
    let mut depth = 0_i64;
    let mut brackets = Vec::new();
    for (index, value) in words.iter().enumerate().skip(start) {
        match value.as_str() {
            "(" | "[" | "{" => brackets.push(value.as_str()),
            ")" | "]" | "}" => {
                let opening = match value.as_str() {
                    ")" => "(",
                    "]" => "[",
                    _ => "{",
                };
                if brackets.pop() != Some(opening) {
                    return ImplMembership::unreadable();
                }
            },
            _ if brackets.is_empty() => {
                depth += i64::from(value == "<") - i64::from(value == ">");
                if depth <= 0 && index != words.len() - 1 {
                    return ImplMembership::unreadable();
                }
            },
            _ => {},
        }
    }
    if depth != 0 || !brackets.is_empty() || words.len() - start <= 2 {
        return ImplMembership::unreadable();
    }
    readable(words[start..].to_vec())
}

/// Read an item through its closing brace or semicolon.
fn span(words: &[String], keyword: usize) -> ItemSpan {
    let braced = matches!(
        word_syntax::word(words, keyword),
        "fn" | "struct" | "enum" | "union" | "trait" | "mod" | "impl"
    );
    let mut cursor = keyword + 1;
    let mut generic_depth = 0_usize;
    while cursor < words.len() {
        match word_syntax::word(words, cursor) {
            "(" | "[" => cursor = word_syntax::closing(words, cursor),
            "<" => generic_depth += 1,
            ">" if generic_depth > 0 => generic_depth -= 1,
            "{" => {
                let end = word_syntax::closing(words, cursor);
                if braced && generic_depth == 0 {
                    return ItemSpan {
                        keyword,
                        end: end.min(words.len() - 1),
                        body: ItemBody::Braced(BodyWords {
                            open:  cursor,
                            close: end,
                        }),
                    };
                }
                cursor = end;
            },
            ";" => {
                return ItemSpan {
                    keyword,
                    end: cursor,
                    body: ItemBody::NoBody,
                };
            },
            _ => {},
        }
        cursor += 1;
    }
    ItemSpan {
        keyword,
        end: words.len() - 1,
        body: ItemBody::NoBody,
    }
}

/// Read the type after `for`, excluding higher-ranked bound separators.
fn impl_header(words: &[String], span: &ItemSpan) -> ImplHeader {
    let end = match &span.body {
        ItemBody::Braced(body) => body.open,
        ItemBody::NoBody => span.end,
    };
    let header = words.get(span.keyword + 1..end).unwrap_or_default();
    let mut depth = 0_i64;
    let mut begin = 0;
    if word_syntax::word(header, 0) == "<" {
        for (index, value) in header.iter().enumerate() {
            depth += i64::from(value == "<") - i64::from(value == ">");
            if depth == 0 {
                begin = index + 1;
                break;
            }
        }
    }
    depth = 0;
    let separator = (begin..header.len()).find(|&index| {
        depth += i64::from(header[index] == "<") - i64::from(header[index] == ">");
        header[index] == "for" && depth == 0 && word_syntax::word(header, index + 1) != "<"
    });
    let type_start = separator.map_or(begin, |index| index + 1);
    let type_words = &header[type_start..];
    let path = type_path(type_words);
    depth = 0;
    let type_end = type_words
        .iter()
        .position(|value| {
            depth += i64::from(value == "<") - i64::from(value == ">");
            value == "where" && depth == 0
        })
        .unwrap_or(type_words.len());
    let implemented_type = implemented_path(&type_words[..type_end]);
    let implemented_trait = separator.map_or_else(ImplMembership::none, |index| {
        implemented_path(&header[begin..index])
    });
    ImplHeader {
        owner: path.last().map_or(ImplOwner::Unreadable, |name| {
            ImplOwner::Readable(name.clone())
        }),
        implemented_trait,
        implemented_type,
    }
}

/// Read each item keyword and its span in source order.
fn item_spans(words: &[String]) -> BTreeMap<usize, ItemSpan> {
    let starts = word_syntax::item_starts(words);
    words
        .iter()
        .enumerate()
        .filter(|(index, value)| {
            starts.contains(index)
                && matches!(
                    value.as_str(),
                    "fn" | "struct"
                        | "enum"
                        | "union"
                        | "trait"
                        | "type"
                        | "const"
                        | "static"
                        | "mod"
                        | "impl"
                        | "use"
                )
        })
        .map(|(index, _)| (index, span(words, index)))
        .collect()
}

/// Record the nearest brace block for every item header.
fn enclosing_braces(
    words: &[String],
    all_spans: &BTreeMap<usize, ItemSpan>,
) -> BTreeMap<usize, usize> {
    let mut enclosing = BTreeMap::new();
    let mut braces = Vec::new();
    for (index, value) in words.iter().enumerate() {
        if all_spans.contains_key(&index)
            && let Some(&parent) = braces.last()
        {
            enclosing.insert(index, parent);
        }
        match value.as_str() {
            "{" => braces.push(index),
            "}" if !braces.is_empty() => {
                braces.pop();
            },
            _ => {},
        }
    }
    enclosing
}

/// Read the impl and trait bodies that directly own declarations.
fn owning_bodies(
    words: &[String],
    all_spans: &BTreeMap<usize, ItemSpan>,
    enclosing: &BTreeMap<usize, usize>,
) -> Vec<OwningBody> {
    let mut owning = Vec::new();
    for (&index, span) in all_spans {
        if let ItemBody::Braced(body) = &span.body {
            match word_syntax::word(words, index) {
                "impl" => {
                    let mut header = impl_header(words, span);
                    if enclosing.contains_key(&index) {
                        header.implemented_trait = header
                            .implemented_trait
                            .at(ImportPosition::BlockScoped(index));
                        header.implemented_type = header
                            .implemented_type
                            .at(ImportPosition::BlockScoped(index));
                    }
                    owning.push(OwningBody {
                        body: body.clone(),
                        header,
                    });
                },
                "trait" if index + 1 < words.len() => owning.push(OwningBody {
                    body:   body.clone(),
                    header: ImplHeader {
                        owner:             ImplOwner::Readable(words[index + 1].clone()),
                        implemented_trait: ImplMembership::none(),
                        implemented_type:  ImplMembership::none(),
                    },
                }),
                _ => {},
            }
        }
    }
    owning
}

/// Read declarations from a request's shared token reading.
pub(crate) fn read_tokens(text: &[char], tokens: &[Token]) -> DeclaredFile {
    let words: Vec<String> = tokens.iter().map(|token| token.value.clone()).collect();
    let mut first_attribute = BTreeMap::new();
    for (start, keyword) in word_syntax::attribute_owners(&words).into_iter().rev() {
        first_attribute.insert(keyword, start);
    }
    let all_spans = item_spans(&words);
    let body_headers: BTreeMap<usize, usize> = all_spans
        .iter()
        .filter_map(|(&index, span)| match &span.body {
            ItemBody::Braced(body) => Some((body.open, index)),
            ItemBody::NoBody => None,
        })
        .collect();
    let enclosing = enclosing_braces(&words, &all_spans);
    let owning = owning_bodies(&words, &all_spans, &enclosing);
    let mut file = DeclaredFile {
        items:  Vec::new(),
        spans:  Vec::new(),
        others: Vec::new(),
        tokens: tokens.to_vec(),
    };
    for (index, span) in all_spans {
        let keyword = word_syntax::word(&words, index);
        if matches!(keyword, "impl" | "use") {
            file.others.push(span);
            continue;
        }
        let offset = match (keyword, word_syntax::word(&words, index + 1)) {
            ("static", "mut") => 2,
            _ => 1,
        };
        let name = word_syntax::word(&words, index + offset);
        if matches!(name, "" | "_" | "fn") || !word_syntax::identifier(name) {
            continue;
        }
        let owning_header = owning
            .iter()
            .find(|body| Some(&body.body.open) == enclosing.get(&index));
        let (owner, implemented_trait, implemented_type) =
            match owning_header.map(|body| &body.header) {
                Some(ImplHeader {
                    owner: ImplOwner::Unreadable,
                    ..
                }) => continue,
                Some(ImplHeader {
                    owner: ImplOwner::Readable(name),
                    implemented_trait,
                    implemented_type,
                }) => (
                    DeclarationOwner::ImplOrTrait(name.clone()),
                    implemented_trait.clone(),
                    implemented_type.clone(),
                ),
                None => (
                    DeclarationOwner::Module,
                    ImplMembership::none(),
                    ImplMembership::none(),
                ),
            };
        let import_header = match owner {
            DeclarationOwner::ImplOrTrait(_) => body_headers[&enclosing[&index]],
            DeclarationOwner::Module => index,
        };
        let import_position = if enclosing.contains_key(&import_header) {
            ImportPosition::BlockScoped(import_header)
        } else {
            ImportPosition::FileLevel
        };
        file.items.push(DeclaredItem {
            keyword: keyword.to_owned(),
            name: name.to_owned(),
            owner,
            keyword_line: tokens[index].line,
            first_line: tokens[*first_attribute.get(&index).unwrap_or(&index)].line,
            last_line: tokens[span.end].line,
            implemented_trait,
            import_position,
            implemented_type,
        });
        file.spans.push(span);
    }
    // Source offsets count characters; the body reader receives the original text separately.
    debug_assert!(file.tokens.iter().all(|token| token.start <= text.len()));
    file
}

/// Read declarations for the tests without a request token reading.
#[cfg(test)]
pub(crate) fn read(text: &[char]) -> DeclaredFile { read_tokens(text, &tokenizer::tokenize(text)) }

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::DeclarationOwner;
    use super::ImplMembership;
    use super::ImportPosition;
    use super::read;

    #[test]
    fn members_keep_their_owner_paths_and_import_scope() -> Result<(), Box<dyn Error>> {
        let source = "fn host() { impl<T> path::Trait<T> for path::Value<T> where T: Copy { fn work() {} } }";
        let file = read(&source.chars().collect::<Vec<_>>());
        let item = file
            .items
            .iter()
            .find(|item| item.name == "work")
            .ok_or("the method is missing")?;
        assert_eq!(
            item.owner,
            DeclarationOwner::ImplOrTrait("Value".to_owned())
        );
        assert!(matches!(
            item.import_position,
            ImportPosition::BlockScoped(_)
        ));
        match &item.implemented_trait {
            ImplMembership::Path(path) => {
                assert_eq!(path.path, ["path", "Trait"]);
                assert_eq!(path.arguments, ["<", "T", ">"]);
                assert!(matches!(path.position, ImportPosition::BlockScoped(_)));
            },
            ImplMembership::State(_) => return Err("the trait path is unreadable".into()),
        }
        Ok(())
    }

    #[test]
    fn attributes_constants_and_unreadable_impl_owners_match_python_items() {
        let source = "#[inline]\nconst fn work() {} static mut STATE: u8 = 1; const _: u8 = 2; impl &Value { fn omitted() {} }";
        let file = read(&source.chars().collect::<Vec<_>>());
        assert_eq!(
            file.items
                .iter()
                .map(|item| (item.keyword.as_str(), item.name.as_str()))
                .collect::<Vec<_>>(),
            [("fn", "work"), ("static", "STATE")]
        );
        assert_eq!(file.items[0].first_line, 1);
        assert_eq!(file.items[0].keyword_line, 2);
    }
}
