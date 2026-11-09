//! `--fix` for `pub_use_outside_subtree`: each test rewrites one fixture and
//! asserts the exact text of every file the fix touched, a re-check free of the
//! finding and of the import shortenings a moved path could leave, and a crate
//! that still builds.

use std::collections::BTreeMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;

use walkdir::WalkDir;

use super::pub_use_outside_subtree;
use crate::support::*;

/// Codes a fixed crate must no longer report: the finding itself, and the
/// import shortenings a rewritten caller path could leave behind.
const SETTLED_CODES: [DiagnosticCode; 3] = [
    DiagnosticCode::PubUseOutsideSubtree,
    DiagnosticCode::ShortenLocalCrateImport,
    DiagnosticCode::ReplaceDeepSuperImport,
];

/// Every `.rs` file of the fixture outside `target/`, keyed by its path
/// relative to `root`.
fn rust_sources(root: &Path) -> BTreeMap<PathBuf, String> {
    WalkDir::new(root)
        .into_iter()
        .filter_entry(|entry| entry.file_name() != "target")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "rs")
        })
        .map(|entry| {
            let relative = entry
                .path()
                .strip_prefix(root)
                .expect("fixture file under root")
                .to_path_buf();
            let contents = fs::read_to_string(entry.path()).expect("read fixture file");
            (relative, contents)
        })
        .collect()
}

/// Runs `--fix` on the fixture, asserts it succeeded, and returns its stderr.
fn run_fix(root: &Path) -> String {
    run_fix_command(mend_command_for(&root.join("Cargo.toml")).arg("--fix"))
}

fn run_fix_command(command: &mut std::process::Command) -> String {
    let output = run_fix_output(command);
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn run_fix_output(command: &mut std::process::Command) -> std::process::Output {
    let output = command.output().expect("run cargo-mend --fix");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "cargo-mend --fix failed:\n{}\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

/// Runs `--fix` and asserts that exactly the `expected` files changed, to
/// exactly the given text; then that a re-check reports none of
/// [`SETTLED_CODES`] and `cargo check --all-targets` passes.
fn assert_fixed(root: &Path, expected: &[(&str, &str)]) { fixed_check_stderr(root, expected); }

/// [`assert_fixed`], and `cargo check --all-targets` reports no warning.
fn assert_fixed_without_warnings(root: &Path, expected: &[(&str, &str)]) {
    let stderr = fixed_check_stderr(root, expected);
    assert!(
        !stderr.contains("warning"),
        "the fixed crate builds with warnings:\n{stderr}"
    );
}

/// The assertions of [`assert_fixed`]; returns the stderr of the passing
/// `cargo check --all-targets`.
fn fixed_check_stderr(root: &Path, expected: &[(&str, &str)]) -> String {
    fixed_check_stderr_after(root, expected, || {
        run_fix(root);
        run_mend_json(&root.join("Cargo.toml"))
    })
}

fn fixed_check_stderr_after(
    root: &Path,
    expected: &[(&str, &str)],
    fix: impl FnOnce() -> Report,
) -> String {
    let mut sources = rust_sources(root);
    for (relative, contents) in expected {
        sources.insert(PathBuf::from(relative), (*contents).to_string());
    }
    let report = fix();
    assert_eq!(rust_sources(root), sources, "fixed sources");
    let left = report
        .findings
        .iter()
        .filter(|finding| SETTLED_CODES.contains(&finding.code))
        .collect::<Vec<_>>();
    assert!(left.is_empty(), "the fixed crate still reports: {left:#?}");

    let check = cargo_command()
        .arg("check")
        .arg("--all-targets")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("run cargo check");
    let stderr = String::from_utf8_lossy(&check.stderr).into_owned();
    assert!(
        check.status.success(),
        "the fixed crate does not build:\n{stderr}"
    );
    stderr
}

/// A sibling's function re-exported under `tool_staging` for callers outside
/// `tool`: `tool/mod.rs` gets the re-export, the callers name `tool`, and the
/// staging file keeps only its own item.
#[test]
fn sibling_function_reexport_moves_to_the_parent_and_callers_follow() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         tool::tool_staging::build_device_recipe() + camera::run()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera.rs",
        "use crate::tool::tool_staging::build_device_recipe;\n\npub(crate) fn run() -> u32 {\n    \
         build_device_recipe() + crate::tool::tool_staging::stage_all()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::build_device_recipe;\n\npub(crate) fn stage_all() -> \
         u32 {\n    2\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    \
                 tool::build_device_recipe() + camera::run()\n}\n",
            ),
            (
                "src/camera.rs",
                "use crate::tool;\nuse crate::tool::tool_staging;\n\npub(crate) fn run() -> u32 \
                 {\n    tool::build_device_recipe() + tool_staging::stage_all()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\npub(crate) \
                 mod tool_staging;\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "pub(crate) fn stage_all() -> u32 {\n    2\n}\n",
            ),
        ],
    );
}

/// A child re-exporting its sibling's type for the parent's facade: the facade
/// names the sibling, and the child keeps a private `use` for its own body.
#[test]
fn child_reexport_behind_a_parent_facade_moves_to_the_parent() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod cascade;\n\npub fn touch() -> i32 {\n    cascade::run()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/cascade/mod.rs",
        "mod attributes;\nmod resolved;\n\npub(crate) use attributes::FontUnit;\n\npub(crate) fn \
         run() -> i32 {\n    FontUnit::default().0 + attributes::alpha()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/cascade/attributes.rs",
        "pub(crate) use super::resolved::FontUnit;\n\npub(super) fn alpha() -> i32 {\n    \
         FontUnit(5).0\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/cascade/resolved.rs",
        "#[derive(Default)]\npub(crate) struct FontUnit(pub(crate) i32);\n",
    );

    // The field narrows on the pass after the re-export moves.
    assert_fixed(
        root,
        &[
            (
                "src/cascade/mod.rs",
                "mod attributes;\nmod resolved;\n\npub(crate) use resolved::FontUnit;\n\n\
                 pub(crate) fn run() -> i32 {\n    FontUnit::default().0 + attributes::alpha()\n\
                 }\n",
            ),
            (
                "src/cascade/attributes.rs",
                "use super::resolved::FontUnit;\n\npub(super) fn alpha() -> i32 {\n    \
                 FontUnit(5).0\n}\n",
            ),
            (
                "src/cascade/resolved.rs",
                "#[derive(Default)]\npub(crate) struct FontUnit(pub(super) i32);\n",
            ),
        ],
    );
}

