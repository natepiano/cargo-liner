use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use regex::Regex;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::parse_file;
use syn::spanned::Spanned;
use syn::visit::Visit;

use super::diagnostics::Report;
use super::imports::UseFix;
use super::imports::ValidatedFixSet;
use super::module_paths::file_module_path;
use super::selection::Selection;

pub struct PubUseFixScan {
    pub fixes:         ValidatedFixSet,
    pub applied_count: usize,
    pub skipped_count: usize,
}

struct PubUseFixFact {
    child_file:      PathBuf,
    child_line:      usize,
    child_item_name: String,
    parent_mod:      PathBuf,
    parent_line:     usize,
    child_module:    String,
}

struct PubUseCandidate {
    child_file:       PathBuf,
    child_line:       usize,
    child_module:     String,
    exported_name:    String,
    parent_mod_path:  Vec<String>,
    target_item_path: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ParentBoundaryKey {
    parent_mod: PathBuf,
    item_start: usize,
    item_end:   usize,
}

struct ValidatedPubUsePlan {
    parent_boundary:  ParentBoundaryKey,
    child_file:       PathBuf,
    child_module:     String,
    exported_name:    String,
    parent_mod_path:  Vec<String>,
    target_item_path: Vec<String>,
    child_narrowing:  UseFix,
}

struct PubUseAnalysis {
    supported_plans: Vec<ValidatedPubUsePlan>,
    skipped_count:   usize,
}

enum CandidateScreening {
    Accept(PubUseCandidate),
    Skip,
}

pub fn scan_selection(selection: &Selection, report: &Report) -> Result<PubUseFixScan> {
    let mut fixes = Vec::new();
    let facts = collect_pub_use_fix_facts(selection, report);
    let analysis = analyze_pub_use_candidates(&facts)?;
    let parent_fix_groups = group_parent_pub_use_plans(analysis.supported_plans.iter());

    for plan in &analysis.supported_plans {
        fixes.push(plan.child_narrowing.clone());
    }

    for (parent_boundary, exports) in parent_fix_groups {
        let removal = build_parent_pub_use_edit_for_exports(&parent_boundary, &exports)?;
        fixes.push(removal);
    }

    fixes.extend(rewrite_subtree_imports_for_plans(
        selection,
        &analysis.supported_plans,
    )?);
    let fixes = ValidatedFixSet::from_vec(fixes)?;

    Ok(PubUseFixScan {
        fixes,
        applied_count: analysis.supported_plans.len(),
        skipped_count: analysis.skipped_count,
    })
}

fn collect_pub_use_fix_facts(selection: &Selection, report: &Report) -> Vec<PubUseFixFact> {
    let mut facts = Vec::new();
    for fact in report.facts.pub_use.iter() {
        let child_rel = normalize_rel_path(&fact.child_path);
        let parent_rel = normalize_rel_path(&fact.parent_path);
        facts.push(PubUseFixFact {
            child_file:      selection.analysis_root.join(&child_rel),
            child_line:      fact.child_line,
            child_item_name: fact.child_item_name.clone(),
            parent_mod:      selection.analysis_root.join(&parent_rel),
            parent_line:     fact.parent_line,
            child_module:    fact.child_module.clone(),
        });
    }

    facts
}

fn analyze_pub_use_candidates(facts: &[PubUseFixFact]) -> Result<PubUseAnalysis> {
    let mut supported_plans = Vec::new();
    let mut skipped_count = 0usize;
    for fact in facts {
        let child_source = fs::read_to_string(&fact.child_file)
            .with_context(|| format!("failed to read {}", fact.child_file.display()))?;
        let parent_source = fs::read_to_string(&fact.parent_mod)
            .with_context(|| format!("failed to read {}", fact.parent_mod.display()))?;
        let Some(parent_export) = resolve_parent_pub_use_export(
            &parent_source,
            fact.parent_line,
            &fact.child_module,
            &fact.child_item_name,
        )
        .with_context(|| {
            format!(
                "failed to resolve exported item from {}:{}",
                fact.parent_mod.display(),
                fact.parent_line
            )
        })?
        else {
            skipped_count += 1;
            continue;
        };

        let src_root = find_src_root(&fact.parent_mod)
            .context("failed to determine src root for parent module")?;

        let parent_mod_path = module_path_from_boundary_file(&src_root, &fact.parent_mod)
            .context("failed to determine parent module path")?;
        let mut target_item_path = parent_mod_path.clone();
        target_item_path.push(fact.child_module.clone());
        target_item_path.push(fact.child_item_name.clone());

        let parent_boundary = ParentBoundaryKey {
            parent_mod: fact.parent_mod.clone(),
            ..parent_export.parent_boundary
        };
        let candidate = PubUseCandidate {
            child_file: fact.child_file.clone(),
            child_line: fact.child_line,
            child_module: fact.child_module.clone(),
            exported_name: parent_export.exported_name,
            parent_mod_path,
            target_item_path,
        };
        match screen_candidate(candidate, &fact.child_item_name, &child_source)? {
            CandidateScreening::Accept(candidate) => {
                supported_plans.push(build_validated_plan(candidate, parent_boundary)?);
            },
            CandidateScreening::Skip => {},
        }
    }

    Ok(PubUseAnalysis {
        supported_plans,
        skipped_count,
    })
}

fn screen_candidate(
    candidate: PubUseCandidate,
    child_item: &str,
    child_source: &str,
) -> Result<CandidateScreening> {
    let export_match = if candidate.exported_name == child_item {
        CandidateExportMatch::Matches
    } else {
        CandidateExportMatch::Mismatch
    };
    let child_visibility = if line_contains_plain_pub(child_source, candidate.child_line)? {
        ChildVisibilityState::PlainPub
    } else {
        ChildVisibilityState::AlreadyNarrowed
    };

    Ok(match (export_match, child_visibility) {
        (CandidateExportMatch::Matches, ChildVisibilityState::PlainPub) => {
            CandidateScreening::Accept(candidate)
        },
        _ => CandidateScreening::Skip,
    })
}

enum CandidateExportMatch {
    Matches,
    Mismatch,
}

enum ChildVisibilityState {
    PlainPub,
    AlreadyNarrowed,
}

fn build_validated_plan(
    candidate: PubUseCandidate,
    parent_boundary: ParentBoundaryKey,
) -> Result<ValidatedPubUsePlan> {
    let child_narrowing = build_child_pub_super_fix(&candidate)?;
    Ok(ValidatedPubUsePlan {
        parent_boundary,
        child_file: candidate.child_file,
        child_module: candidate.child_module,
        exported_name: candidate.exported_name,
        parent_mod_path: candidate.parent_mod_path,
        target_item_path: candidate.target_item_path,
        child_narrowing,
    })
}

fn build_parent_pub_use_edit_for_exports(
    parent_boundary: &ParentBoundaryKey,
    exports: &[(String, String)],
) -> Result<UseFix> {
    let source = fs::read_to_string(&parent_boundary.parent_mod)
        .with_context(|| format!("failed to read {}", parent_boundary.parent_mod.display()))?;
    let file = parse_file(&source).context("failed to parse parent module file")?;
    let offsets = line_offsets(&source);
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if !matches!(item_use.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let (start, end) = item_use_byte_range(&source, &offsets, &item_use);
        if start != parent_boundary.item_start || end != parent_boundary.item_end {
            continue;
        }

        let local_exports = locally_used_exports(&source, &item_use, exports)?;
        let replacement =
            rewrite_parent_pub_use_item_for_exports(&item_use, exports, &local_exports)?;
        return Ok(UseFix {
            path: parent_boundary.parent_mod.clone(),
            start,
            end,
            replacement,
        });
    }

    anyhow::bail!(
        "matching parent `pub use` item not found in {} for span {}..{}",
        parent_boundary.parent_mod.display(),
        parent_boundary.item_start,
        parent_boundary.item_end
    )
}

fn build_child_pub_super_fix(candidate: &PubUseCandidate) -> Result<UseFix> {
    let source = fs::read_to_string(&candidate.child_file)
        .with_context(|| format!("failed to read {}", candidate.child_file.display()))?;
    let line_span = line_span(&source, candidate.child_line)
        .context("failed to compute child visibility line span")?;
    let line_text = &source[line_span.0..line_span.1];
    let Some(relative_start) = line_text.find("pub ") else {
        anyhow::bail!(
            "child item line {} does not contain a plain `pub ` prefix",
            candidate.child_line
        );
    };
    Ok(UseFix {
        path:        candidate.child_file.clone(),
        start:       line_span.0 + relative_start,
        end:         line_span.0 + relative_start + 4,
        replacement: "pub(super) ".to_string(),
    })
}

fn line_contains_plain_pub(source: &str, line: usize) -> Result<bool> {
    let line_span = line_span(source, line).context("failed to compute child item line span")?;
    Ok(source[line_span.0..line_span.1].contains("pub "))
}

fn rewrite_subtree_imports_for_plans(
    selection: &Selection,
    plans: &[ValidatedPubUsePlan],
) -> Result<Vec<UseFix>> {
    let mut plan_groups: BTreeMap<PathBuf, Vec<&ValidatedPubUsePlan>> = BTreeMap::new();
    for plan in plans {
        plan_groups
            .entry(plan.parent_boundary.parent_mod.clone())
            .or_default()
            .push(plan);
    }

    let mut fixes = Vec::new();
    for (parent_mod, parent_plans) in plan_groups {
        let parent_dir = parent_mod
            .parent()
            .context("candidate parent boundary had no parent directory")?;
        for file in rust_source_files(parent_dir)? {
            if file == parent_mod {
                continue;
            }
            if parent_plans.iter().any(|plan| plan.child_file == file) {
                continue;
            }
            fixes.extend(rewrite_in_subtree_imports(
                &selection.analysis_root,
                &file,
                &parent_plans,
            )?);
        }
    }

    dedup_fixes(&mut fixes);
    Ok(fixes)
}

fn rewrite_in_subtree_imports(
    analysis_root: &Path,
    file: &Path,
    plans: &[&ValidatedPubUsePlan],
) -> Result<Vec<UseFix>> {
    let source =
        fs::read_to_string(file).with_context(|| format!("failed to read {}", file.display()))?;
    let syntax =
        parse_file(&source).with_context(|| format!("failed to parse {}", file.display()))?;
    let src_root = find_src_root(file).with_context(|| {
        format!(
            "failed to determine src root for subtree file {} under {}",
            file.display(),
            analysis_root.display()
        )
    })?;
    let base_module_path = file_module_path(&src_root, file)
        .with_context(|| format!("failed to determine module path for {}", file.display()))?;
    let offsets = line_offsets(&source);
    let mut visitor = PubUseFixVisitor {
        file,
        source: &source,
        offsets: &offsets,
        current_module_path: base_module_path,
        plans,
        fixes: Vec::new(),
    };
    visitor.visit_file(&syntax);
    Ok(visitor.fixes)
}

struct PubUseFixVisitor<'a> {
    file:                &'a Path,
    source:              &'a str,
    offsets:             &'a [usize],
    current_module_path: Vec<String>,
    plans:               &'a [&'a ValidatedPubUsePlan],
    fixes:               Vec<UseFix>,
}

