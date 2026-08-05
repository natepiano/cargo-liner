use serde::Deserialize;
use serde::Serialize;
use serde::Serializer;

use super::constants::HINT_ERROR_FIXABLE_WITH_FIX;
use super::constants::HINT_ERROR_FIXABLE_WITH_FIX_PUB_USE;
use super::constants::HINT_WARNING_FIXABLE_WITH_FIX;
use super::constants::HINT_WARNING_FIXABLE_WITH_FIX_PUB_USE;
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
    RestrictedAnnotation,
    #[serde(rename = "fix_field_visibility")]
    FieldVisibility,
    #[serde(rename = "fix_imports_at_top")]
    ImportsAtTop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixSummaryBucket {
    Standard,
    PubUse,
}

impl FixSupport {
    const fn note(self, severity: Severity) -> Option<&'static str> {
        match (severity, self.summary_bucket()) {
            (_, None) => None,
            (Severity::Warning, Some(FixSummaryBucket::Standard)) => {
                Some(HINT_WARNING_FIXABLE_WITH_FIX)
            },
            (Severity::Warning, Some(FixSummaryBucket::PubUse)) => {
                Some(HINT_WARNING_FIXABLE_WITH_FIX_PUB_USE)
            },
            (Severity::Error, Some(FixSummaryBucket::Standard)) => {
                Some(HINT_ERROR_FIXABLE_WITH_FIX)
            },
            (Severity::Error, Some(FixSummaryBucket::PubUse)) => {
                Some(HINT_ERROR_FIXABLE_WITH_FIX_PUB_USE)
            },
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
            | Self::RestrictedAnnotation
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(FixSummaryBucket::Standard),
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
    MessageAndRelated,
}

#[derive(Debug, Clone, Copy)]
enum HeadlineSource {
    Static(&'static str),
    FindingMessage { fallback: &'static str },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DiagnosticSpec {
    headline:        HeadlineSource,
    pub inline_help: Option<&'static str>,
    pub help_anchor: &'static str,
    detail_mode:     DetailMode,
    pub fix_support: FixSupport,
}

static FORBIDDEN_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::FindingMessage {
        fallback: "use of `pub(crate)` is forbidden by policy",
    },
    inline_help: None,
    help_anchor: "forbidden-pub-crate",
    detail_mode: DetailMode::None,
    fix_support: FixSupport::None,
};
static FORBIDDEN_PUB_IN_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::FindingMessage {
        fallback: "use of `pub(in crate::...)` is forbidden by policy",
    },
    inline_help: None,
    help_anchor: "forbidden-pub-in-crate",
    detail_mode: DetailMode::None,
    fix_support: FixSupport::None,
};
static REVIEW_PUB_MOD: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("`pub mod` requires explicit review or allowlisting"),
    inline_help: None,
    help_anchor: "review-pub-mod",
    detail_mode: DetailMode::None,
    fix_support: FixSupport::None,
};
static SUSPICIOUS_PUB: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("`pub` is broader than this nested module boundary"),
    inline_help: None,
    help_anchor: "suspicious-pub",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::None,
};
static UNUSED_PUB: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("`pub` item is not used outside its defining module"),
    inline_help: Some("consider removing `pub`"),
    help_anchor: "unused-pub",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::UnusedPub,
};
static PREFER_MODULE_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("function import should use module-qualified form"),
    inline_help: None,
    help_anchor: "prefer-module-import",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::PreferModuleImport,
};
static INLINE_PATH_QUALIFIED_TYPE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("inline path-qualified type should use a `use` import"),
    inline_help: None,
    help_anchor: "inline-path-qualified-type",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::InlinePathQualifiedType,
};
static SHORTEN_LOCAL_CRATE_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static(
        "crate-relative import can be shortened to a local-relative import",
    ),
    inline_help: None,
    help_anchor: "shorten-local-crate-import",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::ShortenImport,
};
static REPLACE_DEEP_SUPER_IMPORT: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("deep `super::` chain should use a `crate::` path"),
    inline_help: None,
    help_anchor: "replace-deep-super-import",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::ShortenImport,
};
static WILDCARD_PARENT_PUB_USE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("parent module `pub use *` should be explicit"),
    inline_help: Some("consider re-exporting explicit items instead of `*`"),
    help_anchor: "wildcard-parent-pub-use",
    detail_mode: DetailMode::None,
    fix_support: FixSupport::None,
};
static INTERNAL_PARENT_PUB_USE_FACADE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::FindingMessage {
        fallback: "parent module re-export is acting as an internal facade",
    },
    inline_help: Some(
        "consider removing this parent facade and importing the item from its defining child module",
    ),
    help_anchor: "internal-parent-pub-use-facade",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::InternalParentFacade,
};
static NARROW_TO_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static(
        "`pub` exceeds the item's effective reach — use `pub(crate)`",
    ),
    inline_help: Some("consider using: `pub(crate)`"),
    help_anchor: "narrow-to-pub-crate",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::NarrowToPubCrate,
};
static FIELD_VISIBILITY_WIDER_THAN_TYPE: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static("field visibility is wider than its containing type"),
    inline_help: None,
    help_anchor: "field-visibility-wider-than-type",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::FieldVisibility,
};
static IMPORTS_AT_TOP: DiagnosticSpec = DiagnosticSpec {
    headline:    HeadlineSource::Static(
        "`use` statement should live at the top of the file or inline module",
    ),
    inline_help: None,
    help_anchor: "imports-at-top",
    detail_mode: DetailMode::MessageAndRelated,
    fix_support: FixSupport::ImportsAtTop,
};

