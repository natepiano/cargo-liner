//! Conversion of observed processes into build sessions and compile activities.
//!
//! - [`activity`] — compile-activity identity and compiler-to-session attribution
//! - [`build_classifier`] — the mutable caches and the filesystem work around the pure call
//! - [`classify`] — the pure snapshot-to-classification function
//! - [`constants`] — values shared by classification and its caches
//! - [`execution`] — one refresh cycle's classification demand and outcome
//! - [`scope`] — the roots-and-revisions projection of a monitor scope
//! - [`session`] — build-session identity, scope attribution, and session records

mod activity;
#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "the timing harness should fail on invalid fixture configuration"
)]
mod benchmarks;
mod build_classifier;
mod classify;
#[cfg(test)]
mod classify_tests;
mod constants;
mod execution;
mod scope;
mod session;

pub(crate) use build_classifier::BuildClassifier;
#[cfg(test)]
pub(crate) use execution::BuildClassificationExecutionFailure;
pub(crate) use execution::CompileClassificationCancellation;
pub(crate) use execution::CompileClassificationDemand;
pub(crate) use execution::CompileClassificationExecution;
pub(crate) use execution::CompileMonitorGeneration;
pub(crate) use scope::BuildScopeActionability;
pub(crate) use scope::BuildScopeKey;
pub(crate) use scope::CoveredScopeRoots;
pub(crate) use scope::ScopeRootCoverage;
pub(crate) use session::LiveOwnedRoot;
pub(crate) use session::OwnedRootEvidence;
pub(crate) use session::OwnedRootLifecycle;
