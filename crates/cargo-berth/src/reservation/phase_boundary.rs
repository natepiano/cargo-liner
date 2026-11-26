//! Commit ownership inferred from recorded phase boundaries.

use std::path::Path;

use super::Reservation;
use super::ReservationLifecycle;
use super::RetainedReservationSet;
use crate::git;
use crate::git::Reachability;
use crate::git::RepositorySnapshot;
use crate::ids::GitObjectId;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ReservationScope;
use crate::ledger::ScopeKind;
use crate::scope::PathCase;
use crate::worktree::WorktreeHead;
use crate::worktree::WorktreeLiveness;
use crate::worktree::WorktreeRegistry;

pub(crate) struct AmbiguousCheckpoint;

/// Select the first checkpoint before the next live phase's work.
pub(crate) fn first_checkpoint_tip<'head>(
    repository_root: &Path,
    common_git_directory: &Path,
    reservations: &'head RetainedReservationSet,
    reservation: &Reservation,
    head: &'head GitObjectId,
) -> Result<&'head GitObjectId, AmbiguousCheckpoint> {
    let start = observed_claim_head(reservation);
    let mut later = None;
    for claim in other_live_checkout_claims(reservations, reservation) {
        let boundary = observed_claim_head(claim);
        if boundary == head {
            continue;
        }
        // A shared start gives no evidence of which phase owns the later work.
        if boundary == start {
            return Err(AmbiguousCheckpoint);
        }
        if claim.claimed_at() > reservation.claimed_at() && later.replace(claim).is_some() {
            return Err(AmbiguousCheckpoint);
        }
    }
    let Some(next) = later else {
        return Ok(head);
    };
    let boundary = observed_claim_head(next);
    if !is_ancestor(repository_root, start, boundary)
        || !is_ancestor(repository_root, boundary, head)
        || !is_ancestor(repository_root, head, claim_end(next, head))
    {
        return Err(AmbiguousCheckpoint);
    }
    let path_case = PathCase::read(common_git_directory).map_err(|_| AmbiguousCheckpoint)?;
    if !range_touches_only(repository_root, start, boundary, |scope| {
        owns_scope(reservation, scope, path_case)
    }) || !range_touches_only(repository_root, boundary, head, |scope| {
        owns_scope(next, scope, path_case)
            && !reservation
                .scopes()
                .as_slice()
                .iter()
                .any(|declared| declared.overlaps(scope, path_case))
    }) {
        return Err(AmbiguousCheckpoint);
    }
    Ok(boundary)
}

/// Prove that the next live phase owns all commits after the earlier checkpoint.
pub(crate) fn later_claim_owns_suffix(
    repository_root: &Path,
    next: &Reservation,
    checkpoint: &GitObjectId,
    head: &GitObjectId,
    path_case: PathCase,
) -> bool {
    observed_claim_head(next) == checkpoint
        && checkpoint != head
        && is_ancestor(repository_root, checkpoint, head)
        && is_ancestor(repository_root, head, claim_end(next, head))
        && range_touches_only(repository_root, checkpoint, head, |scope| {
            owns_scope(next, scope, path_case)
        })
}

