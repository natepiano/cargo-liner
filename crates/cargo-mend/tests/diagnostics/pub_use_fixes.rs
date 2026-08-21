use serde_json::Value;
use tempfile::TempDir;

use crate::support::*;

#[test]
fn accepted_restricted_stale_facade_is_not_offered_to_any_fixer() {
    for fix_flag in ["--fix", "--fix-pub-use", "--fix-all"] {
        let temp = tempdir().expect("create restricted stale-facade fixture dir");
        fs::create_dir_all(temp.path().join("src/a/b")).expect("create fixture modules");
        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "restricted_stale_facade_fix_fixture"
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
        fs::write(temp.path().join("src/main.rs"), "mod a;\nfn main() {}\n")
            .expect("write fixture root");
        fs::write(temp.path().join("src/a.rs"), "mod b;\n").expect("write outer module");
        fs::write(
            temp.path().join("src/a/b.rs"),
            "mod c;\n#[allow(unused_imports, reason = \"exercise stale facade handling\")]\npub(super) use c::Thing;\n",
        )
        .expect("write stale facade");
        let child_path = temp.path().join("src/a/b/c.rs");
        let child_source = "pub(in crate::a) struct Thing;\n";
        fs::write(&child_path, child_source).expect("write restricted child");

        let report = run_mend_json(&temp.path().join("Cargo.toml"));
        let finding = report
            .findings
            .iter()
            .find(|finding| finding.code == DiagnosticCode::SuspiciousPub)
            .unwrap_or_else(|| panic!("missing restricted stale-facade finding: {report:#?}"));
        assert_eq!(finding.fix_support, FixSupport::None);
        assert_eq!(report.summary.fixable_with_fix, 0);
        assert_eq!(report.summary.fixable_with_fix_pub_use, 0);
        assert_no_stored_pub_use_fix_facts(&temp);

        let output = mend_command()
            .arg("--manifest-path")
            .arg(temp.path().join("Cargo.toml"))
            .arg(fix_flag)
            .output()
            .expect("run restricted stale-facade fixer");
        assert!(
            output.status.success(),
            "{fix_flag} failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(
            fs::read_to_string(&child_path).expect("read restricted child"),
            child_source,
        );
    }
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
        let facts = stored_report
            .get("pub_use_fix_facts")
            .and_then(Value::as_array)
            .expect("read stored pub-use fix facts");
        assert!(
            facts.is_empty(),
            "a restricted annotation must not write a pub-use fix fact: {stored_report:#?}"
        );
    }
    assert!(stored_report_count > 0, "missing stored findings report");
}

#[test]
fn fix_pub_use_reports_import_cleanup_suggestion_after_summary() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_pub_use_reports_import_cleanup_suggestion_after_summary: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create temp fixture dir");

    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_import_cleanup_notice_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer/parent")).expect("create src/outer/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod outer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/outer.rs"), "mod parent;\n").expect("write outer mod");
    fs::write(
        temp.path().join("src/outer/parent.rs"),
        "mod child;\npub use child::SpawnStats;\nuse child::Leftover;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/outer/parent/child.rs"),
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 1);
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    assert_eq!(report.summary.errors, 0);
    assert_eq!(report.summary.warnings, 2);
    assert_eq!(report.summary.fixable_with_fix, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);
    let codes = report
        .findings
        .iter()
        .map(|finding| finding.code.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        codes,
        BTreeSet::from([
            "replace_deep_super_import",
            "internal_parent_pub_use_facade"
        ])
    );
}

#[test]
fn fix_pub_use_handles_child_items_with_attributes() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
fn fix_pub_use_narrows_child_declared_with_pub_on_its_own_line() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_wrapped_pub_fixture"
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
        "pub\nstruct SpawnStats;\n",
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
    assert_eq!(
        child, "pub(super)\nstruct SpawnStats;\n",
        "a `pub` with no trailing space on its line must still be narrowed"
    );
    let parent =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read fixed parent");
    assert!(
        !parent.contains("pub use child::SpawnStats"),
        "stale facade should have been removed: {parent}"
    );
}

#[test]
fn fix_pub_use_edits_the_intended_facade_when_two_share_a_line() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_shared_line_facade_fixture"
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
        "mod child;\npub use child::Alpha; pub use child::Beta;\n",
    )
    .expect("write actor mod");
    fs::write(
        temp.path().join("src/actor/child.rs"),
        "pub struct Alpha;\npub struct Beta;\n",
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
    assert!(
        child.contains("pub(super) struct Alpha;"),
        "first same-line facade should have been narrowed: {child}"
    );
    assert!(
        child.contains("pub(super) struct Beta;"),
        "second same-line facade should have been narrowed: {child}"
    );
    let parent =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read fixed parent");
    assert!(
        !parent.contains("pub use child::"),
        "both same-line facades should have been removed: {parent}"
    );
}