impl Visit<'_> for PubUseFixVisitor<'_> {
    fn visit_item_mod(&mut self, node: &syn::ItemMod) {
        if let Some((_, items)) = &node.content {
            self.current_module_path.push(node.ident.to_string());
            for item in items {
                self.visit_item(item);
            }
            self.current_module_path.pop();
        }
    }

    fn visit_item_use(&mut self, node: &ItemUse) {
        if let Some(replacement) =
            rewrite_use_tree(&self.current_module_path, &node.tree, self.plans)
        {
            let span = node.span();
            let start = offset(self.offsets, span.start());
            let end = offset(self.offsets, span.end());
            let original_item = &self.source[start..end];
            let rewritten =
                original_item.replacen(&replacement.original, &replacement.rewritten, 1);
            self.fixes.push(UseFix {
                path: self.file.to_path_buf(),
                start,
                end,
                replacement: rewritten,
            });
        }
    }
}

struct UseRewrite {
    original:  String,
    rewritten: String,
}

fn rewrite_use_tree(
    current_module_path: &[String],
    tree: &UseTree,
    plans: &[&ValidatedPubUsePlan],
) -> Option<UseRewrite> {
    rewrite_use_tree_with_candidates(current_module_path, tree, plans)
}

struct BaseImport<'a> {
    base_segments: Vec<String>,
    leaf:          &'a UseTree,
}

