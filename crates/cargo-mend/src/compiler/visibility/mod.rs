mod annotation;
mod field;
mod policy;
mod scan;
mod source;
mod use_sites;

use anyhow::Result;
pub(super) use policy::NoFacadeAdvice;
pub(super) use policy::canonical_pub_in_boundary;
pub(super) use policy::classify_no_facade_callers;
pub(super) use policy::no_facade_suggestion;
pub(super) use policy::parent_scope_def_path;
use rustc_middle::ty::TyCtxt;

use super::settings::DriverSettings;

pub(super) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> Result<bool> {
    scan::collect_and_store_findings(tcx, settings)
}
