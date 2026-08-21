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
        "src/bin",
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
        temp.path().join("mend.toml"),
        "[visibility]\npub_in_path = \"permitted\"\n",
    )
    .expect("write fixture visibility config");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod private_parent;
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

pub fn entry() {
    narrow_mod::unexported_top_level();
}
"#,
    )
    .expect("write fixture library");
    fs::write(
        temp.path().join("src/bin/policy.rs"),
        "pub(crate) fn crate_only() {}\nfn main() {}\n",
    )
    .expect("write fixture binary");
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

struct Suspicious;
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
        "mod nested;\npub(crate) struct DeepTarget;\n",
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
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .unwrap_or_else(|| panic!("fixture is missing finding for {code:?}"));
        let headline = spec.headline.resolve(&finding.headline);
        assert!(
            rendered.contains(headline),
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

fn assert_forbidden_visibility_json(output: &str, report: &Report) {
    // `forbidden_pub_in_crate` is no longer uniformly an error. Cargo's level
    // follows `reporting::diagnostics::effective_severity`, which reports
    // anything mend can fix itself as a warning, and the fixture's
    // `pub(in crate::private_parent) fn subtree_only` has no caller outside its
    // own module — so `--fix` removes the annotation and the finding renders as
    // a fixable warning. The cases mend cannot repair on its own still render
    // as errors; `bounded_pub_in_path_is_fixable_only_for_declarations` covers
    // both halves against one fixture.
    let expected_diagnostics = [
        ("overbroad_pub_crate", "warning"),
        ("forbidden_pub_in_crate", "warning"),
    ];

    for (code, expected_level) in expected_diagnostics {
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code.as_str() == code)
            .unwrap_or_else(|| panic!("fixture is missing finding for {code}"));
        let headline = finding.headline.as_str();
        let diagnostic = output
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("parse cargo JSON line"))
            .find(|message| {
                message
                    .pointer("/message/code/code")
                    .and_then(Value::as_str)
                    == Some(code)
            })
            .expect("find forbidden visibility cargo diagnostic");
        let message = diagnostic.get("message").expect("cargo diagnostic message");
        assert_eq!(
            message.get("level").and_then(Value::as_str),
            Some(expected_level),
            "cargo JSON severity for {code}",
        );
        assert_eq!(
            message.get("message").and_then(Value::as_str),
            Some(headline),
            "cargo JSON headline for {code}"
        );
        let rendered = message
            .get("rendered")
            .and_then(Value::as_str)
            .expect("cargo JSON rendered diagnostic");
        assert_eq!(
            rendered.matches(headline).count(),
            1,
            "cargo JSON rendered diagnostic should not repeat its headline for {code}"
        );
        let children = message
            .get("children")
            .and_then(Value::as_array)
            .expect("cargo JSON diagnostic children");
        assert!(
            !children.iter().any(|child| {
                child.get("level").and_then(Value::as_str) == Some("note")
                    && child.get("message").and_then(Value::as_str) == Some(headline)
            }),
            "cargo JSON should not emit a headline note for {code}"
        );
        for help in &finding.help {
            assert!(
                children.iter().any(|child| {
                    child.get("message").and_then(Value::as_str) == Some(help.as_str())
                }),
                "cargo JSON is missing child message for {code}: {help}"
            );
        }
    }
}

fn assert_forbidden_visibility_human(rendered: &str, report: &Report) {
    for (code, level) in [
        (DiagnosticCode::OverbroadPubCrate, "warning"),
        // Fixable now — see the note in `assert_forbidden_visibility_json`.
        (DiagnosticCode::ForbiddenPubInCrate, "warning"),
    ] {
        let headline = report
            .findings
            .iter()
            .find(|finding| finding.code == code)
            .map_or_else(
                || panic!("fixture is missing finding for {code:?}"),
                |finding| finding.headline.as_str(),
            );
        assert!(
            rendered.contains(&format!("{level}: {headline}")),
            "human output should use the finding message as the headline"
        );
        assert!(
            !rendered.contains(&format!("note: {headline}")),
            "human output should not repeat the headline as a note"
        );
    }
    let pub_crate_help = report
        .findings
        .iter()
        .find(|finding| finding.code == DiagnosticCode::OverbroadPubCrate)
        .and_then(|finding| {
            finding
                .help
                .iter()
                .find(|line| line.starts_with("consider "))
        })
        .expect("forbidden pub(crate) fixture help");
    assert!(
        rendered.contains(&format!("help: {pub_crate_help}")),
        "human output is missing forbidden pub(crate) help"
    );
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
    assert_eq!(report.findings.len(), 17);
    assert_summary_matches_findings(&report);
    assert_forbidden_visibility_json(&stdout, &report);

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
    assert_forbidden_visibility_human(&rendered, &report);
}

