use crate::support::*;

#[test]
fn canonical_field_visibility_follows_nested_policy() {
    let temp = tempdir().expect("create canonical field visibility fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "canonical_field_visibility_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture visibility config");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write library root");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) struct Fields {\n    pub(crate) rejected: u8,\n    pub(super) accepted: u8,\n}\n",
    )
    .expect("write field declarations");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden = report
        .findings
        .iter()
        .filter(|finding| {
            finding.path == "src/a/b/c.rs" && finding.code == DiagnosticCode::ForbiddenPubCrate
        })
        .count();
    assert_eq!(forbidden, 1, "unexpected field findings: {report:#?}");
}

#[test]
fn pub_field_on_private_struct_is_flagged() {
    // No `pub` on the struct → no convention defends `pub` on its fields.
    // The `pub` annotation grants nothing because the type itself is private.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_pub_on_private"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod inner;\n").expect("write lib");
    fs::write(
        temp.path().join("src/inner.rs"),
        "struct Hidden {\n    pub leaked: u32,\n}\n",
    )
    .expect("write inner");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert_eq!(
        findings.len(),
        1,
        "pub field on private struct should be flagged: {findings:?}"
    );
}

#[test]
fn pub_crate_field_on_private_struct_is_flagged() {
    // `pub(crate)` on a field of a private struct is also dead — the type
    // caps the field at private regardless.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_pub_crate_on_private"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod inner;\n").expect("write lib");
    fs::write(
        temp.path().join("src/inner.rs"),
        "struct Hidden {\n    pub(crate) leaked: u32,\n}\n",
    )
    .expect("write inner");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert_eq!(
        findings.len(),
        1,
        "pub(crate) field on private struct should be flagged: {findings:?}"
    );
}

#[test]
fn pub_field_on_pub_crate_struct_is_not_flagged() {
    // Conventional Rust shorthand: `pub` on fields of a `pub(crate)` struct
    // is the dominant idiom. Do NOT flag — the cap is implicit and readers
    // understand it.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_idiom_pub_crate"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "pub(crate) struct GhRun {\n    pub id: u64,\n    pub name: String,\n}\n",
    )
    .expect("write lib");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert!(
        findings.is_empty(),
        "idiomatic `pub` field on `pub(crate)` struct must not be flagged: {findings:?}"
    );
}

#[test]
fn pub_field_on_pub_super_struct_is_not_flagged() {
    // Same convention applies to `pub(super)` structs.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_idiom_pub_super"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod inner;\n").expect("write lib");
    fs::write(
        temp.path().join("src/inner.rs"),
        "pub(super) struct Local {\n    pub data: u32,\n}\n",
    )
    .expect("write inner");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert!(
        findings.is_empty(),
        "idiomatic `pub` field on `pub(super)` struct must not be flagged: {findings:?}"
    );
}

#[test]
fn opaque_handle_pattern_is_not_flagged() {
    // Field visibility narrower than type — intentional, never flagged.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_opaque"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod inner;\n").expect("write lib");
    fs::write(
        temp.path().join("src/inner.rs"),
        "pub(crate) struct GqlCheckRun {\n    pub(super) name: String,\n    pub(super) status: String,\n}\n",
    )
    .expect("write inner");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert!(
        findings.is_empty(),
        "opaque-handle pattern must not be flagged: {findings:?}"
    );
}

#[test]
fn private_field_on_any_struct_is_not_flagged() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_private"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "pub(crate) struct Holder {\n    inner: u32,\n}\n",
    )
    .expect("write lib");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::FieldVisibilityWiderThanType)
        .collect();
    assert!(
        findings.is_empty(),
        "private field on any struct must not be flagged: {findings:?}"
    );
}

#[test]
fn fix_removes_dead_pub_annotation() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "field_visibility_fix"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod inner;\n").expect("write lib");
    let inner_path = temp.path().join("src/inner.rs");
    fs::write(
        &inner_path,
        "struct Hidden {\n    pub leaked: u32,\n}\n\npub(crate) fn touch() -> u32 { Hidden { leaked: 1 }.leaked }\n",
    )
    .expect("write inner");

    let status = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .status()
        .expect("run cargo-mend --fix");
    assert!(status.success() || status.code() == Some(0));

    let after = fs::read_to_string(&inner_path).expect("read inner after fix");
    assert!(
        after.contains("    leaked: u32"),
        "expected bare `leaked: u32` after dead `pub` removed, got:\n{after}"
    );
    assert!(
        !after.contains("pub leaked"),
        "expected `pub leaked` to be removed, got:\n{after}"
    );
}
