use serde_json::Value;
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/odd.rs"
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/child.rs"
        }),
        "resolved pub(crate) facade reach must permit its subject: {report:#?}"
    );
}

#[test]
fn variant_reexport_normalizes_to_its_containing_enum() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "the re-exported variant must normalize to Choice: {report:#?}"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::OverbroadPubCrate
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
        }),
        "the Widget facade must reach its associated items through their self type: {report:#?}"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::OverbroadPubCrate
                    && finding.path == "src/a/b/unfacaded.rs"
            })
            .count(),
        1,
        "{report:#?}"
    );
}

#[test]
fn non_applicable_named_facade_keeps_manual_cleanup_when_a_glob_blocks_it() {
    let temp = tempdir().expect("create named and glob fallback fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    assert_pub_crate_is_retained_at_unresolved_glob(&report, "src/a/b/c.rs");
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/a/b/c.rs"
        })
        .unwrap_or_else(|| panic!("missing stale named-facade finding: {report:#?}"));
    assert!(
        finding
            .help
            .iter()
            .any(|line| line == "consider using: `pub(super)`")
    );
    assert!(
        finding
            .help
            .iter()
            .all(|line| !line.contains("auto-fixable"))
    );
}

#[test]
fn ancestor_glob_targeting_descendant_module_retains_exact_pub_crate() {
    let temp = tempdir().expect("create descendant glob target fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "descendant_glob_target_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\npub(crate) use b::c::*;\n",
    )
    .expect("write ancestor glob facade");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "pub(super) mod c;\nmod unused;\n",
    )
    .expect("write intermediate module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(crate) struct Globbed;\n",
    )
    .expect("write glob subject");
    fs::write(
        temp.path().join("src/a/b/unused.rs"),
        "pub(crate) struct Unused;\n",
    )
    .expect("write unused control subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_pub_crate_is_retained_at_unresolved_glob(&report, "src/a/b/c.rs");
    assert!(
        !has_unused_pub(&report, "src/a/b/c.rs"),
        "the ancestor glob must suppress unused_pub for Globbed: {report:#?}"
    );
    let unused_control = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/unused.rs"
        })
        .unwrap_or_else(|| panic!("missing unused control finding: {report:#?}"));
    assert!(
        unused_control
            .help
            .iter()
            .any(|help| help == "consider removing the visibility"),
        "the unrelated control subject must not inherit the glob blocker: {report:#?}"
    );
}

#[test]
fn unused_named_facade_with_outer_glob_has_no_pub_use_fix() {
    let temp = tempdir().expect("create named facade and outer glob fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "named_facade_outer_glob_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\npub use b::*;\n")
        .expect("write outer glob facade");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\npub use child::Thing;\n",
    )
    .expect("write nearest named facade");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/a/b/child.rs"
        })
        .unwrap_or_else(|| panic!("missing stale facade finding: {report:#?}"));
    assert_eq!(
        finding.fix_support,
        FixSupport::None,
        "an unresolvable chain must not advertise a pub-use rewrite: {report:#?}"
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0, "{report:#?}");
}

#[test]
fn inherent_glob_retains_exact_pub_crate_through_the_self_type_module() {
    let temp = tempdir().expect("create split inherent fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    assert_pub_crate_is_retained_at_unresolved_glob(&report, "src/a/b/impls.rs");
    assert!(!has_unused_pub(&report, "src/a/b/impls.rs"), "{report:#?}");
}

#[test]
fn glob_usage_is_attributed_to_the_matching_export_name() {
    let temp = tempdir().expect("create per-name glob fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    write_manifest(&temp, "per_name_glob_usage_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "mod consumer;\nmod exports;\npub(super) use exports::*;\n",
    )
    .expect("write glob facade");
    fs::write(
        temp.path().join("src/a/b/c/consumer.rs"),
        "macro_rules! mention { () => { stringify!(crate::a::b::c::FirstExtra) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write glob consumer");
    fs::write(
        temp.path().join("src/a/b/c/exports.rs"),
        "pub struct First;\npub struct FirstExtra;\n",
    )
    .expect("write glob subjects");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/c/exports.rs"
                && finding.item.as_deref() == Some("struct First")
        }),
        "the unused glob export must retain its stale-facade finding: {report:#?}"
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/c/exports.rs"
                && finding.item.as_deref() == Some("struct FirstExtra")
        }),
        "the used glob export must not inherit the unused export's finding: {report:#?}"
    );
}

