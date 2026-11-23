//! Compare the edits to one binding and its uses.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::ComparedSource;
use super::EditRefusal;
use super::visibility;
use super::visibility::CaptureEdit;
use crate::binding::alignment;
use crate::binding::alignment::Alignment;
use crate::binding::alignment::AlignmentProof;
use crate::binding::alignment::EditForm;
use crate::binding::comment_plan;
use crate::binding::comment_plan::CommentPlan;
use crate::binding::pairs::RenamePairs;
use crate::binding::patterns::BindingScope;
use crate::binding::scan;
use crate::binding::scan::BindingScan;
use crate::rename_admission::BindingRename;
use crate::rename_admission::OwnedRename;

pub(super) enum EditProof<'a> {
    Applied(CommentPlan<'a>),
    Unchanged,
}

pub(super) fn compare<'a>(
    before: &ComparedSource,
    after: &ComparedSource,
    renames: &[BindingRename<'a>],
    pairs: &RenamePairs<'_>,
) -> BTreeMap<OwnedRename, Result<EditProof<'a>, EditRefusal>> {
    let (ComparedSource::Present(Ok(original)), ComparedSource::Present(Ok(edited))) =
        (before, after)
    else {
        return BTreeMap::new();
    };
    let alignment = alignment::binding_alignment(original, edited, pairs);
    let mut results = renames
        .iter()
        .map(|rename| {
            (
                OwnedRename::from(rename),
                edit_proof(original, edited, rename, pairs, &alignment),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if let AlignmentProof::Proved(alignment) = alignment {
        let bad = comment_plan::mismatches(
            original,
            edited,
            &alignment.locations,
            results.values().filter_map(|outcome| match outcome {
                Ok(EditProof::Applied(plan)) => Some(plan),
                _ => None,
            }),
        );
        for result in results.values_mut() {
            if matches!(result, Ok(EditProof::Applied(plan)) if !plan.agrees(&bad)) {
                *result = Err(EditRefusal);
            }
        }
    }
    results
}

fn edit_proof<'a>(
    before: &BindingScan,
    after: &BindingScan,
    rename: &BindingRename<'a>,
    pairs: &RenamePairs<'_>,
    alignment: &AlignmentProof,
) -> Result<EditProof<'a>, EditRefusal> {
    before.check_file_names(rename)?;
    let scope = before.locate(rename)?;
    let (Some(pair), AlignmentProof::Proved(alignment)) =
        (pairs.index(rename.old, rename.new), alignment)
    else {
        return Err(EditRefusal);
    };
    let own = alignment
        .edits
        .iter()
        .filter(|edit| edit.pair == pair && scan::contains(&scope.ranges, edit.before))
        .collect::<Vec<_>>();
    if own.is_empty() {
        return Ok(EditProof::Unchanged);
    }
    if !own.iter().any(|edit| edit.before == scope.declaration) {
        return Err(EditRefusal);
    }
    let comments = CommentPlan::read(before, &scope, rename).map_err(|_| EditRefusal)?;
    let capture_edit = if own.iter().any(|edit| edit.form == EditForm::Capture) {
        CaptureEdit::Edited
    } else {
        CaptureEdit::Unchanged
    };
    before.check_visibility(&scope, rename, capture_edit)?;
    if own.iter().any(|edit| {
        edit.form == EditForm::Name
            && (!before.name_word(edit.before)
                || before.shorthand(edit.before)
                || !after.name_word(edit.after)
                || after.shorthand(edit.after))
    }) {
        return Err(EditRefusal);
    }
    let targets = own
        .iter()
        .filter(|edit| edit.form != EditForm::Capture)
        .map(|edit| edit.after)
        .collect::<BTreeSet<_>>();
    let visible = visible_positions(alignment, &scope, own.iter().map(|edit| edit.after));
    if visible.iter().any(|&index| {
        let word = after.word(index);
        (word == rename.old || word == rename.new && !targets.contains(&index))
            && after.name_word(index)
    }) {
        return Err(EditRefusal);
    }
    if after
        .formats
        .intersection(&visible)
        .any(|&index| visibility::has_capture(after.word(index), rename.old))
    {
        return Err(EditRefusal);
    }
    Ok(EditProof::Applied(comments))
}

fn visible_positions(
    alignment: &Alignment,
    scope: &BindingScope,
    edited: impl Iterator<Item = usize>,
) -> BTreeSet<usize> {
    scope
        .ranges
        .iter()
        .cloned()
        .flatten()
        .filter_map(|index| alignment.locations.get(index).copied())
        .chain(edited)
        .collect()
}
