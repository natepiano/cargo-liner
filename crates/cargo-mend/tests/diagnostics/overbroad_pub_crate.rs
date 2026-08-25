use crate::support::*;

// `pub(crate)` is accepted at any depth when the crate root is the narrowest
// boundary that satisfies callers and signatures. Shallower cases are still
// reported when private or `pub(super)` is sufficient.

#[test]
fn integration_test_support_module_keeps_required_pub_crate() {
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
        finding.code == DiagnosticCode::OverbroadPubCrate
            && finding.path.ends_with("tests/support.rs")
    });
    assert!(
        !has_forbidden,
        "the integration-test crate root is the required boundary: {:?}",
        report.findings,
    );
}

#[test]
fn unused_pub_crate_at_depth_1_is_a_fixable_warning() {
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

    let manifest_path = temp.path().join("Cargo.toml");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--json")
        .output()
        .expect("run cargo-mend --json");
    assert!(
        output.status.success(),
        "a pub(crate) narrowing warning must not fail by default: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let report = parse_mend_json_output(&output.stdout);
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::OverbroadPubCrate)
        .collect();
    assert_eq!(findings.len(), 1, "expected one narrowing: {report:#?}");
    assert!(
        findings[0]
            .help
            .iter()
            .any(|line| line == "consider removing the visibility"),
        "unused crate visibility must narrow to private: {report:#?}",
    );

    let strict_output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .arg("--fail-on-warn")
        .output()
        .expect("run cargo-mend --fail-on-warn");
    assert_eq!(
        strict_output.status.code(),
        Some(2),
        "--fail-on-warn must fail for pub(crate) narrowing advice: {}",
        String::from_utf8_lossy(&strict_output.stderr),
    );
}

#[test]
fn pub_crate_at_depth_2_is_accepted_when_a_facade_requires_crate_reach() {
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
        .filter(|f| f.code == DiagnosticCode::OverbroadPubCrate)
        .collect();
    assert!(
        forbidden.is_empty(),
        "the crate-visible facade requires the helper's crate reach: {forbidden:?}",
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
            f.code == DiagnosticCode::OverbroadPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
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
                finding.code == DiagnosticCode::OverbroadPubCrate
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
            finding.code == DiagnosticCode::OverbroadPubCrate
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c/d.rs"
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
            f.code == DiagnosticCode::OverbroadPubCrate && f.path.ends_with("src/foo/bar/baz.rs")
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c/d.rs"
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c/d.rs"
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c/d.rs"
        }),
        "the outer pub(crate) hop must set the joined boundary to pub(crate): {report:#?}"
    );
}

#[test]
fn pub_crate_at_depth_3_is_accepted_for_a_crate_visible_method_surface() {
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
    assert!(
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate
                || !finding.path.ends_with("src/foo/bar/baz.rs")
        }),
        "crate-visible signature reach must accept Storage's `pub(crate)`: {report:#?}",
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
        .filter(|f| f.code == DiagnosticCode::OverbroadPubCrate && f.path.ends_with("src/foo.rs"))
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
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate
                || !finding.path.ends_with("src/foo/bar/baz.rs")
        }),
        "the cycle guard must accept Storage's required crate reach: {report:#?}",
    );
}

#[test]
fn facade_less_method_is_rewritten_to_its_caller_boundary() {
    let temp = tempdir().expect("create facade-less method fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "facade_less_method_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/panel/conversion")).expect("create sources");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod panel;\npub use panel::Saved;\npub fn entry() -> u32 { panel::entry() }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/panel/mod.rs"),
        "mod conversion;\nmod diegetic;\npub use conversion::Saved;\npub(super) fn entry() -> u32 { diegetic::run() }\n",
    )
    .expect("write panel module");
    fs::write(
        temp.path().join("src/panel/conversion/mod.rs"),
        "mod saved;\npub use saved::Saved;\n",
    )
    .expect("write conversion module");
    let original = r#"pub struct Saved {
    pub width: u32,
}

