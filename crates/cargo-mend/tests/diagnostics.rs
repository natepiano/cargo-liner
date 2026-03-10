#![allow(clippy::expect_used)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use cargo_mend_tests_support::FixSummaryBucket;
use cargo_mend_tests_support::FixSupport;
use cargo_mend_tests_support::diagnostic_specs;
use serde::Deserialize;
use tempfile::tempdir;

fn clear_wrappers(command: &mut Command) -> &mut Command {
    command
        .env_remove("RUSTC")
        .env("RUSTC_WRAPPER", "")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
}

fn cargo_command() -> Command {
    let mut command = Command::new("cargo");
    clear_wrappers(&mut command);
    command
}

fn mend_command() -> Command {
    let mut command = Command::new(mend_bin());
    clear_wrappers(&mut command);
    command
}

fn mend_bin() -> PathBuf { PathBuf::from(env!("CARGO_BIN_EXE_cargo-mend")) }

#[derive(Debug, Deserialize)]
struct Finding {
    code:        String,
    #[serde(default)]
    path:        String,
    #[serde(default)]
    fix_support: FixSupport,
}

#[derive(Debug, Deserialize)]
struct Report {
    summary:  Summary,
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
struct Summary {
    error_count:                    usize,
    warning_count:                  usize,
    fixable_with_fix_count:         usize,
    fixable_with_fix_pub_use_count: usize,
}

#[derive(Clone, Copy)]
struct ExpectedFinding<'a> {
    code:        &'a str,
    fix_support: FixSupport,
}

fn severity_for_code(code: &str) -> &'static str {
    match code {
        "forbidden_pub_crate" | "forbidden_pub_in_crate" | "review_pub_mod" => "error",
        _ => "warning",
    }
}

fn expected_summary(report: &Report) -> Summary {
    let mut summary = Summary {
        error_count:                    0,
        warning_count:                  0,
        fixable_with_fix_count:         0,
        fixable_with_fix_pub_use_count: 0,
    };

    for finding in &report.findings {
        match severity_for_code(&finding.code) {
            "error" => summary.error_count += 1,
            _ => summary.warning_count += 1,
        }

        let fix_support = if matches!(finding.fix_support, FixSupport::None) {
            diagnostic_specs()
                .iter()
                .find(|spec| spec.code == finding.code)
                .expect("known diagnostic code")
                .fix_support
        } else {
            finding.fix_support
        };
        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix_count += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use_count += 1,
            None => {},
        }
    }

    summary
}

fn assert_summary_matches_findings(report: &Report) {
    let expected = expected_summary(report);
    assert_eq!(report.summary.error_count, expected.error_count);
    assert_eq!(report.summary.warning_count, expected.warning_count);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected.fixable_with_fix_pub_use_count
    );
}

