//! End-to-end coverage of the `cargo mend` command line: argument handling,
//! report rendering, and exit codes, exercised against fixture workspaces.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(
    unused_imports,
    reason = "shared test support re-exports more helpers than this file uses"
)]
#![allow(
    dead_code,
    reason = "shared test support defines helpers reused by other integration tests"
)]
#![allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]

mod support;

use std::path::Path;
use std::process::Output;

use serde_json::Value;
use support::*;
use tempfile::TempDir;

const JSON_FLAG: &str = "--json";
const RUN_JSON_EXPECTATION: &str = "run cargo-mend --json";
const RUN_MEND_EXPECTATION: &str = "run cargo-mend";
const CREATE_FIXTURE_DIRECTORY_EXPECTATION: &str = "create temp fixture dir";
const WRITE_MANIFEST_EXPECTATION: &str = "write manifest";
const SOURCE_DIRECTORY: &str = "src";
const CREATE_SOURCE_DIRECTORY_EXPECTATION: &str = "create src";
const LIBRARY_FILE_PATH: &str = "src/lib.rs";
const LIBRARY_HELPERS_SOURCE: &str = "mod helpers;\n";
const WRITE_LIBRARY_EXPECTATION: &str = "write lib";
const HELPERS_FILE_PATH: &str = "src/helpers.rs";
const INTERNAL_FUNCTION_SOURCE: &str = "pub fn internal_fn() {}\n";
const WRITE_HELPERS_EXPECTATION: &str = "write helpers";
const CREATE_WORKSPACE_DIRECTORY_EXPECTATION: &str = "create temp workspace dir";
const MEMBER_SOURCE_DIRECTORY: &str = "member/src";
const CREATE_MEMBER_SOURCE_DIRECTORY_EXPECTATION: &str = "create member src";
const MEMBER_EXAMPLE_DIRECTORY: &str = "member/examples/demo";
const CREATE_MEMBER_EXAMPLE_DIRECTORY_EXPECTATION: &str = "create member example dir";
const WORKSPACE_MANIFEST: &str = "[workspace]\nmembers = [\"member\"]\nresolver = \"3\"\n";
const WRITE_WORKSPACE_MANIFEST_EXPECTATION: &str = "write workspace manifest";
const MEMBER_MANIFEST_PATH: &str = "member/Cargo.toml";
const MEMBER_MANIFEST: &str =
    "[package]\nname = \"workspace_member\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const WRITE_MEMBER_MANIFEST_EXPECTATION: &str = "write member manifest";
const MEMBER_LIBRARY_FILE_PATH: &str = "member/src/lib.rs";
const WRITE_MEMBER_LIBRARY_EXPECTATION: &str = "write member lib";
const MEMBER_HELPERS_FILE_PATH: &str = "member/src/helpers.rs";
const WRITE_MEMBER_HELPERS_EXPECTATION: &str = "write member helpers";
const MEMBER_EXAMPLE_ROOT_PATH: &str = "member/examples/demo/main.rs";
const EXAMPLE_ROOT_SOURCE: &str = "mod helper;\nfn main() {}\n";
const WRITE_EXAMPLE_ROOT_EXPECTATION: &str = "write example root";
const MEMBER_EXAMPLE_HELPER_PATH: &str = "member/examples/demo/helper.rs";
const EXAMPLE_HELPER_SOURCE: &str = "pub fn example_only() {}\n";
const WRITE_EXAMPLE_HELPER_EXPECTATION: &str = "write example helper";
const SHARED_PATH_MANIFEST: &str =
    "[package]\nname = \"cli_shared_path_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const CREATE_TARGET_SOURCE_DIRECTORY_EXPECTATION: &str = "create target source dir";
const SHARED_FIXTURE_PATH: &str = "fixtures.rs";
const SHARED_FIXTURE_SOURCE: &str =
    "pub const SHARED_VALUE: usize = 1;\npub const USED_BY_EXAMPLE: usize = 2;\n";
const WRITE_SHARED_FIXTURE_EXPECTATION: &str = "write shared fixture";
const SHARED_LIBRARY_SOURCE: &str = "#[path = \"../fixtures.rs\"]\nmod fixtures;\n";
const SHARED_EXAMPLE_PATH: &str = "examples/demo.rs";
const SHARED_EXAMPLE_SOURCE: &str = "#[path = \"../fixtures.rs\"]\nmod fixtures;\nuse fixtures::USED_BY_EXAMPLE;\nfn main() { let _ = USED_BY_EXAMPLE; }\n";
const WRITE_EXAMPLE_EXPECTATION: &str = "write example";
const SHARED_INTEGRATION_TEST_PATH: &str = "tests/shared.rs";
const SHARED_INTEGRATION_TEST_SOURCE: &str =
    "#[path = \"../fixtures.rs\"]\nmod fixtures;\n#[test]\nfn compiles() {}\n";
