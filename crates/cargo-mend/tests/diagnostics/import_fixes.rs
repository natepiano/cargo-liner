use crate::support::*;

#[test]
fn fix_keeps_tile_root_exports_and_self_module_imports() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let grid = "use crate::tiles::grid;\npub struct TileDemand(pub usize);\npub enum TileContent { Group(TileDemand) }\n\
                pub fn demand() -> TileDemand { grid::TileDemand(7) }\n\
                #[cfg(test)] mod tests { use crate::TileDemand; #[test] fn demand_is_public() { let demand = TileDemand(3); assert_eq!(demand.0, 3); } }\n";
    let root = batch.add_member("tile_self_import", &[
        ("Cargo.toml", "[package]\nname = \"tile_self_import\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautolib = false\n[[test]]\nname = \"tile_imports\"\npath = \"src/lib.rs\"\n"),
        ("src/lib.rs", "mod tiles;\npub use tiles::TileContent;\npub use tiles::TileDemand;\npub fn run() -> usize { tiles::run() }\n"),
        ("src/tiles/mod.rs", "mod grid;\nmod host;\npub use grid::TileContent;\npub use grid::TileDemand;\npub use host::run;\n"),
        ("src/tiles/grid.rs", grid),
        ("src/tiles/host.rs", "use crate::TileContent;\nuse crate::TileDemand;\npub fn run() -> usize { let demand: TileDemand = super::grid::demand(); match TileContent::Group(demand) { TileContent::Group(demand) => demand.0 } }\n#[cfg(test)] #[path = \"host_tests.rs\"] mod tests;\n"),
        ("src/tiles/host_tests.rs", "use crate::TileContent;\nuse crate::TileDemand;\n#[test] fn content_is_public() { let content = TileContent::Group(TileDemand(5)); let TileContent::Group(demand) = content; assert_eq!(demand.0, 5); }\n"),
    ]);
    let sources = [
        "lib.rs",
        "tiles/mod.rs",
        "tiles/grid.rs",
        "tiles/host.rs",
        "tiles/host_tests.rs",
    ]
    .map(|file| root.join("src").join(file));
    let fixed = batch
        .command()
        .args(["--all-targets", "--fix"])
        .output()
        .expect("fix tile imports");
    let stderr = String::from_utf8_lossy(&fixed.stderr);
    let diagnostics = strip_ansi(&format!(
        "{}\n{stderr}",
        String::from_utf8_lossy(&fixed.stdout)
    ));
    assert!(
        !diagnostics.contains("consider using: `use ;`"),
        "an import must name a path: {diagnostics}"
    );
    assert!(
        fixed.status.success(),
        "{}\n{stderr}",
        String::from_utf8_lossy(&fixed.stdout)
    );
    assert!(!stderr.contains("rolled back"), "{stderr}");
    let after = sources
        .iter()
        .map(|path| fs::read_to_string(path).expect("read fixed source"))
        .collect::<Vec<_>>();
    let root_exports = fs::read_to_string(root.join("src/lib.rs")).expect("read root exports");
    assert!(
        root_exports.contains("pub use tiles::TileContent;"),
        "{root_exports}"
    );
    assert!(
        root_exports.contains("pub use tiles::TileDemand;"),
        "{root_exports}"
    );
    let second = batch
        .command()
        .args(["--all-targets", "--fix"])
        .output()
        .expect("repeat tile fixes");
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let repeated = sources
        .iter()
        .map(|path| fs::read_to_string(path).expect("read repeated source"))
        .collect::<Vec<_>>();
    assert_eq!(repeated, after, "the second fix must preserve all sources");
}

