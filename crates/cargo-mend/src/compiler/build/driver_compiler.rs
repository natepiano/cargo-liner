//! Select the compiler that matches the libraries linked into mend.

use std::env::consts::EXE_SUFFIX;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::ensure;

use crate::compiler::constants::PASSTHROUGH_RUSTC_WRAPPER_ENV;
use crate::compiler::constants::RUSTC_BIN;
use crate::compiler::constants::RUSTC_ENV;
use crate::compiler::constants::RUSTC_WORKSPACE_WRAPPER_ENV;

/// The compiler shared by all driver commands in one mend process.
pub(super) struct DriverCompiler {
    rustc:             PathBuf,
    #[cfg(target_os = "linux")]
    runtime_libraries: PathBuf,
}

impl DriverCompiler {
    pub(super) fn for_build() -> Result<&'static Self> {
        static COMPILER: LazyLock<Result<DriverCompiler>> = LazyLock::new(|| {
            let sysroot =
                option_env!("MEND_BUILD_SYSROOT").context("mend has no recorded build sysroot")?;
            DriverCompiler::from_sysroot(Path::new(sysroot))
        });
        COMPILER.as_ref().map_err(|error| anyhow!("{error:#}"))
    }

    fn from_sysroot(sysroot: &Path) -> Result<Self> {
        let rustc = sysroot.join("bin").join(format!("{RUSTC_BIN}{EXE_SUFFIX}"));
        ensure!(
            rustc.is_file(),
            "mend cannot find rustc in its build sysroot: {} (expected {})",
            sysroot.display(),
            rustc.display()
        );
        Ok(Self {
            rustc,
            #[cfg(target_os = "linux")]
            runtime_libraries: sysroot.join("lib"),
        })
    }

    /// Set the compiler and mend's workspace wrapper together.
    ///
    /// Cargo records the workspace wrapper in each unit's fingerprint.
    /// This lets the build-specific wrapper alias trigger fresh analysis after a mend update.
    /// The user's `RUSTC_WRAPPER` remains active for dependency builds.
    pub(super) fn configure(&self, command: &mut Command, wrapper: &Path) {
        command
            .env(RUSTC_ENV, &self.rustc)
            .env(RUSTC_WORKSPACE_WRAPPER_ENV, wrapper)
            .env_remove(PASSTHROUGH_RUSTC_WRAPPER_ENV);
    }

    #[cfg(target_os = "linux")]
    pub(super) fn configure_runtime_libraries(&self, command: &mut Command) -> Result<()> {
        // Cargo adds rustlib before RUNPATH. Its driver needs the matching top-level libraries.
        let inherited = command
            .get_envs()
            .find(|(key, _)| *key == "LD_LIBRARY_PATH")
            .map_or_else(
                || std::env::var_os("LD_LIBRARY_PATH"),
                |(_, value)| value.map(std::ffi::OsStr::to_os_string),
            );
        let paths = std::iter::once(self.runtime_libraries.clone()).chain(
            inherited
                .as_deref()
                .into_iter()
                .flat_map(std::env::split_paths),
        );
        command.env(
            "LD_LIBRARY_PATH",
            std::env::join_paths(paths).context("cannot set the compiler library path")?,
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::env::consts::EXE_SUFFIX;
    use std::ffi::OsStr;
    #[cfg(unix)]
    use std::io::Write;
    use std::path::Path;
    use std::process::Command;
    #[cfg(unix)]
    use std::process::Stdio;

    use anyhow::Result;

    use super::DriverCompiler;
    use crate::compiler::constants::CARGO_BIN;
    use crate::compiler::constants::CARGO_SUBCOMMAND_CHECK;
    use crate::compiler::constants::PASSTHROUGH_RUSTC_WRAPPER_ENV;
    use crate::compiler::constants::RUSTC_BIN;
    use crate::compiler::constants::RUSTC_ENV;
    use crate::compiler::constants::RUSTC_WORKSPACE_WRAPPER_ENV;

    // Exceed a pipe buffer so a loader failure cannot appear as a successful write.
    #[cfg(unix)]
    const PROBE_PAYLOAD_SIZE: usize = 256 * 1024;

    #[test]
    fn driver_command_uses_build_compiler_and_wrapper_without_passthrough() -> Result<()> {
        let compiler = DriverCompiler::for_build()?;
        let expected = Path::new(env!("MEND_BUILD_SYSROOT"))
            .join("bin")
            .join(format!("{RUSTC_BIN}{EXE_SUFFIX}"));
        let wrapper = Path::new("mend-driver");
        let mut command = Command::new(CARGO_BIN);
        command
            .arg(CARGO_SUBCOMMAND_CHECK)
            .env(RUSTC_ENV, "project-rustc")
            .env(RUSTC_WORKSPACE_WRAPPER_ENV, "project-workspace-wrapper")
            .env(PASSTHROUGH_RUSTC_WRAPPER_ENV, "project-wrapper");
        compiler.configure(&mut command, wrapper);
        assert_eq!(
            command.get_envs().find(|(key, _)| *key == RUSTC_ENV),
            Some((OsStr::new(RUSTC_ENV), Some(expected.as_os_str())))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == RUSTC_WORKSPACE_WRAPPER_ENV),
            Some((
                OsStr::new(RUSTC_WORKSPACE_WRAPPER_ENV),
                Some(wrapper.as_os_str())
            ))
        );
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == PASSTHROUGH_RUSTC_WRAPPER_ENV),
            Some((OsStr::new(PASSTHROUGH_RUSTC_WRAPPER_ENV), None))
        );
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn compiler_library_path_preserves_command_paths() -> Result<()> {
        let compiler = DriverCompiler::for_build()?;
        let existing = [Path::new("project-libraries"), Path::new("cache-libraries")];
        let mut command = Command::new(CARGO_BIN);
        command.env("LD_LIBRARY_PATH", std::env::join_paths(existing)?);
        compiler.configure_runtime_libraries(&mut command)?;
        let path = command
            .get_envs()
            .find_map(|(key, value)| (key == "LD_LIBRARY_PATH").then_some(value).flatten())
            .ok_or_else(|| anyhow::anyhow!("compiler library path"))?;
        assert_eq!(
            std::env::split_paths(path).collect::<Vec<_>>(),
            [
                Path::new(env!("MEND_BUILD_SYSROOT")).join("lib"),
                existing[0].to_path_buf(),
                existing[1].to_path_buf(),
            ]
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn build_script_starts_compiler_with_its_runtime_libraries() -> Result<()> {
        let mut command = build_script_command()?;
        command.arg("exec \"$RUSTC\" --version");
        let output = command.output()?;
        assert!(
            output.status.success(),
            "build script starts rustc: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("rustc "));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn build_script_compiler_probe_reads_stdin() -> Result<()> {
        let fixture = tempfile::tempdir()?;
        let mut command = build_script_command()?;
        command
            .arg("exec \"$RUSTC\" --crate-type=rlib --emit=metadata -o \"$1\" -")
            .arg("build-script")
            .arg(fixture.path().join("probe.rmeta"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn()?;
        let mut input = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("probe stdin"))?;
        let source = " ".repeat(PROBE_PAYLOAD_SIZE) + "pub fn probe() {}\n";
        let written = input.write_all(source.as_bytes());
        drop(input);
        let output = child.wait_with_output()?;
        assert!(
            output.status.success(),
            "compiler probe succeeds: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        written?;
        assert!(fixture.path().join("probe.rmeta").is_file());
        Ok(())
    }

    #[cfg(unix)]
    fn build_script_command() -> Result<Command> {
        let sysroot = Path::new(env!("MEND_BUILD_SYSROOT"));
        let compiler = DriverCompiler::for_build()?;
        let version = Command::new(&compiler.rustc).arg("-vV").output()?;
        let version = String::from_utf8(version.stdout)?;
        let host = version
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .ok_or_else(|| anyhow::anyhow!("rustc host"))?;
        let mut command = Command::new("sh");
        command.arg("-c");
        // Cargo prepends this directory when it starts build scripts and wrappers.
        #[cfg(target_os = "linux")]
        command.env(
            "LD_LIBRARY_PATH",
            sysroot.join("lib/rustlib").join(host).join("lib"),
        );
        #[cfg(target_os = "macos")]
        command.env(
            "DYLD_LIBRARY_PATH",
            sysroot.join("lib/rustlib").join(host).join("lib"),
        );
        #[cfg(target_os = "linux")]
        compiler.configure_runtime_libraries(&mut command)?;
        compiler.configure(&mut command, Path::new("unused-workspace-wrapper"));
        Ok(command)
    }

    #[test]
    fn missing_build_compiler_reports_its_sysroot() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let result = DriverCompiler::from_sysroot(directory.path());
        let error = result
            .err()
            .ok_or_else(|| anyhow::anyhow!("missing rustc must fail"))?;
        assert!(
            error
                .to_string()
                .contains(&directory.path().display().to_string())
        );
        assert!(error.to_string().contains("cannot find rustc"));
        Ok(())
    }
}
