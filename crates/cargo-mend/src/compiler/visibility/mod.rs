mod field_visibility;
mod policy;
mod scan;
mod source;
mod use_sites;

use rustc_middle::ty::TyCtxt;

use super::settings::DriverSettings;

pub(super) fn collect_and_store_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
) -> anyhow::Result<bool> {
    scan::collect_and_store_findings(tcx, settings)
}
