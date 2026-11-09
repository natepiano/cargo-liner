use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::Range;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use proc_macro2::LineColumn;

use super::formatter;
use super::formatter::Formatter;
use super::function_collector;
use super::function_collector::LengthExemption;
use crate::fixes::imports;
use crate::fixes::imports::ImportSource;
use crate::fixes::imports::TaggedFix;
use crate::fixes::imports::UseFix;
use crate::fixes::imports::ValidatedFixSet;
use crate::fixes::runner::FixScans;
use crate::reporting::ExecutionNotice;
use crate::reporting::NoticeKind;
use crate::reporting::Report;
use crate::rust_syntax::ParsedSources;

/// One decision supplies the accepted edits, report filter, and notice.
#[derive(Default)]
pub(in crate::fixes::runner) struct FunctionLengthPlan {
    pub(in crate::fixes::runner) fixes: ValidatedFixSet,
    retained:                           Vec<RetainedFunction>,
    dependencies:                       Vec<PreservedSource>,
    protection:                         LengthProtection,
}

struct PreservedSource {
    path:  PathBuf,
    range: Range<usize>,
}

struct RetainedFunction {
    source: PreservedSource,
    name:   String,
    lines:  usize,
    limit:  usize,
}

#[derive(Default)]
enum LengthProtection {
    #[default]
    Available,
    Unavailable {
        cause: String,
    },
}

impl PreservedSource {
    fn from_fix(fix: &UseFix) -> Self {
        Self {
            path:  fix.path.clone(),
            range: fix.start..fix.end,
        }
    }

    fn touches(&self, fix: &UseFix) -> bool { self.path == fix.path && touches(&self.range, fix) }
}

impl FunctionLengthPlan {
    pub(in crate::fixes::runner) fn check(
        proposals: &[TaggedFix],
        dependent_passes: &[&ValidatedFixSet],
        sources: &ParsedSources,
        package_roots: &[PathBuf],
        workspace: &Path,
    ) -> Result<Self> {
        if proposals.is_empty() {
            return Ok(Self::default());
        }
        let plan = imports::plan_module_imports(proposals.to_vec(), sources, package_roots)?;
        let fixes = ValidatedFixSet::try_from(plan.fixes)?;
        let paths = fixes.iter().map(|fix| &fix.path).collect::<BTreeSet<_>>();
        let changes_function =
            paths
                .into_iter()
                .try_fold(false, |changed, path| -> Result<bool> {
                    if changed || !path.is_file() {
                        return Ok(changed);
                    }
                    let original = std::fs::read_to_string(path)?;
                    let Ok(functions) = function_collector::collect(&original) else {
                        return Ok(true);
                    };
                    Ok(functions.iter().any(|function| {
                        fixes
                            .iter()
                            .any(|fix| fix.path == *path && touches(&function.range, fix))
                    }))
                })?;
        if !changes_function {
            return Ok(Self {
                fixes,
                ..Self::default()
            });
        }
        Self::with_formatter(
            proposals,
            dependent_passes,
            sources,
            package_roots,
            workspace,
            Formatter::detect(),
        )
    }

