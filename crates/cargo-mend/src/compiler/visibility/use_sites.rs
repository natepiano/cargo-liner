//! HIR-level use-site collector.
//!
//! Walks every body in the local crate and emits one `UseSite` per
//! resolved expression-level path reference. The output is persisted with
//! the per-compilation findings so that, after every cargo target
//! compilation has run, `load_report` can compute the union of callers
//! for each item and suppress narrowing-style findings whose proposed
//! tighter visibility would block any actual caller.
//!
//! This catches every reference rustc itself sees, including paths inside
//! macro invocations and paths produced by proc-macro expansion — both
//! of which the source-level scanner cannot.

use std::cell::OnceCell;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::HashSet;
use std::rc::Rc;

use rustc_hir::AmbigArg;
use rustc_hir::Expr;
use rustc_hir::ExprKind;
use rustc_hir::HirId;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::Pat;
use rustc_hir::PatExprKind;
use rustc_hir::PatKind;
use rustc_hir::Path;
use rustc_hir::QPath;
use rustc_hir::TraitItem;
use rustc_hir::Ty;
use rustc_hir::TyKind;
use rustc_hir::UseKind;
use rustc_hir::def::CtorOf;
use rustc_hir::def::DefKind;
use rustc_hir::def::Res;
use rustc_hir::def_id::CRATE_DEF_ID;
use rustc_hir::def_id::CrateNum;
use rustc_hir::def_id::DefId;
use rustc_hir::def_id::LocalDefId;
use rustc_hir::intravisit::Visitor;
use rustc_hir::intravisit::walk_expr;
use rustc_hir::intravisit::walk_impl_item;
use rustc_hir::intravisit::walk_item;
use rustc_hir::intravisit::walk_trait_item;
use rustc_middle::hir::nested_filter::All;
use rustc_middle::ty;
use rustc_middle::ty::AssocContainer;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::Span;

use super::annotation;
use super::annotation::VisibilityReach;
use super::annotation::VisibilitySyntax;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentFacadeUsageByName;
use crate::compiler::persistence::UseSite;
use crate::rust_syntax::PathAnchor;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FacadeUseKind {
    Named,
    Glob,
    ExternCrate,
}

#[derive(Clone, Copy)]
struct FacadeVisibilityDecision {
    is_reexport:       bool,
    spelling:          ParentFacadeSpelling,
    spelling_conflict: bool,
}

impl FacadeVisibilityDecision {
    const fn reexport(spelling: ParentFacadeSpelling) -> Self {
        Self {
            is_reexport: true,
            spelling,
            spelling_conflict: false,
        }
    }

    const fn reexport_with_unknown_spelling() -> Self {
        Self {
            is_reexport:       true,
            spelling:          ParentFacadeSpelling::Other,
            spelling_conflict: true,
        }
    }

    const fn private() -> Self {
        Self {
            is_reexport:       false,
            spelling:          ParentFacadeSpelling::Other,
            spelling_conflict: false,
        }
    }
}

#[derive(Clone)]
pub(super) struct ReexportOccurrence {
    pub(super) use_def_id:        LocalDefId,
    pub(super) owner_module:      LocalDefId,
    pub(super) visibility:        Visibility<DefId>,
    pub(super) facade_spelling:   ParentFacadeSpelling,
    pub(super) spelling_conflict: bool,
    pub(super) use_kind:          FacadeUseKind,
    pub(super) alias:             Option<String>,
    pub(super) export_names:      Vec<String>,
    pub(super) span:              Span,
    pub(super) usage_by_name:     Rc<OnceCell<ParentFacadeUsageByName>>,
}

#[derive(Default)]
pub(super) struct ReexportIndex {
    named:               HashMap<DefId, Vec<ReexportOccurrence>>,
    globs:               HashMap<DefId, Vec<ReexportOccurrence>>,
    direct_use_subjects: HashMap<LocalDefId, DefId>,
    facade_subjects:     HashMap<LocalDefId, LocalDefId>,
    extern_crates:       HashMap<(LocalDefId, String), LocalDefId>,
}

#[derive(Clone)]
pub(super) struct ParentFacadeOccurrences<'index> {
    pub(super) selected:          &'index ReexportOccurrence,
    pub(super) matching:          Vec<&'index ReexportOccurrence>,
    pub(super) spelling_conflict: bool,
}

/// The resolved visibility required by every named facade boundary between an
/// item and its outermost matching re-export.
#[derive(Clone, Copy)]
pub(super) enum FacadeChainResolution<'index> {
    Resolved { required: VisibilityReach },
    Unresolvable { blocker: FacadeChainBlocker<'index> },
}

/// A facade boundary that prevents the chain from supplying a declaration
/// visibility requirement.
#[derive(Clone, Copy)]
pub(super) enum FacadeChainBlocker<'index> {
    Glob(&'index ReexportOccurrence),
    ForeignBoundary(&'index ReexportOccurrence),
}

impl FacadeChainBlocker<'_> {
    pub(super) const fn occurrence(&self) -> &ReexportOccurrence {
        match self {
            Self::Glob(occurrence) | Self::ForeignBoundary(occurrence) => occurrence,
        }
    }
}

/// Nearest-facade metadata and the independently computed full-chain reach.
#[derive(Clone)]
pub(super) struct ParentFacadeAnalysis<'index> {
    pub(super) nearest: ParentFacadeOccurrences<'index>,
    pub(super) chain:   FacadeChainResolution<'index>,
}

impl ReexportIndex {
    pub(super) fn facade_subject(&self, item_def_id: LocalDefId) -> LocalDefId {
        self.facade_subjects
            .get(&item_def_id)
            .copied()
            .unwrap_or(item_def_id)
    }

