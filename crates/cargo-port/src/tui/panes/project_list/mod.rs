//! `ProjectList` pane render bodies.

use std::collections::HashMap;

#[cfg(test)]
use ratatui::widgets::ListItem;
#[cfg(test)]
use tui_pane::Viewport;

#[cfg(test)]
use super::pane_impls::ProjectListPane;
#[cfg(test)]
use crate::project::RootItem;
#[cfg(test)]
use crate::tui::app::ProjectListWidths;
#[cfg(test)]
use crate::tui::pane::PaneRenderCtx;
use crate::tui::project_list::ProjectList;

mod disk;
mod tree_render;

pub use tree_render::render_project_list_pane_body;

pub(super) fn compute_disk_cache(entries: &ProjectList) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    disk::compute_disk_cache(entries)
}

#[cfg(test)]
pub(super) fn formatted_disk_for_item(item: &RootItem) -> String {
    disk::formatted_disk_for_item(item)
}

#[cfg(test)]
pub(super) fn render_tree_items(
    ctx: &PaneRenderCtx<'_>,
    pane: &ProjectListPane,
    viewport: &Viewport,
    widths: &ProjectListWidths,
) -> Vec<ListItem<'static>> {
    tree_render::render_tree_items(ctx, pane, viewport, widths)
}
