use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use proc_macro2::TokenTree;
use quote::ToTokens;
use syn::Attribute;
use syn::Fields;
use syn::ImplItem;
use syn::Item;
use syn::ItemImpl;
use syn::TraitItem;
use syn::Type;
use syn::visit::Visit;

use super::facade;
use super::facade::ParentFacadeReferenceUsage;
use super::facade::ParentFacadeUsage;
use super::settings::DriverSettings;
use super::source_cache;
use super::source_cache::SourceCache;

pub(super) fn child_item_is_exposed_by_other_crate_visible_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
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
            source_root,
            child_file,
            &exposing_item_name,
        )? {
            return Ok(true);
        }
    }

    for item in &file.items {
        let Item::Impl(item_impl) = item else {
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
            source_root,
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
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
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
                source_root,
                candidate_file,
                &exposing_item_name,
            )? {
                return Ok(true);
            }
        }

        for item in &file.items {
            let Item::Impl(item_impl) = item else {
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
                source_root,
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
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = impl_self_type_name(item_impl) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let outward = item_impl.trait_.is_some();
            let is_target = match impl_item {
                ImplItem::Fn(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.sig.ident == item_name =>
                {
                    true
                },
                ImplItem::Const(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                ImplItem::Type(item)
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
                    source_root,
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

pub(super) fn parent_boundary_public_signature_exposes_child_used_outside_parent(
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

pub(super) fn type_is_exposed_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    source_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    Ok(facade::parent_facade_export_status(
        source_cache,
        settings,
        source_root,
        child_file,
        item_name,
    )?
    .is_some_and(|status| status.usage == ParentFacadeUsage::UsedOutsideParentSubtree)
        || facade::public_reexport_exists_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?
        || child_item_is_exposed_by_other_crate_visible_signature(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?
        || child_item_is_exposed_by_sibling_boundary_signature(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?
        || parent_boundary_public_signature_exposes_child_used_outside_parent(
            source_cache,
            settings,
            source_root,
            child_file,
            item_name,
        )?)
}

pub(super) fn public_item_name(item: &Item) -> Option<String> {
    match item {
        Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.sig.ident.to_string())
        },
        Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        _ => None,
    }
}

pub(super) fn public_item_surface_mentions_name(item: &Item, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for variant in &item.variants {
                match &variant.fields {
                    Fields::Named(fields) => {
                        for field in &fields.named {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    Fields::Unnamed(fields) => {
                        for field in &fields.unnamed {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    Fields::Unit => {},
                }
            }
        },
        Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_signature(&item.sig);
        },
        Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            match &item.fields {
                Fields::Named(fields) => {
                    for field in &fields.named {
                        visitor.visit_type(&field.ty);
                    }
                },
                Fields::Unnamed(fields) => {
                    for field in &fields.unnamed {
                        visitor.visit_type(&field.ty);
                    }
                },
                Fields::Unit => {},
            }
        },
        Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for trait_item in &item.items {
                match trait_item {
                    TraitItem::Fn(item) => visitor.visit_signature(&item.sig),
                    TraitItem::Type(item) => {
                        if let Some((_, ty)) = &item.default {
                            visitor.visit_type(ty);
                        }
                    },
                    TraitItem::Const(item) => visitor.visit_type(&item.ty),
                    _ => {},
                }
            }
        },
        Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        _ => {},
    }
    visitor.found == SurfaceReferenceMatch::Found
}

pub(super) fn impl_self_type_name(item_impl: &ItemImpl) -> Option<String> {
    let Type::Path(type_path) = item_impl.self_ty.as_ref() else {
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

pub(super) fn outward_impl_surface_mentions_name(item_impl: &ItemImpl, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    let mut found_public_surface = false;
    let outward = item_impl.trait_.is_some();

    for impl_item in &item_impl.items {
        match impl_item {
            ImplItem::Fn(item) if outward || matches!(item.vis, syn::Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_signature(&item.sig);
                found_public_surface = true;
            },
            ImplItem::Const(item) if outward || matches!(item.vis, syn::Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            ImplItem::Type(item) if outward || matches!(item.vis, syn::Visibility::Public(_)) => {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            _ => {},
        }
    }

    found_public_surface && visitor.found == SurfaceReferenceMatch::Found
}

fn attributes_mention_name(attrs: &[Attribute], item_name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attribute_tokens_mention_name(attr, item_name))
}

fn attribute_tokens_mention_name(attr: &Attribute, item_name: &str) -> bool {
    fn token_tree_mentions_name(tree: &TokenTree, item_name: &str) -> bool {
        match tree {
            TokenTree::Group(group) => group
                .stream()
                .into_iter()
                .any(|tree| token_tree_mentions_name(&tree, item_name)),
            TokenTree::Ident(ident) => ident == item_name,
            TokenTree::Literal(literal) => {
                literal
                    .to_string()
                    .trim_matches('"')
                    .trim_matches('r')
                    .trim_matches('#')
                    == item_name
            },
            TokenTree::Punct(_) => false,
        }
    }

    attr.meta
        .to_token_stream()
        .into_iter()
        .any(|tree| token_tree_mentions_name(&tree, item_name))
}

struct ItemSurfaceReferenceVisitor<'a> {
    item_name: &'a str,
    found:     SurfaceReferenceMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SurfaceReferenceMatch {
    Missing,
    Found,
}

impl<'a> ItemSurfaceReferenceVisitor<'a> {
    const fn new(item_name: &'a str) -> Self {
        Self {
            item_name,
            found: SurfaceReferenceMatch::Missing,
        }
    }
}

impl<'ast> Visit<'ast> for ItemSurfaceReferenceVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.found == SurfaceReferenceMatch::Found {
            return;
        }
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == self.item_name)
        {
            self.found = SurfaceReferenceMatch::Found;
            return;
        }
        syn::visit::visit_path(self, path);
    }
}
