use crate::common::*;

#[test]
fn fix_pub_use_reports_import_cleanup_suggestion_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_import_cleanup_notice_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::SpawnStats;\nuse child::Leftover;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct SpawnStats;\npub struct Leftover;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");

    assert!(stdout.contains("summary:"));
    assert!(
        stderr.contains("mend: applied 1 `pub use` fix(es)"),
        "expected applied pub use notice in stderr:\n{stderr}"
    );
}

#[test]
fn fix_pub_use_rewrites_sibling_imports_and_narrows_child() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_sibling_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["internal_parent_pub_use_facade"]));
}

#[test]
fn fix_pub_use_suppresses_targeted_unused_import_warning_during_discovery() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_suppression_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr
            .contains("mend: suppressing `unused import` warning during `--fix-pub-use` discovery"),
        "expected suppression notice in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("warning: unused import: `child::SpawnStats`"),
        "unexpected forwarded unused-import warning in stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("to apply 1 suggestion"),
        "unexpected forwarded cargo-fix suggestion summary in stderr:\n{stderr}"
    );
}

#[test]
fn dry_run_reports_pub_use_fixes_without_editing_files() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dry_run_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mend: would apply 1 `pub use` fix(es) in dry run"));

    let mod_rs = fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read actor mod");
    let child = fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read child");
    assert!(mod_rs.contains("pub use child::SpawnStats;"));
    assert!(child.contains("pub struct SpawnStats;"));
}

#[test]
fn fix_pub_use_rewrites_nested_descendant_imports() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_nested_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor/nested")).expect("create src/actor/nested");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod nested;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(temp.path().join("src/actor/nested/mod.rs"), "mod deeper;\n")
        .expect("write nested mod");
    fs::write(
        temp.path().join("src/actor/nested/deeper.rs"),
        "use super::super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write deeper");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.error_count, 0);
    assert_eq!(report.summary.warning_count, 1);
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["internal_parent_pub_use_facade"]));
}

#[test]
fn fix_pub_use_handles_child_items_with_attributes() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_attribute_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "#[derive(Debug)]\npub struct SpawnStats;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let child =
        fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read fixed child");
    assert!(child.contains("#[derive(Debug)]\npub(super) struct SpawnStats;"));
}

#[test]
fn fix_pub_use_rolls_back_on_failed_cargo_check() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\nmod broken;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/actor/mod.rs"),
        "mod child;\nmod sibling;\npub use child::SpawnStats;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/actor/sibling.rs"),
        "use super::SpawnStats;\n\nfn use_it(_stats: SpawnStats) {}\n",
    )
    .expect("write sibling");
    fs::write(
        temp.path().join("src/broken.rs"),
        "pub fn broken() -> MissingType { todo!() }\n",
    )
    .expect("write broken");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        !output.status.success(),
        "cargo-mend --fix-pub-use unexpectedly succeeded: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read rolled back mod");
    let child =
        fs::read_to_string(temp.path().join("src/actor/child.rs")).expect("read rolled back child");
    assert!(mod_rs.contains("pub use child::SpawnStats;"));
    assert!(child.contains("pub struct SpawnStats;"));
}

#[test]
fn fix_pub_use_reports_when_nothing_is_fixable() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_noop_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod private_parent;\nuse private_parent::PublicContainer;\n\nfn main() { let _ = std::mem::size_of::<PublicContainer>(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::PublicContainer;\n",
    )
    .expect("write private_parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: no `pub use` fixes available"));
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 2 `pub use` fix(es) in dry run"));
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = fs::read_to_string(temp.path().join("src/parent.rs")).expect("read fixed parent");
    let child =
        fs::read_to_string(temp.path().join("src/parent/child.rs")).expect("read fixed child");
    assert!(!parent.contains("pub use"));
    assert!(child.contains("pub(super) struct Thing;"));
    assert!(child.contains("pub(super) struct Other;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed grouped fixture");
    assert!(
        check.status.success(),
        "cargo check failed after grouped apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_rewrites_multiline_grouped_pub_use_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_multiline_grouped_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{\n    Thing,\n    Other,\n};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");
    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 2 `pub use` fix(es) in dry run"));
    assert!(!stderr.contains("warning: unused imports: `Thing` and `Other`"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_file_parent_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "file_parent_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod private_parent;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/private_parent.rs"),
        "mod child;\npub use child::{PublicContainer, Other};\n",
    )
    .expect("write file parent");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\npub struct Other;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .output()
        .expect("run cargo-mend --fix-pub-use");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent = fs::read_to_string(temp.path().join("src/private_parent.rs"))
        .expect("read fixed file parent");
    let child = fs::read_to_string(temp.path().join("src/private_parent/child.rs"))
        .expect("read fixed child");

    assert!(!parent.contains("pub use"));
    assert!(child.contains("pub(super) struct PublicContainer;"));
    assert!(child.contains("pub(super) struct Other;"));

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .output()
        .expect("cargo check fixed file-parent grouped fixture");
    assert!(
        check.status.success(),
        "cargo check failed after file-parent grouped apply fix: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn fix_pub_use_rewrites_obsidian_style_grouped_file_facades_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "obsidian_style_grouped_facades_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod report;
mod utils;

use report::ReportWriter;
use utils::Sha256Cache;

fn main() {
    let _ = ReportWriter;
    let _ = Sha256Cache;
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        r#"mod report_consumer;
mod report_writer;

pub use report_writer::{ReportDefinition, ReportWriter};
"#,
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        r#"pub trait ReportDefinition {}

pub struct ReportWriter;
"#,
    )
    .expect("write report writer child");
    fs::write(
        temp.path().join("src/report/report_consumer.rs"),
        r#"use super::ReportDefinition;

pub fn accept<T: ReportDefinition>(_value: &T) {}
"#,
    )
    .expect("write report consumer");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;
mod sha256_cache;
mod status_consumer;

pub use file_utils::{collect_repository_files, RepositoryFiles};
pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, CachedImageInfo, Sha256Cache};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub fn collect_repository_files() {}

pub struct RepositoryFiles;
"#,
    )
    .expect("write file utils child");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheEntryStatus {
    Fresh,
}

