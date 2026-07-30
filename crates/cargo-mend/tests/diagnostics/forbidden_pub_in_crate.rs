use tempfile::TempDir;

use crate::support::*;

#[test]
fn restricted_visibility_annotations_are_rejected_once() {
    let temp = tempdir().expect("create temp fixture dir");
    write_sources(
        &temp,
        &[
            (
                "Cargo.toml",
                r#"[package]
name = "restricted_visibility_fixture"
version = "0.1.0"
edition = "2024"
"#,
            ),
            (
                "src/lib.rs",
                "mod fields;\nmod outer;\nmod use_line;\npub(crate) struct Imported;\npub(in crate) fn crate_wide() {}\n",
            ),
            ("src/outer.rs", "mod child;\nmod grandchild;\n"),
            (
                "src/outer/child.rs",
                "pub(in super) fn parent_only() {}\npub(in self) fn current_only() {}\n",
            ),
            (
                "src/outer/grandchild.rs",
                "pub(in super::super) fn root_only() {}\n",
            ),
            ("src/fields.rs", "mod inner;\n"),
            (
                "src/fields/inner.rs",
                "struct Restricted {\n    pub(in crate) crate_wide: u8,\n    pub(in super) parent: u8,\n    pub(in self) current: u8,\n    pub(in super::super) root: u8,\n}\n",
            ),
            ("src/use_line.rs", "pub(in super) use super::Imported;\n"),
        ],
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    assert_rejected_annotations(&report);
}

fn assert_rejected_annotations(report: &Report) {
    assert_codes(report, "src/lib.rs", &[DiagnosticCode::ForbiddenPubCrate]);
    assert_codes(
        report,
        "src/outer/child.rs",
        &[
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
        ],
    );
    assert_codes(
        report,
        "src/outer/grandchild.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_codes(
        report,
        "src/use_line.rs",
        &[DiagnosticCode::ForbiddenPubInCrate],
    );
    assert_codes(
        report,
        "src/fields/inner.rs",
        &[
            DiagnosticCode::ForbiddenPubCrate,
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
            DiagnosticCode::ForbiddenPubInCrate,
        ],
    );

    assert_headline_and_help(
        report,
        "src/lib.rs",
        "use of `pub(in crate)` is forbidden by policy",
        "consider using: `pub(crate)`",
    );
    assert_headline_and_help(
        report,
        "src/outer/child.rs",
        "use of `pub(in super)` is forbidden by policy",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/outer/child.rs",
        "use of `pub(in self)` is forbidden by policy",
        "consider using: `pub(self)`",
    );
    assert_headline_and_help(
        report,
        "src/outer/grandchild.rs",
        "use of `pub(in super::super)` is forbidden by policy",
        "consider using: `pub(crate)`",
    );
    assert_headline_and_help(
        report,
        "src/use_line.rs",
        "use of `pub(in super)` is forbidden by policy",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "use of `pub(in crate)` is forbidden by policy",
        "consider using: `pub(crate)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "use of `pub(in super)` is forbidden by policy",
        "consider using: `pub(super)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "use of `pub(in self)` is forbidden by policy",
        "consider using: `pub(self)`",
    );
    assert_headline_and_help(
        report,
        "src/fields/inner.rs",
        "use of `pub(in super::super)` is forbidden by policy",
        "consider using: `pub(crate)`",
    );
}

fn write_sources(temp: &TempDir, sources: &[(&str, &str)]) {
    for (relative_path, source) in sources {
        let path = temp.path().join(relative_path);
        fs::create_dir_all(path.parent().expect("source path has a parent"))
            .expect("create source parent directory");
        fs::write(path, source).expect("write fixture source");
    }
}

fn assert_codes(report: &Report, suffix: &str, expected: &[DiagnosticCode]) {
    let codes = report
        .findings
        .iter()
        .filter(|finding| finding.path.ends_with(suffix))
        .map(|finding| finding.code)
        .collect::<Vec<_>>();
    assert_eq!(
        codes, expected,
        "unexpected diagnostic code set for {suffix}: {:?}",
        report.findings,
    );
}

fn assert_headline_and_help(report: &Report, suffix: &str, headline: &str, help: &str) {
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.path.ends_with(suffix) && finding.headline == headline)
        .unwrap_or_else(|| {
            panic!(
                "missing headline {headline:?} for {suffix}: {:?}",
                report.findings,
            )
        });
    assert!(
        finding.help.iter().any(|line| line == help),
        "missing help {help:?} for {headline:?}: {:?}",
        finding.help,
    );
}
