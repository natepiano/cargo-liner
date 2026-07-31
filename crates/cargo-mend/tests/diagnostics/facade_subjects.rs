use tempfile::TempDir;

use crate::support::*;

fn write_manifest(temp: &TempDir, package_name: &str, features: bool) {
    let feature_section = if features {
        "\n[features]\npromote = []\n"
    } else {
        ""
    };
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{feature_section}"
        ),
    )
    .expect("write fixture manifest");
}

fn has_unused_pub(report: &Report, path: &str) -> bool {
    report
        .findings
        .iter()
        .any(|finding| finding.code == DiagnosticCode::UnusedPub && finding.path == path)
}

fn has_pub_use_fix(report: &Report) -> bool {
    report
        .findings
        .iter()
        .any(|finding| finding.fix_support == FixSupport::PubUse)
}

fn has_suspicious_pub(report: &Report, path: &str) -> bool {
    report
        .findings
        .iter()
        .any(|finding| finding.code == DiagnosticCode::SuspiciousPub && finding.path == path)
}

#[test]
fn inactive_cfg_facade_does_not_count_as_a_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture module");
    write_manifest(&temp, "inactive_cfg_facade_fixture", true);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\n#[cfg(feature = \"promote\")]\npub use b::Thing;\n",
    )
    .expect("write inactive facade");
    fs::write(temp.path().join("src/a/b.rs"), "pub struct Thing;\n").expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !has_pub_use_fix(&report),
        "inactive facade must not create a pub-use fix: {report:#?}"
    );
}

#[test]
fn macro_generated_facade_counts_as_a_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture module");
    write_manifest(&temp, "macro_facade_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\nmacro_rules! expose { () => { pub use b::Thing; }; }\nexpose!();\n",
    )
    .expect("write macro facade");
    fs::write(temp.path().join("src/a/b.rs"), "pub struct Thing;\n").expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !has_unused_pub(&report, "src/a/b.rs"),
        "macro facade must suppress unused_pub: {report:#?}"
    );
}

#[test]
fn path_module_and_raw_identifier_facades_use_hir_module_identity() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture module");
    write_manifest(&temp, "path_and_raw_facade_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write facade module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "#[path = \"odd.rs\"]\nmod camera_panel;\nmod r#type;\npub(crate) use self::camera_panel::{CameraPanel, UsedInsideSubtree};\npub(crate) use self::r#type::RawPanel;\n",
    )
    .expect("write facade boundary");
    fs::write(
        temp.path().join("src/a/odd.rs"),
        "pub(crate) struct CameraPanel;\npub struct UsedInsideSubtree;\npub struct LocallyReferenced;\nfn uses_facade(_: super::UsedInsideSubtree) {}\nfn uses_local(_: crate::a::b::camera_panel::LocallyReferenced) {}\n",
    )
    .expect("write path module");
    fs::write(
        temp.path().join("src/a/b/type.rs"),
        "pub struct RawPanel;\n",
    )
    .expect("write raw module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/odd.rs"
        }),
        "#[path] facade must permit its nested pub(crate) subject: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::InternalParentPubUseFacade
                && finding.path == "src/a/b.rs"
                && finding.item.as_deref() == Some("pub(crate) use UsedInsideSubtree")
                && finding.headline
                    == "parent module `pub(crate) use` is acting as an internal facade"
        }),
        "a relative use from the logical #[path] module must be classified inside its facade subtree: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::UnusedPub
                && finding.path == "src/a/odd.rs"
                && finding.item.as_deref() == Some("struct LocallyReferenced")
        }),
        "a #[path] module's self-reference must not count as an outside use: {report:#?}"
    );
    assert!(
        !has_unused_pub(&report, "src/a/b/type.rs"),
        "raw identifier facade must suppress unused_pub: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/type.rs"
                && finding.help.iter().any(|help| {
                    help == "parent module also has an `unused import` warning for this \
                            `pub(crate) use` at a/b.rs:5"
                })
        }),
        "a known pub(crate) facade must be quoted exactly in the stale-facade note: {report:#?}"
    );
}

#[test]
fn spaced_facade_visibility_uses_resolved_reach() {
    let temp = tempdir().expect("create spaced visibility fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "spaced_facade_visibility_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\npub (crate) use child::Spaced;\n",
    )
    .expect("write spaced facade");
    fs::write(
        temp.path().join("src/a/b/child.rs"),
        "pub(crate) struct Spaced;\n",
    )
    .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/child.rs"
        }),
        "resolved pub(crate) facade reach must permit its subject: {report:#?}"
    );
}