#[test]
fn fix_imports_keep_modules_declared_in_path_mounted_test_files() {
    let mut batch = DiagnosticBatch::new(
        "[diagnostics]\nsuspicious_pub = false\nunused_pub = false\nnarrow_to_pub_crate = false\npub_use_outside_subtree = false\n[visibility]\npub_in_path = \"permitted\"\n",
    );
    let root = batch.add_member("path_mounted_imports", &[
        ("Cargo.toml", "[package]\nname = \"path_mounted_imports\"\nversion = \"0.1.0\"\nedition = \"2024\"\nautotests = false\n"),
        ("src/lib.rs", "mod propagation;\nmod camera_source;\nmod tool;\nmod normal;\n#[cfg(test)] #[path = \"../tests/test_support/mod.rs\"] mod test_support;\n"),
        ("src/normal.rs", "pub(crate) fn width() -> usize {\n    use std::mem;\n    mem::size_of::<usize>()\n}\n"),
        ("src/propagation.rs", "#[cfg(test)] #[path = \"../tests/propagation_roles/mod.rs\"] pub(crate) mod phase68_tests;\n"),
        ("src/camera_source.rs", "#[cfg(test)] #[path = \"../tests/camera_publication/mod.rs\"] mod phase68_tests;\n"),
        ("src/tool/mod.rs", "#[cfg(test)] #[path = \"../../tests/jack_controls/modulation_mode.rs\"] mod modulation_mode;\n"),
        ("tests/propagation_roles/mod.rs", "pub(crate) mod support;\n"),
        ("tests/propagation_roles/support.rs", "pub(crate) fn value() -> usize { 7 }\n"),
        ("tests/camera_publication/mod.rs", "use crate::propagation::phase68_tests::support;\n#[test] fn publication() { assert_eq!(support::value(), 7); }\n"),
        ("tests/test_support/mod.rs", "pub(crate) mod slider_reset;\n"),
        ("tests/test_support/slider_reset.rs", "pub(crate) fn value() -> usize { 11 }\n"),
        ("tests/jack_controls/modulation_mode.rs", "use crate::test_support::slider_reset;\n#[test] fn reset() { assert_eq!(slider_reset::value(), 11); }\n"),
    ]);
    let output = batch
        .command()
        .args(["--all-targets", "--fix"])
        .output()
        .expect("fix path-mounted imports");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{}\n{stderr}",
        String::from_utf8_lossy(&output.stdout),
    );
    assert!(!stderr.contains("rolled back"), "{stderr}");
    let normal = fs::read_to_string(root.join("src/normal.rs")).expect("read moved import");
    assert!(normal.starts_with("use std::mem;"), "{normal}");
    for (file, import, call) in [
        (
            "tests/camera_publication/mod.rs",
            "use crate::propagation::phase68_tests::support;",
            "support::value()",
        ),
        (
            "tests/jack_controls/modulation_mode.rs",
            "use crate::test_support::slider_reset;",
            "slider_reset::value()",
        ),
    ] {
        let fixed = fs::read_to_string(root.join(file)).expect("read path-mounted test");
        assert!(fixed.contains(import), "{fixed}");
        assert!(fixed.contains(call), "{fixed}");
    }
}

#[test]
fn fix_imports_resolve_nested_src_and_custom_path_targets() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let consumer = "use crate::outer::m::{self};\nuse super::m::one;\nuse super::m::two;\npub fn total() -> usize { one() + two() + m::one() }\n";
    let mut consumers = Vec::new();
    for (name, manifest, root_file, declaration) in [
        (
            "nested_src",
            "[package]\nname = \"nested_src\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            "src/main.rs",
            "pub mod m;\n#[path = \"src/consumer.rs\"] pub mod consumer;\n",
        ),
        (
            "custom_root",
            "[package]\nname = \"custom_root\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[[bin]]\nname = \"custom_root\"\npath = \"entry/start.rs\"\n",
            "entry/start.rs",
            "pub mod m;\n#[path = \"src/consumer.rs\"] pub mod consumer;\n",
        ),
    ] {
        let root = batch.add_member(name, &[
            ("Cargo.toml", manifest),
            (root_file, "#[path = \"../src/outer/mod.rs\"] mod outer;\nfn main() { assert_eq!(outer::consumer::total(), 4); }\n"),
            ("src/outer/mod.rs", declaration),
            ("src/outer/m.rs", "pub fn one() -> usize { 1 }\npub fn two() -> usize { 2 }\n"),
            ("src/outer/src/consumer.rs", consumer),
        ]);
        consumers.push(root.join("src/outer/src/consumer.rs"));
    }
    let output = batch
        .command()
        .arg("--fix")
        .output()
        .expect("fix target-root imports");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for path in consumers {
        let fixed = fs::read_to_string(path).expect("read target-root consumer");
        assert!(fixed.contains("use crate::outer::m::{self};"), "{fixed}");
        assert_eq!(fixed.matches("use ").count(), 1, "{fixed}");
        assert!(fixed.contains("m::two()"), "{fixed}");
    }
}

