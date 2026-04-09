use crate::common::*;

#[test]
fn basic_inline_type_adds_use() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_basic_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example(_x: crate::parent::types::MyType) {}
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
        consumer.contains("use") && consumer.contains("MyType;"),
        "expected use import for MyType, got:\n{consumer}"
    );
    assert!(
        consumer.contains("_x: MyType"),
        "expected bare type name in signature, got:\n{consumer}"
    );
    // The inline path in the function signature should be gone (only appears in the use statement)
    assert!(
        consumer.contains("_x: MyType)"),
        "inline path should be replaced with bare type, got:\n{consumer}"
    );
}

#[test]
fn function_return_type() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_return_fixture"
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
        "pub struct MyType;\nimpl MyType { pub fn new() -> Self { Self } }\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example() -> crate::parent::types::MyType {
    crate::parent::types::MyType::new()
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
        consumer.contains("-> MyType"),
        "expected bare return type, got:\n{consumer}"
    );
    assert!(
        consumer.contains("MyType::new()"),
        "expected bare constructor call, got:\n{consumer}"
    );
}

#[test]
fn multiple_occurrences_one_use() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_multi_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn first(_x: crate::parent::types::MyType) {}
fn second(_x: crate::parent::types::MyType) {}
fn third(_x: crate::parent::types::MyType) {}
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
    // Should have exactly one use statement for MyType
    let use_count = consumer.matches("use").count();
    assert_eq!(
        use_count, 1,
        "expected exactly one use import, got:\n{consumer}"
    );
    // All three occurrences should be replaced
    let bare_count = consumer.matches("_x: MyType").count();
    assert_eq!(bare_count, 3, "expected 3 bare type refs, got:\n{consumer}");
}

#[test]
fn nested_module_inserts_use_in_containing_scope() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_nested_scope_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/tui")).expect("create src/tui");
    fs::write(temp.path().join("src/lib.rs"), "mod tui;\n").expect("write fixture lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod app;\nmod interaction;\n",
    )
    .expect("write fixture tui mod");
    fs::write(
        temp.path().join("src/tui/app.rs"),
        "pub enum SearchMode {\n    Active,\n}\n",
    )
    .expect("write fixture app");
    fs::write(
        temp.path().join("src/tui/interaction.rs"),
        r#"pub fn keep() {}

#[cfg(test)]
mod tests {
    use std::mem;

    #[test]
    fn nested_variant_path_gets_local_use() {
        let _ = mem::size_of_val(&super::super::app::SearchMode::Active);
    }
}
"#,
    )
    .expect("write fixture interaction");

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

    let interaction = fs::read_to_string(temp.path().join("src/tui/interaction.rs"))
        .expect("read fixed interaction");
    assert!(
        interaction.contains(
            "mod tests {\n    use std::mem;\n    use crate::tui::app::SearchMode::Active;\n"
        ),
        "expected nested-module import placement, got:\n{interaction}"
    );
    assert!(
        interaction.contains("mem::size_of_val(&Active);"),
        "expected bare variant reference inside nested module, got:\n{interaction}"
    );
}

#[test]
fn two_types_same_module() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_two_types_fixture"
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
        "pub struct TypeA;\npub struct TypeB;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example(_a: crate::parent::types::TypeA, _b: crate::parent::types::TypeB) {}
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
        consumer.contains("TypeA;") && consumer.contains("TypeB;"),
        "expected use imports for both types, got:\n{consumer}"
    );
    assert!(
        consumer.contains("_a: TypeA") && consumer.contains("_b: TypeB"),
        "expected bare type names, got:\n{consumer}"
    );
}