#[test]
fn raw_identifier_glob_usage_is_attributed_to_the_unraw_export_name() {
    let temp = tempdir().expect("create raw glob usage fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "raw_glob_usage_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\nmod macro_only;\npub(super) use child::*;\n",
    )
    .expect("write glob facade");
    fs::write(
        temp.path().join("src/a/b/child.rs"),
        "pub fn r#type() {}\npub fn other() {}\n",
    )
    .expect("write raw and non-raw facade subjects");
    fs::write(
        temp.path().join("src/a/b/macro_only.rs"),
        "macro_rules! mention { () => { stringify!(crate::a::b::r#type) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write raw identifier macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("fn r#type")
        }),
        "the used raw glob export must not receive a stale-facade finding: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("fn other")
        }),
        "the unused sibling glob export must retain its stale-facade finding: {report:#?}"
    );
}

#[test]
fn raw_identifier_module_segments_match_literal_and_parsed_paths() {
    let temp = tempdir().expect("create raw module usage fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/type/inner")).expect("create raw module fixtures");
    write_manifest(&temp, "raw_module_usage_fixture", false);
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\n#[path = \"../shared.rs\"]\nmod macro_only;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod r#type;\n").expect("write raw module parent");
    fs::write(
        temp.path().join("src/a/type.rs"),
        "mod consumer;\nmod inner;\n",
    )
    .expect("write keyword module");
    fs::write(
        temp.path().join("src/a/type/consumer.rs"),
        "fn use_parsed_path() { super::r#inner::parsed_path(); }\n",
    )
    .expect("write parsed raw module reference");
    fs::write(
        temp.path().join("src/a/type/inner.rs"),
        "mod child;\npub(super) use child::*;\n",
    )
    .expect("write raw module facade");
    fs::write(
        temp.path().join("src/a/type/inner/child.rs"),
        "pub fn macro_path() {}\npub fn parsed_path() {}\npub fn unused() {}\n",
    )
    .expect("write raw module facade subjects");
    fs::write(
        temp.path().join("shared.rs"),
        "macro_rules! mention { () => { stringify!(crate::a::r#type::r#inner::macro_path) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write raw module macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/type/inner/child.rs"
                && finding.item.as_deref() == Some("fn macro_path")
        }),
        "the macro literal through a raw module must retain its facade: {report:#?}"
    );
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/type/inner/child.rs"
                && finding.item.as_deref() == Some("fn parsed_path")
        }),
        "the parsed path through a raw module must retain its facade: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/type/inner/child.rs"
                && finding.item.as_deref() == Some("fn unused")
        }),
        "the unmentioned sibling must retain its stale-facade finding: {report:#?}"
    );
}

#[test]
fn extern_crate_declaration_retains_exact_pub_crate_at_a_foreign_boundary() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate || finding.path != "src/a/b/c/d.rs"
        }),
        "an unresolved foreign boundary must retain exact pub(crate): {report:#?}",
    );
}

#[test]
fn cargo_renamed_extern_crate_reports_a_foreign_chain_blocker() {
    let temp = tempdir().expect("create renamed dependency fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::create_dir_all(temp.path().join("actual-dependency/src"))
        .expect("create dependency source directory");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "renamed_extern_crate_fixture"
version = "0.1.0"
edition = "2024"

[dependencies]
alias = { package = "actual-dependency", path = "actual-dependency" }
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("actual-dependency/Cargo.toml"),
        r#"[package]
name = "actual-dependency"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write dependency manifest");
    fs::write(
        temp.path().join("actual-dependency/src/lib.rs"),
        "pub struct DependencyMarker;\n",
    )
    .expect("write dependency library");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "mod d;\npub(crate) use d::alias;\n",
    )
    .expect("write renamed extern facade");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) extern crate alias;\n",
    )
    .expect("write renamed extern crate declaration");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run cargo-mend against renamed dependency fixture");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_mend_json_output(&output.stdout);
    assert_foreign_boundary_finding(&report, "src/a/b/c/d.rs", "a/b/c/d.rs:1");
    assert_foreign_boundary_finding(&report, "src/a/b/c.rs", "a/b/c.rs:2");
    assert_no_stored_pub_use_fix_facts(&temp);
}