#[test]
fn fix_imports_decline_conflicting_shared_file_mounts() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let shared = "use super::m::one;\nuse super::m::two;\npub fn total() -> usize { use super::m::{self}; one() + two() + m::one() }\n";
    let root = batch.add_member("shared_mounts", &[
        ("src/main.rs", "mod alpha;\nmod beta;\nfn main() { assert_eq!(alpha::consumer::total(), 4); assert_eq!(beta::consumer::total(), 8); }\n"),
        ("src/alpha/mod.rs", "pub mod m { pub fn one() -> usize { 1 } pub fn two() -> usize { 2 } }\n#[path = \"../shared.rs\"] pub mod consumer;\n"),
        ("src/beta/mod.rs", "pub mod m { pub fn one() -> usize { 2 } pub fn two() -> usize { 4 } }\n#[path = \"../shared.rs\"] pub mod consumer;\n"),
        ("src/shared.rs", shared),
    ]);
    let output = batch
        .command()
        .arg("--fix")
        .output()
        .expect("fix shared-mount imports");
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fixed = fs::read_to_string(root.join("src/shared.rs")).expect("read shared file");
    assert_eq!(
        fixed, shared,
        "mount-dependent imports and references must be deferred together"
    );
}

#[test]
fn fix_imports_preserve_public_test_reexport() {
    let mut batch = DiagnosticBatch::new(
        "[diagnostics]\nsuspicious_pub = false\nunused_pub = false\nnarrow_to_pub_crate = false\npub_use_outside_subtree = false\n[visibility]\npub_in_path = \"permitted\"\nallow_pub_mod = [\"public_test_binding/src/main.rs\"]\n",
    );
    let root = batch.add_member("public_test_binding", &[
        ("src/main.rs", "pub mod m;\nmod consumer;\nfn main() { assert_eq!(consumer::total(), 3); }\n#[cfg(test)] #[test] fn reexport_is_reachable_outside_consumer() { assert_eq!(consumer::m::one(), 1); }\n"),
        ("src/m.rs", "pub fn one() -> usize { 1 }\npub fn two() -> usize { 2 }\n"),
        ("src/consumer.rs", "#[cfg(test)] pub use crate::m::{self};\nuse crate::m::one;\nuse crate::m::two;\npub fn total() -> usize { one() + two() }\n"),
    ]);
    let output = batch
        .command()
        .arg("--all-targets")
        .arg("--fix")
        .output()
        .expect("fix conditional reexport imports");
    assert!(
        output.status.success(),
        "normal and test builds must compile: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fixed =
        fs::read_to_string(root.join("src/consumer.rs")).expect("read conditional reexport");
    assert!(
        fixed.contains("#[cfg(test)] pub use crate::m::{self};"),
        "{fixed}"
    );
    assert_eq!(fixed.matches("use ").count(), 2, "{fixed}");
    assert!(
        fixed.contains("m::one()") && fixed.contains("m::two()"),
        "{fixed}"
    );
    // The retained export keeps exactly its original test gate; the private
    // import supplies only the normal build, without widening the public API.
}

#[test]
fn fix_imports_resolve_relative_and_crate_paths_to_one_module() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let consumer = "use crate::outer::m::{self};\nuse super::m::one;\nuse super::m::two;\npub fn total() -> usize { one() + two() + m::one() }\n";
    let root = batch.add_member(
        "relative_module",
        &[
            (
                "src/main.rs",
                "mod outer;\nfn main() { assert_eq!(outer::consumer::total(), 4); }\n",
            ),
            ("src/outer/mod.rs", "pub mod m;\npub mod consumer;\n"),
            (
                "src/outer/m.rs",
                "pub fn one() -> usize { 1 }\npub fn two() -> usize { 2 }\n",
            ),
            ("src/outer/consumer.rs", consumer),
        ],
    );
    let output = batch
        .command()
        .arg("--fix")
        .output()
        .expect("fix relative import fixture");
    assert!(
        output.status.success(),
        "relative and crate imports must compile together: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fixed =
        fs::read_to_string(root.join("src/outer/consumer.rs")).expect("read fixed consumer");
    assert!(fixed.contains("use crate::outer::m::{self};"), "{fixed}");
    assert!(!fixed.contains("use super::m;"), "{fixed}");
    assert_eq!(fixed.matches("use ").count(), 1, "{fixed}");
    assert!(fixed.contains("m::two()"), "{fixed}");
}

#[test]
fn fix_imports_ungated_module_covers_kept_test_import() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let root = batch.add_member("gated_module", &[
        ("src/main.rs", "mod m;\nmod consumer;\nfn main() { assert_eq!(consumer::total(), 3); }\n"),
        ("src/m.rs", "pub fn one() -> usize { 1 }\npub fn two() -> usize { 2 }\n"),
        ("src/consumer.rs", "#[cfg(test)] use crate::m::{self};\nuse crate::m::one;\nuse crate::m::two;\npub fn total() -> usize { one() + two() }\n#[cfg(test)] mod tests { #[test] fn total_uses_the_shared_module() { assert_eq!(super::total(), 3); assert_eq!(super::m::one(), 1); } }\n"),
    ]);
    let output = batch
        .command()
        .arg("--all-targets")
        .arg("--fix")
        .output()
        .expect("fix test-target import fixture");
    assert!(
        output.status.success(),
        "the normal and test builds must both compile: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let fixed = fs::read_to_string(root.join("src/consumer.rs")).expect("read gated consumer");
    assert_eq!(fixed.matches("use ").count(), 1, "{fixed}");
    assert!(!fixed.contains("#[cfg(test)] use"), "{fixed}");
    assert!(fixed.contains("m::one()"), "{fixed}");
    assert!(fixed.contains("m::two()"), "{fixed}");
    assert!(fixed.contains("#[cfg(test)] mod tests"), "{fixed}");
}

#[test]
fn fix_imports_each_module_once_across_function_rewrites_and_moves() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let mut consumers = Vec::new();
    for (name, module_import) in [
        ("existing_grouped_self", "use crate::m::{self};\n"),
        ("moved_grouped_self", ""),
    ] {
        let local_import = if module_import.is_empty() {
            "use crate::m::{self};"
        } else {
            ""
        };
        let source = format!(
            "{module_import}use crate::m::one;\nuse crate::m::two;\nuse crate::m::three;\n\n\
             pub fn total() -> usize {{ {local_import} one() + two() + three() + m::one() }}\n"
        );
        let root = batch.add_member(name, &[
            ("src/main.rs", "mod m;\nmod consumer;\nfn main() { assert_eq!(consumer::total(), 7); }\n"),
            ("src/m.rs", "pub fn one() -> usize { 1 }\npub fn two() -> usize { 2 }\npub fn three() -> usize { 3 }\n"),
            ("src/consumer.rs", &source),
        ]);
        consumers.push(root.join("src/consumer.rs"));
    }
    let output = batch
        .command()
        .arg("--fix")
        .output()
        .expect("fix import batch");
    assert!(
        output.status.success(),
        "fixed crates must compile without duplicate imports: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    for consumer in consumers {
        let source = fs::read_to_string(consumer).expect("read fixed imports");
        assert_eq!(source.matches("use crate::m").count(), 1, "{source}");
        assert!(source.contains("m::two()"), "{source}");
        assert!(source.contains("m::three()"), "{source}");
        assert!(!source.contains("use crate::m::one;"), "{source}");
        assert!(!source.contains("use crate::m::two;"), "{source}");
        assert!(!source.contains("use crate::m::three;"), "{source}");
    }
}

