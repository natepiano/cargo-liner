//! The overview an opening session reads in place of the complete board report.
//!
//! A session that opens in one worktree acts on the integration order alone: which reservations
//! may integrate, which wait, and which overlaps still need a `sequence` answer. The complete
//! report restates every retained reservation, most of it resolved history, so the session reads
//! how many entries each of those three sections holds, one line for each entry naming a
//! reservation its own worktree holds, and the commands that read the rest.

use super::rows::BoardModel;
use super::rows::DeferralConsequence;
use super::rows::OverlapWaitingAction;
use super::rows::ReadyReservation;
use super::rows::UnresolvedOverlap;
use super::rows::WaitingAction;
use super::rows::WaitingEntry;
use super::rows::WaitingHold;
use crate::alert::AlertRouting;
use crate::presentation;
use crate::presentation::RenderedOutputBlock;

/// The heading an overview states when no actionable notice leads the response.
const SESSION_OVERVIEW_SUMMARY: &str =
    "cargo-berth summarized the board's integration order for this session.";
/// Where the reader finds what the overview leaves out.
const BOARD_READING_POINTER: &str = "Read one reservation with `cargo-berth board --reservation <id> --json`, or filter the whole board with jq, as in `cargo-berth board --json | jq '.payload.data.waiting.entries'`.";

/// The overview of `board`'s integration order, or nothing when no entry is ready, waiting, or
/// unresolved.
pub(super) fn session_overview_block(
    board: &BoardModel,
    alert_routing: &AlertRouting,
) -> Option<RenderedOutputBlock> {
    let section_counts = [
        ("Ready now", board.ready_now.entries.len()),
        ("Waiting", board.waiting.entries.len()),
        (
            "Unresolved overlaps",
            board.unresolved_overlaps.entries.len(),
        ),
    ];
    if section_counts.iter().all(|(_, count)| *count == 0) {
        return None;
    }
    let held_here = board
        .ready_now
        .entries
        .iter()
        .filter(|ready| alert_routing.invoking_worktree_holds(&[ready.reservation.reservation_id]))
        .map(ready_line)
        .chain(
            board
                .waiting
                .entries
                .iter()
                .filter(|entry| {
                    alert_routing.invoking_worktree_holds(&[entry.reservation.reservation_id])
                })
                .map(waiting_line),
        )
        .chain(
            board
                .unresolved_overlaps
                .entries
                .iter()
                .filter(|overlap| {
                    alert_routing.invoking_worktree_holds(&[overlap.deferred, overlap.blocker])
                })
                .map(overlap_line),
        )
        .collect::<Vec<_>>();
    let lines = section_counts
        .iter()
        .filter(|(_, count)| *count > 0)
        .map(|(section, count)| {
            let entries = if *count == 1 { "entry" } else { "entries" };
            format!("{section}: {count} {entries}.")
        })
        .chain(std::iter::once(format!(
            "Entries naming a reservation this worktree holds: {}.",
            held_here.len()
        )))
        .chain(held_here)
        .chain(std::iter::once(BOARD_READING_POINTER.to_owned()))
        .collect::<Vec<_>>();
    Some(presentation::engine_message_block(
        SESSION_OVERVIEW_SUMMARY,
        &lines.join("\n"),
    ))
}

fn ready_line(ready: &ReadyReservation) -> String {
    format!(
        "Ready now: reservation {} may integrate; no ordering edge or overlap holds it.",
        ready.reservation.reservation_id
    )
}

fn waiting_line(entry: &WaitingEntry) -> String {
    let held = entry.reservation.reservation_id;
    match &entry.hold {
        WaitingHold::OrderingEdge {
            predecessor,
            action,
            ..
        } => {
            let action = match action {
                WaitingAction::TrunkEvidenceRewritten {
                    instruction,
                    resolve_flag,
                } => format!("{instruction} with `cargo-berth {resolve_flag}`"),
                WaitingAction::PredecessorCheckpoint { instruction }
                | WaitingAction::PredecessorNotIntegrated { instruction }
                | WaitingAction::PredecessorObjectUnknown { instruction }
                | WaitingAction::SuccessorMustIncorporatePredecessor { instruction } => {
                    instruction.clone()
                },
            };
            format!("Waiting: reservation {held} waits on {predecessor}: {action}.")
        },
        WaitingHold::UnresolvedOverlap {
            action: OverlapWaitingAction::OverlapNotSequenced { instruction },
            ..
        } => format!("Waiting: reservation {held}: {instruction}."),
    }
}

fn overlap_line(overlap: &UnresolvedOverlap) -> String {
    let held = match overlap.consequence {
        DeferralConsequence::Both => "both integrations are",
        DeferralConsequence::DeferredOnly => "the deferred integration is",
        DeferralConsequence::BlockerOnly => "the blocker's integration is",
    };
    format!(
        "Unresolved overlap: reservation {} deferred to {}; {held} held until `sequence` orders the pair.",
        overlap.deferred, overlap.blocker
    )
}
