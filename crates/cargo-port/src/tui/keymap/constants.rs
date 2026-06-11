// keymap migration keys

pub(super) const CLEAN_ACTION_KEY: &str = "clean";
pub(super) const CPU_SCOPE_KEY: &str = "cpu";
pub(super) const GIT_SCOPE_KEY: &str = "git";
pub(super) const GLOBAL_SCOPE_KEY: &str = "global";
pub(super) const LANG_SCOPE_KEY: &str = "lang";
pub(super) const PACKAGE_SCOPE_KEY: &str = "package";
pub(super) const PROJECT_LIST_SCOPE_KEY: &str = "project_list";
pub(super) const TARGETS_SCOPE_KEY: &str = "targets";

// src tui keymap load
/// Per-pane scopes that used to hold their own `clean` binding. The
/// action moved to `[global].clean`; on migration the first scope in
/// this list to define `clean` wins, matching the historical
/// registration order.
pub(super) const LEGACY_CLEAN_SCOPES: [&str; 6] = [
    PROJECT_LIST_SCOPE_KEY,
    PACKAGE_SCOPE_KEY,
    GIT_SCOPE_KEY,
    TARGETS_SCOPE_KEY,
    LANG_SCOPE_KEY,
    CPU_SCOPE_KEY,
];
pub(super) const REMOVED_PROJECT_LIST_GLOBAL_ACTIONS: [(&str, &str); 2] =
    [("open_editor", "open_editor"), ("rescan", "rescan")];
