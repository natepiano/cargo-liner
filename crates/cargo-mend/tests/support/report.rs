use serde::Deserialize;

use super::DiagnosticCode;
use super::FixSupport;

#[derive(Debug, Deserialize)]
pub struct Finding {
    pub code:        DiagnosticCode,
    #[serde(default)]
    pub headline:    String,
    #[serde(default)]
    pub path:        String,
    #[serde(default)]
    pub line_start:  usize,
    #[serde(default)]
    pub item:        Option<String>,
    /// `FixSupport::PubUse` when the diagnostic advertised the `--fix-pub-use`
    /// route, `FixSupport::None` otherwise — including for a finding the tool
    /// offers to plain `--fix`, whose note names no variant. Assert fixability
    /// through `AdvertisedFix`, which reads the note itself.
    #[serde(default, rename = "fixability")]
    pub fix_support: FixSupport,
    /// Child help/note messages attached to the diagnostic (the rendered
    /// suggestion lines), captured so tests can assert on suggestion wording.
    #[serde(default)]
    pub help:        Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Report {
    pub summary:  Summary,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
pub struct Summary {
    #[serde(rename = "error_count")]
    pub errors:                   usize,
    #[serde(rename = "warning_count")]
    pub warnings:                 usize,
    #[serde(rename = "fixable_with_fix_count")]
    pub fixable_with_fix:         usize,
    #[serde(rename = "fixable_with_fix_pub_use_count")]
    pub fixable_with_fix_pub_use: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct ExpectedFinding {
    pub code:        DiagnosticCode,
    pub fix_support: FixSupport,
}
