//! The failure families a gate decision reports, and the internal rejection it converts from.

use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use super::permit::ForcedIntegrationPermitReplayError;
use crate::config::ConfigError;
use crate::coordination_identity::CoordinationIdentityRejection;
use crate::edge::MissingReadinessFact;
use crate::git::GitError;
use crate::ids::ReservationId;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::reconcile::GateReconciliationError;
use crate::reconcile::ReconcileError;

#[derive(Debug)]
pub(super) enum GateTransactionRejection {
    Reconciliation(GateReconciliationError),
    Git(GitError),
    PermitReplay(ForcedIntegrationPermitReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    InvalidCanonicalWorktreeRoot,
    ReservationNotEntering(ReservationId),
    NoHoldToForce(ReservationId),
    MissingSkippedHold,
    MissingConstraintFact(MissingReadinessFact),
}

/// A gate decision failed before it could establish safe integration facts.
#[derive(Debug)]
pub(crate) enum GateError {
    Config(ConfigError),
    Ledger(LedgerError),
    Transaction(LedgerTransactionError),
    Reconciliation(ReconcileError),
    Planning(GateReconciliationError),
    Git(GitError),
    PermitReplay(ForcedIntegrationPermitReplayError),
    CoordinationIdentity(CoordinationIdentityRejection),
    ReservationNotEntering(ReservationId),
    NoHoldToForce(ReservationId),
    MissingSkippedHold,
    MissingConstraintFact(MissingReadinessFact),
    HookReportedNoIssuingDirectory,
    UnsupportedSymbolicTrunkUpdate,
}

impl Display for GateError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => error.fmt(formatter),
            Self::Ledger(error) => error.fmt(formatter),
            Self::Transaction(error) => error.fmt(formatter),
            Self::Reconciliation(error) => error.fmt(formatter),
            Self::Planning(error) => error.fmt(formatter),
            Self::Git(error) => error.fmt(formatter),
            Self::PermitReplay(error) => error.fmt(formatter),
            Self::CoordinationIdentity(rejection) => rejection.fmt(formatter),
            Self::ReservationNotEntering(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is not newly reachable in the proposed main update"
            ),
            Self::NoHoldToForce(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} has no integration hold to force"
            ),
            Self::MissingSkippedHold => {
                formatter.write_str("a forced integration found no hold to record")
            },
            Self::MissingConstraintFact(error) => error.fmt(formatter),
            Self::HookReportedNoIssuingDirectory => formatter.write_str(
                "the managed reference-transaction hook did not report its issuing directory",
            ),
            Self::UnsupportedSymbolicTrunkUpdate => formatter.write_str(
                "the configured trunk received a symbolic-ref update instead of a commit update",
            ),
        }
    }
}

impl Error for GateError {}

impl From<GateTransactionRejection> for GateError {
    fn from(rejection: GateTransactionRejection) -> Self {
        match rejection {
            GateTransactionRejection::Reconciliation(error) => Self::Planning(error),
            GateTransactionRejection::Git(error) => Self::Git(error),
            GateTransactionRejection::PermitReplay(error) => Self::PermitReplay(error),
            GateTransactionRejection::CoordinationIdentity(rejection) => {
                Self::CoordinationIdentity(rejection)
            },
            GateTransactionRejection::InvalidCanonicalWorktreeRoot => {
                Self::Ledger(LedgerError::InvalidCanonicalWorktreeRoot)
            },
            GateTransactionRejection::ReservationNotEntering(reservation_id) => {
                Self::ReservationNotEntering(reservation_id)
            },
            GateTransactionRejection::NoHoldToForce(reservation_id) => {
                Self::NoHoldToForce(reservation_id)
            },
            GateTransactionRejection::MissingSkippedHold => Self::MissingSkippedHold,
            GateTransactionRejection::MissingConstraintFact(error) => {
                Self::MissingConstraintFact(error)
            },
        }
    }
}

impl From<ConfigError> for GateError {
    fn from(error: ConfigError) -> Self { Self::Config(error) }
}

impl From<LedgerError> for GateError {
    fn from(error: LedgerError) -> Self { Self::Ledger(error) }
}
