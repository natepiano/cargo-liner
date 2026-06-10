use crate::support::*;

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
        interaction
            .contains("mod tests {\n    use std::mem;\n    use crate::tui::app::SearchMode;\n"),
        "expected nested-module import of the enum type, got:\n{interaction}"
    );
    assert!(
        interaction.contains("mem::size_of_val(&SearchMode::Active);"),
        "expected enum-qualified variant reference inside nested module, got:\n{interaction}"
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
fn flags_std_paths() {
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
        report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "external-crate paths (std/core/third-party) should be flagged"
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
fn existing_pub_use_binding_no_duplicate() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_existing_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod slot {
    pub enum BarSlot {
        Single(u8),
    }
}

pub use slot::BarSlot;

fn is_single(slot: crate::BarSlot) -> bool {
    match slot {
        crate::BarSlot::Single(_) => true,
    }
}

fn main() {
    let _ = is_single(BarSlot::Single(1));
}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed main");
    assert!(
        !main_rs.contains("use crate::BarSlot;"),
        "pub use already binds BarSlot; fix must not add a duplicate import, got:\n{main_rs}"
    );
    assert!(
        main_rs.contains("BarSlot::Single(_)"),
        "expected inline variant path to be rewritten through existing binding, got:\n{main_rs}"
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

/// Struct literal paths (`crate::foo::Bar { .. }`) and pattern paths
/// (`let crate::foo::Bar { .. } = ..`, `Some(crate::foo::Bar(x))`) are not
/// reached by the default `visit_expr_path` / `visit_type_path` passes. This
/// verifies each form is rewritten.
#[test]
fn struct_literal_and_pattern_paths_get_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_struct_literal_fixture"
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
        r#"pub struct MyType { pub x: i32 }
pub struct TupleType(pub i32);
"#,
    )
    .expect("write types");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        r#"pub fn make() -> i32 {
    let value = crate::parent::types::MyType { x: 1 };
    let crate::parent::types::MyType { x } = value;
    let tup = crate::parent::types::TupleType(42);
    let crate::parent::types::TupleType(y) = tup;
    x + y
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
        consumer.contains("use crate::parent::types::MyType;")
            || consumer.contains("use super::types::MyType;"),
        "expected MyType import, got:\n{consumer}"
    );
    assert!(
        consumer.contains("use crate::parent::types::TupleType;")
            || consumer.contains("use super::types::TupleType;"),
        "expected TupleType import, got:\n{consumer}"
    );
    // The `use` lines will still contain the qualified path; ensure the
    // body no longer does.
    let body = consumer
        .lines()
        .filter(|line| !line.trim_start().starts_with("use "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !body.contains("crate::parent::types::MyType"),
        "struct literal / pattern should be rewritten, got body:\n{body}"
    );
    assert!(
        !body.contains("crate::parent::types::TupleType"),
        "tuple-struct literal / pattern should be rewritten, got body:\n{body}"
    );
    assert!(
        consumer.contains("MyType { x: 1 }"),
        "expected bare struct literal, got:\n{consumer}"
    );
    assert!(
        consumer.contains("let MyType { x } = value;"),
        "expected bare struct pattern, got:\n{consumer}"
    );
    assert!(
        consumer.contains("TupleType(42)"),
        "expected bare tuple-struct literal, got:\n{consumer}"
    );
    assert!(
        consumer.contains("let TupleType(y) = tup;"),
        "expected bare tuple-struct pattern, got:\n{consumer}"
    );
}

/// Regression: running `cargo mend --fix` on a file that mixes an enum
/// variant `RustProject::Package(...)`, a same-named struct literal
/// `Package { ... }` (brought in via `use super::*`), and a struct
/// associated-function call `Package::default()` must not introduce an import
/// of the variant `Package` that would shadow the bare struct name.
///
/// This is the cargo-port-uses-cargo-metadata case that motivated the fix.
#[test]
fn enum_variant_and_same_named_struct_do_not_collide() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "enum_struct_collision_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/project")).expect("create src/project");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod project;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/project/mod.rs"),
        r#"#[derive(Default)]
pub struct Package {
    pub name: String,
}

pub enum RustProject {
    Package(Package),
}
"#,
    )
    .expect("write project mod");
    // Consumer: uses the enum variant via a fully-qualified path, uses the
    // struct literal with a bare name (imported via `use super::*` in the
    // inner `tests` module), and calls `Package::default()` as a struct
    // associated function — all three mentions share the leaf `Package`.
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn keep() {}

