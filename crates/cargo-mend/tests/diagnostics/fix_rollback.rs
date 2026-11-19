use crate::support::*;

/// `ImportsAtTop` moves the function import to the file top.
/// The move defers the overlapping `PreferModuleImport` import and call rewrites to the next pass.
/// The compiler reports the moved import in a deprecation warning.
/// The compile-time check rejects the qualified call.
const FIXTURE_SOURCES: &[(&str, &str)] = &[
    (
        "src/main.rs",
        r#"mod scenario;

const SOURCE: &[u8] = include_bytes!("scenario.rs");
const CALL: usize = SOURCE.len() - b"present();\n};\n".len();
// A bare call follows a space. A qualified call follows a colon.
const _: () = assert!(SOURCE[CALL - 1] == b' ', "second-pass module qualification landed");

fn main() {}
"#,
    ),
    (
        "src/scenario.rs",
        r#"#[deprecated]
mod tool;

const _: () = {
    use crate::scenario::tool::helper::present;
    present();
};
"#,
    ),
    (
        "src/scenario/tool.rs",
        "pub(crate) mod helper {\n    pub(crate) const fn present() {}\n}\n",
    ),
];

// Mend validates the binary. `cargo fix --tests` has no test target to build.
const FIXTURE_MANIFEST: &str = r#"[package]
name = "fix_rollback_fixture"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "rollback"
path = "src/main.rs"
test = false
bench = false
"#;

/// Rollback covers the whole invocation, not the pass that failed.
///
/// `main` loops `MendRunner::run` until the fixable set stops shrinking. A
/// snapshot taken per pass restores only to the previous pass's output, so a
/// failure in pass two used to leave pass one's edits on disk while mend still
/// printed "changes were rolled back". The runner's `SessionSnapshot` spans
/// every pass, so the tree comes back byte-identical to what the user had.
#[test]
fn failed_later_pass_restores_the_edits_earlier_passes_left_on_disk() {
    if std::env::var_os("CARGO_MEND_SKIP_NETWORK_TESTS").is_some() {
        eprintln!(
            "skipping failed_later_pass_restores_the_edits_earlier_passes_left_on_disk: \
             CARGO_MEND_SKIP_NETWORK_TESTS is set"
        );
        return;
    }

    let temp = tempdir().expect("create fix rollback fixture dir");
    fs::write(temp.path().join("mend.toml"), MODULE_IMPORT_FIXTURE_CONFIG)
        .expect("configure the rollback fixture");
    fs::write(temp.path().join("Cargo.toml"), FIXTURE_MANIFEST).expect("write fixture manifest");
    for (relative_path, source) in FIXTURE_SOURCES {
        let path = temp.path().join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture source has a parent"))
            .expect("create fixture source dir");
        fs::write(&path, source).expect("write fixture source");
    }
    init_git_repo(temp.path());

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
        .env("CARGO_TERM_PROGRESS_WHEN", "never")
        .arg("--fix-all")
        .output()
        .expect("run cargo-mend --fix-all");
    assert!(
        !output.status.success(),
        "the second pass must fail validation:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compiler failed after applying mend fixes; changes were rolled back"),
        "expected the mend-fix rollback message, got:\n{stderr}"
    );
    assert!(
        stderr.contains("second-pass module qualification landed"),
        "the qualified call must cause the compiler error:\n{stderr}"
    );
    assert!(
        strip_ansi(&stderr)
            .lines()
            .take_while(|line| !line.contains("second-pass module qualification landed"))
            .any(|line| line.ends_with("| use crate::scenario::tool::helper::present;")),
        "the compiler must see the file-level function import before the call rewrite:\n{stderr}"
    );

    for (relative_path, source) in FIXTURE_SOURCES {
        let restored =
            fs::read_to_string(temp.path().join(relative_path)).expect("read restored source");
        assert_eq!(
            restored, *source,
            "{relative_path} must be restored to its pre-invocation text, not to the output of \
             the pass before the one that failed"
        );
    }
}
