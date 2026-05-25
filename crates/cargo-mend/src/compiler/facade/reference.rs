use std::path::Path;

use anyhow::Result;
use syn::Item;

use super::boundary;
use super::boundary::ParentBoundary;
use super::exports;
use super::exports::ParentFacadeExports;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::ExtractedPaths;
use crate::compiler::source_cache::PathOrigin;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::source_cache::UseRename;
use crate::rust_syntax::MODULE_PATH_SEPARATOR;
use crate::rust_syntax::PATH_KEYWORD_CRATE;
use crate::rust_syntax::PATH_KEYWORD_SELF;
use crate::rust_syntax::PATH_KEYWORD_SUPER;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentFacadeUsage {
    Unused,
    UsedInsideSubtreeByRelativeImport,
    UsedInsideSubtreeByRelativePath,
    UsedInsideSubtreeByCrateImport,
    UsedInsideSubtreeByCratePath,
    UsedOutsideSubtree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

pub(super) fn scan_facade_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    parent_boundary: &ParentBoundary,
    exported_names: &ParentFacadeExports,
) -> Result<ParentFacadeUsage> {
    let mut usage = ParentFacadeUsage::Unused;
    for source_path in source_cache.source_files_under(source_root) {
        if source_path == parent_boundary.boundary_file {
            continue;
        }
        let Some(current_module_path) =
            source_cache::module_path_from_source_file(source_root, source_path)
        else {
            continue;
        };
        let Some(extracted) = source_cache.extracted_paths(source_path) else {
            continue;
        };
        match source_references_parent_export(
            extracted,
            &current_module_path,
            &parent_boundary.module_path,
            &exported_names.explicit,
        ) {
            ParentFacadeReferenceUsage::None => {},
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative) => {
                if matches!(usage, ParentFacadeUsage::Unused)
                    && source_path.starts_with(&parent_boundary.subtree_root)
                {
                    usage = ParentFacadeUsage::UsedInsideSubtreeByRelativeImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate) => {
                if matches!(usage, ParentFacadeUsage::Unused)
                    && source_path.starts_with(&parent_boundary.subtree_root)
                {
                    usage = ParentFacadeUsage::UsedInsideSubtreeByCrateImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideSubtreeByRelativePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideSubtreeByCratePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideSubtree;
                    break;
                }
            },
        }
    }

    if !matches!(usage, ParentFacadeUsage::UsedOutsideSubtree)
        && workspace_source_mentions_parent_export_literal(
            source_cache,
            settings,
            parent_boundary,
            &exported_names.explicit,
        )?
    {
        usage = ParentFacadeUsage::UsedOutsideSubtree;
    }

    Ok(usage)
}

