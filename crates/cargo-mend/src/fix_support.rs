use serde::Deserialize;
use serde::Serialize;

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
    NarrowToPubCrate,
    #[serde(rename = "fix_field_visibility")]
    FieldVisibility,
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
            | Self::NarrowToPubCrate
            | Self::FieldVisibility => Some("this warning is auto-fixable with `cargo mend --fix`"),
            Self::PubUse => Some("this warning is auto-fixable with `cargo mend --fix-pub-use`"),
        }
    }

    pub(crate) const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::NarrowToPubCrate
            | Self::FieldVisibility => Some(FixSummaryBucket::Fix),
            Self::PubUse => Some(FixSummaryBucket::PubUse),
        }
    }
}
