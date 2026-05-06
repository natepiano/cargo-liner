use crate::project::AbsolutePath;
use crate::project::DisplayPath;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::WorktreeGroup;
use crate::tui::app::App;
use crate::tui::app::VisibleRow;
use crate::tui::app::target_index::CleanSelection;
use crate::tui::project_list::ProjectList;

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

    /// Map the currently selected row to a [`CleanSelection`] when the
    /// Clean shortcut should be enabled on it.
    pub fn clean_selection(&self) -> Option<CleanSelection> {
        let row = self.project_list.selected_row()?;
        match row {
            VisibleRow::Root { node_index } => {
                let entry = self.project_list.get(node_index)?;
                match &entry.item {
                    RootItem::Rust(rust) => Some(CleanSelection::Project {
                        root: rust.path().clone(),
                    }),
                    RootItem::Worktrees(group) => Some(worktree_group_selection(group)),
                    RootItem::NonRust(_) => None,
                }
            },
            VisibleRow::WorktreeEntry {
                node_index,
                worktree_index,
            } => {
                let entry = self.project_list.get(node_index)?;
                ProjectList::worktree_path_ref(&entry.item, worktree_index).map(|path| {
                    CleanSelection::Project {
                        root: AbsolutePath::from(path),
                    }
                })
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

/// Build a `CleanSelection::WorktreeGroup` from a live
/// [`WorktreeGroup`]. Enum-agnostic (works for both Workspaces and
/// Packages variants) so the caller doesn't have to match twice.
fn worktree_group_selection(group: &WorktreeGroup) -> CleanSelection {
    match group {
        WorktreeGroup::Workspaces { primary, linked } => CleanSelection::WorktreeGroup {
            primary: primary.path().clone(),
            linked:  linked.iter().map(|ws| ws.path().clone()).collect(),
        },
        WorktreeGroup::Packages { primary, linked } => CleanSelection::WorktreeGroup {
            primary: primary.path().clone(),
            linked:  linked.iter().map(|pkg| pkg.path().clone()).collect(),
        },
    }
}
