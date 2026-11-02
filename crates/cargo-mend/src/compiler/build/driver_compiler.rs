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
    rustc: PathBuf,
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
        Ok(Self { rustc })
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
}

#[cfg(test)]
mod tests {
    use std::env::consts::EXE_SUFFIX;
    use std::ffi::OsStr;
    use std::path::Path;
    use std::process::Command;

    use anyhow::Result;

    use super::DriverCompiler;
    use crate::compiler::constants::CARGO_BIN;
    use crate::compiler::constants::CARGO_SUBCOMMAND_CHECK;
    use crate::compiler::constants::PASSTHROUGH_RUSTC_WRAPPER_ENV;
    use crate::compiler::constants::RUSTC_BIN;
    use crate::compiler::constants::RUSTC_ENV;
    use crate::compiler::constants::RUSTC_WORKSPACE_WRAPPER_ENV;

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
