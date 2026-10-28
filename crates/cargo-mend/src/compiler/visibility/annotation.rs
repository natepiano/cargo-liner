use std::cmp::Ordering;

use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;
use rustc_span::def_id::ModId;
use syn::VisRestricted;
use syn::Visibility as SyntaxVisibility;

use super::policy;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PathSpelling {
    CrateRooted,
    Relative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisibilitySyntax {
    Private,
    Public,
    Crate,
    Parent,
    Current,
    InCrate,
    InParent,
    InCurrent,
    InPath(PathSpelling),
}

/// The kind of boundary limiting a reach, independent of its written annotation.
/// Policy asks about the boundary through this type, so it never reads rendered text back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReachBoundary {
    /// Reachable from other crates — `pub`.
    Everywhere,
    /// Capped at the crate root — `pub(crate)`.
    CrateRoot,
    /// Capped at a module below the crate root — `pub(in crate::…)`.
    Module(DefId),
}

impl From<VisibilityReach> for ReachBoundary {
    fn from(reach: VisibilityReach) -> Self {
        match reach.0 {
            Visibility::Public => Self::Everywhere,
            Visibility::Restricted(boundary) if boundary == CRATE_DEF_ID.to_def_id() => {
                Self::CrateRoot
            },
            Visibility::Restricted(boundary) => Self::Module(boundary),
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::compiler) struct VisibilityReach(Visibility<DefId>);

pub(super) enum VisibilityAnnotation<'source> {
    Private,
    Public,
    Crate,
    Parent,
    Current,
    InCrate,
    InParent,
    InCurrent,
    InPath {
        source:   &'source str,
        spelling: PathSpelling,
        reach:    VisibilityReach,
    },
}

impl<'source> VisibilityAnnotation<'source> {
    pub(super) fn from_item(
        source: &'source str,
        target: LocalDefId,
        tcx: TyCtxt<'_>,
    ) -> Option<Self> {
        Self::from_source_and_reach(source, tcx.visibility(target.to_def_id()).into())
    }

    pub(super) const fn source(&self) -> &str {
        match self {
            Self::Private => "",
            Self::Public => "pub",
            Self::Crate => "pub(crate)",
            Self::Parent => "pub(super)",
            Self::Current => "pub(self)",
            Self::InCrate => "pub(in crate)",
            Self::InParent => "pub(in super)",
            Self::InCurrent => "pub(in self)",
            Self::InPath { source, .. } => source,
        }
    }

    pub(super) fn display_source(&self) -> String {
        policy::normalized_annotation_source(self.source())
    }

    pub(super) const fn syntax(&self) -> VisibilitySyntax {
        match self {
            Self::Private => VisibilitySyntax::Private,
            Self::Public => VisibilitySyntax::Public,
            Self::Crate => VisibilitySyntax::Crate,
            Self::Parent => VisibilitySyntax::Parent,
            Self::Current => VisibilitySyntax::Current,
            Self::InCrate => VisibilitySyntax::InCrate,
            Self::InParent => VisibilitySyntax::InParent,
            Self::InCurrent => VisibilitySyntax::InCurrent,
            Self::InPath { spelling, .. } => VisibilitySyntax::InPath(*spelling),
        }
    }

    pub(super) fn reach(&self, target: LocalDefId, tcx: TyCtxt<'_>) -> VisibilityReach {
        let current_module = tcx.parent_module_from_def_id(target);
        let visibility = match self {
            Self::Private | Self::Current | Self::InCurrent => {
                Visibility::Restricted(current_module.to_def_id())
            },
            Self::Public => Visibility::Public,
            Self::Crate | Self::InCrate => Visibility::Restricted(CRATE_DEF_ID.to_def_id()),
            Self::Parent | Self::InParent => {
                let parent_module = tcx.parent_module_from_def_id(current_module.into());
                Visibility::Restricted(parent_module.to_def_id())
            },
            Self::InPath { reach, .. } => return *reach,
        };
        visibility.into()
    }

