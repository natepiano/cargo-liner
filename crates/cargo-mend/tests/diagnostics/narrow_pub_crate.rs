use std::path::Path;

use crate::support::*;

#[test]
fn pub_in_private_top_level_module_is_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() { helpers::internal_fn(); }\n",
    )
    .expect("write lib");
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
fn shallow_public_signature_prevents_pub_crate_narrowing() {
    let temp = tempdir().expect("create shallow signature fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "shallow_signature_floor_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\
         pub fn exposed() -> helpers::PublicSignatureType { helpers::PublicSignatureType }\n\
         pub fn entry() { helpers::safe_to_narrow(); }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub struct PublicSignatureType;\npub fn safe_to_narrow() {}\n",
    )
    .expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            finding.item.as_deref() != Some("struct PublicSignatureType")
                || (finding.code != DiagnosticCode::NarrowToPubCrate
                    && finding.fix_support != FixSupport::NarrowToPubCrate)
        }),
        "a public signature type must not receive pub(crate) narrowing: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.item.as_deref() == Some("fn safe_to_narrow")
        }),
        "missing safe shallow narrowing control: {report:#?}",
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn re_exported_item_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        "mod helpers;\npub use helpers::exported_fn;\n\npub fn entry() { helpers::internal_fn(); }\n",
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
    let exported_flagged: Vec<_> = narrow_findings
        .iter()
        .filter(|f| f.item.as_deref() == Some("fn exported_fn"))
        .collect();
    assert!(
        exported_flagged.is_empty(),
        "re-exported item should not be flagged: {exported_flagged:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|f| f.item.as_deref() == Some("fn internal_fn")),
        "non-exported sibling should still be flagged: {narrow_findings:?}"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_preserves_pub_required_by_private_module_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "private_module_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/planner")).expect("create src/planner");
    fs::create_dir_all(temp.path().join("src/spatial")).expect("create src/spatial");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod planner;\nmod spatial;\n",
    )
    .expect("write lib");
    fs::write(temp.path().join("src/planner/mod.rs"), "mod op;\n").expect("write planner module");
    fs::write(
        temp.path().join("src/planner/op.rs"),
        "pub(crate) use crate::spatial::CrateReexported;\npub use crate::spatial::PubliclyReexported;\n",
    )
    .expect("write planner operation module");
    fs::write(
        temp.path().join("src/spatial/mod.rs"),
        "pub struct CrateReexported;\npub struct PubliclyReexported;\npub struct Unused;\n",
    )
    .expect("write spatial module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings
            .iter()
            .all(|finding| finding.item.as_deref() != Some("struct PubliclyReexported")),
        "bare public re-export target should not be narrowed: {narrow_findings:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|finding| finding.item.as_deref() == Some("struct CrateReexported")),
        "pub(crate) re-export target should still be narrowed: {narrow_findings:?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::UnusedPub
                && finding.item.as_deref() == Some("struct Unused")
        }),
        "unused sibling should still be flagged: {:?}",
        report.findings
    );

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

    let spatial =
        fs::read_to_string(temp.path().join("src/spatial/mod.rs")).expect("read spatial module");
    assert!(
        spatial.contains("pub struct PubliclyReexported;"),
        "bare public re-export target should retain pub: {spatial}"
    );
    assert!(
        spatial.contains("pub(crate) struct CrateReexported;"),
        "pub(crate) re-export target should narrow: {spatial}"
    );
    assert!(
        spatial.contains("struct Unused;") && !spatial.contains("pub struct Unused;"),
        "unused sibling should become private: {spatial}"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_preserves_pub_required_by_public_trait_interface() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "public_trait_interface_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod material;\npub use material::PublicTool;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/material.rs"),
        r#"pub struct PublicState;
pub struct Unused;

macro_rules! define_public_tool {
    () => {
        pub struct PublicTool;

        impl Iterator for PublicTool {
            type Item = PublicState;

            fn next(&mut self) -> Option<PublicState> { None }
        }
    };
}