fn expected_summary_from_findings(expected_findings: &[ExpectedFinding<'_>]) -> Summary {
    let mut summary = Summary {
        error_count:                    0,
        warning_count:                  0,
        fixable_with_fix_count:         0,
        fixable_with_fix_pub_use_count: 0,
    };

    for finding in expected_findings {
        match severity_for_code(finding.code) {
            "error" => summary.error_count += 1,
            _ => summary.warning_count += 1,
        }

        let fix_support = if matches!(finding.fix_support, FixSupport::None) {
            diagnostic_specs()
                .iter()
                .find(|spec| spec.code == finding.code)
                .expect("known diagnostic code")
                .fix_support
        } else {
            finding.fix_support
        };

        match fix_support.summary_bucket() {
            Some(FixSummaryBucket::Fix) => summary.fixable_with_fix_count += 1,
            Some(FixSummaryBucket::FixPubUse) => summary.fixable_with_fix_pub_use_count += 1,
            None => {},
        }
    }

    summary
}

fn expected_summary_text(report: &Report) -> String {
    let mut parts = vec![
        format!("{} error(s)", report.summary.error_count),
        format!("{} warning(s)", report.summary.warning_count),
    ];

    if report.summary.fixable_with_fix_count > 0 {
        parts.push(format!(
            "{} fixable with `--fix`",
            report.summary.fixable_with_fix_count
        ));
    }

    if report.summary.fixable_with_fix_pub_use_count > 0 {
        parts.push(format!(
            "{} fixable with `--fix-pub-use`",
            report.summary.fixable_with_fix_pub_use_count
        ));
    }

    format!("summary: {}", parts.join(", "))
}

fn run_mend_json(manifest_path: &std::path::Path) -> Report {
    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--json")
        .output()
        .expect("run cargo-mend --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse mend json report")
}

#[test]
fn every_diagnostic_has_a_unique_readme_anchor() {
    let readme = include_str!("../README.md");
    let mut seen_codes = BTreeSet::new();
    let mut seen_anchors = BTreeSet::new();

    for spec in diagnostic_specs() {
        assert!(
            seen_codes.insert(spec.code),
            "duplicate diagnostic code: {}",
            spec.code
        );
        assert!(
            seen_anchors.insert(spec.help_anchor),
            "duplicate README anchor: {}",
            spec.help_anchor
        );
        let anchor = format!(r#"<a id="{}"></a>"#, spec.help_anchor);
        assert!(
            readme.contains(&anchor),
            "README is missing anchor for {}: {}",
            spec.code,
            spec.help_anchor
        );
    }
}

#[test]
fn fixture_renders_every_current_diagnostic() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");
    fs::create_dir_all(temp.path().join("src/stale_parent"))
        .expect("create stale nested fixture dir");
    fs::create_dir_all(temp.path().join("src/wild_parent"))
        .expect("create wildcard nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub(crate) fn crate_only() {}
mod private_parent;
mod stale_parent;
mod wild_parent;
pub mod review_mod;
pub use private_parent::PublicContainer;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/review_mod.rs"), "\n").expect("write review mod");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        r#"use crate::private_parent::PublicContainer as ParentContainer;

pub enum LegitDependency {
    Unit,
}

pub(in crate::private_parent) fn subtree_only() {}

pub struct PublicContainer {
    pub dependency: LegitDependency,
}

pub struct Suspicious;
"#,
    )
    .expect("write suspicious child");
    fs::write(
        temp.path().join("src/stale_parent/mod.rs"),
        "mod child;\npub use child::StaleExport;\n",
    )
    .expect("write stale parent");
    fs::write(
        temp.path().join("src/stale_parent/child.rs"),
        "pub struct StaleExport;\n",
    )
    .expect("write stale child");
    fs::write(
        temp.path().join("src/wild_parent/mod.rs"),
        "mod child;\npub use child::*;\n",
    )
    .expect("write wildcard parent");
    fs::write(
        temp.path().join("src/wild_parent/child.rs"),
        "pub struct WildExport;\n",
    )
    .expect("write wildcard child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run cargo-mend against fixture");
    assert!(
        matches!(output.status.code(), Some(1 | 2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Report = serde_json::from_slice(&output.stdout).expect("parse mend json report");
    let codes: BTreeSet<_> = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    let expected_codes: BTreeSet<_> = diagnostic_specs().iter().map(|spec| spec.code).collect();

    assert_eq!(
        codes, expected_codes,
        "fixture should trigger every diagnostic at least once"
    );
    assert_eq!(report.findings.len(), 8);
    assert_summary_matches_findings(&report);

    let rendered_output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo-mend human output");
    assert!(
        matches!(rendered_output.status.code(), Some(1 | 2)),
        "cargo-mend returned unexpected status {:?}: {}",
        rendered_output.status.code(),
        String::from_utf8_lossy(&rendered_output.stderr)
    );
    let rendered = String::from_utf8(rendered_output.stdout).expect("decode human output");

    for spec in diagnostic_specs() {
        assert!(
            rendered.contains(spec.headline),
            "rendered output is missing headline for {}",
            spec.code
        );
        let help_url = format!(
            "https://github.com/natepiano/cargo-mend#{}",
            spec.help_anchor
        );
        assert!(
            rendered.contains(&help_url),
            "rendered output is missing help URL for {}",
            spec.code
        );
    }

    assert!(rendered.contains("help: consider using just `pub` or removing `pub(crate)` entirely"));
    assert!(rendered.contains("help: consider using: `pub(super)`"));
    assert!(
        rendered.contains("help: consider using: `use super::PublicContainer as ParentContainer;`")
    );
    assert!(rendered.contains("note: this warning is auto-fixable with `cargo mend --fix`"));
    assert!(
        rendered.contains("note: this warning is auto-fixable with `cargo mend --fix-pub-use`")
    );
    assert!(rendered.contains(&expected_summary_text(&report)));
    assert!(rendered.contains(
        "parent module also has an `unused import` warning for this `pub use` at stale_parent/mod.rs"
    ));
    assert!(rendered.contains("help: consider re-exporting explicit items instead of `*`"));
}

#[test]
fn fix_rewrites_local_crate_import() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod inner;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/inner.rs"),
        r#"use crate::inner::Thing as LocalThing;

pub struct Thing;
"#,
    )
    .expect("write fixture inner");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let inner = fs::read_to_string(temp.path().join("src/inner.rs")).expect("read fixed file");
    assert!(inner.contains("use Thing as LocalThing;"));
    assert!(!inner.contains("use crate::inner::Thing as LocalThing;"));
    assert!(!inner.contains("use self::Thing as LocalThing;"));
}

#[test]
fn fix_does_not_introduce_self_for_same_module_child_imports() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_plain_child_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod private_parent;\npub use private_parent::PublicContainer;\n",
    )
    .expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub use self::child::PublicContainer;"));
}

