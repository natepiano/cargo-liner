//! Headless reservation-board projection and its machine-readable sections.

mod alerts;
mod answers;
mod error;
mod report;
mod rows;
pub(crate) mod tui;

#[cfg(test)]
mod test_support;

pub(crate) use report::reservation_lifecycle_presentation;
pub(crate) use report::reservation_lifecycle_snapshot;
pub(crate) use rows::BoardModel;
pub(crate) use rows::LiveIncursionMembership;
