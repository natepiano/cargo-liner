mod actions;
mod ci;
mod constants;
mod cpu;
mod data;
mod description;
mod git;
mod hit_test;
mod lang;
mod layout;
mod lints;
mod output;
mod package;
mod pane_data;
mod project_list;
mod spec;
mod system;
mod targets;
mod widths;

#[cfg(test)]
mod tests;

use std::collections::HashMap;

#[cfg(test)]
pub(super) use actions::handle_ci_runs_key;
pub(super) use ci::render_ci_pane_body;
#[cfg(test)]
pub(super) use constants::PREFIX_ROOT_COLLAPSED;
#[cfg(test)]
pub(super) use constants::PREFIX_ROOT_LEAF;
#[cfg(test)]
pub(super) use constants::PREFIX_WT_FLAT;
#[cfg(test)]
pub(super) use cpu::CPU_PANE_WIDTH;
pub(super) use cpu::CpuPane;
#[cfg(test)]
pub(super) use cpu::cpu_required_pane_height;
pub(super) use data::DetailCacheKey;
pub(super) use description::DescriptionBlock;
pub(super) use description::EmptyDescriptionBehavior;
pub(super) use description::SyncedDescriptionHeight;
#[cfg(test)]
pub(super) use description::placeholder_text;
pub(super) use description::sync_floor;
pub(super) use git::GitPane;
#[cfg(test)]
pub(super) use git::git_label_width;
pub(super) use hit_test::hit_test_table_row;
pub(super) use lang::LangPane;
pub(super) use layout::BottomRow;
pub(super) use layout::resolve_layout;
pub(super) use layout::tab_order;
pub(super) use layout::top_pane_widths;
pub(super) use lints::render_lints_pane_body;
pub(super) use output::OutputPane;
pub(super) use package::PackagePane;
#[cfg(test)]
pub(super) use package::detail_column_scroll_offset;
#[cfg(test)]
pub(super) use package::package_label_width;
#[cfg(test)]
pub(super) use package::stats_column_width;
pub(super) use pane_data::BuildMode;
pub(super) use pane_data::CiData;
#[cfg(test)]
pub(super) use pane_data::CiEmptyState;
pub(super) use pane_data::CiFetchKind;
pub(super) use pane_data::DetailField;
pub(super) use pane_data::DetailPaneData;
pub(super) use pane_data::GitData;
pub(super) use pane_data::GitRow;
pub(super) use pane_data::LintsData;
#[cfg(test)]
pub(super) use pane_data::LintsProjectKind;
pub(super) use pane_data::PackageData;
#[cfg(test)]
pub(super) use pane_data::PackagePresence;
pub(super) use pane_data::PackageRow;
#[cfg(test)]
pub(super) use pane_data::PackageSection;
pub(super) use pane_data::PendingCiFetch;
pub(super) use pane_data::PendingExampleRun;
#[cfg(test)]
pub(super) use pane_data::PullRequestPolling;
pub(super) use pane_data::PullRequestRow;
pub(super) use pane_data::PullRequestSection;
pub(super) use pane_data::PullRequestSectionState;
pub(super) use pane_data::RemoteRow;
pub(super) use pane_data::RunTargetKind;
pub(super) use pane_data::TargetEntry;
#[cfg(test)]
pub(super) use pane_data::TargetSource;
pub(super) use pane_data::TargetsData;
pub(super) use pane_data::WorktreeInfo;
pub(super) use pane_data::build_ci_data;
pub(super) use pane_data::build_lints_data;
pub(super) use pane_data::build_pane_data;
pub(super) use pane_data::build_pane_data_for_member;
pub(super) use pane_data::build_pane_data_for_submodule;
pub(super) use pane_data::build_pane_data_for_vendored;
pub(super) use pane_data::build_pane_data_for_workspace_ref;
pub(super) use pane_data::build_target_list_from_data;
pub(super) use pane_data::copy_payload_for_ci;
pub(super) use pane_data::copy_payload_for_git;
pub(super) use pane_data::copy_payload_for_lints;
pub(super) use pane_data::copy_payload_for_output;
pub(super) use pane_data::copy_payload_for_package;
pub(super) use pane_data::copy_payload_for_targets;
pub(super) use pane_data::format_ahead_behind;
pub(super) use pane_data::format_date;
pub(super) use pane_data::format_duration;
pub(super) use pane_data::format_time;
pub(super) use pane_data::format_timestamp;
pub(super) use pane_data::git_fields_from_data;
pub(super) use pane_data::git_has_description_row;
pub(super) use pane_data::git_row_at;
pub(super) use pane_data::github_stars_is_unreachable_placeholder;
pub(super) use pane_data::max_top_pane_inner_height;
pub(super) use pane_data::package_first_selectable_row;
pub(super) use pane_data::package_last_selectable_row;
pub(super) use pane_data::package_nearest_selectable_row;
pub(super) use pane_data::package_row_is_selectable;
pub(super) use pane_data::package_rows_from_data;
pub(super) use pane_data::package_selectable_row_at_or_after;
pub(super) use pane_data::package_selectable_row_at_or_before;
pub(super) use project_list::ProjectListPane;
#[cfg(test)]
use ratatui::widgets::ListItem;
pub(super) use spec::PaneBehavior;
pub(super) use spec::PaneId;
pub(super) use spec::behavior;
pub(super) use spec::size_spec;
pub(super) use system::Panes;
pub(super) use targets::CargoGroup;
pub(super) use targets::RunningListRow;
pub(super) use targets::TargetsPane;
pub(super) use targets::build_running_list;
pub(super) use targets::build_running_rows;
pub(super) use targets::format_start_age;
pub(super) use targets::outline_subtree_len;
pub(super) use targets::resolve_kill_request;
use tui_pane::FocusedPane;
#[cfg(test)]
use tui_pane::Viewport;
pub(super) use widths::compute_project_list_widths;
#[cfg(test)]
pub(super) use widths::name_width_with_gutter;

