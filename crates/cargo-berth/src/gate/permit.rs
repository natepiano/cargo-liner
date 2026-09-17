//! One-use forced permits and the unconditional environment release valve.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use super::rewrite_map::RewriteMapPair;
use crate::config::BerthConfig;
use crate::config::Enrollment;
use crate::ids::CoordinationRunId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::GitObjectId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ledger;
use crate::ledger::BypassCause;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::BypassRecording;
use crate::ledger::BypassedAction;
use crate::ledger::BypassedMergeIdentity;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::ledger::PendingBypassMarkerId;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;
use crate::reservation::IntegrationProofSubjectRevision;

const BYPASS_ENVIRONMENT: &str = "CARGO_BERTH_BYPASS";
const BYPASS_ENVIRONMENT_ENABLED_VALUE: &str = "1";
const BYPASSED_MERGE_IDENTITY_ENVIRONMENT: &str = "CARGO_BERTH_BYPASSED_MERGE_ID";
pub(super) const PENDING_BYPASS_FILE_PREFIX: &str = "cargo-berth-pending-bypass-";
pub(super) const PENDING_BYPASS_FILE_SUFFIX: &str = ".json";

/// One unconsumed permit reconstructed from append-only truth.
#[derive(Clone)]
pub(crate) struct AvailableForcedIntegrationPermit {
    /// The stable one-use permit identity.
    pub(crate) permit_id:      ForcedIntegrationPermitId,
    /// The reservation whose next trunk entry this permit may authorize.
    pub(crate) reservation_id: ReservationId,
    /// The user's non-empty explanation.
    pub(crate) reason:         ForcedIntegrationReason,
    /// The exact holds visible when the permit was issued.
    pub(crate) skipped_holds:  SkippedIntegrationHoldSet,
}

/// The outcome of trying to retain an environment-bypass audit fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EnvironmentBypassRetentionOutcome {
    /// The audit fact is durable in the append-only journal.
    Journalled,
    /// The audit fact is durable as a marker for a later session to report.
    PendingMarker,
    /// The repository is not enrolled, so no shared audit destination applies.
    Unenrolled,
    /// The ref update was permitted, but neither durable audit destination accepted the fact.
    Unrecorded,
}

/// The shared marker schema used when an environment bypass cannot reach the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PendingEnvironmentBypass {
    /// The operation the bypass allowed; markers written before edits could be bypassed
    /// name no action and were always left by a trunk update.
    #[serde(default = "integration_bypass")]
    action:          BypassedAction,
    /// Why ordinary validation was bypassed.
    cause:           BypassCause,
    /// Whether the marker writer retained the override's occurrence time.
    occurrence_time: PendingEnvironmentBypassOccurrenceTime,
}

/// Durable facts awaiting reconciliation, including legacy untagged bypass payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
enum PendingReconciliationMarker {
    /// A branch rewrite whose map must survive Git removing its rebase state.
    BranchRewrite(PendingBranchRewrite),
    /// An environment override whose audit record still needs to be appended.
    EnvironmentBypass(PendingEnvironmentBypass),
}

/// The discriminator separating rewrite markers from legacy bypass payloads.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BranchRewriteMarkerKind {
    /// A rewrite map, which never produces an environment-bypass audit or notice.
    BranchRewrite,
}

/// Rewrite facts retained before any committed hook path takes the ledger lock.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PendingBranchRewrite {
    /// The marker's wire discriminator.
    kind:                                         BranchRewriteMarkerKind,
    /// Old-to-new pairs whose destinations this branch's rewrite created.
    pub(crate) pairs:                             Vec<RewriteMapPair>,
    /// The issuing checkout's administrative directory, including linked checkouts.
    pub(crate) worktree_administrative_directory: PathBuf,
    /// Committed branch destinations, independent of rewrite-pair order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) new_tips:                          Vec<GitObjectId>,
    /// Commits introduced by this branch's rewrite, fixed before any reconciliation.
    pub(crate) created_commits:                   Vec<GitObjectId>,
    /// Definitively evaluated subjects, so a deferred marker can advance on later passes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) completed_subjects:                Vec<CompletedRewriteSubject>,
}

/// A proof subject whose definitive rewrite verdict already consumed its turn.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct CompletedRewriteSubject {
    /// The reservation whose candidate was evaluated.
    pub(crate) reservation_id: ReservationId,
    /// The immutable proof revision used for that evaluation.
    pub(crate) subject:        IntegrationProofSubjectRevision,
}

