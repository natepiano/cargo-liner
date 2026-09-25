//! Enrollment of existing work into the shared reservation journal.

use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;

use super::WorktreeRegistry;
use super::constants::ENROLLMENT_SEQUENCE_REASON;
use super::constants::MERGE_BASE_NO_COMMON_ANCESTOR_EXIT_CODE;
use super::constants::OPERATION_IN_PROGRESS_MARKERS;
use super::liveness::WorktreeEnrollmentCandidate;
use crate::answer::AuthorizedOverlap;
use crate::answer::AuthorizedOverlapSet;
use crate::answer::ConflictAuthorization;
use crate::config::BerthConfig;
use crate::coordination_identity::CoordinationIdentityProvenance;
use crate::drift;
use crate::drift::WorkingTreeFingerprint;
use crate::git;
use crate::git::GitCommandOutputAvailability;
use crate::git::GitError;
use crate::git::HeadAttachment;
use crate::ids::CoordinationRunId;
use crate::ids::GitObjectId;
use crate::ids::ReservationId;
use crate::ids::ReservationScopePath;
use crate::ids::WorktreeId;
use crate::ledger;
use crate::ledger::CanonicalWorktreeRoot;
use crate::ledger::ClaimHeadSnapshot;
use crate::ledger::ClaimSource;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::LedgerTransactionOutcome;
use crate::ledger::ReservationPurpose;
use crate::ledger::ReservationScope;
use crate::ledger::ReservationScopeSet;
use crate::ledger::ScopeKind;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::reservation::ActingHeadContainment;
use crate::reservation::RetainedReservationSet;
use crate::scope::DeclaredReservationScopeSet;
use crate::scope::PathCase;

/// Reservations acquired and unresolved ordering decisions discovered by initialization.
#[derive(Clone, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct WorktreeEnrollmentReport {
    /// Worktrees that acquired their first reservation in this invocation.
    pub(crate) enrolled: Vec<EnrolledWorktree>,
    /// Enrollment deferrals still awaiting an explicit sequence decision.
    pub(crate) overlaps: Vec<EnrollmentOverlap>,
    /// Candidates whose enrollment could not be completed.
    pub(crate) failures: Vec<WorktreeEnrollmentFailure>,
}

/// A worktree's first reservation, protecting its complete current footprint.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct EnrolledWorktree {
    /// The newly acquired reservation.
    pub(crate) reservation_id: ReservationId,
    /// The checkout containing the work.
    pub(crate) worktree_root:  PathBuf,
    /// The branch name, or a description of the detached HEAD.
    pub(crate) branch:         String,
}

/// One unresolved pair created by enrollment, including either ordering command.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct EnrollmentOverlap {
    /// One endpoint of the pending pair.
    pub(crate) first_reservation_id:  ReservationId,
    /// The other endpoint of the pending pair.
    pub(crate) second_reservation_id: ReservationId,
    /// Paths for which an ordering decision is still required.
    pub(crate) shared_scopes:         ReservationScopeSet,
    /// Ready-to-run sequence commands, one for each order.
    pub(crate) sequence_commands:     [String; 2],
}

/// Why a registered checkout could not be enrolled.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorktreeEnrollmentFailureReason {
    /// A merge, rebase, cherry-pick, or revert is underway.
    OperationInProgress,
    /// HEAD and trunk have no resolvable common ancestor.
    NoMergeBase,
    /// A repository observation or publication failed.
    GitFailure,
    /// Git retains a registration whose checkout cannot be used.
    Unavailable,
    /// The complete claim exceeds the journal's single-record limit.
    RecordTooLarge,
}

impl WorktreeEnrollmentFailureReason {
    /// The stable failure label shared by text and structured reports.
    pub(crate) const fn as_str(&self) -> &'static str {
        match self {
            Self::OperationInProgress => "operation_in_progress",
            Self::NoMergeBase => "no_merge_base",
            Self::GitFailure => "git_failure",
            Self::Unavailable => "unavailable",
            Self::RecordTooLarge => "record_too_large",
        }
    }
}

/// A failure confined to one candidate; other worktrees continue enrollment.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
pub(crate) struct WorktreeEnrollmentFailure {
    /// The registered checkout that could not be enrolled.
    pub(crate) worktree_root: PathBuf,
    /// The semantic failure category.
    pub(crate) reason:        WorktreeEnrollmentFailureReason,
    /// Details explaining the failed observation or append.
    pub(crate) diagnostic:    String,
}

