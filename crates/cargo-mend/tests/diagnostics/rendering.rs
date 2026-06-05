use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;

use crate::support::*;

#[test]
fn every_diagnostic_has_a_unique_readme_anchor() {
    let readme = include_str!("../../README.md");
    let mut seen_anchors = BTreeSet::new();

    for &code in DiagnosticCode::ALL {
        let spec = diagnostic_spec(code);
        assert!(
            seen_anchors.insert(spec.help_anchor),
            "duplicate README anchor: {}",
            spec.help_anchor
        );
        let anchor = format!(r#"<a id="{}"></a>"#, spec.help_anchor);
        assert!(
            readme.contains(&anchor),
            "README is missing anchor for {:?}: {}",
            code,
            spec.help_anchor
        );
    }
}

fn create_all_diagnostics_fixture() -> TempDir {
    let temp = tempdir().expect("create temp fixture dir");
    for dir in [
        "src/private_parent",
        "src/stale_parent",
        "src/wild_parent",
        "src/type_parent",
        "src/func_parent",
        "src/internal_parent",
        "src/deep_parent/nested",
        "src/field_visibility_parent",
        "src/in_body_use",
        "src/unused_pub",
    ] {
        fs::create_dir_all(temp.path().join(dir)).expect("create fixture dir");
    }

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
mod func_parent;
mod type_parent;
mod deep_parent;
mod narrow_mod;
mod field_visibility_parent;
mod in_body_use;
mod unused_pub;
pub mod review_mod;
pub use private_parent::PublicContainer;

fn main() {
    narrow_mod::unexported_top_level();
}
"#,
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/review_mod.rs"), "\n").expect("write review mod");
    fs::write(
        temp.path().join("src/narrow_mod.rs"),
        "pub fn unexported_top_level() {}\n",
    )
    .expect("write narrow mod");
    fs::write(
        temp.path().join("src/unused_pub.rs"),
        "pub fn local_only() {}\n",
    )
    .expect("write unused_pub mod");
    write_diagnostic_fixture_modules(temp.path());

    temp
}

fn write_diagnostic_fixture_modules(root: &Path) {
    fs::write(
        root.join("src/type_parent/mod.rs"),
        "mod types;\nmod consumer;\n",
    )
    .expect("write type_parent mod");
    fs::write(
        root.join("src/type_parent/types.rs"),
        "pub struct MyWidget;\n",
    )
    .expect("write type_parent types");
    fs::write(
        root.join("src/type_parent/consumer.rs"),
        "fn example(_w: crate::type_parent::types::MyWidget) {}\n",
    )
    .expect("write type_parent consumer");
    fs::write(
        root.join("src/func_parent/mod.rs"),
        "mod utils;\nmod consumer;\n",
    )
    .expect("write func_parent mod");
    fs::write(
        root.join("src/func_parent/utils.rs"),
        "pub fn do_thing() -> i32 { 42 }\n",
    )
    .expect("write func_parent utils");
    fs::write(
        root.join("src/func_parent/consumer.rs"),
        "use crate::func_parent::utils::do_thing;\n\nfn example() -> i32 { do_thing() }\n",
    )
    .expect("write func_parent consumer");
    fs::write(
        root.join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private parent");
    fs::write(
        root.join("src/private_parent/child.rs"),
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
    fs::write(
        root.join("src/internal_parent.rs"),
        "mod child;\nmod sibling;\npub use child::InternalFacade;\n",
    )
    .expect("write internal parent");
    fs::write(
        root.join("src/internal_parent/child.rs"),
        "pub struct InternalFacade;\n",
    )
    .expect("write internal child");
    fs::write(
        root.join("src/internal_parent/sibling.rs"),
        "use super::InternalFacade;\n\nfn use_parent_facade(_value: InternalFacade) {}\n",
    )
    .expect("write internal sibling");
    fs::write(
        root.join("src/stale_parent/mod.rs"),
        "mod child;\npub use child::StaleExport;\n",
    )
    .expect("write stale parent");
    fs::write(
        root.join("src/stale_parent/child.rs"),
        "pub struct StaleExport;\n",
    )
    .expect("write stale child");
    fs::write(
        root.join("src/wild_parent/mod.rs"),
        "mod child;\npub use child::*;\n",
    )
    .expect("write wildcard parent");
    fs::write(
        root.join("src/wild_parent/child.rs"),
        "pub struct WildExport;\n",
    )
    .expect("write wildcard child");
    fs::write(
        root.join("src/deep_parent/mod.rs"),
        "mod nested;\npub struct DeepTarget;\n",
    )
    .expect("write deep parent mod");
    fs::write(root.join("src/deep_parent/nested/mod.rs"), "mod leaf;\n")
        .expect("write deep nested mod");
    fs::write(
        root.join("src/deep_parent/nested/leaf.rs"),
        "use super::super::DeepTarget;\n\nfn use_it(_target: DeepTarget) {}\n",
    )
    .expect("write deep leaf");
    write_field_visibility_fixture(root);
    write_in_body_use_fixture(root);
}

fn write_in_body_use_fixture(root: &Path) {
    fs::write(root.join("src/in_body_use/mod.rs"), "mod consumer;\n")
        .expect("write in_body_use mod");
    fs::write(
        root.join("src/in_body_use/consumer.rs"),
        "fn example() {\n    use std::collections::HashMap;\n    let _map: HashMap<u8, u8> = HashMap::new();\n}\n",
    )
    .expect("write in_body_use consumer");
}

fn write_field_visibility_fixture(root: &Path) {
    fs::write(root.join("src/field_visibility_parent.rs"), "mod hidden;\n")
        .expect("write field-vis parent");
    fs::write(
        root.join("src/field_visibility_parent/hidden.rs"),
        "struct Hidden {\n    pub leaked: u32,\n}\n",
    )
    .expect("write field-vis fixture");
}

fn assert_rendered_diagnostics(report: &Report, rendered: &str) {
    for &code in DiagnosticCode::ALL {
        let spec = diagnostic_spec(code);
        assert!(
            rendered.contains(spec.headline),
            "rendered output is missing headline for {code:?}",
        );
        let help_url = format!(
            "https://github.com/natepiano/cargo-mend#{}",
            spec.help_anchor
        );
        assert!(
            rendered.contains(&help_url),
            "rendered output is missing help URL for {code:?}",
        );
    }

    assert!(rendered.contains("help: consider using just `pub` or removing `pub(crate)` entirely"));
    assert!(rendered.contains("help: consider using: `pub(crate)`"));
    assert!(rendered.contains("help: consider using: `pub(super)`"));
    assert!(
        rendered.contains("help: consider using: `use super::PublicContainer as ParentContainer;`")
    );
    assert!(rendered.contains(
        "help: consider removing this parent facade and importing the item from its defining child module"
    ));
    for finding in &report.findings {
        if let Some(note) = fix_support_for(finding.code, finding.fix_support).note() {
            assert!(
                rendered.contains(note),
                "rendered output is missing fix note for {:?}",
                finding.code
            );
        }
    }
    assert!(rendered.contains(&expected_summary_text(report)));
    assert!(rendered.contains(
        "parent module also has an `unused import` warning for this `pub use` at stale_parent/mod.rs"
    ));
    assert!(rendered.contains("help: consider re-exporting explicit items instead of `*`"));
}

#[test]
fn fixture_renders_every_current_diagnostic() {
    let temp = create_all_diagnostics_fixture();

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

    let report = parse_mend_json_output(&output.stdout);
    let stdout = String::from_utf8(output.stdout).expect("decode mend JSON output");
    let last_message: Value = serde_json::from_str(stdout.lines().last().expect("last JSON line"))
        .expect("parse build-finished JSON message");
    assert_eq!(
        last_message.get("reason").and_then(Value::as_str),
        Some("build-finished")
    );
    assert_eq!(
        last_message.get("success").and_then(Value::as_bool),
        Some(false)
    );
    let codes: BTreeSet<_> = report.findings.iter().map(|finding| finding.code).collect();
    let expected_codes: BTreeSet<_> = DiagnosticCode::ALL.iter().copied().collect();

    assert_eq!(
        codes, expected_codes,
        "fixture should trigger every diagnostic at least once"
    );
    assert_eq!(report.findings.len(), 16);
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

    assert_rendered_diagnostics(&report, &rendered);
}

#[test]
fn cached_findings_reused_across_different_target_selections() {
    // Regression: previously, the on-disk findings cache rejected reuse
    // whenever the cargo CLI flags differed between runs (e.g.
    // `--lib` vs `--all-targets`). When cargo's own fingerprint
    // correctly skipped recompiling the lib, the wrapper never re-ran,
    // and the lib-only findings cache was discarded — making
    // `--all-targets` look like a strict subset of `--lib`. Cache
    // reuse must depend only on the underlying source (which cargo
    // already tracks) plus the diagnostic config — not on the user's
    // target-selection flags.
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "selection_cache_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/inner")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "pub mod inner;\n").expect("write lib");
    fs::write(temp.path().join("src/inner/mod.rs"), "pub mod leaf;\n").expect("write inner mod");
    fs::write(
        temp.path().join("src/inner/leaf.rs"),
        "pub(in crate::inner) fn cross_module_helper() -> i32 { 42 }\n",
    )
    .expect("write leaf");

    let manifest = temp.path().join("Cargo.toml");

    let baseline = run_mend_json(&manifest);
    let baseline_count = baseline
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubInCrate)
        .count();
    assert!(
        baseline_count > 0,
        "baseline run should report the forbidden_pub_in_crate finding"
    );

    // After the lib was just compiled, an `--all-targets` run must
    // surface the same finding even though cargo will skip recompiling
    // the lib (its rmeta is fresh from the baseline run).
    let all_targets_output = mend_command()
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--all-targets")
        .arg("--json")
        .output()
        .expect("run cargo-mend --all-targets --json");
    let all_targets: Report = parse_mend_json_output(&all_targets_output.stdout);
    let all_targets_count = all_targets
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubInCrate)
        .count();
    assert_eq!(
        all_targets_count,
        baseline_count,
        "--all-targets after lib-only run should reuse the cached lib finding (cargo will skip \
         recompiling the lib, so the wrapper does not re-emit); got codes: {:?}",
        all_targets
            .findings
            .iter()
            .map(|f| f.code)
            .collect::<Vec<_>>()
    );

    // And going back to the default selection should still see it
    // (no recompile happens — pure cache replay).
    let third = run_mend_json(&manifest);
    let third_count = third
        .findings
        .iter()
        .filter(|f| f.code == DiagnosticCode::ForbiddenPubInCrate)
        .count();
    assert_eq!(
        third_count, baseline_count,
        "default selection should still see the cached finding after the --all-targets run"
    );
}