/// A decoded rewrite marker and its deletion destination after successful reconciliation.
#[derive(Clone, Debug)]
pub(crate) struct PendingBranchRewriteMarker {
    /// The durable marker removed only after the journal append succeeds.
    pub(crate) path:    PathBuf,
    /// The rewrite map and the checkout whose rebase must finish before importing it.
    pub(crate) rewrite: PendingBranchRewrite,
}

const fn integration_bypass() -> BypassedAction { BypassedAction::Integration }

/// The only occurrence-time states a pending marker is permitted to record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PendingEnvironmentBypassOccurrenceTime {
    /// The marker writer retained the actual occurrence time.
    Known { at: RecordedAt },
    /// The fallback writer could not obtain a parseable time.
    Unavailable,
}

impl From<PendingEnvironmentBypassOccurrenceTime> for BypassOccurrenceTime {
    fn from(occurrence_time: PendingEnvironmentBypassOccurrenceTime) -> Self {
        match occurrence_time {
            PendingEnvironmentBypassOccurrenceTime::Known { at } => Self::Known { at },
            PendingEnvironmentBypassOccurrenceTime::Unavailable => Self::Unavailable,
        }
    }
}

/// Marker imports and marker alerts prepared from one locked journal replay.
pub(crate) struct PendingBypassRecovery {
    imports:                Vec<PendingBypassMarkerImport>,
    completed_markers:      Vec<RecoveredPendingBypassMarker>,
    unrecorded_occurrences: Vec<BypassOccurrenceTime>,
}

/// One decoded marker whose audit operation is still absent from the journal.
pub(crate) struct PendingBypassMarkerImport {
    operation:       JournalOperation,
    marker_id:       PendingBypassMarkerId,
    marker_path:     PathBuf,
    occurrence_time: BypassOccurrenceTime,
}

/// One pending marker whose audit fact is durable and whose file can be deleted.
pub(crate) struct RecoveredPendingBypassMarker {
    id:   PendingBypassMarkerId,
    path: PathBuf,
}

impl PendingBypassMarkerImport {
    /// Borrow the idempotent operation attempted for this marker.
    pub(crate) const fn operation(&self) -> &JournalOperation { &self.operation }

    /// Borrow the occurrence fact shown when this import still cannot be appended.
    pub(crate) const fn occurrence_time(&self) -> &BypassOccurrenceTime { &self.occurrence_time }

    /// Convert a successful import into a completed marker recovery.
    pub(crate) fn into_recovered_marker(self) -> RecoveredPendingBypassMarker {
        RecoveredPendingBypassMarker {
            id:   self.marker_id,
            path: self.marker_path,
        }
    }
}

impl RecoveredPendingBypassMarker {
    /// Borrow the durable marker identity reported for this invocation.
    pub(crate) const fn id(&self) -> &PendingBypassMarkerId { &self.id }

    /// Borrow the marker path removed after its audit fact became durable.
    fn path(&self) -> &Path { &self.path }
}

impl PendingBypassRecovery {
    /// Take decoded marker imports whose journal operations are still absent.
    pub(crate) fn take_imports(&mut self) -> Vec<PendingBypassMarkerImport> {
        std::mem::take(&mut self.imports)
    }

    /// Take markers safe to delete after all imports are durably appended.
    pub(crate) fn take_completed_markers(&mut self) -> Vec<RecoveredPendingBypassMarker> {
        std::mem::take(&mut self.completed_markers)
    }

    /// Take occurrence facts for markers that could not be decoded and journalled.
    pub(crate) fn take_unrecorded_occurrences(&mut self) -> Vec<BypassOccurrenceTime> {
        std::mem::take(&mut self.unrecorded_occurrences)
    }
}

/// Return whether the unconditional release valve is active.
pub(crate) fn environment_bypass_requested() -> bool {
    std::env::var_os(BYPASS_ENVIRONMENT)
        .as_deref()
        .is_some_and(|value| value == OsStr::new(BYPASS_ENVIRONMENT_ENABLED_VALUE))
}

