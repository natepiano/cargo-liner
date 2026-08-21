use std::path::Path;

use crate::support::*;

fn assert_prefer_module_fixture_compiles(manifest_path: &Path) {
    let check = cargo_command()
        .arg("check")
        .arg("--all-targets")
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
        .expect("check prefer-module-import fixture");
    assert!(
        check.status.success(),
        "fixture must compile before mend: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_all_preserves_cfg_on_rewritten_module_import() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_all_preserves_cfg_on_rewritten_module_import: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create cfg-gated import fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_gated_module_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source dir");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod consumer;\nmod process_observation;\n\nfn main() {\n    #[cfg(not(test))]\n    consumer::signal();\n}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/process_observation.rs"),
        "pub(crate) mod identity {\n    pub(crate) fn revalidate() {}\n}\n",
    )
    .expect("write identity module");
    fs::write(
        temp.path().join("src/consumer.rs"),
        "#[cfg(not(test))]\nuse crate::process_observation::identity::revalidate;\n\n#[cfg(not(test))]\npub(crate) fn signal() {\n    revalidate();\n}\n",
    )
    .expect("write consumer");
    let manifest_path = temp.path().join("Cargo.toml");
    assert_prefer_module_fixture_compiles(&manifest_path);

    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--fix-all")
        .output()
        .expect("run cargo-mend --fix-all");
    assert!(
        output.status.success(),
        "cargo-mend --fix-all failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed consumer");
    assert!(
        consumer.contains("#[cfg(not(test))]\nuse crate::process_observation::identity;"),
        "the module import must retain the function import's cfg gate:\n{consumer}"
    );
    assert!(
        consumer.contains("identity::revalidate();"),
        "the call must use the rewritten module import:\n{consumer}"
    );
}

#[test]
fn compiler_fix_validation_restores_cfg_incomplete_edit() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping compiler_fix_validation_restores_cfg_incomplete_edit: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create compiler rollback fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "compiler_fix_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source dir");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod consumer;\nmod process_observation;\n\nfn main() {\n    #[cfg(not(test))]\n    consumer::signal();\n}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/process_observation.rs"),
        "pub(crate) mod identity {\n    pub(crate) fn revalidate() {}\n}\n",
    )
    .expect("write identity module");
    let consumer_path = temp.path().join("src/consumer.rs");
    let original = "use crate::process_observation::identity;\n\n#[cfg(not(test))]\npub(crate) fn signal() {\n    identity::revalidate();\n}\n";
    fs::write(&consumer_path, original).expect("write consumer");
    let manifest_path = temp.path().join("Cargo.toml");
    assert_prefer_module_fixture_compiles(&manifest_path);

    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--fix-compiler")
        .output()
        .expect("run cargo-mend --fix-compiler");
    assert!(
        !output.status.success(),
        "cfg-incomplete cargo fix must fail validation"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compiler failed after applying compiler fixes; changes were rolled back"),
        "expected compiler-fix rollback message, got:\n{stderr}"
    );
    let restored = fs::read_to_string(consumer_path).expect("read restored consumer");
    assert_eq!(
        restored, original,
        "failed compiler-fix validation must restore its source edits"
    );
}

#[test]
fn basic_function_import_rewrite() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "prefer_module_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn example() -> i32 {
    do_thing()
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("use crate::parent::utils;") || consumer.contains("use super::utils;"),
        "expected module import, got:\n{consumer}"
    );
    assert!(
        consumer.contains("utils::do_thing()"),
        "expected qualified call, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::do_thing;"),
        "function import should be removed, got:\n{consumer}"
    );
}

#[test]
fn multiple_references_all_qualified() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "multi_ref_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn first() -> i32 { do_thing() }
fn second() -> i32 { do_thing() }
fn third() -> i32 { do_thing() }
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    let count = consumer.matches("utils::do_thing()").count();
    assert_eq!(count, 3, "expected 3 qualified calls, got:\n{consumer}");
}

#[test]
fn super_path_rewrite() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_path_fixture"
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
        "mod child;\nmod sibling;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        r#"use super::child::do_thing;

fn example() -> i32 { do_thing() }
"#,
    )
    .expect("write sibling");

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

    let sibling =
        fs::read_to_string(temp.path().join("src/parent/sibling.rs")).expect("read fixed file");
    assert!(
        sibling.contains("use super::child;"),
        "expected module import, got:\n{sibling}"
    );
    assert!(
        sibling.contains("child::do_thing()"),
        "expected qualified call, got:\n{sibling}"
    );

    // Idempotency: running again should produce zero findings
    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "fix should be idempotent — second run should have no prefer_module_import findings, got: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.code == DiagnosticCode::PreferModuleImport)
            .map(|f| &f.path)
            .collect::<Vec<_>>()
    );
}

#[test]
fn multiple_functions_same_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "multi_func_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn func_a() -> i32 { 1 }\npub fn func_b() -> i32 { 2 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::func_a;
use crate::parent::utils::func_b;

fn example() -> i32 { func_a() + func_b() }
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    // Should have one module import (possibly deduplicated) and qualified calls
    assert!(
        consumer.contains("utils::func_a()"),
        "expected qualified call to func_a, got:\n{consumer}"
    );
    assert!(
        consumer.contains("utils::func_b()"),
        "expected qualified call to func_b, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::func_a;"),
        "function import for func_a should be removed, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::func_b;"),
        "function import for func_b should be removed, got:\n{consumer}"
    );
}

#[test]
fn skips_type_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "skip_type_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::child::MyType;\n\nfn use_it(_thing: MyType) {}\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "PascalCase imports should not be flagged as prefer_module_import"
    );
}

