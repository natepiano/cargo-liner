use std::path::Path;

use crate::project::RootItem;
use crate::tui::app::App;
use crate::tui::render;

impl App {
    pub fn formatted_disk(&self, path: &Path) -> String {
        let bytes = self
            .projects()
            .at_path(path)
            .and_then(|project| project.disk_usage_bytes)
            .unwrap_or(0);
        render::format_bytes(bytes)
    }

    /// Aggregate disk usage for a `RootItem`.
    pub fn formatted_disk_for_item(item: &RootItem) -> String {
        item.disk_usage_bytes()
            .map_or_else(|| render::format_bytes(0), render::format_bytes)
    }
}
