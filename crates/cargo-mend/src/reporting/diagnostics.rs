use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

use super::constants::HINT_FIXABLE_WITH_FIX;
use super::constants::HINT_FIXABLE_WITH_FIX_PUB_USE;
use crate::config::DiagnosticCode;
use crate::constants::HELP_URL_BASE;

// --- FixSupport (folded from former fix_support.rs) ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FixSupport {
    #[default]
    None,
    ShortenImport,
    PreferModuleImport,
    InlinePathQualifiedType,
    #[serde(rename = "fix_pub_use")]
    PubUse,
    NeedsManualPubUseCleanup,
    InternalParentFacade,
    UnusedPub,
    NarrowToPubCrate,
    #[serde(rename = "fix_field_visibility")]
    FieldVisibility,
    #[serde(rename = "fix_imports_at_top")]
    ImportsAtTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixSummaryBucket {
    Fix,
    PubUse,
}

impl FixSupport {
    pub(crate) const fn note(self) -> Option<&'static str> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(HINT_FIXABLE_WITH_FIX),
            Self::PubUse => Some(HINT_FIXABLE_WITH_FIX_PUB_USE),
        }
    }

    pub(crate) const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(FixSummaryBucket::Fix),
            Self::PubUse => Some(FixSummaryBucket::PubUse),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy)]
enum DetailMode {
    None,
    MessageRelatedAndFix,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DiagnosticSpec {
    pub headline:    &'static str,
    pub inline_help: Option<&'static str>,
    pub help_anchor: &'static str,
    detail_mode:     DetailMode,
    pub fixability:  FixSupport,
}

static FORBIDDEN_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    "use of `pub(crate)` is forbidden by policy",
    inline_help: None,
    help_anchor: "forbidden-pub-crate",
    detail_mode: DetailMode::None,
    fixability:  FixSupport::None,
};
static FORBIDDEN_PUB_IN_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    "use of `pub(in crate::...)` is forbidden by policy",
    inline_help: None,
    help_anchor: "forbidden-pub-in-crate",
    detail_mode: DetailMode::None,
    fixability:  FixSupport::None,
};
static REVIEW_PUB_MOD: DiagnosticSpec = DiagnosticSpec {
    headline:    "`pub mod` requires explicit review or allowlisting",
    inline_help: None,
    help_anchor: "review-pub-mod",
    detail_mode: DetailMode::None,
    fixability:  FixSupport::None,
};
static SUSPICIOUS_PUB: DiagnosticSpec = DiagnosticSpec {
    headline:    "`pub` is broader than this nested module boundary",
    inline_help: Some("consider using: `pub(super)`"),
    help_anchor: "suspicious-pub",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::None,
};
static UNUSED_PUB: DiagnosticSpec = DiagnosticSpec {
    headline:    "`pub` item is not used outside its defining module",
    inline_help: Some("consider removing `pub`"),
    help_anchor: "unused-pub",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::UnusedPub,
};
static PREFER_MODULE_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    "function import should use module-qualified form",
    inline_help: None,
    help_anchor: "prefer-module-import",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::PreferModuleImport,
};
static INLINE_PATH_QUALIFIED_TYPE: DiagnosticSpec = DiagnosticSpec {
    headline:    "inline path-qualified type should use a `use` import",
    inline_help: None,
    help_anchor: "inline-path-qualified-type",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::InlinePathQualifiedType,
};
static SHORTEN_LOCAL_CRATE_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    "crate-relative import can be shortened to a local-relative import",
    inline_help: None,
    help_anchor: "shorten-local-crate-import",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::ShortenImport,
};
static REPLACE_DEEP_SUPER_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    "deep `super::` chain should use a `crate::` path",
    inline_help: None,
    help_anchor: "replace-deep-super-import",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::ShortenImport,
};
static WILDCARD_PARENT_PUB_USE: DiagnosticSpec = DiagnosticSpec {
    headline:    "parent module `pub use *` should be explicit",
    inline_help: Some("consider re-exporting explicit items instead of `*`"),
    help_anchor: "wildcard-parent-pub-use",
    detail_mode: DetailMode::None,
    fixability:  FixSupport::None,
};
static INTERNAL_PARENT_PUB_USE_FACADE: DiagnosticSpec = DiagnosticSpec {
    headline:    "parent module `pub use` is acting as an internal facade",
    inline_help: Some(
        "consider removing this parent facade and importing the item from its defining child module",
    ),
    help_anchor: "internal-parent-pub-use-facade",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::InternalParentFacade,
};
static NARROW_TO_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    "`pub` exceeds the item's effective reach — use `pub(crate)`",
    inline_help: Some("consider using: `pub(crate)`"),
    help_anchor: "narrow-to-pub-crate",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::NarrowToPubCrate,
};
static FIELD_VISIBILITY_WIDER_THAN_TYPE: DiagnosticSpec = DiagnosticSpec {
    headline:    "field visibility is wider than its containing type",
    inline_help: None,
    help_anchor: "field-visibility-wider-than-type",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::FieldVisibility,
};
static IMPORTS_AT_TOP: DiagnosticSpec = DiagnosticSpec {
    headline:    "`use` statement should live at the top of the file or inline module",
    inline_help: None,
    help_anchor: "imports-at-top",
    detail_mode: DetailMode::MessageRelatedAndFix,
    fixability:  FixSupport::ImportsAtTop,
};

