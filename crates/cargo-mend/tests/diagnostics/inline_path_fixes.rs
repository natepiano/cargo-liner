use crate::support::*;

const ABSOLUTE_PATH_PROJECT_MANIFEST: &str = r#"[package]
name="project"
version="0.1.0"
edition="2024"
[workspace]
[dependencies]
dep={path="../dep"}
"#;

const ABSOLUTE_PATH_DEPENDENCY_MANIFEST: &str = r#"[package]
name="dep"
version="0.1.0"
edition="2024"
[workspace]
"#;

const ABSOLUTE_PATH_DEPENDENCY_SOURCE: &str = r#"pub mod events {
    pub struct PointerCancel {
        pub entity: u32,
        pub pointer: Pointer,
        pub hit: crate::backend::HitData,
    }
    pub struct Pointer(pub u32);
    impl Pointer {
        pub fn new(id: u32) -> Self {
            Self(id)
        }
    }
}
pub mod backend {
    pub struct HitData(pub f32);
    impl HitData {
        pub fn new(depth: f32) -> Self {
            Self(depth)
        }
    }
}
pub mod prelude {
    pub use crate::World;
    pub use crate::events::*;
}
pub struct World;
impl World {
    pub fn trigger<T>(&mut self, _: T) {}
}
"#;

const ABSOLUTE_PATH_TOOL_SOURCE: &str = r#"use dep::prelude::*;

pub fn run(world: &mut World, pointers: &[u32]) {
    for pointer in pointers {
        world.trigger(::dep::events::PointerCancel {
            entity: 1,
            pointer: ::dep::events::Pointer::new(*pointer),
            hit: dep::backend::HitData::new(0.0),
        });
    }
}
"#;

const ABSOLUTE_PATH_LIBRARY_SOURCE: &str = r#"mod tool;
pub use tool::run;
"#;

const ABSOLUTE_PATH_SOURCE_DIRECTORIES: [&str; 2] = ["project/src", "dep/src"];
const ABSOLUTE_PATH_PROJECT_MANIFEST_PATH: &str = "project/Cargo.toml";
const ABSOLUTE_PATH_TOOL_SOURCE_PATH: &str = "project/src/tool.rs";
const ABSOLUTE_PATH_FIXTURE_FILES: [(&str, &str); 5] = [
    (
        ABSOLUTE_PATH_PROJECT_MANIFEST_PATH,
        ABSOLUTE_PATH_PROJECT_MANIFEST,
    ),
    ("dep/Cargo.toml", ABSOLUTE_PATH_DEPENDENCY_MANIFEST),
    ("dep/src/lib.rs", ABSOLUTE_PATH_DEPENDENCY_SOURCE),
    ("project/src/lib.rs", ABSOLUTE_PATH_LIBRARY_SOURCE),
    (ABSOLUTE_PATH_TOOL_SOURCE_PATH, ABSOLUTE_PATH_TOOL_SOURCE),
];

const SHADOWED_STD_MANIFEST: &str = r#"[package]
name="shadowed_std"
version="0.1.0"
edition="2024"
[workspace]
"#;
const SHADOWED_STD_SOURCE: &str = r#"mod other {
    pub(crate) mod std {
        pub(crate) mod time {
            pub(crate) struct Duration;
        }
        pub(crate) fn marker() {}
    }
}
mod consumer {
    use crate::other::std;
    pub(crate) fn duration() -> ::std::time::Duration {
        std::marker();
        ::std::time::Duration::from_secs(1)
    }
}
fn main() { let _ = consumer::duration(); }
"#;
const SHADOWED_STD_SOURCE_PATH: &str = "src/main.rs";
const SHADOWED_STD_MANIFEST_PATH: &str = "Cargo.toml";
const SHADOWED_STD_SOURCE_DIRECTORY: &str = "src";

