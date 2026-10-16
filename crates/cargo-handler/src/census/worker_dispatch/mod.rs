//! Literal worker launcher commands and their roster-backed thread identities.

use std::collections::BTreeMap;
use std::fs;
use std::iter::Peekable;
use std::path::Path;
use std::path::PathBuf;
use std::str::Chars;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

/// A shell either remains ordinary work or names a roster-backed worker.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerDispatch {
    /// No worker identity could be established at scan time.
    #[default]
    Plain,
    /// The launcher's seat and thread were found in its roster.
    Worker(ResolvedWorkerDispatch),
}

impl WorkerDispatch {
    /// Ordinary shells need no additional probe field.
    pub(super) const fn is_plain(&self) -> bool { matches!(self, Self::Plain) }
}

/// A worker seat with the thread identity needed to join its shell and row.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct ResolvedWorkerDispatch {
    /// The complete seat key in the mesh roster.
    seat:                 String,
    /// The recorded thread belonging to that seat.
    pub(super) thread_id: String,
    /// The responsibility passed after the launcher's task argument.
    job:                  String,
    /// The seat with its worktree directory prefix removed.
    short_seat:           String,
}

impl ResolvedWorkerDispatch {
    /// The worker row's display name, on one line however the job was written.
    pub(crate) fn label(&self) -> String {
        let job = self.job.split_whitespace().collect::<Vec<_>>().join(" ");
        format!("{} · {job}", self.short_seat)
    }
}

/// How the launcher selects its seat before its roster is read.
#[derive(Clone, Debug, Eq, PartialEq)]
enum SeatSelection {
    /// An explicit `--to` names the roster entry.
    Named(String),
    /// A new dispatch is the entry bearing its launcher pid.
    Launcher,
}

/// A literal implement.sh invocation, before roster resolution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ParsedDispatch {
    /// The directory containing `mesh_roster.json`.
    pub(super) session_dir: PathBuf,
    /// The worktree whose basename prefixes its seat.
    worktree:               PathBuf,
    /// How this invocation locates its roster entry.
    seat:                   SeatSelection,
    /// The launcher's responsibility argument.
    job:                    String,
}

/// The command cannot safely be interpreted as one worker invocation.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct NotWorkerDispatch;

impl ParsedDispatch {
    /// Read literal assignments followed by bash implement.sh without executing them.
    pub(super) fn parse(command: &str) -> Result<Self, NotWorkerDispatch> {
        Self::from_arguments(&command_words(command)?)
    }

    /// Read argv from the actual launcher process without re-splitting it.
    pub(super) fn from_arguments(words: &[String]) -> Result<Self, NotWorkerDispatch> {
        let mut args = words.iter().map(String::as_str);
        let program = args.next().ok_or(NotWorkerDispatch)?;
        if Path::new(program)
            .file_name()
            .is_none_or(|name| name != "bash")
        {
            return Err(NotWorkerDispatch);
        }
        let script = args.next().ok_or(NotWorkerDispatch)?;
        if !Path::new(script).ends_with(".claude/scripts/delegate/implement.sh") {
            return Err(NotWorkerDispatch);
        }
        let mut args = args.peekable();
        let mut seat = SeatSelection::Launcher;
        while args.peek().is_some_and(|word| word.starts_with("--")) {
            match args.next() {
                Some("--to") => {
                    let name = args
                        .next()
                        .filter(|name| !name.is_empty())
                        .ok_or(NotWorkerDispatch)?;
                    seat = SeatSelection::Named(name.to_owned());
                },
                Some("--phase") => {
                    args.next()
                        .filter(|phase| !phase.is_empty())
                        .ok_or(NotWorkerDispatch)?;
                },
                _ => return Err(NotWorkerDispatch),
            }
        }
        let session_dir = args
            .next()
            .filter(|dir| !dir.is_empty())
            .ok_or(NotWorkerDispatch)?;
        let worktree = args
            .next()
            .filter(|dir| !dir.is_empty())
            .ok_or(NotWorkerDispatch)?;
        args.next()
            .filter(|prompt| !prompt.is_empty())
            .ok_or(NotWorkerDispatch)?;
        match args.next() {
            Some("impl" | "test" | "review" | "fix") => {},
            _ => return Err(NotWorkerDispatch),
        }
        let job = args
            .next()
            .filter(|job| !job.is_empty())
            .ok_or(NotWorkerDispatch)?;
        // The remaining launcher positions are pass kind, activity, fix pass,
        // team role, and an optional mesh prefix. Nothing may follow them.
        let remaining: Vec<_> = args.collect();
        match remaining.as_slice() {
            ["impl" | "test" | "review" | "fix", _, _, team_role]
            | ["impl" | "test" | "review" | "fix", _, _, team_role, _]
                if !team_role.is_empty() => {},
            _ => return Err(NotWorkerDispatch),
        }
        Ok(Self {
            session_dir: PathBuf::from(session_dir),
            worktree: PathBuf::from(worktree),
            seat,
            job: job.to_owned(),
        })
    }