impl Saved {
    pub(crate) fn apply_world_conversion(&self) -> u32 {
        self.width
    }
}
"#;
    fs::write(temp.path().join("src/panel/conversion/saved.rs"), original)
        .expect("write saved module");
    fs::write(
        temp.path().join("src/panel/diegetic.rs"),
        "use super::conversion::Saved;\npub(super) fn run() -> u32 {\n    let saved = Saved { width: 3 };\n    saved.apply_world_conversion()\n}\n",
    )
    .expect("write caller module");

    let manifest_path = temp.path().join("Cargo.toml");
    let report = run_mend_json(&manifest_path);
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::OverbroadPubCrate
                && finding.path.ends_with("src/panel/conversion/saved.rs")
                && finding.line_start == 6
        })
        .unwrap_or_else(|| panic!("missing method finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|help| help == "consider using: `pub(in crate::panel)`"),
        "facade-less advice must name the caller boundary: {report:#?}",
    );
    assert_eq!(
        report.summary.fixable_with_fix, 1,
        "the method annotation must be offered to `--fix`: {report:#?}",
    );

    assert_human_report_advertises_warning_fix(&manifest_path);

    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let fixed = original.replace("pub(crate) fn", "pub(in crate::panel) fn");
    assert_eq!(
        fs::read_to_string(temp.path().join("src/panel/conversion/saved.rs"))
            .expect("read fixed method"),
        fixed,
        "only the method annotation may change",
    );

    let fixed_report = run_mend_json(&manifest_path);
    assert!(
        fixed_report.findings.iter().all(|finding| {
            !finding.path.ends_with("src/panel/conversion/saved.rs")
                || finding.line_start != 6
                || !matches!(
                    finding.code,
                    DiagnosticCode::OverbroadPubCrate | DiagnosticCode::ForbiddenPubInCrate
                )
        }),
        "the applied boundary must be accepted on the next run: {fixed_report:#?}",
    );
}

#[test]
fn named_fields_are_rewritten_to_their_caller_boundaries() {
    let temp = tempdir().expect("create named-field fixture dir");
    let original = write_named_field_fixture(temp.path());

    let manifest_path = temp.path().join("Cargo.toml");
    let report = run_mend_json(&manifest_path);
    assert_named_field_findings(&report);

    assert_human_report_advertises_warning_fix(&manifest_path);

    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    assert_eq!(
        fs::read_to_string(temp.path().join("src/render/panel_shapes/primitive.rs"))
            .expect("read fixed fields"),
        original
            .replace("pub(crate) panel", "pub(super) panel")
            .replace("pub(crate) source", "pub(in crate::render) source")
            .replace("pub(crate) patterned", "pub(super) patterned")
            .replace("pub(crate) offset_field", "pub(super) offset_field"),
        "each field must receive only its required visibility",
    );

    let fixed_report = run_mend_json(&manifest_path);
    assert!(
        fixed_report.findings.iter().all(|finding| {
            !finding
                .path
                .ends_with("src/render/panel_shapes/primitive.rs")
                || !matches!(
                    finding.code,
                    DiagnosticCode::OverbroadPubCrate | DiagnosticCode::ForbiddenPubInCrate
                )
        }),
        "the applied field boundaries must be accepted on the next run: {fixed_report:#?}",
    );
}

#[test]
fn crate_wide_field_keeps_pub_crate() {
    let temp = tempdir().expect("create type-capped field fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "type_capped_field_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/restore/target_position")).expect("create sources");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod managed;\nmod restore;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/restore/mod.rs"),
        "mod target_position;\npub(crate) use target_position::TargetPosition;\n",
    )
    .expect("write restore module");
    fs::write(
        temp.path().join("src/restore/target_position/mod.rs"),
        "mod target;\npub(crate) use target::TargetPosition;\n",
    )
    .expect("write target position module");
    fs::write(
        temp.path().join("src/restore/target_position/target.rs"),
        "pub(crate) struct TargetPosition {\n    pub(crate) physical_position: Option<i32>,\n}\n",
    )
    .expect("write target module");
    fs::write(
        temp.path().join("src/managed.rs"),
        "#[cfg(test)]\nmod tests {\n    use crate::restore::TargetPosition;\n\n    #[test]\n    fn reads_target_position() {\n        let target = TargetPosition { physical_position: Some(1) };\n        assert_eq!(target.physical_position, Some(1));\n    }\n}\n",
    )
    .expect("write managed module");

    let manifest_path = temp.path().join("Cargo.toml");
    let report = run_mend_json(&manifest_path);
    assert!(
        report.findings.iter().all(|finding| {
            !finding
                .path
                .ends_with("src/restore/target_position/target.rs")
                || finding.line_start != 2
                || !matches!(
                    finding.code,
                    DiagnosticCode::OverbroadPubCrate
                        | DiagnosticCode::ForbiddenPubInCrate
                        | DiagnosticCode::SuspiciousPub
                )
        }),
        "the field's crate-wide caller requires `pub(crate)`: {report:#?}",
    );
}

const NAMED_FIELD_ORIGINAL: &str = r#"pub struct Key {
    pub(crate) panel: u32,
    pub(crate) source: u32,
}

pub struct PatternKey {
    pub(crate) patterned: u32,
}

pub(super) fn pattern_key() -> PatternKey {
    PatternKey { patterned: 5 }
}

pub struct OffsetKey {
    pub(crate) offset_field: u32,
}
"#;

