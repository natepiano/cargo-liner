//! Route an explicit reservation through its resolved session owner.

use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use super::HarnessSessionId;
use super::HarnessSessionIdentity;
use super::SessionIdentityStore;
use super::SessionReservationIdentity;
use crate::ids::ReservationId;
use crate::ledger;
use crate::ledger::Ledger;
use crate::ledger::WorktreeContext;
use crate::output::CommandVerb;
use crate::output::OutputEnvelope;

static COMMAND_RESERVATION: OnceLock<OwnedReservationCommand> = OnceLock::new();

/// One command's holder directory and session identity.
struct OwnedReservationCommand {
    root:             PathBuf,
    ledger_directory: PathBuf,
    identity:         SessionReservationIdentity,
}

/// The directory a command discovers its worktree from: the selected holder's
/// root, or else the process directory. The journal still records the process
/// directory.
pub(crate) fn command_directory() -> std::io::Result<PathBuf> {
    COMMAND_RESERVATION
        .get()
        .map_or_else(std::env::current_dir, |command| Ok(command.root.clone()))
}

pub(super) fn selected_identity(ledger_directory: &Path) -> Option<SessionReservationIdentity> {
    COMMAND_RESERVATION
        .get()
        .filter(|command| command.ledger_directory == ledger_directory)
        .map(|command| command.identity)
}

/// Route a command to the holder of an active reservation that the resolved
/// session claimed. Any other reservation keeps the existing worktree rules.
pub(crate) fn select_owned_reservation(
    reservation_id: ReservationId,
    verb: CommandVerb,
) -> Result<(), Box<OutputEnvelope>> {
    let HarnessSessionIdentity::Available {
        harness_session_id, ..
    } = HarnessSessionId::from_current_process()
    else {
        return Ok(());
    };
    let directory = std::env::current_dir()
        .map_err(|error| OutputEnvelope::ledger_unreadable(verb, &error.to_string()))?;
    let context = WorktreeContext::discover(&directory)
        .map_err(|error| OutputEnvelope::ledger_error(verb, &error))?;
    let ledger_directory = context.ledger_directory();
    let store = SessionIdentityStore::read_for_removal(
        &ledger_directory.join(SessionIdentityStore::FILE_NAME),
    )
    .map_err(|error| OutputEnvelope::ledger_unreadable(verb, &error.to_string()))?;
    let journal = Ledger::open_from_discovered_worktree(&context)
        .and_then(|ledger| ledger.read_validated_journal())
        .map_err(|error| OutputEnvelope::ledger_error(verb, &error))?;
    let reservations = journal
        .into_reservations()
        .map_err(|error| OutputEnvelope::replay_failure(verb, &error))?;
    // Let the existing validator report a stale current selection.
    if store
        .identities
        .get(&harness_session_id)
        .is_some_and(|identity| {
            !reservations.iter().any(|reservation| {
                reservation.id() == identity.reservation_id
                    && reservation.is_active_for_coordination_run(identity.coordination_run_id)
            })
        })
    {
        return Ok(());
    }
    // A reservation this session does not own, or one no longer active, keeps
    // the existing worktree rules and their refusals.
    let Some(run) = store
        .owned_reservations
        .get(&harness_session_id)
        .and_then(|owned| owned.get(&reservation_id))
    else {
        return Ok(());
    };
    let Some(reservation) = reservations.iter().find(|reservation| {
        reservation.id() == reservation_id && reservation.is_active_for_coordination_run(*run)
    }) else {
        return Ok(());
    };
    let holder = WorktreeContext::discover(reservation.worktree_root().as_ref())
        .map_err(|error| OutputEnvelope::ledger_error(verb, &error))?;
    let holder_id = ledger::read_worktree_identity(holder.administrative_directory())
        .map_err(|error| OutputEnvelope::ledger_error(verb, &error))?;
    if holder_id != reservation.actor().worktree
        || holder.common_git_directory() != context.common_git_directory()
    {
        return Err(Box::new(OutputEnvelope::invalid_input(
            verb,
            "the requested reservation holder identity has changed",
        )));
    }
    let command = OwnedReservationCommand {
        root: holder.repository_root().to_path_buf(),
        ledger_directory,
        identity: SessionReservationIdentity::new(*run, reservation_id),
    };
    COMMAND_RESERVATION.set(command).map_err(|_| {
        Box::new(OutputEnvelope::invalid_input(
            verb,
            "this command already selected a reservation",
        ))
    })
}
