use crate::common::*;

#[test]
fn every_diagnostic_has_a_unique_readme_anchor() {
    let readme = include_str!("../../README.md");
    let mut seen_codes = BTreeSet::new();
    let mut seen_anchors = BTreeSet::new();

    for spec in diagnostic_specs() {
        assert!(
            seen_codes.insert(spec.code),
            "duplicate diagnostic code: {}",
            spec.code
        );
        assert!(
            seen_anchors.insert(spec.help_anchor),
            "duplicate README anchor: {}",
            spec.help_anchor
        );
        let anchor = format!(r#"<a id="{}"></a>"#, spec.help_anchor);
        assert!(
            readme.contains(&anchor),
            "README is missing anchor for {}: {}",
            spec.code,
            spec.help_anchor
        );
    }
}

#[test]
fn fixture_renders_every_current_diagnostic() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");
    fs::create_dir_all(temp.path().join("src/stale_parent"))
        .expect("create stale nested fixture dir");
    fs::create_dir_all(temp.path().join("src/wild_parent"))
        .expect("create wildcard nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"pub(crate) fn crate_only() {}
mod private_parent;
mod internal_parent;
mod stale_parent;
mod wild_parent;
pub mod review_mod;
pub use private_parent::PublicContainer;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/review_mod.rs"), "\n").expect("write review mod");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        r#"use crate::private_parent::PublicContainer as ParentContainer;

pub enum LegitDependency {
    Unit,
}

pub(in crate::private_parent) fn subtree_only() {}

pub struct PublicContainer {
    pub dependency: LegitDependency,
}

pub struct Suspicious;
"#,
    )
    .expect("write suspicious child");
    fs::create_dir_all(temp.path().join("src/internal_parent")).expect("create internal parent");
    fs::write(
        temp.path().join("src/internal_parent.rs"),
        "mod child;\nmod sibling;\npub use child::InternalFacade;\n",
    )
    .expect("write internal parent");
    fs::write(
        temp.path().join("src/internal_parent/child.rs"),
        "pub struct InternalFacade;\n",
    )
    .expect("write internal child");
    fs::write(
        temp.path().join("src/internal_parent/sibling.rs"),
        "use super::InternalFacade;\n\nfn use_parent_facade(_value: InternalFacade) {}\n",
    )
    .expect("write internal sibling");
    fs::write(
        temp.path().join("src/stale_parent/mod.rs"),
        "mod child;\npub use child::StaleExport;\n",
    )
    .expect("write stale parent");
    fs::write(
        temp.path().join("src/stale_parent/child.rs"),
        "pub struct StaleExport;\n",
    )
    .expect("write stale child");
    fs::write(
        temp.path().join("src/wild_parent/mod.rs"),
        "mod child;\npub use child::*;\n",
    )
    .expect("write wildcard parent");
    fs::write(
        temp.path().join("src/wild_parent/child.rs"),
        "pub struct WildExport;\n",
    )
    .expect("write wildcard child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run cargo-mend against fixture");
    assert!(
        matches!(output.status.code(), Some(1 | 2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let report: Report = serde_json::from_slice(&output.stdout).expect("parse mend json report");
    let codes: BTreeSet<_> = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect();
    let expected_codes: BTreeSet<_> = diagnostic_specs().iter().map(|spec| spec.code).collect();

    assert_eq!(
        codes, expected_codes,
        "fixture should trigger every diagnostic at least once"
    );
    assert_eq!(report.findings.len(), 9);
    assert_summary_matches_findings(&report);

    let rendered_output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run cargo-mend human output");
    assert!(
        matches!(rendered_output.status.code(), Some(1 | 2)),
        "cargo-mend returned unexpected status {:?}: {}",
        rendered_output.status.code(),
        String::from_utf8_lossy(&rendered_output.stderr)
    );
    let rendered =
        strip_ansi(&String::from_utf8(rendered_output.stdout).expect("decode human output"));

    for spec in diagnostic_specs() {
        assert!(
            rendered.contains(spec.headline),
            "rendered output is missing headline for {}",
            spec.code
        );
        let help_url = format!(
            "https://github.com/natepiano/cargo-mend#{}",
            spec.help_anchor
        );
        assert!(
            rendered.contains(&help_url),
            "rendered output is missing help URL for {}",
            spec.code
        );
    }

    assert!(rendered.contains("help: consider using just `pub` or removing `pub(crate)` entirely"));
    assert!(rendered.contains("help: consider using: `pub(super)`"));
    assert!(
        rendered.contains("help: consider using: `use super::PublicContainer as ParentContainer;`")
    );
    assert!(rendered.contains(
        "help: consider removing this parent facade and importing the item from its defining child module"
    ));
    for finding in &report.findings {
        if let Some(note) = fix_support_for(&finding.code, finding.fix_support).note() {
            assert!(
                rendered.contains(note),
                "rendered output is missing fix note for {}",
                finding.code
            );
        }
    }
    assert!(rendered.contains(&expected_summary_text(&report)));
    assert!(rendered.contains(
        "parent module also has an `unused import` warning for this `pub use` at stale_parent/mod.rs"
    ));
    assert!(rendered.contains("help: consider re-exporting explicit items instead of `*`"));
}

#[test]
fn project_root_allow_pub_mod_suppresses_local_review_pub_mod() {
    let temp = tempdir().expect("create temp project dir");
    fs::create_dir_all(temp.path().join("src/private_tools")).expect("create project dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("mend.toml"),
        r#"[visibility]
allow_pub_mod = ["src/private_tools/mod.rs"]
"#,
    )
    .expect("write local mend config");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_tools;

fn main() {}
"#,
    )
    .expect("write main");
    fs::write(
        temp.path().join("src/private_tools/mod.rs"),
        "pub mod helper;\n",
    )
    .expect("write allowlisted mod");
    fs::write(
        temp.path().join("src/private_tools/helper.rs"),
        "pub fn run() {}\n",
    )
    .expect("write helper");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == "review_pub_mod" && finding.path == "src/private_tools/mod.rs"
        }),
        "project-root allow_pub_mod should suppress local pub mod review"
    );
}