const WRITE_INTEGRATION_TEST_EXPECTATION: &str = "write integration test";
const EXAMPLE_DIRECTORY: &str = "examples/demo";
const CREATE_EXAMPLE_DIRECTORY_EXPECTATION: &str = "create example dir";
const LIBRARY_ONLY_SOURCE: &str = "pub fn library_only() {}\n";
const WRITE_LIBRARY_HELPERS_EXPECTATION: &str = "write lib helpers";
const EXAMPLE_ROOT_PATH: &str = "examples/demo/main.rs";
const EXAMPLE_HELPER_PATH: &str = "examples/demo/helper.rs";
const CLEAN_LIBRARY_SOURCE: &str = "pub fn exported() {}\n";
const WRITE_CLEAN_LIBRARY_EXPECTATION: &str = "write clean lib";
const VERSION_FLAG: &str = "--version";
const RUN_VERSION_EXPECTATION: &str = "run cargo-mend --version";
const DECODE_STDOUT_EXPECTATION: &str = "decode stdout";
const BUILD_INFO_FLAG: &str = "--build-info";
const RUN_BUILD_INFO_EXPECTATION: &str = "run cargo-mend --build-info";
const BUILD_INFO_GIT_HASH_FIELD: &str = "\ngit_hash: ";
const BUILD_INFO_BUILD_ID_FIELD: &str = "\nbuild_id: ";
const BUILD_INFO_SYSROOT_FIELD: &str = "\nbuild_sysroot: ";
const DEFAULT_FIXTURE_PACKAGE: &str = "cli_default_fixture";
const POSITIONAL_MANIFEST_FIXTURE_PACKAGE: &str = "cli_positional_manifest_fixture";
const UTF8_PATH_EXPECTATION: &str = "utf-8 path";
const JSON_CLEAN_MANIFEST: &str =
    "[package]\nname = \"cli_json_clean_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
const USED_INTERNAL_FUNCTION_LIBRARY_SOURCE: &str =
    "mod helpers;\n\npub fn exported() { helpers::internal_fn(); }\n";
const FIRST_JSON_LINE_EXPECTATION: &str = "first JSON line";
const PARSE_FIRST_JSON_EXPECTATION: &str = "parse first";
const LAST_JSON_LINE_EXPECTATION: &str = "last JSON line";
const PARSE_LAST_JSON_EXPECTATION: &str = "parse last";
const DECODE_STDERR_EXPECTATION: &str = "decode stderr";
const CARGO_REASON_FIELD: &str = "reason";
const COMPILER_MESSAGE_REASON: &str = "compiler-message";
const PACKAGE_ID_FIELD: &str = "package_id";
const MANIFEST_PATH_FIELD: &str = "manifest_path";
const MESSAGE_TYPE_POINTER: &str = "/message/$message_type";
const DIAGNOSTIC_MESSAGE_TYPE: &str = "diagnostic";
const FIRST_SPAN_SOURCE_POINTER: &str = "/message/spans/0/text/0/text";
const EXPECTED_INTERNAL_FUNCTION_LINE: &str = "pub fn internal_fn() {}";
const BUILD_FINISHED_REASON: &str = "build-finished";
const BUILD_SUCCESS_FIELD: &str = "success";
const WORKSPACE_FLAG: &str = "--workspace";
const ALL_TARGETS_FLAG: &str = "--all-targets";
const SHARED_CONSTANT_ITEM: &str = "const SHARED_VALUE";
const FIX_FLAG: &str = "--fix";
const READ_SHARED_FIXTURE_EXPECTATION: &str = "read shared fixture";
const FIXED_SHARED_FIXTURE_SOURCE: &str =
    "const SHARED_VALUE: usize = 1;\npub(crate) const USED_BY_EXAMPLE: usize = 2;\n";
