//! Conversion of observed processes into build sessions and compile activities.
//!
//! - [`activity`] — compile-activity identity and compiler-to-session attribution
//! - [`build_classifier`] — the mutable caches and the filesystem work around the pure call
//! - [`classify`] — the pure snapshot-to-classification function
//! - [`constants`] — values shared by classification and its caches
//! - [`scope`] — the roots-and-revisions projection of a monitor scope
//! - [`session`] — build-session identity, scope attribution, and session records

mod activity;
mod build_classifier;
mod classify;
#[cfg(test)]
mod classify_tests;
mod constants;
mod scope;
mod session;

pub(crate) use scope::BuildScopeActionability;
pub(crate) use scope::BuildScopeKey;
pub(crate) use session::LiveOwnedRoot;
pub(crate) use session::OwnedRootEvidence;
pub(crate) use session::OwnedRootLifecycle;
