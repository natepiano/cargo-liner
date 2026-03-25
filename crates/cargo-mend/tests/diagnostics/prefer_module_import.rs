use crate::common::*;

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
            .any(|f| f.code == "prefer_module_import"),
        "fix should be idempotent — second run should have no prefer_module_import findings, got: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.code == "prefer_module_import")
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
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
            .any(|f| f.code == "prefer_module_import"),
        "`use super::module;` should not be flagged as prefer_module_import"
    );
}