#[test]
fn absolute_paths_keep_the_external_crate_when_its_name_is_shadowed() {
    let fixture = tempdir().expect("create shadowed std fixture");
    fs::create_dir_all(fixture.path().join(SHADOWED_STD_SOURCE_DIRECTORY))
        .expect("create source directory");
    fs::write(
        fixture.path().join(SHADOWED_STD_MANIFEST_PATH),
        SHADOWED_STD_MANIFEST,
    )
    .expect("write manifest");
    fs::write(
        fixture.path().join(SHADOWED_STD_SOURCE_PATH),
        SHADOWED_STD_SOURCE,
    )
    .expect("write shadowed std fixture");
    let output = mend_command_for(&fixture.path().join(SHADOWED_STD_MANIFEST_PATH))
        .arg("--fix")
        .output()
        .expect("fix absolute paths with a shadowed crate name");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "mend --fix failed: {stderr}");
    assert!(
        !stderr.contains("rolled back"),
        "unexpected rollback: {stderr}"
    );
    let source = fs::read_to_string(fixture.path().join(SHADOWED_STD_SOURCE_PATH))
        .expect("read fixed shadowed std fixture");
    assert!(
        !source.contains("use crate::other::std::time::Duration;"),
        "{source}"
    );
}

#[test]
fn absolute_inline_paths_with_glob_prelude_build_after_fix() {
    struct ValidAbsolutePaths;

    impl<'ast> syn::visit::Visit<'ast> for ValidAbsolutePaths {
        fn visit_path(&mut self, path: &'ast syn::Path) {
            assert!(
                path.leading_colon.is_none() || path.segments.len() != 1,
                "fixed source contains an absolute path without a crate name"
            );
            syn::visit::visit_path(self, path);
        }
    }

    let fixture = tempdir().expect("create absolute path fixture");
    for directory in ABSOLUTE_PATH_SOURCE_DIRECTORIES {
        fs::create_dir_all(fixture.path().join(directory))
            .expect("create fixture source directory");
    }
    for (path, source) in ABSOLUTE_PATH_FIXTURE_FILES {
        fs::write(fixture.path().join(path), source).expect("write absolute path fixture");
    }
    let output = mend_command_for(&fixture.path().join(ABSOLUTE_PATH_PROJECT_MANIFEST_PATH))
        .arg("--fix")
        .output()
        .expect("fix absolute inline paths");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "mend --fix failed: {stderr}");
    assert!(
        !stderr.contains("rolled back"),
        "unexpected rollback: {stderr}"
    );
    let tool = fs::read_to_string(fixture.path().join(ABSOLUTE_PATH_TOOL_SOURCE_PATH))
        .expect("read fixed absolute paths");
    assert!(tool.contains("world.trigger(PointerCancel {"), "{tool}");
    let syntax = syn::parse_file(&tool).expect("parse fixed absolute paths");
    syn::visit::Visit::visit_file(&mut ValidAbsolutePaths, &syntax);
}

fn basic_inline_type_adds_use(batch: &mut DiagnosticBatch) -> impl FnOnce() + use<> {
    let root = batch.add_module("basic_inline_type_adds_use", &[("mod.rs", "")]);
    fs::create_dir_all(root.join("parent")).expect("create src/parent");
    fs::write(root.join("mod.rs"), "mod parent;\nfn main() {}\n").expect("write fixture main");
    fs::write(root.join("parent.rs"), "mod types;\nmod consumer;\n").expect("write parent mod");
    fs::write(root.join("parent/types.rs"), "pub(super) struct MyType;\n").expect("write types");
    fs::write(
        root.join("parent/consumer.rs"),
        r#"fn example(_x: super::types::MyType) {}
"#,
    )
    .expect("write consumer");

    move || {
        let consumer =
            fs::read_to_string(root.join("parent/consumer.rs")).expect("read fixed file");
        assert!(
            consumer.contains("use") && consumer.contains("MyType;"),
            "expected use import for MyType, got:\n{consumer}"
        );
        assert!(
            consumer.contains("_x: MyType"),
            "expected bare type name in signature, got:\n{consumer}"
        );
        // The inline path in the function signature should be gone (only appears in the use
        // statement)
        assert!(
            consumer.contains("_x: MyType)"),
            "inline path should be replaced with bare type, got:\n{consumer}"
        );
    }
}

