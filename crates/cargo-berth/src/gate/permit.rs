//! One-use forced permits and the unconditional environment release valve.

use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::ids::CoordinationRunId;
use crate::ids::ForcedIntegrationPermitId;
use crate::ids::RecordedAt;
use crate::ids::ReservationId;
use crate::ledger;
use crate::ledger::BypassCause;
use crate::ledger::BypassOccurrenceTime;
use crate::ledger::BypassRecording;
use crate::ledger::BypassedAction;
use crate::ledger::BypassedMergeIdentity;
use crate::ledger::EditAuthorization;
use crate::ledger::ForcedIntegrationReason;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;
use crate::ledger::Ledger;
use crate::ledger::PendingBypassMarkerId;
use crate::ledger::SkippedIntegrationHoldSet;
use crate::ledger::TransactionValidation;
use crate::ledger::WorktreeContext;

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
    /// The ref update was permitted, but neither durable audit destination accepted the fact.
    Unrecorded,
}

/// The shared marker schema used when an environment bypass cannot reach the journal.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PendingEnvironmentBypass {
    /// Why ordinary integration validation was bypassed.
    cause:           BypassCause,
    /// Whether the marker writer retained the override's occurrence time.
    occurrence_time: PendingEnvironmentBypassOccurrenceTime,
}

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
    completed_marker_paths: Vec<PathBuf>,
    unrecorded_occurrences: Vec<BypassOccurrenceTime>,
}

/// One decoded marker whose audit operation is still absent from the journal.
pub(crate) struct PendingBypassMarkerImport {
    operation:       JournalOperation,
    marker_path:     PathBuf,
    occurrence_time: BypassOccurrenceTime,
}

impl PendingBypassMarkerImport {
    /// Borrow the idempotent operation attempted for this marker.
    pub(crate) const fn operation(&self) -> &JournalOperation { &self.operation }

    /// Borrow the marker path that can be deleted only after a successful append.
    pub(crate) fn marker_path(&self) -> &Path { &self.marker_path }

    /// Borrow the occurrence fact shown when this import still cannot be appended.
    pub(crate) const fn occurrence_time(&self) -> &BypassOccurrenceTime { &self.occurrence_time }
}

impl PendingBypassRecovery {
    /// Take decoded marker imports whose journal operations are still absent.
    pub(crate) fn take_imports(&mut self) -> Vec<PendingBypassMarkerImport> {
        std::mem::take(&mut self.imports)
    }

