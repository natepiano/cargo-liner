use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    ForbiddenPubCrate,
    ForbiddenPubInCrate,
    ReviewPubMod,
    SuspiciousPub,
    UnusedPub,
    PreferModuleImport,
    InlinePathQualifiedType,
    ShortenLocalCrateImport,
    ReplaceDeepSuperImport,
    WildcardParentPubUse,
    InternalParentPubUseFacade,
    NarrowToPubCrate,
    FieldVisibilityWiderThanType,
    ImportsAtTop,
}

impl DiagnosticCode {
    pub const ALL: &[Self] = &[
        Self::ForbiddenPubCrate,
        Self::ForbiddenPubInCrate,
        Self::ReviewPubMod,
        Self::SuspiciousPub,
        Self::UnusedPub,
        Self::PreferModuleImport,
        Self::InlinePathQualifiedType,
        Self::ShortenLocalCrateImport,
        Self::ReplaceDeepSuperImport,
        Self::WildcardParentPubUse,
        Self::InternalParentPubUseFacade,
        Self::NarrowToPubCrate,
        Self::FieldVisibilityWiderThanType,
        Self::ImportsAtTop,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ForbiddenPubCrate => "forbidden_pub_crate",
            Self::ForbiddenPubInCrate => "forbidden_pub_in_crate",
            Self::ReviewPubMod => "review_pub_mod",
            Self::SuspiciousPub => "suspicious_pub",
            Self::UnusedPub => "unused_pub",
            Self::PreferModuleImport => "prefer_module_import",
            Self::InlinePathQualifiedType => "inline_path_qualified_type",
            Self::ShortenLocalCrateImport => "shorten_local_crate_import",
            Self::ReplaceDeepSuperImport => "replace_deep_super_import",
            Self::WildcardParentPubUse => "wildcard_parent_pub_use",
            Self::InternalParentPubUseFacade => "internal_parent_pub_use_facade",
            Self::NarrowToPubCrate => "narrow_to_pub_crate",
            Self::FieldVisibilityWiderThanType => "field_visibility_wider_than_type",
            Self::ImportsAtTop => "imports_at_top",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FixSupport {
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
pub enum FixSummaryBucket {
    Standard,
    PubUse,
}

impl FixSupport {
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some("this warning is auto-fixable with `cargo mend --fix`"),
            Self::PubUse => Some("this warning is auto-fixable with `cargo mend --fix-pub-use`"),
        }
    }

    pub const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::UnusedPub
            | Self::NarrowToPubCrate
            | Self::FieldVisibility
            | Self::ImportsAtTop => Some(FixSummaryBucket::Standard),
            Self::PubUse => Some(FixSummaryBucket::PubUse),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticSpec {
    pub headline:    &'static str,
    pub help_anchor: &'static str,
    pub fix_support: FixSupport,
}

pub const fn diagnostic_spec(code: DiagnosticCode) -> &'static DiagnosticSpec {
    const FORBIDDEN_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    "use of `pub(crate)` is forbidden by policy",
        help_anchor: "forbidden-pub-crate",
        fix_support: FixSupport::None,
    };
    const FORBIDDEN_PUB_IN_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    "use of `pub(in crate::...)` is forbidden by policy",
        help_anchor: "forbidden-pub-in-crate",
        fix_support: FixSupport::None,
    };
    const REVIEW_PUB_MOD: DiagnosticSpec = DiagnosticSpec {
        headline:    "`pub mod` requires explicit review or allowlisting",
        help_anchor: "review-pub-mod",
        fix_support: FixSupport::None,
    };
    const SUSPICIOUS_PUB: DiagnosticSpec = DiagnosticSpec {
        headline:    "`pub` is broader than this nested module boundary",
        help_anchor: "suspicious-pub",
        fix_support: FixSupport::None,
    };
    const UNUSED_PUB: DiagnosticSpec = DiagnosticSpec {
        headline:    "`pub` item is not used outside its defining module",
        help_anchor: "unused-pub",
        fix_support: FixSupport::UnusedPub,
    };
    const PREFER_MODULE_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    "function import should use module-qualified form",
        help_anchor: "prefer-module-import",
        fix_support: FixSupport::PreferModuleImport,
    };
    const INLINE_PATH_QUALIFIED_TYPE: DiagnosticSpec = DiagnosticSpec {
        headline:    "inline path-qualified type should use a `use` import",
        help_anchor: "inline-path-qualified-type",
        fix_support: FixSupport::InlinePathQualifiedType,
    };
    const SHORTEN_LOCAL_CRATE_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    "crate-relative import can be shortened to a local-relative import",
        help_anchor: "shorten-local-crate-import",
        fix_support: FixSupport::ShortenImport,
    };
    const REPLACE_DEEP_SUPER_IMPORT: DiagnosticSpec = DiagnosticSpec {
        headline:    "deep `super::` chain should use a `crate::` path",
        help_anchor: "replace-deep-super-import",
        fix_support: FixSupport::ShortenImport,
    };
    const WILDCARD_PARENT_PUB_USE: DiagnosticSpec = DiagnosticSpec {
        headline:    "parent module `pub use *` should be explicit",
        help_anchor: "wildcard-parent-pub-use",
        fix_support: FixSupport::None,
    };
    const INTERNAL_PARENT_PUB_USE_FACADE: DiagnosticSpec = DiagnosticSpec {
        headline:    "parent module `pub use` is acting as an internal facade",
        help_anchor: "internal-parent-pub-use-facade",
        fix_support: FixSupport::InternalParentFacade,
    };
    const NARROW_TO_PUB_CRATE: DiagnosticSpec = DiagnosticSpec {
        headline:    "`pub` exceeds the item's effective reach — use `pub(crate)`",
        help_anchor: "narrow-to-pub-crate",
        fix_support: FixSupport::NarrowToPubCrate,
    };
    const FIELD_VISIBILITY_WIDER_THAN_TYPE: DiagnosticSpec = DiagnosticSpec {
        headline:    "field visibility is wider than its containing type",
        help_anchor: "field-visibility-wider-than-type",
        fix_support: FixSupport::FieldVisibility,
    };
    const IMPORTS_AT_TOP: DiagnosticSpec = DiagnosticSpec {
        headline:    "`use` statement should live at the top of the file or inline module",
        help_anchor: "imports-at-top",
        fix_support: FixSupport::ImportsAtTop,
    };

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
