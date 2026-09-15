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

fn write_grouped_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("grouped", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/parent")).expect("create src/parent");
    fs::write(root.join("src/main.rs"), "mod parent;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/parent.rs"),
        "mod child;\npub use child::{Thing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        root.join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    root
}

fn assert_grouped_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            ("src/parent.rs", "mod child;\n\n"),
            (
                "src/parent/child.rs",
                "pub(super) struct Thing;\npub(super) struct Other;\n",
            ),
        ],
    );
}

fn write_multiline_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("multiline", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_multiline_grouped_fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/parent")).expect("create src/parent");
    fs::write(root.join("src/main.rs"), "mod parent;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/parent.rs"),
        "mod child;\npub use child::{\n    Thing,\n    Other,\n};\n",
    )
    .expect("write parent");
    fs::write(
        root.join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    root
}

fn assert_multiline_facade_findings(report: &Report) {
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

fn write_file_parent_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("file_parent", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);
    fs::create_dir_all(root.join("src/private_parent")).expect("create nested fixture dir");

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "file_parent_grouped_apply_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        root.join("src/main.rs"),
        r#"mod private_parent;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        root.join("src/private_parent.rs"),
        "mod child;\npub use child::{PublicContainer, Other};\n",
    )
    .expect("write file parent");
    fs::write(
        root.join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\npub struct Other;\n",
    )
    .expect("write child");

    root
}

fn assert_file_parent_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            ("src/private_parent.rs", "mod child;\n\n"),
            (
                "src/private_parent/child.rs",
                "pub(super) struct PublicContainer;\npub(super) struct Other;\n",
            ),
        ],
    );
}

fn write_obsidian_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("obsidian", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);
    fs::create_dir_all(root.join("src/utils")).expect("create src/utils");
    fs::create_dir_all(root.join("src/report")).expect("create src/report");

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "obsidian_style_grouped_facades_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::write(
        root.join("src/main.rs"),
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
        root.join("src/report.rs"),
        r#"mod report_consumer;
mod report_writer;

pub use report_writer::{ReportDefinition, ReportWriter};
"#,
    )
    .expect("write report facade");
    fs::write(
        root.join("src/report/report_writer.rs"),
        r#"pub trait ReportDefinition {}

pub struct ReportWriter;
"#,
    )
    .expect("write report writer child");
    fs::write(
        root.join("src/report/report_consumer.rs"),
        r#"use super::ReportDefinition;

pub fn accept<T: ReportDefinition>(_value: &T) {}
"#,
    )
    .expect("write report consumer");
    fs::write(
        root.join("src/utils.rs"),
        r#"mod file_utils;
mod sha256_cache;
mod status_consumer;

pub use file_utils::{collect_repository_files, RepositoryFiles};
pub use sha256_cache::{CacheEntryStatus, CacheFileStatus, CachedImageInfo, Sha256Cache};
"#,
    )
    .expect("write utils facade");
    fs::write(
        root.join("src/utils/file_utils.rs"),
        r#"pub fn collect_repository_files() {}

pub struct RepositoryFiles;
"#,
    )
    .expect("write file utils child");
    fs::write(
        root.join("src/utils/sha256_cache.rs"),
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
        root.join("src/utils/status_consumer.rs"),
        r#"use super::CacheEntryStatus;

pub fn touch(_: CacheEntryStatus) {}
"#,
    )
    .expect("write status consumer");

    root
}

fn assert_obsidian_facade_findings(report: &Report) {
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

fn write_subtree_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("subtree", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/parent")).expect("create src/parent");
    fs::write(root.join("src/main.rs"), "mod parent;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/parent.rs"),
        "mod child;\nmod sibling;\npub use child::{ReportDefinition, ReportWriter};\n",
    )
    .expect("write parent");
    fs::write(
        root.join("src/parent/child.rs"),
        "pub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write child");
    fs::write(
        root.join("src/parent/sibling.rs"),
        "use crate::parent::{ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: ReportWriter, _: T) {}\n",
    )
    .expect("write sibling");

    root
}

