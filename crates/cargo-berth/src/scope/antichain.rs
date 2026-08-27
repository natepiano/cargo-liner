//! Minimal-antichain reduction for declared reservation scopes.

use super::PathCase;
use super::ReservationScope;
use super::ReservationScopeSet;
use super::ScopeKind;

pub(super) fn reduce(scopes: ReservationScopeSet, path_case: PathCase) -> ReservationScopeSet {
    let mut reduced = Vec::new();
    for (candidate_index, candidate) in scopes.as_slice().iter().enumerate() {
        let contained = scopes
            .as_slice()
            .iter()
            .enumerate()
            .any(|(holder_index, holder)| {
                candidate_index != holder_index
                    && contains(holder, candidate, path_case)
                    && (!contains(candidate, holder, path_case) || holder_index < candidate_index)
            });
        if !contained {
            reduced.push(candidate.clone());
        }
    }
    ReservationScopeSet::try_from(reduced).unwrap_or(scopes)
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

    use super::PathCase;
    use super::ScopeKind;
    use super::reduce;
    use crate::scope::DeclaredReservationScopeSet;

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
