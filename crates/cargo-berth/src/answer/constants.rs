//! Engine-authored explanations recorded with overlap authorizations.

/// The explanation for a deferral recorded by `default_answer = "holder_first"`.
pub(super) const DEFAULT_ANSWER_HOLDER_FIRST_REASON: &str = "Recorded by the repository default answer (default_answer = \"holder_first\" in .claude/config/berth.toml): this worktree defers the integration order to every holder of the overlapping paths";

/// The explanation for a deferral recorded by enrollment.
pub(super) const ENROLLMENT_AUTHORIZATION_REASON: &str =
    "Enrollment preserves shared edits; sequence these reservations before integration";
