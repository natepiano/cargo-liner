//! Map the recorded binding to the current item and reverse only its proved edits.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::constants::NAMED_WIDTH_PATTERN;
use super::declaration::Declaration;
use super::pairs::PairIndex;
use super::pairs::RenamePairs;
use super::patterns::BindingScope;
use super::presence;
use super::presence::Presence;
use super::presence::PresenceRefusal;
use super::scan;
use super::scan::BindingScan;
use crate::enclosing_items::EnclosingItem;
use crate::enclosing_items::OwnerBody;
use crate::rename_admission::BindingRename;
use crate::rename_admission::PolicyInput;
use crate::rename_admission::RefusalCause;
use crate::rename_admission::RenameRefusal;
use crate::word_syntax;

pub(crate) struct MappedLocalRename {
    pub(crate) item_identity:        Vec<String>,
    pub(crate) declaration_position: usize,
    pub(crate) edit_mapping:         Vec<(usize, usize)>,
}

pub(crate) enum CurrentAdmission {
    Mapped(MappedLocalRename),
    Refused(PresenceRefusal),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EditForm {
    Shorthand,
    Name,
    Capture,
}

pub(super) struct BindingEdit {
    pub(super) before: usize,
    pub(super) after:  usize,
    pub(super) form:   EditForm,
    pub(super) pair:   PairIndex,
}

pub(super) struct Alignment {
    pub(super) edits:     Vec<BindingEdit>,
    pub(super) locations: Vec<usize>,
}

pub(super) enum AlignmentProof {
    Proved(Alignment),
    Unproved,
}

struct ItemAlignment<'a> {
    alignment: Alignment,
    old_item:  &'a EnclosingItem,
    old_start: usize,
    start:     usize,
}

pub(crate) fn map_current_rename(
    current: &str,
    rename: &BindingRename<'_>,
    previous: &BindingScan,
    old_scope: &BindingScope,
    policy: &PolicyInput,
) -> CurrentAdmission {
    let current_scan;
    let scan = if previous.source.text() == current {
        previous
    } else {
        let Ok(reading) = BindingScan::read(current.to_owned()) else {
            return refused(RefusalCause::TokensUnread, rename);
        };
        current_scan = reading;
        &current_scan
    };
    let pairs = RenamePairs::from(std::slice::from_ref(rename));
    map_approved_renames(scan, rename, previous, old_scope, policy, &pairs)
}

pub(super) fn map_approved_renames(
    scan: &BindingScan,
    rename: &BindingRename<'_>,
    previous: &BindingScan,
    old_scope: &BindingScope,
    policy: &PolicyInput,
    pairs: &RenamePairs<'_>,
) -> CurrentAdmission {
    let attempt = coordinate_refusal(scan, rename, policy);
    if matches!(
        attempt,
        Presence::Refused(PresenceRefusal::MissingPolicyFlag)
    ) {
        return CurrentAdmission::Refused(PresenceRefusal::MissingPolicyFlag);
    }
    let Some(pair) = pairs.index(rename.old, rename.new) else {
        return refused(RefusalCause::EditsUnproved, rename);
    };
    let aligned = match align_items(previous, scan, old_scope, pairs) {
        Ok(alignment) => alignment,
        Err(RefusalCause::EditsUnproved) => {
            let cause = match attempt {
                Presence::Refused(PresenceRefusal::Unproved(refusal)) => refusal.reason,
                _ => RefusalCause::EditsUnproved,
            };
            return refused(cause, rename);
        },
        Err(cause) => return refused(cause, rename),
    };
    let Some(&current_index) = aligned
        .alignment
        .locations
        .get(old_scope.declaration - aligned.old_start)
    else {
        return refused(RefusalCause::DeclarationPositionUnproved, rename);
    };
    let Ok(scope) = scan.scope_at(aligned.start + current_index) else {
        return refused(RefusalCause::DeclarationPositionUnproved, rename);
    };
    if scope.kind != old_scope.kind {
        return refused(RefusalCause::DeclarationPositionUnproved, rename);
    }
    if scan.unread_binding(&scope, rename) || previous.unread_binding(old_scope, rename) {
        return refused(RefusalCause::BindingVisibilityUnread, rename);
    }
    let edits = match replacements(previous, scan, old_scope, &scope, &aligned, rename, pair) {
        Ok(edits) => edits,
        Err(cause) => return refused(cause, rename),
    };
    let presence = if edits.is_empty() && scan.word(scope.declaration) == rename.old {
        scan.presence(&scope, rename.new, policy)
    } else {
        restored_presence(scan, &scope, &edits, rename, policy)
    };
    if let Presence::Refused(refusal) = presence {
        return CurrentAdmission::Refused(refusal);
    }
    if !previous.has_text(rename.line, rename.text) {
        return refused(RefusalCause::FindingTextAbsent, rename);
    }
    CurrentAdmission::Mapped(MappedLocalRename {
        item_identity:        aligned.old_item.identity.clone(),
        declaration_position: previous.token_offset(old_scope.declaration),
        edit_mapping:         aligned
            .alignment
            .locations
            .into_iter()
            .enumerate()
            .map(|(old, new)| {
                (
                    previous.token_offset(aligned.old_start + old),
                    scan.token_offset(aligned.start + new),
                )
            })
            .collect(),
    })
}

