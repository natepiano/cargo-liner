use crate::support::*;

// Depth boundary for the shallow-private policy: depth 1 and depth 2
// are shallow (pub(crate) allowed), depth 3+ is nested (pub(crate) forbidden
// unless the parent facade caps at `pub(crate) use`).
// See `resolve_module_location` and `allow_pub_crate_by_policy` in
// src/compiler/visibility/policy.rs, and the 0.13.0 CHANGELOG entry.

#[test]
fn integration_test_support_module_pub_crate_is_rejected() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
fn pub_crate_at_depth_3_names_exact_boundary_for_supported_facade_spellings() {
    for (package_name, facade_visibility) in [
        ("depth_3_pub_super_fixture", "pub(super)"),
        ("depth_3_pub_in_super_fixture", "pub(in super)"),
    ] {
        let temp = tempdir().expect("create temp fixture dir");

        fs::write(
            temp.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            ),
        )
        .expect("write manifest");
        fs::write(
            temp.path().join("mend.toml"),
            "[visibility]\npub_in_path = \"permitted\"\n",
        )
        .expect("write visibility config");
        fs::create_dir_all(temp.path().join("src/foo/bar")).expect("create src/foo/bar");
        fs::write(temp.path().join("src/lib.rs"), "mod foo;\n").expect("write lib");
        fs::write(temp.path().join("src/foo/mod.rs"), "mod bar;\n").expect("write foo/mod.rs");
        fs::write(
            temp.path().join("src/foo/bar/mod.rs"),
            format!("mod baz;\n{facade_visibility} use baz::helper;\n"),
        )
        .expect("write foo/bar/mod.rs");
        fs::write(
            temp.path().join("src/foo/bar/baz.rs"),
            "pub(crate) fn helper() {}\n",
        )
        .expect("write foo/bar/baz.rs");

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.path.ends_with("src/foo/bar/baz.rs")
            })
            .expect("depth-three parent facade should reject pub(crate)");
        assert!(
            finding
                .help
                .iter()
                .any(|help| help == "consider using: `pub(in crate::foo)`"),
            "{facade_visibility} parent facade help was wrong: {:?}",
            finding.help,
        );
    }
}

#[test]
fn restricted_facade_sets_the_exact_declaration_boundary() {
    let temp = tempdir().expect("create restricted facade fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "restricted_facade_reach_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write visibility config");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "mod child;\npub(in crate::a) use child::helper;\n",
    )
    .expect("write restricted facade");
    fs::write(
        temp.path().join("src/a/b/c/child.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path == "src/a/b/c/child.rs"
        })
        .unwrap_or_else(|| panic!("missing forbidden visibility finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|help| help == "consider using: `pub(in crate::a)`"),
        "{report:#?}"
    );
}

#[test]
fn restricted_facade_chain_renders_the_resolved_boundary_exactly() {
    let temp = tempdir().expect("create restricted facade chain fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "restricted_facade_chain_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write library root");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(in crate::a) use c::d::helper;\n",
    )
    .expect("write restricted outer facade");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) mod d;\npub(super) use d::helper;\n",
    )
    .expect("write inner facade");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c/d.rs"
        })
        .unwrap_or_else(|| panic!("missing restricted chain finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|help| help == "consider using: `pub(in crate::a)`"),
        "the restricted facade chain must render its resolved crate::a boundary: {report:#?}"
    );
}

#[test]
fn pub_crate_at_depth_3_fires_when_parent_does_not_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
fn named_reexport_beside_glob_keeps_chain_resolvable() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "named_beside_glob_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write lib");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write a");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(crate) use c::helper;\npub(super) use c::*;\n",
    )
    .expect("write b");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(crate) fn helper() {}\n",
    )
    .expect("write c");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "a named re-export must take precedence over a sibling glob: {report:#?}"
    );
}

