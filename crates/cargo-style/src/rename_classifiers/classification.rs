//! Recognize the file and name changes authorized by a finding.

use std::path::Path;

use fancy_regex::Error;
use fancy_regex::Regex;

use super::constants::CASE_INSENSITIVE_RUST_FILE;
use super::constants::CLAUSE_END;
use super::constants::CODE_NOUN;
use super::constants::NO_LEAD;
use super::constants::RUST_FILE;
use super::constants::RUST_KEYWORDS;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;
use crate::word_syntax;

/// The source reads required by a finding's edit.
pub(crate) enum FindingChange<'a> {
    FileMove(FileRename<'a>),
    QualifiedRename { old: &'a str },
    ModuleOwners,
    RecordedPaths,
}

/// The old and new paths of a move between sibling Rust files.
pub(crate) struct FileRename<'a> {
    pub(crate) old: &'a str,
    pub(crate) new: String,
}

enum FileMoveApproval<'a> {
    Approved(FileRename<'a>),
    Unapproved,
}

enum QualifiedRenameApproval<'a> {
    Approved { old: &'a str },
    Unapproved,
}

/// The grammar for approved renames and other edits to module owners.
pub(crate) struct RenameClassifiers {
    file_rename:      Regex,
    any_file_rename:  Regex,
    second_file_pair: Regex,
    code_move:        Regex,
    clause_start:     Regex,
    negated_verb:     Regex,
    negated_subject:  Regex,
    subject_move_end: Regex,
    negated_object:   Regex,
    negated_noun:     Regex,
    noun_move_end:    Regex,
}