    fn from_source_and_reach(source: &'source str, reach: VisibilityReach) -> Option<Self> {
        if source.is_empty() {
            return Some(Self::Private);
        }

        let visibility = syn::parse_str::<SyntaxVisibility>(source).ok()?;
        match visibility {
            SyntaxVisibility::Inherited => Some(Self::Private),
            SyntaxVisibility::Public(_) => Some(Self::Public),
            SyntaxVisibility::Restricted(restricted) => {
                Self::from_restricted_source(source, &restricted, reach)
            },
        }
    }

    fn from_restricted_source(
        source: &'source str,
        restricted: &VisRestricted,
        reach: VisibilityReach,
    ) -> Option<Self> {
        let first_segment = restricted.path.segments.first()?.ident.to_string();
        let segment_count = restricted.path.segments.len();
        match (
            restricted.in_token.is_some(),
            segment_count,
            first_segment.as_str(),
        ) {
            (false, 1, "crate") => Some(Self::Crate),
            (false, 1, "super") => Some(Self::Parent),
            (false, 1, "self") => Some(Self::Current),
            (true, 1, "crate") => Some(Self::InCrate),
            (true, 1, "super") => Some(Self::InParent),
            (true, 1, "self") => Some(Self::InCurrent),
            (true, _, "crate") => Some(Self::InPath {
                source,
                spelling: PathSpelling::CrateRooted,
                reach,
            }),
            (true, _, _) => Some(Self::InPath {
                source,
                spelling: PathSpelling::Relative,
                reach,
            }),
            (false, _, _) => None,
        }
    }
}

impl From<Visibility<DefId>> for VisibilityReach {
    fn from(visibility: Visibility<DefId>) -> Self { Self(visibility) }
}

impl From<Visibility<ModId>> for VisibilityReach {
    fn from(visibility: Visibility<ModId>) -> Self { Self(visibility.map_id(ModId::to_def_id)) }
}

impl From<ScopeReach<DefId>> for VisibilityReach {
    fn from(reach: ScopeReach<DefId>) -> Self {
        match reach {
            ScopeReach::Public => Self(Visibility::Public),
            ScopeReach::Restricted(boundary) => Self(Visibility::Restricted(boundary)),
        }
    }
}

impl VisibilityReach {
    pub(in crate::compiler) fn compare(self, other: Self, tcx: TyCtxt<'_>) -> Option<Ordering> {
        let mut dominance = |lhs, rhs| Self::from(lhs).dominance(Self::from(rhs), tcx);
        self.reach().compare(other.reach(), &mut dominance)
    }

    pub(in crate::compiler) fn join(self, other: Self, tcx: TyCtxt<'_>) -> Self {
        let mut dominance = |lhs, rhs| Self::from(lhs).dominance(Self::from(rhs), tcx);
        let mut parent_module = |module: DefId| {
            module.as_local().map_or(module, |local_module| {
                tcx.parent_module_from_def_id(local_module).to_def_id()
            })
        };
        Self::from(
            self.reach()
                .join(other.reach(), &mut dominance, &mut parent_module),
        )
    }

    /// Ported from rustc's `Visibility::is_at_least`, removed in 1.97.
    /// Public reaches require public visibility; restricted reaches require accessibility
    /// from their boundary. Keep rustc's `is_public` / `is_accessible_from` semantics.
    pub(super) fn dominance(self, other: Self, tcx: TyCtxt<'_>) -> ReachDominance {
        let accessible = match other.0 {
            Visibility::Public => self.0.is_public(),
            Visibility::Restricted(boundary) => self.0.is_accessible_from(boundary, tcx),
        };
        if accessible {
            ReachDominance::Dominates
        } else {
            ReachDominance::DoesNotDominate
        }
    }

    pub(super) fn widening(self, other: Self, tcx: TyCtxt<'_>) -> ReachWidening {
        let mut dominance = |lhs, rhs| Self::from(lhs).dominance(Self::from(rhs), tcx);
        self.reach().widening(other.reach(), &mut dominance)
    }

    pub(super) const fn audience(self) -> VisibilityAudience { self.reach().audience() }

    pub(super) fn boundary(self) -> ReachBoundary { ReachBoundary::from(self) }

