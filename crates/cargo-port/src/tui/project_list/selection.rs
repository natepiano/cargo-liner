use tui_pane::ExpansionState;

use super::ExpandKey;
use crate::config::NonRustInclusion;
use crate::tui::project_list_state::ProjectList;

/// Refresh the visible rows after an expansion mutation.
pub(super) struct SelectionMutation<'a> {
    pub(super) project_list: &'a mut ProjectList,
    pub(super) non_rust:     NonRustInclusion,
}

impl SelectionMutation<'_> {
    /// Toggle the shared expansion state and return its new value.
    pub(super) fn toggle_expand(&mut self, key: ExpandKey) -> ExpansionState {
        self.project_list.expanded.toggle(key, 0)
    }
}

impl Drop for SelectionMutation<'_> {
    fn drop(&mut self) {
        self.project_list
            .recompute_visibility(self.non_rust.includes_non_rust());
    }
}