#[test]
fn skips_constant_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "skip_const_fixture"
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
        "mod constants;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/constants.rs"),
        "pub const MAX_SIZE: usize = 100;\n",
    )
    .expect("write constants");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::constants::MAX_SIZE;\n\nfn use_it() -> usize { MAX_SIZE }\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "UPPER_SNAKE_CASE imports should not be flagged as prefer_module_import"
    );
}

#[test]
fn skips_grouped_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "skip_grouped_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn func_a() -> i32 { 1 }\npub fn func_b() -> i32 { 2 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::utils::{func_a, func_b};\n\nfn use_it() -> i32 { func_a() + func_b() }\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "grouped imports should not be flagged as prefer_module_import"
    );
}

#[test]
fn skips_renamed_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "skip_rename_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::utils::do_thing as other;\n\nfn use_it() -> i32 { other() }\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "renamed imports should not be flagged as prefer_module_import"
    );
}

#[test]
fn skips_std_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "skip_std_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"use std::mem::swap;

fn main() {
    let mut a = 1;
    let mut b = 2;
    swap(&mut a, &mut b);
}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "std imports should not be flagged as prefer_module_import"
    );
}

#[test]
fn dry_run_no_edits() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dry_run_prefer_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn example() -> i32 { do_thing() }
"#,
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

    // File should be unchanged
    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(
        consumer.contains("use crate::parent::utils::do_thing;"),
        "dry-run should not modify files"
    );
}

#[test]
fn read_only_reports_findings() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "readonly_prefer_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn example() -> i32 { do_thing() }
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "read-only mode should report prefer_module_import findings"
    );

    // File should be unchanged
    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after read-only");
    assert!(
        consumer.contains("use crate::parent::utils::do_thing;"),
        "read-only mode should not modify files"
    );
}

#[test]
fn nothing_to_fix() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nothing_prefer_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "clean project should not have prefer_module_import findings"
    );
}

#[test]
fn function_used_as_value() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "value_ref_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing(_x: i32) -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn example() -> i32 {
    let values = vec![1, 2, 3];
    values.into_iter().map(do_thing).sum()
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains(".map(utils::do_thing)"),
        "function reference as value should be qualified, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::do_thing;")
            && !consumer.contains("use super::utils::do_thing;"),
        "function import should be removed, got:\n{consumer}"
    );
}

#[test]
fn super_path_multiple_functions_same_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_multi_func_fixture"
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
        "mod types;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/types.rs"),
        r#"pub struct Obstacle;
pub fn is_point_blocked(_pos: i32, _obs: &[Obstacle]) -> bool { false }
pub fn is_segment_blocked(_start: i32, _end: i32, _obs: &[Obstacle]) -> bool { false }
"#,
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use super::types::Obstacle;
use super::types::is_point_blocked;
use super::types::is_segment_blocked;

fn example(obs: &[Obstacle]) -> bool {
    is_point_blocked(0, obs) || is_segment_blocked(0, 1, obs)
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    // Should have the type import preserved, one module import, and qualified calls
    assert!(
        consumer.contains("use super::types::Obstacle;"),
        "type import should be preserved, got:\n{consumer}"
    );
    assert!(
        consumer.contains("use super::types;"),
        "expected module import for types, got:\n{consumer}"
    );
    assert!(
        consumer.contains("types::is_point_blocked("),
        "expected qualified call to is_point_blocked, got:\n{consumer}"
    );
    assert!(
        consumer.contains("types::is_segment_blocked("),
        "expected qualified call to is_segment_blocked, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use super::types::is_point_blocked;"),
        "function import for is_point_blocked should be removed, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use super::types::is_segment_blocked;"),
        "function import for is_segment_blocked should be removed, got:\n{consumer}"
    );
    // Should NOT have bare "use super;" (the over-shortening bug)
    let lines: Vec<&str> = consumer.lines().collect();
    assert!(
        !lines.iter().any(|line| line.trim() == "use super;"),
        "should not produce bare 'use super;', got:\n{consumer}"
    );
}

#[test]
fn two_segment_super_module_import_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "two_seg_super_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use super::utils;

fn example() -> i32 { utils::do_thing() }
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "`use super::module;` should not be flagged as prefer_module_import"
    );
}

#[test]
fn project_config_disables_prefer_module_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "config_disable_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("mend.toml"),
        r#"[diagnostics]
prefer_module_import = false

[visibility]
pub_in_path = "permitted"
"#,
    )
    .expect("write mend.toml");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::do_thing;

fn example() -> i32 { do_thing() }
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "disabled diagnostic should produce no findings"
    );
}

#[test]
fn skips_super_super_module_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_super_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/extras/visualization")).expect("create dirs");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod extras;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/extras.rs"),
        "mod support;\nmod visualization;\n",
    )
    .expect("write extras mod");
    fs::write(
        temp.path().join("src/extras/support.rs"),
        "pub fn helper() -> i32 { 42 }\npub struct CameraBasis;\n",
    )
    .expect("write support");
    fs::write(
        temp.path().join("src/extras/visualization.rs"),
        "mod convex_hull;\n",
    )
    .expect("write visualization mod");
    fs::write(
        temp.path().join("src/extras/visualization/convex_hull.rs"),
        r#"use super::super::support;
use super::super::support::CameraBasis;

fn example(_basis: CameraBasis) -> i32 { support::helper() }
"#,
    )
    .expect("write convex_hull");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let false_positives: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::PreferModuleImport && f.path.contains("convex_hull"))
        .collect();
    assert!(
        false_positives.is_empty(),
        "`use super::super::module;` should not be flagged, got: {:?}",
        false_positives.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn skips_function_import_when_mod_declared_in_same_file() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "mod_conflict_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod input;

use crate::input::button_zoom_just_pressed;

fn main() { button_zoom_just_pressed(); }
"#,
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/input.rs"),
        "pub fn button_zoom_just_pressed() {}\n",
    )
    .expect("write input");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "function import should not be flagged when `mod` declaration exists in same file"
    );
}