    pub(super) fn to_source(self, tcx: TyCtxt<'_>) -> String {
        match self.boundary() {
            ReachBoundary::Everywhere => String::from("pub"),
            ReachBoundary::CrateRoot => String::from("pub(crate)"),
            ReachBoundary::Module(boundary) => {
                format!("pub(in crate::{})", tcx.def_path_str(boundary))
            },
        }
    }

    const fn reach(self) -> ScopeReach<DefId> {
        match self.0 {
            Visibility::Public => ScopeReach::Public,
            Visibility::Restricted(boundary) => ScopeReach::Restricted(boundary),
        }
    }
}

pub(in crate::compiler) fn anchored(
    reach: VisibilityReach,
    target: LocalDefId,
    tcx: TyCtxt<'_>,
) -> VisibilityReach {
    let target_module = tcx.parent_module_from_def_id(target).to_def_id();
    let mut dominance =
        |lhs, rhs| VisibilityReach::from(lhs).dominance(VisibilityReach::from(rhs), tcx);
    let mut parent_module = |module: DefId| {
        module.as_local().map_or(module, |local_module| {
            tcx.parent_module_from_def_id(local_module).to_def_id()
        })
    };
    reach
        .reach()
        .anchored(target_module, &mut dominance, &mut parent_module)
        .into()
}

pub(in crate::compiler) fn capped_by_enclosing_modules(
    mut reach: VisibilityReach,
    declaration: LocalDefId,
    tcx: TyCtxt<'_>,
) -> Option<VisibilityReach> {
    let mut module = tcx.parent_module_from_def_id(declaration).to_local_def_id();
    while module != CRATE_DEF_ID {
        let module_reach = VisibilityReach::from(tcx.visibility(module.to_def_id()));
        reach = match reach.compare(module_reach, tcx) {
            Some(Ordering::Equal | Ordering::Less) => reach,
            Some(Ordering::Greater) => module_reach,
            None => return None,
        };
        module = tcx.parent_module_from_def_id(module).to_local_def_id();
    }
    Some(reach)
}

/// Directional accessibility used to compare reaches and find a shared ancestor.
/// Preserves rustc's removed `Visibility::is_at_least`: public reaches require public
/// visibility, while restricted reaches require accessibility from their boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReachDominance {
    Dominates,
    DoesNotDominate,
}

/// Strict widening requires forward dominance and excludes equal or incomparable reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReachWidening {
    StrictlyWider,
    NotWider,
}

/// Only public visibility reaches callers outside the crate; restricted scopes stay inside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VisibilityAudience {
    Public,
    Restricted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeReach<Scope> {
    Public,
    Restricted(Scope),
}

impl<Scope: Copy + Eq> ScopeReach<Scope> {
    const fn audience(self) -> VisibilityAudience {
        match self {
            Self::Public => VisibilityAudience::Public,
            Self::Restricted(_) => VisibilityAudience::Restricted,
        }
    }

    fn widening(
        self,
        other: Self,
        dominance: &mut impl FnMut(Self, Self) -> ReachDominance,
    ) -> ReachWidening {
        match dominance(self, other) {
            ReachDominance::DoesNotDominate => ReachWidening::NotWider,
            ReachDominance::Dominates => match dominance(other, self) {
                ReachDominance::Dominates => ReachWidening::NotWider,
                ReachDominance::DoesNotDominate => ReachWidening::StrictlyWider,
            },
        }
    }

    fn compare(
        self,
        other: Self,
        dominance: &mut impl FnMut(Self, Self) -> ReachDominance,
    ) -> Option<Ordering> {
        match (dominance(self, other), dominance(other, self)) {
            (ReachDominance::Dominates, ReachDominance::Dominates) => Some(Ordering::Equal),
            (ReachDominance::Dominates, ReachDominance::DoesNotDominate) => Some(Ordering::Greater),
            (ReachDominance::DoesNotDominate, ReachDominance::Dominates) => Some(Ordering::Less),
            (ReachDominance::DoesNotDominate, ReachDominance::DoesNotDominate) => None,
        }
    }

