use crate::common::*;

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
fn lifts_use_from_fn_body_to_file_top() {
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
    let lifted_count = lib.matches("use crate::child::Movable;").count();
    assert_eq!(
        lifted_count, 1,
        "expected exactly one `use crate::child::Movable;`, got:\n{lib}"
    );
    let top_use_index = lib
        .find("use crate::child::Movable;")
        .expect("`use` should appear");
    let fn_index = lib.find("fn example()").expect("fn should still exist");
    assert!(
        top_use_index < fn_index,
        "use should be lifted above fn, got:\n{lib}"
    );
}

#[test]
fn lifts_use_in_inline_mod_to_top_of_inline_mod() {
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
        "expected lifted use inside inline mod, got body:\n{inside_inline_mod}"
    );
    let above_mod = &lib[..mod_start];
    assert!(
        !above_mod.contains("use crate::Outer;"),
        "use should not be lifted above the inline mod, got:\n{lib}"
    );
}

#[test]
fn preserves_cfg_gated_use() {
    let temp = tempdir().expect("create temp fixture dir");
    write_manifest(temp.path(), "imports_at_top_cfg");
    fs::create_dir_all(temp.path().join("src")).expect("create src");
    fs::write(
        temp.path().join("src/lib.rs"),
        r#"pub struct Inner;

fn example() {
    #[cfg(test)]
    use crate::Inner;
    let _inner: () = ();
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
    assert!(
        lib.contains("#[cfg(test)]\n    use crate::Inner;"),
        "attributed use should stay in place, got:\n{lib}"
    );
    let top_uses = lib
        .lines()
        .take_while(|line| !line.contains("fn example"))
        .filter(|line| line.trim_start().starts_with("use crate::Inner"))
        .count();
    assert_eq!(
        top_uses, 0,
        "no top-level lift should have happened, got:\n{lib}"
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
    // The in-body `use crate::b::Foo;` must stay because lifting it would
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
