use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use quote::ToTokens;
use syn::visit::Visit;

use super::DriverSettings;
use super::facade;
use super::facade::ParentFacadeReferenceUsage;
use super::facade::ParentFacadeUsage;
use super::source_cache;
use super::source_cache::SourceCache;

pub(super) fn child_item_is_exposed_by_other_crate_visible_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let Some(exposing_item_name) = public_item_name(item) else {
            continue;
        };
        if exposing_item_name == item_name {
            continue;
        }
        if !public_item_surface_mentions_name(item, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            &exposing_item_name,
        )? {
            return Ok(true);
        }
    }

    for item in &file.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = impl_self_type_name(item_impl) else {
            continue;
        };
        if self_type_name == item_name {
            continue;
        }
        if !outward_impl_surface_mentions_name(item_impl, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            &self_type_name,
        )? {
            return Ok(true);
        }
    }

    Ok(false)
}

pub(super) fn child_item_is_exposed_by_sibling_boundary_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = facade::parent_boundary_for_child(src_root, child_file) else {
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
            let Some(exposing_item_name) = public_item_name(item) else {
                continue;
            };
            if exposing_item_name == item_name {
                continue;
            }
            if !public_item_surface_mentions_name(item, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                src_root,
                candidate_file,
                &exposing_item_name,
            )? {
                return Ok(true);
            }
        }

        for item in &file.items {
            let syn::Item::Impl(item_impl) = item else {
                continue;
            };
            let Some(self_type_name) = impl_self_type_name(item_impl) else {
                continue;
            };
            if self_type_name == item_name {
                continue;
            }
            if !outward_impl_surface_mentions_name(item_impl, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                src_root,
                candidate_file,
                &self_type_name,
            )? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub(super) fn impl_item_is_exposed_by_exported_self_type(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = impl_self_type_name(item_impl) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let outward = item_impl.trait_.is_some();
            let is_target = match impl_item {
                syn::ImplItem::Fn(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.sig.ident == item_name =>
                {
                    true
                },
                syn::ImplItem::Const(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                syn::ImplItem::Type(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
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
                    src_root,
                    check_file,
                    &self_type_name,
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
            syn::Item::Struct(item) => &item.ident,
            syn::Item::Enum(item) => &item.ident,
            syn::Item::Type(item) => &item.ident,
            syn::Item::Union(item) => &item.ident,
            _ => continue,
        };
        if name == type_name {
            return true;
        }
    }
    false
}

pub(super) fn parent_boundary_public_signature_exposes_child_used_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = facade::parent_boundary_for_child(src_root, child_file) else {
        return Ok(false);
    };

    let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
        return Ok(false);
    };

    let mut exposing_names = Vec::new();
    for item in &file.items {
        let Some(exposing_item_name) = public_item_name(item) else {
            continue;
        };
        if public_item_surface_mentions_name(item, item_name) {
            exposing_names.push(exposing_item_name);
        }
    }

    if exposing_names.is_empty() {
        return Ok(false);
    }

    for source_file in source_cache.source_files_under(src_root) {
        if source_file == parent_boundary.boundary_file
            || source_file.starts_with(&parent_boundary.subtree_root)
        {
            continue;
        }
        let Some(current_module_path) =
            source_cache::module_path_from_source_file(src_root, source_file)
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

pub(super) fn type_is_exposed_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    Ok(facade::parent_facade_export_status(
        source_cache,
        settings,
        src_root,
        child_file,
        item_name,
    )?
    .is_some_and(|status| status.usage == ParentFacadeUsage::UsedOutsideParentSubtree)
        || facade::public_reexport_exists_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            item_name,
        )?
        || child_item_is_exposed_by_other_crate_visible_signature(
            source_cache,
            settings,
            src_root,
            child_file,
            item_name,
        )?
        || child_item_is_exposed_by_sibling_boundary_signature(
            source_cache,
            settings,
            src_root,
            child_file,
            item_name,
        )?
        || parent_boundary_public_signature_exposes_child_used_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            item_name,
        )?)
}

pub(super) fn public_item_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.sig.ident.to_string())
        },
        syn::Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        _ => None,
    }
}

pub(super) fn public_item_surface_mentions_name(item: &syn::Item, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        syn::Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        syn::Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for variant in &item.variants {
                match &variant.fields {
                    syn::Fields::Named(fields) => {
                        for field in &fields.named {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    syn::Fields::Unnamed(fields) => {
                        for field in &fields.unnamed {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    syn::Fields::Unit => {},
                }
            }
        },
        syn::Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_signature(&item.sig);
        },
        syn::Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        syn::Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            match &item.fields {
                syn::Fields::Named(fields) => {
                    for field in &fields.named {
                        visitor.visit_type(&field.ty);
                    }
                },
                syn::Fields::Unnamed(fields) => {
                    for field in &fields.unnamed {
                        visitor.visit_type(&field.ty);
                    }
                },
                syn::Fields::Unit => {},
            }
        },
        syn::Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for trait_item in &item.items {
                match trait_item {
                    syn::TraitItem::Fn(item) => visitor.visit_signature(&item.sig),
                    syn::TraitItem::Type(item) => {
                        if let Some((_, ty)) = &item.default {
                            visitor.visit_type(ty);
                        }
                    },
                    syn::TraitItem::Const(item) => visitor.visit_type(&item.ty),
                    _ => {},
                }
            }
        },
        syn::Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        _ => {},
    }
    visitor.found
}

pub(super) fn impl_self_type_name(item_impl: &syn::ItemImpl) -> Option<String> {
    let syn::Type::Path(type_path) = item_impl.self_ty.as_ref() else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub(super) fn outward_impl_surface_mentions_name(
    item_impl: &syn::ItemImpl,
    item_name: &str,
) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    let mut found_public_surface = false;
    let outward = item_impl.trait_.is_some();

    for impl_item in &item_impl.items {
        match impl_item {
            syn::ImplItem::Fn(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_signature(&item.sig);
                found_public_surface = true;
            },
            syn::ImplItem::Const(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            syn::ImplItem::Type(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            _ => {},
        }
    }

    found_public_surface && visitor.found
}

fn attributes_mention_name(attrs: &[syn::Attribute], item_name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attribute_tokens_mention_name(attr, item_name))
}

fn attribute_tokens_mention_name(attr: &syn::Attribute, item_name: &str) -> bool {
    fn token_tree_mentions_name(tree: &proc_macro2::TokenTree, item_name: &str) -> bool {
        match tree {
            proc_macro2::TokenTree::Group(group) => group
                .stream()
                .into_iter()
                .any(|tree| token_tree_mentions_name(&tree, item_name)),
            proc_macro2::TokenTree::Ident(ident) => ident == item_name,
            proc_macro2::TokenTree::Literal(literal) => {
                literal
                    .to_string()
                    .trim_matches('"')
                    .trim_matches('r')
                    .trim_matches('#')
                    == item_name
            },
            proc_macro2::TokenTree::Punct(_) => false,
        }
    }

    attr.meta
        .to_token_stream()
        .into_iter()
        .any(|tree| token_tree_mentions_name(&tree, item_name))
}

struct ItemSurfaceReferenceVisitor<'a> {
    item_name: &'a str,
    found:     bool,
}

impl<'a> ItemSurfaceReferenceVisitor<'a> {
    const fn new(item_name: &'a str) -> Self {
        Self {
            item_name,
            found: false,
        }
    }
}

impl<'ast> Visit<'ast> for ItemSurfaceReferenceVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.found {
            return;
        }
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == self.item_name)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_path(self, path);
    }
}
