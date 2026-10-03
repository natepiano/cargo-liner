//! Engine-authored explanations recorded with overlap authorizations.

/// The explanation for an override recorded by `default_answer = "first_ready"`.
pub(super) const DEFAULT_ANSWER_FIRST_READY_REASON: &str = "Recorded by the repository default answer (default_answer = \"first_ready\" in .claude/config/berth.toml): no integration order is recorded; whichever checkpoint is ready first merges first and the other lane merges that work in";

/// The explanation for a deferral recorded by enrollment.
pub(super) const ENROLLMENT_AUTHORIZATION_REASON: &str =
    "Enrollment preserves shared edits; sequence these reservations before integration";