#[test]
fn skips_crate_path_module_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "crate_path_module_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent/nested")).expect("create dirs");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/parent/mod.rs"),
        "mod nested;\nmod consumer;\npub mod support;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/support.rs"),
        "pub fn helper() -> i32 { 42 }\n",
    )
    .expect("write support");
    fs::write(
        temp.path().join("src/parent/nested/mod.rs"),
        "mod leaf;\npub mod child_support;\n",
    )
    .expect("write nested mod");
    fs::write(
        temp.path().join("src/parent/nested/child_support.rs"),
        "pub fn nested_helper() -> i32 { 7 }\n",
    )
    .expect("write child_support");
    fs::write(
        temp.path().join("src/parent/nested/leaf.rs"),
        "use crate::parent::support;\nuse crate::parent::nested::child_support;\n\nfn example() -> i32 { support::helper() + child_support::nested_helper() }\n",
    )
    .expect("write leaf");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::support;\n\nfn example() -> i32 { support::helper() }\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "crate:: path importing a module should not be flagged as prefer_module_import, got: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.code == DiagnosticCode::PreferModuleImport)
            .map(|f| &f.path)
            .collect::<Vec<_>>()
    );
}

#[test]
fn fix_qualifies_bare_refs_inside_macros() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "macro_ref_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        r#"#[derive(Debug, PartialEq)]
pub enum Status { Ready, NotReady }

pub fn check_status() -> Status { Status::Ready }
"#,
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils::check_status;
use crate::parent::utils::Status;

fn example() -> bool {
    matches!(check_status(), Status::Ready)
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("utils::check_status()"),
        "expected qualified call inside matches!, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::check_status;")
            && !consumer.contains("use super::utils::check_status;"),
        "function import should be removed, got:\n{consumer}"
    );
}

#[test]
fn deletes_function_import_when_module_already_imported() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "already_imported_fixture"
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
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn format_bytes(bytes: u64) -> String { format!(\"{bytes}\") }\npub fn truncate() -> i32 { 0 }\n",
    )
    .expect("write utils");
    // The module is already imported (used by `truncate`), and the function is
    // also imported separately. Rewriting the function import to
    // `use crate::parent::utils;` would duplicate the existing module import
    // (E0252), so the function import must be deleted instead.
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::utils;
use crate::parent::utils::format_bytes;

fn example() -> String {
    let _ = utils::truncate();
    format!("{}", format_bytes(42))
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    // The existing module import is kept (the shorten-import rule may rewrite it
    // to the `super::` sibling form), and it must appear exactly once — the
    // redundant function import is deleted rather than rewritten to a duplicate.
    let module_import_count = consumer.matches("use crate::parent::utils;").count()
        + consumer.matches("use super::utils;").count();
    assert_eq!(
        module_import_count, 1,
        "module import must be kept exactly once, not duplicated, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::parent::utils::format_bytes;")
            && !consumer.contains("use super::utils::format_bytes;"),
        "redundant function import should be removed, got:\n{consumer}"
    );
    assert!(
        consumer.contains("utils::format_bytes(42)"),
        "call site should be qualified, got:\n{consumer}"
    );
}

/// Two deep imports whose modules share a leaf name each want `use controller;`.
/// The combining layer drops both before anything is written, so reporting them
/// named the same two fixes on every run and applied neither — the shape that
/// left `--fix-all` printing identical output forever.
#[test]
fn skips_function_imports_whose_modules_share_a_leaf_name() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "leaf_name_collision_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/orbit_cam")).expect("create src/orbit_cam");
    fs::create_dir_all(temp.path().join("src/free_cam")).expect("create src/free_cam");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod free_cam;\nmod installation;\nmod orbit_cam;\nfn main() { installation::install(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/orbit_cam/mod.rs"),
        "pub(crate) mod controller;\n",
    )
    .expect("write orbit_cam mod");
    fs::write(
        temp.path().join("src/orbit_cam/controller.rs"),
        "pub(crate) fn install_orbit() {}\n",
    )
    .expect("write orbit_cam::controller");
    fs::write(
        temp.path().join("src/free_cam/mod.rs"),
        "pub(crate) mod controller;\n",
    )
    .expect("write free_cam mod");
    fs::write(
        temp.path().join("src/free_cam/controller.rs"),
        "pub(crate) fn install_free() {}\n",
    )
    .expect("write free_cam::controller");
    let installation = temp.path().join("src/installation.rs");
    fs::write(
        &installation,
        r"use crate::free_cam::controller::install_free;
use crate::orbit_cam::controller::install_orbit;

pub(crate) fn install() {
    install_orbit();
    install_free();
}
",
    )
    .expect("write installation");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "a leaf-name collision must not be reported as fixable: {report:#?}"
    );

    let before = fs::read_to_string(&installation).expect("read installation");
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
    let after = fs::read_to_string(&installation).expect("read installation after fix");
    assert_eq!(
        before, after,
        "both colliding imports must be left untouched, got:\n{after}"
    );
}

