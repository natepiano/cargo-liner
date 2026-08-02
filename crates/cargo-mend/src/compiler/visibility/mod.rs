mod annotation;
mod field;
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
pub(super) use policy::def_path_is_descendant;
pub(super) use policy::no_facade_headline;
pub(super) use policy::no_facade_suggestion;
pub(super) use policy::parent_scope_def_path;
use rustc_middle::ty::TyCtxt;
pub(super) use use_sites::ReexportIndex;

use super::settings::DriverSettings;

pub(super) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> Result<bool> {
    scan::collect_and_store_findings(tcx, settings)
}