/// A module acting as a crate-wide hub for another module's type: the crate
/// root's re-export and the internal callers name the defining module.
#[test]
fn crate_hub_reexport_moves_to_the_crate_root() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod free_cam;\nmod input;\nmod stage;\n\npub use input::Orbit;\n\npub fn touch() -> u32 \
         {\n    stage::run()\n}\n",
    );
    pub_use_outside_subtree::write(root, "src/free_cam.rs", "pub struct Orbit;\n");
    pub_use_outside_subtree::write(
        root,
        "src/input/mod.rs",
        "pub use crate::free_cam::Orbit;\n\npub(crate) fn poll() -> u32 {\n    3\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/stage.rs",
        "use crate::input;\nuse crate::input::Orbit;\n\npub(crate) fn run() -> u32 {\n    let \
         _orbit = Orbit;\n    input::poll()\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod free_cam;\nmod input;\nmod stage;\n\npub use free_cam::Orbit;\n\npub fn \
                 touch() -> u32 {\n    stage::run()\n}\n",
            ),
            (
                "src/input/mod.rs",
                "pub(crate) fn poll() -> u32 {\n    3\n}\n",
            ),
            (
                "src/stage.rs",
                "use crate::input;\nuse crate::free_cam::Orbit;\n\npub(crate) fn run() -> u32 \
                 {\n    let _orbit = Orbit;\n    input::poll()\n}\n",
            ),
        ],
    );
}

/// A re-export no caller names is deleted with the blank line after it.
#[test]
fn unused_reexport_is_deleted() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run() + \
         platform::discover()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera.rs",
        "pub(crate) use crate::platform::discover;\n\npub(crate) fn run() -> i32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    assert_fixed(
        root,
        &[("src/camera.rs", "pub(crate) fn run() -> i32 {\n    1\n}\n")],
    );
}

/// A grouped `use` keeps the name inside its own subtree and loses only the
/// flagged one.
#[test]
fn grouped_reexport_loses_only_the_flagged_name() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run() + \
         camera::discover()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera.rs",
        "mod lens;\n\npub(crate) use crate::{camera::lens::focus, platform::discover};\n\n\
         pub(crate) fn run() -> i32 {\n    focus()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera/lens.rs",
        "pub(crate) fn focus() -> i32 {\n    2\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run() + \
                 platform::discover()\n}\n",
            ),
            (
                "src/camera.rs",
                "mod lens;\n\npub(crate) use lens::focus;\n\npub(crate) fn run() -> i32 {\n    \
                 focus()\n}\n",
            ),
        ],
    );
}

/// The `#[cfg]` on a moved re-export is copied onto the new one, which lands
/// after the parent's last `use`.
#[test]
fn cfg_reexport_carries_its_cfg_to_the_new_reexport() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n\n#[cfg(unix)]\npub fn touch() -> u32 {\n    \
         tool::tool_staging::build_device_recipe() + tool::tool_staging::stage_all() + \
         tool::run()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\nuse reset_scene::reset_all;\n\n\
         pub(super) fn run() -> u32 {\n    reset_all()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n\npub(super) fn reset_all() -> \
         u32 {\n    0\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "#[cfg(unix)]\npub(crate) use super::reset_scene::build_device_recipe;\n\npub(crate) fn \
         stage_all() -> u32 {\n    2\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod tool;\n\n#[cfg(unix)]\npub fn touch() -> u32 {\n    \
                 tool::build_device_recipe() + tool::tool_staging::stage_all() + tool::run()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "mod reset_scene;\npub(crate) mod tool_staging;\n\nuse reset_scene::reset_all;\n\
                 #[cfg(unix)]\npub(crate) use reset_scene::build_device_recipe;\n\npub(super) fn \
                 run() -> u32 {\n    reset_all()\n}\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "pub(crate) fn stage_all() -> u32 {\n    2\n}\n",
            ),
        ],
    );
}

/// Callers in a `#[path]`-mounted file outside `src/` and in an
/// `include!`-mounted file are rewritten like any other caller.
#[test]
fn callers_in_path_mounted_and_included_files_are_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n#[path = \"../extra/probe.rs\"]\nmod probe;\n\n\
         include!(\"../extra/included.rs\");\n\npub fn touch() -> u32 {\n    probe::run() + \
         included() + tool::tool_staging::stage_all()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "extra/probe.rs",
        "use crate::tool::tool_staging::Recipe;\n\npub(crate) fn run() -> u32 {\n    Recipe(1).0\n\
         }\n",
    );
    pub_use_outside_subtree::write(
        root,
        "extra/included.rs",
        "fn included() -> u32 {\n    let tool::tool_staging::Recipe(value) = \
         tool::tool_staging::Recipe(3);\n    value\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) struct Recipe(pub(crate) u32);\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::Recipe;\n\npub(crate) fn stage_all() -> u32 {\n    2\n\
         }\n",
    );

    assert_fixed(
        root,
        &[
            (
                "extra/probe.rs",
                "use crate::tool::Recipe;\n\npub(crate) fn run() -> u32 {\n    Recipe(1).0\n}\n",
            ),
            (
                "extra/included.rs",
                "fn included() -> u32 {\n    let tool::Recipe(value) = tool::Recipe(3);\n    \
                 value\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::Recipe;\nmod reset_scene;\npub(crate) mod \
                 tool_staging;\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "pub(crate) fn stage_all() -> u32 {\n    2\n}\n",
            ),
        ],
    );
}

