//! Constants for the shared reservation ledger.

// file names
pub(super) const JOURNAL_FILE_NAME: &str = "journal.ndjson";
pub(super) const LOCK_FILE_NAME: &str = "mutation.lock";
pub(super) const PROJECTION_FILE_NAME: &str = "reservations.json";
pub(super) const PROJECTION_TEMPORARY_FILE_NAME: &str = "reservations.json.tmp";
pub(super) const REPO_INSTANCE_ID_FILE_NAME: &str = "repo-instance-id";
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Worktree reconciliation consumes this identity-file locator; no verb reaches it yet."
    )
)]
pub(super) const WORKTREE_ID_FILE_NAME: &str = "cargo-berth-worktree-id";

// journal limits
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "The journal writer enforces this record limit; no writer path reaches it yet."
    )
)]
pub(super) const MAXIMUM_JOURNAL_RECORD_BYTES: usize = 16 * 1_024;

// ledger layout
pub(super) const LEDGER_DIRECTORY_NAME: &str = "cargo-berth";

// wire format
/// The one schema version the journal writes and the projection expects; the
/// two must never drift apart.
pub(super) const CURRENT_SCHEMA_VERSION: u32 = 1;
