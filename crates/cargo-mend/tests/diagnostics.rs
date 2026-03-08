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
    findings: Vec<Finding>,
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
    assert!(inner.contains("use self::Thing as LocalThing;"));
    assert!(!inner.contains("use crate::inner::Thing as LocalThing;"));
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
    assert!(mod_rs.contains("pub use self::child::PublicContainer;"));
    assert!(!mod_rs.contains("pub use crate::private_parent::child::PublicContainer;"));
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
    assert!(!inner.contains("use self::Thing as LocalThing;"));
}

mod cargo_vischeck_tests_support {
    #![allow(dead_code)]
    include!("../src/diagnostics.rs");

    pub fn diagnostic_specs() -> &'static [DiagnosticSpec] { DIAGNOSTICS }
}
