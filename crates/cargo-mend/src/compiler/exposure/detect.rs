use std::cell::Cell;
use std::cell::RefCell;
use std::cmp::Ordering;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::Result;
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use rustc_hir::FieldDef;
use rustc_hir::ItemKind;
use rustc_hir::def::DefKind;
use rustc_hir::def::Res;
use rustc_middle::ty;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::Span;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;
use rustc_span::def_id::LocalModDefId;
use syn::Field;
use syn::ImplItem;
use syn::Item;
use syn::ItemImpl;
use syn::spanned::Spanned;

use super::super::sweep_counters;
use super::visitor;
use super::visitor::ItemSignatureCarrier;
use super::visitor::OutwardDeclaration;
use super::visitor::OutwardDeclarationClassification;
use super::visitor::OutwardDeclarationKind;
use crate::compiler::facade;
use crate::compiler::facade::ModuleSourceMap;
use crate::compiler::facade::ParentFacadeUsage;
use crate::compiler::settings::DriverSettings;
use crate::compiler::source_cache::NameMention;
use crate::compiler::source_cache::SourceCache;
use crate::compiler::visibility;
use crate::compiler::visibility::ReexportIndex;
use crate::compiler::visibility::VisibilityReach;

/// Items already on the exposure-evaluation stack.
///
/// Two public items whose signatures mention each other (`Alpha` holds a
/// `Beta` field, `Beta` holds an `Alpha` field) would otherwise recurse
/// through `type_is_exposed_outside_parent` forever and overflow the stack.
/// A revisited item contributes no new exposure path, so it evaluates to
/// `None` and any real exposure is found on another branch of the walk.
type VisitedItems = FxHashSet<LocalDefId>;
type FacadeExposure<'a> =
    dyn FnMut(LocalDefId, &Path, &str) -> Result<Option<VisibilityReach>> + 'a;

pub(in crate::compiler) struct ExposureContext<'source, 'tcx> {
    pub source_cache:             &'source SourceCache,
    pub settings:                 &'source DriverSettings,
    pub source_root:              &'source Path,
    pub tcx:                      TyCtxt<'tcx>,
    pub module_sources:           &'source ModuleSourceMap,
    pub reexport_index:           &'source ReexportIndex,
    /// Memoizes [`module_scopes`] — see that function for why.
    pub module_scope_cache:       ModuleScopeCache<'source>,
    /// Memoizes [`boundary_scopes`] — see that function for why.
    pub boundary_scope_cache:     BoundaryScopeCache<'source>,
    /// Memoizes [`type_is_exposed_outside_parent`] across every analyzed item.
    pub signature_exposure_cache: &'source SignatureExposureCache,
}

/// The exterior reach [`type_is_exposed_outside_parent`] found for an item,
/// keyed by that item and reused for the rest of the crate's analysis.
///
/// The walk reaches the same item through every signature that mentions it, and
/// the answer depends only on the item: the callback
/// `assess_signature_exposure_allowance` supplies captures the crate-wide
/// `VisibilityContext` and nothing about the item being scanned. One cache
/// therefore serves every item in the crate.
///
/// `cycle_cuts` is what makes reuse sound. `type_is_exposed_outside_parent`
/// answers `None` for an item already on the walk's stack, which under-reports
/// that item's exposure; a result computed while such a cut happened beneath it
/// is an under-approximation, correct only for the stack that produced it.
/// Counting cuts lets a completed call tell whether any occurred within it, and
/// only a call that saw none is stored.
#[derive(Default)]
pub(in crate::compiler) struct SignatureExposureCache {
    exterior_reaches: RefCell<FxHashMap<LocalDefId, Option<VisibilityReach>>>,
    cycle_cuts:       Cell<u64>,
}

/// The result of a [`SignatureExposureCache`] lookup: no memoized answer yet,
/// or the memoized exterior reach — itself either nothing or a
/// [`VisibilityReach`].
#[derive(Clone, Copy)]
enum CachedExteriorReach {
    Absent,
    Unexposed,
    Exposed(VisibilityReach),
}

impl SignatureExposureCache {
    fn exterior_reach(&self, item_def_id: LocalDefId) -> CachedExteriorReach {
        let Some(exterior_reach) = self.exterior_reaches.borrow().get(&item_def_id).copied() else {
            return CachedExteriorReach::Absent;
        };
        exterior_reach.map_or(CachedExteriorReach::Unexposed, CachedExteriorReach::Exposed)
    }

    fn store_exterior_reach(
        &self,
        item_def_id: LocalDefId,
        exterior_reach: Option<VisibilityReach>,
    ) {
        self.exterior_reaches
            .borrow_mut()
            .insert(item_def_id, exterior_reach);
    }

    /// Records that the cycle guard cut a branch, marking every call currently
    /// on the stack as holding an under-approximation.
    fn record_cycle_cut(&self) { self.cycle_cuts.set(self.cycle_cuts.get() + 1); }

    const fn cycle_cuts(&self) -> u64 { self.cycle_cuts.get() }
}

/// The per-file results of [`module_scopes`], keyed by source path.
#[derive(Default)]
pub(in crate::compiler) struct ModuleScopeCache<'syntax> {
    by_file: RefCell<FxHashMap<PathBuf, Rc<[ModuleScope<'syntax>]>>>,
}

impl<'syntax> ModuleScopeCache<'syntax> {
    fn get(&self, source_file: &Path) -> Option<Rc<[ModuleScope<'syntax>]>> {
        self.by_file.borrow().get(source_file).map(Rc::clone)
    }

    fn insert(&self, source_file: &Path, scopes: &Rc<[ModuleScope<'syntax>]>) {
        self.by_file
            .borrow_mut()
            .insert(source_file.to_path_buf(), Rc::clone(scopes));
    }
}

#[derive(Clone, Copy)]
struct ModuleScope<'syntax> {
    module: LocalDefId,
    items:  &'syntax [Item],
}

