//! Branch rewrites and the phase-anchor re-anchoring one forces.

use std::collections::HashSet;
use std::convert::Infallible;
use std::fs;
use std::io;
use std::io::ErrorKind;
use std::path::Path;

use super::RewriteCreatedCommits;
use super::error::GateError;
use super::error::GateTransactionRejection;
use super::map_phase_interval;
use super::permit;
use super::reference_transaction::ReferenceObject;
use super::reference_transaction::ReferenceUpdate;
use super::rewrite_map::PhaseRewriteMapping;
use super::rewrite_map::RewriteMapPair;
use crate::git;
use crate::git::GitCommandOutputAvailability;
use crate::git::GitError;
use crate::git::Reachability;
use crate::git::ScopedPatchComparison;
use crate::git::ScopedPatchTargetHistory;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ledger;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::FullRefName;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerCommittedActionError;
use crate::ledger::LedgerCommittedActionOutcome;
use crate::ledger::LedgerError;
use crate::ledger::ReconciliationValidation;
use crate::ledger::ReservationSnapshot;
use crate::ledger::WorktreeContext;
use crate::reconcile::GateReconciliationError;
use crate::reservation::Reservation;
use crate::reservation::ReservationLifecycle;
use crate::reservation::RetainedReservationSet;

/// One branch whose new tip no longer contains the tip it replaced.
#[derive(Clone)]
pub(super) struct BranchRewrite {
    /// The full `refs/heads/...` name the transaction moved.
    reference: FullRefName,
    /// The tip the branch carried before the rewrite.
    previous:  GitObjectId,
    /// The tip the branch carries now.
    proposed:  GitObjectId,
}

/// Select the branch updates that discarded history rather than extending it.
///
/// A rebase, an amend, and a reset all land here; an ordinary commit and a fast-forward
/// merge do not, because their previous tip survives in the proposed history.
/// Call only for committed transactions, when the branch reflog contains this update.
pub(super) fn branch_rewrites(
    issuing_directory: &Path,
    updates: &[&ReferenceUpdate],
) -> Result<Vec<BranchRewrite>, GateError> {
    let mut rewrites = Vec::new();
    for update in updates {
        let ReferenceObject::Object(proposed) = &update.proposed else {
            continue;
        };
        let previous = match &update.previous {
            ReferenceObject::Object(previous) => previous.clone(),
            ReferenceObject::Absent => {
                match apply_rebase_previous_tip(issuing_directory, &update.reference, proposed)? {
                    ApplyRebasePreviousTip::Verified(previous) => previous,
                    ApplyRebasePreviousTip::NotApplyBranch => continue,
                }
            },
            ReferenceObject::Symbolic(_) => continue,
        };
        if &previous == proposed {
            continue;
        }
        match git::reachability(issuing_directory, &previous, proposed).map_err(GateError::Git)? {
            Reachability::NotAncestor => rewrites.push(BranchRewrite {
                reference: update.reference.clone(),
                previous,
                proposed: proposed.clone(),
            }),
            Reachability::Ancestor | Reachability::ObjectUnknown => {},
        }
    }
    Ok(rewrites)
}

/// Whether an apply rebase establishes the branch's replaced history.
enum ApplyRebasePreviousTip {
    /// No apply map exists, or a stopped rebase does not update this branch.
    NotApplyBranch,
    /// Rebase metadata or the uninterrupted branch reflog supplies the previous tip.
    Verified(GitObjectId),
}

/// Recover an apply rebase's omitted previous tip and verify every old map commit.
fn apply_rebase_previous_tip(
    issuing_directory: &Path,
    reference: &FullRefName,
    proposed: &GitObjectId,
) -> Result<ApplyRebasePreviousTip, GateError> {
    let context = WorktreeContext::discover(issuing_directory)?;
    let directory = context.administrative_directory().join("rebase-apply");
    let contents = match fs::read_to_string(directory.join("rewritten")) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ApplyRebasePreviousTip::NotApplyBranch);
        },
        Err(error) => return Err(LedgerError::Io(error).into()),
    };
    let previous = if directory
        .join("onto")
        .try_exists()
        .map_err(LedgerError::Io)?
    {
        match stopped_apply_rebase_previous_tip(&directory, reference).map_err(LedgerError::Io)? {
            ApplyRebasePreviousTip::Verified(previous) => previous,
            ApplyRebasePreviousTip::NotApplyBranch => {
                return Ok(ApplyRebasePreviousTip::NotApplyBranch);
            },
        }
    } else {
        uninterrupted_apply_rebase_previous_tip(context.repository_root(), reference, proposed)?
    };
    let pairs = decode_rewrite_map(&contents).map_err(LedgerError::Io)?;
    for pair in pairs {
        if git::reachability(context.repository_root(), &pair.old, &previous)
            .map_err(GateError::Git)?
            != Reachability::Ancestor
        {
            return Err(LedgerError::Io(io::Error::new(
                ErrorKind::InvalidData,
                "apply rebase previous tip does not contain every old map commit",
            ))
            .into());
        }
    }
    Ok(ApplyRebasePreviousTip::Verified(previous))
}

/// A stopped apply rebase records its original tip independently of branch reflog gaps.
fn stopped_apply_rebase_previous_tip(
    directory: &Path,
    reference: &FullRefName,
) -> io::Result<ApplyRebasePreviousTip> {
    let head_name = fs::read_to_string(directory.join("head-name"))?;
    // Git strips only the line ending; a valid ref name may end in Unicode whitespace.
    let head_name = head_name.trim_end_matches(['\n', '\r']);
    // A detached rebase records this literal name and never updates a branch.
    if head_name == "detached HEAD" {
        return Ok(ApplyRebasePreviousTip::NotApplyBranch);
    }
    if !head_name.starts_with("refs/heads/") {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "stopped apply rebase head-name is not a local branch",
        ));
    }
    let rebase_reference: FullRefName = head_name.parse().map_err(io::Error::other)?;
    if &rebase_reference != reference {
        return Ok(ApplyRebasePreviousTip::NotApplyBranch);
    }
    let previous = fs::read_to_string(directory.join("orig-head"))?
        .trim()
        .parse()
        .map_err(io::Error::other)?;
    Ok(ApplyRebasePreviousTip::Verified(previous))
}

