use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::TaggedFix;
use super::ValidatedFixSet;
use crate::reporting::AppliedFixCounts;

/// Writes every fix in `fixes` and returns the edits that actually landed, per
/// notice kind. A fix whose range no longer fits its file is skipped and
/// counted nowhere — the run must not claim an edit it did not write.
pub(in crate::fixes) fn apply_fixes(fixes: &ValidatedFixSet) -> Result<AppliedFixCounts> {
    let mut by_file: BTreeMap<&Path, Vec<&TaggedFix>> = BTreeMap::new();
    for tagged in fixes.tagged() {
        by_file
            .entry(tagged.fix.path.as_path())
            .or_default()
            .push(tagged);
    }
    let mut applied = AppliedFixCounts::default();
    for (path, mut file_fixes) in by_file {
        let mut text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        // Apply later edits first so earlier offsets remain valid. When two
        // fixes share a start offset (a replacement at [N..M] and an insertion
        // at [N..N]), apply the wider replacement first so [N..M] still
        // targets the original bytes.
        file_fixes.sort_by_key(|tagged| Reverse((tagged.fix.start, tagged.fix.end)));
        for tagged in file_fixes {
            let fix = &tagged.fix;
            if fix.end <= text.len() && fix.start <= fix.end {
                text.replace_range(fix.start..fix.end, &fix.replacement);
                if let Some(fix_kind) = tagged.fix_kind {
                    applied.record(fix_kind);
                }
            }
        }
        fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))?;
    }

    Ok(applied)
}

pub(in crate::fixes) fn snapshot_files(fixes: &ValidatedFixSet) -> Result<Vec<(PathBuf, String)>> {
    let mut unique_paths = BTreeSet::new();
    for fix in fixes.iter() {
        unique_paths.insert(fix.path.clone());
    }

    let mut snapshots = Vec::new();
    for path in unique_paths {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        snapshots.push((path, text));
    }
    Ok(snapshots)
}

pub(in crate::fixes) fn restore_files(snapshots: &[(PathBuf, String)]) -> Result<()> {
    for (path, text) in snapshots {
        fs::write(path, text).with_context(|| format!("failed to restore {}", path.display()))?;
    }
    Ok(())
}