#[cfg(test)]
mod tests {
    use crate::project::Package;

    #[test]
    fn build_package() {
        let pkg = crate::project::RustProject::Package(Package {
            name: "demo".into(),
            ..Package::default()
        });
        let _ = pkg;
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
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed consumer");

    // `Package::default()` is a struct associated-fn call. prefer_module_import
    // must NOT treat `Package` as a module (filesystem is case-insensitive on
    // macOS/Windows) and must not add a module-style `use crate::project::Package;`.
    assert!(
        !consumer.contains("use crate::project::Package::"),
        "unexpected variant-leaf import introduced, got:\n{consumer}"
    );

    // The enum-variant rewrite must import the parent type, not the variant,
    // so the bare `Package` struct (from the existing `use`) is not shadowed.
    assert!(
        consumer.contains("use crate::project::RustProject;"),
        "expected import of the enum type `RustProject`, got:\n{consumer}"
    );
    assert!(
        consumer.contains("RustProject::Package(Package {"),
        "expected enum-qualified variant, got:\n{consumer}"
    );

    // The subsequent `cargo check` that `cargo mend --fix` runs as validation
    // must succeed; the explicit build below guards against silent regressions.
    let check = cargo_command()
        .arg("check")
        .arg("--tests")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo check");
    assert!(
        check.status.success(),
        "post-fix cargo check failed: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

/// Multi-byte UTF-8 characters earlier on the same line (em-dashes, accented
/// letters, etc.) must not shift the byte offset of a path that gets
/// rewritten. `proc_macro2::LineColumn::column` is a character index, not a
/// byte index — the `offset()` helper has to convert.
#[test]
fn multi_byte_chars_do_not_corrupt_replacement_span() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_multibyte_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        // The em-dash `—` is 3 bytes / 1 column. Without the byte conversion
        // the rewrite of `std::cmp::Ordering::Equal` lands 2 bytes too early,
        // corrupting `align(...)` to garbage.
        "pub fn align(_s: &str) -> std::cmp::Ordering {\n    let _ = \"—\";\n    \
         std::cmp::Ordering::Equal\n}\n\nfn main() {}\n",
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        main_rs.contains("use std::cmp::Ordering;"),
        "expected `use std::cmp::Ordering;` insertion, got:\n{main_rs}"
    );
    // Body must contain a clean `Ordering::Equal` — the rewrite of
    // `std::cmp::Ordering::Equal` must not have shifted into surrounding text.
    let body = main_rs
        .lines()
        .filter(|line| !line.trim_start().starts_with("use "))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        body.contains("Ordering::Equal"),
        "expected clean `Ordering::Equal` in body, got body:\n{body}"
    );
    assert!(
        !body.contains("std::cmp::Ordering"),
        "body should not retain fully-qualified path, got body:\n{body}"
    );
}

/// A file that uses `Result::ok` as a method reference (e.g.
/// `.filter_map(Result::ok)`) is relying on the prelude `Result`. If a
/// separate `io::Result<T>` appears in the same file, the lint must not add
/// `use io::Result;` — that would shadow the prelude `Result` and silently
/// change which type `Result::ok` resolves through, often producing a
/// confusing trait-bound error.
#[test]
fn multi_segment_path_with_pascal_first_segment_blocks_shadowing_import() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_result_shadow_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"use std::io;

pub fn open(path: &str) -> io::Result<String> {
    let _ = [Ok::<_, io::Error>(())].into_iter().filter_map(Result::ok);
    std::fs::read_to_string(path)
}

fn main() {
    let _ = open("x");
}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        !main_rs.contains("use io::Result;") && !main_rs.contains("use std::io::Result;"),
        "must not import a name that would shadow prelude `Result`, got:\n{main_rs}"
    );
}

/// `impl crate::path::Trait for Type` puts the trait path in
/// `ItemImpl::trait_`, which is a bare `syn::Path` — not visited as a
/// `TypePath`. The visitor must hook `visit_item_impl` explicitly.
#[test]
fn impl_trait_path_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_impl_trait_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/pane")).expect("create src/pane");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod pane;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/pane/mod.rs"),
        "pub trait Hittable {\n    fn hit(&self) -> bool;\n}\n",
    )
    .expect("write pane mod");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub struct Manager;