/// Best-effort audit an already-authorized environment bypass without waiting for the lock.
///
/// Enrollment is a property of the repository here, not of the worktree the developer stands in.
/// The managed `reference-transaction` hook changes to the policy worktree before invoking this
/// binary, so `invocation_directory` names that checkout and the configuration read below asks
/// whether the repository ever enrolled. That is the right granularity: a trunk update is a
/// repository-wide event, and a gate a worktree could opt out of by deleting its own configuration
/// would not be a gate. Only when the repository itself is unenrolled -- no configuration in the
/// policy worktree, or no policy worktree left to change into -- is there no shared audit
/// destination, and nothing is written.
pub(crate) fn record_environment_bypass(
    invocation_directory: &Path,
    action: BypassedAction,
) -> EnvironmentBypassRetentionOutcome {
    let Ok(worktree_context) = WorktreeContext::discover(invocation_directory) else {
        return EnvironmentBypassRetentionOutcome::Unrecorded;
    };
    match BerthConfig::read(&worktree_context.configuration_lookup()) {
        Ok(Enrollment::Unconfigured { .. }) => {
            return EnvironmentBypassRetentionOutcome::Unenrolled;
        },
        Ok(Enrollment::Enrolled(_)) | Err(_) => {},
    }
    let journal_mutation_actor = ledger::resolve_identity(&worktree_context);
    let cause = BypassCause::EnvironmentOverride {
        bypassed_merge: bypassed_merge_identity(),
    };
    let journalled = Ledger::open(worktree_context.repository_root())
        .and_then(|ledger| {
            let journal_mutation_actor = journal_mutation_actor?;
            ledger
                .try_transact(
                    journal_mutation_actor.worktree_id,
                    journal_mutation_actor.coordination_run_id,
                    |_| {
                        TransactionValidation::<()>::Append(Box::new(JournalOperation::Bypass {
                            action,
                            cause: cause.clone(),
                            occurrence_time: BypassOccurrenceTime::EventRecordedAt,
                            recording: BypassRecording::Direct,
                        }))
                    },
                )
                .map(|_| ())
                .map_err(|error| match error {
                    LedgerTransactionError::LedgerUnreadable(error) => error,
                    LedgerTransactionError::LockContention
                    | LedgerTransactionError::CorrectableInput(_)
                    | LedgerTransactionError::DerivedRecordTooLarge { .. } => {
                        LedgerError::BypassAuditUnavailable
                    },
                })
        })
        .is_ok();
    if journalled {
        return EnvironmentBypassRetentionOutcome::Journalled;
    }
    if write_pending_marker(worktree_context.common_git_directory(), action, cause).is_ok() {
        EnvironmentBypassRetentionOutcome::PendingMarker
    } else {
        EnvironmentBypassRetentionOutcome::Unrecorded
    }
}

fn bypassed_merge_identity() -> BypassedMergeIdentity {
    std::env::var(BYPASSED_MERGE_IDENTITY_ENVIRONMENT)
        .ok()
        .and_then(|token| BypassedMergeIdentity::from_hook_token(&token).ok())
        .unwrap_or_else(|| CoordinationRunId::new().into())
}

fn write_pending_marker(
    common_git_directory: &Path,
    action: BypassedAction,
    cause: BypassCause,
) -> Result<(), std::io::Error> {
    let marker = PendingEnvironmentBypass {
        action,
        cause,
        occurrence_time: PendingEnvironmentBypassOccurrenceTime::Known {
            at: RecordedAt::now(),
        },
    };
    write_reconciliation_marker(
        common_git_directory,
        &PendingReconciliationMarker::EnvironmentBypass(marker),
    )
}

/// Persist the issuing checkout's rewrite map without acquiring the ledger lock.
pub(super) fn write_branch_rewrite_marker(
    common_git_directory: &Path,
    worktree_administrative_directory: &Path,
    pairs: Vec<RewriteMapPair>,
    new_tips: Vec<GitObjectId>,
    created_commits: Vec<GitObjectId>,
) -> Result<(), std::io::Error> {
    write_reconciliation_marker(
        common_git_directory,
        &PendingReconciliationMarker::BranchRewrite(PendingBranchRewrite {
            kind: BranchRewriteMarkerKind::BranchRewrite,
            pairs,
            worktree_administrative_directory: worktree_administrative_directory.to_path_buf(),
            new_tips,
            created_commits,
            completed_subjects: Vec::new(),
        }),
    )
}

/// Share the create-and-sync durability protocol across both marker kinds.
fn write_reconciliation_marker(
    common_git_directory: &Path,
    marker: &PendingReconciliationMarker,
) -> Result<(), std::io::Error> {
    let marker_path = common_git_directory.join(format!(
        "{PENDING_BYPASS_FILE_PREFIX}{}{PENDING_BYPASS_FILE_SUFFIX}",
        Uuid::now_v7()
    ));
    let encoded = serde_json::to_vec(&marker).map_err(std::io::Error::other)?;
    let mut marker_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(marker_path)?;
    marker_file.write_all(&encoded)?;
    marker_file.write_all(b"\n")?;
    marker_file.sync_all()?;
    fs::File::open(common_git_directory)?.sync_all()?;
    Ok(())
}