#[test]
fn successive_json_runs_reuse_cached_findings_for_same_scope() {
    let temp = create_all_diagnostics_fixture();

    let first = run_mend_json(&temp.path().join("Cargo.toml"));
    let second = run_mend_json(&temp.path().join("Cargo.toml"));

    let first_codes: BTreeSet<_> = first.findings.iter().map(|finding| finding.code).collect();
    let second_codes: BTreeSet<_> = second.findings.iter().map(|finding| finding.code).collect();

    assert_eq!(first.findings.len(), 16);
    assert_eq!(second.findings.len(), first.findings.len());
    assert_eq!(second_codes, first_codes);
    assert_eq!(second.summary.errors, first.summary.errors);
    assert_eq!(second.summary.warnings, first.summary.warnings);
    assert_eq!(
        second.summary.fixable_with_fix,
        first.summary.fixable_with_fix
    );
    assert_eq!(
        second.summary.fixable_with_fix_pub_use,
        first.summary.fixable_with_fix_pub_use
    );
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
            finding.code == DiagnosticCode::ReviewPubMod
                && finding.path == "src/private_tools/mod.rs"
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
            finding.code == DiagnosticCode::ReviewPubMod
                && finding.path == "mcp/src/private_tools/mod.rs"
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
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "app/src/tool/field_placement.rs"
        }),
        "literal workspace sibling crate paths should preserve the parent facade: {:#?}",
        report.findings
    );
    assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
}