#[test]
fn suspicious_pub_dynamic_help_matches_all_renderers() {
    let temp = create_stale_restricted_facade_fixture();
    let expected_help =
        "remove the parent facade and the now-unneeded `pub(in crate::a)` annotation";

    let json_output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--json")
        .output()
        .expect("run JSON suspicious-pub fixture");
    let stdout = String::from_utf8(json_output.stdout).expect("decode JSON output");
    let diagnostic = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("parse cargo JSON line"))
        .find(|message| {
            message
                .pointer("/message/code/code")
                .and_then(Value::as_str)
                == Some("suspicious_pub")
        })
        .expect("find suspicious-pub cargo diagnostic");
    let message = diagnostic.get("message").expect("cargo diagnostic message");
    let help_children = message
        .get("children")
        .and_then(Value::as_array)
        .expect("cargo diagnostic children")
        .iter()
        .filter(|child| child.get("level").and_then(Value::as_str) == Some("help"))
        .collect::<Vec<_>>();
    assert_eq!(
        help_children.len(),
        1,
        "unexpected help children: {message:#?}"
    );
    assert_eq!(
        help_children[0].get("message").and_then(Value::as_str),
        Some(expected_help),
    );
    let cargo_rendered = message
        .get("rendered")
        .and_then(Value::as_str)
        .expect("cargo rendered diagnostic");

    let human_output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("run human suspicious-pub fixture");
    let human_rendered =
        strip_ansi(&String::from_utf8(human_output.stdout).expect("decode human output"));
    assert_eq!(
        rendered_help_text(cargo_rendered, expected_help),
        rendered_help_text(&human_rendered, expected_help),
    );
    assert_eq!(
        rendered_help_text(cargo_rendered, expected_help),
        expected_help
    );
    assert!(!human_rendered.contains("help: consider using: `pub(super)`"));
}

fn create_stale_restricted_facade_fixture() -> TempDir {
    let temp = tempdir().expect("create stale restricted facade fixture dir");
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "stale_restricted_facade_fixture"
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
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod c;\npub(super) use c::Thing;\n",
    )
    .expect("write stale facade");
    fs::write(
        temp.path().join("src/a/b/c.rs"),
        "pub(in crate::a) struct Thing;\n",
    )
    .expect("write accepted restricted declaration");
    temp
}

fn rendered_help_text<'a>(rendered: &'a str, expected_help: &str) -> &'a str {
    rendered
        .lines()
        .find_map(|line| {
            line.split_once("help: ")
                .map(|(_, help)| help)
                .filter(|help| *help == expected_help)
        })
        .unwrap_or_else(|| panic!("missing help {expected_help:?} in:\n{rendered}"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    assert_eq!(first.findings.len(), 17);
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
pub_in_path = "permitted"
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
pub_in_path = "permitted"
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
fn workspace_sibling_literal_crate_paths_do_not_count_as_facade_usage() {
    let temp = tempdir().expect("create temp workspace dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
        report.findings.iter().all(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "app/src/tool/field_placement.rs"
        }),
        "workspace sibling crate literals must not count as app facade usage: {:#?}",
        report.findings
    );
    assert_eq!(report.findings.len(), 2, "{:#?}", report.findings);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 2);
}

#[test]
fn workspace_descendant_macro_literal_counts_as_inside_facade_usage() {
    let temp = tempdir().expect("create descendant literal workspace dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("app/src/a")).expect("create app fixture modules");
    fs::create_dir_all(temp.path().join("support/src")).expect("create support fixture module");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["app", "support"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("app/Cargo.toml"),
        r#"[package]
name = "descendant_literal_app_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write app manifest");
    fs::write(
        temp.path().join("app/src/main.rs"),
        "mod a;\nfn main() {}\n",
    )
    .expect("write app main");
    fs::write(
        temp.path().join("app/src/a.rs"),
        "mod child;\nmod descendant;\npub use child::Thing;\n",
    )
    .expect("write app facade");
    fs::write(
        temp.path().join("app/src/a/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write facade subject");
    fs::write(
        temp.path().join("app/src/a/descendant.rs"),
        "const _: &str = stringify!(crate::a::Thing);\n",
    )
    .expect("write descendant macro literal");
    fs::write(
        temp.path().join("support/Cargo.toml"),
        r#"[package]
name = "descendant_literal_support_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write support manifest");
    fs::write(temp.path().join("support/src/lib.rs"), "").expect("write support library root");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "app/src/a/child.rs"
        })
        .unwrap_or_else(|| panic!("missing inside-subtree facade finding: {report:#?}"));
    assert!(
        finding.help.iter().any(|help| {
            help.contains("only used through crate-relative paths inside its own subtree")
        }),
        "the descendant macro literal must remain inside the facade subtree: {report:#?}"
    );
}

