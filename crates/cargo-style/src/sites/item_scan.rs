//! Read the line-oriented Rust item shape used by the Python generators.

use regex::Regex;

use super::constants::VISIBILITY_PATTERN;
use super::failure::SitesFailure;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

pub(super) struct ItemScanner {
    structures:   Regex,
    enumerations: Regex,
    fields:       Regex,
    variants:     Regex,
    tokens:       Regex,
    type_idents:  Regex,
}

impl ItemScanner {
    pub(super) fn new() -> Result<Self, SitesFailure> {
        let space = class_ranges(SPACE);
        let word = class_ranges(WORD);
        let vis = visibility(&space);
        let compile = |pattern: String| Regex::new(&pattern).map_err(SitesFailure::ItemPattern);
        Ok(Self {
            structures:   compile(format!(
                r"^[{space}]*{vis}struct[{space}]+([A-Za-z_][{word}]*)"
            ))?,
            enumerations: compile(format!(
                r"^[{space}]*{vis}enum[{space}]+([A-Za-z_][{word}]*)"
            ))?,
            fields:       compile(format!(
                r"^[{space}]*{vis}(?:r#)?([a-z_][{word}]*)[{space}]*:[{space}]*(.+?),?[{space}]*$",
            ))?,
            variants:     compile(format!(
                r"^[{space}]*([A-Z][{word}]*)[{space}]*(?:[,({{=]|$)"
            ))?,
            tokens:       compile(format!(r"(?:r#)?[A-Za-z_][{word}]*|[^{space}]"))?,
            type_idents:  compile(format!(r"[A-Za-z_][{word}]*"))?,
        })
    }

    pub(super) fn structs<'a>(&self, lines: &[&'a str]) -> Vec<StructDef<'a>> {
        self.named_bodies(lines, &self.structures)
            .map(|(name, declaration, body)| {
                let mut fields = Vec::new();
                body.members(lines, |line, text| {
                    if let Some(captures) = self.fields.captures(text)
                        && let Some(name) = captures.get(1)
                        && let Some(field_type) = captures.get(2)
                    {
                        fields.push(FieldDef {
                            name: name.as_str(),
                            line,
                            type_text: field_type.as_str().trim_matches(tokenizer::python_space),
                        });
                    }
                });
                StructDef {
                    name,
                    line: declaration.line + 1,
                    fields,
                }
            })
            .collect()
    }

    pub(super) fn enums<'a>(&self, lines: &[&'a str]) -> Vec<EnumDef<'a>> {
        self.named_bodies(lines, &self.enumerations)
            .map(|(name, _, body)| {
                let mut variants = Vec::new();
                body.members(lines, |line, text| {
                    if let Some(captures) = self.variants.captures(text)
                        && let Some(name) = captures.get(1)
                    {
                        variants.push(VariantDef {
                            name: name.as_str(),
                            line,
                        });
                    }
                });
                EnumDef { name, variants }
            })
            .collect()
    }

    pub(super) fn type_idents<'a>(&self, text: &'a str) -> impl Iterator<Item = &'a str> {
        self.type_idents.find_iter(text).map(|ident| ident.as_str())
    }

    fn named_bodies<'a>(
        &self,
        lines: &[&'a str],
        declaration: &Regex,
    ) -> impl Iterator<Item = (&'a str, Position, ItemBody)> {
        lines.iter().enumerate().filter_map(|(line, text)| {
            let name = declaration.captures(text)?.get(1)?;
            let declaration = Position {
                line,
                column: name.end(),
            };
            let body = self.find_item_body(lines, declaration)?;
            Some((name.as_str(), declaration, body))
        })
    }

    fn find_item_body(&self, lines: &[&str], start: Position) -> Option<ItemBody> {
        let mut stack = Vec::new();
        let mut clause = WhereClause::Absent;
        for (line, text) in lines.iter().enumerate().skip(start.line) {
            let column = start.column_on(line);
            for token in self.tokens.find_iter(&text[column..]) {
                let top = stack.last().copied();
                let token_pos = Position {
                    line,
                    column: column + token.start(),
                };
                match token.as_str() {
                    "{" if top == Some("{") => stack.push("{"),
                    "}" if top == Some("{") => {
                        let _ = stack.pop();
                    },
                    _ if top == Some("{") => {},
                    "<" | ">" if matches!(top, Some("(" | "[")) => {},
                    ">" if text[..token_pos.column].ends_with(['-', '=']) => {},
                    closer @ (")" | "]" | "}" | ">") => {
                        if stack.pop() != opener(closer) {
                            return None;
                        }
                    },
                    "(" | "[" | "<" => stack.push(token.as_str()),
                    "{" if top.is_some() => stack.push("{"),
                    "{" => return self.body(lines, token_pos),
                    ";" if top == Some("<") => return None,
                    "where" if top.is_none() => clause = WhereClause::Present,
                    ";" | "=" | "use" if top.is_none() => return None,
                    "," if top.is_none() && matches!(clause, WhereClause::Absent) => return None,
                    _ => {},
                }
            }
        }
        None
    }

    pub(super) fn body_after(&self, lines: &[&str], start: Position) -> FollowingBody {
        self.find_item_body(lines, start)
            .map_or(FollowingBody::Absent, |body| {
                FollowingBody::Present(body.close)
            })
    }

    fn body(&self, lines: &[&str], open: Position) -> Option<ItemBody> {
        let close = find_body_close(lines, open, |_, _| {});
        if let BodyClose::ClosedAt(close) = close
            && matches!(
                self.next_token(
                    lines,
                    Position {
                        column: close.column + 1,
                        ..close
                    }
                ),
                Some("=" | "|" | "if"),
            )
        {
            return None;
        }
        Some(ItemBody { open, close })
    }

    fn next_token<'a>(&self, lines: &[&'a str], start: Position) -> Option<&'a str> {
        lines
            .iter()
            .enumerate()
            .skip(start.line)
            .find_map(|(line, text)| {
                self.tokens
                    .find(&text[start.column_on(line)..])
                    .map(|token| token.as_str())
            })
    }
}

