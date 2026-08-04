mod boundary;
mod exports;
mod reference;

pub(super) use boundary::LogicalParentBoundary;
pub(super) use boundary::ModuleSourceMap;
pub(super) use exports::ParentFacadeExportRequest;
pub(super) use exports::ParentFacadeExportStatus;
pub(super) use exports::ParentFacadeFixSupport;
pub(super) use exports::ParentFacadeReach;
pub(super) use exports::ParentFacadeSpelling;
#[cfg(all(test, feature = "test-counters"))]
pub(super) use exports::facade_resolution_count;
#[cfg(all(test, feature = "test-counters"))]
pub(super) use exports::facade_resolution_request_count;
#[cfg(all(test, feature = "test-counters"))]
pub(super) use exports::facade_usage_scan_count;
pub(super) use exports::parent_facade_export_status;
#[cfg(feature = "test-counters")]
pub(super) use exports::record_facade_resolution;
#[cfg(feature = "test-counters")]
pub(super) use exports::record_facade_resolution_request;
#[cfg(all(test, feature = "test-counters"))]
pub(super) use exports::reset_performance_counters;
pub(super) use reference::ParentFacadeUsage;
pub(super) use reference::ParentFacadeUsageByName;
pub(super) use reference::path_exists_outside_child_module;
pub(super) use reference::path_exists_outside_module;
pub(super) use reference::workspace_source_parent_export_literal_usage;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::LocalDefId;

pub(super) fn logical_parent_boundary_for_child(
    tcx: TyCtxt<'_>,
    child_item: LocalDefId,
) -> Option<LogicalParentBoundary> {
    boundary::logical_parent_boundary_for_child(tcx, child_item)
}