#[test]
fn fix_pub_use_skips_child_whose_finding_lost_cross_target_reconciliation() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_cross_target_suppression_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    // The lib root's `pub mod actor;` is what exposes the module to both
    // targets; allowlist it so `review_pub_mod` does not fail the run before
    // the pub-use fixer gets to it.
    fs::write(
        temp.path().join("mend.toml"),
        "[visibility]\nallow_pub_mod = [\"src/lib.rs\"]\n",
    )
    .expect("write fixture visibility config");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    // The lib and the bin both compile `src/actor/child.rs`. Only the bin sees
    // the facade as internal, so `apply_shared_source_intersection` drops the
    // `suspicious_pub` finding that authorized the narrowing.
    fs::write(temp.path().join("src/lib.rs"), "pub mod actor;\n").expect("write fixture lib root");
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
    let child_path = temp.path().join("src/actor/child.rs");
    let child_source = "pub struct SpawnStats;\n";
    fs::write(&child_path, child_source).expect("write child");

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

    assert_stored_pub_use_fix_fact_exists(&temp);
    assert_eq!(
        fs::read_to_string(&child_path).expect("read child"),
        child_source,
        "a fact whose finding lost cross-target reconciliation must not be applied"
    );
    // An unchanged child alone does not prove the load-time prune: a
    // `CandidateScreening::Skip` or an unresolved parent export also leaves the
    // child alone, but both count into `PubUseNotice`'s `skipped_unsupported`
    // and print the skip clause. A pruned fact never reaches the scan, so the
    // notice must carry no skip clause at all.
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(
        stderr.contains("mend: no `pub use` fixes available"),
        "the pruned fact should leave no pub-use fix candidates: {stderr}"
    );
    assert!(
        !stderr.contains("unsupported `pub use` candidate"),
        "the fact must be pruned at load, not skipped during the scan: {stderr}"
    );
    let parent =
        fs::read_to_string(temp.path().join("src/actor/mod.rs")).expect("read parent module");
    assert!(
        parent.contains("pub use child::SpawnStats;"),
        "the parent facade must survive alongside the unapplied child narrowing: {parent}"
    );
}

/// Confirms the driver did persist a pub-use fix fact, so a test that then sees
/// no edit is observing the load-time prune rather than a fact that was never
/// written.
fn assert_stored_pub_use_fix_fact_exists(temp: &TempDir) {
    let findings_dir = temp.path().join("target/mend-findings");
    let mut stored_fact_count = 0;
    for entry in fs::read_dir(&findings_dir).expect("read stored findings directory") {
        let path = entry.expect("read stored finding entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).expect("read stored findings report");
        let stored_report = serde_json::from_slice::<Value>(&bytes).expect("parse stored report");
        stored_fact_count += stored_report
            .get("pub_use_fix_facts")
            .and_then(Value::as_array)
            .expect("read stored pub-use fix facts")
            .len();
    }
    assert!(
        stored_fact_count > 0,
        "fixture must persist a pub-use fix fact for the prune to discard"
    );
}

#[test]
fn fix_pub_use_rolls_back_on_failed_cargo_check() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
        ExpectedFinding {
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected_summary.fixable_with_fix_pub_use
    );
}

#[test]
fn fix_pub_use_rewrites_grouped_pub_use_in_file_parent_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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
    assert!(codes.contains("unused_pub"));
    assert_eq!(report.summary.fixable_with_fix, 2);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 6);
}

#[test]
fn fix_pub_use_rewrites_grouped_in_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        BTreeSet::from(["internal_parent_pub_use_facade", "unused_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 2);
}

#[test]
fn fix_pub_use_rewrites_mixed_grouped_subtree_imports_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        BTreeSet::from(["internal_parent_pub_use_facade", "unused_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix, 1);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 3);
}

#[test]
fn fix_pub_use_preserves_parent_local_access_with_private_use() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    assert_eq!(report.summary.fixable_with_fix, 0);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);
}

fn create_preserve_exports_fixture() -> TempDir {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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

    temp
}

