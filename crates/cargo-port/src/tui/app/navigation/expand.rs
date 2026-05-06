use crate::tui::app::App;

impl App {
    pub(super) fn selected_is_expandable(&self) -> bool {
        let selected = self.project_list.cursor();
        self.visible_rows()
            .get(selected)
            .copied()
            .and_then(|row| self.project_list.expand_key_for_row(row))
            .is_some()
    }

    pub fn expand(&mut self) -> bool {
        if !self.selected_is_expandable() {
            return false;
        }
        let selected = self.project_list.cursor();
        let Some(row) = self.visible_rows().get(selected).copied() else {
            return false;
        };
        let Some(key) = self.project_list.expand_key_for_row(row) else {
            return false;
        };
        self.project_list.expanded.insert(key)
    }
}
