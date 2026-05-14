use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use syn::File;
use syn::Item;
use syn::ItemUse;
use syn::UseTree;
use syn::Visibility;

use super::settings::DriverSettings;
use super::source_cache;
use super::source_cache::ExtractedPaths;
use super::source_cache::PathOrigin;
use super::source_cache::SourceCache;
use super::source_cache::UseRename;
use crate::constants::MODULE_PATH_SEPARATOR;
use crate::constants::PATH_KEYWORD_CRATE;
use crate::constants::PATH_KEYWORD_SELF;
use crate::constants::PATH_KEYWORD_SUPER;
use crate::constants::RUST_LIB_FILE;
use crate::constants::RUST_MAIN_FILE;
use crate::constants::RUST_MODULE_FILE;
use crate::module_paths;

#[derive(Debug, Clone)]
pub(super) struct ParentBoundary {
    pub boundary_file: PathBuf,
    pub subtree_root:  PathBuf,
    pub module_path:   Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ParentFacadeExports {
    pub explicit:      Vec<String>,
    pub fix_supported: ParentFacadeFixSupport,
    pub visibility:    Option<ParentFacadeVisibility>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ParentFacadeExportStatus {
    pub usage:           ParentFacadeUsage,
    pub fix_supported:   ParentFacadeFixSupport,
    pub visibility:      ParentFacadeVisibility,
    pub parent_path:     PathBuf,
    pub parent_rel_path: String,
    pub parent_line:     usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum ParentFacadeFixSupport {
    #[default]
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentFacadeUsage {
    Unused,
    UsedInsideParentSubtreeByRelativeImport,
    UsedInsideParentSubtreeByRelativePath,
    UsedInsideParentSubtreeByCrateImport,
    UsedInsideParentSubtreeByCratePath,
    UsedOutsideParentSubtree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentFacadeVisibility {
    Public,
    Super,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

/// Check whether `root_module` (lib.rs / main.rs) re-exports `item_name`
/// from the child module that `child_file` belongs to.
pub(super) fn root_module_exports_item(
    source_cache: &SourceCache,
    root_module: &Path,
    child_file: &Path,
    item_name: &str,
) -> bool {
    let Some(child_module_name) = module_paths::module_name_for_child_boundary_file(child_file)
    else {
        return false;
    };
    let Some(file) = source_cache.parsed_file(root_module) else {
        return false;
    };
    let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);
    !exports.explicit.is_empty()
}

pub(super) fn parent_facade_export_status(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<Option<ParentFacadeExportStatus>> {
    let Some(initial_boundary) = parent_boundary_for_child(source_root, child_file) else {
        return Ok(None);
    };

    // Walk up from the immediate parent through ancestors until we find a
    // boundary that re-exports `item_name`, or run out of ancestors.
    let mut current_child: PathBuf = child_file.to_path_buf();
    let mut parent_boundary = initial_boundary;

    let exported_names = loop {
        let Some(child_module_name) =
            module_paths::module_name_for_child_boundary_file(&current_child)
        else {
            return Ok(None);
        };

        let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
            return Ok(None);
        };
        let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);

        if !exports.explicit.is_empty() {
            break exports;
        }

        // Not found at this level — walk up to the next ancestor.
        current_child.clone_from(&parent_boundary.boundary_file);
        let Some(next_boundary) = parent_of_boundary(source_root, &current_child) else {
            return Ok(None);
        };
        parent_boundary = next_boundary;
    };

    let parent_rel_path = parent_boundary
        .boundary_file
        .strip_prefix(source_root)
        .unwrap_or(&parent_boundary.boundary_file)
        .to_string_lossy()
        .replace('\\', "/");
    let parent_source = source_cache.read_source(&parent_boundary.boundary_file)?;
    let parent_line = source_cache::first_line_matching(parent_source, item_name).unwrap_or(1);

    let usage = scan_facade_usage(
        source_cache,
        settings,
        source_root,
        &parent_boundary,
        &exported_names,
    )?;

    Ok(Some(ParentFacadeExportStatus {
        usage,
        fix_supported: exported_names.fix_supported,
        visibility: exported_names
            .visibility
            .unwrap_or(ParentFacadeVisibility::Public),
        parent_path: parent_boundary.boundary_file,
        parent_rel_path,
        parent_line,
    }))
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
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate) => {
                if matches!(usage, ParentFacadeUsage::Unused)
                    && source_path.starts_with(&parent_boundary.subtree_root)
                {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByCratePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
        }
    }

    if !matches!(usage, ParentFacadeUsage::UsedOutsideParentSubtree)
        && workspace_source_mentions_parent_export_literal(
            source_cache,
            settings,
            parent_boundary,
            &exported_names.explicit,
        )?
    {
        usage = ParentFacadeUsage::UsedOutsideParentSubtree;
    }

    Ok(usage)
}

pub(super) fn workspace_source_mentions_parent_export_literal(
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

pub(super) fn parent_boundary_for_child(
    source_root: &Path,
    child_file: &Path,
) -> Option<ParentBoundary> {
    let parent_dir = child_file.parent()?;
    let parent_module_rs = parent_dir.join(RUST_MODULE_FILE);
    if parent_module_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_module_rs,
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_dir(source_root, parent_dir)?,
        });
    }

    let parent_file = parent_dir.with_extension("rs");
    if parent_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_file.clone(),
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_boundary_file(source_root, &parent_file)?,
        });
    }

    None
}

