// ANSI codes
pub(crate) const ANSI_BOLD: &str = "1";
pub(crate) const ANSI_BOLD_BLUE: &str = "1;34";
pub(crate) const ANSI_BOLD_GREEN: &str = "1;32";
pub(crate) const ANSI_BOLD_RED: &str = "1;31";
pub(crate) const ANSI_BOLD_YELLOW: &str = "1;33";
pub(crate) const ANSI_DIM: &str = "2";

// Config
pub(crate) const APP_NAME: &str = "cargo-mend";
pub(crate) const GLOBAL_CONFIG_FILE: &str = "config.toml";

// Environment variables
pub(crate) const CONFIG_FINGERPRINT_ENV: &str = "MEND_CONFIG_FINGERPRINT";
pub(crate) const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
pub(crate) const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
pub(crate) const DRIVER_ENV: &str = "MEND_DRIVER";
pub(crate) const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
pub(crate) const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";

// Exit codes
pub(crate) const EXIT_CODE_ERROR: u8 = 1;
pub(crate) const EXIT_CODE_WARNING: u8 = 2;

// Findings
pub(crate) const FINDINGS_SCHEMA_VERSION: u32 = 13;

// Visibility
pub(crate) const PUB_VISIBILITY_PREFIX: &str = "pub ";