/// A refused caller import keeps its whole fact while another fact sharing
/// the same owner region still moves and compiles.
#[test]
fn included_import_refusal_keeps_its_reexport_and_applies_independent_fact() {
    let temp = tempdir().expect("create included import fixture");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        concat!(
            "mod tool;\ninclude!(\"../extra/included.rs\");\n",
            "pub fn touch() -> u32 { included() + tool::tool_staging::independent() }\n",
        ),
    );
    let included = concat!(
        "use crate::tool::tool_staging::Recipe;\n",
        "fn included() -> u32 { Recipe(3).0 }\n",
    );
    pub_use_outside_subtree::write(root, "extra/included.rs", included);
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        concat!(
            "pub(crate) struct Recipe(pub(crate) u32);\n",
            "pub(crate) fn independent() -> u32 { 7 }\n",
        ),
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::{Recipe, independent};\n",
    );

    let before = rust_sources(root);
    let preview =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["--fix", "--dry-run"]));
    assert_eq!(rust_sources(root), before, "dry run changed the fixture");
    assert_eq!(
        preview
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{preview}",
    );
    assert!(preview.contains("would apply"), "{preview}");
    let stderr = run_fix(root);
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{stderr}",
    );
    assert!(!stderr.contains("rolled back"), "{stderr}");
    assert_eq!(
        fs::read_to_string(root.join("extra/included.rs")).expect("read kept caller"),
        included,
    );
    let owner = fs::read_to_string(root.join("src/tool/tool_staging/mod.rs"))
        .expect("read partially rewritten shared owner");
    assert!(owner.contains("Recipe"), "{owner}");
    assert!(!owner.contains("independent"), "{owner}");
    let ancestor = fs::read_to_string(root.join("src/tool/mod.rs")).expect("read ancestor");
    assert!(
        ancestor.contains("use reset_scene::independent;"),
        "{ancestor}"
    );
    assert!(!ancestor.contains("Recipe"), "{ancestor}");
    let caller = fs::read_to_string(root.join("src/lib.rs")).expect("read accepted caller");
    assert!(caller.contains("tool::independent()"), "{caller}");
    assert!(!caller.contains("tool_staging::independent"), "{caller}");
    assert_workspace_builds(root);
}

/// An inline module left with nothing once its re-export moves is deleted
/// with its doc comment, and a caller inside the parent names the sibling.
#[test]
fn emptied_inline_owner_module_is_deleted() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub fn touch() -> u32 {\n    tool::staging::build_device_recipe() + \
         tool::run()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\n/// Hands staging callers the scene recipe.\npub(crate) mod staging \
         {\n    pub(crate) use super::reset_scene::build_device_recipe;\n}\n\npub(crate) fn run() \
         -> u32 {\n    staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod tool;\n\npub fn touch() -> u32 {\n    tool::build_device_recipe() + \
                 tool::run()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\n\npub(crate) \
                 fn run() -> u32 {\n    reset_scene::build_device_recipe()\n}\n",
            ),
        ],
    );
}

/// An emptied inline module that a `pub(crate) use` elsewhere still names
/// stays as an empty module, so that `use` keeps resolving.
#[test]
fn emptied_inline_owner_module_stays_while_a_pub_use_names_it() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    let config_path = root.join("mend.toml");
    let config = fs::read_to_string(&config_path).expect("read fixture mend.toml");
    fs::write(
        config_path,
        format!("{config}allow_pub_mod = [\"src/tool/mod.rs\"]\n"),
    )
    .expect("write fixture mend.toml");
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub(crate) use tool::staging;\n\npub fn touch() -> u32 {\n    \
         staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\npub(crate) mod staging {\n    pub(crate) use \
         super::reset_scene::build_device_recipe;\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod tool;\n\npub(crate) use tool::staging;\n\npub fn touch() -> u32 {\n    \
                 tool::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\n\npub(crate) \
                 mod staging {\n}\n",
            ),
        ],
    );
}

/// A caller that imports the owner module and calls through it loses that
/// import once its only path moves to the parent.
#[test]
fn module_import_of_the_owner_is_removed_once_its_callers_move() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    camera::run()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera.rs",
        "use crate::tool::tool_staging;\n\npub(crate) fn run() -> u32 {\n    \
         tool_staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::build_device_recipe;\n",
    );

    // `prefer_module_import` then imports `tool` for the moved call; the
    // emptied staging file and its now private `mod` line stay.
    assert_fixed(
        root,
        &[
            (
                "src/camera.rs",
                "use crate::tool;\n\npub(crate) fn run() -> u32 {\n    \
                 tool::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\nmod \
                 tool_staging;\n",
            ),
            ("src/tool/tool_staging/mod.rs", ""),
        ],
    );
}

/// A caller path written inside a macro call is rewritten in place.
#[test]
fn caller_inside_a_macro_call_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub fn touch() {\n    assert_eq!(tool::tool_staging::build_device_recipe(), \
         1);\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::build_device_recipe;\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod tool;\n\npub fn touch() {\n    assert_eq!(tool::build_device_recipe(), 1);\n\
                 }\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\nmod \
                 tool_staging;\n",
            ),
            ("src/tool/tool_staging/mod.rs", ""),
        ],
    );
}