fn write_named_field_fixture(root: &std::path::Path) -> &'static str {
    pin_pub_in_path(root, PubInPath::Permitted);
    fs::write(
        root.join("mend.toml"),
        "[diagnostics]\nsuspicious_pub = false\n\n[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture config");
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "named_field_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(root.join("src/render/panel_shapes")).expect("create sources");
    fs::write(
        root.join("src/lib.rs"),
        "mod render;\npub fn entry() -> u32 { render::entry() }\n",
    )
    .expect("write lib");
    fs::write(
        root.join("src/render/mod.rs"),
        "mod panel_shapes;\npub(crate) fn entry() -> u32 {\n    let key = panel_shapes::make();\n    panel_shapes::read_panel(&key) + panel_shapes::read_patterned() + panel_shapes::offset() as u32 + key.source\n}\n",
    )
    .expect("write render module");
    fs::write(
        root.join("src/render/panel_shapes/mod.rs"),
        "mod batching;\nmod primitive;\npub(super) use primitive::Key;\npub(super) fn make() -> Key { batching::make() }\npub(super) fn read_panel(key: &Key) -> u32 { batching::read_panel(key) }\npub(super) fn read_patterned() -> u32 { batching::read_patterned() }\npub(super) fn offset() -> usize { batching::offset() }\n",
    )
    .expect("write panel_shapes module");
    fs::write(
        root.join("src/render/panel_shapes/batching.rs"),
        "use super::primitive::{self, Key, OffsetKey, PatternKey};\n\npub(super) fn make() -> Key {\n    Key { panel: 2, source: 3 }\n}\n\npub(super) fn read_panel(key: &Key) -> u32 {\n    let Key { panel, .. } = key;\n    *panel\n}\n\npub(super) fn read_patterned() -> u32 {\n    let PatternKey { patterned } = primitive::pattern_key();\n    patterned\n}\n\npub(super) fn offset() -> usize {\n    std::mem::offset_of!(OffsetKey, offset_field)\n}\n",
    )
    .expect("write batching module");
    fs::write(
        root.join("src/render/panel_shapes/primitive.rs"),
        NAMED_FIELD_ORIGINAL,
    )
    .expect("write primitive module");
    NAMED_FIELD_ORIGINAL
}

fn assert_named_field_findings(report: &Report) {
    let field_findings = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::OverbroadPubCrate
                && finding
                    .path
                    .ends_with("src/render/panel_shapes/primitive.rs")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        field_findings.len(),
        4,
        "all field annotations must be reported: {report:#?}",
    );
    let has_help = |line_start, expected: &str| {
        field_findings.iter().any(|finding| {
            finding.line_start == line_start && finding.help.iter().any(|help| help == expected)
        })
    };
    assert!(
        has_help(2, "consider using: `pub(super)`"),
        "the field used only under the parent module needs `pub(super)`: {report:#?}",
    );
    assert!(
        has_help(3, "consider using: `pub(in crate::render)`"),
        "the field read from the outer module needs its exact boundary: {report:#?}",
    );
    assert!(
        has_help(7, "consider using: `pub(super)`"),
        "a field named in a sibling-module pattern needs `pub(super)`: {report:#?}",
    );
    assert!(
        has_help(15, "consider using: `pub(super)`"),
        "a field named by `offset_of!` in a sibling module needs `pub(super)`: {report:#?}",
    );
    assert!(
        field_findings.iter().all(|finding| {
            finding
                .help
                .iter()
                .any(|line| line.contains("auto-fixable"))
        }),
        "all exact field boundaries must be offered to `--fix`: {report:#?}",
    );
}

#[test]
fn tuple_fields_are_rewritten_to_their_caller_boundary() {
    let temp = tempdir().expect("create tuple-field fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "tuple_field_boundary_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create sources");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod a;\npub fn entry() -> u32 { a::entry() }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/a/mod.rs"),
        "mod b;\npub(crate) fn entry() -> u32 { b::entry() }\n",
    )
    .expect("write a module");
    fs::write(
        temp.path().join("src/a/b/mod.rs"),
        "mod c;\npub(super) fn entry() -> u32 { c::Tuple(4).0 }\n",
    )
    .expect("write b module");
    fs::write(
        temp.path().join("src/a/b/c/mod.rs"),
        "pub struct Tuple(\n    pub(crate) u32,\n);\n",
    )
    .expect("write c module");

    let manifest_path = temp.path().join("Cargo.toml");
    let report = run_mend_json(&manifest_path);
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::OverbroadPubCrate
                && finding.path.ends_with("src/a/b/c/mod.rs")
                && finding.line_start == 2
        })
        .unwrap_or_else(|| panic!("missing tuple-field finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|help| help == "consider using: `pub(super)`"),
        "the tuple field needs its constructor caller boundary: {report:#?}",
    );
    assert!(
        finding
            .help
            .iter()
            .any(|help| help.contains("auto-fixable")),
        "the tuple-field boundary must be offered to `--fix`: {report:#?}",
    );

    let output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest_path)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/a/b/c/mod.rs")).expect("read fixed tuple field"),
        "pub(super) struct Tuple(\n    pub(super) u32,\n);\n",
        "the tuple struct and field must receive only their required visibility",
    );

    let fixed_report = run_mend_json(&manifest_path);
    assert!(
        fixed_report.findings.iter().all(|finding| {
            !finding.path.ends_with("src/a/b/c/mod.rs")
                || !matches!(
                    finding.code,
                    DiagnosticCode::OverbroadPubCrate | DiagnosticCode::ForbiddenPubInCrate
                )
        }),
        "the applied tuple-field boundary must be accepted: {fixed_report:#?}",
    );
}

