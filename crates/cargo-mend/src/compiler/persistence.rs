use std::ffi::OsStr;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;

use super::settings;
use crate::config::DiagnosticCode;
use crate::constants::FINDINGS_SCHEMA_VERSION;
use crate::diagnostics::CompilerWarningFacts;
use crate::diagnostics::Finding;
use crate::diagnostics::PubUseFixFact;
use crate::diagnostics::Report;
use crate::diagnostics::ReportFacts;
use crate::diagnostics::ReportSummary;
use crate::diagnostics::Severity;
use crate::fix_support::FixSupport;
use crate::selection::Selection;

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredReport {
    pub version:              u32,
    #[serde(default)]
    pub analysis_fingerprint: String,
    #[serde(default)]
    pub scope_fingerprint:    String,
    pub package_root:         String,
    #[serde(default)]
    pub crate_root_file:      String,
    pub config_fingerprint:   String,
    pub findings:             Vec<StoredFinding>,
    #[serde(default)]
    pub pub_use_fix_facts:    Vec<StoredPubUseFixFact>,
    #[serde(default)]
    pub compiler_warnings:    CompilerWarningFacts,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredFinding {
    pub severity:      Severity,
    pub code:          DiagnosticCode,
    pub path:          String,
    pub line:          usize,
    pub column:        usize,
    pub highlight_len: usize,
    pub source_line:   String,
    pub item:          Option<String>,
    pub message:       String,
    pub suggestion:    Option<String>,
    #[serde(default)]
    pub fixability:    FixSupport,
    #[serde(default)]
    pub related:       Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredPubUseFixFact {
    pub child_path:      String,
    pub child_line:      usize,
    pub child_item_name: String,
    pub parent_path:     String,
    pub parent_line:     usize,
    pub child_module:    String,
}

#[derive(Default)]
pub(super) struct FindingsSink {
    pub findings:          Vec<StoredFinding>,
    pub pub_use_fix_facts: Vec<StoredPubUseFixFact>,
}

pub(super) fn prepare_findings_dir(target_directory: &Path) -> Result<PathBuf> {
    let findings_dir = target_directory.join("mend-findings");
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create findings directory {}",
            findings_dir.display()
        )
    })?;
    Ok(findings_dir)
}

pub(super) fn load_report(
    findings_dir: &Path,
    selection: &Selection,
    config_fingerprint: &str,
) -> Result<Report> {
    let selected_roots: Vec<PathBuf> = selection.package_roots.clone();
    let selected_root_strings: Vec<String> = selected_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let selected_canonical_roots: Vec<PathBuf> = selected_roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect();
    let mut findings = Vec::new();
    let mut pub_use_fix_facts = Vec::new();

    for entry in fs::read_dir(findings_dir).with_context(|| {
        format!(
            "failed to read findings directory {}",
            findings_dir.display()
        )
    })? {
        let entry = entry?;
        if entry.path().extension().and_then(OsStr::to_str) != Some("json") {
            continue;
        }

        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read findings file {}", entry.path().display()))?;
        let Ok(stored) = serde_json::from_str::<StoredReport>(&text) else {
            continue;
        };
        if !stored_report_matches_selection(
            &stored,
            &selected_roots,
            &selected_root_strings,
            &selected_canonical_roots,
            config_fingerprint,
        ) {
            continue;
        }
        extend_report_from_stored(
            &mut findings,
            &mut pub_use_fix_facts,
            stored,
            selection.analysis_root.as_path(),
        );
    }

    findings.sort_by(|a, b| {
        (
            a.severity, &a.path, a.line, a.column, &a.code, &a.item, &a.message,
        )
            .cmp(&(
                b.severity, &b.path, b.line, b.column, &b.code, &b.item, &b.message,
            ))
    });
    findings.dedup_by(|a, b| {
        a.severity == b.severity
            && a.code == b.code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
    });

    Ok(Report {
        root: selection_root_string(selection.analysis_root.as_path()),
        summary: ReportSummary::default(),
        findings,
        facts: ReportFacts {
            pub_use:           pub_use_fix_facts.into(),
            compiler_warnings: CompilerWarningFacts::None,
        },
    })
}

fn stored_report_matches_selection(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
    config_fingerprint: &str,
) -> bool {
    // Source-level freshness is delegated to cargo: when sources change,
    // cargo recompiles the target, the wrapper re-runs, and this file is
    // overwritten. Cache reuse therefore must NOT depend on the cargo
    // CLI flags (`--lib`, `--all-targets`, ...); those select targets,
    // not findings, and gating on them would discard valid cached
    // findings whenever cargo skipped a recompile.
    stored.version == FINDINGS_SCHEMA_VERSION
        && stored.analysis_fingerprint == settings::current_analysis_fingerprint()
        && stored.config_fingerprint == config_fingerprint
        && stored_crate_root_exists(stored)
        && stored_matches_selected_root(
            stored,
            selected_roots,
            selected_root_strings,
            selected_canonical_roots,
        )
}

fn stored_crate_root_exists(stored: &StoredReport) -> bool {
    stored.crate_root_file.is_empty() || {
        let crate_root = Path::new(&stored.crate_root_file);
        if crate_root.is_absolute() {
            crate_root.exists()
        } else {
            Path::new(&stored.package_root).join(crate_root).exists()
        }
    }
}

fn stored_matches_selected_root(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
) -> bool {
    selected_root_strings
        .iter()
        .any(|root| root == &stored.package_root)
        || fs::canonicalize(Path::new(&stored.package_root))
            .ok()
            .is_some_and(|stored_root| {
                selected_canonical_roots
                    .iter()
                    .any(|selected_root| selected_root == &stored_root)
            })
        || (stored.package_root.is_empty() && selected_roots.len() == 1)
}

fn extend_report_from_stored(
    findings: &mut Vec<Finding>,
    pub_use_fix_facts: &mut Vec<PubUseFixFact>,
    stored: StoredReport,
    analysis_root: &Path,
) {
    for finding in stored.findings {
        findings.push(Finding {
            severity:      finding.severity,
            code:          finding.code,
            path:          relativize_path(&finding.path, analysis_root),
            line:          finding.line,
            column:        finding.column,
            highlight_len: finding.highlight_len,
            source_line:   finding.source_line,
            item:          finding.item,
            message:       finding.message,
            suggestion:    finding.suggestion,
            fixability:    finding.fixability,
            related:       finding
                .related
                .map(|related| relativize_path(&related, analysis_root)),
        });
    }
    for fact in stored.pub_use_fix_facts {
        pub_use_fix_facts.push(PubUseFixFact {
            child_path:      relativize_path(&fact.child_path, analysis_root),
            child_line:      fact.child_line,
            child_item_name: fact.child_item_name,
            parent_path:     relativize_path(&fact.parent_path, analysis_root),
            parent_line:     fact.parent_line,
            child_module:    fact.child_module,
        });
    }
}

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    absolute.strip_prefix(analysis_root).map_or_else(
        |_| path.to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

pub(super) fn cache_filename_for(package_root: &Path, crate_root_file: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    package_root.hash(&mut hasher);
    crate_root_file.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}
