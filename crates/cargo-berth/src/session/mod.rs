//! Durable harness-session identity mappings beside the shared journal.

use std::collections::BTreeMap;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::fs;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::io::Write;
use std::path::Path;
use std::str::FromStr;
use std::sync::OnceLock;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::Serialize;
use uuid::Uuid;

use crate::ids::CoordinationRunId;
use crate::ids::ReservationId;
use crate::ledger::HARNESS_SESSION_ENVIRONMENT;
use crate::ledger::JournalEvent;
use crate::ledger::JournalOperation;

static CURRENT_PROCESS_HARNESS_SESSION: OnceLock<HookHarnessSessionSelection> = OnceLock::new();

/// One harness session identifier supplied to a single command invocation.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub(crate) struct HarnessSessionId(String);

impl HarnessSessionId {
    /// Maximum number of characters accepted from a private hook boundary.
    pub(crate) const MAXIMUM_CHARACTERS: usize = 256;

    fn from_current_process() -> HarnessSessionIdentity {
        match CURRENT_PROCESS_HARNESS_SESSION.get() {
            Some(HookHarnessSessionSelection::Session(harness_session_id)) => {
                HarnessSessionIdentity::Available(harness_session_id.clone())
            },
            Some(HookHarnessSessionSelection::NoSession) => HarnessSessionIdentity::Unavailable,
            None => std::env::var_os(HARNESS_SESSION_ENVIRONMENT)
                .and_then(|value| value.into_string().ok())
                .and_then(|value| value.parse().ok())
                .map_or(
                    HarnessSessionIdentity::Unavailable,
                    HarnessSessionIdentity::Available,
                ),
        }
    }
}

/// The harness session identity a private hook boundary established for this process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum HookHarnessSessionSelection {
    /// The boundary parsed one valid harness session identifier from its payload.
    Session(HarnessSessionId),
    /// The boundary supplied no usable identifier, so this process has no session at all.
    NoSession,
}

/// Establish the harness session identity a private hook boundary read from its payload.
///
/// `NoSession` is a decision, not an absence: it stops `HARNESS_SESSION_ENVIRONMENT` being
/// consulted, so a payload without a session identity cannot adopt the session identity of
/// whichever process launched the hook. The first selection in a process wins, and a hook
/// binary makes exactly one before any reservation lookup.
pub(crate) fn select_current_process_harness_session(selection: HookHarnessSessionSelection) {
    std::mem::drop(CURRENT_PROCESS_HARNESS_SESSION.set(selection));
}

impl FromStr for HarnessSessionId {
    type Err = InvalidHarnessSessionId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let character_count = value.chars().try_fold(0, |character_count, character| {
            if character_count == Self::MAXIMUM_CHARACTERS || character.is_control() {
                Err(InvalidHarnessSessionId)
            } else {
                Ok(character_count + 1)
            }
        })?;
        if character_count > 0 {
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

/// The current harness session's mapping state for locked first-touch selection.
#[derive(Clone, Copy)]
pub(crate) enum FirstTouchSessionReservationMapping {
    /// The current harness session maps to one run and reservation.
    Mapped(SessionReservationIdentity),
    /// A valid harness session id has no readable reservation mapping.
    AvailableSessionWithoutMapping,
    /// The current invocation supplied no usable harness session id.
    HarnessSessionUnavailable,
}

/// Whether the current command's harness-session identity was published.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum SessionIdentityMappingPublication {
    /// The mapping reflects the reservation, including when no harness session required an entry.
    Published,
    /// An explicit selection affected this command but could not become session state.
    ExplicitSelectionAppliesOnlyToCurrentInvocation {
        /// Why a later command cannot reuse the explicit selection.
        reason: ExplicitSelectionPersistenceReason,
    },
    /// The reservation is durable, but its disposable mapping update failed.
    Unavailable {
        /// The mapping publication failure.
        #[schemars(length(min = 1))]
        diagnostic: String,
    },
}

/// Why an explicit reservation selection cannot become reusable harness-session state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExplicitSelectionPersistenceReason {
    /// The command supplied no usable harness session identifier.
    HarnessSessionUnavailable,
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
            (unavailable @ Self::Unavailable { .. }, _)
            | (_, unavailable @ Self::Unavailable { .. }) => unavailable,
            (Self::Published, next) => next,
            (
                current_invocation @ Self::ExplicitSelectionAppliesOnlyToCurrentInvocation {
                    ..
                },
                _,
            ) => current_invocation,
        }
    }

    /// Report whether an explicit selection can persist for a later invocation.
    pub(crate) fn for_explicit_reservation_selection(self) -> Self {
        match (self, HarnessSessionId::from_current_process()) {
            (unavailable @ Self::Unavailable { .. }, _) => unavailable,
            (publication, HarnessSessionIdentity::Available(_)) => publication,
            (_, HarnessSessionIdentity::Unavailable) => {
                Self::ExplicitSelectionAppliesOnlyToCurrentInvocation {
                    reason: ExplicitSelectionPersistenceReason::HarnessSessionUnavailable,
                }
            },
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
    match resolve_first_touch_mapping(ledger_directory) {
        FirstTouchSessionReservationMapping::Mapped(session_reservation_identity) => {
            SessionIdentityLookup::Mapped(session_reservation_identity)
        },
        FirstTouchSessionReservationMapping::AvailableSessionWithoutMapping
        | FirstTouchSessionReservationMapping::HarnessSessionUnavailable => {
            SessionIdentityLookup::Unavailable
        },
    }
}

/// Resolve whether locked first-touch selection has a session and reservation mapping.
pub(crate) fn resolve_first_touch_mapping(
    ledger_directory: &Path,
) -> FirstTouchSessionReservationMapping {
    let HarnessSessionIdentity::Available(harness_session_id) =
        HarnessSessionId::from_current_process()
    else {
        return FirstTouchSessionReservationMapping::HarnessSessionUnavailable;
    };
    let mapping_path = ledger_directory.join(SessionIdentityStore::FILE_NAME);
    fs::read(&mapping_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SessionIdentityStore>(&bytes).ok())
        .and_then(|store| store.identities.get(&harness_session_id).copied())
        .map_or(
            FirstTouchSessionReservationMapping::AvailableSessionWithoutMapping,
            FirstTouchSessionReservationMapping::Mapped,
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
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Self::default()),
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
