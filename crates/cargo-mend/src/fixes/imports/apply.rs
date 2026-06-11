use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;

use super::UseFix;
use super::ValidatedFixSet;

pub fn apply_fixes(fixes: &ValidatedFixSet) -> Result<usize> {
    let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
    for fix in fixes.iter() {
        by_file.entry(fix.path.as_path()).or_default().push(fix);
    }
    let mut applied = 0usize;
    for (path, mut file_fixes) in by_file {
        let mut text = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        // Apply later edits first so earlier offsets remain valid. When two
        // fixes share a start offset (a replacement at [N..M] and an insertion
        // at [N..N]), apply the wider replacement first so [N..M] still
        // targets the original bytes.
        file_fixes.sort_by_key(|fix| Reverse((fix.start, fix.end)));
        for fix in file_fixes {
            if fix.end <= text.len() && fix.start <= fix.end {
                text.replace_range(fix.start..fix.end, &fix.replacement);
                applied += 1;
            }
        }
        fs::write(path, text).with_context(|| format!("failed to write {}", path.display()))?;
    }

    Ok(applied)
}

pub fn snapshot_files(fixes: &ValidatedFixSet) -> Result<Vec<(PathBuf, String)>> {
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

pub fn restore_files(snapshots: &[(PathBuf, String)]) -> Result<()> {
    for (path, text) in snapshots {
        fs::write(path, text).with_context(|| format!("failed to restore {}", path.display()))?;
    }
    Ok(())
}
