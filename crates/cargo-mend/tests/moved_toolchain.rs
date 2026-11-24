//! Check the build-script inputs and output when a toolchain moves.

#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::process::Command;

use anyhow::Context;
use anyhow::Result;

#[path = "../build.rs"]
mod build_script;

const TEST_TOOLCHAIN: &str = "MEND_TEST_MOVED_TOOLCHAIN";
const TEST_HOST: &str = "test-host";
const RUSTC_SHIM: &str = "#!/bin/sh\nreadlink \"$MEND_TEST_MOVED_TOOLCHAIN\"\n";

#[test]
fn moved_toolchain_refreshes_cached_sysroot_and_rpaths() -> Result<()> {
    if env::var_os(TEST_TOOLCHAIN).is_some() {
        build_script::main();
        return Ok(());
    }

    let fixture = tempfile::tempdir()?;
    let root = fixture.path();
    let rustc = root.join("rustc");
    fs::write(&rustc, RUSTC_SHIM)?;
    fs::set_permissions(&rustc, fs::Permissions::from_mode(0o755))?;
    let old_sysroot = root.join("toolchain-old");
    let new_sysroot = root.join("toolchain-new");
    fs::create_dir_all(old_sysroot.join("bin"))?;
    let old_rustc = old_sysroot.join("bin/rustc");
    fs::write(&old_rustc, "")?;
    let toolchain = root.join("toolchain");
    symlink(&old_sysroot, &toolchain)?;

    let mut build_script = Command::new(env::current_exe()?);
    build_script
        .current_dir(root)
        .args([
            "--exact",
            "moved_toolchain_refreshes_cached_sysroot_and_rpaths",
            "--nocapture",
        ])
        .env("RUSTC", rustc)
        .env("HOST", TEST_HOST)
        .env(TEST_TOOLCHAIN, &toolchain);
    let original = run_build_script(&mut build_script)?;
    assert_sysroot_directives(&original, &old_sysroot);

    fs::rename(&old_sysroot, &new_sysroot)?;
    fs::remove_file(&toolchain)?;
    symlink(&new_sysroot, &toolchain)?;
    assert!(!old_rustc.try_exists()?);
    assert_directive(
        &original,
        &format!("cargo:rerun-if-changed={}", old_rustc.display()),
    );

    let rebuilt = run_build_script(&mut build_script)?;
    assert_sysroot_directives(&rebuilt, &new_sysroot);
    assert!(!rebuilt.contains(old_sysroot.to_str().context("old sysroot path")?));
    Ok(())
}

fn run_build_script(command: &mut Command) -> Result<String> {
    let output = command.output().context("run the build script")?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).context("read build-script directives")
}

fn assert_sysroot_directives(output: &str, sysroot: &Path) {
    let library = sysroot.join("lib");
    let rustlib = library.join("rustlib").join(TEST_HOST).join("lib");
    assert_directive(
        output,
        &format!("cargo:rustc-env=MEND_BUILD_SYSROOT={}", sysroot.display()),
    );
    assert_directive(
        output,
        &format!("cargo:rustc-link-search=native={}", library.display()),
    );
    for path in [library, rustlib] {
        assert_directive(
            output,
            &format!("cargo:rustc-link-arg=-Wl,-rpath,{}", path.display()),
        );
    }
}

fn assert_directive(output: &str, directive: &str) {
    assert!(
        output.lines().any(|line| line == directive),
        "Missing {directive} in:\n{output}"
    );
}
