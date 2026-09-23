use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::FixScans;
use super::MendRunner;
use crate::fixes::imports;
use crate::fixes::imports::TaggedFix;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::fixes::prefer_module_import::PreferModuleImportScan;
use crate::reporting::FixKind;
use crate::reporting::MendFailure;

/// A fix's byte range within one file: `(path, start, end)`.
type FileRange<'a> = (&'a Path, usize, usize);

impl MendRunner<'_> {
    pub(super) fn combined_fixes(fix_scans: FixScans<'_>) -> Result<ValidatedFixSet, MendFailure> {
        let prefer_ranges: Vec<FileRange<'_>> = fix_scans
            .module_imports
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .map(file_range)
            .collect();
        // The deletion half of every imports-at-top move: the whole in-body
        // `use` line. The insertion half is an empty range at the scope top,
        // so `start < end` keeps only the deletions.
        let moved_use_ranges: Vec<FileRange<'_>> = fix_scans
            .imports_at_top
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .filter(|fix| fix.start < fix.end)
            .map(file_range)
            .collect();

        // Each pass tags its fixes with the notice they report under here,
        // where the pass is still known. The tag rides through the conflicting-
        // group drop and through dedup, so what the run announces is what it
        // wrote rather than what it scanned.
        let mut fixes = Vec::new();

        if let Some(scan) = fix_scans.imports {
            fixes.extend(tag(
                Some(FixKind::Import),
                scan.fixes.iter().filter(|fix| {
                    !overlaps_any(fix, &prefer_ranges) && !lies_inside_any(fix, &moved_use_ranges)
                }),
            ));
        }
        if let Some(scan) = fix_scans.module_imports {
            fixes.extend(tag(
                Some(FixKind::Import),
                module_import_fixes_outside_moved_uses(scan, &moved_use_ranges).into_iter(),
            ));
        }
        if let Some(scan) = fix_scans.inline_types {
            fixes.extend(tag(Some(FixKind::Import), scan.fixes.iter()));
        }
        if let Some(scan) = fix_scans.unused_pub {
            fixes.extend(tag(Some(FixKind::PubRemoval), scan.fixes.iter()));
        }
        if let Some(scan) = fix_scans.narrowed_pub {
            fixes.extend(tag(Some(FixKind::Narrowing), scan.fixes.iter()));
        }
        if let Some(scan) = fix_scans.restricted_annotation {
            fixes.extend(tag(Some(FixKind::Annotation), scan.fixes.iter()));
        }
        if let Some(scan) = fix_scans.field_visibility {
            fixes.extend(tag(Some(FixKind::FieldVisibility), scan.fixes.iter()));
        }
        if let Some(scan) = fix_scans.imports_at_top {
            fixes.extend(tag(Some(FixKind::Import), scan.fixes.iter()));
        }
        // `pub_use` tallies its own applied and skipped edits at scan time and
        // renders its own notice, so its fixes carry no kind and are counted
        // nowhere else.
        if let Some(scan) = fix_scans.pub_use {
            fixes.extend(tag(None, scan.fixes.iter()));
        }

        let fixes = drop_conflicting_import_groups(fixes);

        imports::ValidatedFixSet::try_from(fixes).map_err(MendFailure::Unexpected)
    }
}

/// Pairs each fix with the notice kind of the pass that proposed it.
fn tag<'a>(
    fix_kind: Option<FixKind>,
    fixes: impl Iterator<Item = &'a UseFix>,
) -> impl Iterator<Item = TaggedFix> {
    fixes.map(move |fix| TaggedFix {
        fix_kind,
        fix: fix.clone(),
    })
}

fn file_range(fix: &UseFix) -> FileRange<'_> { (fix.path.as_path(), fix.start, fix.end) }

