use std::path::Path;

use anyhow::Result;
use rustc_middle::ty::TyCtxt;

use super::boundary;
use super::boundary::ModuleSourceMap;
use super::boundary::ParentBoundary;
use super::exports::ParentFacadeExports;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache::ExtractedPaths;
use crate::compiler::source_cache::PathOrigin;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::source_cache::UseRename;
use crate::rust_syntax::PathAnchor;

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
enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

pub(super) fn scan_facade_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    parent_boundary: &ParentBoundary,
    exported_names: &ParentFacadeExports,
) -> Result<ParentFacadeUsage> {
    let mut usage = ParentFacadeUsage::Unused;
    'source_files: for source_path in source_cache.source_files_under(source_root) {
        let Some(extracted) = source_cache.extracted_paths(source_path) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_path)
        {
            if source_path == parent_boundary.boundary_file
                && current_module_path == parent_boundary.module_path
            {
                continue;
            }
            let reference_usage = source_references_parent_export(
                extracted,
                &current_module_path,
                &module_suffix,
                &parent_boundary.module_path,
                &exported_names.explicit,
            );
            let inside_subtree =
                module_path_is_descendant(&current_module_path, &parent_boundary.module_path);
            match reference_usage {
                ParentFacadeReferenceUsage::None => {},
                ParentFacadeReferenceUsage::Import(PathOrigin::Relative) => {
                    if matches!(usage, ParentFacadeUsage::Unused) && inside_subtree {
                        usage = ParentFacadeUsage::UsedInsideSubtreeByRelativeImport;
                    } else if !inside_subtree {
                        usage = ParentFacadeUsage::UsedOutsideSubtree;
                        break 'source_files;
                    }
                },
                ParentFacadeReferenceUsage::Import(PathOrigin::Crate) => {
                    if matches!(usage, ParentFacadeUsage::Unused) && inside_subtree {
                        usage = ParentFacadeUsage::UsedInsideSubtreeByCrateImport;
                    } else if !inside_subtree {
                        usage = ParentFacadeUsage::UsedOutsideSubtree;
                        break 'source_files;
                    }
                },
                ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative) => {
                    if inside_subtree {
                        usage = ParentFacadeUsage::UsedInsideSubtreeByRelativePath;
                    } else {
                        usage = ParentFacadeUsage::UsedOutsideSubtree;
                        break 'source_files;
                    }
                },
                ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate) => {
                    if inside_subtree {
                        usage = ParentFacadeUsage::UsedInsideSubtreeByCratePath;
                    } else {
                        usage = ParentFacadeUsage::UsedOutsideSubtree;
                        break 'source_files;
                    }
                },
            }
        }
    }

    if !matches!(usage, ParentFacadeUsage::UsedOutsideSubtree)
        && workspace_source_mentions_parent_export_literal(
            source_cache,
            settings,
            &parent_boundary.module_path,
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
    module_path: &[String],
    exported_names: &[String],
) -> Result<bool> {
    if settings.config_root == settings.package_root {
        return Ok(false);
    }

    if module_path.is_empty() {
        return Ok(false);
    }

    let module_prefix = format!("crate::{}", module_path.join("::"));
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

fn source_references_parent_export(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> ParentFacadeReferenceUsage {
    for extracted_path in &extracted.expr_paths {
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        if matching_origin_indexed(
            &extracted_path.segments,
            extracted_path.origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(extracted_path.origin);
        }
        if let Some(resolved) = resolve_alias_expr_path(
            &extracted_path.segments,
            module_suffix,
            &extracted.use_renames,
        ) && matching_origin_indexed(
            &resolved,
            extracted_path.origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(extracted_path.origin);
        }
    }

    let mut import_usage = ParentFacadeReferenceUsage::None;
    for extracted_path in &extracted.use_paths {
        if extracted_path.module_suffix != module_suffix {
            continue;
        }
        if matching_origin_indexed(
            &extracted_path.segments,
            extracted_path.origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            import_usage = merge_reference_usage(
                import_usage,
                ParentFacadeReferenceUsage::Import(extracted_path.origin),
            );
        }
    }

    import_usage
}

/// Resolves the first segment of an `expr_path` through module aliases.
///
/// Given `["test_utils", "assert_test_case"]` and a rename mapping
/// `test_utils → ["crate", "test_support"]`, returns
/// `["crate", "test_support", "assert_test_case"]`.
fn resolve_alias_expr_path(
    raw: &[String],
    module_suffix: &[String],
    renames: &[UseRename],
) -> Option<Vec<String>> {
    let first = raw.first()?;
    let rename = renames
        .iter()
        .find(|rename| rename.module_suffix == module_suffix && rename.alias == *first)?;
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

    let Some(path_anchor) = PathAnchor::first(raw) else {
        return Vec::new();
    };
    match path_anchor {
        PathAnchor::Crate => return vec![raw[1..].to_vec()],
        PathAnchor::SelfMod => {
            let mut resolved = current_module_path.to_vec();
            resolved.extend(raw[1..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::Super => {
            let mut index = 0usize;
            let mut resolved = current_module_path.to_vec();
            while raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::Super)
            {
                if resolved.pop().is_none() {
                    return Vec::new();
                }
                index += 1;
            }
            if raw
                .get(index)
                .is_some_and(|segment| PathAnchor::from(segment.as_str()) == PathAnchor::SelfMod)
            {
                index += 1;
            }
            resolved.extend(raw[index..].iter().cloned());
            return vec![resolved];
        },
        PathAnchor::SelfType | PathAnchor::Name => {},
    }

    (0..=current_module_path.len())
        .map(|prefix_len| {
            let mut resolved = current_module_path[..prefix_len].to_vec();
            resolved.extend(raw.iter().cloned());
            resolved
        })
        .collect()
}

const fn merge_reference_usage(
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

pub fn path_exists_outside_child_module(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    for source_file in source_cache.source_files_under(source_root) {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_file)
        {
            if module_path_is_descendant(&current_module_path, child_module_path) {
                continue;
            }
            if extracted_paths_mention_child_item(
                extracted,
                &current_module_path,
                &module_suffix,
                child_module_path,
                item_name,
            ) {
                return true;
            }
        }
    }

    false
}

pub fn path_exists_outside_module(
    source_cache: &SourceCache,
    source_root: &Path,
    tcx: TyCtxt<'_>,
    module_sources: &ModuleSourceMap,
    module_path: &[String],
    item_names: &[String],
) -> bool {
    for source_file in source_cache.source_files_under(source_root) {
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        for (current_module_path, module_suffix) in
            active_module_contexts(extracted, module_sources, tcx, source_file)
        {
            if module_path_is_descendant(&current_module_path, module_path) {
                continue;
            }
            if !matches!(
                source_references_parent_export(
                    extracted,
                    &current_module_path,
                    &module_suffix,
                    module_path,
                    item_names,
                ),
                ParentFacadeReferenceUsage::None
            ) {
                return true;
            }
        }
    }
    false
}

fn extracted_paths_mention_child_item(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_suffix: &[String],
    child_module_path: &[String],
    item_name: &str,
) -> bool {
    extracted.use_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && resolved_path_mentions_child_item(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            )
    }) || extracted.expr_paths.iter().any(|extracted_path| {
        extracted_path.module_suffix == module_suffix
            && (resolved_path_mentions_child_item(
                &extracted_path.segments,
                current_module_path,
                child_module_path,
                item_name,
            ) || resolve_alias_expr_path(
                &extracted_path.segments,
                module_suffix,
                &extracted.use_renames,
            )
            .is_some_and(|resolved| {
                resolved_path_mentions_child_item(
                    &resolved,
                    current_module_path,
                    child_module_path,
                    item_name,
                )
            }))
    })
}

fn active_module_contexts(
    extracted: &ExtractedPaths,
    module_sources: &ModuleSourceMap,
    tcx: TyCtxt<'_>,
    source_file: &Path,
) -> Vec<(Vec<String>, Vec<String>)> {
    let mut contexts = Vec::new();
    for root_module in module_sources.root_modules_for_file(tcx, source_file) {
        let root_path = boundary::module_path(tcx, root_module);
        for module_suffix in lexical_module_suffixes(extracted) {
            let mut current_module_path = root_path.clone();
            current_module_path.extend(module_suffix.iter().cloned());
            if module_sources.file_contains_module_path(tcx, source_file, &current_module_path)
                && !contexts
                    .iter()
                    .any(|(path, _)| path == &current_module_path)
            {
                contexts.push((current_module_path, module_suffix.to_vec()));
            }
        }
    }
    contexts
}

fn lexical_module_suffixes(extracted: &ExtractedPaths) -> Vec<&[String]> {
    let mut suffixes = Vec::new();
    for extracted_path in extracted.use_paths.iter().chain(&extracted.expr_paths) {
        let module_suffix = extracted_path.module_suffix.as_slice();
        if !suffixes.contains(&module_suffix) {
            suffixes.push(module_suffix);
        }
    }
    suffixes
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

fn module_path_is_descendant(candidate: &[String], parent: &[String]) -> bool {
    candidate == parent || (candidate.len() > parent.len() && candidate[..parent.len()] == *parent)
}