#[test]
fn nested_module_inserts_use_in_containing_scope() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_nested_scope_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/tui")).expect("create src/tui");
    fs::write(temp.path().join("src/lib.rs"), "mod tui;\n").expect("write fixture lib");
    fs::write(
        temp.path().join("src/tui/mod.rs"),
        "mod app;\nmod interaction;\n",
    )
    .expect("write fixture tui mod");
    fs::write(
        temp.path().join("src/tui/app.rs"),
        "pub enum SearchMode {\n    Active,\n}\n",
    )
    .expect("write fixture app");
    fs::write(
        temp.path().join("src/tui/interaction.rs"),
        r#"pub fn keep() {}

#[cfg(test)]
mod tests {
    use std::mem;

    #[test]
    fn nested_variant_path_gets_local_use() {
        let _ = mem::size_of_val(&super::super::app::SearchMode::Active);
    }
}
"#,
    )
    .expect("write fixture interaction");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let interaction = fs::read_to_string(temp.path().join("src/tui/interaction.rs"))
        .expect("read fixed interaction");
    assert!(
        interaction
            .contains("mod tests {\n    use std::mem;\n    use crate::tui::app::SearchMode;\n"),
        "expected nested-module import of the enum type, got:\n{interaction}"
    );
    assert!(
        interaction.contains("mem::size_of_val(&SearchMode::Active);"),
        "expected enum-qualified variant reference inside nested module, got:\n{interaction}"
    );
}

#[test]
fn nested_module_uses_parent_import_for_partial_path() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_parent_import_fixture"
version = "0.1.0"
edition = "2024"

[features]
monitor-probe = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/monitors")).expect("create src/monitors");
    fs::write(temp.path().join("src/lib.rs"), "mod monitors;\n").expect("write fixture lib");
    fs::write(
        temp.path().join("src/monitors/mod.rs"),
        "#[cfg(feature = \"monitor-probe\")]\nmod monitor_probe;\nmod topology;\n",
    )
    .expect("write monitors mod");
    fs::write(
        temp.path().join("src/monitors/monitor_probe.rs"),
        "pub(super) struct InjectedTopologyProbeRecords;\n",
    )
    .expect("write monitor probe");
    fs::write(
        temp.path().join("src/monitors/topology.rs"),
        r#"#[cfg(feature = "monitor-probe")]
#[cfg_attr(feature = "monitor-probe", cfg(target_os = "macos"))]
use super::monitor_probe;

pub fn keep() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "monitor-probe")]
    #[cfg_attr(feature = "monitor-probe", cfg(target_os = "macos"))]
    fn first() -> Option<monitor_probe::InjectedTopologyProbeRecords> {
        None
    }

    #[cfg(feature = "monitor-probe")]
    #[cfg_attr(feature = "monitor-probe", cfg(target_os = "macos"))]
    fn second() -> Option<monitor_probe::InjectedTopologyProbeRecords> {
        first()
    }
}
"#,
    )
    .expect("write topology");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let topology =
        fs::read_to_string(temp.path().join("src/monitors/topology.rs")).expect("read topology");
    assert!(
        topology.contains(
            "    #[cfg(feature = \"monitor-probe\")]\n    #[cfg_attr(feature = \"monitor-probe\", cfg(target_os = \"macos\"))]\n    use super::monitor_probe::InjectedTopologyProbeRecords;"
        ),
        "expected the nested import to resolve through the parent scope, got:\n{topology}"
    );
    assert_eq!(
        topology
            .matches("Option<InjectedTopologyProbeRecords>")
            .count(),
        2,
        "expected both partial paths to use the imported type, got:\n{topology}"
    );

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--all-features")
        .arg("--tests")
        .output()
        .expect("check fixed fixture with all features");
    assert!(
        check.status.success(),
        "fixed fixture failed with all features: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn synthesized_import_inherits_use_site_cfg() {
    let temp = tempdir().expect("create cfg-gated inline path fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_gated_inline_path_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        "#![deny(warnings)]\nmod watcher;\nfn main() { watcher::go(); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/watcher.rs"),
        r#"use std::sync::mpsc;

pub fn go() { let (_tx, _rx) = mpsc::sync_channel::<()>(1); }

#[cfg(test)]
pub(super) struct TestWatcher {
    pub(super) watcher_sender: mpsc::SyncSender<()>,
}

#[cfg(test)]
mod tests {
    use super::TestWatcher;
    use std::sync::mpsc;

    #[test]
    fn constructs_test_watcher() {
        let (watcher_sender, _watcher_receiver) = mpsc::sync_channel(1);
        let watcher = TestWatcher { watcher_sender };
        let _sender = watcher.watcher_sender;
    }
}
"#,
    )
    .expect("write watcher module");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let watcher =
        fs::read_to_string(temp.path().join("src/watcher.rs")).expect("read fixed watcher");
    assert!(
        watcher.contains("#[cfg(test)]\nuse std::sync::mpsc::SyncSender;"),
        "expected the synthesized import to inherit cfg(test), got:\n{watcher}"
    );
    assert!(
        watcher.contains("watcher_sender: SyncSender<()>"),
        "expected the field type to use the synthesized import, got:\n{watcher}"
    );

    // The successful mend fix validates both normal and test targets.
}

