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
mod pane_impls;
mod project_list;
mod spec;
mod support;
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
pub(super) use layout::LayoutCache;
pub(super) use layout::resolve_layout;
pub(super) use layout::tab_order;
pub(super) use lints::render_lints_pane_body;
pub(super) use output::render_output_panel;
pub(super) use package::RenderStyles;
#[cfg(test)]
pub(super) use package::description_lines;
#[cfg(test)]
pub(super) use package::detail_column_scroll_offset;
#[cfg(test)]
pub(super) use package::package_label_width;
#[cfg(test)]
pub(super) use package::stats_column_width;
pub(super) use pane_impls::hit_test_table_row;
pub(super) use project_list::compute_disk_cache;
pub(super) use project_list::formatted_disk;
pub(super) use project_list::formatted_disk_for_item;
pub(super) use project_list::render_project_list;
#[cfg(test)]
pub(super) use project_list::render_tree_items;
pub(super) use spec::PaneBehavior;
pub(super) use spec::PaneId;
pub(super) use spec::behavior;
pub(super) use spec::size_spec;
pub(super) use support::BuildMode;
pub(super) use support::CiData;
#[cfg(test)]
pub(super) use support::CiEmptyState;
pub(super) use support::CiFetchKind;
pub(super) use support::DetailField;
pub(super) use support::DetailPaneData;
pub(super) use support::GitData;
pub(super) use support::GitRow;
pub(super) use support::LintsData;
pub(super) use support::PackageData;
pub(super) use support::PendingCiFetch;
pub(super) use support::PendingExampleRun;
pub(super) use support::RemoteRow;
pub(super) use support::RunTargetKind;
pub(super) use support::TargetsData;
pub(super) use support::WorktreeInfo;
pub(super) use support::build_ci_data;
pub(super) use support::build_lints_data;
pub(super) use support::build_pane_data;
pub(super) use support::build_pane_data_for_member;
pub(super) use support::build_pane_data_for_submodule;
pub(super) use support::build_pane_data_for_vendored;
pub(super) use support::build_pane_data_for_workspace_ref;
pub(super) use support::build_target_list_from_data;
pub(super) use support::format_date;
pub(super) use support::format_duration;
pub(super) use support::format_time;
pub(super) use support::format_timestamp;
pub(super) use support::git_fields_from_data;
pub(super) use support::git_row_at;
pub(super) use support::package_fields_from_data;
pub(super) use system::DispatchArgs;
pub(super) use system::Panes;
pub(super) use targets::render_empty_targets_panel;
pub(super) use targets::render_targets_panel;
use tui_pane::FocusedPane;
pub(super) use widths::compute_project_list_widths;
#[cfg(test)]
pub(super) use widths::name_width_with_gutter;

use super::app::App;
use super::integration::framework_keymap::AppPaneId;
use super::integration::framework_keymap::CpuAction;
use super::integration::framework_keymap::LangAction;
use super::integration::framework_keymap::NavigationAction;
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