#[test]
fn fix_pub_use_preserves_exports_used_outside_parent_via_normal_paths() {
    let temp = create_preserve_exports_fixture();

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
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
        ExpectedFinding {
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
        ExpectedFinding {
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
        ExpectedFinding {
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
        ExpectedFinding {
            code:        DiagnosticCode::SuspiciousPub,
            fix_support: FixSupport::PubUse,
        },
    ];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected_summary.fixable_with_fix_pub_use
    );
}

#[test]
fn fix_pub_use_rewrites_obsidian_report_style_private_parent_use_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        BTreeSet::from(["internal_parent_pub_use_facade", "unused_pub"])
    );
    assert_eq!(report.summary.fixable_with_fix, 3);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 2);
}

#[test]
fn fix_pub_use_skips_grouped_pub_use_with_rename_in_dry_run() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
        code:        DiagnosticCode::SuspiciousPub,
        fix_support: FixSupport::PubUse,
    }];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected_summary.fixable_with_fix_pub_use
    );
}

#[test]
fn fix_pub_use_rewrites_pub_super_parent_facade_in_apply_mode() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_pub_super_parent_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer/parent")).expect("create src/outer/parent");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod outer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(temp.path().join("src/outer.rs"), "mod parent;\n").expect("write outer mod");
    fs::write(
        temp.path().join("src/outer/parent.rs"),
        "mod child;\npub(super) use child::SpawnStats;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/outer/parent/child.rs"),
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
        "cargo-mend --fix-pub-use failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(
        stderr.contains("mend: applied 1 `pub use` fix(es)"),
        "expected pub(super) parent facade to be fixed; stderr was:\n{stderr}"
    );

    let parent_after =
        fs::read_to_string(temp.path().join("src/outer/parent.rs")).expect("read parent");
    assert!(
        !parent_after.contains("pub(super) use child::SpawnStats"),
        "parent re-export should be removed after fix; got:\n{parent_after}"
    );

    let child_after =
        fs::read_to_string(temp.path().join("src/outer/parent/child.rs")).expect("read child");
    assert!(
        child_after.contains("pub(super) struct SpawnStats"),
        "child item should be narrowed to pub(super); got:\n{child_after}"
    );
}

#[test]
fn fix_pub_use_self_heals_unused_imports_left_behind() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_pub_use_self_heals_unused_imports_left_behind: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    // After `--fix-pub-use` rewrites a re-export, sibling files that imported
    // through the now-defunct facade can be left with `unused import`
    // warnings. The orchestrator must run `cargo fix` automatically so
    // `CompilerWarningFacts::UnusedImportWarnings` and every fixable category
    // are empty in a single invocation.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_self_heal_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer/parent")).expect("create dirs");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod outer;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(temp.path().join("src/outer.rs"), "mod parent;\n").expect("write outer");
    // `Leftover` is imported but never referenced — a pre-existing unused
    // import that should also be cleaned up by the chained `cargo fix`.
    fs::write(
        temp.path().join("src/outer/parent.rs"),
        "mod child;\npub use child::SpawnStats;\nuse child::Leftover;\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/outer/parent/child.rs"),
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
        "cargo-mend --fix-pub-use failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let parent_after =
        fs::read_to_string(temp.path().join("src/outer/parent.rs")).expect("read parent");
    assert!(
        !parent_after.contains("use child::Leftover"),
        "self-heal should have removed the unused `use child::Leftover` line; parent.rs:\n{parent_after}"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("consider running cargo fix"),
        "the manual cleanup hint must no longer be emitted; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("imports may now be unused"),
        "the manual cleanup hint must no longer be emitted; stderr:\n{stderr}"
    );
}

#[test]
fn fix_all_converges_in_one_invocation() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping fix_all_converges_in_one_invocation: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    // `--fix-all` must loop the passes until the tree stops changing, so the
    // user never needs to re-run. Fixture: a pub-use rewrite cascade that
    // leaves an unused import (caught by chained cargo fix), with no further
    // mend findings expected on the second scan.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_all_converges_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer/parent")).expect("create dirs");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod outer;\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(temp.path().join("src/outer.rs"), "mod parent;\n").expect("write outer");
    fs::write(
        temp.path().join("src/outer/parent.rs"),
        "mod child;\npub use child::SpawnStats;\nuse child::Leftover;\n",
    )
    .expect("write parent");
    fs::write(
        temp.path().join("src/outer/parent/child.rs"),
        "pub struct SpawnStats;\npub struct Leftover;\n",
    )
    .expect("write child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-all")
        .output()
        .expect("run cargo-mend --fix-all");
    assert!(
        output.status.success(),
        "cargo-mend --fix-all failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    // After convergence, a fresh read-only scan must report zero warnings,
    // zero errors, and zero fixables in every JSON report category.
    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(
        report.summary.errors, 0,
        "errors after --fix-all: {:#?}",
        report.findings
    );
    assert_eq!(
        report.summary.fixable_with_fix, 0,
        "fixable_with_fix should be zero after --fix-all converges"
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use, 0,
        "fixable_with_fix_pub_use should be zero after --fix-all converges"
    );
}