fn rewrite_use_tree_with_candidates(
    current_module_path: &[String],
    tree: &UseTree,
    plans: &[&ValidatedPubUsePlan],
) -> Option<UseRewrite> {
    let base_import = split_base_import(tree)?;
    let absolute_base = absolute_use_path(current_module_path, &base_import.base_segments)?;
    let grouped_targets = collect_group_rewrite_targets(&absolute_base, base_import.leaf, plans);
    if grouped_targets.is_empty() {
        return None;
    }

    let rewritten = rewrite_leaf_under_base(
        current_module_path,
        &absolute_base,
        base_import.leaf,
        &grouped_targets,
    )?;
    let original = render_use_tree(tree).ok()?;
    (rewritten != original).then_some(UseRewrite {
        original,
        rewritten,
    })
}

fn split_base_import(tree: &UseTree) -> Option<BaseImport<'_>> {
    let mut base_segments = Vec::new();
    let mut cursor = tree;
    loop {
        match cursor {
            UseTree::Path(path) => {
                base_segments.push(path.ident.to_string());
                cursor = &path.tree;
            },
            _ => {
                return Some(BaseImport {
                    base_segments,
                    leaf: cursor,
                });
            },
        }
    }
}

#[derive(Clone)]
struct GroupedRewriteTarget {
    original_name: String,
    rename:        Option<String>,
    target_path:   Vec<String>,
}

