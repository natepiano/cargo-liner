use crate::support::*;

#[test]
fn basic_function_import_rewrite() {
    let temp = tempdir().expect("create temp fixture dir");

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

#[test]
fn inline_call_inserts_use_and_qualifies() {
    let temp = tempdir().expect("create temp fixture dir");

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
