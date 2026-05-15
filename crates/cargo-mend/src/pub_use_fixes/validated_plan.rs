use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use proc_macro2::LineColumn;
use syn::ItemMod;
use syn::ItemUse;
use syn::UseTree;
use syn::spanned::Spanned;
use syn::visit::Visit;

use crate::constants::MODULE_PATH_SEPARATOR;
use crate::constants::PATH_KEYWORD_CRATE;
use crate::constants::PATH_KEYWORD_SELF;
use crate::constants::PATH_KEYWORD_SUPER;
use crate::constants::RUST_SOURCE_FILE_EXTENSION;
use crate::constants::SOURCE_DIR_SRC;
use crate::imports::UseFix;
use crate::module_paths;
use crate::pub_use_fixes::parent_boundary::ParentBoundaryKey;
use crate::selection::Selection;

pub(super) struct ValidatedPubUsePlan {
    pub(super) parent_boundary:    ParentBoundaryKey,
    pub(super) child_file:         PathBuf,
    pub(super) child_module:       String,
    pub(super) exported_name:      String,
    pub(super) parent_module_path: Vec<String>,
    pub(super) target_item_path:   Vec<String>,
    pub(super) child_narrowing:    UseFix,
}

pub(super) fn rewrite_subtree_imports_for_plans(
    selection: &Selection,
    plans: &[ValidatedPubUsePlan],
) -> Result<Vec<UseFix>> {
    let mut plan_groups: BTreeMap<PathBuf, Vec<&ValidatedPubUsePlan>> = BTreeMap::new();
    for plan in plans {
        plan_groups
            .entry(plan.parent_boundary.parent_module.clone())
            .or_default()
            .push(plan);
    }

    let mut fixes = Vec::new();
    for (parent_module, parent_plans) in plan_groups {
        let parent_dir = parent_module
            .parent()
            .context("candidate parent boundary had no parent directory")?;
        for file in rust_source_files(parent_dir)? {
            if file == parent_module {
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
        syn::parse_file(&source).with_context(|| format!("failed to parse {}", file.display()))?;
    let source_root = find_source_root(file).with_context(|| {
        format!(
            "failed to determine src root for subtree file {} under {}",
            file.display(),
            analysis_root.display()
        )
    })?;
    let base_module_path = module_paths::file_module_path(&source_root, file)
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
    fn visit_item_mod(&mut self, node: &ItemMod) {
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
                import_group: None,
            });
        }
    }
}

struct UseRewrite {
    original:  String,
    rewritten: String,
}

struct BaseImport<'a> {
    base_segments: Vec<String>,
    leaf:          &'a UseTree,
}

#[derive(Clone)]
struct GroupedRewriteTarget {
    original_name: String,
    rename:        Option<String>,
    target_path:   Vec<String>,
}

fn rewrite_use_tree(
    current_module_path: &[String],
    tree: &UseTree,
    plans: &[&ValidatedPubUsePlan],
) -> Option<UseRewrite> {
    rewrite_use_tree_with_candidates(current_module_path, tree, plans)
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
                plan.parent_module_path == absolute_base && name.ident == plan.exported_name
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
                plan.parent_module_path == absolute_base && rename.rename == plan.exported_name
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
            let mut rewritten_lines = Vec::new();
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
                            rewritten_lines.push(relative_path_from_module(
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
                            rewritten_lines.push(relative_path_from_module(
                                current_module_path,
                                &preserved_path,
                                Some(&rename.rename.to_string()),
                            ));
                        }
                    },
                    _ => rewritten_lines.push(format!(
                        "{}::{}",
                        relative_path_from_module(current_module_path, absolute_base, None),
                        render_use_tree(item).ok()?
                    )),
                }
            }

            for (module_path, names) in regrouped {
                let relative_base =
                    relative_path_from_module(current_module_path, &module_path, None);
                for (name, rename) in names {
                    let rendered = rename.as_ref().map_or_else(
                        || format!("{relative_base}::{name}"),
                        |rename| format!("{relative_base}::{name} as {rename}"),
                    );
                    rewritten_lines.push(rendered);
                }
            }

            (!rewritten_lines.is_empty()).then_some(rewritten_lines.join(";\nuse "))
        },
        UseTree::Glob(_) | UseTree::Path(_) => None,
    }
}

fn absolute_use_path(current_module_path: &[String], segments: &[String]) -> Option<Vec<String>> {
    let first = segments.first()?.as_str();
    match first {
        PATH_KEYWORD_CRATE => Some(segments[1..].to_vec()),
        PATH_KEYWORD_SELF => Some(
            current_module_path
                .iter()
                .cloned()
                .chain(segments[1..].iter().cloned())
                .collect(),
        ),
        PATH_KEYWORD_SUPER => {
            let mut module = current_module_path.to_vec();
            let mut index = 0usize;
            while segments
                .get(index)
                .is_some_and(|seg| seg == PATH_KEYWORD_SUPER)
            {
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
        segments.push(PATH_KEYWORD_SUPER.to_string());
    }
    segments.extend(target_path[common..].iter().cloned());
    format_path(&segments, rename)
}

fn format_path(segments: &[String], rename: Option<&str>) -> String {
    let mut path = segments.join(MODULE_PATH_SEPARATOR);
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

pub(super) fn render_use_tree(tree: &UseTree) -> Result<String> {
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

pub(super) fn line_offsets(text: &str) -> Vec<usize> {
    let mut offsets = vec![0];
    for (idx, ch) in text.char_indices() {
        if ch == '\n' {
            offsets.push(idx + 1);
        }
    }
    offsets
}

pub(super) fn offset(line_offsets: &[usize], position: LineColumn) -> usize {
    line_offsets
        .get(position.line.saturating_sub(1))
        .copied()
        .unwrap_or(0)
        + position.column
}

pub(super) fn line_span(source: &str, line: usize) -> Option<(usize, usize)> {
    let offsets = line_offsets(source);
    let start = *offsets.get(line.saturating_sub(1))?;
    let end = offsets.get(line).copied().unwrap_or(source.len());
    Some((start, end))
}

pub(super) fn find_source_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|ancestor| ancestor.file_name().and_then(OsStr::to_str) == Some(SOURCE_DIR_SRC))
        .map(Path::to_path_buf)
}

fn rust_source_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(dir, &mut files)?;
    Ok(files)
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if path.extension().and_then(OsStr::to_str) == Some(RUST_SOURCE_FILE_EXTENSION) {
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