    #[cfg(test)]
    pub(super) fn parent_facade_occurrence(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> Option<&ReexportOccurrence> {
        self.parent_facade_analysis(tcx, item_def_id, facade_subject)
            .map(|analysis| analysis.nearest.selected)
    }

    pub(super) fn parent_facade_analysis(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> Option<ParentFacadeAnalysis<'_>> {
        let mut child_module: LocalDefId = tcx.parent_module_from_def_id(facade_subject).into();
        let subject = self
            .direct_use_subjects
            .get(&facade_subject)
            .copied()
            .unwrap_or_else(|| facade_subject.to_def_id());
        if !subject.is_local() {
            let occurrence = self
                .named
                .get(&subject)
                .into_iter()
                .flatten()
                .find(|occurrence| occurrence.use_def_id == facade_subject)?;
            return Some(ParentFacadeAnalysis {
                nearest: ParentFacadeOccurrences {
                    selected:          occurrence,
                    matching:          vec![occurrence],
                    spelling_conflict: occurrence.spelling_conflict,
                },
                chain:   FacadeChainResolution::Unresolvable {
                    blocker: FacadeChainBlocker::ForeignBoundary(occurrence),
                },
            });
        }
        if child_module == CRATE_DEF_ID {
            return None;
        }

        let mut nearest = None;
        let mut required: Option<VisibilityReach> = None;
        loop {
            let parent_module: LocalDefId = tcx.parent_module_from_def_id(child_module).into();
            let named_occurrences =
                self.matching_named_occurrences(tcx, item_def_id, subject, parent_module);
            if let Some(selected) = Self::widest_applicable_occurrence(
                tcx,
                item_def_id,
                subject,
                named_occurrences.iter().copied(),
            ) {
                let occurrences = ParentFacadeOccurrences {
                    selected,
                    spelling_conflict: Self::spelling_conflict(selected, &named_occurrences, tcx),
                    matching: named_occurrences,
                };
                let boundary_reach = Self::joined_occurrence_reach(tcx, &occurrences.matching)?;
                if nearest.is_none() {
                    nearest = Some(occurrences);
                }
                required = Some(
                    required.map_or(boundary_reach, |current| current.join(boundary_reach, tcx)),
                );
            } else {
                let glob_occurrences =
                    self.matching_glob_occurrences(tcx, subject, child_module, parent_module);
                if let Some(blocking_glob) = Self::widest_applicable_occurrence(
                    tcx,
                    item_def_id,
                    subject,
                    glob_occurrences.iter().copied(),
                ) {
                    let nearest = nearest.unwrap_or_else(|| ParentFacadeOccurrences {
                        selected:          blocking_glob,
                        spelling_conflict: Self::spelling_conflict(
                            blocking_glob,
                            &glob_occurrences,
                            tcx,
                        ),
                        matching:          glob_occurrences,
                    });
                    return Some(ParentFacadeAnalysis {
                        nearest,
                        chain: FacadeChainResolution::Unresolvable {
                            blocker: FacadeChainBlocker::Glob(blocking_glob),
                        },
                    });
                }
            }
            if parent_module == CRATE_DEF_ID {
                let required = required?;
                return nearest.map(|nearest| ParentFacadeAnalysis {
                    nearest,
                    chain: FacadeChainResolution::Resolved {
                        required: annotation::anchored(required, item_def_id, tcx),
                    },
                });
            }
            child_module = parent_module;
        }
    }

    pub(super) fn has_public_reexport(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> bool {
        self.public_reexport_occurrences(tcx, item_def_id, facade_subject)
            .next()
            .is_some()
    }

    pub(super) fn has_public_reexport_outside_parent(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> bool {
        let parent_module: LocalDefId = tcx.parent_module_from_def_id(item_def_id).into();
        self.public_reexport_occurrences(tcx, item_def_id, facade_subject)
            .any(|occurrence| !Self::is_module_within(tcx, occurrence.owner_module, parent_module))
    }

    fn public_reexport_occurrences<'a>(
        &'a self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> impl Iterator<Item = &'a ReexportOccurrence> {
        let subject = facade_subject.to_def_id();
        self.named
            .get(&subject)
            .into_iter()
            .flatten()
            .chain(
                Self::glob_containers(
                    tcx,
                    tcx.parent_module_from_def_id(facade_subject).into(),
                    subject,
                )
                .filter_map(|container| self.globs.get(&container))
                .flatten(),
            )
            .filter(move |occurrence| {
                occurrence.visibility.is_public()
                    && Self::occurrence_applies_to_item(tcx, item_def_id, subject, occurrence)
            })
    }

    fn widest_applicable_occurrence<'a>(
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        subject: DefId,
        occurrences: impl Iterator<Item = &'a ReexportOccurrence>,
    ) -> Option<&'a ReexportOccurrence> {
        occurrences
            .filter(|occurrence| {
                Self::occurrence_applies_to_item(tcx, item_def_id, subject, occurrence)
            })
            .reduce(|widest, occurrence| {
                match VisibilityReach::from(occurrence.visibility)
                    .compare(VisibilityReach::from(widest.visibility), tcx)
                {
                    Some(Ordering::Greater) => occurrence,
                    Some(Ordering::Less) => widest,
                    Some(Ordering::Equal) | None => {
                        Self::preferred_equal_reach_occurrence(widest, occurrence)
                    },
                }
            })
    }

    fn preferred_equal_reach_occurrence<'a>(
        left: &'a ReexportOccurrence,
        right: &'a ReexportOccurrence,
    ) -> &'a ReexportOccurrence {
        let left_priority = FacadeSpellingPriority::from(left.facade_spelling);
        let right_priority = FacadeSpellingPriority::from(right.facade_spelling);
        if left_priority > right_priority {
            return left;
        }
        if right_priority > left_priority {
            return right;
        }
        if left.alias.as_deref() <= right.alias.as_deref() {
            left
        } else {
            right
        }
    }

    fn spelling_conflict(
        selected: &ReexportOccurrence,
        occurrences: &[&ReexportOccurrence],
        tcx: TyCtxt<'_>,
    ) -> bool {
        occurrences.iter().any(|occurrence| {
            VisibilityReach::from(occurrence.visibility)
                .compare(VisibilityReach::from(selected.visibility), tcx)
                == Some(Ordering::Equal)
                && (occurrence.spelling_conflict
                    || occurrence.facade_spelling != selected.facade_spelling)
        })
    }

    fn joined_occurrence_reach(
        tcx: TyCtxt<'_>,
        occurrences: &[&ReexportOccurrence],
    ) -> Option<VisibilityReach> {
        let (first, remaining) = occurrences.split_first()?;
        Some(remaining.iter().fold(
            VisibilityReach::from(first.visibility),
            |current, occurrence| current.join(VisibilityReach::from(occurrence.visibility), tcx),
        ))
    }

    fn matching_named_occurrences<'a>(
        &'a self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        subject: DefId,
        parent_module: LocalDefId,
    ) -> Vec<&'a ReexportOccurrence> {
        Self::distinct_use_occurrences(
            self.named
                .get(&subject)
                .into_iter()
                .flatten()
                .filter(|occurrence| {
                    occurrence.owner_module == parent_module
                        && Self::occurrence_applies_to_item(tcx, item_def_id, subject, occurrence)
                })
                .collect(),
        )
    }

    fn matching_glob_occurrences<'a>(
        &'a self,
        tcx: TyCtxt<'_>,
        subject: DefId,
        child_module: LocalDefId,
        parent_module: LocalDefId,
    ) -> Vec<&'a ReexportOccurrence> {
        Self::distinct_use_occurrences(
            Self::glob_containers(tcx, child_module, subject)
                .filter_map(|container| self.globs.get(&container))
                .flatten()
                .filter(|occurrence| occurrence.owner_module == parent_module)
                .collect(),
        )
    }

    fn occurrence_applies_to_item(
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        subject: DefId,
        occurrence: &ReexportOccurrence,
    ) -> bool {
        if item_def_id.to_def_id() == subject {
            return true;
        }
        let item_reach: VisibilityReach = tcx.visibility(item_def_id.to_def_id()).into();
        let facade_reach: VisibilityReach = tcx
            .local_visibility(occurrence.use_def_id)
            .map_id(LocalDefId::to_def_id)
            .into();
        item_reach.is_at_least(facade_reach, tcx)
    }

    fn distinct_use_occurrences(occurrences: Vec<&ReexportOccurrence>) -> Vec<&ReexportOccurrence> {
        let mut use_def_ids = HashSet::new();
        occurrences
            .into_iter()
            .filter(|occurrence| use_def_ids.insert(occurrence.use_def_id))
            .collect()
    }

    fn glob_containers(
        tcx: TyCtxt<'_>,
        child_module: LocalDefId,
        subject: DefId,
    ) -> impl Iterator<Item = DefId> {
        let mut containers = Vec::new();
        if let Some(local_subject) = subject.as_local() {
            let subject_module: LocalDefId = tcx.parent_module_from_def_id(local_subject).into();
            if Self::is_module_within(tcx, subject_module, child_module) {
                let mut module = subject_module;
                loop {
                    containers.push(module.to_def_id());
                    if module == child_module {
                        break;
                    }
                    module = tcx.parent_module_from_def_id(module).into();
                }
            } else {
                containers.push(child_module.to_def_id());
            }
        } else {
            containers.push(child_module.to_def_id());
        }
        if matches!(tcx.def_kind(subject), DefKind::Enum) {
            containers.push(subject);
        }
        containers.into_iter()
    }

    fn is_module_within(tcx: TyCtxt<'_>, mut module: LocalDefId, ancestor: LocalDefId) -> bool {
        loop {
            if module == ancestor {
                return true;
            }
            if module == CRATE_DEF_ID {
                return false;
            }
            module = tcx.parent_module_from_def_id(module).into();
        }
    }

    fn insert_named(&mut self, subject: DefId, occurrence: ReexportOccurrence) {
        self.named.entry(subject).or_default().push(occurrence);
    }

    fn insert_glob(&mut self, container: DefId, occurrence: ReexportOccurrence) {
        self.globs.entry(container).or_default().push(occurrence);
    }

    fn insert_extern_crate(&mut self, tcx: TyCtxt<'_>, item: &Item<'_>) {
        let ItemKind::ExternCrate(_, ident) = item.kind else {
            return;
        };
        let owner_module: LocalDefId = tcx.parent_module_from_def_id(item.owner_id.def_id).into();
        self.extern_crates
            .insert((owner_module, ident.name.to_string()), item.owner_id.def_id);
        if let Some(subject) = foreign_extern_crate_subject(tcx, item) {
            self.direct_use_subjects
                .entry(item.owner_id.def_id)
                .or_insert(subject);
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum FacadeSpellingPriority {
    Other,
    Public,
    Crate,
    Super,
}

impl From<ParentFacadeSpelling> for FacadeSpellingPriority {
    fn from(spelling: ParentFacadeSpelling) -> Self {
        match spelling {
            ParentFacadeSpelling::Other => Self::Other,
            ParentFacadeSpelling::Public => Self::Public,
            ParentFacadeSpelling::Crate => Self::Crate,
            ParentFacadeSpelling::Super => Self::Super,
        }
    }
}

struct SubjectNormalizer<'tcx> {
    tcx:                 TyCtxt<'tcx>,
    inherent_self_types: HashMap<DefId, DefId>,
}

impl SubjectNormalizer<'_> {
    fn normalized_subject(&mut self, target: DefId) -> DefId {
        match self.tcx.def_kind(target) {
            DefKind::Variant | DefKind::Ctor(CtorOf::Struct, _) => self.tcx.parent(target),
            DefKind::Ctor(CtorOf::Variant, _) => self.tcx.parent(self.tcx.parent(target)),
            DefKind::AssocFn | DefKind::AssocConst { .. } | DefKind::AssocTy
                if matches!(
                    self.tcx.associated_item(target).container,
                    AssocContainer::InherentImpl
                ) =>
            {
                self.inherent_self_type(target).unwrap_or(target)
            },
            _ => target,
        }
    }

    fn inherent_self_type(&mut self, item_def_id: DefId) -> Option<DefId> {
        let impl_def_id = self.tcx.parent(item_def_id);
        if let Some(subject) = self.inherent_self_types.get(&impl_def_id) {
            return Some(*subject);
        }
        let subject = self
            .tcx
            .type_of(impl_def_id)
            .instantiate_identity()
            .skip_normalization()
            .ty_adt_def()
            .map(ty::AdtDef::did)?;
        self.inherent_self_types.insert(impl_def_id, subject);
        Some(subject)
    }
}

