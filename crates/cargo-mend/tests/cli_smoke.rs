#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(unused_imports, reason = "shared test support re-exports more helpers than this file uses")]
#![allow(dead_code, reason = "shared test support defines helpers reused by other integration tests")]
#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]

mod common;

use std::path::Path;
use std::process::Output;

use common::*;

fn run_mend_json_in(dir: &Path, args: &[&str]) -> Report {
    let output = mend_command()
        .current_dir(dir)
        .args(args)
        .arg("--json")
        .output()
        .expect("run cargo-mend --json");
    assert!(
        matches!(output.status.code(), Some(0..=2)),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "parse mend json report: {error}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn run_mend_in(dir: &Path, args: &[&str]) -> Output {
    mend_command()
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run cargo-mend")
}

fn create_simple_lib_fixture(name: &str) -> tempfile::TempDir {
    let temp = tempdir().expect("create temp fixture dir");
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ),
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write helpers");
    temp
}

fn create_workspace_with_example_fixture() -> tempfile::TempDir {
    let temp = tempdir().expect("create temp workspace dir");
    fs::create_dir_all(temp.path().join("member/src")).expect("create member src");
    fs::create_dir_all(temp.path().join("member/examples/demo"))
        .expect("create member example dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("member/Cargo.toml"),
        "[package]\nname = \"workspace_member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write member manifest");
    fs::write(temp.path().join("member/src/lib.rs"), "mod helpers;\n").expect("write member lib");
    fs::write(
        temp.path().join("member/src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write member helpers");
    fs::write(
        temp.path().join("member/examples/demo/main.rs"),
        "mod helper;\nfn main() {}\n",
    )
    .expect("write example root");
    fs::write(
        temp.path().join("member/examples/demo/helper.rs"),
        "pub fn example_only() {}\n",
    )
    .expect("write example helper");

    temp
}

fn create_simple_workspace_fixture() -> tempfile::TempDir {
    let temp = tempdir().expect("create temp workspace dir");
    fs::create_dir_all(temp.path().join("member/src")).expect("create member src");

    fs::write(
        temp.path().join("Cargo.toml"),
        "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n",
    )
    .expect("write workspace manifest");
    fs::write(
        temp.path().join("member/Cargo.toml"),
        "[package]\nname = \"workspace_member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write member manifest");
    fs::write(temp.path().join("member/src/lib.rs"), "mod helpers;\n").expect("write member lib");
    fs::write(
        temp.path().join("member/src/helpers.rs"),
        "pub fn internal_fn() {}\n",
    )
    .expect("write member helpers");

    temp
}

fn create_lib_and_example_fixture(name: &str) -> tempfile::TempDir {
    let temp = tempdir().expect("create temp fixture dir");
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ),
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::create_dir_all(temp.path().join("examples/demo")).expect("create example dir");
    fs::write(temp.path().join("src/lib.rs"), "mod helpers;\n").expect("write lib");
    fs::write(
        temp.path().join("src/helpers.rs"),
        "pub fn library_only() {}\n",
    )
    .expect("write lib helpers");
    fs::write(
        temp.path().join("examples/demo/main.rs"),
        "mod helper;\nfn main() {}\n",
    )
    .expect("write example root");
    fs::write(
        temp.path().join("examples/demo/helper.rs"),
        "pub fn example_only() {}\n",
    )
    .expect("write example helper");
    temp
}

fn create_clean_lib_with_example_fixture(name: &str) -> tempfile::TempDir {
    let temp = tempdir().expect("create temp fixture dir");
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
        ),
    )
    .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::create_dir_all(temp.path().join("examples/demo")).expect("create example dir");
    fs::write(temp.path().join("src/lib.rs"), "pub fn exported() {}\n").expect("write clean lib");
    fs::write(
        temp.path().join("examples/demo/main.rs"),
        "mod helper;\nfn main() {}\n",
    )
    .expect("write example root");
    fs::write(
        temp.path().join("examples/demo/helper.rs"),
        "pub fn example_only() {}\n",
    )
    .expect("write example helper");
    temp
}

#[test]
fn default_invocation_from_package_root_reports_findings() {
    let temp = create_simple_lib_fixture("cli_default_fixture");

    let report = run_mend_json_in(temp.path(), &[]);
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .collect();

    assert_eq!(narrow_findings.len(), 1);
    assert_eq!(narrow_findings[0].path, "src/helpers.rs");
    assert_summary_matches_findings(&report);
}

#[test]
fn workspace_flag_from_workspace_root_reports_member_findings() {
    let temp = create_simple_workspace_fixture();

    let report = run_mend_json_in(temp.path(), &["--workspace"]);
    let narrow_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .collect();

    assert_eq!(narrow_findings.len(), 1);
    assert_eq!(narrow_findings[0].path, "member/src/helpers.rs");
    assert_summary_matches_findings(&report);
}

#[test]
fn workspace_all_targets_includes_example_target_findings() {
    let temp = create_workspace_with_example_fixture();

    let report = run_mend_json_in(temp.path(), &["--workspace", "--all-targets"]);
    let narrow_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        narrow_paths,
        BTreeSet::from(["member/examples/demo/helper.rs", "member/src/helpers.rs"])
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn lib_flag_limits_analysis_to_library_target() {
    let temp = create_lib_and_example_fixture("cli_lib_fixture");

    let report = run_mend_json_in(temp.path(), &["--lib"]);
    let narrow_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(narrow_paths, BTreeSet::from(["src/helpers.rs"]));
    assert_summary_matches_findings(&report);
}

#[test]
fn named_example_limits_analysis_to_example_target() {
    let temp = create_clean_lib_with_example_fixture("cli_named_example_fixture");

    let report = run_mend_json_in(temp.path(), &["--example", "demo"]);
    let narrow_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::NarrowToPubCrate)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(narrow_paths, BTreeSet::from(["examples/demo/helper.rs"]));
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_dry_run_smoke_reports_and_preserves_files() {
    let temp = create_simple_lib_fixture("cli_fix_dry_run_fixture");

    let output = run_mend_in(temp.path(), &["--fix", "--dry-run"]);
    assert!(
        output.status.success(),
        "cargo-mend --fix --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("would apply 1"));

    let helpers = fs::read_to_string(temp.path().join("src/helpers.rs")).expect("read helpers");
    assert!(helpers.contains("pub fn internal_fn()"));
}