/// The per-boundary results of [`boundary_scopes`], keyed by boundary module.
#[derive(Default)]
pub(in crate::compiler) struct BoundaryScopeCache<'syntax> {
    by_boundary: RefCell<FxHashMap<LocalDefId, Rc<[BoundaryFileScopes<'syntax>]>>>,
}

impl<'syntax> BoundaryScopeCache<'syntax> {
    fn get(&self, boundary: LocalDefId) -> Option<Rc<[BoundaryFileScopes<'syntax>]>> {
        self.by_boundary.borrow().get(&boundary).map(Rc::clone)
    }

    fn insert(&self, boundary: LocalDefId, scopes: &Rc<[BoundaryFileScopes<'syntax>]>) {
        self.by_boundary
            .borrow_mut()
            .insert(boundary, Rc::clone(scopes));
    }
}

/// The module scopes one file contributes to a boundary.
///
/// Grouped by file because the sweep's name check is per file: checking it once
/// here instead of once per scope is what keeps the boundary index from trading
/// the old file-level prune away.
struct BoundaryFileScopes<'syntax> {
    source_file:   PathBuf,
    module_scopes: Vec<ModuleScope<'syntax>>,
}

#[derive(Clone, Copy)]
struct SignatureTarget<'name> {
    def_id: LocalDefId,
    name:   &'name str,
}

struct ParentBoundaryExposer {
    name:               String,
    item_def_id:        LocalDefId,
    definition_file:    PathBuf,
    signature_carriers: Vec<ResolvedItemSignatureCarrier>,
}

struct ResolvedImplSignatureCarrier {
    associated_item: LocalDefId,
    implementation:  LocalDefId,
    contract:        ImplSignatureContract,
}

struct ResolvedImplSelfType {
    item_def_id:     LocalDefId,
    name:            String,
    definition_file: PathBuf,
}

struct ResolvedImplSignatureSurface {
    self_type:          ResolvedImplSelfType,
    signature_carriers: Vec<ResolvedImplSignatureCarrier>,
}

struct ResolvedFieldSignatureCarrier {
    field: LocalDefId,
}

enum ResolvedItemSignatureCarrier {
    Declaration { declaration: LocalDefId },
    Field(ResolvedFieldSignatureCarrier),
}

enum SourceFieldIdentity {
    Named {
        name:          String,
        byte_position: usize,
    },
    Unnamed {
        field_index:         usize,
        field_byte_position: usize,
        type_byte_position:  usize,
    },
}

enum ImplSignatureContract {
    Inherent,
    Trait { trait_def_id: DefId },
}

#[derive(Clone, Copy)]
enum SignatureCarrierReachSource {
    ContainingModulePath,
    DeclarationPath,
}

#[derive(Clone, Copy)]
enum ImplSignatureCarrierReachSource<'path> {
    Module(SignatureCarrierReachSource),
    ParentBoundary { module_path: &'path [String] },
}

impl SignatureCarrierReachSource {
    fn outward_reach(
        self,
        ctx: &ExposureContext<'_, '_>,
        containing_module: LocalDefId,
        declaration: LocalDefId,
        independent_outward_reach: Option<VisibilityReach>,
    ) -> Option<VisibilityReach> {
        let carrier_reach = match self {
            Self::ContainingModulePath => effective_path_reach(ctx, containing_module)?,
            Self::DeclarationPath => {
                let declaration_reach = effective_path_reach(ctx, declaration)?;
                let private_reach =
                    VisibilityReach::from(Visibility::Restricted(containing_module.to_def_id()));
                matches!(
                    declaration_reach.compare(private_reach, ctx.tcx),
                    Some(Ordering::Greater)
                )
                .then_some(declaration_reach)?
            },
        };
        Some(
            independent_outward_reach.map_or(carrier_reach, |independent_reach| {
                carrier_reach.join(independent_reach, ctx.tcx)
            }),
        )
    }
}

impl ImplSignatureCarrierReachSource<'_> {
    fn self_type_outward_reach(
        self,
        ctx: &ExposureContext<'_, '_>,
        containing_module: LocalDefId,
        self_type: &ResolvedImplSelfType,
        visited: &mut VisitedItems,
        facade_exposes: &mut FacadeExposure<'_>,
    ) -> Result<Option<VisibilityReach>> {
        let recursive_outward_reach = type_is_exposed_outside_parent(
            ctx,
            self_type.item_def_id,
            &self_type.definition_file,
            &self_type.name,
            visited,
            facade_exposes,
        )?;
        match self {
            Self::Module(signature_carrier_reach_source) => Ok(signature_carrier_reach_source
                .outward_reach(
                    ctx,
                    containing_module,
                    self_type.item_def_id,
                    recursive_outward_reach,
                )),
            Self::ParentBoundary { module_path } => {
                let mut outward_reach = recursive_outward_reach;
                if parent_boundary_item_is_used_outside_parent(ctx, module_path, &self_type.name)? {
                    accumulate_exposure_reach(
                        ctx,
                        self_type.item_def_id,
                        &mut outward_reach,
                        effective_path_reach(ctx, self_type.item_def_id),
                    );
                }
                Ok(outward_reach)
            },
        }
    }
}