const LIBRARY_FIXTURE_PACKAGE: &str = "cli_lib_fixture";
const LIBRARY_FLAG: &str = "--lib";
const NAMED_EXAMPLE_FIXTURE_PACKAGE: &str = "cli_named_example_fixture";
const EXAMPLE_FLAG: &str = "--example";
const EXAMPLE_NAME: &str = "demo";
const DRY_RUN_FIXTURE_PACKAGE: &str = "cli_fix_dry_run_fixture";
const DRY_RUN_FLAG: &str = "--dry-run";
const EXPECTED_DRY_RUN_SUMMARY: &str = "would apply 1";
const READ_HELPERS_EXPECTATION: &str = "read helpers";
const EXPECTED_PUBLIC_INTERNAL_FUNCTION: &str = "pub fn internal_fn()";
const MIN_ACCEPTED_EXIT_CODE: i32 = 0;
const MAX_ACCEPTED_EXIT_CODE: i32 = 2;

fn run_mend_json_in(dir: &Path, args: &[&str]) -> Report {
    let output = mend_command()
        .current_dir(dir)
        .args(args)
        .arg(JSON_FLAG)
        .output()
        .expect(RUN_JSON_EXPECTATION);
    assert!(
        matches!(
            output.status.code(),
            Some(MIN_ACCEPTED_EXIT_CODE..=MAX_ACCEPTED_EXIT_CODE)
        ),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_mend_json_output(&output.stdout)
}

fn run_mend_in(dir: &Path, args: &[&str]) -> Output {
    mend_command()
        .current_dir(dir)
        .args(args)
        .output()
        .expect(RUN_MEND_EXPECTATION)
}

fn create_simple_lib_fixture(name: &str) -> TempDir {
    let temp = tempdir().expect(CREATE_FIXTURE_DIRECTORY_EXPECTATION);
    fs::write(
        temp.path().join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect(WRITE_MANIFEST_EXPECTATION);
    fs::create_dir_all(temp.path().join(SOURCE_DIRECTORY))
        .expect(CREATE_SOURCE_DIRECTORY_EXPECTATION);
    fs::write(temp.path().join(LIBRARY_FILE_PATH), LIBRARY_HELPERS_SOURCE)
        .expect(WRITE_LIBRARY_EXPECTATION);
    fs::write(
        temp.path().join(HELPERS_FILE_PATH),
        INTERNAL_FUNCTION_SOURCE,
    )
    .expect(WRITE_HELPERS_EXPECTATION);
    temp
}

fn create_workspace_with_example_fixture() -> TempDir {
    let temp = tempdir().expect(CREATE_WORKSPACE_DIRECTORY_EXPECTATION);
    fs::create_dir_all(temp.path().join(MEMBER_SOURCE_DIRECTORY))
        .expect(CREATE_MEMBER_SOURCE_DIRECTORY_EXPECTATION);
    fs::create_dir_all(temp.path().join(MEMBER_EXAMPLE_DIRECTORY))
        .expect(CREATE_MEMBER_EXAMPLE_DIRECTORY_EXPECTATION);

    fs::write(temp.path().join("Cargo.toml"), WORKSPACE_MANIFEST)
        .expect(WRITE_WORKSPACE_MANIFEST_EXPECTATION);
    fs::write(temp.path().join(MEMBER_MANIFEST_PATH), MEMBER_MANIFEST)
        .expect(WRITE_MEMBER_MANIFEST_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_LIBRARY_FILE_PATH),
        LIBRARY_HELPERS_SOURCE,
    )
    .expect(WRITE_MEMBER_LIBRARY_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_HELPERS_FILE_PATH),
        INTERNAL_FUNCTION_SOURCE,
    )
    .expect(WRITE_MEMBER_HELPERS_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_EXAMPLE_ROOT_PATH),
        EXAMPLE_ROOT_SOURCE,
    )
    .expect(WRITE_EXAMPLE_ROOT_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_EXAMPLE_HELPER_PATH),
        EXAMPLE_HELPER_SOURCE,
    )
    .expect(WRITE_EXAMPLE_HELPER_EXPECTATION);

    temp
}

fn create_shared_path_fixture() -> TempDir {
    let temp = tempdir().expect(CREATE_FIXTURE_DIRECTORY_EXPECTATION);
    fs::write(temp.path().join("Cargo.toml"), SHARED_PATH_MANIFEST)
        .expect(WRITE_MANIFEST_EXPECTATION);
    for directory in ["src", "examples", "tests"] {
        fs::create_dir(temp.path().join(directory))
            .expect(CREATE_TARGET_SOURCE_DIRECTORY_EXPECTATION);
    }
    fs::write(temp.path().join(SHARED_FIXTURE_PATH), SHARED_FIXTURE_SOURCE)
        .expect(WRITE_SHARED_FIXTURE_EXPECTATION);
    fs::write(temp.path().join(LIBRARY_FILE_PATH), SHARED_LIBRARY_SOURCE)
        .expect(WRITE_LIBRARY_EXPECTATION);
    fs::write(temp.path().join(SHARED_EXAMPLE_PATH), SHARED_EXAMPLE_SOURCE)
        .expect(WRITE_EXAMPLE_EXPECTATION);
    fs::write(
        temp.path().join(SHARED_INTEGRATION_TEST_PATH),
        SHARED_INTEGRATION_TEST_SOURCE,
    )
    .expect(WRITE_INTEGRATION_TEST_EXPECTATION);
    temp
}

