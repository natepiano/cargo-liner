use std::path::PathBuf;
use std::process::Command;

fn main() {
    #[cfg(target_os = "macos")]
    configure_macos_rpath();
}

#[cfg(target_os = "macos")]
fn configure_macos_rpath() {
    let sysroot = sysroot_path();
    let host = host_triple();
    let rustc_lib_dir = sysroot.join("lib");
    let target_lib_dir = sysroot.join("lib").join("rustlib").join(host).join("lib");

    println!(
        "cargo:rustc-link-arg=-Wl,-rpath,{}",
        rustc_lib_dir.display()
    );
    println!(
        "cargo:rustc-link-arg=-Wl,-rpath,{}",
        target_lib_dir.display()
    );
}

#[cfg(target_os = "macos")]
fn sysroot_path() -> PathBuf {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("failed to run `rustc --print sysroot`");
    assert!(
        output.status.success(),
        "`rustc --print sysroot` failed with status {}",
        output.status
    );
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("sysroot output was not UTF-8")
            .trim(),
    )
}

#[cfg(target_os = "macos")]
fn host_triple() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("failed to run `rustc -vV`");
    assert!(
        output.status.success(),
        "`rustc -vV` failed with status {}",
        output.status
    );
    String::from_utf8(output.stdout)
        .expect("rustc -vV output was not UTF-8")
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .expect("rustc -vV did not report a host triple")
}