fn collect_group_rewrite_targets(
    absolute_base: &[String],
    leaf: &UseTree,
    plans: &[&ValidatedPubUsePlan],
) -> Vec<GroupedRewriteTarget> {
    let mut targets = Vec::new();
    collect_group_rewrite_targets_inner(absolute_base, leaf, plans, &mut targets);
    targets
}

fn collect_group_rewrite_targets_inner(
    absolute_base: &[String],
    leaf: &UseTree,
    plans: &[&ValidatedPubUsePlan],
    targets: &mut Vec<GroupedRewriteTarget>,
) {
    match leaf {
        UseTree::Name(name) => {
            if let Some(plan) = plans.iter().find(|plan| {
                plan.parent_mod_path == absolute_base && name.ident == plan.exported_name
            }) {
                targets.push(GroupedRewriteTarget {
                    original_name: name.ident.to_string(),
                    rename:        None,
                    target_path:   plan.target_item_path.clone(),
                });
            }
        },
        UseTree::Rename(rename) => {
            if let Some(plan) = plans.iter().find(|plan| {
                plan.parent_mod_path == absolute_base && rename.rename == plan.exported_name
            }) {
                targets.push(GroupedRewriteTarget {
                    original_name: rename.ident.to_string(),
                    rename:        Some(rename.rename.to_string()),
                    target_path:   plan.target_item_path.clone(),
                });
            }
        },
        UseTree::Group(group) => {
            for item in &group.items {
                collect_group_rewrite_targets_inner(absolute_base, item, plans, targets);
            }
        },
        UseTree::Glob(_) | UseTree::Path(_) => {},
    }
}