    /// Join the explicit seat, or this dispatch's pid, to a recorded thread.
    pub(super) fn resolve(&self, roster: &WorkerRoster, dispatch_pid: u32) -> WorkerDispatch {
        let entry = match &self.seat {
            SeatSelection::Named(seat) => roster.seats.get_key_value(seat),
            SeatSelection::Launcher => {
                let mut matches = roster.seats.iter().filter(|(_, value)| {
                    value.launcher == LauncherIdentity::Recorded(dispatch_pid)
                });
                let first = matches.next();
                if matches.next().is_some() {
                    return WorkerDispatch::Plain;
                }
                first
            },
        };
        let Some((seat, entry)) = entry else {
            return WorkerDispatch::Plain;
        };

        let short_seat = self
            .worktree
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| seat.strip_prefix(&format!("{name}-")))
            .unwrap_or(seat)
            .to_owned();
        WorkerDispatch::Worker(ResolvedWorkerDispatch {
            seat: seat.clone(),
            thread_id: entry.thread_id.clone(),
            job: self.job.clone(),
            short_seat,
        })
    }
}

/// Whether a roster entry records its launcher's process identity.
#[derive(Debug, Eq, PartialEq)]
enum LauncherIdentity {
    /// Finished seats may no longer have a launcher pid.
    Unrecorded,
    /// The pid can identify a dispatch without an explicit seat.
    Recorded(u32),
}

/// A roster entry with a usable thread identity.
#[derive(Debug)]
struct RosterWorker {
    /// The nonempty thread id to join to the census.
    thread_id: String,
    /// The dispatch pid, where the roster records it.
    launcher:  LauncherIdentity,
}

/// The seat entries read once per session directory during a scan.
#[derive(Debug, Default)]
pub(super) struct WorkerRoster {
    /// Entries whose thread identity was validated at the JSON boundary.
    seats: BTreeMap<String, RosterWorker>,
}

impl WorkerRoster {
    /// Malformed or unavailable rosters provide no worker identities.
    pub(super) fn parse(text: &str) -> Self {
        let entries: BTreeMap<String, Value> = serde_json::from_str(text).unwrap_or_default();
        let seats = entries
            .into_iter()
            .filter_map(|(seat, value)| {
                if seat.is_empty() {
                    return None;
                }
                let thread_id = value.get("thread_id")?.as_str()?.trim();
                if thread_id.is_empty() {
                    return None;
                }
                let launcher = value
                    .get("launcher_pid")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|pid| u32::try_from(pid).ok())
                    .map_or(LauncherIdentity::Unrecorded, LauncherIdentity::Recorded);
                Some((
                    seat,
                    RosterWorker {
                        thread_id: thread_id.to_owned(),
                        launcher,
                    },
                ))
            })
            .collect();
        Self { seats }
    }

    /// Read the roster only during census scanning.
    pub(super) fn read(session_dir: &Path) -> Self {
        fs::read_to_string(session_dir.join("mesh_roster.json"))
            .map(|text| Self::parse(&text))
            .unwrap_or_default()
    }
}

/// A thread's original label, or a matched dispatch's responsibility.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) enum DispatchLabel<'a> {
    /// No shell was folded into this row.
    #[default]
    Original,
    /// The same tile contains this worker's dispatch.
    Worker(&'a ResolvedWorkerDispatch),
}

/// A complete literal word, classified before its position in the command is known.
#[derive(Debug)]
enum LiteralWord {
    /// An unquoted shell identifier followed by an unquoted equals sign.
    Assignment(String),
    /// A word that cannot introduce an environment assignment.
    Argument(String),
}

impl LiteralWord {
    /// Both kinds retain their full text for use after the command word.
    const fn text_mut(&mut self) -> &mut String {
        match self {
            Self::Assignment(text) | Self::Argument(text) => text,
        }
    }

