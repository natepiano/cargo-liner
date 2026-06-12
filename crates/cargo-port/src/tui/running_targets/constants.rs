// cargo install paths

pub(super) const CARGO_BIN_DIR: &str = "bin";

// src tui running_targets app_tick
pub(super) const BENCHES_DIR: &str = "benches";
pub(super) const EXAMPLES_DIR: &str = "examples";
pub(super) const SOURCE_DIR: &str = "src";

// src tui running_targets mod
/// Ceiling on the ancestor walk, against parent-link cycles from PID reuse.
/// Real process trees are nowhere near this deep.
pub(super) const ANCESTOR_WALK_CAP: usize = 32;
pub(super) const MIN_HEX_HASH_LEN: usize = 16;
