use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
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
        code:        "suspicious_bare_pub",
        headline:    "bare `pub` is not publicly re-exported by its parent module",
        inline_help: Some("consider using: `pub(super)`"),
        help_anchor: "suspicious-bare-pub",
    },
];

pub(super) fn diagnostic_spec(code: &str) -> &'static DiagnosticSpec {
    DIAGNOSTICS
        .iter()
        .find(|spec| spec.code == code)
        .unwrap_or_else(|| panic!("unknown diagnostic code: {code}"))
}

#[derive(Debug, Clone, Serialize)]
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
}

#[derive(Debug, Default, Serialize)]
pub(super) struct Report {
    pub(super) root:     String,
    pub(super) findings: Vec<Finding>,
}

impl Report {
    pub(super) fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.severity == Severity::Error)
    }

    pub(super) fn has_warnings(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity == Severity::Warning)
    }
}

pub(super) fn finding_headline(finding: &Finding) -> String {
    diagnostic_spec(&finding.code).headline.to_string()
}

pub(super) fn detail_reasons(finding: &Finding) -> Vec<String> {
    match finding.code.as_str() {
        "suspicious_bare_pub" => {
            let reasons = split_message(&finding.message);
            if reasons
                .iter()
                .any(|reason| reason == "appears unused outside its defining file")
            {
                vec!["it appears unused outside its defining file".to_string()]
            } else {
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

pub(super) fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(&finding.code).inline_help
}

pub(super) fn finding_help_url(finding: &Finding) -> String {
    format!(
        "https://github.com/natepiano/cargo-vischeck#{}",
        diagnostic_spec(&finding.code).help_anchor
    )
}

fn split_message(message: &str) -> Vec<String> {
    message
        .split(", and ")
        .flat_map(|part| part.split("; "))
        .flat_map(|part| part.split(", "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}
