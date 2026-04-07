use crate::common::*;

#[test]
fn pub_in_private_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_eq!(narrow_findings[0].item.as_deref(), Some("fn internal_fn"));
    assert_summary_matches_findings(&report);
}

#[test]
fn re_exported_item_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings.is_empty(),
        "re-exported item should not be flagged: {narrow_findings:?}"
    );
}

#[test]
fn mixed_items_only_non_exported_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_mixed_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\npub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(narrow_findings.len(), 1);
    assert_eq!(narrow_findings[0].item.as_deref(), Some("fn internal_fn"));
    assert_summary_matches_findings(&report);
}

#[test]
fn mod_rs_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_modrs_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/helpers")).expect("create src/helpers");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers/mod.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers mod.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding for mod.rs, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn binary_crate_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_bin_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod helpers;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert_eq!(
        narrow_findings.len(),
        1,
        "expected 1 narrow_to_pub_crate finding in binary crate, got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn dry_run_reports_fix_count() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_dryrun_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn first() {}\npub fn second() {}\n",
    )
    .expect("write helpers");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix --dry-run");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains('2'),
        "expected dry-run to mention 2 fixes: {combined}"
    );

    // Verify files were NOT modified
    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read helpers");
    assert!(
        helpers.contains("pub fn first()"),
        "dry-run should not modify files"
    );
}

#[test]
fn fix_replaces_pub_with_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\npub use helpers::exported_fn;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn exported_fn() {}\npub fn internal_fn() {}\npub struct InternalStruct;\n",
    )
    .expect("write helpers");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read fixed file");
    assert!(
        helpers.contains("pub fn exported_fn()"),
        "re-exported item should stay `pub`: {helpers}"
    );
    assert!(
        helpers.contains("pub(crate) fn internal_fn()"),
        "non-exported fn should be narrowed to pub(crate): {helpers}"
    );
    assert!(
        helpers.contains("pub(crate) struct InternalStruct"),
        "non-exported struct should be narrowed to pub(crate): {helpers}"
    );
}

#[test]
fn methods_on_re_exported_type_are_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_impl_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod types;\npub use types::MyWidget;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/types.rs"),
        r#"pub struct MyWidget;

impl MyWidget {
    pub fn exported_method() -> Self { Self }
    pub fn another_method(&self) -> i32 { 42 }
}
"#,
    )
    .expect("write types");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings.is_empty(),
        "methods on re-exported type should not be flagged: {narrow_findings:?}"
    );
}

#[test]
fn methods_on_non_exported_type_are_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_impl_internal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        r#"pub struct InternalHelper;

impl InternalHelper {
    pub fn do_work(&self) -> i32 { 42 }
}
"#,
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    // Both the struct and its method should be flagged
    assert_eq!(
        narrow_findings.len(),
        2,
        "expected 2 narrow_to_pub_crate findings (struct + method), got {}: {narrow_findings:?}",
        narrow_findings.len(),
    );
    assert_summary_matches_findings(&report);
}