define_public_tool!();
"#,
    )
    .expect("write material");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .collect();
    assert!(
        narrow_findings
            .iter()
            .all(|finding| finding.item.as_deref() != Some("struct PublicState")),
        "public trait interface type should not be narrowed: {narrow_findings:?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::UnusedPub
                && finding.item.as_deref() == Some("struct Unused")
        }),
        "unused sibling should still be flagged: {:?}",
        report.findings
    );

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

    let material = fs::read_to_string(temp.path().join("src/material.rs")).expect("read material");
    assert!(
        material.contains("pub struct PublicState;"),
        "public trait interface type should retain pub: {material}"
    );
    assert!(
        material.contains("struct Unused;") && !material.contains("pub struct Unused;"),
        "unused sibling should become private: {material}"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn mixed_items_only_non_exported_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        "mod helpers;\npub use helpers::exported_fn;\n\npub fn entry() { helpers::internal_fn(); }\n",
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() { helpers::internal_fn(); }\n",
    )
    .expect("write lib");
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
fn binary_crate_top_level_item_is_narrowed_to_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        "mod helpers;\nfn main() { helpers::internal_fn(); }\n",
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
        "the crate-root caller requires pub(crate): {narrow_findings:?}",
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn dry_run_reports_fix_count() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        "mod helpers;\npub use helpers::exported_fn;\n\npub fn entry() {\n    helpers::internal_fn();\n    let _ = helpers::InternalStruct;\n}\n",
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
fn fix_narrows_a_pub_written_on_its_own_line() {
    // The annotation and the item it applies to need not share a line. The fix
    // must still land, and must replace only the annotation: advertising a fix
    // and then editing nothing is the failure this pins.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_own_line_pub_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() {\n    helpers::internal_fn();\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub\nfn internal_fn() {}\n",
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read fixed helpers"),
        "pub(crate)\nfn internal_fn() {}\n",
    );
}

#[test]
fn fix_narrows_a_tab_indented_pub() {
    // A finding's column is rustc's display column, which charges a tab four
    // columns. Reading it as a byte offset lands past the `pub` and the fix
    // silently never lands.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_tab_indented_pub_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() {\n    helpers::Helper::internal_fn();\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub struct Helper;\n\nimpl Helper {\n\tpub fn internal_fn() {}\n}\n",
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read fixed helpers"),
        "pub(crate) struct Helper;\n\nimpl Helper {\n\tpub(crate) fn internal_fn() {}\n}\n",
    );
}

#[test]
fn methods_on_re_exported_type_are_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        "mod types;\npub use types::MyWidget;\n\npub fn entry() -> i32 {\n    let helper = types::InternalHelper;\n    helper.do_work()\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/types.rs"),
        r#"pub struct MyWidget;

impl MyWidget {
    pub fn exported_method() -> Self { Self }
    pub fn another_method(&self) -> i32 { 42 }
}

pub struct InternalHelper;

impl InternalHelper {
    pub fn do_work(&self) -> i32 { 7 }
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
    let re_exported_type_flagged: Vec<_> = narrow_findings
        .iter()
        .filter(|f| {
            matches!(
                f.item.as_deref(),
                Some("struct MyWidget" | "fn exported_method" | "fn another_method")
            )
        })
        .collect();
    assert!(
        re_exported_type_flagged.is_empty(),
        "methods on re-exported type should not be flagged: {re_exported_type_flagged:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|f| f.item.as_deref() == Some("struct InternalHelper")),
        "non-exported sibling type should still be flagged: {narrow_findings:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|f| f.item.as_deref() == Some("fn do_work")),
        "non-exported sibling method should still be flagged: {narrow_findings:?}"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn type_reachable_via_reexported_enum_variant_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_reachable_variant_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod constants;\nmod types;\npub use constants::DEFAULT_FRAME;\npub use types::Icon;\n\npub fn entry() -> &'static str {\n    let frame = types::InternalFrame;\n    frame.label()\n}\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/constants.rs"),
        r#"use crate::types::FrameCycle;

 pub const DEFAULT_FRAME: FrameCycle = FrameCycle::new(&["a", "b"]);
"#,
    )
    .expect("write constants");
    fs::write(
        temp.path().join("src/types.rs"),
        r#"pub struct FrameCycle {
    frames: &'static [&'static str],
}