#[test]
fn skips_function_import_when_name_collides_with_other_module() {
    // Two distinct modules share the bare name `geometry`:
    //   - `crate::overlay::geometry` (imported as `use super::geometry;`)
    //   - `crate::geometry` (source of the function `extract_vertices`)
    // Rewriting the function import to `use crate::geometry;` would collide with
    // the existing `geometry` import (E0252) and misroute the call (E0425), so
    // mend must leave the function import untouched.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "name_collision_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/geometry")).expect("create src/geometry");
    fs::create_dir_all(temp.path().join("src/overlay/geometry"))
        .expect("create src/overlay/geometry");
    fs::create_dir_all(temp.path().join("src/overlay/render")).expect("create src/overlay/render");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod geometry;\nmod overlay;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/geometry/mod.rs"),
        "pub(crate) fn extract_vertices() -> i32 { 0 }\n",
    )
    .expect("write crate::geometry");
    fs::write(
        temp.path().join("src/overlay/mod.rs"),
        "mod geometry;\nmod render;\n",
    )
    .expect("write overlay mod");
    fs::write(
        temp.path().join("src/overlay/geometry/mod.rs"),
        "pub(crate) struct Edge;\n",
    )
    .expect("write overlay::geometry");
    fs::write(
        temp.path().join("src/overlay/render/mod.rs"),
        "mod bounds;\n",
    )
    .expect("write render mod");
    fs::write(
        temp.path().join("src/overlay/render/bounds.rs"),
        r#"use super::super::geometry;
use super::super::geometry::Edge;
use crate::geometry::extract_vertices;

fn example() -> (Edge, i32) {
    let _ = geometry::Edge;
    (Edge, extract_vertices())
}
"#,
    )
    .expect("write bounds");

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

    let bounds =
        fs::read_to_string(temp.path().join("src/overlay/render/bounds.rs")).expect("read fixed");
    assert!(
        bounds.contains("use crate::geometry::extract_vertices;"),
        "colliding function import must be left untouched, got:\n{bounds}"
    );
    assert!(
        !bounds.contains("use crate::geometry;"),
        "must not introduce a duplicate `geometry` module import, got:\n{bounds}"
    );
    assert!(
        bounds.contains("extract_vertices()") && !bounds.contains("geometry::extract_vertices()"),
        "call site must stay bare, got:\n{bounds}"
    );
}

#[test]
fn inline_call_inserts_use_and_qualifies() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_call_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod layout;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/layout.rs"),
        "pub fn set_root_grow_height(_tree: &mut i32) {}\n",
    )
    .expect("write layout");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example() {
    let mut tree = 0;
    crate::parent::layout::set_root_grow_height(&mut tree);
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("use crate::parent::layout;") || consumer.contains("use super::layout;"),
        "expected module import to be inserted, got:\n{consumer}"
    );
    assert!(
        consumer.contains("layout::set_root_grow_height(&mut tree)"),
        "expected qualified call, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("crate::parent::layout::set_root_grow_height")
            && !consumer.contains("super::layout::set_root_grow_height"),
        "fully-qualified call should be rewritten, got:\n{consumer}"
    );

    // Idempotency: a second run should report no inline-call findings
    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "fix should be idempotent — second run should have no prefer_module_import findings"
    );
}

#[test]
fn inline_call_reuses_existing_module_use() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_reuse_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod layout;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/layout.rs"),
        "pub fn set_root_grow_height(_tree: &mut i32) {}\n",
    )
    .expect("write layout");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use super::layout;

fn example() {
    let mut tree = 0;
    crate::parent::layout::set_root_grow_height(&mut tree);
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("layout::set_root_grow_height(&mut tree)"),
        "expected qualified call, got:\n{consumer}"
    );
    // The pre-existing `use super::layout;` should be the only module import;
    // no duplicate insertion
    let use_count = consumer.matches("use super::layout;").count()
        + consumer.matches("use crate::parent::layout;").count();
    assert_eq!(
        use_count, 1,
        "should not duplicate module import, got:\n{consumer}"
    );
}

#[test]
fn inline_call_skipped_when_mod_declared_same_file() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_mod_conflict_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod layout;

fn main() {
    let mut tree = 0;
    crate::layout::set_root_grow_height(&mut tree);
}
"#,
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/layout.rs"),
        "pub fn set_root_grow_height(_tree: &mut i32) {}\n",
    )
    .expect("write layout");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "inline call should not be flagged when `mod` declaration exists in same file"
    );
}

#[test]
fn inline_call_skipped_inside_nested_mod_block() {
    // Regression: the fixer used to insert `use super::layout;` at file top
    // while rewriting the call site inside `mod tests`. At file top `super`
    // means a different module than inside the nested `mod tests`, so the
    // inserted use is unused and the nested call site loses its binding.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_nested_mod_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod layout;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/layout.rs"),
        "pub fn set_root_grow_height(_tree: &mut i32) {}\n",
    )
    .expect("write layout");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example() {}

#[cfg(test)]
mod tests {
    #[test]
    fn calls_layout() {
        let mut tree = 0;
        crate::parent::layout::set_root_grow_height(&mut tree);
    }
}
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "inline call inside a nested `mod` block should not be flagged — \
         scope would break if the use were inserted at file top"
    );

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

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read consumer");
    assert!(
        consumer.contains("crate::parent::layout::set_root_grow_height"),
        "nested-mod call site should be left untouched, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use super::layout;")
            && !consumer.contains("use crate::parent::layout;"),
        "no use should be inserted at file top, got:\n{consumer}"
    );
}