#[test]
fn variant_reexport_normalizes_to_its_containing_enum() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "variant_subject_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\nmod unfacaded;\npub(crate) use c::Choice::Selected;\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(crate) enum Choice { Selected }\n",
    )
    .expect("write subjects");
    fs::write(
        temp.path().join("src/a/b/unfacaded.rs"),
        "pub(crate) struct Unfacaded;\n",
    )
    .expect("write control subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "the re-exported variant must normalize to Choice: {report:#?}"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.path == "src/a/b/unfacaded.rs"
            })
            .count(),
        1,
        "{report:#?}"
    );
}

#[test]
fn inherent_items_follow_their_self_type_facade_subject() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "inherent_visibility_cap_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\nmod unfacaded;\npub(crate) use c::Widget;\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(crate) struct Widget;\nimpl Widget { pub(crate) fn facade_method() {} pub(crate) const FACADE_CONST: usize = 1; }\n",
    )
    .expect("write facade-reachable inherent items");
    fs::write(
        temp.path().join("src/a/b/unfacaded.rs"),
        "pub(crate) struct Unfacaded;\n",
    )
    .expect("write control subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "the Widget facade must reach its associated items through their self type: {report:#?}"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate
                    && finding.path == "src/a/b/unfacaded.rs"
            })
            .count(),
        1,
        "{report:#?}"
    );
}

#[test]
fn non_applicable_named_facade_falls_back_to_glob() {
    let temp = tempdir().expect("create named and glob fallback fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "named_glob_fallback_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub use c::Widget as PublicWidget;\npub(crate) use c::*;\n",
    )
    .expect("write named and glob facades");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub struct Widget;\nimpl Widget { pub(crate) fn through_glob() {} }\n",
    )
    .expect("write inherent subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "a named facade above the method cap must fall back to the matching glob: {report:#?}"
    );
}

#[test]
fn inherent_glob_uses_the_self_type_module() {
    let temp = tempdir().expect("create split inherent fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture module");
    write_manifest(&temp, "split_inherent_glob_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod types;\nmod impls;\npub(crate) use types::*;\n",
    )
    .expect("write glob facade");
    fs::write(
        temp.path().join("src/a/b/types.rs"),
        "pub(crate) struct Widget;\n",
    )
    .expect("write self type");
    fs::write(
        temp.path().join("src/a/b/impls.rs"),
        "use super::types::Widget;\nimpl Widget { pub(crate) fn through_type_glob() {} }\n",
    )
    .expect("write inherent implementation");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/impls.rs"
        }),
        "an inherent item must query globs for its self type module: {report:#?}"
    );
}

#[test]
fn extern_crate_reexport_matches_the_local_declaration_subject() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    write_manifest(&temp, "extern_crate_subject_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "mod d;\npub(crate) use d::core_alias;\npub use core::fmt::Error;\n",
    )
    .expect("write extern facade");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) extern crate core as core_alias;\n",
    )
    .expect("write extern crate declaration");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c/d.rs"
        }),
        "the local extern crate facade must not be treated as foreign: {report:#?}"
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.path == "src/a/b/c/d.rs"
                && finding
                    .help
                    .iter()
                    .any(|help| help.contains("consider using: `pub`"))
        }),
        "extern crate subject must never produce a synthetic pub suggestion: {report:#?}"
    );
}