    fn join(
        self,
        other: Self,
        dominance: &mut impl FnMut(Self, Self) -> ReachDominance,
        parent_module: &mut impl FnMut(Scope) -> Scope,
    ) -> Self {
        match self.compare(other, dominance) {
            Some(Ordering::Equal | Ordering::Greater) => self,
            Some(Ordering::Less) => other,
            None => match (self, other) {
                (Self::Restricted(lhs), Self::Restricted(rhs)) => {
                    let mut common_ancestor = lhs;
                    loop {
                        let candidate = Self::Restricted(common_ancestor);
                        if dominance(candidate, Self::Restricted(rhs)) == ReachDominance::Dominates
                        {
                            return candidate;
                        }
                        let parent = parent_module(common_ancestor);
                        if parent == common_ancestor {
                            return Self::Public;
                        }
                        common_ancestor = parent;
                    }
                },
                (Self::Public, Self::Public | Self::Restricted(_))
                | (Self::Restricted(_), Self::Public) => Self::Public,
            },
        }
    }

    fn anchored(
        self,
        target_module: Scope,
        dominance: &mut impl FnMut(Self, Self) -> ReachDominance,
        parent_module: &mut impl FnMut(Scope) -> Scope,
    ) -> Self {
        self.join(Self::Restricted(target_module), dominance, parent_module)
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use ReachDominance::DoesNotDominate;
    use ReachDominance::Dominates;
    use rustc_middle::ty::Visibility;
    use rustc_span::def_id::CRATE_DEF_ID;
    use rustc_span::def_id::DefId;
    use rustc_span::def_id::DefIndex;
    use rustc_span::def_id::LOCAL_CRATE;

    use super::PathSpelling;
    use super::ReachBoundary;
    use super::ReachDominance;
    use super::ReachWidening;
    use super::ScopeReach;
    use super::VisibilityAnnotation;
    use super::VisibilityAudience;
    use super::VisibilityReach;
    use super::VisibilitySyntax;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TestModule {
        Crate,
        Ancestor,
        Left,
        Right,
        Isolated,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum ModuleAncestry {
        Descendant,
        Outside,
    }

    impl TestModule {
        const fn parent(self) -> Self {
            match self {
                Self::Crate | Self::Ancestor => Self::Crate,
                Self::Left | Self::Right => Self::Ancestor,
                Self::Isolated => Self::Isolated,
            }
        }

        fn ancestry(self, ancestor: Self) -> ModuleAncestry {
            let mut current = self;
            loop {
                if current == ancestor {
                    return ModuleAncestry::Descendant;
                }
                if current == Self::Crate {
                    return ModuleAncestry::Outside;
                }
                let parent = current.parent();
                if parent == current {
                    return ModuleAncestry::Outside;
                }
                current = parent;
            }
        }
    }

    type TestReach = ScopeReach<TestModule>;

    #[test]
    fn directional_dominance_preserves_each_partial_order_outcome() {
        for (forward, reverse, expected) in [
            (
                ReachDominance::Dominates,
                ReachDominance::Dominates,
                Some(Ordering::Equal),
            ),
            (
                ReachDominance::Dominates,
                ReachDominance::DoesNotDominate,
                Some(Ordering::Greater),
            ),
            (
                ReachDominance::DoesNotDominate,
                ReachDominance::Dominates,
                Some(Ordering::Less),
            ),
            (
                ReachDominance::DoesNotDominate,
                ReachDominance::DoesNotDominate,
                None,
            ),
        ] {
            let mut calls = Vec::new();
            let mut dominance = |lhs, rhs| {
                calls.push((lhs, rhs));
                if calls.len() == 1 { forward } else { reverse }
            };
            let left = TestReach::Restricted(TestModule::Left);
            let right = TestReach::Restricted(TestModule::Right);
            assert_eq!(left.compare(right, &mut dominance), expected);
            assert_eq!(calls, [(left, right), (right, left)]);
        }
    }

    #[test]
    fn strict_widening_preserves_short_circuit_and_comparison_direction() {
        for (forward, reverse, expected, count) in [
            (Dominates, Dominates, ReachWidening::NotWider, 2),
            (Dominates, DoesNotDominate, ReachWidening::StrictlyWider, 2),
            (DoesNotDominate, Dominates, ReachWidening::NotWider, 1),
            (DoesNotDominate, DoesNotDominate, ReachWidening::NotWider, 1),
        ] {
            let left = TestReach::Restricted(TestModule::Left);
            let right = TestReach::Restricted(TestModule::Right);
            let mut calls = Vec::new();
            let mut dominance = |lhs, rhs| {
                calls.push((lhs, rhs));
                if calls.len() == 1 { forward } else { reverse }
            };
            assert_eq!(left.widening(right, &mut dominance), expected);
            assert_eq!(calls.len(), count);
            assert_eq!(calls[0], (left, right));
            if count == 2 {
                assert_eq!(calls[1], (right, left));
            }
        }
    }

    #[test]
    fn audience_keeps_crate_and_module_restrictions_inside_the_crate() {
        for (visibility, expected) in [
            (Visibility::Public, VisibilityAudience::Public),
            (
                Visibility::Restricted(CRATE_DEF_ID.to_def_id()),
                VisibilityAudience::Restricted,
            ),
            (
                Visibility::Restricted(DefId {
                    krate: LOCAL_CRATE,
                    index: DefIndex::from_u32(1),
                }),
                VisibilityAudience::Restricted,
            ),
        ] {
            assert_eq!(VisibilityReach::from(visibility).audience(), expected);
        }
    }

    #[test]
    fn module_ancestry_preserves_self_root_siblings_and_fixed_points() {
        for (module, ancestor, expected) in [
            (
                TestModule::Left,
                TestModule::Left,
                ModuleAncestry::Descendant,
            ),
            (
                TestModule::Left,
                TestModule::Ancestor,
                ModuleAncestry::Descendant,
            ),
            (
                TestModule::Left,
                TestModule::Crate,
                ModuleAncestry::Descendant,
            ),
            (
                TestModule::Crate,
                TestModule::Crate,
                ModuleAncestry::Descendant,
            ),
            (TestModule::Crate, TestModule::Left, ModuleAncestry::Outside),
            (TestModule::Left, TestModule::Right, ModuleAncestry::Outside),
            (
                TestModule::Isolated,
                TestModule::Left,
                ModuleAncestry::Outside,
            ),
            (
                TestModule::Isolated,
                TestModule::Isolated,
                ModuleAncestry::Descendant,
            ),
        ] {
            assert_eq!(module.ancestry(ancestor), expected);
        }
    }

    #[test]
    fn resolved_boundaries_preserve_module_identity_and_policy_kind() {
        let public = VisibilityReach::from(Visibility::<DefId>::Public);
        let root = VisibilityReach::from(Visibility::Restricted(CRATE_DEF_ID.to_def_id()));
        let module_id = DefId {
            krate: LOCAL_CRATE,
            index: DefIndex::from_u32(1),
        };
        let module = VisibilityReach::from(Visibility::Restricted(module_id));

        assert_eq!(public.boundary(), ReachBoundary::Everywhere);
        assert_eq!(root.boundary(), ReachBoundary::CrateRoot);
        assert_eq!(module.boundary(), ReachBoundary::Module(module_id));
    }

    #[test]
    fn classifies_visibility_annotation_variants() {
        let reach = VisibilityReach::from(Visibility::<DefId>::Public);

        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("", reach),
            Some(VisibilityAnnotation::Private)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub", reach),
            Some(VisibilityAnnotation::Public)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(crate)", reach),
            Some(VisibilityAnnotation::Crate)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(in crate)", reach),
            Some(VisibilityAnnotation::InCrate)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(in super)", reach),
            Some(VisibilityAnnotation::InParent)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(in self)", reach),
            Some(VisibilityAnnotation::InCurrent)
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(in crate::a::b)", reach),
            Some(VisibilityAnnotation::InPath {
                spelling: PathSpelling::CrateRooted,
                ..
            })
        ));
        assert!(matches!(
            VisibilityAnnotation::from_source_and_reach("pub(in super::super)", reach),
            Some(VisibilityAnnotation::InPath {
                spelling: PathSpelling::Relative,
                ..
            })
        ));
    }