#[test]
fn fix_rewrites_local_crate_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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

/// A pass that writes nothing leaves the tree unchanged, so `--fix` stops after
/// it instead of checking the crate again. Cargo replays a fresh unit's
/// warnings, so a second check would print the warning a second time.
#[test]
fn fix_with_nothing_to_fix_checks_the_crate_once() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_single_pass_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "fn main() {}\n\nfn never_called() {}\n",
    )
    .expect("write fixture main");

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    assert_eq!(
        stderr
            .matches("function `never_called` is never used")
            .count(),
        1,
        "a no-op --fix must check the crate once:\n{stderr}"
    );
    assert!(stderr.contains("mend: no import fixes available"));
}

#[test]
fn fix_reports_applied_notice_after_summary() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "fix_applied_notice_fixture"
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
        "mod child;\nmod consumer;\n",
    )
    .expect("write parent mod");
    fs::write(
        temp.path().join("src/outer/parent/child.rs"),
        "pub struct Thing;\n",
    )
    .expect("write child");
    fs::write(
        temp.path().join("src/outer/parent/consumer.rs"),
        "use crate::outer::parent::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write consumer");

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
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
    // One edit moves the import and the other strips an unreachable `pub`, so
    // each is counted under its own fix kind rather than as two import fixes.
    assert!(stderr.contains("mend: would apply 1 import fix(es) in dry run"));
    assert!(stderr.contains("would apply 1 `pub` removal(s) in dry run"));

    let consumer = fs::read_to_string(temp.path().join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(consumer.contains("use crate::parent::child::Thing;"));
}

