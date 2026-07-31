use anyhow::Result;

use super::annotation::VisibilityAnnotation;
use crate::compiler::facade::ParentFacadeReach;
use crate::compiler::persistence::FindingsSink;

mod classify;
mod finding_params;
mod record;
mod visibility_context;
mod visit;

pub(super) use classify::CrateKind;
pub(super) use classify::ModuleLocation;
pub(super) use classify::ParentVisibility;
pub(super) use classify::SignatureExposure;
pub(super) use finding_params::AllowanceReason;
pub(super) use finding_params::FindingParams;
pub(super) use finding_params::SuspiciousPubAssessment;
pub(super) use finding_params::SuspiciousPubInput;
pub(super) use visibility_context::ItemCategory;
pub(super) use visibility_context::ItemInfo;
pub(super) use visibility_context::VisibilityContext;
pub(super) use visibility_context::collect_and_store_findings;

pub(super) fn record_forbidden_visibility_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_reach: Option<ParentFacadeReach>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let finding_context = classify::visibility_finding_context(ctx, item);
    record::record_forbidden_visibility_annotation(
        ctx,
        item,
        annotation,
        &finding_context,
        parent_facade_reach,
        sink,
    )
}