    #[test]
    fn reports_written_syntax_without_reclassifying_path_annotations() {
        let reach = VisibilityReach::from(Visibility::<DefId>::Public);
        let annotation = VisibilityAnnotation::from_source_and_reach("pub(in crate::a)", reach);

        assert!(annotation.is_some_and(|annotation| {
            annotation.source() == "pub(in crate::a)"
                && annotation.syntax() == VisibilitySyntax::InPath(PathSpelling::CrateRooted)
        }));
    }

    #[test]
    fn compares_equal_ancestral_and_sibling_reaches() {
        assert_eq!(
            compare(
                TestReach::Restricted(TestModule::Left),
                TestReach::Restricted(TestModule::Left),
            ),
            Some(Ordering::Equal)
        );
        assert_eq!(
            compare(
                TestReach::Restricted(TestModule::Crate),
                TestReach::Restricted(TestModule::Left),
            ),
            Some(Ordering::Greater)
        );
        assert_eq!(
            compare(
                TestReach::Restricted(TestModule::Left),
                TestReach::Restricted(TestModule::Crate),
            ),
            Some(Ordering::Less)
        );
        assert_eq!(
            compare(
                TestReach::Restricted(TestModule::Left),
                TestReach::Restricted(TestModule::Right),
            ),
            None
        );
    }

