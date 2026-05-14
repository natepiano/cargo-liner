mod actions;
mod ci;
mod constants;
mod cpu;
mod data;
mod git;
mod lang;
mod layout;
mod lints;
mod output;
mod package;
mod pane_data;
mod pane_impls;
mod project_list;
mod spec;
mod system;
mod targets;
mod widths;

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(super) use actions::handle_ci_runs_key;
#[cfg(test)]
pub(super) use ci::CI_COMPACT_DURATION_WIDTH;
#[cfg(test)]
pub(super) use ci::ci_table_shows_durations;
#[cfg(test)]
pub(super) use ci::ci_total_width;
pub(super) use ci::render_ci_pane_body;
#[cfg(test)]
pub(super) use constants::PREFIX_ROOT_COLLAPSED;
#[cfg(test)]
pub(super) use constants::PREFIX_ROOT_LEAF;
#[cfg(test)]
pub(super) use constants::PREFIX_WT_FLAT;
#[cfg(test)]
pub(super) use cpu::CPU_PANE_WIDTH;
#[cfg(test)]
pub(super) use cpu::cpu_required_pane_height;
pub(super) use data::DetailCacheKey;
#[cfg(test)]
pub(super) use git::git_label_width;
pub(super) use layout::BottomRow;
pub(super) use layout::resolve_layout;
pub(super) use layout::tab_order;
pub(super) use lints::render_lints_pane_body;
#[cfg(test)]
pub(super) use package::description_lines;
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
pub(super) use pane_data::PackageData;
pub(super) use pane_data::PendingCiFetch;
pub(super) use pane_data::PendingExampleRun;
pub(super) use pane_data::RemoteRow;
pub(super) use pane_data::RunTargetKind;
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
pub(super) use pane_data::format_date;
pub(super) use pane_data::format_duration;
pub(super) use pane_data::format_time;
pub(super) use pane_data::format_timestamp;
pub(super) use pane_data::git_fields_from_data;
pub(super) use pane_data::git_row_at;
pub(super) use pane_data::package_fields_from_data;
pub(super) use pane_impls::hit_test_table_row;
pub(super) use project_list::compute_disk_cache;
pub(super) use project_list::formatted_disk;
pub(super) use project_list::formatted_disk_for_item;
#[cfg(test)]
pub(super) use project_list::render_tree_items;
pub(super) use spec::PaneBehavior;
pub(super) use spec::PaneId;
pub(super) use spec::behavior;
pub(super) use spec::size_spec;
pub(super) use system::DispatchArgs;
pub(super) use system::Panes;
use tui_pane::FocusedPane;
pub(super) use widths::compute_project_list_widths;
#[cfg(test)]
pub(super) use widths::name_width_with_gutter;

use super::app::App;
use super::integration::AppPaneId;
use super::integration::CpuAction;
use super::integration::LangAction;
use super::integration::NavigationAction;
pub(super) use super::state::CiDisplay;
pub(super) use super::state::Lint;
pub(super) use super::state::LintDisplay;
use crate::keymap::CiRunsAction;
use crate::keymap::GitAction;
use crate::keymap::LintsAction;
use crate::keymap::PackageAction;
use crate::keymap::TargetsAction;

pub(super) fn dispatch_package_action(action: PackageAction, app: &mut App) {
    actions::dispatch_package_action(action, app);
}

pub(super) fn dispatch_git_action(action: GitAction, app: &mut App) {
    actions::dispatch_git_action(action, app);
}

pub(super) fn dispatch_targets_action(action: TargetsAction, app: &mut App) {
    actions::dispatch_targets_action(action, app);
}

pub(super) fn dispatch_lang_action(action: LangAction, app: &mut App) {
    actions::dispatch_lang_action(action, app);
}

pub(super) const fn dispatch_cpu_action(action: CpuAction, app: &mut App) {
    actions::dispatch_cpu_action(action, app);
}

pub(super) fn dispatch_lints_action(action: LintsAction, app: &mut App) {
    actions::dispatch_lints_action(action, app);
}

pub(super) fn dispatch_ci_runs_action(action: CiRunsAction, app: &mut App) {
    actions::dispatch_ci_runs_action(action, app);
}

pub(super) fn dispatch_navigation_action(
    action: NavigationAction,
    focused: FocusedPane<AppPaneId>,
    app: &mut App,
) {
    actions::dispatch_navigation_action(action, focused, app);
}