fn create_simple_workspace_fixture() -> TempDir {
    let temp = tempdir().expect(CREATE_WORKSPACE_DIRECTORY_EXPECTATION);
    fs::create_dir_all(temp.path().join(MEMBER_SOURCE_DIRECTORY))
        .expect(CREATE_MEMBER_SOURCE_DIRECTORY_EXPECTATION);

    fs::write(temp.path().join("Cargo.toml"), WORKSPACE_MANIFEST)
        .expect(WRITE_WORKSPACE_MANIFEST_EXPECTATION);
    fs::write(temp.path().join(MEMBER_MANIFEST_PATH), MEMBER_MANIFEST)
        .expect(WRITE_MEMBER_MANIFEST_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_LIBRARY_FILE_PATH),
        LIBRARY_HELPERS_SOURCE,
    )
    .expect(WRITE_MEMBER_LIBRARY_EXPECTATION);
    fs::write(
        temp.path().join(MEMBER_HELPERS_FILE_PATH),
        INTERNAL_FUNCTION_SOURCE,
    )
    .expect(WRITE_MEMBER_HELPERS_EXPECTATION);

    temp
}

fn create_lib_and_example_fixture(name: &str) -> TempDir {
    let temp = tempdir().expect(CREATE_FIXTURE_DIRECTORY_EXPECTATION);
    fs::write(
        temp.path().join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect(WRITE_MANIFEST_EXPECTATION);
    fs::create_dir_all(temp.path().join(SOURCE_DIRECTORY))
        .expect(CREATE_SOURCE_DIRECTORY_EXPECTATION);
    fs::create_dir_all(temp.path().join(EXAMPLE_DIRECTORY))
        .expect(CREATE_EXAMPLE_DIRECTORY_EXPECTATION);
    fs::write(temp.path().join(LIBRARY_FILE_PATH), LIBRARY_HELPERS_SOURCE)
        .expect(WRITE_LIBRARY_EXPECTATION);
    fs::write(temp.path().join(HELPERS_FILE_PATH), LIBRARY_ONLY_SOURCE)
        .expect(WRITE_LIBRARY_HELPERS_EXPECTATION);
    fs::write(temp.path().join(EXAMPLE_ROOT_PATH), EXAMPLE_ROOT_SOURCE)
        .expect(WRITE_EXAMPLE_ROOT_EXPECTATION);
    fs::write(temp.path().join(EXAMPLE_HELPER_PATH), EXAMPLE_HELPER_SOURCE)
        .expect(WRITE_EXAMPLE_HELPER_EXPECTATION);
    temp
}

fn create_clean_lib_with_example_fixture(name: &str) -> TempDir {
    let temp = tempdir().expect(CREATE_FIXTURE_DIRECTORY_EXPECTATION);
    fs::write(
        temp.path().join("Cargo.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
    )
    .expect(WRITE_MANIFEST_EXPECTATION);
    fs::create_dir_all(temp.path().join(SOURCE_DIRECTORY))
        .expect(CREATE_SOURCE_DIRECTORY_EXPECTATION);
    fs::create_dir_all(temp.path().join(EXAMPLE_DIRECTORY))
        .expect(CREATE_EXAMPLE_DIRECTORY_EXPECTATION);
    fs::write(temp.path().join(LIBRARY_FILE_PATH), CLEAN_LIBRARY_SOURCE)
        .expect(WRITE_CLEAN_LIBRARY_EXPECTATION);
    fs::write(temp.path().join(EXAMPLE_ROOT_PATH), EXAMPLE_ROOT_SOURCE)
        .expect(WRITE_EXAMPLE_ROOT_EXPECTATION);
    fs::write(temp.path().join(EXAMPLE_HELPER_PATH), EXAMPLE_HELPER_SOURCE)
        .expect(WRITE_EXAMPLE_HELPER_EXPECTATION);
    temp
}

#[test]
fn version_flag_prints_version_and_exits_successfully() {
    let output = mend_command()
        .arg(VERSION_FLAG)
        .output()
        .expect(RUN_VERSION_EXPECTATION);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());

    let stdout = String::from_utf8(output.stdout).expect(DECODE_STDOUT_EXPECTATION);
    assert_eq!(stdout, format!("mend {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn build_info_flag_prints_build_metadata_and_exits_successfully() {
    let output = mend_command()
        .arg(BUILD_INFO_FLAG)
        .output()
        .expect(RUN_BUILD_INFO_EXPECTATION);

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());

    let stdout = String::from_utf8(output.stdout).expect(DECODE_STDOUT_EXPECTATION);
    assert!(stdout.starts_with(&format!("cargo-mend {}\n", env!("CARGO_PKG_VERSION"))));
    assert!(stdout.contains(BUILD_INFO_GIT_HASH_FIELD));
    assert!(stdout.contains(BUILD_INFO_BUILD_ID_FIELD));
    assert!(stdout.contains(BUILD_INFO_SYSROOT_FIELD));
}

#[test]
fn default_invocation_from_package_root_reports_findings() {
    let temp = create_simple_lib_fixture(DEFAULT_FIXTURE_PACKAGE);

    let report = run_mend_json_in(temp.path(), &[]);
    let unused_pub_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .collect();

    assert_eq!(unused_pub_findings.len(), 1);
    assert_eq!(unused_pub_findings[0].path, HELPERS_FILE_PATH);
    assert_summary_matches_findings(&report);
}

#[test]
fn positional_manifest_path_reports_findings() {
    let temp = create_simple_lib_fixture(POSITIONAL_MANIFEST_FIXTURE_PACKAGE);

    let report = run_mend_json_in(
        temp.path(),
        &[temp.path().to_str().expect(UTF8_PATH_EXPECTATION)],
    );
    let unused_pub_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .collect();

    assert_eq!(unused_pub_findings.len(), 1);
    assert_eq!(unused_pub_findings[0].path, HELPERS_FILE_PATH);
    assert_summary_matches_findings(&report);
}

#[test]
fn json_success_emits_clean_stdout_and_no_stderr_noise() {
    let temp = tempdir().expect(CREATE_FIXTURE_DIRECTORY_EXPECTATION);
    fs::write(temp.path().join("Cargo.toml"), JSON_CLEAN_MANIFEST)
        .expect(WRITE_MANIFEST_EXPECTATION);
    fs::create_dir_all(temp.path().join(SOURCE_DIRECTORY))
        .expect(CREATE_SOURCE_DIRECTORY_EXPECTATION);
    fs::write(
        temp.path().join(LIBRARY_FILE_PATH),
        USED_INTERNAL_FUNCTION_LIBRARY_SOURCE,
    )
    .expect(WRITE_LIBRARY_EXPECTATION);
    fs::write(
        temp.path().join(HELPERS_FILE_PATH),
        INTERNAL_FUNCTION_SOURCE,
    )
    .expect(WRITE_HELPERS_EXPECTATION);

    let output = mend_command()
        .current_dir(temp.path())
        .arg(JSON_FLAG)
        .output()
        .expect(RUN_JSON_EXPECTATION);
    assert!(
        matches!(
            output.status.code(),
            Some(MIN_ACCEPTED_EXIT_CODE..=MAX_ACCEPTED_EXIT_CODE)
        ),
        "cargo-mend returned unexpected status {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect(DECODE_STDOUT_EXPECTATION);
    let json_lines = stdout.lines().collect::<Vec<_>>();
    let first_message: Value =
        serde_json::from_str(json_lines.first().expect(FIRST_JSON_LINE_EXPECTATION))
            .expect(PARSE_FIRST_JSON_EXPECTATION);
    let last_message: Value =
        serde_json::from_str(json_lines.last().expect(LAST_JSON_LINE_EXPECTATION))
            .expect(PARSE_LAST_JSON_EXPECTATION);
    let report = parse_mend_json_output(stdout.as_bytes());
    let stderr = strip_ansi(&String::from_utf8(output.stderr).expect(DECODE_STDERR_EXPECTATION));

    assert_eq!(
        first_message
            .get(CARGO_REASON_FIELD)
            .and_then(Value::as_str),
        Some(COMPILER_MESSAGE_REASON)
    );
    assert!(
        first_message
            .get(PACKAGE_ID_FIELD)
            .and_then(Value::as_str)
            .is_some_and(|package_id| !package_id.is_empty())
    );
    assert!(
        first_message
            .get(MANIFEST_PATH_FIELD)
            .and_then(Value::as_str)
            .is_some_and(|manifest_path| manifest_path.ends_with("Cargo.toml"))
    );
    assert_eq!(
        first_message.pointer(MESSAGE_TYPE_POINTER),
        Some(&Value::String(DIAGNOSTIC_MESSAGE_TYPE.to_string()))
    );
    assert_eq!(
        first_message
            .pointer(FIRST_SPAN_SOURCE_POINTER)
            .and_then(Value::as_str),
        Some(EXPECTED_INTERNAL_FUNCTION_LINE)
    );
    assert_eq!(
        last_message.get(CARGO_REASON_FIELD).and_then(Value::as_str),
        Some(BUILD_FINISHED_REASON)
    );
    assert_eq!(
        last_message
            .get(BUILD_SUCCESS_FIELD)
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        stderr, "",
        "expected successful `--json` run to keep stderr quiet"
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn workspace_flag_from_workspace_root_reports_member_findings() {
    let temp = create_simple_workspace_fixture();

    let report = run_mend_json_in(temp.path(), &[WORKSPACE_FLAG]);
    let unused_pub_findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .collect();

    assert_eq!(unused_pub_findings.len(), 1);
    assert_eq!(unused_pub_findings[0].path, MEMBER_HELPERS_FILE_PATH);
    assert_summary_matches_findings(&report);
}

#[test]
fn workspace_all_targets_includes_example_target_findings() {
    let temp = create_workspace_with_example_fixture();

    let report = run_mend_json_in(temp.path(), &[WORKSPACE_FLAG, ALL_TARGETS_FLAG]);
    let unused_pub_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        unused_pub_paths,
        BTreeSet::from([MEMBER_EXAMPLE_HELPER_PATH, MEMBER_HELPERS_FILE_PATH])
    );
    assert_summary_matches_findings(&report);
}

#[test]
fn all_targets_fix_edits_shared_path_module_once() {
    let temp = create_shared_path_fixture();

    let report = run_mend_json_in(temp.path(), &[ALL_TARGETS_FLAG]);
    let unused_pub_findings = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .collect::<Vec<_>>();
    assert_eq!(unused_pub_findings.len(), 1);
    assert_eq!(unused_pub_findings[0].path, SHARED_FIXTURE_PATH);
    assert_eq!(
        unused_pub_findings[0].item.as_deref(),
        Some(SHARED_CONSTANT_ITEM)
    );

    let output = run_mend_in(temp.path(), &[ALL_TARGETS_FLAG, FIX_FLAG]);
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(temp.path().join(SHARED_FIXTURE_PATH))
            .expect(READ_SHARED_FIXTURE_EXPECTATION),
        FIXED_SHARED_FIXTURE_SOURCE
    );
}

#[test]
fn lib_flag_limits_analysis_to_library_target() {
    let temp = create_lib_and_example_fixture(LIBRARY_FIXTURE_PACKAGE);

    let report = run_mend_json_in(temp.path(), &[LIBRARY_FLAG]);
    let unused_pub_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(unused_pub_paths, BTreeSet::from([HELPERS_FILE_PATH]));
    assert_summary_matches_findings(&report);
}

#[test]
fn named_example_limits_analysis_to_example_target() {
    let temp = create_clean_lib_with_example_fixture(NAMED_EXAMPLE_FIXTURE_PACKAGE);

    let report = run_mend_json_in(temp.path(), &[EXAMPLE_FLAG, EXAMPLE_NAME]);
    let unused_pub_paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::UnusedPub)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();

    assert_eq!(unused_pub_paths, BTreeSet::from([EXAMPLE_HELPER_PATH]));
    assert_summary_matches_findings(&report);
}

#[test]
fn fix_dry_run_smoke_reports_and_preserves_files() {
    let temp = create_simple_lib_fixture(DRY_RUN_FIXTURE_PACKAGE);

    let output = run_mend_in(temp.path(), &[FIX_FLAG, DRY_RUN_FLAG]);
    assert!(
        output.status.success(),
        "cargo-mend --fix --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect(DECODE_STDERR_EXPECTATION);
    assert!(stderr.contains(EXPECTED_DRY_RUN_SUMMARY));

    let helpers =
        fs::read_to_string(temp.path().join(HELPERS_FILE_PATH)).expect(READ_HELPERS_EXPECTATION);
    assert!(helpers.contains(EXPECTED_PUBLIC_INTERNAL_FUNCTION));
}