fn rewrite_leaf_under_base(
    current_module_path: &[String],
    absolute_base: &[String],
    leaf: &UseTree,
    targets: &[GroupedRewriteTarget],
) -> Option<String> {
    match leaf {
        UseTree::Name(_) | UseTree::Rename(_) => {
            let target = targets.first()?;
            Some(relative_path_from_module(
                current_module_path,
                &target.target_path,
                target.rename.as_deref(),
            ))
        },
        UseTree::Group(group) => {
            let target_by_name = targets
                .iter()
                .map(|target| (target.original_name.clone(), target))
                .collect::<BTreeMap<_, _>>();
            let mut preserved = Vec::new();
            let mut regrouped = BTreeMap::<Vec<String>, Vec<(String, Option<String>)>>::new();

            for item in &group.items {
                match item {
                    UseTree::Name(name) => {
                        if let Some(target) = target_by_name.get(&name.ident.to_string()) {
                            let module_path =
                                target.target_path[..target.target_path.len() - 1].to_vec();
                            regrouped
                                .entry(module_path)
                                .or_default()
                                .push((name.ident.to_string(), target.rename.clone()));
                        } else {
                            let mut preserved_path = absolute_base.to_vec();
                            preserved_path.push(name.ident.to_string());
                            preserved.push(relative_path_from_module(
                                current_module_path,
                                &preserved_path,
                                None,
                            ));
                        }
                    },
                    UseTree::Rename(rename) => {
                        if let Some(target) = target_by_name.get(&rename.rename.to_string()) {
                            let module_path =
                                target.target_path[..target.target_path.len() - 1].to_vec();
                            regrouped
                                .entry(module_path)
                                .or_default()
                                .push((rename.ident.to_string(), target.rename.clone()));
                        } else {
                            let mut preserved_path = absolute_base.to_vec();
                            preserved_path.push(rename.ident.to_string());
                            preserved.push(relative_path_from_module(
                                current_module_path,
                                &preserved_path,
                                Some(&rename.rename.to_string()),
                            ));
                        }
                    },
                    _ => preserved.push(format!(
                        "{}::{}",
                        relative_path_from_module(current_module_path, absolute_base, None),
                        render_use_tree(item).ok()?
                    )),
                }
            }

            let mut rewritten_lines = preserved;
            for (module_path, names) in regrouped {
                let relative_base =
                    relative_path_from_module(current_module_path, &module_path, None);
                let rendered = if let [(name, rename)] = names.as_slice() {
                    rename.as_ref().map_or_else(
                        || format!("{relative_base}::{name}"),
                        |rename| format!("{relative_base}::{name} as {rename}"),
                    )
                } else {
                    let grouped_names = names
                        .iter()
                        .map(|(name, rename)| {
                            rename.as_ref().map_or_else(
                                || name.clone(),
                                |rename| format!("{name} as {rename}"),
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("{relative_base}::{{{grouped_names}}}")
                };
                rewritten_lines.push(rendered);
            }

            (!rewritten_lines.is_empty()).then_some(rewritten_lines.join(";\nuse "))
        },
        UseTree::Glob(_) | UseTree::Path(_) => None,
    }
}

fn absolute_use_path(current_module_path: &[String], segments: &[String]) -> Option<Vec<String>> {
    let first = segments.first()?.as_str();
    match first {
        "crate" => Some(segments[1..].to_vec()),
        "self" => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments[1..].iter().cloned())
                .collect(),
        ),
        "super" => {
            let mut module = current_module_path.to_vec();
            let mut index = 0usize;
            while segments.get(index).is_some_and(|seg| seg == "super") {
                module.pop()?;
                index += 1;
            }
            Some(
                module
                    .into_iter()
                    .chain(segments[index..].iter().cloned())
                    .collect(),
            )
        },
        _ => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments.iter().cloned())
                .collect(),
        ),
    }
}

fn relative_path_from_module(
    current_module_path: &[String],
    target_path: &[String],
    rename: Option<&str>,
) -> String {
    let common = common_prefix_len(current_module_path, target_path);
    let up_count = current_module_path.len().saturating_sub(common);
    let mut segments = Vec::new();
    for _ in 0..up_count {
        segments.push("super".to_string());
    }
    segments.extend(target_path[common..].iter().cloned());
    format_path(&segments, rename)
}

fn format_path(segments: &[String], rename: Option<&str>) -> String {
    let mut path = segments.join("::");
    if let Some(rename) = rename {
        path.push_str(" as ");
        path.push_str(rename);
    }
    path
}

