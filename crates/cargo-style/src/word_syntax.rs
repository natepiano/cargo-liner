//! Shared word readers ported from the Python style readers.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;

/// One inline module and its opening and closing word indexes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InlineModule {
    pub(crate) name: String,
    start:           usize,
    end:             usize,
}

/// Read a word, with an empty value after the last word.
pub(crate) fn word(words: &[String], index: usize) -> &str {
    words.get(index).map_or("", String::as_str)
}

/// Strip raw prefixes from identifier tokens only.
pub(crate) fn plain_words(tokens: &[Token]) -> Vec<String> {
    tokens
        .iter()
        .map(|token| match token.kind {
            TokenKind::Identifier => token
                .value
                .strip_prefix("r#")
                .unwrap_or(&token.value)
                .to_owned(),
            _ => token.value.clone(),
        })
        .collect()
}

/// Whether a word matches the Python ASCII identifier pattern.
pub(crate) fn identifier(value: &str) -> bool {
    let plain = value.strip_prefix("r#").unwrap_or(value);
    let mut characters = plain.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// The matching closer, or the word count when delimiters do not match.
pub(crate) fn closing(words: &[String], start: usize) -> usize {
    let closer = |value| match value {
        "{" => "}",
        "(" => ")",
        "[" => "]",
        _ => "",
    };
    let mut stack = vec![closer(word(words, start))];
    for (index, value) in words.iter().enumerate().skip(start + 1) {
        match value.as_str() {
            "{" | "(" | "[" => stack.push(closer(value)),
            "}" | ")" | "]" => {
                if stack.pop() != Some(value.as_str()) {
                    return words.len();
                }
                if stack.is_empty() {
                    return index;
                }
            },
            _ => {},
        }
    }
    words.len()
}

/// Every attribute from its hash to the word after its closing bracket.
pub(crate) fn attribute_spans(words: &[String]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut cursor = 0;
    while cursor < words.len() {
        if word(words, cursor) != "#" {
            cursor += 1;
            continue;
        }
        let start = cursor;
        cursor += 1;
        if word(words, cursor) == "!" {
            cursor += 1;
        }
        if cursor == words.len() || word(words, cursor) != "[" {
            continue;
        }
        let mut depth = 1;
        cursor += 1;
        while cursor < words.len() && depth > 0 {
            match word(words, cursor) {
                "[" => depth += 1,
                "]" => depth -= 1,
                _ => {},
            }
            cursor += 1;
        }
        spans.push((start, cursor));
    }
    spans
}

/// The item keyword that each attribute decorates.
pub(crate) fn attribute_owners(words: &[String]) -> BTreeMap<usize, usize> {
    let spans = attribute_spans(words);
    let following: BTreeMap<_, _> = spans.iter().copied().collect();
    let mut owners = BTreeMap::new();
    for (start, end) in spans {
        let mut cursor = end;
        while cursor < words.len() {
            if let Some(&next) = following.get(&cursor) {
                cursor = next;
                continue;
            }
            let value = word(words, cursor);
            if matches!(
                value,
                "fn" | "struct"
                    | "union"
                    | "enum"
                    | "type"
                    | "trait"
                    | "const"
                    | "static"
                    | "mod"
                    | "impl"
            ) && !(value == "const" && word(words, cursor + 1) == "fn")
            {
                owners.insert(start, cursor);
                break;
            }
            match value {
                "pub" => {
                    cursor += 1;
                    if word(words, cursor) == "(" {
                        let mut depth = 1;
                        cursor += 1;
                        while cursor < words.len() && depth > 0 {
                            match word(words, cursor) {
                                "(" => depth += 1,
                                ")" => depth -= 1,
                                _ => {},
                            }
                            cursor += 1;
                        }
                    }
                },
                "async" | "unsafe" | "default" | "auto" | "const" => cursor += 1,
                "extern" => {
                    cursor += 1;
                    if word(words, cursor).starts_with('"') {
                        cursor += 1;
                    }
                },
                _ => break,
            }
        }
    }
    owners
}

/// All attribute words grouped by the item keyword they decorate.
pub(crate) fn decorated_items(words: &[String]) -> BTreeMap<usize, Vec<String>> {
    let owners = attribute_owners(words);
    let mut items: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (start, end) in attribute_spans(words) {
        if let Some(&owner) = owners.get(&start) {
            items
                .entry(owner)
                .or_default()
                .extend_from_slice(&words[start..end]);
        }
    }
    items
}

/// The word indexes where an item keyword can stand.
pub(crate) fn item_starts(words: &[String]) -> BTreeSet<usize> {
    let mut starts = BTreeSet::from([0]);
    for (index, value) in words.iter().enumerate() {
        if matches!(
            value.as_str(),
            "{" | "}" | ";" | "pub" | "unsafe" | "async" | "const" | "extern" | "default"
        ) && (index == 0 || word(words, index - 1) != "'")
        {
            starts.insert(index + 1);
        }
        if value == "pub" && word(words, index + 1) == "(" && index + 1 < words.len() {
            starts.insert(closing(words, index + 1) + 1);
        }
        if index > 0 && word(words, index - 1) == "extern" && value.starts_with('"') {
            starts.insert(index + 1);
        }
    }
    starts.extend(attribute_spans(words).into_iter().map(|(_, end)| end));
    starts
}

/// The matched delimiter pairs, following the Python stack rules.
fn delimiter_ends(words: &[String]) -> BTreeMap<usize, usize> {
    let mut stack = Vec::new();
    let mut ends = BTreeMap::new();
    for (index, value) in words.iter().enumerate() {
        match value.as_str() {
            "{" | "(" | "[" => stack.push(index),
            "}" | ")" | "]" => {
                let opener = match value.as_str() {
                    "}" => "{",
                    ")" => "(",
                    _ => "[",
                };
                if stack
                    .last()
                    .is_some_and(|&start| word(words, start) == opener)
                    && let Some(start) = stack.pop()
                {
                    ends.insert(start, index);
                }
            },
            _ => {},
        }
    }
    ends
}

/// Each macro tree, from its bang to the word after its closer.
fn macro_spans(words: &[String], ends: &BTreeMap<usize, usize>) -> Vec<(usize, usize)> {
    words
        .iter()
        .enumerate()
        .filter(|(_, value)| value.as_str() == "!")
        .filter_map(|(index, _)| {
            let begin = index
                + if index > 0 && word(words, index - 1) == "macro_rules" {
                    2
                } else {
                    1
                };
            ends.get(&begin).map(|&end| (index, end + 1))
        })
        .collect()
}

/// Item starts outside macro token trees.
pub(crate) fn code_item_starts(words: &[String]) -> BTreeSet<usize> {
    let in_macros: BTreeSet<_> = macro_spans(words, &delimiter_ends(words))
        .into_iter()
        .flat_map(|(start, end)| start + 1..end)
        .collect();
    item_starts(words).difference(&in_macros).copied().collect()
}

/// The first body brace after a name, or the word count for a declaration.
fn body_start(words: &[String], keyword: usize) -> usize {
    (keyword + 2..words.len())
        .find(|&index| matches!(word(words, index), "{" | ";"))
        .filter(|&index| word(words, index) == "{")
        .unwrap_or(words.len())
}

/// Inline modules in source order, with outer modules before their children.
pub(crate) fn inline_modules(words: &[String], starts: &BTreeSet<usize>) -> Vec<InlineModule> {
    words
        .iter()
        .enumerate()
        .take(words.len().saturating_sub(1))
        .filter(|(index, value)| value.as_str() == "mod" && starts.contains(index))
        .filter_map(|(index, _)| {
            let start = body_start(words, index);
            (start < words.len()).then(|| InlineModule {
                name: words[index + 1].clone(),
                start,
                end: closing(words, start),
            })
        })
        .collect()
}

/// Inline module bodies that hold the word, in outermost-first order.
pub(crate) fn enclosing(modules: &[InlineModule], index: usize) -> Vec<InlineModule> {
    modules
        .iter()
        .filter(|module| module.start < index && index < module.end)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::attribute_owners;
    use super::code_item_starts;
    use super::decorated_items;
    use super::enclosing;
    use super::inline_modules;
    use super::plain_words;
    use crate::tokenizer;

    #[test]
    fn attributes_and_visibility_attach_to_the_next_item() {
        for source in [
            "#[a] #[b] pub(crate) const fn f() {}",
            "#[a] #[b] extern \"C\" fn f() {}",
        ] {
            let words = plain_words(&tokenizer::tokenize(&source.chars().collect::<Vec<_>>()));
            let owners = attribute_owners(&words);
            let owner = words.iter().position(|word| word == "fn");
            assert_eq!(
                owners.values().copied().collect::<Vec<_>>(),
                owner.into_iter().cycle().take(2).collect::<Vec<_>>()
            );
            assert_eq!(decorated_items(&words).len(), 1);
        }
    }

    #[test]
    fn module_scopes_exclude_macro_trees_and_keep_raw_names_plain() {
        let source: Vec<char> = "mod r#outer { make! { mod fake {} } mod inner { mod leaf; } }"
            .chars()
            .collect();
        let words = plain_words(&tokenizer::tokenize(&source));
        let modules = inline_modules(&words, &code_item_starts(&words));
        assert_eq!(
            modules
                .iter()
                .map(|module| module.name.as_str())
                .collect::<Vec<_>>(),
            ["outer", "inner"]
        );
        let leaf = words
            .iter()
            .position(|word| word == "leaf")
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(
            leaf.iter()
                .flat_map(|&index| enclosing(&modules, index))
                .map(|module| module.name)
                .collect::<Vec<_>>(),
            ["outer", "inner"]
        );
    }
}
