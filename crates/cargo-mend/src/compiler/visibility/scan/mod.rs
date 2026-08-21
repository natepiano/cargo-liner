use anyhow::Result;

use super::annotation::VisibilityAnnotation;
use super::annotation::VisibilitySyntax;
use super::policy;
use super::policy::SignatureExposure;
use super::use_sites::ParentFacadeAnalysis;
use crate::compiler::persistence::FindingsSink;

mod classify;
mod finding_params;
mod record;
mod visibility_context;
mod visit;

pub(super) use classify::CrateKind;
pub(super) use classify::ModuleLocation;
pub(super) use classify::ParentVisibility;
pub(super) use finding_params::AllowanceReason;
pub(super) use finding_params::FindingParams;
pub(super) use finding_params::SuspiciousPubAssessment;
pub(super) use finding_params::SuspiciousPubInput;
pub(super) use visibility_context::ItemCategory;
pub(super) use visibility_context::ItemInfo;
pub(super) use visibility_context::VisibilityContext;
pub(super) use visibility_context::collect_and_store_findings;

#[derive(Clone, Copy)]
pub(super) enum ExposureConsumers {
    AllVisibilityFindings,
    ForbiddenAnnotation,
}

pub(super) fn signature_exposure_for_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    consumers: ExposureConsumers,
) -> Result<SignatureExposure> {
    if !annotation_consumes_signature_exposure(annotation.syntax(), consumers) {
        return Ok(SignatureExposure::Contained);
    }
    policy::signature_exposure_reach(ctx, item.def_id, item.file_path, item.name)
}

const fn annotation_consumes_signature_exposure(
    syntax: VisibilitySyntax,
    consumers: ExposureConsumers,
) -> bool {
    let forbidden_annotation = matches!(
        syntax,
        VisibilitySyntax::Crate
            | VisibilitySyntax::InCrate
            | VisibilitySyntax::InParent
            | VisibilitySyntax::InCurrent
            | VisibilitySyntax::InPath(_)
    );
    forbidden_annotation
        || matches!(
            (syntax, consumers),
            (
                VisibilitySyntax::Public,
                ExposureConsumers::AllVisibilityFindings
            )
        )
}

pub(super) fn record_forbidden_visibility_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    sink: &mut FindingsSink,
) -> Result<bool> {
    let finding_context = classify::visibility_finding_context(ctx, item);
    let signature_exposure = signature_exposure_for_annotation(
        ctx,
        item,
        annotation,
        ExposureConsumers::ForbiddenAnnotation,
    )?;
    record::record_forbidden_visibility_annotation(
        ctx,
        item,
        annotation,
        &finding_context,
        parent_facade_analysis,
        signature_exposure,
        sink,
    )
}

#[cfg(test)]
mod tests {
    use super::ExposureConsumers;
    use super::VisibilitySyntax;
    use super::annotation_consumes_signature_exposure;
    use crate::compiler::visibility::annotation::PathSpelling;

    #[test]
    fn signature_exposure_analysis_is_lazy_by_annotation_consumer() {
        for consumers in [
            ExposureConsumers::AllVisibilityFindings,
            ExposureConsumers::ForbiddenAnnotation,
        ] {
            assert!(!annotation_consumes_signature_exposure(
                VisibilitySyntax::Private,
                consumers,
            ));
        }
        assert!(annotation_consumes_signature_exposure(
            VisibilitySyntax::Public,
            ExposureConsumers::AllVisibilityFindings,
        ));
        assert!(!annotation_consumes_signature_exposure(
            VisibilitySyntax::Public,
            ExposureConsumers::ForbiddenAnnotation,
        ));
        for syntax in [
            VisibilitySyntax::Crate,
            VisibilitySyntax::InCrate,
            VisibilitySyntax::InParent,
            VisibilitySyntax::InCurrent,
            VisibilitySyntax::InPath(PathSpelling::CrateRooted),
            VisibilitySyntax::InPath(PathSpelling::Relative),
        ] {
            assert!(annotation_consumes_signature_exposure(
                syntax,
                ExposureConsumers::ForbiddenAnnotation,
            ));
        }
    }
}
