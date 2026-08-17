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
use std::rc::Rc;

use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use rustc_hir::AmbigArg;
use rustc_hir::Expr;
use rustc_hir::ExprField;
use rustc_hir::ExprKind;
use rustc_hir::HirId;
use rustc_hir::ImplItem;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_hir::Pat;
use rustc_hir::PatExprKind;
use rustc_hir::PatField;
use rustc_hir::PatKind;
use rustc_hir::Path;
use rustc_hir::QPath;
use rustc_hir::TraitItem;
use rustc_hir::TraitRef;
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
use rustc_hir::intravisit::walk_trait_ref;
use rustc_middle::hir::nested_filter::All;
use rustc_middle::middle::privacy::EffectiveVisibilities;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty;
use rustc_middle::ty::AssocContainer;
use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::Ident;
use rustc_span::Span;

use super::annotation;
use super::annotation::VisibilityReach;
use super::annotation::VisibilitySyntax;
use crate::compiler::facade::ParentFacadeSpelling;
use crate::compiler::facade::ParentFacadeUsageByName;
use crate::compiler::persistence::UseSiteIndex;
use crate::compiler::persistence::UseSiteReference;
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
pub(in crate::compiler) struct ReexportIndex {
    named:               FxHashMap<DefId, Vec<ReexportOccurrence>>,
    globs:               FxHashMap<DefId, Vec<ReexportOccurrence>>,
    direct_use_subjects: FxHashMap<LocalDefId, DefId>,
    facade_subjects:     FxHashMap<LocalDefId, LocalDefId>,
    extern_crates:       FxHashMap<(LocalDefId, String), LocalDefId>,
}

#[derive(Clone, Copy)]
pub(super) enum ExactGlobSubjectResolution {
    Unresolved,
    Resolved { visibility: Visibility<DefId> },
}

#[derive(Clone)]
pub(super) struct ParentFacadeOccurrences<'index> {
    pub(super) selected:          &'index ReexportOccurrence,
    pub(super) matching:          Vec<&'index ReexportOccurrence>,
    pub(super) spelling_conflict: bool,
}

#[derive(Clone, Copy)]
struct ApplicableReexportReach<'index> {
    occurrence:                  &'index ReexportOccurrence,
    reach:                       VisibilityReach,
    requires_public_declaration: bool,
}

enum ExportedAncestorPathReachResolution {
    Reachable(VisibilityReach),
    IncomparableVisibility,
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

/// Visibility still required by facade boundaries outside the nearest facade.
#[derive(Clone, Copy)]
pub(super) enum RetainedFacadeRequirement {
    Absent,
    Required(VisibilityReach),
}

impl RetainedFacadeRequirement {
    fn join(self, reach: VisibilityReach, tcx: TyCtxt<'_>) -> Self {
        match self {
            Self::Absent => Self::Required(reach),
            Self::Required(current) => Self::Required(current.join(reach, tcx)),
        }
    }
}

/// Nearest-facade metadata and the independently computed full-chain reach.
#[derive(Clone)]
pub(super) struct ParentFacadeAnalysis<'index> {
    pub(super) nearest:                     ParentFacadeOccurrences<'index>,
    pub(super) chain:                       FacadeChainResolution<'index>,
    pub(super) retained_facade_requirement: RetainedFacadeRequirement,
}

