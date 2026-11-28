//! Recognize the file and name changes authorized by a finding.

use std::path::Path;

use fancy_regex::Error;
use fancy_regex::Regex;

use super::constants::CASE_INSENSITIVE_RUST_FILE;
use super::file_rename;
use super::file_rename::FileMoveApproval;
use super::file_rename::FileRenamePlan;
use super::qualified_rename::QualifiedRename;
use super::qualified_rename::QualifiedRenameApproval;
use crate::binding::NameChangePolicy;
use crate::binding::SingleNameChange;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;
use crate::tokenizer;
use crate::unicode_tables::WORD;
use crate::word_syntax;

/// The file or item rename described by a finding.
pub(crate) enum RenameIntent<'a> {
    FileMoves(FileRenamePlan),
    Qualified(QualifiedRename<'a>),
    Binding,
    Unparsed,
    OtherKind,
}

impl<'a> RenameIntent<'a> {
    pub(crate) fn read(path: &str, edit: &'a FindingEdit) -> Self {
        if !matches!(edit.kind, FindingKind::Rename) {
            return Self::OtherKind;
        }
        match FileRenamePlan::read(path, edit) {
            FileMoveApproval::Approved(plan) => return Self::FileMoves(plan),
            FileMoveApproval::Rejected => return Self::Unparsed,
            FileMoveApproval::Unapproved => {},
        }
        let target = item_target(&edit.after);
        if target && !item_target(&edit.before) {
            return Self::Unparsed;
        }
        if let QualifiedRenameApproval::Approved(rename) = QualifiedRename::read(edit) {
            return Self::Qualified(rename);
        }
        if (target
            || edit
                .before
                .contains(['(', ')', ':', '=', '{', '}', '[', ']', '.']))
            && matches!(
                SingleNameChange::read(&edit.before, &edit.after, NameChangePolicy::Refactor),
                SingleNameChange::Changed { .. }
            )
        {
            Self::Binding
        } else {
            Self::Unparsed
        }
    }
}

/// The source reads required by a finding's edit.
pub(crate) enum FindingChange<'a> {
    FileMoves(FileRenamePlan),
    QualifiedRename { old: &'a str },
    ModuleOwners,
    RecordedPaths,
}

/// The pattern for instructions that name the file in a change.
pub(crate) struct RenameClassifiers {
    any_file_rename: Regex,
}

impl RenameClassifiers {
    pub(crate) fn new() -> Result<Self, Error> {
        Ok(Self {
            any_file_rename: Regex::new(&format!(
                r"(?i)rename ({CASE_INSENSITIVE_RUST_FILE}) to {CASE_INSENSITIVE_RUST_FILE}"
            ))?,
        })
    }

    pub(crate) fn classify<'a>(
        &self,
        path: &'a str,
        edit: &'a FindingEdit,
    ) -> Result<FindingChange<'a>, Error> {
        match edit.kind {
            FindingKind::Rename => {
                if let FileMoveApproval::Approved(plan) = FileRenamePlan::read(path, edit) {
                    return Ok(FindingChange::FileMoves(plan));
                }
                if let QualifiedRenameApproval::Approved(rename) = QualifiedRename::read(edit) {
                    return Ok(FindingChange::QualifiedRename { old: rename.old });
                }
                for matched in self.any_file_rename.captures_iter(&edit.after) {
                    let matched = matched?;
                    let (Some(instruction), Some(old)) = (matched.get(0), matched.get(1)) else {
                        continue;
                    };
                    let preceded_by_word_or_path = edit.after[..instruction.start()]
                        .chars()
                        .next_back()
                        .is_some_and(|character| {
                            matches!(character, '/' | '.' | '-')
                                || tokenizer::in_table(WORD, character)
                        });
                    if !preceded_by_word_or_path
                        && file_rename::file_prefix(path, old.as_str()).is_some()
                    {
                        return Ok(FindingChange::ModuleOwners);
                    }
                }
                Ok(FindingChange::RecordedPaths)
            },
            FindingKind::Restructure if matches!(path.rsplit_once('.'), Some((_, "rs"))) => {
                Ok(FindingChange::ModuleOwners)
            },
            FindingKind::Restructure | FindingKind::Type | FindingKind::Other(_) => {
                Ok(FindingChange::RecordedPaths)
            },
        }
    }
}

fn item_target(edit: &str) -> bool {
    let target = edit.trim_matches(tokenizer::python_space);
    let mut components = target.strip_prefix("::").unwrap_or(target).rsplit("::");
    components.next().is_some_and(word_syntax::identifier)
        && components.all(|owner| !owner.starts_with("r#") && word_syntax::identifier(owner))
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