#[test]
fn name_collision_skips_both() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_collision_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent/mod_a")).expect("create src/parent/mod_a");
    fs::create_dir_all(temp.path().join("src/parent/mod_b")).expect("create src/parent/mod_b");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod mod_a;\nmod mod_b;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(temp.path().join("src/parent/mod_a.rs"), "pub struct Foo;\n").expect("write mod_a");
    fs::write(temp.path().join("src/parent/mod_b.rs"), "pub struct Foo;\n").expect("write mod_b");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example(_a: crate::parent::mod_a::Foo, _b: crate::parent::mod_b::Foo) {}
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
    // Both Foo types have a name collision, so neither should be fixed
    assert!(
        consumer.contains("crate::parent::mod_a::Foo") || consumer.contains("super::mod_a::Foo"),
        "collision should leave inline paths unchanged, got:\n{consumer}"
    );
}

#[test]
fn skips_std_paths() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_std_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {
    let _map: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "std paths should not be flagged"
    );
}

#[test]
fn skips_use_statements() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_skip_use_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::types::MyType;\n\nfn example(_x: MyType) {}\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "use statements should not be flagged as inline path types"
    );
}

#[test]
fn super_path() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_super_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "fn example(_x: super::types::MyType) {}\n",
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
        consumer.contains("use super::types::MyType;"),
        "expected use import for super path type, got:\n{consumer}"
    );
    assert!(
        consumer.contains("_x: MyType"),
        "expected bare type name, got:\n{consumer}"
    );
}

#[test]
fn existing_use_no_duplicate() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_existing_use_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"use crate::parent::types::MyType;

fn example(_x: MyType, _y: crate::parent::types::MyType) {}
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
    // Should still have exactly one use statement
    let use_count = consumer
        .lines()
        .filter(|line| line.starts_with("use ") && line.contains("MyType"))
        .count();
    assert_eq!(
        use_count, 1,
        "should not duplicate existing use import, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("crate::parent::types::MyType"),
        "inline path should be replaced, got:\n{consumer}"
    );
}

#[test]
fn dry_run_no_edits() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_dry_run_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "fn example(_x: crate::parent::types::MyType) {}\n",
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

    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(
        consumer.contains("crate::parent::types::MyType"),
        "dry-run should not modify files"
    );
}

#[test]
fn read_only_reports_findings() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_readonly_fixture"
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
        "pub struct MyType;\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "fn example(_x: crate::parent::types::MyType) {}\n",
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "read-only mode should report inline_path_qualified_type findings"
    );
}

#[test]
fn nothing_to_fix() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_nothing_fixture"
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
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "clean project should not have inline_path_qualified_type findings"
    );
}

#[test]
fn generic_type_params_preserved() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_generic_fixture"
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
        "pub struct Container<T>(pub T);\n",
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"fn example(_x: crate::parent::types::Container<String>) {}
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
        consumer.contains("Container<String>"),
        "generic params should be preserved, got:\n{consumer}"
    );
    assert!(
        consumer.contains("_x: Container<String>"),
        "expected bare type with generics, got:\n{consumer}"
    );
    assert!(
        consumer.contains("_x: Container<String>") && !consumer.contains("_x: crate::"),
        "inline path should be replaced with bare type, got:\n{consumer}"
    );
}

#[test]
fn bare_name_shadowing_skipped() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_shadow_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/error")).expect("create src/error");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod error;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/error.rs"),
        "pub type Result<T> = core::result::Result<T, String>;\n",
    )
    .expect("write error mod");
    // consumer uses both prelude Result<T, E> and crate::error::Result<T> inline
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"fn uses_prelude() -> Result<String, String> {
    Ok("hello".to_string())
}
fn uses_inline() -> crate::error::Result<String> {
    Ok("hello".to_string())
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
    // The inline path should NOT be replaced because doing so would
    // add `use crate::error::Result;` which shadows prelude `Result<T, E>`
    assert!(
        consumer.contains("crate::error::Result<String>"),
        "inline path should be left alone to avoid shadowing prelude Result, got:\n{consumer}"
    );
    // Prelude usage should remain unchanged
    assert!(
        consumer.contains("Result<String, String>"),
        "prelude Result should be unchanged, got:\n{consumer}"
    );
}
