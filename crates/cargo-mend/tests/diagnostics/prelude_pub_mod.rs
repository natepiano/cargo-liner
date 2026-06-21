use crate::support::*;

fn write_manifest(temp: &std::path::Path, name: &str) {
    fs::write(
        temp.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2024"
"#
        ),
    )
    .expect("write fixture manifest");
}

#[test]
fn crate_root_prelude_is_not_flagged_without_override() {
    let temp = tempdir().expect("create temp project dir");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    write_manifest(temp.path(), "prelude_fixture");
    fs::write(
        temp.path().join("src/main.rs"),
        "pub mod prelude;\n\npub fn run() {}\n\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(temp.path().join("src/prelude.rs"), "pub use super::run;\n").expect("write prelude");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        !report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ReviewPubMod && finding.path == "src/main.rs"
        }),
        "crate-root `pub mod prelude;` should be exempt by default, findings: {:?}",
        report.findings
    );
}

#[test]
fn nested_prelude_is_still_flagged() {
    let temp = tempdir().expect("create temp project dir");
    fs::create_dir_all(temp.path().join("src/inner")).expect("create src/inner");
    write_manifest(temp.path(), "nested_prelude_fixture");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod inner;\n\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(temp.path().join("src/inner/mod.rs"), "pub mod prelude;\n").expect("write inner mod");
    fs::write(
        temp.path().join("src/inner/prelude.rs"),
        "pub fn run() {}\n",
    )
    .expect("write nested prelude");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ReviewPubMod && finding.path == "src/inner/mod.rs"
        }),
        "nested `pub mod prelude;` should still be reviewed, findings: {:?}",
        report.findings
    );
}

#[test]
fn crate_root_non_prelude_pub_mod_is_still_flagged() {
    let temp = tempdir().expect("create temp project dir");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    write_manifest(temp.path(), "non_prelude_fixture");
    fs::write(
        temp.path().join("src/main.rs"),
        "pub mod helpers;\n\nfn main() {}\n",
    )
    .expect("write main");
    fs::write(temp.path().join("src/helpers.rs"), "pub fn run() {}\n").expect("write helpers");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report.findings.iter().any(|finding| {
            finding.code == DiagnosticCode::ReviewPubMod && finding.path == "src/main.rs"
        }),
        "crate-root `pub mod helpers;` should still be reviewed, findings: {:?}",
        report.findings
    );
}