/// The complete work observed before any enrollment transaction takes its lock.
struct EnrollmentFootprint {
    context:       WorktreeContext,
    worktree_id:   WorktreeId,
    history:       ReservationHistory,
    head:          GitObjectId,
    trunk:         GitObjectId,
    merge_base:    GitObjectId,
    head_snapshot: ClaimHeadSnapshot,
    branch:        String,
    scopes:        ReservationScopeSet,
    working_tree:  WorkingTreeFingerprint,
}

/// Claim history permanently excludes a worktree from automatic acquisition.
#[derive(Clone, Copy)]
enum ReservationHistory {
    NeverReserved,
    AlreadyReserved,
}

/// A successful observation may establish that a checkout has nothing to protect.
enum FootprintObservation {
    Empty,
    Work(Box<EnrollmentFootprint>),
}

/// The locked replay can skip a competing acquisition or reject invalid replay.
enum EnrollmentRejection {
    AlreadyReserved,
    InvalidReplay(String),
}

/// Paths both worktrees would still contribute to one another's current HEAD.
struct MutualWorkRemainder {
    candidate_paths: Vec<ReservationScopePath>,
    holder_paths:    Vec<ReservationScopePath>,
}

impl MutualWorkRemainder {
    fn contains(&self, scope: &ReservationScope, path_case: PathCase) -> bool {
        [&self.candidate_paths, &self.holder_paths]
            .iter()
            .all(|paths| {
                paths.iter().any(|path| {
                    scope.overlaps(
                        &ReservationScope {
                            path: path.clone(),
                            kind: ScopeKind::File,
                        },
                        path_case,
                    )
                })
            })
    }
}

/// A history race is an ordinary skip, distinct from a committed acquisition.
enum EnrollmentOutcome {
    AlreadyReserved,
    Enrolled(EnrolledWorktree),
}

/// Enroll never-reserved registered worktrees after configuration and hook installation.
pub(crate) fn enroll_worktrees(
    context: &WorktreeContext,
    config: &BerthConfig,
) -> Result<WorktreeEnrollmentReport, LedgerError> {
    let ledger = Ledger::open_from_discovered_worktree(context)?;
    let events = ledger.read_validated_events()?;
    let mut report = WorktreeEnrollmentReport::default();
    let registry = match WorktreeRegistry::read(context) {
        Ok(registry) => registry,
        Err(error) => {
            report.failures.push(failure(
                context.repository_root(),
                WorktreeEnrollmentFailureReason::GitFailure,
                format!("git worktree list --porcelain: {error}"),
            ));
            report.overlaps = unresolved_enrollment_overlaps(&events);
            return Ok(report);
        },
    };
    let path_case = PathCase::read(context.common_git_directory())
        .map_err(|error| LedgerError::Io(std::io::Error::other(error.to_string())))?;
    let mut footprints = Vec::new();
    for candidate in registry.enrollment_candidates() {
        match candidate {
            WorktreeEnrollmentCandidate::Unavailable { root } => {
                report.failures.push(failure(
                    &root,
                    WorktreeEnrollmentFailureReason::Unavailable,
                    "registered worktree is prunable or its checkout is unavailable".to_owned(),
                ));
            },
            WorktreeEnrollmentCandidate::Eligible(candidate_context) => {
                let identity = match ledger::worktree_identity(
                    candidate_context.administrative_directory(),
                    candidate_context.worktree_kind(),
                ) {
                    Ok(identity) => identity,
                    Err(error) => {
                        report.failures.push(failure(
                            candidate_context.repository_root(),
                            WorktreeEnrollmentFailureReason::Unavailable,
                            error.to_string(),
                        ));
                        continue;
                    },
                };
                let history = reservation_history(&events, identity.id);
                match observe_footprint(candidate_context, identity.id, history, config, path_case)
                {
                    Ok(FootprintObservation::Work(footprint)) => footprints.push(*footprint),
                    Err(error) if matches!(history, ReservationHistory::NeverReserved) => {
                        report.failures.push(error);
                    },
                    Ok(FootprintObservation::Empty) | Err(_) => {},
                }
            },
        }
    }
    for candidate in &footprints {
        if matches!(candidate.history, ReservationHistory::AlreadyReserved) {
            continue;
        }
        match enroll_candidate(&ledger, candidate, &footprints, path_case) {
            Ok(EnrollmentOutcome::Enrolled(enrolled)) => report.enrolled.push(enrolled),
            Ok(EnrollmentOutcome::AlreadyReserved) => {},
            Err(error) => report.failures.push(error),
        }
    }
    report.overlaps = unresolved_enrollment_overlaps(&ledger.read_validated_events()?);
    Ok(report)
}