fn common_prefix_len(left: &[String], right: &[String]) -> usize {
    left.iter()
        .zip(right.iter())
        .take_while(|(l, r)| l == r)
        .count()
}

fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

fn offset(line_offsets: &[usize], position: proc_macro2::LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
}

fn line_span(source: &str, line: usize) -> Option<(usize, usize)> {
    let offsets = line_offsets(source);
    let start = *offsets.get(line.saturating_sub(1))?;
    let end = offsets.get(line).copied().unwrap_or(source.len());
    Some((start, end))
}

struct ParentExportResolution {
    exported_name:   String,
    parent_boundary: ParentBoundaryKey,
}

fn resolve_parent_pub_use_export(
    source: &str,
    line: usize,
    child_module_name: &str,
    item_name: &str,
) -> Result<Option<ParentExportResolution>> {
    let file = parse_file(source).context("failed to parse parent module file")?;
    let offsets = line_offsets(source);
    for item in file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        if !matches!(item_use.vis, syn::Visibility::Public(_)) {
            continue;
        }
        let start_line = item_use.span().start().line;
        let end_line = item_use.span().end().line;
        if !(start_line..=end_line).contains(&line) {
            continue;
        }
        if parent_pub_use_exports_item(&item_use.tree, child_module_name, item_name) {
            let (item_start, item_end) = item_use_byte_range(source, &offsets, &item_use);
            return Ok(Some(ParentExportResolution {
                exported_name:   item_name.to_string(),
                parent_boundary: ParentBoundaryKey {
                    parent_mod: PathBuf::new(),
                    item_start,
                    item_end,
                },
            }));
        }
        return Ok(None);
    }
    Ok(None)
}

fn parent_pub_use_exports_item(tree: &UseTree, child_module_name: &str, item_name: &str) -> bool {
    parent_pub_use_exports_item_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn parent_pub_use_exports_item_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            parent_pub_use_exports_item_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = if prefix.first().is_some_and(|segment| segment == "self") {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            normalized.len() == 1 && normalized[0] == child_module_name && name.ident == item_name
        },
        UseTree::Group(group) => group.items.iter().any(|item| {
            parent_pub_use_exports_item_with_prefix(
                prefix.clone(),
                item,
                child_module_name,
                item_name,
            )
        }),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

fn rewrite_parent_pub_use_item_for_exports(
    item_use: &ItemUse,
    exports: &[(String, String)],
    local_exports: &[(String, String)],
) -> Result<String> {
    let Some(rewritten_tree) = remove_exports_from_use_tree(Vec::new(), &item_use.tree, exports)
    else {
        return Ok(render_parent_local_use_lines(local_exports));
    };
    let mut lines = vec![format!("pub use {};", render_use_tree(&rewritten_tree)?)];
    let local_lines = render_parent_local_use_lines(local_exports);
    if !local_lines.is_empty() {
        lines.push(local_lines);
    }
    Ok(lines.join("\n"))
}

fn remove_exports_from_use_tree(
    prefix: Vec<String>,
    tree: &UseTree,
    exports: &[(String, String)],
) -> Option<UseTree> {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            let rewritten = remove_exports_from_use_tree(next, &path.tree, exports)?;
            Some(UseTree::Path(syn::UsePath {
                ident:        path.ident.clone(),
                colon2_token: path.colon2_token,
                tree:         Box::new(rewritten),
            }))
        },
        UseTree::Name(name) => {
            let normalized = if prefix.first().is_some_and(|segment| segment == "self") {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            if normalized.len() == 1
                && exports.iter().any(|(child_module_name, item_name)| {
                    normalized[0] == *child_module_name && name.ident == item_name
                })
            {
                None
            } else {
                Some(tree.clone())
            }
        },
        UseTree::Group(group) => {
            let kept_items = group
                .items
                .iter()
                .filter_map(|item| remove_exports_from_use_tree(prefix.clone(), item, exports))
                .collect::<Vec<_>>();
            match kept_items.as_slice() {
                [] => None,
                [only] => Some(only.clone()),
                _ => {
                    let mut punctuated = syn::punctuated::Punctuated::new();
                    for item in kept_items {
                        punctuated.push(item);
                    }
                    Some(UseTree::Group(syn::UseGroup {
                        brace_token: group.brace_token,
                        items:       punctuated,
                    }))
                },
            }
        },
        UseTree::Rename(_) | UseTree::Glob(_) => Some(tree.clone()),
    }
}