struct UseSiteCollector<'a, 'tcx> {
    tcx:                       TyCtxt<'tcx>,
    /// Def-id of the nearest enclosing module. Updated as the visitor
    /// descends into `mod` items so each call site is tagged with the
    /// module path it lives in (not the function or impl that contains
    /// it).
    current_module:            DefId,
    out:                       &'a mut Vec<UseSite>,
    public_visibility_targets: &'a mut HashSet<LocalDefId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InterfaceVisibility {
    Public,
    Restricted,
}

struct InterfaceReach {
    module:     DefId,
    visibility: InterfaceVisibility,
}

impl<'tcx> UseSiteCollector<'_, 'tcx> {
    fn record_target(&mut self, target: DefId) {
        // Skip references to items in other crates — narrowing decisions
        // only apply to local items.
        if target.is_local() {
            self.push_site(target);
        }
        match self.tcx.def_kind(target) {
            // A reference to a type alias also reaches every type the alias
            // names: `type M = Wrapper<Inner>` exposes `Inner` wherever `M`
            // is used, even though `Inner` never appears at the use site.
            // Record those component types under the same caller module so
            // narrowing findings see the reach that flows through the alias.
            // Foreign aliases can still name a local type, so this runs
            // regardless of where the alias itself lives.
            DefKind::TyAlias => self.record_alias_components(target),
            // Calling a function reaches every local type named in its
            // signature: `fn f() -> Guard` exposes `Guard` at the call site
            // even though `Guard` never appears there. Record those signature
            // types under the same caller module so narrowing findings see
            // the reach that flows through the call. Without this, removing
            // `pub` from a type returned by (or passed to) a `pub(crate)` fn
            // leaves a private type in that fn's signature (E0446) and rolls
            // `--fix` back.
            DefKind::Fn | DefKind::AssocFn => self.record_fn_signature_components(target),
            _ => {},
        }
    }

    /// Record every local type named in a function's signature as used from
    /// the current caller module, following each type's public field graph the
    /// same way alias components are. `fn_sig` yields the declared input and
    /// output types; a module-private field still caps reach, so genuinely
    /// internal types stay flagged.
    fn record_fn_signature_components(&mut self, func: DefId) {
        let signature = self.tcx.fn_sig(func).instantiate_identity();
        let mut seen = HashSet::new();
        for input_or_output in signature.skip_binder().inputs_and_output {
            for arg in input_or_output.walk() {
                if let Some(component) = arg.as_type()
                    && let ty::TyKind::Adt(adt_def, _) = component.kind()
                {
                    self.record_exposed_adt(adt_def.did(), &mut seen);
                }
            }
        }
    }

    /// Record every local type named in an alias's right-hand side as used
    /// from the current caller module. `type_of` returns the aliased type
    /// with nested eager aliases already expanded, so walking it yields the
    /// concrete types the alias exposes.
    fn record_alias_components(&mut self, alias: DefId) {
        let aliased = self
            .tcx
            .type_of(alias)
            .instantiate_identity()
            .skip_normalization();
        let mut seen = HashSet::new();
        for arg in aliased.walk() {
            if let Some(component) = arg.as_type()
                && let ty::TyKind::Adt(adt_def, _) = component.kind()
            {
                self.record_exposed_adt(adt_def.did(), &mut seen);
            }
        }
    }

    /// Record a local type as used from the current caller module, then walk
    /// its public field graph: a `pub` field of an alias-exposed type makes
    /// the field's type reachable wherever the alias is used, so those types
    /// must keep matching visibility too. Fields that do not escape the
    /// type's own module are not followed — they expose nothing further.
    fn record_exposed_adt(&mut self, did: DefId, seen: &mut HashSet<DefId>) {
        let Some(local) = did.as_local() else {
            return;
        };
        if !seen.insert(did) {
            return;
        }
        self.push_site(did);
        let owning_module = self.tcx.parent_module_from_def_id(local).to_def_id();
        for field in self.tcx.adt_def(did).all_fields() {
            if !self.field_escapes_module(field.did, owning_module) {
                continue;
            }
            for arg in self
                .tcx
                .type_of(field.did)
                .instantiate_identity()
                .skip_normalization()
                .walk()
            {
                if let Some(component) = arg.as_type()
                    && let ty::TyKind::Adt(adt_def, _) = component.kind()
                {
                    self.record_exposed_adt(adt_def.did(), seen);
                }
            }
        }
    }

    /// Record every local ADT named in a trait impl's interface — trait-ref
    /// type arguments, associated type bindings, associated const types, and
    /// associated fn signatures — as used from the widest module the
    /// interface reaches. HIR holds post-expansion items, so this covers
    /// interface mentions that exist in no source file: `#[derive(AsBindGroup)]`
    /// on a `pub(crate)` type generates `type Data = TextExtensionKey;`,
    /// which requires `TextExtensionKey` to stay at least `pub(crate)`
    /// (E0446). Without these sites, `unused_pub` suggests removing `pub`
    /// and the `--fix` validation fails and rolls back.
    fn record_trait_impl_interface(&mut self, impl_def: LocalDefId) {
        if !matches!(
            self.tcx.def_kind(impl_def.to_def_id()),
            DefKind::Impl { of_trait: true }
        ) {
            return;
        }
        let trait_ref = self
            .tcx
            .impl_trait_ref(impl_def)
            .instantiate_identity()
            .skip_normalization();
        let self_adt = trait_ref.self_ty().ty_adt_def().map(ty::AdtDef::did);

        let previous_module = self.current_module;
        let interface_reach = self.interface_reach(trait_ref.def_id, self_adt);
        self.current_module = interface_reach.module;

        let mut seen = HashSet::new();
        for arg in trait_ref.args {
            if let Some(arg_type) = arg.as_type() {
                self.record_interface_component_types(
                    arg_type,
                    self_adt,
                    interface_reach.visibility,
                    &mut seen,
                );
            }
        }
        for assoc_def_id in self.tcx.associated_item_def_ids(impl_def) {
            match self.tcx.def_kind(*assoc_def_id) {
                DefKind::AssocTy | DefKind::AssocConst { .. } => {
                    let assoc_type = self
                        .tcx
                        .type_of(*assoc_def_id)
                        .instantiate_identity()
                        .skip_normalization();
                    self.record_interface_component_types(
                        assoc_type,
                        self_adt,
                        interface_reach.visibility,
                        &mut seen,
                    );
                },
                DefKind::AssocFn => {
                    let signature = self.tcx.fn_sig(*assoc_def_id).instantiate_identity();
                    for input_or_output in signature.skip_binder().inputs_and_output {
                        self.record_interface_component_types(
                            input_or_output,
                            self_adt,
                            interface_reach.visibility,
                            &mut seen,
                        );
                    }
                },
                _ => {},
            }
        }

        self.current_module = previous_module;
    }

    /// The widest module a trait impl's interface is usable from: the
    /// narrower of the trait's visibility and the self type's visibility.
    /// `Public` on both sides reaches the whole crate (and beyond), so the
    /// crate root stands in as the caller module.
    fn interface_reach(&self, trait_def_id: DefId, self_adt: Option<DefId>) -> InterfaceReach {
        let trait_visibility = self.tcx.visibility(trait_def_id);
        let self_visibility =
            self_adt.map_or(Visibility::Public, |adt_did| self.tcx.visibility(adt_did));
        match (trait_visibility, self_visibility) {
            (Visibility::Restricted(trait_scope), Visibility::Restricted(self_scope)) => {
                let module = if self.tcx.is_descendant_of(trait_scope, self_scope) {
                    trait_scope
                } else {
                    self_scope
                };
                InterfaceReach {
                    module,
                    visibility: InterfaceVisibility::Restricted,
                }
            },
            (Visibility::Restricted(scope), Visibility::Public)
            | (Visibility::Public, Visibility::Restricted(scope)) => InterfaceReach {
                module:     scope,
                visibility: InterfaceVisibility::Restricted,
            },
            (Visibility::Public, Visibility::Public) => InterfaceReach {
                module:     CRATE_DEF_ID.to_def_id(),
                visibility: InterfaceVisibility::Public,
            },
        }
    }

    /// Record every local ADT mentioned in `component_type` as used from the
    /// current caller module. The impl's own self type is skipped: narrowing
    /// the self type narrows the interface with it, so the interface imposes
    /// no visibility floor on it.
    fn record_interface_component_types(
        &mut self,
        component_type: ty::Ty<'tcx>,
        self_adt: Option<DefId>,
        interface_visibility: InterfaceVisibility,
        seen: &mut HashSet<DefId>,
    ) {
        for arg in component_type.walk() {
            if let Some(component) = arg.as_type()
                && let ty::TyKind::Adt(adt_def, _) = component.kind()
                && adt_def.did().is_local()
                && Some(adt_def.did()) != self_adt
                && seen.insert(adt_def.did())
            {
                if interface_visibility == InterfaceVisibility::Public {
                    self.public_visibility_targets
                        .insert(adt_def.did().expect_local());
                }
                self.push_site(adt_def.did());
            }
        }
    }

    /// True when `field` is visible beyond `owning_module` — i.e. its
    /// visibility is `pub` or restricted to a scope wider than the type's own
    /// module. A module-private field caps the reach of its type and is not
    /// followed.
    fn field_escapes_module(&self, field: DefId, owning_module: DefId) -> bool {
        match self.tcx.visibility(field) {
            Visibility::Public => true,
            Visibility::Restricted(scope) => scope != owning_module,
        }
    }

    fn push_site(&mut self, target: DefId) {
        self.out.push(UseSite {
            target_def_path:        self.tcx.def_path_str(target),
            caller_module_def_path: self.tcx.def_path_str(self.current_module),
        });
    }

    fn record_qpath(&mut self, qpath: &QPath<'_>, hir_id: HirId) {
        let res = match qpath {
            QPath::Resolved(_, path) => path.res,
            QPath::TypeRelative(..) => {
                // Type-relative paths (e.g. `Foo::method`) need typeck to
                // resolve. Best-effort lookup via typeck_results.
                let owner = hir_id.owner.def_id;
                if !self.tcx.has_typeck_results(owner) {
                    return;
                }
                let typeck = self.tcx.typeck(owner);
                typeck.qpath_res(qpath, hir_id)
            },
        };
        if let Res::Def(_, def_id) = res {
            self.record_target(def_id);
        }
    }
}

