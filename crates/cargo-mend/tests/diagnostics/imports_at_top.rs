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

fn moves_use_from_fn_body_to_file_top(batch: &mut DiagnosticBatch) -> impl FnOnce() + use<> {
    let root = batch.add_module("moves_use_from_fn_body_to_file_top", &[("mod.rs", "")]);
    fs::write(
        root.join("mod.rs"),
        r#"mod child {
    pub struct Movable;
}

fn example() {
    use self::child::Movable;
    let _movable = Movable;
}
"#,
    )
    .expect("write lib.rs");

    move || {
        let lib = fs::read_to_string(root.join("mod.rs")).expect("read lib.rs");
        let moved_count = lib.matches("use self::child::Movable;").count();
        assert_eq!(
            moved_count, 1,
            "expected exactly one `use self::child::Movable;`, got:\n{lib}"
        );
        let top_use_index = lib
            .find("use self::child::Movable;")
            .expect("`use` should appear");
        let fn_index = lib.find("fn example()").expect("fn should still exist");
        assert!(
            top_use_index < fn_index,
            "use should be moved above fn, got:\n{lib}"
        );
    }
}

fn moves_use_in_inline_mod_to_top_of_inline_mod(
    batch: &mut DiagnosticBatch,
) -> impl FnOnce() + use<> {
    let root = batch.add_module(
        "moves_use_in_inline_mod_to_top_of_inline_mod",
        &[("mod.rs", "")],
    );
    fs::write(
        root.join("mod.rs"),
        r#"pub struct Outer;

mod inner {
    fn example() {
        use super::Outer;
        let _outer = Outer;
    }
}
"#,
    )
    .expect("write lib.rs");

    move || {
        let lib = fs::read_to_string(root.join("mod.rs")).expect("read lib.rs");
        let mod_start = lib
            .find("mod inner {")
            .expect("inline mod should still exist");
        let mod_end = lib[mod_start..]
            .find('}')
            .map(|relative| mod_start + relative)
            .expect("closing brace of inline mod");
        let inside_inline_mod = &lib[mod_start..mod_end];
        assert!(
            inside_inline_mod.contains("use super::Outer;"),
            "expected moved use inside inline mod, got body:\n{inside_inline_mod}"
        );
        let above_mod = &lib[..mod_start];
        assert!(
            !above_mod.contains("use super::Outer;"),
            "use should not be moved above the inline mod, got:\n{lib}"
        );
    }
}

#[test]
fn moves_cfg_gated_use_carrying_its_gate() {
    // A `#[cfg]`-gated `use` sitting directly in a fn body (not nested in a
    // gated block) is moved to the file top with its `#[cfg]` carried along,
    // so the import stays conditionally compiled instead of becoming
    // unconditional. The traits live in a submodule so the in-body `use` is
    // load-bearing for the `handle.ext()` call.
    //
    // Both sides of the gate are spelled out because mend compiles the fixture
    // to validate its own fix: a gate that is false on the host would configure
    // the only import out and leave the call with no trait in scope. The two
    // traits share the `ext` method name so exactly one is in scope per target
    // and the call site stays unconditional.
    let temp = tempdir().expect("create temp fixture dir");
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
    write_manifest(temp.path(), "imports_at_top_cfg");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"mod ext {
    pub trait UnixExt {
        fn ext(&self) -> u64;
    }
    pub trait OtherExt {
        fn ext(&self) -> u64;
    }
    impl UnixExt for super::Handle {
        fn ext(&self) -> u64 {
            0
        }
    }
    impl OtherExt for super::Handle {
        fn ext(&self) -> u64 {
            1
        }
    }
}

pub struct Handle;

pub fn call(handle: &Handle) -> u64 {
    #[cfg(unix)]
    use crate::ext::UnixExt;
    #[cfg(not(unix))]
    use crate::ext::OtherExt;
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
    // Each gate travels with its moved import to the file top.
    assert!(
        lib.contains("#[cfg(unix)]\nuse crate::ext::UnixExt;"),
        "unix-gated use should move to the top carrying its #[cfg], got:\n{lib}"
    );
    assert!(
        lib.contains("#[cfg(not(unix))]\nuse crate::ext::OtherExt;"),
        "non-unix-gated use should move to the top carrying its #[cfg], got:\n{lib}"
    );
    // The in-body copies are gone.
    let fn_start = lib.find("pub fn call").expect("fn should still exist");
    assert!(
        !lib[fn_start..].contains("use crate::ext::"),
        "in-body uses should be removed, got:\n{lib}"
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
    pin_pub_in_path(temp.path(), PubInPath::Permitted);
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

fn skips_when_bare_name_collides_with_existing_top_import(
    batch: &mut DiagnosticBatch,
) -> impl FnOnce() + use<> {
    let root = batch.add_module(
        "skips_when_bare_name_collides_with_existing_top_import",
        &[("mod.rs", "")],
    );
    fs::write(
        root.join("mod.rs"),
        r#"mod a {
    pub struct Foo;
}
mod b {
    pub struct Foo;
}

use self::a::Foo;

fn example() {
    use self::b::Foo;
    let _x: Foo = Foo;
}
"#,
    )
    .expect("write lib.rs");

    move || {
        let lib = fs::read_to_string(root.join("mod.rs")).expect("read lib.rs");
        // The in-body `use self::b::Foo;`
        // must stay because moving it would collide with the top-level `use
        // self::a::Foo;`.
        assert!(
            lib.contains("use self::b::Foo;"),
            "colliding in-body use should stay, got:\n{lib}"
        );
    }
}

fn dedupes_when_use_already_at_top(batch: &mut DiagnosticBatch) -> impl FnOnce() + use<> {
    let root = batch.add_module("dedupes_when_use_already_at_top", &[("mod.rs", "")]);
    fs::write(
        root.join("mod.rs"),
        r#"mod a {
    pub struct Foo;
}

use self::a::Foo;

fn example() {
    use self::a::Foo;
    let _foo = Foo;
}
"#,
    )
    .expect("write lib.rs");

    move || {
        let lib = fs::read_to_string(root.join("mod.rs")).expect("read lib.rs");
        let use_count = lib.matches("use self::a::Foo;").count();
        assert_eq!(
            use_count, 1,
            "duplicate in-body use should be deleted; expected one top-level use, got:\n{lib}"
        );
        let fn_index = lib.find("fn example()").expect("fn should still exist");
        let use_index = lib.find("use self::a::Foo;").expect("use should exist");
        assert!(
            use_index < fn_index,
            "remaining use should be the top-level one, got:\n{lib}"
        );
    }
}

#[test]
fn moves_and_deduplicates_uses_in_independent_files() {
    let mut batch = DiagnosticBatch::new_crate("[visibility]\npub_in_path = \"permitted\"\n");
    let moves_use_from_fn_body_to_file_top = moves_use_from_fn_body_to_file_top(&mut batch);
    let moves_use_in_inline_mod_to_top_of_inline_mod =
        moves_use_in_inline_mod_to_top_of_inline_mod(&mut batch);
    let skips_when_bare_name_collides_with_existing_top_import =
        skips_when_bare_name_collides_with_existing_top_import(&mut batch);
    let dedupes_when_use_already_at_top = dedupes_when_use_already_at_top(&mut batch);
    let output = batch.command().arg("--fix").output().expect("fix batch");
    assert!(
        output.status.success(),
        "batch fix failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    moves_use_from_fn_body_to_file_top();
    moves_use_in_inline_mod_to_top_of_inline_mod();
    skips_when_bare_name_collides_with_existing_top_import();
    dedupes_when_use_already_at_top();
}