pub(in crate::compiler) fn child_item_is_exposed_by_other_crate_visible_signature(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    crate_visible_signature_exposes_item(
        ctx,
        item_def_id,
        child_file,
        item_name,
        &mut VisitedItems::default(),
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
) -> Result<Option<VisibilityReach>> {
    if ctx.source_cache.name_mention(child_file, item_name) == NameMention::Absent {
        return Ok(None);
    }
    let item_module: LocalDefId = ctx.tcx.parent_module_from_def_id(item_def_id).into();

    let mut exposure_reach = None;
    for &module_scope in module_scopes(ctx, child_file).iter() {
        if module_scope.module != item_module {
            continue;
        }
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exposure_reach,
            module_signature_exposes_item(
                ctx,
                &module_scope,
                child_file,
                SignatureTarget {
                    def_id: item_def_id,
                    name:   item_name,
                },
                visited,
                facade_exposes,
                SignatureCarrierReachSource::DeclarationPath,
            )?,
        );
    }
    Ok(exposure_reach)
}

fn module_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    module_scope: &ModuleScope<'_>,
    source_file: &Path,
    target: SignatureTarget<'_>,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
    signature_carrier_reach_source: SignatureCarrierReachSource,
) -> Result<Option<VisibilityReach>> {
    let mut exposure_reach = None;

    for item in module_scope.items {
        let OutwardDeclarationClassification::Outward(exposing_declaration) =
            visitor::classify_outward_declaration(item)
        else {
            continue;
        };
        if exposing_declaration.name == target.name {
            continue;
        }
        let source_signature_carriers =
            visitor::potentially_outward_item_surface_carriers_mentioning_name(
                exposing_declaration.item,
                target.name,
            );
        if source_signature_carriers.is_empty() {
            continue;
        }
        let Some(exposing_item_def_id) = resolve_active_declaration(
            ctx,
            module_scope.module,
            source_file,
            &exposing_declaration,
        ) else {
            continue;
        };
        let signature_carriers = resolve_item_signature_carriers(
            ctx,
            source_file,
            source_signature_carriers,
            exposing_item_def_id,
        );
        let recursive_outward_reach = type_is_exposed_outside_parent(
            ctx,
            exposing_item_def_id,
            source_file,
            &exposing_declaration.name,
            visited,
            facade_exposes,
        )?;
        let Some(outward_reach) = signature_carrier_reach_source.outward_reach(
            ctx,
            module_scope.module,
            exposing_item_def_id,
            recursive_outward_reach,
        ) else {
            continue;
        };
        let Some(exposing_item_reach) =
            effective_declaration_reach(ctx, exposing_item_def_id, outward_reach)
        else {
            continue;
        };
        for signature_carrier in signature_carriers {
            let carrier_reach = signature_carrier.effective_outward_reach(
                ctx,
                module_scope.module,
                exposing_item_reach,
            );
            accumulate_exposure_reach(ctx, target.def_id, &mut exposure_reach, carrier_reach);
        }
    }

    accumulate_exposure_reach(
        ctx,
        target.def_id,
        &mut exposure_reach,
        module_impl_signature_exposes_item(
            ctx,
            module_scope,
            source_file,
            target,
            visited,
            facade_exposes,
            ImplSignatureCarrierReachSource::Module(signature_carrier_reach_source),
        )?,
    );

    Ok(exposure_reach)
}

fn module_impl_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    module_scope: &ModuleScope<'_>,
    source_file: &Path,
    target: SignatureTarget<'_>,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
    signature_carrier_reach_source: ImplSignatureCarrierReachSource<'_>,
) -> Result<Option<VisibilityReach>> {
    let mut exposure_reach = None;
    for item in module_scope.items {
        let Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(signature_surface) = resolve_impl_signature_surface(
            ctx,
            module_scope.module,
            source_file,
            item_impl,
            target.name,
        ) else {
            continue;
        };
        if signature_surface.self_type.item_def_id == target.def_id {
            continue;
        }
        let Some(self_type_outward_reach) = signature_carrier_reach_source
            .self_type_outward_reach(
                ctx,
                module_scope.module,
                &signature_surface.self_type,
                visited,
                facade_exposes,
            )?
        else {
            continue;
        };
        let Some(self_type_reach) = effective_declaration_reach(
            ctx,
            signature_surface.self_type.item_def_id,
            self_type_outward_reach,
        ) else {
            continue;
        };
        for signature_carrier in signature_surface.signature_carriers {
            let carrier_reach = signature_carrier.effective_outward_reach(
                ctx,
                module_scope.module,
                self_type_reach,
            );
            accumulate_exposure_reach(ctx, target.def_id, &mut exposure_reach, carrier_reach);
        }
    }
    Ok(exposure_reach)
}

pub(in crate::compiler) fn child_item_is_exposed_by_sibling_boundary_signature(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    sibling_boundary_signature_exposes_item(
        ctx,
        item_def_id,
        item_name,
        &mut VisitedItems::default(),
        facade_exposes,
    )
}

fn sibling_boundary_signature_exposes_item(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    let Some(parent_boundary) = facade::logical_parent_boundary_for_child(ctx.tcx, item_def_id)
    else {
        return Ok(None);
    };
    let child_module: LocalDefId = ctx.tcx.parent_module_from_def_id(item_def_id).into();
    let mut exposure_reach = None;

    let candidate_files = boundary_scopes(ctx, parent_boundary.module());
    sweep_counters::record_sweep(candidate_files.len());
    for candidate_file in candidate_files.iter() {
        if ctx
            .source_cache
            .name_mention(&candidate_file.source_file, item_name)
            == NameMention::Absent
        {
            continue;
        }
        sweep_counters::record_file_scanned(candidate_file.module_scopes.len());
        for module_scope in &candidate_file.module_scopes {
            if ctx
                .module_sources
                .module_is_within(ctx.tcx, module_scope.module, child_module)
            {
                continue;
            }
            sweep_counters::record_scope_analyzed();

            accumulate_exposure_reach(
                ctx,
                item_def_id,
                &mut exposure_reach,
                module_signature_exposes_item(
                    ctx,
                    module_scope,
                    &candidate_file.source_file,
                    SignatureTarget {
                        def_id: item_def_id,
                        name:   item_name,
                    },
                    visited,
                    facade_exposes,
                    SignatureCarrierReachSource::ContainingModulePath,
                )?,
            );
        }
    }

    Ok(exposure_reach)
}