    /// After the assignment prefix, every word is an ordinary argv entry.
    fn into_argument(self) -> String {
        match self {
            Self::Assignment(text) | Self::Argument(text) => text,
        }
    }
}

/// Quoting before the first equals sign rules out an assignment name.
#[derive(Default)]
enum WordBeingLexed {
    /// Whitespace has not yet been followed by a word.
    #[default]
    Empty,
    /// No quote, escape, or equals sign has appeared in this word yet.
    UnquotedPrefix(String),
    /// The word's assignment syntax has been determined.
    Classified(LiteralWord),
}

impl WordBeingLexed {
    /// Only an unquoted equals sign after a shell identifier makes an assignment.
    fn push_unquoted(&mut self, character: char) {
        match self {
            Self::Empty if character == '=' => {
                *self = Self::Classified(LiteralWord::Argument(character.to_string()));
            },
            Self::Empty => *self = Self::UnquotedPrefix(character.to_string()),
            Self::UnquotedPrefix(text) if character == '=' => {
                let mut characters = text.chars();
                let valid_name = characters
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
                    && characters
                        .all(|character| character.is_ascii_alphanumeric() || character == '_');
                let mut text = std::mem::take(text);
                text.push(character);
                *self = Self::Classified(if valid_name {
                    LiteralWord::Assignment(text)
                } else {
                    LiteralWord::Argument(text)
                });
            },
            Self::UnquotedPrefix(text) => text.push(character),
            Self::Classified(word) => word.text_mut().push(character),
        }
    }

    /// Quoted or escaped text can belong to an assignment value, but never its name.
    fn push_quoted(&mut self, quoted: &str) {
        match self {
            Self::Empty => *self = Self::Classified(LiteralWord::Argument(quoted.to_owned())),
            Self::UnquotedPrefix(text) => {
                let mut text = std::mem::take(text);
                text.push_str(quoted);
                *self = Self::Classified(LiteralWord::Argument(text));
            },
            Self::Classified(word) => word.text_mut().push_str(quoted),
        }
    }

    /// Complete a word without turning whitespace into an empty argument.
    fn finish(&mut self, words: &mut Vec<LiteralWord>) {
        match std::mem::take(self) {
            Self::Empty => {},
            Self::UnquotedPrefix(text) => words.push(LiteralWord::Argument(text)),
            Self::Classified(word) => words.push(word),
        }
    }
}

/// The literal quoting forms accepted inside a shell word.
#[derive(Clone, Copy)]
enum QuotedSegment {
    /// Single quotes preserve every character until the closing quote.
    Single,
    /// Double quotes allow escaped punctuation, but expansions remain forbidden.
    Double,
    /// Dollar-single quotes decode Bash's ANSI-C escapes without expansion.
    AnsiC,
}

impl QuotedSegment {
    /// Read after the opening delimiter, consuming the closing delimiter too.
    fn read(self, chars: &mut Peekable<Chars<'_>>) -> Result<String, NotWorkerDispatch> {
        if matches!(self, Self::AnsiC) {
            return read_ansi_c_segment(chars);
        }
        let mut quoted = String::new();
        loop {
            match (self, chars.next()) {
                (Self::Single, Some('\'')) | (Self::Double, Some('"')) => return Ok(quoted),
                (Self::Double, Some('\\')) => match chars.next() {
                    Some(escaped @ ('$' | '`' | '"' | '\\')) => quoted.push(escaped),
                    Some('\n') => {},
                    Some(other) => {
                        quoted.push('\\');
                        quoted.push(other);
                    },
                    None => return Err(NotWorkerDispatch),
                },
                (Self::Double, Some('$' | '`')) | (_, None) => return Err(NotWorkerDispatch),
                (_, Some(inner)) => quoted.push(inner),
            }
        }
    }
}

/// Decode bytes first, since octal and hex escapes may form one UTF-8 character.
fn read_ansi_c_segment(chars: &mut Peekable<Chars<'_>>) -> Result<String, NotWorkerDispatch> {
    let mut quoted = Vec::new();
    loop {
        match chars.next() {
            Some('\'') => break,
            Some('\\') => read_ansi_c_escape(chars, &mut quoted)?,
            Some(inner) => quoted.extend_from_slice(inner.encode_utf8(&mut [0; 4]).as_bytes()),
            None => return Err(NotWorkerDispatch),
        }
    }
    // Bash words cannot contain NUL: it ends this quoted segment's decoded text.
    if let Some(end) = quoted.iter().position(|byte| *byte == 0) {
        quoted.truncate(end);
    }
    String::from_utf8(quoted).map_err(|_| NotWorkerDispatch)
}

