use serde::Deserialize;

use super::DiagnosticCode;
use super::FixSupport;

#[derive(Debug, Deserialize)]
pub struct Finding {
    pub code:        DiagnosticCode,
    #[serde(default)]
    pub path:        String,
    #[serde(default)]
    pub item:        Option<String>,
    #[serde(default)]
    pub fix_support: FixSupport,
    /// Child help/note messages attached to the diagnostic (the rendered
    /// suggestion lines), captured so tests can assert on suggestion wording.
    #[serde(default)]
    pub help:        Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Report {
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
pub struct ExpectedFinding {
    pub code:        DiagnosticCode,
    pub fix_support: FixSupport,
}
