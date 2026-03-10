use crate::common::*;

#[test]
fn fix_rewrites_local_crate_import() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod inner;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/inner.rs"),
        r#"use crate::inner::Thing as LocalThing;

pub struct Thing;
"#,
    )
    .expect("write fixture inner");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let inner = fs::read_to_string(temp.path().join("src/inner.rs")).expect("read fixed file");
    assert!(inner.contains("use Thing as LocalThing;"));
    assert!(!inner.contains("use crate::inner::Thing as LocalThing;"));
    assert!(!inner.contains("use self::Thing as LocalThing;"));
}

#[test]
fn fix_does_not_introduce_self_for_same_module_child_imports() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_plain_child_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod private_parent;\npub use private_parent::PublicContainer;\n",
    )
    .expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub use self::child::PublicContainer;"));
}

#[test]
fn fix_preserves_pub_use_visibility() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod private_parent;\npub use private_parent::PublicContainer;\n",
    )
    .expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub use crate::private_parent::child::PublicContainer;"));
}

#[test]
fn fix_preserves_pub_crate_use_visibility() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_pub_crate_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(temp.path().join("src/lib.rs"), "mod private_parent;\n").expect("write fixture lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\npub(crate) use crate::private_parent::child::PublicContainer;\n",
    )
    .expect("write fixture mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct PublicContainer;\n",
    )
    .expect("write fixture child");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let mod_rs =
        fs::read_to_string(temp.path().join("src/private_parent/mod.rs")).expect("read fixed mod");
    assert!(mod_rs.contains("pub(crate) use child::PublicContainer;"));
    assert!(!mod_rs.contains("pub(crate) use crate::private_parent::child::PublicContainer;"));
}

#[test]
fn fix_rolls_back_on_failed_cargo_check() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod inner;
mod broken;

fn main() {}
"#,
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/inner.rs"),
        r#"use crate::inner::Thing as LocalThing;

pub struct Thing;
"#,
    )
    .expect("write fixture inner");
    fs::write(
        temp.path().join("src/broken.rs"),
        "pub fn broken() -> MissingType { todo!() }\n",
    )
    .expect("write fixture broken");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        !output.status.success(),
        "cargo-mend --fix unexpectedly succeeded: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let inner =
        fs::read_to_string(temp.path().join("src/inner.rs")).expect("read rolled back file");
    assert!(inner.contains("use crate::inner::Thing as LocalThing;"));
    assert!(!inner.contains("use Thing as LocalThing;"));
}

#[test]
fn fix_reports_when_nothing_is_fixable() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_noop_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stderr.contains("mend: no import fixes available"));
}

#[test]
fn fix_reports_noop_notice_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_noop_order_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");
    assert!(stdout.contains("No findings."));
    assert!(stderr.contains("mend: no import fixes available"));
}

#[test]
fn fix_reports_applied_notice_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_applied_notice_fixture"
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
        "mod child;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed unexpectedly: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).expect("decode stdout");
    let stderr = String::from_utf8(output.stderr).expect("decode stderr");

    assert!(stdout.contains("summary:"));
    assert!(stderr.contains("mend: applied 1 import fix(es)"));
}

#[test]
fn dry_run_reports_import_fixes_without_editing_files() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "dry_run_import_fixture"
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
    fs::write(temp.path().join("src/parent.rs"), "mod child;\n").expect("write parent mod");
    fs::write(
        temp.path().join("src/parent/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/parent/consumer.rs"),
        "use crate::parent::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write consumer");

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix")
        .arg("--dry-run")
        .output()
        .expect("run cargo-mend --fix --dry-run");
    assert!(
        output.status.success(),
        "cargo-mend --fix --dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("mend: would apply 1 import fix(es) in dry run"));

    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(consumer.contains("use crate::parent::child::Thing;"));
}

#[test]
fn already_local_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "already_local_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/outer")).expect("create src/outer");
    fs::write(temp.path().join("src/lib.rs"), "mod outer;\n").expect("write lib");
    fs::write(
        temp.path().join("src/outer/mod.rs"),
        "mod child;\nmod sibling;\n",
    )
    .expect("write outer mod");
    fs::write(
        temp.path().join("src/outer/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/outer/sibling.rs"),
        "use super::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write sibling");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

#[test]
fn top_level_peer_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "top_level_peer_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod keyboard;\nmod window_event;\n",
    )
    .expect("write lib");
    fs::write(
        temp.path().join("src/keyboard.rs"),
        "use crate::window_event::write_input_event;\n\npub fn call() { write_input_event(); }\n",
    )
    .expect("write keyboard");
    fs::write(
        temp.path().join("src/window_event.rs"),
        "pub fn write_input_event() {}\n",
    )
    .expect("write window_event");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}

#[test]
fn grouped_imports_are_ignored_safely() {
    let temp = tempdir().expect("create temp fixture dir");

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "grouped_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/private_parent")).expect("create src/private_parent");
    fs::write(temp.path().join("src/lib.rs"), "mod private_parent;\n").expect("write lib");
    fs::write(
        temp.path().join("src/private_parent/mod.rs"),
        "mod child;\nuse crate::private_parent::child::{Bar, Baz};\n\nfn use_it(_bar: Bar, _baz: Baz) {}\n",
    )
    .expect("write private_parent mod");
    fs::write(
        temp.path().join("src/private_parent/child.rs"),
        "pub struct Bar;\npub struct Baz;\n",
    )
    .expect("write child");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    let expected_findings: [ExpectedFinding<'_>; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix_count,
        expected_summary.fixable_with_fix_count
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use_count,
        expected_summary.fixable_with_fix_pub_use_count
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == "shorten_local_crate_import")
    );
}