/// A re-export of a module that itself holds a flagged re-export: the outer
/// one moves first and the inner one on the next pass, so no caller is sent to
/// a path the same pass empties. `prefer_module_import` then imports `tool`
/// for the moved call.
#[test]
fn reexport_of_another_owner_module_is_fixed_on_a_later_pass() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    camera::run() + \
         camera::staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/camera.rs",
        "pub(super) use crate::tool::staging;\n\npub(super) fn run() -> u32 {\n    \
         staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\npub(crate) mod staging {\n    pub(crate) use \
         super::reset_scene::build_device_recipe;\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/lib.rs",
                "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    camera::run() + \
                 tool::build_device_recipe()\n}\n",
            ),
            (
                "src/camera.rs",
                "use crate::tool;\n\npub(super) fn run() -> u32 {\n    \
                 tool::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\n",
            ),
        ],
    );
}

/// Four private forwarding links converge in one validated batch.
/// The crate root keeps the direct path to the defining module.
#[test]
fn reexport_chain_collapses_in_one_run() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[package]\nname = \"chain_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\ntest = false\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod input;\nmod orbit_cam;\n\npub use orbit_cam::Gain;\n",
    );
    pub_use_outside_subtree::write(root, "src/input.rs", "pub struct Gain(pub u32);\n");
    pub_use_outside_subtree::write(
        root,
        "src/orbit_cam/mod.rs",
        "mod controls;\n\npub use controls::Gain;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/orbit_cam/controls/mod.rs",
        "mod bindings;\n\npub use bindings::Gain;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/orbit_cam/controls/bindings/mod.rs",
        "mod preset;\n\npub use preset::Gain;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/orbit_cam/controls/bindings/preset.rs",
        "pub use crate::input::Gain;\n",
    );

    fixed_check_stderr_after(
        root,
        &[
            (
                "src/lib.rs",
                "mod input;\nmod orbit_cam;\n\npub use input::Gain;\n",
            ),
            ("src/orbit_cam/mod.rs", "mod controls;\n"),
            ("src/orbit_cam/controls/mod.rs", "mod bindings;\n"),
            ("src/orbit_cam/controls/bindings/mod.rs", "mod preset;\n"),
            ("src/orbit_cam/controls/bindings/preset.rs", ""),
        ],
        || fix_chain_with_check_budget(root),
    );
}

/// A macro's module argument prevents one path rewrite from reaching its use.
#[test]
fn failed_chain_link_keeps_the_other_validated_fixes() {
    let temp = tempdir().expect("create macro chain fixture");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[package]\nname = \"macro_chain_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\ntest = false\n",
    );
    let sources = [
        (
            "src/lib.rs",
            "mod input;\nmod orbit_cam;\n\npub use orbit_cam::Gain;\n\nmacro_rules! read_gain { ($module:ident) => { crate::$module::Gain(7).0 }; }\npub fn read() -> u32 { read_gain!(orbit_cam) }\n",
        ),
        ("src/input.rs", "pub struct Gain(pub u32);\n"),
        (
            "src/orbit_cam/mod.rs",
            "mod controls;\n\npub use controls::Gain;\n",
        ),
        ("src/orbit_cam/controls.rs", "pub use crate::input::Gain;\n"),
    ];
    for (path, source) in sources {
        pub_use_outside_subtree::write(root, path, source);
    }
    let stderr = run_fix(root);
    assert!(
        stderr.contains("error[E0425]"),
        "the batch must fail a real check: {stderr}"
    );
    assert!(stderr.contains("chain batch did not validate"), "{stderr}");
    assert!(
        stderr.contains("retrying remaining links as one batch"),
        "{stderr}"
    );
    assert!(!stderr.contains("retrying individual links"), "{stderr}");
    assert!(stderr.contains("chain link did not validate"), "{stderr}");
    assert!(!stderr.contains("rolled back"), "{stderr}");
    let expected = sources
        .into_iter()
        .map(|(path, source)| {
            let fixed = match path {
                "src/orbit_cam/mod.rs" => "mod controls;\n\npub use crate::input::Gain;\n",
                "src/orbit_cam/controls.rs" => "",
                _ => source,
            };
            (PathBuf::from(path), fixed.to_string())
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        rust_sources(root),
        expected,
        "only the invalid link must be restored"
    );
}

/// A failed chain does not prevent an independent chain from collapsing.
#[test]
fn failed_chain_link_keeps_an_independent_chain() {
    let temp = tempdir().expect("create independent chains fixture");
    let root = temp.path();
    let sources = [
        (
            "src/lib.rs",
            "mod input;\nmod orbit_cam;\nmod output;\nmod meter;\n\npub use orbit_cam::Gain;\npub use meter::Scale;\n\nmacro_rules! read_gain { ($module:ident) => { crate::$module::Gain(7).0 }; }\npub fn read() -> u32 { read_gain!(orbit_cam) }\n",
        ),
        ("src/input.rs", "pub struct Gain(pub u32);\n"),
        (
            "src/orbit_cam/mod.rs",
            "mod controls;\n\npub use controls::Gain;\n",
        ),
        ("src/orbit_cam/controls.rs", "pub use crate::input::Gain;\n"),
        ("src/output.rs", "pub struct Scale(pub u32);\n"),
        (
            "src/meter/mod.rs",
            "mod controls;\n\npub use controls::Scale;\n",
        ),
        ("src/meter/controls.rs", "pub use crate::output::Scale;\n"),
    ];
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[package]\nname = \"independent_chains_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[lib]\ntest = false\n",
    );
    for (path, source) in sources {
        pub_use_outside_subtree::write(root, path, source);
    }
    let stderr = run_fix(root);
    assert!(stderr.contains("error[E0425]"), "{stderr}");
    assert!(stderr.contains("chain batch did not validate"), "{stderr}");
    assert!(stderr.contains("retrying individual links"), "{stderr}");
    let expected = sources
        .into_iter()
        .map(|(path, source)| {
            let fixed = match path {
                "src/lib.rs" => "mod input;\nmod orbit_cam;\nmod output;\nmod meter;\n\npub use orbit_cam::Gain;\npub use output::Scale;\n\nmacro_rules! read_gain { ($module:ident) => { crate::$module::Gain(7).0 }; }\npub fn read() -> u32 { read_gain!(orbit_cam) }\n",
                "src/orbit_cam/mod.rs" => "mod controls;\n\npub use crate::input::Gain;\n",
                "src/meter/mod.rs" => "mod controls;\n",
                "src/orbit_cam/controls.rs" | "src/meter/controls.rs" => "",
                _ => source,
            };
            (PathBuf::from(path), fixed.to_string())
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        rust_sources(root),
        expected,
        "the valid chain must collapse"
    );
}