#[test]
fn standalone_descendant_macro_literal_counts_as_inside_facade_usage() {
    let temp = tempdir().expect("create standalone descendant literal dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create standalone fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "standalone_descendant_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write standalone fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write standalone fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\nmod descendant;\npub use child::Thing;\n",
    )
    .expect("write standalone facade");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write standalone facade subject");
    fs::write(
        temp.path().join("src/a/descendant.rs"),
        "const _: &str = stringify!(crate::a::Thing);\n",
    )
    .expect("write standalone descendant macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/a/child.rs"
        })
        .unwrap_or_else(|| panic!("missing standalone inside-subtree facade finding: {report:#?}"));
    assert!(
        finding.help.iter().any(|help| {
            help.contains("only used through crate-relative paths inside its own subtree")
        }),
        "the standalone descendant literal must remain inside the facade subtree: {report:#?}"
    );
}

#[test]
fn inactive_nested_module_literal_counts_as_inside_facade_usage() {
    let temp = tempdir().expect("create inactive nested literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inactive_nested_literal_fixture"
version = "0.1.0"
edition = "2024"

[features]
hidden = []
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\n#[cfg(feature = \"hidden\")]\nmod hidden;\npub use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/hidden.rs"),
        "const _: &str = stringify!(crate::a::Thing);\n",
    )
    .expect("write inactive nested module");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/a/child.rs"
        })
        .unwrap_or_else(|| panic!("missing inactive-subtree facade finding: {report:#?}"));
    assert!(
        finding.help.iter().any(|help| {
            help.contains("only used through crate-relative paths inside its own subtree")
        }),
        "the inactive nested literal must be attributed to module a: {report:#?}"
    );
}

#[test]
fn out_of_source_root_macro_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create out-of-root literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "out_of_root_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\n#[path = \"../shared.rs\"]\nmod shared;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\npub(super) use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("shared.rs"),
        "macro_rules! mention { () => { stringify!(crate::a::b::Thing) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write out-of-root macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "the current-crate macro literal outside src must count as facade usage: {report:#?}"
    );
}

#[test]
fn out_of_source_root_parsed_path_counts_as_facade_usage() {
    let temp = tempdir().expect("create out-of-root parsed path fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "out_of_root_parsed_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\n#[path = \"../shared.rs\"]\nmod consumer;\npub(super) use child::{ParsedThing, UnusedThing};\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/child.rs"),
        "pub struct ParsedThing;\npub struct UnusedThing;\n",
    )
    .expect("write facade subjects");
    fs::write(
        temp.path().join("shared.rs"),
        "fn consume(_: super::ParsedThing) {}\n",
    )
    .expect("write out-of-root parsed path");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct ParsedThing")
        }),
        "the parsed path outside src must count as facade usage: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct UnusedThing")
        }),
        "the unreferenced sibling must retain its stale-facade finding: {report:#?}"
    );

    let findings_dir = temp.path().join("target/mend-findings");
    let mut parsed_path_fact = false;
    let mut unused_fact = false;
    for entry in fs::read_dir(&findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        let fix_facts = stored_report
            .get("pub_use_fix_facts")
            .and_then(Value::as_array)
            .expect("read stored pub-use fix facts");
        for fact in fix_facts {
            match fact.get("child_item_name").and_then(Value::as_str) {
                Some("ParsedThing") => parsed_path_fact = true,
                Some("UnusedThing") => unused_fact = true,
                _ => {},
            }
        }
    }
    assert!(
        !parsed_path_fact,
        "the parsed path outside src must prevent a ParsedThing pub-use fix fact"
    );
    assert!(
        unused_fact,
        "the unreferenced sibling must retain its pub-use fix fact"
    );
}

#[test]
fn cfg_attr_path_candidate_macro_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create cfg_attr path fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_attr_path_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\n#[cfg_attr(any(), path = \"conditional.rs\")]\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(temp.path().join("src/consumer.rs"), "")
        .expect("write conventional module candidate");
    fs::write(
        temp.path().join("src/conditional.rs"),
        "const _: &str = stringify!(crate::a::Thing);\n",
    )
    .expect("write conditional path candidate");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "the conditional path candidate must contribute literal facade usage: {report:#?}"
    );
}

