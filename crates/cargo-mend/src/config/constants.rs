// application paths
pub(crate) const APP_NAME: &str = "cargo-mend";
pub(crate) const CONFIG_FILE_NAME: &str = "mend.toml";
pub(crate) const GLOBAL_CONFIG_FILE: &str = "config.toml";

// global config
/// Stamped into every global config so one-time migrations run once. Bump when a
/// shipped default changes and the value already on disk must be rewritten.
pub(crate) const CONFIG_VERSION: i64 = 1;
pub(crate) const CONFIG_VERSION_KEY: &str = "config_version";
pub(crate) const DIAGNOSTICS_TABLE_KEY: &str = "diagnostics";
pub(crate) const LEGACY_OVERBROAD_PUB_CRATE_KEY: &str = "forbidden_pub_crate";
pub(crate) const PRELUDE_COMMENT: &str =
    "# default-on; set false to review crate-root prelude modules too\n";
pub(crate) const PRELUDE_KEY: &str = "allow_prelude_pub_mod";
pub(crate) const PUB_IN_PATH_COMMENT: &str =
    "# required (default) reviews pub; permitted also accepts it; forbidden rejects pub(in ...)\n";
pub(crate) const PUB_IN_PATH_KEY: &str = "pub_in_path";
pub(crate) const VISIBILITY_TABLE_KEY: &str = "visibility";
