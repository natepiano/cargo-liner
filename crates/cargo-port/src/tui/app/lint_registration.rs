use super::App;
use crate::lint::RegisterProjectRequest;
use crate::project;

impl App {
    pub(super) fn register_lint_for_root_items(&self) -> usize {
        let Some(runtime) = self.lint.runtime() else {
            return 0;
        };
        let mut projects = Vec::new();
        for entry in self.project_list.lint_runtime_root_entries() {
            if !entry.is_rust {
                continue;
            }
            projects.push(
                RegisterProjectRequest::new(
                    project::home_relative_path(&entry.path),
                    entry.path,
                    entry.is_rust,
                )
                .with_linked_primary_root(entry.linked_primary_root),
            );
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
