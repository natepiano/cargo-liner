use std::path::Path;

use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::terminal;

impl App {
    pub fn detail_path_is_affected(&self, path: &Path) -> bool {
        let Some(selected_path) = self.selected_project_path() else {
            return false;
        };
        if selected_path == path {
            return true;
        }
        // Check if both paths resolve to the same lint-owning node (e.g.,
        // a worktree group where one entry's status change affects the
        // root rollup displayed in the detail pane).
        self.projects()
            .lint_at_path(selected_path)
            .zip(self.projects().lint_at_path(path))
            .is_some_and(|(a, b)| std::ptr::eq(a, b))
    }
    /// Spawn a priority fetch for the selected project if it hasn't been loaded yet.
    pub fn maybe_priority_fetch(&mut self) {
        let Some(abs_path) = self.selected_project_path().map(Path::to_path_buf) else {
            return;
        };
        let abs_key: AbsolutePath = abs_path.clone().into();
        let display_path = self
            .selected_display_path()
            .unwrap_or_else(|| abs_key.display_path());
        let name = self
            .panes()
            .package()
            .content()
            .map(|d| d.title_name.clone())
            .filter(|n| n != "-");
        if self
            .projects()
            .at_path(abs_key.as_path())
            .is_none_or(|p| p.disk_usage_bytes.is_none())
            && self.scan.priority_fetch_path() != Some(&abs_key)
        {
            self.scan.set_priority_fetch_path(Some(abs_key));
            let abs_str = abs_path.display().to_string();
            terminal::spawn_priority_fetch(self, display_path.as_str(), &abs_str, name.as_ref());
        }
    }
}
