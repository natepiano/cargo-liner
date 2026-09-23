use std::collections::BTreeSet;

use crate::support::*;

pub(super) fn write(root: &std::path::Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture dir");
    }
    fs::write(path, contents).expect("write fixture file");
}

pub(super) fn write_manifest(root: &std::path::Path) {
    pin_pub_in_path(root, PubInPath::Permitted);
    write(
        root,
        "Cargo.toml",
        r#"[package]
name = "pub_use_outside_subtree_fixture"
version = "0.1.0"
edition = "2024"
"#,
    );
}

/// The `pub_use_outside_subtree` findings of a report, keyed by their path
/// under `src/`.
fn subtree_findings(report: &Report) -> Vec<(String, &Finding)> {
    report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::PubUseOutsideSubtree)
        .map(|finding| {
            let relative = finding
                .path
                .split_once("src/")
                .map_or(finding.path.as_str(), |(_, relative)| relative)
                .to_string();
            (relative, finding)
        })
        .collect()
}

/// Runs mend on a fixture holding one re-export `--fix` cannot rewrite, and
/// asserts that one finding stays an error whose help names `reason`.
fn assert_single_unfixable_finding(root: &std::path::Path, path: &str, reason: &str) {
    let report = run_mend_json(&root.join("Cargo.toml"));
    let findings = subtree_findings(&report);
    let [(relative, finding)] = findings.as_slice() else {
        panic!("expected exactly one finding, full report: {report:#?}");
    };
    assert_eq!(relative, path, "full report: {report:#?}");
    assert_eq!(
        AdvertisedFix::from_notes(finding.help.iter().map(String::as_str)),
        AdvertisedFix::NotOffered,
        "an unfixable re-export must not advertise `--fix`: {report:#?}"
    );
    assert!(
        finding
            .help
            .iter()
            .any(|line| line.contains("no automatic fix") && line.contains(reason)),
        "the help must name why no automatic fix applies: {report:#?}"
    );
    assert!(
        report.summary.errors >= 1,
        "an unfixable finding stays an error: {report:#?}"
    );
    assert_summary_matches_findings(&report);
}

/// Every shape in one crate: the re-exports that leave their module's subtree
/// are flagged, and the facades, private imports, prelude, and extern-crate
/// re-exports beside them are not.
fn write_fixture(root: &std::path::Path) {
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod cascade;\nmod platform;\npub mod prelude;\nmod tool;\n\npub fn \
         touch() -> i32 {\n    tool::run() + camera::run() + cascade::run()\n}\n",
    );
    // The crate prelude re-exports its parent's items by design.
    write(root, "src/prelude.rs", "pub use super::touch;\n");

    // A parent facade re-exporting its own child is the sanctioned shape, even
    // when the child's item is itself a (flagged) sideways re-export.
    write(
        root,
        "src/tool/mod.rs",
        "mod palette;\nmod reset_scene;\nmod tool_staging;\n\npub(crate) use \
         tool_staging::build_device_recipe;\npub(crate) use tool_staging::stage_all;\n\npub(crate) \
         fn run() -> i32 {\n    build_device_recipe() + stage_all() + palette::run()\n}\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> i32 {\n    1\n}\n",
    );
    // Flagged: a sibling's function handed out under `tool_staging`.
    write(
        root,
        "src/tool/tool_staging/mod.rs",
        "mod stage;\n\npub(crate) use super::reset_scene::build_device_recipe;\npub(crate) use \
         stage::stage_all;\n",
    );
    write(
        root,
        "src/tool/tool_staging/stage.rs",
        "pub(crate) fn stage_all() -> i32 {\n    2\n}\n",
    );
    // Flagged: an inline module re-exporting from a sibling of its own.
    write(
        root,
        "src/tool/palette/mod.rs",
        "mod panel;\n\nmod panel_access {\n    pub(super) use super::panel::row_id;\n}\n\npub(super) \
         fn run() -> i32 {\n    panel_access::row_id()\n}\n",
    );
    write(
        root,
        "src/tool/palette/panel.rs",
        "pub(super) fn row_id() -> i32 {\n    3\n}\n",
    );

    // Flagged: a leaf reaching across the crate with `crate::`. The extern
    // crate re-export below it leaves no local subtree.
    write(
        root,
        "src/camera.rs",
        "pub(crate) use crate::platform::discover;\npub(crate) use std::time::Duration;\n\npub(crate) \
         fn run() -> i32 {\n    discover() + i32::from(Duration::ZERO.is_zero())\n}\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    write(
        root,
        "src/cascade/mod.rs",
        "mod attributes;\nmod resolved;\n\npub(crate) use attributes::FontUnit;\n\npub(crate) struct \
         Shared;\n\npub(crate) fn run() -> i32 {\n    let _ = attributes::Shared;\n    \
         FontUnit::default().0 + attributes::alpha()\n}\n",
    );
    // Flagged: a sibling's type and the parent's type. The private import on
    // the third line re-exports nothing.
    write(
        root,
        "src/cascade/attributes.rs",
        "pub(crate) use super::Shared;\npub(crate) use super::resolved::FontUnit;\nuse \
         super::resolved::TextAlpha;\n\npub(crate) fn alpha() -> i32 {\n    TextAlpha(5).0\n}\n",
    );
    write(
        root,
        "src/cascade/resolved.rs",
        "#[derive(Default)]\npub(crate) struct FontUnit(pub(crate) i32);\n\npub(crate) struct \
         TextAlpha(pub(crate) i32);\n",
    );
}

