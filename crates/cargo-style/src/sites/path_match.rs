//! Match catalog and committed ignore paths with shell patterns.

use regex::Error;
use regex::Regex;

pub(super) struct PathPattern {
    regex: Regex,
}

impl PathPattern {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let mut expression = String::from("(?s)\\A");
        let mut characters = pattern.chars();
        while let Some(character) = characters.next() {
            match character {
                '*' => expression.push_str(".*"),
                '?' => expression.push('.'),
                '[' => {
                    if let Some(length) = class_length(characters.clone()) {
                        let members = characters.by_ref().take(length).collect::<Vec<_>>();
                        expression.push_str(&class_expression(&members));
                        let _ = characters.next();
                    } else {
                        expression.push_str("\\[");
                    }
                },
                literal => expression.push_str(&regex::escape(literal.encode_utf8(&mut [0; 4]))),
            }
        }
        expression.push_str("\\z");
        Regex::new(&expression).map(|regex| Self { regex })
    }

    pub(super) fn matches(&self, path: &str) -> bool { self.regex.is_match(path) }
}

fn class_length(mut rest: impl Iterator<Item = char> + Clone) -> Option<usize> {
    let prefix = usize::from(rest.clone().next() == Some('!'));
    if prefix != 0 {
        let _ = rest.next();
    }
    let bracket = usize::from(rest.clone().next() == Some(']'));
    rest.skip(bracket)
        .position(|character| character == ']')
        .map(|length| prefix + bracket + length)
}

/// Translate one bracket class as Python `fnmatch` does. It drops an empty range such as `z-a`.
/// A leading `!` stays in the first chunk, so an empty range before a later `!` exposes it as a
/// negation. A class left with no members never matches; a lone `!` matches any character.
fn class_expression(members: &[char]) -> String {
    let class = range_chunks(members)
        .iter()
        .map(|chunk| {
            chunk.iter().fold(String::new(), |mut text, &member| {
                if matches!(member, '[' | ']' | '^' | '\\' | '-' | '&' | '|' | '~') {
                    text.push('\\');
                }
                text.push(member);
                text
            })
        })
        .collect::<Vec<_>>()
        .join("-");
    match class.strip_prefix('!') {
        // Python writes `(?!)`. A Rust class with no members also never matches.
        _ if class.is_empty() => r"[^\x00-\x{10FFFF}]".to_owned(),
        Some("") => ".".to_owned(),
        Some(negated) => format!("[^{negated}]"),
        None => format!("[{class}]"),
    }
}

/// Split a class at its range hyphens as Python `fnmatch` does; a leading `!` is not a range
/// end. Then remove each empty range, such as `z-a`, with both of its end characters. The chunks
/// join with `-` as ranges.
fn range_chunks(body: &[char]) -> Vec<Vec<char>> {
    let mut chunks: Vec<Vec<char>> = Vec::new();
    let mut start = 0;
    let mut search = 1 + usize::from(body.first() == Some(&'!'));
    while let Some(offset) = body
        .get(search..)
        .and_then(|rest| rest.iter().position(|&member| member == '-'))
    {
        let hyphen = search + offset;
        chunks.push(body[start..hyphen].to_vec());
        start = hyphen + 1;
        search = hyphen + 3;
    }
    let last = body.get(start..).unwrap_or_default();
    if last.is_empty()
        && let Some(previous) = chunks.last_mut()
    {
        // A trailing hyphen is a literal member.
        previous.push('-');
    } else {
        chunks.push(last.to_vec());
    }
    for index in (1..chunks.len()).rev() {
        if chunks[index - 1].last() > chunks[index].first() {
            let next = chunks.remove(index);
            let previous = &mut chunks[index - 1];
            let _ = previous.pop();
            previous.extend(next.into_iter().skip(1));
        }
    }
    chunks
}

pub(super) struct WorkspacePattern {
    parts: Vec<WorkspacePart>,
}

enum WorkspacePart {
    AnyDepth,
    Component(PathPattern),
}

impl WorkspacePattern {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let parts = path_parts(pattern)
            .map(|part| match part {
                "**" => Ok(WorkspacePart::AnyDepth),
                part => PathPattern::new(part).map(WorkspacePart::Component),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { parts })
    }

    pub(super) fn matches(&self, directory: &str) -> bool {
        fn matches(parts: &[&str], patterns: &[WorkspacePart]) -> bool {
            match (parts, patterns) {
                (_, []) => parts.is_empty(),
                (_, [WorkspacePart::AnyDepth, rest @ ..]) => {
                    matches(parts, rest)
                        || parts
                            .split_first()
                            .is_some_and(|(_, tail)| matches(tail, patterns))
                },
                ([part, tail @ ..], [WorkspacePart::Component(pattern), rest @ ..]) => {
                    pattern.matches(part) && matches(tail, rest)
                },
                _ => false,
            }
        }
        matches(&path_parts(directory).collect::<Vec<_>>(), &self.parts)
    }
}

pub(super) fn path_parts(path: &str) -> impl Iterator<Item = &str> {
    path.split('/').filter(|part| !matches!(*part, "" | "."))
}
