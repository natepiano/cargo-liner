#![allow(clippy::expect_used)]
#![allow(clippy::needless_raw_string_hashes)]
#![allow(clippy::struct_field_names)]
#![allow(clippy::too_many_lines)]

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

use cargo_mend_tests_support::diagnostic_specs;
use serde::Deserialize;
use tempfile::tempdir;

fn mend_bin() -> PathBuf {
    static BUILD_ONCE: OnceLock<()> = OnceLock::new();
    BUILD_ONCE.get_or_init(|| {
        let status = Command::new("cargo")
            .arg("build")
            .arg("--bin")
            .arg("cargo-mend")
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .env("RUSTC_WRAPPER", "")
            .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
            .status()
            .expect("build cargo-mend binary for integration tests");
        assert!(status.success(), "failed to build cargo-mend test binary");
    });
    let current = std::env::current_exe().expect("current exe path");
    current
        .parent()
        .expect("deps dir")
        .parent()
        .expect("debug dir")
        .join("cargo-mend")
}

#[derive(Debug, Deserialize)]
struct Finding {
    code: String,
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

fn run_mend_json(manifest_path: &std::path::Path) -> Report {
    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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
    assert_eq!(report.summary.error_count, 3);
    assert_eq!(report.summary.warning_count, 5);
    assert_eq!(report.summary.fixable_with_fix_count, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 1);

    let rendered_output = Command::new(mend_bin())
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
    assert!(rendered.contains(
        "summary: 3 error(s), 5 warning(s), 1 fixable with `--fix`, 1 fixable with `--fix-pub-use`"
    ));
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let follow_up = Command::new(mend_bin())
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

    let repeat_fix = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\npub use child::SpawnStats;\n\nfn keep(_stats: SpawnStats) {}\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");

    let output = Command::new(mend_bin())
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

    let output = Command::new(mend_bin())
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
fn fix_pub_use_skips_unsupported_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_skip_fixture"
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
        "mod child;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    let output = Command::new(mend_bin())
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
    assert!(stderr.contains("mend: no `pub use` fixes available"));
}

#[test]
fn fix_pub_use_skips_multiline_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_multiline_grouped_skip_fixture"
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
        "mod child;\npub use child::{\n    Thing,\n    Other,\n};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    let output = Command::new(mend_bin())
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
    assert!(
        stderr
            .contains("mend: suppressing `unused import` warning during `--fix-pub-use` discovery")
    );
    assert!(stderr.contains("mend: no `pub use` fixes available"));
    assert!(!stderr.contains("warning: unused imports: `Thing` and `Other`"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
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

    pub const fn diagnostic_specs() -> &'static [DiagnosticSpec] { DIAGNOSTICS }
}
