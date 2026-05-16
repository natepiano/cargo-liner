use super::ItemInfo;
use super::VisibilityContext;
use crate::compiler::settings;
use crate::compiler::visibility::policy;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrateKind {
    Binary,
    Library,
    IntegrationTest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleLocation {
    CrateRoot,
    ShallowPrivate,
    Nested,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentVisibility {
    Public,
    Private,
}

pub(super) struct VisibilityFindingContext {
    pub(super) crate_kind:        CrateKind,
    pub(super) config_rel_path:   Option<String>,
    pub(super) module_location:   ModuleLocation,
    pub(super) parent_visibility: ParentVisibility,
}

pub(super) fn visibility_finding_context(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
) -> VisibilityFindingContext {
    let crate_kind = policy::crate_kind_for_root(ctx.root_module, &ctx.settings.package_root);
    let config_rel_path = settings::config_relative_path_for_settings(item.file_path, ctx.settings);
    let parent_module = ctx.tcx.parent_module_from_def_id(item.def_id);
    let parent_visibility = if ctx
        .tcx
        .local_visibility(parent_module.to_local_def_id())
        .is_public()
    {
        ParentVisibility::Public
    } else {
        ParentVisibility::Private
    };
    let module_location = policy::resolve_module_location(ctx.tcx, parent_module.to_local_def_id());

    VisibilityFindingContext {
        crate_kind,
        config_rel_path,
        module_location,
        parent_visibility,
    }
}
