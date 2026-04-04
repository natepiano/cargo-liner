use std::collections::HashSet;

use super::types::App;
use super::types::LintRollupKey;
use super::types::VisibleRow;
use crate::lint::LintStatus;
use crate::project::RustProject;
use crate::scan::ProjectNode;

impl App {
    pub(super) fn rebuild_lint_rollups(&mut self) {
        self.lint_rollup_status.clear();
        self.lint_rollup_paths.clear();
        self.lint_rollup_keys_by_path.clear();

        let mut registrations: Vec<(LintRollupKey, Vec<String>)> = Vec::new();
        for (node_index, node) in self.nodes.iter().enumerate() {
            registrations.push((
                LintRollupKey::Root { node_index },
                Self::lint_root_paths_for_node(node),
            ));
            for (worktree_index, worktree) in node.worktrees.iter().enumerate() {
                registrations.push((
                    LintRollupKey::Worktree {
                        node_index,
                        worktree_index,
                    },
                    Self::lint_root_paths_for_worktree(worktree),
                ));
            }
        }

        for (key, paths) in registrations {
            self.register_lint_rollup(key, paths);
        }

        let keys: Vec<LintRollupKey> = self.lint_rollup_paths.keys().copied().collect();
        for key in keys {
            self.recompute_lint_rollup(key);
        }
    }

    fn register_lint_rollup(&mut self, key: LintRollupKey, mut paths: Vec<String>) {
        let mut seen = HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        for path in &paths {
            self.lint_rollup_keys_by_path
                .entry(path.clone())
                .or_default()
                .push(key);
        }
        self.lint_rollup_paths.insert(key, paths);
    }

    pub(super) fn update_lint_rollups_for_path(&mut self, path: &str) {
        let Some(keys) = self.lint_rollup_keys_by_path.get(path).cloned() else {
            return;
        };
        for key in keys {
            self.recompute_lint_rollup(key);
        }
    }

    fn recompute_lint_rollup(&mut self, key: LintRollupKey) {
        let Some(paths) = self.lint_rollup_paths.get(&key) else {
            self.lint_rollup_status.remove(&key);
            return;
        };
        let statuses: Vec<LintStatus> = paths
            .iter()
            .filter_map(|path| self.lint_status.get(path).cloned())
            .collect();
        let status = Self::aggregate_lint_rollup_statuses(&statuses);
        if matches!(status, LintStatus::NoLog) {
            self.lint_rollup_status.remove(&key);
        } else {
            self.lint_rollup_status.insert(key, status);
        }
    }

    fn aggregate_lint_rollup_statuses(statuses: &[LintStatus]) -> LintStatus {
        let running_statuses: Vec<LintStatus> = statuses
            .iter()
            .filter(|status| matches!(status, LintStatus::Running(_)))
            .cloned()
            .collect();
        if !running_statuses.is_empty() {
            return LintStatus::aggregate(running_statuses);
        }
        LintStatus::aggregate(statuses.iter().cloned())
    }

    fn lint_root_paths_for_node(node: &ProjectNode) -> Vec<String> {
        std::iter::once(node.project.path.clone())
            .chain(
                node.worktrees
                    .iter()
                    .map(|worktree| worktree.project.path.clone()),
            )
            .collect()
    }

    fn lint_root_paths_for_worktree(node: &ProjectNode) -> Vec<String> {
        vec![node.project.path.clone()]
    }

    pub(super) fn selected_lint_rollup_key(&self) -> Option<LintRollupKey> {
        match self.selected_row()? {
            VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. } => {
                Some(LintRollupKey::Root { node_index })
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            }
            | VisibleRow::WorktreeGroupHeader {
                node_index,
                worktree_index,
                ..
            } => Some(LintRollupKey::Worktree {
                node_index,
                worktree_index,
            }),
            VisibleRow::Member { .. }
            | VisibleRow::Vendored { .. }
            | VisibleRow::WorktreeMember { .. }
            | VisibleRow::WorktreeVendored { .. } => None,
        }
    }

    pub(super) fn lint_status_for_rollup_key(&self, key: LintRollupKey) -> Option<&LintStatus> {
        self.lint_rollup_status.get(&key)
    }

    /// Lint icon frame for the current animation state, or a blank space if lint is
    /// disabled or no log exists.
    pub fn lint_icon(&self, project: &RustProject) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status.get(&project.path) else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn lint_icon_for_root(&self, node_index: usize) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status_for_rollup_key(LintRollupKey::Root { node_index })
        else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn lint_icon_for_worktree(&self, node_index: usize, worktree_index: usize) -> &'static str {
        use crate::constants::LINT_NO_LOG;

        if !self.lint_enabled() {
            return LINT_NO_LOG;
        }
        let Some(status) = self.lint_status_for_rollup_key(LintRollupKey::Worktree {
            node_index,
            worktree_index,
        }) else {
            return LINT_NO_LOG;
        };
        status.icon().frame_at(self.animation_elapsed())
    }

    pub fn selected_lint_icon(&self, project: &RustProject) -> Option<&'static str> {
        if !self.lint_enabled() {
            return None;
        }
        match self.selected_row() {
            Some(VisibleRow::Root { node_index } | VisibleRow::GroupHeader { node_index, .. }) => {
                self.lint_status_for_rollup_key(LintRollupKey::Root { node_index })
                    .map(|status| status.icon().frame_at(self.animation_elapsed()))
            },
            Some(
                VisibleRow::WorktreeEntry {
                    node_index,
                    worktree_index,
                }
                | VisibleRow::WorktreeGroupHeader {
                    node_index,
                    worktree_index,
                    ..
                },
            ) => self
                .lint_status_for_rollup_key(LintRollupKey::Worktree {
                    node_index,
                    worktree_index,
                })
                .map(|status| status.icon().frame_at(self.animation_elapsed())),
            Some(
                VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => self
                .lint_status
                .get(&project.path)
                .map(|status| status.icon().frame_at(self.animation_elapsed())),
        }
    }
}