/// A `#[cfg(test)] use` binds the name only in test builds. An ungated
/// occurrence of the same path must stay fully qualified — rewriting it bare
/// leaves non-test builds without an import, and inserting an ungated `use`
/// would collide with the gated one. A gated occurrence is still rewritten
/// through the gated import.
#[test]
fn cfg_test_gated_existing_import_keeps_ungated_path_qualified() {
    let temp = tempdir().expect("create cfg-gated existing import fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "cfg_gated_existing_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod types {
    pub enum Layout {
        Sampleable,
        Other,
    }
}

#[cfg(test)]
use crate::types::Layout;

fn pick() -> u8 {
    match crate::types::Layout::Sampleable {
        crate::types::Layout::Sampleable => 1,
        crate::types::Layout::Other => 2,
    }
}

#[cfg(test)]
fn pick_in_tests() -> crate::types::Layout {
    crate::types::Layout::Other
}

fn main() {
    let _ = pick();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_layout() {
        let _ = Layout::Other;
        let _ = pick_in_tests();
    }
}
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        main_source.contains("match crate::types::Layout::Sampleable"),
        "ungated occurrence must stay qualified when the only import is cfg(test)-gated:\n{main_source}"
    );
    assert!(
        main_source.contains("#[cfg(test)]\nuse crate::types::Layout;"),
        "the gated import must survive unchanged:\n{main_source}"
    );
    assert!(
        main_source.contains("fn pick_in_tests() -> Layout {\n    Layout::Other\n}"),
        "cfg(test)-gated occurrence should be rewritten through the gated import:\n{main_source}"
    );

    // The successful mend fix validates both normal and test targets.
}

