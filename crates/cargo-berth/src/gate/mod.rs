//! Locked target-update decisions shared by `integrate` and the git hook.

mod audit;
mod decision;
mod error;
pub(crate) mod install;
pub(crate) mod permit;
mod reference_transaction;
mod rewrite;
pub(crate) mod rewrite_map;

pub(crate) use decision::GateDecision;
pub(crate) use decision::GateResult;
pub(crate) use decision::IntegrationRequest;
pub(crate) use decision::IntegrationViolation;
pub(crate) use decision::evaluate_integration;
pub(crate) use error::GateError;
pub(crate) use reference_transaction::IssuingCheckout;
pub(crate) use reference_transaction::ManagedTrunkDeletion;
pub(crate) use reference_transaction::ProposedTargetMove;
pub(crate) use reference_transaction::REFERENCE_TRANSACTION_ISSUING_DIRECTORY_ENVIRONMENT;
pub(crate) use reference_transaction::ReferenceTransaction;
pub(crate) use reference_transaction::ReferenceTransactionParseError;
pub(crate) use reference_transaction::ReferenceTransactionPhase;
pub(crate) use reference_transaction::TrunkReferencePresence;
pub(crate) use reference_transaction::evaluate_reference_transaction;
pub(crate) use reference_transaction::parse_reference_transaction;
#[cfg(test)]
pub(crate) use rewrite::RewriteBase;
#[cfg(test)]
pub(crate) use rewrite::capture_rewrite_created_commits;
pub(crate) use rewrite::rewrite_phase_commits;
pub(crate) use rewrite::rewritten_first_parent_history;
pub(crate) use rewrite_map::RewriteCreatedCommits;
pub(crate) use rewrite_map::map_phase_interval;
