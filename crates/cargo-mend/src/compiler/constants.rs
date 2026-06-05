// build-fingerprint fallbacks
pub(super) const BUILD_ID_FALLBACK: &str = "nobuild";
pub(super) const GIT_HASH_FALLBACK: &str = "nogit";

// driver-IPC environment variables
pub(crate) const CONFIG_FINGERPRINT_ENV: &str = "MEND_CONFIG_FINGERPRINT";
pub(crate) const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
pub(crate) const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
pub(crate) const DRIVER_ENV: &str = "MEND_DRIVER";
pub(crate) const DRIVER_ENV_ENABLED: &str = "1";
pub(crate) const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
pub(crate) const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
pub(crate) const RUSTC_WORKSPACE_WRAPPER_ENV: &str = "RUSTC_WORKSPACE_WRAPPER";
pub(crate) const SCOPE_FINGERPRINT_ENV: &str = "MEND_SCOPE_FINGERPRINT";

// findings
pub(crate) const FINDINGS_DIR_NAME: &str = "mend-findings";
pub(crate) const FINDINGS_SCHEMA_VERSION: u32 = 16;

// file extensions
pub(crate) const JSON_FILE_EXTENSION: &str = "json";

// source-tree directories
pub(crate) const SOURCE_DIR_BENCHES: &str = "benches";
pub(crate) const SOURCE_DIR_EXAMPLES: &str = "examples";
pub(crate) const SOURCE_DIR_SRC: &str = "src";
pub(crate) const SOURCE_DIR_TESTS: &str = "tests";