#[test]
fn function_use_inside_nested_mod_shortens_against_nested_path() {
    // Regression (bevy_lagrange): a `use crate::parent::utils::do_thing;`
    // inside `mod tests` was being rewritten to `use super::utils;`. The
    // detector treated the file's module path as the current path and
    // ignored the inline `mod tests`, so `up_count` was off by one and
    // `super` resolved to the wrong parent at the use site, producing
    // E0432: unresolved import.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nested_use_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod utils;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write utils");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example() {}

#[cfg(test)]
mod tests {
    use crate::parent::utils::do_thing;

    #[test]
    fn calls_it() {
        assert_eq!(do_thing(), 42);
    }
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read consumer");
    assert!(
        !consumer.contains("use super::utils;"),
        "must not shorten to `super::utils` from inside `mod tests` — \
         `super` there points at `parent::consumer`, not `parent`. Got:\n{consumer}"
    );
    assert!(
        consumer.contains("use crate::parent::utils;")
            || consumer.contains("use super::super::utils;"),
        "expected the use to stay absolute (or use `super::super`), got:\n{consumer}"
    );
    assert!(
        consumer.contains("utils::do_thing()"),
        "expected qualified call, got:\n{consumer}"
    );
}

#[test]
fn skips_bare_ref_shadowed_by_local_binding() {
    // Regression: when a `let NAME = ...;` binding shadows an imported
    // function with the same name, later bare references to NAME refer to
    // the local, not the function. The fixer used to rewrite every bare
    // ident match to `module::NAME`, producing `fn item` where `f32` was
    // expected and triggering rollback on compile.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "shadow_local_fixture"
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
        "mod scaling;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/scaling.rs"),
        "pub fn dot_radius(_a: f32, _b: f32) -> f32 { 1.0 }\n",
    )
    .expect("write scaling");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use super::scaling::dot_radius;

fn consume(_x: f32) {}
fn apply_minus(_x: f32) -> f32 { 0.0 }

fn example(font_size: f32, scale: f32) -> f32 {
    let dot_radius = dot_radius(font_size, scale);
    consume(dot_radius);
    apply_minus(-dot_radius)
}
"#,
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
        "cargo-mend --fix failed (rollback expected when bug is present): {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("use super::scaling;"),
        "expected module import, got:\n{consumer}"
    );
    assert!(
        consumer.contains("let dot_radius = scaling::dot_radius(font_size, scale);"),
        "let RHS (the actual function call) should be qualified, got:\n{consumer}"
    );
    assert!(
        consumer.contains("consume(dot_radius);"),
        "bare reference to local must NOT be qualified, got:\n{consumer}"
    );
    assert!(
        consumer.contains("apply_minus(-dot_radius)"),
        "unary-minus over local must NOT be qualified, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("consume(scaling::dot_radius)")
            && !consumer.contains("-scaling::dot_radius"),
        "must not rewrite local-variable references, got:\n{consumer}"
    );
}

#[test]
fn skips_struct_literal_field_shorthand() {
    // Regression: struct literal field shorthand `Foo { name }` requires a
    // bare ident (it's both the field name and the value local). Replacing
    // the value with `module::name` produces a parse error. The fixer must
    // leave shorthand inits alone (or expand them to `name: module::name`),
    // not blindly rewrite the bare token.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "shorthand_init_fixture"
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
        "mod scaling;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/scaling.rs"),
        "pub fn dot_radius(_a: f32, _b: f32) -> f32 { 1.0 }\n",
    )
    .expect("write scaling");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use super::scaling::dot_radius;

pub struct ArrowGeometry {
    pub dot_radius: f32,
    pub origin_y:   f32,
}

fn build(font_size: f32, scale: f32, origin_y: f32) -> ArrowGeometry {
    let dot_radius = dot_radius(font_size, scale);
    ArrowGeometry {
        dot_radius,
        origin_y,
    }
}
"#,
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
        "cargo-mend --fix failed (rollback expected when bug is present): {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/parent/consumer.rs")).expect("read fixed file");
    // Shorthand must survive intact: not `scaling::dot_radius,` in the literal.
    assert!(
        !consumer.contains("scaling::dot_radius,"),
        "shorthand init must not be rewritten to a qualified path, got:\n{consumer}"
    );
    // The function call on the let RHS should still be qualified.
    assert!(
        consumer.contains("let dot_radius = scaling::dot_radius(font_size, scale);"),
        "let RHS function call should be qualified, got:\n{consumer}"
    );
}

/// Inline call where the target module IS the file's own parent.
///
/// `parent/child.rs` calling `crate::parent::do_thing(...)` would shorten to a
/// degenerate `use super;` — invalid Rust. The fix instead rewrites the call
/// to `super::do_thing(...)` and emits no `use` statement.
#[test]
fn inline_call_to_parent_module_uses_super() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "parent_inline_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod source;\npub(crate) use source::do_thing;\nmod child;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/source.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write source");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        r#"fn example() -> i32 {
    crate::parent::do_thing()
}
"#,
    )
    .expect("write child");

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

    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed file");
    assert!(
        child.contains("super::do_thing()"),
        "expected `super::do_thing()`, got:\n{child}"
    );
    assert!(
        !child.contains("use super;"),
        "must not insert invalid `use super;`, got:\n{child}"
    );
    assert!(
        !child.contains("use crate::parent;") && !child.contains("use super::parent;"),
        "must not insert an import for the file's own parent, got:\n{child}"
    );
    assert!(
        !child.contains("crate::parent::do_thing"),
        "fully-qualified call should be rewritten, got:\n{child}"
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "fix should be idempotent — second run should have no prefer_module_import findings"
    );
}

/// Existing `use crate::parent::do_thing;` import inside `parent/child.rs`:
/// the import is dropped entirely and bare `do_thing(...)` calls become
/// `super::do_thing(...)`.
#[test]
fn function_import_from_parent_module_drops_use() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "parent_use_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod source;\npub(crate) use source::do_thing;\nmod child;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/source.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write source");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        r#"use crate::parent::do_thing;

