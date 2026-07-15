// application paths
pub(crate) const APP_NAME: &str = "cargo-mend";
pub(crate) const CONFIG_FILE_NAME: &str = "mend.toml";
pub(crate) const GLOBAL_CONFIG_FILE: &str = "config.toml";

// global config
pub(crate) const DIAGNOSTICS_TABLE_KEY: &str = "diagnostics";
pub(crate) const PRELUDE_COMMENT: &str =
    "# default-on; set false to review crate-root prelude modules too\n";
pub(crate) const PRELUDE_KEY: &str = "allow_prelude_pub_mod";
pub(crate) const VISIBILITY_TABLE_KEY: &str = "visibility";
