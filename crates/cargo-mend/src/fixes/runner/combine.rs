use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::FixScans;
use super::MendRunner;
use crate::fixes::imports;
use crate::fixes::imports::ValidatedFixSet;
use crate::reporting::MendFailure;

impl MendRunner<'_> {
    pub(super) fn combined_fixes(fix_scans: FixScans<'_>) -> Result<ValidatedFixSet, MendFailure> {
        let prefer_ranges: Vec<(&Path, usize, usize)> = fix_scans
            .module_imports
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .map(|fix| (fix.path.as_path(), fix.start, fix.end))
            .collect();

        let mut fixes = Vec::new();

        if let Some(scan) = fix_scans.imports {
            for fix in scan.fixes.iter() {
                let overlaps = prefer_ranges.iter().any(|(path, start, end)| {
                    fix.path.as_path() == *path && fix.start < *end && *start < fix.end
                });
                if !overlaps {
                    fixes.push(fix.clone());
                }
            }
        }
        if let Some(scan) = fix_scans.module_imports {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.inline_types {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.unused_pub {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.narrowed_pub {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.field_visibility {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.imports_at_top {
            fixes.extend(scan.fixes.iter().cloned());
        }
        if let Some(scan) = fix_scans.pub_use {
            fixes.extend(scan.fixes.iter().cloned());
        }

        let fixes = drop_conflicting_import_groups(fixes);

        imports::ValidatedFixSet::try_from(fixes).map_err(MendFailure::Unexpected)
    }
}

/// Drops grouped import fixes that reserve the same bare name for different
/// full paths within one file. Untagged fixes pass through unchanged.
fn drop_conflicting_import_groups(fixes: Vec<imports::UseFix>) -> Vec<imports::UseFix> {
    let mut bare_name_to_paths: BTreeMap<(PathBuf, String), BTreeSet<String>> = BTreeMap::new();
    for fix in &fixes {
        if let Some(group) = &fix.import_group {
            bare_name_to_paths
                .entry((fix.path.clone(), group.bare_name.clone()))
                .or_default()
                .insert(group.full_path.clone());
        }
    }

    let conflicting: BTreeSet<(PathBuf, String)> = bare_name_to_paths
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(key, _)| key)
        .collect();

    if conflicting.is_empty() {
        return fixes;
    }

    fixes
        .into_iter()
        .filter(|fix| {
            fix.import_group.as_ref().is_none_or(|group| {
                !conflicting.contains(&(fix.path.clone(), group.bare_name.clone()))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::FixScans;
    use super::MendRunner;
    use super::ValidatedFixSet;
    use super::drop_conflicting_import_groups;
    use crate::fixes::imports::ImportGroup;
    use crate::fixes::imports::ImportScan;
    use crate::fixes::imports::UseFix;
    use crate::fixes::prefer_module_import::PreferModuleImportScan;

    fn tagged(path: &str, start: usize, replacement: &str, bare: &str, full: &str) -> UseFix {
        range_fix(
            path,
            start,
            start,
            replacement,
            Some(ImportGroup {
                bare_name: bare.to_string(),
                full_path: full.to_string(),
            }),
        )
    }

    fn untagged(path: &str, start: usize, replacement: &str) -> UseFix {
        range_fix(path, start, start, replacement, None)
    }

    fn range_fix(
        path: &str,
        start: usize,
        end: usize,
        replacement: &str,
        import_group: Option<ImportGroup>,
    ) -> UseFix {
        UseFix {
            path: PathBuf::from(path),
            start,
            end,
            replacement: replacement.to_string(),
            import_group,
        }
    }

    fn fix_scans_with_imports<'a>(
        imports: &'a ImportScan,
        module_imports: &'a PreferModuleImportScan,
    ) -> FixScans<'a> {
        FixScans {
            imports:          Some(imports),
            module_imports:   Some(module_imports),
            inline_types:     None,
            unused_pub:       None,
            narrowed_pub:     None,
            field_visibility: None,
            imports_at_top:   None,
            pub_use:          None,
        }
    }

    fn import_scan(fixes: Vec<UseFix>) -> anyhow::Result<ImportScan> {
        Ok(ImportScan {
            findings: Vec::new(),
            fixes:    ValidatedFixSet::try_from(fixes)?,
        })
    }

    fn module_import_scan(fixes: Vec<UseFix>) -> anyhow::Result<PreferModuleImportScan> {
        Ok(PreferModuleImportScan {
            findings: Vec::new(),
            fixes:    ValidatedFixSet::try_from(fixes)?,
        })
    }

    fn combined_fix_set(fix_scans: FixScans<'_>) -> anyhow::Result<ValidatedFixSet> {
        MendRunner::combined_fixes(fix_scans).map_err(|err| anyhow::anyhow!("{err:?}"))
    }

    #[test]
    fn combined_fixes_drops_shorten_import_when_prefer_module_import_overlaps() -> anyhow::Result<()>
    {
        let shorten_imports = import_scan(vec![range_fix(
            "src/lib.rs",
            10,
            20,
            "use super::Thing;",
            None,
        )])?;
        let module_imports = module_import_scan(vec![range_fix(
            "src/lib.rs",
            15,
            25,
            "use crate::module;",
            None,
        )])?;

        let fixes = combined_fix_set(fix_scans_with_imports(&shorten_imports, &module_imports))?;
        let replacements = fixes
            .iter()
            .map(|fix| fix.replacement.as_str())
            .collect::<Vec<_>>();

        assert_eq!(replacements, vec!["use crate::module;"]);
        Ok(())
    }

    #[test]
    fn combined_fixes_keeps_adjacent_shorten_import_and_prefer_module_import() -> anyhow::Result<()>
    {
        let shorten_imports = import_scan(vec![range_fix(
            "src/lib.rs",
            10,
            20,
            "use super::Thing;",
            None,
        )])?;
        let module_imports = module_import_scan(vec![range_fix(
            "src/lib.rs",
            20,
            30,
            "use crate::module;",
            None,
        )])?;

        let fixes = combined_fix_set(fix_scans_with_imports(&shorten_imports, &module_imports))?;
        let replacements = fixes
            .iter()
            .map(|fix| fix.replacement.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            replacements,
            vec!["use super::Thing;", "use crate::module;"]
        );
        Ok(())
    }

    #[test]
    fn no_conflicts_pass_through_unchanged() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::foo::Bar;\n",
                "Bar",
                "crate::foo::Bar",
            ),
            tagged("src/a.rs", 50, "Bar", "Bar", "crate::foo::Bar"),
            tagged(
                "src/a.rs",
                0,
                "use crate::foo::Baz;\n",
                "Baz",
                "crate::foo::Baz",
            ),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn same_bare_name_different_paths_drops_all_tagged() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged("src/a.rs", 50, "Package", "Package", "crate::a::Package"),
            tagged(
                "src/a.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
            tagged("src/a.rs", 75, "Package", "Package", "crate::b::Package"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert!(
            result.is_empty(),
            "conflicting-group fixes should all be dropped, got {result:?}"
        );
    }

    #[test]
    fn same_bare_name_same_full_path_kept() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged("src/a.rs", 50, "Package", "Package", "crate::a::Package"),
            tagged("src/a.rs", 80, "Package", "Package", "crate::a::Package"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn conflict_isolated_per_file() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged(
                "src/b.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn untagged_fixes_always_pass_through_even_with_conflicts() {
        let fixes = vec![
            tagged(
                "src/a.rs",
                0,
                "use crate::a::Package;\n",
                "Package",
                "crate::a::Package",
            ),
            tagged(
                "src/a.rs",
                0,
                "use crate::b::Package;\n",
                "Package",
                "crate::b::Package",
            ),
            untagged("src/a.rs", 100, "use super::other;"),
        ];
        let result = drop_conflicting_import_groups(fixes);
        assert_eq!(result.len(), 1);
        assert!(result[0].import_group.is_none());
    }
}