impl FrameCycle {
    pub const fn new(frames: &'static [&'static str]) -> Self {
        Self { frames }
    }

    pub fn first(&self) -> &'static str {
        self.frames[0]
    }
}

pub enum Icon {
    Static(&'static str),
    Animated(FrameCycle),
}

pub struct InternalFrame;

impl InternalFrame {
    pub fn label(&self) -> &'static str { "internal" }
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
    let frame_cycle_flagged: Vec<_> = narrow_findings
        .iter()
        .filter(|f| {
            f.item.as_deref() == Some("struct FrameCycle") || f.item.as_deref() == Some("fn first")
        })
        .collect();
    assert!(
        frame_cycle_flagged.is_empty(),
        "struct FrameCycle and its `first` method are reachable through the re-exported \
         `Icon::Animated` variant and the `DEFAULT_FRAME` const, so they must not be flagged \
         for narrowing: {frame_cycle_flagged:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|f| f.item.as_deref() == Some("struct InternalFrame")),
        "non-exported sibling type should still be flagged: {narrow_findings:?}"
    );
    assert!(
        narrow_findings
            .iter()
            .any(|f| f.item.as_deref() == Some("fn label")),
        "non-exported sibling method should still be flagged: {narrow_findings:?}"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn methods_on_non_exported_type_are_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod helpers;\n\npub fn entry() -> i32 {\n    let helper = helpers::InternalHelper;\n    helper.do_work()\n}\n",
    )
    .expect("write lib");
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
fn pub_at_depth_3_is_narrowed_when_parent_caps_at_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "narrow_nested_fixture"
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
        "mod baz;\npub(crate) use baz::helper;\n\
         pub(crate) fn use_helper() { let _ = helper; }\n",
    )
    .expect("write foo/bar/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        "pub fn helper() {}\n",
    )
    .expect("write foo/bar/baz.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let narrow_count = report
        .findings
        .iter()
        .filter(|f| {
            f.code == DiagnosticCode::NarrowToPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
        })
        .count();
    assert_eq!(
        narrow_count, 1,
        "bare `pub` at depth 3 should be flagged for narrowing when the parent facade re-exports \
         as `pub(crate) use`: {:?}",
        report.findings,
    );
}

#[test]
fn restricted_facade_and_crate_signature_join_to_pub_crate_narrowing() {
    let temp = tempdir().expect("create joined reach fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "joined_facade_signature_narrowing_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create nested modules");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod a;\n\
         pub use a::public_signature;\n\
         pub fn exercise() { let _ = a::crate_wide_signature(); a::exercise_equal(); }\n",
    )
    .expect("write library root");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\n\
         pub(crate) use b::crate_wide_signature;\n\
         pub use b::public_signature;\n\
         pub(crate) fn exercise_equal() { let _ = b::equal_signature(); }\n",
    )
    .expect("write signature carriers");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\n\
         pub(super) use c::{CrateWide, Equal, Public};\n\
         pub fn crate_wide_signature() -> CrateWide { CrateWide }\n\
         pub(super) fn equal_signature() -> Equal { Equal }\n\
         pub fn public_signature() -> Public { Public }\n",
    )
    .expect("write restricted facade and equal signature");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub struct CrateWide;\npub struct Equal;\npub struct Public;\n",
    )
    .expect("write facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.item.as_deref() == Some("struct CrateWide")
        }),
        "the joined crate-wide requirement must narrow CrateWide: {report:#?}",
    );
    for retained_item in ["struct Equal", "struct Public"] {
        assert!(
            report.findings.iter().all(|finding| {
                finding.code != DiagnosticCode::NarrowToPubCrate
                    || finding.item.as_deref() != Some(retained_item)
            }),
            "the equal and public controls must retain bare pub for {retained_item}: {report:#?}",
        );
    }
    assert_summary_matches_findings(&report);
}