fn overlaps_any(fix: &UseFix, ranges: &[FileRange<'_>]) -> bool {
    ranges.iter().any(|(path, start, end)| {
        fix.path.as_path() == *path && fix.start < *end && *start < fix.end
    })
}

fn lies_inside_any(fix: &UseFix, ranges: &[FileRange<'_>]) -> bool {
    ranges.iter().any(|(path, start, end)| {
        fix.path.as_path() == *path && *start <= fix.start && fix.end <= *end
    })
}

/// The prefer-module-import fixes that survive an imports-at-top move of the
/// same `use` line in the same run. A `use` rewrite whose range lies inside a
/// moved line is dropped together with every call-site rewrite in its
/// `ImportGroup`: keeping the `module::function()` call sites without the
/// `use module;` they depend on would not compile. The move keeps the
/// statement text, so the next run rewrites the line at the module top and the
/// call sites with it.
fn module_import_fixes_outside_moved_uses<'a>(
    scan: &'a PreferModuleImportScan,
    moved_use_ranges: &[FileRange<'_>],
) -> Vec<&'a UseFix> {
    let moved_groups: BTreeSet<(&Path, &str, &str)> = scan
        .fixes
        .iter()
        .filter(|fix| lies_inside_any(fix, moved_use_ranges))
        .filter_map(|fix| {
            fix.import_group.as_ref().map(|group| {
                (
                    fix.path.as_path(),
                    group.bare_name.as_str(),
                    group.full_path.as_str(),
                )
            })
        })
        .collect();
    scan.fixes
        .iter()
        .filter(|fix| {
            !lies_inside_any(fix, moved_use_ranges)
                && fix.import_group.as_ref().is_none_or(|group| {
                    !moved_groups.contains(&(
                        fix.path.as_path(),
                        group.bare_name.as_str(),
                        group.full_path.as_str(),
                    ))
                })
        })
        .collect()
}

/// Drops grouped import fixes that reserve the same bare name for different
/// full paths within one file. Untagged fixes pass through unchanged.
fn drop_conflicting_import_groups(fixes: Vec<TaggedFix>) -> Vec<TaggedFix> {
    let mut bare_name_to_paths: BTreeMap<(PathBuf, String), BTreeSet<String>> = BTreeMap::new();
    for tagged in &fixes {
        if let Some(group) = &tagged.fix.import_group {
            bare_name_to_paths
                .entry((tagged.fix.path.clone(), group.bare_name.clone()))
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
        .filter(|tagged| {
            tagged.fix.import_group.as_ref().is_none_or(|group| {
                !conflicting.contains(&(tagged.fix.path.clone(), group.bare_name.clone()))
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::FixScans;
    use super::MendRunner;
    use super::TaggedFix;
    use super::ValidatedFixSet;
    use super::drop_conflicting_import_groups;
    use crate::fixes::imports::ImportGroup;
    use crate::fixes::imports::ImportScan;
    use crate::fixes::imports::UseFix;
    use crate::fixes::imports_at_top::ImportsAtTopScan;
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

    /// Exercises the drop over plain `UseFix` values; the notice kind rides
    /// along untouched, so the tests only care about the fixes.
    fn drop_conflicts(fixes: Vec<UseFix>) -> Vec<UseFix> {
        let tagged = fixes
            .into_iter()
            .map(|fix| TaggedFix {
                fix_kind: None,
                fix,
            })
            .collect();
        drop_conflicting_import_groups(tagged)
            .into_iter()
            .map(|tagged| tagged.fix)
            .collect()
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
            imports:               Some(imports),
            module_imports:        Some(module_imports),
            inline_types:          None,
            unused_pub:            None,
            narrowed_pub:          None,
            restricted_annotation: None,
            field_visibility:      None,
            imports_at_top:        None,
            pub_use:               None,
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

    fn imports_at_top_scan(fixes: Vec<UseFix>) -> anyhow::Result<ImportsAtTopScan> {
        Ok(ImportsAtTopScan {
            findings: Vec::new(),
            fixes:    ValidatedFixSet::try_from(fixes)?,
        })
    }

    fn fix_scans_with_moved_uses<'a>(
        imports: Option<&'a ImportScan>,
        module_imports: Option<&'a PreferModuleImportScan>,
        imports_at_top: &'a ImportsAtTopScan,
    ) -> FixScans<'a> {
        FixScans {
            imports,
            module_imports,
            inline_types: None,
            unused_pub: None,
            narrowed_pub: None,
            restricted_annotation: None,
            field_visibility: None,
            imports_at_top: Some(imports_at_top),
            pub_use: None,
        }
    }

    /// The pair imports-at-top emits for an in-body
    /// `use crate::tool::helper::present;` at bytes 60..93: the line inserted
    /// at the module top and the deletion of the whole indented line.
    fn moved_present_use() -> anyhow::Result<ImportsAtTopScan> {
        let present_group = || {
            Some(ImportGroup {
                bare_name: "present".to_string(),
                full_path: "crate::tool::helper::present".to_string(),
            })
        };
        imports_at_top_scan(vec![
            range_fix(
                "src/lib.rs",
                0,
                0,
                "use crate::tool::helper::present;\n",
                present_group(),
            ),
            range_fix("src/lib.rs", 56, 94, "", present_group()),
        ])
    }

    fn helper_module_group() -> ImportGroup {
        ImportGroup {
            bare_name: "helper".to_string(),
            full_path: "crate::tool::helper".to_string(),
        }
    }

    fn combined_fix_set(fix_scans: FixScans<'_>) -> anyhow::Result<ValidatedFixSet> {
        MendRunner::combined_fixes(fix_scans).map_err(|err| anyhow::anyhow!("{err:?}"))
    }

    fn replacements(fixes: &ValidatedFixSet) -> Vec<&str> {
        fixes.iter().map(|fix| fix.replacement.as_str()).collect()
    }

    #[test]
    fn combined_fixes_drops_prefer_module_import_group_inside_imports_at_top_move()
    -> anyhow::Result<()> {
        let module_imports = module_import_scan(vec![
            range_fix(
                "src/lib.rs",
                60,
                93,
                "use crate::tool::helper;",
                Some(helper_module_group()),
            ),
            range_fix(
                "src/lib.rs",
                100,
                107,
                "helper::present",
                Some(helper_module_group()),
            ),
        ])?;
        let imports_at_top = moved_present_use()?;

        let fixes = combined_fix_set(fix_scans_with_moved_uses(
            None,
            Some(&module_imports),
            &imports_at_top,
        ))?;

        assert_eq!(
            replacements(&fixes),
            vec!["use crate::tool::helper::present;\n", ""],
            "the use rewrite and its call-site rewrite are both deferred to the next run"
        );
        Ok(())
    }

    #[test]
    fn combined_fixes_keeps_prefer_module_import_outside_imports_at_top_move() -> anyhow::Result<()>
    {
        let module_imports = module_import_scan(vec![range_fix(
            "src/lib.rs",
            10,
            20,
            "use crate::other;",
            Some(ImportGroup {
                bare_name: "other".to_string(),
                full_path: "crate::other".to_string(),
            }),
        )])?;
        let imports_at_top = moved_present_use()?;

        let fixes = combined_fix_set(fix_scans_with_moved_uses(
            None,
            Some(&module_imports),
            &imports_at_top,
        ))?;

        assert_eq!(
            replacements(&fixes),
            vec![
                "use crate::tool::helper::present;\n",
                "use crate::other;",
                ""
            ]
        );
        Ok(())
    }

    #[test]
    fn combined_fixes_drops_shorten_import_inside_imports_at_top_move() -> anyhow::Result<()> {
        let shorten_imports = import_scan(vec![range_fix(
            "src/lib.rs",
            60,
            93,
            "use super::helper::present;",
            None,
        )])?;
        let imports_at_top = moved_present_use()?;

        let fixes = combined_fix_set(fix_scans_with_moved_uses(
            Some(&shorten_imports),
            None,
            &imports_at_top,
        ))?;

        assert_eq!(
            replacements(&fixes),
            vec!["use crate::tool::helper::present;\n", ""]
        );
        Ok(())
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
        let result = drop_conflicts(fixes);
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
        let result = drop_conflicts(fixes);
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
        let result = drop_conflicts(fixes);
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
        let result = drop_conflicts(fixes);
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
        let result = drop_conflicts(fixes);
        assert_eq!(result.len(), 1);
        assert!(result[0].import_group.is_none());
    }
}