/// List decoded rewrite markers independently of environment-bypass reporting.
pub(crate) fn pending_branch_rewrite_markers(
    common_git_directory: &Path,
) -> Result<Vec<PendingBranchRewriteMarker>, std::io::Error> {
    let mut markers = Vec::new();
    for entry in fs::read_dir(common_git_directory)? {
        let entry = entry?;
        if !is_pending_marker_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        // A concurrent importer may have removed this file; other unreadable shared
        // markers remain the bypass importer's responsibility to report.
        let Ok(contents) = fs::read(entry.path()) else {
            continue;
        };
        if let Ok(PendingReconciliationMarker::BranchRewrite(rewrite)) =
            serde_json::from_slice(&contents)
        {
            markers.push(PendingBranchRewriteMarker {
                path: entry.path(),
                rewrite,
            });
        }
    }
    markers.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(markers)
}

/// Delete rewrite markers only after their accepted resnapshots are durable.
pub(crate) fn delete_branch_rewrite_markers(
    markers: &[PendingBranchRewriteMarker],
) -> Result<(), std::io::Error> {
    delete_marker_paths(markers.iter().map(|marker| marker.path.as_path()))
}

/// Persist completed candidates atomically after the reconciliation append succeeds.
pub(crate) fn update_branch_rewrite_markers(
    markers: &[PendingBranchRewriteMarker],
) -> Result<(), std::io::Error> {
    for marker in markers {
        let contents = match fs::read(&marker.path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        let mut rewrite: PendingBranchRewrite =
            serde_json::from_slice(&contents).map_err(std::io::Error::other)?;
        for subject in &marker.rewrite.completed_subjects {
            if !rewrite.completed_subjects.contains(subject) {
                rewrite.completed_subjects.push(subject.clone());
            }
        }
        let directory = marker.path.parent().ok_or_else(|| {
            std::io::Error::new(
                ErrorKind::InvalidInput,
                "rewrite marker has no parent directory",
            )
        })?;
        let temporary_path =
            directory.join(format!(".cargo-berth-rewrite-progress-{}", Uuid::now_v7()));
        let encoded = serde_json::to_vec(&PendingReconciliationMarker::BranchRewrite(rewrite))
            .map_err(std::io::Error::other)?;
        let outcome = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            file.write_all(&encoded)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&temporary_path, &marker.path)?;
            fs::File::open(directory)?.sync_all()
        })();
        if outcome.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        outcome?;
    }
    Ok(())
}

/// Recognize the shared pending-marker filename contract.
fn is_pending_marker_name(name: &str) -> bool {
    name.starts_with(PENDING_BYPASS_FILE_PREFIX) && name.ends_with(PENDING_BYPASS_FILE_SUFFIX)
}

/// Recognize rewrite payloads even when their facts cannot be decoded for import.
fn is_branch_rewrite_payload(contents: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(contents).is_ok_and(|value| {
        value.get("kind").and_then(serde_json::Value::as_str) == Some("branch_rewrite")
    })
}

/// Count durable bypass markers left because the journal could not accept the event.
pub(crate) fn pending_environment_bypass_count(
    common_git_directory: &Path,
) -> Result<u64, std::io::Error> {
    let count = fs::read_dir(common_git_directory)?
        .filter_map(Result::ok)
        .filter(|entry| {
            is_pending_marker_name(&entry.file_name().to_string_lossy())
                && !fs::read(entry.path())
                    .is_ok_and(|contents| is_branch_rewrite_payload(&contents))
        })
        .count();
    u64::try_from(count).map_err(std::io::Error::other)
}

