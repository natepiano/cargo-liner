use super::config_reload;
use super::config_reload::ConfigHandler;
use super::config_reload::ConfigKey;
use super::framework_keymap::AppPaneId;

// src tui integration config_reload
pub(super) const CONFIG_HANDLERS: &[ConfigHandler] = &[
    ConfigHandler {
        key:  ConfigKey::CpuPollMs,
        mark: config_reload::mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::CpuLowUtilizationMax,
        mark: config_reload::mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::CpuMediumUtilizationMax,
        mark: config_reload::mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::InlineDirs,
        mark: config_reload::mark_regroup_members,
    },
    ConfigHandler {
        key:  ConfigKey::IncludeNonRust,
        mark: config_reload::mark_include_non_rust,
    },
    ConfigHandler {
        key:  ConfigKey::CiRunCount,
        mark: config_reload::mark_full_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::MainBranch,
        mark: config_reload::mark_full_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::OtherPrimaryBranches,
        mark: config_reload::mark_full_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::IncludeDirs,
        mark: config_reload::mark_full_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::CacheRoot,
        mark: config_reload::mark_full_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::CacheRoot,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintEnabled,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintInclude,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintExclude,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintCommands,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintCacheSize,
        mark: config_reload::mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintOnDiscovery,
        mark: config_reload::mark_refresh_lint_runtime,
    },
];

// src tui integration framework_keymap
pub(super) const CI_RUNS_TAB_ORDER: i16 = 7;
pub(super) const CPU_TAB_ORDER: i16 = 4;
pub(super) const GIT_TAB_ORDER: i16 = 2;
/// Display ordering for the keymap-help overlay's per-pane sections.
/// Mirrors the prior `push_app_pane_rows` hardcoded order so the
/// overlay still surfaces sections in the cargo-port-preferred
/// sequence.
pub(super) const KEYMAP_OVERLAY_PANE_ORDER: &[AppPaneId] = &[
    AppPaneId::ProjectList,
    AppPaneId::Package,
    AppPaneId::Git,
    AppPaneId::Targets,
    AppPaneId::Lints,
    AppPaneId::CiRuns,
    AppPaneId::Output,
    AppPaneId::Finder,
];
pub(super) const LANG_TAB_ORDER: i16 = 3;
pub(super) const LINTS_TAB_ORDER: i16 = 6;
pub(super) const OUTPUT_TAB_ORDER: i16 = 8;
pub(super) const PACKAGE_TAB_ORDER: i16 = 1;
pub(super) const PROJECT_LIST_TAB_ORDER: i16 = 0;
pub(super) const TARGETS_TAB_ORDER: i16 = 5;
