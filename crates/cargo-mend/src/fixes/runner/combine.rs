use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use super::FixScans;
use super::MendRunner;
use super::function_length::FunctionLengthPlan;
use crate::fixes::imports;
use crate::fixes::imports::ImportPlan;
use crate::fixes::imports::TaggedFix;
use crate::fixes::imports::UseFix;
#[cfg(test)]
use crate::fixes::imports::ValidatedFixSet;
use crate::fixes::prefer_module_import::PreferModuleImportScan;
use crate::reporting::FixKind;
use crate::reporting::MendFailure;
use crate::rust_syntax::ParsedSources;

/// A fix's byte range within one file: `(path, start, end)`.
type FileRange<'a> = (&'a Path, usize, usize);

impl MendRunner<'_> {
    #[cfg(test)]
    pub(super) fn combined_fixes(
        fix_scans: FixScans<'_>,
        sources: &ParsedSources,
        package_roots: &[PathBuf],
    ) -> Result<ValidatedFixSet, MendFailure> {
        let workspace = package_roots
            .first()
            .map_or_else(|| Path::new("."), PathBuf::as_path);
        Self::combined_function_plan(fix_scans, sources, package_roots, workspace)
            .map(|plan| plan.fixes)
    }

    pub(super) fn combined_function_plan(
        fix_scans: FixScans<'_>,
        sources: &ParsedSources,
        package_roots: &[PathBuf],
        workspace: &Path,
    ) -> Result<FunctionLengthPlan, MendFailure> {
        FunctionLengthPlan::check(
            &Self::combined_proposals(fix_scans),
            &fix_scans
                .subtree_reexports
                .map(|scan| &scan.fixes)
                .into_iter()
                .chain(fix_scans.pub_use.map(|scan| &scan.fixes))
                .collect::<Vec<_>>(),
            sources,
            package_roots,
            workspace,
        )
        .map_err(MendFailure::Unexpected)
    }

    pub(super) fn combined_import_plan(
        fix_scans: FixScans<'_>,
        sources: &ParsedSources,
        package_roots: &[PathBuf],
    ) -> anyhow::Result<ImportPlan> {
        imports::plan_module_imports(Self::combined_proposals(fix_scans), sources, package_roots)
    }

    fn combined_proposals(fix_scans: FixScans<'_>) -> Vec<TaggedFix> {
        let prefer_ranges: Vec<FileRange<'_>> = fix_scans
            .module_imports
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .map(file_range)
            .collect();
        // `moved_use_ranges` collects the deletion half of every imports-at-top move: the whole
        // in-body `use` line. The insertion half is an empty range at the scope top,
        // so `start < end` keeps only the deletions.
        let moved_use_ranges: Vec<FileRange<'_>> = fix_scans
            .imports_at_top
            .iter()
            .flat_map(|scan| scan.fixes.iter())
            .filter(|fix| fix.start < fix.end)
            .map(file_range)
            .collect();

        // `tag` pairs each pass's fixes with `FixKind` in `TaggedFix` here,
        // where the pass is still known. The tag stays on each fix through the
        // conflicting-group drop and through dedup, so what the run announces
        // is what it wrote rather than what it scanned.
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
        // `FixScans::pub_use` tallies its own applied and skipped edits at scan time and
        // renders its own notice, so its fixes carry no kind and are counted
        // nowhere else.
        if let Some(scan) = fix_scans.pub_use {
            fixes.extend(tag(None, scan.fixes.iter()));
        }

        // `fixes_clear_of_ranges` gives `FixScans::subtree_reexports` every overlap: its owner
        // edit, caller rewrites, and ancestor re-export compile only together, while a fix
        // dropped from another pass is proposed again on the next convergence
        // pass.
        let mut fixes = fixes_clear_of_ranges(fixes, &subtree_reexport_ranges(fix_scans));
        if let Some(scan) = fix_scans.subtree_reexports {
            fixes.extend(tag(Some(FixKind::Import), scan.fixes.iter()));
        }

        drop_conflicting_import_groups(fixes)
    }
}

impl FixScans<'_> {
    /// Leaves `pub_use` out of this pass when any of its fixes overlaps a
    /// subtree re-export fix. Its parent rewrite and child narrowing compile
    /// only together, so the pass cannot lose single fixes the way the other
    /// passes do. Its notice and applied count leave with it, and the next
    /// convergence pass proposes it again.
    pub(super) fn without_pub_use_overlapping_subtree_reexports(self) -> Self {
        let subtree_ranges = subtree_reexport_ranges(self);
        let pub_use = self.pub_use.filter(|scan| {
            !scan
                .fixes
                .iter()
                .any(|fix| overlaps_any(fix, &subtree_ranges))
        });
        Self { pub_use, ..self }
    }
}

