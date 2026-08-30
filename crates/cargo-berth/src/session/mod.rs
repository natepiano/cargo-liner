//! Durable harness-session identity mappings beside the shared journal.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;

use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ledger::HARNESS_SESSION_ENVIRONMENT;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;

static CURRENT_PROCESS_HARNESS_SESSION: OnceLock<HarnessSessionId> = OnceLock::new();

/// One harness session identifier supplied to a single command invocation.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct HarnessSessionId(String);

impl HarnessSessionId {
    const MAXIMUM_CHARACTERS: usize = 256;

    fn from_current_process() -> HarnessSessionIdentity {
        if let Some(harness_session_id) = CURRENT_PROCESS_HARNESS_SESSION.get().cloned() {
            return HarnessSessionIdentity::Available(harness_session_id);
        }
        std::env::var_os(HARNESS_SESSION_ENVIRONMENT)
            .and_then(|value| value.into_string().ok())
            .and_then(|value| value.parse().ok())
            .map_or(
                HarnessSessionIdentity::Unavailable,
                HarnessSessionIdentity::Available,
            )
    }
}

/// Whether a private hook boundary selected its harness session for this process.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CurrentProcessHarnessSessionSelection {
    /// This process will resolve and publish mappings for the selected session.
    Selected,
    /// Another private boundary had already selected a session in this process.
    AlreadySelected,
}

/// Select the harness session parsed by a private hook boundary.
pub(crate) fn select_current_process_harness_session(
    harness_session_id: HarnessSessionId,
) -> CurrentProcessHarnessSessionSelection {
    if CURRENT_PROCESS_HARNESS_SESSION
        .set(harness_session_id)
        .is_ok()
    {
        CurrentProcessHarnessSessionSelection::Selected
    } else {
        CurrentProcessHarnessSessionSelection::AlreadySelected
    }
}

impl FromStr for HarnessSessionId {
    type Err = InvalidHarnessSessionId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if !value.is_empty()
            && value.len() <= Self::MAXIMUM_CHARACTERS
            && value.chars().all(|character| !character.is_control())
        {
            Ok(Self(value.to_owned()))
        } else {
            Err(InvalidHarnessSessionId)
        }
    }
}

/// A harness session id was absent or unsuitable for durable lookup.
#[derive(Clone, Debug, Eq, PartialEq)]
enum HarnessSessionIdentity {
    /// The current invocation supplied a valid harness session id.
    Available(HarnessSessionId),
    /// The current invocation supplied no usable harness session id.
    Unavailable,
}

/// The active coordination identity assigned to one harness session.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct SessionReservationIdentity {
    /// The coordination run recorded on the claim.
    coordination_run_id: CoordinationRunId,
    /// The reservation selected for later edit and drift attribution.
    reservation_id:      ReservationId,
}

impl SessionReservationIdentity {
    /// Build the identity assigned by one successful claim.
    pub(crate) const fn new(
        coordination_run_id: CoordinationRunId,
        reservation_id: ReservationId,
    ) -> Self {
        Self {
            coordination_run_id,
            reservation_id,
        }
    }

    /// Return the coordination run recorded by the successful claim.
    pub(crate) const fn coordination_run_id(self) -> CoordinationRunId { self.coordination_run_id }

    /// Return the reservation selected by this harness session.
    pub(crate) const fn reservation_id(self) -> ReservationId { self.reservation_id }
}

/// The result of consulting the disposable session identity mapping.
#[derive(Clone, Copy)]
pub(crate) enum SessionIdentityLookup {
    /// The current harness session has one mapped run and reservation.
    Mapped(SessionReservationIdentity),
    /// No readable mapping answers the current harness session.
    Unavailable,
}

/// Whether the current command's harness-session identity was published.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum SessionIdentityMappingPublication {
    /// The mapping reflects the reservation, including when no harness session required an entry.
    Published,
    /// The reservation is durable, but its disposable mapping update failed.
    Unavailable {
        /// The mapping publication failure.
        diagnostic: String,
    },
}

/// The result of removing only the mapping selected by this process's harness session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CurrentSessionMappingRemoval {
    /// The current harness session's mapping was removed.
    Removed,
    /// The current harness session was valid but had no stored mapping.
    AlreadyAbsent,
    /// The process supplied no usable harness session identifier.
    CurrentSessionUnavailable,
}

impl SessionIdentityMappingPublication {
    /// Retain an unavailable result across a transaction with several appends.
    pub(crate) fn merge(self, next: Self) -> Self {
        match (self, next) {
            (unavailable @ Self::Unavailable { .. }, _) => unavailable,
            (Self::Published, next) => next,
        }
    }
}

/// The complete disposable mapping published atomically under the ledger.
#[derive(Default, Deserialize, Serialize)]
struct SessionIdentityStore {
    identities: BTreeMap<HarnessSessionId, SessionReservationIdentity>,
}

/// A session identity file could not be encoded or published.
#[derive(Debug)]
pub(crate) enum SessionIdentityStoreError {
    /// A filesystem operation failed.
    Io(std::io::Error),
    /// The mapping could not be encoded as JSON.
    Encoding(serde_json::Error),
    /// The existing mapping could not be decoded as JSON.
    Decoding(serde_json::Error),
}

/// A harness session id was empty, too long, or contained a control character.
#[derive(Debug)]
pub(crate) struct InvalidHarnessSessionId;

