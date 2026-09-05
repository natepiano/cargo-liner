//! Locked trunk-update decisions shared by `integrate` and the git hook.

mod audit;
mod decision;
mod error;
pub(crate) mod install;
pub(crate) mod permit;
mod reference_transaction;
mod rewrite;

pub(crate) use decision::GateDecision;
pub(crate) use decision::GateResult;
pub(crate) use decision::IntegrationRequest;
pub(crate) use decision::IntegrationViolation;
pub(crate) use decision::evaluate_integration;
pub(crate) use error::GateError;
pub(crate) use reference_transaction::ManagedTrunkDeletion;
pub(crate) use reference_transaction::REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT;
pub(crate) use reference_transaction::ReferenceTransaction;
pub(crate) use reference_transaction::ReferenceTransactionParseError;
pub(crate) use reference_transaction::ReferenceTransactionPhase;
pub(crate) use reference_transaction::TrunkReferencePresence;
pub(crate) use reference_transaction::evaluate_reference_transaction;
pub(crate) use reference_transaction::parse_reference_transaction;