impl<'tcx> Visitor<'tcx> for UseSiteCollector<'_, 'tcx> {
    type NestedFilter = All;

    fn maybe_tcx(&mut self) -> TyCtxt<'tcx> { self.tcx }

    fn visit_item(&mut self, item: &'tcx Item<'tcx>) {
        let prev = self.current_module;
        if matches!(item.kind, ItemKind::Mod(..)) {
            self.current_module = item.owner_id.def_id.to_def_id();
        } else {
            self.current_module = self
                .tcx
                .parent_module_from_def_id(item.owner_id.def_id)
                .to_def_id();
        }
        if matches!(item.kind, ItemKind::Impl(..)) {
            self.record_trait_impl_interface(item.owner_id.def_id);
        }
        walk_item(self, item);
        self.current_module = prev;
    }

    fn visit_impl_item(&mut self, item: &'tcx ImplItem<'tcx>) {
        let prev = self.current_module;
        self.current_module = self
            .tcx
            .parent_module_from_def_id(item.owner_id.def_id)
            .to_def_id();
        walk_impl_item(self, item);
        self.current_module = prev;
    }

    fn visit_trait_item(&mut self, item: &'tcx TraitItem<'tcx>) {
        let prev = self.current_module;
        self.current_module = self
            .tcx
            .parent_module_from_def_id(item.owner_id.def_id)
            .to_def_id();
        walk_trait_item(self, item);
        self.current_module = prev;
    }