fn first() -> i32 { do_thing() }
fn second() -> i32 { do_thing() + 1 }
"#,
    )
    .expect("write child");

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

    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed file");
    assert!(
        !child.contains("use crate::parent::do_thing"),
        "function-import line should be deleted, got:\n{child}"
    );
    assert!(
        !child.contains("use super;") && !child.contains("use crate::parent;"),
        "must not insert a parent-module import, got:\n{child}"
    );
    assert!(
        child.contains("super::do_thing()"),
        "first reference should become `super::do_thing()`, got:\n{child}"
    );
    assert!(
        child.matches("super::do_thing()").count() >= 2,
        "both references should be rewritten, got:\n{child}"
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "fix should be idempotent — second run should have no prefer_module_import findings"
    );
}

/// A parent-module function import referenced from inside an inline
/// `#[cfg(test)] mod tests`. There `super` is the file's own module, not the
/// file's parent, so the rewrite needs `super::super::fn(...)`. A single
/// `super::` made the fixed code fail to compile on the lib test target
/// (E0425) and mend rolled everything back.
#[test]
fn parent_module_reference_inside_inline_test_mod() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "parent_test_mod_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(temp.path().join("src/lib.rs"), "mod parent;\n").expect("write lib");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod source;\npub(crate) use source::do_thing;\nmod child;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/source.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write source");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        r#"use crate::parent::do_thing;

pub(super) fn example() -> i32 { do_thing() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_through_glob() {
        assert_eq!(do_thing(), example());
    }
}
"#,
    )
    .expect("write child");

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

    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed file");
    assert!(
        !child.contains("use crate::parent::do_thing"),
        "function-import line should be deleted, got:\n{child}"
    );
    assert!(
        child.contains("fn example() -> i32 { super::do_thing() }"),
        "file-level reference should become `super::do_thing()`, got:\n{child}"
    );
    assert!(
        child.contains("super::super::do_thing()"),
        "reference inside `mod tests` should become `super::super::do_thing()`, got:\n{child}"
    );
}

/// Two separate parent-module function imports in the same file. Both `use`
/// lines must be deleted and every reference rewritten to `super::fn(...)`.
#[test]
fn parent_module_multiple_function_imports() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "parent_multi_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod source;\npub(crate) use source::do_thing;\npub(crate) use source::other_thing;\nmod child;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/source.rs"),
        "pub fn do_thing() -> i32 { 42 }\npub fn other_thing() -> i32 { 7 }\n",
    )
    .expect("write source");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        r#"use crate::parent::do_thing;
use crate::parent::other_thing;

fn example() -> i32 { do_thing() + other_thing() }
"#,
    )
    .expect("write child");

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

    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed file");
    assert!(
        !child.contains("use crate::parent::do_thing")
            && !child.contains("use crate::parent::other_thing"),
        "both function-import lines should be deleted, got:\n{child}"
    );
    assert!(
        !child.contains("use super;") && !child.contains("use crate::parent;"),
        "must not insert a parent-module import, got:\n{child}"
    );
    assert!(
        child.contains("super::do_thing()") && child.contains("super::other_thing()"),
        "every reference should be rewritten with `super::`, got:\n{child}"
    );
}

/// In the same file, mix one parent-module call and one sibling-module call.
/// The parent target gets `super::fn(...)` with no `use`. The sibling target
/// follows the standard treatment: a sibling `use` import + module-prefixed call.
#[test]
fn parent_and_sibling_inline_calls_in_same_file() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "parent_sibling_mix_fixture"
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
    .expect("write main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod source;\npub(crate) use source::parent_fn;\nmod sibling;\nmod child;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/source.rs"),
        "pub fn parent_fn() -> i32 { 1 }\n",
    )
    .expect("write source");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "pub fn sibling_fn() -> i32 { 2 }\n",
    )
    .expect("write sibling");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        r#"fn example() -> i32 {
    crate::parent::parent_fn() + crate::parent::sibling::sibling_fn()
}
"#,
    )
    .expect("write child");

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

    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed file");
    assert!(
        child.contains("super::parent_fn()"),
        "parent-target call should become `super::parent_fn()`, got:\n{child}"
    );
    assert!(
        child.contains("use super::sibling;") || child.contains("use crate::parent::sibling;"),
        "sibling target should add a sibling module import, got:\n{child}"
    );
    assert!(
        child.contains("sibling::sibling_fn()"),
        "sibling target should be rewritten with module prefix, got:\n{child}"
    );
    assert!(
        !child.contains("use super;"),
        "must not insert invalid `use super;`, got:\n{child}"
    );
    assert!(
        !child.contains("crate::parent::parent_fn")
            && !child.contains("crate::parent::sibling::sibling_fn"),
        "fully-qualified calls should be rewritten, got:\n{child}"
    );
}

/// Regression (`hana_tool_graph`): `use crate::constants::parameter_fields;`
/// imports an inline `pub mod parameter_fields { ... }` declared inside
/// `constants.rs`. The module check used to look only for
/// `constants/parameter_fields.rs` / `.../mod.rs` on disk, so the `snake_case`
/// module name was misclassified as a function import and rewritten to
/// `use crate::constants;`, orphaning every `parameter_fields::CONST` use site.
#[test]
fn skips_import_of_inline_mod_in_parent_file() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_mod_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod constants;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/constants.rs"),
        r#"pub mod parameter_fields {
    pub const GROUP_MIX: &str = "mix";
}
"#,
    )
    .expect("write constants");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"use crate::constants::parameter_fields;

pub fn run() -> &'static str {
    parameter_fields::GROUP_MIX
}
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "importing an inline `mod` block from its parent's file must not be \
         flagged as a function import"
    );
}

