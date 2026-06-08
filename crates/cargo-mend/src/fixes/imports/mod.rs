mod path;
mod scan;

use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use anyhow::bail;

use crate::reporting::Finding;
use crate::selection::Selection;

pub(super) struct ImportScan {
    pub(super) findings: Vec<Finding>,
    pub(super) fixes:    ValidatedFixSet,
}

/// Identifies a group of `UseFix`es that belong to a single "import + its
/// dependent rewrites" unit. When two passes independently propose imports
/// that would bind the same bare name to different full paths in the same
/// file, the combining layer drops every fix that carries a conflicting
/// `ImportGroup`, keeping rewrites and the `use` insertion in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ImportGroup {
    /// The bare name that the `use` brings into scope (e.g. `Package`).
    pub(super) bare_name: String,
    /// The full path the `use` resolves (e.g. `crate::project::Package`).
    pub(super) full_path: String,
}

#[derive(Debug, Clone)]
pub(super) struct UseFix {
    pub(super) path:         PathBuf,
    pub(super) start:        usize,
    pub(super) end:          usize,
    pub(super) replacement:  String,
    /// When set, this fix is part of a larger group that must be kept or
    /// dropped together. See `ImportGroup`.
    pub(super) import_group: Option<ImportGroup>,
}

#[derive(Debug, Clone)]
pub(super) struct ValidatedFixSet {
    fixes: Vec<UseFix>,
}

impl TryFrom<Vec<UseFix>> for ValidatedFixSet {
    type Error = Error;

    fn try_from(mut fixes: Vec<UseFix>) -> Result<Self> {
        fixes.sort_by(|left, right| {
            (&left.path, left.start, left.end, &left.replacement).cmp(&(
                &right.path,
                right.start,
                right.end,
                &right.replacement,
            ))
        });
        fixes.dedup_by(|left, right| {
            left.path == right.path
                && left.start == right.start
                && left.end == right.end
                && left.replacement == right.replacement
        });

        let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
        for fix in &fixes {
            by_file.entry(fix.path.as_path()).or_default().push(fix);
        }

        for (path, mut file_fixes) in by_file {
            file_fixes.sort_by_key(|fix| (fix.start, fix.end));
            let mut previous_fix: Option<&UseFix> = None;
            for fix in file_fixes {
                if fix.start > fix.end {
                    bail!(
                        "invalid fix range {}..{} for {}",
                        fix.start,
                        fix.end,
                        path.display()
                    );
                }
                if let Some(previous) = previous_fix
                    && fix.start < previous.end
                {
                    bail!(
                        "overlapping fixes detected for {}: {}..{} ({:?}) overlaps {}..{} ({:?})",
                        path.display(),
                        previous.start,
                        previous.end,
                        previous.replacement,
                        fix.start,
                        fix.end,
                        fix.replacement
                    );
                }
                previous_fix = Some(fix);
            }
        }

        Ok(Self { fixes })
    }
}

impl ValidatedFixSet {
    pub(super) const fn is_empty(&self) -> bool { self.fixes.is_empty() }

    pub(super) fn iter(&self) -> impl Iterator<Item = &UseFix> { self.fixes.iter() }
}

pub(super) fn scan_selection(selection: &Selection) -> Result<ImportScan> {
    scan::scan_selection(selection)
}

pub(super) fn apply_fixes(fixes: &ValidatedFixSet) -> Result<usize> {
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

pub(super) fn snapshot_files(fixes: &ValidatedFixSet) -> Result<Vec<(PathBuf, String)>> {
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

pub(super) fn restore_files(snapshots: &[(PathBuf, String)]) -> Result<()> {
    for (path, text) in snapshots {
        fs::write(path, text).with_context(|| format!("failed to restore {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::UseFix;
    use super::ValidatedFixSet;

    #[test]
    fn validated_fix_set_allows_adjacent_non_overlapping_ranges() {
        let path = PathBuf::from("src/lib.rs");
        let fixes = vec![
            UseFix {
                path:         path.clone(),
                start:        100,
                end:          110,
                replacement:  "first".to_string(),
                import_group: None,
            },
            UseFix {
                path,
                start: 110,
                end: 120,
                replacement: "second".to_string(),
                import_group: None,
            },
        ];

        let validated_result = ValidatedFixSet::try_from(fixes);
        assert!(
            validated_result.is_ok(),
            "adjacent edits should be valid: {}",
            validated_result
                .as_ref()
                .err()
                .map_or_else(String::new, |err| format!("{err:#}"))
        );
        let Ok(validated) = validated_result else {
            return;
        };
        assert_eq!(validated.iter().count(), 2);
    }
}
