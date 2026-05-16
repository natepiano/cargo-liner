mod boundary;
mod exports;
mod reference;

pub(super) use boundary::parent_boundary_for_child;
pub(super) use exports::ParentFacadeExportStatus;
pub(super) use exports::parent_facade_export_status;
pub(super) use exports::root_module_exports_item;
pub(super) use reference::ParentFacadeReferenceUsage;
pub(super) use reference::ParentFacadeUsage;
pub(super) use reference::public_reexport_exists_outside_parent;
pub(super) use reference::source_references_parent_export;
pub(super) use reference::workspace_source_mentions_parent_export_literal;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ParentFacadeFixSupport {
    #[default]
    Unsupported,
    Supported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParentFacadeVisibility {
    Public,
    Crate,
    Super,
}