/// Only the re-exports that leave their module's subtree are flagged. Each
/// one here has a rewrite `--fix` can apply, so each advertises the `--fix`
/// note and reports as a warning.
#[test]
fn flags_only_reexports_that_leave_the_module_subtree() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_fixture(root);

    let report = run_mend_json(&root.join("Cargo.toml"));
    let findings = subtree_findings(&report);
    let flagged = findings
        .iter()
        .map(|(relative, finding)| (relative.clone(), finding.line_start))
        .collect::<BTreeSet<_>>();

    let expected = BTreeSet::from([
        ("camera.rs".to_string(), 1),
        ("cascade/attributes.rs".to_string(), 1),
        ("cascade/attributes.rs".to_string(), 2),
        ("tool/palette/mod.rs".to_string(), 4),
        ("tool/tool_staging/mod.rs".to_string(), 3),
    ]);
    assert_eq!(flagged, expected, "full report: {:#?}", report.findings);
    for (relative, finding) in &findings {
        assert_eq!(
            AdvertisedFix::from_notes(finding.help.iter().map(String::as_str)),
            AdvertisedFix::WithFix,
            "`{relative}:{}` must advertise `--fix`: {report:#?}",
            finding.line_start
        );
    }
    assert_eq!(
        report.summary.errors, 0,
        "a finding `--fix` can rewrite is a warning: {report:#?}"
    );
    assert_summary_matches_findings(&report);
}

/// A re-export that renames its item with `as` stays an error without a fix:
/// callers name the new name, which exists only at the re-export.
#[test]
fn renaming_reexport_stays_an_error_without_a_fix() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "pub(crate) use crate::platform::discover as find_device;\n\npub(crate) fn run() -> i32 \
         {\n    find_device()\n}\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    assert_single_unfixable_finding(root, "camera.rs", "renames the item with `as`");
}

/// A glob re-export names no single item to move, so it stays an error
/// without a fix.
#[test]
fn glob_reexport_stays_an_error_without_a_fix() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "pub(crate) use crate::platform::*;\n\npub(crate) fn run() -> i32 {\n    discover()\n}\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    assert_single_unfixable_finding(root, "camera.rs", "is a glob");
}

/// A re-export in a module reachable from outside the crate through a `pub
/// mod` chain is a public path; removing it would break downstream callers, so
/// it stays an error without a fix.
#[test]
fn reexport_in_public_module_stays_an_error_without_a_fix() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(root, "src/lib.rs", "pub mod device;\nmod platform;\n");
    write(root, "src/device/mod.rs", "pub mod camera;\n");
    write(
        root,
        "src/device/camera.rs",
        "pub use crate::platform::discover;\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub fn discover() -> i32 {\n    4\n}\n",
    );

    assert_single_unfixable_finding(
        root,
        "device/camera.rs",
        "is reachable from outside the crate",
    );
}

/// A `use` a `macro_rules!` expansion writes has no source line of its own to
/// rewrite, so it is not reported; the same re-export written by hand beside
/// it still is.
#[test]
fn reexport_written_by_a_macro_is_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\nmod tool;\n\npub fn touch() -> i32 {\n    camera::run() + \
         tool::run()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "macro_rules! reexport_discover {\n    () => {\n        pub(crate) use \
         crate::platform::discover;\n    };\n}\n\nreexport_discover!();\n\npub(crate) fn run() \
         -> i32 {\n    discover()\n}\n",
    );
    write(
        root,
        "src/tool.rs",
        "pub(crate) use crate::platform::discover;\n\npub(crate) fn run() -> i32 {\n    \
         discover()\n}\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    let report = run_mend_json(&root.join("Cargo.toml"));
    let flagged = subtree_findings(&report)
        .into_iter()
        .map(|(relative, finding)| (relative, finding.line_start))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        flagged,
        BTreeSet::from([("tool.rs".to_string(), 1)]),
        "only the hand-written re-export is reported: {report:#?}"
    );
}
