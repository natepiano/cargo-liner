use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use cargo_vischeck_tests_support::diagnostic_specs;
use serde::Deserialize;
use tempfile::tempdir;

fn vischeck_bin() -> PathBuf {
    static BUILD_ONCE: OnceLock<()> = OnceLock::new();
    BUILD_ONCE.get_or_init(|| {
        let status = Command::new("cargo")
            .arg("build")
            .arg("--bin")
            .arg("cargo-vischeck")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("RUSTC_WRAPPER", "")
            .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
            .status()
            .expect("build cargo-vischeck binary for integration tests");
        assert!(status.success(), "failed to build cargo-vischeck test binary");
    });
    let current = std::env::current_exe().expect("current exe path");
    current
        .parent()
        .expect("deps dir")
        .parent()
        .expect("debug dir")
        .join("cargo-vischeck")
}

#[derive(Debug, Deserialize)]
struct Finding {
    code: String,
}

#[derive(Debug, Deserialize)]
struct Report {
    summary: Summary,
    findings: Vec<Finding>,
}

#[derive(Debug, Deserialize)]
struct Summary {
    error_count: usize,
    warning_count: usize,
    fixable_count: usize,
}

fn run_vischeck_json(manifest_path: &std::path::Path) -> Report {
    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--json")
        .output()
        .expect("run cargo-vischeck --json");
    assert!(
        matches!(output.status.code(), Some(0) | Some(1) | Some(2)),
        "cargo-vischeck returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse vischeck json report")
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run cargo-vischeck against fixture");
    assert!(
        matches!(output.status.code(), Some(1) | Some(2)),
        "cargo-vischeck returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Report =
        serde_json::from_slice(&output.stdout).expect("parse vischeck json report");
    let codes: BTreeSet<_> = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    let expected_codes: BTreeSet<_> = diagnostic_specs().iter().map(|spec| spec.code).collect();

    assert_eq!(
        codes, expected_codes,
        "fixture should trigger every diagnostic exactly once"
    );
    assert_eq!(
        report.findings.len(),
        diagnostic_specs().len(),
        "fixture should trigger one finding per diagnostic"
    );
    assert_eq!(report.summary.error_count, 3);
    assert_eq!(report.summary.warning_count, 2);
    assert_eq!(report.summary.fixable_count, 1);

    let rendered_output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo-vischeck human output");
    assert!(
        matches!(rendered_output.status.code(), Some(1) | Some(2)),
        "cargo-vischeck returned unexpected status {:?}: {}",
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
            "https://github.com/natepiano/cargo-vischeck#{}",
            spec.help_anchor
        );
        assert!(
            rendered.contains(&help_url),
            "rendered output is missing help URL for {}",
            spec.code
        );
    }

    assert!(
        rendered.contains(
            "help: consider using just `pub` or removing `pub(crate)` entirely"
        )
    );
    assert!(rendered.contains("help: consider using: `pub(super)`"));
    assert!(
        rendered.contains("help: consider using: `use super::PublicContainer as ParentContainer;`")
    );
    assert!(rendered.contains("summary: 3 error(s), 2 warning(s), 1 fixable with `--fix`"));
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed: {}\n{}",
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed: {}\n{}",
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed: {}\n{}",
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
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod private_parent;\n",
    )
    .expect("write fixture lib");
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed: {}\n{}",
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        !output.status.success(),
        "cargo-vischeck --fix unexpectedly succeeded: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let inner = fs::read_to_string(temp.path().join("src/inner.rs")).expect("read rolled back file");
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("vischeck: no import fixes available"));
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

    let output = Command::new(vischeck_bin())
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-vischeck --fix");
    assert!(
        output.status.success(),
        "cargo-vischeck --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stdout.contains("No findings."));
    assert!(stderr.contains("vischeck: no import fixes available"));
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
    fs::write(temp.path().join("src/outer/child.rs"), "pub struct Thing;\n")
        .expect("write child");
    fs::write(
        temp.path().join("src/outer/sibling.rs"),
        "use super::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write sibling");

    let report = run_vischeck_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.fixable_count, 0);
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

    let report = run_vischeck_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.fixable_count, 0);
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

    let report = run_vischeck_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.fixable_count, 0);
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

mod cargo_vischeck_tests_support {
    #![allow(dead_code)]
    include!("../src/diagnostics.rs");

    pub fn diagnostic_specs() -> &'static [DiagnosticSpec] { DIAGNOSTICS }
}