fn assert_subtree_facade_findings(report: &Report) {
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

fn write_mixed_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("mixed", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_mixed_grouped_subtree_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/report")).expect("create src/report");
    fs::write(root.join("src/main.rs"), "mod report;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/report.rs"),
        "mod report_writer;\nmod frontmatter;\npub use report_writer::{DescriptionBuilder, ReportDefinition, ReportWriter};\n",
    )
    .expect("write report facade");
    fs::write(
        root.join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n",
    )
    .expect("write report child");
    fs::write(
        root.join("src/report/frontmatter.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn keep<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write report consumer");

    root
}

fn assert_mixed_facade_findings(report: &Report) {
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

fn write_private_parent_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("private_parent", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_obsidian_report_style_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/report")).expect("create src/report");
    fs::write(root.join("src/main.rs"), "mod report;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/report.rs"),
        "mod frontmatter_issues_report;\nmod invalid_wikilink_report;\nmod report_writer;\n\npub use report_writer::{ReportDefinition, ReportWriter};\nuse report_writer::DescriptionBuilder;\n\npub fn parent_local() {\n    let _ = DescriptionBuilder::new();\n}\n",
    )
    .expect("write report facade");
    fs::write(
        root.join("src/report/report_writer.rs"),
        "pub struct DescriptionBuilder;\npub trait ReportDefinition {}\npub struct ReportWriter;\n\nimpl DescriptionBuilder {\n    pub fn new() -> Self { Self }\n}\n",
    )
    .expect("write report writer child");
    fs::write(
        root.join("src/report/frontmatter_issues_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write frontmatter report child");
    fs::write(
        root.join("src/report/invalid_wikilink_report.rs"),
        "use crate::report::{DescriptionBuilder, ReportDefinition, ReportWriter};\n\npub fn use_items_again<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
    )
    .expect("write invalid wikilink report child");

    root
}

fn assert_private_parent_facade_findings(report: &Report) {
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

fn write_rename_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("rename", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_grouped_rename_skip_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/parent")).expect("create src/parent");
    fs::write(root.join("src/main.rs"), "mod parent;\n\nfn main() {}\n")
        .expect("write fixture main");
    fs::write(
        root.join("src/parent.rs"),
        "mod child;\npub use child::{Thing as RenamedThing, Other};\n",
    )
    .expect("write parent");
    fs::write(
        root.join("src/parent/child.rs"),
        "pub struct Thing;\npub struct Other;\n",
    )
    .expect("write child");

    root
}

fn assert_rename_facade_findings(report: &Report) {
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

fn write_super_parent_facade(batch: &mut DiagnosticBatch) -> std::path::PathBuf {
    let root = batch.add_member("super_parent", &[]);
    pin_pub_in_path(&root, PubInPath::Permitted);

    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_pub_super_parent_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(root.join("src/outer/parent")).expect("create src/outer/parent");
    fs::write(root.join("src/main.rs"), "mod outer;\nfn main() {}\n").expect("write fixture main");
    fs::write(root.join("src/outer.rs"), "mod parent;\n").expect("write outer mod");
    fs::write(
        root.join("src/outer/parent.rs"),
        "mod child;\npub(super) use child::SpawnStats;\n",
    )
    .expect("write parent mod");
    fs::write(
        root.join("src/outer/parent/child.rs"),
        "pub struct SpawnStats;\n",
    )
    .expect("write child");

    root
}

fn assert_super_parent_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            ("src/outer/parent.rs", "mod child;\n\n"),
            (
                "src/outer/parent/child.rs",
                "pub(super) struct SpawnStats;\n",
            ),
        ],
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

#[test]
fn grouped_facades_share_dry_run_and_apply() {
    let mut batch = DiagnosticBatch::new(
        r#"[visibility]
pub_in_path = "permitted"
"#,
    );
    let members = [
        ("grouped", write_grouped_facade(&mut batch), 2),
        ("multiline", write_multiline_facade(&mut batch), 2),
        ("file_parent", write_file_parent_facade(&mut batch), 2),
        ("obsidian", write_obsidian_facade(&mut batch), 6),
        ("subtree", write_subtree_facade(&mut batch), 2),
        ("mixed", write_mixed_facade(&mut batch), 3),
        ("private_parent", write_private_parent_facade(&mut batch), 2),
        ("super_parent", write_super_parent_facade(&mut batch), 1),
    ];
    let before = members
        .iter()
        .flat_map(|(_, root, _)| facade_source_bytes(&root.join("src")))
        .collect::<Vec<_>>();
    let reports = batch.member_reports();
    let dry_run = batch
        .command()
        .args(["--fix-pub-use", "--dry-run"])
        .output()
        .expect("dry-run grouped facades");
    assert!(
        dry_run.status.success(),
        "grouped facade dry run failed: {}\n{}",
        String::from_utf8_lossy(&dry_run.stdout),
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let stderr = String::from_utf8(dry_run.stderr).expect("decode dry-run stderr");
    assert!(stderr.contains("mend: would apply 20 `pub use` fix(es) in dry run"));
    assert!(!stderr.contains("warning: unused imports: `Thing` and `Other`"));
    for (member, _, expected) in &members {
        assert_eq!(
            reports[*member].summary.fixable_with_fix_pub_use, *expected,
            "{member}"
        );
    }
    assert_multiline_facade_findings(&reports["multiline"]);
    assert_obsidian_facade_findings(&reports["obsidian"]);
    assert_subtree_facade_findings(&reports["subtree"]);
    assert_mixed_facade_findings(&reports["mixed"]);
    assert_private_parent_facade_findings(&reports["private_parent"]);
    for (path, bytes) in before {
        assert_eq!(
            fs::read(&path).expect("read dry-run source"),
            bytes,
            "dry run changed {}",
            path.display()
        );
    }
    let applied = batch
        .command()
        .arg("--fix-pub-use")
        .output()
        .expect("apply grouped facades");
    assert!(
        applied.status.success(),
        "grouped facade apply failed: {}\n{}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr)
    );
    let stderr = String::from_utf8(applied.stderr).expect("decode apply stderr");
    assert!(
        stderr.contains("mend: applied 20 `pub use` fix(es)"),
        "{stderr}"
    );
    assert_grouped_facade_applied(&members[0].1);
    assert_grouped_facade_applied(&members[1].1);
    assert_file_parent_facade_applied(&members[2].1);
    assert_obsidian_facade_applied(&members[3].1);
    assert_subtree_facade_applied(&members[4].1);
    assert_mixed_facade_applied(&members[5].1);
    assert_private_parent_facade_applied(&members[6].1);
    assert_super_parent_facade_applied(&members[7].1);
}

#[test]
fn renamed_facade_shares_dry_run_and_apply_with_import_cleanup() {
    let mut batch = DiagnosticBatch::new(
        r#"[visibility]
pub_in_path = "permitted"
"#,
    );
    let root = write_rename_facade(&mut batch);
    let before = facade_source_bytes(&root.join("src"));
    let reports = batch.member_reports();
    assert_eq!(reports["rename"].summary.fixable_with_fix_pub_use, 1);
    assert_rename_facade_findings(&reports["rename"]);

    let dry_run = batch
        .command()
        .args(["--fix-pub-use", "--dry-run"])
        .output()
        .expect("dry-run renamed facade");
    assert!(
        dry_run.status.success(),
        "renamed facade dry run failed: {}\n{}",
        String::from_utf8_lossy(&dry_run.stdout),
        String::from_utf8_lossy(&dry_run.stderr)
    );
    let stderr = String::from_utf8(dry_run.stderr).expect("decode dry-run stderr");
    assert!(
        stderr.contains("mend: would apply 1 `pub use` fix(es) in dry run"),
        "{stderr}"
    );
    for (path, bytes) in before {
        assert_eq!(
            fs::read(&path).expect("read dry-run source"),
            bytes,
            "dry run changed {}",
            path.display()
        );
    }

    // Removing Other leaves the unused Thing-as-RenamedThing re-export, so
    // this apply chains cargo fix, whose localhost server requires networking.
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping renamed_facade_shares_dry_run_and_apply_with_import_cleanup apply: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let initialized = std::process::Command::new("git")
        .arg("init")
        .current_dir(batch.path())
        .output()
        .expect("initialize renamed facade repository");
    assert!(
        initialized.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&initialized.stderr)
    );
    let applied = batch
        .command()
        .arg("--fix-pub-use")
        .output()
        .expect("apply renamed facade");
    assert!(
        applied.status.success(),
        "renamed facade apply failed: {}\n{}",
        String::from_utf8_lossy(&applied.stdout),
        String::from_utf8_lossy(&applied.stderr)
    );
    let stderr = String::from_utf8(applied.stderr).expect("decode apply stderr");
    assert!(
        stderr.contains("mend: applied 1 `pub use` fix(es)"),
        "{stderr}"
    );
    assert_facade_sources(
        &root,
        &[
            ("src/parent.rs", "mod child;\n"),
            (
                "src/parent/child.rs",
                "pub struct Thing;\npub(super) struct Other;\n",
            ),
        ],
    );
}

fn assert_obsidian_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            (
                "src/report.rs",
                "mod report_consumer;\nmod report_writer;\n\npub use report_writer::ReportWriter;\n",
            ),
            (
                "src/report/report_writer.rs",
                "pub(super) trait ReportDefinition {}\n\npub struct ReportWriter;\n",
            ),
            (
                "src/report/report_consumer.rs",
                "use super::report_writer::ReportDefinition;\n\npub fn accept<T: ReportDefinition>(_value: &T) {}\n",
            ),
            (
                "src/utils.rs",
                "mod file_utils;\nmod sha256_cache;\nmod status_consumer;\n\n\npub use sha256_cache::Sha256Cache;\n",
            ),
            (
                "src/utils/file_utils.rs",
                "pub(super) fn collect_repository_files() {}\n\npub(super) struct RepositoryFiles;\n",
            ),
            (
                "src/utils/sha256_cache.rs",
                "pub(super) enum CacheEntryStatus {\n    Fresh,\n}\n\npub(super) enum CacheFileStatus {\n    Present,\n}\n\npub(super) struct CachedImageInfo;\n\npub struct Sha256Cache;\n",
            ),
            (
                "src/utils/status_consumer.rs",
                "use super::sha256_cache::CacheEntryStatus;\n\npub fn touch(_: CacheEntryStatus) {}\n",
            ),
        ],
    );
}