/// Prove that a new checkpoint excludes pending commits from overlapping foreign phases.
pub(crate) fn foreign_ranges_are_disjoint(
    repository: &RepositorySnapshot,
    reservations: &RetainedReservationSet,
    reservation: &Reservation,
    previous: &GitObjectId,
    candidate: &GitObjectId,
    target: &GitObjectId,
) -> bool {
    let mut foreign = reservations
        .iter()
        .filter(|other| {
            other.actor().worktree != reservation.actor().worktree
                && !matches!(other.lifecycle(), ReservationLifecycle::Released { .. })
        })
        .peekable();
    if previous == candidate || foreign.peek().is_none() {
        return true;
    }
    let common_git_directory = repository.context().common_git_directory();
    let Ok(path_case) = PathCase::read(common_git_directory) else {
        return false;
    };
    let mut overlapping = foreign
        .filter(|other| {
            other.scopes().as_slice().iter().any(|foreign_scope| {
                reservation
                    .scopes()
                    .as_slice()
                    .iter()
                    .any(|declared| declared.overlaps(foreign_scope, path_case))
            })
        })
        .peekable();
    if overlapping.peek().is_none() {
        return true;
    }
    let Ok(registry) = WorktreeRegistry::read(repository) else {
        return false;
    };
    overlapping.all(|other| {
        let observed =
            registry.classify(reservation.actor().repository, common_git_directory, other);
        let WorktreeHead::Resolved(foreign_tip) = observed.head else {
            return false;
        };
        if !matches!(
            observed.liveness,
            WorktreeLiveness::Live | WorktreeLiveness::Unavailable
        ) {
            return false;
        }
        let foreign_start = observed_claim_head(other);
        let excluded = [previous, target, foreign_start];
        let excludes_foreign_work = |tip: &GitObjectId| {
            tip == foreign_start
                || git::commit_ranges_are_disjoint(
                    repository.repository_root(),
                    candidate,
                    tip,
                    &excluded,
                )
        };
        let recorded_tip = claim_end(other, &foreign_tip);
        excludes_foreign_work(&foreign_tip)
            && (recorded_tip == &foreign_tip || excludes_foreign_work(recorded_tip))
    })
}

/// Find later live reservations that used the same checkout.
pub(crate) fn later_holder_claims<'reservations>(
    reservations: &'reservations RetainedReservationSet,
    reservation: &Reservation,
) -> impl Iterator<Item = &'reservations Reservation> {
    let claimed_at = reservation.claimed_at().clone();
    other_live_checkout_claims(reservations, reservation)
        .filter(move |other| other.claimed_at() > &claimed_at)
}

fn other_live_checkout_claims<'reservations>(
    reservations: &'reservations RetainedReservationSet,
    reservation: &Reservation,
) -> impl Iterator<Item = &'reservations Reservation> {
    let worktree = reservation.actor().worktree;
    let id = reservation.id();
    reservations.iter().filter(move |other| {
        other.actor().worktree == worktree
            && other.id() != id
            && !matches!(other.lifecycle(), ReservationLifecycle::Released { .. })
    })
}

/// Return the commit the checkout had when the claim was recorded.
fn observed_claim_head(reservation: &Reservation) -> &GitObjectId {
    match reservation.head_snapshot() {
        ClaimHeadSnapshot::Branch { head, .. } | ClaimHeadSnapshot::Detached { head } => {
            head.as_ref()
        },
    }
}

fn claim_end<'head>(
    reservation: &'head Reservation,
    head: &'head GitObjectId,
) -> &'head GitObjectId {
    match reservation.lifecycle() {
        ReservationLifecycle::Outstanding { protected_tip } => protected_tip.as_ref(),
        ReservationLifecycle::Active | ReservationLifecycle::Released { .. } => head,
    }
}

fn is_ancestor(repository_root: &Path, ancestor: &GitObjectId, tip: &GitObjectId) -> bool {
    matches!(
        git::reachability(repository_root, ancestor, tip),
        Ok(Reachability::Ancestor)
    )
}

fn owns_scope(reservation: &Reservation, scope: &ReservationScope, path_case: PathCase) -> bool {
    reservation
        .scopes()
        .as_slice()
        .iter()
        .any(|declared| declared.contains(scope, path_case))
}

fn range_touches_only(
    repository_root: &Path,
    start: &GitObjectId,
    end: &GitObjectId,
    owns: impl Fn(&ReservationScope) -> bool,
) -> bool {
    git::commit_range_touches_only(repository_root, start, end, |path| {
        let Some(path) = std::str::from_utf8(path)
            .ok()
            .and_then(|path| path.parse().ok())
        else {
            return false;
        };
        owns(&ReservationScope {
            path,
            kind: ScopeKind::File,
        })
    })
}