#[test]
fn crate_wide_tuple_field_keeps_pub_crate() {
    let temp = tempdir().expect("create crate-wide tuple-field fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "crate_wide_tuple_field_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create sources");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod consumer;\nmod values;\npub fn entry() -> u32 { consumer::entry() }\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/consumer.rs"),
        "pub(super) fn entry() -> u32 { crate::values::Tuple(4).0 }\n",
    )
    .expect("write consumer");
    fs::write(
        temp.path().join("src/values.rs"),
        "pub(crate) struct Tuple(pub(crate) u32);\n",
    )
    .expect("write values");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate
                || !finding.path.ends_with("src/values.rs")
        }),
        "crate-root sibling use requires `pub(crate)` on the tuple and its field: {report:#?}",
    );
}

fn assert_human_report_advertises_warning_fix(manifest_path: &std::path::Path) {
    let output = mend_command()
        .arg("--manifest-path")
        .arg(manifest_path)
        .output()
        .expect("run cargo-mend human report");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let rendered = strip_ansi(&format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    ));
    assert!(
        rendered.contains("this warning is auto-fixable with `cargo mend --fix`"),
        "the human diagnostic must advertise the annotation rewrite: {rendered}",
    );
    assert!(
        rendered.lines().any(|line| line.starts_with("summary:")
            && line.contains("mend warning")
            && line.contains("fixable with `cargo mend --fix`")),
        "the warning summary must count the annotation rewrite: {rendered}",
    );
}

#[test]
fn a_module_of_foreign_reexports_keeps_the_pub_crate_its_callers_need() {
    // A module whose whole content is `pub(crate) use other_crate::THING;`
    // is reached from outside its parent through the re-export. The item
    // at the end of that path lives in another crate, so walking up from
    // the target finds no local module to record -- and without the
    // modules the path itself names, this module read as reached only
    // from its own parent and had its visibility taken away, leaving the
    // crate not compiling.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members  = ["upstream", "app"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");

    fs::create_dir_all(temp.path().join("upstream/src")).expect("create upstream src");
    fs::write(
        temp.path().join("upstream/Cargo.toml"),
        r#"[package]
name = "reexport_upstream_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write upstream manifest");
    fs::write(
        temp.path().join("upstream/src/lib.rs"),
        "pub const DISCOVERY_INTERVAL: u32 = 5;\n",
    )
    .expect("write upstream lib");

    fs::create_dir_all(temp.path().join("app/src/screens")).expect("create app src");
    fs::write(
        temp.path().join("app/Cargo.toml"),
        r#"[package]
name = "reexport_app_fixture"
version = "0.1.0"
edition = "2024"

[dependencies]
reexport_upstream_fixture = { path = "../upstream" }
"#,
    )
    .expect("write app manifest");
    fs::write(
        temp.path().join("app/src/lib.rs"),
        "mod readiness;\nmod screens;\n",
    )
    .expect("write app lib");
    fs::write(
        temp.path().join("app/src/screens/mod.rs"),
        "pub(crate) mod constants;\n",
    )
    .expect("write screens");
    fs::write(
        temp.path().join("app/src/screens/constants.rs"),
        "pub(crate) use reexport_upstream_fixture::DISCOVERY_INTERVAL;\n",
    )
    .expect("write constants");
    fs::write(
        temp.path().join("app/src/readiness.rs"),
        "use crate::screens::constants::DISCOVERY_INTERVAL;\n\npub(crate) fn go() -> u32 { \
         DISCOVERY_INTERVAL }\n",
    )
    .expect("write readiness");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("app/Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run cargo-mend --json");
    let report = parse_mend_json_output(&output.stdout);

    let narrowed_the_module = report.findings.iter().any(|finding| {
        finding.code == DiagnosticCode::OverbroadPubCrate
            && finding.path.ends_with("screens/mod.rs")
    });
    assert!(
        !narrowed_the_module,
        "the re-export module is reached from outside `screens`: {report:#?}",
    );
}