#[test]
fn foreign_dependency_glob_reports_a_foreign_chain_blocker() {
    let temp = tempdir().expect("create foreign glob fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b/c")).expect("create fixture modules");
    fs::create_dir_all(temp.path().join("actual-dependency/src"))
        .expect("create dependency source directory");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "foreign_glob_fixture"
version = "0.1.0"
edition = "2024"

[dependencies]
actual-dependency = { path = "actual-dependency" }
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("actual-dependency/Cargo.toml"),
        r#"[package]
name = "actual-dependency"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write dependency manifest");
    fs::write(
        temp.path().join("actual-dependency/src/lib.rs"),
        "pub struct DependencyMarker;\n",
    )
    .expect("write dependency library");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(temp.path().join("src/a/b.rs"), "mod c;\n").expect("write middle module");
    fs::write(temp.path().join("src/a/b/c.rs"), "mod d;\n").expect("write inner module");
    fs::write(
        temp.path().join("src/a/b/c/d.rs"),
        "pub(crate) use actual_dependency::*;\n",
    )
    .expect("write foreign glob re-export");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_foreign_boundary_finding(&report, "src/a/b/c/d.rs", "a/b/c/d.rs:1");
    assert_no_stored_pub_use_fix_facts(&temp);
}

fn assert_foreign_boundary_finding(report: &Report, path: &str, _blocker_location: &str) {
    assert!(
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate || finding.path != path
        }),
        "an unresolved foreign boundary must retain exact pub(crate) at {path}: {report:#?}",
    );
}

fn assert_no_stored_pub_use_fix_facts(temp: &TempDir) {
    let findings_dir = temp.path().join("target/mend-findings");
    let mut stored_report_count = 0;
    for entry in fs::read_dir(&findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        stored_report_count += 1;
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        let fix_facts = stored_report
            .get("pub_use_fix_facts")
            .and_then(Value::as_array)
            .expect("read stored pub-use fix facts");
        assert!(
            fix_facts.is_empty(),
            "a foreign chain blocker must not write a pub-use fix fact: {stored_report:#?}"
        );
    }
    assert!(stored_report_count > 0, "missing stored findings report");
}

#[test]
fn inactive_and_macro_generated_globs_follow_active_hir() {
    let inactive = tempdir().expect("create inactive glob fixture dir");
    pin_pub_in_path(inactive.path(), PubInPath::Permitted);
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
                finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
            })
            .count(),
        1,
        "the inactive glob must not protect Globbed while the active named facade permits Named: {inactive_report:#?}"
    );

    let generated = tempdir().expect("create generated glob fixture dir");

    pin_pub_in_path(generated.path(), PubInPath::Permitted);
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

    pin_pub_in_path(precedence.path(), PubInPath::Permitted);
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
                finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
            })
            .count(),
        0,
        "the named facade permits Named while unresolved glob reach leaves exact pub(crate) on Globbed unchanged: {precedence_report:#?}"
    );
}

#[test]
fn private_import_defers_narrowing_until_the_import_is_removed() {
    let temp = tempdir().expect("create private import fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
        "the private import must not count as semantic use: {report:#?}"
    );
    assert_eq!(report.summary.fixable_with_fix, 0, "{report:#?}");
}

#[test]
fn crate_root_pub_crate_facade_keeps_deep_subject_allowed() {
    let temp = tempdir().expect("create crate-root facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b/c.rs"
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
        pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/deep")).expect("create path module directory");
    write_manifest(&temp, "logical_top_level_path_fixture", false);
    fs::write(
        temp.path().join("src/lib.rs"),
        "#[path = \"deep/odd.rs\"]\nmod facade;\npub fn entry() { facade::used(); }\n",
    )
    .expect("write fixture library");
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
fn feature_excluded_module_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create feature-excluded module fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "feature_excluded_facade_usage_fixture"
version = "0.1.0"
edition = "2024"

[features]
hidden = []
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\n#[cfg(feature = \"hidden\")]\nmod hidden;\nfn main() { #[cfg(feature = \"hidden\")] hidden::use_it(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::{Thing, UnusedThing};\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/child.rs"),
        "pub struct Thing;\npub struct UnusedThing;\n",
    )
    .expect("write facade subjects");
    fs::write(
        temp.path().join("src/hidden.rs"),
        "pub fn use_it() { let _thing: crate::a::Thing = crate::a::Thing; }\n",
    )
    .expect("write feature-excluded consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            finding.path != "src/a/child.rs"
                || finding.item.as_deref() != Some("struct Thing")
                || finding.code != DiagnosticCode::SuspiciousPub
        }),
        "the feature-excluded crate path must retain Thing's facade: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct UnusedThing")
                && finding.code == DiagnosticCode::SuspiciousPub
        }),
        "the unreferenced sibling must retain its stale-facade finding: {report:#?}"
    );
}

