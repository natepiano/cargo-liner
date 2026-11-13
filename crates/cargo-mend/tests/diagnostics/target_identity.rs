use tempfile::TempDir;

use super::compiled_report;
use crate::support::*;

#[test]
fn target_identity_keeps_separate_declaration_callers() {
    let temp = caller_fixture("library.rs", "binary.rs");
    let report = compiled_report(&temp.path().join("Cargo.toml"));
    assert_codes_at(&report, "src/library.rs", &[DiagnosticCode::UnusedPub]);
    assert_codes_at(
        &report,
        "src/binary.rs",
        &[DiagnosticCode::NarrowToPubCrate],
    );
}

#[test]
fn target_identity_unions_shared_declaration_callers() {
    let temp = caller_fixture("shared.rs", "shared.rs");
    let report = compiled_report(&temp.path().join("Cargo.toml"));
    assert_codes_at(
        &report,
        "src/shared.rs",
        &[DiagnosticCode::NarrowToPubCrate],
    );
}

fn caller_fixture(library_source: &str, binary_source: &str) -> TempDir {
    let temp = fixture(
        "[lib]\ntest = false\nbench = false\n\n[[bin]]\nname = \"target_identity_fixture\"\npath = \"src/main.rs\"\ntest = false\nbench = false\n",
        &[
            (library_source, "pub fn helper() {}\n"),
            (binary_source, "pub fn helper() {}\n"),
        ],
    );
    fs::write(
        temp.path().join("src/lib.rs"),
        format!("#![no_std]\n#[path = {library_source:?}]\nmod api;\n"),
    )
    .expect("write the library root");
    fs::write(
        temp.path().join("src/main.rs"),
        format!("#[path = {binary_source:?}]\nmod api;\nfn main() {{ api::helper(); }}\n"),
    )
    .expect("write the binary root");
    temp
}

#[test]
fn target_identity_library_selection_keeps_shared_sources() {
    let temp = fixture(
        "[lib]\ntest = false\nbench = false\n\n[[bin]]\nname = \"target_identity_fixture\"\npath = \"src/main.rs\"\ntest = false\nbench = false\n",
        &[
            (
                "lib.rs",
                "#![no_std]\nmod library;\nmod shared;\n#[cfg(any())]\nmod binary;\n",
            ),
            ("main.rs", "mod binary;\nmod shared;\nfn main() {}\n"),
            ("library.rs", "pub(crate) fn library_marker() {}\n"),
            ("binary.rs", "pub(crate) fn binary_marker() {}\n"),
            ("shared.rs", "pub(crate) fn shared_marker() {}\n"),
        ],
    );
    assert_selected_paths(&temp, &["--lib"], &["src/library.rs", "src/shared.rs"]);
}

#[test]
fn target_identity_named_binary_selection_separates_roots_in_one_folder() {
    let temp = fixture(
        "[[bin]]\nname = \"one\"\npath = \"src/one.rs\"\ntest = false\nbench = false\n\n[[bin]]\nname = \"two\"\npath = \"src/two.rs\"\ntest = false\nbench = false\n",
        &[
            ("one.rs", "mod selected;\nmod shared;\nfn main() {}\n"),
            ("two.rs", "mod excluded;\nmod shared;\nfn main() {}\n"),
            ("selected.rs", "pub(crate) fn selected_marker() {}\n"),
            ("excluded.rs", "pub(crate) fn excluded_marker() {}\n"),
            ("shared.rs", "pub(crate) fn shared_marker() {}\n"),
        ],
    );
    assert_selected_paths(
        &temp,
        &["--bin", "one"],
        &["src/selected.rs", "src/shared.rs"],
    );
}

#[test]
fn target_identity_named_example_selection_separates_roots_in_one_folder() {
    let temp = fixture(
        "[[example]]\nname = \"one\"\npath = \"src/one.rs\"\ntest = false\nbench = false\n\n[[example]]\nname = \"two\"\npath = \"src/two.rs\"\ntest = false\nbench = false\n",
        &[
            ("one.rs", "mod selected;\nmod shared;\nfn main() {}\n"),
            ("two.rs", "mod excluded;\nmod shared;\nfn main() {}\n"),
            ("selected.rs", "pub(crate) fn selected_marker() {}\n"),
            ("excluded.rs", "pub(crate) fn excluded_marker() {}\n"),
            ("shared.rs", "pub(crate) fn shared_marker() {}\n"),
        ],
    );
    assert_selected_paths(
        &temp,
        &["--example", "one"],
        &["src/selected.rs", "src/shared.rs"],
    );
}

fn fixture(targets: &str, sources: &[(&str, &str)]) -> TempDir {
    let temp = tempdir().expect("create the target identity fixture");
    fs::create_dir(temp.path().join("src")).expect("create the source directory");
    fs::write(
        temp.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"target_identity_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautoexamples = false\nautobins = false\n\n{targets}\n[profile.dev]\ndebug = 0\nincremental = false\n"
        ),
    )
    .expect("write the fixture manifest");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    for (path, source) in sources {
        fs::write(temp.path().join("src").join(path), source).expect("write a fixture source");
    }
    temp
}

fn assert_selected_paths(temp: &TempDir, arguments: &[&str], expected: &[&str]) {
    let output = mend_command_for(&temp.path().join("Cargo.toml"))
        .args(arguments)
        .arg("--json")
        .output()
        .expect("run the selected target analysis");
    assert!(
        output.status.success(),
        "target selection failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report = parse_mend_json_output(&output.stdout);
    let paths = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::OverbroadPubCrate)
        .map(|finding| finding.path.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(paths, expected.iter().copied().collect(), "{report:#?}");
}