#[test]
fn fix_pub_use_self_heal_does_not_run_cargo_fix_when_no_unused_imports() {
    // Negative case: when --fix-pub-use applies edits but the validation
    // pass observes no `unused import` warnings, the orchestrator must NOT
    // chain cargo fix (which would be a no-op compile cost).
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_no_cascade_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/actor")).expect("create src/actor");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod actor;\nfn main() {}\n",
    )
    .expect("write main");
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
    // `cargo fix` produces a `Compiling` line (or its own progress); since
    // we suppressed --fix-pub-use's own discovery output, the absence of an
    // additional cargo-fix Compiling pass is the cheapest negative signal.
    // The robust positive signal: the apply-pub-use notice still appears.
    assert!(
        stderr.contains("mend: applied 1 `pub use` fix(es)"),
        "apply notice missing; stderr:\n{stderr}"
    );
}

/// Writes a crate whose only reference to a parent facade lives inside the
/// facade's own subtree — the case `internal_parent_pub_use_facade` reports.
/// `subtree_use_site` is the body of `src/tool/inner.rs`, which is what
/// distinguishes an importing subtree from one that writes the path inline.
fn write_internal_facade_fixture(temp: &TempDir, subtree_use_site: &str) {
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::create_dir_all(temp.path().join("src/tool")).expect("create src/tool");
    for (relative_path, source) in [
        (
            "Cargo.toml",
            "[package]\nname = \"internal_facade_fix_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        ),
        (
            "src/main.rs",
            "mod tool;\n\nfn main() {\n    tool::touch();\n}\n",
        ),
        (
            "src/tool/mod.rs",
            "mod inner;\nmod widget;\n\npub use widget::Widget;\n\npub(crate) fn touch() {\n    inner::use_widget();\n}\n",
        ),
        ("src/tool/inner.rs", subtree_use_site),
        ("src/tool/widget.rs", "pub struct Widget;\n"),
    ] {
        fs::write(temp.path().join(relative_path), source)
            .unwrap_or_else(|error| panic!("write {relative_path}: {error}"));
    }
}

fn apply_pub_use_fix(temp: &TempDir) {
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
}

#[test]
fn fix_pub_use_removes_an_internal_facade_and_repoints_its_subtree_import() {
    let temp = tempdir().expect("create internal facade fixture dir");
    write_internal_facade_fixture(
        &temp,
        "use super::Widget;\n\npub(super) fn use_widget() {\n    let _ = Widget;\n}\n",
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let finding = report
        .findings
        .iter()
        .find(|finding| finding.code == DiagnosticCode::InternalParentPubUseFacade)
        .unwrap_or_else(|| panic!("missing internal facade finding: {report:#?}"));
    assert_eq!(finding.fix_support, FixSupport::PubUse);
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);

    apply_pub_use_fix(&temp);

    let parent = fs::read_to_string(temp.path().join("src/tool/mod.rs")).expect("read parent");
    assert!(
        !parent.contains("pub use widget::Widget;"),
        "facade survived the fix: {parent}"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/tool/inner.rs")).expect("read subtree importer"),
        "use super::widget::Widget;\n\npub(super) fn use_widget() {\n    let _ = Widget;\n}\n",
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/tool/widget.rs")).expect("read child"),
        "pub(super) struct Widget;\n",
    );
}

#[test]
fn fix_pub_use_removes_an_internal_facade_and_repoints_its_inline_subtree_path() {
    let temp = tempdir().expect("create internal facade fixture dir");
    write_internal_facade_fixture(
        &temp,
        "pub(super) fn use_widget() {\n    let _ = super::Widget;\n}\n",
    );

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert_eq!(report.summary.fixable_with_fix_pub_use, 1);

    apply_pub_use_fix(&temp);

    let parent = fs::read_to_string(temp.path().join("src/tool/mod.rs")).expect("read parent");
    assert!(
        !parent.contains("pub use widget::Widget;"),
        "facade survived the fix: {parent}"
    );
    // The path is rewritten in place: nothing moves to a `use` line, because the
    // subtree never had one.
    assert_eq!(
        fs::read_to_string(temp.path().join("src/tool/inner.rs")).expect("read subtree path site"),
        "pub(super) fn use_widget() {\n    let _ = super::widget::Widget;\n}\n",
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("src/tool/widget.rs")).expect("read child"),
        "pub(super) struct Widget;\n",
    );
}