/// Recover an uninterrupted apply rebase's omitted previous tip from its branch reflog.
///
/// Without `onto`, `RecordedDestinations` keeps only map destinations that this rebase
/// just created. No older reflog value can reach them, so a reflog gap cannot exclude
/// a created commit. User commands cannot run in this worktree before the rebase stops.
fn uninterrupted_apply_rebase_previous_tip(
    repository_root: &Path,
    reference: &FullRefName,
    proposed: &GitObjectId,
) -> Result<GitObjectId, GateError> {
    let current = branch_reflog_tip(repository_root, reference, 0).map_err(GateError::Git)?;
    if &current != proposed {
        return Err(LedgerError::Io(io::Error::new(
            ErrorKind::InvalidData,
            "apply rebase branch reflog does not end at the proposed tip",
        ))
        .into());
    }
    branch_reflog_tip(repository_root, reference, 1).map_err(GateError::Git)
}

/// Read one branch reflog entry through Git for either reference storage backend.
fn branch_reflog_tip(
    repository_root: &Path,
    reference: &FullRefName,
    entry: usize,
) -> Result<GitObjectId, GitError> {
    let output = match git::execute_read_only_git(
        repository_root,
        &["rev-parse", "--verify", &format!("{reference}@{{{entry}}}")],
    ) {
        GitCommandOutputAvailability::Available(output) => output,
        GitCommandOutputAvailability::Unavailable(error) => {
            return Err(GitError::Io(error));
        },
    };
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: "rev-parse",
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .trim()
        .parse()
        .map_err(GitError::InvalidObjectId)
}

/// One branch's immutable rewrite facts, shared by marker persistence and active re-anchoring.
pub(super) struct BranchRewriteEvent {
    /// The branch move whose pre-transaction history bounds this event.
    branch:          BranchRewrite,
    /// Only pairs whose destinations belong to this branch's introduced history.
    pairs:           Vec<RewriteMapPair>,
    /// Introduced commits observed before reconciliation can move retention refs.
    created_commits: Vec<GitObjectId>,
}

/// The boundary that identifies commits introduced by a rewrite operation.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RewriteBase {
    /// The rebase backend recorded this history boundary beside its commit map.
    RebaseOnto(GitObjectId),
    /// An uninterrupted apply rebase records every created commit as a map destination.
    RecordedDestinations,
    /// An amend or reset has only the replaced tip and nominated pairs as boundaries.
    TipReplacement,
}

/// One rewrite operation's commit nominations and introduced-history boundary.
struct RewriteOperation {
    /// The operation's rule for bounding introduced history, independent of ref moves.
    base:  RewriteBase,
    /// Every nomination supplied by the operation, before filtering for each branch.
    pairs: Vec<RewriteMapPair>,
}

/// Copy each branch's rewrite facts using the issuing operation's history boundaries.
pub(super) fn capture_branch_rewrites(
    issuing_directory: &Path,
    rewrites: &[BranchRewrite],
) -> Result<Vec<BranchRewriteEvent>, GateError> {
    if rewrites.is_empty() {
        return Ok(Vec::new());
    }
    let context = WorktreeContext::discover(issuing_directory)?;
    let operation =
        read_rewrite_map(context.administrative_directory(), rewrites).map_err(LedgerError::Io)?;
    let events = rewrites
        .iter()
        .map(|rewrite| {
            let created_commits = capture_rewrite_created_commits(
                context.repository_root(),
                context.administrative_directory(),
                &rewrite.proposed,
                &rewrite.previous,
                &operation.pairs,
                &operation.base,
            )
            .map_err(GateError::Git)?;
            let created = RewriteCreatedCommits::from_commits(created_commits.iter().cloned());
            Ok(BranchRewriteEvent {
                branch: rewrite.clone(),
                pairs: created.branch_pairs(&operation.pairs),
                created_commits,
            })
        })
        .collect::<Result<Vec<_>, GateError>>()?;
    for event in &events {
        permit::write_branch_rewrite_marker(
            context.common_git_directory(),
            context.administrative_directory(),
            event.pairs.clone(),
            vec![event.branch.proposed.clone()],
            event.created_commits.clone(),
        )
        .map_err(LedgerError::Io)?;
    }
    Ok(events)
}

/// Read the old protected phase commits in oldest-first topological order.
pub(crate) fn rewrite_phase_commits(
    repository_root: &Path,
    phase_start: &GitObjectId,
    protected_tip: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    read_rewrite_history(
        repository_root,
        &[
            "--reverse",
            "--topo-order",
            &format!("{phase_start}..{protected_tip}"),
        ],
    )
}

/// Read a rewritten tip's complete first-parent history, including its root base.
pub(crate) fn rewritten_first_parent_history(
    repository_root: &Path,
    tip: &GitObjectId,
) -> Result<Vec<GitObjectId>, GitError> {
    read_rewrite_history(
        repository_root,
        &["--reverse", "--first-parent", &tip.to_string()],
    )
}

/// Capture a branch's introduced commits independently of all current ref values.
pub(crate) fn capture_rewrite_created_commits(
    repository_root: &Path,
    worktree_administrative_directory: &Path,
    new_tip: &GitObjectId,
    previous_tip: &GitObjectId,
    pairs: &[RewriteMapPair],
    base: &RewriteBase,
) -> Result<Vec<GitObjectId>, GitError> {
    // An explicit absolute git directory makes Git recognize the issuing worktree;
    // discovering Git from inside its administrative directory can count its HEAD twice.
    let directory = fs::canonicalize(worktree_administrative_directory).map_err(GitError::Io)?;
    let directory = directory.to_str().ok_or_else(|| {
        GitError::Io(io::Error::new(
            ErrorKind::InvalidInput,
            "rewrite administrative directory is not UTF-8",
        ))
    })?;
    let mut arguments = vec![
        format!("--git-dir={directory}"),
        "rev-list".to_owned(),
        "--ignore-missing".to_owned(),
        new_tip.to_string(),
        "--not".to_owned(),
    ];
    if let RewriteBase::RebaseOnto(onto) = base {
        arguments.push(onto.to_string());
    }
    arguments.extend(pairs.iter().map(|pair| pair.old.to_string()));
    arguments.push(previous_tip.to_string());
    let mut created = decode_rewrite_history(git::execute_read_only_git(
        repository_root,
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    ))?;
    if *base == RewriteBase::RecordedDestinations {
        let destinations = pairs.iter().map(|pair| &pair.new).collect::<HashSet<_>>();
        created.retain(|commit| destinations.contains(commit));
    }
    Ok(created)
}

