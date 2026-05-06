use crate::project::DisplayPath;
use crate::project::RootItem;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;

impl App {
    /// Returns the `RootItem` when a root row is selected.
    pub fn selected_item(&self) -> Option<&RootItem> {
        match self.project_list.selected_row()? {
            VisibleRow::Root { node_index } => {
                self.project_list.get(node_index).map(|entry| &entry.item)
            },
            _ => None,
        }
    }

    /// Resolve the display path of the currently selected row using `project_list_items`.
    pub fn selected_display_path(&self) -> Option<DisplayPath> {
        let rows = self.visible_rows();
        let selected = self.project_list.cursor();
        let row = rows.get(selected)?;
        self.project_list.display_path_for_row(*row)
    }
}
