//! Minimal-antichain reduction for declared reservation scopes.

use std::collections::HashSet;

use super::PathCase;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ScopeKind;

/// Drop every scope another scope contains, keeping the first of equal scopes, in input order.
///
/// A scope goes when a tree sits on a strict ancestor of its path, when it is a file and a tree
/// has its path, or when an earlier scope has its path and kind. Each path becomes a key of
/// case-folded components once, and each scope is decided by looking up its own key and its
/// ancestor prefixes, so no pair of scopes is ever compared.
pub(super) fn reduce(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet {
    let keys = scopes
        .as_slice()
        .iter()
        .map(|scope| component_keys(scope, path_case))
        .collect::<Vec<_>>();
    let tree_keys = scopes
        .as_slice()
        .iter()
        .zip(&keys)
        .filter(|(scope, _)| scope.kind == ScopeKind::Tree)
        .map(|(_, key)| key.as_slice())
        .collect::<HashSet<_>>();
    let mut seen_files = HashSet::new();
    let mut seen_trees = HashSet::new();
    let mut reduced = Vec::new();
    for (scope, key) in scopes.as_slice().iter().zip(&keys) {
        let first_of_its_kind = match scope.kind {
            ScopeKind::File => seen_files.insert(key.as_slice()),
            ScopeKind::Tree => seen_trees.insert(key.as_slice()),
        };
        let file_under_same_path_tree =
            scope.kind == ScopeKind::File && tree_keys.contains(key.as_slice());
        let under_ancestor_tree = (1..key.len()).any(|end| tree_keys.contains(&key[..end]));
        if first_of_its_kind && !file_under_same_path_tree && !under_ancestor_tree {
            reduced.push(scope.clone());
        }
    }
    ReservationScopeSet::try_from(reduced).unwrap_or(scopes)
}

pub(super) fn component_keys(scope: &ReservationScope, path_case: PathCase) -> Vec<String> {
    scope
        .path
        .to_string()
        .split('/')
        .map(|component| path_case.component_key(component))
        .collect()
}

pub(super) fn overlaps(
    left: &ReservationScope,
    right: &ReservationScope,
    path_case: PathCase,
) -> bool {
    contains(left, right, path_case) || contains(right, left, path_case)
}

pub(super) fn contains(
    holder: &ReservationScope,
    candidate: &ReservationScope,
    path_case: PathCase,
) -> bool {
    let candidate_path = candidate.path.to_string();
    (holder.kind == ScopeKind::Tree || candidate.kind == ScopeKind::File)
        && holder.covers_path_by(
            candidate_path.as_bytes(),
            |holder_component, candidate_component| {
                let (Ok(holder_component), Ok(candidate_component)) = (
                    str::from_utf8(holder_component),
                    str::from_utf8(candidate_component),
                ) else {
                    return false;
                };
                path_case.component_eq(holder_component, candidate_component)
            },
        )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::path::PathBuf;
    use std::str::FromStr;
    use std::time::Duration;
    use std::time::Instant;

    use super::PathCase;
    use super::ReservationScope;
    use super::ReservationScopeSet;
    use super::ScopeKind;
    use super::contains;
    use super::reduce;
    use crate::ids::ReservationScopePath;
    use crate::scope::DeclaredReservationScopeSet;

    /// The pairwise reduction `reduce` replaced, kept as the reference its answers must match.
    fn reduce_by_pairs(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet {
        let mut reduced = Vec::new();
        for (candidate_index, candidate) in scopes.as_slice().iter().enumerate() {
            let contained = scopes
                .as_slice()
                .iter()
                .enumerate()
                .any(|(holder_index, holder)| {
                    candidate_index != holder_index
                        && contains(holder, candidate, path_case)
                        && (!contains(candidate, holder, path_case)
                            || holder_index < candidate_index)
                });
            if !contained {
                reduced.push(candidate.clone());
            }
        }
        ReservationScopeSet::try_from(reduced).unwrap_or(scopes)
    }

    fn scope(kind: ScopeKind, path: &str) -> ReservationScope {
        ReservationScope {
            path: ReservationScopePath::from_str(path).expect("test path should be valid"),
            kind,
        }
    }

    fn scope_set(scopes: Vec<ReservationScope>) -> ReservationScopeSet {
        ReservationScopeSet::try_from(scopes).expect("test scopes should be non-empty")
    }

    /// Nested trees, a tree and a file on one path, repeats, siblings, and case-only differences.
    fn equivalence_pool() -> Vec<ReservationScope> {
        vec![
            scope(ScopeKind::Tree, "a"),
            scope(ScopeKind::File, "a"),
            scope(ScopeKind::Tree, "a/b"),
            scope(ScopeKind::File, "a/b"),
            scope(ScopeKind::Tree, "A/b"),
            scope(ScopeKind::File, "a/B"),
            scope(ScopeKind::File, "a/b/c"),
            scope(ScopeKind::Tree, "a/b/c"),
            scope(ScopeKind::File, "a/x"),
            scope(ScopeKind::Tree, "ab"),
            scope(ScopeKind::File, "A"),
            scope(ScopeKind::Tree, "a/B"),
        ]
    }

    fn assert_matches_pairwise(scopes: &[ReservationScope]) {
        for path_case in [PathCase::Sensitive, PathCase::Insensitive] {
            let input = scope_set(scopes.to_vec());
            assert_eq!(
                reduce(input.clone(), path_case),
                reduce_by_pairs(input, path_case),
                "{path_case:?} reduction of {scopes:?}"
            );
        }
    }

    #[test]
    fn reduction_matches_the_pairwise_reference_on_every_short_ordering() {
        let pool = equivalence_pool();
        for first in &pool {
            assert_matches_pairwise(std::slice::from_ref(first));
            for second in &pool {
                assert_matches_pairwise(&[first.clone(), second.clone()]);
                for third in &pool {
                    assert_matches_pairwise(&[first.clone(), second.clone(), third.clone()]);
                }
            }
        }
    }

    #[test]
    fn reduction_matches_the_pairwise_reference_on_long_mixed_lists() {
        let pool = equivalence_pool();
        let mut state = 0x9e37_79b9_u32;
        for _ in 0..300 {
            let scopes = (0..9)
                .map(|_| {
                    state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    pool[usize::try_from(state >> 16).expect("a u32 fits in usize") % pool.len()]
                        .clone()
                })
                .collect::<Vec<_>>();
            assert_matches_pairwise(&scopes);
        }
    }

    /// One worktree's 13 blocking reservations, each protecting the same 364 files: the shape
    /// that held the post-tool-use hook in a spin for 600 seconds.
    #[test]
    fn reduction_of_the_reported_shape_is_immediate() {
        let distinct = (0..28)
            .flat_map(|module| {
                (0..13).map(move |part| {
                    scope(
                        ScopeKind::File,
                        &format!("crates/hana/src/geometry/material_{module}/part_{part}.rs"),
                    )
                })
            })
            .collect::<Vec<_>>();
        assert_eq!(distinct.len(), 364);
        let joined = scope_set(
            std::iter::repeat_n(distinct.iter().cloned(), 13)
                .flatten()
                .collect(),
        );
        assert_eq!(joined.as_slice().len(), 4_732);

        let started = Instant::now();
        let reduced = reduce(joined, PathCase::Insensitive);
        let elapsed = started.elapsed();

        assert_eq!(reduced.as_slice(), distinct.as_slice());
        assert!(
            elapsed < Duration::from_millis(100),
            "reducing 4,732 scopes took {elapsed:?}"
        );
    }

    #[test]
    fn reduction_keeps_only_minimal_containing_scopes() {
        let declared = DeclaredReservationScopeSet::parse(
            vec![
                PathBuf::from("tree:crates/hana"),
                PathBuf::from("file:crates/hana/src/lib.rs"),
                PathBuf::from("tree:crates/hana/src"),
                PathBuf::from("file:Cargo.toml"),
            ],
            ScopeKind::Tree,
        )
        .expect("scopes should parse");
        let scopes = declared.into_minimal_antichain(PathCase::Sensitive);

        assert_eq!(scopes.as_slice().len(), 2);
        assert_eq!(scopes.as_slice()[0].path.to_string(), "crates/hana");
        assert_eq!(scopes.as_slice()[1].path.to_string(), "Cargo.toml");
    }

    #[test]
    fn same_path_tree_contains_same_path_file() {
        let declared = DeclaredReservationScopeSet::parse(
            vec![PathBuf::from("file:target"), PathBuf::from("tree:target")],
            ScopeKind::File,
        )
        .expect("scopes should parse");
        let scopes = reduce(declared.0, PathCase::Sensitive);

        assert_eq!(scopes.as_slice().len(), 1);
        assert_eq!(scopes.as_slice()[0].kind, ScopeKind::Tree);
    }
}