/// A nested module that sees `io` only through `use super::*;` must resolve
/// `io::ErrorKind` to the same absolute `std::io::ErrorKind` the file scope
/// resolves to. Before ancestor resolution the nested scope produced
/// `use super::io::ErrorKind;`, the same-name/different-path group was
/// dropped whole, and `--fix` reported the fixes applied while editing
/// nothing.
#[test]
fn glob_visible_parent_import_resolves_to_shared_absolute_use() {
    let temp = tempdir().expect("create glob-visible parent import fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "glob_visible_parent_import_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"use std::io;

fn not_found_kind() -> io::ErrorKind {
    io::ErrorKind::NotFound
}

mod codec {
    use super::*;

    pub fn invalid_kind() -> io::ErrorKind {
        io::ErrorKind::InvalidData
    }
}

fn main() {
    let _ = not_found_kind();
    let _ = codec::invalid_kind();
}
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        main_source.contains("use std::io::ErrorKind;"),
        "both scopes should import the shared absolute path:\n{main_source}"
    );
    assert!(
        !main_source.contains("use super::io::ErrorKind;"),
        "the nested scope must not fall back to a super-qualified import:\n{main_source}"
    );
    assert!(
        main_source.contains("fn not_found_kind() -> ErrorKind {\n    ErrorKind::NotFound\n}"),
        "the file-scope occurrences should be rewritten bare:\n{main_source}"
    );
    assert!(
        main_source
            .contains("fn invalid_kind() -> ErrorKind {\n        ErrorKind::InvalidData\n    }"),
        "the nested occurrences should be rewritten bare:\n{main_source}"
    );
    assert!(
        !main_source.contains("io::ErrorKind::"),
        "no qualified occurrence should remain:\n{main_source}"
    );

    // The successful mend fix already validates this target configuration.
}

#[test]
fn synthesized_import_keeps_only_mixed_cfg_attr_gate() {
    let temp = tempdir().expect("create mixed cfg_attr fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "mixed_cfg_attr_fixture"
version = "0.1.0"
edition = "2024"

[features]
x = []
y = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod types {
    #[derive(Clone)]
    pub struct Thing;
}

#[cfg_attr(feature = "x", cfg(feature = "y"), derive(Clone))]
struct Holder(crate::types::Thing);

fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        main_source.contains(
            "#[cfg_attr(feature = \"x\", cfg(feature = \"y\"))]\nuse crate::types::Thing;"
        ),
        "expected the synthesized import to inherit only the cfg gate:\n{main_source}"
    );
    assert!(
        !main_source.contains(
            "#[cfg_attr(feature = \"x\", cfg(feature = \"y\"), derive(Clone))]\nuse crate::types::Thing;"
        ),
        "derive must not be copied onto the synthesized import:\n{main_source}"
    );

    for cargo_arguments in [
        &["--features", "x"][..],
        &["--features", "y"][..],
        &["--all-features"][..],
    ] {
        let check = cargo_command()
            .arg("check")
            .args(cargo_arguments)
            .arg("--manifest-path")
            .arg(temp.path().join("Cargo.toml"))
            .output()
            .expect("check fixed mixed cfg_attr fixture");
        assert!(
            check.status.success(),
            "fixed fixture failed for cargo arguments {cargo_arguments:?}: {}\n{}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
    }
}

#[test]
fn synthesized_import_inherits_nested_cfg_attr_gate() {
    let temp = tempdir().expect("create nested cfg_attr fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "nested_cfg_attr_fixture"
version = "0.1.0"
edition = "2024"

[features]
a = []
b = []
c = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod types {
    pub struct Thing;
}

#[cfg_attr(feature = "a", cfg_attr(feature = "b", cfg(feature = "c")))]
struct Holder(crate::types::Thing);

fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        main_source.contains(
            "#[cfg_attr(feature = \"a\", cfg_attr(feature = \"b\", cfg(feature = \"c\")))]\nuse crate::types::Thing;"
        ),
        "expected the synthesized import to inherit the nested cfg gate:\n{main_source}"
    );
}

#[test]
fn synthesized_import_ignores_non_gating_cfg_attr() {
    let temp = tempdir().expect("create non-gating cfg_attr fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "non_gating_cfg_attr_fixture"
version = "0.1.0"
edition = "2024"

[features]
decorate = []
outer = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod types {
    #[derive(Clone)]
    pub struct Thing;
}

#[cfg_attr(feature = "outer", cfg_attr(feature = "decorate", derive(Clone)))]
struct Holder(crate::types::Thing);

fn main() {}
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        main_source.contains("use crate::types::Thing;"),
        "expected a synthesized Thing import:\n{main_source}"
    );
    assert!(
        !main_source.contains(
            "#[cfg_attr(feature = \"outer\", cfg_attr(feature = \"decorate\", derive(Clone)))]\nuse crate::types::Thing;"
        ),
        "a nested derive cfg_attr must not be copied onto an import:\n{main_source}"
    );

    let check = cargo_command()
        .arg("check")
        .arg("--manifest-path")
        .arg(temp.path().join("Cargo.toml"))
        .arg("--all-features")
        .output()
        .expect("check fixed fixture with all features");
    assert!(
        check.status.success(),
        "fixed fixture failed with all features: {}\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
}

#[test]
fn synthesized_import_declines_different_cfg_gates() {
    let temp = tempdir().expect("create different cfg gate fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "different_cfg_gate_inline_path_fixture"
version = "0.1.0"
edition = "2024"

[features]
first = []
second = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create fixture source directory");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"use std::sync::mpsc;

fn keep_import_used() { let (_sender, _receiver) = mpsc::sync_channel::<()>(1); }

#[cfg(feature = "first")]
struct FirstWatcher {
    sender: mpsc::SyncSender<()>,
}

#[cfg(feature = "second")]
struct SecondWatcher {
    sender: mpsc::SyncSender<()>,
}

fn main() { keep_import_used(); }
"#,
    )
    .expect("write fixture main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_source =
        fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed fixture");
    assert!(
        !main_source.contains("use std::sync::mpsc::SyncSender;"),
        "different cfg gates must not produce one synthesized import:\n{main_source}"
    );
    assert_eq!(
        main_source.matches("sender: mpsc::SyncSender<()>").count(),
        2,
        "different cfg gates must keep both qualified field types:\n{main_source}"
    );
}

#[test]
fn flags_std_paths() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_std_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"fn main() {
    let _map: std::collections::HashMap<String, i32> = std::collections::HashMap::new();
}
"#,
    )
    .expect("write main");

    let report = run_mend_json(&temp.path().join("Cargo.toml"));
    assert!(
        report
            .findings
            .iter()
            .any(|f| f.code == DiagnosticCode::InlinePathQualifiedType),
        "external-crate paths (std/core/third-party) should be flagged"
    );
}

