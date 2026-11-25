//! Check if the approved file rename exists at the target.

use super::AppliedError;
use super::RenameApplied;
use super::read_source;
use crate::finding_source::FindingSource;
use crate::finding_source::TreeFile;
use crate::rename_classifiers::FileRename;
use crate::rename_classifiers::FileRenamePlan;

pub(super) fn check(
    plan: &FileRenamePlan,
    source: &mut FindingSource<'_>,
    target: &str,
) -> Result<RenameApplied, AppliedError> {
    if plan.companions.is_empty() {
        return check_move(&plan.first, source, target);
    }
    for moved in plan.moves() {
        if !matches!(
            check_move(moved, source, target)?,
            RenameApplied::AppliedInCode
        ) {
            return Ok(RenameApplied::NotApplied {
                path: moved.old.clone(),
            });
        }
    }
    Ok(RenameApplied::AppliedInCode)
}

fn check_move(
    rename: &FileRename,
    source: &mut FindingSource<'_>,
    target: &str,
) -> Result<RenameApplied, AppliedError> {
    let old = matches!(
        read_source(source, target, &rename.old)?,
        TreeFile::Bytes(_)
    );
    let new = matches!(
        read_source(source, target, &rename.new)?,
        TreeFile::Bytes(_)
    );
    if old == new {
        return Ok(RenameApplied::NoAppliedCheck {
            kind: "rename".to_owned(),
        });
    }
    for file in [&rename.old, &rename.new] {
        let folder = file.strip_suffix(".rs").unwrap_or(file);
        if !matches!(read_source(source, target, folder)?, TreeFile::Missing) {
            return Ok(RenameApplied::NoAppliedCheck {
                kind: "rename".to_owned(),
            });
        }
    }
    Ok(if new {
        RenameApplied::AppliedInCode
    } else {
        RenameApplied::NotApplied {
            path: rename.old.clone(),
        }
    })
}
