use crate::support::*;

// Depth boundary for the shallow-private policy: depth 1 and depth 2
// are shallow (pub(crate) allowed), depth 3+ is nested (pub(crate) forbidden
// unless the parent facade caps at `pub(crate) use`).
// See `resolve_module_location` and `allow_pub_crate_by_policy` in
// src/compiler/visibility/policy.rs, and the 0.13.0 CHANGELOG entry.

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
fn pub_crate_at_depth_3_is_allowed_when_parent_caps_at_pub_crate() {
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
        forbidden_count, 0,
        "pub(crate) at depth 3 should be permitted when the parent facade re-exports as \
         `pub(crate) use` (modifier matches the cap): {:?}",
        report.findings,
    );
}

#[test]
fn pub_crate_at_depth_3_fires_when_parent_reexports_as_pub_super() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_3_pub_super_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(temp.path().join("src/foo/mod.rs"), "mod bar;\n").expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(super) use baz::helper;\n",
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
        "pub(crate) at depth 3 should fire when the parent re-exports as `pub(super) use` \
         (facade visibility is Super, not Crate): {:?}",
        report.findings,
    );
}

#[test]
fn pub_crate_at_depth_3_fires_when_parent_does_not_reexport() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_3_no_reexport_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
    fs::write(temp.path().join("src/foo/mod.rs"), "mod bar;\n").expect("write foo/mod.rs");
    fs::write(temp.path().join("src/foo/bar/mod.rs"), "mod baz;\n").expect("write foo/bar/mod.rs");
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
        "pub(crate) at depth 3 should fire when the parent does not re-export it: {:?}",
        report.findings,
    );
}

#[test]
fn pub_crate_at_depth_3_suggests_pub_when_structurally_exposed_by_return_type() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "depth_3_structural_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod consumer;\nmod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo/mod.rs"),
        "mod bar;\npub(crate) use bar::Cache;\n",
    )
    .expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(crate) use baz::Cache;\n",
    )
    .expect("write foo/bar/mod.rs");
    // `Cache` is a named, re-exported crate export, so its own `pub(crate)` is
    // capped by the facade and is not flagged. `Storage` is returned by Cache's
    // public method but never named outside this module: it is exposed only
    // structurally, so narrowing it to `pub(super)` would fail
    // `private_interfaces` and the correct modifier is `pub`.
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        r#"pub(crate) struct Cache;

impl Cache {
    pub fn commit(&self) -> Storage {
        Storage { mesh: 0 }
    }
}

pub(crate) struct Storage {
    pub mesh: u32,
}
"#,
    )
    .expect("write foo/bar/baz.rs");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub(crate) fn use_storage() -> u32 {
    let cache = crate::foo::Cache;
    let storage = cache.commit();
    storage.mesh
}
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let storage_finding = report
        .findings
        .iter()
        .find(|f| {
            f.code == DiagnosticCode::ForbiddenPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected forbidden_pub_crate on structurally-exposed `Storage`: {:?}",
                report.findings,
            )
        });
    assert!(
        storage_finding
            .help
            .iter()
            .any(|line| line.contains("consider using `pub`")),
        "a structurally-exposed pub(crate) item must suggest `pub`, not `pub(super)`: {:?}",
        storage_finding.help,
    );
}

#[test]
fn pub_crate_in_library_pub_mod_fires() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "pub_mod_shallow_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "pub mod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write foo");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden_count = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubCrate && f.path.ends_with("src/foo.rs"))
        .count();
    assert_eq!(
        forbidden_count, 1,
        "pub(crate) inside a `pub mod` (public-parent ShallowPrivate) should fire: {:?}",
        report.findings,
    );
}

#[test]
fn mutually_referencing_public_signatures_do_not_overflow_exposure_walk() {
    // Regression: the structural-exposure walk follows public signatures from
    // item to item. `Alpha`'s public field graph mentions `Beta` and `Beta`'s
    // mentions `Alpha`, which used to recurse Alpha -> Beta -> Alpha forever
    // and overflow the compiler-driver stack. The walk must terminate AND
    // still find `Storage`'s real exposure through `Cache::commit`.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "mutual_signature_cycle_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
    fs::write(temp.path().join("src/lib.rs"), "mod consumer;\nmod foo;\n").expect("write lib");
    fs::write(
        temp.path().join("src/foo/mod.rs"),
        "mod bar;\npub(crate) use bar::Cache;\n",
    )
    .expect("write foo/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/mod.rs"),
        "mod baz;\npub(crate) use baz::Cache;\n",
    )
    .expect("write foo/bar/mod.rs");
    fs::write(
        temp.path().join("src/foo/bar/baz.rs"),
        r#"pub(crate) struct Cache;

impl Cache {
    pub fn commit(&self) -> Storage {
        Storage { mesh: 0 }
    }
}

pub(crate) struct Storage {
    pub mesh: u32,
}

pub struct Alpha {
    pub storage: Option<Box<Storage>>,
    pub beta: Option<Box<Beta>>,
}

pub struct Beta {
    pub alpha: Option<Box<Alpha>>,
}
"#,
    )
    .expect("write foo/bar/baz.rs");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub(crate) fn use_storage() -> u32 {
    let cache = crate::foo::Cache;
    let storage = cache.commit();
    storage.mesh
}
"#,
    )
    .expect("write consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let storage_finding = report
        .findings
        .iter()
        .find(|f| {
            f.code == DiagnosticCode::ForbiddenPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
        })
        .unwrap_or_else(|| {
            panic!(
                "expected forbidden_pub_crate on structurally-exposed `Storage`: {:?}",
                report.findings,
            )
        });
    assert!(
        storage_finding
            .help
            .iter()
            .any(|line| line.contains("consider using `pub`")),
        "the cycle guard must not hide Storage's real exposure through Cache::commit: {:?}",
        storage_finding.help,
    );
}