#[test]
fn outside_package_root_macro_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create outside-package literal fixture dir");
    let package_root = temp.path().join("package");
    fs::create_dir_all(package_root.join("src/a/b")).expect("create fixture modules");
    // The package root is a subdirectory here, and config discovery does not
    // climb above it — the pin has to land beside the manifest, not beside it.
    pin_pub_in_path(&package_root, PubInPath::Permitted);

    fs::write(
        package_root.join("Cargo.toml"),
        r#"[package]
name = "outside_package_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        package_root.join("src/main.rs"),
        "mod a;\n#[path = \"../../shared.rs\"]\nmod shared;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(package_root.join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        package_root.join("src/a/b.rs"),
        "mod child;\npub(super) use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(package_root.join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("shared.rs"),
        "macro_rules! mention { () => { stringify!(crate::a::b::Thing) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write outside-package macro literal");

    let report = run_mend_json(&package_root.join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "the current-crate macro literal outside the package must count as facade usage: \
         {report:#?}"
    );
}

#[test]
fn macro_only_descendant_literal_counts_as_inside_facade_usage() {
    let temp = tempdir().expect("create macro-only descendant fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "macro_only_descendant_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "macro_rules! swallow { ($($tokens:tt)*) => {}; }\n\
         mod child;\nmod macro_only;\npub(super) use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/b/macro_only.rs"),
        "swallow!(crate::a::b::Thing);\n",
    )
    .expect("write macro-only descendant");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "the macro-only descendant must keep its used pub(super) facade: {report:#?}"
    );
}

#[test]
fn foreign_crate_macro_literal_does_not_count_as_facade_usage() {
    let temp = tempdir().expect("create foreign crate literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "foreign_crate_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\nmod macro_only;\npub(super) use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/b/macro_only.rs"),
        "const _: &str = stringify!(some_crate::a::b::Thing);\n",
    )
    .expect("write foreign crate macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "a foreign crate macro literal must not count as current-crate facade usage: {report:#?}"
    );
}

#[test]
fn literal_crate_paths_accept_separator_trivia_but_reject_foreign_crates() {
    let temp = tempdir().expect("create separator trivia fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "literal_separator_trivia_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::{CommentThing, ForeignThing, SpaceThing};\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/child.rs"),
        "pub struct CommentThing;\npub struct ForeignThing;\npub struct SpaceThing;\n",
    )
    .expect("write facade subjects");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"const _: &str = stringify!(crate
    :: a :: SpaceThing);
const _: &str = stringify!(crate::a /* separator comment */ :: CommentThing);
const _: &str = stringify!(some_crate :: a :: ForeignThing);
"#,
    )
    .expect("write literal consumers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for used_item in ["struct CommentThing", "struct SpaceThing"] {
        assert!(
            !report.findings.iter().any(|finding| {
                finding.code == DiagnosticCode::SuspiciousPub
                    && finding.path == "src/a/child.rs"
                    && finding.item.as_deref() == Some(used_item)
            }),
            "separator trivia must preserve {used_item}: {report:#?}"
        );
    }
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/child.rs"
                && finding.item.as_deref() == Some("struct ForeignThing")
        }),
        "a foreign crate path must not preserve ForeignThing: {report:#?}"
    );
}

#[test]
fn literal_crate_paths_ignore_comment_markers_inside_literals() {
    let temp = tempdir().expect("create literal marker fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "literal_marker_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod a;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/a.rs"),
        "mod child;\npub use child::{BlockThing, CharThing, RawThing, UrlThing};\n",
    )
    .expect("write facade module");
    fs::write(
        temp.path().join("src/a/child.rs"),
        "pub struct BlockThing;\npub struct CharThing;\npub struct RawThing;\npub struct UrlThing;\n",
    )
    .expect("write facade subjects");
    fs::write(
        temp.path().join("src/consumer.rs"),
        r##"const _: (&str, &str) = ("https://example.com", stringify!(crate::a::UrlThing));
const _: (&str, &str) = (r#"\"//raw.example"#, stringify!(crate::a::RawThing));
const _: (char, &str, &str) = ('"', "https://char.example", stringify!(crate::a::CharThing));
const _: (&str, &str) = ("*/", stringify!(crate::a /* separator */ :: BlockThing));
"##,
    )
    .expect("write literal marker consumers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    for used_item in [
        "struct BlockThing",
        "struct CharThing",
        "struct RawThing",
        "struct UrlThing",
    ] {
        assert!(
            !report.findings.iter().any(|finding| {
                finding.code == DiagnosticCode::SuspiciousPub
                    && finding.path == "src/a/child.rs"
                    && finding.item.as_deref() == Some(used_item)
            }),
            "literal comment markers must preserve {used_item}: {report:#?}"
        );
    }
}

#[test]
fn unicode_literal_paths_respect_identifier_boundaries() {
    let temp = tempdir().expect("create Unicode literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/café")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "unicode_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        "#[path = \"café.rs\"]\nmod café;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/café.rs"),
        "#[path = \"café/child.rs\"]\nmod child;\npub use child::{Café, Thing};\n",
    )
    .expect("write Unicode facade module");
    fs::write(
        temp.path().join("src/café/child.rs"),
        "pub struct Café;\npub struct Thing;\n",
    )
    .expect("write Unicode facade subjects");
    fs::write(
        temp.path().join("src/consumer.rs"),
        "macro_rules! swallow { ($($tokens:tt)*) => {}; }\n\
         swallow!(crate::café::Thing);\n\
         swallow!(crate::café::Caféø);\n",
    )
    .expect("write Unicode literal consumers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/café/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "the Unicode module path must preserve Thing: {report:#?}"
    );
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/café/child.rs"
                && finding.item.as_deref() == Some("struct Café")
        }),
        "Café inside the longer Caféø identifier must not count as usage: {report:#?}"
    );
}