#[test]
fn cfg_test_module_literal_counts_as_facade_usage_in_default_run() {
    let temp = tempdir().expect("create cfg-test module fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");
    write_manifest(&temp, "cfg_test_facade_usage_fixture", false);
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod a;\n#[cfg(test)]\nmod tests;\npub fn entry() {}\n",
    )
    .expect("write fixture library");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::{Thing, UnusedThing};\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/child.rs"),
        "pub struct Thing;\npub struct UnusedThing;\n",
    )
    .expect("write facade subjects");
    fs::write(
        temp.path().join("src/tests.rs"),
        "#[test]\nfn uses_facade() { let _thing: crate::a::Thing = crate::a::Thing; }\n",
    )
    .expect("write cfg-test consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().all(|finding| {
            finding.path != "src/a/child.rs"
                || finding.item.as_deref() != Some("struct Thing")
                || finding.code != DiagnosticCode::SuspiciousPub
        }),
        "the cfg-test crate path must retain Thing's facade: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct UnusedThing")
                && finding.code == DiagnosticCode::SuspiciousPub
        }),
        "the unreferenced sibling must retain its stale-facade finding: {report:#?}"
    );
}

#[test]
fn inline_sibling_signature_uses_its_restricted_outward_reach() {
    let temp = tempdir().expect("create inline signature fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
            finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b.rs"
        })
        .flat_map(|finding| &finding.help)
        .collect::<Vec<_>>();
    assert!(
        forbidden_help
            .iter()
            .any(|help| help.as_str() == "consider removing the visibility"),
        "Hidden must retain the non-exposed recommendation: {report:#?}"
    );
    assert!(
        forbidden_help
            .iter()
            .all(|help| !help.contains("consider using: `pub(crate)`")),
        "Exposed's exact crate-visible signature boundary must be accepted: {report:#?}"
    );
    assert_eq!(
        report
            .findings
            .iter()
            .filter(|finding| {
                finding.code == DiagnosticCode::OverbroadPubCrate && finding.path == "src/a/b.rs"
            })
            .count(),
        1,
        "only the unexposed annotation should remain a finding: {report:#?}",
    );
}

#[test]
fn module_glob_reexport_retains_exact_pub_crate_on_enum_subjects() {
    let temp = tempdir().expect("create enum glob fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    assert_pub_crate_is_retained_at_unresolved_glob(&report, "src/a/b/child.rs");
}

fn assert_pub_crate_is_retained_at_unresolved_glob(report: &Report, path: &str) {
    assert!(
        report.findings.iter().all(|finding| {
            finding.code != DiagnosticCode::OverbroadPubCrate || finding.path != path
        }),
        "an unresolved glob must retain exact pub(crate) at {path}: {report:#?}",
    );
}

#[test]
fn duplicate_facades_collectively_reject_fixes_and_track_later_alias_usage() {
    let temp = tempdir().expect("create duplicate facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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

    pin_pub_in_path(alias_use.path(), PubInPath::Permitted);
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

    pin_pub_in_path(mixed_visibility.path(), PubInPath::Permitted);
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
        "usage through a narrower alias must not hide an unused wider facade: {mixed_visibility_report:#?}"
    );
}

#[test]
fn used_inner_super_alias_keeps_its_allowance_below_a_wider_facade() {
    let temp = tempdir().expect("create mixed-reach facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    write_manifest(&temp, "used_inner_super_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod b;\npub(crate) use b::child::thing;\n",
    )
    .expect("write wider outer facade");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "pub(super) mod child;\nmod user;\npub(super) use child::thing as parent_thing;\n",
    )
    .expect("write used inner facade");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub fn thing() {}\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/b/user.rs"),
        "fn use_parent_alias() { super::parent_thing(); }\n",
    )
    .expect("write super alias use");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !has_suspicious_pub(&report, "src/a/b/child.rs"),
        "the used inner pub(super) alias must retain its allowance below the unused wider facade: {report:#?}"
    );
}

