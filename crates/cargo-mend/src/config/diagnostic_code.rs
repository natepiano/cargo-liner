use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticCode {
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
    pub(crate) const ALL: &[Self] = &[
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

    pub(crate) const fn as_str(self) -> &'static str {
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