/// Count the initial check and the batch validation for four forwarding links.
fn fix_chain_with_check_budget(root: &Path) -> Report {
    let mut command = mend_command_for(&root.join("Cargo.toml"));
    command.args(["--fix", "--json"]);
    #[cfg(unix)]
    {
        let tools = tempdir().expect("create check counter directory");
        let cargo = tools.path().join("cargo");
        let count = tools.path().join("checks");
        let metadata_count = tools.path().join("metadata");
        fs::write(&cargo, "#!/bin/sh\nif [ \"$1\" = check ]; then printf x >> \"$MEND_TEST_CHECK_STARTS\"; fi\nif [ \"$1\" = metadata ]; then printf x >> \"$MEND_TEST_METADATA_STARTS\"; fi\nexec \"$MEND_TEST_REAL_CARGO\" \"$@\"\n")
            .expect("write counting cargo entry");
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o700))
            .expect("make counting cargo entry executable");
        let inherited_path = std::env::var_os("PATH").expect("test PATH");
        let path = std::env::join_paths(
            std::iter::once(tools.path().to_path_buf())
                .chain(std::env::split_paths(&inherited_path)),
        )
        .expect("prepend counting cargo entry");
        command
            .env("PATH", path)
            .env("CARGO", &cargo)
            .env("MEND_TEST_REAL_CARGO", env!("CARGO"))
            .env("MEND_TEST_CHECK_STARTS", &count)
            .env("MEND_TEST_METADATA_STARTS", &metadata_count);
        let output = run_fix_output(&mut command);
        let starts = fs::read(&count)
            .expect("read real cargo check starts")
            .len();
        assert!(
            starts <= 2,
            "one chain batch needs at most two checks, observed {starts}"
        );
        let metadata_starts = fs::read(&metadata_count)
            .expect("read workspace metadata starts")
            .len();
        assert_eq!(
            metadata_starts, 1,
            "the workspace inventory must be resolved once"
        );
        report_from_mend_output(&output)
    }
    #[cfg(not(unix))]
    report_from_mend_output(&run_fix_output(&mut command))
}

/// A caller in another file reaching the owner module through a `use ... as`
/// alias of it is redirected like a caller naming the owner module directly.
#[test]
fn caller_through_a_module_alias_in_another_file_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod frames;\nmod platform;\nmod screen;\n\npub fn touch() -> u32 {\n    \
         screen::show()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/frames.rs",
        "pub(crate) struct ScreenFrame(pub(crate) u32);\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/platform/mod.rs",
        "pub(crate) mod screen_capture_stream;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/platform/screen_capture_stream.rs",
        "pub(crate) use crate::frames::ScreenFrame;\n\npub(crate) fn open() -> u32 {\n    \
         ScreenFrame(1).0\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/screen/mod.rs",
        "use crate::platform::screen_capture_stream as capture_stream;\n\nmod view;\n\n\
         pub(crate) fn show() -> u32 {\n    view::render() + capture_stream::open()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/screen/view.rs",
        "use crate::screen::capture_stream::ScreenFrame;\n\npub(super) fn render() -> u32 {\n    \
         ScreenFrame(2).0\n}\n",
    );

    assert_fixed(
        root,
        &[
            (
                "src/platform/screen_capture_stream.rs",
                "use crate::frames::ScreenFrame;\n\npub(crate) fn open() -> u32 {\n    \
                 ScreenFrame(1).0\n}\n",
            ),
            (
                "src/screen/view.rs",
                "use crate::frames::ScreenFrame;\n\npub(super) fn render() -> u32 {\n    \
                 ScreenFrame(2).0\n}\n",
            ),
        ],
    );
}

/// A `mod tests { use super::*; }` caller that reaches the owner module
/// through its parent's `use crate::tool;` is redirected with the other
/// callers, and the ancestor re-export only that test caller names is written
/// under `#[cfg(test)]`, so a non-test build has no unused import.
#[test]
fn test_module_caller_through_a_glob_of_a_module_import_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod application_readiness;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         application_readiness::ready()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/application_readiness.rs",
        "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
         #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn builds_the_recipe() \
         {\n        assert_eq!(tool::tool_staging::build_device_recipe() + ready(), 4);\n    }\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\npub(crate) fn stage() -> u32 {\n    \
         tool_staging::stage_all() + tool_staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::build_device_recipe;\n\npub(crate) fn stage_all() -> \
         u32 {\n    2\n}\n",
    );

    assert_fixed_without_warnings(
        root,
        &[
            (
                "src/application_readiness.rs",
                "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
                 #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn \
                 builds_the_recipe() {\n        assert_eq!(tool::build_device_recipe() + ready(), \
                 4);\n    }\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "#[cfg(test)]\npub(crate) use reset_scene::build_device_recipe;\nmod \
                 reset_scene;\nmod tool_staging;\n\npub(crate) fn stage() -> u32 {\n    tool_staging::stage_all() \
                 + reset_scene::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "pub(super) fn stage_all() -> u32 {\n    2\n}\n",
            ),
        ],
    );
}