fn reservation_history(events: &[JournalEvent], worktree_id: WorktreeId) -> ReservationHistory {
    if events.iter().any(|event| {
        event.actor.worktree == worktree_id
            && matches!(event.operation, JournalOperation::Claim { .. })
    }) {
        ReservationHistory::AlreadyReserved
    } else {
        ReservationHistory::NeverReserved
    }
}

fn observe_footprint(
    context: WorktreeContext,
    worktree_id: WorktreeId,
    history: ReservationHistory,
    config: &BerthConfig,
    path_case: PathCase,
) -> Result<FootprintObservation, WorktreeEnrollmentFailure> {
    let root = context.repository_root();
    for marker in OPERATION_IN_PROGRESS_MARKERS {
        if context.administrative_directory().join(marker).exists() {
            return Err(failure(
                root,
                WorktreeEnrollmentFailureReason::OperationInProgress,
                format!("{marker} exists in the worktree administrative directory"),
            ));
        }
    }
    let head =
        git::head_object_id(root).map_err(|error| resolution_failure(root, "HEAD", &error))?;
    let trunk = git::branch_object_id(root, &config.trunk)
        .map_err(|error| resolution_failure(root, &config.trunk, &error))?;
    let merge_base = observe_merge_base(root, &trunk, &head)?;
    let committed = git::unmerged_branch_paths(root, &trunk, &head).map_err(|error| {
        failure(
            root,
            WorktreeEnrollmentFailureReason::GitFailure,
            format!("git merge-tree {trunk} {head}: {error}"),
        )
    })?;
    let working_tree = drift::observe_merge_working_tree(root).map_err(|error| {
        failure(
            root,
            WorktreeEnrollmentFailureReason::GitFailure,
            format!("git status --porcelain: {error}"),
        )
    })?;
    let paths = committed
        .into_iter()
        .chain(working_tree.tracked_paths.iter().cloned())
        .chain(working_tree.untracked_paths.iter().cloned())
        .collect();
    let Ok(scopes) = DeclaredReservationScopeSet::from_file_paths(paths) else {
        return Ok(FootprintObservation::Empty);
    };
    let scopes = scopes.into_exact_file_antichain(path_case);
    let (head_snapshot, branch) = match git::head_attachment(root).map_err(|error| {
        failure(
            root,
            WorktreeEnrollmentFailureReason::GitFailure,
            format!("git symbolic-ref HEAD: {error}"),
        )
    })? {
        HeadAttachment::Branch { full_ref } => {
            let branch = full_ref.to_string();
            (
                ClaimHeadSnapshot::Branch {
                    full_ref,
                    head: head.clone().into(),
                },
                branch,
            )
        },
        HeadAttachment::Detached => (
            ClaimHeadSnapshot::Detached {
                head: head.clone().into(),
            },
            format!("detached at {head}"),
        ),
    };
    Ok(FootprintObservation::Work(Box::new(EnrollmentFootprint {
        context,
        worktree_id,
        history,
        head,
        trunk,
        merge_base,
        head_snapshot,
        branch,
        scopes,
        working_tree,
    })))
}

fn resolution_failure(root: &Path, reference: &str, error: &GitError) -> WorktreeEnrollmentFailure {
    let reason = match error {
        GitError::CommandFailed { .. } => WorktreeEnrollmentFailureReason::NoMergeBase,
        _ => WorktreeEnrollmentFailureReason::GitFailure,
    };
    failure(root, reason, format!("git rev-parse {reference}: {error}"))
}

