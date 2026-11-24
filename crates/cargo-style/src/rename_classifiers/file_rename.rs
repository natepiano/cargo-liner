//! Read a finding's complete file rename instruction.

use std::collections::HashSet;
use std::path::Path;
use std::sync::LazyLock;

use fancy_regex::Captures;
use fancy_regex::Error;
use fancy_regex::Regex;

use super::classification;
use super::constants::CASE_INSENSITIVE_RUST_FILE;
use super::constants::END_OF_RENAME;
use super::constants::FILE_INSTRUCTION_VERB;
use super::constants::PAIR_SEPARATOR;
use super::constants::PYTHON_SPACE;
use super::constants::REFERENCE_NAME;
use super::constants::REFERENCE_SEPARATOR;
use super::constants::REFERENCE_TARGET;
use super::constants::RENAME_DESCRIPTION;
use super::constants::RENAME_FILE_PAIR;
use super::constants::RENAME_VERB;
use super::constants::RUST_FILE;
use super::constants::SENTENCE_SEPARATOR;
use crate::binding;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;
use crate::tokenizer;

/// The complete list of moves in instruction order.
pub(crate) struct FileRenamePlan {
    pub(crate) first:      FileRename,
    pub(crate) companions: Vec<FileRename>,
}

impl FileRenamePlan {
    pub(crate) fn read(path: &str, edit: &FindingEdit) -> FileMoveApproval {
        if !matches!(edit.kind, FindingKind::Rename) {
            return FileMoveApproval::Unapproved;
        }
        let Ok(patterns) = INSTRUCTIONS.as_ref() else {
            return FileMoveApproval::Rejected;
        };
        patterns
            .read(path, edit)
            .unwrap_or(FileMoveApproval::Rejected)
    }

    pub(crate) fn moves(&self) -> impl Iterator<Item = &FileRename> {
        std::iter::once(&self.first).chain(&self.companions)
    }

    fn resolve(path: &str, first: NamedMove<'_>, others: &[NamedMove<'_>]) -> FileMoveApproval {
        let named = || std::iter::once(first).chain(others.iter().copied());
        let Some((anchor, prefix)) =
            named().find_map(|pair| file_prefix(path, pair.old).map(|prefix| (pair.old, prefix)))
        else {
            return FileMoveApproval::Rejected;
        };
        let folder = parent(path);
        let mut moves = named().map(|pair| pair.resolve(path, anchor, prefix, folder));
        let Some(Ok(first)) = moves.next() else {
            return FileMoveApproval::Rejected;
        };
        let Ok(companions) = moves.collect::<Result<Vec<_>, _>>() else {
            return FileMoveApproval::Rejected;
        };
        let plan = Self { first, companions };
        let mut seen = HashSet::new();
        if !plan
            .moves()
            .flat_map(|moved| [&moved.old, &moved.new])
            .all(|path| seen.insert(path))
        {
            return FileMoveApproval::Rejected;
        }
        FileMoveApproval::Approved(plan)
    }
}

pub(crate) enum FileMoveApproval {
    Approved(FileRenamePlan),
    Rejected,
    Unapproved,
}

/// The old and new paths of a move between sibling Rust files.
pub(crate) struct FileRename {
    pub(crate) old: String,
    pub(crate) new: String,
}

#[derive(Clone, Copy)]
struct NamedMove<'a> {
    old: &'a str,
    new: &'a str,
}

struct UnresolvedMove;

impl<'a> NamedMove<'a> {
    fn capture(captures: &Captures<'a>, instruction: &'a str) -> Option<(Self, &'a str)> {
        let moved = Self {
            old: captures.name("old")?.as_str(),
            new: captures.name("new")?.as_str(),
        };
        Some((moved, &instruction[captures.get(0)?.end()..]))
    }

