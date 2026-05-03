use ratatui::style::Style;

use super::App;
use crate::constants::LINT_NO_LOG;
use crate::lint::LintStatus;
use super::VisibleRow;
use crate::tui::columns::LintCell;
use crate::tui::constants::ACCENT_COLOR;

impl App {
    /// Whether the currently selected row is a lint-owning node.
    ///
    /// Only roots and worktree entries own lint state. Members, vendored
    /// packages, and group headers do not — the match is exhaustive so new
    /// variants must be classified.
    pub fn selected_row_owns_lint(&self) -> bool {
        match self.selected_row() {
            Some(
                VisibleRow::Root { .. }
                | VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. },
            ) => true,
            Some(
                VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::Submodule { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => false,
        }
    }

    /// Resolve a [`LintStatus`] to the [`LintCell`] (icon + style pair)
    /// rendered in the Lint column. Single source of truth: the icon and
    /// style cannot drift because both derive from the same status here.
    /// Returns the `NoLog` cell when lint is disabled.
    pub fn lint_cell(&self, status: &LintStatus) -> LintCell {
        if !self.lint_enabled() {
            return LintCell::from_parts(LINT_NO_LOG, Style::default());
        }
        let icon = status.icon().frame_at(self.animation_elapsed());
        let style = if matches!(status, LintStatus::Running(_)) {
            Style::default().fg(ACCENT_COLOR)
        } else {
            Style::default()
        };
        LintCell::from_parts(icon, style)
    }
}
