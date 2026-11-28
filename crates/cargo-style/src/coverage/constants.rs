//! Named coverage request and ledger values.

// command arguments
pub(super) const CELLS_FLAG: &str = "--cells";
pub(super) const CRATE_FLAG: &str = "--crate";
pub(super) const LEDGER_FLAG: &str = "--ledger";
pub(super) const REPO_FLAG: &str = "--repo";
pub(super) const RULE_FLAG: &str = "--rule";

// lint configuration
pub(super) const MEND_GLOBAL_CONFIG: &str = "cargo-mend/config.toml";
pub(super) const MEND_REPO_CONFIG: &str = ".cargo/mend.toml";

// protocol
pub(super) const COVERAGE_REPLY_SCHEMA: u32 = 1;
