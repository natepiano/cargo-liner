use anyhow::Result;

use super::annotation::VisibilityAnnotation;
use super::annotation::VisibilitySyntax;
use super::policy;
use super::policy::SignatureExposure;
use super::use_sites::ParentFacadeAnalysis;
use crate::compiler::persistence::FindingsSink;

mod classify;
mod finding_params;
mod module_mounts;
mod pub_use_outside_subtree;
mod record;
mod subtree_reexport_fix;
mod visibility_context;
mod visit;

pub(super) use classify::CrateKind;
pub(super) use classify::ModuleLocation;
pub(super) use classify::ParentVisibility;
pub(super) use finding_params::AllowanceReason;
pub(super) use finding_params::FindingParams;
pub(super) use finding_params::SuspiciousPubAssessment;
pub(super) use finding_params::SuspiciousPubInput;
pub(in crate::compiler) use visibility_context::FindingsPersistence;
pub(super) use visibility_context::ItemCategory;
pub(super) use visibility_context::ItemInfo;
pub(super) use visibility_context::VisibilityContext;
pub(super) use visibility_context::collect_and_store_findings;

/// A handled annotation stops later diagnostics even when its finding was suppressed.
/// Otherwise `record_review_pub_mod` could replace a suppressed diagnostic on the same line.
/// Both narrowing diagnostics must respect callers in excluded configurations: narrowing or
/// removing their annotation would prevent those configurations from compiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AnnotationHandling {
    Handled,
    Continue,
}

#[derive(Clone, Copy)]
pub(super) enum ExposureConsumers {
    AllVisibilityFindings,
    ForbiddenAnnotation,
}

pub(super) fn signature_exposure_for_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item_info: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    exposure_consumers: ExposureConsumers,
) -> Result<SignatureExposure> {
    match signature_exposure_demand(annotation.syntax(), exposure_consumers) {
        SignatureExposureDemand::NotRequired => Ok(SignatureExposure::Contained),
        SignatureExposureDemand::Required => policy::signature_exposure_reach(
            ctx,
            item_info.def_id,
            item_info.file_path,
            item_info.name,
        ),
    }
}

/// Signature exposure is only computed for annotations the selected consumer analyzes.
/// Forbidden annotations always need it; a plain `pub` only needs it for the full scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SignatureExposureDemand {
    Required,
    NotRequired,
}

const fn signature_exposure_demand(
    syntax: VisibilitySyntax,
    consumers: ExposureConsumers,
) -> SignatureExposureDemand {
    match syntax {
        VisibilitySyntax::Crate
        | VisibilitySyntax::InCrate
        | VisibilitySyntax::InParent
        | VisibilitySyntax::InCurrent
        | VisibilitySyntax::InPath(_) => SignatureExposureDemand::Required,
        VisibilitySyntax::Public => match consumers {
            ExposureConsumers::AllVisibilityFindings => SignatureExposureDemand::Required,
            ExposureConsumers::ForbiddenAnnotation => SignatureExposureDemand::NotRequired,
        },
        VisibilitySyntax::Private | VisibilitySyntax::Parent | VisibilitySyntax::Current => {
            SignatureExposureDemand::NotRequired
        },
    }
}

pub(super) fn record_forbidden_visibility_annotation(
    ctx: &VisibilityContext<'_, '_>,
    item_info: &ItemInfo<'_>,
    annotation: &VisibilityAnnotation<'_>,
    parent_facade_analysis: Option<&ParentFacadeAnalysis<'_>>,
    findings_sink: &mut FindingsSink,
) -> Result<AnnotationHandling> {
    let finding_context = classify::visibility_finding_context(ctx, item_info);
    let signature_exposure = signature_exposure_for_annotation(
        ctx,
        item_info,
        annotation,
        ExposureConsumers::ForbiddenAnnotation,
    )?;
    record::record_forbidden_visibility_annotation(
        ctx,
        item_info,
        annotation,
        &finding_context,
        parent_facade_analysis,
        signature_exposure,
        findings_sink,
    )
}

#[cfg(test)]
mod tests {
    use super::ExposureConsumers;
    use super::SignatureExposureDemand;
    use super::VisibilitySyntax;
    use super::signature_exposure_demand;
    use crate::compiler::visibility::annotation::PathSpelling;

    #[test]
    fn signature_exposure_analysis_is_lazy_by_annotation_consumer() {
        for exposure_consumers in [
            ExposureConsumers::AllVisibilityFindings,
            ExposureConsumers::ForbiddenAnnotation,
        ] {
            for syntax in [
                VisibilitySyntax::Private,
                VisibilitySyntax::Parent,
                VisibilitySyntax::Current,
            ] {
                assert_eq!(
                    signature_exposure_demand(syntax, exposure_consumers),
                    SignatureExposureDemand::NotRequired,
                );
            }
            for syntax in [
                VisibilitySyntax::Crate,
                VisibilitySyntax::InCrate,
                VisibilitySyntax::InParent,
                VisibilitySyntax::InCurrent,
                VisibilitySyntax::InPath(PathSpelling::CrateRooted),
                VisibilitySyntax::InPath(PathSpelling::Relative),
            ] {
                assert_eq!(
                    signature_exposure_demand(syntax, exposure_consumers),
                    SignatureExposureDemand::Required,
                );
            }
        }
        assert_eq!(
            signature_exposure_demand(
                VisibilitySyntax::Public,
                ExposureConsumers::AllVisibilityFindings,
            ),
            SignatureExposureDemand::Required,
        );
        assert_eq!(
            signature_exposure_demand(
                VisibilitySyntax::Public,
                ExposureConsumers::ForbiddenAnnotation,
            ),
            SignatureExposureDemand::NotRequired,
        );
    }
}
