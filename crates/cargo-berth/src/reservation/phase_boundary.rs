//! Commit ownership inferred from recorded phase boundaries.

use std::path::Path;

use super::KeptWork;
use super::KeptWorkRelease;
use super::ReleaseDisposition;
use super::Reservation;
use super::ReservationLifecycle;
use super::RetainedProtectedTip;
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

#[derive(Clone, Copy)]
enum CheckpointRange {
    First,
    Resnapshot,
}

/// Select the first checkpoint that excludes other live reservations' work.
pub(crate) fn first_checkpoint_tip<'head>(
    repository: &RepositorySnapshot,
    reservations: &'head RetainedReservationSet,
    reservation: &Reservation,
    head: &'head GitObjectId,
    target: &GitObjectId,
) -> Result<&'head GitObjectId, AmbiguousCheckpoint> {
    let tip = same_checkout_checkpoint_tip(repository, reservations, reservation, head, target)?;
    let foreign = reservations.iter().filter(|other| {
        other.actor().worktree != reservation.actor().worktree
            && (!matches!(other.lifecycle(), ReservationLifecycle::Released { .. })
                || pending_checkpoint(other).is_some())
    });
    if !checkout_ranges_are_disjoint(
        repository,
        reservation,
        foreign,
        observed_claim_head(reservation),
        tip,
        target,
        CheckpointRange::First,
    ) {
        return Err(AmbiguousCheckpoint);
    }
    Ok(tip)
}

fn same_checkout_checkpoint_tip<'head>(
    repository: &RepositorySnapshot,
    reservations: &'head RetainedReservationSet,
    reservation: &Reservation,
    head: &'head GitObjectId,
    target: &GitObjectId,
) -> Result<&'head GitObjectId, AmbiguousCheckpoint> {
    let start = observed_claim_head(reservation);
    let mut earlier = Vec::new();
    let mut later = None;
    for claim in other_checkout_claims(reservations, reservation) {
        if matches!(claim.lifecycle(), ReservationLifecycle::Released { .. }) {
            if claim.claimed_at() <= reservation.claimed_at() && pending_checkpoint(claim).is_some()
            {
                earlier.push(claim);
            }
            continue;
        }
        let boundary = observed_claim_head(claim);
        if boundary == head {
            continue;
        }
        // A shared start gives no evidence of which phase owns the later work.
        if boundary == start {
            return Err(AmbiguousCheckpoint);
        }
        if claim.claimed_at() > reservation.claimed_at() {
            if later.replace(claim).is_some() {
                return Err(AmbiguousCheckpoint);
            }
        } else {
            earlier.push(claim);
        }
    }
    let tip = later.map_or(head, observed_claim_head);
    let repository_root = repository.repository_root();
    let range_start = reservation.reanchored_claim_head().unwrap_or(start);
    for claim in other_checkout_claims(reservations, reservation) {
        let pending_tip = match claim.lifecycle() {
            ReservationLifecycle::Outstanding { protected_tip } => protected_tip.as_ref(),
            ReservationLifecycle::Released {
                disposition: ReleaseDisposition::KeptOnBranch(kept),
            } => match &kept.work {
                KeptWork::Known(tip)
                    if repository.object_reader().commit(&tip.to_string()).is_ok() =>
                {
                    tip
                },
                KeptWork::HoldTransfer => continue,
                KeptWork::Known(_) | KeptWork::Unknown => return Err(AmbiguousCheckpoint),
            },
            ReservationLifecycle::Active | ReservationLifecycle::Released { .. } => continue,
        };
        if range_start != tip
            && !excludes_claim_work(
                repository_root,
                claim,
                pending_tip,
                range_start,
                tip,
                target,
            )
        {
            return Err(AmbiguousCheckpoint);
        }
    }
    if earlier.is_empty() && later.is_none() {
        return Ok(tip);
    }
    let path_case = PathCase::read(repository.context().common_git_directory())
        .map_err(|_| AmbiguousCheckpoint)?;
    if !excludes_earlier_work(
        repository_root,
        reservation,
        &earlier,
        later,
        tip,
        path_case,
    ) {
        return Err(AmbiguousCheckpoint);
    }
    if later.is_some_and(|next| {
        !later_claim_owns_suffix(repository_root, reservation, next, tip, head, path_case)
    }) {
        return Err(AmbiguousCheckpoint);
    }
    Ok(tip)
}

fn excludes_earlier_work(
    repository_root: &Path,
    reservation: &Reservation,
    earlier: &[&Reservation],
    later: Option<&Reservation>,
    tip: &GitObjectId,
    path_case: PathCase,
) -> bool {
    let reanchored = reservation.reanchored_claim_head();
    let start = reanchored.unwrap_or_else(|| observed_claim_head(reservation));
    start == tip
        || is_ancestor(repository_root, start, tip)
            && range_touches_only(repository_root, start, tip, |scope| {
                (later.is_none() || owns_scope(reservation, scope, path_case))
                    && earlier.iter().all(|claim| match claim.lifecycle() {
                        ReservationLifecycle::Active => !overlaps_scope(claim, scope, path_case),
                        ReservationLifecycle::Outstanding { .. } => {
                            !overlaps_scope(claim, scope, path_case)
                                || reanchored.is_some() && owns_scope(reservation, scope, path_case)
                        },
                        ReservationLifecycle::Released { .. } => true,
                    })
            })
}

