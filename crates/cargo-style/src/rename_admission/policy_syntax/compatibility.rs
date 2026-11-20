//! Reject TOML syntax that Python 3.13 does not accept.

use toml::value::Datetime;
use toml_parser::decoder::Encoding;
use toml_parser::parser::Event;
use toml_parser::parser::EventKind;

const KEY_EXPECTED: &str = "Invalid initial character for a key part";
const INLINE_TABLE_UNCLOSED: &str = "Unclosed inline table";

pub(super) struct SyntaxIssue {
    pub(super) position: usize,
    pub(super) reason:   &'static str,
}

enum Container {
    Array,
    Inline(InlinePosition),
}

enum InlinePosition {
    Open,
    Key,
    Assignment,
    Value,
    Separator,
}

pub(super) fn first_error(text: &str, events: &[Event]) -> Option<SyntaxIssue> {
    if text.starts_with('\u{feff}') {
        return Some(SyntaxIssue {
            position: 0,
            reason:   "Invalid statement",
        });
    }

    let mut containers = Vec::new();
    for event in events {
        let start = event.span().start();
        let end = event.span().end();
        let issue = match event.kind() {
            EventKind::SimpleKey | EventKind::Scalar => {
                let issue = text.get(start..end).and_then(|raw| match event.encoding() {
                    Some(Encoding::BasicString | Encoding::MlBasicString) => {
                        unsupported_escape(raw).map(|position| SyntaxIssue {
                            position: start + position,
                            reason:   "Unescaped '\\' in a string",
                        })
                    },
                    None if event.kind() == EventKind::Scalar => {
                        invalid_datetime(raw, start, containers.last())
                    },
                    _ => None,
                });
                let position = if event.kind() == EventKind::SimpleKey {
                    InlinePosition::Assignment
                } else {
                    InlinePosition::Separator
                };
                inline_position(&mut containers, position);
                issue
            },
            EventKind::ArrayOpen | EventKind::InlineTableOpen => {
                inline_position(&mut containers, InlinePosition::Separator);
                containers.push(if event.kind() == EventKind::ArrayOpen {
                    Container::Array
                } else {
                    Container::Inline(InlinePosition::Open)
                });
                None
            },
            EventKind::ArrayClose | EventKind::InlineTableClose => {
                if matches!(
                    containers.pop(),
                    Some(Container::Inline(InlinePosition::Key))
                ) {
                    Some(SyntaxIssue {
                        position: start,
                        reason:   KEY_EXPECTED,
                    })
                } else {
                    None
                }
            },
            EventKind::Comment | EventKind::Newline => match containers.last() {
                Some(Container::Inline(position)) => Some(SyntaxIssue {
                    position: start,
                    reason:   match position {
                        InlinePosition::Open | InlinePosition::Key => KEY_EXPECTED,
                        InlinePosition::Assignment => {
                            "Expected '=' after a key in a key/value pair"
                        },
                        InlinePosition::Value => "Invalid value",
                        InlinePosition::Separator => INLINE_TABLE_UNCLOSED,
                    },
                }),
                _ => None,
            },
            EventKind::KeySep | EventKind::ValueSep => {
                inline_position(&mut containers, InlinePosition::Key);
                None
            },
            EventKind::KeyValSep => {
                inline_position(&mut containers, InlinePosition::Value);
                None
            },
            EventKind::Error => return None,
            _ => None,
        };
        if issue.is_some() {
            return issue;
        }
    }
    None
}

const fn inline_position(containers: &mut [Container], position: InlinePosition) {
    if let Some(Container::Inline(current)) = containers.last_mut() {
        *current = position;
    }
}

fn unsupported_escape(raw: &str) -> Option<usize> {
    let mut bytes = raw.bytes().enumerate();
    while let Some((_, byte)) = bytes.next() {
        if byte == b'\\'
            && let Some((position, b'e' | b'x')) = bytes.next()
        {
            return Some(position + 1);
        }
    }
    None
}

fn invalid_datetime(raw: &str, start: usize, container: Option<&Container>) -> Option<SyntaxIssue> {
    if !matches!(raw.as_bytes().get(2), Some(b':')) && !matches!(raw.as_bytes().get(4), Some(b'-'))
    {
        return None;
    }
    let datetime = raw.parse::<Datetime>().ok()?;
    if datetime.date.is_some_and(|date| date.year == 0) {
        return Some(SyntaxIssue {
            position: start,
            reason:   "Invalid date or datetime",
        });
    }
    let time = datetime.time?;
    if time.second.is_some_and(|seconds| seconds < 60) {
        return None;
    }
    // Python reads a datetime without a valid time as a date. It reads a time
    // without valid seconds as an integer if the hour has no leading zero.
    let relative = if datetime.date.is_some() {
        10 + usize::from(raw.as_bytes().get(10) == Some(&b' '))
    } else if time.hour < 10 {
        1
    } else {
        2
    };
    Some(SyntaxIssue {
        position: start + relative,
        reason:   match container {
            Some(Container::Array) => "Unclosed array",
            Some(Container::Inline(_)) => INLINE_TABLE_UNCLOSED,
            None => "Expected newline or end of document after a statement",
        },
    })
}