/// Prepare stable, idempotent imports for every pending bypass marker.
pub(crate) fn prepare_pending_bypass_recovery(
    common_git_directory: &Path,
    events: &[JournalEvent],
) -> Result<PendingBypassRecovery, std::io::Error> {
    let mut imports = Vec::new();
    let mut completed_markers = Vec::new();
    let mut unrecorded_occurrences = Vec::new();
    for entry in fs::read_dir(common_git_directory)? {
        let entry = entry?;
        let Ok(file_name) = entry.file_name().into_string() else {
            continue;
        };
        if !file_name.starts_with(PENDING_BYPASS_FILE_PREFIX)
            || !file_name.ends_with(PENDING_BYPASS_FILE_SUFFIX)
        {
            continue;
        }
        let marker_path = entry.path();
        let contents = fs::read(&marker_path);
        if contents
            .as_ref()
            .is_ok_and(|contents| is_branch_rewrite_payload(contents))
        {
            continue;
        }
        let marker_result = contents
            .and_then(|contents| serde_json::from_slice(&contents).map_err(std::io::Error::other));
        let Ok(PendingReconciliationMarker::EnvironmentBypass(marker)) = marker_result else {
            unrecorded_occurrences.push(BypassOccurrenceTime::Unavailable);
            continue;
        };
        let marker_id = PendingBypassMarkerId::from_file_name(file_name);
        let already_imported = events.iter().any(|event| {
            matches!(
                &event.operation,
                JournalOperation::Bypass {
                    recording: BypassRecording::PendingMarker {
                        marker_id: imported_id,
                    },
                    ..
                } if imported_id == &marker_id
            )
        });
        if already_imported {
            completed_markers.push(RecoveredPendingBypassMarker {
                id:   marker_id,
                path: marker_path,
            });
        } else {
            let occurrence_time = BypassOccurrenceTime::from(marker.occurrence_time);
            let operation = JournalOperation::Bypass {
                action:          marker.action,
                cause:           marker.cause,
                occurrence_time: occurrence_time.clone(),
                recording:       BypassRecording::PendingMarker {
                    marker_id: marker_id.clone(),
                },
            };
            imports.push(PendingBypassMarkerImport {
                operation,
                marker_id,
                marker_path,
                occurrence_time,
            });
        }
    }
    Ok(PendingBypassRecovery {
        imports,
        completed_markers,
        unrecorded_occurrences,
    })
}

/// Delete only markers whose matching journal operation is already durable.
pub(crate) fn delete_recovered_bypass_markers(
    recovered_markers: &[RecoveredPendingBypassMarker],
) -> Result<(), std::io::Error> {
    delete_marker_paths(
        recovered_markers
            .iter()
            .map(RecoveredPendingBypassMarker::path),
    )
}

/// Remove completed markers idempotently and sync each affected directory once.
fn delete_marker_paths<'path>(
    marker_paths: impl Iterator<Item = &'path Path>,
) -> Result<(), std::io::Error> {
    let mut changed_directories = HashSet::new();
    for marker_path in marker_paths {
        match fs::remove_file(marker_path) {
            Ok(()) => {
                if let Some(parent) = marker_path.parent() {
                    changed_directories.insert(parent.to_path_buf());
                }
            },
            Err(error) if error.kind() == ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
    }
    for changed_directory in changed_directories {
        fs::File::open(changed_directory)?.sync_all()?;
    }
    Ok(())
}

/// Replay permits and reject duplicate issuance, duplicate consumption, or mismatched use.
pub(crate) fn available_forced_integration_permits(
    events: &[JournalEvent],
) -> Result<Vec<AvailableForcedIntegrationPermit>, ForcedIntegrationPermitReplayError> {
    let mut issued = HashMap::new();
    let mut consumed = HashSet::new();
    for event in events {
        match &event.operation {
            JournalOperation::ForcedIntegrationPermit {
                permit_id,
                reservation_id,
                reason,
                skipped_holds,
            } => {
                if issued
                    .insert(
                        *permit_id,
                        AvailableForcedIntegrationPermit {
                            permit_id:      *permit_id,
                            reservation_id: *reservation_id,
                            reason:         reason.clone(),
                            skipped_holds:  skipped_holds.clone(),
                        },
                    )
                    .is_some()
                {
                    return Err(ForcedIntegrationPermitReplayError::DuplicatePermit(
                        *permit_id,
                    ));
                }
            },
            JournalOperation::ConsumeForcedIntegrationPermit {
                permit_id,
                reservation_id,
            } => {
                let Some(permit) = issued.get(permit_id) else {
                    return Err(ForcedIntegrationPermitReplayError::UnknownPermit(
                        *permit_id,
                    ));
                };
                if permit.reservation_id != *reservation_id {
                    return Err(ForcedIntegrationPermitReplayError::ReservationMismatch(
                        *permit_id,
                    ));
                }
                if !consumed.insert(*permit_id) {
                    return Err(ForcedIntegrationPermitReplayError::AlreadyConsumed(
                        *permit_id,
                    ));
                }
            },
            _ => {},
        }
    }
    Ok(issued
        .into_values()
        .filter(|permit| !consumed.contains(&permit.permit_id))
        .collect())
}

/// One-use permit facts in the journal are internally inconsistent.
#[derive(Debug)]
pub(crate) enum ForcedIntegrationPermitReplayError {
    /// The same semantic permit identity was issued twice.
    DuplicatePermit(ForcedIntegrationPermitId),
    /// A consumption names no earlier permit.
    UnknownPermit(ForcedIntegrationPermitId),
    /// A permit was consumed for a reservation other than the one it authorized.
    ReservationMismatch(ForcedIntegrationPermitId),
    /// A permit was consumed more than once.
    AlreadyConsumed(ForcedIntegrationPermitId),
}

impl Display for ForcedIntegrationPermitReplayError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicatePermit(permit_id) => {
                write!(
                    formatter,
                    "forced-integration permit {permit_id} was issued twice"
                )
            },
            Self::UnknownPermit(permit_id) => write!(
                formatter,
                "forced-integration permit {permit_id} was consumed before it was issued"
            ),
            Self::ReservationMismatch(permit_id) => write!(
                formatter,
                "forced-integration permit {permit_id} was consumed by the wrong reservation"
            ),
            Self::AlreadyConsumed(permit_id) => write!(
                formatter,
                "forced-integration permit {permit_id} was consumed more than once"
            ),
        }
    }
}

