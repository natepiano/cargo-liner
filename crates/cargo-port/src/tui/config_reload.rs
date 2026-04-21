use crate::config::CargoPortConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConfigKey {
    CpuPollMs,
    CpuGreenMax,
    CpuYellowMax,
    InvertScroll,
    IncludeNonRust,
    CiRunCount,
    Editor,
    MainBranch,
    OtherPrimaryBranches,
    IncludeDirs,
    InlineDirs,
    StatusFlashSecs,
    TaskLingerSecs,
    DiscoveryShimmerSecs,
    CacheRoot,
    LintEnabled,
    LintInclude,
    LintExclude,
    LintCommands,
    LintCacheSize,
    LintOnDiscovery,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ReloadDecision {
    #[default]
    Skip,
    Apply,
}

impl ReloadDecision {
    pub(super) const fn should_apply(self) -> bool { matches!(self, Self::Apply) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ReloadActions {
    pub rebuild_tree:         ReloadDecision,
    pub refresh_cpu:          ReloadDecision,
    pub rescan:               ReloadDecision,
    pub refresh_lint_runtime: ReloadDecision,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ReloadContext {
    pub scan_complete:       bool,
    pub has_cached_non_rust: bool,
}

#[derive(Clone, Copy)]
struct ConfigHandler {
    key:  ConfigKey,
    mark: fn(&mut ReloadActions, &CargoPortConfig, &CargoPortConfig, ReloadContext),
}

const CONFIG_HANDLERS: &[ConfigHandler] = &[
    ConfigHandler {
        key:  ConfigKey::CpuPollMs,
        mark: mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::CpuGreenMax,
        mark: mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::CpuYellowMax,
        mark: mark_refresh_cpu,
    },
    ConfigHandler {
        key:  ConfigKey::InlineDirs,
        mark: mark_rebuild_tree,
    },
    ConfigHandler {
        key:  ConfigKey::IncludeNonRust,
        mark: mark_include_non_rust,
    },
    ConfigHandler {
        key:  ConfigKey::CiRunCount,
        mark: mark_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::MainBranch,
        mark: mark_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::OtherPrimaryBranches,
        mark: mark_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::IncludeDirs,
        mark: mark_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::CacheRoot,
        mark: mark_rescan,
    },
    ConfigHandler {
        key:  ConfigKey::CacheRoot,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintEnabled,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintInclude,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintExclude,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintCommands,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintCacheSize,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key:  ConfigKey::LintOnDiscovery,
        mark: mark_refresh_lint_runtime,
    },
];

const fn mark_rebuild_tree(
    actions: &mut ReloadActions,
    _old: &CargoPortConfig,
    _new: &CargoPortConfig,
    _context: ReloadContext,
) {
    actions.rebuild_tree = ReloadDecision::Apply;
}

const fn mark_rescan(
    actions: &mut ReloadActions,
    _old: &CargoPortConfig,
    _new: &CargoPortConfig,
    _context: ReloadContext,
) {
    actions.rescan = ReloadDecision::Apply;
}

const fn mark_refresh_lint_runtime(
    actions: &mut ReloadActions,
    _old: &CargoPortConfig,
    _new: &CargoPortConfig,
    _context: ReloadContext,
) {
    actions.refresh_lint_runtime = ReloadDecision::Apply;
}

const fn mark_refresh_cpu(
    actions: &mut ReloadActions,
    _old: &CargoPortConfig,
    _new: &CargoPortConfig,
    _context: ReloadContext,
) {
    actions.refresh_cpu = ReloadDecision::Apply;
}

const fn mark_include_non_rust(
    actions: &mut ReloadActions,
    old: &CargoPortConfig,
    new: &CargoPortConfig,
    context: ReloadContext,
) {
    if !context.scan_complete {
        actions.rescan = ReloadDecision::Apply;
        return;
    }

    let enabling_non_rust = !old.tui.include_non_rust.includes_non_rust()
        && new.tui.include_non_rust.includes_non_rust();
    if enabling_non_rust && !context.has_cached_non_rust {
        actions.rescan = ReloadDecision::Apply;
    } else {
        actions.rebuild_tree = ReloadDecision::Apply;
    }
}

pub(super) fn changed_keys(old: &CargoPortConfig, new: &CargoPortConfig) -> Vec<ConfigKey> {
    let mut keys = Vec::new();

    if old.cpu.poll_ms != new.cpu.poll_ms {
        keys.push(ConfigKey::CpuPollMs);
    }
    if old.cpu.green_max_percent != new.cpu.green_max_percent {
        keys.push(ConfigKey::CpuGreenMax);
    }
    if old.cpu.yellow_max_percent != new.cpu.yellow_max_percent {
        keys.push(ConfigKey::CpuYellowMax);
    }
    if old.mouse.invert_scroll != new.mouse.invert_scroll {
        keys.push(ConfigKey::InvertScroll);
    }
    if old.tui.include_non_rust != new.tui.include_non_rust {
        keys.push(ConfigKey::IncludeNonRust);
    }
    if old.tui.ci_run_count != new.tui.ci_run_count {
        keys.push(ConfigKey::CiRunCount);
    }
    if old.tui.editor != new.tui.editor {
        keys.push(ConfigKey::Editor);
    }
    if old.tui.main_branch != new.tui.main_branch {
        keys.push(ConfigKey::MainBranch);
    }
    if old.tui.other_primary_branches != new.tui.other_primary_branches {
        keys.push(ConfigKey::OtherPrimaryBranches);
    }
    if old.tui.include_dirs != new.tui.include_dirs {
        keys.push(ConfigKey::IncludeDirs);
    }
    if old.tui.inline_dirs != new.tui.inline_dirs {
        keys.push(ConfigKey::InlineDirs);
    }
    if old.tui.status_flash_secs.to_bits() != new.tui.status_flash_secs.to_bits() {
        keys.push(ConfigKey::StatusFlashSecs);
    }
    if old.tui.task_linger_secs.to_bits() != new.tui.task_linger_secs.to_bits() {
        keys.push(ConfigKey::TaskLingerSecs);
    }
    if old.tui.discovery_shimmer_secs.to_bits() != new.tui.discovery_shimmer_secs.to_bits() {
        keys.push(ConfigKey::DiscoveryShimmerSecs);
    }
    if old.cache.root != new.cache.root {
        keys.push(ConfigKey::CacheRoot);
    }
    if old.lint.enabled != new.lint.enabled {
        keys.push(ConfigKey::LintEnabled);
    }
    if old.lint.include != new.lint.include {
        keys.push(ConfigKey::LintInclude);
    }
    if old.lint.exclude != new.lint.exclude {
        keys.push(ConfigKey::LintExclude);
    }
    if old.lint.commands != new.lint.commands {
        keys.push(ConfigKey::LintCommands);
    }
    if old.lint.cache_size != new.lint.cache_size {
        keys.push(ConfigKey::LintCacheSize);
    }
    if old.lint.on_discovery != new.lint.on_discovery {
        keys.push(ConfigKey::LintOnDiscovery);
    }

    keys
}

pub(super) fn collect_reload_actions(
    old: &CargoPortConfig,
    new: &CargoPortConfig,
    context: ReloadContext,
) -> ReloadActions {
    let mut actions = ReloadActions::default();

    for key in changed_keys(old, new) {
        for handler in CONFIG_HANDLERS {
            if handler.key == key {
                (handler.mark)(&mut actions, old, new, context);
            }
        }
    }

    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_keys_include_value_only_settings_without_actions() {
        let mut new = CargoPortConfig::default();
        new.mouse.invert_scroll.toggle();
        new.tui.editor = "helix".to_string();
        new.tui.status_flash_secs = 10.0;
        new.tui.discovery_shimmer_secs = 4.0;

        let keys = changed_keys(&CargoPortConfig::default(), &new);

        assert!(keys.contains(&ConfigKey::InvertScroll));
        assert!(keys.contains(&ConfigKey::Editor));
        assert!(keys.contains(&ConfigKey::StatusFlashSecs));
        assert!(keys.contains(&ConfigKey::DiscoveryShimmerSecs));
        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions::default()
        );
    }

    #[test]
    fn reload_actions_coalesce_rescan_triggers() {
        let mut new = CargoPortConfig::default();
        new.tui.ci_run_count = 9;
        new.tui.include_dirs = vec!["rust".to_string()];
        new.tui.include_non_rust.toggle();

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Skip,
                refresh_cpu:          ReloadDecision::Skip,
                rescan:               ReloadDecision::Apply,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn completed_scan_rebuilds_tree_when_hiding_cached_non_rust_projects() {
        let mut old = CargoPortConfig::default();
        old.tui.include_non_rust.toggle();
        let mut new = old.clone();
        new.tui.include_non_rust.toggle();

        assert_eq!(
            collect_reload_actions(
                &old,
                &new,
                ReloadContext {
                    scan_complete:       true,
                    has_cached_non_rust: true,
                },
            ),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Apply,
                refresh_cpu:          ReloadDecision::Skip,
                rescan:               ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
        let mut new = CargoPortConfig::default();
        new.tui.include_non_rust.toggle();

        assert_eq!(
            collect_reload_actions(
                &CargoPortConfig::default(),
                &new,
                ReloadContext {
                    scan_complete:       true,
                    has_cached_non_rust: false,
                },
            ),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Skip,
                refresh_cpu:          ReloadDecision::Skip,
                rescan:               ReloadDecision::Apply,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn reload_actions_coalesce_lint_triggers() {
        let mut new = CargoPortConfig::default();
        new.lint.enabled = true;
        new.lint.include = vec!["hana".to_string()];
        new.lint.commands = vec![crate::config::default_clippy_lint_command()];

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Skip,
                refresh_cpu:          ReloadDecision::Skip,
                rescan:               ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Apply,
            }
        );
    }

    #[test]
    fn cache_root_marks_rescan_and_lint_runtime_refresh() {
        let mut new = CargoPortConfig::default();
        new.cache.root = "tmp-cache".to_string();

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Skip,
                refresh_cpu:          ReloadDecision::Skip,
                rescan:               ReloadDecision::Apply,
                refresh_lint_runtime: ReloadDecision::Apply,
            }
        );
    }

    #[test]
    fn cpu_settings_mark_refresh_cpu_only() {
        let mut new = CargoPortConfig::default();
        new.cpu.poll_ms = 1500;
        new.cpu.green_max_percent = 55;

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                rebuild_tree:         ReloadDecision::Skip,
                refresh_cpu:          ReloadDecision::Apply,
                rescan:               ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }
}
