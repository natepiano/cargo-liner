mod boundary;
mod exports;
mod reference;

pub(super) use boundary::ModuleSourceMap;
pub(super) use boundary::logical_parent_boundary_for_child;
pub(super) use boundary::module_is_within;
pub(super) use exports::ParentFacadeExportRequest;
pub(super) use exports::ParentFacadeExportStatus;
pub(super) use exports::ParentFacadeFixSupport;
pub(super) use exports::ParentFacadeReach;
pub(super) use exports::ParentFacadeSpelling;
pub(super) use exports::ParentFacadeVisibility;
pub(super) use exports::parent_facade_export_status;
pub(super) use reference::ParentFacadeUsage;
pub(super) use reference::path_exists_outside_child_module;
pub(super) use reference::path_exists_outside_module;
pub(super) use reference::workspace_source_mentions_parent_export_literal;