    fn resolve(
        self,
        path: &str,
        anchor: &str,
        prefix: &str,
        folder: &Path,
    ) -> Result<FileRename, UnresolvedMove> {
        let old_file = self.old;
        let new_file = self.new;
        let (old, new) = if old_file == anchor && parent(old_file) == parent(new_file) {
            (path.to_owned(), format!("{prefix}{new_file}"))
        } else {
            let old = from_folder(old_file, folder);
            let new = from_folder(new_file, parent(&old));
            (old, new)
        };
        let old_stem = classification::stem(&old);
        let new_stem = classification::stem(&new);
        if (old_file == anchor && old != path)
            || parent(&old) != parent(&new)
            || old_stem == new_stem
            || [old_stem, new_stem].into_iter().any(|name| {
                matches!(name, "lib" | "main" | "mod")
                    || name.is_ascii() && !binding::local_name(name)
            })
        {
            return Err(UnresolvedMove);
        }
        Ok(FileRename { old, new })
    }
}

struct InstructionPatterns {
    first:       Regex,
    companion:   Regex,
    description: Regex,
    reference:   Regex,
    end:         Regex,
    file_verb:   Regex,
    file_side:   Regex,
}

static INSTRUCTIONS: LazyLock<Result<InstructionPatterns, Error>> =
    LazyLock::new(InstructionPatterns::compile);

impl InstructionPatterns {
    fn compile() -> Result<Self, Error> {
        let pair = RENAME_FILE_PAIR.replace("{file}", CASE_INSENSITIVE_RUST_FILE);
        let target = REFERENCE_TARGET
            .replace("{file}", CASE_INSENSITIVE_RUST_FILE)
            .replace("{name}", REFERENCE_NAME);
        let regex = |pattern: &str| Regex::new(&pattern.replace(r"\s", PYTHON_SPACE));
        Ok(Self {
            first:       regex(&format!(r"(?i)\A{RENAME_VERB}{pair}"))?,
            companion:   regex(&format!(
                r"(?i)\A(?:{SENTENCE_SEPARATOR}{RENAME_VERB}|{PAIR_SEPARATOR}(?:{RENAME_VERB})?){pair}"
            ))?,
            description: regex(&format!("(?i){RENAME_DESCRIPTION}"))?,
            reference:   regex(&format!(
                r"(?i)\A(?:{SENTENCE_SEPARATOR}|{PAIR_SEPARATOR})update\s+{target}(?:{REFERENCE_SEPARATOR}{target})*"
            ))?,
            end:         regex(END_OF_RENAME)?,
            file_verb:   regex(&format!("(?i){FILE_INSTRUCTION_VERB}"))?,
            file_side:   regex(&format!(r"\A(?:{RUST_FILE}|\x60{RUST_FILE}\x60)\z"))?,
        })
    }

    fn read(&self, path: &str, edit: &FindingEdit) -> Result<FileMoveApproval, Error> {
        let instruction = edit.after.trim_matches(tokenizer::python_space);
        let Some(first) = self.first.captures(instruction)? else {
            return Ok(
                if self.file_verb.is_match(instruction)?
                    || self.file_side.is_match(instruction)?
                    || self
                        .file_side
                        .is_match(edit.before.trim_matches(tokenizer::python_space))?
                {
                    FileMoveApproval::Rejected
                } else {
                    FileMoveApproval::Unapproved
                },
            );
        };
        let Some((first, mut rest)) = NamedMove::capture(&first, instruction) else {
            return Ok(FileMoveApproval::Rejected);
        };
        if let Some(description) = self.description.find(rest)? {
            rest = &rest[description.end()..];
        }
        let mut companions = Vec::new();
        while !self.end.is_match(rest)? {
            if let Some(pair) = self.companion.captures(rest)? {
                let Some((pair, tail)) = NamedMove::capture(&pair, rest) else {
                    return Ok(FileMoveApproval::Rejected);
                };
                companions.push(pair);
                rest = tail;
            } else if let Some(update) = self.reference.find(rest)? {
                rest = &rest[update.end()..];
            } else {
                return Ok(FileMoveApproval::Rejected);
            }
        }
        Ok(FileRenamePlan::resolve(path, first, &companions))
    }
}

pub(super) fn file_prefix<'a>(path: &'a str, named: &str) -> Option<&'a str> {
    path.strip_suffix(named)
        .filter(|prefix| prefix.is_empty() || prefix.ends_with('/'))
}

fn parent(path: &str) -> &Path { Path::new(path).parent().unwrap_or_else(|| Path::new("")) }

fn from_folder(path: &str, folder: &Path) -> String {
    if path.contains('/') {
        path.to_owned()
    } else {
        folder.join(path).to_string_lossy().into_owned()
    }
}