impl ReexportIndex {
    pub(in crate::compiler) fn facade_subject(&self, item_def_id: LocalDefId) -> LocalDefId {
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
                nearest:                     ParentFacadeOccurrences {
                    selected:          occurrence,
                    matching:          vec![occurrence],
                    spelling_conflict: occurrence.spelling_conflict,
                },
                chain:                       FacadeChainResolution::Unresolvable {
                    blocker: FacadeChainBlocker::ForeignBoundary(occurrence),
                },
                retained_facade_requirement: RetainedFacadeRequirement::Absent,
            });
        }
        if child_module == CRATE_DEF_ID {
            return None;
        }

        let mut nearest = None;
        let mut required: Option<VisibilityReach> = None;
        let mut retained_facade_requirement = RetainedFacadeRequirement::Absent;
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
                } else {
                    retained_facade_requirement =
                        retained_facade_requirement.join(boundary_reach, tcx);
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
                        retained_facade_requirement,
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
                    retained_facade_requirement,
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
        self.applicable_reexport_reaches(tcx, item_def_id, facade_subject)
            .any(|reexport| reexport.requires_public_declaration)
    }

    /// Whether a `pub use` that rustc requires a `pub` declaration for sits
    /// outside the item's own ancestor modules.
    ///
    /// A re-export in an ancestor is the parent facade, and the narrowing
    /// fixers rewrite that line together with the declaration. One anywhere
    /// else — a sibling module, say — has no facade line to move with it, so
    /// narrowing the declaration alone leaves the `pub use` naming an item that
    /// is no longer `pub` and the crate stops compiling with E0364.
    pub(super) fn has_public_reexport_outside_ancestors(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> bool {
        let parent_module: LocalDefId = tcx.parent_module_from_def_id(item_def_id).into();
        self.applicable_reexport_reaches(tcx, item_def_id, facade_subject)
            .any(|reexport| {
                reexport.requires_public_declaration
                    && !Self::is_module_within(tcx, parent_module, reexport.occurrence.owner_module)
            })
    }

    pub(in crate::compiler) fn applicable_reexport_reaches_outside_parent(
        &self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> impl Iterator<Item = VisibilityReach> {
        self.applicable_reexports_outside_parent(tcx, item_def_id, facade_subject)
            .map(|reexport| reexport.reach)
    }

    fn applicable_reexports_outside_parent<'index>(
        &'index self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> impl Iterator<Item = ApplicableReexportReach<'index>> {
        let parent_module: LocalDefId = tcx.parent_module_from_def_id(item_def_id).into();
        self.applicable_reexport_reaches(tcx, item_def_id, facade_subject)
            .filter(move |reexport| {
                !Self::is_module_within(tcx, reexport.occurrence.owner_module, parent_module)
            })
    }

    /// Re-export reaches supplied by resolved module ancestors, capped by the
    /// declaration and every intervening descendant module.
    pub(in crate::compiler) fn applicable_exported_ancestor_path_reaches(
        &self,
        tcx: TyCtxt<'_>,
        declaration: LocalDefId,
    ) -> impl Iterator<Item = VisibilityReach> {
        let mut reaches = Vec::new();
        let mut exported_ancestor = if matches!(tcx.def_kind(declaration.to_def_id()), DefKind::Mod)
        {
            declaration
        } else {
            tcx.parent_module_from_def_id(declaration).into()
        };

        while exported_ancestor != CRATE_DEF_ID {
            let facade_subject = self.facade_subject(exported_ancestor);
            for reexport in
                self.applicable_reexports_outside_parent(tcx, exported_ancestor, facade_subject)
            {
                if let ExportedAncestorPathReachResolution::Reachable(reach) =
                    Self::reach_through_descendant_path(
                        tcx,
                        declaration,
                        exported_ancestor,
                        reexport.reach,
                    )
                {
                    reaches.push(reach);
                }
            }
            exported_ancestor = tcx.parent_module_from_def_id(exported_ancestor).into();
        }

        reaches.into_iter()
    }

    fn reach_through_descendant_path(
        tcx: TyCtxt<'_>,
        declaration: LocalDefId,
        exported_ancestor: LocalDefId,
        exported_ancestor_reach: VisibilityReach,
    ) -> ExportedAncestorPathReachResolution {
        let mut path_segment = declaration;
        let mut path_reach = exported_ancestor_reach;
        while path_segment != exported_ancestor {
            let segment_reach = VisibilityReach::from(tcx.visibility(path_segment.to_def_id()));
            path_reach = match path_reach.compare(segment_reach, tcx) {
                Some(Ordering::Equal | Ordering::Less) => path_reach,
                Some(Ordering::Greater) => segment_reach,
                None => {
                    return ExportedAncestorPathReachResolution::IncomparableVisibility;
                },
            };
            path_segment = tcx.parent_module_from_def_id(path_segment).into();
        }
        ExportedAncestorPathReachResolution::Reachable(annotation::anchored(
            path_reach,
            declaration,
            tcx,
        ))
    }

    fn applicable_reexport_reaches<'index>(
        &'index self,
        tcx: TyCtxt<'_>,
        item_def_id: LocalDefId,
        facade_subject: LocalDefId,
    ) -> impl Iterator<Item = ApplicableReexportReach<'index>> {
        let subject = facade_subject.to_def_id();
        let mut use_def_ids = FxHashSet::default();
        self.named
            .get(&subject)
            .into_iter()
            .flatten()
            .chain(self.globs.values().flatten())
            .filter_map(move |occurrence| {
                if !use_def_ids.insert(occurrence.use_def_id) {
                    return None;
                }
                let effective_visibility = match occurrence.use_kind {
                    FacadeUseKind::Glob => {
                        let ExactGlobSubjectResolution::Resolved { visibility } =
                            Self::exact_glob_subject_resolution(tcx, subject, occurrence)
                        else {
                            return None;
                        };
                        visibility
                    },
                    FacadeUseKind::Named | FacadeUseKind::ExternCrate => occurrence.visibility,
                };
                let effective_reach = VisibilityReach::from(effective_visibility);
                let private_reach = VisibilityReach::from(Visibility::Restricted(
                    occurrence.owner_module.to_def_id(),
                ));
                if effective_reach.compare(private_reach, tcx) != Some(Ordering::Greater)
                    || !Self::occurrence_applies_to_item(tcx, item_def_id, subject, effective_reach)
                {
                    return None;
                }
                let capped_reach = annotation::capped_by_enclosing_modules(
                    effective_reach,
                    occurrence.use_def_id,
                    tcx,
                )?;
                Some(ApplicableReexportReach {
                    occurrence,
                    reach: annotation::anchored(capped_reach, item_def_id, tcx),
                    requires_public_declaration: match occurrence.use_kind {
                        FacadeUseKind::Named | FacadeUseKind::ExternCrate => {
                            occurrence.visibility.is_public()
                        },
                        FacadeUseKind::Glob => effective_reach.is_public(),
                    },
                })
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
                Self::occurrence_applies_to_item(
                    tcx,
                    item_def_id,
                    subject,
                    VisibilityReach::from(occurrence.visibility),
                )
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
                        && Self::occurrence_applies_to_item(
                            tcx,
                            item_def_id,
                            subject,
                            VisibilityReach::from(occurrence.visibility),
                        )
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
        occurrence_reach: VisibilityReach,
    ) -> bool {
        if item_def_id.to_def_id() == subject {
            return true;
        }
        let item_reach: VisibilityReach = tcx.visibility(item_def_id.to_def_id()).into();
        item_reach.is_at_least(occurrence_reach, tcx)
    }

    pub(super) fn exact_glob_subject_resolution(
        tcx: TyCtxt<'_>,
        subject: DefId,
        occurrence: &ReexportOccurrence,
    ) -> ExactGlobSubjectResolution {
        tcx.module_children_local(occurrence.owner_module)
            .iter()
            .find_map(|child| {
                let resolves_subject = child.res.opt_def_id().is_some_and(|exported| {
                    Self::normalized_export_subject(tcx, exported) == subject
                });
                let resolves_occurrence = child.reexport_chain.first().is_some_and(|reexport| {
                    reexport.id() == Some(occurrence.use_def_id.to_def_id())
                });
                (resolves_subject && resolves_occurrence).then_some(
                    ExactGlobSubjectResolution::Resolved {
                        visibility: child.vis,
                    },
                )
            })
            .unwrap_or(ExactGlobSubjectResolution::Unresolved)
    }

    fn normalized_export_subject(tcx: TyCtxt<'_>, exported: DefId) -> DefId {
        match tcx.def_kind(exported) {
            DefKind::Variant | DefKind::Ctor(CtorOf::Struct, _) => tcx.parent(exported),
            DefKind::Ctor(CtorOf::Variant, _) => tcx.parent(tcx.parent(exported)),
            _ => exported,
        }
    }

    fn distinct_use_occurrences(occurrences: Vec<&ReexportOccurrence>) -> Vec<&ReexportOccurrence> {
        let mut use_def_ids = FxHashSet::default();
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
    inherent_self_types: FxHashMap<DefId, DefId>,
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
    /// Distinct `(referenced item, calling module)` pairs. Def-ids, not
    /// rendered paths: the same pair recurs once per syntactic reference,
    /// so deduplicating here and rendering in [`collect_use_sites`] pays
    /// `def_path_str` once per distinct def-id instead of twice per
    /// occurrence.
    out:                       &'a mut FxHashSet<(DefId, DefId, UseSiteReference)>,
    public_visibility_targets: &'a mut FxHashSet<LocalDefId>,
    effective_visibilities:    &'a EffectiveVisibilities,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InterfaceVisibility {
    Public,
    Restricted,
}

#[derive(Clone, Copy)]
struct InterfaceReach {
    module:     DefId,
    visibility: InterfaceVisibility,
}

impl<'tcx> UseSiteCollector<'_, 'tcx> {
    fn record_target(&mut self, target: DefId) {
        let original_kind = self.tcx.def_kind(target);
        let target = match original_kind {
            DefKind::Variant | DefKind::Ctor(CtorOf::Struct, _) => self.tcx.parent(target),
            DefKind::Ctor(CtorOf::Variant, _) => self.tcx.parent(self.tcx.parent(target)),
            _ => target,
        };
        // Skip references to items in other crates — narrowing decisions
        // only apply to local items.
        if target.is_local() {
            self.push_site(target, UseSiteReference::Named);
            self.record_target_modules(target);
        }
        // Naming a tuple-struct constructor requires every positional field
        // to be visible at the call site. This covers construction, using the
        // constructor as a function value, and tuple-struct patterns. Numeric
        // field access and `offset_of!` are recorded by their dedicated paths.
        if matches!(original_kind, DefKind::Ctor(CtorOf::Struct, _)) {
            for field in &self.tcx.adt_def(target).non_enum_variant().fields {
                self.record_target(field.did);
            }
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

    fn record_import_target(
        &mut self,
        target: DefId,
        visibility_scope: DefId,
        reference: UseSiteReference,
    ) {
        let Some(local_target) = target.as_local() else {
            return;
        };
        self.push_site_from(target, visibility_scope, reference);
        let mut module: LocalDefId = self.tcx.parent_module_from_def_id(local_target).into();
        loop {
            self.push_site_from(module.to_def_id(), visibility_scope, reference);
            if module == CRATE_DEF_ID {
                return;
            }
            module = self.tcx.parent_module_from_def_id(module).into();
        }
    }

    /// A path to an item also requires every module segment on the way to
    /// that item. Record those modules so a restricted module is never
    /// advised to become private while a caller still reaches a descendant
    /// through it.
    fn record_target_modules(&mut self, target: DefId) {
        let Some(local_target) = target.as_local() else {
            return;
        };
        let mut module: LocalDefId = self.tcx.parent_module_from_def_id(local_target).into();
        loop {
            self.push_site(module.to_def_id(), UseSiteReference::Named);
            if module == CRATE_DEF_ID {
                return;
            }
            module = self.tcx.parent_module_from_def_id(module).into();
        }
    }

    /// Record every local type named in a function's signature as used from
    /// the current caller module, following each type's public field graph the
    /// same way alias components are. `fn_sig` yields the declared input and
    /// output types; a module-private field still caps reach, so genuinely
    /// internal types stay flagged.
    fn record_fn_signature_components(&mut self, func: DefId) {
        let signature = self.tcx.fn_sig(func).instantiate_identity();
        let mut seen = FxHashSet::default();
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
        let mut seen = FxHashSet::default();
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
    fn record_exposed_adt(&mut self, did: DefId, seen: &mut FxHashSet<DefId>) {
        let Some(local) = did.as_local() else {
            return;
        };
        if !seen.insert(did) {
            return;
        }
        self.push_site(did, UseSiteReference::ThroughSignature);
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

        let mut seen = FxHashSet::default();
        for arg in trait_ref.args {
            if let Some(arg_type) = arg.as_type() {
                self.record_interface_component_types(
                    arg_type,
                    self_adt,
                    interface_reach.visibility,
                    UseSiteReference::ThroughSignature,
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
                        UseSiteReference::ThroughSignature,
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
                            UseSiteReference::ThroughSignature,
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
        reference: UseSiteReference,
        seen: &mut FxHashSet<DefId>,
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
                self.push_site(adt_def.did(), reference);
            }
        }
    }

    /// Record every local ADT named in a declaration's own interface — a
    /// function signature, a const/static type, or a struct/enum field type —
    /// as used from the widest module that declaration is reachable from.
    /// rustc's `private_interfaces` lint compares the two: `pub fn
    /// to_launch_params(&self) -> LaunchParams` on a type reachable at
    /// `pub(crate)` warns as soon as `LaunchParams` drops below `pub(crate)`,
    /// even when nothing calls the method. `record_fn_signature_components`
    /// only fires at call sites, so a declaration with no caller wide enough
    /// left `--fix` free to narrow the named type and introduce the warning.
    fn record_declaration_interface(&mut self, def_id: LocalDefId) {
        let Some(reach) = self.declaration_reach(def_id) else {
            return;
        };
        match self.tcx.def_kind(def_id.to_def_id()) {
            DefKind::Fn | DefKind::AssocFn => {
                let signature = self.tcx.fn_sig(def_id).instantiate_identity();
                let self_adt = self.impl_self_adt(def_id);
                let inputs_and_output = signature.skip_binder().inputs_and_output;
                self.record_reachable_component_types(reach, self_adt, inputs_and_output);
            },
            DefKind::Const { .. } | DefKind::Static { .. } | DefKind::AssocConst { .. } => {
                let declared_type = self
                    .tcx
                    .type_of(def_id)
                    .instantiate_identity()
                    .skip_normalization();
                let self_adt = self.impl_self_adt(def_id);
                self.record_reachable_component_types(reach, self_adt, [declared_type]);
            },
            _ => {},
        }
    }

    /// Field types are checked against the field's own reach, not the type's:
    /// `pub spawn_insert_example: Option<SpawnInsertExample>` on a `TypeGuide`
    /// reachable at `pub(in crate::brp_tools)` requires `SpawnInsertExample` to
    /// stay usable from there. The owning ADT is skipped — narrowing it narrows
    /// its fields with it.
    fn record_field_declaration_interfaces(&mut self, adt_def_id: LocalDefId) {
        let self_adt = Some(adt_def_id.to_def_id());
        let fields: Vec<DefId> = self
            .tcx
            .adt_def(adt_def_id)
            .all_fields()
            .map(|field| field.did)
            .collect();
        for field in fields {
            let Some(local_field) = field.as_local() else {
                continue;
            };
            let Some(reach) = self.declaration_reach(local_field) else {
                continue;
            };
            let field_type = self
                .tcx
                .type_of(field)
                .instantiate_identity()
                .skip_normalization();
            self.record_reachable_component_types(reach, self_adt, [field_type]);
        }
    }

    /// The widest module a declaration is reachable from, as rustc's
    /// `private_interfaces` lint computes it: the effective visibility at
    /// `Level::Reachable`, which already accounts for private ancestor modules
    /// and widening `pub use` re-exports. No entry means the declaration is
    /// reachable from nowhere outside its own module and constrains nothing.
    fn declaration_reach(&self, def_id: LocalDefId) -> Option<InterfaceReach> {
        let effective_visibility = self.effective_visibilities.effective_vis(def_id)?;
        match effective_visibility.at_level(Level::Reachable).to_def_id() {
            Visibility::Public => Some(InterfaceReach {
                module:     CRATE_DEF_ID.to_def_id(),
                visibility: InterfaceVisibility::Public,
            }),
            Visibility::Restricted(module) => Some(InterfaceReach {
                module,
                visibility: InterfaceVisibility::Restricted,
            }),
        }
    }

    fn record_reachable_component_types(
        &mut self,
        reach: InterfaceReach,
        self_adt: Option<DefId>,
        component_types: impl IntoIterator<Item = ty::Ty<'tcx>>,
    ) {
        let previous_module = self.current_module;
        self.current_module = reach.module;
        let mut seen = FxHashSet::default();
        for component_type in component_types {
            self.record_interface_component_types(
                component_type,
                self_adt,
                reach.visibility,
                UseSiteReference::DeclarationInterface,
                &mut seen,
            );
        }
        self.current_module = previous_module;
    }

    /// The self type of the impl block a declaration belongs to, if any. A
    /// method's signature always names its own self type, and its reach is
    /// bounded by that type's, so the mention imposes no floor on it.
    fn impl_self_adt(&self, def_id: LocalDefId) -> Option<DefId> {
        let parent = self.tcx.parent(def_id.to_def_id());
        if !matches!(self.tcx.def_kind(parent), DefKind::Impl { .. }) {
            return None;
        }
        self.tcx
            .type_of(parent)
            .instantiate_identity()
            .skip_normalization()
            .ty_adt_def()
            .map(ty::AdtDef::did)
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

    fn push_site(&mut self, target: DefId, reference: UseSiteReference) {
        self.push_site_from(target, self.current_module, reference);
    }

    fn push_site_from(&mut self, target: DefId, caller_module: DefId, reference: UseSiteReference) {
        self.out.insert((target, caller_module, reference));
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

    fn record_type_dependent_target(&mut self, hir_id: HirId) {
        let owner = hir_id.owner.def_id;
        if self.tcx.has_typeck_results(owner)
            && let Some(def_id) = self.tcx.typeck(owner).type_dependent_def_id(hir_id)
        {
            self.record_target(def_id);
        }
    }

    fn record_field_target(&mut self, base: &'tcx Expr<'tcx>, hir_id: HirId) {
        let owner = hir_id.owner.def_id;
        if !self.tcx.has_typeck_results(owner) {
            return;
        }
        let typeck = self.tcx.typeck(owner);
        let ty::TyKind::Adt(adt_def, _) = typeck.expr_ty_adjusted(base).kind() else {
            return;
        };
        let Some(field_index) = typeck.opt_field_index(hir_id) else {
            return;
        };
        self.record_target(adt_def.non_enum_variant().fields[field_index].did);
    }

    fn record_struct_expr_field_targets(
        &mut self,
        expr: &'tcx Expr<'tcx>,
        fields: &'tcx [ExprField<'tcx>],
    ) {
        let owner = expr.hir_id.owner.def_id;
        if !self.tcx.has_typeck_results(owner) {
            return;
        }
        let typeck = self.tcx.typeck(owner);
        let ty::TyKind::Adt(adt_def, _) = typeck.expr_ty(expr).kind() else {
            return;
        };
        if adt_def.is_enum() {
            return;
        }
        let variant = adt_def.non_enum_variant();
        if adt_def.is_struct() {
            for field in &variant.fields {
                self.record_target(field.did);
            }
            return;
        }
        for field in fields {
            if let Some(field_index) = typeck.opt_field_index(field.hir_id) {
                self.record_target(variant.fields[field_index].did);
            }
        }
    }

    fn record_struct_pat_field_targets(
        &mut self,
        pat: &'tcx Pat<'tcx>,
        fields: &'tcx [PatField<'tcx>],
    ) {
        let owner = pat.hir_id.owner.def_id;
        if !self.tcx.has_typeck_results(owner) {
            return;
        }
        let typeck = self.tcx.typeck(owner);
        let ty::TyKind::Adt(adt_def, _) = typeck.pat_ty(pat).kind() else {
            return;
        };
        if adt_def.is_enum() {
            return;
        }
        let variant = adt_def.non_enum_variant();
        for field in fields {
            if let Some(field_index) = typeck.opt_field_index(field.hir_id) {
                self.record_target(variant.fields[field_index].did);
            }
        }
    }

    fn record_offset_of_field_targets(&mut self, ty: &'tcx Ty<'tcx, ()>, fields: &'tcx [Ident]) {
        let owner = ty.hir_id.owner.def_id;
        if !self.tcx.has_typeck_results(owner) {
            return;
        }
        let typeck = self.tcx.typeck(owner);
        let Some(mut current_ty) = typeck.node_type_opt(ty.hir_id) else {
            return;
        };
        for field_name in fields {
            let ty::TyKind::Adt(adt_def, args) = current_ty.kind() else {
                return;
            };
            if adt_def.is_enum() {
                return;
            }
            let variant = adt_def.non_enum_variant();
            let Some(field) = variant
                .fields
                .iter()
                .find(|field| field.name == field_name.name)
            else {
                return;
            };
            self.record_target(field.did);
            current_ty = field.ty(self.tcx, args).skip_normalization();
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
        if let Visibility::Restricted(scope) = self.tcx.local_visibility(item.owner_id.def_id)
            && let ItemKind::Use(path, UseKind::Single(_)) = item.kind
        {
            let visibility_scope = scope.to_def_id();
            let reference = if visibility_scope == self.current_module {
                UseSiteReference::PrivateImport
            } else {
                UseSiteReference::RestrictedImport
            };
            for resolution in path.res.present_items() {
                if let Res::Def(_, target) = resolution {
                    self.record_import_target(target, visibility_scope, reference);
                }
            }
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
            // Method-call dispatch is type-dependent, not path-based. The
            // callee def-id lives in `TypeckResults`.
            ExprKind::MethodCall(..) => {
                self.record_type_dependent_target(expr.hir_id);
            },
            ExprKind::Field(base, ..) => self.record_field_target(base, expr.hir_id),
            ExprKind::OffsetOf(ty, fields) => {
                self.record_offset_of_field_targets(ty, fields);
            },
            ExprKind::Struct(qpath, fields, ..) => {
                self.record_qpath(qpath, expr.hir_id);
                self.record_struct_expr_field_targets(expr, fields);
            },
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

    fn visit_trait_ref(&mut self, trait_ref: &'tcx TraitRef<'tcx>) {
        if let Res::Def(_, def_id) = trait_ref.path.res {
            self.record_target(def_id);
        }
        walk_trait_ref(self, trait_ref);
    }

    fn visit_pat(&mut self, pat: &'tcx Pat<'tcx>) {
        match &pat.kind {
            PatKind::Expr(expr) if let PatExprKind::Path(qpath) = &expr.kind => {
                self.record_qpath(qpath, expr.hir_id);
            },
            PatKind::Struct(_, fields, _) => self.record_struct_pat_field_targets(pat, fields),
            PatKind::TupleStruct(qpath, ..) => self.record_qpath(qpath, pat.hir_id),
            _ => {},
        }
        rustc_hir::intravisit::walk_pat(self, pat);
    }
}

/// Walk the entire crate's HIR and index every resolved
/// expression/type/pattern path reference by the referenced item. The
/// caller module is the nearest enclosing module def (defaults to the
/// crate root).
pub(super) fn collect_use_sites(
    tcx: TyCtxt<'_>,
    public_visibility_targets: &mut FxHashSet<LocalDefId>,
) -> UseSiteIndex {
    let mut pairs = FxHashSet::default();
    let mut collector = UseSiteCollector {
        tcx,
        current_module: CRATE_DEF_ID.to_def_id(),
        out: &mut pairs,
        public_visibility_targets,
        effective_visibilities: tcx.effective_visibilities(()),
    };
    let crate_items = tcx.hir_crate_items(());
    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        collector.visit_item(item);
        let item_def_id = item.owner_id.def_id;
        if matches!(
            tcx.def_kind(item_def_id.to_def_id()),
            DefKind::Struct | DefKind::Union | DefKind::Enum
        ) {
            collector.record_field_declaration_interfaces(item_def_id);
        }
        collector.record_declaration_interface(item_def_id);
    }
    for impl_item_id in crate_items.impl_items() {
        let impl_item = tcx.hir_impl_item(impl_item_id);
        collector.visit_impl_item(impl_item);
        collector.record_declaration_interface(impl_item.owner_id.def_id);
    }
    for trait_item_id in crate_items.trait_items() {
        let trait_item = tcx.hir_trait_item(trait_item_id);
        collector.visit_trait_item(trait_item);
        collector.record_declaration_interface(trait_item.owner_id.def_id);
    }

    let mut index = UseSiteIndex::default();
    let mut def_paths: FxHashMap<DefId, String> = FxHashMap::default();
    for (target, caller_module, reference) in pairs {
        let target_def_path = def_paths
            .entry(target)
            .or_insert_with(|| tcx.def_path_str(target))
            .clone();
        let caller_module_def_path = def_paths
            .entry(caller_module)
            .or_insert_with(|| tcx.def_path_str(caller_module))
            .clone();
        index.insert(target_def_path, caller_module_def_path, reference);
    }
    index
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
        inherent_self_types: FxHashMap::default(),
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
        Some(VisibilitySyntax::Current | VisibilitySyntax::InCurrent) => {
            FacadeVisibilityDecision::private()
        },
        Some(
            VisibilitySyntax::InCrate | VisibilitySyntax::InParent | VisibilitySyntax::InPath(_),
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

    use super::ExactGlobSubjectResolution;
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
            "mod a {\n    pub(crate) mod self_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod parent_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod root_local { pub(crate) extern crate core as core_alias; }\n    pub(crate) mod child {\n        pub(crate) mod grandchild {\n            pub(crate) use crate::a::root_local::core_alias as crate_alias;\n        }\n        pub(crate) use super::parent_local::core_alias as super_alias;\n    }\n    pub(crate) use self::self_local::core_alias as self_alias;\n}\nmod facade {\n    pub(crate) mod child {\n        pub(crate) struct Widget;\n        impl Widget {\n            pub(crate) fn accepted_method() {}\n            pub(crate) const ACCEPTED_CONST: usize = 1;\n            pub(super) fn capped_method() {}\n            pub(super) const CAPPED_CONST: usize = 1;\n        }\n    }\n    pub(crate) use child::Widget;\n}\nmod outward_glob {\n    mod b { pub struct Carrier; }\n    mod hidden { pub use super::b::*; }\n    pub use hidden::*;\n}\nmod shadowed_glob {\n    mod b { pub struct Carrier; }\n    mod hidden { pub struct Carrier; pub use super::b::*; }\n}\nmod visibility_filtered_glob {\n    mod source {\n        pub(super) struct RestrictedCarrier;\n        pub struct PublicCarrier;\n    }\n    pub use source::*;\n}\nmod spelling {\n    mod child { pub struct Subject; }\n    pub(super) use child::Subject;\n}\npub use core::fmt::Error as ForeignError;\nfn main() {}\n",
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

        assert_foreign_reexport_behavior(tcx, &index)?;

        assert_facade_subject_behavior(tcx, &index, crate_module)?;

        assert_outward_glob_behavior(tcx, &index, crate_module)?;
        assert_shadowed_glob_behavior(tcx, &index, crate_module)?;
        assert_visibility_filtered_glob_behavior(tcx, &index, crate_module)?;
        Ok(())
    }

    fn assert_foreign_reexport_behavior(tcx: TyCtxt<'_>, index: &ReexportIndex) -> Result<()> {
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
        Ok(())
    }

    fn assert_facade_subject_behavior(
        tcx: TyCtxt<'_>,
        index: &ReexportIndex,
        crate_module: LocalDefId,
    ) -> Result<()> {
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

    fn assert_outward_glob_behavior(
        tcx: TyCtxt<'_>,
        index: &ReexportIndex,
        crate_module: LocalDefId,
    ) -> Result<()> {
        let outward_glob_module = child_module(tcx, crate_module, "outward_glob")?;
        let glob_container = child_module(tcx, outward_glob_module, "b")?;
        let hidden_module = child_module(tcx, outward_glob_module, "hidden")?;
        let carrier = child_item(tcx, glob_container, "Carrier")?;
        let occurrences = index
            .applicable_reexports_outside_parent(tcx, carrier, carrier)
            .collect::<Vec<_>>();
        assert_eq!(occurrences.len(), 2);
        assert!(occurrences.iter().all(|reexport| {
            reexport.occurrence.use_kind == FacadeUseKind::Glob
                && matches!(
                    ReexportIndex::exact_glob_subject_resolution(
                        tcx,
                        carrier.to_def_id(),
                        reexport.occurrence,
                    ),
                    ExactGlobSubjectResolution::Resolved { .. }
                )
        }));
        let inner_occurrence = occurrences
            .iter()
            .find(|reexport| reexport.occurrence.owner_module == hidden_module)
            .ok_or_else(|| anyhow!("missing inner glob occurrence"))?;
        let outer_occurrence = occurrences
            .iter()
            .find(|reexport| reexport.occurrence.owner_module == outward_glob_module)
            .ok_or_else(|| anyhow!("missing outer glob occurrence"))?;
        assert_eq!(
            inner_occurrence.reach.to_source(tcx),
            "pub(in crate::outward_glob)"
        );
        assert_eq!(outer_occurrence.reach.to_source(tcx), "pub(crate)");
        assert_eq!(
            inner_occurrence
                .reach
                .join(outer_occurrence.reach, tcx)
                .to_source(tcx),
            "pub(crate)"
        );
        Ok(())
    }

    fn assert_shadowed_glob_behavior(
        tcx: TyCtxt<'_>,
        index: &ReexportIndex,
        crate_module: LocalDefId,
    ) -> Result<()> {
        let shadowed_glob_module = child_module(tcx, crate_module, "shadowed_glob")?;
        let shadowed_glob_container = child_module(tcx, shadowed_glob_module, "b")?;
        let shadowed_carrier = child_item(tcx, shadowed_glob_container, "Carrier")?;
        assert!(
            index
                .applicable_reexport_reaches_outside_parent(
                    tcx,
                    shadowed_carrier,
                    shadowed_carrier,
                )
                .next()
                .is_none(),
            "the importing module's Carrier must shadow the original glob subject"
        );
        Ok(())
    }

    fn assert_visibility_filtered_glob_behavior(
        tcx: TyCtxt<'_>,
        index: &ReexportIndex,
        crate_module: LocalDefId,
    ) -> Result<()> {
        let module = child_module(tcx, crate_module, "visibility_filtered_glob")?;
        let source = child_module(tcx, module, "source")?;
        let restricted_carrier = child_item(tcx, source, "RestrictedCarrier")?;
        let public_carrier = child_item(tcx, source, "PublicCarrier")?;
        let occurrence = index
            .globs
            .get(&source.to_def_id())
            .into_iter()
            .flatten()
            .find(|occurrence| occurrence.owner_module == module)
            .ok_or_else(|| anyhow!("missing visibility-filtered glob occurrence"))?;
        let ExactGlobSubjectResolution::Resolved {
            visibility: restricted_child_visibility,
        } = ReexportIndex::exact_glob_subject_resolution(
            tcx,
            restricted_carrier.to_def_id(),
            occurrence,
        )
        else {
            return Err(anyhow!("restricted glob child did not resolve"));
        };
        assert_eq!(
            VisibilityReach::from(restricted_child_visibility).to_source(tcx),
            "pub(in crate::visibility_filtered_glob)"
        );
        assert!(
            index
                .applicable_reexport_reaches_outside_parent(
                    tcx,
                    restricted_carrier,
                    restricted_carrier,
                )
                .next()
                .is_none(),
            "a restricted child of a public glob must not count as a public re-export"
        );
        let public_occurrences = index
            .applicable_reexport_reaches_outside_parent(tcx, public_carrier, public_carrier)
            .collect::<Vec<_>>();
        assert_eq!(public_occurrences.len(), 1);
        assert_eq!(public_occurrences[0].to_source(tcx), "pub(crate)");
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
