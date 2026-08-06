use std::time::Duration;

// binary names
pub(super) const CARGO_BIN: &str = "cargo";
pub(super) const RUSTC_BIN: &str = "rustc";

// build-fingerprint fallbacks
pub(super) const BUILD_ID_FALLBACK: &str = "nobuild";
pub(super) const GIT_HASH_FALLBACK: &str = "nogit";

// cargo cli flags
pub(super) const CARGO_FLAG_ALL_FEATURES: &str = "--all-features";
pub(crate) const CARGO_FLAG_ALL_TARGETS: &str = "--all-targets";
pub(super) const CARGO_FLAG_ALLOW_DIRTY: &str = "--allow-dirty";
pub(super) const CARGO_FLAG_ALLOW_STAGED: &str = "--allow-staged";
pub(crate) const CARGO_FLAG_EXCLUDE: &str = "--exclude";
pub(super) const CARGO_FLAG_FEATURES: &str = "--features";
pub(crate) const CARGO_FLAG_MANIFEST_PATH: &str = "--manifest-path";
pub(super) const CARGO_FLAG_NO_DEFAULT_FEATURES: &str = "--no-default-features";
pub(crate) const CARGO_FLAG_PACKAGE: &str = "--package";
pub(super) const CARGO_FLAG_TESTS: &str = "--tests";
pub(crate) const CARGO_FLAG_WORKSPACE: &str = "--workspace";

// cargo output protocol
pub(super) const CARGO_PROGRESS_PREFIX_BLOCKING: &str = "Blocking waiting for file lock";
pub(super) const CARGO_PROGRESS_PREFIX_BUILDING: &str = "Building ";
pub(super) const CARGO_PROGRESS_PREFIX_CHECKING: &str = "Checking ";
pub(super) const CARGO_PROGRESS_PREFIX_COMPILING: &str = "Compiling ";
pub(super) const CARGO_PROGRESS_PREFIX_FINISHED: &str = "Finished ";
pub(super) const CARGO_PROGRESS_PREFIX_FRESH: &str = "Fresh ";
pub(super) const CARGO_UNUSED_IMPORT_WARNING: &str = "warning: unused import:";
pub(super) const CARGO_UNUSED_IMPORTS_WARNING: &str = "warning: unused imports:";
pub(super) const CARGO_WARNING_SUMMARY_PREFIX: &str = "warning: `";
pub(super) const CARGO_WARNING_SUMMARY_TOKEN_GENERATED: &str = " generated ";
pub(super) const CARGO_WARNING_SUMMARY_TOKEN_TO_APPLY: &str = "to apply ";

// cargo subcommands
pub(super) const CARGO_SUBCOMMAND_CHECK: &str = "check";
pub(super) const CARGO_SUBCOMMAND_FIX: &str = "fix";
pub(crate) const CARGO_SUBCOMMAND_MEND: &str = "mend";

// diagnostic severity prefixes
pub(crate) const DIAGNOSTIC_SEVERITY_ERROR_PREFIX: &str = "error:";
pub(crate) const DIAGNOSTIC_SEVERITY_WARNING_PREFIX: &str = "warning:";

// driver-ipc environment variables
pub(super) const ANALYZING_DIR_NAME: &str = "analyzing";
pub(super) const CARGO_PRIMARY_PACKAGE_ENV: &str = "CARGO_PRIMARY_PACKAGE";
pub(super) const CONFIG_FINGERPRINT_ENV: &str = "MEND_CONFIG_FINGERPRINT";
pub(super) const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
pub(super) const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
pub(crate) const DRIVER_ENV: &str = "MEND_DRIVER";
pub(super) const DRIVER_ENV_ENABLED: &str = "1";
pub(super) const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
pub(super) const PASSTHROUGH_RUSTC_WRAPPER_ENV: &str = "MEND_PASSTHROUGH_RUSTC_WRAPPER";
pub(super) const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
pub(super) const RUSTC_WORKSPACE_WRAPPER_ENV: &str = "RUSTC_WORKSPACE_WRAPPER";
pub(super) const SCOPE_FINGERPRINT_ENV: &str = "MEND_SCOPE_FINGERPRINT";

// file extensions
pub(super) const JSON_FILE_EXTENSION: &str = "json";

// findings
pub(super) const FINDINGS_DIR_NAME: &str = "mend-findings";
pub(super) const FINDINGS_SCHEMA_VERSION: u32 = 27;

// progress indicator
pub(super) const PROGRESS_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
pub(super) const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

// source-tree directories
pub(super) const SOURCE_DIR_BENCHES: &str = "benches";
pub(super) const SOURCE_DIR_EXAMPLES: &str = "examples";
pub(crate) const SOURCE_DIR_SRC: &str = "src";
pub(super) const SOURCE_DIR_TESTS: &str = "tests";

// visibility policy
pub(super) const PRELUDE_MODULE_NAME: &str = "prelude";

// wrapper alias
pub(super) const WRAPPER_DIR_NAME: &str = "mend-wrapper";
pub(super) const WRAPPER_ALIAS_STAGING_EXTENSION: &str = "staging";