impl std::error::Error for ForcedIntegrationPermitReplayError {}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::ffi::OsStr;
    use std::fs;
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::process::Command;

    use tempfile::tempdir;

    use super::BYPASS_ENVIRONMENT;
    use super::BYPASS_ENVIRONMENT_ENABLED_VALUE;
    use super::PENDING_BYPASS_FILE_PREFIX;
    use super::PENDING_BYPASS_FILE_SUFFIX;
    use super::PendingEnvironmentBypass;
    use super::PendingEnvironmentBypassOccurrenceTime;
    use super::write_pending_marker;
    use crate::gate::install;
    use crate::ledger::BypassCause;
    use crate::ledger::BypassedAction;
    use crate::ledger::BypassedMergeIdentity;

    #[test]
    fn rewrite_scan_leaves_unreadable_markers_to_bypass_recovery() {
        let directory = tempdir().expect("marker directory should exist");
        let marker_path = |name| {
            directory.path().join(format!(
                "{PENDING_BYPASS_FILE_PREFIX}{name}{PENDING_BYPASS_FILE_SUFFIX}"
            ))
        };
        let missing_marker = marker_path("vanished");
        symlink(directory.path().join("missing-target"), &missing_marker)
            .expect("dangling marker should simulate a vanished read");
        assert_eq!(
            fs::read(&missing_marker)
                .expect_err("dangling marker should fail to read")
                .kind(),
            ErrorKind::NotFound
        );
        let unreadable_marker = marker_path("unreadable");
        fs::create_dir(&unreadable_marker).expect("unreadable marker should exist");
        assert!(fs::read(&unreadable_marker).is_err());
        let undecodable_marker = marker_path("undecodable");
        fs::write(&undecodable_marker, b"invalid marker").expect("undecodable marker should exist");
        super::write_branch_rewrite_marker(
            directory.path(),
            directory.path(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("valid rewrite should write");
        write_pending_marker(
            directory.path(),
            BypassedAction::Integration,
            environment_bypass_cause("read-errors"),
        )
        .expect("valid bypass should write");

        let rewrites = super::pending_branch_rewrite_markers(directory.path())
            .expect("unreadable and undecodable markers should not abort the rewrite scan");
        assert_eq!(rewrites.len(), 1);
        let mut recovery = super::prepare_pending_bypass_recovery(directory.path(), &[])
            .expect("bypass recovery should still report unreadable markers");
        assert_eq!(recovery.take_imports().len(), 1);
        assert_eq!(recovery.take_unrecorded_occurrences().len(), 3);
        assert!(fs::symlink_metadata(missing_marker).is_ok());
        assert!(unreadable_marker.is_dir());
        assert!(undecodable_marker.is_file());
        assert!(super::pending_branch_rewrite_markers(&directory.path().join("absent")).is_err());
    }

    #[test]
    fn rewrite_markers_never_become_bypass_counts_audits_or_notices() {
        let directory = tempdir().expect("marker directory should exist");
        let administrative_directory = directory.path().join("worktrees/linked");
        super::write_branch_rewrite_marker(
            directory.path(),
            &administrative_directory,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("rewrite marker should write");
        write_pending_marker(
            directory.path(),
            BypassedAction::Integration,
            environment_bypass_cause("mixed"),
        )
        .expect("bypass marker should write");
        assert_eq!(
            super::pending_environment_bypass_count(directory.path())
                .expect("markers should count"),
            1
        );
        let mut bypass = super::prepare_pending_bypass_recovery(directory.path(), &[])
            .expect("recovery should prepare");
        assert_eq!(bypass.take_imports().len(), 1);
        assert!(bypass.take_unrecorded_occurrences().is_empty());
        let rewrites = super::pending_branch_rewrite_markers(directory.path())
            .expect("rewrite markers should decode");
        assert_eq!(rewrites.len(), 1);
        assert_eq!(
            rewrites[0].rewrite.worktree_administrative_directory,
            administrative_directory
        );
        super::delete_branch_rewrite_markers(&rewrites)
            .expect("rewrite cleanup should succeed without bypass reporting");
        super::delete_branch_rewrite_markers(&rewrites)
            .expect("rewrite cleanup should be idempotent");
        assert!(
            super::pending_branch_rewrite_markers(directory.path())
                .expect("markers should list")
                .is_empty()
        );
        assert_eq!(
            super::pending_environment_bypass_count(directory.path())
                .expect("bypass should remain"),
            1
        );
    }

    #[test]
    fn rewrite_markers_require_the_created_commits_observed_at_capture() {
        let directory = tempdir().expect("marker directory should exist");
        let commit = "1111111111111111111111111111111111111111"
            .parse()
            .expect("commit should parse");
        super::write_branch_rewrite_marker(
            directory.path(),
            directory.path(),
            Vec::new(),
            Vec::new(),
            vec![commit],
        )
        .expect("rewrite marker should write");
        let markers = super::pending_branch_rewrite_markers(directory.path())
            .expect("rewrite marker should decode");
        assert_eq!(markers[0].rewrite.created_commits.len(), 1);
        let mut missing = serde_json::to_value(&markers[0].rewrite).expect("marker should encode");
        missing
            .as_object_mut()
            .expect("marker should be an object")
            .remove("created_commits");
        assert!(serde_json::from_value::<super::PendingBranchRewrite>(missing.clone()).is_err());
        fs::write(
            &markers[0].path,
            serde_json::to_vec(&missing).expect("marker should encode"),
        )
        .expect("unreleased marker should write");
        assert!(
            super::pending_branch_rewrite_markers(directory.path())
                .expect("markers should list")
                .is_empty()
        );
        let mut recovery = super::prepare_pending_bypass_recovery(directory.path(), &[])
            .expect("undecodable rewrite should remain separate from bypass reporting");
        assert!(recovery.take_imports().is_empty());
        assert!(recovery.take_unrecorded_occurrences().is_empty());
        assert!(markers[0].path.is_file());
    }

    #[test]
    fn rewrite_marker_progress_preserves_completed_subjects_across_stale_preflights() {
        let directory = tempdir().expect("marker directory should exist");
        super::write_branch_rewrite_marker(
            directory.path(),
            directory.path(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("rewrite marker should write");
        let original =
            super::pending_branch_rewrite_markers(directory.path()).expect("marker should decode");
        let mut first = original.clone();
        let mut second = original;
        for markers in [&mut first, &mut second] {
            markers[0]
                .rewrite
                .completed_subjects
                .push(super::CompletedRewriteSubject {
                    reservation_id: crate::ids::ReservationId::new(),
                    subject:        serde_json::from_str("1")
                        .expect("initial revision should decode"),
                });
            super::update_branch_rewrite_markers(markers)
                .expect("completed subject should persist");
        }
        let merged = super::pending_branch_rewrite_markers(directory.path())
            .expect("updated marker should decode");
        assert_eq!(merged[0].rewrite.completed_subjects.len(), 2);
        assert_eq!(
            super::pending_environment_bypass_count(directory.path())
                .expect("markers should count"),
            0
        );
        super::delete_branch_rewrite_markers(&merged).expect("completed marker should delete");
        super::update_branch_rewrite_markers(&first)
            .expect("stale progress should not recreate a deleted marker");
        assert!(
            super::pending_branch_rewrite_markers(directory.path())
                .expect("markers should list")
                .is_empty()
        );
    }

    #[test]
    fn both_marker_writers_share_the_typed_occurrence_time_schema() {
        let rust_directory = tempdir().expect("Rust marker directory should exist");
        write_pending_marker(
            rust_directory.path(),
            BypassedAction::Integration,
            environment_bypass_cause("rust-writer"),
        )
        .expect("Rust marker should write");
        let rust_marker = read_pending_marker(rust_directory.path());

        let shell_known_directory = tempdir().expect("shell marker directory should exist");
        let inherited_path = std::env::var_os("PATH").expect("test PATH should exist");
        write_shell_marker(shell_known_directory.path(), &inherited_path);
        let shell_known_marker = read_pending_marker(shell_known_directory.path());

        let shell_unavailable_directory =
            tempdir().expect("unavailable-time marker directory should exist");
        write_shell_marker(shell_unavailable_directory.path(), OsStr::new(""));
        let shell_unavailable_marker = read_pending_marker(shell_unavailable_directory.path());

        assert!(matches!(
            rust_marker.cause,
            BypassCause::EnvironmentOverride { .. }
        ));
        assert!(matches!(
            rust_marker.occurrence_time,
            PendingEnvironmentBypassOccurrenceTime::Known { .. }
        ));
        assert!(matches!(
            shell_known_marker.cause,
            BypassCause::EnvironmentOverride { .. }
        ));
        assert!(matches!(
            shell_known_marker.occurrence_time,
            PendingEnvironmentBypassOccurrenceTime::Known { .. }
        ));
        assert_eq!(
            shell_unavailable_marker.occurrence_time,
            PendingEnvironmentBypassOccurrenceTime::Unavailable
        );

        let unavailable_json = serde_json::to_vec(&shell_unavailable_marker)
            .expect("unavailable occurrence time should serialize");
        let round_tripped = serde_json::from_slice::<PendingEnvironmentBypass>(&unavailable_json)
            .expect("unavailable occurrence time should deserialize");
        assert_eq!(round_tripped, shell_unavailable_marker);
    }

    #[test]
    fn pending_marker_time_cannot_default_to_the_later_journal_event() {
        let cause = serde_json::json!({
            "cause": {
                "kind": "environment_override",
                "bypassed_merge": "schema-regression",
            },
        });
        assert!(serde_json::from_value::<PendingEnvironmentBypass>(cause).is_err());

        let event_recorded_at = serde_json::json!({
            "cause": {
                "kind": "environment_override",
                "bypassed_merge": "schema-regression",
            },
            "occurrence_time": {"status": "event_recorded_at"},
        });
        assert!(serde_json::from_value::<PendingEnvironmentBypass>(event_recorded_at).is_err());
    }

    fn environment_bypass_cause(token: &str) -> BypassCause {
        BypassCause::EnvironmentOverride {
            bypassed_merge: BypassedMergeIdentity::from_hook_token(token)
                .expect("test bypass identity should be non-empty"),
        }
    }

    fn write_shell_marker(common_git_directory: &Path, path: &OsStr) {
        let policy_worktree = common_git_directory.join("policy-worktree");
        fs::create_dir(&policy_worktree).expect("policy worktree should exist");
        let script_path = common_git_directory.join("reference-transaction");
        let script = install::reference_transaction_hook_script_for_test(
            common_git_directory,
            &policy_worktree,
            "refs/heads/main",
        );
        fs::write(&script_path, script).expect("managed hook should write");
        let mut permissions = fs::metadata(&script_path)
            .expect("managed hook metadata should read")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script_path, permissions).expect("managed hook should be executable");

        let output = Command::new(script_path)
            .arg("prepared")
            .current_dir(policy_worktree)
            .env(BYPASS_ENVIRONMENT, BYPASS_ENVIRONMENT_ENABLED_VALUE)
            .env(install::EXECUTABLE_ENVIRONMENT, "/missing/cargo-berth")
            .env("PATH", path)
            .output()
            .expect("managed hook should run");
        assert!(output.status.success());
    }

    fn read_pending_marker(common_git_directory: &Path) -> PendingEnvironmentBypass {
        let marker_path = fs::read_dir(common_git_directory)
            .expect("common git directory should read")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.starts_with(PENDING_BYPASS_FILE_PREFIX)
                        && name.ends_with(PENDING_BYPASS_FILE_SUFFIX)
                })
            })
            .expect("pending marker should exist");
        let encoded = fs::read(marker_path).expect("pending marker should read");
        serde_json::from_slice(&encoded).expect("pending marker should deserialize")
    }
}
