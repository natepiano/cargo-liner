use std::collections::BTreeSet;

use crate::support::*;

fn write(root: &std::path::Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create fixture dir");
    }
    fs::write(path, contents).expect("write fixture file");
}

/// Every shape in one crate: the re-exports that leave their module's subtree
/// are flagged, and the facades, private imports, prelude, and extern-crate
/// re-exports beside them are not.
fn write_fixture(root: &std::path::Path) {
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

/// Only the re-exports that leave their module's subtree are flagged, and
/// each is an error.
#[test]
fn flags_only_reexports_that_leave_the_module_subtree() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_fixture(root);

    let report = run_mend_json(&root.join("Cargo.toml"));
    let flagged = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::PubUseOutsideSubtree)
        .map(|finding| {
            let relative = finding
                .path
                .split_once("src/")
                .map_or(finding.path.as_str(), |(_, relative)| relative)
                .to_string();
            (relative, finding.line_start)
        })
        .collect::<BTreeSet<_>>();

    let expected = BTreeSet::from([
        ("camera.rs".to_string(), 1),
        ("cascade/attributes.rs".to_string(), 1),
        ("cascade/attributes.rs".to_string(), 2),
        ("tool/palette/mod.rs".to_string(), 4),
        ("tool/tool_staging/mod.rs".to_string(), 3),
    ]);
    assert_eq!(flagged, expected, "full report: {:#?}", report.findings);
    assert_eq!(
        report.summary.errors,
        expected.len(),
        "each finding is an error so a gate stops on it"
    );
}