    #[test]
    fn joins_to_the_wider_reach_or_common_ancestor() {
        assert_eq!(
            join(
                TestReach::Restricted(TestModule::Crate),
                TestReach::Restricted(TestModule::Left),
            ),
            TestReach::Restricted(TestModule::Crate)
        );
        assert_eq!(
            join(
                TestReach::Restricted(TestModule::Left),
                TestReach::Restricted(TestModule::Crate),
            ),
            TestReach::Restricted(TestModule::Crate)
        );
        assert_eq!(
            join(
                TestReach::Restricted(TestModule::Left),
                TestReach::Restricted(TestModule::Right),
            ),
            TestReach::Restricted(TestModule::Ancestor)
        );
        assert_eq!(
            join(TestReach::Public, TestReach::Restricted(TestModule::Left)),
            TestReach::Public
        );
        assert_eq!(
            join(TestReach::Restricted(TestModule::Left), TestReach::Public),
            TestReach::Public
        );
    }

    #[test]
    fn joins_to_public_when_parent_module_reaches_a_fixed_point() {
        assert_eq!(
            join(
                TestReach::Restricted(TestModule::Isolated),
                TestReach::Restricted(TestModule::Left),
            ),
            TestReach::Public
        );
    }

    #[test]
    fn anchors_sibling_reaches_to_the_declaration_ancestor() {
        let anchored =
            anchored_to_module(TestReach::Restricted(TestModule::Right), TestModule::Left);

        assert_eq!(anchored, TestReach::Restricted(TestModule::Ancestor));
        assert_ne!(anchored, TestReach::Restricted(TestModule::Right));
    }

    fn compare(lhs: TestReach, rhs: TestReach) -> Option<Ordering> {
        let mut dominance = test_dominance;
        lhs.compare(rhs, &mut dominance)
    }

    fn join(lhs: TestReach, rhs: TestReach) -> TestReach {
        let mut dominance = test_dominance;
        let mut parent_module = TestModule::parent;
        lhs.join(rhs, &mut dominance, &mut parent_module)
    }

    fn anchored_to_module(reach: TestReach, target_module: TestModule) -> TestReach {
        let mut dominance = test_dominance;
        let mut parent_module = TestModule::parent;
        reach.anchored(target_module, &mut dominance, &mut parent_module)
    }

    fn test_dominance(lhs: TestReach, rhs: TestReach) -> ReachDominance {
        match (lhs, rhs) {
            (TestReach::Public, TestReach::Public | TestReach::Restricted(_)) => {
                ReachDominance::Dominates
            },
            (TestReach::Restricted(_), TestReach::Public) => ReachDominance::DoesNotDominate,
            (TestReach::Restricted(reach), TestReach::Restricted(boundary)) => {
                match boundary.ancestry(reach) {
                    ModuleAncestry::Descendant => ReachDominance::Dominates,
                    ModuleAncestry::Outside => ReachDominance::DoesNotDominate,
                }
            },
        }
    }
}
