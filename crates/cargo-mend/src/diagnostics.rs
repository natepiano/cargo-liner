use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixKind {
    None,
    ImportRewrite,
    ParentPubUse,
}

impl FixKind {
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::ImportRewrite => Some("this warning is auto-fixable with `cargo mend --fix`"),
            Self::ParentPubUse => {
                Some("this warning is auto-fixable with `cargo mend --fix-pub-use`")
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum DetailMode {
    None,
    MessageRelatedAndFix(FixKind),
}

#[derive(Debug, Clone, Copy)]
pub struct DiagnosticSpec {
    pub code:        &'static str,
    pub headline:    &'static str,
    pub inline_help: Option<&'static str>,
    pub help_anchor: &'static str,
    detail_mode:     DetailMode,
    pub fix_kind:    FixKind,
}

pub const DIAGNOSTICS: &[DiagnosticSpec] = &[
    DiagnosticSpec {
        code:        "forbidden_pub_crate",
        headline:    "use of `pub(crate)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-crate",
        detail_mode: DetailMode::None,
        fix_kind:    FixKind::None,
    },
    DiagnosticSpec {
        code:        "forbidden_pub_in_crate",
        headline:    "use of `pub(in crate::...)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-in-crate",
        detail_mode: DetailMode::None,
        fix_kind:    FixKind::None,
    },
    DiagnosticSpec {
        code:        "review_pub_mod",
        headline:    "`pub mod` requires explicit review or allowlisting",
        inline_help: None,
        help_anchor: "review-pub-mod",
        detail_mode: DetailMode::None,
        fix_kind:    FixKind::None,
    },
    DiagnosticSpec {
        code:        "shorten_local_crate_import",
        headline:    "crate-relative import can be shortened to a local-relative import",
        inline_help: None,
        help_anchor: "shorten-local-crate-import",
        detail_mode: DetailMode::MessageRelatedAndFix(FixKind::ImportRewrite),
        fix_kind:    FixKind::ImportRewrite,
    },
    DiagnosticSpec {
        code:        "wildcard_parent_pub_use",
        headline:    "parent module `pub use *` should be explicit",
        inline_help: Some("consider re-exporting explicit items instead of `*`"),
        help_anchor: "wildcard-parent-pub-use",
        detail_mode: DetailMode::None,
        fix_kind:    FixKind::None,
    },
    DiagnosticSpec {
        code:        "suspicious_pub",
        headline:    "`pub` is broader than this nested module boundary",
        inline_help: Some("consider using: `pub(super)`"),
        help_anchor: "suspicious-pub",
        detail_mode: DetailMode::MessageRelatedAndFix(FixKind::None),
        fix_kind:    FixKind::None,
    },
];

pub fn diagnostic_spec(code: &str) -> &'static DiagnosticSpec {
    let Some(spec) = DIAGNOSTICS.iter().find(|candidate| candidate.code == code) else {
        unreachable!("unknown diagnostic code: {code}");
    };
    spec
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub severity:      Severity,
    pub code:          String,
    pub path:          String,
    pub line:          usize,
    pub column:        usize,
    pub highlight_len: usize,
    pub source_line:   String,
    pub item:          Option<String>,
    pub message:       String,
    pub suggestion:    Option<String>,
    #[serde(default)]
    pub fix_kind:      Option<FixKind>,
    #[serde(default)]
    pub related:       Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub root:     String,
    pub summary:  ReportSummary,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ReportSummary {
    #[serde(rename = "error_count")]
    pub errors:   usize,
    #[serde(rename = "warning_count")]
    pub warnings: usize,
    #[serde(rename = "fixable_count")]
    pub fixable:  usize,
}

impl Report {
    pub const fn has_errors(&self) -> bool { self.summary.errors > 0 }

    pub const fn has_warnings(&self) -> bool { self.summary.warnings > 0 }

    pub(super) fn refresh_summary(&mut self) {
        self.summary = ReportSummary {
            errors:   self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Error)
                .count(),
            warnings: self
                .findings
                .iter()
                .filter(|f| f.severity == Severity::Warning)
                .count(),
            fixable:  self
                .findings
                .iter()
                .filter(|f| effective_fix_kind(f).note().is_some())
                .count(),
        };
    }
}

fn effective_fix_kind(finding: &Finding) -> FixKind {
    finding
        .fix_kind
        .unwrap_or_else(|| diagnostic_spec(&finding.code).fix_kind)
}

pub fn finding_headline(finding: &Finding) -> String {
    diagnostic_spec(&finding.code).headline.to_string()
}

pub fn detail_reasons(finding: &Finding) -> Vec<String> {
    match diagnostic_spec(&finding.code).detail_mode {
        DetailMode::None => Vec::new(),
        DetailMode::MessageRelatedAndFix(default_fix_kind) => {
            let mut reasons = Vec::new();
            if !finding.message.is_empty() {
                reasons.push(finding.message.clone());
            }
            if let Some(related) = &finding.related {
                reasons.push(related.clone());
            }
            if let Some(note) = effective_fix_kind(finding)
                .note()
                .or_else(|| default_fix_kind.note())
            {
                reasons.push(note.to_string());
            }
            reasons
        },
    }
}

pub fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(&finding.code).inline_help
}

pub fn custom_inline_help_text(finding: &Finding) -> Option<&str> { finding.suggestion.as_deref() }

pub fn finding_help_url(finding: &Finding) -> String {
    format!(
        "https://github.com/natepiano/cargo-mend#{}",
        diagnostic_spec(&finding.code).help_anchor
    )
}