/// Resolve the current process's harness session from the mapping beside the journal.
pub(crate) fn resolve(ledger_directory: &Path) -> SessionIdentityLookup {
    let HarnessSessionIdentity::Available(harness_session_id) =
        HarnessSessionId::from_current_process()
    else {
        return SessionIdentityLookup::Unavailable;
    };
    let mapping_path = ledger_directory.join(SessionIdentityStore::FILE_NAME);
    fs::read(&mapping_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SessionIdentityStore>(&bytes).ok())
        .and_then(|store| store.identities.get(&harness_session_id).copied())
        .map_or(
            SessionIdentityLookup::Unavailable,
            SessionIdentityLookup::Mapped,
        )
}

/// Apply the mapping consequence of one already-appended journal event.
pub(crate) fn apply_journal_event(
    ledger_directory: &Path,
    event: &JournalEvent,
) -> SessionIdentityMappingPublication {
    let mapping_path = ledger_directory.join(SessionIdentityStore::FILE_NAME);
    let publication = match &event.operation {
        JournalOperation::Claim { reservation_id, .. }
        | JournalOperation::Widen { reservation_id, .. } => {
            return publish_reservation_identity(
                ledger_directory,
                event.actor.run,
                *reservation_id,
            );
        },
        JournalOperation::Checkpoint { reservation_id, .. }
        | JournalOperation::Release { reservation_id, .. } => {
            let mut store = SessionIdentityStore::read_for_update(&mapping_path);
            store
                .identities
                .retain(|_, identity| identity.reservation_id != *reservation_id);
            store.publish(ledger_directory, &mapping_path)
        },
        _ => Ok(()),
    };
    publication.into()
}

/// Publish the current harness session's known coordination identity.
pub(crate) fn publish_reservation_identity(
    ledger_directory: &Path,
    coordination_run_id: CoordinationRunId,
    reservation_id: ReservationId,
) -> SessionIdentityMappingPublication {
    let HarnessSessionIdentity::Available(harness_session_id) =
        HarnessSessionId::from_current_process()
    else {
        return SessionIdentityMappingPublication::Published;
    };
    let mapping_path = ledger_directory.join(SessionIdentityStore::FILE_NAME);
    let mut store = SessionIdentityStore::read_for_update(&mapping_path);
    store.identities.insert(
        harness_session_id,
        SessionReservationIdentity::new(coordination_run_id, reservation_id),
    );
    store.publish(ledger_directory, &mapping_path).into()
}

/// Remove only the current process's harness-session mapping.
pub(crate) fn remove_current_mapping(
    ledger_directory: &Path,
) -> Result<CurrentSessionMappingRemoval, SessionIdentityStoreError> {
    let HarnessSessionIdentity::Available(harness_session_id) =
        HarnessSessionId::from_current_process()
    else {
        return Ok(CurrentSessionMappingRemoval::CurrentSessionUnavailable);
    };
    let mapping_path = ledger_directory.join(SessionIdentityStore::FILE_NAME);
    let mut store = SessionIdentityStore::read_for_removal(&mapping_path)?;
    if store.identities.remove(&harness_session_id).is_none() {
        return Ok(CurrentSessionMappingRemoval::AlreadyAbsent);
    }
    store.publish(ledger_directory, &mapping_path)?;
    Ok(CurrentSessionMappingRemoval::Removed)
}

impl SessionIdentityStore {
    const FILE_NAME: &'static str = "session-identities.json";

    fn read_for_update(path: &Path) -> Self {
        fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn read_for_removal(path: &Path) -> Result<Self, SessionIdentityStoreError> {
        match fs::read(path) {
            Ok(bytes) => {
                serde_json::from_slice(&bytes).map_err(SessionIdentityStoreError::Decoding)
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(SessionIdentityStoreError::Io(error)),
        }
    }

    fn publish(
        &self,
        ledger_directory: &Path,
        mapping_path: &Path,
    ) -> Result<(), SessionIdentityStoreError> {
        let bytes = serde_json::to_vec(self).map_err(SessionIdentityStoreError::Encoding)?;
        let publication_id = Uuid::now_v7();
        let temporary_path =
            ledger_directory.join(format!("{}.{publication_id}.tmp", Self::FILE_NAME));
        let publication = (|| -> Result<(), std::io::Error> {
            let mut temporary_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary_path)?;
            temporary_file.write_all(&bytes)?;
            temporary_file.sync_all()?;
            fs::rename(&temporary_path, mapping_path)?;
            fs::File::open(ledger_directory)?.sync_all()
        })();
        if publication.is_err() {
            std::mem::drop(fs::remove_file(&temporary_path));
        }
        publication.map_err(SessionIdentityStoreError::Io)
    }
}

impl From<Result<(), SessionIdentityStoreError>> for SessionIdentityMappingPublication {
    fn from(publication: Result<(), SessionIdentityStoreError>) -> Self {
        publication.map_or_else(
            |error| Self::Unavailable {
                diagnostic: error.to_string(),
            },
            |()| Self::Published,
        )
    }
}

impl Display for SessionIdentityStoreError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "session identity mapping I/O failed: {error}"),
            Self::Encoding(error) => {
                write!(
                    formatter,
                    "session identity mapping encoding failed: {error}"
                )
            },
            Self::Decoding(error) => {
                write!(
                    formatter,
                    "session identity mapping decoding failed: {error}"
                )
            },
        }
    }
}

impl std::error::Error for SessionIdentityStoreError {}

impl Display for InvalidHarnessSessionId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(
            "a harness session id must be non-empty, bounded, and contain no control characters",
        )
    }
}

impl std::error::Error for InvalidHarnessSessionId {}
