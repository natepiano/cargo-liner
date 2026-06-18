use super::constants::CONFIG_HANDLERS;
use crate::config::CargoPortConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConfigKey {
    CpuPollMs,
    CpuLowUtilizationMax,
    CpuMediumUtilizationMax,
    InvertScroll,
    IncludeNonRust,
    CiRunCount,
    Editor,
    MainBranch,
    OtherPrimaryBranches,
    IncludeDirs,
    InlineDirs,
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
pub enum ReloadDecision {
    #[default]
    Skip,
    Apply,
}

impl ReloadDecision {
    pub const fn should_apply(self) -> bool { matches!(self, Self::Apply) }
}

/// What the config reload should do to the project tree. The three
/// variants are ordered by escalation precedence: `FullRescan` wins
/// over `RegroupMembers`, which wins over `None`. The enum makes the
/// mutual exclusion exhaustive at the type level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TreeReaction {
    #[default]
    None,
    RegroupMembers,
    FullRescan,
}

impl TreeReaction {
    const fn escalate(&mut self, to: Self) {
        if (to as u8) > (*self as u8) {
            *self = to;
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReloadActions {
    pub tree:                 TreeReaction,
    pub refresh_cpu:          ReloadDecision,
    pub refresh_lint_runtime: ReloadDecision,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScanState {
    Complete,
    #[default]
    Pending,
}

impl ScanState {
    const fn is_complete(self) -> bool { matches!(self, Self::Complete) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NonRustCacheState {
    Present,
    #[default]
    Missing,
}

impl NonRustCacheState {
    const fn is_present(self) -> bool { matches!(self, Self::Present) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReloadContext {
    pub scan:           ScanState,
    pub non_rust_cache: NonRustCacheState,
}

#[derive(Clone, Copy)]
pub(super) struct ConfigHandler {
    pub(super) key:  ConfigKey,
    pub(super) mark: fn(&mut ReloadActions, &CargoPortConfig, &CargoPortConfig, ReloadContext),
}

pub(super) const fn mark_regroup_members(
    actions: &mut ReloadActions,
    _: &CargoPortConfig,
    _: &CargoPortConfig,
    _: ReloadContext,
) {
    actions.tree.escalate(TreeReaction::RegroupMembers);
}

pub(super) const fn mark_full_rescan(
    actions: &mut ReloadActions,
    _: &CargoPortConfig,
    _: &CargoPortConfig,
    _: ReloadContext,
) {
    actions.tree.escalate(TreeReaction::FullRescan);
}

pub(super) const fn mark_refresh_lint_runtime(
    actions: &mut ReloadActions,
    _: &CargoPortConfig,
    _: &CargoPortConfig,
    _: ReloadContext,
) {
    actions.refresh_lint_runtime = ReloadDecision::Apply;
}

pub(super) const fn mark_refresh_cpu(
    actions: &mut ReloadActions,
    _: &CargoPortConfig,
    _: &CargoPortConfig,
    _: ReloadContext,
) {
    actions.refresh_cpu = ReloadDecision::Apply;
}

pub(super) const fn mark_include_non_rust(
    actions: &mut ReloadActions,
    old: &CargoPortConfig,
    new: &CargoPortConfig,
    context: ReloadContext,
) {
    if !context.scan.is_complete() {
        actions.tree.escalate(TreeReaction::FullRescan);
        return;
    }

    let enabling_non_rust = !old.tui.include_non_rust.includes_non_rust()
        && new.tui.include_non_rust.includes_non_rust();
    if enabling_non_rust && !context.non_rust_cache.is_present() {
        actions.tree.escalate(TreeReaction::FullRescan);
    } else {
        actions.tree.escalate(TreeReaction::RegroupMembers);
    }
}

fn changed_keys(old: &CargoPortConfig, new: &CargoPortConfig) -> Vec<ConfigKey> {
    let mut keys = Vec::new();

    if old.cpu.poll_ms != new.cpu.poll_ms {
        keys.push(ConfigKey::CpuPollMs);
    }
    if old.cpu.low_utilization_max_percent != new.cpu.low_utilization_max_percent {
        keys.push(ConfigKey::CpuLowUtilizationMax);
    }
    if old.cpu.medium_utilization_max_percent != new.cpu.medium_utilization_max_percent {
        keys.push(ConfigKey::CpuMediumUtilizationMax);
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

pub fn collect_reload_actions(
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
    use crate::config::LintIndicator;
    use crate::config::NonRustInclusion;
    use crate::config::ScrollDirection;

    #[test]
    fn changed_keys_include_value_only_settings_without_actions() {
        let mut new = CargoPortConfig::default();
        new.mouse.invert_scroll = ScrollDirection::Normal;
        new.tui.editor = "helix".to_string();
        new.tui.discovery_shimmer_secs = 4.0;

        let keys = changed_keys(&CargoPortConfig::default(), &new);

        assert!(keys.contains(&ConfigKey::InvertScroll));
        assert!(keys.contains(&ConfigKey::Editor));
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
        new.tui.include_non_rust = NonRustInclusion::Include;

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                tree:                 TreeReaction::FullRescan,
                refresh_cpu:          ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn completed_scan_rebuilds_tree_when_hiding_cached_non_rust_projects() {
        let mut old = CargoPortConfig::default();
        old.tui.include_non_rust = NonRustInclusion::Include;
        let mut new = old.clone();
        new.tui.include_non_rust = NonRustInclusion::Exclude;

        assert_eq!(
            collect_reload_actions(
                &old,
                &new,
                ReloadContext {
                    scan:           ScanState::Complete,
                    non_rust_cache: NonRustCacheState::Present,
                },
            ),
            ReloadActions {
                tree:                 TreeReaction::RegroupMembers,
                refresh_cpu:          ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn completed_scan_rescans_when_enabling_non_rust_without_cached_projects() {
        let mut new = CargoPortConfig::default();
        new.tui.include_non_rust = NonRustInclusion::Include;

        assert_eq!(
            collect_reload_actions(
                &CargoPortConfig::default(),
                &new,
                ReloadContext {
                    scan:           ScanState::Complete,
                    non_rust_cache: NonRustCacheState::Missing,
                },
            ),
            ReloadActions {
                tree:                 TreeReaction::FullRescan,
                refresh_cpu:          ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }

    #[test]
    fn reload_actions_coalesce_lint_triggers() {
        let mut new = CargoPortConfig::default();
        new.lint.enabled = LintIndicator::Enabled;
        new.lint.include = vec!["hana".to_string()];
        new.lint.commands = vec![crate::config::default_clippy_lint_command()];

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                tree:                 TreeReaction::None,
                refresh_cpu:          ReloadDecision::Skip,
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
                tree:                 TreeReaction::FullRescan,
                refresh_cpu:          ReloadDecision::Skip,
                refresh_lint_runtime: ReloadDecision::Apply,
            }
        );
    }

    #[test]
    fn cpu_settings_mark_refresh_cpu_only() {
        let mut new = CargoPortConfig::default();
        new.cpu.poll_ms = 1500;
        new.cpu.low_utilization_max_percent = 55;

        assert_eq!(
            collect_reload_actions(&CargoPortConfig::default(), &new, ReloadContext::default()),
            ReloadActions {
                tree:                 TreeReaction::None,
                refresh_cpu:          ReloadDecision::Apply,
                refresh_lint_runtime: ReloadDecision::Skip,
            }
        );
    }
}
