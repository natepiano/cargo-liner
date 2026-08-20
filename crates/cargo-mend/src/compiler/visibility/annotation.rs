use std::cmp::Ordering;

use rustc_middle::ty::TyCtxt;
use rustc_middle::ty::Visibility;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::DefId;
use rustc_span::def_id::LocalDefId;
use syn::VisRestricted;
use syn::Visibility as SyntaxVisibility;

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

/// How far a reach extends, independent of how any annotation spells it. The
/// three variants are exactly the three forms `VisibilityReach::to_source`
/// renders, so policy can ask the question without reading rendered text back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReachBoundary {
    /// Reachable from other crates — `pub`.
    Everywhere,
    /// Capped at the crate root — `pub(crate)`.
    CrateRoot,
    /// Capped at a module below the crate root — `pub(in crate::…)`.
    Module,
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
        super::policy::normalized_annotation_source(self.source())
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
        let mut is_at_least = |lhs, rhs| Self::from(lhs).is_at_least(Self::from(rhs), tcx);
        self.reach().compare(other.reach(), &mut is_at_least)
    }

    pub(in crate::compiler) fn join(self, other: Self, tcx: TyCtxt<'_>) -> Self {
        let mut is_at_least = |lhs, rhs| Self::from(lhs).is_at_least(Self::from(rhs), tcx);
        let mut parent_module = |module: DefId| {
            module.as_local().map_or(module, |local_module| {
                tcx.parent_module_from_def_id(local_module).to_def_id()
            })
        };
        Self::from(
            self.reach()
                .join(other.reach(), &mut is_at_least, &mut parent_module),
        )
    }

    /// Ported from rustc's `Visibility::is_at_least`, removed in 1.97. Returns
    /// `true` when `self` is at least as visible as `other`. Body is a verbatim
    /// copy of the removed method, expressed in terms of the still-present
    /// `is_public` / `is_accessible_from`.
    pub(super) fn is_at_least(self, other: Self, tcx: TyCtxt<'_>) -> bool {
        match other.0 {
            Visibility::Public => self.0.is_public(),
            Visibility::Restricted(boundary) => self.0.is_accessible_from(boundary, tcx),
        }
    }

    pub(super) fn is_strictly_wider(self, other: Self, tcx: TyCtxt<'_>) -> bool {
        self.is_at_least(other, tcx) && !other.is_at_least(self, tcx)
    }

    pub(super) fn is_public(self) -> bool { self.0.is_public() }

    pub(super) fn boundary(self) -> ReachBoundary {
        match self.0 {
            Visibility::Public => ReachBoundary::Everywhere,
            Visibility::Restricted(boundary) if boundary == CRATE_DEF_ID.to_def_id() => {
                ReachBoundary::CrateRoot
            },
            Visibility::Restricted(_) => ReachBoundary::Module,
        }
    }

    pub(super) fn to_source(self, tcx: TyCtxt<'_>) -> String {
        match self.0 {
            Visibility::Public => String::from("pub"),
            Visibility::Restricted(boundary) if boundary == CRATE_DEF_ID.to_def_id() => {
                String::from("pub(crate)")
            },
            Visibility::Restricted(boundary) => {
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
    let mut is_at_least =
        |lhs, rhs| VisibilityReach::from(lhs).is_at_least(VisibilityReach::from(rhs), tcx);
    let mut parent_module = |module: DefId| {
        module.as_local().map_or(module, |local_module| {
            tcx.parent_module_from_def_id(local_module).to_def_id()
        })
    };
    reach
        .reach()
        .anchored(target_module, &mut is_at_least, &mut parent_module)
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScopeReach<Scope> {
    Public,
    Restricted(Scope),
}

impl<Scope: Copy + Eq> ScopeReach<Scope> {
    fn compare(
        self,
        other: Self,
        is_at_least: &mut impl FnMut(Self, Self) -> bool,
    ) -> Option<Ordering> {
        match (is_at_least(self, other), is_at_least(other, self)) {
            (true, true) => Some(Ordering::Equal),
            (true, false) => Some(Ordering::Greater),
            (false, true) => Some(Ordering::Less),
            (false, false) => None,
        }
    }

    fn join(
        self,
        other: Self,
        is_at_least: &mut impl FnMut(Self, Self) -> bool,
        parent_module: &mut impl FnMut(Scope) -> Scope,
    ) -> Self {
        match self.compare(other, is_at_least) {
            Some(Ordering::Equal | Ordering::Greater) => self,
            Some(Ordering::Less) => other,
            None => match (self, other) {
                (Self::Restricted(lhs), Self::Restricted(rhs)) => {
                    let mut common_ancestor = lhs;
                    loop {
                        let candidate = Self::Restricted(common_ancestor);
                        if is_at_least(candidate, Self::Restricted(rhs)) {
                            return candidate;
                        }
                        let parent = parent_module(common_ancestor);
                        if parent == common_ancestor {
                            return Self::Public;
                        }
                        common_ancestor = parent;
                    }
                },
                (Self::Public, _) | (_, Self::Public) => Self::Public,
            },
        }
    }

    fn anchored(
        self,
        target_module: Scope,
        is_at_least: &mut impl FnMut(Self, Self) -> bool,
        parent_module: &mut impl FnMut(Scope) -> Scope,
    ) -> Self {
        self.join(Self::Restricted(target_module), is_at_least, parent_module)
    }
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;

    use rustc_middle::ty::Visibility;

    use super::PathSpelling;
    use super::ScopeReach;
    use super::VisibilityAnnotation;
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

    impl TestModule {
        const fn parent(self) -> Self {
            match self {
                Self::Crate | Self::Ancestor => Self::Crate,
                Self::Left | Self::Right => Self::Ancestor,
                Self::Isolated => Self::Isolated,
            }
        }

        fn is_descendant_of(self, ancestor: Self) -> bool {
            let mut current = self;
            loop {
                if current == ancestor {
                    return true;
                }
                if current == Self::Crate {
                    return false;
                }
                let parent = current.parent();
                if parent == current {
                    return false;
                }
                current = parent;
            }
        }
    }

    type TestReach = ScopeReach<TestModule>;

    #[test]
    fn classifies_visibility_annotation_variants() {
        let reach = VisibilityReach::from(Visibility::Public);

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
        let reach = VisibilityReach::from(Visibility::Public);
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
        let mut is_at_least = test_is_at_least;
        lhs.compare(rhs, &mut is_at_least)
    }

    fn join(lhs: TestReach, rhs: TestReach) -> TestReach {
        let mut is_at_least = test_is_at_least;
        let mut parent_module = TestModule::parent;
        lhs.join(rhs, &mut is_at_least, &mut parent_module)
    }

    fn anchored_to_module(reach: TestReach, target_module: TestModule) -> TestReach {
        let mut is_at_least = test_is_at_least;
        let mut parent_module = TestModule::parent;
        reach.anchored(target_module, &mut is_at_least, &mut parent_module)
    }

    fn test_is_at_least(lhs: TestReach, rhs: TestReach) -> bool {
        match (lhs, rhs) {
            (TestReach::Public, _) => true,
            (TestReach::Restricted(_), TestReach::Public) => false,
            (TestReach::Restricted(reach), TestReach::Restricted(boundary)) => {
                boundary.is_descendant_of(reach)
            },
        }
    }
}
