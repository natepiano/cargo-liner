//! Read the rule catalog's closed frontmatter grammar.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

use crate::source_lines;

pub(super) type Fields = BTreeMap<String, Field>;

pub(super) enum Field {
    Scalar(String),
    List(Vec<String>),
    Candidates(Fields),
}

enum Block {
    Closed,
    Open { key: String, indent: Indentation },
}

enum Indentation {
    FirstEntry,
    Set(usize),
}

pub(super) fn parse(text: &str) -> Result<Fields, String> {
    let mut lines = source_lines::split_lines(text).map(without_line_ending);
    let first = lines.next().unwrap_or_default();
    if first.starts_with('\u{feff}') || (first.trim() == "---" && first != "---") {
        return Err("invalid frontmatter opening delimiter or BOM".to_owned());
    }
    let mut fields = Fields::new();
    if first != "---" {
        return Ok(fields);
    }
    let mut block = Block::Closed;
    for (index, line) in lines.enumerate() {
        if line == "---" {
            return Ok(fields);
        }
        let result = read_line(line, &mut fields, &mut block);
        if let Err(reason) = result {
            return Err(format!("{}: {reason}", index + 2));
        }
    }
    Err("unterminated frontmatter".to_owned())
}

pub(super) fn without_line_ending(line: &str) -> &str {
    line.trim_end_matches([
        '\r', '\n', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}',
        '\u{2029}',
    ])
}

fn read_line(line: &str, fields: &mut Fields, block: &mut Block) -> Result<(), String> {
    let stripped = line.trim();
    let indent = line.len() - line.trim_start().len();
    if line.contains('\u{feff}') {
        return Err("BOM in frontmatter".to_owned());
    }
    if line[..indent].contains('\t') {
        return Err("tab indentation in frontmatter".to_owned());
    }
    if stripped == "---" {
        return Err("indented or padded frontmatter delimiter".to_owned());
    }
    if stripped.is_empty() || stripped.starts_with('#') {
        return Ok(());
    }
    if indent == 0
        && let Some((key, raw)) = entry(stripped)
    {
        *block = Block::Closed;
        let vacant = match fields.entry(key.to_owned()) {
            Entry::Vacant(vacant) => vacant,
            Entry::Occupied(_) => return Err(format!("duplicate frontmatter key: {key}")),
        };
        if raw.is_empty() || raw.starts_with('#') {
            let _ = vacant.insert(if key == "candidates" {
                Field::Candidates(Fields::new())
            } else {
                Field::List(Vec::new())
            });
            *block = Block::Open {
                key:    key.to_owned(),
                indent: Indentation::FirstEntry,
            };
        } else {
            let _ = vacant.insert(value(raw)?);
        }
        return Ok(());
    }
    let Block::Open {
        key,
        indent: previous,
    } = block
    else {
        return Err("line outside a frontmatter block or invalid indentation".to_owned());
    };
    match previous {
        Indentation::FirstEntry => *previous = Indentation::Set(indent),
        Indentation::Set(expected) if *expected != indent => {
            return Err("inconsistent frontmatter block indentation".to_owned());
        },
        Indentation::Set(_) => {},
    }
    match fields.get_mut(key) {
        Some(Field::Candidates(candidates)) if indent > 0 => {
            let (key, raw) =
                entry(stripped).ok_or("unsupported frontmatter block entry or indentation")?;
            match candidates.entry(key.to_owned()) {
                Entry::Vacant(vacant) => {
                    let _ = vacant.insert(value(raw)?);
                },
                Entry::Occupied(_) => return Err(format!("duplicate candidates key: {key}")),
            }
        },
        Some(Field::List(items)) => items.push(scalar(
            stripped
                .strip_prefix("- ")
                .ok_or("unsupported frontmatter block entry or indentation")?,
        )?),
        _ => return Err("unsupported frontmatter block entry or indentation".to_owned()),
    }
    Ok(())
}

fn entry(line: &str) -> Option<(&str, &str)> {
    let (key, raw) = line.split_once(':')?;
    let first = *key.as_bytes().first()?;
    let valid = (first.is_ascii_alphabetic() || first == b'_')
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
        && (raw.is_empty() || raw.starts_with([' ', '\t']));
    valid.then_some((key, raw.trim()))
}

fn comment(text: &str) -> &str {
    text.match_indices('#')
        .find(|(index, _)| text[..*index].ends_with([' ', '\t']))
        .map_or(text, |(index, _)| &text[..index])
}

fn scalar(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    let Some(first) = raw.chars().next() else {
        return Err("missing frontmatter scalar".to_owned());
    };
    if matches!(first, '\'' | '"') {
        let (text, tail) = quoted(raw, first)?;
        if !tail.trim().is_empty() && !comment(tail).trim().is_empty() {
            return Err("unexpected text after frontmatter quote".to_owned());
        }
        return Ok(text);
    }
    let plain = comment(raw).trim_end();
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) || plain.contains(": ") || plain.contains('\t') {
        return Err("invalid frontmatter plain scalar".to_owned());
    }
    Ok(plain.to_owned())
}

fn quoted(raw: &str, quote: char) -> Result<(String, &str), String> {
    let mut characters = raw.char_indices().skip(1).peekable();
    while let Some((index, character)) = characters.next() {
        if quote == '"' && character == '\\' {
            let _ = characters.next();
        } else if character == quote {
            if quote == '\'' && characters.peek().is_some_and(|&(_, next)| next == '\'') {
                let _ = characters.next();
                continue;
            }
            let end = index + 1;
            let value = if quote == '"' {
                serde_json::from_str(&raw[..end]).map_err(|error| error.to_string())?
            } else {
                raw[1..index].replace("''", "'")
            };
            return Ok((value, &raw[end..]));
        }
    }
    Err("unclosed frontmatter quote".to_owned())
}

fn value(raw: &str) -> Result<Field, String> {
    let Some(mut tail) = raw.strip_prefix('[').map(str::trim_start) else {
        return scalar(raw).map(Field::Scalar);
    };
    let mut items = Vec::new();
    while !tail.starts_with(']') {
        let first = tail
            .chars()
            .next()
            .filter(|&character| character != '#')
            .ok_or("unterminated frontmatter inline list")?;
        let (item, rest) = if matches!(first, '\'' | '"') {
            quoted(tail, first)?
        } else {
            let end = tail.find([',', ']']).unwrap_or(tail.len());
            if end == 0 {
                return Err("missing frontmatter list item".to_owned());
            }
            let raw_item = &tail[..end];
            if raw_item.contains('[') || comment(raw_item) != raw_item {
                return Err("invalid or unterminated frontmatter inline list".to_owned());
            }
            (scalar(raw_item)?, &tail[end..])
        };
        items.push(item);
        tail = rest.trim_start();
        if let Some(rest) = tail.strip_prefix(',') {
            tail = rest.trim_start();
            if tail.starts_with(']') {
                return Err("missing frontmatter list item".to_owned());
            }
        } else if !tail.starts_with(']') {
            return Err("unterminated frontmatter inline list".to_owned());
        }
    }
    let tail = &tail[1..];
    if !tail.trim().is_empty() && !comment(tail).trim().is_empty() {
        return Err("unexpected text after frontmatter inline list".to_owned());
    }
    Ok(Field::List(items))
}