    fn visit_expr(&mut self, expr: &'tcx Expr<'tcx>) {
        match &expr.kind {
            ExprKind::Path(qpath) => self.record_qpath(qpath, expr.hir_id),
            ExprKind::MethodCall(..) => {
                // Method-call dispatch is type-dependent, not path-based.
                // The callee def-id lives in TypeckResults, not in any
                // QPath the visitor descends into.
                let owner = expr.hir_id.owner.def_id;
                if self.tcx.has_typeck_results(owner)
                    && let Some(def_id) = self.tcx.typeck(owner).type_dependent_def_id(expr.hir_id)
                {
                    self.record_target(def_id);
                }
            },
            ExprKind::Struct(qpath, ..) => self.record_qpath(qpath, expr.hir_id),
            _ => {},
        }
        walk_expr(self, expr);
    }

    fn visit_ty(&mut self, ty: &'tcx Ty<'tcx, AmbigArg>) {
        if let TyKind::Path(qpath) = &ty.kind {
            self.record_qpath(qpath, ty.hir_id);
        }
        rustc_hir::intravisit::walk_ty(self, ty);
    }

    fn visit_pat(&mut self, pat: &'tcx Pat<'tcx>) {
        if let PatKind::Expr(expr) = &pat.kind
            && let PatExprKind::Path(qpath) = &expr.kind
        {
            self.record_qpath(qpath, expr.hir_id);
        }
        rustc_hir::intravisit::walk_pat(self, pat);
    }
}

/// Walk the entire crate's HIR and append every resolved
/// expression/type/pattern path reference to `out`. The caller module is
/// the nearest enclosing module def (defaults to the crate root).
pub(super) fn collect_use_sites(
    tcx: TyCtxt<'_>,
    out: &mut Vec<UseSite>,
    public_visibility_targets: &mut HashSet<LocalDefId>,
) {
    let mut collector = UseSiteCollector {
        tcx,
        current_module: CRATE_DEF_ID.to_def_id(),
        out,
        public_visibility_targets,
    };
    let crate_items = tcx.hir_crate_items(());
    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        collector.visit_item(item);
    }
    for impl_item_id in crate_items.impl_items() {
        let impl_item = tcx.hir_impl_item(impl_item_id);
        collector.visit_impl_item(impl_item);
    }
    for trait_item_id in crate_items.trait_items() {
        let trait_item = tcx.hir_trait_item(trait_item_id);
        collector.visit_trait_item(trait_item);
    }
}

/// Build an active-HIR index of re-export occurrences.
///
/// The index never derives a module identity from a source filename. That
/// keeps `#[cfg]`, macro-generated imports, `#[path]` modules, raw identifiers,
/// and grouped imports aligned with the compiler's resolved item graph.
pub(super) fn reexport_index(tcx: TyCtxt<'_>) -> ReexportIndex {
    let crate_items = tcx.hir_crate_items(());
    let mut index = ReexportIndex::default();
    let mut normalizer = SubjectNormalizer {
        tcx,
        inherent_self_types: HashMap::new(),
    };

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        index.insert_extern_crate(tcx, item);
    }

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        let visibility = tcx
            .local_visibility(item.owner_id.def_id)
            .map_id(LocalDefId::to_def_id);
        let owner_module: LocalDefId = tcx.parent_module_from_def_id(item.owner_id.def_id).into();
        let visibility_syntax = visibility_syntax(tcx, item);
        let parent_module: LocalDefId = tcx.parent_module_from_def_id(owner_module).into();
        let visibility_decision =
            facade_visibility_decision(visibility_syntax, visibility, owner_module, parent_module);
        if matches!(item.kind, ItemKind::Use(..) | ItemKind::ExternCrate(..))
            && !visibility_decision.is_reexport
        {
            continue;
        }
        let base_occurrence = ReexportOccurrence {
            use_def_id: item.owner_id.def_id,
            owner_module,
            visibility,
            facade_spelling: visibility_decision.spelling,
            spelling_conflict: visibility_decision.spelling_conflict,
            use_kind: FacadeUseKind::Named,
            alias: None,
            export_names: Vec::new(),
            span: item.vis_span,
            usage_by_name: Rc::new(OnceCell::new()),
        };

        match item.kind {
            ItemKind::Use(path, UseKind::Single(alias)) => {
                let mut occurrence = base_occurrence;
                occurrence.alias = Some(alias.name.to_string());
                occurrence.export_names.push(alias.name.to_string());
                for resolution in path.res.present_items() {
                    let Res::Def(_, target) = resolution else {
                        continue;
                    };
                    let subject = resolved_named_use_subject(
                        &index,
                        &mut normalizer,
                        tcx,
                        owner_module,
                        path,
                        target,
                    );
                    index
                        .direct_use_subjects
                        .entry(item.owner_id.def_id)
                        .or_insert(subject);
                    index.insert_named(subject, occurrence.clone());
                }
            },
            ItemKind::Use(path, UseKind::Glob) => {
                let mut occurrence = base_occurrence;
                occurrence.use_kind = FacadeUseKind::Glob;
                for resolution in path.res.present_items() {
                    let Res::Def(def_kind, container) = resolution else {
                        continue;
                    };
                    if matches!(def_kind, DefKind::Mod | DefKind::Enum) {
                        let mut container_occurrence = occurrence.clone();
                        container_occurrence.export_names = glob_export_names(tcx, container);
                        if !container.is_local() {
                            index
                                .direct_use_subjects
                                .entry(item.owner_id.def_id)
                                .or_insert(container);
                            index.insert_named(container, container_occurrence.clone());
                        }
                        index.insert_glob(container, container_occurrence);
                    }
                }
            },
            ItemKind::ExternCrate(..) => {
                insert_extern_crate_occurrence(&mut index, tcx, item, base_occurrence);
            },
            _ => {},
        }
    }

    for impl_item_id in crate_items.impl_items() {
        let item = tcx.hir_impl_item(impl_item_id);
        if let Some(subject) = normalizer
            .normalized_subject(item.owner_id.def_id.to_def_id())
            .as_local()
        {
            index.facade_subjects.insert(item.owner_id.def_id, subject);
        }
    }

    index
}