fn observe_merge_base(
    root: &Path,
    trunk: &GitObjectId,
    head: &GitObjectId,
) -> Result<GitObjectId, WorktreeEnrollmentFailure> {
    let command = format!("git merge-base {trunk} {head}");
    match git::execute_read_only_git(root, &["merge-base", &trunk.to_string(), &head.to_string()]) {
        GitCommandOutputAvailability::Available(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout)
                .trim()
                .parse()
                .map_err(|error| {
                    failure(
                        root,
                        WorktreeEnrollmentFailureReason::GitFailure,
                        format!("{command}: {error}"),
                    )
                })
        },
        GitCommandOutputAvailability::Available(output) => {
            let reason = if output.status.code() == Some(MERGE_BASE_NO_COMMON_ANCESTOR_EXIT_CODE) {
                WorktreeEnrollmentFailureReason::NoMergeBase
            } else {
                WorktreeEnrollmentFailureReason::GitFailure
            };
            Err(failure(
                root,
                reason,
                format!(
                    "{command}: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ))
        },
        GitCommandOutputAvailability::Unavailable(error) => Err(failure(
            root,
            WorktreeEnrollmentFailureReason::GitFailure,
            format!("{command}: {error}"),
        )),
    }
}

fn failure(
    root: &Path,
    reason: WorktreeEnrollmentFailureReason,
    diagnostic: String,
) -> WorktreeEnrollmentFailure {
    WorktreeEnrollmentFailure {
        worktree_root: root.to_path_buf(),
        reason,
        diagnostic,
    }
}

fn mutual_remainders(
    candidate: &EnrollmentFootprint,
    footprints: &[EnrollmentFootprint],
) -> Result<HashMap<WorktreeId, MutualWorkRemainder>, WorktreeEnrollmentFailure> {
    let mut remainders = HashMap::new();
    for holder in footprints {
        if holder.worktree_id == candidate.worktree_id {
            continue;
        }
        let candidate_paths = remaining_work(candidate, &holder.head)?;
        let holder_paths = remaining_work(holder, &candidate.head).map_err(|mut error| {
            error.worktree_root = candidate.context.repository_root().to_path_buf();
            error
        })?;
        remainders.insert(
            holder.worktree_id,
            MutualWorkRemainder {
                candidate_paths,
                holder_paths,
            },
        );
    }
    Ok(remainders)
}

fn remaining_work(
    worktree: &EnrollmentFootprint,
    other_head: &GitObjectId,
) -> Result<Vec<ReservationScopePath>, WorktreeEnrollmentFailure> {
    let mut paths = git::unmerged_branch_paths(
        worktree.context.repository_root(),
        other_head,
        &worktree.head,
    )
    .map_err(|error| {
        failure(
            worktree.context.repository_root(),
            WorktreeEnrollmentFailureReason::GitFailure,
            format!("git merge-tree {other_head} {}: {error}", worktree.head),
        )
    })?;
    paths.extend(worktree.working_tree.tracked_paths.iter().cloned());
    paths.extend(worktree.working_tree.untracked_paths.iter().cloned());
    Ok(paths)
}

fn enroll_candidate(
    ledger: &Ledger,
    candidate: &EnrollmentFootprint,
    footprints: &[EnrollmentFootprint],
    path_case: PathCase,
) -> Result<EnrollmentOutcome, WorktreeEnrollmentFailure> {
    let root = candidate.context.repository_root();
    let git_failure = |diagnostic| {
        failure(
            root,
            WorktreeEnrollmentFailureReason::GitFailure,
            diagnostic,
        )
    };
    let events = ledger
        .read_validated_events()
        .map_err(|error| git_failure(error.to_string()))?;
    let reservations =
        RetainedReservationSet::replay(&events).map_err(|error| git_failure(error.to_string()))?;
    let containment = ActingHeadContainment::observe_at_head(
        &reservations,
        root,
        candidate.worktree_id,
        &candidate.head,
    );
    let remainders = mutual_remainders(candidate, footprints)?;
    let worktree_root: CanonicalWorktreeRoot = root
        .to_str()
        .ok_or_else(|| git_failure("worktree root is not UTF-8".to_owned()))?
        .parse::<CanonicalWorktreeRoot>()
        .map_err(|error| git_failure(error.to_string()))?;
    let purpose = ReservationPurpose::Explained(
        format!("Enroll existing work on {}", candidate.branch)
            .parse()
            .map_err(|error| git_failure(format!("enrollment purpose: {error}")))?,
    );
    let reservation_id = ReservationId::new();
    let run = CoordinationRunId::new();
    let outcome = ledger
        .transact(candidate.worktree_id, run, |state| {
            if matches!(
                reservation_history(state.events(), candidate.worktree_id),
                ReservationHistory::AlreadyReserved
            ) {
                return TransactionValidation::Reject(EnrollmentRejection::AlreadyReserved);
            }
            let reservations = match RetainedReservationSet::replay(state.events()) {
                Ok(reservations) => reservations.with_acting_head_containment(containment),
                Err(error) => {
                    return TransactionValidation::Reject(EnrollmentRejection::InvalidReplay(
                        error.to_string(),
                    ));
                },
            };
            let authorization =
                enrollment_authorization(&reservations, candidate, &remainders, path_case);
            TransactionValidation::Append(Box::new(JournalOperation::Claim {
                reservation_id,
                scopes: candidate.scopes.clone(),
                source: ClaimSource::Enrolled,
                purpose,
                trunk_at_claim: candidate.trunk.clone().into(),
                head_snapshot: candidate.head_snapshot.clone(),
                phase_start_head: candidate.merge_base.clone().into(),
                worktree_root,
                worktree_administrative_locator: candidate.context.administrative_locator().clone(),
                authorization,
                coordination_identity_provenance: CoordinationIdentityProvenance::NotPresented,
            }))
        })
        .map_err(|error| {
            let reason = match &error {
                LedgerTransactionError::CorrectableInput(_) => {
                    WorktreeEnrollmentFailureReason::RecordTooLarge
                },
                _ => WorktreeEnrollmentFailureReason::GitFailure,
            };
            failure(root, reason, error.to_string())
        })?;
    match outcome {
        LedgerTransactionOutcome::Rejected(EnrollmentRejection::AlreadyReserved) => {
            Ok(EnrollmentOutcome::AlreadyReserved)
        },
        LedgerTransactionOutcome::Rejected(EnrollmentRejection::InvalidReplay(error)) => {
            Err(git_failure(error))
        },
        LedgerTransactionOutcome::Appended { .. } => {
            candidate
                .context
                .publish_coordination_run_marker(run)
                .map_err(|error| {
                    git_failure(format!("publish coordination run marker: {error}"))
                })?;
            Ok(EnrollmentOutcome::Enrolled(EnrolledWorktree {
                reservation_id,
                worktree_root: root.to_path_buf(),
                branch: candidate.branch.clone(),
            }))
        },
    }
}

fn enrollment_authorization(
    reservations: &RetainedReservationSet,
    candidate: &EnrollmentFootprint,
    remainders: &HashMap<WorktreeId, MutualWorkRemainder>,
    path_case: PathCase,
) -> ConflictAuthorization {
    let overlaps = reservations
        .conflicts_for_claim(&candidate.scopes, candidate.worktree_id, path_case)
        .into_iter()
        .filter_map(|mut conflict| {
            if let Some(remainder) = remainders.get(&conflict.holder_worktree_id()) {
                let shared = conflict
                    .overlapping_scopes
                    .as_slice()
                    .iter()
                    .filter(|scope| remainder.contains(scope, path_case))
                    .cloned()
                    .collect::<Vec<_>>();
                let Ok(shared) = ReservationScopeSet::try_from(shared) else {
                    return None;
                };
                conflict.overlapping_scopes = shared;
            }
            Some(AuthorizedOverlap::from(&conflict))
        })
        .collect::<Vec<_>>();
    AuthorizedOverlapSet::try_from(overlaps).map_or(ConflictAuthorization::NoConflict, |overlaps| {
        ConflictAuthorization::Enrollment { overlaps }
    })
}

fn unresolved_enrollment_overlaps(events: &[JournalEvent]) -> Vec<EnrollmentOverlap> {
    let mut pending = Vec::new();
    for event in events {
        match &event.operation {
            JournalOperation::Claim {
                reservation_id,
                authorization: ConflictAuthorization::Enrollment { overlaps },
                ..
            }
            | JournalOperation::Widen {
                reservation_id,
                authorization: ConflictAuthorization::Enrollment { overlaps },
                ..
            } => {
                for overlap in overlaps.as_slice() {
                    let first = *reservation_id;
                    let second = overlap.reservation_id;
                    let Ok(shared_scopes) =
                        ReservationScopeSet::try_from(overlap.scopes.as_slice().to_vec())
                    else {
                        continue;
                    };
                    pending.push(EnrollmentOverlap {
                        first_reservation_id: first,
                        second_reservation_id: second,
                        shared_scopes,
                        sequence_commands: [
                            format!(
                                "cargo berth sequence {first} {second} --why '{ENROLLMENT_SEQUENCE_REASON}'"
                            ),
                            format!(
                                "cargo berth sequence {second} {first} --why '{ENROLLMENT_SEQUENCE_REASON}'"
                            ),
                        ],
                    });
                }
            },
            JournalOperation::ResolveDefer {
                deferred_reservation_id,
                blocker_reservation_id,
                ..
            } => {
                pending.retain(|pair| {
                    !(pair.first_reservation_id == *deferred_reservation_id
                        && pair.second_reservation_id == *blocker_reservation_id)
                });
            },
            _ => {},
        }
    }
    pending
}
