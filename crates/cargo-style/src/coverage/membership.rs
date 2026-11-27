//! Derive the cells and their default judgment scopes from synced membership.

use std::collections::BTreeSet;

use super::catalog::CoverageRule;
use crate::constants::UNCOVERED;
use crate::site_ledger::InputScope;
use crate::sites::Reach;
use crate::sites::RuleInputScope;
use crate::sites::SiteMembership;
use crate::sites::Tier;
use crate::tip_view;
use crate::tip_view::RepositoryTip;

pub(super) enum CoverageComponent<'a> {
    Base { id: &'a str, tier: Tier },
    Uncovered(String),
}

impl<'a> CoverageComponent<'a> {
    pub(super) const fn base(id: &'a str, rule: &CoverageRule) -> Self {
        Self::Base {
            id,
            tier: rule.policy.tier,
        }
    }

    pub(super) fn uncovered(id: &str) -> Self { Self::Uncovered(format!("{id}{UNCOVERED}")) }

    pub(super) fn id(&self) -> &str {
        match self {
            Self::Base { id, .. } => id,
            Self::Uncovered(id) => id,
        }
    }

    pub(super) const fn tier(&self) -> Tier {
        match self {
            Self::Base { tier, .. } => *tier,
            Self::Uncovered(_) => Tier::C,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct CoveredPath<'a> {
    pub(super) path:   &'a str,
    pub(super) member: &'a str,
}

impl CoveredPath<'_> {
    pub(super) fn judgment_scope(&self, tier: Tier, rule_scope: RuleInputScope) -> InputScope {
        let rust_path = tip_view::is_rust_path(self.path);
        match (tier, rust_path, self.path == self.member, rule_scope) {
            (Tier::B, false, _, _) => InputScope::File,
            (Tier::D, true, false, RuleInputScope::File | RuleInputScope::Crate) => {
                InputScope::Module
            },
            (Tier::D, _, true, RuleInputScope::File) => InputScope::Crate,
            _ => rule_scope.into(),
        }
    }
}

pub(super) struct CellMembership<'a> {
    pub(super) tip:   &'a RepositoryTip,
    pub(super) sites: &'a SiteMembership,
}

impl<'a> CellMembership<'a> {
    pub(super) fn paths(
        &self,
        component: &CoverageComponent<'_>,
        reach: &Reach,
        scope: RuleInputScope,
    ) -> Vec<CoveredPath<'a>> {
        let rust = self.tip.rust_paths().filter(|path| reach.includes(path));
        let sites = self.sites.paths(component.id());
        let paths: Vec<_> = match (component.tier(), scope) {
            (Tier::A | Tier::C, _) => rust.collect(),
            (Tier::B, _) => sites.collect(),
            (Tier::D, RuleInputScope::Manifest) => {
                return rust
                    .chain(sites)
                    .map(|path| self.tip.crate_of(path))
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .map(|member| CoveredPath {
                        path: member,
                        member,
                    })
                    .collect();
            },
            (Tier::D, _) => rust
                .chain(sites)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            (Tier::E | Tier::Unclassified, _) => Vec::new(),
        };
        paths
            .into_iter()
            .map(|path| CoveredPath {
                path,
                member: self.tip.crate_of(path),
            })
            .collect()
    }
}
