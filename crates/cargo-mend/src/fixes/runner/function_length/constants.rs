// Clippy uses this default when the workspace does not set a threshold.
pub(super) const CLIPPY_DEFAULT_TOO_MANY_LINES_THRESHOLD: usize = 100;
pub(super) const RUSTUP_COMMAND: &str = "rustup";
pub(super) const RUSTFMT_COMMAND: &str = "rustfmt";
pub(super) const NIGHTLY_RUSTFMT_ARGUMENTS: &[&str] = &["run", "nightly", RUSTFMT_COMMAND];
pub(super) const NIGHTLY_RUSTFMT_LOOKUP_ARGUMENTS: &[&str] =
    &["which", "--toolchain", "nightly", RUSTFMT_COMMAND];
pub(super) const RUSTFMT_ARGUMENTS: &[&str] = &["--emit", "stdout", "--quiet", "--edition"];
pub(super) const RUSTFMT_LOCAL_FILE_ARGUMENTS: &[&str] = &["--config", "skip_children=true"];
pub(super) const VERSION_ARGUMENT: &str = "--version";
pub(super) const DEFAULT_EDITION: &str = "2015";