#[test]
fn fix_preserves_pub_use_visibility() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod private_parent;\npub use private_parent::PublicContainer;\n",
    )
    .expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub use crate::private_parent::child::PublicContainer;"));
}

#[test]
fn fix_preserves_pub_crate_use_visibility() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_crate_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(temp.path().join("src/lib.rs"), "mod private_parent;\n").expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub(crate) use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub(crate) use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub(crate) use crate::private_parent::child::PublicContainer;"));
}

#[test]
fn fix_rolls_back_on_failed_cargo_check() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod inner;
mod broken;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/inner.rs"),
        r#"use crate::inner::Thing as LocalThing;

pub struct Thing;
"#,
    )
    .expect("write fixture inner");
    fs::write(
        temp.path().join("src/broken.rs"),
        "pub fn broken() -> MissingType { todo!() }\n",
    )
    .expect("write fixture broken");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        !output.status.success(),
        "cargo-mend --fix unexpectedly succeeded: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let inner =
        fs::read_to_string(temp.path().join("src/inner.rs")).expect("read rolled back file");
    assert!(inner.contains("use crate::inner::Thing as LocalThing;"));
    assert!(!inner.contains("use Thing as LocalThing;"));
}

#[test]
fn fix_reports_when_nothing_is_fixable() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_noop_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: no import fixes available"));
}

#[test]
fn fix_reports_noop_notice_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_noop_order_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stdout.contains("No findings."));
    assert!(stderr.contains("mend: no import fixes available"));
}

#[test]
fn fix_reports_applied_notice_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_applied_notice_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");

    assert!(stdout.contains("summary:"));
    assert!(stderr.contains("mend: applied 1 import fix(es)"));
}

#[test]
fn fix_pub_use_reports_import_cleanup_suggestion_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_import_cleanup_notice_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\nuse child::Leftover;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct SpawnStats;\npub struct Leftover;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");

    assert!(stdout.contains("summary:"));
    assert!(
        stderr.contains("mend: applied 1 `pub use` fix(es)"),
        "expected applied pub use notice in stderr:\n{stderr}"
    );
}

#[test]
fn dry_run_reports_import_fixes_without_editing_files() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dry_run_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/parent.rs"), "mod child;\n").expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mend: would apply 1 import fix(es) in dry run"));

    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(consumer.contains("use crate::parent::child::Thing;"));
}