    fn with_formatter(
        proposals: &[TaggedFix],
        dependent_passes: &[&ValidatedFixSet],
        sources: &ParsedSources,
        package_roots: &[PathBuf],
        workspace: &Path,
        formatter: Formatter,
    ) -> Result<Self> {
        if let Formatter::Unavailable { cause } = formatter {
            let plan = imports::plan_module_imports(proposals.to_vec(), sources, package_roots)?;
            return Ok(Self {
                fixes: ValidatedFixSet::try_from(plan.fixes)?,
                protection: LengthProtection::Unavailable { cause },
                ..Self::default()
            });
        }
        let limit = formatter::threshold(workspace)?;
        let originals = proposals
            .iter()
            .filter(|tagged| tagged.fix.path.is_file())
            .map(|tagged| tagged.fix.path.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(|path| std::fs::read_to_string(&path).map(|text| (path, text)))
            .collect::<std::io::Result<BTreeMap<_, _>>>()?;
        let mut decision = Self::default();
        loop {
            // These passes move definitions and callers as one unit.
            decision.dependencies = dependent_passes
                .iter()
                .filter(|pass| {
                    pass.iter().any(|fix| {
                        decision
                            .retained
                            .iter()
                            .any(|kept| kept.source.touches(fix))
                    })
                })
                .flat_map(|pass| pass.iter().map(PreservedSource::from_fix))
                .collect();
            let preserved = decision.preserved_sources().collect::<Vec<_>>();
            let selected = preserve_dependencies(proposals, &originals, &preserved);
            let plan = imports::plan_module_imports(selected, sources, package_roots)?;
            let fixes = ValidatedFixSet::try_from(plan.fixes)?;
            let added =
                decision.measure_functions(&originals, &fixes, &formatter, workspace, limit)?;
            if added.is_empty() {
                decision.fixes = fixes;
                break;
            }
            decision.retained.extend(added);
        }
        let preserved = decision.preserved_sources().collect::<Vec<_>>();
        let dependencies = proposals
            .iter()
            .filter_map(|tagged| {
                let fix = &tagged.fix;
                (group_is_retained(fix, proposals, &preserved)
                    && originals
                        .get(&fix.path)
                        .and_then(|text| text.get(fix.start..fix.end))
                        .is_some_and(is_import))
                .then(|| PreservedSource::from_fix(fix))
            })
            .collect::<Vec<_>>();
        decision.dependencies.extend(dependencies);
        Ok(decision)
    }

    fn measure_functions(
        &self,
        originals: &BTreeMap<PathBuf, String>,
        fixes: &ValidatedFixSet,
        formatter: &Formatter,
        workspace: &Path,
        limit: usize,
    ) -> Result<Vec<RetainedFunction>> {
        let mut added = Vec::new();
        for (path, original) in originals {
            let Ok(before) = function_collector::collect(original) else {
                continue;
            };
            let candidates = before
                .iter()
                .enumerate()
                .filter(|(_, function)| {
                    function.lines <= limit
                        && matches!(function.length_exemption, LengthExemption::Limited)
                        && !self.retained.iter().any(|kept| {
                            kept.source.path == *path && kept.source.range == function.range
                        })
                        && fixes
                            .iter()
                            .any(|fix| fix.path == *path && touches(&function.range, fix))
                })
                .collect::<Vec<_>>();
            if candidates.is_empty() {
                continue;
            }
            let file_fixes = ValidatedFixSet::try_from(
                fixes
                    .tagged()
                    .filter(|tagged| tagged.fix.path == *path)
                    .cloned()
                    .collect::<Vec<_>>(),
            )?;
            let rewritten = file_fixes.rewritten_files()?;
            let Some(proposed) = rewritten.files.get(path) else {
                continue;
            };
            let after =
                function_collector::collect(&formatter.formatted(proposed, path, workspace)?)?;
            for (index, function) in candidates {
                if let Some(formatted) = after.get(index)
                    && formatted.name == function.name
                    && formatted.lines > limit
                {
                    added.push(RetainedFunction {
                        source: PreservedSource {
                            path:  path.clone(),
                            range: function.range.clone(),
                        },
                        name: function.name.clone(),
                        lines: formatted.lines,
                        limit,
                    });
                }
            }
        }
        Ok(added)
    }

    /// Keep pass accounting consistent with edits that survive retention.
    pub(in crate::fixes::runner) fn accepted_scans<'a>(&self, scans: FixScans<'a>) -> FixScans<'a> {
        FixScans {
            pub_use: scans
                .pub_use
                .filter(|scan| !self.pass_is_retained(&scan.fixes)),
            subtree_reexports: scans
                .subtree_reexports
                .filter(|scan| !self.pass_is_retained(&scan.fixes)),
            ..scans
        }
    }

    fn pass_is_retained(&self, fixes: &ValidatedFixSet) -> bool {
        let preserved = self.preserved_sources().collect::<Vec<_>>();
        fixes.iter().any(|fix| is_preserved(fix, &preserved))
    }

    pub(in crate::fixes::runner) fn filter_report(
        &self,
        report: &mut Report,
        analysis_root: &Path,
    ) -> Result<()> {
        let mut text = BTreeMap::new();
        for source in self.preserved_sources() {
            if !text.contains_key(&source.path) {
                text.insert(source.path.clone(), std::fs::read_to_string(&source.path)?);
            }
        }
        report.findings.retain(|finding| {
            let path = analysis_root.join(&finding.path);
            let path = std::fs::canonicalize(&path).unwrap_or(path);
            let Some(source) = text.get(&path) else {
                return true;
            };
            let position = LineColumn {
                line:   finding.line,
                column: finding.column.saturating_sub(1),
            };
            let index = ImportSource::new(source);
            let Some(location) = index.locate(position, position) else {
                return true;
            };
            !self
                .preserved_sources()
                .any(|kept| kept.path == path && kept.range.contains(&location.start()))
        });
        report.refresh_summary();
        Ok(())
    }

    pub(in crate::fixes::runner) fn notice(&self) -> Option<ExecutionNotice> {
        let mut kinds = self
            .retained
            .iter()
            .map(|kept| NoticeKind::FunctionRetained {
                path:     kept.source.path.clone(),
                function: kept.name.clone(),
                lines:    kept.lines,
                limit:    kept.limit,
            })
            .collect::<Vec<_>>();
        if let LengthProtection::Unavailable { cause } = &self.protection {
            kinds.push(NoticeKind::FunctionLengthUnavailable {
                cause: cause.clone(),
            });
        }
        if kinds.is_empty() {
            None
        } else {
            Some(ExecutionNotice::from(kinds))
        }
    }

    pub(in crate::fixes::runner) fn merge_notice(
        &self,
        notice: Option<ExecutionNotice>,
    ) -> Option<ExecutionNotice> {
        match (notice, self.notice()) {
            (Some(mut current), Some(additional)) => {
                current.merge(additional);
                Some(current)
            },
            (current @ Some(_), None) | (None, current) => current,
        }
    }

    fn preserved_sources(&self) -> impl Iterator<Item = &PreservedSource> {
        self.retained
            .iter()
            .map(|kept| &kept.source)
            .chain(&self.dependencies)
    }
}