/// Every module scope strictly inside `boundary`, grouped by file and collected
/// once per boundary.
///
/// [`sibling_boundary_signature_exposes_item`] runs once per analyzed item —
/// 209,564 times on `hana_diegetic` — and used to reach its candidates by walking
/// all 168 source files and every module scope in them, 35 million file visits
/// and 7.4 million scope visits that discarded roughly 85% of both. Which scopes
/// lie inside a boundary depends on the boundary alone, and `SourceCache` and
/// the module tree are immutable for the run, so one walk per boundary answers
/// every sweep that shares it. The two filters that do depend on the item —
/// [`SourceCache::name_mention`] and the exclusion of the child module's own
/// subtree — stay at the call site.
///
/// Files are walked in the same order as before, and files contributing no scope
/// are dropped, so the scopes reach [`accumulate_exposure_reach`] in the same
/// order.
fn boundary_scopes<'source>(
    ctx: &ExposureContext<'source, '_>,
    boundary: LocalDefId,
) -> Rc<[BoundaryFileScopes<'source>]> {
    if let Some(scopes) = ctx.boundary_scope_cache.get(boundary) {
        return scopes;
    }

    let mut scopes = Vec::new();
    for source_file in ctx.source_cache.source_files_under(ctx.source_root).iter() {
        let inside: Vec<ModuleScope<'source>> = module_scopes(ctx, source_file)
            .iter()
            .copied()
            .filter(|module_scope| {
                module_scope.module != boundary
                    && ctx
                        .module_sources
                        .module_is_within(ctx.tcx, module_scope.module, boundary)
            })
            .collect();
        if !inside.is_empty() {
            scopes.push(BoundaryFileScopes {
                source_file:   source_file.clone(),
                module_scopes: inside,
            });
        }
    }

    let scopes: Rc<[BoundaryFileScopes<'source>]> = scopes.into();
    ctx.boundary_scope_cache.insert(boundary, &scopes);
    scopes
}

pub(in crate::compiler) fn impl_item_is_exposed_by_exported_self_type(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    _: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    if !matches!(
        ctx.tcx.def_kind(item_def_id),
        DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy
    ) {
        return Ok(None);
    }
    let Some(impl_def_id) = ctx.tcx.parent(item_def_id.to_def_id()).as_local() else {
        return Ok(None);
    };
    let signature_carrier = ResolvedImplSignatureCarrier::new(ctx.tcx, item_def_id, impl_def_id);
    let Some(self_type_def_id) = ctx
        .tcx
        .type_of(signature_carrier.implementation)
        .instantiate_identity()
        .skip_normalization()
        .ty_adt_def()
        .map(ty::AdtDef::did)
        .and_then(DefId::as_local)
    else {
        return Ok(None);
    };
    let Some(definition_file) = ctx
        .module_sources
        .canonical_span_file(ctx.tcx, ctx.tcx.def_span(self_type_def_id))
    else {
        return Ok(None);
    };
    let self_type_name = ctx.tcx.item_name(self_type_def_id.to_def_id()).to_string();
    let Some(self_type_outward_reach) = type_is_exposed_outside_parent(
        ctx,
        self_type_def_id,
        &definition_file,
        &self_type_name,
        &mut VisitedItems::default(),
        facade_exposes,
    )?
    else {
        return Ok(None);
    };
    let Some(self_type_reach) =
        effective_declaration_reach(ctx, self_type_def_id, self_type_outward_reach)
    else {
        return Ok(None);
    };
    Ok(signature_carrier
        .effective_outward_reach(
            ctx,
            ctx.tcx.parent_module_from_def_id(item_def_id).into(),
            self_type_reach,
        )
        .map(|reach| visibility::anchored(reach, item_def_id, ctx.tcx)))
}

pub(in crate::compiler) fn parent_boundary_public_signature_exposes_child_used_outside_parent(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    parent_boundary_signature_exposes_child(
        ctx,
        item_def_id,
        item_name,
        &mut VisitedItems::default(),
        facade_exposes,
    )
}

fn parent_boundary_signature_exposes_child(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    let Some(parent_boundary) = facade::logical_parent_boundary_for_child(ctx.tcx, item_def_id)
    else {
        return Ok(None);
    };

    let mut exposing_items = Vec::new();
    let mut impl_exposure_reach = None;
    for boundary_file in ctx.module_sources.source_files(parent_boundary.module()) {
        if ctx.source_cache.name_mention(boundary_file, item_name) == NameMention::Absent {
            continue;
        }
        for &module_scope in module_scopes(ctx, boundary_file).iter() {
            if module_scope.module != parent_boundary.module() {
                continue;
            }
            accumulate_exposure_reach(
                ctx,
                item_def_id,
                &mut impl_exposure_reach,
                module_impl_signature_exposes_item(
                    ctx,
                    &module_scope,
                    boundary_file,
                    SignatureTarget {
                        def_id: item_def_id,
                        name:   item_name,
                    },
                    visited,
                    facade_exposes,
                    ImplSignatureCarrierReachSource::ParentBoundary {
                        module_path: parent_boundary.module_path(),
                    },
                )?,
            );
            for item in module_scope.items {
                let Some(exposer) = parent_boundary_exposer_for_item(
                    ctx,
                    parent_boundary.module(),
                    boundary_file,
                    item,
                    item_name,
                ) else {
                    continue;
                };
                if exposing_items
                    .iter()
                    .any(|existing: &ParentBoundaryExposer| {
                        existing.item_def_id == exposer.item_def_id
                    })
                {
                    continue;
                }
                exposing_items.push(exposer);
            }
        }
    }

    let mut exposure_reach = impl_exposure_reach;
    for exposer in exposing_items {
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exposure_reach,
            parent_boundary_exposer_reach(
                ctx,
                item_def_id,
                parent_boundary.module_path(),
                exposer,
                visited,
                facade_exposes,
            )?,
        );
    }

    Ok(exposure_reach)
}

