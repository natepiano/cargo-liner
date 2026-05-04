use std::collections::HashSet;
use std::path::Path;

use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorktreeGroup;
use crate::tui::app::App;

impl App {
    pub fn is_deleted(&self, path: &Path) -> bool {
        use crate::project::Visibility;
        self.projects()
            .at_path(path)
            .is_some_and(|project| project.visibility == Visibility::Deleted)
    }

    pub fn selected_project_is_deleted(&self) -> bool {
        self.selected_project_path()
            .is_some_and(|path| self.is_deleted(path))
    }

    pub fn is_rust_at_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| {
            if item
                .submodules()
                .iter()
                .any(|submodule| submodule.path.as_path() == path)
            {
                return false;
            }
            (item.path() == path || item.at_path(path).is_some()) && item.is_rust()
        })
    }

    pub fn is_vendored_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => {
                ws.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Rust(RustProject::Package(pkg)) => {
                pkg.vendored().iter().any(|v| v.path() == path)
            },
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|ws| ws.vendored().iter().any(|v| v.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Packages {
                primary, linked, ..
            }) => std::iter::once(primary)
                .chain(linked.iter())
                .any(|pkg| pkg.vendored().iter().any(|v| v.path() == path)),
            RootItem::NonRust(_) => false,
        })
    }

    pub fn is_workspace_member_path(&self, path: &Path) -> bool {
        self.projects().iter().any(|item| match &item.item {
            RootItem::Rust(RustProject::Workspace(ws)) => ws
                .groups()
                .iter()
                .any(|g| g.members().iter().any(|m| m.path() == path)),
            RootItem::Worktrees(WorktreeGroup::Workspaces {
                primary, linked, ..
            }) => std::iter::once(primary).chain(linked.iter()).any(|ws| {
                ws.groups()
                    .iter()
                    .any(|g| g.members().iter().any(|m| m.path() == path))
            }),
            _ => false,
        })
    }

    pub fn prune_inactive_project_state(&mut self) {
        let mut all_paths: HashSet<AbsolutePath> = HashSet::new();
        self.projects().for_each_leaf_path(|path, _| {
            all_paths.insert(AbsolutePath::from(path));
        });
        self.scan
            .pending_git_first_commit_mut()
            .retain(|path, _| all_paths.contains(path));
        self.ci
            .fetch_tracker_mut()
            .retain(|path| all_paths.contains(path));
    }
}