/// A `mod tests { use super::*; }` whose only use of the glob is a path
/// through the parent's `use crate::tool;` keeps that spelling, so the glob
/// stays used and the crate builds with no warning.
#[test]
fn test_module_caller_keeps_its_glob_used() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod application_readiness;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         application_readiness::ready() + tool::tool_staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/application_readiness.rs",
        "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
         #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn builds_the_recipe() \
         {\n        assert_eq!(tool::tool_staging::build_device_recipe(), 1);\n    }\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\npub(crate) fn stage() -> u32 {\n    \
         tool_staging::stage_all() + tool_staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::build_device_recipe;\n\npub(crate) fn stage_all() -> \
         u32 {\n    2\n}\n",
    );

    assert_fixed_without_warnings(
        root,
        &[
            (
                "src/lib.rs",
                "mod application_readiness;\nmod tool;\n\npub fn touch() -> u32 {\n    \
                 application_readiness::ready() + tool::build_device_recipe()\n}\n",
            ),
            (
                "src/application_readiness.rs",
                "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
                 #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn \
                 builds_the_recipe() {\n        assert_eq!(tool::build_device_recipe(), 1);\n    \
                 }\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\nmod reset_scene;\nmod \
                 tool_staging;\n\npub(crate) fn stage() -> u32 {\n    tool_staging::stage_all() \
                 + reset_scene::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "pub(super) fn stage_all() -> u32 {\n    2\n}\n",
            ),
        ],
    );
}

/// Re-exports the owner's `#[cfg(test)] mod tests;`, in its own file, still
/// reaches through `use super::*;`: the owner keeps a private `use` of each
/// under `#[cfg(test)]`, so a non-test build has no unused import. The
/// function's `use` is not rewritten to a module import, which would leave
/// the bare call in the test file unresolved.
#[test]
fn file_test_module_reaching_the_name_through_a_glob_keeps_a_cfg_test_use() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod tool;\n\nuse tool::tool_staging::DeviceRecipe;\n\npub fn touch() -> u32 {\n    \
         DeviceRecipe(1).0 + tool::tool_staging::build_device_recipe()\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) struct DeviceRecipe(pub(crate) u32);\n\npub(crate) fn build_device_recipe() \
         -> u32 {\n    1\n}\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::DeviceRecipe;\npub(crate) use \
         super::reset_scene::build_device_recipe;\n\n#[cfg(test)]\nmod tests;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/tool/tool_staging/tests.rs",
        "use super::*;\n\n#[test]\nfn builds_the_recipe() {\n    \
         assert_eq!(DeviceRecipe(build_device_recipe()).0, 1);\n}\n",
    );

    assert_fixed_without_warnings(
        root,
        &[
            (
                "src/lib.rs",
                "mod tool;\n\nuse tool::DeviceRecipe;\n\npub fn touch() -> u32 {\n    \
                 DeviceRecipe(1).0 + tool::build_device_recipe()\n}\n",
            ),
            (
                "src/tool/mod.rs",
                "pub(crate) use reset_scene::build_device_recipe;\npub(crate) use \
                 reset_scene::DeviceRecipe;\nmod reset_scene;\npub(crate) mod tool_staging;\n",
            ),
            (
                "src/tool/tool_staging/mod.rs",
                "#[cfg(test)]\nuse super::reset_scene::DeviceRecipe;\n#[cfg(test)]\nuse \
                 super::reset_scene::build_device_recipe;\n\n#[cfg(test)]\nmod tests;\n",
            ),
        ],
    );
}

/// A file mounted by two parents holds one re-export seen from two modules;
/// `--fix` cannot rewrite it for both, so it names the findings in its skipped
/// notice, leaves the source alone, and the findings stay warnings.
#[test]
fn unrewritable_findings_are_counted_in_the_skipped_notice() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    pub_use_outside_subtree::write_manifest(root);
    pub_use_outside_subtree::write(
        root,
        "src/lib.rs",
        "mod first;\nmod platform;\nmod second;\n\npub fn touch() -> i32 {\n    first::run() + \
         second::run()\n}\n",
    );
    for parent in ["src/first.rs", "src/second.rs"] {
        pub_use_outside_subtree::write(
            root,
            parent,
            "#[path = \"shared.rs\"]\nmod shared;\n\npub(super) fn run() -> i32 {\n    \
             shared::discover()\n}\n",
        );
    }
    pub_use_outside_subtree::write(
        root,
        "src/shared.rs",
        "pub(super) use crate::platform::discover;\n",
    );
    pub_use_outside_subtree::write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );
    let sources = rust_sources(root);

    let stderr = run_fix(root);
    assert!(
        stderr.contains("skipped 1 `pub_use_outside_subtree` finding(s) `--fix` could not rewrite"),
        "the notice must count the skipped findings:\n{stderr}"
    );
    assert_eq!(rust_sources(root), sources, "no source may change");

    let report = run_mend_json(&root.join("Cargo.toml"));
    let flagged = report
        .findings
        .iter()
        .filter(|finding| finding.code == DiagnosticCode::PubUseOutsideSubtree)
        .count();
    assert_eq!(flagged, 2, "full report: {report:#?}");
    assert_eq!(
        report.summary.errors, 0,
        "a skipped finding stays a warning: {report:#?}"
    );
}