/// Counterpart of the inline-mod skip: an inline *call* into a function that
/// lives in an inline `mod` block gets the standard treatment — insert a
/// module `use` and qualify the call — now that the module check can see
/// inline `mod` declarations.
#[test]
fn inline_call_into_inline_mod_rewrites_with_module_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_mod_call_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("mend.toml"),
        r#"[diagnostics]
review_pub_mod = false

[visibility]
pub_in_path = "permitted"
"#,
    )
    .expect("write mend.toml");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod constants;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/constants.rs"),
        r#"pub mod helpers {
    pub fn compute() -> i32 { 7 }
}
"#,
    )
    .expect("write constants");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() -> i32 {
    crate::constants::helpers::compute()
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("use crate::constants::helpers;"),
        "expected a module import for the inline mod, got:\n{consumer}"
    );
    assert!(
        consumer.contains("helpers::compute()")
            && !consumer.contains("crate::constants::helpers::compute()"),
        "expected qualified call, got:\n{consumer}"
    );
}

/// Regression (`hana_tool_graph` `fusion.rs`): a `use crate::effect::fn;` inside
/// `mod tests` was rewritten to `use crate::effect;`, and that planned rewrite
/// suppressed the top-level `use crate::effect;` insertion needed by an inline
/// call at file top level — the tests-scope import doesn't bind there (E0433).
#[test]
fn inline_call_use_inserted_when_nested_mod_rewrites_same_module() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nested_rewrite_suppression_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod effect;\nmod consumer;\nfn main() { consumer::run(); }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/effect.rs"),
        "pub fn classify() {}\npub fn register() {}\n",
    )
    .expect("write effect");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn run() {
    crate::effect::classify();
}

#[cfg(test)]
mod tests {
    use crate::effect::register;

    #[test]
    fn calls_register() {
        register();
    }
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed file");
    assert!(
        consumer.contains("effect::classify()") && !consumer.contains("crate::effect::classify()"),
        "top-level call should be qualified with the bare module, got:\n{consumer}"
    );
    let use_count = consumer.matches("use crate::effect;").count();
    assert_eq!(
        use_count, 2,
        "expected a top-level `use crate::effect;` AND one inside `mod tests`, got:\n{consumer}"
    );
    assert!(
        consumer.contains("effect::register()"),
        "tests-scope reference should be qualified, got:\n{consumer}"
    );
}

/// Two function imports of the same module — one at file top level, one inside
/// `mod tests` — must each be rewritten in place. The file-global dedup used to
/// rewrite the first and delete the second, leaving the tests scope with
/// qualified calls but no binding (E0433).
#[test]
fn function_imports_same_module_top_level_and_in_nested_mod() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cross_scope_dedup_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod effect;\nmod consumer;\nfn main() { consumer::run(); }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/effect.rs"),
        "pub fn classify() {}\npub fn register() {}\n",
    )
    .expect("write effect");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"use crate::effect::classify;

pub fn run() {
    classify();
}

#[cfg(test)]
mod tests {
    use crate::effect::register;

    #[test]
    fn calls_register() {
        register();
    }
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed file");
    let use_count = consumer.matches("use crate::effect;").count();
    assert_eq!(
        use_count, 2,
        "each scope needs its own module import, got:\n{consumer}"
    );
    assert!(
        consumer.contains("effect::classify()") && consumer.contains("effect::register()"),
        "both references should be qualified, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::effect::classify;")
            && !consumer.contains("use crate::effect::register;"),
        "both function imports should be rewritten, got:\n{consumer}"
    );
}

/// A top-level `use crate::effect;` does not bind inside `mod tests`, so a
/// function import inside the nested mod must be rewritten in place — not
/// deleted as "already imported" (which strands the tests-scope references).
#[test]
fn nested_mod_function_import_rewrites_when_module_imported_at_top_level() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nested_already_imported_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod effect;\nmod consumer;\nfn main() { consumer::run(); }\n",
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/effect.rs"),
        "pub fn classify() {}\npub fn register() {}\n",
    )
    .expect("write effect");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"use crate::effect;

pub fn run() {
    effect::classify();
}

#[cfg(test)]
mod tests {
    use crate::effect::register;

    #[test]
    fn calls_register() {
        register();
    }
}
"#,
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
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed file");
    let use_count = consumer.matches("use crate::effect;").count();
    assert_eq!(
        use_count, 2,
        "the nested import must be rewritten in place, not deleted, got:\n{consumer}"
    );
    assert!(
        consumer.contains("effect::register()"),
        "tests-scope reference should be qualified, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("use crate::effect::register;"),
        "the nested function import should be rewritten, got:\n{consumer}"
    );
}

#[test]
fn skips_functions_named_by_an_attribute() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "attr_named_fixture"
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
        "mod defaults;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/defaults.rs"),
        "pub fn make_default() -> i32 { 1 }\n",
    )
    .expect("write defaults");
    // `#[deprecated(note = "...")]` stands in for `#[serde(default = "...")]`:
    // both name a function in a string literal that no path visitor can reach,
    // and only the inert one keeps this fixture dependency-free.
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::defaults::make_default;

#[deprecated(note = "make_default")]
pub struct Legacy;

fn example() -> i32 {
    make_default()
}
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::PreferModuleImport),
        "a function named by an attribute string must not be flagged: {:?}",
        report.findings
    );
}

/// The inline-call rewrite inserts `use module;` at file scope. When the only
/// call sites sit under a `#[cfg]`, that import must repeat the gate — an
/// ungated `use` of a configured-out module is E0432.
#[test]
fn statement_cfg_gate_reaches_inserted_module_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_call_cfg_fixture"
version = "0.1.0"
edition = "2024"