fn glob_export_names(tcx: TyCtxt<'_>, container: DefId) -> Vec<String> {
    let Some(container) = container.as_local() else {
        return Vec::new();
    };
    let mut names = tcx
        .module_children_local(container)
        .iter()
        .map(|child| child.ident.name.to_string())
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn foreign_extern_crate_subject(tcx: TyCtxt<'_>, item: &Item<'_>) -> Option<DefId> {
    let ItemKind::ExternCrate(original_name, ident) = item.kind else {
        return None;
    };
    if let Some(crate_num) = tcx.extern_mod_stmt_cnum(item.owner_id.def_id) {
        return Some(crate_num.as_def_id());
    }
    let crate_name = original_name.unwrap_or(ident.name);
    tcx.crates(())
        .iter()
        .copied()
        .find(|crate_num| tcx.crate_name(*crate_num) == crate_name)
        .map(CrateNum::as_def_id)
}

fn insert_extern_crate_occurrence(
    index: &mut ReexportIndex,
    tcx: TyCtxt<'_>,
    item: &Item<'_>,
    mut occurrence: ReexportOccurrence,
) {
    let ItemKind::ExternCrate(_, ident) = item.kind else {
        return;
    };
    occurrence.use_kind = FacadeUseKind::ExternCrate;
    occurrence.alias = Some(ident.name.to_string());
    occurrence.export_names.push(ident.name.to_string());
    index.insert_named(item.owner_id.def_id.to_def_id(), occurrence.clone());
    if let Some(subject) = foreign_extern_crate_subject(tcx, item) {
        index
            .direct_use_subjects
            .insert(item.owner_id.def_id, subject);
        index.insert_named(subject, occurrence);
    }
}

fn resolved_named_use_subject<Resolution>(
    index: &ReexportIndex,
    normalizer: &mut SubjectNormalizer<'_>,
    tcx: TyCtxt<'_>,
    owner_module: LocalDefId,
    path: &Path<'_, Resolution>,
    target: DefId,
) -> DefId {
    local_extern_crate_subject(index, tcx, owner_module, path).map_or_else(
        || normalizer.normalized_subject(target),
        |subject| {
            index
                .direct_use_subjects
                .get(&subject)
                .copied()
                .unwrap_or_else(|| subject.to_def_id())
        },
    )
}

fn local_extern_crate_subject<Resolution>(
    index: &ReexportIndex,
    tcx: TyCtxt<'_>,
    owner_module: LocalDefId,
    path: &Path<'_, Resolution>,
) -> Option<LocalDefId> {
    let mut module = owner_module;
    for (segment_index, segment) in path.segments.iter().enumerate() {
        if segment_index + 1 == path.segments.len() {
            return index
                .extern_crates
                .get(&(module, segment.ident.name.to_string()))
                .copied();
        }
        match segment.ident.name.as_str() {
            "self" => {},
            "super" => module = tcx.parent_module_from_def_id(module).into(),
            "crate" if segment_index == 0 => module = CRATE_DEF_ID,
            _ => {
                let child = tcx
                    .module_children_local(module)
                    .iter()
                    .find(|child| child.ident.name == segment.ident.name)?;
                match child.res {
                    Res::Def(DefKind::Mod, def_id) => module = def_id.as_local()?,
                    _ => return None,
                }
            },
        }
    }
    None
}

fn visibility_syntax(tcx: TyCtxt<'_>, item: &Item<'_>) -> Option<VisibilitySyntax> {
    let source_map = tcx.sess.source_map();
    let spelling = source_map.span_to_snippet(item.vis_span).ok()?;
    annotation::VisibilityAnnotation::from_item(&spelling, item.owner_id.def_id, tcx)
        .map(|annotation| annotation.syntax())
}

fn facade_visibility_decision(
    visibility_syntax: Option<VisibilitySyntax>,
    visibility: Visibility<DefId>,
    owner_module: LocalDefId,
    parent_module: LocalDefId,
) -> FacadeVisibilityDecision {
    match visibility_syntax {
        Some(VisibilitySyntax::Private) => FacadeVisibilityDecision::private(),
        Some(VisibilitySyntax::Public) => {
            FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Public)
        },
        Some(VisibilitySyntax::Crate) => {
            FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Crate)
        },
        Some(VisibilitySyntax::Parent) => {
            FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Super)
        },
        Some(
            VisibilitySyntax::Current
            | VisibilitySyntax::InCrate
            | VisibilitySyntax::InParent
            | VisibilitySyntax::InCurrent
            | VisibilitySyntax::InPath(_),
        ) => FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Other),
        None => fallback_facade_visibility_decision(visibility, owner_module, parent_module),
    }
}