#[test]
fn nested_public_signature_exceeds_pub_crate_facade_floor() {
    let temp = tempdir().expect("create nested signature fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nested_signature_floor_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create nested modules");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod a;\n\
         pub use a::make;\n\
         fn retain_target(_: a::Target) {}\n\
         pub fn entry() { a::safe_to_narrow(); }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\n\
         pub use b::make;\n\
         pub(crate) use b::{Target, safe_to_narrow};\n",
    )
    .expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\n\
         pub use c::make;\n\
         pub(crate) use c::{Target, safe_to_narrow};\n",
    )
    .expect("write parent module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub struct Target;\n\
         pub fn make() -> Target { Target }\n\
         pub fn safe_to_narrow() {}\n",
    )
    .expect("write facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            finding.item.as_deref() != Some("struct Target")
                || (finding.code != DiagnosticCode::NarrowToPubCrate
                    && finding.fix_support != FixSupport::NarrowToPubCrate)
        }),
        "a public signature type must exceed its pub(crate) facade floor: {report:#?}",
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.item.as_deref() == Some("fn safe_to_narrow")
        }),
        "missing safe nested narrowing control: {report:#?}",
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn super_to_crate_chain_narrows_only_the_crate_wide_item() {
    let temp = tempdir().expect("create super to crate consumer fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_to_crate_consumer_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("mend.toml"),
        r#"[visibility]
allow_pub_mod = ["src/a/b/c.rs"]
pub_in_path = "permitted"
"#,
    )
    .expect("write fixture visibility config");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod a;\nfn use_crate_wide() { a::crate_wide(); }\n",
    )
    .expect("write library root");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\npub(crate) use b::crate_wide;\n\
         fn use_parent_only() { b::parent_only(); }\n",
    )
    .expect("write outer crate facade and consumers");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(crate) use c::d::crate_wide;\n\
         pub(super) use c::d::parent_only;\n",
    )
    .expect("write crate and parent facade hops");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) mod d;\npub(super) use d::{crate_wide, parent_only};\n",
    )
    .expect("write nearest super facade hops");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub fn crate_wide() {}\npub fn parent_only() {}\n",
    )
    .expect("write facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path == "src/a/b/c/d.rs"
                && finding.item.as_deref() == Some("fn crate_wide")
        }),
        "the super to crate chain must narrow its crate-wide item: {report:#?}"
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path == "src/a/b/c/d.rs"
                && finding.item.as_deref() == Some("fn parent_only")
        }),
        "the super-only chain must keep its narrower boundary: {report:#?}"
    );
}

#[test]
fn binary_pub_at_depth_3_is_narrowed_when_parent_caps_at_pub_crate() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "binary_narrow_nested_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/main.rs"), "mod foo;\nfn main() {}\n").expect("write main");
    fs::write(temp.path().join("src/foo/mod.rs"), "mod bar;\n").expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(crate) use baz::helper;\n\
         pub(crate) fn use_helper() { let _ = helper; }\n",
    )
    .expect("write foo/bar/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        "pub fn helper() {}\n",
    )
    .expect("write foo/bar/baz.rs");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path.ends_with("src/foo/bar/baz.rs")
        }),
        "a binary parent facade capped at pub(crate) must request pub(crate): {report:#?}"
    );
}

