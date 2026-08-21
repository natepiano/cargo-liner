//! How far an item's visibility may be widened before a trait impl leaks.
//!
//! Widening looks like the safe half of a visibility edit: every name that
//! already resolved still resolves. That holds for name resolution and fails
//! for rustc's private-interfaces rule. Raising an ADT's visibility raises the
//! effective visibility of every trait impl it is the self type of, and rustc
//! then requires every type named in those impls' interfaces to be at least
//! that visible. Widen `Thing` to `pub` while `impl TryFrom<u8> for Thing`
//! names a `pub(in crate::a)` error type and the build stops with E0446, which
//! rolls the whole `--fix` batch back.
//!
//! [`collect_interface_ceilings`] answers, per self type, how far it can go
//! before that happens and which type would be left behind — enough for
//! `--fix` to decline the edit and for the finding to name what to widen
//! first.
//!
//! [`super::use_sites`] reads the same impls from the other direction:
//! `record_trait_impl_interface` records the floor an interface puts on its
//! components, this records the ceiling those components put on the self type.
//! A change to what counts as an interface belongs in both.

use std::collections::hash_map::Entry;

use rustc_hash::FxHashMap;
use rustc_hir::def::DefKind;
use rustc_hir::def_id::DefId;
use rustc_hir::def_id::LocalDefId;
use rustc_middle::ty;
use rustc_middle::ty::TraitRef;
use rustc_middle::ty::Ty;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::TyKind;
use rustc_middle::ty::print::PrintTraitRefExt;
use rustc_middle::ty::print::with_forced_trimmed_paths;

use super::annotation::VisibilityReach;
use super::policy;

/// The narrowest type in a trait impl's interface, and what it means for the
/// impl's self type.
pub(super) struct InterfaceCeiling {
    /// The widest visibility the self type may be given. Anything wider and
    /// rustc rejects the impl (E0446).
    pub(super) reach:       VisibilityReach,
    /// The type that would be left behind, as a crate-rooted path:
    /// `crate::a::b::c::PrepError`.
    pub(super) leaked_type: String,
    /// The impl that would expose it: `impl TryFrom<u8> for Thing`.
    pub(super) impl_header: String,
}

/// Index every local ADT that is the self type of a trait impl by the ceiling
/// its impls impose. Most ADTs get no entry: a type with no trait impl, or
/// whose impls name nothing narrower than the traits they implement, is free
/// to widen.
pub(super) fn collect_interface_ceilings(
    tcx: TyCtxt<'_>,
) -> FxHashMap<LocalDefId, InterfaceCeiling> {
    let mut ceilings: FxHashMap<LocalDefId, InterfaceCeiling> = FxHashMap::default();
    for item_id in tcx.hir_crate_items(()).free_items() {
        let impl_def = item_id.owner_id.def_id;
        if !matches!(
            tcx.def_kind(impl_def.to_def_id()),
            DefKind::Impl { of_trait: true }
        ) {
            continue;
        }
        let trait_ref = tcx
            .impl_trait_ref(impl_def)
            .instantiate_identity()
            .skip_normalization();
        let Some(self_adt) = trait_ref.self_ty().ty_adt_def().map(ty::AdtDef::did) else {
            continue;
        };
        let Some(local_self_adt) = self_adt.as_local() else {
            continue;
        };
        let Some(ceiling) = impl_ceiling(tcx, impl_def, trait_ref, self_adt) else {
            continue;
        };
        match ceilings.entry(local_self_adt) {
            Entry::Occupied(mut occupied) => {
                if occupied.get().reach.is_strictly_wider(ceiling.reach, tcx) {
                    occupied.insert(ceiling);
                }
            },
            Entry::Vacant(vacant) => {
                vacant.insert(ceiling);
            },
        }
    }
    ceilings
}

/// The narrowest interface component of one impl that actually constrains its
/// self type.
///
/// An impl reaches only as far as the narrower of its trait and its self type,
/// so a component already at least as visible as the trait is capped by the
/// trait and constrains the self type not at all. Every visibility read here is
/// the declared one, which is the side rustc's private-interfaces check
/// compares against.
fn impl_ceiling<'tcx>(
    tcx: TyCtxt<'tcx>,
    impl_def: LocalDefId,
    trait_ref: TraitRef<'tcx>,
    self_adt: DefId,
) -> Option<InterfaceCeiling> {
    let trait_reach = VisibilityReach::from(tcx.visibility(trait_ref.def_id));
    let (leaked_def_id, reach) = interface_components(tcx, impl_def, trait_ref)
        .into_iter()
        .flat_map(ty::Ty::walk)
        .filter_map(ty::GenericArg::as_type)
        .filter_map(|component_type| match component_type.kind() {
            TyKind::Adt(adt_def, _) => Some(adt_def.did()),
            _ => None,
        })
        .filter(|component| component.is_local() && *component != self_adt)
        .map(|component| (component, VisibilityReach::from(tcx.visibility(component))))
        .filter(|(_, component_reach)| !component_reach.is_at_least(trait_reach, tcx))
        .fold(
            None,
            |narrowest: Option<(DefId, VisibilityReach)>, candidate| match narrowest {
                Some(current) if !current.1.is_strictly_wider(candidate.1, tcx) => Some(current),
                _ => Some(candidate),
            },
        )?;
    Some(InterfaceCeiling {
        reach,
        leaked_type: policy::crate_rooted_def_path(&tcx.def_path_str(leaked_def_id)),
        impl_header: impl_header(trait_ref),
    })
}

/// Every type in an impl's interface: the trait-ref arguments, and the declared
/// type of each associated item. These are post-expansion HIR items, so an
/// associated type a derive macro wrote counts the same as one in source.
fn interface_components<'tcx>(
    tcx: TyCtxt<'tcx>,
    impl_def: LocalDefId,
    trait_ref: TraitRef<'tcx>,
) -> Vec<Ty<'tcx>> {
    let mut components: Vec<Ty<'tcx>> = trait_ref
        .args
        .iter()
        .filter_map(ty::GenericArg::as_type)
        .collect();
    for assoc_def_id in tcx.associated_item_def_ids(impl_def) {
        match tcx.def_kind(*assoc_def_id) {
            DefKind::AssocTy | DefKind::AssocConst { .. } => components.push(
                tcx.type_of(*assoc_def_id)
                    .instantiate_identity()
                    .skip_normalization(),
            ),
            DefKind::AssocFn => components.extend(
                tcx.fn_sig(*assoc_def_id)
                    .instantiate_identity()
                    .skip_binder()
                    .inputs_and_output,
            ),
            _ => {},
        }
    }
    components
}

/// How the impl reads back to someone looking for it in source:
/// `impl TryFrom<u8> for Thing`. Trimmed paths are what makes it match the
/// line to search for — the untrimmed form spells the same impl
/// `impl std::convert::TryFrom<u8> for a::b::c::pose::Thing`, which appears in
/// no file.
fn impl_header(trait_ref: TraitRef<'_>) -> String {
    with_forced_trimmed_paths!(format!(
        "impl {} for {}",
        trait_ref.print_only_trait_path(),
        trait_ref.self_ty()
    ))
}