const fn touches(range: &Range<usize>, fix: &UseFix) -> bool {
    if fix.start == fix.end {
        (range.start == fix.start && range.end == fix.end)
            || (range.start < fix.start && fix.end < range.end)
    } else {
        fix.start < range.end && range.start < fix.end
    }
}

fn is_import(text: &str) -> bool { syn::parse_str::<syn::ItemUse>(text.trim()).is_ok() }

fn is_preserved(fix: &UseFix, preserved: &[&PreservedSource]) -> bool {
    preserved.iter().any(|source| source.touches(fix))
}

fn same_import_group(left: &UseFix, right: &UseFix) -> bool {
    left.path == right.path
        && left
            .import_group
            .as_ref()
            .zip(right.import_group.as_ref())
            .is_some_and(|(left, right)| {
                left.bare_name == right.bare_name && left.target_path() == right.target_path()
            })
}

fn group_is_retained(
    fix: &UseFix,
    proposals: &[TaggedFix],
    preserved: &[&PreservedSource],
) -> bool {
    proposals.iter().any(|proposal| {
        same_import_group(fix, &proposal.fix) && is_preserved(&proposal.fix, preserved)
    })
}

/// Keep original imports while other functions still use a rewritten module path.
fn preserve_dependencies(
    proposals: &[TaggedFix],
    originals: &BTreeMap<PathBuf, String>,
    preserved: &[&PreservedSource],
) -> Vec<TaggedFix> {
    proposals
        .iter()
        .filter_map(|tagged| {
            let fix = &tagged.fix;
            if is_preserved(fix, preserved) {
                return None;
            }
            let mut selected = tagged.clone();
            if group_is_retained(fix, proposals, preserved) {
                let original = originals.get(&fix.path)?.get(fix.start..fix.end)?;
                if is_import(original) {
                    if fix.replacement.trim().is_empty()
                        || !has_surviving_reference(fix, proposals, originals, preserved)
                    {
                        return None;
                    }
                    selected.fix.replacement = format!("{original}\n{}", fix.replacement);
                } else if fix.start == fix.end
                    && is_import(&fix.replacement)
                    && !has_surviving_reference(fix, proposals, originals, preserved)
                {
                    return None;
                }
            }
            Some(selected)
        })
        .collect()
}

fn has_surviving_reference(
    fix: &UseFix,
    proposals: &[TaggedFix],
    originals: &BTreeMap<PathBuf, String>,
    preserved: &[&PreservedSource],
) -> bool {
    proposals.iter().any(|other| {
        let other = &other.fix;
        other.start < other.end
            && same_import_group(fix, other)
            && !is_preserved(other, preserved)
            && originals
                .get(&other.path)
                .and_then(|text| text.get(other.start..other.end))
                .is_some_and(|text| !is_import(text))
    })
}

#[cfg(test)]
mod tests {
    use anyhow::Context;

    use crate::fixes::runner::function_length::FunctionLengthFixture;
    use crate::fixes::runner::function_length::constants::CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD;
    use crate::reporting::Report;
    #[test]
    fn function_length_notice_and_report_stay_stable_after_apply() -> anyhow::Result<()> {
        let fixture = FunctionLengthFixture::new(
            concat!(
                "use crate::unusually_long_module_name::update;\n",
                "fn retained() {\n",
                "    // The blank and comment lines do not count.\n\n",
                "    update(1111111111, 2222222222, 3333333333);\n",
                "}\nfn changed() { update(1, 2, 3); }\n",
            ),
            2,
        )?;
        let (first, report) = fixture.plan()?;
        assert_eq!(
            report.summary.warnings,
            0,
            "findings: {:?}; notice: {:?}",
            report.findings,
            first.notice()
        );
        assert!(report.findings.is_empty());
        let notice = first.notice().context("retained function notice")?.render();
        assert!(notice.contains("src/scenarios.rs::retained"));
        assert!(
            notice.contains("3/2 body lines; split the function"),
            "{notice}"
        );
        crate::fixes::imports::apply_fixes(&first.fixes)?;
        let (second, report) = fixture.plan()?;
        assert!(
            second.fixes.is_empty(),
            "the second fix does not change the file"
        );
        assert!(report.findings.is_empty());
        assert_eq!(second.notice().context("repeat notice")?.render(), notice);
        Ok(())
    }

