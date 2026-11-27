//! Match catalog paths, committed ignore paths and generator exemptions.

use regex::Error;
use regex::Regex;

use crate::tip_view;

pub(super) struct PathPattern {
    regex: Regex,
}

impl PathPattern {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let mut expression = String::from("(?s)\\A");
        push_fnmatch(&mut expression, pattern, ".*", ".");
        expression.push_str("\\z");
        Regex::new(&expression).map(|regex| Self { regex })
    }

    pub(super) fn matches(&self, path: &str) -> bool { self.regex.is_match(path) }
}

pub(super) struct WorkspaceGlob(Regex);

impl WorkspaceGlob {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let mut expression = String::from("(?s)\\A");
        expression.push_str(posix_anchor(pattern));
        let mut parts = tip_view::path_parts(pattern).peekable();
        while let Some(part) = parts.next() {
            match (part, parts.peek()) {
                ("*", None) => expression.push_str("[^/]+"),
                ("*", Some(_)) => expression.push_str("[^/]+/"),
                ("**", None) => expression.push_str(".*"),
                ("**", Some(&"**")) => {},
                ("**", Some(_)) => expression.push_str("(?:.+/)?"),
                (part, next) => {
                    push_fnmatch(&mut expression, part, "[^/]*", "[^/]");
                    if next.is_some() {
                        expression.push('/');
                    }
                },
            }
        }
        expression.push_str("\\z");
        Regex::new(&expression).map(Self)
    }

    pub(super) fn matches(&self, path: &str) -> bool {
        self.0.is_match(if path == "." { "" } else { path })
    }
}

pub(super) fn posix_anchor(path: &str) -> &'static str {
    if path.starts_with("//") && !path.starts_with("///") {
        "//"
    } else if path.starts_with('/') {
        "/"
    } else {
        ""
    }
}

fn push_fnmatch(expression: &mut String, pattern: &str, star: &str, question: &str) {
    let mut characters = pattern.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '*' => {
                while characters.next_if_eq(&'*').is_some() {}
                expression.push_str(star);
            },
            '?' => expression.push_str(question),
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
}

pub(super) fn is_exempt(path: &str, exemptions: &[String]) -> bool {
    let rooted = format!("/{path}");
    exemptions
        .iter()
        .map(|entry| entry.trim_matches('/'))
        .any(|entry| {
            !entry.is_empty()
                && (path == entry
                    || rooted.ends_with(&format!("/{entry}"))
                    || rooted.contains(&format!("/{entry}/")))
        })
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

impl WorkspacePattern {
    pub(super) fn new(pattern: &str) -> Result<Self, Error> {
        let parts = tip_view::path_parts(pattern)
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
                    // WorkspacePart::AnyDepth can use zero path components.
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
        matches(
            &tip_view::path_parts(directory).collect::<Vec<_>>(),
            &self.parts,
        )
    }
}

enum WorkspacePart {
    AnyDepth,
    Component(PathPattern),
}