/// The exposer `item` becomes when its signature mentions `item_name`.
///
/// `None` covers every way an item declines to expose: it is not an outward
/// declaration, its surface never names `item_name`, its source declaration has
/// no active definition in `module`, or none of the named carriers resolve.
fn parent_boundary_exposer_for_item(
    ctx: &ExposureContext<'_, '_>,
    module: LocalDefId,
    definition_file: &Path,
    item: &Item,
    item_name: &str,
) -> Option<ParentBoundaryExposer> {
    let OutwardDeclarationClassification::Outward(exposing_declaration) =
        visitor::classify_outward_declaration(item)
    else {
        return None;
    };
    let source_signature_carriers =
        visitor::potentially_outward_item_surface_carriers_mentioning_name(
            exposing_declaration.item,
            item_name,
        );
    if source_signature_carriers.is_empty() {
        return None;
    }
    let exposing_item_def_id =
        resolve_active_declaration(ctx, module, definition_file, &exposing_declaration)?;
    let signature_carriers = resolve_item_signature_carriers(
        ctx,
        definition_file,
        source_signature_carriers,
        exposing_item_def_id,
    );
    if signature_carriers.is_empty() {
        return None;
    }
    Some(ParentBoundaryExposer {
        name: exposing_declaration.name,
        item_def_id: exposing_item_def_id,
        definition_file: definition_file.to_path_buf(),
        signature_carriers,
    })
}

fn parent_boundary_exposer_reach(
    ctx: &ExposureContext<'_, '_>,
    target: LocalDefId,
    parent_module_path: &[String],
    exposer: ParentBoundaryExposer,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    let mut outward_reach = type_is_exposed_outside_parent(
        ctx,
        exposer.item_def_id,
        &exposer.definition_file,
        &exposer.name,
        visited,
        facade_exposes,
    )?;
    if parent_boundary_item_is_used_outside_parent(ctx, parent_module_path, &exposer.name)? {
        accumulate_exposure_reach(
            ctx,
            exposer.item_def_id,
            &mut outward_reach,
            effective_path_reach(ctx, exposer.item_def_id),
        );
    }
    let Some(outward_reach) = outward_reach else {
        return Ok(None);
    };
    let Some(exposing_item_reach) =
        effective_declaration_reach(ctx, exposer.item_def_id, outward_reach)
    else {
        return Ok(None);
    };
    let containing_module: LocalDefId = ctx
        .tcx
        .parent_module_from_def_id(exposer.item_def_id)
        .into();
    let mut exposure_reach = None;
    for signature_carrier in exposer.signature_carriers {
        let carrier_reach =
            signature_carrier.effective_outward_reach(ctx, containing_module, exposing_item_reach);
        accumulate_exposure_reach(ctx, target, &mut exposure_reach, carrier_reach);
    }
    Ok(exposure_reach)
}

fn type_is_exposed_outside_parent(
    ctx: &ExposureContext<'_, '_>,
    item_def_id: LocalDefId,
    child_file: &Path,
    item_name: &str,
    visited: &mut VisitedItems,
    facade_exposes: &mut FacadeExposure<'_>,
) -> Result<Option<VisibilityReach>> {
    match ctx.signature_exposure_cache.exterior_reach(item_def_id) {
        CachedExteriorReach::Absent => {},
        CachedExteriorReach::Unexposed => return Ok(None),
        CachedExteriorReach::Exposed(exterior_reach) => return Ok(Some(exterior_reach)),
    }
    if !visited.insert(item_def_id) {
        ctx.signature_exposure_cache.record_cycle_cut();
        return Ok(None);
    }
    let cycle_cuts_before = ctx.signature_exposure_cache.cycle_cuts();

    let result = (|| {
        let mut exterior_reach = None;
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exterior_reach,
            facade_exposes(item_def_id, child_file, item_name)?,
        );
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exterior_reach,
            crate_visible_signature_exposes_item(
                ctx,
                item_def_id,
                child_file,
                item_name,
                visited,
                facade_exposes,
            )?,
        );
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exterior_reach,
            sibling_boundary_signature_exposes_item(
                ctx,
                item_def_id,
                item_name,
                visited,
                facade_exposes,
            )?,
        );
        accumulate_exposure_reach(
            ctx,
            item_def_id,
            &mut exterior_reach,
            parent_boundary_signature_exposes_child(
                ctx,
                item_def_id,
                item_name,
                visited,
                facade_exposes,
            )?,
        );

        Ok(exterior_reach)
    })();
    visited.remove(&item_def_id);
    if let Ok(exterior_reach) = result.as_ref()
        && ctx.signature_exposure_cache.cycle_cuts() == cycle_cuts_before
    {
        ctx.signature_exposure_cache
            .store_exterior_reach(item_def_id, *exterior_reach);
    }
    result
}

fn accumulate_exposure_reach(
    ctx: &ExposureContext<'_, '_>,
    target: LocalDefId,
    current: &mut Option<VisibilityReach>,
    next: Option<VisibilityReach>,
) {
    let Some(next) = next else {
        return;
    };
    let next = visibility::anchored(next, target, ctx.tcx);
    *current = Some(current.map_or(next, |current| current.join(next, ctx.tcx)));
}

fn effective_declaration_reach(
    ctx: &ExposureContext<'_, '_>,
    declaration: LocalDefId,
    outward_reach: VisibilityReach,
) -> Option<VisibilityReach> {
    let declared_reach = VisibilityReach::from(ctx.tcx.visibility(declaration.to_def_id()));
    match declared_reach.compare(outward_reach, ctx.tcx) {
        Some(Ordering::Equal | Ordering::Less) => Some(declared_reach),
        Some(Ordering::Greater) => Some(outward_reach),
        None => None,
    }
}