#[test]
fn fix_pub_use_rewrites_sibling_imports_and_narrows_child() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_sibling_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mod_rs =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read fixed actor mod");
    let child =
        fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read fixed child");
    let sibling =
        fs::read_to_string(temp.path().join("src/actor/sibling.rs")).expect("read fixed sibling");

    assert!(!mod_rs.contains("pub use child::SpawnStats;"));
    assert!(child.contains("pub(super) struct SpawnStats;"));
    assert!(sibling.contains("use super::child::SpawnStats;"));
    assert!(!sibling.contains("use super::SpawnStats;"));

    let follow_up = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("rerun cargo-mend after fix-pub-use");
    assert!(
        follow_up.status.success(),
        "follow-up cargo-mend failed: {}\n{}",
        String::from_utf8_lossy(&follow_up.stdout),
        String::from_utf8_lossy(&follow_up.stderr)
    );
    assert!(String::from_utf8_lossy(&follow_up.stdout).contains("No findings."));

    let repeat_fix = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("rerun cargo-mend --fix-pub-use after fix");
    assert!(
        repeat_fix.status.success(),
        "repeat cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&repeat_fix.stdout),
        String::from_utf8_lossy(&repeat_fix.stderr)
    );
    assert!(
        String::from_utf8_lossy(&repeat_fix.stderr).contains("mend: no `pub use` fixes available")
    );
}

#[test]
fn fix_pub_use_suppresses_targeted_unused_import_warning_during_discovery() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_suppression_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .contains("mend: suppressing `unused import` warning during `--fix-pub-use` discovery"),
        "expected suppression notice in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("warning: unused import: `child::SpawnStats`"),
        "unexpected forwarded unused-import warning in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("to apply 1 suggestion"),
        "unexpected forwarded cargo-fix suggestion summary in stderr:\n{stderr}"
    );
}

#[test]
fn dry_run_reports_pub_use_fixes_without_editing_files() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dry_run_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mend: would apply 1 `pub use` fix(es) in dry run"));

    let mod_rs = fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read actor mod");
    let child = fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read child");
    let sibling =
        fs::read_to_string(temp.path().join("src/actor/sibling.rs")).expect("read sibling");

    assert!(mod_rs.contains("pub use child::SpawnStats;"));
    assert!(child.contains("pub struct SpawnStats;"));
    assert!(sibling.contains("use super::SpawnStats;"));
}

#[test]
fn fix_pub_use_rewrites_nested_descendant_imports() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_nested_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor/nested")).expect("create src/actor/nested");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod nested;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(temp.path().join("src/actor/nested/mod.rs"), "mod deeper;\n")
        .expect("write nested mod");
    fs::write(
        temp.path().join("src/actor/nested/deeper.rs"),
        "use super::super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write deeper");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let deeper = fs::read_to_string(temp.path().join("src/actor/nested/deeper.rs"))
        .expect("read fixed deeper");
    assert!(deeper.contains("use super::super::child::SpawnStats;"));
    assert!(!deeper.contains("use super::super::SpawnStats;"));
}

#[test]
fn fix_pub_use_handles_child_items_with_attributes() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_attribute_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "#[derive(Debug)]\npub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let child =
        fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read fixed child");
    let sibling =
        fs::read_to_string(temp.path().join("src/actor/sibling.rs")).expect("read fixed sibling");

    assert!(child.contains("#[derive(Debug)]\npub(super) struct SpawnStats;"));
    assert!(sibling.contains("use super::child::SpawnStats;"));
}

#[test]
fn fix_pub_use_rolls_back_on_failed_cargo_check() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\nmod broken;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");
    fs::write(
        temp.path().join("src/broken.rs"),
        "pub fn broken() -> MissingType { todo!() }\n",
    )
    .expect("write broken");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        !output.status.success(),
        "cargo-mend --fix-pub-use unexpectedly succeeded: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read rolled back mod");
    let child =
        fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read rolled back child");
    assert!(mod_rs.contains("pub use child::SpawnStats;"));
    assert!(child.contains("pub struct SpawnStats;"));
}

