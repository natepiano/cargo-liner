use crate::project::DisplayPath;
use crate::project::RootItem;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::project_list::ProjectListRowDisplayPathResolution;

impl App {
    /// Returns the `RootItem` when a root row is selected.
    pub(in crate::tui) fn selected_item(&self) -> Option<&RootItem> {
        match self.project_list.selected_row()? {
            VisibleRow::Root { node_index } => self
                .project_list
                .get(node_index)
                .map(|entry| &entry.root_item),
            _ => None,
        }
    }

    /// Resolve the display path of the currently selected row using `project_list_items`.
    pub(in crate::tui::app) fn selected_display_path(&self) -> Option<DisplayPath> {
        let rows = self.visible_rows();
        let selected = self.project_list.cursor();
        let row = rows.get(selected)?;
        match self.project_list.display_path_for_row(*row) {
            ProjectListRowDisplayPathResolution::Resolved(display_path) => Some(display_path),
            ProjectListRowDisplayPathResolution::RowUnavailable => None,
        }
    }
}