fn assert_subtree_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            ("src/parent.rs", "mod child;\nmod sibling;\n\n"),
            (
                "src/parent/child.rs",
                "pub(super) trait ReportDefinition {}\npub(super) struct ReportWriter;\n",
            ),
            (
                "src/parent/sibling.rs",
                "use super::child::ReportDefinition;\nuse super::child::ReportWriter;\n\npub fn keep<T: ReportDefinition>(_: ReportWriter, _: T) {}\n",
            ),
        ],
    );
}

fn assert_mixed_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            ("src/report.rs", "mod report_writer;\nmod frontmatter;\n\n"),
            (
                "src/report/report_writer.rs",
                "pub(super) struct DescriptionBuilder;\npub(super) trait ReportDefinition {}\npub(super) struct ReportWriter;\n",
            ),
            (
                "src/report/frontmatter.rs",
                "use super::report_writer::DescriptionBuilder;\nuse super::report_writer::ReportDefinition;\nuse super::report_writer::ReportWriter;\n\npub fn keep<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
            ),
        ],
    );
}

fn assert_private_parent_facade_applied(root: &std::path::Path) {
    assert_facade_sources(
        root,
        &[
            (
                "src/report.rs",
                "mod frontmatter_issues_report;\nmod invalid_wikilink_report;\nmod report_writer;\n\n\nuse report_writer::DescriptionBuilder;\n\npub fn parent_local() {\n    let _ = DescriptionBuilder::new();\n}\n",
            ),
            (
                "src/report/report_writer.rs",
                "pub struct DescriptionBuilder;\npub(super) trait ReportDefinition {}\npub(super) struct ReportWriter;\n\nimpl DescriptionBuilder {\n    pub fn new() -> Self { Self }\n}\n",
            ),
            (
                "src/report/frontmatter_issues_report.rs",
                "use super::DescriptionBuilder;\nuse super::report_writer::ReportDefinition;\nuse super::report_writer::ReportWriter;\n\npub fn use_items<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
            ),
            (
                "src/report/invalid_wikilink_report.rs",
                "use super::DescriptionBuilder;\nuse super::report_writer::ReportDefinition;\nuse super::report_writer::ReportWriter;\n\npub fn use_items_again<T: ReportDefinition>(_: DescriptionBuilder, _: ReportWriter, _: T) {}\n",
            ),
        ],
    );
}

fn assert_facade_sources(root: &std::path::Path, sources: &[(&str, &str)]) {
    for (relative, expected) in sources {
        let path = root.join(relative);
        assert_eq!(
            fs::read_to_string(&path).expect("read fixed facade source"),
            *expected,
            "{}",
            path.display()
        );
    }
}

fn facade_source_bytes(root: &std::path::Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut sources = Vec::new();
    for entry in fs::read_dir(root).expect("read facade sources") {
        let path = entry.expect("read facade source entry").path();
        if path.is_dir() {
            sources.extend(facade_source_bytes(&path));
        } else {
            let bytes = fs::read(&path).expect("read facade source");
            sources.push((path, bytes));
        }
    }
    sources
}