fn subtree_reexport_ranges(fix_scans: FixScans<'_>) -> Vec<FileRange<'_>> {
    fix_scans
        .subtree_reexports
        .iter()
        .flat_map(|scan| scan.fixes.iter())
        .map(file_range)
        .collect()
}

#[derive(Clone, Copy)]
enum FixOverlap {
    Overlapping,
    Clear,
}

impl FixOverlap {
    fn for_fix(fix: &UseFix, ranges: &[FileRange<'_>]) -> Self {
        if overlaps_any(fix, ranges) {
            Self::Overlapping
        } else {
            Self::Clear
        }
    }
}

#[derive(Clone, Copy)]
enum MovedUseRelation {
    Inside,
    Outside,
}

impl MovedUseRelation {
    fn for_fix(fix: &UseFix, ranges: &[FileRange<'_>]) -> Self {
        if lies_inside_any(fix, ranges) {
            Self::Inside
        } else {
            Self::Outside
        }
    }
}

/// Drops every fix that overlaps one of `ranges`, together with every fix
/// sharing an `ImportGroup` with a dropped one: a `use` rewrite kept without
/// its call-site rewrites, or an in-body `use` deleted without its insertion at
/// the scope top, would not compile. An insertion at `k` overlaps `[s, e)` only
/// when `s < k < e`.
fn fixes_clear_of_ranges(fixes: Vec<TaggedFix>, ranges: &[FileRange<'_>]) -> Vec<TaggedFix> {
    let classified: Vec<_> = fixes
        .into_iter()
        .map(|tagged| (FixOverlap::for_fix(&tagged.fix, ranges), tagged))
        .collect();
    let overlapped_groups: BTreeSet<(PathBuf, String, String)> = classified
        .iter()
        .filter_map(|(overlap, tagged)| match overlap {
            FixOverlap::Overlapping => Some(tagged),
            FixOverlap::Clear => None,
        })
        .filter_map(|tagged| {
            tagged.fix.import_group.as_ref().map(|group| {
                (
                    tagged.fix.path.clone(),
                    group.bare_name.clone(),
                    group.target_path().to_string(),
                )
            })
        })
        .collect();
    classified
        .into_iter()
        .filter_map(|(overlap, tagged)| match overlap {
            FixOverlap::Overlapping => None,
            FixOverlap::Clear => {
                let group_clear = tagged.fix.import_group.as_ref().is_none_or(|group| {
                    !overlapped_groups.contains(&(
                        tagged.fix.path.clone(),
                        group.bare_name.clone(),
                        group.target_path().to_string(),
                    ))
                });
                group_clear.then_some(tagged)
            },
        })
        .collect()
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
    let classified: Vec<_> = scan
        .fixes
        .iter()
        .map(|fix| (MovedUseRelation::for_fix(fix, moved_use_ranges), fix))
        .collect();
    let moved_groups: BTreeSet<(&Path, &str, &str)> = classified
        .iter()
        .filter_map(|(relation, fix)| match relation {
            MovedUseRelation::Inside => Some(*fix),
            MovedUseRelation::Outside => None,
        })
        .filter_map(|fix| {
            fix.import_group.as_ref().map(|group| {
                (
                    fix.path.as_path(),
                    group.bare_name.as_str(),
                    group.target_path(),
                )
            })
        })
        .collect();
    classified
        .into_iter()
        .filter_map(|(relation, fix)| match relation {
            MovedUseRelation::Inside => None,
            MovedUseRelation::Outside => {
                let group_clear = fix.import_group.as_ref().is_none_or(|group| {
                    !moved_groups.contains(&(
                        fix.path.as_path(),
                        group.bare_name.as_str(),
                        group.target_path(),
                    ))
                });
                group_clear.then_some(fix)
            },
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
                .insert(group.target_path().to_string());
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
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    use anyhow::Context;

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
    use crate::fixes::pub_use_fixes::PubUseFixScan;
    use crate::fixes::runner::function_length::FunctionLengthFixture;
    use crate::fixes::subtree_reexport::SubtreeReexportScan;

    #[test]
    fn function_length_keeps_local_import_and_rewrites_other_function() -> anyhow::Result<()> {
        let fixture = FunctionLengthFixture::new(
            concat!(
                "use crate::unusually_long_module_name::update;\n",
                "fn kept() {\n",
                "    use crate::auxiliary::VALUE;\n",
                "    std::hint::black_box(VALUE);\n",
                "    update(1111111111, 2222222222, 3333333333);\n",
                "    update(1111111111, 2222222222, 3333333333);\n",
                "}\n",
                "fn changed() {\n",
                "    update(1, 2, 3);\n",
                "}\n",
            ),
            5,
        )?;
        let selection = fixture.selection()?;
        let path = selection.manifest_dir.join("src/scenarios.rs");
        let sources = crate::rust_syntax::ParsedSources::default();
        let module_imports =
            crate::fixes::prefer_module_import::scan_selection(&selection, &sources)?;
        assert!(
            !module_imports.fixes.is_empty(),
            "the fixture has module rewrites"
        );
        let imports = import_scan(Vec::new())?;
        let moved = crate::fixes::imports_at_top::scan_selection(&selection, &sources)?;
        let fixes = MendRunner::combined_fixes(
            FixScans {
                imports_at_top: Some(&moved),
                ..fix_scans_with_imports(&imports, &module_imports)
            },
            &sources,
            &selection.package_roots,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        crate::fixes::imports::apply_fixes(&fixes)?;
        let rewritten = std::fs::read_to_string(&path)?;
        let kept = rewritten
            .split("fn kept() {")
            .nth(1)
            .context("kept function")?
            .split("fn changed()")
            .next()
            .context("kept body")?;
        assert!(
            kept.contains("use crate::auxiliary::VALUE;"),
            "the retained function keeps its local import: {rewritten}"
        );
        assert_eq!(kept.matches("    update(").count(), 2);
        assert!(rewritten.contains("unusually_long_module_name::update(1, 2, 3)"));
        syn::parse_file(&rewritten)?;
        Ok(())
    }

    #[test]
    fn retained_caller_keeps_its_subtree_reexport_dependency() -> anyhow::Result<()> {
        let fixture = FunctionLengthFixture::new(
            concat!(
                "use crate::unusually_long_module_name::update;\n",
                "fn retained() {\n",
                "    crate::owner::ping();\n",
                "    update(1111111111, 2222222222, 3333333333);\n",
                "}\nfn changed() { update(1, 2, 3); }\n",
            ),
            3,
        )?;
        let selection = fixture.selection()?;
        let root = &selection.manifest_dir;
        let lib = root.join("src/lib.rs");
        let report = fixture.subtree_report()?;
        let sources = crate::rust_syntax::ParsedSources::default();
        let module_imports =
            crate::fixes::prefer_module_import::scan_selection(&selection, &sources)?;
        let subtree =
            crate::fixes::subtree_reexport::scan_selection(&selection, &report, &sources, |_| {
                Ok(BTreeSet::default())
            })?;
        assert!(
            !subtree.fixes.is_empty(),
            "the real scanner relocates the re-export"
        );
        let plan = MendRunner::combined_function_plan(
            FixScans {
                module_imports: Some(&module_imports),
                subtree_reexports: Some(&subtree),
                ..FixScans::default()
            },
            &sources,
            &selection.package_roots,
            &selection.workspace_root,
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        crate::fixes::imports::apply_fixes(&plan.fixes)?;
        let caller = std::fs::read_to_string(root.join("src/scenarios.rs"))?;
        assert!(caller.contains("crate::owner::ping();"));
        assert!(
            std::fs::read_to_string(root.join("src/owner.rs"))?
                .contains("pub(crate) use crate::source::ping;")
        );
        assert!(std::fs::read_to_string(lib)?.contains("mod owner;"));
        assert!(caller.contains("unusually_long_module_name::update(1, 2, 3)"));
        Ok(())
    }

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

    /// Exercises the drop over plain `UseFix` values; the drop never reads the
    /// notice kind, so the tests only care about the fixes.
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
            subtree_reexports:     None,
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
            subtree_reexports: None,
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

    /// A subtree re-export pass whose one fix replaces bytes `10..20` of
    /// `src/lib.rs`.
    fn subtree_reexport_scan() -> anyhow::Result<SubtreeReexportScan> {
        Ok(SubtreeReexportScan {
            fixes:   ValidatedFixSet::try_from(vec![range_fix(
                "src/lib.rs",
                10,
                20,
                "use super::stage::f;",
                None,
            )])?,
            skipped: 0,
        })
    }

    fn fix_scans_with_subtree_reexports<'a>(
        imports: Option<&'a ImportScan>,
        module_imports: Option<&'a PreferModuleImportScan>,
        subtree_reexports: &'a SubtreeReexportScan,
    ) -> FixScans<'a> {
        FixScans {
            imports,
            module_imports,
            inline_types: None,
            unused_pub: None,
            narrowed_pub: None,
            restricted_annotation: None,
            field_visibility: None,
            imports_at_top: None,
            pub_use: None,
            subtree_reexports: Some(subtree_reexports),
        }
    }

    fn combined_fix_set(fix_scans: FixScans<'_>) -> anyhow::Result<ValidatedFixSet> {
        MendRunner::combined_fixes(
            fix_scans,
            &crate::rust_syntax::ParsedSources::default(),
            &[],
        )
        .map_err(|err| anyhow::anyhow!("{err:?}"))
    }

    fn replacements(fixes: &ValidatedFixSet) -> Vec<&str> {
        fixes.iter().map(|fix| fix.replacement.as_str()).collect()
    }

    #[test]
    fn overlap_filter_keeps_insertions_at_both_range_boundaries() {
        let fixes = [10, 15, 20].map(|offset| TaggedFix {
            fix_kind: None,
            fix:      untagged("src/lib.rs", offset, "insert"),
        });
        let ranges = [(std::path::Path::new("src/lib.rs"), 10, 20)];
        let remaining = super::fixes_clear_of_ranges(Vec::from(fixes), &ranges);
        assert_eq!(
            remaining
                .iter()
                .map(|tagged| tagged.fix.start)
                .collect::<Vec<_>>(),
            [10, 20]
        );
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
    fn crate_anchor_does_not_make_same_target_groups_conflict() {
        let fixes = vec![
            tagged("src/a.rs", 0, "use crate::helper;\n", "helper", "helper"),
            tagged(
                "src/a.rs",
                50,
                "use crate::helper::{self};\n",
                "helper",
                "crate::helper",
            ),
        ];
        let remaining = drop_conflicts(fixes);
        assert_eq!(
            remaining.len(),
            2,
            "both proposals must reach the shared import owner"
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

    #[test]
    fn subtree_reexport_fix_drops_an_insertion_inside_its_range() -> anyhow::Result<()> {
        let imports = import_scan(vec![untagged("src/lib.rs", 15, "use crate::x;\n")])?;
        let subtree_reexports = subtree_reexport_scan()?;

        let fixes = combined_fix_set(fix_scans_with_subtree_reexports(
            Some(&imports),
            None,
            &subtree_reexports,
        ))?;

        assert_eq!(replacements(&fixes), vec!["use super::stage::f;"]);
        Ok(())
    }

    #[test]
    fn subtree_reexport_fix_keeps_insertions_at_its_boundaries() -> anyhow::Result<()> {
        let imports = import_scan(vec![
            untagged("src/lib.rs", 10, "use crate::before;\n"),
            untagged("src/lib.rs", 20, "use crate::after;\n"),
        ])?;
        let subtree_reexports = subtree_reexport_scan()?;

        let fixes = combined_fix_set(fix_scans_with_subtree_reexports(
            Some(&imports),
            None,
            &subtree_reexports,
        ))?;

        assert_eq!(
            replacements(&fixes),
            vec![
                "use crate::before;\n",
                "use super::stage::f;",
                "use crate::after;\n",
            ]
        );
        Ok(())
    }

    #[test]
    fn subtree_reexport_fix_drops_an_overlapped_import_group_whole() -> anyhow::Result<()> {
        let module_imports = module_import_scan(vec![
            range_fix(
                "src/lib.rs",
                0,
                8,
                "use crate::tool::helper;",
                Some(helper_module_group()),
            ),
            range_fix(
                "src/lib.rs",
                12,
                18,
                "helper::present",
                Some(helper_module_group()),
            ),
        ])?;
        let subtree_reexports = subtree_reexport_scan()?;

        let fixes = combined_fix_set(fix_scans_with_subtree_reexports(
            None,
            Some(&module_imports),
            &subtree_reexports,
        ))?;

        assert_eq!(
            replacements(&fixes),
            vec!["use super::stage::f;"],
            "the call-site rewrite overlaps, so its `use` rewrite outside the range goes too"
        );
        Ok(())
    }

    #[test]
    fn pub_use_pass_is_left_out_whole_when_one_fix_overlaps_a_subtree_reexport_fix()
    -> anyhow::Result<()> {
        let pub_use = PubUseFixScan {
            fixes:   ValidatedFixSet::try_from(vec![
                untagged("src/lib.rs", 0, "pub(crate) use child::f;\n"),
                range_fix("src/lib.rs", 12, 18, "", None),
            ])?,
            applied: 1,
            skipped: 0,
        };
        let subtree_reexports = subtree_reexport_scan()?;
        let overlapping = FixScans {
            pub_use: Some(&pub_use),
            ..fix_scans_with_subtree_reexports(None, None, &subtree_reexports)
        };
        let clear = FixScans {
            subtree_reexports: None,
            ..overlapping
        };

        assert!(
            overlapping
                .without_pub_use_overlapping_subtree_reexports()
                .pub_use
                .is_none()
        );
        assert!(
            clear
                .without_pub_use_overlapping_subtree_reexports()
                .pub_use
                .is_some()
        );
        Ok(())
    }
}