#[test]
fn fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code() {
    // The `cargo fix` invocation underneath `cargo mend --fix-compiler`
    // binds a localhost TCP socket for its diagnostic server. Sandboxed
    // runners (style-fix worktrees, restricted CI) refuse that bind with
    // `Operation not permitted (os error 1)`. Skip when we detect we're
    // in such a sandbox so the failure does not block automated runs.
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_compiler_does_not_remove_reexport_used_only_by_cfg_test_code: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    // Regression test for the cfg(test) reachability bug. The `pub use` in
    // lib.rs is referenced only from `#[cfg(test)] mod tests`. Under
    // lib-only compilation rustc emits `unused_imports` because cfg(test)
    // is stripped. Today, `cargo mend --fix-compiler` chains `cargo fix`,
    // which deletes the re-export — and then the test target stops
    // compiling because `crate::helper` no longer resolves.
    //
    // After the redesign, mend's analysis pass must compile under
    // `--all-targets` so the test caller is visible and rustc does NOT emit
    // the `unused_imports` warning. The re-export must survive.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_test_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/inner_facade")).expect("create src/inner_facade");
    // The `pub use` lives inside a private parent module, so its visibility
    // is effectively private — exactly the case where rustc fires the
    // `unused_imports` lint when the only callers are stripped by cfg.
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod inner_facade;\n\
         pub fn entry() { inner_facade::live() }\n\
         \n\
         #[cfg(test)]\n\
         mod tests {\n\
             #[test]\n\
             fn calls_helper() { crate::inner_facade::helper(7); }\n\
         }\n",
    )
    .expect("write lib.rs");
    fs::write(
        temp.path().join("src/inner_facade/mod.rs"),
        "mod child;\n\
         pub use child::helper;\n\
         \n\
         pub fn live() {}\n",
    )
    .expect("write inner_facade/mod.rs");
    fs::write(
        temp.path().join("src/inner_facade/child.rs"),
        "pub fn helper(_n: i32) {}\n",
    )
    .expect("write inner_facade/child.rs");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-compiler")
        .output()
        .expect("run cargo-mend --fix-compiler");
    assert!(
        output.status.success(),
        "cargo-mend --fix-compiler failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade_after = fs::read_to_string(temp.path().join("src/inner_facade/mod.rs"))
        .expect("read inner_facade/mod.rs");
    assert!(
        facade_after.contains("pub use child::helper"),
        "the re-export reached only from #[cfg(test)] must NOT be removed; inner_facade/mod.rs after:\n{facade_after}",
    );

    // And the project must still compile under `cargo nextest run --no-run` —
    // i.e. mend left the tree in a state where every target builds.
    let test_build = std::process::Command::new("cargo")
        .arg("nextest")
        .arg("run")
        .arg("--no-run")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo nextest run --no-run");
    assert!(
        test_build.status.success(),
        "test target must still compile after --fix-compiler:\n{}\n{}",
        String::from_utf8_lossy(&test_build.stdout),
        String::from_utf8_lossy(&test_build.stderr)
    );
}

#[test]
fn fix_compiler_keeps_reexport_used_only_by_feature_gated_module() {
    // `cargo fix` starts an internal diagnostic server. Sandboxed runners
    // can reject its localhost socket, so skip this integration test when
    // the harness identifies that environment.
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_compiler_keeps_reexport_used_only_by_feature_gated_module: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create feature-gated re-export fixture dir");

    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_feature_reexport_fixture"
version = "0.1.0"
edition = "2024"

[features]
hidden = []
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a")).expect("create src/a");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\n\
         #[cfg(feature = \"hidden\")]\n\
         mod hidden;\n\
         \n\
         fn main() {\n\
             #[cfg(feature = \"hidden\")]\n\
             hidden::use_it();\n\
         }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\n",
    )
    .expect("write a");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n").expect("write child");
    fs::write(
        temp.path().join("src/hidden.rs"),
        "pub fn use_it() {\n    let _thing: crate::a::Thing = crate::a::Thing;\n}\n",
    )
    .expect("write hidden");

    let git_init = std::process::Command::new("git")
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("initialize fixture git repository");
    assert!(
        git_init.status.success(),
        "git init failed:\n{}\n{}",
        String::from_utf8_lossy(&git_init.stdout),
        String::from_utf8_lossy(&git_init.stderr)
    );

    assert_feature_fixture_compiles(
        temp.path(),
        &[],
        "default feature set before --fix-compiler",
    );
    assert_feature_fixture_compiles(
        temp.path(),
        &["--features", "hidden"],
        "hidden feature before --fix-compiler",
    );

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-compiler")
        .output()
        .expect("run cargo-mend --fix-compiler");
    assert!(
        output.status.success(),
        "cargo-mend --fix-compiler failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade_after = fs::read_to_string(temp.path().join("src/a.rs")).expect("read a");
    assert!(
        facade_after.contains("pub use child::Thing"),
        "the re-export used only by #[cfg(feature = \"hidden\")] must remain; a.rs after:\n{facade_after}",
    );

    assert_feature_fixture_compiles(temp.path(), &[], "default feature set after --fix-compiler");
    assert_feature_fixture_compiles(
        temp.path(),
        &["--features", "hidden"],
        "hidden feature after --fix-compiler",
    );
}

