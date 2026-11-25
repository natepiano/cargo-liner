//! Read policy TOML and report syntax errors in the form used by Python.

mod compatibility;

use toml::Table;
use toml::Value;
use toml::de::Error as TomlError;
use toml_parser::ParseError;
use toml_parser::Source;
use toml_parser::parser;
use toml_parser::parser::Event;
use toml_parser::parser::EventKind;
use toml_parser::parser::RecursionGuard;

use super::outcome;
use super::policy::PolicyError;

const RECURSION_LIMIT: u32 = 80;

pub(super) fn parse(text: &str) -> Result<Table, PolicyError> {
    let mut events = Vec::new();
    let mut receiver = RecursionGuard::new(&mut events, RECURSION_LIMIT);
    let mut syntax = None;
    let tokens = Source::new(text).lex().into_vec();
    parser::parse_document(&tokens, &mut receiver, &mut syntax);
    let issue = compatibility::first_error(text, &events);
    if let Some(issue) = issue
        && syntax
            .as_ref()
            .and_then(ParseError::unexpected)
            .is_none_or(|span| issue.position <= span.start())
    {
        return Err(syntax_at(text, issue.position, issue.reason));
    }
    text.parse()
        .map_err(|error| syntax_error(text, &events, &error))
}

fn syntax_error(text: &str, events: &[Event], error: &TomlError) -> PolicyError {
    let start = error.span().map_or(text.len(), |range| range.start);
    let start = text.floor_char_boundary(start.min(text.len()));
    let prefix = &text[..start];
    let message = error
        .message()
        .split(", expected ")
        .next()
        .unwrap_or_default();
    let next = text[start..].chars().next();
    if matches!(
        message,
        "invalid literal string"
            | "invalid basic string"
            | "invalid multi-line literal string"
            | "invalid multi-line basic string"
    ) {
        if let Some(character) = next.filter(char::is_ascii_control) {
            return syntax_at(
                text,
                start,
                &format!(
                    "Illegal character {}",
                    outcome::python_repr(&character.to_string())
                ),
            );
        }
        return syntax_at(text, text.len(), "Unterminated string");
    }
    let (reason, position) = match message {
        "unclosed array" => {
            let reason = if prefix.trim_end().ends_with(['[', ',']) {
                "Invalid value"
            } else {
                "Unclosed array"
            };
            (reason, text.len())
        },
        "missing comma between array elements" => ("Unclosed array", start),
        "unclosed inline table" | "missing comma between key-value pairs" => {
            ("Unclosed inline table", skip_toml_space(text, start))
        },
        "extra comma in array" | "unexpected `=` in array" => ("Invalid value", start),
        "extra comma in inline table" | "missing key for inline table element" => {
            ("Invalid initial character for a key part", start)
        },
        "invalid key-value pair" | "missing assignment between key-value pairs" => (
            "Expected '=' after a key in a key/value pair",
            skip_toml_space(text, start),
        ),
        "unused string delimiter" | "unclosed string" | "unterminated string" => {
            ("Unterminated string", text.len())
        },
        "missing escaped value" => (
            "Unescaped '\\' in a string",
            start + next.map_or(0, char::len_utf8),
        ),
        "invalid value" | "missing value" | "missing value for key" | "key with no value" => {
            ("Invalid value", skip_toml_space(text, start))
        },
        "duplicate key" => {
            if let Some(error) = duplicate_header(text, events, start) {
                return error;
            }
            let end = text[start..]
                .find(['\n', '#'])
                .map_or(text.len(), |end| start + end);
            let value_end = text[..end].trim_end_matches([' ', '\t']).len();
            ("Cannot overwrite a value", skip_toml_space(text, value_end))
        },
        "unclosed table" => ("Expected ']' at the end of a table declaration", start),
        "unclosed array table" => ("Expected ']]' at the end of an array declaration", start),
        "invalid unquoted key"
            if prefix
                .rsplit('\n')
                .next()
                .is_none_or(|line| line.trim().is_empty()) =>
        {
            ("Invalid statement", start)
        },
        _ if prefix.trim().is_empty() => ("Invalid statement", start),
        _ => (
            "Expected newline or end of document after a statement",
            start,
        ),
    };
    syntax_at(text, position, reason)
}

fn duplicate_header(text: &str, events: &[Event], start: usize) -> Option<PolicyError> {
    let line_start = text[..start].rfind('\n').map_or(0, |newline| newline + 1);
    let open = events
        .iter()
        .find(|event| {
            event.kind() == EventKind::StdTableOpen
                && (line_start..=start).contains(&event.span().start())
        })?
        .span()
        .start();
    let close = events
        .iter()
        .find(|event| event.kind() == EventKind::StdTableClose && event.span().start() >= start)?
        .span()
        .start();
    let header = text[open..=close].parse::<Table>().ok()?;
    let mut previous = text[..open].parse::<Table>().ok()?;
    let mut keys = Vec::new();
    let mut table = &header;
    while let Some((key, Value::Table(nested))) = table.iter().next() {
        keys.push(outcome::python_repr(key));
        match previous.remove(key)? {
            Value::Table(value) => previous = value,
            Value::Array(mut values) if !nested.is_empty() => {
                let Some(Value::Table(value)) = values.pop() else {
                    return Some(syntax_at(text, close, "Cannot overwrite a value"));
                };
                previous = value;
            },
            Value::Array(_) => break,
            _ => return Some(syntax_at(text, close, "Cannot overwrite a value")),
        }
        table = nested;
    }
    let comma = if keys.len() == 1 { "," } else { "" };
    let reason = format!("Cannot declare ({}{comma}) twice", keys.join(", "));
    Some(syntax_at(text, close, &reason))
}

fn syntax_at(text: &str, position: usize, reason: &str) -> PolicyError {
    let location = if position >= text.len() {
        "at end of document".to_owned()
    } else {
        let prefix = &text[..position];
        let line = prefix.matches('\n').count() + 1;
        let column = prefix
            .rsplit('\n')
            .next()
            .map_or(1, |line| line.chars().count() + 1);
        format!("at line {line}, column {column}")
    };
    PolicyError::Syntax(format!("{reason} ({location})"))
}

fn skip_toml_space(text: &str, start: usize) -> usize {
    let tail = &text[start..];
    text.len() - tail.trim_start_matches([' ', '\t']).len()
}
