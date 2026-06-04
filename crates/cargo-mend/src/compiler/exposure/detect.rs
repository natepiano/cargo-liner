use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use syn::ImplItem;
use syn::Item;
use syn::Visibility;

use super::visitor;
use crate::compiler::facade;
use crate::compiler::facade::ParentFacadeReferenceUsage;
use crate::compiler::facade::ParentFacadeUsage;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache;
use crate::compiler::source_cache::SourceCache;

/// `(file, item)` pairs already on the exposure-evaluation stack.
///
/// Two public items whose signatures mention each other (`Alpha` holds a
/// `Beta` field, `Beta` holds an `Alpha` field) would otherwise recurse
/// through `type_is_exposed_outside_parent` forever and overflow the stack.
/// A revisited pair contributes no new exposure path, so it evaluates to
/// `false` and any real exposure is found on another branch of the walk.
type VisitedItems = HashSet<(PathBuf, String)>;

pub fn child_item_is_exposed_by_other_crate_visible_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    crate_visible_signature_exposes_item(
        source_cache,
        settings,
        source_root,
        child_file,
        item_name,
        &mut VisitedItems::new(),
    )
}

fn crate_visible_signature_exposes_item(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let Some(exposing_item_name) = visitor::public_item_name(item) else {
            continue;
        };
        if exposing_item_name == item_name {
            continue;
        }
        if !visitor::public_item_surface_mentions_name(item, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            &exposing_item_name,
            visited,
        )? {
            return Ok(true);
        }
    }

    for item in &file.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = visitor::impl_self_type_name(item_impl) else {
            continue;
        };
        if self_type_name == item_name {
            continue;
        }
        if !visitor::outward_impl_surface_mentions_name(item_impl, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            &self_type_name,
            visited,
        )? {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn child_item_is_exposed_by_sibling_boundary_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    sibling_boundary_signature_exposes_item(
        source_cache,
        settings,
        source_root,
        child_file,
        item_name,
        &mut VisitedItems::new(),
    )
}

fn sibling_boundary_signature_exposes_item(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
) -> Result<bool> {
    let Some(parent_boundary) = facade::parent_boundary_for_child(source_root, child_file) else {
        return Ok(false);
    };

    for candidate_file in source_cache.source_files_under(&parent_boundary.subtree_root) {
        if candidate_file == child_file || candidate_file == parent_boundary.boundary_file {
            continue;
        }

        let Some(file) = source_cache.parsed_file(candidate_file) else {
            continue;
        };

        for item in &file.items {
            let Some(exposing_item_name) = visitor::public_item_name(item) else {
                continue;
            };
            if exposing_item_name == item_name {
                continue;
            }
            if !visitor::public_item_surface_mentions_name(item, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                source_root,
                candidate_file,
                &exposing_item_name,
                visited,
            )? {
                return Ok(true);
            }
        }

        for item in &file.items {
            let Item::Impl(item_impl) = item else {
                continue;
            };
            let Some(self_type_name) = visitor::impl_self_type_name(item_impl) else {
                continue;
            };
            if self_type_name == item_name {
                continue;
            }
            if !visitor::outward_impl_surface_mentions_name(item_impl, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                source_root,
                candidate_file,
                &self_type_name,
                visited,
            )? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn impl_item_is_exposed_by_exported_self_type(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    let mut visited = VisitedItems::new();
    for item in &file.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = visitor::impl_self_type_name(item_impl) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let outward = item_impl.trait_.is_some();
            let is_target = match impl_item {
                ImplItem::Fn(item)
                    if (outward || matches!(item.vis, Visibility::Public(_)))
                        && item.sig.ident == item_name =>
                {
                    true
                },
                ImplItem::Const(item)
                    if (outward || matches!(item.vis, Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                ImplItem::Type(item)
                    if (outward || matches!(item.vis, Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                _ => false,
            };

            if is_target {
                let definition_file =
                    find_type_definition_file(source_cache, child_file, &self_type_name);
                let check_file = definition_file.as_deref().unwrap_or(child_file);
                if type_is_exposed_outside_parent(
                    source_cache,
                    settings,
                    source_root,
                    check_file,
                    &self_type_name,
                    &mut visited,
                )? {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

/// When an `impl` block for a type lives in a different child module than the
/// type definition (e.g. `impl App` in `focus.rs` while `struct App` is in
/// `types.rs`), the exposure check must use the definition file — not the impl
/// file — so that `parent_facade_export_status` resolves the correct child
/// module name.
///
/// Returns `Some(path)` if the type is defined in a sibling file, `None` if it
/// is defined in `child_file` itself or cannot be located.
fn find_type_definition_file(
    source_cache: &SourceCache,
    child_file: &Path,
    type_name: &str,
) -> Option<PathBuf> {
    if file_defines_type(source_cache, child_file, type_name) {
        return None;
    }

    let parent_dir = child_file.parent()?;
    for path in source_cache.source_files_under(parent_dir) {
        if path == child_file {
            continue;
        }
        if file_defines_type(source_cache, path, type_name) {
            return Some(path.to_path_buf());
        }
    }

    None
}

fn file_defines_type(source_cache: &SourceCache, path: &Path, type_name: &str) -> bool {
    let Some(file) = source_cache.parsed_file(path) else {
        return false;
    };
    for item in &file.items {
        let name = match item {
            Item::Struct(item) => &item.ident,
            Item::Enum(item) => &item.ident,
            Item::Type(item) => &item.ident,
            Item::Union(item) => &item.ident,
            _ => continue,
        };
        if name == type_name {
            return true;
        }
    }
    false
}

pub fn parent_boundary_public_signature_exposes_child_used_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = facade::parent_boundary_for_child(source_root, child_file) else {
        return Ok(false);
    };

    let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
        return Ok(false);
    };

    let mut exposing_names = Vec::new();
    for item in &file.items {
        let Some(exposing_item_name) = visitor::public_item_name(item) else {
            continue;
        };
        if visitor::public_item_surface_mentions_name(item, item_name) {
            exposing_names.push(exposing_item_name);
        }
    }

    if exposing_names.is_empty() {
        return Ok(false);
    }

    for source_file in source_cache.source_files_under(source_root) {
        if source_file == parent_boundary.boundary_file
            || source_file.starts_with(&parent_boundary.subtree_root)
        {
            continue;
        }
        let Some(current_module_path) =
            source_cache::module_path_from_source_file(source_root, source_file)
        else {
            continue;
        };
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        if !matches!(
            facade::source_references_parent_export(
                extracted,
                &current_module_path,
                &parent_boundary.module_path,
                &exposing_names,
            ),
            ParentFacadeReferenceUsage::None
        ) {
            return Ok(true);
        }
    }

    if facade::workspace_source_mentions_parent_export_literal(
        source_cache,
        settings,
        &parent_boundary,
        &exposing_names,
    )? {
        return Ok(true);
    }

    Ok(false)
}

fn type_is_exposed_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
) -> Result<bool> {
    if !visited.insert((child_file.to_path_buf(), item_name.to_string())) {
        return Ok(false);
    }
    Ok(facade::parent_facade_export_status(
        source_cache,
        settings,
        source_root,
        child_file,
        item_name,
    )?
    .is_some_and(|status| status.usage == ParentFacadeUsage::UsedOutsideSubtree)
        || facade::public_reexport_exists_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?
        || crate_visible_signature_exposes_item(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
            visited,
        )?
        || sibling_boundary_signature_exposes_item(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
            visited,
        )?
        || parent_boundary_public_signature_exposes_child_used_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?)
}