#[test]
fn fix_compiler_keeps_reexport_used_by_negated_feature_gate() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_compiler_keeps_reexport_used_by_negated_feature_gate: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create negated-feature re-export fixture dir");

    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_negated_feature_reexport_fixture"
version = "0.1.0"
edition = "2024"

[features]
default = ["a"]
a = []
b = []
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a")).expect("create src/a");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod a;

macro_rules! emit_feature_gated_items {
    () => {
        #[cfg(all(feature = "a", not(feature = "b")))]
        mod active_without_b;

        fn main() {
            #[cfg(all(feature = "a", not(feature = "b")))]
            active_without_b::use_it();
        }
    };
}

emit_feature_gated_items!();
"#,
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\n",
    )
    .expect("write a");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n").expect("write child");
    fs::write(
        temp.path().join("src/active_without_b.rs"),
        "pub fn use_it() {\n    let _thing: crate::a::Thing = crate::a::Thing;\n}\n",
    )
    .expect("write negated-feature module");

    let git_init = std::process::Command::new("git")
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("initialize fixture git repository");
    assert!(
        git_init.status.success(),
        "git init failed:\n{}\n{}",
        String::from_utf8_lossy(&git_init.stdout),
        String::from_utf8_lossy(&git_init.stderr)
    );

    assert_feature_fixture_compiles(
        temp.path(),
        &[],
        "default feature set before --fix-compiler",
    );

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-compiler")
        .output()
        .expect("run cargo-mend --fix-compiler");
    assert!(
        output.status.success(),
        "cargo-mend --fix-compiler failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let facade_after = fs::read_to_string(temp.path().join("src/a.rs")).expect("read a");
    assert!(
        facade_after.contains("pub use child::Thing"),
        "the re-export used by a negated feature gate must remain; a.rs after:\n{facade_after}",
    );
    assert_feature_fixture_compiles(temp.path(), &[], "default feature set after --fix-compiler");
}

