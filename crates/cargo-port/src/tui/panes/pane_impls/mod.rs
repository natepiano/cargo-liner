use ratatui::layout::Rect;

use super::git::GitVisualRowSpan;

mod cpu_pane;
mod git_pane;
mod helpers;
mod lang_pane;
mod output_pane;
mod package_pane;
mod project_list_pane;
mod targets_pane;

pub use cpu_pane::CpuPane;
pub use git_pane::GitPane;
pub use helpers::hit_test_table_row;
pub use lang_pane::LangPane;
pub use output_pane::OutputPane;
pub use package_pane::PackagePane;
pub use project_list_pane::ProjectListPane;
pub use targets_pane::TargetsPane;

pub(super) fn set_git_row_layout(
    pane: &mut GitPane,
    description_rect: Option<Rect>,
    content_area: Rect,
    row_offset: usize,
    row_spans: Vec<GitVisualRowSpan>,
) {
    pane.set_row_layout(description_rect, content_area, row_offset, row_spans);
}
