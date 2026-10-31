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
use crate::ledger::FullRefName;
use crate::ledger::IntegrationTarget;
use crate::ledger::LedgerError;
use crate::ledger::LedgerTransactionError;
use crate::reconcile::GateReconciliationError;
use crate::reconcile::ReconcileError;

#[derive(Debug)]
pub(super) enum GateTransactionRejection {
    ReconciliationPreparation(ReconcileError),
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
    UnsupportedSymbolicTargetUpdate(IntegrationTarget),
    MultipleGatedReferences(Vec<FullRefName>),
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
            Self::CoordinationIdentity(coordination_identity_rejection) => {
                coordination_identity_rejection.fmt(formatter)
            },
            Self::ReservationNotEntering(reservation_id) => write!(
                formatter,
                "reservation {reservation_id} is not newly reachable in the proposed target update"
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
            Self::UnsupportedSymbolicTargetUpdate(integration_target) => write!(
                formatter,
                "target {} received a symbolic-ref update instead of a commit update",
                integration_target.short_name(),
            ),
            Self::MultipleGatedReferences(references) => write!(
                formatter,
                "one transaction would move multiple gated refs ({}); move them one at a time",
                references
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }
}

impl Error for GateError {}

impl From<GateTransactionRejection> for GateError {
    fn from(gate_transaction_rejection: GateTransactionRejection) -> Self {
        match gate_transaction_rejection {
            GateTransactionRejection::ReconciliationPreparation(error) => {
                Self::Reconciliation(error)
            },
            GateTransactionRejection::Reconciliation(error) => Self::Planning(error),
            GateTransactionRejection::Git(error) => Self::Git(error),
            GateTransactionRejection::PermitReplay(error) => Self::PermitReplay(error),
            GateTransactionRejection::CoordinationIdentity(coordination_identity_rejection) => {
                Self::CoordinationIdentity(coordination_identity_rejection)
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
