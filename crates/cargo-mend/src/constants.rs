// ansi codes
pub(crate) const ANSI_BOLD: &str = "1";
pub(crate) const ANSI_BOLD_BLUE: &str = "1;34";
pub(crate) const ANSI_BOLD_GREEN: &str = "1;32";
pub(crate) const ANSI_BOLD_RED: &str = "1;31";
pub(crate) const ANSI_BOLD_YELLOW: &str = "1;33";
pub(crate) const ANSI_DIM: &str = "2";

// cargo cli flags
pub(crate) const CARGO_FLAG_ALL_TARGETS: &str = "--all-targets";
pub(crate) const CARGO_FLAG_ALLOW_DIRTY: &str = "--allow-dirty";
pub(crate) const CARGO_FLAG_ALLOW_STAGED: &str = "--allow-staged";
pub(crate) const CARGO_FLAG_EXCLUDE: &str = "--exclude";
pub(crate) const CARGO_FLAG_MANIFEST_PATH: &str = "--manifest-path";
pub(crate) const CARGO_FLAG_PACKAGE: &str = "--package";
pub(crate) const CARGO_FLAG_TESTS: &str = "--tests";
pub(crate) const CARGO_FLAG_WORKSPACE: &str = "--workspace";

// cargo subcommands
pub(crate) const CARGO_BIN: &str = "cargo";
pub(crate) const CARGO_SUBCOMMAND_CHECK: &str = "check";
pub(crate) const CARGO_SUBCOMMAND_FIX: &str = "fix";

// cargo target kinds
pub(crate) const CARGO_TARGET_KIND_BENCH: &str = "bench";
pub(crate) const CARGO_TARGET_KIND_BIN: &str = "bin";
pub(crate) const CARGO_TARGET_KIND_EXAMPLE: &str = "example";
pub(crate) const CARGO_TARGET_KIND_LIB: &str = "lib";
pub(crate) const CARGO_TARGET_KIND_MAIN: &str = "main";
pub(crate) const CARGO_TARGET_KIND_TEST: &str = "test";

// color mode
pub(crate) const CARGO_TERM_COLOR_ALWAYS: &str = "always";
pub(crate) const CARGO_TERM_COLOR_NEVER: &str = "never";
pub(crate) const CLICOLOR_DISABLED_VALUE: &str = "0";
pub(crate) const TERM_DUMB_VALUE: &str = "dumb";

// config
pub(crate) const APP_NAME: &str = "cargo-mend";
pub(crate) const DEFAULT_GLOBAL_CONFIG_TOML: &str = r"# cargo-mend global configuration
# See https://github.com/natepiano/cargo-mend#diagnostics for details on each rule.
# Per-project overrides go in mend.toml at your project or workspace root.

[diagnostics]
forbidden_pub_crate = true
forbidden_pub_in_crate = true
review_pub_mod = true
suspicious_pub = true
prefer_module_import = true
inline_path_qualified_type = true
shorten_local_crate_import = true
replace_deep_super_import = true
wildcard_parent_pub_use = true
internal_parent_pub_use_facade = true
narrow_to_pub_crate = true
field_visibility_wider_than_type = true
";
pub(crate) const GLOBAL_CONFIG_FILE: &str = "config.toml";

// diagnostics help
pub(crate) const DIAGNOSTICS_HELP_NAME_COLUMN_WIDTH: usize = 40;

// environment variables
pub(crate) const CARGO_TERM_COLOR_ENV: &str = "CARGO_TERM_COLOR";
pub(crate) const CLICOLOR_ENV: &str = "CLICOLOR";
pub(crate) const CLICOLOR_FORCE_ENV: &str = "CLICOLOR_FORCE";
pub(crate) const CONFIG_FINGERPRINT_ENV: &str = "MEND_CONFIG_FINGERPRINT";
pub(crate) const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
pub(crate) const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
pub(crate) const DRIVER_ENV: &str = "MEND_DRIVER";
pub(crate) const DRIVER_ENV_ENABLED: &str = "1";
pub(crate) const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
pub(crate) const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
pub(crate) const RUSTC_WORKSPACE_WRAPPER_ENV: &str = "RUSTC_WORKSPACE_WRAPPER";
pub(crate) const SCOPE_FINGERPRINT_ENV: &str = "MEND_SCOPE_FINGERPRINT";
pub(crate) const TERM_ENV: &str = "TERM";

// exit codes
pub(crate) const EXIT_CODE_ERROR: u8 = 1;
pub(crate) const EXIT_CODE_WARNING: u8 = 2;

// file names
pub(crate) const CARGO_MANIFEST_FILE: &str = "Cargo.toml";
pub(crate) const RUST_LIB_FILE: &str = "lib.rs";
pub(crate) const RUST_MAIN_FILE: &str = "main.rs";
pub(crate) const RUST_MODULE_FILE: &str = "mod.rs";

// findings
pub(crate) const FINDINGS_SCHEMA_VERSION: u32 = 13;

// fix execution
/// Maximum number of mend passes during `--fix-all`. Prevents an infinite
/// loop if a fix oscillates; in practice convergence happens in 1–2 passes.
pub(crate) const FIX_ALL_MAX_PASSES: usize = 5;

// path keywords
pub(crate) const PATH_KEYWORD_CRATE: &str = "crate";
pub(crate) const PATH_KEYWORD_SELF: &str = "self";
pub(crate) const PATH_KEYWORD_SUPER: &str = "super";

// rust module paths
pub(crate) const MODULE_GLOB_SUFFIX: &str = "::*";
pub(crate) const MODULE_PATH_SEPARATOR: &str = "::";

// rust source files
pub(crate) const RUST_SOURCE_FILE_EXTENSION: &str = "rs";
pub(crate) const RUST_SOURCE_FILE_SUFFIX: &str = ".rs";

// source-tree directories
pub(crate) const SOURCE_DIR_BENCHES: &str = "benches";
pub(crate) const SOURCE_DIR_EXAMPLES: &str = "examples";
pub(crate) const SOURCE_DIR_SRC: &str = "src";
pub(crate) const SOURCE_DIR_TESTS: &str = "tests";

// visibility
pub(crate) const PUB_CRATE_VISIBILITY: &str = "pub(crate)";
pub(crate) const PUB_IN_CRATE_VISIBILITY_PREFIX: &str = "pub(in crate::";
pub(crate) const PUB_VISIBILITY_PREFIX: &str = "pub ";
pub(crate) const PUB_VISIBILITY_TOKEN: &str = "pub";
