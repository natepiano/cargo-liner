use super::App;
use crate::lint::RegisterProjectRequest;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;

impl App {
    pub(super) fn register_lint_for_root_items(&self) -> usize {
        let Some(runtime) = self.lint.runtime() else {
            return 0;
        };
        let mut projects = Vec::new();
        for entry in &self.project_list {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    projects.push(RegisterProjectRequest {
                        project_label: ws.display_path().into_string(),
                        abs_path:      ws.path().clone(),
                        is_rust:       true,
                    });
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    projects.push(RegisterProjectRequest {
                        project_label: pkg.display_path().into_string(),
                        abs_path:      pkg.path().clone(),
                        is_rust:       true,
                    });
                },
                RootItem::Worktrees(group) => {
                    for entry in group.iter_entries() {
                        projects.push(RegisterProjectRequest {
                            project_label: entry.display_path().into_string(),
                            abs_path:      entry.path().clone(),
                            is_rust:       true,
                        });
                    }
                },
                RootItem::NonRust(_) => {},
            }
        }
        let count = projects.len();
        runtime.sync_projects(projects);
        tracing::trace!(
            target: tui_pane::PERF_LOG_TARGET,
            count,
            "lint_register_root_items"
        );
        count
    }
}
