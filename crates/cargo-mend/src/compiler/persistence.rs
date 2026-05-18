use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::hash_map::DefaultHasher;
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
use crate::reporting::CompilerWarningFacts;
use crate::reporting::Finding;
use crate::reporting::FixSupport;
use crate::reporting::PubUseFixFact;
use crate::reporting::Report;
use crate::reporting::ReportFacts;
use crate::reporting::ReportSummary;
use crate::reporting::Severity;
use crate::rust_syntax::MODULE_PATH_SEPARATOR;
use crate::selection::Selection;

// file extensions
pub(crate) const JSON_FILE_EXTENSION: &str = "json";

// findings
pub(crate) const FINDINGS_SCHEMA_VERSION: u32 = 14;

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
    #[serde(default)]
    pub use_sites:            Vec<UseSite>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct UseSite {
    /// Canonical def-path of the referenced item, e.g.
    /// `crate::tui::panes::cpu::cpu_required_pane_height`.
    pub target_def_path:        String,
    /// Canonical def-path of the module containing the call site, e.g.
    /// `crate::tui::render::tests`.
    pub caller_module_def_path: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(super) struct StoredFinding {
    pub severity:                Severity,
    pub diagnostic_code:         DiagnosticCode,
    pub path:                    String,
    pub line:                    usize,
    pub column:                  usize,
    pub highlight_len:           usize,
    pub source_line:             String,
    pub item:                    Option<String>,
    pub message:                 String,
    pub suggestion:              Option<String>,
    #[serde(default)]
    pub fixability:              FixSupport,
    #[serde(default)]
    pub related:                 Option<String>,
    /// Canonical def-path of the item this finding is about. Set on
    /// narrowing-style findings so cross-compilation merge can look up the
    /// item's callers post-hoc and suppress findings that would break the
    /// build under the proposed narrower visibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_def_path:           Option<String>,
    /// Canonical def-path of the proposed narrower scope. For a finding
    /// suggesting `pub(super)`, this is the parent module's def-path. The
    /// finding is suppressed if any caller's module is not a descendant of
    /// this scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrower_scope_def_path: Option<String>,
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
    pub use_sites:         Vec<UseSite>,
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
    let mut matched_reports: Vec<StoredReport> = Vec::new();

    for entry in fs::read_dir(findings_dir).with_context(|| {
        format!(
            "failed to read findings directory {}",
            findings_dir.display()
        )
    })? {
        let entry = entry?;
        if entry.path().extension().and_then(OsStr::to_str) != Some(JSON_FILE_EXTENSION) {
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
        matched_reports.push(stored);
    }

    // Apply cross-compilation intersection for narrowing-style findings.
    // A `pub fn` flagged as narrowable to `pub(super)` by the lib
    // compilation may have callers in the lib-test compilation that the
    // narrowing would block; the lib-test compilation will not emit the
    // finding in that case. Same for `internal_parent_pub_use_facade` —
    // if the parent re-export is referenced by test code, the test
    // compilation does not flag it. So we keep narrowing-style findings
    // ONLY when every compilation analyzing the same crate root agrees.
    apply_cross_compilation_intersection(&mut matched_reports);
    // Caller-aware suppression: for suspicious_pub findings that carry a
    // structured item_def_path / narrower_scope_def_path, consult the
    // union of HIR-collected use sites across all compilations. If any
    // caller (including ones inside macro expansions or proc-macro
    // output) lives outside the proposed narrower scope, drop the
    // finding — the proposed narrowing would break a real call site.
    apply_caller_aware_suppression(&mut matched_reports);

    let mut findings = Vec::new();
    let mut pub_use_fix_facts = Vec::new();
    for stored in matched_reports {
        extend_report_from_stored(
            &mut findings,
            &mut pub_use_fix_facts,
            stored,
            selection.analysis_root.as_path(),
        );
    }

    sort_and_dedup_findings(&mut findings);

    // Dedup pub-use fix facts the same way: with bin + bin-test now
    // writing separate cache files, the same fact can be emitted twice
    // and would otherwise cause `--fix-pub-use` to attempt the same
    // rewrite from two compilations.
    pub_use_fix_facts.sort_by(|a, b| {
        (
            &a.child_path,
            a.child_line,
            &a.child_item_name,
            &a.parent_path,
            a.parent_line,
            &a.child_module,
        )
            .cmp(&(
                &b.child_path,
                b.child_line,
                &b.child_item_name,
                &b.parent_path,
                b.parent_line,
                &b.child_module,
            ))
    });
    pub_use_fix_facts.dedup_by(|a, b| {
        a.child_path == b.child_path
            && a.child_line == b.child_line
            && a.child_item_name == b.child_item_name
            && a.parent_path == b.parent_path
            && a.parent_line == b.parent_line
            && a.child_module == b.child_module
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

fn sort_and_dedup_findings(findings: &mut Vec<Finding>) {
    findings.sort_by(|a, b| {
        (
            a.severity,
            &a.path,
            a.line,
            a.column,
            &a.diagnostic_code,
            &a.item,
            &a.message,
        )
            .cmp(&(
                b.severity,
                &b.path,
                b.line,
                b.column,
                &b.diagnostic_code,
                &b.item,
                &b.message,
            ))
    });
    findings.dedup_by(|a, b| {
        a.severity == b.severity
            && a.diagnostic_code == b.diagnostic_code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
    });
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
            severity:        finding.severity,
            diagnostic_code: finding.diagnostic_code,
            path:            relativize_path(&finding.path, analysis_root),
            line:            finding.line,
            column:          finding.column,
            highlight_len:   finding.highlight_len,
            source_line:     finding.source_line,
            item:            finding.item,
            message:         finding.message,
            suggestion:      finding.suggestion,
            fixability:      finding.fixability,
            related:         finding
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

fn apply_caller_aware_suppression(reports: &mut [StoredReport]) {
    // Build the union map: target item def-path -> set of caller modules.
    // Cloned into owned strings so we can release the immutable borrow on
    // `reports` before iterating mutably.
    let mut callers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for report in reports.iter() {
        for site in &report.use_sites {
            callers
                .entry(site.target_def_path.clone())
                .or_default()
                .insert(site.caller_module_def_path.clone());
        }
    }

    for report in reports.iter_mut() {
        report.findings.retain(|finding| {
            // Only narrowing-style findings carry the structured fields.
            // Everything else passes through unchanged.
            let Some(item_path) = finding.item_def_path.as_deref() else {
                return true;
            };
            let Some(narrower_scope) = finding.narrower_scope_def_path.as_deref() else {
                return true;
            };
            let Some(caller_set) = callers.get(item_path) else {
                // No callers seen anywhere — finding is correct, retain.
                return true;
            };
            // Keep the finding only if every caller's module is a
            // descendant of the proposed narrower scope. If any caller
            // lives outside, the narrowing would break that call site.
            caller_set
                .iter()
                .all(|caller| def_path_is_descendant(caller, narrower_scope))
        });
    }
}

/// True when `caller_path` lives within the module subtree rooted at
/// `narrower_scope`. Both arguments come from `tcx.def_path_str`, so they
/// share the rendering convention (`crate::a::b`). Equality and prefix
/// match (with `::` boundary) are both considered descendants.
fn def_path_is_descendant(caller_path: &str, narrower_scope: &str) -> bool {
    if caller_path == narrower_scope {
        return true;
    }
    if let Some(rest) = caller_path.strip_prefix(narrower_scope)
        && rest.starts_with(MODULE_PATH_SEPARATOR)
    {
        return true;
    }
    false
}

/// Codes whose findings propose narrowing visibility. For these, an
/// agreement across every compilation analyzing the same crate root is
/// required — a finding from the lib compilation is dropped if any sibling
/// compilation (lib-test, etc.) does not flag the same item. Codes not in
/// this set use the default union/dedup behavior.
const fn requires_cross_compilation_agreement(code: DiagnosticCode) -> bool {
    matches!(
        code,
        DiagnosticCode::SuspiciousPub | DiagnosticCode::InternalParentPubUseFacade
    )
}

fn finding_intersection_key(finding: &StoredFinding) -> (DiagnosticCode, String, usize, usize) {
    (
        finding.diagnostic_code,
        finding.path.clone(),
        finding.line,
        finding.column,
    )
}

fn apply_cross_compilation_intersection(reports: &mut [StoredReport]) {
    if reports.len() < 2 {
        return;
    }

    // Group reports by crate-root file so the intersection is taken across
    // sibling compilations of the same crate (lib + lib-test, etc.).
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (idx, report) in reports.iter().enumerate() {
        groups
            .entry(report.crate_root_file.clone())
            .or_default()
            .push(idx);
    }

    for indices in groups.into_values() {
        if indices.len() < 2 {
            continue;
        }

        // Build the per-key emission count across the group, restricted to
        // codes that require cross-compilation agreement.
        let mut emission_count: BTreeMap<(DiagnosticCode, String, usize, usize), usize> =
            BTreeMap::new();
        for &idx in &indices {
            let mut seen_in_this_report: BTreeSet<(DiagnosticCode, String, usize, usize)> =
                BTreeSet::new();
            for finding in &reports[idx].findings {
                if requires_cross_compilation_agreement(finding.diagnostic_code) {
                    let key = finding_intersection_key(finding);
                    if seen_in_this_report.insert(key.clone()) {
                        *emission_count.entry(key).or_default() += 1;
                    }
                }
            }
        }

        let group_size = indices.len();

        // Drop findings that did not appear in every compilation of the
        // group. (They are false positives caused by single-compilation
        // visibility — typically lib-only views that strip cfg(test).)
        for &idx in &indices {
            reports[idx].findings.retain(|finding| {
                if !requires_cross_compilation_agreement(finding.diagnostic_code) {
                    return true;
                }
                let key = finding_intersection_key(finding);
                emission_count.get(&key).copied().unwrap_or(0) == group_size
            });
        }
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

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum CacheBuildKind {
    Library,
    Test,
}

pub(super) fn cache_filename_for(
    package_root: &Path,
    crate_root_file: &Path,
    build_kind: CacheBuildKind,
) -> String {
    let mut hasher = DefaultHasher::new();
    package_root.hash(&mut hasher);
    crate_root_file.hash(&mut hasher);
    // Without this byte, the bin (non-test) and bin-test compilations of a
    // binary crate share the same `crate_root_file` and overwrite each
    // other's findings — leaving the cross-compilation intersection in
    // `load_report` with only one report to consult, so it can't detect
    // findings the other compilation would contradict.
    build_kind.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}