[features]
test = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "#[cfg(any(test, feature = \"test\"))]\nmod adapter;\nmod plugin;\nfn main() { plugin::build(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/adapter.rs"),
        "pub fn install_topology() {}\n",
    )
    .expect("write adapter module");
    fs::write(
        temp.path().join("src/plugin.rs"),
        r#"pub fn build() {
    #[cfg(any(test, feature = "test"))]
    crate::adapter::install_topology();
}
"#,
    )
    .expect("write plugin module");

    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));

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

    let plugin = fs::read_to_string(temp.path().join("src/plugin.rs")).expect("read fixed plugin");
    assert!(
        plugin.contains("#[cfg(any(test, feature = \"test\"))]\nuse crate::adapter;"),
        "expected the inserted module import to inherit the statement's cfg, got:\n{plugin}"
    );

    for cargo_arguments in [&[][..], &["--features", "test"][..]] {
        let check = cargo_command()
            .arg("check")
            .arg("--all-targets")
            .args(cargo_arguments)
            .arg("--manifest-path")
            .arg(temp.path().join("Cargo.toml"))
            .output()
            .expect("check fixed inline-call fixture");
        assert!(
            check.status.success(),
            "fixed fixture failed for cargo arguments {cargo_arguments:?}: {}\n{}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
    }
}

fn write_lib_fixture(directory: &Path, package: &str, modules: &[(&str, &str)]) {
    fs::write(
        directory.join("Cargo.toml"),
        format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect("write fixture manifest");
    fs::create_dir_all(directory.join("src")).expect("create fixture src");
    for (name, contents) in modules {
        fs::write(directory.join("src").join(name), contents).expect("write fixture module");
    }
}

fn run_mend_fix(manifest_path: &Path) {
    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn method_call_in_macro_keeps_its_bare_method_name() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    write_lib_fixture(
        temp.path(),
        "macro_method_fixture",
        &[
            ("lib.rs", "mod ledger;\nmod sequence;\n"),
            (
                "ledger.rs",
                r"pub struct Progress(f32);

impl Progress {
    pub const fn normalized(self) -> f32 { self.0 }
}

pub fn normalized(elapsed: f32, total: f32) -> f32 { elapsed / total }
",
            ),
            (
                "sequence.rs",
                r"use crate::ledger::Progress;
use crate::ledger::normalized;

pub fn span(elapsed: f32, total: f32) -> f32 { normalized(elapsed, total) }

pub fn check(progress: Progress) { assert_eq!(progress.normalized(), 0.5); }
",
            ),
        ],
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));

    run_mend_fix(&temp.path().join("Cargo.toml"));

    let sequence =
        fs::read_to_string(temp.path().join("src/sequence.rs")).expect("read fixed sequence");
    assert!(
        sequence.contains("ledger::normalized(elapsed, total)"),
        "expected the free-function call to be module qualified, got:\n{sequence}"
    );
    assert!(
        sequence.contains("progress.normalized()"),
        "an inherent method named like the import must stay bare inside a macro, got:\n{sequence}"
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));
}

#[test]
fn local_fn_in_inline_module_shadows_the_imported_function() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    write_lib_fixture(
        temp.path(),
        "shadowed_fn_fixture",
        &[
            ("lib.rs", "mod plugin;\nmod reconcile;\n"),
            (
                "reconcile.rs",
                "pub fn reconcile(value: u32) -> u32 { value }\n",
            ),
            (
                "plugin.rs",
                r"use crate::reconcile::reconcile;

pub fn build() -> u32 { reconcile(1) }

#[cfg(test)]
mod tests {
    fn reconcile() -> u32 { 7 }

    #[test]
    fn the_local_definition_wins() {
        let staged = reconcile();
        assert_eq!(staged, 7);
    }
}
",
            ),
        ],
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));

    run_mend_fix(&temp.path().join("Cargo.toml"));

    let plugin = fs::read_to_string(temp.path().join("src/plugin.rs")).expect("read fixed plugin");
    assert!(
        plugin.contains("reconcile::reconcile(1)"),
        "expected the file-scope call to be module qualified, got:\n{plugin}"
    );
    assert!(
        plugin.contains("let staged = reconcile();"),
        "a call to the inline module's own fn must stay bare, got:\n{plugin}"
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));
}

#[test]
fn inline_module_reimport_keeps_the_function_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    write_lib_fixture(
        temp.path(),
        "reimported_fn_fixture",
        &[
            ("lib.rs", "mod capabilities;\nmod reconcile;\n"),
            (
                "capabilities.rs",
                "pub fn reflect_component_for(value: u32) -> u32 { value + 1 }\n",
            ),
            (
                "reconcile.rs",
                r"use crate::capabilities::reflect_component_for;

pub fn run(value: u32) -> u32 { reflect_component_for(value) }

#[cfg(test)]
mod tests {
    use super::reflect_component_for;

    #[test]
    fn reflects_the_component() {
        let reflected = reflect_component_for(1);
        assert_eq!(reflected, 2);
    }
}
",
            ),
        ],
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));

    run_mend_fix(&temp.path().join("Cargo.toml"));

    let reconcile =
        fs::read_to_string(temp.path().join("src/reconcile.rs")).expect("read fixed reconcile");
    assert!(
        reconcile.contains("use crate::capabilities::reflect_component_for;"),
        "an inline module re-importing the name pins the file-scope import, got:\n{reconcile}"
    );
    assert!(
        reconcile.contains("use super::reflect_component_for;"),
        "the inline module's own import must be left intact, got:\n{reconcile}"
    );
    assert_prefer_module_fixture_compiles(&temp.path().join("Cargo.toml"));
}
