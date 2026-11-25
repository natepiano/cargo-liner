//! Read an approved name change within one qualified owner.

use crate::ledger_read::FindingEdit;
use crate::ledger_read::FindingKind;
use crate::word_syntax;

pub(crate) enum QualifiedRenameApproval<'a> {
    Approved(QualifiedRename<'a>),
    Unapproved,
}

pub(crate) struct QualifiedRename<'a> {
    pub(crate) owner: &'a str,
    pub(crate) old:   &'a str,
    pub(crate) new:   &'a str,
}

impl<'a> QualifiedRename<'a> {
    pub(crate) fn read(edit: &'a FindingEdit) -> QualifiedRenameApproval<'a> {
        // Scan from the left to match Python when colons overlap.
        let delimiter = "::";
        let (Some((old_separator, _)), Some((new_separator, _))) = (
            edit.before.match_indices(delimiter).last(),
            edit.after.match_indices(delimiter).last(),
        ) else {
            return QualifiedRenameApproval::Unapproved;
        };
        let (old_parent, old_name) = edit.before.split_at(old_separator);
        let (new_parent, new_name) = edit.after.split_at(new_separator);
        let old = &old_name[delimiter.len()..];
        let new = &new_name[delimiter.len()..];
        if !matches!(edit.kind, FindingKind::Rename)
            || old_parent != new_parent
            || old == new
            || !word_syntax::identifier(old)
            || !word_syntax::identifier(new)
        {
            return QualifiedRenameApproval::Unapproved;
        }
        QualifiedRenameApproval::Approved(Self {
            owner: old_parent,
            old,
            new,
        })
    }
}
