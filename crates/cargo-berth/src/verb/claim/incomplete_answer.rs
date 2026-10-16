//! Incomplete overlap answers and the complete claim a caller can retry.

use std::collections::HashSet;
use std::fmt::Write as _;

use super::recovery_command::ClaimRecoveryCommand;
use crate::answer::DeferAnswerRequest;
use crate::answer::PermissiveOverlapAnswer;
use crate::answer::PermissiveOverlapAuthorizationRequest;
use crate::coordination_identity;
use crate::ids::ReservationId;
use crate::ids::WireOrderedReservationIds;
use crate::ledger::OrderingDirection;
use crate::reservation::ReservationConflict;

/// The overlap decision one holder receives in the retry command.
#[derive(Clone, Copy, Eq, PartialEq)]
enum ClaimAnswerFlag {
    Before,
    After,
    Defer,
    Override,
    ChooseOrdering,
}

impl ClaimAnswerFlag {
    const fn argument(self) -> &'static str {
        match self {
            Self::Before => "--before",
            Self::After => "--after",
            Self::Defer => "--defer",
            Self::Override => "--override",
            Self::ChooseOrdering => "--<before|after>",
        }
    }
}

/// An explicit answer whose named holders differ from the locked conflict set.
pub(super) enum IncompleteClaimAnswer {
    Permissive(PermissiveOverlapAuthorizationRequest),
    Defer(DeferAnswerRequest),
}

impl IncompleteClaimAnswer {
    fn flags(&self) -> Vec<(ReservationId, ClaimAnswerFlag)> {
        let mut flags = match self {
            Self::Permissive(request) => match &request.answer {
                PermissiveOverlapAnswer::Sequence { blockers, .. } => blockers
                    .iter()
                    .map(|holder| {
                        (
                            holder.blocker,
                            match holder.direction {
                                OrderingDirection::RequesterBeforeHolder => ClaimAnswerFlag::Before,
                                OrderingDirection::HolderBeforeRequester => ClaimAnswerFlag::After,
                            },
                        )
                    })
                    .collect(),
                PermissiveOverlapAnswer::Override { blocker } => {
                    vec![(*blocker, ClaimAnswerFlag::Override)]
                },
            },
            Self::Defer(request) => request
                .blockers
                .iter()
                .map(|id| (*id, ClaimAnswerFlag::Defer))
                .collect(),
        };
        flags.sort_by_cached_key(|(id, _)| id.wire_ordering_key());
        flags
    }

    fn reason(&self) -> String {
        match self {
            Self::Permissive(request) => request.reason.to_string(),
            Self::Defer(request) => request.reason.to_string(),
        }
    }
}

/// Locked holder facts and the caller's answer retained without changing the blocked payload.
pub(super) struct IncompleteClaimAnswerRejection {
    pub(super) conflicts:  Vec<ReservationConflict>,
    answer:                IncompleteClaimAnswer,
    unnamed:               WireOrderedReservationIds,
    no_longer_conflicting: WireOrderedReservationIds,
}

impl IncompleteClaimAnswerRejection {
    pub(super) fn new(conflicts: Vec<ReservationConflict>, answer: IncompleteClaimAnswer) -> Self {
        let named = answer
            .flags()
            .into_iter()
            .map(|(id, _)| id)
            .collect::<HashSet<_>>();
        let holders = conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect::<HashSet<_>>();
        let unnamed =
            WireOrderedReservationIds::sorted(holders.difference(&named).copied().collect());
        let no_longer_conflicting =
            WireOrderedReservationIds::sorted(named.difference(&holders).copied().collect());
        Self {
            conflicts,
            answer,
            unnamed,
            no_longer_conflicting,
        }
    }

    pub(super) fn message(&self, recovery: &ClaimRecoveryCommand) -> String {
        let named = self
            .answer
            .flags()
            .iter()
            .map(|(id, _)| id.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        let unnamed = self
            .unnamed
            .as_slice()
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let mut message = if self.unnamed.as_slice().is_empty() {
            format!(
                "The answer names {named}, including reservations that no longer overlap the requested paths; one claim answers every holder."
            )
        } else {
            let hold = if self.unnamed.as_slice().len() == 1 {
                "holds"
            } else {
                "hold"
            };
            format!(
                "The answer names {named} but not {unnamed}, which also {hold} paths that overlap the requested paths; one claim answers every holder."
            )
        };
        for id in self.no_longer_conflicting.as_slice() {
            let _ = write!(
                message,
                " {id} no longer overlaps the requested paths, so the command leaves it out."
            );
        }
        if matches!(
            &self.answer,
            IncompleteClaimAnswer::Permissive(PermissiveOverlapAuthorizationRequest {
                answer: PermissiveOverlapAnswer::Override { .. },
                ..
            })
        ) && self
            .conflicts
            .iter()
            .map(|conflict| conflict.reservation_id)
            .collect::<HashSet<_>>()
            .len()
            > 1
        {
            message.push_str(" --override settles one holder; narrow the requested scopes until one holder remains, or answer every holder with --before, --after or --defer.");
        } else {
            let _ = write!(message, " Run: `{}`", self.complete_command(recovery));
        }
        message
    }

    fn complete_command(&self, recovery: &ClaimRecoveryCommand) -> String {
        let mut command = recovery.leading_arguments();
        let flags = self.answer.flags();
        let missing_flag = match flags.first() {
            Some((_, first)) if flags.iter().all(|(_, flag)| flag == first) => *first,
            _ => ClaimAnswerFlag::ChooseOrdering,
        };
        for (id, flag) in &flags {
            if !self.no_longer_conflicting.as_slice().contains(id) {
                command.extend([flag.argument().to_owned(), id.to_string()]);
            }
        }
        for id in self.unnamed.as_slice() {
            command.extend([missing_flag.argument().to_owned(), id.to_string()]);
        }
        command.push(format!(
            "--overlap-why={}",
            coordination_identity::shell_quote(&self.answer.reason())
        ));
        recovery.finish_arguments(&mut command);
        command.join(" ")
    }
}