pub(crate) fn diagnostic_spec(code: DiagnosticCode) -> &'static DiagnosticSpec {
    match code {
        DiagnosticCode::ForbiddenPubCrate => &FORBIDDEN_PUB_CRATE,
        DiagnosticCode::ForbiddenPubInCrate => &FORBIDDEN_PUB_IN_CRATE,
        DiagnosticCode::ReviewPubMod => &REVIEW_PUB_MOD,
        DiagnosticCode::SuspiciousPub => &SUSPICIOUS_PUB,
        DiagnosticCode::UnusedPub => &UNUSED_PUB,
        DiagnosticCode::PreferModuleImport => &PREFER_MODULE_IMPORT,
        DiagnosticCode::InlinePathQualifiedType => &INLINE_PATH_QUALIFIED_TYPE,
        DiagnosticCode::ShortenLocalCrateImport => &SHORTEN_LOCAL_CRATE_IMPORT,
        DiagnosticCode::ReplaceDeepSuperImport => &REPLACE_DEEP_SUPER_IMPORT,
        DiagnosticCode::WildcardParentPubUse => &WILDCARD_PARENT_PUB_USE,
        DiagnosticCode::InternalParentPubUseFacade => &INTERNAL_PARENT_PUB_USE_FACADE,
        DiagnosticCode::NarrowToPubCrate => &NARROW_TO_PUB_CRATE,
        DiagnosticCode::FieldVisibilityWiderThanType => &FIELD_VISIBILITY_WIDER_THAN_TYPE,
        DiagnosticCode::ImportsAtTop => &IMPORTS_AT_TOP,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Finding {
    pub severity:        Severity,
    pub diagnostic_code: DiagnosticCode,
    pub path:            String,
    pub line:            usize,
    pub column:          usize,
    pub highlight_len:   usize,
    pub source_line:     String,
    pub item:            Option<String>,
    pub message:         String,
    pub suggestion:      Option<String>,
    #[serde(default)]
    pub fixability:      FixSupport,
    #[serde(default)]
    pub related:         Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Report {
    pub root:     String,
    pub summary:  ReportSummary,
    pub findings: Vec<Finding>,
    #[serde(default)]
    pub facts:    ReportFacts,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct ReportSummary {
    #[serde(rename = "error_count")]
    pub errors:                   usize,
    #[serde(rename = "warning_count")]
    pub warnings:                 usize,
    #[serde(rename = "fixable_with_fix_count")]
    pub fixable_with_fix:         usize,
    #[serde(rename = "fixable_with_fix_pub_use_count")]
    pub fixable_with_fix_pub_use: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct ReportFacts {
    #[serde(default)]
    pub pub_use:           PubUseFixFacts,
    #[serde(default)]
    pub compiler_warnings: CompilerWarningFacts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PubUseFixFact {
    pub child_path:      String,
    pub child_line:      usize,
    pub child_item_name: String,
    pub parent_path:     String,
    pub parent_line:     usize,
    pub child_module:    String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PubUseFixFacts {
    #[serde(default)]
    facts: Vec<PubUseFixFact>,
}

impl From<Vec<PubUseFixFact>> for PubUseFixFacts {
    fn from(facts: Vec<PubUseFixFact>) -> Self { Self { facts } }
}

impl PubUseFixFacts {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &PubUseFixFact> { self.facts.iter() }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CompilerWarningFacts {
    #[default]
    None,
    UnusedImportWarnings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildOutcome {
    Failed,
    Succeeded,
}

impl BuildOutcome {
    pub(crate) const fn is_success(self) -> bool { matches!(self, Self::Succeeded) }
}

impl Serialize for BuildOutcome {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bool(self.is_success())
    }
}

impl Report {
    pub(crate) const fn outcome(&self) -> BuildOutcome {
        if self.summary.errors > 0 {
            BuildOutcome::Failed
        } else {
            BuildOutcome::Succeeded
        }
    }

    pub(crate) const fn has_warnings(&self) -> bool { self.summary.warnings > 0 }

    pub(crate) fn refresh_summary(&mut self) {
        self.summary = ReportSummary {
            errors:                   self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count(),
            warnings:                 self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .count(),
            fixable_with_fix:         self
                .findings
                .iter()
                .filter(|f| effective_fixability(f).summary_bucket() == Some(FixSummaryBucket::Fix))
                .count(),
            fixable_with_fix_pub_use: self
                .findings
                .iter()
                .filter(|f| {
                    effective_fixability(f).summary_bucket() == Some(FixSummaryBucket::PubUse)
                })
                .count(),
        };
    }
}

pub(crate) fn effective_fixability(finding: &Finding) -> FixSupport {
    if matches!(finding.fixability, FixSupport::None) {
        diagnostic_spec(finding.diagnostic_code).fixability
    } else {
        finding.fixability
    }
}

pub(crate) fn finding_headline(finding: &Finding) -> String {
    diagnostic_spec(finding.diagnostic_code)
        .headline
        .to_string()
}

pub(crate) fn detail_reasons(finding: &Finding) -> Vec<String> {
    match diagnostic_spec(finding.diagnostic_code).detail_mode {
        DetailMode::None => Vec::new(),
        DetailMode::MessageRelatedAndFix => {
            let mut reasons = Vec::new();
            if !finding.message.is_empty() {
                reasons.push(finding.message.clone());
            }
            if let Some(related) = &finding.related {
                reasons.push(related.clone());
            }
            if let Some(note) = effective_fixability(finding).note() {
                reasons.push(note.to_string());
            }
            reasons
        },
    }
}

pub(crate) fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(finding.diagnostic_code).inline_help
}

pub(crate) fn custom_inline_help_text(finding: &Finding) -> Option<&str> {
    finding.suggestion.as_deref()
}

pub(crate) fn finding_help_url(finding: &Finding) -> String {
    format!(
        "{HELP_URL_BASE}#{}",
        diagnostic_spec(finding.diagnostic_code).help_anchor
    )
}