impl ResolvedImplSignatureCarrier {
    fn new(tcx: TyCtxt<'_>, associated_item: LocalDefId, implementation: LocalDefId) -> Self {
        let contract = match tcx.def_kind(implementation.to_def_id()) {
            DefKind::Impl { of_trait: true } => ImplSignatureContract::Trait {
                trait_def_id: tcx
                    .impl_trait_ref(implementation)
                    .instantiate_identity()
                    .skip_normalization()
                    .def_id,
            },
            _ => ImplSignatureContract::Inherent,
        };
        Self {
            associated_item,
            implementation,
            contract,
        }
    }

    fn effective_outward_reach(
        &self,
        ctx: &ExposureContext<'_, '_>,
        containing_module: LocalDefId,
        self_type_reach: VisibilityReach,
    ) -> Option<VisibilityReach> {
        let associated_item_reach =
            effective_declaration_reach(ctx, self.associated_item, self_type_reach)?;
        let contract_reach = match self.contract {
            ImplSignatureContract::Inherent => associated_item_reach,
            ImplSignatureContract::Trait { trait_def_id } => {
                let trait_reach = effective_trait_reach(ctx, trait_def_id)?;
                cap_reach(ctx, associated_item_reach, trait_reach)?
            },
        };
        let private_reach =
            VisibilityReach::from(Visibility::Restricted(containing_module.to_def_id()));
        matches!(
            contract_reach.compare(private_reach, ctx.tcx),
            Some(Ordering::Greater)
        )
        .then_some(contract_reach)
    }
}

impl ResolvedFieldSignatureCarrier {
    fn effective_outward_reach(
        &self,
        ctx: &ExposureContext<'_, '_>,
        containing_module: LocalDefId,
        enclosing_type_reach: VisibilityReach,
    ) -> Option<VisibilityReach> {
        let field_reach = effective_declaration_reach(ctx, self.field, enclosing_type_reach)?;
        let private_reach =
            VisibilityReach::from(Visibility::Restricted(containing_module.to_def_id()));
        matches!(
            field_reach.compare(private_reach, ctx.tcx),
            Some(Ordering::Greater)
        )
        .then_some(field_reach)
    }
}

impl ResolvedItemSignatureCarrier {
    fn effective_outward_reach(
        &self,
        ctx: &ExposureContext<'_, '_>,
        containing_module: LocalDefId,
        surface_item_reach: VisibilityReach,
    ) -> Option<VisibilityReach> {
        match self {
            Self::Declaration { declaration } => {
                let declaration_reach =
                    effective_declaration_reach(ctx, *declaration, surface_item_reach)?;
                let private_reach =
                    VisibilityReach::from(Visibility::Restricted(containing_module.to_def_id()));
                matches!(
                    declaration_reach.compare(private_reach, ctx.tcx),
                    Some(Ordering::Greater)
                )
                .then_some(declaration_reach)
            },
            Self::Field(field_carrier) => {
                field_carrier.effective_outward_reach(ctx, containing_module, surface_item_reach)
            },
        }
    }
}

fn effective_trait_reach(
    ctx: &ExposureContext<'_, '_>,
    trait_def_id: DefId,
) -> Option<VisibilityReach> {
    let declared_reach = VisibilityReach::from(ctx.tcx.visibility(trait_def_id));
    let Some(local_trait) = trait_def_id.as_local() else {
        return Some(declared_reach);
    };
    let mut effective_reach = effective_path_reach(ctx, local_trait)?;
    let facade_subject = ctx.reexport_index.facade_subject(local_trait);
    for reach in ctx
        .reexport_index
        .applicable_reexport_reaches_outside_parent(ctx.tcx, local_trait, facade_subject)
    {
        effective_reach = effective_reach.join(reach, ctx.tcx);
    }
    Some(effective_reach)
}

fn cap_reach(
    ctx: &ExposureContext<'_, '_>,
    reach: VisibilityReach,
    cap: VisibilityReach,
) -> Option<VisibilityReach> {
    match reach.compare(cap, ctx.tcx) {
        Some(Ordering::Equal | Ordering::Less) => Some(reach),
        Some(Ordering::Greater) => Some(cap),
        None => None,
    }
}

fn effective_path_reach(
    ctx: &ExposureContext<'_, '_>,
    declaration: LocalDefId,
) -> Option<VisibilityReach> {
    let declared_reach = VisibilityReach::from(ctx.tcx.visibility(declaration.to_def_id()));
    let mut effective_reach =
        visibility::capped_by_enclosing_modules(declared_reach, declaration, ctx.tcx)?;
    for reach in ctx
        .reexport_index
        .applicable_exported_ancestor_path_reaches(ctx.tcx, declaration)
    {
        effective_reach = effective_reach.join(reach, ctx.tcx);
    }
    Some(effective_reach)
}

fn parent_boundary_item_is_used_outside_parent(
    ctx: &ExposureContext<'_, '_>,
    parent_module_path: &[String],
    exposing_item_name: &str,
) -> Result<bool> {
    let exposing_names = [exposing_item_name.to_string()];
    if facade::path_exists_outside_module(
        ctx.source_cache,
        ctx.source_root,
        ctx.tcx,
        ctx.module_sources,
        parent_module_path,
        &exposing_names,
    ) {
        return Ok(true);
    }

    Ok(facade::workspace_source_parent_export_literal_usage(
        ctx.source_cache,
        ctx.settings,
        ctx.tcx,
        ctx.module_sources,
        parent_module_path,
        &exposing_names,
    )?
    .values()
    .any(|usage| matches!(usage, ParentFacadeUsage::UsedOutsideSubtree)))
}

