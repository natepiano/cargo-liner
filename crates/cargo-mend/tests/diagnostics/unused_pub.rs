use crate::common::*;

#[test]
fn pub_item_used_only_inside_module_subtree_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "unused_pub_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/helpers")).expect("create helpers dir");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers/mod.rs"),
        "mod child;\n\npub fn shared_with_child() {}\n",
    )
    .expect("write helpers mod");
    fs::write(
        temp.path().join("src/helpers/child.rs"),
        "fn call_parent() { super::shared_with_child(); }\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let unused_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .collect();
    assert_eq!(
        unused_findings.len(),
        1,
        "expected exactly one unused_pub finding: {:?}",
        report.findings
    );
    assert_eq!(
        unused_findings[0].item.as_deref(),
        Some("fn shared_with_child")
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::NarrowToPubCrate
                || finding.code == DiagnosticCode::SuspiciousPub),
        "unused_pub should take priority over weaker visibility findings: {:?}",
        report.findings
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn parent_caller_suppresses_unused_pub_and_keeps_pub_crate_narrowing() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "outside_caller_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() { helpers::shared(); }\n",
    )
    .expect("write lib");
    fs::write(temp.path().join("src/helpers.rs"), "pub fn shared() {}\n").expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::UnusedPub),
        "parent caller should suppress unused_pub: {:?}",
        report.findings
    );
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.item.as_deref() == Some("fn shared")),
        "outside in-crate caller should keep narrow_to_pub_crate: {:?}",
        report.findings
    );
}

#[test]
fn library_root_pub_item_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "root_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "pub fn external_api() {}\n").expect("write lib");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.is_empty(),
        "library root public API should be left alone: {:?}",
        report.findings
    );
}

#[test]
fn fix_removes_pub_from_items_and_methods() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "unused_pub_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        r#"pub struct LocalHelper;

impl LocalHelper {
    pub fn local_method(&self) {}
}
"#,
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

    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read helpers");
    assert!(
        helpers.contains("struct LocalHelper;"),
        "struct visibility should be removed: {helpers}"
    );
    assert!(
        helpers.contains("fn local_method(&self) {}"),
        "method visibility should be removed: {helpers}"
    );
    assert!(
        !helpers.contains("pub struct") && !helpers.contains("pub fn"),
        "unused public annotations should be gone: {helpers}"
    );
}
