use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde_json::from_str;

use super::StoredReport;
use super::caller_aware;
use super::intersection;
use super::visibility_priority;
use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
use crate::compiler::constants::JSON_FILE_EXTENSION;
use crate::compiler::settings;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::Finding;
use crate::reporting::PubUseFixFact;
use crate::reporting::Report;
use crate::reporting::ReportFacts;
use crate::reporting::ReportSummary;
use crate::selection::Selection;

pub fn load_report(
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
        let Ok(stored) = from_str::<StoredReport>(&text) else {
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

    intersection::apply_cross_compilation_intersection(&mut matched_reports);
    caller_aware::apply_caller_aware_suppression(&mut matched_reports);
    visibility_priority::apply_visibility_narrowing_priority(&mut matched_reports);

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
    sort_and_dedup_pub_use_fix_facts(&mut pub_use_fix_facts);

    Ok(Report {
        root: selection_root_string(selection.analysis_root.as_path()),
        summary: ReportSummary::default(),
        findings,
        facts: ReportFacts {
            pub_use_fix_facts:      pub_use_fix_facts.into(),
            compiler_warning_facts: CompilerWarningFacts::None,
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

fn sort_and_dedup_pub_use_fix_facts(pub_use_fix_facts: &mut Vec<PubUseFixFact>) {
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
}

fn stored_report_matches_selection(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
    config_fingerprint: &str,
) -> bool {
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
            fix_support:     finding.fix_support,
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

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    absolute.strip_prefix(analysis_root).map_or_else(
        |_| path.to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;

    use serde_json::to_vec_pretty;
    use tempfile::TempDir;
    use tempfile::tempdir;

    use super::load_report;
    use crate::compiler::constants::FINDINGS_SCHEMA_VERSION;
    use crate::compiler::persistence::StoredFinding;
    use crate::compiler::persistence::StoredPubUseFixFact;
    use crate::compiler::persistence::StoredReport;
    use crate::compiler::settings;
    use crate::config::DiagnosticCode;
    use crate::reporting::CompilerWarningFacts;
    use crate::reporting::FixSupport;
    use crate::reporting::Severity;
    use crate::selection::Selection;
    use crate::selection::SelectionScope;

    const CONFIG_FINGERPRINT: &str = "config-fingerprint";

    struct PersistenceFixture {
        temp:         TempDir,
        findings_dir: PathBuf,
        package_root: PathBuf,
        crate_root:   PathBuf,
    }

    impl PersistenceFixture {
        fn new() -> Self {
            let temp = tempdir().expect("create persistence fixture");
            let package_root = temp.path().join("package");
            let source_dir = package_root.join("src");
            fs::create_dir_all(&source_dir).expect("create package src dir");
            let crate_root = source_dir.join("lib.rs");
            fs::write(&crate_root, "pub fn item() {}\n").expect("write crate root");
            let findings_dir = temp.path().join("findings");
            fs::create_dir_all(&findings_dir).expect("create findings dir");
            Self {
                temp,
                findings_dir,
                package_root,
                crate_root,
            }
        }

        fn selection(&self) -> Selection {
            self.selection_with_roots(vec![self.package_root.clone()])
        }

        fn selection_with_roots(&self, package_roots: Vec<PathBuf>) -> Selection {
            Selection {
                manifest_path: self.package_root.join("Cargo.toml"),
                manifest_dir: self.package_root.clone(),
                workspace_root: self.package_root.clone(),
                target_directory: self.temp.path().join("target"),
                analysis_root: self.package_root.clone(),
                scope: SelectionScope::SinglePackage,
                package_roots,
                packages: Vec::new(),
            }
        }

        fn write_report(&self, file_name: &str, report: &StoredReport) {
            fs::write(
                self.findings_dir.join(file_name),
                to_vec_pretty(report).expect("serialize stored report"),
            )
            .expect("write stored report");
        }

        fn write_malformed_json(&self, file_name: &str) {
            fs::write(self.findings_dir.join(file_name), b"{ not json")
                .expect("write malformed report");
        }

        fn report_with_findings(&self, findings: Vec<StoredFinding>) -> StoredReport {
            StoredReport {
                version: FINDINGS_SCHEMA_VERSION,
                analysis_fingerprint: settings::current_analysis_fingerprint(),
                scope_fingerprint: "scope".to_string(),
                package_root: self.package_root.to_string_lossy().into_owned(),
                crate_root_file: self.crate_root.to_string_lossy().into_owned(),
                config_fingerprint: CONFIG_FINGERPRINT.to_string(),
                findings,
                pub_use_fix_facts: Vec::new(),
                compiler_warning_facts: CompilerWarningFacts::None,
                use_sites: Vec::new(),
            }
        }
    }

    #[test]
    fn malformed_json_file_is_ignored() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let report = fixture.report_with_findings(vec![finding]);
        fixture.write_malformed_json("broken.json");
        fixture.write_report("valid.json", &report);

        let loaded = load_report(
            &fixture.findings_dir,
            &fixture.selection(),
            CONFIG_FINGERPRINT,
        )
        .expect("load report");

        assert_eq!(loaded.findings.len(), 1);
    }

    #[test]
    fn wrong_schema_or_fingerprint_reports_are_rejected() {
        let fixture = PersistenceFixture::new();
        let mut wrong_schema = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_schema.version = FINDINGS_SCHEMA_VERSION + 1;
        let mut wrong_analysis = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_analysis.analysis_fingerprint = "old-analysis".to_string();
        let mut wrong_config = fixture.report_with_findings(vec![stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        )]);
        wrong_config.config_fingerprint = "old-config".to_string();
        fixture.write_report("schema.json", &wrong_schema);
        fixture.write_report("analysis.json", &wrong_analysis);
        fixture.write_report("config.json", &wrong_config);

        let loaded = load_report(
            &fixture.findings_dir,
            &fixture.selection(),
            CONFIG_FINGERPRINT,
        )
        .expect("load report");

        assert!(loaded.findings.is_empty());
    }

    #[test]
    fn missing_crate_root_report_is_rejected() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.crate_root_file = "src/missing.rs".to_string();
        fixture.write_report("missing-root.json", &report);

        let loaded = load_report(
            &fixture.findings_dir,
            &fixture.selection(),
            CONFIG_FINGERPRINT,
        )
        .expect("load report");

        assert!(loaded.findings.is_empty());
    }

    #[test]
    fn canonical_selected_roots_are_accepted() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let report = fixture.report_with_findings(vec![finding]);
        fixture.write_report("canonical-root.json", &report);
        let selected_root = fixture.package_root.join("src").join("..");
        let selection = fixture.selection_with_roots(vec![selected_root]);

        let loaded = load_report(&fixture.findings_dir, &selection, CONFIG_FINGERPRINT)
            .expect("load report");

        assert_eq!(loaded.findings.len(), 1);
    }

    #[test]
    fn empty_package_root_compatibility_is_retained() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.package_root.clear();
        report.crate_root_file.clear();
        fixture.write_report("legacy-root.json", &report);

        let loaded = load_report(
            &fixture.findings_dir,
            &fixture.selection(),
            CONFIG_FINGERPRINT,
        )
        .expect("load report");

        assert_eq!(loaded.findings.len(), 1);
    }

    #[test]
    fn serialized_driver_report_loads_back_through_load_report() {
        let fixture = PersistenceFixture::new();
        let finding = stored_finding(
            DiagnosticCode::ForbiddenPubCrate,
            &fixture.crate_root,
            "item",
            1,
        );
        let mut report = fixture.report_with_findings(vec![finding]);
        report.pub_use_fix_facts.push(StoredPubUseFixFact {
            child_path:      fixture.crate_root.to_string_lossy().into_owned(),
            child_line:      2,
            child_item_name: "Child".to_string(),
            parent_path:     fixture.crate_root.to_string_lossy().into_owned(),
            parent_line:     3,
            child_module:    "child".to_string(),
        });
        fixture.write_report("driver-report.json", &report);

        let loaded = load_report(
            &fixture.findings_dir,
            &fixture.selection(),
            CONFIG_FINGERPRINT,
        )
        .expect("load report");
        let facts = loaded.facts.pub_use_fix_facts.iter().collect::<Vec<_>>();

        assert_eq!(loaded.findings.len(), 1);
        assert_eq!(loaded.findings[0].path, "src/lib.rs");
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].child_path, "src/lib.rs");
    }

    fn stored_finding(
        diagnostic_code: DiagnosticCode,
        path: &Path,
        item: &str,
        line: usize,
    ) -> StoredFinding {
        StoredFinding {
            severity: Severity::Warning,
            diagnostic_code,
            path: path.to_string_lossy().into_owned(),
            line,
            column: 1,
            highlight_len: 3,
            source_line: "pub fn item() {}".to_string(),
            item: Some(item.to_string()),
            message: format!("{item} should change visibility"),
            suggestion: Some("use narrower visibility".to_string()),
            fix_support: FixSupport::None,
            related: None,
            item_def_path: None,
            narrower_scope_def_path: None,
        }
    }
}
