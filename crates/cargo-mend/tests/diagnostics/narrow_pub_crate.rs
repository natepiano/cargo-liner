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

#[test]
fn integration_test_support_module_is_not_narrowed() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_tests_support_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "").expect("write lib");
    fs::create_dir_all(temp.path().join("tests")).expect("create tests");
    fs::write(temp.path().join("tests/support.rs"), "pub fn helper() {}\n").expect("write support");
    fs::write(
        temp.path().join("tests/consumer.rs"),
        "mod support;\n\n#[test]\nfn uses_support() { support::helper(); }\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--all-targets")
        .arg("--json")
        .output()
        .expect("run cargo-mend --all-targets --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_mend_json_output(&output.stdout);

    let narrow_on_support: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path.ends_with("tests/support.rs")
        })
        .collect();
    assert!(
        narrow_on_support.is_empty(),
        "narrow_to_pub_crate should not fire in tests/: {narrow_on_support:?}",
    );

    let forbidden_on_support: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path.ends_with("tests/support.rs")
        })
        .collect();
    assert!(
        forbidden_on_support.is_empty(),
        "forbidden_pub_crate should not fire on pub items in tests/: {forbidden_on_support:?}",
    );
}

#[test]
fn integration_test_support_module_pub_crate_is_rejected() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "forbidden_tests_support_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "").expect("write lib");
    fs::create_dir_all(temp.path().join("tests")).expect("create tests");
    fs::write(
        temp.path().join("tests/support.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write support");
    fs::write(
        temp.path().join("tests/consumer.rs"),
        "mod support;\n\n#[test]\nfn uses_support() { support::helper(); }\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--all-targets")
        .arg("--json")
        .output()
        .expect("run cargo-mend --all-targets --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_mend_json_output(&output.stdout);

    let has_forbidden = report.findings.iter().any(|finding| {
        finding.code == DiagnosticCode::ForbiddenPubCrate
            && finding.path.ends_with("tests/support.rs")
    });
    assert!(
        has_forbidden,
        "expected forbidden_pub_crate on pub(crate) in tests/support.rs: {:?}",
        report.findings,
    );
}

// Depth boundary for the `ShallowPrivateModule` policy: depth 1 and depth 2
// are shallow (pub(crate) allowed), depth 3+ is nested (pub(crate) forbidden).
// See `resolve_module_location` in src/compiler/visibility/policy.rs.

#[test]
fn pub_crate_at_depth_1_is_allowed() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_1_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write foo");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubCrate)
        .collect();
    assert!(
        forbidden.is_empty(),
        "depth-1 pub(crate) in a private module should be allowed (shallow): {forbidden:?}",
    );
}

#[test]
fn pub_crate_at_depth_2_is_allowed() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_2_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo")).expect("create src/foo");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo/mod.rs"),
        "mod bar;\npub(crate) use bar::helper;\n",
    )
    .expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write foo/bar.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubCrate)
        .collect();
    assert!(
        forbidden.is_empty(),
        "depth-2 pub(crate) in a private module subtree should be allowed (shallow): {forbidden:?}",
    );
}

#[test]
fn pub_crate_at_depth_3_is_forbidden() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_3_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo/mod.rs"),
        "mod bar;\npub(crate) use bar::helper;\n",
    )
    .expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(crate) use baz::helper;\n",
    )
    .expect("write foo/bar/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write foo/bar/baz.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden_count = report
        .findings
        .iter()
        .filter(|f| {
            f.code == DiagnosticCode::ForbiddenPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
        })
        .count();
    assert_eq!(
        forbidden_count, 1,
        "depth-3 pub(crate) in a nested module should be forbidden: {:?}",
        report.findings,
    );
}
