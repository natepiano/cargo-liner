mod annotation;
mod field;
mod interface_ceiling;
mod policy;
mod scan;
mod source;
mod use_sites;

pub(super) use annotation::VisibilityReach;
pub(super) use annotation::anchored;
pub(super) use annotation::capped_by_enclosing_modules;
use anyhow::Result;
pub(super) use policy::NoFacadeVisibilityRepair;
pub(super) use policy::classify_no_facade_callers;
pub(super) use policy::common_ancestor_def_path;
pub(super) use policy::crate_rooted_def_path;
pub(super) use policy::def_path_is_descendant;
pub(super) use policy::forbidden_pub_in_headline;
pub(super) use policy::interface_leak_note;
pub(super) use policy::is_annotation_policy_headline;
pub(super) use policy::no_facade_caller_note;
pub(super) use policy::no_facade_headline;
pub(super) use policy::no_facade_suggestion;
pub(super) use policy::parent_scope_def_path;
pub(super) use policy::resolved_boundary_headline;
pub(super) use policy::resolved_boundary_note;
use rustc_middle::ty::TyCtxt;
pub(super) use use_sites::ReexportIndex;

use super::settings::DriverSettings;

pub(super) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> Result<bool> {
    scan::collect_and_store_findings(tcx, settings)
}
