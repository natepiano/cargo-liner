use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Result;
use rustc_hir::def::DefKind;
use rustc_hir::def::Res;
use rustc_middle::ty;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;
use syn::Item;

use super::visitor;
use crate::compiler::facade;
use crate::compiler::facade::ModuleSourceMap;
use crate::compiler::facade::ParentFacadeUsage;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache::SourceCache;

/// Items already on the exposure-evaluation stack.
///
/// Two public items whose signatures mention each other (`Alpha` holds a
/// `Beta` field, `Beta` holds an `Alpha` field) would otherwise recurse
/// through `type_is_exposed_outside_parent` forever and overflow the stack.
/// A revisited item contributes no new exposure path, so it evaluates to
/// `false` and any real exposure is found on another branch of the walk.
type VisitedItems = HashSet<LocalDefId>;
type FacadeExposure<'a> = dyn FnMut(LocalDefId, &Path, &str) -> Result<bool> + 'a;

pub struct ExposureContext<'source, 'tcx> {
    pub source_cache:   &'source SourceCache,
    pub settings:       &'source DriverSettings,
    pub source_root:    &'source Path,
    pub tcx:            TyCtxt<'tcx>,
    pub module_sources: &'source ModuleSourceMap,
}

struct ModuleScope<'syntax> {
    module: LocalDefId,
    items:  &'syntax [Item],
}

pub fn child_item_is_exposed_by_other_crate_visible_signature(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    crate_visible_signature_exposes_item(
        ctx,
        item_def_id,
        child_file,
        item_name,
        &mut VisitedItems::new(),
        facade_exposes,
    )
}

fn crate_visible_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    let Some(file) = ctx.source_cache.parsed_file(child_file) else {
        return Ok(false);
    };
    let item_module: LocalDefId = ctx.tcx.parent_module_from_def_id(item_def_id).into();

    for module_scope in module_scopes(ctx, child_file, &file.items) {
        if module_scope.module == item_module
            && module_signature_exposes_item(
                ctx,
                module_scope.module,
                module_scope.items,
                child_file,
                item_name,
                visited,
                facade_exposes,
            )?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn module_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    module: LocalDefId,
    items: &[Item],
    source_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    for item in items {
        let Some(exposing_item_name) = visitor::public_item_name(item) else {
            continue;
        };
        if exposing_item_name == item_name
            || !visitor::public_item_surface_mentions_name(item, item_name)
        {
            continue;
        }
        let Some(exposing_item_def_id) = module_item_def_id(ctx.tcx, module, &exposing_item_name)
        else {
            continue;
        };
        if type_is_exposed_outside_parent(
            ctx,
            exposing_item_def_id,
            source_file,
            &exposing_item_name,
            visited,
            facade_exposes,
        )? {
            return Ok(true);
        }
    }

    for item in items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = visitor::impl_self_type_name(item_impl) else {
            continue;
        };
        if self_type_name == item_name
            || !visitor::outward_impl_surface_mentions_name(item_impl, item_name)
        {
            continue;
        }
        let Some(self_type_def_id) = module_item_def_id(ctx.tcx, module, &self_type_name) else {
            continue;
        };
        if type_is_exposed_outside_parent(
            ctx,
            self_type_def_id,
            source_file,
            &self_type_name,
            visited,
            facade_exposes,
        )? {
            return Ok(true);
        }
    }

    Ok(false)
}

pub fn child_item_is_exposed_by_sibling_boundary_signature(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    sibling_boundary_signature_exposes_item(
        ctx,
        item_def_id,
        item_name,
        &mut VisitedItems::new(),
        facade_exposes,
    )
}

