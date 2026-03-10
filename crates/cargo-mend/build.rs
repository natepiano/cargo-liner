#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::io;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

fn main() {
    if let Some(git_hash) = git_commit_hash() {
        println!("cargo:rustc-env=MEND_GIT_HASH={git_hash}");
    }
    if let Some(build_id) = build_id() {
        println!("cargo:rustc-env=MEND_BUILD_ID={build_id}");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    if let Err(error) = configure_unix_rpath() {
        eprintln!("cargo-mend build script failed: {error}");
        std::process::exit(1);
    }
}

fn build_id() -> Option<String> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    Some(duration.as_nanos().to_string())
}

fn git_commit_hash() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    let hash = stdout.trim();
    if hash.is_empty() {
        return None;
    }
    Some(hash.to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn configure_unix_rpath() -> Result<(), String> {
    let sysroot = sysroot_path()?;
    let host = host_triple()?;
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
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sysroot_path() -> Result<PathBuf, String> {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(command_error("failed to run `rustc --print sysroot`"))?;
    if !output.status.success() {
        return Err(format!(
            "`rustc --print sysroot` failed with status {}",
            output.status
        ));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("sysroot output was not UTF-8: {error}"))?;
    Ok(PathBuf::from(stdout.trim()))
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn host_triple() -> Result<String, String> {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(command_error("failed to run `rustc -vV`"))?;
    if !output.status.success() {
        return Err(format!("`rustc -vV` failed with status {}", output.status));
    }

    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("rustc -vV output was not UTF-8: {error}"))?;
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("host: ").map(str::to_string))
        .ok_or_else(|| "`rustc -vV` did not report a host triple".to_string())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn command_error(context: &'static str) -> impl FnOnce(io::Error) -> String {
    move |error| format!("{context}: {error}")
}