/// A second package mounts the same re-export as its public prelude. Only the
/// first package has a sideways facade and redirectable callers.
#[test]
fn shared_reexport_needed_by_another_package_stays_manual() {
    let temp = tempdir().expect("create shared re-export workspace");
    let root = temp.path();
    // The second package intentionally exposes its platform module. Review it
    // here so this fixture isolates preservation of the shared re-export.
    pub_use_outside_subtree::write(
        root,
        "mend.toml",
        "[visibility]\npub_in_path = \"permitted\"\nallow_pub_mod = [\"b/src/lib.rs\"]\n",
    );
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
    );
    for name in ["a", "b"] {
        pub_use_outside_subtree::write(
            root,
            &format!("{name}/Cargo.toml"),
            &format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        );
        pub_use_outside_subtree::write(
            root,
            &format!("{name}/src/platform.rs"),
            "pub fn discover() -> u8 { 7 }\n",
        );
    }
    let shared = "pub use crate::platform::discover;\n";
    pub_use_outside_subtree::write(root, "shared.rs", shared);
    pub_use_outside_subtree::write(
        root,
        "a/src/lib.rs",
        concat!(
            "mod platform;\n#[path = \"../../shared.rs\"] mod bridge;\n",
            "pub fn run() -> u8 { bridge::discover() }\n",
        ),
    );
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        concat!(
            "pub mod platform;\n#[path = \"../../shared.rs\"] pub mod prelude;\n",
            "pub fn run() -> u8 { prelude::discover() }\n",
        ),
    );

    let stderr = run_fix_command(mend_command_for(&root.join("Cargo.toml")).args([
        "--workspace",
        "--all-targets",
        "--fix",
    ]));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read shared source"),
        shared
    );
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "the kept re-export is reported once: {stderr}",
    );
    let check = cargo_command()
        .args(["check", "--workspace", "--all-targets", "--quiet"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("check both packages after mend");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

/// Both packages mount the same re-export under ordinary module names. One
/// package can additionally need a private import for a descendant glob user.
#[derive(Clone, Copy)]
enum SharedReexportUse {
    SameRewrite,
    DescendantGlobInFirstPackage,
}

fn shared_reexport_workspace(callers: SharedReexportUse) -> tempfile::TempDir {
    let temp = tempdir().expect("create two-package re-export workspace");
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "mend.toml",
        "[visibility]\npub_in_path = \"permitted\"\n",
    );
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
    );
    for (package, owner) in [("a", "bridge"), ("b", "gateway")] {
        pub_use_outside_subtree::write(
            root,
            &format!("{package}/Cargo.toml"),
            &format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        );
        pub_use_outside_subtree::write(
            root,
            &format!("{package}/src/platform.rs"),
            "pub fn discover() -> u8 { 7 }\n",
        );
        let (declaration, local_call) = match callers {
            SharedReexportUse::SameRewrite => (
                format!("#[path = \"../../shared.rs\"] mod {owner};"),
                String::new(),
            ),
            SharedReexportUse::DescendantGlobInFirstPackage => {
                let local = if package == "a" {
                    concat!(
                        "mod callers { use super::*; pub(super) fn local() -> u8 { discover() } }\n",
                        "pub fn local() -> u8 { callers::local() }\n",
                    )
                } else {
                    ""
                };
                pub_use_outside_subtree::write(
                    root,
                    &format!("{package}/src/{owner}.rs"),
                    &format!("include!(\"../../shared.rs\");\n{local}"),
                );
                let call = if package == "a" {
                    format!(" + {owner}::local()")
                } else {
                    String::new()
                };
                (format!("mod {owner};"), call)
            },
        };
        pub_use_outside_subtree::write(
            root,
            &format!("{package}/src/lib.rs"),
            &format!(
                "mod platform;\n{declaration}\npub fn run() -> u8 {{ {owner}::discover(){local_call} }}\n"
            ),
        );
    }
    pub_use_outside_subtree::write(root, "shared.rs", "pub use crate::platform::discover;\n");
    temp
}

fn assert_workspace_builds(root: &Path) {
    let check = cargo_command()
        .args(["check", "--workspace", "--all-targets", "--quiet"])
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .output()
        .expect("check both packages");
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn package_filtered_local_reexport_is_rewritten() {
    let temp = tempdir().expect("create package-local re-export workspace");
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        "[workspace]\nmembers = [\"a\", \"b\"]\nresolver = \"2\"\n",
    );
    pub_use_outside_subtree::write(
        root,
        "mend.toml",
        "[visibility]\npub_in_path = \"permitted\"\n",
    );
    for package in ["a", "b"] {
        pub_use_outside_subtree::write(
            root,
            &format!("{package}/Cargo.toml"),
            &format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        );
    }
    pub_use_outside_subtree::write(
        root,
        "a/src/lib.rs",
        "mod platform;\nmod bridge;\npub fn run() -> u8 { bridge::discover() }\n",
    );
    pub_use_outside_subtree::write(root, "a/src/platform.rs", "pub fn discover() -> u8 { 7 }\n");
    pub_use_outside_subtree::write(
        root,
        "a/src/bridge.rs",
        "pub use crate::platform::discover;\n",
    );
    pub_use_outside_subtree::write(root, "b/src/lib.rs", "pub fn independent() -> u8 { 9 }\n");
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["-p", "a", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("a/src/bridge.rs")).expect("read rewritten owner"),
        ""
    );
    assert_eq!(
        fs::read_to_string(root.join("a/src/lib.rs")).expect("read redirected caller"),
        "mod platform;\nmod bridge;\npub fn run() -> u8 { platform::discover() }\n"
    );
    assert!(!stderr.contains("skipped"), "{stderr}");
    assert_workspace_builds(root);
}

#[test]
fn package_filtered_shared_reexport_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    let shared = fs::read_to_string(root.join("shared.rs")).expect("read shared source");
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["-p", "a", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read kept source"),
        shared
    );
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{stderr}"
    );
    assert_workspace_builds(root);
}

