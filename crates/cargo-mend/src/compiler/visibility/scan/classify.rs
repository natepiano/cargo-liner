use super::ItemInfo;
use super::VisibilityContext;
use crate::compiler::settings;
use crate::compiler::visibility::policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler::visibility) enum CrateKind {
    Binary,
    Library,
    IntegrationTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler::visibility) enum ModuleLocation {
    CrateRoot,
    ShallowPrivate,
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::compiler::visibility) enum ParentVisibility {
    Public,
    Private,
}

pub(super) struct VisibilityFindingContext {
    pub(super) crate_kind:           CrateKind,
    pub(super) config_rel_path:      Option<String>,
    pub(super) logical_module_depth: usize,
    pub(super) module_location:      ModuleLocation,
    pub(super) parent_visibility:    ParentVisibility,
}

pub(super) fn visibility_finding_context(
    visibility_context: &VisibilityContext<'_, '_>,
    item_info: &ItemInfo<'_>,
) -> VisibilityFindingContext {
    let crate_kind = policy::crate_kind_for_root(
        visibility_context.root_module,
        &visibility_context.settings.package_root,
    );
    let config_rel_path = settings::config_relative_path_for_settings(
        item_info.file_path,
        visibility_context.settings,
    );
    let parent_module = visibility_context
        .tcx
        .parent_module_from_def_id(item_info.def_id);
    let parent_visibility = if visibility_context
        .tcx
        .local_visibility(parent_module.to_local_def_id())
        .is_public()
    {
        ParentVisibility::Public
    } else {
        ParentVisibility::Private
    };
    let module_location =
        policy::resolve_module_location(visibility_context.tcx, parent_module.to_local_def_id());
    let logical_module_depth =
        policy::module_depth(visibility_context.tcx, parent_module.to_local_def_id());

    VisibilityFindingContext {
        crate_kind,
        config_rel_path,
        logical_module_depth,
        module_location,
        parent_visibility,
    }
}