#[test]
fn dollar_crate_macro_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create dollar crate literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dollar_crate_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\nmod macro_only;\npub(super) use child::Thing;\n",
    )
    .expect("write facade module");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub struct Thing;\n")
        .expect("write facade subject");
    fs::write(
        temp.path().join("src/a/b/macro_only.rs"),
        "macro_rules! mention { () => { stringify!($crate::a::b::Thing) }; }\n\
         const _: &str = mention!();\n",
    )
    .expect("write dollar crate macro literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub
                && finding.path == "src/a/b/child.rs"
                && finding.item.as_deref() == Some("struct Thing")
        }),
        "a dollar-crate macro literal must count as current-crate facade usage: {report:#?}"
    );
}

#[test]
fn raw_identifier_macro_literal_counts_as_facade_usage() {
    let temp = tempdir().expect("create raw identifier literal fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "raw_identifier_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
    fs::write(
        temp.path().join("src/a/b.rs"),
        "mod child;\nmod macro_only;\npub(super) use child::r#type;\n",
    )
    .expect("write raw identifier facade");
    fs::write(temp.path().join("src/a/b/child.rs"), "pub fn r#type() {}\n")
        .expect("write raw identifier facade subject");
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
        "the raw identifier macro literal must keep its used facade: {report:#?}"
    );
}

#[test]
fn inline_descendant_macro_literal_counts_as_inside_facade_usage() {
    let temp = tempdir().expect("create inline descendant literal dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src")).expect("create inline fixture source dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_descendant_literal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write inline fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod a {
    mod child {
        pub struct Thing;
    }

    mod descendant {
        const _: &str = stringify!(crate::a::Thing);
    }

    pub use child::Thing;
}

fn main() {}
"#,
    )
    .expect("write inline facade fixture");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "src/main.rs"
        })
        .unwrap_or_else(|| panic!("missing inline inside-subtree facade finding: {report:#?}"));
    assert!(
        finding.help.iter().any(|help| {
            help.contains("only used through crate-relative paths inside its own subtree")
        }),
        "the inline descendant literal must remain inside the facade subtree: {report:#?}"
    );
}

#[test]
fn same_package_binary_literal_does_not_count_as_library_facade_usage() {
    let temp = tempdir().expect("create package workspace dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("app/src/bin")).expect("create binary source dir");
    fs::create_dir_all(temp.path().join("app/src/tool")).expect("create library module dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[workspace]
members = ["app"]
resolver = "3"
"#,
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("app/Cargo.toml"),
        r#"[package]
name = "same_package_crates_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write package manifest");
    fs::write(temp.path().join("app/src/lib.rs"), "mod tool;\n").expect("write library root");
    fs::write(
        temp.path().join("app/src/tool.rs"),
        "mod item;\npub use item::Thing;\n",
    )
    .expect("write library facade");
    fs::write(
        temp.path().join("app/src/tool/item.rs"),
        "pub struct Thing;\n",
    )
    .expect("write library facade subject");
    fs::write(
        temp.path().join("app/src/bin/probe.rs"),
        "const _: &str = stringify!(crate::tool::Thing);\nfn main() {}\n",
    )
    .expect("write binary literal");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::SuspiciousPub && finding.path == "app/src/tool/item.rs"
        }),
        "a binary target's crate path must not count as library facade usage: {:#?}",
        report.findings
    );
}