#[test]
fn workspace_root_allow_pub_mod_suppresses_member_review_pub_mod() {
    let temp = tempdir().expect("create temp workspace dir");
    fs::create_dir_all(temp.path().join("mcp/src/private_tools")).expect("create member dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["mcp"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("mend.toml"),
        r#"[visibility]
allow_pub_mod = ["mcp/src/private_tools/mod.rs"]
"#,
    )
    .expect("write workspace mend config");
    fs::write(
        temp.path().join("mcp/Cargo.toml"),
        r#"[package]
name = "member_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write member manifest");
    fs::write(
        temp.path().join("mcp/src/main.rs"),
        r#"mod private_tools;

fn main() {}
"#,
    )
    .expect("write member main");
    fs::write(
        temp.path().join("mcp/src/private_tools/mod.rs"),
        "pub mod helper;\n",
    )
    .expect("write allowlisted member mod");
    fs::write(
        temp.path().join("mcp/src/private_tools/helper.rs"),
        "pub fn run() {}\n",
    )
    .expect("write member helper");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == "review_pub_mod" && finding.path == "mcp/src/private_tools/mod.rs"
        }),
        "workspace-root allow_pub_mod should suppress member pub mod review"
    );
}

#[test]
fn workspace_sibling_literal_crate_paths_preserve_parent_pub_use_facade() {
    let temp = tempdir().expect("create temp workspace dir");
    fs::create_dir_all(temp.path().join("app/src/tool")).expect("create app dirs");
    fs::create_dir_all(temp.path().join("macros/src")).expect("create macros dirs");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["app", "macros"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("app/Cargo.toml"),
        r#"[package]
name = "app_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write app manifest");
    fs::write(
        temp.path().join("app/src/main.rs"),
        r#"mod tool;

fn main() {}
"#,
    )
    .expect("write app main");
    fs::write(
        temp.path().join("app/src/tool.rs"),
        r#"mod field_placement;

pub use field_placement::{FieldPlacementInfo, HasFieldPlacement};
"#,
    )
    .expect("write tool facade");
    fs::write(
        temp.path().join("app/src/tool/field_placement.rs"),
        r#"pub struct FieldPlacementInfo;

pub trait HasFieldPlacement {}
"#,
    )
    .expect("write tool child");
    fs::write(
        temp.path().join("macros/Cargo.toml"),
        r#"[package]
name = "macros_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write macros manifest");
    fs::write(
        temp.path().join("macros/src/lib.rs"),
        r#"const _: &str = stringify!(
    crate::tool::HasFieldPlacement
    crate::tool::FieldPlacementInfo
);
"#,
    )
    .expect("write macros lib");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == "suspicious_pub" && finding.path == "app/src/tool/field_placement.rs"
        }),
        "literal workspace sibling crate paths should preserve the parent facade: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}