/// Consume one escape after its backslash; unknown escapes keep the backslash.
fn read_ansi_c_escape(
    chars: &mut Peekable<Chars<'_>>,
    quoted: &mut Vec<u8>,
) -> Result<(), NotWorkerDispatch> {
    match chars.next().ok_or(NotWorkerDispatch)? {
        'a' => quoted.push(7),
        'b' => quoted.push(8),
        'e' | 'E' => quoted.push(27),
        'f' => quoted.push(12),
        'n' => quoted.push(b'\n'),
        'r' => quoted.push(b'\r'),
        't' => quoted.push(b'\t'),
        'v' => quoted.push(11),
        escaped @ ('\\' | '\'' | '"' | '?') => {
            quoted.extend_from_slice(escaped.encode_utf8(&mut [0; 4]).as_bytes());
        },
        first @ '0'..='7' => {
            let digits: String = std::iter::once(first)
                .chain(std::iter::from_fn(|| chars.next_if(|c| c.is_digit(8))).take(2))
                .collect();
            let value = u32::from_str_radix(&digits, 8).map_err(|_| NotWorkerDispatch)?;
            quoted.push(value.to_le_bytes()[0]);
        },
        escape @ ('x' | 'u' | 'U') => {
            let limit = match escape {
                'x' => 2,
                'u' => 4,
                _ => 8,
            };
            let digits: String = std::iter::from_fn(|| chars.next_if(char::is_ascii_hexdigit))
                .take(limit)
                .collect();
            if digits.is_empty() {
                quoted.push(b'\\');
                quoted.extend_from_slice(escape.encode_utf8(&mut [0; 4]).as_bytes());
            } else {
                let value = u32::from_str_radix(&digits, 16).map_err(|_| NotWorkerDispatch)?;
                if escape == 'x' {
                    quoted.push(value.to_le_bytes()[0]);
                } else {
                    let character = char::from_u32(value).ok_or(NotWorkerDispatch)?;
                    quoted.extend_from_slice(character.encode_utf8(&mut [0; 4]).as_bytes());
                }
            }
        },
        'c' => {
            if chars.peek().is_none_or(|next| *next == '\'') {
                quoted.extend_from_slice(b"\\c");
                return Ok(());
            }
            let control = chars
                .next()
                .filter(char::is_ascii)
                .ok_or(NotWorkerDispatch)?;
            if control == '\\' {
                chars.next_if(|next| *next == '\\');
            }
            let byte = u8::try_from(u32::from(control)).map_err(|_| NotWorkerDispatch)?;
            quoted.push(if byte == b'?' { 127 } else { byte & 31 });
        },
        other => {
            quoted.push(b'\\');
            quoted.extend_from_slice(other.encode_utf8(&mut [0; 4]).as_bytes());
        },
    }
    Ok(())
}

/// Split one literal command into launcher argv, consuming leading assignments.
/// Expansions and command operators cannot establish a safe worker identity.
fn command_words(command: &str) -> Result<Vec<String>, NotWorkerDispatch> {
    let mut words = Vec::new();
    let mut word = WordBeingLexed::Empty;
    let mut chars = command.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            ' ' | '\t' => word.finish(&mut words),
            '\'' => word.push_quoted(&QuotedSegment::Single.read(&mut chars)?),
            '"' => word.push_quoted(&QuotedSegment::Double.read(&mut chars)?),
            '$' => {
                if chars.next() != Some('\'') {
                    return Err(NotWorkerDispatch);
                }
                word.push_quoted(&QuotedSegment::AnsiC.read(&mut chars)?);
            },
            '\\' => match chars.next() {
                Some('\n') => {},
                Some(escaped) => word.push_quoted(&escaped.to_string()),
                None => return Err(NotWorkerDispatch),
            },
            '\n' | '`' | ';' | '&' | '|' | '(' | ')' | '<' | '>' => {
                return Err(NotWorkerDispatch);
            },
            other => word.push_unquoted(other),
        }
    }
    word.finish(&mut words);
    Ok(words
        .into_iter()
        .skip_while(|word| matches!(word, LiteralWord::Assignment(_)))
        .map(LiteralWord::into_argument)
        .collect())
}

#[cfg(test)]
mod tests;
