use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct DiagnosticSpec {
    pub(super) code:        &'static str,
    pub(super) headline:    &'static str,
    pub(super) inline_help: Option<&'static str>,
    pub(super) help_anchor: &'static str,
}

pub(super) const DIAGNOSTICS: &[DiagnosticSpec] = &[
    DiagnosticSpec {
        code:        "forbidden_pub_crate",
        headline:    "use of `pub(crate)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-crate",
    },
    DiagnosticSpec {
        code:        "forbidden_pub_in_crate",
        headline:    "use of `pub(in crate::...)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-in-crate",
    },
    DiagnosticSpec {
        code:        "review_pub_mod",
        headline:    "`pub mod` requires explicit review or allowlisting",
        inline_help: None,
        help_anchor: "review-pub-mod",
    },
    DiagnosticSpec {
        code:        "shorten_local_crate_import",
        headline:    "crate-relative import can be shortened to a local-relative import",
        inline_help: None,
        help_anchor: "shorten-local-crate-import",
    },
    DiagnosticSpec {
        code:        "suspicious_pub",
        headline:    "`pub` is broader than this nested module boundary",
        inline_help: Some("consider using: `pub(super)`"),
        help_anchor: "suspicious-pub",
    },
    DiagnosticSpec {
        code:        "unnecessary_parent_pub_use",
        headline:    "parent `pub use` is not used outside its module subtree",
        inline_help: Some(
            "consider removing this `pub use` and narrowing the child item with `pub(super)`",
        ),
        help_anchor: "unnecessary-parent-pub-use",
    },
];

pub(super) fn diagnostic_spec(code: &str) -> &'static DiagnosticSpec {
    DIAGNOSTICS
        .iter()
        .find(|spec| spec.code == code)
        .unwrap_or_else(|| panic!("unknown diagnostic code: {code}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Finding {
    pub(super) severity:      Severity,
    pub(super) code:          String,
    pub(super) path:          String,
    pub(super) line:          usize,
    pub(super) column:        usize,
    pub(super) highlight_len: usize,
    pub(super) source_line:   String,
    pub(super) item:          Option<String>,
    pub(super) message:       String,
    pub(super) suggestion:    Option<String>,
    #[serde(default)]
    pub(super) related:       Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct Report {
    pub(super) root:     String,
    pub(super) summary:  ReportSummary,
    pub(super) findings: Vec<Finding>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ReportSummary {
    pub(super) error_count:   usize,
    pub(super) warning_count: usize,
    pub(super) fixable_count: usize,
}

impl Report {
    pub(super) fn has_errors(&self) -> bool { self.summary.error_count > 0 }

    pub(super) fn has_warnings(&self) -> bool { self.summary.warning_count > 0 }

    pub(super) fn refresh_summary(&mut self) {
        self.summary = ReportSummary {
            error_count:   self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count(),
            warning_count: self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .count(),
            fixable_count: self
                .findings
                .iter()
                .filter(|f| f.code == "shorten_local_crate_import")
                .count(),
        };
    }
}

pub(super) fn finding_headline(finding: &Finding) -> String {
    diagnostic_spec(&finding.code).headline.to_string()
}

pub(super) fn detail_reasons(finding: &Finding) -> Vec<String> {
    match finding.code.as_str() {
        "suspicious_pub" => {
            let mut reasons = Vec::new();
            if !finding.message.is_empty() {
                reasons.push(finding.message.clone());
            }
            if let Some(related) = &finding.related {
                reasons.push(related.clone());
            }
            reasons
        },
        "unnecessary_parent_pub_use" => {
            let mut reasons = Vec::new();
            if !finding.message.is_empty() {
                reasons.push(finding.message.clone());
            }
            if let Some(related) = &finding.related {
                reasons.push(related.clone());
            }
            reasons
        },
        "shorten_local_crate_import" => {
            if finding.message.is_empty() {
                vec!["this warning is auto-fixable with `cargo vischeck --fix`".to_string()]
            } else {
                vec![
                    finding.message.clone(),
                    "this warning is auto-fixable with `cargo vischeck --fix`".to_string(),
                ]
            }
        },
        _ => Vec::new(),
    }
}

pub(super) fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(&finding.code).inline_help
}

pub(super) fn custom_inline_help_text(finding: &Finding) -> Option<&str> {
    finding.suggestion.as_deref()
}

pub(super) fn finding_help_url(finding: &Finding) -> String {
    format!(
        "https://github.com/natepiano/cargo-vischeck#{}",
        diagnostic_spec(&finding.code).help_anchor
    )
}