impl crate::pane::Hittable for Manager {
    fn hit(&self) -> bool { false }
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
        consumer.contains("use crate::pane::Hittable;"),
        "expected `use crate::pane::Hittable;` insertion, got:\n{consumer}"
    );
    assert!(
        consumer.contains("impl Hittable for Manager"),
        "expected `impl Hittable for Manager`, got:\n{consumer}"
    );
    assert!(
        !consumer.contains("impl crate::pane::Hittable"),
        "fully-qualified trait path should be rewritten, got:\n{consumer}"
    );
}

/// Importing a name that has prelude meaning (`Result`, `Box`, `Option`, ...)
/// silently changes what every future bare reference to that name resolves
/// to. The lint must skip these — even when the file currently doesn't write
/// bare `Result<T, E>` and the shadow-detection heuristic alone would clear
/// it.
#[test]
fn does_not_import_prelude_names() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_prelude_skip_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        // `std::fmt::Result` is the type alias for `Result<(), fmt::Error>`.
        // Bringing it in as plain `Result` would shadow the prelude generic.
        r#"use std::fmt;

pub struct Marker;

impl fmt::Display for Marker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("marker")
    }
}

fn main() {}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        !main_rs.contains("use fmt::Result;") && !main_rs.contains("use std::fmt::Result;"),
        "must not import a name that shadows prelude `Result`, got:\n{main_rs}"
    );
}

/// When a file already has `use std::fmt;` (or any other parent module) and
/// writes `fmt::Display` inline, the new import the lint adds must be
/// absolute (`use std::fmt::Display;`), not partial (`use fmt::Display;`).
/// Partial imports look fine but break silently if the parent import is
/// later reordered or removed.
#[test]
fn partial_path_import_is_resolved_to_absolute() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_partial_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"use std::sync::mpsc;

pub fn make() -> mpsc::Sender<i32> {
    let (tx, _) = mpsc::channel();
    tx
}

fn main() {
    let _ = make();
}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        main_rs.contains("use std::sync::mpsc::Sender;"),
        "expected absolute `use std::sync::mpsc::Sender;`, got:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("use mpsc::Sender;"),
        "partial-path import is brittle — must be absolute, got:\n{main_rs}"
    );
}

/// Associated items on a generic type parameter (`S::Ok`, `B::Item`,
/// `Idx::Output`, even an unconventional lowercase `t::Item`) must not be
/// treated as crate-qualified paths. The visitor tracks generics via
/// `syn::Generics::params` rather than guessing from naming, so any name
/// the surrounding scope introduces as a type parameter is recognized.
#[test]
fn generic_type_param_associated_items_are_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_generic_assoc_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub trait Sink {
    type Ok;
    type Error;
    fn finish(self) -> Result<Self::Ok, Self::Error>;
}

pub fn run<S: Sink>(s: S) -> Result<S::Ok, S::Error> {
    s.finish()
}

pub trait Bucket {
    type Item;
}

pub fn first<B: Bucket>(_b: &B) -> Option<B::Item> {
    None
}

// PascalCase generic name — already covered by the PascalCase-first-segment
// filter, but exercise the generics-tracker path too.
pub fn next_idx<Idx: Iterator>(it: &mut Idx) -> Option<Idx::Item> {
    it.next()
}

// Non-idiomatic lowercase generic. `non_camel_case_types` warns, but this
// verifies generic tracking comes from syntax, not naming convention.
#[allow(
    non_camel_case_types,
    reason = "fixture intentionally covers lowercase generic parameters"
)]
pub fn lower<t: Bucket>(_b: &t) -> Option<t::Item> {
    None
}

// `syn::Generics::params` on an impl block remains in scope while visiting
// `ImplItemFn` bodies; the method's own signature can add generics.
pub struct Wrap<T>(pub T);
impl<T: Bucket> Wrap<T> {
    pub fn nested<R: Sink>(_r: R) -> Option<(T::Item, R::Ok, R::Error)> {
        None
    }
}

fn main() {}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let inline_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::InlinePathQualifiedType)
        .collect();
    assert!(
        inline_findings.is_empty(),
        "associated items on generic params must not be flagged, got: \
         {inline_findings:?}"
    );
}

