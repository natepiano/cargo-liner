mod actions;
mod ci;
mod cpu;
mod data;
mod dispatch;
mod git;
mod lang;
mod layout;
mod lints;
mod package;
mod pane_impls;
mod spec;
mod support;
mod system;

#[cfg(test)]
mod tests;

pub(super) use actions::handle_ci_runs_key;
pub(super) use actions::handle_detail_key;
pub(super) use actions::handle_lints_key;
#[cfg(test)]
pub(super) use ci::CI_COMPACT_DURATION_WIDTH;
#[cfg(test)]
pub(super) use ci::ci_table_shows_durations;
#[cfg(test)]
pub(super) use ci::ci_total_width;
// `render_ci_panel` removed in Phase 8.11 — CiPane::render is the
// trait method now. `render_tiled_pane` dispatches via `panes.dispatch_ci_render`.
#[cfg(test)]
pub(super) use cpu::CPU_PANE_WIDTH;
#[cfg(test)]
pub(super) use cpu::cpu_required_pane_height;
// `render_cpu_panel` removed in Phase 8.9 — CpuPane::render is the
// trait method now. `render_tiled_pane` dispatches via `panes.cpu_mut().render`.
pub(super) use data::DetailCacheKey;
pub(super) use data::PaneDataStore;
// Phase 7 foundation types live in `dispatch` and `pane_impls` and
// stay private to this module during Phase 7. Consumers outside
// `panes/` start wiring up in Phase 8 as render/input bodies
// migrate; the re-exports land then.
#[cfg(test)]
pub(super) use git::git_label_width;
pub(super) use git::render_git_panel;
// `render_lang_panel_standalone` removed in Phase 8.12 — LangPane::render
// is the trait method now.
pub(super) use layout::BottomRow;
pub(super) use layout::LayoutCache;
pub(super) use layout::resolve_layout;
pub(super) use layout::tab_order;
// `render_lints_panel` removed in Phase 8.10 — LintsPane::render is the
// trait method now. `render_tiled_pane` dispatches via `panes.dispatch_lints_render`.
pub(super) use package::RenderStyles;
#[cfg(test)]
pub(super) use package::description_lines;
#[cfg(test)]
pub(super) use package::detail_column_scroll_offset;
#[cfg(test)]
pub(super) use package::package_label_width;
pub(super) use package::render_empty_targets_panel;
pub(super) use package::render_package_panel;
pub(super) use package::render_targets_panel;
#[cfg(test)]
pub(super) use package::stats_column_width;
pub(super) use spec::PaneBehavior;
pub(super) use spec::PaneId;
pub(super) use spec::behavior;
pub(super) use spec::has_row_hitboxes;
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
