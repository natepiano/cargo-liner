//! `--fix` for `pub_use_outside_subtree`: each test rewrites one fixture and
//! asserts the exact text of every file the fix touched, a re-check free of the
//! finding and of the import shortenings a moved path could leave, and a crate
//! that still builds.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use walkdir::WalkDir;

use super::pub_use_outside_subtree::write;
use super::pub_use_outside_subtree::write_manifest;
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
    let output = mend_command()
        .arg("--manifest-path")
        .arg(root.join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "cargo-mend --fix failed:\n{}\n{stderr}",
        String::from_utf8_lossy(&output.stdout)
    );
    stderr
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
    let mut sources = rust_sources(root);
    for (relative, contents) in expected {
        sources.insert(PathBuf::from(relative), (*contents).to_string());
    }
    run_fix(root);
    assert_eq!(rust_sources(root), sources, "fixed sources");

    let report = run_mend_json(&root.join("Cargo.toml"));
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         tool::tool_staging::build_device_recipe() + camera::run()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "use crate::tool::tool_staging::build_device_recipe;\n\npub(crate) fn run() -> u32 {\n    \
         build_device_recipe() + crate::tool::tool_staging::stage_all()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod cascade;\n\npub fn touch() -> i32 {\n    cascade::run()\n}\n",
    );
    write(
        root,
        "src/cascade/mod.rs",
        "mod attributes;\nmod resolved;\n\npub(crate) use attributes::FontUnit;\n\npub(crate) fn \
         run() -> i32 {\n    FontUnit::default().0 + attributes::alpha()\n}\n",
    );
    write(
        root,
        "src/cascade/attributes.rs",
        "pub(crate) use super::resolved::FontUnit;\n\npub(super) fn alpha() -> i32 {\n    \
         FontUnit(5).0\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod free_cam;\nmod input;\nmod stage;\n\npub use input::Orbit;\n\npub fn touch() -> u32 \
         {\n    stage::run()\n}\n",
    );
    write(root, "src/free_cam.rs", "pub struct Orbit;\n");
    write(
        root,
        "src/input/mod.rs",
        "pub use crate::free_cam::Orbit;\n\npub(crate) fn poll() -> u32 {\n    3\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run() + \
         platform::discover()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "pub(crate) use crate::platform::discover;\n\npub(crate) fn run() -> i32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod platform;\n\npub fn touch() -> i32 {\n    camera::run() + \
         camera::discover()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "mod lens;\n\npub(crate) use crate::{camera::lens::focus, platform::discover};\n\n\
         pub(crate) fn run() -> i32 {\n    focus()\n}\n",
    );
    write(
        root,
        "src/camera/lens.rs",
        "pub(crate) fn focus() -> i32 {\n    2\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod tool;\n\n#[cfg(unix)]\npub fn touch() -> u32 {\n    \
         tool::tool_staging::build_device_recipe() + tool::tool_staging::stage_all() + \
         tool::run()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\nuse reset_scene::reset_all;\n\n\
         pub(super) fn run() -> u32 {\n    reset_all()\n}\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n\npub(super) fn reset_all() -> \
         u32 {\n    0\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod tool;\n#[path = \"../extra/probe.rs\"]\nmod probe;\n\n\
         include!(\"../extra/included.rs\");\n\npub fn touch() -> u32 {\n    probe::run() + \
         included() + tool::tool_staging::stage_all()\n}\n",
    );
    write(
        root,
        "extra/probe.rs",
        "use crate::tool::tool_staging::Recipe;\n\npub(crate) fn run() -> u32 {\n    Recipe(1).0\n\
         }\n",
    );
    write(
        root,
        "extra/included.rs",
        "fn included() -> u32 {\n    let tool::tool_staging::Recipe(value) = \
         tool::tool_staging::Recipe(3);\n    value\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) struct Recipe(pub(crate) u32);\n",
    );
    write(
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

/// An inline module left with nothing once its re-export moves is deleted
/// with its doc comment, and a caller inside the parent names the sibling.
#[test]
fn emptied_inline_owner_module_is_deleted() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub fn touch() -> u32 {\n    tool::staging::build_device_recipe() + \
         tool::run()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\n/// Hands staging callers the scene recipe.\npub(crate) mod staging \
         {\n    pub(crate) use super::reset_scene::build_device_recipe;\n}\n\npub(crate) fn run() \
         -> u32 {\n    staging::build_device_recipe()\n}\n",
    );
    write(
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
    write_manifest(root);
    let config_path = root.join("mend.toml");
    let config = fs::read_to_string(&config_path).expect("read fixture mend.toml");
    fs::write(
        config_path,
        format!("{config}allow_pub_mod = [\"src/tool/mod.rs\"]\n"),
    )
    .expect("write fixture mend.toml");
    write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub(crate) use tool::staging;\n\npub fn touch() -> u32 {\n    \
         staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\npub(crate) mod staging {\n    pub(crate) use \
         super::reset_scene::build_device_recipe;\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    camera::run()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "use crate::tool::tool_staging;\n\npub(crate) fn run() -> u32 {\n    \
         tool_staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod tool;\n\npub fn touch() {\n    assert_eq!(tool::tool_staging::build_device_recipe(), \
         1);\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod camera;\nmod tool;\n\npub fn touch() -> u32 {\n    camera::run() + \
         camera::staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/camera.rs",
        "pub(super) use crate::tool::staging;\n\npub(super) fn run() -> u32 {\n    \
         staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\n\npub(crate) mod staging {\n    pub(crate) use \
         super::reset_scene::build_device_recipe;\n}\n",
    );
    write(
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

/// A chain of four re-exports, each forwarding the one below it: removing one
/// link turns the next into the finding, and one `--fix` run keeps passing
/// until the crate root names the defining module.
#[test]
fn reexport_chain_collapses_in_one_run() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod input;\nmod orbit_cam;\n\npub use orbit_cam::Gain;\n",
    );
    write(root, "src/input.rs", "pub struct Gain(pub u32);\n");
    write(
        root,
        "src/orbit_cam/mod.rs",
        "mod controls;\n\npub use controls::Gain;\n",
    );
    write(
        root,
        "src/orbit_cam/controls/mod.rs",
        "mod bindings;\n\npub use bindings::Gain;\n",
    );
    write(
        root,
        "src/orbit_cam/controls/bindings/mod.rs",
        "mod preset;\n\npub use preset::Gain;\n",
    );
    write(
        root,
        "src/orbit_cam/controls/bindings/preset.rs",
        "pub use crate::input::Gain;\n",
    );

    assert_fixed(
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
    );
}

/// A caller in another file reaching the owner module through a `use ... as`
/// alias of it is redirected like a caller naming the owner module directly.
#[test]
fn caller_through_a_module_alias_in_another_file_is_rewritten() {
    let temp = tempdir().expect("create temp fixture dir");
    let root = temp.path();
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod frames;\nmod platform;\nmod screen;\n\npub fn touch() -> u32 {\n    \
         screen::show()\n}\n",
    );
    write(
        root,
        "src/frames.rs",
        "pub(crate) struct ScreenFrame(pub(crate) u32);\n",
    );
    write(
        root,
        "src/platform/mod.rs",
        "pub(crate) mod screen_capture_stream;\n",
    );
    write(
        root,
        "src/platform/screen_capture_stream.rs",
        "pub(crate) use crate::frames::ScreenFrame;\n\npub(crate) fn open() -> u32 {\n    \
         ScreenFrame(1).0\n}\n",
    );
    write(
        root,
        "src/screen/mod.rs",
        "use crate::platform::screen_capture_stream as capture_stream;\n\nmod view;\n\n\
         pub(crate) fn show() -> u32 {\n    view::render() + capture_stream::open()\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod application_readiness;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         application_readiness::ready()\n}\n",
    );
    write(
        root,
        "src/application_readiness.rs",
        "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
         #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn builds_the_recipe() \
         {\n        assert_eq!(tool::tool_staging::build_device_recipe() + ready(), 4);\n    }\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\npub(crate) fn stage() -> u32 {\n    \
         tool_staging::stage_all() + tool_staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod application_readiness;\nmod tool;\n\npub fn touch() -> u32 {\n    \
         application_readiness::ready() + tool::tool_staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/application_readiness.rs",
        "use crate::tool;\n\npub(crate) fn ready() -> u32 {\n    tool::stage()\n}\n\n\
         #[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn builds_the_recipe() \
         {\n        assert_eq!(tool::tool_staging::build_device_recipe(), 1);\n    }\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n\npub(crate) fn stage() -> u32 {\n    \
         tool_staging::stage_all() + tool_staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) fn build_device_recipe() -> u32 {\n    1\n}\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod tool;\n\nuse tool::tool_staging::DeviceRecipe;\n\npub fn touch() -> u32 {\n    \
         DeviceRecipe(1).0 + tool::tool_staging::build_device_recipe()\n}\n",
    );
    write(
        root,
        "src/tool/mod.rs",
        "mod reset_scene;\npub(crate) mod tool_staging;\n",
    );
    write(
        root,
        "src/tool/reset_scene.rs",
        "pub(crate) struct DeviceRecipe(pub(crate) u32);\n\npub(crate) fn build_device_recipe() \
         -> u32 {\n    1\n}\n",
    );
    write(
        root,
        "src/tool/tool_staging/mod.rs",
        "pub(crate) use super::reset_scene::DeviceRecipe;\npub(crate) use \
         super::reset_scene::build_device_recipe;\n\n#[cfg(test)]\nmod tests;\n",
    );
    write(
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
    write_manifest(root);
    write(
        root,
        "src/lib.rs",
        "mod first;\nmod platform;\nmod second;\n\npub fn touch() -> i32 {\n    first::run() + \
         second::run()\n}\n",
    );
    for parent in ["src/first.rs", "src/second.rs"] {
        write(
            root,
            parent,
            "#[path = \"shared.rs\"]\nmod shared;\n\npub(super) fn run() -> i32 {\n    \
             shared::discover()\n}\n",
        );
    }
    write(
        root,
        "src/shared.rs",
        "pub(super) use crate::platform::discover;\n",
    );
    write(
        root,
        "src/platform.rs",
        "pub(crate) fn discover() -> i32 {\n    4\n}\n",
    );
    let sources = rust_sources(root);

    let stderr = run_fix(root);
    assert!(
        stderr.contains("skipped 2 `pub_use_outside_subtree` finding(s) `--fix` could not rewrite"),
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