/// Find the parent boundary of an existing boundary file itself.
///
/// `parent_boundary_for_child` cannot be called on a `mod.rs` file because it
/// would find itself.  This helper handles both `mod.rs` and named boundary
/// files (e.g. `tools.rs`).
pub(super) fn parent_of_boundary(
    source_root: &Path,
    boundary_file: &Path,
) -> Option<ParentBoundary> {
    if boundary_file.file_name()?.to_str() != Some(RUST_MODULE_FILE) {
        return parent_boundary_for_child(source_root, boundary_file);
    }

    // For mod.rs the enclosing directory IS the module, so go up one more
    // level to reach the parent module's directory.
    let container_dir = boundary_file.parent()?.parent()?;

    let module_rs = container_dir.join(RUST_MODULE_FILE);
    if module_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: module_rs,
            subtree_root:  container_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_dir(source_root, container_dir)?,
        });
    }

    let named_file = container_dir.with_extension("rs");
    if named_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: named_file.clone(),
            subtree_root:  container_dir.to_path_buf(),
            module_path:   source_cache::module_path_from_boundary_file(source_root, &named_file)?,
        });
    }

    for name in [RUST_LIB_FILE, RUST_MAIN_FILE] {
        let root = container_dir.join(name);
        if root.is_file() {
            return Some(ParentBoundary {
                boundary_file: root,
                subtree_root:  container_dir.to_path_buf(),
                module_path:   Vec::new(),
            });
        }
    }

    None
}

pub(super) fn exported_names_from_parent_boundary(
    file: &File,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeExports {
    let mut exported = ParentFacadeExports::default();
    for item in &file.items {
        let Item::Use(item_use) = item else {
            continue;
        };
        let Some(visibility) = parent_facade_visibility(&item_use.vis) else {
            continue;
        };
        exported.visibility = Some(exported.visibility.map_or(visibility, |existing| existing));
        collect_matching_pub_use_exports(item_use, child_module_name, item_name, &mut exported);
    }
    exported.explicit.sort();
    exported.explicit.dedup();
    exported
}

fn collect_matching_pub_use_exports(
    item_use: &ItemUse,
    child_module_name: &str,
    item_name: &str,
    exported: &mut ParentFacadeExports,
) {
    if pub_use_is_fix_supported(&item_use.tree, child_module_name, item_name) {
        exported.fix_supported = ParentFacadeFixSupport::Supported;
    }
    let mut paths = Vec::new();
    source_cache::flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
    for path in paths {
        let normalized = if path
            .first()
            .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
        {
            &path[1..]
        } else {
            &path[..]
        };
        if normalized.len() >= 2
            && normalized[0] == child_module_name
            && normalized[1..].iter().any(|segment| segment == item_name)
            && let Some(export_name) = normalized.last()
        {
            exported.explicit.push(export_name.clone());
        }
    }
}

fn pub_use_is_fix_supported(tree: &UseTree, child_module_name: &str, item_name: &str) -> bool {
    pub_use_is_fix_supported_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn pub_use_is_fix_supported_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            pub_use_is_fix_supported_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = if prefix
                .first()
                .is_some_and(|segment| segment == PATH_KEYWORD_SELF)
            {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            normalized.len() == 1 && normalized[0] == child_module_name && name.ident == item_name
        },
        UseTree::Group(group) => group.items.iter().any(|item| {
            pub_use_is_fix_supported_with_prefix(prefix.clone(), item, child_module_name, item_name)
        }),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

pub(super) fn parent_facade_visibility(vis: &Visibility) -> Option<ParentFacadeVisibility> {
    match vis {
        Visibility::Public(_) => Some(ParentFacadeVisibility::Public),
        Visibility::Restricted(restricted)
            if restricted.path.segments.len() == 1
                && restricted.path.segments[0].ident == PATH_KEYWORD_SUPER =>
        {
            Some(ParentFacadeVisibility::Super)
        },
        _ => None,
    }
}

pub(super) fn source_references_parent_export(
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

pub(super) fn public_reexport_exists_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = parent_boundary_for_child(source_root, child_file) else {
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
            let Some(_) = parent_facade_visibility(&item_use.vis) else {
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

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::ParentFacadeExports;
    use super::ParentFacadeFixSupport;
    use super::ParentFacadeVisibility;
    use super::exported_names_from_parent_boundary;

    #[test]
    fn grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
        let file = syn::parse_file(source).unwrap();
        let exports =
            exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
        assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn multiline_grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.explicit, vec!["Thing".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }

    #[test]
    fn grouped_parent_pub_use_with_rename_is_manual_only() {
        let source = "pub use child::{Thing as RenamedThing, Other};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(
            exports,
            ParentFacadeExports {
                explicit:      vec!["RenamedThing".to_string()],
                fix_supported: ParentFacadeFixSupport::Unsupported,
                visibility:    Some(ParentFacadeVisibility::Public),
            }
        );

        let exports = exported_names_from_parent_boundary(&file, "child", "Other");
        assert_eq!(exports.explicit, vec!["Other".to_string()]);
        assert_eq!(exports.fix_supported, ParentFacadeFixSupport::Supported);
    }
}
