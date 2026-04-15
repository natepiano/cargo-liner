mod ci_panel;
mod interaction;
mod lints_panel;
mod model;
mod render;
mod timestamp;

#[cfg(test)]
mod tests;

pub(super) use ci_panel::render_ci_panel;
pub(super) use interaction::handle_ci_runs_key;
pub(super) use interaction::handle_detail_key;
pub(super) use interaction::handle_lints_key;
pub(super) use lints_panel::render_lints_panel;
pub(super) use model::CiFetchKind;
pub(super) use model::DetailField;
pub(super) use model::DetailInfo;
pub(super) use model::PendingCiFetch;
pub(super) use model::PendingExampleRun;
pub(super) use model::RunTargetKind;
pub(super) use model::build_detail_info;
pub(super) use model::build_detail_info_for_member;
pub(super) use model::build_detail_info_for_submodule;
pub(super) use model::build_detail_info_for_workspace_ref;
pub(super) use model::build_target_list;
pub(super) use model::git_fields;
pub(super) use model::package_fields;
pub(super) use render::RenderStyles;
pub(super) use render::render_detail_panel;
pub(super) use render::render_git_panel;
pub(super) use render::render_lang_panel_standalone;
pub(super) use render::render_targets_panel;
