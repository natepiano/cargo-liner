use serde::Deserialize;

use super::cargo_mend_tests_support::DiagnosticCode;
use super::cargo_mend_tests_support::FixSupport;

#[derive(Debug, Deserialize)]
pub struct Finding {
    pub code:        DiagnosticCode,
    #[serde(default)]
    pub path:        String,
    #[serde(default)]
    pub item:        Option<String>,
    #[serde(default)]
    pub fix_support: FixSupport,
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