pub(super) fn coordinate_refusal(
    scan: &BindingScan,
    rename: &BindingRename<'_>,
    policy: &PolicyInput,
) -> Presence {
    let exact = scan.has_text(rename.line, rename.text);
    match scan.declare(rename) {
        Ok(Declaration::NonLocal) => Presence::Clear,
        Ok(Declaration::Local(scope)) => {
            let result = scan.presence(&scope, rename.new, policy);
            if exact || !matches!(result, Presence::Clear) {
                result
            } else {
                unproved_presence(RefusalCause::FindingTextAbsent, rename)
            }
        },
        Err(_) if scan.proves_companion(rename) => Presence::Clear,
        Err(cause) => unproved_presence(
            if exact {
                cause
            } else {
                RefusalCause::FindingTextAbsent
            },
            rename,
        ),
    }
}

pub(super) fn unproved_presence(cause: RefusalCause, rename: &BindingRename<'_>) -> Presence {
    Presence::Refused(PresenceRefusal::Unproved(RenameRefusal::at_finding(
        cause, rename,
    )))
}

fn refused(cause: RefusalCause, rename: &BindingRename<'_>) -> CurrentAdmission {
    CurrentAdmission::Refused(PresenceRefusal::Unproved(RenameRefusal::at_finding(
        cause, rename,
    )))
}

fn align_items<'a>(
    previous: &'a BindingScan,
    scan: &BindingScan,
    old_scope: &BindingScope,
    pairs: &RenamePairs<'_>,
) -> Result<ItemAlignment<'a>, RefusalCause> {
    let old_items = previous
        .items()
        .map_err(|_| RefusalCause::RecordedItemUnread)?;
    let items = scan.items().map_err(|_| RefusalCause::RecordedItemUnread)?;
    let enclosing = presence::owners(old_items, old_scope.declaration);
    let old_item = *enclosing
        .last()
        .ok_or(RefusalCause::ContainingItemMissing)?;
    let item = items
        .iter()
        .find(|candidate| candidate.identity == old_item.identity)
        .ok_or(RefusalCause::ItemPathNotUnique)?;
    if !presence::unique_owners(&enclosing, old_items)
        || !presence::unique_owners(&enclosing, items)
    {
        return Err(RefusalCause::ItemPathNotUnique);
    }
    let (OwnerBody::Proved(old_body), OwnerBody::Proved(body)) = (&old_item.body, &item.body)
    else {
        return Err(RefusalCause::ItemBodyUnread);
    };
    let mut old_start = old_item.tokens.start;
    let mut start = item.tokens.start;
    let mut alignment = align_region(
        previous,
        scan,
        old_start..old_body.end,
        start..body.end,
        pairs,
    );
    if matches!(alignment, AlignmentProof::Unproved) && old_body.contains(&old_scope.declaration) {
        if old_body.is_empty() || body.is_empty() {
            return Err(RefusalCause::DeclarationPositionUnproved);
        }
        old_start = old_body.start;
        start = body.start;
        alignment = align_region(
            previous,
            scan,
            old_start..old_body.end,
            start..body.end,
            pairs,
        );
    }
    let AlignmentProof::Proved(alignment) = alignment else {
        return Err(RefusalCause::EditsUnproved);
    };
    Ok(ItemAlignment {
        alignment,
        old_item,
        old_start,
        start,
    })
}

