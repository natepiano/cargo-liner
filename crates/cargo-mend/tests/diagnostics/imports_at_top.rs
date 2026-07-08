use crate::support::*;

fn write_manifest(dir: &std::path::Path, package_name: &str) {
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2024"
"#
        ),
    )
    .expect("write fixture manifest");
}

#[test]
fn moves_use_from_fn_body_to_file_top() {
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_basic");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod child {
    pub struct Movable;
}

fn example() {
    use crate::child::Movable;
    let _movable = Movable;
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    let moved_count = lib.matches("use crate::child::Movable;").count();
    assert_eq!(
        moved_count, 1,
        "expected exactly one `use crate::child::Movable;`, got:\n{lib}"
    );
    let top_use_index = lib
        .find("use crate::child::Movable;")
        .expect("`use` should appear");
    let fn_index = lib.find("fn example()").expect("fn should still exist");
    assert!(
        top_use_index < fn_index,
        "use should be moved above fn, got:\n{lib}"
    );
}

#[test]
fn moves_use_in_inline_mod_to_top_of_inline_mod() {
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_inline_mod");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"pub struct Outer;

mod inner {
    fn example() {
        use crate::Outer;
        let _outer = Outer;
    }
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    let mod_start = lib
        .find("mod inner {")
        .expect("inline mod should still exist");
    let mod_end = lib[mod_start..]
        .find('}')
        .map(|relative| mod_start + relative)
        .expect("closing brace of inline mod");
    let inside_inline_mod = &lib[mod_start..mod_end];
    assert!(
        inside_inline_mod.contains("use crate::Outer;"),
        "expected moved use inside inline mod, got body:\n{inside_inline_mod}"
    );
    let above_mod = &lib[..mod_start];
    assert!(
        !above_mod.contains("use crate::Outer;"),
        "use should not be moved above the inline mod, got:\n{lib}"
    );
}

#[test]
fn moves_cfg_gated_use_carrying_its_gate() {
    // A `#[cfg]`-gated `use` sitting directly in a fn body (not nested in a
    // gated block) is moved to the file top with its `#[cfg]` carried along,
    // so the import stays conditionally compiled instead of becoming
    // unconditional. The trait lives in a submodule so the in-body `use` is
    // load-bearing for the `handle.ext()` call.
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_cfg");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod ext {
    pub trait Ext {
        fn ext(&self) -> u64;
    }
    impl Ext for super::Handle {
        fn ext(&self) -> u64 {
            0
        }
    }
}

pub struct Handle;

pub fn call(handle: &Handle) -> u64 {
    #[cfg(unix)]
    use crate::ext::Ext;
    handle.ext()
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    // The gate travels with the moved import to the file top.
    assert!(
        lib.contains("#[cfg(unix)]\nuse crate::ext::Ext;"),
        "gated use should move to the top carrying its #[cfg], got:\n{lib}"
    );
    // The in-body copy is gone.
    let fn_start = lib.find("pub fn call").expect("fn should still exist");
    assert!(
        !lib[fn_start..].contains("use crate::ext::Ext;"),
        "in-body use should be removed, got:\n{lib}"
    );
}

#[test]
fn moves_use_from_cfg_gated_block_carrying_the_gate() {
    // The winit platform pattern: each in-body `use` lives inside a
    // `#[cfg]`-gated `let` block. The `#[cfg]` sits on the enclosing `let`, not
    // on the `use`. Each import moves to the file top carrying the enclosing
    // gate, so it stays conditionally compiled; the gated `let` block stays put
    // (minus the `use`). Traits live in a submodule so the moved imports are
    // not redundant self-imports at the crate root.
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_cfg_block");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod platform {
    pub trait MacNativeId {
        fn mac_native_id(&self) -> u64;
    }
    pub trait OtherNativeId {
        fn other_native_id(&self) -> u64;
    }
    impl MacNativeId for super::Handle {
        fn mac_native_id(&self) -> u64 {
            0
        }
    }
    impl OtherNativeId for super::Handle {
        fn other_native_id(&self) -> u64 {
            0
        }
    }
}

pub struct Handle;

pub fn native_id(handle: &Handle) -> u64 {
    #[cfg(target_os = "macos")]
    let raw = {
        use crate::platform::MacNativeId;
        handle.mac_native_id()
    };
    #[cfg(not(target_os = "macos"))]
    let raw = {
        use crate::platform::OtherNativeId;
        handle.other_native_id()
    };
    raw
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    // Each import moves to the top with the enclosing block's gate carried onto
    // it, so it stays configured out on the non-matching target.
    assert!(
        lib.contains("#[cfg(target_os = \"macos\")]\nuse crate::platform::MacNativeId;"),
        "macos-gated use should move carrying its gate, got:\n{lib}"
    );
    assert!(
        lib.contains("#[cfg(not(target_os = \"macos\"))]\nuse crate::platform::OtherNativeId;"),
        "other-gated use should move carrying its gate, got:\n{lib}"
    );
    // The gated `let` blocks stay in the fn; only the `use` lines left them.
    let fn_start = lib.find("pub fn native_id").expect("fn should still exist");
    assert!(
        !lib[fn_start..].contains("use crate::platform::"),
        "in-body uses should be removed from the fn body, got:\n{lib}"
    );
}

#[test]
fn skips_when_bare_name_collides_with_existing_top_import() {
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_collision");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod a {
    pub struct Foo;
}
mod b {
    pub struct Foo;
}

use crate::a::Foo;

fn example() {
    use crate::b::Foo;
    let _x: Foo = Foo;
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    // The in-body `use crate::b::Foo;` must stay because moving it would
    // collide with the top-level `use crate::a::Foo;`.
    assert!(
        lib.contains("use crate::b::Foo;"),
        "colliding in-body use should stay, got:\n{lib}"
    );
}

#[test]
fn dedupes_when_use_already_at_top() {
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_dedupe");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod a {
    pub struct Foo;
}

use crate::a::Foo;

fn example() {
    use crate::a::Foo;
    let _foo = Foo;
}
"#,
    )
    .expect("write lib.rs");

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

    let lib = fs::read_to_string(temp.path().join("src/lib.rs")).expect("read lib.rs");
    let use_count = lib.matches("use crate::a::Foo;").count();
    assert_eq!(
        use_count, 1,
        "duplicate in-body use should be deleted; expected one top-level use, got:\n{lib}"
    );
    let fn_index = lib.find("fn example()").expect("fn should still exist");
    let use_index = lib.find("use crate::a::Foo;").expect("use should exist");
    assert!(
        use_index < fn_index,
        "remaining use should be the top-level one, got:\n{lib}"
    );
}
