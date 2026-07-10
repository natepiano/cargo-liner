use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Error;
use anyhow::Result;
use anyhow::bail;

use crate::reporting::Finding;

pub struct ImportScan {
    pub findings: Vec<Finding>,
    pub fixes:    ValidatedFixSet,
}

/// Identifies a group of `UseFix`es that belong to a single "import + its
/// dependent rewrites" unit. When two passes independently propose imports
/// that would bind the same bare name to different full paths in the same
/// file, the combining layer drops every fix that carries a conflicting
/// `ImportGroup`, keeping rewrites and the `use` insertion in sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportGroup {
    /// The bare name that the `use` brings into scope (e.g. `Package`).
    pub bare_name: String,
    /// The full path the `use` resolves (e.g. `crate::project::Package`).
    pub full_path: String,
}

#[derive(Debug, Clone)]
pub struct UseFix {
    pub path:         PathBuf,
    pub start:        usize,
    pub end:          usize,
    pub replacement:  String,
    /// When set, this fix is part of a larger group that must be kept or
    /// dropped together. See `ImportGroup`.
    pub import_group: Option<ImportGroup>,
}

#[derive(Debug, Clone)]
pub struct ValidatedFixSet {
    fixes: Vec<UseFix>,
}

impl ValidatedFixSet {
    pub const fn is_empty(&self) -> bool { self.fixes.is_empty() }

    pub fn iter(&self) -> impl Iterator<Item = &UseFix> { self.fixes.iter() }
}

impl TryFrom<Vec<UseFix>> for ValidatedFixSet {
    type Error = Error;

    fn try_from(mut fixes: Vec<UseFix>) -> Result<Self> {
        for fix in &mut fixes {
            fix.path = fs::canonicalize(&fix.path).unwrap_or_else(|_| fix.path.clone());
        }
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