fn align_region(
    previous: &BindingScan,
    scan: &BindingScan,
    before: Range<usize>,
    after: Range<usize>,
    pairs: &RenamePairs<'_>,
) -> AlignmentProof {
    if previous.words[before.clone()] == scan.words[after.clone()] {
        return AlignmentProof::Proved(Alignment {
            edits:     Vec::new(),
            locations: (0..before.len()).collect(),
        });
    }
    let Ok(before) = BindingScan::read(previous.region(before)) else {
        return AlignmentProof::Unproved;
    };
    let Ok(after) = BindingScan::read(scan.region(after)) else {
        return AlignmentProof::Unproved;
    };
    binding_alignment(&before, &after, pairs)
}

pub(super) fn binding_alignment(
    before: &BindingScan,
    after: &BindingScan,
    pairs: &RenamePairs<'_>,
) -> AlignmentProof {
    let mut edits = Vec::new();
    let mut locations = Vec::with_capacity(before.words.len());
    let (mut old_index, mut new_index) = (0, 0);
    while old_index < before.words.len() && new_index < after.words.len() {
        locations.push(new_index);
        let old = before.word(old_index);
        let new = after.word(new_index);
        if new == old
            && after.word(new_index + 1) == ":"
            && let Some(pair) = pairs.index(old, after.word(new_index + 2))
            && before.shorthand(old_index)
        {
            edits.push(BindingEdit {
                before: old_index,
                after: new_index + 2,
                form: EditForm::Shorthand,
                pair,
            });
            old_index += 1;
            new_index += 3;
            continue;
        }
        if old == new
            && before.word(old_index + 1) == ":"
            && let Some(pair) = pairs.index(before.word(old_index + 2), new)
            && after.shorthand(new_index)
        {
            locations.extend([new_index, new_index]);
            edits.push(BindingEdit {
                before: old_index + 2,
                after: new_index,
                form: EditForm::Shorthand,
                pair,
            });
            old_index += 3;
            new_index += 1;
            continue;
        }
        if old != new {
            if let Some(pair) = pairs.index(old, new) {
                edits.push(BindingEdit {
                    before: old_index,
                    after: new_index,
                    form: EditForm::Name,
                    pair,
                });
            } else if before.formats.contains(&old_index)
                && after.formats.contains(&new_index)
                && let Some(captures) = capture_alignment(old, new, pairs)
            {
                edits.extend(captures.into_iter().map(|pair| BindingEdit {
                    before: old_index,
                    after: new_index,
                    form: EditForm::Capture,
                    pair,
                }));
            } else {
                return AlignmentProof::Unproved;
            }
        }
        old_index += 1;
        new_index += 1;
    }
    if old_index == before.words.len() && new_index == after.words.len() {
        AlignmentProof::Proved(Alignment { edits, locations })
    } else {
        AlignmentProof::Unproved
    }
}