#[test]
fn inactive_and_macro_generated_globs_follow_active_hir() {
    let inactive = tempdir().expect("create inactive glob fixture dir");
    fs::create_dir_all(inactive.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&inactive, "inactive_glob_facade_fixture", true);
    fs::write(
        inactive.path().join("src/main.rs"),
        "mod a;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(inactive.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        inactive.path().join("src/a/b.rs"),
        "mod c;\n#[cfg(feature = \"promote\")]\npub(crate) use c::*;\npub(crate) use c::Named;\n",
    )
    .expect("write inactive glob facade");
    fs::write(
        inactive.path().join("src/a/b/c.rs"),
        "pub(crate) struct Globbed;\npub(crate) struct Named;\n",
    )
    .expect("write glob subjects");

    let inactive_report = run_mend_json(&inactive.path().join("Cargo.toml"));
    assert_eq!(
        inactive_report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
            })
            .count(),
        1,
        "inactive glob must leave Globbed forbidden while the active named facade permits Named: {inactive_report:#?}"
    );

    let generated = tempdir().expect("create generated glob fixture dir");
    fs::create_dir_all(generated.path().join("src/a")).expect("create fixture module");
    write_manifest(&generated, "macro_glob_facade_fixture", false);
    fs::write(
        generated.path().join("src/main.rs"),
        "mod a;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        generated.path().join("src/a.rs"),
        "mod b;\nmacro_rules! expose { () => { pub use b::*; }; }\nexpose!();\n",
    )
    .expect("write macro glob facade");
    fs::write(generated.path().join("src/a/b.rs"), "pub struct Globbed;\n")
        .expect("write glob subject");

    let generated_report = run_mend_json(&generated.path().join("Cargo.toml"));
    assert!(
        !has_unused_pub(&generated_report, "src/a/b.rs"),
        "macro glob facade must suppress unused_pub: {generated_report:#?}"
    );

    let precedence = tempdir().expect("create named and glob fixture dir");
    fs::create_dir_all(precedence.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&precedence, "named_before_glob_fixture", false);
    fs::write(
        precedence.path().join("src/main.rs"),
        "mod a;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(precedence.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        precedence.path().join("src/a/b.rs"),
        "mod c;\npub(crate) use c::Named;\npub(super) use c::*;\n",
    )
    .expect("write active named and glob facades");
    fs::write(
        precedence.path().join("src/a/b/c.rs"),
        "pub(crate) struct Named;\npub(crate) struct Globbed;\n",
    )
    .expect("write precedence subjects");

    let precedence_report = run_mend_json(&precedence.path().join("Cargo.toml"));
    assert_eq!(
        precedence_report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
            })
            .count(),
        1,
        "{precedence_report:#?}"
    );
}

#[test]
fn private_import_does_not_suppress_unused_pub() {
    let temp = tempdir().expect("create private import fixture dir");
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture module");
    write_manifest(&temp, "private_import_facade_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\nmacro_rules! import_thing { () => { use child::Thing; }; }\nimport_thing!();\n",
    )
    .expect("write private import macro");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write unused public item");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        has_unused_pub(&report, "src/a/child.rs"),
        "a private import must not count as a facade: {report:#?}"
    );
}

#[test]
fn crate_root_pub_crate_facade_keeps_deep_subject_allowed() {
    let temp = tempdir().expect("create crate-root facade fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "crate_root_pub_crate_facade_fixture", false);
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\npub(crate) use a::b::c::Facaded;\nfn main() { let _ = Facaded; }\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "pub(crate) mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "pub(crate) mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(crate) struct Facaded;\npub(crate) struct Unfacaded;\n",
    )
    .expect("write facade subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden_count = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/c.rs"
        })
        .count();
    assert_eq!(
        forbidden_count, 1,
        "only the unfacaded deep subject should be forbidden: {report:#?}"
    );
}

#[test]
fn restricted_parent_facades_require_manual_cleanup() {
    for (package_name, facade_visibility) in [
        ("pub_crate_parent_facade_fixture", "pub(crate)"),
        ("pub_in_parent_facade_fixture", "pub(in crate::a)"),
    ] {
        let temp = tempdir().expect("create restricted facade fixture dir");
        fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create deep fixture modules");
        write_manifest(&temp, package_name, false);
        fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
            .expect("write fixture main");
        fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
        fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
        fs::write(
            temp.path().join("src/a/b/c.rs"),
            format!("mod child;\n{facade_visibility} use child::Thing;\n"),
        )
        .expect("write restricted facade");
        fs::write(
            temp.path().join("src/a/b/c/child.rs"),
            "pub struct Thing;\n",
        )
        .expect("write facade subject");

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        let finding = report
            .findings
            .iter()
            .find(|finding| {
                finding.code == DiagnosticCode::SuspiciousPub
                    && finding.path == "src/a/b/c/child.rs"
            })
            .expect("find stale restricted facade result");
        assert_ne!(
            finding.fix_support,
            FixSupport::PubUse,
            "restricted facades cannot advertise a pub-use rewrite: {report:#?}"
        );
    }
}

