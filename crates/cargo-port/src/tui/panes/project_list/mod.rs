//! `ProjectList` pane render bodies.

use ratatui::widgets::ListItem;
use tui_pane::Viewport;

#[cfg(test)]
use crate::project::RootItem;
use crate::tui::columns::ProjectListWidths;
use crate::tui::project_list_state::DiskCacheKey;
use crate::tui::render_context::PaneRenderCtx;

mod disk;
mod pane;
mod tree_render;
mod tree_rows;

pub use pane::ProjectListPane;

pub(super) fn compute_disk_cache(key: &DiskCacheKey) -> Vec<u64> { disk::compute_disk_cache(key) }

#[cfg(test)]
pub(super) fn formatted_disk_for_item(item: &RootItem) -> String {
    disk::formatted_disk_for_item(item)
}

pub(super) fn render_tree_items(
    ctx: &PaneRenderCtx<'_>,
    pane: &ProjectListPane,
    viewport: &Viewport,
    widths: &ProjectListWidths,
) -> Vec<ListItem<'static>> {
    tree_rows::render_tree_items(ctx, pane, viewport, widths)
}

pub(in crate::tui) use tree_rows::ProjectRowPresentation;
pub(in crate::tui) use tree_rows::ProjectRowWidthInput;
pub(in crate::tui) use tree_rows::project_width_presentations;