#[test]
fn used_super_alias_keeps_its_allowance_at_equal_reach() {
    let temp = tempdir().expect("create equal-reach facade fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture module");
    write_manifest(&temp, "used_super_equal_reach_fixture", false);
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\nmod user;\npub(crate) use child::Thing;\npub(super) use child::Thing as ParentThing;\n",
    )
    .expect("write equal-reach facades");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/user.rs"),
        "fn use_parent_alias(_: super::ParentThing) {}\n",
    )
    .expect("write super alias use");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !has_suspicious_pub(&report, "src/a/child.rs"),
        "equal resolved reaches must not erase the used pub(super) spelling: {report:#?}"
    );
}

#[test]
fn inline_module_references_use_their_lexical_module_identity() {
    let temp = tempdir().expect("create inline lexical reference fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
        pin_pub_in_path(temp.path(), PubInPath::Permitted);
        fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
        write_manifest(&temp, package_name, false);
        fs::write(
            temp.path().join("src/lib.rs"),
            "mod a;\npub fn entry() {}\n",
        )
        .expect("write fixture library");
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
        pin_pub_in_path(temp.path(), PubInPath::Permitted);
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

fn write_macro_consumed_facade_fixture(temp: &TempDir) {
    for (relative_path, source) in [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"app\", \"macros\"]\nresolver = \"3\"\n",
        ),
        ("mend.toml", "[visibility]\npub_in_path = \"permitted\"\n"),
        (
            "app/Cargo.toml",
            "[package]\nname = \"macro_consumed_facade_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nmacros_fixture = { path = \"../macros\" }\n",
        ),
        (
            "app/src/main.rs",
            "mod consumer;\nmod tool;\n\nfn main() {\n    consumer::run();\n}\n",
        ),
        (
            "app/src/consumer.rs",
            "macros_fixture::make_widget!();\n\npub(crate) fn run() {\n    crate::tool::touch();\n    let _ = make();\n}\n",
        ),
        (
            "app/src/tool/mod.rs",
            "mod inner;\nmod local_only;\nmod widget;\n\npub use local_only::LocalOnly;\npub use widget::Widget;\n\npub(crate) fn touch() {\n    inner::use_both();\n}\n",
        ),
        (
            "app/src/tool/inner.rs",
            "use super::LocalOnly;\nuse super::Widget;\n\npub(super) fn use_both() {\n    let _ = LocalOnly;\n    let _ = Widget;\n}\n",
        ),
        ("app/src/tool/local_only.rs", "pub struct LocalOnly;\n"),
        ("app/src/tool/widget.rs", "pub struct Widget;\n"),
        (
            "macros/Cargo.toml",
            "[package]\nname = \"macros_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[lib]\nproc-macro = true\n",
        ),
        (
            "macros/src/lib.rs",
            "use proc_macro::TokenStream;\n\n#[proc_macro]\npub fn make_widget(_: TokenStream) -> TokenStream {\n    \"fn make() -> crate::tool::Widget { crate::tool::Widget }\"\n        .parse()\n        .expect(\"parse generated widget factory\")\n}\n",
        ),
    ] {
        let path = temp.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture source directory");
        }
        fs::write(path, source).expect("write fixture source");
    }
}

// The facade scan reads source text, so a path a proc macro in another crate
// emits is invisible to it: `crate::tool::Widget` lives only inside
// `macros_fixture`'s generated token stream. Both facades below are imported
// relatively from inside `crate::tool`, which is what makes them candidates —
// only the HIR use sites, collected after macro expansion, separate the one
// that also serves `crate::consumer` from the one that does not.
#[test]
fn a_facade_a_proc_macro_reaches_is_not_reported_as_internal() {
    let temp = tempdir().expect("create macro consumed facade fixture dir");
    write_macro_consumed_facade_fixture(&temp);

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let facade_finding = |item: &str| {
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::InternalParentPubUseFacade
                && finding.path == "app/src/tool/mod.rs"
                && finding.item.as_deref() == Some(item)
        })
    };
    assert!(
        !facade_finding("pub use Widget"),
        "a facade reached by macro-generated code must not be reported as internal: {report:#?}"
    );
    assert!(
        facade_finding("pub use LocalOnly"),
        "a facade used only inside its own subtree must still be reported: {report:#?}"
    );
}