#[test]
fn already_local_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    let expected_findings: [ExpectedFinding; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix,
        expected_summary.fixable_with_fix
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected_summary.fixable_with_fix_pub_use
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::ShortenLocalCrateImport)
    );
}

#[test]
fn top_level_peer_imports_are_not_reported() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::ShortenLocalCrateImport),
        "top-level peer imports should not trigger shorten_local_crate_import"
    );
}

#[test]
fn grouped_imports_are_ignored_safely() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

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
    let expected_findings: [ExpectedFinding; 0] = [];
    let expected_summary = expected_summary_from_findings(&expected_findings);
    assert_eq!(
        report.summary.fixable_with_fix,
        expected_summary.fixable_with_fix
    );
    assert_eq!(
        report.summary.fixable_with_fix_pub_use,
        expected_summary.fixable_with_fix_pub_use
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::ShortenLocalCrateImport)
    );
}

fn deep_super_is_flagged_and_fixed(batch: &mut DiagnosticBatch) -> impl FnOnce(&Report) + use<> {
    let root = batch.add_module("deep_super_is_flagged_and_fixed", &[("mod.rs", "")]);
    fs::create_dir_all(root.join("tui/columns")).expect("create src/tui/columns");
    fs::write(root.join("mod.rs"), "mod tui;\n\nfn main() {}\n").expect("write fixture main");
    fs::write(
        root.join("tui/mod.rs"),
        "mod columns;\npub struct ResolvedWidths;\n",
    )
    .expect("write tui mod");
    fs::write(root.join("tui/columns/mod.rs"), "mod render;\n").expect("write columns mod");
    fs::write(
        root.join("tui/columns/render.rs"),
        "use super::super::ResolvedWidths;\n\nfn use_it(_widths: ResolvedWidths) {}\n",
    )
    .expect("write render");

    move |report| {
        let findings = findings_at(
            report,
            "src/deep_super_is_flagged_and_fixed/tui/columns/render.rs",
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == DiagnosticCode::ReplaceDeepSuperImport),
            "deep super::super:: should trigger replace_deep_super_import, got: {:?}",
            findings.iter().map(|f| &f.code).collect::<Vec<_>>()
        );

        let render =
            fs::read_to_string(root.join("tui/columns/render.rs")).expect("read fixed file");
        assert!(
            render.contains("use crate::deep_super_is_flagged_and_fixed::tui::ResolvedWidths;"),
            "expected crate::deep_super_is_flagged_and_fixed::tui::ResolvedWidths, got: {render}"
        );
        assert!(!render.contains("use super::super::ResolvedWidths;"));
    }
}

fn triple_super_is_flagged_and_fixed(batch: &mut DiagnosticBatch) -> impl FnOnce(&Report) + use<> {
    let root = batch.add_module("triple_super_is_flagged_and_fixed", &[("mod.rs", "")]);
    fs::create_dir_all(root.join("a/b/c")).expect("create src/a/b/c");
    fs::write(root.join("mod.rs"), "mod a;\n\nfn main() {}\n").expect("write main");
    fs::write(root.join("a/mod.rs"), "mod b;\npub struct Root;\n").expect("write a mod");
    fs::write(root.join("a/b/mod.rs"), "mod c;\n").expect("write b mod");
    fs::write(root.join("a/b/c/mod.rs"), "mod leaf;\n").expect("write c mod");
    fs::write(
        root.join("a/b/c/leaf.rs"),
        "use super::super::super::Root;\n\nfn use_it(_root: Root) {}\n",
    )
    .expect("write leaf");

    move |report| {
        let findings = findings_at(
            report,
            "src/triple_super_is_flagged_and_fixed/a/b/c/leaf.rs",
        );
        assert!(
            findings
                .iter()
                .any(|finding| finding.code == DiagnosticCode::ReplaceDeepSuperImport),
            "super::super::super:: should trigger replace_deep_super_import, got: {:?}",
            findings.iter().map(|f| &f.code).collect::<Vec<_>>()
        );

        let leaf = fs::read_to_string(root.join("a/b/c/leaf.rs")).expect("read fixed file");
        assert!(
            leaf.contains("use crate::triple_super_is_flagged_and_fixed::a::Root;"),
            "expected crate::triple_super_is_flagged_and_fixed::a::Root, got: {leaf}"
        );
        assert!(!leaf.contains("super::super::super::"));
    }
}