fn resolve_active_declaration(
    ctx: &ExposureContext<'_, '_>,
    module: LocalDefId,
    source_file: &Path,
    source_declaration: &OutwardDeclaration<'_>,
) -> Option<LocalDefId> {
    let Ok(source) = ctx.source_cache.read_source(source_file) else {
        return None;
    };
    let source_byte_position = source_byte_position(source, source_declaration.identifier_span)?;
    let canonical_source_file = ctx.module_sources.canonical_source_file(source_file);
    let (active_module, _, _) = ctx.tcx.hir_get_module(LocalModDefId::new_unchecked(module));
    for item_id in active_module.item_ids {
        let declaration_item = ctx.tcx.hir_item(*item_id);
        let local_def_id = declaration_item.owner_id.def_id;
        let declaration_module: LocalDefId = ctx.tcx.parent_module_from_def_id(local_def_id).into();
        let Some(declaration_name) = ctx.tcx.opt_item_name(local_def_id.to_def_id()) else {
            continue;
        };
        if declaration_module != module
            || declaration_name.as_str() != source_declaration.name
            || !declaration_kind_is_compatible(
                source_declaration.kind,
                ctx.tcx.def_kind(local_def_id.to_def_id()),
            )
        {
            continue;
        }
        let Some(declaration_span) = ctx.tcx.def_ident_span(local_def_id.to_def_id()) else {
            continue;
        };
        if !ctx.module_sources.span_is_in_file(
            ctx.tcx,
            declaration_span,
            canonical_source_file.as_ref(),
        ) || rustc_source_byte_position(ctx.tcx, declaration_span) != source_byte_position
        {
            continue;
        }
        return Some(local_def_id);
    }
    None
}

const fn declaration_kind_is_compatible(
    source_kind: OutwardDeclarationKind,
    def_kind: DefKind,
) -> bool {
    matches!(
        (source_kind, def_kind),
        (
            visitor::OutwardDeclarationKind::Const,
            DefKind::Const { .. }
        ) | (visitor::OutwardDeclarationKind::Enum, DefKind::Enum)
            | (visitor::OutwardDeclarationKind::Function, DefKind::Fn)
            | (
                visitor::OutwardDeclarationKind::Static,
                DefKind::Static { .. }
            )
            | (visitor::OutwardDeclarationKind::Struct, DefKind::Struct)
            | (visitor::OutwardDeclarationKind::Trait, DefKind::Trait)
            | (visitor::OutwardDeclarationKind::TypeAlias, DefKind::TyAlias)
            | (visitor::OutwardDeclarationKind::Union, DefKind::Union)
    )
}

fn resolve_impl_signature_carrier(
    ctx: &ExposureContext<'_, '_>,
    module: LocalDefId,
    source_file: &Path,
    source_item: &ImplItem,
) -> Option<ResolvedImplSignatureCarrier> {
    let (source_name, source_span) = match source_item {
        ImplItem::Const(item) => (item.ident.to_string(), item.ident.span()),
        ImplItem::Fn(item) => (item.sig.ident.to_string(), item.sig.ident.span()),
        ImplItem::Type(item) => (item.ident.to_string(), item.ident.span()),
        _ => return None,
    };
    let source_name = source_name.strip_prefix("r#").unwrap_or(&source_name);
    let Ok(source) = ctx.source_cache.read_source(source_file) else {
        return None;
    };
    let source_byte_position = source_byte_position(source, source_span)?;
    let canonical_source_file = ctx.module_sources.canonical_source_file(source_file);
    for item_id in ctx.tcx.hir_crate_items(()).impl_items() {
        let item = ctx.tcx.hir_impl_item(item_id);
        let item_module: LocalDefId = ctx
            .tcx
            .parent_module_from_def_id(item.owner_id.def_id)
            .into();
        if item_module != module
            || item.ident.name.as_str() != source_name
            || rustc_source_byte_position(ctx.tcx, item.ident.span) != source_byte_position
        {
            continue;
        }
        // Resolving a span's file is the widest test, so it is applied last:
        // only a candidate already matching on module, name, and byte offset
        // reaches it.
        if !ctx
            .module_sources
            .span_is_in_file(ctx.tcx, item.span, canonical_source_file.as_ref())
        {
            continue;
        }
        let associated_item = item.owner_id.def_id;
        let implementation = ctx.tcx.parent(associated_item.to_def_id()).as_local()?;
        return Some(ResolvedImplSignatureCarrier::new(
            ctx.tcx,
            associated_item,
            implementation,
        ));
    }
    None
}

fn resolve_impl_signature_surface(
    ctx: &ExposureContext<'_, '_>,
    module: LocalDefId,
    source_file: &Path,
    source_impl: &ItemImpl,
    item_name: &str,
) -> Option<ResolvedImplSignatureSurface> {
    let mut resolved_source_carriers =
        visitor::potentially_outward_impl_surface_items_mentioning_name(source_impl, item_name)
            .into_iter()
            .filter_map(|source_item| {
                resolve_impl_signature_carrier(ctx, module, source_file, source_item)
            });
    let first_signature_carrier = resolved_source_carriers.next()?;
    let implementation = first_signature_carrier.implementation;
    let mut signature_carriers = vec![first_signature_carrier];
    for signature_carrier in resolved_source_carriers {
        if signature_carrier.implementation != implementation {
            return None;
        }
        signature_carriers.push(signature_carrier);
    }
    let self_type_def_id = ctx
        .tcx
        .type_of(implementation)
        .instantiate_identity()
        .skip_normalization()
        .ty_adt_def()
        .map(ty::AdtDef::did)
        .and_then(DefId::as_local)?;
    let definition_file = ctx
        .module_sources
        .canonical_span_file(ctx.tcx, ctx.tcx.def_span(self_type_def_id))?;
    Some(ResolvedImplSignatureSurface {
        self_type: ResolvedImplSelfType {
            item_def_id: self_type_def_id,
            name: ctx.tcx.item_name(self_type_def_id.to_def_id()).to_string(),
            definition_file,
        },
        signature_carriers,
    })
}

