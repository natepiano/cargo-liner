//! Constants for the shared reservation ledger.

use std::time::Duration;

// file names
pub(super) const COORDINATION_RUN_MARKER_FILE_NAME: &str = "cargo-berth-run-id";
pub(super) const COORDINATION_RUN_MARKER_RETIREMENT_SUFFIX: &str = "retiring";
pub(super) const JOURNAL_FILE_NAME: &str = "journal.ndjson";
pub(super) const LOCK_FILE_NAME: &str = "mutation.lock";
pub(super) const PROJECTION_FILE_NAME: &str = "reservations.json";
pub(super) const PROJECTION_TEMPORARY_FILE_NAME: &str = "reservations.json.tmp";
pub(super) const REPO_INSTANCE_ID_FILE_NAME: &str = "repo-instance-id";
pub(super) const WORKTREE_ID_FILE_NAME: &str = "cargo-berth-worktree-id";

// process context
pub(super) const COORDINATION_RUN_ENVIRONMENT: &str = "CARGO_BERTH_RUN";

// lock acquisition
pub(super) const MUTATING_VERB_CONTENTION_TOLERANCE: Duration = Duration::from_secs(10);
pub(super) const MUTATION_LOCK_INITIAL_RETRY_INTERVAL: Duration = Duration::from_millis(50);
pub(super) const MUTATION_LOCK_MAXIMUM_RETRY_INTERVAL: Duration = Duration::from_secs(1);

// git reference validation
pub(super) const DELETE_CONTROL_BYTE: u8 = 0x7f;

// journal limits
pub(super) const MAXIMUM_JOURNAL_RECORD_BYTES: usize = 16 * 1_024;

// ledger layout
pub(super) const LEDGER_DIRECTORY_NAME: &str = "cargo-berth";

// wire format
/// The one schema version the journal writes and the projection expects; the
/// two must never drift apart.
pub(super) const CURRENT_SCHEMA_VERSION: u32 = 1;