use super::app::App;
#[cfg(test)]
use super::app::ProjectListWidths;
use super::integration::AppPaneId;
use super::integration::NavAction;
use super::keymap::CiRunsAction;
use super::keymap::GitAction;
use super::keymap::LintsAction;
use super::keymap::PackageAction;
use super::keymap::TargetsAction;
use super::project_list::ProjectList;
#[cfg(test)]
use super::render_context::PaneRenderCtx;
pub(super) use super::state::CiDisplay;
pub(super) use super::state::Lint;
pub(super) use super::state::LintDisplay;
#[cfg(test)]
use crate::project::RootItem;

pub(super) fn dispatch_package_action(action: PackageAction, app: &mut App) {
    actions::dispatch_package_action(action, app);
}

pub(super) fn dispatch_git_action(action: GitAction, app: &mut App) {
    actions::dispatch_git_action(action, app);
}

pub(super) fn dispatch_targets_action(action: TargetsAction, app: &mut App) {
    actions::dispatch_targets_action(action, app);
}

pub(super) fn execute_target_kill(app: &mut App, pid: u32, create_time: u64) {
    actions::execute_target_kill(app, pid, create_time);
}

pub(super) fn sync_running_targets_cursor(app: &mut App) { actions::sync_running_cursor_pid(app); }

pub(super) fn compute_disk_cache(entries: &ProjectList) -> (Vec<u64>, HashMap<usize, Vec<u64>>) {
    project_list::compute_disk_cache(entries)
}

#[cfg(test)]
pub(super) fn formatted_disk_for_item(item: &RootItem) -> String {
    project_list::formatted_disk_for_item(item)
}

#[cfg(test)]
pub(super) fn render_tree_items(
    ctx: &PaneRenderCtx<'_>,
    pane: &ProjectListPane,
    viewport: &Viewport,
    widths: &ProjectListWidths,
) -> Vec<ListItem<'static>> {
    project_list::render_tree_items(ctx, pane, viewport, widths)
}

pub(super) fn toggle_targets_tree_row(app: &mut App) -> bool {
    actions::toggle_targets_tree_row(app)
}

pub(super) fn dispatch_lints_action(action: LintsAction, app: &mut App) {
    actions::dispatch_lints_action(action, app);
}

pub(super) fn dispatch_ci_runs_action(action: CiRunsAction, app: &mut App) {
    actions::dispatch_ci_runs_action(action, app);
}

pub(super) fn dispatch_navigation_action(
    action: NavAction,
    focused: FocusedPane<AppPaneId>,
    app: &mut App,
) {
    actions::dispatch_navigation_action(action, focused, app);
}

pub(super) fn navigate_package_detail(app: &mut App, action: NavAction) {
    actions::navigate_package_detail(app, action);
}

pub(super) fn request_clean(app: &mut App) { actions::request_clean(app); }