pub(super) struct StructDef<'a> {
    pub(super) name:   &'a str,
    pub(super) line:   usize,
    pub(super) fields: Vec<FieldDef<'a>>,
}

pub(super) struct FieldDef<'a> {
    pub(super) name:      &'a str,
    pub(super) line:      usize,
    pub(super) type_text: &'a str,
}

pub(super) struct EnumDef<'a> {
    pub(super) name:     &'a str,
    pub(super) variants: Vec<VariantDef<'a>>,
}

pub(super) struct VariantDef<'a> {
    pub(super) name: &'a str,
    pub(super) line: usize,
}

#[derive(Clone, Copy)]
pub(super) struct Position {
    pub(super) line:   usize,
    pub(super) column: usize,
}

impl Position {
    const fn column_on(self, line: usize) -> usize {
        if line == self.line { self.column } else { 0 }
    }
}

enum WhereClause {
    Absent,
    Present,
}

struct ItemBody {
    open:  Position,
    close: BodyClose,
}

impl ItemBody {
    fn members<'a>(self, lines: &[&'a str], member: impl FnMut(usize, &'a str)) {
        let _ = find_body_close(lines, self.open, member);
    }
}

fn opener(closer: &str) -> Option<&str> {
    match closer {
        ")" => Some("("),
        "]" => Some("["),
        "}" => Some("{"),
        ">" => Some("<"),
        _ => None,
    }
}

pub(super) enum FollowingBody {
    Absent,
    Present(BodyClose),
}

#[derive(Clone, Copy)]
pub(super) enum BodyClose {
    ClosedAt(Position),
    Unclosed,
}

fn find_body_close<'a>(
    lines: &[&'a str],
    open: Position,
    mut member: impl FnMut(usize, &'a str),
) -> BodyClose {
    let mut depth = 0;
    for (line, text) in lines.iter().enumerate().skip(open.line) {
        if depth == 1 && line != open.line {
            member(line + 1, text);
        }
        let start = open.column_on(line);
        for (offset, byte) in text[start..].bytes().enumerate() {
            match byte {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        return BodyClose::ClosedAt(Position {
                            line,
                            column: start + offset,
                        });
                    }
                },
                _ => {},
            }
        }
    }
    BodyClose::Unclosed
}

pub(super) fn class_ranges(ranges: &[(u32, u32)]) -> String {
    ranges
        .iter()
        .map(|(first, last)| {
            if first == last {
                format!(r"\x{{{first:x}}}")
            } else {
                format!(r"\x{{{first:x}}}-\x{{{last:x}}}")
            }
        })
        .collect()
}

pub(super) fn visibility(space: &str) -> String {
    VISIBILITY_PATTERN.replace(r"\s", &format!("[{space}]"))
}

pub(super) fn camel_words(name: &str) -> impl Iterator<Item = String> {
    let mut remaining = name;
    std::iter::from_fn(move || {
        let start = remaining.find(|character: char| character.is_ascii_alphanumeric())?;
        remaining = &remaining[start..];
        let bytes = remaining.as_bytes();
        let upper = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_uppercase())
            .count();
        let end = match upper {
            0 => lowercase_run(bytes),
            1 if bytes.get(1).is_some_and(u8::is_ascii_lowercase) => 1 + lowercase_run(&bytes[1..]),
            _ if bytes.get(upper).is_some_and(u8::is_ascii_lowercase) => upper - 1,
            _ => upper,
        };
        let (word, tail) = remaining.split_at(end);
        remaining = tail;
        Some(word.to_ascii_lowercase())
    })
}

fn lowercase_run(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .take_while(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        .count()
}

pub(super) fn snake_case(name: &str) -> String { camel_words(name).collect::<Vec<_>>().join("_") }