/// Prove that the next live phase owns all commits after the earlier checkpoint.
pub(crate) fn later_claim_owns_suffix(
    repository_root: &Path,
    reservation: &Reservation,
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
            owns_scope(next, scope, path_case) && !overlaps_scope(reservation, scope, path_case)
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
            other
                .scopes()
                .as_slice()
                .iter()
                .any(|scope| overlaps_scope(reservation, scope, path_case))
        })
        .peekable();
    if overlapping.peek().is_none() {
        return true;
    }
    checkout_ranges_are_disjoint(
        repository,
        reservation,
        overlapping,
        previous,
        candidate,
        target,
        CheckpointRange::Resnapshot,
    )
}

fn checkout_ranges_are_disjoint<'claim>(
    repository: &RepositorySnapshot,
    reservation: &Reservation,
    claims: impl Iterator<Item = &'claim Reservation>,
    previous: &GitObjectId,
    candidate: &GitObjectId,
    target: &GitObjectId,
    checkpoint_range: CheckpointRange,
) -> bool {
    let mut claims = claims.peekable();
    if previous == candidate || claims.peek().is_none() {
        return true;
    }
    let Ok(registry) = WorktreeRegistry::read(repository) else {
        return false;
    };
    let common_git_directory = repository.context().common_git_directory();
    let repository_root = repository.repository_root();
    claims.all(|claim| {
        let excludes_work =
            |tip| excludes_claim_work(repository_root, claim, tip, previous, candidate, target);
        if pending_checkpoint(claim).is_some_and(|checkpoint| !excludes_work(checkpoint)) {
            return false;
        }
        if matches!(claim.lifecycle(), ReservationLifecycle::Released { .. }) {
            return true;
        }
        let observed =
            registry.classify(reservation.actor().repository, common_git_directory, claim);
        if !matches!(
            observed.liveness,
            WorktreeLiveness::Live | WorktreeLiveness::Unavailable
        ) {
            return false;
        }
        let WorktreeHead::Resolved(tip) = observed.head else {
            return false;
        };
        excludes_work(&tip)
            || matches!(checkpoint_range, CheckpointRange::First)
                && git::checkout_imported_linear_candidate(
                    repository_root,
                    previous,
                    candidate,
                    &tip,
                    &[previous, target, observed_claim_head(claim)],
                )
    })
}

fn excludes_claim_work(
    repository_root: &Path,
    claim: &Reservation,
    claim_tip: &GitObjectId,
    previous: &GitObjectId,
    candidate: &GitObjectId,
    target: &GitObjectId,
) -> bool {
    let claim_start = observed_claim_head(claim);
    claim_tip == claim_start
        || git::commit_ranges_are_disjoint(
            repository_root,
            candidate,
            claim_tip,
            &[previous, target, claim_start],
        )
}

/// Return the recorded tip whose work can still be absent from the target.
fn pending_checkpoint(reservation: &Reservation) -> Option<&GitObjectId> {
    match reservation.lifecycle() {
        ReservationLifecycle::Outstanding { protected_tip } => Some(protected_tip.as_ref()),
        ReservationLifecycle::Released {
            disposition:
                ReleaseDisposition::KeptOnBranch(KeptWorkRelease {
                    work: KeptWork::Known(_) | KeptWork::Unknown,
                    ..
                }),
        } => {
            let RetainedProtectedTip::Retained(tip) = reservation.protected_checkpoint() else {
                return None;
            };
            Some(tip.as_ref())
        },
        ReservationLifecycle::Active | ReservationLifecycle::Released { .. } => None,
    }
}

/// Find later live reservations that used the same checkout.
pub(crate) fn later_holder_claims<'reservations>(
    reservations: &'reservations RetainedReservationSet,
    reservation: &Reservation,
) -> impl Iterator<Item = &'reservations Reservation> {
    let claimed_at = reservation.claimed_at().clone();
    other_checkout_claims(reservations, reservation).filter(move |other| {
        other.claimed_at() > &claimed_at
            && !matches!(other.lifecycle(), ReservationLifecycle::Released { .. })
    })
}

fn other_checkout_claims<'reservations>(
    reservations: &'reservations RetainedReservationSet,
    reservation: &Reservation,
) -> impl Iterator<Item = &'reservations Reservation> {
    let worktree = reservation.actor().worktree;
    let id = reservation.id();
    reservations
        .iter()
        .filter(move |other| other.actor().worktree == worktree && other.id() != id)
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

fn overlaps_scope(
    reservation: &Reservation,
    scope: &ReservationScope,
    path_case: PathCase,
) -> bool {
    reservation
        .scopes()
        .as_slice()
        .iter()
        .any(|declared| declared.overlaps(scope, path_case))
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