pub fn workspace_source_mentions_parent_export_literal(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    parent_boundary: &ParentBoundary,
    exported_names: &[String],
) -> Result<bool> {
    if settings.config_root == settings.package_root {
        return Ok(false);
    }

    if parent_boundary.module_path.is_empty() {
        return Ok(false);
    }

    let module_prefix = format!(
        "{PATH_KEYWORD_CRATE}{MODULE_PATH_SEPARATOR}{}",
        parent_boundary.module_path.join(MODULE_PATH_SEPARATOR)
    );
    let findings_root = settings
        .findings_dir
        .parent()
        .map_or_else(|| settings.findings_dir.clone(), Path::to_path_buf);

    for file in source_cache.source_files_under(&settings.config_root) {
        if file.starts_with(&settings.package_root)
            || file.starts_with(&settings.findings_dir)
            || file.starts_with(&findings_root)
        {
            continue;
        }
        let source = source_cache.read_source(file)?;
        if exported_names.iter().any(|name| {
            let pattern = format!("{module_prefix}::{name}");
            source.contains(&pattern)
        }) {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn source_references_parent_export(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> ParentFacadeReferenceUsage {
    for (raw, origin) in &extracted.expr_paths {
        if matching_origin_indexed(
            raw,
            *origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(*origin);
        }
        if let Some(resolved) = resolve_alias_expr_path(raw, &extracted.use_renames)
            && matching_origin_indexed(
                &resolved,
                *origin,
                current_module_path,
                module_path,
                exported_names,
            )
            .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(*origin);
        }
    }

    let mut import_usage = ParentFacadeReferenceUsage::None;
    for (raw, origin) in &extracted.use_paths {
        if matching_origin_indexed(
            raw,
            *origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            import_usage =
                merge_reference_usage(import_usage, ParentFacadeReferenceUsage::Import(*origin));
        }
    }

    import_usage
}

/// Resolves the first segment of an `expr_path` through module aliases.
///
/// Given `["test_utils", "assert_test_case"]` and a rename mapping
/// `test_utils → ["crate", "test_support"]`, returns
/// `["crate", "test_support", "assert_test_case"]`.
fn resolve_alias_expr_path(raw: &[String], renames: &[UseRename]) -> Option<Vec<String>> {
    let first = raw.first()?;
    let rename = renames.iter().find(|rename| rename.alias == *first)?;
    let mut resolved = rename.original_path.clone();
    resolved.extend(raw[1..].iter().cloned());
    Some(resolved)
}

fn matching_origin_indexed(
    raw: &[String],
    origin: PathOrigin,
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> Option<PathOrigin> {
    resolve_module_relative_paths(raw, current_module_path)
        .into_iter()
        .find(|segments| {
            segments.len() == module_path.len() + 1
                && segments[..module_path.len()] == *module_path
                && exported_names
                    .iter()
                    .any(|name| name == &segments[module_path.len()])
        })
        .map(|_| origin)
}

pub(super) fn resolve_module_relative_paths(
    raw: &[String],
    current_module_path: &[String],
) -> Vec<Vec<String>> {
    if raw.is_empty() {
        return Vec::new();
    }

    if raw.first().map(String::as_str) == Some(PATH_KEYWORD_CRATE) {
        return vec![raw[1..].to_vec()];
    }

    if raw.first().map(String::as_str) == Some(PATH_KEYWORD_SELF) {
        let mut resolved = current_module_path.to_vec();
        resolved.extend(raw[1..].iter().cloned());
        return vec![resolved];
    }

    if raw.first().map(String::as_str) == Some(PATH_KEYWORD_SUPER) {
        let mut index = 0usize;
        let mut resolved = current_module_path.to_vec();
        while raw
            .get(index)
            .is_some_and(|segment| segment == PATH_KEYWORD_SUPER)
        {
            if resolved.pop().is_none() {
                return Vec::new();
            }
            index += 1;
        }
        if raw
            .get(index)
            .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
        {
            index += 1;
        }
        resolved.extend(raw[index..].iter().cloned());
        return vec![resolved];
    }

    (0..=current_module_path.len())
        .map(|prefix_len| {
            let mut resolved = current_module_path[..prefix_len].to_vec();
            resolved.extend(raw.iter().cloned());
            resolved
        })
        .collect()
}

pub(super) const fn merge_reference_usage(
    current: ParentFacadeReferenceUsage,
    next: ParentFacadeReferenceUsage,
) -> ParentFacadeReferenceUsage {
    match (current, next) {
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate)
        },
        _ => ParentFacadeReferenceUsage::None,
    }
}

pub fn public_reexport_exists_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = boundary::parent_boundary_for_child(source_root, child_file) else {
        return Ok(false);
    };
    let Some(child_module_path) =
        source_cache::module_path_from_source_file(source_root, child_file)
    else {
        return Ok(false);
    };

    for source_file in source_cache.source_files_under(source_root) {
        if source_file.starts_with(&parent_boundary.subtree_root) {
            continue;
        }
        let Some(file) = source_cache.parsed_file(source_file) else {
            continue;
        };
        let Some(current_module_path) =
            source_cache::module_path_from_source_file(source_root, source_file)
        else {
            continue;
        };

        for item in &file.items {
            let Item::Use(item_use) = item else {
                continue;
            };
            let Some(_) = exports::parent_facade_visibility(&item_use.vis) else {
                continue;
            };
            let mut paths = Vec::new();
            source_cache::flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
            for path in paths {
                for resolved in resolve_module_relative_paths(&path, &current_module_path) {
                    if resolved.len() != child_module_path.len() + 1 {
                        continue;
                    }
                    if resolved[..child_module_path.len()] == *child_module_path
                        && resolved[child_module_path.len()] == item_name
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }

    if settings.config_root != settings.package_root {
        let module_prefix = format!(
            "{PATH_KEYWORD_CRATE}{MODULE_PATH_SEPARATOR}{}",
            child_module_path.join(MODULE_PATH_SEPARATOR)
        );
        let findings_root = settings
            .findings_dir
            .parent()
            .map_or_else(|| settings.findings_dir.clone(), Path::to_path_buf);

        for file in source_cache.source_files_under(&settings.config_root) {
            if file.starts_with(&settings.package_root)
                || file.starts_with(&settings.findings_dir)
                || file.starts_with(&findings_root)
            {
                continue;
            }
            let source = source_cache.read_source(file)?;
            let pattern = format!("{module_prefix}::{item_name}");
            if source.contains(&pattern) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn path_exists_outside_child_module(
    source_cache: &SourceCache,
    source_root: &Path,
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    for source_file in source_cache.source_files_under(source_root) {
        let Some(current_module_path) =
            source_cache::module_path_from_source_file(source_root, source_file)
        else {
            continue;
        };
        if module_path_is_descendant(&current_module_path, child_module_path) {
            continue;
        }
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        if extracted_paths_mention_child_item(
            extracted,
            &current_module_path,
            child_module_path,
            item_name,
        ) {
            return true;
        }
    }

    false
}

fn extracted_paths_mention_child_item(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    extracted.use_paths.iter().any(|(path, _)| {
        resolved_path_mentions_child_item(path, current_module_path, child_module_path, item_name)
    }) || extracted.expr_paths.iter().any(|(path, _)| {
        resolved_path_mentions_child_item(path, current_module_path, child_module_path, item_name)
            || resolve_alias_expr_path(path, &extracted.use_renames).is_some_and(|resolved| {
                resolved_path_mentions_child_item(
                    &resolved,
                    current_module_path,
                    child_module_path,
                    item_name,
                )
            })
    })
}

fn resolved_path_mentions_child_item(
    path: &[String],
    current_module_path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    resolve_module_relative_paths(path, current_module_path)
        .into_iter()
        .any(|resolved| path_mentions_child_item(&resolved, child_module_path, item_name))
        || relative_tail_mentions_child_item(path, child_module_path, item_name)
}

fn path_mentions_child_item(
    path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    path.len() > child_module_path.len()
        && path[..child_module_path.len()] == *child_module_path
        && (path[child_module_path.len()] == item_name || path[child_module_path.len()] == "*")
}

fn relative_tail_mentions_child_item(
    path: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    if path.first().map(String::as_str) == Some(PATH_KEYWORD_CRATE) || child_module_path.is_empty()
    {
        return false;
    }

    let mut tail_start = 0usize;
    while path
        .get(tail_start)
        .is_some_and(|segment| segment == PATH_KEYWORD_SUPER)
    {
        tail_start += 1;
    }
    if path
        .get(tail_start)
        .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
    {
        tail_start += 1;
    }
    let tail = &path[tail_start..];

    (1..=child_module_path.len()).any(|suffix_len| {
        tail.len() > suffix_len
            && child_module_path[child_module_path.len() - suffix_len..] == tail[..suffix_len]
            && (tail[suffix_len] == item_name || tail[suffix_len] == "*")
    })
}

fn module_path_is_descendant(candidate: &[String], parent: &[String]) -> bool {
    candidate == parent || (candidate.len() > parent.len() && candidate[..parent.len()] == *parent)
}