impl RenameClassifiers {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self {
            file_rename:      Regex::new(&format!(
                r"\ARename ({RUST_FILE}) to ({RUST_FILE})(?=$|[\s;,:)]|\.(?:\s|$))"
            ))?,
            any_file_rename:  Regex::new(&format!(
                r"(?i)(?<![\w/.-])rename ({CASE_INSENSITIVE_RUST_FILE}) to {CASE_INSENSITIVE_RUST_FILE}"
            ))?,
            second_file_pair: Regex::new(&format!(
                r"(?i){CASE_INSENSITIVE_RUST_FILE}\s+to\s+{CASE_INSENSITIVE_RUST_FILE}"
            ))?,
            code_move:        Regex::new(concat!(
                r"(?i)(?<![\w-])(?:mov(?:e|es|ed|ing)|split(?:s|ting)?|",
                r"merg(?:e|es|ed|ing))(?![\w-])"
            ))?,
            clause_start:     Regex::new(r"(?i)[.;:,()]|\b(?:and|but|then)\b")?,
            negated_verb:     Regex::new(concat!(
                r"(?i)(?:\b(?:does|do|did|will|shall|must|should|may|can|could|would)\s+not",
                r"|n't|\bcannot|\bnever|\bwithout)\s*$"
            ))?,
            negated_subject:  Regex::new(&format!(
                r"(?i)\A\s*(?:nothing|none|{NO_LEAD}(?:\s+[\w-]+){{0,3}}?\s+{CODE_NOUN})\s*\z"
            ))?,
            subject_move_end: Regex::new(&format!(
                r"(?i)\A(?:\s+(?:into|out|from|to|between|across|within)\b|{CLAUSE_END})"
            ))?,
            negated_object:   Regex::new(&format!(
                r"(?i)\A\s+{NO_LEAD}(?:\s+[\w-]+){{0,3}}?\s+{CODE_NOUN}{CLAUSE_END}"
            ))?,
            negated_noun:     Regex::new(&format!(r"(?i)\A\s*{NO_LEAD}(?:\s+[\w-]+){{0,3}}\s*\z"))?,
            noun_move_end:    Regex::new(r"(?i)\A\s+(?:is|are|was|were)\b")?,
        })
    }

    pub(crate) fn classify<'a>(
        &self,
        path: &'a str,
        edit: &'a FindingEdit,
    ) -> Result<FindingChange<'a>, Error> {
        match edit.kind {
            FindingKind::Rename => {
                if let FileMoveApproval::Approved(moved) = self.file_rename(path, &edit.after)? {
                    return Ok(FindingChange::FileMove(moved));
                }
                if let QualifiedRenameApproval::Approved { old } =
                    qualified_rename(&edit.before, &edit.after)
                {
                    return Ok(FindingChange::QualifiedRename { old });
                }
                for matched in self.any_file_rename.captures_iter(&edit.after) {
                    if matched?
                        .get(1)
                        .is_some_and(|old| names_file(path, old.as_str()))
                    {
                        return Ok(FindingChange::ModuleOwners);
                    }
                }
                Ok(FindingChange::RecordedPaths)
            },
            FindingKind::Restructure if matches!(path.rsplit_once('.'), Some((_, "rs"))) => {
                Ok(FindingChange::ModuleOwners)
            },
            FindingKind::Restructure | FindingKind::Other => Ok(FindingChange::RecordedPaths),
        }
    }

    fn file_rename<'a>(&self, path: &'a str, after: &str) -> Result<FileMoveApproval<'a>, Error> {
        let Some(matched) = self.file_rename.captures(after)? else {
            return Ok(FileMoveApproval::Unapproved);
        };
        let (Some(phrase), Some(old), Some(new)) = (matched.get(0), matched.get(1), matched.get(2))
        else {
            return Ok(FileMoveApproval::Unapproved);
        };
        let old = old.as_str();
        let new = new.as_str();
        if !names_file(path, old)
            || Path::new(old).parent() != Path::new(new).parent()
            || stem(old) == stem(new)
            || [stem(old), stem(new)].into_iter().any(reserved_stem)
            || self.names_other_change(&after[phrase.end()..])?
        {
            return Ok(FileMoveApproval::Unapproved);
        }
        Ok(FileMoveApproval::Approved(FileRename {
            old: path,
            new: format!("{}{new}", &path[..path.len() - old.len()]),
        }))
    }

    fn names_other_change(&self, rest: &str) -> Result<bool, Error> {
        if self.second_file_pair.is_match(rest)? {
            return Ok(true);
        }
        for matched in self.code_move.find_iter(rest) {
            let verb = matched?;
            let before = &rest[..verb.start()];
            let after = &rest[verb.end()..];
            let mut clause_start = 0;
            for boundary in self.clause_start.find_iter(before) {
                clause_start = boundary?.end();
            }
            let clause = &before[clause_start..];
            if !(self.negated_verb.is_match(clause)?
                || self.negated_object.is_match(after)?
                || (self.negated_subject.is_match(clause)?
                    && self.subject_move_end.is_match(after)?)
                || (self.negated_noun.is_match(clause)? && self.noun_move_end.is_match(after)?))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

fn qualified_rename<'a>(before: &'a str, after: &str) -> QualifiedRenameApproval<'a> {
    let before: Vec<_> = before.split("::").collect();
    let after: Vec<_> = after.split("::").collect();
    let (Some((&old, old_owner)), Some((&new, new_owner))) =
        (before.split_last(), after.split_last())
    else {
        return QualifiedRenameApproval::Unapproved;
    };
    if !old_owner.is_empty()
        && old_owner == new_owner
        && old != new
        && word_syntax::identifier(old)
        && word_syntax::identifier(new)
    {
        QualifiedRenameApproval::Approved { old }
    } else {
        QualifiedRenameApproval::Unapproved
    }
}

fn names_file(path: &str, named: &str) -> bool {
    path.strip_suffix(named)
        .is_some_and(|prefix| prefix.is_empty() || prefix.ends_with('/'))
}

/// The module name of a Rust file whose path can end with mod.rs.
pub(crate) fn module_name(path: &str) -> &str {
    if filename(path) == "mod.rs" {
        let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
        filename(parent)
    } else {
        stem(path)
    }
}

pub(crate) fn filename(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

pub(crate) fn stem(path: &str) -> &str {
    Path::new(path)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("")
}

fn reserved_stem(name: &str) -> bool {
    matches!(name, "mod" | "lib" | "main")
        || RUST_KEYWORDS
            .split_ascii_whitespace()
            .any(|keyword| keyword == name)
}