#[test]
fn super_to_super_chain_uses_a_non_root_restricted_boundary() {
    let temp = tempdir().expect("create stacked super facade fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_to_super_chain_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write visibility config");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write library root");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(super) use c::d::exact;\npub(super) use c::d::wide;\n",
    )
    .expect("write outer super facade");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) mod d;\npub(super) use d::exact;\npub(super) use d::wide;\n",
    )
    .expect("write inner super facade");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(in crate::a) fn exact() {}\npub(crate) fn wide() {}\n",
    )
    .expect("write facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubInCrate
                && finding.path == "src/a/b/c/d.rs"
                && finding.headline.contains("pub(in crate::a)")
        }),
        "the exact crate::a boundary must be accepted: {report:#?}"
    );
    let wide = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c/d.rs"
        })
        .unwrap_or_else(|| panic!("missing too-wide pub(crate) finding: {report:#?}"));
    assert!(
        wide.help
            .iter()
            .any(|help| help == "consider using: `pub(in crate::a)`"),
        "the joined non-root boundary must be rendered exactly: {report:#?}"
    );
    assert!(
        wide.help.iter().all(|help| {
            help != "consider using: `pub(crate)`" && help != "consider using: `pub(super)`"
        }),
        "the joined non-root boundary must be neither crate-wide nor a second pub(super): {report:#?}"
    );
}

#[test]
fn renamed_facades_resolve_the_chain_without_advertising_an_auto_fix() {
    let temp = tempdir().expect("create renamed facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "renamed_facade_chain_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write library root");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(super) use c::d::boundary as attach;\n",
    )
    .expect("write outer renamed facade");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) mod d;\npub(super) use d::boundary as inner_boundary;\npub(super) use d::manual as manual_alias;\n",
    )
    .expect("write inner renamed facades");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) fn boundary() {}\npub fn manual() {}\n",
    )
    .expect("write renamed facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let boundary = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c/d.rs"
        })
        .unwrap_or_else(|| panic!("missing renamed chain boundary: {report:#?}"));
    assert!(
        boundary
            .help
            .iter()
            .any(|help| help == "consider using: `pub(in crate::a)`"),
        "the renamed chain must provide its joined restricted boundary: {report:#?}"
    );
    let manual = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/c/d.rs"
                && finding.item.as_deref() == Some("fn manual")
        })
        .unwrap_or_else(|| panic!("missing manual-only renamed facade finding: {report:#?}"));
    assert_ne!(
        manual.fix_support,
        FixSupport::PubUse,
        "a renamed facade cannot advertise a pub-use rewrite: {report:#?}"
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0, "{report:#?}");
}

#[test]
fn super_to_crate_chain_permits_the_pub_crate_boundary() {
    let temp = tempdir().expect("create super to crate facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "super_to_crate_chain_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::write(temp.path().join("src/lib.rs"), "mod a;\n").expect("write library root");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(crate) use c::d::thing;\n",
    )
    .expect("write crate-wide outer facade");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(super) mod d;\npub(super) use d::thing;\n",
    )
    .expect("write inner super facade");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) fn thing() {}\n",
    )
    .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c/d.rs"
        }),
        "the outer pub(crate) hop must set the joined boundary to pub(crate): {report:#?}"
    );
}

#[test]
fn pub_crate_at_depth_3_requires_structure_for_a_crate_visible_method_surface() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    // `Cache` is capped by its crate-visible facade. Callers must access both
    // `Cache` and `Cache::commit`, so the method exposes `Storage` at crate
    // reach even though the method itself is written as bare `pub`.
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
        .find(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path.ends_with("src/foo/bar/baz.rs")
        })
        .unwrap_or_else(|| panic!("missing Storage finding: {report:#?}"));
    assert!(
        storage_finding.help.iter().any(|help| {
            help
                == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
        }),
        "crate-visible method exposure requires structural advice: {report:#?}",
    );
}

#[test]
fn pub_crate_in_library_pub_mod_fires() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    // and overflow the compiler-driver stack. The walk must terminate and
    // retain `Storage`'s crate-visible exposure through `Cache::commit`.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate
                && finding.path.ends_with("src/foo/bar/baz.rs")
                && finding.help.iter().any(|help| {
                    help
                        == "move the item into `crate`, or add an explicit facade at `crate` and rerun `cargo mend`"
                })
        }),
        "the cycle guard must retain Storage's crate-visible structural requirement: {report:#?}",
    );
}