    #[test]
    fn function_length_exempts_allowed_expected_and_already_long_functions() -> anyhow::Result<()> {
        let fixture = FunctionLengthFixture::new(
            concat!(
                "use crate::unusually_long_module_name::update;\n",
                "#[allow(clippy::too_many_lines)]\n",
                "fn allowed() { update(1111111111, 2222222222, 3333333333); }\n",
                "#[expect(clippy::too_many_lines)]\n",
                "fn expected() { update(1111111111, 2222222222, 3333333333); }\n",
                "fn already_long() {\n",
                "    std::hint::black_box(1);\n    std::hint::black_box(2);\n",
                "    std::hint::black_box(3);\n    update(1111111111, 2222222222, 3333333333);\n}\n",
            ),
            3,
        )?;
        let (plan, report) = fixture.plan()?;
        assert!(plan.notice().is_none());
        assert!(!report.findings.is_empty());
        crate::fixes::imports::apply_fixes(&plan.fixes)?;
        let text = std::fs::read_to_string(fixture.0.path().join("src/scenarios.rs"))?;
        assert_eq!(
            text.matches("unusually_long_module_name::update(").count(),
            3
        );
        Ok(())
    }

    #[test]
    fn function_length_uses_clippy_default_without_a_workspace_override() -> anyhow::Result<()> {
        let limit = CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD;
        let calls = "    update(1111111111, 2222222222, 3333333333);\n".repeat(limit / 3 + 1);
        let text = format!(
            "use crate::unusually_long_module_name::update;\nfn retained() {{\n{calls}}}\n"
        );
        let fixture = FunctionLengthFixture::new(&text, limit)?;
        std::fs::remove_file(fixture.0.path().join("clippy.toml"))?;
        let (plan, report) = fixture.plan()?;
        assert!(
            plan.fixes.is_empty(),
            "fixes: {:?}; notice: {:?}",
            plan.fixes,
            plan.notice()
        );
        assert!(report.findings.is_empty());
        let notice = plan.notice().context("default threshold notice")?.render();
        assert!(
            notice.contains(&format!("{}/{limit} body lines", (limit / 3 + 1) * 3)),
            "{notice}"
        );
        Ok(())
    }

    #[test]
    fn missing_formatter_keeps_fixes_and_report_with_one_notice() -> anyhow::Result<()> {
        let fixture = FunctionLengthFixture::new(
            concat!(
                "use crate::unusually_long_module_name::update;\n",
                "fn changed() { update(1, 2, 3); }\n",
            ),
            1,
        )?;
        let selection = fixture.selection()?;
        let sources = crate::rust_syntax::ParsedSources::default();
        let scan = crate::fixes::prefer_module_import::scan_selection(&selection, &sources)?;
        let proposals = scan.fixes.tagged().cloned().collect::<Vec<_>>();
        let mut report = Report {
            findings: scan.findings,
            ..Default::default()
        };
        let count = report.findings.len();
        let plan = super::FunctionLengthPlan::with_formatter(
            &proposals,
            &[],
            &sources,
            &selection.package_roots,
            &selection.workspace_root,
            super::Formatter::Unavailable {
                cause: "rustup and rustfmt are absent".to_string(),
            },
        )?;
        plan.filter_report(&mut report, &selection.analysis_root)?;
        assert_eq!(report.findings.len(), count);
        assert!(!plan.fixes.is_empty());
        crate::fixes::imports::apply_fixes(&plan.fixes)?;
        assert!(
            std::fs::read_to_string(selection.manifest_dir.join("src/scenarios.rs"))?
                .contains("unusually_long_module_name::update(1, 2, 3)")
        );
        let notice = plan
            .merge_notice(plan.notice())
            .context("missing formatter notice")?
            .render();
        assert_eq!(
            notice
                .matches("function length protection is unavailable")
                .count(),
            1
        );
        assert!(notice.contains("rustup and rustfmt are absent"));
        Ok(())
    }
}
