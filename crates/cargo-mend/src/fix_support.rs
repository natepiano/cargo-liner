use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FixSupport {
    #[default]
    None,
    ShortenImport,
    FixPubUse,
    NeedsManualPubUseCleanup,
    InternalParentFacade,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixSummaryBucket {
    Fix,
    FixPubUse,
}

impl FixSupport {
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport => Some("this warning is auto-fixable with `cargo mend --fix`"),
            Self::FixPubUse => Some("this warning is auto-fixable with `cargo mend --fix-pub-use`"),
        }
    }

    pub const fn summary_bucket(self) -> Option<FixSummaryBucket> {
        match self {
            Self::None | Self::NeedsManualPubUseCleanup | Self::InternalParentFacade => None,
            Self::ShortenImport => Some(FixSummaryBucket::Fix),
            Self::FixPubUse => Some(FixSummaryBucket::FixPubUse),
        }
    }
}
