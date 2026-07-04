use std::time::Duration;

// binary names
pub(crate) const CARGO_BIN: &str = "cargo";
pub(crate) const RUSTC_BIN: &str = "rustc";

// build-fingerprint fallbacks
pub(super) const BUILD_ID_FALLBACK: &str = "nobuild";
pub(super) const GIT_HASH_FALLBACK: &str = "nogit";

// cargo cli flags
pub(crate) const CARGO_FLAG_ALL_TARGETS: &str = "--all-targets";
pub(crate) const CARGO_FLAG_ALLOW_DIRTY: &str = "--allow-dirty";
pub(crate) const CARGO_FLAG_ALLOW_STAGED: &str = "--allow-staged";
pub(crate) const CARGO_FLAG_EXCLUDE: &str = "--exclude";
pub(crate) const CARGO_FLAG_MANIFEST_PATH: &str = "--manifest-path";
pub(crate) const CARGO_FLAG_PACKAGE: &str = "--package";
pub(crate) const CARGO_FLAG_TESTS: &str = "--tests";
pub(crate) const CARGO_FLAG_WORKSPACE: &str = "--workspace";

// cargo output protocol
pub(crate) const CARGO_PROGRESS_PREFIX_BLOCKING: &str = "Blocking waiting for file lock";
pub(crate) const CARGO_PROGRESS_PREFIX_BUILDING: &str = "Building ";
pub(crate) const CARGO_PROGRESS_PREFIX_CHECKING: &str = "Checking ";
pub(crate) const CARGO_PROGRESS_PREFIX_COMPILING: &str = "Compiling ";
pub(crate) const CARGO_PROGRESS_PREFIX_FINISHED: &str = "Finished ";
pub(crate) const CARGO_PROGRESS_PREFIX_FRESH: &str = "Fresh ";
pub(crate) const CARGO_UNUSED_IMPORT_WARNING: &str = "warning: unused import:";
pub(crate) const CARGO_UNUSED_IMPORTS_WARNING: &str = "warning: unused imports:";
pub(crate) const CARGO_WARNING_SUMMARY_PREFIX: &str = "warning: `";
pub(crate) const CARGO_WARNING_SUMMARY_TOKEN_GENERATED: &str = " generated ";
pub(crate) const CARGO_WARNING_SUMMARY_TOKEN_TO_APPLY: &str = "to apply ";

// cargo subcommands
pub(crate) const CARGO_SUBCOMMAND_CHECK: &str = "check";
pub(crate) const CARGO_SUBCOMMAND_FIX: &str = "fix";
pub(crate) const CARGO_SUBCOMMAND_MEND: &str = "mend";

// diagnostic severity prefixes
pub(crate) const DIAGNOSTIC_SEVERITY_ERROR_PREFIX: &str = "error:";
pub(crate) const DIAGNOSTIC_SEVERITY_WARNING_PREFIX: &str = "warning:";

// driver-ipc environment variables
pub(crate) const CARGO_PRIMARY_PACKAGE_ENV: &str = "CARGO_PRIMARY_PACKAGE";
pub(crate) const CONFIG_FINGERPRINT_ENV: &str = "MEND_CONFIG_FINGERPRINT";
pub(crate) const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
pub(crate) const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
pub(crate) const DRIVER_ENV: &str = "MEND_DRIVER";
pub(crate) const DRIVER_ENV_ENABLED: &str = "1";
pub(crate) const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
pub(crate) const PASSTHROUGH_RUSTC_WRAPPER_ENV: &str = "MEND_PASSTHROUGH_RUSTC_WRAPPER";
pub(crate) const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
pub(crate) const RUSTC_WRAPPER_ENV: &str = "RUSTC_WRAPPER";
pub(crate) const RUSTC_WORKSPACE_WRAPPER_ENV: &str = "RUSTC_WORKSPACE_WRAPPER";
pub(crate) const SCOPE_FINGERPRINT_ENV: &str = "MEND_SCOPE_FINGERPRINT";

// file extensions
pub(crate) const JSON_FILE_EXTENSION: &str = "json";

// findings
pub(crate) const FINDINGS_DIR_NAME: &str = "mend-findings";
pub(crate) const FINDINGS_SCHEMA_VERSION: u32 = 16;

// progress indicator
pub(super) const PROGRESS_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];
pub(super) const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);

// source-tree directories
pub(crate) const SOURCE_DIR_BENCHES: &str = "benches";
pub(crate) const SOURCE_DIR_EXAMPLES: &str = "examples";
pub(crate) const SOURCE_DIR_SRC: &str = "src";
pub(crate) const SOURCE_DIR_TESTS: &str = "tests";