fn fallback_facade_visibility_decision(
    visibility: Visibility<DefId>,
    owner_module: LocalDefId,
    parent_module: LocalDefId,
) -> FacadeVisibilityDecision {
    match visibility {
        Visibility::Public => FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Public),
        Visibility::Restricted(scope) if scope == CRATE_DEF_ID.to_def_id() => {
            // At the crate root, rustc resolves both a private `use` and an
            // explicit `pub(crate) use` to `CRATE_DEF_ID`. When the source span
            // is unavailable, keep this as a facade: excluding a real
            // `pub(crate)` re-export would create a false finding.
            // Its spelling is unknown, though: this reach can also come from
            // `pub(super)` in a crate-root child or `pub(in crate)`.
            FacadeVisibilityDecision::reexport_with_unknown_spelling()
        },
        Visibility::Restricted(scope) if scope == parent_module.to_def_id() => {
            // `pub(super)` and `pub(in super)` have the same resolved scope.
            FacadeVisibilityDecision::reexport_with_unknown_spelling()
        },
        Visibility::Restricted(scope) if scope == owner_module.to_def_id() => {
            FacadeVisibilityDecision::private()
        },
        Visibility::Restricted(_) => {
            FacadeVisibilityDecision::reexport(ParentFacadeSpelling::Other)
        },
    }
}

/// Returns the def-path of `LocalDefId` as a `String`, e.g.
/// `tui::panes::cpu::cpu_required_pane_height`. Local def-paths are rendered
/// root-relative with no leading `crate::` and no crate-name segment.
pub(super) fn def_path_string(tcx: TyCtxt<'_>, def_id: LocalDefId) -> String {
    tcx.def_path_str(def_id.to_def_id())
}

/// Returns the def-path of the parent module of `def_id`. For a function
/// in `crate::tui::panes::cpu`, returns `crate::tui::panes::cpu`. Used
/// when synthesizing the proposed narrower scope for a `pub(super)`
/// suggestion.
pub(super) fn parent_module_def_path(tcx: TyCtxt<'_>, def_id: LocalDefId) -> String {
    let parent = tcx.parent_module_from_def_id(def_id);
    tcx.def_path_str(parent.to_def_id())
}