fn single_super_is_not_flagged_as_deep(
    batch: &mut DiagnosticBatch,
) -> impl FnOnce(&Report) + use<> {
    let root = batch.add_module("single_super_is_not_flagged_as_deep", &[("mod.rs", "")]);
    fs::create_dir_all(root.join("outer")).expect("create src/outer");
    fs::write(root.join("mod.rs"), "mod outer;\n").expect("write lib");
    fs::write(root.join("outer/mod.rs"), "mod child;\nmod sibling;\n").expect("write outer mod");
    fs::write(root.join("outer/child.rs"), "pub struct Thing;\n").expect("write child");
    fs::write(
        root.join("outer/sibling.rs"),
        "use super::child::Thing;\n\nfn use_it(_thing: Thing) {}\n",
    )
    .expect("write sibling");

    move |report| {
        let report = member_report(report, "src/single_super_is_not_flagged_as_deep");
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.code == DiagnosticCode::ReplaceDeepSuperImport),
            "single super:: should not trigger replace_deep_super_import"
        );
    }
}

/// A module attached with `#[path]` does not live where its directory says. Its
/// imports must not be shortened against the directory-derived module path: the
/// `super::` that produces names the wrong parent, and the crate stops compiling.
#[test]
fn fix_leaves_a_path_attached_module_import_alone() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "path_attribute_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/platform")).expect("create src/platform");
    fs::create_dir_all(temp.path().join("src/stream")).expect("create src/stream");
    fs::write(
        temp.path().join("src/lib.rs"),
        "mod platform;\nmod stream;\n\n\
         pub fn open() { drop(platform::open_camera_stream()); }\n",
    )
    .expect("write fixture lib");
    fs::write(
        temp.path().join("src/platform/mod.rs"),
        "#[path = \"../stream/macos.rs\"]\nmod camera_stream;\n\n\
         pub(crate) use camera_stream::open_camera_stream;\n",
    )
    .expect("write fixture platform module");
    fs::write(
        temp.path().join("src/stream/mod.rs"),
        "pub(crate) struct CameraFrame;\n",
    )
    .expect("write fixture stream module");
    fs::write(
        temp.path().join("src/stream/macos.rs"),
        "use crate::stream::CameraFrame;\n\n\
         pub(crate) fn open_camera_stream() -> CameraFrame { CameraFrame }\n",
    )
    .expect("write fixture detached module");

    let output = mend_command_for(&temp.path().join("Cargo.toml"))
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let detached =
        fs::read_to_string(temp.path().join("src/stream/macos.rs")).expect("read fixed file");
    assert!(
        detached.contains("use crate::stream::CameraFrame;"),
        "the crate-absolute import was rewritten: {detached}"
    );
    assert!(
        !detached.contains("use super::"),
        "a super import was introduced against the wrong parent: {detached}"
    );
}

#[test]
fn super_path_depths_are_reported_and_fixed_independently() {
    let mut batch = DiagnosticBatch::new_crate("[visibility]\npub_in_path = \"permitted\"\n");
    let deep_super_is_flagged_and_fixed = deep_super_is_flagged_and_fixed(&mut batch);
    let triple_super_is_flagged_and_fixed = triple_super_is_flagged_and_fixed(&mut batch);
    let single_super_is_not_flagged_as_deep = single_super_is_not_flagged_as_deep(&mut batch);
    let report = batch.report();
    let output = batch.command().arg("--fix").output().expect("fix batch");
    assert!(
        output.status.success(),
        "batch fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    deep_super_is_flagged_and_fixed(&report);
    triple_super_is_flagged_and_fixed(&report);
    single_super_is_not_flagged_as_deep(&report);
}
