use crate::config::Config;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConfigKey {
    InvertScroll,
    IncludeNonRust,
    CiRunCount,
    Editor,
    IncludeDirs,
    InlineDirs,
    StatusFlashSecs,
    CacheRoot,
    LintEnabled,
    LintInclude,
    LintExclude,
    LintCommands,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct ReloadActions {
    pub rebuild_tree: bool,
    pub rescan: bool,
    pub refresh_lint_runtime: bool,
}

#[derive(Clone, Copy)]
struct ConfigHandler {
    key: ConfigKey,
    mark: fn(&mut ReloadActions),
}

const CONFIG_HANDLERS: &[ConfigHandler] = &[
    ConfigHandler {
        key: ConfigKey::InlineDirs,
        mark: mark_rebuild_tree,
    },
    ConfigHandler {
        key: ConfigKey::IncludeNonRust,
        mark: mark_rescan,
    },
    ConfigHandler {
        key: ConfigKey::CiRunCount,
        mark: mark_rescan,
    },
    ConfigHandler {
        key: ConfigKey::IncludeDirs,
        mark: mark_rescan,
    },
    ConfigHandler {
        key: ConfigKey::CacheRoot,
        mark: mark_rescan,
    },
    ConfigHandler {
        key: ConfigKey::CacheRoot,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key: ConfigKey::LintEnabled,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key: ConfigKey::LintInclude,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key: ConfigKey::LintExclude,
        mark: mark_refresh_lint_runtime,
    },
    ConfigHandler {
        key: ConfigKey::LintCommands,
        mark: mark_refresh_lint_runtime,
    },
];

const fn mark_rebuild_tree(actions: &mut ReloadActions) {
    actions.rebuild_tree = true;
}

const fn mark_rescan(actions: &mut ReloadActions) {
    actions.rescan = true;
}

const fn mark_refresh_lint_runtime(actions: &mut ReloadActions) {
    actions.refresh_lint_runtime = true;
}

pub(super) fn changed_keys(old: &Config, new: &Config) -> Vec<ConfigKey> {
    let mut keys = Vec::new();

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
    if old.tui.include_dirs != new.tui.include_dirs {
        keys.push(ConfigKey::IncludeDirs);
    }
    if old.tui.inline_dirs != new.tui.inline_dirs {
        keys.push(ConfigKey::InlineDirs);
    }
    if old.tui.status_flash_secs.to_bits() != new.tui.status_flash_secs.to_bits() {
        keys.push(ConfigKey::StatusFlashSecs);
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

    keys
}

pub(super) fn collect_reload_actions(old: &Config, new: &Config) -> ReloadActions {
    let mut actions = ReloadActions::default();

    for key in changed_keys(old, new) {
        for handler in CONFIG_HANDLERS {
            if handler.key == key {
                (handler.mark)(&mut actions);
            }
        }
    }

    actions
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn changed_keys_include_value_only_settings_without_actions() {
        let mut new = Config::default();
        new.mouse.invert_scroll.toggle();
        new.tui.editor = "helix".to_string();
        new.tui.status_flash_secs = 5.0;

        let keys = changed_keys(&Config::default(), &new);

        assert!(keys.contains(&ConfigKey::InvertScroll));
        assert!(keys.contains(&ConfigKey::Editor));
        assert!(keys.contains(&ConfigKey::StatusFlashSecs));
        assert_eq!(
            collect_reload_actions(&Config::default(), &new),
            ReloadActions::default()
        );
    }

    #[test]
    fn reload_actions_coalesce_rescan_triggers() {
        let mut new = Config::default();
        new.tui.ci_run_count = 9;
        new.tui.include_dirs = vec!["rust".to_string()];
        new.tui.include_non_rust.toggle();

        assert_eq!(
            collect_reload_actions(&Config::default(), &new),
            ReloadActions {
                rebuild_tree: false,
                rescan: true,
                refresh_lint_runtime: false,
            }
        );
    }

    #[test]
    fn reload_actions_coalesce_lint_triggers() {
        let mut new = Config::default();
        new.lint.enabled = true;
        new.lint.include = vec!["hana".to_string()];
        new.lint.commands = vec![crate::config::default_clippy_lint_command()];

        assert_eq!(
            collect_reload_actions(&Config::default(), &new),
            ReloadActions {
                rebuild_tree: false,
                rescan: false,
                refresh_lint_runtime: true,
            }
        );
    }

    #[test]
    fn cache_root_marks_rescan_and_lint_runtime_refresh() {
        let mut new = Config::default();
        new.cache.root = "tmp-cache".to_string();

        assert_eq!(
            collect_reload_actions(&Config::default(), &new),
            ReloadActions {
                rebuild_tree: false,
                rescan: true,
                refresh_lint_runtime: true,
            }
        );
    }
}
