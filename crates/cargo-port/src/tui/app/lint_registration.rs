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
        let mut count = 0;
        for entry in &self.project_list {
            match &entry.item {
                RootItem::Rust(RustProject::Workspace(ws)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: ws.display_path().into_string(),
                        abs_path:      ws.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Rust(RustProject::Package(pkg)) => {
                    runtime.register_project(RegisterProjectRequest {
                        project_label: pkg.display_path().into_string(),
                        abs_path:      pkg.path().clone(),
                        is_rust:       true,
                    });
                    count += 1;
                },
                RootItem::Worktrees(group) => {
                    for entry in group.iter_entries() {
                        runtime.register_project(RegisterProjectRequest {
                            project_label: entry.display_path().into_string(),
                            abs_path:      entry.path().clone(),
                            is_rust:       true,
                        });
                        count += 1;
                    }
                },
                RootItem::NonRust(_) => {},
            }
        }
        tracing::info!(count, "lint_register_root_items");
        count
    }
}