/// Decode a rewrite history query through the shared Git execution boundary.
fn read_rewrite_history(
    repository_root: &Path,
    revisions: &[&str],
) -> Result<Vec<GitObjectId>, GitError> {
    let arguments = [&["rev-list"], revisions].concat();
    decode_rewrite_history(git::execute_read_only_git(repository_root, &arguments))
}

/// Decode complete commit ids while preserving unavailable or failed Git reads.
fn decode_rewrite_history(
    output: GitCommandOutputAvailability,
) -> Result<Vec<GitObjectId>, GitError> {
    let output = match output {
        GitCommandOutputAvailability::Available(output) => output,
        GitCommandOutputAvailability::Unavailable(error) => {
            return Err(GitError::Io(error));
        },
    };
    if !output.status.success() {
        return Err(GitError::CommandFailed {
            command: "rev-list",
            stderr:  String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    String::from_utf8(output.stdout)
        .map_err(GitError::InvalidOutput)?
        .lines()
        .map(|line| line.parse().map_err(GitError::InvalidObjectId))
        .collect()
}

/// Read either rebase backend's map, or nominate tip replacement for an amend/reset.
fn read_rewrite_map(
    administrative_directory: &Path,
    rewrites: &[BranchRewrite],
) -> io::Result<RewriteOperation> {
    for (backend, map_name) in [
        ("rebase-merge", "rewritten-list"),
        ("rebase-apply", "rewritten"),
    ] {
        let directory = administrative_directory.join(backend);
        let contents = match fs::read_to_string(directory.join(map_name)) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let base = match fs::read_to_string(directory.join("onto")) {
            Ok(onto) => RewriteBase::RebaseOnto(onto.trim().parse().map_err(io::Error::other)?),
            Err(error) if backend == "rebase-apply" && error.kind() == ErrorKind::NotFound => {
                RewriteBase::RecordedDestinations
            },
            Err(error) => return Err(error),
        };
        let pairs = decode_rewrite_map(&contents)?;
        return Ok(RewriteOperation { base, pairs });
    }
    Ok(RewriteOperation {
        base:  RewriteBase::TipReplacement,
        pairs: rewrites
            .iter()
            .map(|rewrite| RewriteMapPair {
                old: rewrite.previous.clone(),
                new: rewrite.proposed.clone(),
            })
            .collect(),
    })
}

/// Decode complete old/new nominations without accepting partial or malformed pairs.
fn decode_rewrite_map(contents: &str) -> io::Result<Vec<RewriteMapPair>> {
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let mut fields = line.split_whitespace();
            let (Some(old), Some(new), None) = (fields.next(), fields.next(), fields.next()) else {
                return Err(io::Error::new(
                    ErrorKind::InvalidData,
                    "invalid Git rewrite map pair",
                ));
            };
            Ok(RewriteMapPair {
                old: old.parse().map_err(io::Error::other)?,
                new: new.parse().map_err(io::Error::other)?,
            })
        })
        .collect()
}

/// Move every active reservation's phase start onto the rewritten history of its branch.
///
/// `<phase_start_head>..HEAD` only means "the commits this phase authored" while the
/// proposed history still contains the anchor. A rebase makes that false with no signal
/// of its own, and drift then reads the new base's commits as this phase's work, raising
/// incursions against worktrees that did nothing and widening onto files nobody here
/// opened. Re-anchoring restores the range's meaning at the moment the branch moves.
///
/// The reservations are found by the branch each one recorded at claim time. Git refuses
/// to check one branch out in two worktrees at once, so a branch name identifies the
/// acting worktree even though the hook has already changed directory away from it.
pub(super) fn reanchor_rewritten_phases(
    invocation_directory: &Path,
    worktree_context: &WorktreeContext,
    events: &[BranchRewriteEvent],
) -> Result<(), GateError> {
    let ledger = Ledger::open(invocation_directory)?;
    let journal_mutation_actor = ledger::resolve_identity(worktree_context)?
        .journal_mutation_actor_for(CoordinationRunId::new());
    let repository_root = worktree_context.repository_root();
    let outcome = ledger
        .transact_reconciliation(
            journal_mutation_actor.worktree_id,
            journal_mutation_actor.coordination_run_id,
            |state| {
                let reservations = match RetainedReservationSet::replay(state.events()) {
                    Ok(reservations) => reservations,
                    Err(error) => {
                        return ReconciliationValidation::Reject(
                            GateTransactionRejection::Reconciliation(
                                GateReconciliationError::Reservation(error),
                            ),
                        );
                    },
                };
                ReconciliationValidation::Apply {
                    cover_actors:           std::collections::HashMap::default(),
                    operations:             resnapshot_operations(
                        repository_root,
                        &reservations,
                        events,
                    ),
                    recoverable_operations: Vec::new(),
                    action:                 (),
                }
            },
            |(), _, _| Ok::<(), Infallible>(()),
        )
        .map_err(|error| match error {
            LedgerCommittedActionError::Transaction(error) => GateError::Transaction(error),
            LedgerCommittedActionError::Action(error) => match error {},
        })?;
    match outcome {
        LedgerCommittedActionOutcome::Appended { output: (), .. } => Ok(()),
        LedgerCommittedActionOutcome::Rejected(rejection) => Err(rejection.into()),
    }
}

/// Compute one replacement anchor per active reservation the rewrites moved.
///
/// A reservation whose anchor git cannot recompute keeps the one it has. The branch has
/// already moved by the time this runs, so refusing the whole transaction would neither
/// undo the rewrite nor leave the ledger in a better state than a stale anchor does.
fn resnapshot_operations(
    repository_root: &Path,
    reservations: &RetainedReservationSet,
    events: &[BranchRewriteEvent],
) -> Vec<JournalOperation> {
    let mut operations = Vec::new();
    for reservation in reservations.iter() {
        if !matches!(reservation.lifecycle(), ReservationLifecycle::Active) {
            continue;
        }
        let ClaimHeadSnapshot::Branch { full_ref, .. } = reservation.head_snapshot() else {
            continue;
        };
        let Some(event) = events
            .iter()
            .find(|event| &event.branch.reference == full_ref)
        else {
            continue;
        };
        let phase_start = reservation.phase_start_head();
        let created = RewriteCreatedCommits::from_commits(event.created_commits.iter().cloned());
        let ActivePhaseAnchor::Replace(claim_snapshot) = active_phase_anchor(
            repository_root,
            reservation,
            &event.branch,
            &event.pairs,
            &created,
        ) else {
            continue;
        };
        if &claim_snapshot == phase_start.as_ref() {
            continue;
        }
        operations.push(JournalOperation::Resnapshot {
            reservation_id: reservation.id(),
            snapshot:       ReservationSnapshot::Active { claim_snapshot },
        });
    }
    operations
}

/// Whether a rewritten active phase has a justified replacement start.
enum ActivePhaseAnchor {
    /// The old anchor remains until a later rewrite can locate the phase.
    Preserve,
    /// Mapping with scoped replay, or the legacy patch inference, located this phase start.
    Replace(GitObjectId),
}

/// Keep conflict-resolved active work inside the interval a later checkpoint protects.
fn active_phase_anchor(
    repository_root: &Path,
    reservation: &Reservation,
    rewrite: &BranchRewrite,
    pairs: &[RewriteMapPair],
    created: &RewriteCreatedCommits,
) -> ActivePhaseAnchor {
    let phase_start = reservation.phase_start_head().as_ref();
    let Ok(old_phase) = rewrite_phase_commits(repository_root, phase_start, &rewrite.previous)
    else {
        return ActivePhaseAnchor::Preserve;
    };
    if !pairs.iter().any(|pair| old_phase.contains(&pair.old)) {
        return git::rewritten_phase_anchor(
            repository_root,
            phase_start,
            &rewrite.previous,
            &rewrite.proposed,
        )
        .map_or(ActivePhaseAnchor::Preserve, ActivePhaseAnchor::Replace);
    }
    let Ok(history) = rewritten_first_parent_history(repository_root, &rewrite.proposed) else {
        return ActivePhaseAnchor::Preserve;
    };
    let PhaseRewriteMapping::Mapped(interval) =
        map_phase_interval(pairs, &old_phase, &history, created)
    else {
        return ActivePhaseAnchor::Preserve;
    };
    match git::scoped_patch_equivalence_with_target_history(
        repository_root,
        phase_start,
        reservation.scopes(),
        &rewrite.previous,
        &interval.protected_tip,
        ScopedPatchTargetHistory::MappedDestinations {
            commits: &interval.destinations,
        },
    ) {
        Ok(ScopedPatchComparison::Equivalent) => {
            ActivePhaseAnchor::Replace(interval.phase_start_head)
        },
        Ok(ScopedPatchComparison::Different | ScopedPatchComparison::Unavailable) | Err(_) => {
            ActivePhaseAnchor::Preserve
        },
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fmt::Write;
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::BranchRewrite;
    use super::ReferenceObject;
    use super::ReferenceUpdate;
    use super::RewriteBase;
    use super::read_rewrite_map;
    use crate::gate::rewrite_map::RewriteMapPair;
    use crate::ids::GitObjectId;

    #[test]
    fn capture_preserves_many_to_one_pairs_from_both_rebase_backends() -> Result<(), Box<dyn Error>>
    {
        let old = "1111111111111111111111111111111111111111";
        let other_old = "2222222222222222222222222222222222222222";
        let new = "3333333333333333333333333333333333333333";
        for (backend, name) in [
            ("rebase-merge", "rewritten-list"),
            ("rebase-apply", "rewritten"),
        ] {
            let directory = tempdir()?;
            fs::create_dir(directory.path().join(backend))?;
            fs::write(
                directory.path().join(backend).join(name),
                format!("{old} {new}\n{other_old} {new}\n"),
            )?;
            let expected_base = if backend == "rebase-merge" {
                fs::write(
                    directory.path().join(backend).join("onto"),
                    format!("{old}\n"),
                )?;
                RewriteBase::RebaseOnto(old.parse()?)
            } else {
                RewriteBase::RecordedDestinations
            };
            let operation = read_rewrite_map(directory.path(), &[])?;
            assert_eq!(operation.base, expected_base);
            assert_eq!(
                operation.pairs,
                vec![
                    RewriteMapPair {
                        old: old.parse()?,
                        new: new.parse()?,
                    },
                    RewriteMapPair {
                        old: other_old.parse()?,
                        new: new.parse()?,
                    },
                ]
            );
        }
        Ok(())
    }

    #[test]
    fn capture_nominates_tip_replacement_when_no_rebase_map_exists() -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let rewrite = BranchRewrite {
            reference: "refs/heads/topic".parse()?,
            previous:  "1111111111111111111111111111111111111111".parse()?,
            proposed:  "2222222222222222222222222222222222222222".parse()?,
        };
        let operation = read_rewrite_map(directory.path(), std::slice::from_ref(&rewrite))?;
        assert_eq!(operation.base, RewriteBase::TipReplacement);
        assert_eq!(
            operation.pairs,
            vec![RewriteMapPair {
                old: rewrite.previous,
                new: rewrite.proposed,
            }]
        );
        Ok(())
    }

    #[test]
    fn uninterrupted_apply_capture_keeps_only_reachable_map_destinations()
    -> Result<(), Box<dyn Error>> {
        let directory = tempdir()?;
        let root = directory.path();
        git_output(root, &["init", "--quiet", "--initial-branch", "main"])?;
        git_output(root, &["config", "user.name", "Test User"])?;
        git_output(root, &["config", "user.email", "test@example.com"])?;
        let tree = git_output(root, &["mktree"])?;
        let base = git_output(root, &["commit-tree", &tree, "-m", "base"])?;
        let commit = |parent: &str, message: &str| {
            git_output(root, &["commit-tree", &tree, "-p", parent, "-m", message])
        };
        let old = commit(&base, "old topic")?;
        let other_old = commit(&old, "other old topic")?;
        let trunk = commit(&base, "intervening trunk")?;
        let onto = commit(&trunk, "new base")?;
        let proposed = commit(&onto, "rewritten topic")?;
        let other_new = commit(&proposed, "other rewritten topic")?;
        git_output(root, &["update-ref", "refs/heads/main", &onto])?;
        git_output(root, &["update-ref", "refs/heads/topic", &proposed])?;
        let rebase = root.join(".git/rebase-apply");
        fs::create_dir(&rebase)?;
        fs::write(
            rebase.join("rewritten"),
            format!("{old} {proposed}\n{other_old} {other_new}\n"),
        )?;
        assert!(!rebase.join("onto").exists());
        let rewrite = BranchRewrite {
            reference: "refs/heads/topic".parse()?,
            previous:  old.parse()?,
            proposed:  proposed.parse()?,
        };
        let operation = read_rewrite_map(&root.join(".git"), std::slice::from_ref(&rewrite))?;
        assert_eq!(operation.base, RewriteBase::RecordedDestinations);
        let events = super::capture_branch_rewrites(root, std::slice::from_ref(&rewrite))?;
        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.created_commits, vec![rewrite.proposed.clone()]);
        for excluded in [&trunk, &onto, &other_new] {
            assert!(!event.created_commits.contains(&excluded.parse()?));
        }
        assert_eq!(
            event.pairs,
            vec![RewriteMapPair {
                old: rewrite.previous,
                new: rewrite.proposed.clone(),
            }]
        );
        let markers = super::permit::pending_branch_rewrite_markers(&root.join(".git"))?;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].rewrite.pairs, event.pairs);
        assert_eq!(markers[0].rewrite.created_commits, event.created_commits);
        assert_eq!(markers[0].rewrite.new_tips, vec![rewrite.proposed]);
        Ok(())
    }

    #[test]
    fn zero_previous_apply_capture_recovers_the_branch_reflog_tip() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_rebase_map("rebase-apply", "rewritten")?;
        let upper = &fixture.branches[1];
        git_output(
            fixture.directory.path(),
            &[
                "update-ref",
                &upper.reference.to_string(),
                &upper.proposed.to_string(),
            ],
        )?;
        let events = capture_zero_previous_update(&fixture, 1)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].branch.previous, upper.previous);
        assert_eq!(events[0].pairs, fixture.pairs());
        assert_eq!(
            events[0].created_commits,
            vec![upper.proposed.clone(), fixture.branches[0].proposed.clone()]
        );
        let markers =
            super::permit::pending_branch_rewrite_markers(&fixture.directory.path().join(".git"))?;
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].rewrite.created_commits,
            events[0].created_commits
        );
        Ok(())
    }

    #[test]
    fn stopped_apply_reflog_gap_keeps_the_unrecorded_split_commit() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_stopped_apply_rebase(1)?;
        let root = fixture.directory.path();
        let upper = &fixture.branches[1];
        let tree = git_output(root, &["mktree"])?;
        let misleading_previous = git_output(
            root,
            &[
                "commit-tree",
                &tree,
                "-p",
                &upper.previous.to_string(),
                "-p",
                &fixture.split_first.to_string(),
                "-m",
                "earlier branch tip containing the original phase and first split",
            ],
        )?;
        for tip in [
            misleading_previous.clone(),
            upper.previous.to_string(),
            upper.proposed.to_string(),
        ] {
            git_output(root, &["update-ref", &upper.reference.to_string(), &tip])?;
        }
        // Drop the reset entry without rewriting adjacent records: @{1} now names M,
        // while the newest record still says the branch moved from orig-head to V.
        git_output(
            root,
            &["reflog", "delete", &format!("{}@{{1}}", upper.reference)],
        )?;
        let misleading_previous: GitObjectId = misleading_previous.parse()?;
        assert_eq!(
            super::branch_reflog_tip(root, &upper.reference, 0)?,
            upper.proposed
        );
        assert_eq!(
            super::branch_reflog_tip(root, &upper.reference, 1)?,
            misleading_previous
        );
        assert_ne!(misleading_previous, upper.previous);
        for pair in fixture.pairs() {
            assert_eq!(
                crate::git::reachability(root, &pair.old, &misleading_previous)?,
                crate::git::Reachability::Ancestor
            );
        }
        assert_eq!(
            crate::git::reachability(root, &fixture.split_first, &misleading_previous)?,
            crate::git::Reachability::Ancestor
        );
        let events = capture_zero_previous_update(&fixture, 1)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].branch.previous, upper.previous);
        let expected_created = vec![
            upper.proposed.clone(),
            fixture.branches[0].proposed.clone(),
            fixture.split_first.clone(),
        ];
        assert_eq!(events[0].created_commits, expected_created);
        assert_eq!(events[0].pairs, fixture.pairs());
        let markers = super::permit::pending_branch_rewrite_markers(&root.join(".git"))?;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].rewrite.created_commits, expected_created);
        Ok(())
    }

    #[test]
    fn stopped_apply_skips_other_branch_creation_and_captures_its_own_update()
    -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_stopped_apply_rebase(1)?;
        let root = fixture.directory.path();
        let unrelated = ReferenceUpdate {
            reference: "refs/heads/unrelated".parse()?,
            previous:  ReferenceObject::Absent,
            proposed:  ReferenceObject::Object(fixture.base.clone()),
        };
        git_output(
            root,
            &[
                "update-ref",
                &unrelated.reference.to_string(),
                &fixture.base.to_string(),
            ],
        )?;
        assert!(super::branch_reflog_tip(root, &unrelated.reference, 1).is_err());
        let rewrites = super::branch_rewrites(root, &[&unrelated])?;
        assert!(super::capture_branch_rewrites(root, &rewrites)?.is_empty());
        assert!(super::permit::pending_branch_rewrite_markers(&root.join(".git"))?.is_empty());
        let upper = &fixture.branches[1];
        git_output(
            root,
            &[
                "update-ref",
                &upper.reference.to_string(),
                &upper.proposed.to_string(),
            ],
        )?;
        let own = ReferenceUpdate {
            reference: upper.reference.clone(),
            previous:  ReferenceObject::Absent,
            proposed:  ReferenceObject::Object(upper.proposed.clone()),
        };
        let rewrites = super::branch_rewrites(root, &[&unrelated, &own])?;
        let events = super::capture_branch_rewrites(root, &rewrites)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].branch.reference, upper.reference);
        assert_eq!(events[0].branch.previous, upper.previous);
        let markers = super::permit::pending_branch_rewrite_markers(&root.join(".git"))?;
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].rewrite.new_tips, vec![upper.proposed.clone()]);
        Ok(())
    }

    #[test]
    fn stopped_detached_apply_rebase_skips_branch_creation() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        let root = fixture.directory.path();
        git_output(
            root,
            &["checkout", "--quiet", "--detach", &fixture.base.to_string()],
        )?;
        fixture.write_stopped_apply_rebase(1)?;
        let directory = root.join(".git/rebase-apply");
        fs::write(directory.join("head-name"), "detached HEAD\n")?;
        // Branch creation must skip before trying to read the rebase's original tip.
        fs::remove_file(directory.join("orig-head"))?;
        git_output(root, &["branch", "side"])?;
        let update = ReferenceUpdate {
            reference: "refs/heads/side".parse()?,
            previous:  ReferenceObject::Absent,
            proposed:  ReferenceObject::Object(fixture.base.clone()),
        };
        assert!(super::branch_reflog_tip(root, &update.reference, 1).is_err());
        let rewrites = super::branch_rewrites(root, &[&update])?;
        assert!(rewrites.is_empty());
        assert!(super::capture_branch_rewrites(root, &rewrites)?.is_empty());
        assert!(super::permit::pending_branch_rewrite_markers(&root.join(".git"))?.is_empty());
        Ok(())
    }

    #[test]
    fn stopped_apply_recovery_needs_no_branch_reflog() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_stopped_apply_rebase(1)?;
        let upper = &fixture.branches[1];
        let root = fixture.directory.path();
        git_output(
            root,
            &[
                "update-ref",
                &upper.reference.to_string(),
                &upper.proposed.to_string(),
            ],
        )?;
        git_output(
            root,
            &[
                "reflog",
                "expire",
                "--expire=all",
                &upper.reference.to_string(),
            ],
        )?;
        assert!(git_output(root, &["reflog", "show", &upper.reference.to_string()])?.is_empty());
        assert!(super::branch_reflog_tip(root, &upper.reference, 1).is_err());
        let events = capture_zero_previous_update(&fixture, 1)?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].branch.previous, upper.previous);
        assert!(events[0].created_commits.contains(&fixture.split_first));
        Ok(())
    }

    #[test]
    fn stopped_apply_missing_invalid_or_unreadable_metadata_fails_without_a_marker()
    -> Result<(), Box<dyn Error>> {
        for name in ["orig-head", "head-name"] {
            for failure in ["missing", "empty", "invalid", "unreadable"] {
                let fixture = StackedRewriteFixture::new()?;
                fixture.write_stopped_apply_rebase(1)?;
                let path = fixture
                    .directory
                    .path()
                    .join(".git/rebase-apply")
                    .join(name);
                fs::remove_file(&path)?;
                match failure {
                    "empty" => fs::write(&path, " \t\n")?,
                    "invalid" if name == "head-name" => {
                        fs::write(&path, "refs/heads/invalid branch\n")?;
                    },
                    "invalid" => fs::write(&path, "invalid metadata\n")?,
                    "unreadable" => fs::create_dir(&path)?,
                    _ => {},
                }
                assert!(
                    capture_zero_previous_update(&fixture, 1).is_err(),
                    "{name}: {failure}"
                );
                assert!(
                    super::permit::pending_branch_rewrite_markers(
                        &fixture.directory.path().join(".git")
                    )?
                    .is_empty(),
                    "{name}: {failure}"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn stopped_apply_original_tip_must_contain_every_old_map_commit() -> Result<(), Box<dyn Error>>
    {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_stopped_apply_rebase(1)?;
        fs::write(
            fixture.directory.path().join(".git/rebase-apply/orig-head"),
            fixture.branches[0].previous.to_string(),
        )?;
        assert_zero_previous_capture_failure(&fixture, 1, "does not contain every old map commit")
    }

    #[test]
    fn zero_previous_without_an_apply_map_keeps_skipping_capture() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        assert!(capture_zero_previous_update(&fixture, 1)?.is_empty());
        assert!(
            super::permit::pending_branch_rewrite_markers(&fixture.directory.path().join(".git"))?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn mismatched_newest_apply_reflog_entry_fails_capture_without_a_marker()
    -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_rebase_map("rebase-apply", "rewritten")?;
        // The branch still names its old tip, so its newest entry cannot prove the update.
        assert_zero_previous_capture_failure(&fixture, 1, "does not end at the proposed tip")
    }

    #[test]
    fn apply_reflog_previous_must_contain_every_old_map_commit() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_rebase_map("rebase-apply", "rewritten")?;
        let lower = &fixture.branches[0];
        git_output(
            fixture.directory.path(),
            &[
                "update-ref",
                &lower.reference.to_string(),
                &lower.proposed.to_string(),
            ],
        )?;
        assert_zero_previous_capture_failure(&fixture, 0, "does not contain every old map commit")
    }

    #[test]
    fn missing_apply_branch_reflog_fails_capture_without_a_marker() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_rebase_map("rebase-apply", "rewritten")?;
        let upper = &fixture.branches[1];
        git_output(
            fixture.directory.path(),
            &[
                "update-ref",
                &upper.reference.to_string(),
                &upper.proposed.to_string(),
            ],
        )?;
        git_output(
            fixture.directory.path(),
            &[
                "reflog",
                "expire",
                "--expire=all",
                &fixture.branches[1].reference.to_string(),
            ],
        )?;
        assert_zero_previous_capture_failure(&fixture, 1, "rev-parse")
    }

    /// Exercise committed branch discovery and capture with Git's omitted previous value.
    fn capture_zero_previous_update(
        fixture: &StackedRewriteFixture,
        branch: usize,
    ) -> Result<Vec<super::BranchRewriteEvent>, super::GateError> {
        let rewrite = &fixture.branches[branch];
        let update = ReferenceUpdate {
            reference: rewrite.reference.clone(),
            previous:  ReferenceObject::Absent,
            proposed:  ReferenceObject::Object(rewrite.proposed.clone()),
        };
        let rewrites = super::branch_rewrites(fixture.directory.path(), &[&update])?;
        super::capture_branch_rewrites(fixture.directory.path(), &rewrites)
    }

    /// Refuse unverifiable previous history before any branch marker can be persisted.
    fn assert_zero_previous_capture_failure(
        fixture: &StackedRewriteFixture,
        branch: usize,
        reason: &str,
    ) -> Result<(), Box<dyn Error>> {
        let error = capture_zero_previous_update(fixture, branch)
            .err()
            .ok_or_else(|| std::io::Error::other("unverifiable apply capture succeeded"))?;
        assert!(error.to_string().contains(reason), "{error}");
        assert!(
            super::permit::pending_branch_rewrite_markers(&fixture.directory.path().join(".git"))?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn branch_markers_keep_only_pairs_in_their_created_history() -> Result<(), Box<dyn Error>> {
        for stacked in [false, true] {
            let directory = tempdir()?;
            let root = directory.path();
            git_output(root, &["init", "--quiet", "--initial-branch", "main"])?;
            git_output(root, &["config", "user.name", "Test User"])?;
            git_output(root, &["config", "user.email", "test@example.com"])?;
            let tree = git_output(root, &["mktree"])?;
            let base = git_output(root, &["commit-tree", &tree, "-m", "base"])?;
            let old_a = git_output(root, &["commit-tree", &tree, "-p", &base, "-m", "old a"])?;
            let old_b_base = if stacked { &old_a } else { &base };
            let old_b = git_output(
                root,
                &["commit-tree", &tree, "-p", old_b_base, "-m", "old b"],
            )?;
            let new_a = git_output(root, &["commit-tree", &tree, "-p", &base, "-m", "new a"])?;
            let new_b_base = if stacked { &new_a } else { &base };
            let new_b = git_output(
                root,
                &["commit-tree", &tree, "-p", new_b_base, "-m", "new b"],
            )?;
            for (reference, tip) in [
                ("refs/heads/main", &base),
                ("refs/heads/a", &new_a),
                ("refs/heads/b", &new_b),
            ] {
                git_output(root, &["update-ref", reference, tip])?;
            }
            let rewrites = [
                BranchRewrite {
                    reference: "refs/heads/a".parse()?,
                    previous:  old_a.parse()?,
                    proposed:  new_a.parse()?,
                },
                BranchRewrite {
                    reference: "refs/heads/b".parse()?,
                    previous:  old_b.parse()?,
                    proposed:  new_b.parse()?,
                },
            ];
            let rebase = root.join(".git/rebase-merge");
            fs::create_dir(&rebase)?;
            fs::write(rebase.join("onto"), &base)?;
            fs::write(
                rebase.join("rewritten-list"),
                format!("{old_a} {new_a}\n{old_b} {new_b}\n"),
            )?;
            let events = super::capture_branch_rewrites(root, &rewrites)?;
            assert_eq!(events.len(), 2);
            let markers = super::permit::pending_branch_rewrite_markers(&root.join(".git"))?;
            assert_eq!(markers.len(), 2);
            for (event, marker) in events.iter().zip(&markers) {
                assert_eq!(event.pairs, marker.rewrite.pairs);
                assert_eq!(event.created_commits, marker.rewrite.created_commits);
            }
            assert_eq!(
                markers[0].rewrite.pairs,
                vec![RewriteMapPair {
                    old: old_a.parse()?,
                    new: new_a.parse()?,
                }]
            );
            let expected_b = if stacked { 2 } else { 1 };
            assert_eq!(markers[1].rewrite.pairs.len(), expected_b);
            assert!(
                markers[1]
                    .rewrite
                    .pairs
                    .iter()
                    .any(|pair| pair.old.to_string() == old_b)
            );
            assert_eq!(
                markers[1]
                    .rewrite
                    .pairs
                    .iter()
                    .any(|pair| pair.old.to_string() == old_a),
                stacked
            );
        }
        Ok(())
    }

    #[test]
    fn upper_capture_keeps_the_split_after_lower_moves_first() -> Result<(), Box<dyn Error>> {
        assert_stacked_capture_order([0, 1])
    }

    #[test]
    fn lower_capture_keeps_the_split_after_upper_moves_first() -> Result<(), Box<dyn Error>> {
        assert_stacked_capture_order([1, 0])
    }

    /// Exercise separate committed transactions sharing one still-live rebase map.
    fn assert_stacked_capture_order(order: [usize; 2]) -> Result<(), Box<dyn Error>> {
        for (backend, name) in [
            ("rebase-merge", "rewritten-list"),
            ("rebase-apply", "rewritten"),
        ] {
            let fixture = StackedRewriteFixture::new()?;
            fixture.write_stopped_rebase(backend, name)?;
            for index in order {
                let rewrite = &fixture.branches[index];
                git_output(
                    fixture.directory.path(),
                    &[
                        "update-ref",
                        &rewrite.reference.to_string(),
                        &rewrite.proposed.to_string(),
                    ],
                )?;
                let events = super::capture_branch_rewrites(
                    fixture.directory.path(),
                    std::slice::from_ref(rewrite),
                )?;
                assert_eq!(events.len(), 1);
                let event = &events[0];
                let mut expected_created = vec![
                    fixture.branches[0].proposed.clone(),
                    fixture.split_first.clone(),
                ];
                if index == 1 {
                    expected_created.insert(0, fixture.branches[1].proposed.clone());
                }
                assert_eq!(event.created_commits, expected_created);
                assert_eq!(event.pairs, fixture.pairs()[..=index]);
                let markers = super::permit::pending_branch_rewrite_markers(
                    &fixture.directory.path().join(".git"),
                )?;
                let marker = markers
                    .iter()
                    .find(|marker| marker.rewrite.new_tips == vec![rewrite.proposed.clone()])
                    .ok_or_else(|| std::io::Error::other("branch marker missing"))?;
                assert_eq!(marker.rewrite.created_commits, expected_created);
                assert_eq!(marker.rewrite.pairs, event.pairs);
            }
        }
        Ok(())
    }

    #[test]
    fn invalid_or_unreadable_onto_fails_capture_without_a_marker() -> Result<(), Box<dyn Error>> {
        for (backend, name) in [
            ("rebase-merge", "rewritten-list"),
            ("rebase-apply", "rewritten"),
        ] {
            for failure in ["invalid", "unreadable"] {
                assert_onto_failure(backend, name, failure)?;
            }
        }
        Ok(())
    }

    #[test]
    fn merge_map_without_onto_fails_capture_without_a_marker() -> Result<(), Box<dyn Error>> {
        assert_onto_failure("rebase-merge", "rewritten-list", "missing")
    }

    /// Reject a backend's damaged boundary even when the other backend has a valid one.
    fn assert_onto_failure(backend: &str, name: &str, failure: &str) -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        fixture.write_stopped_rebase(backend, name)?;
        let onto = fixture
            .directory
            .path()
            .join(".git")
            .join(backend)
            .join("onto");
        fs::remove_file(&onto)?;
        match failure {
            "invalid" => fs::write(&onto, "not an object id\n")?,
            "unreadable" => fs::create_dir(&onto)?,
            _ => {},
        }
        let other_backend = if backend == "rebase-merge" {
            "rebase-apply"
        } else {
            "rebase-merge"
        };
        let other_directory = fixture.directory.path().join(".git").join(other_backend);
        fs::create_dir(&other_directory)?;
        fs::write(other_directory.join("onto"), fixture.base.to_string())?;
        assert!(
            super::capture_branch_rewrites(fixture.directory.path(), &fixture.branches).is_err(),
            "{backend}: {failure}"
        );
        assert!(
            super::permit::pending_branch_rewrite_markers(&fixture.directory.path().join(".git"))?
                .is_empty()
        );
        Ok(())
    }

    #[test]
    fn tip_replacement_without_a_map_captures_the_proposed_commit() -> Result<(), Box<dyn Error>> {
        let fixture = StackedRewriteFixture::new()?;
        let root = fixture.directory.path();
        let tree = git_output(root, &["mktree"])?;
        let proposed = git_output(
            root,
            &[
                "commit-tree",
                &tree,
                "-p",
                &fixture.base.to_string(),
                "-m",
                "amended lower",
            ],
        )?;
        let rewrite = BranchRewrite {
            reference: fixture.branches[0].reference.clone(),
            previous:  fixture.branches[0].previous.clone(),
            proposed:  proposed.parse()?,
        };
        git_output(
            root,
            &["update-ref", &rewrite.reference.to_string(), &proposed],
        )?;
        let events = super::capture_branch_rewrites(root, std::slice::from_ref(&rewrite))?;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].created_commits, vec![rewrite.proposed.clone()]);
        assert_eq!(
            events[0].pairs,
            vec![RewriteMapPair {
                old: rewrite.previous,
                new: rewrite.proposed,
            }]
        );
        let markers = super::permit::pending_branch_rewrite_markers(&root.join(".git"))?;
        assert_eq!(markers.len(), 1);
        assert_eq!(
            markers[0].rewrite.created_commits,
            events[0].created_commits
        );
        Ok(())
    }

    /// A split lower commit and its replayed upper commit before either branch moves.
    struct StackedRewriteFixture {
        /// The repository retaining the old and rewritten commit graphs.
        directory:   TempDir,
        /// The operation boundary shared by both branch events.
        base:        GitObjectId,
        /// The unrecorded first half of the lower commit's split.
        split_first: GitObjectId,
        /// The lower and upper branch rewrites, in that order.
        branches:    [BranchRewrite; 2],
    }

    impl StackedRewriteFixture {
        /// Build T->A->B and T->S1->S2->B' without moving either old branch.
        fn new() -> Result<Self, Box<dyn Error>> {
            let directory = tempdir()?;
            let root = directory.path();
            git_output(root, &["init", "--quiet", "--initial-branch", "main"])?;
            git_output(root, &["config", "user.name", "Test User"])?;
            git_output(root, &["config", "user.email", "test@example.com"])?;
            let tree = git_output(root, &["mktree"])?;
            let base = git_output(root, &["commit-tree", &tree, "-m", "base"])?;
            let commit = |parent: &str, message: &str| {
                git_output(root, &["commit-tree", &tree, "-p", parent, "-m", message])
            };
            let old_lower = commit(&base, "old lower")?;
            let old_upper = commit(&old_lower, "old upper")?;
            let split_first = commit(&base, "first split")?;
            let new_lower = commit(&split_first, "second split")?;
            let new_upper = commit(&new_lower, "rewritten upper")?;
            for (reference, tip) in [
                ("refs/heads/main", &base),
                ("refs/heads/lower", &old_lower),
                ("refs/heads/upper", &old_upper),
            ] {
                git_output(root, &["update-ref", reference, tip])?;
            }
            Ok(Self {
                directory,
                base: base.parse()?,
                split_first: split_first.parse()?,
                branches: [
                    BranchRewrite {
                        reference: "refs/heads/lower".parse()?,
                        previous:  old_lower.parse()?,
                        proposed:  new_lower.parse()?,
                    },
                    BranchRewrite {
                        reference: "refs/heads/upper".parse()?,
                        previous:  old_upper.parse()?,
                        proposed:  new_upper.parse()?,
                    },
                ],
            })
        }

        /// Read the complete operation map, including the other branch's nomination.
        fn pairs(&self) -> Vec<RewriteMapPair> {
            self.branches
                .iter()
                .map(|branch| RewriteMapPair {
                    old: branch.previous.clone(),
                    new: branch.proposed.clone(),
                })
                .collect()
        }

        /// Record a stopped rebase's map and base; the apply backend writes onto on stopping.
        fn write_stopped_rebase(&self, backend: &str, name: &str) -> Result<(), Box<dyn Error>> {
            self.write_rebase_map(backend, name)?;
            let directory = self.directory.path().join(".git").join(backend);
            fs::write(directory.join("onto"), format!("{}\n", self.base))?;
            Ok(())
        }

        /// Record the branch identity and original tip that a stopped apply rebase saves.
        fn write_stopped_apply_rebase(&self, branch: usize) -> Result<(), Box<dyn Error>> {
            self.write_stopped_rebase("rebase-apply", "rewritten")?;
            let directory = self.directory.path().join(".git/rebase-apply");
            fs::write(
                directory.join("head-name"),
                format!("{}\n", self.branches[branch].reference),
            )?;
            fs::write(
                directory.join("orig-head"),
                format!("{}\n", self.branches[branch].previous),
            )?;
            Ok(())
        }

        /// Record only the map, as an uninterrupted apply backend does.
        fn write_rebase_map(&self, backend: &str, name: &str) -> Result<(), Box<dyn Error>> {
            let directory = self.directory.path().join(".git").join(backend);
            fs::create_dir(&directory)?;
            let mut contents = String::new();
            for pair in self.pairs() {
                writeln!(contents, "{} {}", pair.old, pair.new)?;
            }
            fs::write(directory.join(name), contents)?;
            Ok(())
        }
    }

    /// Run a fixture Git command, keeping failures visible to the unit test.
    fn git_output(root: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
        let output = std::process::Command::new("git")
            .args(arguments)
            .current_dir(root)
            .output()?;
        if !output.status.success() {
            return Err(std::io::Error::other(String::from_utf8_lossy(&output.stderr)).into());
        }
        Ok(String::from_utf8(output.stdout)?.trim().to_owned())
    }
}