fn resolve_item_signature_carriers(
    ctx: &ExposureContext<'_, '_>,
    source_file: &Path,
    source_carriers: Vec<ItemSignatureCarrier<'_>>,
    surface_item: LocalDefId,
) -> Vec<ResolvedItemSignatureCarrier> {
    source_carriers
        .into_iter()
        .filter_map(|source_carrier| match source_carrier {
            ItemSignatureCarrier::Declaration => Some(ResolvedItemSignatureCarrier::Declaration {
                declaration: surface_item,
            }),
            ItemSignatureCarrier::StructOrUnionField { field, field_index } => {
                let field_carrier = resolve_field_signature_carrier(
                    ctx,
                    source_file,
                    field,
                    field_index,
                    surface_item,
                )?;
                Some(ResolvedItemSignatureCarrier::Field(field_carrier))
            },
        })
        .collect()
}

fn resolve_field_signature_carrier(
    ctx: &ExposureContext<'_, '_>,
    source_file: &Path,
    source_field: &Field,
    field_index: usize,
    containing_type: LocalDefId,
) -> Option<ResolvedFieldSignatureCarrier> {
    let Ok(source) = ctx.source_cache.read_source(source_file) else {
        return None;
    };
    let source_identity = source_field_identity(source, source_field, field_index)?;
    let item = ctx.tcx.hir_expect_item(containing_type);
    let (ItemKind::Struct(_, _, variant_data) | ItemKind::Union(_, _, variant_data)) = &item.kind
    else {
        return None;
    };
    let canonical_source_file = ctx.module_sources.canonical_source_file(source_file);
    for field in variant_data.fields() {
        if !ctx
            .module_sources
            .span_is_in_file(ctx.tcx, field.span, canonical_source_file.as_ref())
            || !source_identity.matches_hir_field(ctx.tcx, field)
        {
            continue;
        }
        return Some(ResolvedFieldSignatureCarrier {
            field: field.def_id,
        });
    }
    None
}

fn source_field_identity(
    source: &str,
    field: &Field,
    field_index: usize,
) -> Option<SourceFieldIdentity> {
    if let Some(identifier) = &field.ident {
        let byte_position = source_byte_position(source, identifier.span())?;
        let source_name = identifier.to_string();
        return Some(SourceFieldIdentity::Named {
            name: source_name
                .strip_prefix("r#")
                .unwrap_or(&source_name)
                .to_string(),
            byte_position,
        });
    }
    let field_byte_position = source_byte_position(source, field.span())?;
    let type_byte_position = source_byte_position(source, field.ty.span())?;
    Some(SourceFieldIdentity::Unnamed {
        field_index,
        field_byte_position,
        type_byte_position,
    })
}

impl SourceFieldIdentity {
    fn matches_hir_field(&self, tcx: TyCtxt<'_>, field: &FieldDef<'_>) -> bool {
        match self {
            Self::Named {
                name,
                byte_position,
            } => {
                field.ident.name.as_str() == name
                    && rustc_source_byte_position(tcx, field.ident.span) == *byte_position
            },
            Self::Unnamed {
                field_index,
                field_byte_position,
                type_byte_position,
            } => {
                let hir_field_position = rustc_source_byte_position(tcx, field.span);
                let hir_definition_position =
                    rustc_source_byte_position(tcx, tcx.def_span(field.def_id));
                field
                    .ident
                    .name
                    .as_str()
                    .parse::<usize>()
                    .is_ok_and(|index| index == *field_index)
                    || hir_field_position == *field_byte_position
                    || hir_field_position == *type_byte_position
                    || hir_definition_position == *field_byte_position
                    || hir_definition_position == *type_byte_position
            },
        }
    }
}

fn rustc_source_byte_position(tcx: TyCtxt<'_>, span: Span) -> usize {
    let source_position = tcx.sess.source_map().lookup_byte_offset(span.lo());
    source_position.sf.original_relative_byte_pos(span.lo()).0 as usize
}

fn source_byte_position(source: &str, source_span: proc_macro2::Span) -> Option<usize> {
    let byte_position = source_span.byte_range().start;
    (byte_position <= source.len() && source.is_char_boundary(byte_position))
        .then_some(byte_position)
}

/// The module scopes of `source_file`, built once per file and reused.
///
/// Building them walks the file's whole item tree, and every analyzed item asks
/// for the same files again — a whole-crate sweep runs once per item, so the
/// repeat rate is the item count. The parsed cache is immutable for the run, so
/// the file path alone identifies the result.
///
/// Callers get an [`Rc`] rather than a borrow of the cache: the loop bodies that
/// walk these scopes call back into analysis, which reaches this function again
/// and would hit an outstanding [`RefCell`] borrow.
fn module_scopes<'source>(
    ctx: &ExposureContext<'source, '_>,
    source_file: &Path,
) -> Rc<[ModuleScope<'source>]> {
    if let Some(scopes) = ctx.module_scope_cache.get(source_file) {
        return scopes;
    }

    let mut scopes = Vec::new();
    if let Some(file) = ctx.source_cache.parsed_file(source_file) {
        for root_module in ctx
            .module_sources
            .root_modules_for_file(ctx.tcx, source_file)
        {
            collect_module_scopes(ctx.tcx, root_module, &file.items, &mut scopes);
        }
    }

    let scopes: Rc<[ModuleScope<'source>]> = scopes.into();
    ctx.module_scope_cache.insert(source_file, &scopes);
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