pub enum CacheFileStatus {
    Present,
}

pub struct CachedImageInfo;

pub struct Sha256Cache;
"#,
    )
    .expect("write sha256 child");
    fs::write(
        temp.path().join("src/utils/status_consumer.rs"),
        r#"use super::CacheEntryStatus;

pub fn touch(_: CacheEntryStatus) {}
"#,
    )
    .expect("write status consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert!(codes.contains("internal_parent_pub_use_facade"));
    assert!(codes.contains("suspicious_pub"));
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 4);
}

#[test]
fn fix_pub_use_rewrites_grouped_in_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{ReportDefinition, ReportWriter};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/sibling.rs"),
        "use crate::parent::{ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: ReportWriter, _: T) {}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from(["internal_parent_pub_use_facade", "suspicious_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn fix_pub_use_rewrites_mixed_grouped_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_mixed_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod report;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        "mod report_writer;\nmod frontmatter;\npub use report_writer::{DescriptionBuilder, ReportDefinition, ReportWriter};\n",
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write report child");
    fs::write(
        temp.path().join("src/report/frontmatter.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write report consumer");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from(["internal_parent_pub_use_facade", "suspicious_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn fix_pub_use_preserves_parent_local_access_with_private_use() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_parent_local_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\nuse crate::parent::InlineCodeExcluder;\n\nfn main() { let _ = InlineCodeExcluder::new(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{CodeBlockExcluder, InlineCodeExcluder};\n\nfn build() -> (CodeBlockExcluder, InlineCodeExcluder) {\n    (CodeBlockExcluder::new(), InlineCodeExcluder::new())\n}\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct CodeBlockExcluder;\npub struct InlineCodeExcluder;\nimpl CodeBlockExcluder { pub fn new() -> Self { Self } }\nimpl InlineCodeExcluder { pub fn new() -> Self { Self } }\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(codes, BTreeSet::from(["internal_parent_pub_use_facade"]));
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn fix_pub_use_preserves_exports_used_outside_parent_via_normal_paths() {
    let temp = tempdir().expect("create temp fixture dir");
    fs::create_dir_all(temp.path().join("src/utils")).expect("create src/utils");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_preserves_path_based_exports_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod config;
mod utils;

fn main() {
    config::run();
}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/config.rs"),
        r#"pub fn run() {
    let _ = crate::utils::expand_tilde("~/vault");
}
"#,
    )
    .expect("write fixture config");
    fs::write(
        temp.path().join("src/utils.rs"),
        r#"mod file_utils;
mod sha256_cache;

pub use file_utils::{expand_tilde, RepositoryFiles};
pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, CachedImageInfo, Sha256Cache};
"#,
    )
    .expect("write utils facade");
    fs::write(
        temp.path().join("src/utils/file_utils.rs"),
        r#"pub fn expand_tilde(_path: &str) -> String {
    String::from("/tmp/vault")
}

pub struct RepositoryFiles;
"#,
    )
    .expect("write file utils child");
    fs::write(
        temp.path().join("src/utils/sha256_cache.rs"),
        r#"pub enum CacheEntryStatus {
    Fresh,
}

pub enum CacheFileStatus {
    Present,
}

pub struct CachedImageInfo;

pub struct Sha256Cache;
"#,
    )
    .expect("write sha256 child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 5 `pub use` fix(es) in dry run"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
        ExpectedFinding {
            code:        "suspicious_pub",
            fix_support: FixSupport::FixPubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}

#[test]
fn fix_pub_use_rewrites_obsidian_report_style_private_parent_use_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_obsidian_report_style_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/report")).expect("create src/report");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod report;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/report.rs"),
        "mod frontmatter_issues_report;\nmod invalid_wikilink_report;\nmod report_writer;\n\npub use report_writer::{ReportDefinition, ReportWriter};\nuse report_writer::DescriptionBuilder;\n\npub fn parent_local() {\n    let _ = DescriptionBuilder::new();\n}\n",
    )
    .expect("write report facade");
    fs::write(
        temp.path().join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n\nimpl DescriptionBuilder {\n    pub fn new() -> Self { Self }\n}\n",
    )
    .expect("write report writer child");
    fs::write(
        temp.path().join("src/report/frontmatter_issues_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write frontmatter report child");
    fs::write(
        temp.path().join("src/report/invalid_wikilink_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items_again<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write invalid wikilink report child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from(["internal_parent_pub_use_facade", "suspicious_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix_count, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use_count, 0);
}

#[test]
fn fix_pub_use_skips_grouped_pub_use_with_rename_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_rename_skip_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/parent")).expect("create src/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod parent;\n\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/parent.rs"),
        "mod child;\npub use child::{Thing as RenamedThing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-pub-use")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix-pub-use --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix-pub-use --dry-run failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: would apply 1 `pub use` fix(es) in dry run"));

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings = [ExpectedFinding {
        code:        "suspicious_pub",
        fix_support: FixSupport::FixPubUse,
    }];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
}