fn assert_feature_fixture_compiles(fixture_dir: &Path, cargo_args: &[&str], configuration: &str) {
    let output = std::process::Command::new("cargo")
        .arg("check")
        .args(cargo_args)
        .current_dir(fixture_dir)
        .output()
        .expect("check feature fixture");
    assert!(
        output.status.success(),
        "{configuration} must compile:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn fix_does_not_narrow_pub_fn_used_only_from_cfg_test_caller() {
    // Regression for the cross-compilation merge issue: a `pub fn` whose
    // only outside-of-parent-subtree caller lives in a `#[cfg(test)] mod
    // tests` block must NOT be narrowed to `pub(super)`. The lib
    // compilation strips cfg(test) and sees no external callers, so it
    // would emit a `suspicious_pub` finding. The lib-test compilation
    // sees the test caller and emits no finding. Mend's
    // cross-compilation merge takes the intersection for narrowing-style
    // findings, so the lib's finding is suppressed.
    //
    // Likewise, the parent re-export in `panes/mod.rs` is referenced by
    // the test caller via `super::panes::cpu_required_pane_height(...)`,
    // so it must NOT be flagged as a stale internal facade.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cross_compile_merge_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create src/tui/panes");
    fs::write(temp.path().join("src/lib.rs"), "mod tui;\n").expect("write lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod panes;\nmod render;\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\npub use cpu::cpu_required_pane_height;\npub(crate) use cpu::compute;\n\npub(crate) fn use_compute() { let _ = compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let _ = crate::tui::panes::cpu_required_pane_height(12); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs")
                    && f.item.as_deref() == Some("pub use cpu_required_pane_height"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only from #[cfg(test)] callers must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
    let compute_flagged = report.findings.iter().any(|f| {
        (matches!(
            f.code,
            DiagnosticCode::NarrowToPubCrate | DiagnosticCode::SuspiciousPub
        ) && f.path.contains("panes/cpu.rs")
            && f.item.as_deref() == Some("fn compute"))
            || (f.code == DiagnosticCode::InternalParentPubUseFacade
                && f.path.contains("panes/mod.rs")
                && f.item.as_deref() == Some("pub use compute"))
    });
    assert!(
        compute_flagged,
        "`compute` should still be reported inside the fixture: {report:#?}",
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_does_not_narrow_pub_fn_for_cfg_test_gated_pub_super_reexport() {
    // A `#[cfg(test)] pub(super) use ...` re-export in the parent module
    // remains reachable from a `#[cfg(test)] mod tests` caller outside the
    // parent subtree. Mend must not flag the underlying `pub fn` as
    // narrowable or the re-export as removable.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "binary_cfg_test_pub_super_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create dirs");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod tui;\npub fn entry() { tui::entry() }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod panes;\nmod render;\npub(super) fn entry() { panes::entry(); }\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\n#[cfg(test)]\npub(super) use cpu::cpu_required_pane_height;\npub(crate) use cpu::compute;\n\npub(super) fn entry() { let _ = compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { let _ = crate::tui::panes::cpu_required_pane_height(12); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs")
                    && f.item.as_deref() == Some("pub use cpu_required_pane_height"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only from #[cfg(test)] callers must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
    let compute_flagged = report.findings.iter().any(|f| {
        (matches!(
            f.code,
            DiagnosticCode::NarrowToPubCrate | DiagnosticCode::SuspiciousPub
        ) && f.path.contains("panes/cpu.rs")
            && f.item.as_deref() == Some("fn compute"))
            || (f.code == DiagnosticCode::InternalParentPubUseFacade
                && f.path.contains("panes/mod.rs")
                && f.item.as_deref() == Some("pub use compute"))
    });
    assert!(
        compute_flagged,
        "`compute` should still be reported inside the fixture: {report:#?}",
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_does_not_narrow_pub_fn_called_only_from_cfg_test_assert_macro() {
    // The hard case: the cfg(test) test caller invokes the function
    // *inside* an `assert_eq!` macro. syn's AST walker doesn't descend
    // into macro tokens, so without macro-aware analysis the source-level
    // facade scanner reports the re-export as "unused" and the analyzer
    // proposes narrowing the function plus removing the re-export. That
    // fix breaks the test build (E0425).
    //
    // This test passes when either (a) the source-level scanner walks
    // macro token streams, or (b) HIR-level reachability is used.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "macro_caller_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/panes")).expect("create dirs");
    fs::write(temp.path().join("src/lib.rs"), "mod tui;\n").expect("write lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod panes;\nmod render;\n",
    )
    .expect("write tui/mod");
    fs::write(
        temp.path().join("src/tui/panes/mod.rs"),
        "mod cpu;\n#[cfg(test)]\npub(super) use cpu::cpu_required_pane_height;\npub(crate) use cpu::compute;\n\npub(crate) fn use_compute() { let _ = compute(0); }\n",
    )
    .expect("write panes/mod");
    fs::write(
        temp.path().join("src/tui/panes/cpu.rs"),
        "pub fn cpu_required_pane_height(_n: u16) -> u16 { compute(_n) }\npub fn compute(_n: u16) -> u16 { 1 }\n",
    )
    .expect("write panes/cpu");
    // Caller invokes the function inside an assert_eq! — the path lives
    // in the macro token stream, not in the parsed AST.
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert_eq!(crate::tui::panes::cpu_required_pane_height(12), 1); }\n}\n",
    )
    .expect("write tui/render");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));

    let bad_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| {
            (f.code == DiagnosticCode::SuspiciousPub
                && f.path.contains("panes/cpu.rs")
                && f.item.as_deref() == Some("fn cpu_required_pane_height"))
                || (f.code == DiagnosticCode::InternalParentPubUseFacade
                    && f.path.contains("panes/mod.rs")
                    && f.item.as_deref() == Some("pub use cpu_required_pane_height"))
        })
        .collect();
    assert!(
        bad_findings.is_empty(),
        "items reachable only via a macro-wrapped #[cfg(test)] caller must not be flagged for narrowing or pub-use removal; got: {bad_findings:#?}",
    );
    let compute_flagged = report.findings.iter().any(|f| {
        (matches!(
            f.code,
            DiagnosticCode::NarrowToPubCrate | DiagnosticCode::SuspiciousPub
        ) && f.path.contains("panes/cpu.rs")
            && f.item.as_deref() == Some("fn compute"))
            || (f.code == DiagnosticCode::InternalParentPubUseFacade
                && f.path.contains("panes/mod.rs")
                && f.item.as_deref() == Some("pub use compute"))
    });
    assert!(
        compute_flagged,
        "`compute` should still be reported inside the fixture: {report:#?}",
    );
    assert_summary_matches_findings(&report);
}

/// A `pub use` in a sibling module pins its target at `pub`: rustc's E0364
/// requires a re-exported item to be at least as visible as the re-export,
/// whether or not the re-export itself is reachable from outside. Only a parent
/// facade travels with the declaration when the narrowing fixers rewrite it, so
/// a sibling `pub use` must leave the declaration alone while a sibling with no
/// such re-export still narrows.
#[test]
fn fix_preserves_pub_required_by_sibling_reexport_in_nested_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Required);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "sibling_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/tui/columns")).expect("create src/tui/columns");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod tui;\n\nfn main() { println!(\"{}\", tui::total()); }\n",
    )
    .expect("write binary root");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod columns;\nmod render;\n\npub(crate) fn total() -> usize { render::width() }\n",
    )
    .expect("write tui module");
    fs::write(
        temp.path().join("src/tui/render.rs"),
        "use super::columns::DOUBLE_PINNED;\nuse super::columns::NARROWABLE;\nuse super::columns::PINNED;\n\npub(super) fn width() -> usize { NARROWABLE + PINNED + DOUBLE_PINNED }\n",
    )
    .expect("write render module");
    fs::write(
        temp.path().join("src/tui/columns/mod.rs"),
        "mod constants;\nmod project;\n\npub(super) use self::constants::NARROWABLE;\npub(super) use self::constants::PINNED;\npub(super) use self::project::DOUBLE_PINNED;\n",
    )
    .expect("write columns module");
    fs::write(
        temp.path().join("src/tui/columns/constants.rs"),
        "pub const NARROWABLE: usize = 1;\npub const PINNED: usize = 7;\n",
    )
    .expect("write columns constants");
    fs::write(
        temp.path().join("src/tui/columns/project.rs"),
        "pub use super::constants::PINNED;\n\npub(in crate::tui) const DOUBLE_PINNED: usize = PINNED * 2;\n",
    )
    .expect("write columns project module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let pinned = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("const PINNED")
        })
        .unwrap_or_else(|| panic!("pinned constant should still be reported: {report:#?}"));
    assert_eq!(
        AdvertisedFix::from_notes(pinned.help.iter().map(String::as_str)),
        AdvertisedFix::NotOffered,
        "sibling `pub use` pins the declaration, so no fix may be advertised: {pinned:#?}"
    );

    let narrowable = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.item.as_deref() == Some("const NARROWABLE")
        })
        .unwrap_or_else(|| panic!("unpinned sibling should still be reported: {report:#?}"));
    assert_eq!(
        AdvertisedFix::from_notes(narrowable.help.iter().map(String::as_str)),
        AdvertisedFix::WithFix,
        "sibling without a public re-export should still be fixable: {narrowable:#?}"
    );
    assert_summary_matches_findings(&report);

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

    let constants = fs::read_to_string(temp.path().join("src/tui/columns/constants.rs"))
        .expect("read columns constants");
    assert!(
        constants.contains("pub const PINNED: usize = 7;"),
        "sibling `pub use` target should retain pub: {constants}"
    );
    assert!(
        !constants.contains("pub const NARROWABLE"),
        "unpinned sibling should still be narrowed: {constants}"
    );
}
