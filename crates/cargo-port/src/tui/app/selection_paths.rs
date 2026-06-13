use crate::project::AbsolutePath;
use crate::tui::project_list::ExpandTarget;
use crate::tui::terminal;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionSync {
    #[default]
    Stable,
    Changed,
}

impl SelectionSync {
    pub const fn is_changed(self) -> bool { matches!(self, Self::Changed) }
}

#[derive(Debug, Default)]
pub struct SelectionPaths {
    pub last_selected:      Option<AbsolutePath>,
    pub selected_project:   Option<AbsolutePath>,
    pub collapsed_selected: Option<AbsolutePath>,
    pub collapsed_anchor:   Option<AbsolutePath>,
    /// Expansion targets waiting to be applied once the tree is built (see
    /// `App::handle_scan_result`), then drained. Seeded from `tree_state.toml`
    /// at startup and re-seeded from the live tree on every rescan, so a
    /// rescan rebuilds with the same containers open.
    pub pending_expanded:   Vec<ExpandTarget>,
}

impl SelectionPaths {
    pub fn new() -> Self {
        let (last_selected, pending_expanded) = terminal::load_tree_state();
        Self {
            last_selected,
            pending_expanded,
            ..Self::default()
        }
    }
}
