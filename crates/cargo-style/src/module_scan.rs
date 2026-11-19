//! Module declarations and their path attributes, ported from `module_declaration_text`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use serde::Serialize;

#[cfg(test)]
use crate::tokenizer;
use crate::tokenizer::Token;
use crate::tokenizer::TokenKind;
use crate::word_syntax;

/// The module declarations of readable source, or unreadable token offsets.
#[derive(Debug, Serialize)]
pub(crate) enum ModuleScanReading {
    Value(ModuleScan),
    TokenSpansUnreadable,
}

/// A file's module declarations in the Python source order.
#[derive(Debug, Serialize)]
pub(crate) struct ModuleScan {
    declarations: Vec<ModuleDeclaration>,
}

/// A module declaration with its inline scope and path form.
#[derive(Debug, Serialize)]
struct ModuleDeclaration {
    name:  String,
    scope: Vec<String>,
    path:  ModulePath,
}

/// The path form that the Python declaration scan retains.
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
enum ModulePath {
    Literal { literal: String },
    Other(PathForm),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PathForm {
    Default,
    Uncertain,
}

/// Read module declaration facts from source characters.
#[cfg(test)]
fn read(text: &[char]) -> ModuleScanReading { read_tokens(&tokenizer::tokenize(text)) }

/// Read module declaration facts from the request's one token read.
pub(crate) fn read_tokens(tokens: &[Token]) -> ModuleScanReading {
    if tokens.iter().any(|token| token.kind == TokenKind::Unknown) {
        return ModuleScanReading::TokenSpansUnreadable;
    }
    let words = word_syntax::plain_words(tokens);
    let starts = word_syntax::code_item_starts(&words);
    let modules = word_syntax::inline_modules(&words, &starts);
    let attributes = word_syntax::decorated_items(&words);
    let spans: BTreeMap<_, _> = word_syntax::attribute_spans(&words).into_iter().collect();
    let mut paths: BTreeMap<_, _> = attributes
        .iter()
        .filter(|(owner, decorated)| {
            word_syntax::word(&words, **owner) == "mod"
                && decorated.iter().any(|value| value == "path")
        })
        .map(|(&owner, _)| (owner, ModulePath::Other(PathForm::Uncertain)))
        .collect();
    for (start, owner) in word_syntax::attribute_owners(&words) {
        let named = plain_path(&words, start, spans[&start]);
        if paths.contains_key(&owner)
            && starts.contains(&owner)
            && attributes[&owner]
                .iter()
                .filter(|value| value.as_str() == "path")
                .count()
                == 1
            && word_syntax::word(&words, owner + 2) == ";"
            && let PlainPath::Literal(literal) = named
        {
            paths.insert(owner, ModulePath::Literal { literal });
        }
    }
    let owners: BTreeSet<_> = paths
        .keys()
        .copied()
        .chain(
            words
                .iter()
                .enumerate()
                .filter(|(index, value)| {
                    value.as_str() == "mod"
                        && starts.contains(index)
                        && word_syntax::word(&words, index + 2) == ";"
                })
                .map(|(index, _)| index),
        )
        .collect();
    let declarations = owners
        .into_iter()
        .map(|owner| ModuleDeclaration {
            name:  word_syntax::word(&words, owner + 1).to_owned(),
            scope: word_syntax::enclosing(&modules, owner)
                .into_iter()
                .map(|module| module.name)
                .collect(),
            path:  paths
                .remove(&owner)
                .unwrap_or(ModulePath::Other(PathForm::Default)),
        })
        .collect();
    ModuleScanReading::Value(ModuleScan { declarations })
}

/// An attribute is a plain quoted path or has another form.
pub(crate) enum PlainPath {
    Literal(String),
    Other,
}

/// Read the exact six-word attribute form accepted by the Python scan.
pub(crate) fn plain_path(words: &[String], start: usize, end: usize) -> PlainPath {
    if word_syntax::word(words, start + 1) != "["
        || word_syntax::word(words, start + 2) != "path"
        || word_syntax::word(words, start + 3) != "="
    {
        return PlainPath::Other;
    }
    let value = word_syntax::word(words, start + 4);
    if end != start + 6 && value != "\"\"" {
        return PlainPath::Other;
    }
    match value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        Some(value) if !value.contains('\\') => PlainPath::Literal(value.to_owned()),
        _ => PlainPath::Other,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::Error;

    use super::ModuleScanReading;
    use super::read;

    #[test]
    fn module_paths_keep_the_python_order_scopes_and_uncertainty() -> Result<(), Error> {
        for (source, expected) in [
            (
                "mod outer { mod inner; } mod r#type;",
                "{\"declarations\":[{\"name\":\"inner\",\"scope\":[\"outer\"],\"path\":\"default\"},{\"name\":\"type\",\"scope\":[],\"path\":\"default\"}]}",
            ),
            (
                "#[path = \"other.rs\"] mod file;",
                "{\"declarations\":[{\"name\":\"file\",\"scope\":[],\"path\":{\"literal\":\"other.rs\"}}]}",
            ),
            (
                "#[cfg_attr(test, path = \"other.rs\")] mod file;",
                "{\"declarations\":[{\"name\":\"file\",\"scope\":[],\"path\":\"uncertain\"}]}",
            ),
            (
                "mod outer { #[path = \"other.rs\"] mod file; }",
                "{\"declarations\":[{\"name\":\"file\",\"scope\":[\"outer\"],\"path\":{\"literal\":\"other.rs\"}}]}",
            ),
            (
                "make! { mod fake; } fn f() { mod local; }",
                "{\"declarations\":[{\"name\":\"local\",\"scope\":[],\"path\":\"default\"}]}",
            ),
            (
                "#[path = \"a\\x.rs\"] mod file;",
                "{\"declarations\":[{\"name\":\"file\",\"scope\":[],\"path\":\"uncertain\"}]}",
            ),
        ] {
            let reading = read(&source.chars().collect::<Vec<_>>());
            let ModuleScanReading::Value(value) = reading else {
                assert!(matches!(reading, ModuleScanReading::Value(_)), "{source}");
                return Ok(());
            };
            assert_eq!(
                serde_json::to_value(value)?,
                serde_json::from_str::<serde_json::Value>(expected)?,
                "{source}"
            );
        }
        Ok(())
    }

    #[test]
    fn unknown_tokens_make_module_declarations_unreadable() {
        for source in ["mod a; `", "mod a; /* unfinished"] {
            assert!(matches!(
                read(&source.chars().collect::<Vec<_>>()),
                ModuleScanReading::TokenSpansUnreadable
            ));
        }
    }
}
