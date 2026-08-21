use std::path::Path;
use std::process::Command;

use crate::support::*;

/// A `--fix-all` fixture that needs more than one convergence pass.
///
/// Pass one rewrites `src/outer/parent.rs` — the `pub use child::SpawnStats;`
/// facade collapses into the call site and `begin` narrows to `pub(super)` —
/// and validates, so those edits stay on disk. The chained `cargo fix` then
/// deletes the now-unused `use crate::deep::thing::Widget;`, and only once that
/// import is gone does `src/deep.rs` become narrowable. That narrowing is pass
/// two's edit, and `build.rs` fails the build the moment it lands.
const FIXTURE_SOURCES: &[(&str, &str)] = &[
    (
        "src/main.rs",
        "mod deep;\nmod outer;\n\nfn main() {\n    outer::go();\n}\n",
    ),
    (
        "src/deep.rs",
        "pub(crate) mod thing;\n\npub(crate) fn seed() {\n    let _ = thing::make();\n}\n",
    ),
    (
        "src/deep/thing.rs",
        "pub struct Widget;\n\npub fn make() -> Widget {\n    Widget\n}\n",
    ),
    (
        "src/outer.rs",
        "mod parent;\n\npub(crate) fn go() {\n    parent::begin();\n}\n",
    ),
    (
        "src/outer/parent.rs",
        "mod child;\npub use child::SpawnStats;\nuse crate::deep::thing::Widget;\n\npub(crate) fn \
         begin() {\n    let _ = child::SpawnStats;\n}\n",
    ),
    ("src/outer/parent/child.rs", "pub struct SpawnStats;\n"),
];

/// Fails the build as soon as the pass-two narrowing of `src/deep.rs` lands,
/// which is what drives that pass's validation failure.
const FIXTURE_BUILD_SCRIPT: &str = r#"fn main() {
    println!("cargo::rerun-if-changed=src");
    let deep = std::fs::read_to_string("src/deep.rs").unwrap();
    assert!(
        deep.contains("pub(crate) mod thing;"),
        "second-pass narrowing landed"
    );
}
"#;

const FIXTURE_MANIFEST: &str = r#"[package]
name = "fix_rollback_fixture"
version = "0.1.0"
edition = "2024"
"#;

/// `--fix-all` chains `cargo fix`, which refuses to edit a package that is not
/// under version control. An empty repository is enough — mend already passes
/// `--allow-dirty` and `--allow-staged`.
fn init_git_repo(project_root: &Path) {
    let status = Command::new("git")
        .arg("init")
        .arg("--quiet")
        .arg(project_root)
        .status()
        .expect("run git init");
    assert!(status.success(), "git init failed for the fixture");
}

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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(temp.path().join("Cargo.toml"), FIXTURE_MANIFEST).expect("write fixture manifest");
    fs::write(temp.path().join("build.rs"), FIXTURE_BUILD_SCRIPT)
        .expect("write fixture build script");
    for (relative_path, source) in FIXTURE_SOURCES {
        let path = temp.path().join(relative_path);
        fs::create_dir_all(path.parent().expect("fixture source has a parent"))
            .expect("create fixture source dir");
        fs::write(&path, source).expect("write fixture source");
    }
    init_git_repo(temp.path());

    let output = mend_command()
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--fix-all")
        .output()
        .expect("run cargo-mend --fix-all");
    assert!(
        !output.status.success(),
        "the fixture's build script must fail the second pass's validation:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("compiler failed after applying mend fixes; changes were rolled back"),
        "expected the mend-fix rollback message, got:\n{stderr}"
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