/// Qualified includes in an unselected package are still possible mounts.
#[test]
fn package_filtered_reexport_used_through_qualified_include_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        concat!(
            "mod platform;\nmod gateway { std::include!(\"../../shared.rs\"); }\n",
            "pub fn run() -> u8 { gateway::discover() }\n",
        ),
    );
    assert_filtered_reexport_stays_manual(root);
}

#[test]
fn package_filtered_reexport_used_through_raw_include_stays_manual() {
    for include in ["r#include", "std::r#include"] {
        let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
        let root = temp.path();
        pub_use_outside_subtree::write(
            root,
            "b/src/lib.rs",
            &format!(
                "mod platform;\nmod gateway {{ {include}!(\"../../shared.rs\"); }}\n\
                 pub fn run() -> u8 {{ gateway::discover() }}\n"
            ),
        );
        assert_filtered_reexport_stays_manual(root);
    }
}

#[test]
fn package_filtered_reexport_used_through_raw_path_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        concat!(
            "mod platform;\n#[r#path = \"../../shared.rs\"] mod gateway;\n",
            "pub fn run() -> u8 { gateway::discover() }\n",
        ),
    );
    assert_filtered_reexport_stays_manual(root);
}

#[test]
fn package_filtered_reexport_used_through_raw_cfg_attr_path_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        concat!(
            "mod platform;\n#[r#cfg_attr(all(), r#path = \"../../shared.rs\")] mod gateway;\n",
            "pub fn run() -> u8 { gateway::discover() }\n",
        ),
    );
    assert_filtered_reexport_stays_manual(root);
}

#[test]
fn package_filtered_reexport_with_nonliteral_raw_include_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        concat!(
            "mod platform;\nmod gateway { r#include!(concat!(\"../../\", \"shared.rs\")); }\n",
            "pub fn run() -> u8 { gateway::discover() }\n",
        ),
    );
    assert_filtered_reexport_stays_manual(root);
}

/// A function-local module cannot be placed in the crate module inventory;
/// the unselected package must conservatively veto the shared edit.
#[test]
fn package_filtered_reexport_used_by_block_local_module_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "b/src/lib.rs",
        &format!(
            "mod platform;\npub fn run() -> u8 {{ #[path = {:?}] mod gateway; gateway::discover() }}\n",
            root.join("shared.rs"),
        ),
    );
    assert_filtered_reexport_stays_manual(root);
}

fn assert_filtered_reexport_stays_manual(root: &Path) {
    let shared = fs::read_to_string(root.join("shared.rs")).expect("read shared source");
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["-p", "a", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read kept source"),
        shared
    );
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{stderr}"
    );
    assert_workspace_builds(root);
}

/// Directory nesting alone is not a mount from the unselected root package.
#[test]
fn package_filtered_nested_member_local_reexport_is_rewritten() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    pub_use_outside_subtree::write(
        root,
        "Cargo.toml",
        concat!(
            "[package]\nname = \"b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            "[workspace]\nmembers = [\"a\"]\nresolver = \"2\"\n",
        ),
    );
    pub_use_outside_subtree::write(root, "src/lib.rs", "pub fn independent() -> u8 { 9 }\n");
    pub_use_outside_subtree::write(
        root,
        "a/src/lib.rs",
        "mod platform;\nmod bridge;\npub fn run() -> u8 { bridge::discover() }\n",
    );
    pub_use_outside_subtree::write(
        root,
        "a/src/bridge.rs",
        "pub use crate::platform::discover;\n",
    );
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["-p", "a", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("a/src/bridge.rs")).expect("read rewritten owner"),
        ""
    );
    assert_eq!(
        fs::read_to_string(root.join("a/src/lib.rs")).expect("read redirected caller"),
        "mod platform;\nmod bridge;\npub fn run() -> u8 { platform::discover() }\n",
    );
    assert!(!stderr.contains("skipped"), "{stderr}");
    assert_workspace_builds(root);
}

#[test]
fn package_manifest_shared_reexport_stays_manual() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    let shared = fs::read_to_string(root.join("shared.rs")).expect("read shared source");
    let stderr = run_fix_command(mend_command_for(&root.join("a/Cargo.toml")).arg("--fix"));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read kept source"),
        shared
    );
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{stderr}"
    );
    assert_workspace_builds(root);
}

#[test]
fn shared_reexport_with_agreeing_packages_is_rewritten() {
    let temp = shared_reexport_workspace(SharedReexportUse::SameRewrite);
    let root = temp.path();
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["--workspace", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read rewritten source"),
        ""
    );
    assert!(!stderr.contains("skipped"), "{stderr}");
    for (package, owner) in [("a", "bridge"), ("b", "gateway")] {
        let caller = fs::read_to_string(root.join(format!("{package}/src/lib.rs")))
            .expect("read redirected caller");
        assert!(
            !caller.contains(&format!("{owner}::discover()")),
            "{caller}"
        );
    }
    assert_workspace_builds(root);
}

#[test]
fn shared_reexport_with_different_mount_edits_is_reported_once() {
    let temp = shared_reexport_workspace(SharedReexportUse::DescendantGlobInFirstPackage);
    let root = temp.path();
    let shared = fs::read_to_string(root.join("shared.rs")).expect("read shared source");
    let stderr =
        run_fix_command(mend_command_for(&root.join("Cargo.toml")).args(["--workspace", "--fix"]));
    assert_eq!(
        fs::read_to_string(root.join("shared.rs")).expect("read kept source"),
        shared
    );
    assert_eq!(
        stderr
            .matches("skipped 1 `pub_use_outside_subtree`")
            .count(),
        1,
        "{stderr}"
    );
    assert_workspace_builds(root);
}
