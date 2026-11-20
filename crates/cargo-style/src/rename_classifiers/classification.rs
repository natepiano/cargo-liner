//! Recognize the file and name changes authorized by a finding.

use std::path::Path;

use fancy_regex::Error;
use fancy_regex::Regex;

use super::constants::CASE_INSENSITIVE_RUST_FILE;
use super::file_rename::FileMoveApproval;
use super::file_rename::FileRename;
use super::qualified_rename::QualifiedRename;
use super::qualified_rename::QualifiedRenameApproval;
use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;

/// The source reads required by a finding's edit.
pub(crate) enum FindingChange<'a> {
    FileMove(FileRename<'a>),
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
                r"(?i)(?<![\w/.-])rename ({CASE_INSENSITIVE_RUST_FILE}) to {CASE_INSENSITIVE_RUST_FILE}"
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
                if let FileMoveApproval::Approved(moved) = FileRename::read(path, edit) {
                    return Ok(FindingChange::FileMove(moved));
                }
                if let QualifiedRenameApproval::Approved(rename) = QualifiedRename::read(edit) {
                    return Ok(FindingChange::QualifiedRename { old: rename.old });
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
            FindingKind::Restructure | FindingKind::Other(_) => Ok(FindingChange::RecordedPaths),
        }
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