fn sibling_boundary_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    let Some(parent_boundary) = facade::logical_parent_boundary_for_child(ctx.tcx, item_def_id)
    else {
        return Ok(false);
    };
    let child_module: LocalDefId = ctx.tcx.parent_module_from_def_id(item_def_id).into();

    for candidate_file in ctx.source_cache.source_files_under(ctx.source_root) {
        let Some(file) = ctx.source_cache.parsed_file(candidate_file) else {
            continue;
        };
        for module_scope in module_scopes(ctx, candidate_file, &file.items) {
            let candidate_module = module_scope.module;
            if candidate_module == parent_boundary.module
                || facade::module_is_within(ctx.tcx, candidate_module, child_module)
                || !facade::module_is_within(ctx.tcx, candidate_module, parent_boundary.module)
            {
                continue;
            }

            if module_signature_exposes_item(
                ctx,
                candidate_module,
                module_scope.items,
                candidate_file,
                item_name,
                visited,
                facade_exposes,
            )? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

pub fn impl_item_is_exposed_by_exported_self_type(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    _: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    if !matches!(
        ctx.tcx.def_kind(item_def_id),
        DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy
    ) {
        return Ok(false);
    }
    let impl_def_id = ctx.tcx.parent(item_def_id.to_def_id());
    let Some(self_type_def_id) = ctx
        .tcx
        .type_of(impl_def_id)
        .instantiate_identity()
        .skip_normalization()
        .ty_adt_def()
        .map(ty::AdtDef::did)
        .and_then(DefId::as_local)
    else {
        return Ok(false);
    };
    let Some(definition_file) = real_file_path(ctx.tcx, ctx.tcx.def_span(self_type_def_id)) else {
        return Ok(false);
    };
    let self_type_name = ctx.tcx.item_name(self_type_def_id.to_def_id()).to_string();
    type_is_exposed_outside_parent(
        ctx,
        self_type_def_id,
        &definition_file,
        &self_type_name,
        &mut VisitedItems::new(),
        facade_exposes,
    )
}

pub fn parent_boundary_public_signature_exposes_child_used_outside_parent(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = facade::logical_parent_boundary_for_child(ctx.tcx, item_def_id)
    else {
        return Ok(false);
    };

    let mut exposing_names = Vec::new();
    for boundary_file in ctx.module_sources.source_files(parent_boundary.module) {
        let Some(file) = ctx.source_cache.parsed_file(boundary_file) else {
            continue;
        };
        for module_scope in module_scopes(ctx, boundary_file, &file.items) {
            if module_scope.module != parent_boundary.module {
                continue;
            }
            for item in module_scope.items {
                let Some(exposing_item_name) = visitor::public_item_name(item) else {
                    continue;
                };
                if visitor::public_item_surface_mentions_name(item, item_name)
                    && module_item_def_id(ctx.tcx, parent_boundary.module, &exposing_item_name)
                        .is_some()
                    && !exposing_names.contains(&exposing_item_name)
                {
                    exposing_names.push(exposing_item_name);
                }
            }
        }
    }

    if exposing_names.is_empty() {
        return Ok(false);
    }

    if facade::path_exists_outside_module(
        ctx.source_cache,
        ctx.source_root,
        ctx.tcx,
        ctx.module_sources,
        &parent_boundary.module_path,
        &exposing_names,
    ) {
        return Ok(true);
    }

    if facade::workspace_source_parent_export_literal_usage(
        ctx.source_cache,
        ctx.settings,
        ctx.tcx,
        ctx.module_sources,
        &parent_boundary.module_path,
        &exposing_names,
    )?
    .values()
    .any(|usage| matches!(usage, ParentFacadeUsage::UsedOutsideSubtree))
    {
        return Ok(true);
    }

    Ok(false)
}

fn type_is_exposed_outside_parent(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<bool> {
    if !visited.insert(item_def_id) {
        return Ok(false);
    }
    Ok(facade_exposes(item_def_id, child_file, item_name)?
        || crate_visible_signature_exposes_item(
            ctx,
            item_def_id,
            child_file,
            item_name,
            visited,
            facade_exposes,
        )?
        || sibling_boundary_signature_exposes_item(
            ctx,
            item_def_id,
            item_name,
            visited,
            facade_exposes,
        )?
        || parent_boundary_public_signature_exposes_child_used_outside_parent(
            ctx,
            item_def_id,
            item_name,
        )?)
}

fn module_item_def_id(tcx: TyCtxt<'_>, module: LocalDefId, item_name: &str) -> Option<LocalDefId> {
    tcx.module_children_local(module).iter().find_map(|child| {
        if child.ident.name.as_str() != item_name {
            return None;
        }
        match child.res {
            Res::Def(_, def_id) => def_id.as_local(),
            _ => None,
        }
    })
}

fn module_scopes<'syntax>(
    ctx: &ExposureContext<'_, '_>,
    source_file: &Path,
    items: &'syntax [Item],
) -> Vec<ModuleScope<'syntax>> {
    let mut scopes = Vec::new();
    for root_module in ctx
        .module_sources
        .root_modules_for_file(ctx.tcx, source_file)
    {
        collect_module_scopes(ctx.tcx, root_module, items, &mut scopes);
    }
    scopes
}

fn collect_module_scopes<'syntax>(
    tcx: TyCtxt<'_>,
    module: LocalDefId,
    items: &'syntax [Item],
    scopes: &mut Vec<ModuleScope<'syntax>>,
) {
    scopes.push(ModuleScope { module, items });
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        let Some((_, child_items)) = &item_mod.content else {
            continue;
        };
        let source_name = item_mod.ident.to_string();
        let child_name = source_name.strip_prefix("r#").unwrap_or(&source_name);
        let Some(child_module) = module_def_id(tcx, module, child_name) else {
            continue;
        };
        collect_module_scopes(tcx, child_module, child_items, scopes);
    }
}

fn module_def_id(tcx: TyCtxt<'_>, module: LocalDefId, module_name: &str) -> Option<LocalDefId> {
    tcx.module_children_local(module).iter().find_map(|child| {
        if child.ident.name.as_str() != module_name {
            return None;
        }
        match child.res {
            Res::Def(DefKind::Mod, def_id) => def_id.as_local(),
            _ => None,
        }
    })
}

fn real_file_path(tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
    let source_map = tcx.sess.source_map();
    let file = source_map.lookup_char_pos(span.lo()).file;
    match file.name.clone() {
        FileName::Real(real) => real
            .local_path()
            .map(|path| fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())),
        _ => None,
    }
}