    /// Take marker paths safe to delete after all imports are durably appended.
    pub(crate) fn take_completed_marker_paths(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.completed_marker_paths)
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
pub(crate) fn record_environment_bypass(
    invocation_directory: &Path,
) -> EnvironmentBypassRetentionOutcome {
    let Ok(worktree_context) = WorktreeContext::discover(invocation_directory) else {
        return EnvironmentBypassRetentionOutcome::Unrecorded;
    };
    let coordination_run_id = coordination_run_id(&worktree_context);
    let cause = BypassCause::EnvironmentOverride {
        bypassed_merge: bypassed_merge_identity(),
    };
    let journalled = Ledger::open(worktree_context.repository_root())
        .and_then(|ledger| {
            let worktree_identity = ledger::worktree_identity(
                worktree_context.administrative_directory(),
                worktree_context.worktree_kind(),
            )?;
            ledger
                .try_transact(worktree_identity.id, coordination_run_id, |_| {
                    TransactionValidation::<()>::Append(Box::new(JournalOperation::Bypass {
                        action:          BypassedAction::Integration,
                        cause:           cause.clone(),
                        occurrence_time: BypassOccurrenceTime::EventRecordedAt,
                        recording:       BypassRecording::Direct,
                    }))
                })
                .map(|_| ())
                .map_err(|error| match error {
                    crate::ledger::LedgerTransactionError::LedgerUnreadable(error) => error,
                    crate::ledger::LedgerTransactionError::LockContention
                    | crate::ledger::LedgerTransactionError::CorrectableInput(_) => {
                        crate::ledger::LedgerError::BypassAuditUnavailable
                    },
                })
        })
        .is_ok();
    if journalled {
        return EnvironmentBypassRetentionOutcome::Journalled;
    }
    if write_pending_marker(worktree_context.common_git_directory(), cause).is_ok() {
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

fn coordination_run_id(worktree_context: &WorktreeContext) -> CoordinationRunId {
    match EditAuthorization::resolve(
        worktree_context.administrative_directory(),
        &worktree_context.ledger_directory(),
    ) {
        EditAuthorization::Session {
            coordination_run_id: run,
            ..
        }
        | EditAuthorization::Environment(run)
        | EditAuthorization::Marker {
            coordination_run_id: run,
            ..
        } => run,
        EditAuthorization::Unidentified => CoordinationRunId::new(),
    }
}

fn write_pending_marker(
    common_git_directory: &Path,
    cause: BypassCause,
) -> Result<(), std::io::Error> {
    let marker_path = common_git_directory.join(format!(
        "{PENDING_BYPASS_FILE_PREFIX}{}{PENDING_BYPASS_FILE_SUFFIX}",
        Uuid::now_v7()
    ));
    let marker = PendingEnvironmentBypass {
        cause,
        occurrence_time: PendingEnvironmentBypassOccurrenceTime::Known {
            at: RecordedAt::now(),
        },
    };
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

/// Count durable bypass markers left because the journal could not accept the event.
pub(crate) fn pending_environment_bypass_count(
    common_git_directory: &Path,
) -> Result<u64, std::io::Error> {
    let count = fs::read_dir(common_git_directory)?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name.starts_with(PENDING_BYPASS_FILE_PREFIX)
                && name.ends_with(PENDING_BYPASS_FILE_SUFFIX)
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
    let mut completed_marker_paths = Vec::new();
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
        let marker_result = fs::read(&marker_path)
            .and_then(|contents| serde_json::from_slice(&contents).map_err(std::io::Error::other));
        let Ok(marker): Result<PendingEnvironmentBypass, _> = marker_result else {
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
            completed_marker_paths.push(marker_path);
        } else {
            let occurrence_time = BypassOccurrenceTime::from(marker.occurrence_time);
            let operation = JournalOperation::Bypass {
                action:          BypassedAction::Integration,
                cause:           marker.cause,
                occurrence_time: occurrence_time.clone(),
                recording:       BypassRecording::PendingMarker { marker_id },
            };
            imports.push(PendingBypassMarkerImport {
                operation,
                marker_path,
                occurrence_time,
            });
        }
    }
    Ok(PendingBypassRecovery {
        imports,
        completed_marker_paths,
        unrecorded_occurrences,
    })
}

/// Delete only markers whose matching journal operation is already durable.
pub(crate) fn delete_recovered_bypass_markers(
    marker_paths: &[PathBuf],
) -> Result<(), std::io::Error> {
    let mut changed_directories = HashSet::new();
    for marker_path in marker_paths {
        match fs::remove_file(marker_path) {
            Ok(()) => {
                if let Some(parent) = marker_path.parent() {
                    changed_directories.insert(parent.to_path_buf());
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
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
                    return Err(ForcedIntegrationPermitReplayError::ReservationMismatch {
                        permit_id: *permit_id,
                    });
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
    ReservationMismatch {
        permit_id: ForcedIntegrationPermitId,
    },
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
            Self::ReservationMismatch { permit_id } => write!(
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
    use std::os::unix::fs::PermissionsExt;
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
    use crate::gate::install::reference_transaction_hook_script_for_test;
    use crate::ledger::BypassCause;
    use crate::ledger::BypassedMergeIdentity;

    #[test]
    fn both_marker_writers_share_the_typed_occurrence_time_schema() {
        let rust_directory = tempdir().expect("Rust marker directory should exist");
        write_pending_marker(
            rust_directory.path(),
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
        let script = reference_transaction_hook_script_for_test(
            Path::new("/missing/cargo-berth"),
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