#[test]
fn renamed_private_import_keeps_qualified_type() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    let source = r#"mod capture {
    pub trait CaptureError {}
}

use std::io;
use capture::CaptureError as Error;

fn completed_capture() -> Result<(), io::Error> {
    Err(io::Error::other("capture failed"))
}

fn boxed_error<T: Error + 'static>(error: T) -> Box<dyn Error> {
    Box::new(error)
}

fn main() {}
"#;
    fs::write(temp.path().join("Cargo.toml"),
        "[package]\nname = \"inline_renamed_collision_fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n")
        .expect("write manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create source directory");
    fs::write(temp.path().join("src/main.rs"), source).expect("write source");
    let manifest_path = temp.path().join("Cargo.toml");

    let output = mend_command_for(&manifest_path)
        .arg("--fix")
        .output()
        .expect("run cargo-mend --fix");
    assert!(
        output.status.success(),
        "cargo-mend --fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed source");
    assert_eq!(
        main_rs,
        source.replacen("pub trait CaptureError", "pub(crate) trait CaptureError", 1)
    );
}

#[test]
fn existing_pub_use_binding_no_duplicate() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_existing_pub_use_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        r#"mod slot {
    pub enum BarSlot {
        Single(u8),
    }
}

pub use slot::BarSlot;

fn is_single(slot: crate::BarSlot) -> bool {
    match slot {
        crate::BarSlot::Single(_) => true,
    }
}