pub(super) fn parent_module_path_segments(tcx: TyCtxt<'_>, def_id: LocalDefId) -> Vec<String> {
    let mut segments = parent_module_def_path(tcx, def_id)
        .split("::")
        .filter(|segment| !segment.is_empty())
        .map(String::from)
        .collect::<Vec<_>>();
    if PathAnchor::first(&segments) == Some(PathAnchor::Crate) {
        segments.remove(0);
    }
    segments
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;
    use anyhow::anyhow;
    use rustc_driver::Callbacks;
    use rustc_driver::Compilation;
    use rustc_hir::ItemKind;
    use rustc_hir::UseKind;
    use rustc_hir::def::DefKind;
    use rustc_hir::def::Res;
    use rustc_interface::interface::Compiler;
    use rustc_middle::ty::TyCtxt;
    use rustc_middle::ty::Visibility;
    use rustc_span::def_id::CRATE_DEF_ID;
    use rustc_span::def_id::DefId;
    use rustc_span::def_id::LocalDefId;
    use tempfile::tempdir;

    use super::FacadeChainBlocker;
    use super::FacadeChainResolution;
    use super::FacadeUseKind;
    use super::ParentFacadeSpelling;
    use super::ReexportIndex;
    use super::VisibilityReach;
    use super::facade_visibility_decision;
    use super::reexport_index;

    #[test]
    fn reexport_index_propagates_local_extern_reexports_to_foreign_subjects() -> Result<()> {
        let temp = tempdir()?;
        let source = temp.path().join("fixture.rs");
        let output = temp.path().join("fixture.rmeta");
        fs::write(
            &source,
            "mod a {\n    pub(crate) mod self_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod parent_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod root_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod child {\n        pub(crate) mod grandchild {\n            pub(crate) use crate::a::root_local::core_alias as crate_alias;\n        }\n        pub(crate) use super::parent_local::core_alias as super_alias;\n    }\n    pub(crate) use self::self_local::core_alias as self_alias;\n}\nmod facade {\n    pub(crate) mod child {\n        pub(crate) struct Widget;\n        impl Widget {\n            pub(crate) fn accepted_method() {}\n            pub(crate) const ACCEPTED_CONST: usize = 1;\n            pub(super) fn capped_method() {}\n            pub(super) const CAPPED_CONST: usize = 1;\n        }\n    }\n    pub(crate) use child::Widget;\n}\nmod spelling {\n    mod child { pub struct Subject; }\n    pub(super) use child::Subject;\n}\npub use core::fmt::Error as ForeignError;\nfn main() {}\n",
        )?;

        let arguments = vec![
            String::from("rustc"),
            source.display().to_string(),
            String::from("--crate-name"),
            String::from("reexport_index_fixture"),
            String::from("--edition=2024"),
            String::from("--emit=metadata"),
            String::from("-o"),
            output.display().to_string(),
        ];
        let mut callbacks = IndexAssertions::default();
        rustc_driver::catch_with_exit_code(|| {
            rustc_driver::run_compiler(&arguments, &mut callbacks);
        });

        callbacks
            .result
            .ok_or_else(|| anyhow!("index assertions did not run"))?
    }

    #[derive(Default)]
    struct IndexAssertions {
        result: Option<Result<()>>,
    }

    impl Callbacks for IndexAssertions {
        fn after_analysis(&mut self, _: &Compiler, tcx: TyCtxt<'_>) -> Compilation {
            self.result = Some(assert_index_behavior(tcx));
            Compilation::Stop
        }
    }

    fn assert_index_behavior(tcx: TyCtxt<'_>) -> Result<()> {
        let index = reexport_index(tcx);
        let crate_module: LocalDefId = CRATE_DEF_ID;
        let a_module = child_module(tcx, crate_module, "a")?;
        let nested_child_module = child_module(tcx, a_module, "child")?;
        let nested_grandchild_module = child_module(tcx, nested_child_module, "grandchild")?;
        assert_unsnippable_visibility_fallback(
            nested_grandchild_module,
            nested_child_module,
            a_module,
        );
        assert_local_extern_reexport(tcx, &index, a_module, "self_local", "self_alias")?;
        assert_local_extern_reexport(tcx, &index, a_module, "parent_local", "super_alias")?;
        assert_local_extern_reexport(tcx, &index, a_module, "root_local", "crate_alias")?;

        let foreign_target = foreign_reexport_target(tcx)?;
        let foreign_occurrences = index
            .named
            .get(&foreign_target)
            .ok_or_else(|| anyhow!("missing foreign re-export occurrence"))?;
        assert!(foreign_occurrences.iter().any(|occurrence| {
            occurrence.use_kind == FacadeUseKind::Named
                && occurrence.alias.as_deref() == Some("ForeignError")
        }));
        let local_reach = VisibilityReach::from(Visibility::Restricted(CRATE_DEF_ID.to_def_id()));
        let foreign_reach = VisibilityReach::from(Visibility::Restricted(foreign_target));
        assert_eq!(
            tcx.parent_module_from_def_id(CRATE_DEF_ID).to_def_id(),
            CRATE_DEF_ID.to_def_id(),
            "the crate root must be its own parent module"
        );
        assert_eq!(
            local_reach.join(foreign_reach, tcx).to_source(tcx),
            "pub",
            "a foreign boundary must reach the fixed point without leaving the local crate"
        );

        let facade_module = child_module(tcx, crate_module, "facade")?;
        let facade_child_module = child_module(tcx, facade_module, "child")?;
        let widget = child_item(tcx, facade_child_module, "Widget")?;
        let accepted_method = impl_item(tcx, "accepted_method")?;
        let accepted_const = impl_item(tcx, "ACCEPTED_CONST")?;
        let capped_method = impl_item(tcx, "capped_method")?;
        let capped_const = impl_item(tcx, "CAPPED_CONST")?;

        for item in [accepted_method, accepted_const, capped_method, capped_const] {
            assert_eq!(index.facade_subject(item), widget);
        }
        for item in [accepted_method, accepted_const] {
            let Some(analysis) = index.parent_facade_analysis(tcx, item, widget) else {
                return Err(anyhow!("missing parent facade analysis"));
            };
            let FacadeChainResolution::Resolved { required } = analysis.chain else {
                return Err(anyhow!("local facade chain should resolve"));
            };
            assert_eq!(required.to_source(tcx), "pub(crate)");
        }
        for item in [capped_method, capped_const] {
            assert!(index.parent_facade_analysis(tcx, item, widget).is_none());
        }

        let spelling_module = child_module(tcx, crate_module, "spelling")?;
        let spelling_child = child_module(tcx, spelling_module, "child")?;
        let spelling_subject = child_item(tcx, spelling_child, "Subject")?;
        let spelling_occurrence = index
            .parent_facade_occurrence(tcx, spelling_subject, spelling_subject)
            .ok_or_else(|| anyhow!("missing pub(super) facade occurrence"))?;
        assert_eq!(
            spelling_occurrence.facade_spelling,
            ParentFacadeSpelling::Super
        );
        Ok(())
    }

    fn assert_unsnippable_visibility_fallback(
        owner_module: LocalDefId,
        parent_module: LocalDefId,
        distant_ancestor: LocalDefId,
    ) {
        let crate_module: LocalDefId = CRATE_DEF_ID;
        let public =
            facade_visibility_decision(None, Visibility::Public, owner_module, parent_module);
        assert!(public.is_reexport);
        assert_eq!(public.spelling, ParentFacadeSpelling::Public);
        assert!(!public.spelling_conflict);

        let private = facade_visibility_decision(
            None,
            Visibility::Restricted(owner_module.to_def_id()),
            owner_module,
            parent_module,
        );
        assert!(!private.is_reexport);
        assert_eq!(private.spelling, ParentFacadeSpelling::Other);
        assert!(!private.spelling_conflict);

        let parent = facade_visibility_decision(
            None,
            Visibility::Restricted(parent_module.to_def_id()),
            owner_module,
            parent_module,
        );
        assert!(parent.is_reexport);
        assert!(parent.spelling_conflict);

        let distant_parent = facade_visibility_decision(
            None,
            Visibility::Restricted(distant_ancestor.to_def_id()),
            owner_module,
            parent_module,
        );
        assert!(distant_parent.is_reexport);
        assert_eq!(distant_parent.spelling, ParentFacadeSpelling::Other);
        assert!(!distant_parent.spelling_conflict);

        let crate_root = facade_visibility_decision(
            None,
            Visibility::Restricted(crate_module.to_def_id()),
            crate_module,
            crate_module,
        );
        assert!(crate_root.is_reexport);
        assert!(crate_root.spelling_conflict);
    }

    fn assert_local_extern_reexport(
        tcx: TyCtxt<'_>,
        index: &ReexportIndex,
        extern_parent_module: LocalDefId,
        extern_module_name: &str,
        expected_alias: &str,
    ) -> Result<()> {
        let extern_module = child_module(tcx, extern_parent_module, extern_module_name)?;
        let extern_def_id = index
            .extern_crates
            .get(&(extern_module, String::from("core_alias")))
            .copied()
            .ok_or_else(|| anyhow!("missing local extern crate declaration"))?;
        let foreign_subject = index
            .direct_use_subjects
            .get(&extern_def_id)
            .copied()
            .ok_or_else(|| anyhow!("missing foreign subject for local extern crate"))?;
        let occurrences = index
            .named
            .get(&foreign_subject)
            .ok_or_else(|| anyhow!("missing re-export occurrence for foreign subject"))?;
        let occurrence = occurrences
            .iter()
            .find(|occurrence| {
                occurrence.use_kind == FacadeUseKind::Named
                    && occurrence.alias.as_deref() == Some(expected_alias)
            })
            .ok_or_else(|| anyhow!("missing expected local extern re-export"))?;
        assert_eq!(
            index.direct_use_subjects.get(&occurrence.use_def_id),
            Some(&foreign_subject)
        );
        let analysis = index
            .parent_facade_analysis(tcx, occurrence.use_def_id, occurrence.use_def_id)
            .ok_or_else(|| anyhow!("missing foreign boundary analysis"))?;
        assert!(matches!(
            analysis.chain,
            FacadeChainResolution::Unresolvable {
                blocker: FacadeChainBlocker::ForeignBoundary(_),
            }
        ));
        Ok(())
    }

    fn child_module(tcx: TyCtxt<'_>, parent: LocalDefId, name: &str) -> Result<LocalDefId> {
        tcx.module_children_local(parent)
            .iter()
            .find_map(|child| match child.res {
                Res::Def(DefKind::Mod, def_id) if child.ident.name.as_str() == name => {
                    def_id.as_local()
                },
                _ => None,
            })
            .ok_or_else(|| anyhow!("missing module {name}"))
    }

    fn child_item(tcx: TyCtxt<'_>, parent: LocalDefId, name: &str) -> Result<LocalDefId> {
        tcx.module_children_local(parent)
            .iter()
            .find_map(|child| match child.res {
                Res::Def(_, def_id) if child.ident.name.as_str() == name => def_id.as_local(),
                _ => None,
            })
            .ok_or_else(|| anyhow!("missing item {name}"))
    }

    fn impl_item(tcx: TyCtxt<'_>, name: &str) -> Result<LocalDefId> {
        for item_id in tcx.hir_crate_items(()).impl_items() {
            let item = tcx.hir_impl_item(item_id);
            if item.ident.name.as_str() == name {
                return Ok(item.owner_id.def_id);
            }
        }
        Err(anyhow!("missing inherent item {name}"))
    }

    fn foreign_reexport_target(tcx: TyCtxt<'_>) -> Result<DefId> {
        for item_id in tcx.hir_crate_items(()).free_items() {
            let item = tcx.hir_item(item_id);
            let ItemKind::Use(path, UseKind::Single(alias)) = item.kind else {
                continue;
            };
            if alias.name.as_str() != "ForeignError" {
                continue;
            }
            for resolution in path.res.present_items() {
                if let Res::Def(_, target) = resolution
                    && !target.is_local()
                {
                    return Ok(target);
                }
            }
        }
        Err(anyhow!("missing foreign re-export target"))
    }
}
