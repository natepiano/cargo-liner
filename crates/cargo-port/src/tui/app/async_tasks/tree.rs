use std::time::Instant;

use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::scan;
use crate::tui::app::App;
use crate::tui::app::ExpandKey::Group;
use crate::tui::app::ExpandKey::Node;
use crate::tui::app::ExpandKey::Worktree;
use crate::tui::app::ExpandKey::WorktreeGroup;
use crate::tui::app::types::ScanPhase;
use crate::tui::panes::PaneId;
#[cfg(test)]
use crate::tui::project_list::ProjectList;

impl App {
    #[cfg(test)]
    pub fn apply_tree_build(&mut self, projects: ProjectList) {
        let selected_path = self
            .selected_project_path()
            .map(AbsolutePath::from)
            .or_else(|| self.project_list.paths_mut().last_selected.clone());
        let should_focus_project_list = false;
        self.mutate_tree().replace_all(projects);
        self.prune_inactive_project_state();
        self.register_lint_for_root_items();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Try to restore selection
        if let Some(path) = selected_path {
            self.select_project_in_tree(path.as_path());
        } else if !self.projects().is_empty() {
            self.project_list.set_cursor(0);
        }
        if should_focus_project_list {
            self.focus.set(PaneId::ProjectList);
        }
        self.sync_selected_project();
    }
    pub(super) fn capture_legacy_root_expansions(&self) -> Vec<LegacyRootExpansion> {
        self.projects()
            .iter()
            .enumerate()
            .filter_map(|(ni, entry)| {
                if !self.project_list.expanded().contains(&Node(ni)) {
                    return None;
                }

                match &entry.item {
                    RootItem::Rust(RustProject::Workspace(ws)) => Some(LegacyRootExpansion {
                        root_path:      ws.path().clone(),
                        old_node_index: ni,
                        had_children:   ws.has_members() || !ws.vendored().is_empty(),
                        named_groups:   ws
                            .groups()
                            .iter()
                            .enumerate()
                            .filter_map(|(gi, group)| {
                                group
                                    .is_named()
                                    .then(|| self.project_list.expanded().contains(&Group(ni, gi)))
                                    .filter(|expanded| *expanded)
                                    .map(|_| gi)
                            })
                            .collect(),
                    }),
                    RootItem::Rust(RustProject::Package(pkg)) => Some(LegacyRootExpansion {
                        root_path:      pkg.path().clone(),
                        old_node_index: ni,
                        had_children:   !pkg.vendored().is_empty(),
                        named_groups:   Vec::new(),
                    }),
                    _ => None,
                }
            })
            .collect()
    }
    pub(super) fn migrate_legacy_root_expansions(&mut self, legacy: &[LegacyRootExpansion]) {
        let (roots, expanded) = self.project_list.iter_with_expanded_mut();
        // Snapshot path-and-item pairs so the iterator borrow ends before we
        // mutate `expanded` below.
        let entries: Vec<(usize, &RootItem)> = roots
            .enumerate()
            .map(|(idx, entry)| (idx, &entry.item))
            .collect();
        for legacy_root in legacy {
            let Some((current_index, item)) = entries
                .iter()
                .find(|(_, item)| item.path() == legacy_root.root_path.as_path())
                .map(|(idx, item)| (*idx, *item))
            else {
                continue;
            };

            match item {
                RootItem::Worktrees(
                    group @ crate::project::WorktreeGroup::Workspaces { primary, .. },
                ) if group.renders_as_group() => {
                    expanded.insert(Node(current_index));
                    if legacy_root.had_children {
                        expanded.insert(Worktree(current_index, 0));
                    }
                    for &group_index in &legacy_root.named_groups {
                        if primary.groups().get(group_index).is_some() {
                            expanded.insert(WorktreeGroup(current_index, 0, group_index));
                        }
                        expanded.remove(&Group(legacy_root.old_node_index, group_index));
                    }
                },
                RootItem::Worktrees(group @ crate::project::WorktreeGroup::Packages { .. })
                    if group.renders_as_group() =>
                {
                    expanded.insert(Node(current_index));
                    if legacy_root.had_children {
                        expanded.insert(Worktree(current_index, 0));
                    }
                },
                _ => {},
            }
        }
    }
    pub(super) fn rebuild_visible_rows_now(&mut self) {
        let include_non_rust = self.config.include_non_rust().includes_non_rust();
        self.project_list.recompute_visibility(include_non_rust);
    }
    pub fn rescan(&mut self) {
        self.project_list.clear();
        // disk_usage lives on project items — cleared with projects above
        self.ci.fetch_tracker_mut().clear();
        self.ci.clear_display_modes();
        self.clear_all_lint_state();
        self.lint
            .set_cache_usage(crate::lint::CacheUsage::default());
        self.net.clear_for_tree_change();
        self.scan.discovery_shimmers_mut().clear();
        self.scan.scan_state_mut().phase = ScanPhase::Running;
        self.scan.scan_state_mut().started_at = Instant::now();
        self.scan.scan_state_mut().run_count += 1;
        self.startup.reset();
        tracing::info!(
            kind = "rescan",
            run = self.scan.scan_state().run_count,
            "scan_start"
        );
        self.scan.set_priority_fetch_path(None);
        self.focus.set(PaneId::ProjectList);
        self.overlays.close_settings();
        self.overlays.close_finder();
        self.reset_project_panes();
        self.project_list.paths_mut().selected_project = None;
        self.inflight.clear_pending_ci_fetch();
        self.project_list.expanded_mut().clear();
        self.project_list.set_cursor(0);
        self.panes
            .project_list_mut()
            .viewport_mut()
            .set_scroll_offset(0);
        self.scan.bump_generation();
        let scan_dirs = scan::resolve_include_dirs(&self.config.current().tui.include_dirs);
        let (tx, rx) = scan::spawn_streaming_scan(
            scan_dirs,
            &self.config.current().tui.inline_dirs,
            self.config.include_non_rust(),
            self.net.http_client(),
            self.scan.metadata_store_handle(),
        );
        self.background.swap_bg_channel(tx, rx);
        self.respawn_watcher();
        let current_config = self.config.current().clone();
        self.refresh_lint_runtime_from_config(&current_config);
    }
}

#[derive(Clone)]
pub(super) struct LegacyRootExpansion {
    root_path:      AbsolutePath,
    old_node_index: usize,
    had_children:   bool,
    named_groups:   Vec<usize>,
}
