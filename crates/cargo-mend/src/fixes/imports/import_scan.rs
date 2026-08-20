use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Error;
use anyhow::Result;
use anyhow::bail;

use crate::reporting::AppliedFixCounts;
use crate::reporting::Finding;
use crate::reporting::FixKind;

pub(in crate::fixes) struct ImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

/// Identifies a group of `UseFix`es that belong to a single "import + its
/// dependent rewrites" unit. When two passes independently propose imports
/// that would bind the same bare name to different full paths in the same
/// file, the combining layer drops every fix that carries a conflicting
/// `ImportGroup`, keeping rewrites and the `use` insertion in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::fixes) struct ImportGroup {
    /// The bare name that the `use` brings into scope (e.g. `Package`).
    pub bare_name: String,
    /// The full path the `use` resolves (e.g. `crate::project::Package`).
    pub full_path: String,
}

#[derive(Debug, Clone)]
pub(in crate::fixes) struct UseFix {
    pub path:         PathBuf,
    pub start:        usize,
    pub end:          usize,
    pub replacement:  String,
    /// When set, this fix is part of a larger group that must be kept or
    /// dropped together. See `ImportGroup`.
    pub import_group: Option<ImportGroup>,
}

/// A fix paired with the notice kind it reports under once it is written.
///
/// `fix_kind` is `None` for the `pub_use` pass, which tallies its own applied
/// and skipped edits at scan time and renders its own notice.
#[derive(Debug, Clone)]
pub(in crate::fixes) struct TaggedFix {
    pub fix_kind: Option<FixKind>,
    pub fix:      UseFix,
}

#[derive(Debug, Clone)]
pub(in crate::fixes) struct ValidatedFixSet {
    fixes: Vec<TaggedFix>,
}

impl ValidatedFixSet {
    pub(in crate::fixes) const fn is_empty(&self) -> bool { self.fixes.is_empty() }

    pub(in crate::fixes) fn iter(&self) -> impl Iterator<Item = &UseFix> {
        self.fixes.iter().map(|tagged| &tagged.fix)
    }

    pub(in crate::fixes) fn tagged(&self) -> impl Iterator<Item = &TaggedFix> { self.fixes.iter() }

    /// What applying this set would write, per notice kind. A dry run counts
    /// this rather than the scans' findings, because dedup and the
    /// conflicting-import-group drop both happen before a set exists.
    pub(in crate::fixes) fn counts(&self) -> AppliedFixCounts {
        let mut counts = AppliedFixCounts::default();
        for tagged in &self.fixes {
            if let Some(fix_kind) = tagged.fix_kind {
                counts.record(fix_kind);
            }
        }
        counts
    }
}

/// A scan's own fixes, before the runner combines them. Nothing writes these
/// directly — `runner::combine` tags every fix with the kind of the pass it came
/// from on the way into the applied set — so they carry no kind here.
impl TryFrom<Vec<UseFix>> for ValidatedFixSet {
    type Error = Error;

    fn try_from(fixes: Vec<UseFix>) -> Result<Self> {
        Self::try_from(
            fixes
                .into_iter()
                .map(|fix| TaggedFix {
                    fix_kind: None,
                    fix,
                })
                .collect::<Vec<_>>(),
        )
    }
}

impl TryFrom<Vec<TaggedFix>> for ValidatedFixSet {
    type Error = Error;

    fn try_from(mut fixes: Vec<TaggedFix>) -> Result<Self> {
        for tagged in &mut fixes {
            tagged.fix.path =
                fs::canonicalize(&tagged.fix.path).unwrap_or_else(|_| tagged.fix.path.clone());
        }
        fixes.sort_by(|left, right| {
            (
                &left.fix.path,
                left.fix.start,
                left.fix.end,
                &left.fix.replacement,
            )
                .cmp(&(
                    &right.fix.path,
                    right.fix.start,
                    right.fix.end,
                    &right.fix.replacement,
                ))
        });
        // Two passes proposing the byte-identical edit collapse to one. The sort
        // is stable, so the survivor — and the kind credited with it — is the one
        // `runner::combine` collected first.
        fixes.dedup_by(|left, right| {
            left.fix.path == right.fix.path
                && left.fix.start == right.fix.start
                && left.fix.end == right.fix.end
                && left.fix.replacement == right.fix.replacement
        });

        let mut by_file: BTreeMap<&Path, Vec<&UseFix>> = BTreeMap::new();
        for tagged in &fixes {
            by_file
                .entry(tagged.fix.path.as_path())
                .or_default()
                .push(&tagged.fix);
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use anyhow::Result;
    use tempfile::tempdir;

    use super::UseFix;
    use super::ValidatedFixSet;

    #[test]
    fn validated_fix_set_deduplicates_paths_to_same_file() -> Result<()> {
        let temp = tempdir()?;
        let fixtures_path = temp.path().join("fixtures.rs");
        fs::write(&fixtures_path, "pub const VALUE: usize = 1;\n")?;
        let aliases = ["src/../fixtures.rs", "examples/../fixtures.rs"];
        for directory in ["src", "examples"] {
            fs::create_dir(temp.path().join(directory))?;
        }
        let fixes: Vec<UseFix> = aliases
            .map(|path| UseFix {
                path:         temp.path().join(path),
                start:        0,
                end:          "pub ".len(),
                replacement:  String::new(),
                import_group: None,
            })
            .into_iter()
            .collect();

        let validated = ValidatedFixSet::try_from(fixes)?;
        let paths = validated
            .iter()
            .map(|fix| fix.path.as_path())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec![fs::canonicalize(fixtures_path)?]);
        Ok(())
    }

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