fn main() {
    let _ = is_single(BarSlot::Single(1));
}
"#,
    )
    .expect("write main");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let main_rs = fs::read_to_string(temp.path().join("src/main.rs")).expect("read fixed main");
    assert!(
        !main_rs.contains("use crate::BarSlot;"),
        "pub use already binds BarSlot; fix must not add a duplicate import, got:\n{main_rs}"
    );
    assert!(
        main_rs.contains("BarSlot::Single(_)"),
        "expected inline variant path to be rewritten through existing binding, got:\n{main_rs}"
    );
}

#[test]
fn read_only_and_dry_run_preserve_dirty_and_clean_sources() {
    let mut batch = DiagnosticBatch::new("[visibility]\npub_in_path = \"permitted\"\n");
    let source = "fn example(_x: crate::parent::types::MyType) {}\n";
    let dirty = batch.add_member(
        "dirty",
        &[
            ("src/main.rs", "mod parent;\nfn main() {}\n"),
            ("src/parent.rs", "mod types;\nmod consumer;\n"),
            ("src/parent/types.rs", "pub struct MyType;\n"),
            ("src/parent/consumer.rs", source),
        ],
    );
    let clean = batch.add_member("clean", &[("src/main.rs", "fn main() {}\n")]);
    let report = batch.report();
    assert!(
        findings_at(&report, "dirty/src/parent/consumer.rs")
            .iter()
            .any(|finding| finding.code == DiagnosticCode::InlinePathQualifiedType),
        "read-only mode should report inline_path_qualified_type findings"
    );
    assert!(
        !member_report(&report, "clean")
            .findings
            .iter()
            .any(|finding| finding.code == DiagnosticCode::InlinePathQualifiedType),
        "clean project should not have inline_path_qualified_type findings"
    );
    let output = batch
        .command()
        .args(["--fix", "--dry-run"])
        .output()
        .expect("dry-run batch");
    assert!(
        output.status.success(),
        "batch dry-run failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let consumer = fs::read_to_string(dirty.join("src/parent/consumer.rs"))
        .expect("read consumer after dry-run");
    assert!(
        consumer.contains("crate::parent::types::MyType"),
        "dry-run should not modify files"
    );
    assert_eq!(consumer, source);
    assert_eq!(
        fs::read_to_string(clean.join("src/main.rs")).expect("read clean source"),
        "fn main() {}\n"
    );
}

/// Regression: running `cargo mend --fix` on a file that mixes an enum
/// variant `RustProject::Package(...)`, a same-named struct literal
/// `Package { ... }` (brought in via `use super::*`), and a struct
/// associated-function call `Package::default()` must not introduce an import
/// of the variant `Package` that would shadow the bare struct name.
///
/// This is the cargo-port-uses-cargo-metadata case that motivated the fix.
#[test]
fn enum_variant_and_same_named_struct_do_not_collide() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "enum_struct_collision_fixture"
version = "0.1.0"
edition = "2024"
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src/project")).expect("create src/project");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod project;\nmod consumer;\nfn main() {}\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/project/mod.rs"),
        r#"#[derive(Default)]
pub struct Package {
    pub name: String,
}