fn render_parent_local_use_lines(exports: &[(String, String)]) -> String {
    if exports.is_empty() {
        return String::new();
    }

    let mut grouped = BTreeMap::<String, Vec<String>>::new();
    for (child_module, item_name) in exports {
        grouped
            .entry(child_module.clone())
            .or_default()
            .push(item_name.clone());
    }

    let mut lines = Vec::new();
    for (child_module, item_names) in grouped {
        let rendered = match item_names.as_slice() {
            [only] => format!("use {child_module}::{only};"),
            _ => format!("use {child_module}::{{{}}};", item_names.join(", ")),
        };
        lines.push(rendered);
    }
    lines.join("\n")
}

fn item_use_byte_range(source: &str, offsets: &[usize], item_use: &ItemUse) -> (usize, usize) {
    let start = offset(offsets, item_use.span().start());
    let end = source[start..]
        .find(';')
        .map_or(source.len(), |semicolon_offset| {
            start + semicolon_offset + 1
        });
    (start, end)
}

fn group_parent_pub_use_plans<'a>(
    plans: impl Iterator<Item = &'a ValidatedPubUsePlan>,
) -> BTreeMap<ParentBoundaryKey, Vec<(String, String)>> {
    let mut groups = BTreeMap::new();
    for plan in plans {
        groups
            .entry(plan.parent_boundary.clone())
            .or_insert_with(Vec::new)
            .push((plan.child_module.clone(), plan.exported_name.clone()));
    }
    groups
}

fn render_use_tree(tree: &UseTree) -> Result<String> {
    match tree {
        UseTree::Path(path) => Ok(format!("{}::{}", path.ident, render_use_tree(&path.tree)?)),
        UseTree::Name(name) => Ok(name.ident.to_string()),
        UseTree::Rename(rename) => Ok(format!("{} as {}", rename.ident, rename.rename)),
        UseTree::Glob(_) => Ok("*".to_string()),
        UseTree::Group(group) => {
            let mut rendered_items = Vec::new();
            for item in &group.items {
                rendered_items.push(render_use_tree(item)?);
            }
            Ok(format!("{{{}}}", rendered_items.join(", ")))
        },
    }
}

fn locally_used_exports(
    source: &str,
    item_use: &ItemUse,
    exports: &[(String, String)],
) -> Result<Vec<(String, String)>> {
    let offsets = line_offsets(source);
    let (start, end) = item_use_byte_range(source, &offsets, item_use);
    let mut source_without_use = source.to_string();
    source_without_use.replace_range(start..end, "");

    let mut locally_used = Vec::new();
    for (child_module, item_name) in exports {
        let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(item_name)))
            .with_context(|| format!("failed to build local-use regex for {item_name}"))?;
        if pattern.is_match(&source_without_use) {
            locally_used.push((child_module.clone(), item_name.clone()));
        }
    }
    Ok(locally_used)
}

fn normalize_rel_path(path: impl AsRef<Path>) -> String {
    path.as_ref().to_string_lossy().replace('\\', "/")
}

fn module_path_from_dir(src_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(src_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

fn module_path_from_boundary_file(src_root: &Path, boundary_file: &Path) -> Option<Vec<String>> {
    if boundary_file.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        return module_path_from_dir(src_root, boundary_file.parent()?);
    }

    file_module_path(src_root, boundary_file)
}

fn rust_source_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(dir, &mut files)?;
    Ok(files)
}

fn find_src_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(|name| name.to_str()) == Some("src"))
        .map(Path::to_path_buf)
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

fn dedup_fixes(fixes: &mut Vec<UseFix>) {
    let mut seen = BTreeSet::new();
    fixes.retain(|fix| {
        seen.insert((
            fix.path.clone(),
            fix.start,
            fix.end,
            fix.replacement.clone(),
        ))
    });
}
