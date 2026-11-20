//! Map the recorded binding to the current item and reverse only its proved edits.

use std::collections::BTreeSet;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

use super::constants::NAMED_WIDTH_PATTERN;
use super::declaration::Declaration;
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
enum EditForm {
    Shorthand,
    Name,
    Capture,
}

struct BindingEdit {
    before: usize,
    after:  usize,
    form:   EditForm,
}

struct Alignment {
    edits:     Vec<BindingEdit>,
    locations: Vec<usize>,
}

enum AlignmentProof {
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
    let attempt = coordinate_refusal(scan, rename, policy);
    if matches!(
        attempt,
        Presence::Refused(PresenceRefusal::MissingPolicyFlag)
    ) {
        return CurrentAdmission::Refused(PresenceRefusal::MissingPolicyFlag);
    }
    let aligned = match align_items(previous, scan, old_scope, rename) {
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
    let edits = match replacements(previous, scan, old_scope, &scope, &aligned, rename) {
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

fn coordinate_refusal(
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

fn unproved_presence(cause: RefusalCause, rename: &BindingRename<'_>) -> Presence {
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
    rename: &BindingRename<'_>,
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
        rename,
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
            rename,
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
    rename: &BindingRename<'_>,
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
    binding_alignment(&before, &after, rename)
}

fn binding_alignment(
    before: &BindingScan,
    after: &BindingScan,
    rename: &BindingRename<'_>,
) -> AlignmentProof {
    let mut edits = Vec::new();
    let mut locations = Vec::with_capacity(before.words.len());
    let (mut a, mut b) = (0, 0);
    while a < before.words.len() && b < after.words.len() {
        locations.push(b);
        let old = before.word(a);
        let new = after.word(b);
        if new == old
            && after.word(b + 1) == ":"
            && same_pair(old, after.word(b + 2), rename)
            && before.shorthand(a)
        {
            edits.push(BindingEdit {
                before: a,
                after:  b + 2,
                form:   EditForm::Shorthand,
            });
            a += 1;
            b += 3;
            continue;
        }
        if old == new
            && before.word(a + 1) == ":"
            && same_pair(before.word(a + 2), new, rename)
            && after.shorthand(b)
        {
            locations.extend([b, b]);
            edits.push(BindingEdit {
                before: a + 2,
                after:  b,
                form:   EditForm::Shorthand,
            });
            a += 3;
            b += 1;
            continue;
        }
        if old != new {
            let form = if same_pair(old, new, rename) {
                EditForm::Name
            } else if before.formats.contains(&a)
                && after.formats.contains(&b)
                && capture_alignment(old, new, rename)
            {
                EditForm::Capture
            } else {
                return AlignmentProof::Unproved;
            };
            edits.push(BindingEdit {
                before: a,
                after: b,
                form,
            });
        }
        a += 1;
        b += 1;
    }
    if a == before.words.len() && b == after.words.len() {
        AlignmentProof::Proved(Alignment { edits, locations })
    } else {
        AlignmentProof::Unproved
    }
}

fn same_pair(old: &str, new: &str, rename: &BindingRename<'_>) -> bool {
    old == rename.old && new == rename.new
}

fn capture_alignment(left: &str, right: &str, rename: &BindingRename<'_>) -> bool {
    let original = captures(left);
    let edited = captures(right);
    if original.len() != edited.len() {
        return false;
    }
    let mut rebuilt = String::new();
    let mut cursor = 0;
    for (source, target) in original.into_iter().zip(edited) {
        let x = &left[source.clone()];
        let y = &right[target];
        if x != y && (!same_pair(x, y, rename) || left.as_bytes()[source.start - 1] != b'{') {
            return false;
        }
        rebuilt.push_str(&left[cursor..source.start]);
        rebuilt.push_str(y);
        cursor = source.end;
    }
    rebuilt.push_str(&left[cursor..]);
    rebuilt == right
}

static NAMED_WIDTH: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(NAMED_WIDTH_PATTERN).ok());

fn captures(literal: &str) -> Vec<Range<usize>> {
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
) -> Result<BTreeSet<(usize, usize)>, RefusalCause> {
    let mut replacements = BTreeSet::new();
    for edit in &aligned.alignment.edits {
        let before = aligned.old_start + edit.before;
        let after = aligned.start + edit.after;
        if !scan::contains(&old_scope.ranges, before) {
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