#[test]
fn fix_pub_use_reports_when_nothing_is_fixable() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_noop_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\nuse private_parent::PublicContainer;\n\nfn main() { let _ = std::mem::size_of::<PublicContainer>(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private_parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: no `pub use` fixes available"));
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use super::Thing;\n\npub fn keep(_: Thing) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 2 `pub use` fix(es) in dry run"));
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use super::Thing;\n\npub fn keep(_: Thing) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = fs::read_to_string(temp.path().join("src/parent.rs")).expect("read fixed parent");
    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed child");
    let sibling =
        fs::read_to_string(temp.path().join("src/parent/sibling.rs")).expect("read fixed sibling");

    assert!(!parent.contains("pub use"));
    assert!(child.contains("pub(super) struct Thing;"));
    assert!(child.contains("pub(super) struct Other;"));
    assert!(sibling.contains("use super::child::Thing;"));
    assert!(!sibling.contains("use super::Thing;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed grouped fixture");
    assert!(
        check.status.success(),
        "cargo check failed after grouped apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_rewrites_multiline_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_multiline_grouped_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{\n    Thing,\n    Other,\n};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use super::Thing;\n\npub fn keep(_: Thing) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 2 `pub use` fix(es) in dry run"));
    assert!(!stderr.contains("warning: unused imports: `Thing` and `Other`"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_file_parent_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_parent_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::{PublicContainer, Other};\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\npub struct Other;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = fs::read_to_string(temp.path().join("src/private_parent.rs"))
        .expect("read fixed file parent");
    let child = fs::read_to_string(temp.path().join("src/private_parent/child.rs"))
        .expect("read fixed child");

    assert!(!parent.contains("pub use"));
    assert!(child.contains("pub(super) struct PublicContainer;"));
    assert!(child.contains("pub(super) struct Other;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed file-parent grouped fixture");
    assert!(
        check.status.success(),
        "cargo check failed after file-parent grouped apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_rewrites_obsidian_style_grouped_file_facades_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "obsidian_style_grouped_facades_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod report;
mod utils;

use report::ReportWriter;
use utils::Sha256Cache;

fn main() {
    let _ = ReportWriter;
    let _ = Sha256Cache;
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        r#"mod report_consumer;
mod report_writer;

pub use report_writer::{ReportDefinition, ReportWriter};
"#,
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        r#"pub trait ReportDefinition {}

pub struct ReportWriter;
"#,
    )
    .expect("write report writer child");
    fs::write(
        temp.path().join("src/report/report_consumer.rs"),
        r#"use super::ReportDefinition;

pub fn accept<T: ReportDefinition>(_value: &T) {}
"#,
    )
    .expect("write report consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;
mod sha256_cache;
mod status_consumer;

pub use file_utils::{collect_repository_files, RepositoryFiles};
pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, CachedImageInfo, Sha256Cache};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub fn collect_repository_files() {}

pub struct RepositoryFiles;
"#,
    )
    .expect("write file utils child");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheEntryStatus {
    Fresh,
}

pub enum CacheFileStatus {
    Present,
}

pub struct CachedImageInfo;

pub struct Sha256Cache;
"#,
    )
    .expect("write sha256 child");
    fs::write(
        temp.path().join("src/utils/status_consumer.rs"),
        r#"use super::CacheEntryStatus;

pub fn touch(_: CacheEntryStatus) {}
"#,
    )
    .expect("write status consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let report_facade =
        fs::read_to_string(temp.path().join("src/report.rs")).expect("read fixed report facade");
    let report_child = fs::read_to_string(temp.path().join("src/report/report_writer.rs"))
        .expect("read fixed report child");
    let report_consumer = fs::read_to_string(temp.path().join("src/report/report_consumer.rs"))
        .expect("read fixed report consumer");
    let utils_facade =
        fs::read_to_string(temp.path().join("src/utils.rs")).expect("read fixed utils facade");
    let utils_child = fs::read_to_string(temp.path().join("src/utils/sha256_cache.rs"))
        .expect("read fixed utils child");
    let status_consumer = fs::read_to_string(temp.path().join("src/utils/status_consumer.rs"))
        .expect("read fixed status consumer");

    assert!(report_facade.contains("pub use report_writer::ReportWriter;"));
    assert!(!report_facade.contains("ReportDefinition"));
    assert!(report_child.contains("pub(super) trait ReportDefinition {}"));
    assert!(report_consumer.contains("use super::report_writer::ReportDefinition;"));
    assert!(!report_consumer.contains("use super::ReportDefinition;"));

    assert!(!utils_facade.contains("collect_repository_files"));
    assert!(!utils_facade.contains("RepositoryFiles"));
    assert!(utils_facade.contains("pub use sha256_cache::Sha256Cache;"));
    assert!(!utils_facade.contains("CacheEntryStatus"));
    assert!(!utils_facade.contains("CacheFileStatus"));
    assert!(!utils_facade.contains("CachedImageInfo"));
    assert!(utils_child.contains("pub(super) enum CacheEntryStatus"));
    assert!(utils_child.contains("pub(super) enum CacheFileStatus"));
    assert!(utils_child.contains("pub(super) struct CachedImageInfo;"));
    assert!(status_consumer.contains("use super::sha256_cache::CacheEntryStatus;"));
    assert!(!status_consumer.contains("use super::CacheEntryStatus;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed obsidian-style facade fixture");
    assert!(
        check.status.success(),
        "cargo check failed after obsidian-style apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_rewrites_grouped_in_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{ReportDefinition, ReportWriter};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use crate::parent::{ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: ReportWriter, _: T) {}\n",
    )
    .expect("write sibling");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let sibling =
        fs::read_to_string(temp.path().join("src/parent/sibling.rs")).expect("read fixed sibling");
    assert!(sibling.contains("use super::child::{ReportDefinition, ReportWriter};"));
    assert!(!sibling.contains("use crate::parent::{ReportDefinition, ReportWriter};"));
}

#[test]
fn fix_pub_use_rewrites_mixed_grouped_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_mixed_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod report;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        "mod report_writer;\nmod frontmatter;\npub use report_writer::{DescriptionBuilder, ReportDefinition, ReportWriter};\n",
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write report child");
    fs::write(
        temp.path().join("src/report/frontmatter.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write report consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade = fs::read_to_string(temp.path().join("src/report.rs")).expect("read fixed facade");
    let child = fs::read_to_string(temp.path().join("src/report/report_writer.rs"))
        .expect("read fixed child");
    let consumer = fs::read_to_string(temp.path().join("src/report/frontmatter.rs"))
        .expect("read fixed consumer");

    assert!(!facade.contains("pub use"));
    assert!(!facade.contains("DescriptionBuilder"));
    assert!(!facade.contains("ReportDefinition"));
    assert!(!facade.contains("ReportWriter"));
    assert!(child.contains("pub(super) struct DescriptionBuilder;"));
    assert!(child.contains("pub(super) trait ReportDefinition {}"));
    assert!(child.contains("pub(super) struct ReportWriter;"));
    assert!(consumer.contains(
        "use super::report_writer::{DescriptionBuilder, ReportDefinition, ReportWriter};"
    ));
    assert!(!consumer.contains("use crate::report::DescriptionBuilder;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed mixed grouped fixture");
    assert!(
        check.status.success(),
        "cargo check failed after mixed grouped apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_preserves_parent_local_access_with_private_use() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_parent_local_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nuse crate::parent::InlineCodeExcluder;\n\nfn main() { let _ = InlineCodeExcluder::new(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{CodeBlockExcluder, InlineCodeExcluder};\n\nfn build() -> (CodeBlockExcluder, InlineCodeExcluder) {\n    (CodeBlockExcluder::new(), InlineCodeExcluder::new())\n}\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct CodeBlockExcluder;\npub struct InlineCodeExcluder;\nimpl CodeBlockExcluder { pub fn new() -> Self { Self } }\nimpl InlineCodeExcluder { pub fn new() -> Self { Self } }\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = fs::read_to_string(temp.path().join("src/parent.rs")).expect("read fixed parent");
    assert!(parent.contains("pub use child::InlineCodeExcluder;"));
    assert!(parent.contains("use child::CodeBlockExcluder;"));
    assert!(!parent.contains("pub use child::{CodeBlockExcluder, InlineCodeExcluder};"));
}

#[test]
fn fix_pub_use_preserves_exports_used_outside_parent_via_normal_paths() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_preserves_path_based_exports_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod config;
mod utils;

fn main() {
    config::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/config.rs"),
        r#"pub fn run() {
    let _ = crate::utils::expand_tilde("~/vault");
}
"#,
    )
    .expect("write fixture config");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;
mod sha256_cache;

pub use file_utils::{expand_tilde, RepositoryFiles};
pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, CachedImageInfo, Sha256Cache};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub fn expand_tilde(_path: &str) -> String {
    String::from("/tmp/vault")
}

pub struct RepositoryFiles;
"#,
    )
    .expect("write file utils child");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheEntryStatus {
    Fresh,
}

pub enum CacheFileStatus {
    Present,
}

pub struct CachedImageInfo;

pub struct Sha256Cache;
"#,
    )
    .expect("write sha256 child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 5 `pub use` fix(es) in dry run"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_other_crate_visible_signature() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod utils;

fn main() {
    let repo = utils::collect_repository_files();
    consumer::consume(repo);
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn consume(_: impl Sized) {}
"#,
    )
    .expect("write fixture consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;

pub use file_utils::{collect_repository_files, RepositoryFiles};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub struct RepositoryFiles;

pub fn collect_repository_files() -> RepositoryFiles {
    RepositoryFiles
}
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/utils/file_utils.rs"),
        "expected no suspicious_pub for child type exposed by another crate-visible signature, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_exported_method_signatures() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_method_signature_exposure_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod utils;

fn main() {
    consumer::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() {
    let (_, _) = crate::utils::load_cache();
    let mut cache = crate::utils::Sha256Cache;
    let _ = cache.get_or_update();
}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod sha256_cache;

pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, Sha256Cache};

pub fn load_cache() -> (Sha256Cache, CacheFileStatus) {
    Sha256Cache::load_or_create()
}
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheFileStatus {
    Present,
}

pub enum CacheEntryStatus {
    Fresh,
}

pub struct Sha256Cache;

impl Sha256Cache {
    pub fn load_or_create() -> (Self, CacheFileStatus) {
        (Self, CacheFileStatus::Present)
    }

    pub fn get_or_update(&mut self) -> CacheEntryStatus {
        CacheEntryStatus::Fresh
    }
}
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/utils/sha256_cache.rs"),
        "expected no suspicious_pub for child types exposed by exported method signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn pub_use_fix_does_not_trigger_when_child_type_is_exposed_by_parent_boundary_signature() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/wikilink")).expect("create src/wikilink");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_use_parent_boundary_signature_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod consumer;
mod wikilink;

fn main() {
    consumer::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() {
    let extracted = crate::wikilink::extract();
    let _ = extracted.valid.len();
}
"#,
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/wikilink.rs"),
        r#"mod wikilink_types;

pub use wikilink_types::{ParsedExtractedWikilinks, ParsedInvalidWikilink};

pub fn extract() -> ParsedExtractedWikilinks {
    ParsedExtractedWikilinks { valid: vec![], invalid: vec![] }
}
"#,
    )
    .expect("write wikilink facade");
    fs::write(
        temp.path().join("src/wikilink/wikilink_types.rs"),
        r#"pub struct ParsedExtractedWikilinks {
    pub valid: Vec<String>,
    pub invalid: Vec<ParsedInvalidWikilink>,
}

pub struct ParsedInvalidWikilink;
"#,
    )
    .expect("write child module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "suspicious_pub"
                && finding.path == "src/wikilink/wikilink_types.rs"),
        "expected no suspicious_pub for child types exposed by parent boundary signatures, got: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn fix_pub_use_rewrites_obsidian_report_style_private_parent_use_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_obsidian_report_style_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod report;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        "mod frontmatter_issues_report;\nmod invalid_wikilink_report;\nmod report_writer;\n\npub use report_writer::{ReportDefinition, ReportWriter};\nuse report_writer::DescriptionBuilder;\n\npub fn parent_local() {\n    let _ = DescriptionBuilder::new();\n}\n",
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n\nimpl DescriptionBuilder {\n    pub fn new() -> Self { Self }\n}\n",
    )
    .expect("write report writer child");
    fs::write(
        temp.path().join("src/report/frontmatter_issues_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write frontmatter report child");
    fs::write(
        temp.path().join("src/report/invalid_wikilink_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items_again<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write invalid wikilink report child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade = fs::read_to_string(temp.path().join("src/report.rs")).expect("read fixed facade");
    let child = fs::read_to_string(temp.path().join("src/report/report_writer.rs"))
        .expect("read fixed child");
    let frontmatter =
        fs::read_to_string(temp.path().join("src/report/frontmatter_issues_report.rs"))
            .expect("read fixed frontmatter child");
    let invalid = fs::read_to_string(temp.path().join("src/report/invalid_wikilink_report.rs"))
        .expect("read fixed invalid child");

    assert!(!facade.contains("pub use report_writer::{ReportDefinition, ReportWriter};"));
    assert!(facade.contains("use report_writer::DescriptionBuilder;"));
    assert!(child.contains("pub(super) trait ReportDefinition {}"));
    assert!(child.contains("pub(super) struct ReportWriter;"));
    assert!(!child.contains("pub(super) struct DescriptionBuilder;"));
    assert!(frontmatter.contains("use super::DescriptionBuilder;"));
    assert!(frontmatter.contains("use super::report_writer::{ReportDefinition, ReportWriter};"));
    assert!(invalid.contains("use super::DescriptionBuilder;"));
    assert!(invalid.contains("use super::report_writer::{ReportDefinition, ReportWriter};"));
    assert!(!frontmatter.contains("use report::DescriptionBuilder;"));
    assert!(!invalid.contains("use report::DescriptionBuilder;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed obsidian report style fixture");
    assert!(
        check.status.success(),
        "cargo check failed after obsidian report style apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_skips_grouped_pub_use_with_rename_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_rename_skip_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{Thing as RenamedThing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 1 `pub use` fix(es) in dry run"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [ExpectedFinding {
        code:        "suspicious_pub",
        fix_support: FixSupport::FixPubUse,
    }];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}

#[test]
fn already_local_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "already_local_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer")).expect("create src/outer");
    fs::write(temp.path().join("src/lib.rs"), "mod outer;\n").expect("write lib");
    fs::write(
        temp.path().join("src/outer/mod.rs"),
        "mod child;\nmod sibling;\n",
    )
    .expect("write outer mod");
    fs::write(
        temp.path().join("src/outer/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/outer/sibling.rs"),
        "use super::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

#[test]
fn top_level_peer_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "top_level_peer_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod keyboard;\nmod window_event;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/keyboard.rs"),
        "use crate::window_event::write_input_event;\n\npub fn call() { write_input_event(); }\n",
    )
    .expect("write keyboard");
    fs::write(
        temp.path().join("src/window_event.rs"),
        "pub fn write_input_event() {}\n",
    )
    .expect("write window_event");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

#[test]
fn grouped_imports_are_ignored_safely() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "grouped_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(temp.path().join("src/lib.rs"), "mod private_parent;\n").expect("write lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\nuse crate::private_parent::child::{Bar, Baz};\n\nfn use_it(_bar: Bar, _baz: Baz) {}\n",
    )
    .expect("write private_parent mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct Bar;\npub struct Baz;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

#[test]
fn suspicious_pub_is_suppressed_for_parent_facade_used_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "facade_positive_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

use crate::private_parent::PublicContainer;

fn main() {
    let _ = std::mem::size_of::<PublicContainer>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_still_warns_for_parent_facade_unused_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "facade_negative_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 1);
    assert_eq!(report.findings.len(), 1);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["suspicious_pub"]));
}

#[test]
fn suspicious_pub_is_suppressed_for_file_parent_facade_used_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_facade_positive_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

use crate::private_parent::PublicContainer;

fn main() {
    let _ = std::mem::size_of::<PublicContainer>();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 0);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    assert!(report.findings.is_empty());
}

#[test]
fn suspicious_pub_still_warns_for_file_parent_facade_unused_outside_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_facade_negative_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 1);
    assert_eq!(report.findings.len(), 1);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["suspicious_pub"]));
}

#[test]
fn wildcard_parent_pub_use_warns() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "wildcard_parent_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::*;\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("wildcard_parent_pub_use"));
}

mod cargo_mend_tests_support {
    #![allow(dead_code)]
    mod fix_support {
        include!("../src/fix_support.rs");
    }

    mod diagnostics_impl {
        include!("../src/diagnostics.rs");
    }

    pub use diagnostics_impl::*;
    pub use fix_support::FixSummaryBucket;
    pub use fix_support::FixSupport;

    pub const fn diagnostic_specs() -> &'static [DiagnosticSpec] { DIAGNOSTICS }
}
