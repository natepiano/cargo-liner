//! Read the approved paths of a file rename.

use std::path::Path;
use std::sync::LazyLock;

use regex::Error as RegexError;
use regex::Regex;

use super::constants::CLAUSE;
use super::constants::CLAUSE_END;
use super::constants::CODE_NOUN;
use super::constants::FILE_RENAME;
use super::constants::MOVEMENT;
use super::constants::NEGATED_NOUN;
use super::constants::NEGATED_OBJECT;
use super::constants::NEGATED_SUBJECT;
use super::constants::NEGATED_VERB;
use super::constants::NOUN_END;
use super::constants::POSITIVE_NO;
use super::constants::RUST_FILE;
use super::constants::SECOND_FILE_PAIR;
use super::constants::SUBJECT_END;
use crate::binding;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;
use crate::tokenizer;
use crate::unicode_tables::SPACE;
use crate::unicode_tables::WORD;

pub(crate) enum FileMoveApproval<'a> {
    Approved(FileRename<'a>),
    Unapproved,
}

/// The old and new paths of a move between sibling Rust files.
pub(crate) struct FileRename<'a> {
    pub(crate) old: &'a str,
    pub(crate) new: String,
}

struct InstructionPatterns {
    rename:          Regex,
    second_pair:     Regex,
    movement:        Regex,
    clause:          Regex,
    negated_verb:    Regex,
    negated_subject: Regex,
    negated_noun:    Regex,
    negated_object:  Regex,
    subject_end:     Regex,
    noun_end:        Regex,
    positive_no:     Regex,
}

static INSTRUCTIONS: LazyLock<Result<InstructionPatterns, RegexError>> =
    LazyLock::new(InstructionPatterns::compile);

impl InstructionPatterns {
    fn compile() -> Result<Self, RegexError> {
        Ok(Self {
            rename:          Regex::new(&FILE_RENAME.replace("{file}", RUST_FILE))?,
            second_pair:     Regex::new(&SECOND_FILE_PAIR.replace("{file}", RUST_FILE))?,
            movement:        Regex::new(MOVEMENT)?,
            clause:          Regex::new(CLAUSE)?,
            negated_verb:    Regex::new(NEGATED_VERB)?,
            negated_subject: Regex::new(&NEGATED_SUBJECT.replace("{noun}", CODE_NOUN))?,
            negated_noun:    Regex::new(NEGATED_NOUN)?,
            negated_object:  Regex::new(
                &NEGATED_OBJECT
                    .replace("{noun}", CODE_NOUN)
                    .replace("{end}", CLAUSE_END),
            )?,
            subject_end:     Regex::new(&SUBJECT_END.replace("{end}", CLAUSE_END))?,
            noun_end:        Regex::new(NOUN_END)?,
            positive_no:     Regex::new(POSITIVE_NO)?,
        })
    }

    fn names_other_change(&self, rest: &str) -> bool {
        let folded = fold_instruction(rest);
        let rest = folded.as_str();
        if self.second_pair.is_match(rest) {
            return true;
        }
        for movement in self.movement.find_iter(rest) {
            let before = &rest[..movement.start()];
            let after = &rest[movement.end()..];
            let in_word = |character: char| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | 'é')
            };
            if before.chars().next_back().is_some_and(in_word)
                || after.chars().next().is_some_and(in_word)
            {
                continue;
            }
            let start = self
                .clause
                .find_iter(before)
                .last()
                .map_or(0, |mark| mark.end());
            let clause = &before[start..];
            let negated_clause = !self.positive_no.is_match(clause)
                && (self.negated_subject.is_match(clause) && self.subject_end.is_match(after)
                    || self.negated_noun.is_match(clause) && self.noun_end.is_match(after));
            let negated_object =
                self.negated_object.is_match(after) && !self.positive_no.is_match(after);
            if !(self.negated_verb.is_match(clause) || negated_clause || negated_object) {
                return true;
            }
        }
        false
    }
}

// Preserve Python's space and word classes in the ASCII instruction patterns.
fn fold_instruction(text: &str) -> String {
    text.chars()
        .map(|character| match character {
            'İ' | 'ı' => 'i',
            'ſ' => 's',
            'K' => 'k',
            _ if tokenizer::in_table(SPACE, character) => ' ',
            _ if character.is_ascii() => character.to_ascii_lowercase(),
            _ if tokenizer::in_table(WORD, character) => 'é',
            _ => '§',
        })
        .collect()
}

impl<'a> FileRename<'a> {
    pub(crate) fn read(path: &'a str, edit: &FindingEdit) -> FileMoveApproval<'a> {
        if !matches!(edit.kind, FindingKind::Rename) {
            return FileMoveApproval::Unapproved;
        }
        let Ok(patterns) = INSTRUCTIONS.as_ref() else {
            return FileMoveApproval::Unapproved;
        };
        let after = &edit.after;
        let Some(captures) = patterns.rename.captures(after) else {
            return FileMoveApproval::Unapproved;
        };
        let (whole, [old, new]) = captures.extract();
        let rest = &after[whole.len()..];
        if !ends_path(rest) || patterns.names_other_change(rest) {
            return FileMoveApproval::Unapproved;
        }
        let Some(prefix) = path
            .strip_suffix(old)
            .filter(|prefix| prefix.is_empty() || prefix.ends_with('/'))
        else {
            return FileMoveApproval::Unapproved;
        };
        let (old_path, new_path) = (Path::new(old), Path::new(new));
        if old_path.parent() != new_path.parent()
            || old == new
            || [old_path, new_path].iter().any(|path| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_none_or(|stem| {
                        matches!(stem, "lib" | "main" | "mod") || !binding::local_name(stem)
                    })
            })
        {
            return FileMoveApproval::Unapproved;
        }
        FileMoveApproval::Approved(Self {
            old: path,
            new: format!("{prefix}{new}"),
        })
    }
}

fn ends_path(rest: &str) -> bool {
    let space = |character| tokenizer::in_table(SPACE, character);
    rest.chars().next().is_none_or(|character| {
        space(character)
            || matches!(character, ';' | ',' | ':' | ')')
            || character == '.' && rest[1..].chars().next().is_none_or(space)
    })
}