fn capture_alignment(left: &str, right: &str, pairs: &RenamePairs<'_>) -> Option<Vec<PairIndex>> {
    let original = captures(left);
    let edited = captures(right);
    if original.len() != edited.len() {
        return None;
    }
    let mut changes = Vec::new();
    let mut cursor = 0;
    let mut target_cursor = 0;
    for (source, target) in original.into_iter().zip(edited) {
        if left[cursor..source.start] != right[target_cursor..target.start] {
            return None;
        }
        let old = &left[source.clone()];
        let new = &right[target.clone()];
        if old != new {
            if left.as_bytes()[source.start - 1] != b'{' {
                return None;
            }
            changes.push(pairs.index(old, new)?);
        }
        cursor = source.end;
        target_cursor = target.end;
    }
    (left[cursor..] == right[target_cursor..]).then_some(changes)
}

static NAMED_WIDTH: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(NAMED_WIDTH_PATTERN).ok());

pub(super) fn captures(literal: &str) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let bytes = literal.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        if matches!(bytes.get(cursor..cursor + 2), Some(b"{{" | b"}}")) {
            cursor += 2;
            continue;
        }
        if bytes[cursor] != b'{' {
            cursor += 1;
            continue;
        }
        let start = cursor + 1;
        let Some(relative) = literal[start..].find('}') else {
            break;
        };
        let end = start + relative;
        let field = &literal[start..end];
        let name = field.split(':').next().unwrap_or_default();
        if word_syntax::identifier(name) {
            result.push(start..start + name.len());
        }
        if let Some(width) = NAMED_WIDTH.as_ref() {
            result.extend(
                width
                    .find_iter(field)
                    .map(|name| start + name.start()..start + name.end() - 1),
            );
        }
        cursor = end + 1;
    }
    result
}

fn replacements(
    previous: &BindingScan,
    scan: &BindingScan,
    old_scope: &BindingScope,
    scope: &BindingScope,
    aligned: &ItemAlignment<'_>,
    rename: &BindingRename<'_>,
    pair: PairIndex,
) -> Result<BTreeSet<(usize, usize)>, RefusalCause> {
    let mut replacements = BTreeSet::new();
    for edit in &aligned.alignment.edits {
        let before = aligned.old_start + edit.before;
        let after = aligned.start + edit.after;
        if edit.pair != pair || !scan::contains(&old_scope.ranges, before) {
            continue;
        }
        if !scan::contains(&scope.ranges, after) {
            return Err(RefusalCause::EditsUnproved);
        }
        let start = scan.token_offset(after);
        if edit.form == EditForm::Capture
            && previous.formats.contains(&before)
            && scan.formats.contains(&after)
        {
            let current = scan.word(after);
            for new in captures(current) {
                if current[new.clone()] == *rename.new {
                    replacements.insert((
                        start + current[..new.start].chars().count(),
                        start + current[..new.end].chars().count(),
                    ));
                }
            }
        } else if edit.form == EditForm::Name
            && previous.name_word(before)
            && scan.name_word(after)
            && !previous.shorthand(before)
            && !scan.shorthand(after)
        {
            replacements.insert((start, scan.token_end(after)));
        } else {
            return Err(RefusalCause::EditsUnproved);
        }
    }
    Ok(replacements)
}

fn restored_presence(
    scan: &BindingScan,
    scope: &BindingScope,
    edits: &BTreeSet<(usize, usize)>,
    rename: &BindingRename<'_>,
    policy: &PolicyInput,
) -> Presence {
    let declaration = scan.token_offset(scope.declaration);
    if !edits.iter().any(|&(start, _)| start == declaration) {
        return unproved_presence(RefusalCause::DeclarationEditUnproved, rename);
    }
    let mut restored = scan.chars.clone();
    for &(start, finish) in edits.iter().rev() {
        restored.splice(start..finish, rename.old.chars());
    }
    BindingScan::read(restored.into_iter().collect()).map_or_else(
        |_| unproved_presence(RefusalCause::TokensUnread, rename),
        |restored| restored.presence(scope, rename.new, policy),
    )
}