#[test]
fn logical_top_level_path_module_uses_top_level_diagnostics() {
    let temp = tempdir().expect("create logical top-level path fixture dir");
    fs::create_dir_all(temp.path().join("src/deep")).expect("create path module directory");
    write_manifest(&temp, "logical_top_level_path_fixture", false);
    fs::write(
        temp.path().join("src/main.rs"),
        "#[path = \"deep/odd.rs\"]\nmod facade;\nfn main() { facade::used(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/deep/odd.rs"),
        "mod child { pub struct Exported; }\npub use child::*;\npub fn used() {}\n",
    )
    .expect("write logical top-level module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::NarrowToPubCrate
                && finding.path == "src/deep/odd.rs"
                && finding.item.as_deref() == Some("fn used")
        }),
        "logical top-level items must use the top-level narrowing diagnostic: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::WildcardParentPubUse
                && finding.path == "src/deep/odd.rs"
        }),
        "logical top-level wildcard facades must be reviewed regardless of file layout: {report:#?}"
    );
}

#[test]
fn inline_sibling_signature_exposes_deep_inline_subject() {
    let temp = tempdir().expect("create inline signature fixture dir");
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");
    write_manifest(&temp, "inline_sibling_signature_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\nmod caller;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod target { pub(crate) struct Exposed; pub(crate) struct Hidden; }\nmod api { pub fn make() -> super::target::Exposed { super::target::Exposed } }\npub(crate) use api::make;\n",
    )
    .expect("write inline target and signature modules");
    fs::write(
        temp.path().join("src/a/caller.rs"),
        "fn call() { let _ = super::b::make(); }\n",
    )
    .expect("write signature caller");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let forbidden_help = report
        .findings
        .iter()
        .filter(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b.rs"
        })
        .flat_map(|finding| &finding.help)
        .collect::<Vec<_>>();
    assert!(
        forbidden_help
            .iter()
            .any(|help| help.contains("this item is exposed through a public signature")),
        "Exposed must receive the inline signature recommendation: {report:#?}"
    );
    assert!(
        forbidden_help
            .iter()
            .any(|help| help.contains("consider using `pub(super)`")),
        "Hidden must retain the non-exposed recommendation: {report:#?}"
    );
}

#[test]
fn module_glob_reexports_enum_subjects() {
    let temp = tempdir().expect("create enum glob fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "module_glob_enum_subject_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\npub(crate) use child::*;\n",
    )
    .expect("write glob facade");
    fs::write(
        temp.path().join("src/a/b/child.rs"),
        "pub(crate) enum Choice { Selected }\n",
    )
    .expect("write enum subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ForbiddenPubCrate && finding.path == "src/a/b/child.rs"
        }),
        "a module glob must reach the contained enum: {report:#?}"
    );
}

