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
    FixPubUse,
    NeedsManualPubUseCleanup,
    InternalParentFacade,
    NarrowToPubCrate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FixSummaryBucket {
    Fix,
    FixPubUse,
}

impl FixSupport {
    pub(crate) const fn note(self) -> Option<&'static str> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::NarrowToPubCrate => {
                Some("this warning is auto-fixable with `cargo mend --fix`")
            },
            Self::FixPubUse => Some("this warning is auto-fixable with `cargo mend --fix-pub-use`"),
        }
    }

    pub(crate) const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport
            | Self::PreferModuleImport
            | Self::InlinePathQualifiedType
            | Self::NarrowToPubCrate => Some(FixSummaryBucket::Fix),
            Self::FixPubUse => Some(FixSummaryBucket::FixPubUse),
        }
    }
}