/// The visibility annotation the compiler pass read off the item this finding
/// is about. Fixers rewrite [`Self::Bare`] and specifically approved
/// [`Self::Restricted`] annotations. [`Self::Unknown`] means the pass captured
/// no annotation text, so there is nothing for a fixer to verify its edit
/// against.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum WrittenVisibility {
    #[default]
    Unknown,
    Bare,
    /// A `pub(crate)`, `pub(super)`, `pub(self)`, or `pub(in <path>)`
    /// annotation, held as the exact source text — which may span lines.
    Restricted(String),
}

impl From<Option<String>> for WrittenVisibility {
    fn from(visibility_annotation: Option<String>) -> Self {
        match visibility_annotation {
            None => Self::Unknown,
            Some(source) if source == "pub" => Self::Bare,
            Some(source) => Self::Restricted(source),
        }
    }
}

/// The scope a finding proposes narrowing its item to, and what a fixer is
/// allowed to do with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) enum NarrowerScope {
    /// The finding proposes no narrowing.
    #[default]
    Unproposed,
    /// Canonical def-path of the item's enclosing module, recorded so
    /// cross-compilation merge can drop the finding when a caller lives outside
    /// that module. It is a suppression key, not an annotation to write.
    SuppressionKey(String),
    /// Canonical def-path of the exact module boundary the item needs, without
    /// the leading `crate::`. `pub(in crate::<path>)` built from this is a
    /// correct rewrite of the item's bare `pub`.
    ExactBoundary(String),
}

impl NarrowerScope {
    /// Rebuilds the scope from the two stored fields that carry it. The
    /// def-path alone cannot say which kind of scope it is; the compiler pass
    /// records that by attaching [`FixSupport::RestrictedAnnotation`] to the
    /// same finding, which it does only for a resolved exact boundary.
    pub(crate) fn resolve(def_path: Option<String>, fix_support: FixSupport) -> Self {
        match (def_path, fix_support) {
            (None, _) => Self::Unproposed,
            (Some(def_path), FixSupport::RestrictedAnnotation) => Self::ExactBoundary(def_path),
            (Some(def_path), _) => Self::SuppressionKey(def_path),
        }
    }
}

/// What the compiler pass resolved about the visibility of the item a finding
/// is about. Findings about import syntax carry the [`Default`] value, which
/// says exactly that: no annotation was read and no narrowing was proposed.
#[derive(Debug, Clone, Default)]
pub(crate) struct ItemVisibility {
    pub written:        WrittenVisibility,
    pub narrower_scope: NarrowerScope,
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
    #[serde(default, rename = "fixability")]
    pub fix_support:     FixSupport,
    #[serde(default)]
    pub related:         Option<String>,
    /// Skipped by serde on purpose: the fixers need this in-process and nothing
    /// outside cargo-mend reads it, so publishing it would create a JSON
    /// contract with no consumer.
    #[serde(skip)]
    pub item_visibility: ItemVisibility,
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
    #[serde(rename = "pub_use")]
    pub pub_use_fix_facts:      PubUseFixFacts,
    #[serde(default)]
    pub all_features_coverage:  AllFeaturesCoverage,
    #[serde(default, rename = "compiler_warnings")]
    pub compiler_warning_facts: CompilerWarningFacts,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AllFeaturesCoverage {
    #[default]
    NotGuaranteed,
    Superset,
}

impl AllFeaturesCoverage {
    pub(crate) const fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Superset, Self::Superset) => Self::Superset,
            (Self::NotGuaranteed, _) | (_, Self::NotGuaranteed) => Self::NotGuaranteed,
        }
    }
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

impl PubUseFixFacts {
    pub(crate) fn iter(&self) -> impl Iterator<Item = &PubUseFixFact> { self.facts.iter() }
}

impl From<Vec<PubUseFixFact>> for PubUseFixFacts {
    fn from(facts: Vec<PubUseFixFact>) -> Self { Self { facts } }
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
                .filter(|f| {
                    effective_fixability(f).summary_bucket() == Some(FixSummaryBucket::Standard)
                })
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

pub(crate) fn effective_fixability(finding: &Finding) -> FixSupport {
    if matches!(finding.fix_support, FixSupport::None) {
        diagnostic_spec(finding.diagnostic_code).fix_support
    } else {
        finding.fix_support
    }
}

pub(crate) fn fixability_note(finding: &Finding) -> Option<&'static str> {
    effective_fixability(finding).note(finding.severity)
}