#[test]
fn duplicate_facades_collectively_reject_fixes_and_track_later_alias_usage() {
    let temp = tempdir().expect("create duplicate facade fixture dir");
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture module");
    write_manifest(&temp, "duplicate_facade_fix_support_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\npub use child::Thing as RenamedThing;\n",
    )
    .expect("write duplicate facades");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/a/child.rs"
        })
        .expect("find suspicious-pub result for the duplicate facade");
    assert_ne!(finding.fix_support, FixSupport::PubUse, "{report:#?}");

    let alias_use = tempdir().expect("create later alias usage fixture dir");
    fs::create_dir_all(alias_use.path().join("src/a")).expect("create fixture module");
    write_manifest(&alias_use, "duplicate_facade_alias_usage_fixture", false);
    fs::write(
        alias_use.path().join("src/main.rs"),
        "mod a;\nmod caller;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        alias_use.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\npub use child::Thing as RenamedThing;\n",
    )
    .expect("write duplicate facades");
    fs::write(
        alias_use.path().join("src/a/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write facade subject");
    fs::write(
        alias_use.path().join("src/caller.rs"),
        "fn touch(_: crate::a::RenamedThing) {}\n",
    )
    .expect("write later alias use");

    let alias_use_report = run_mend_json(&alias_use.path().join("Cargo.toml"));
    assert!(
        !has_suspicious_pub(&alias_use_report, "src/a/child.rs"),
        "a later re-export alias used outside the facade subtree must suppress suspicious_pub: {alias_use_report:#?}"
    );

    let mixed_visibility = tempdir().expect("create mixed visibility facade fixture dir");
    fs::create_dir_all(mixed_visibility.path().join("src/a")).expect("create fixture module");
    write_manifest(
        &mixed_visibility,
        "duplicate_facade_mixed_visibility_fixture",
        false,
    );
    fs::write(
        mixed_visibility.path().join("src/main.rs"),
        "mod a;\nmod caller;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        mixed_visibility.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\npub(crate) use child::Thing as InternalThing;\n",
    )
    .expect("write mixed visibility facades");
    fs::write(
        mixed_visibility.path().join("src/a/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write facade subject");
    fs::write(
        mixed_visibility.path().join("src/caller.rs"),
        "use crate::a::InternalThing;\n",
    )
    .expect("write narrow alias use");

    let mixed_visibility_report = run_mend_json(&mixed_visibility.path().join("Cargo.toml"));
    assert!(
        has_suspicious_pub(&mixed_visibility_report, "src/a/child.rs"),
        "usage through a pub(crate) alias must not justify the unused pub facade: {mixed_visibility_report:#?}"
    );
}

#[test]
fn inline_module_references_use_their_lexical_module_identity() {
    let temp = tempdir().expect("create inline lexical reference fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "inline_lexical_reference_fixture", false);
    fs::write(
        temp.path().join("src/main.rs"),
        "mod child;\nmod a;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\nmod inner { mod deeper { fn marker(_: Option<()>) {} } }\nfn top_level_reference(_: super::super::child::Thing) {}\n",
    )
    .expect("write inline and top-level references");
    fs::write(
        temp.path().join("src/child.rs"),
        "pub(crate) struct Thing;\n",
    )
    .expect("write real relative path target");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write unrelated nested subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        has_unused_pub(&report, "src/a/b/child.rs"),
        "a top-level super path must resolve in the file module, not its inline child: {report:#?}"
    );
}

#[test]
fn equal_reach_usage_stays_attached_to_its_own_spelling() {
    for (package_name, facades) in [
        (
            "equal_reach_super_first_fixture",
            "mod b;\npub(super) use b::c::Thing;\npub(crate) use b::c::Thing as InternalThing;\n",
        ),
        (
            "equal_reach_crate_first_fixture",
            "mod b;\npub(crate) use b::c::Thing as InternalThing;\npub(super) use b::c::Thing;\n",
        ),
    ] {
        let temp = tempdir().expect("create equal-reach facade fixture dir");
        fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
        write_manifest(&temp, package_name, false);
        fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
            .expect("write fixture main");
        fs::write(temp.path().join("src/a.rs"), facades).expect("write conflicting facades");
        fs::write(
            temp.path().join("src/a/b.rs"),
            "pub(super) mod c;\nmod user;\n",
        )
        .expect("write middle module");
        fs::write(temp.path().join("src/a/b/c.rs"), "pub struct Thing;\n")
            .expect("write facade subject");
        fs::write(
            temp.path().join("src/a/b/user.rs"),
            "fn use_alias(_: super::super::InternalThing) {}\n",
        )
        .expect("write used crate-spelled alias");

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert!(
            report.findings.iter().any(|finding| {
                finding.code == DiagnosticCode::InternalParentPubUseFacade
                    && finding.path == "src/a.rs"
            }),
            "usage from the crate-spelled alias must be assessed without the unused super-spelled alias: {report:#?}"
        );
    }
}

#[test]
fn equal_reach_crate_and_other_spellings_do_not_recommend_narrowing() {
    for (package_name, facades) in [
        (
            "equal_reach_crate_then_other_fixture",
            "mod b;\npub(crate) use b::c::Thing;\npub(in crate) use b::c::Thing as RestrictedThing;\n",
        ),
        (
            "equal_reach_other_then_crate_fixture",
            "mod b;\npub(in crate) use b::c::Thing as RestrictedThing;\npub(crate) use b::c::Thing;\n",
        ),
    ] {
        let temp = tempdir().expect("create crate and other spelling fixture dir");
        fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
        write_manifest(&temp, package_name, false);
        fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
            .expect("write fixture main");
        fs::write(temp.path().join("src/a.rs"), facades).expect("write conflicting facades");
        fs::write(temp.path().join("src/a/b.rs"), "pub(super) mod c;\n")
            .expect("write middle module");
        fs::write(temp.path().join("src/a/b/c.rs"), "pub struct Thing;\n")
            .expect("write facade subject");

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        assert!(
            !report.findings.iter().any(|finding| {
                finding.code == DiagnosticCode::NarrowToPubCrate && finding.path == "src/a/b/c.rs"
            }),
            "conflicting equal-reach crate spellings must suppress narrowing: {report:#?}"
        );
    }
}
