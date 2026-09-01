//! Whether a conflicted scoped replay still proves one reservation's integration.

use crate::ids::GitObjectId;
use crate::scope::ReservationScopeSet;
use crate::scope::ScopeKind;

/// Whether a conflicted replay is still usable for one reservation's proof.
pub(super) enum ScopedMergeConflictCoverage {
    /// Every reported conflict path lies outside the reservation scopes.
    OutsideReservationScopes,
    /// At least one reported conflict path is covered by the reservation.
    CoveredByReservation,
    /// Git moved a reserved file aside because the target replaced it with a directory.
    DisplacedReservedFile,
    /// Git's conflict records did not satisfy the documented `-z` record layout.
    Unreadable,
}

pub(super) fn scoped_merge_conflict_coverage(
    merge_tree_output: &[u8],
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> ScopedMergeConflictCoverage {
    let mut records = merge_tree_output.split(|byte| *byte == b'\0');
    let Some(tree_object_id) = records.next() else {
        return ScopedMergeConflictCoverage::Unreadable;
    };
    if tree_object_id.is_empty() {
        return ScopedMergeConflictCoverage::Unreadable;
    }

    let mut conflict_paths = Vec::new();
    let mut conflict_record_count = 0;
    loop {
        let Some(record) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if record.is_empty() {
            if conflict_record_count == 0 {
                return ScopedMergeConflictCoverage::Unreadable;
            }
            break;
        }
        conflict_record_count += 1;
        let Some(path_separator) = record.iter().position(|byte| *byte == b'\t') else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let metadata = &record[..path_separator];
        let path = &record[path_separator + 1..];
        if metadata.split(|byte| *byte == b' ').count() != 3 || path.is_empty() {
            return ScopedMergeConflictCoverage::Unreadable;
        }
        if scoped_merge_conflict_path_is_covered(path, scopes) {
            return ScopedMergeConflictCoverage::CoveredByReservation;
        }
        conflict_paths.push(path);
    }

    loop {
        let Some(path_count) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if path_count.is_empty() {
            return ScopedMergeConflictCoverage::OutsideReservationScopes;
        }
        let Ok(path_count) = str::from_utf8(path_count) else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let Ok(path_count) = path_count.parse::<usize>() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let mut message_paths = Vec::with_capacity(path_count);
        for _ in 0..path_count {
            let Some(path) = records.next() else {
                return ScopedMergeConflictCoverage::Unreadable;
            };
            if path.is_empty() {
                return ScopedMergeConflictCoverage::Unreadable;
            }
            message_paths.push(path);
        }
        let Some(conflict_type) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        let Some(message) = records.next() else {
            return ScopedMergeConflictCoverage::Unreadable;
        };
        if conflict_type.is_empty() || message.is_empty() {
            return ScopedMergeConflictCoverage::Unreadable;
        }
        if conflict_type == b"CONFLICT (file/directory)"
            && scoped_merge_displaced_reserved_file(
                &conflict_paths,
                &message_paths,
                scopes,
                protected_tip,
            )
        {
            return ScopedMergeConflictCoverage::DisplacedReservedFile;
        }
    }
}

fn scoped_merge_conflict_path_is_covered(
    conflict_path: &[u8],
    scopes: &ReservationScopeSet,
) -> bool {
    scopes.covers_path(conflict_path)
}

fn scoped_merge_displaced_reserved_file(
    conflict_paths: &[&[u8]],
    message_paths: &[&[u8]],
    scopes: &ReservationScopeSet,
    protected_tip: &GitObjectId,
) -> bool {
    scopes.as_slice().iter().any(|scope| {
        if scope.kind != ScopeKind::File {
            return false;
        }
        let reserved_path = scope.path.to_string();
        let displaced_path = format!("{reserved_path}~{protected_tip}");
        conflict_paths.contains(&displaced_path.as_bytes())
            && message_paths.contains(&reserved_path.as_bytes())
            && message_paths.contains(&displaced_path.as_bytes())
    })
}