pub(crate) fn finding_headline(finding: &Finding) -> String {
    match diagnostic_spec(finding.diagnostic_code).headline {
        HeadlineSource::Static(headline) => headline.to_string(),
        HeadlineSource::FindingMessage { fallback } => {
            if finding.message.is_empty() {
                fallback.to_string()
            } else {
                finding.message.clone()
            }
        },
    }
}

pub(crate) fn finding_message_not_in_headline(finding: &Finding) -> Option<&str> {
    if finding.message.is_empty()
        || matches!(
            diagnostic_spec(finding.diagnostic_code).headline,
            HeadlineSource::FindingMessage { .. }
        )
    {
        None
    } else {
        Some(&finding.message)
    }
}

pub(crate) fn detail_reasons(finding: &Finding) -> Vec<String> {
    let mut reasons = match diagnostic_spec(finding.diagnostic_code).detail_mode {
        DetailMode::None => Vec::new(),
        DetailMode::MessageAndRelated => {
            let mut reasons = Vec::new();
            if let Some(message) = finding_message_not_in_headline(finding) {
                reasons.push(message.to_string());
            }
            if let Some(related) = &finding.related {
                reasons.push(related.clone());
            }
            reasons
        },
    };
    if let Some(note) = fixability_note(finding) {
        reasons.push(note.to_string());
    }
    reasons
}

pub(crate) fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(finding.diagnostic_code).inline_help
}

pub(crate) fn custom_inline_help_text(finding: &Finding) -> Option<&str> {
    finding.suggestion.as_deref()
}

pub(crate) fn resolved_inline_help_text(finding: &Finding) -> Option<&str> {
    custom_inline_help_text(finding).or_else(|| inline_help_text(finding))
}

pub(crate) fn finding_help_url(finding: &Finding) -> String {
    format!(
        "{HELP_URL_BASE}#{}",
        diagnostic_spec(finding.diagnostic_code).help_anchor
    )
}

#[cfg(test)]
mod tests {
    use super::Finding;
    use super::FixSupport;
    use super::ItemVisibility;
    use super::Severity;
    use super::finding_headline;
    use crate::config::DiagnosticCode;

    #[test]
    fn finding_message_headlines_use_messages_with_static_fallbacks() {
        for (diagnostic_code, fallback) in [
            (
                DiagnosticCode::ForbiddenPubCrate,
                "use of `pub(crate)` is forbidden by policy",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "use of `pub(in crate::...)` is forbidden by policy",
            ),
            (
                DiagnosticCode::InternalParentPubUseFacade,
                "parent module re-export is acting as an internal facade",
            ),
        ] {
            let mut finding = Finding {
                severity: Severity::Error,
                diagnostic_code,
                path: "src/lib.rs".to_string(),
                line: 1,
                column: 1,
                highlight_len: 1,
                source_line: "pub(crate) struct Example;".to_string(),
                item: Some("Example".to_string()),
                message: "custom forbidden visibility outcome".to_string(),
                suggestion: None,
                fix_support: FixSupport::None,
                related: None,
                item_visibility: ItemVisibility::default(),
            };

            assert_eq!(
                finding_headline(&finding),
                "custom forbidden visibility outcome"
            );

            finding.message.clear();
            assert_eq!(finding_headline(&finding), fallback);
        }

        for (diagnostic_code, message) in [
            (
                DiagnosticCode::ForbiddenPubCrate,
                "use of `pub(crate)` does not match the parent facade boundary",
            ),
            (
                DiagnosticCode::ForbiddenPubCrate,
                "`pub(in crate)` is a redundant spelling of `pub(crate)`",
            ),
            (
                DiagnosticCode::ForbiddenPubCrate,
                "`pub(in crate)` is wider than the exact parent facade boundary",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "parent facade caps reach at `pub(crate)`",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "use of `pub(in crate::video_plane)` is disabled by project visibility policy",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "parent facade does not provide a resolvable visibility boundary",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "use of `pub(in crate::a)` outside an exact facade boundary is forbidden by policy",
            ),
            (
                DiagnosticCode::ForbiddenPubInCrate,
                "no policy-allowed visibility keeps this item reachable where it is used: private and `pub(super)` are too narrow, and no facade caps `pub`",
            ),
        ] {
            let finding = Finding {
                severity: Severity::Error,
                diagnostic_code,
                path: "src/lib.rs".to_string(),
                line: 1,
                column: 1,
                highlight_len: 1,
                source_line: "pub(crate) struct Example;".to_string(),
                item: Some("Example".to_string()),
                message: message.to_string(),
                suggestion: None,
                fix_support: FixSupport::None,
                related: None,
                item_visibility: ItemVisibility::default(),
            };

            assert_eq!(finding_headline(&finding), message);
        }
    }
}