/// Generic params used inside the *body* of a function (closure parameter
/// types, `let` annotations, etc.) must not be flagged. Regression test —
/// `visit_signature` previously pushed/popped generics around the signature
/// only, leaving the body visited without them, so `|x: &T::Item| ...`
/// inside `fn foo<T>(...)` was misread as a crate-qualified path.
#[test]
fn generic_type_param_in_fn_body_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_generic_body_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub trait Bucket {
    type Item;
}

// Closure parameter type uses the fn's generic — used to be flagged because
// the generic was popped before the body was visited.
pub fn drive<A: Bucket>(_items: &[A::Item]) -> usize {
    let _count = |_x: &A::Item, acc: &mut usize| {
        *acc += 1;
    };
    0
}

// Same situation inside an impl method.
pub struct Wrap<T>(pub T);
impl<T: Bucket> Wrap<T> {
    pub fn run(&self, _items: &[T::Item]) -> usize {
        let _take = |_x: &T::Item| {};
        0
    }
}

// Trait method body referencing the impl's generic.
pub trait Driver<T: Bucket> {
    fn drive(&self, items: &[T::Item]) -> usize {
        let _take = |_x: &T::Item| {};
        items.len()
    }
}

fn main() {}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let inline_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::InlinePathQualifiedType)
        .collect();
    assert!(
        inline_findings.is_empty(),
        "generic-param associated items in fn bodies must not be flagged, got: \
         {inline_findings:?}"
    );
}

/// `Type::Variant` (enum variant patterns / associated items) must not be
/// treated as a crate-qualified path. The first segment is `PascalCase`,
/// which means it's a type — suggesting `use Type;` is wrong.
#[test]
fn enum_variant_two_segment_path_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_enum_variant_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub struct Package;

pub enum RustProject {
    Package(Package),
}

pub fn unwrap(item: RustProject) -> Option<Package> {
    let RustProject::Package(pkg) = item else {
        return None;
    };
    Some(pkg)
}

fn main() {}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "`Type::Variant` patterns must not be flagged, got findings: {:?}",
        report
            .findings
            .iter()
            .filter(|f| f.code == DiagnosticCode::InlinePathQualifiedType)
            .collect::<Vec<_>>()
    );
}

/// External-crate enum-variant paths like `notify::WatcherKind::NullWatcher`
/// should be rewritten the same way intra-crate enum variants are: import
/// the enum (`use notify::WatcherKind;`) and rewrite the call site to
/// `WatcherKind::NullWatcher`.
#[test]
fn external_crate_enum_variant_path_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_external_variant_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    // Stand-in for an external crate: a top-level `mod notify` with a
    // PascalCase enum and variant, used inline as `notify::WatcherKind::NullWatcher`.
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod notify {
    pub enum WatcherKind {
        NullWatcher,
    }
}

fn pick() -> notify::WatcherKind {
    notify::WatcherKind::NullWatcher
}

fn main() {
    let _ = pick();
}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        main_rs.contains("use notify::WatcherKind;"),
        "expected `use notify::WatcherKind;` insertion, got:\n{main_rs}"
    );
    assert!(
        main_rs.contains("WatcherKind::NullWatcher"),
        "expected rewrite to `WatcherKind::NullWatcher`, got:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("notify::WatcherKind::NullWatcher"),
        "fully-qualified variant should be rewritten, got:\n{main_rs}"
    );
}

/// `InlinePathScan` rewrites external-crate paths in argument or return
/// position (`fn render(&mut self, frame: &mut ratatui::Frame<'_>)`) to a
/// top-level `use ratatui::Frame;` import the same way intra-crate paths are.
#[test]
fn external_crate_path_in_argument_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_ext_crate_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub struct Frame;

pub fn take(_frame: &mut std::collections::BTreeMap<String, i32>) {}

fn main() {}
"#,
    )
    .expect("write main");

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

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed file");
    assert!(
        main_rs.contains("use std::collections::BTreeMap;"),
        "expected `use std::collections::BTreeMap;` insertion, got:\n{main_rs}"
    );
    assert!(
        main_rs.contains("&mut BTreeMap<String, i32>"),
        "expected bare type with generics, got:\n{main_rs}"
    );
    assert!(
        !main_rs.contains("std::collections::BTreeMap<"),
        "fully-qualified path should be rewritten, got:\n{main_rs}"
    );
}
