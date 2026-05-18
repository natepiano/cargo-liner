use std::path::Path;

use rustc_span::Span;
use rustc_span::def_id::LocalDefId;

use super::classify::CrateKind;
use super::classify::ModuleLocation;
use super::classify::ParentVisibility;
use crate::compiler::facade::ParentFacadeExportStatus;
use crate::config::DiagnosticCode;
use crate::reporting::FixSupport;
use crate::reporting::Severity;

pub struct SuspiciousPubInput<'a> {
    pub def_id:            LocalDefId,
    pub file_path:         &'a Path,
    pub config_rel_path:   Option<&'a str>,
    pub parent_visibility: ParentVisibility,
    pub module_location:   ModuleLocation,
    pub crate_kind:        CrateKind,
    pub kind_label:        Option<&'static str>,
    pub name:              Option<&'a str>,
    pub highlight_span:    Span,
}

pub struct FindingParams {
    pub severity:                Severity,
    pub diagnostic_code:         DiagnosticCode,
    pub item:                    Option<String>,
    pub message:                 String,
    pub suggestion:              Option<String>,
    pub fixability:              FixSupport,
    pub related:                 Option<String>,
    pub item_def_path:           Option<String>,
    pub narrower_scope_def_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AllowanceReason {
    Allowlist,
    ParentIsPublic,
    ShallowPrivatePolicy,
    ReachablePublicApi,
    ParentFacadeUsedOutsideParent,
    InternalParentFacadeBoundary,
    ExposedByOtherCrateVisibleSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuspiciousPubAssessment {
    Allowed(AllowanceReason),
    ReviewInternalParentFacade {
        related: Option<String>,
    },
    Warn {
        fixability:           FixSupport,
        related:              Option<String>,
        stale_parent_pub_use: Option<ParentFacadeExportStatus>,
    },
}