pub enum RustProject {
    Package(Package),
}
"#,
    )
    .expect("write project mod");
    // Consumer: uses the enum variant via a fully-qualified path, uses the
    // struct literal with a bare name (imported via `use super::*` in the
    // inner `tests` module), and calls `Package::default()` as a struct
    // associated function — all three mentions share the leaf `Package`.
    fs::write(
        temp.path().join("src/consumer.rs"),
        r#"pub fn keep() {}

#[cfg(test)]
mod tests {
    use crate::project::Package;

    #[test]
    fn build_package() {
        let pkg = crate::project::RustProject::Package(Package {
            name: "demo".into(),
            ..Package::default()
        });
        let _ = pkg;
    }
}
"#,
    )
    .expect("write consumer");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let consumer =
        fs::read_to_string(temp.path().join("src/consumer.rs")).expect("read fixed consumer");

    // `Package::default()` is a struct associated-fn call. prefer_module_import
    // must NOT treat `Package` as a module (filesystem is case-insensitive on
    // macOS/Windows) and must not add a module-style `use crate::project::Package;`.
    assert!(
        !consumer.contains("use crate::project::Package::"),
        "unexpected variant-leaf import introduced, got:\n{consumer}"
    );

    // The enum-variant rewrite must import the parent type, not the variant,
    // so the bare `Package` struct (from the existing `use`) is not shadowed.
    assert!(
        consumer.contains("use crate::project::RustProject;"),
        "expected import of the enum type `RustProject`, got:\n{consumer}"
    );
    assert!(
        consumer.contains("RustProject::Package(Package {"),
        "expected enum-qualified variant, got:\n{consumer}"
    );

    // The successful mend fix already validates this target configuration.
}

/// A `#[cfg]` written on a statement attaches to the statement's expression,
/// not to any item, so the visitor sees the gate only through that expression.
/// Missing it makes the synthesized `use` ungated while the target item stays
/// configured out — E0432 in every build without the feature.
#[test]
fn statement_cfg_gate_reaches_synthesized_import() {
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);

    fs::write(
        temp.path().join("Cargo.toml"),
        r#"[package]
name = "inline_stmt_cfg_fixture"
version = "0.1.0"
edition = "2024"

[features]
test = []
"#,
    )
    .expect("write fixture manifest");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/main.rs"),
        "mod reporter;\nmod plugin;\nfn main() { plugin::build(&mut plugin::App); }\n",
    )
    .expect("write fixture main");
    fs::write(
        temp.path().join("src/reporter.rs"),
        r#"#[cfg(any(test, feature = "test"))]
#[derive(Default)]
pub struct InjectedDisplays {
    pub entities: Vec<u32>,
}
"#,
    )
    .expect("write reporter module");
    fs::write(
        temp.path().join("src/plugin.rs"),
        r#"pub struct App;

impl App {
    pub fn init_resource<T: Default>(&mut self) {}
}

pub fn build(app: &mut App) {
    #[cfg(any(test, feature = "test"))]
    app.init_resource::<crate::reporter::InjectedDisplays>();
}
"#,
    )
    .expect("write plugin module");

    let output = super::rewrite_inline_path_fixture(&temp.path().join("Cargo.toml"));
    assert!(
        output.status.success(),
        "scan, fix and cargo check failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let plugin = fs::read_to_string(temp.path().join("src/plugin.rs")).expect("read fixed plugin");
    assert!(
        plugin.contains(
            "#[cfg(any(test, feature = \"test\"))]\nuse crate::reporter::InjectedDisplays;"
        ),
        "expected the synthesized import to inherit the statement's cfg, got:\n{plugin}"
    );

    {
        let cargo_arguments = ["--features", "test"];
        let check = cargo_command()
            .arg("check")
            .arg("--all-targets")
            .args(cargo_arguments)
            .arg("--manifest-path")
            .arg(temp.path().join("Cargo.toml"))
            .output()
            .expect("check fixed statement-gated fixture");
        assert!(
            check.status.success(),
            "fixed fixture failed for cargo arguments {cargo_arguments:?}: {}\n{}",
            String::from_utf8_lossy(&check.stdout),
            String::from_utf8_lossy(&check.stderr)
        );
    }
}

#[test]
fn fix_cli_rewrites_inline_path_and_validates_sources() {
    let mut batch = DiagnosticBatch::new_crate("[visibility]\npub_in_path = \"permitted\"\n");
    let basic_inline_type_adds_use = basic_inline_type_adds_use(&mut batch);
    batch.single_compile_targets();
    let output = batch.command().arg("--fix").output().expect("fix batch");
    assert!(
        output.status.success(),
        "batch fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    basic_inline_type_adds_use();
}
