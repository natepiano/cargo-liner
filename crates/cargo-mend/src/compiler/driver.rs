use std::env;
use std::ffi::OsString;
use std::iter;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use anyhow::bail;
use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

use super::constants::CARGO_PRIMARY_PACKAGE_ENV;
use super::constants::PASSTHROUGH_RUSTC_WRAPPER_ENV;
use super::constants::RUSTC_BIN;
use super::settings::DriverSettings;
use super::visibility;
use crate::reporting::EXIT_CODE_ERROR;

#[derive(Debug)]
struct AnalysisCallbacks {
    driver_settings: DriverSettings,
    error:           Option<Error>,
}

impl AnalysisCallbacks {
    const fn new(driver_settings: DriverSettings) -> Self {
        Self {
            driver_settings,
            error: None,
        }
    }
}

impl Callbacks for AnalysisCallbacks {
    fn after_analysis(&mut self, _: &Compiler, tcx: TyCtxt<'_>) -> Compilation {
        match visibility::collect_and_store_findings(tcx, &self.driver_settings) {
            Ok(true | false) => Compilation::Continue,
            Err(err) => {
                self.error = Some(err);
                Compilation::Stop
            },
        }
    }
}

pub(crate) fn driver_main() -> ExitCode {
    match driver_main_impl() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("mend: {err:#}");
            ExitCode::from(EXIT_CODE_ERROR)
        },
    }
}

fn driver_main_impl() -> Result<ExitCode> {
    let wrapper_args: Vec<OsString> = env::args_os().collect();
    if wrapper_args.len() < 2 {
        bail!("compiler driver expected rustc wrapper arguments");
    }
    let Ok(driver_settings) = DriverSettings::from_env() else {
        return passthrough_to_rustc(&wrapper_args);
    };
    if should_passthrough_with_existing_wrapper() {
        return passthrough_to_rustc(&wrapper_args);
    }

    let rustc_args: Vec<String> = iter::once(RUSTC_BIN.to_string())
        .chain(
            wrapper_args
                .into_iter()
                .skip(2)
                .map(|arg| arg.to_string_lossy().into_owned()),
        )
        .collect();

    let mut callbacks = AnalysisCallbacks::new(driver_settings);
    let compiler_exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&rustc_args, &mut callbacks);
    })
    .into_exit_code();

    let exit_code = callbacks.error.map_or(compiler_exit_code, |err| {
        eprintln!("mend: {err:#}");
        ExitCode::FAILURE
    });

    Ok(exit_code)
}

fn should_passthrough_with_existing_wrapper() -> bool {
    passthrough_rustc_wrapper().is_some() && env::var_os(CARGO_PRIMARY_PACKAGE_ENV).is_none()
}

fn passthrough_to_rustc(wrapper_args: &[OsString]) -> Result<ExitCode> {
    let invocation = PassthroughInvocation::new(wrapper_args, passthrough_rustc_wrapper())?;
    let status = invocation
        .command()
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to invoke rustc passthrough from mend wrapper")?;
    Ok(exit_code_from_i32(
        status.code().unwrap_or_else(|| i32::from(EXIT_CODE_ERROR)),
    ))
}

#[derive(Debug, PartialEq, Eq)]
struct PassthroughInvocation {
    program: OsString,
    args:    Vec<OsString>,
}

impl PassthroughInvocation {
    fn new(wrapper_args: &[OsString], rustc_wrapper: Option<OsString>) -> Result<Self> {
        let rustc = wrapper_args
            .get(1)
            .context("compiler driver expected rustc path in wrapper arguments")?;
        let rustc_args = wrapper_args.iter().skip(2).cloned();

        let invocation = match rustc_wrapper {
            Some(program) => Self {
                program,
                args: iter::once(rustc.clone()).chain(rustc_args).collect(),
            },
            None => Self {
                program: rustc.clone(),
                args:    rustc_args.collect(),
            },
        };
        Ok(invocation)
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        command
    }
}

fn passthrough_rustc_wrapper() -> Option<OsString> {
    env::var_os(PASSTHROUGH_RUSTC_WRAPPER_ENV).filter(|value| !value.is_empty())
}

/// Compatibility trait for `rustc_driver::catch_with_exit_code` which returns
/// `i32` on stable 1.94 and `ExitCode` from 1.95+ (PR #150379).
trait IntoExitCode {
    fn into_exit_code(self) -> ExitCode;
}

impl IntoExitCode for i32 {
    fn into_exit_code(self) -> ExitCode {
        ExitCode::from(u8::try_from(self).unwrap_or(EXIT_CODE_ERROR))
    }
}

impl IntoExitCode for ExitCode {
    fn into_exit_code(self) -> ExitCode { self }
}

fn exit_code_from_i32(code: i32) -> ExitCode {
    let normalized_code = u8::try_from(code).unwrap_or(EXIT_CODE_ERROR);
    ExitCode::from(normalized_code)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use anyhow::Result;

    use super::PassthroughInvocation;

    #[test]
    fn passthrough_invocation_uses_rustc_without_existing_wrapper() -> Result<()> {
        let wrapper_args = wrapper_args();

        let invocation = PassthroughInvocation::new(&wrapper_args, None)?;

        assert_eq!(
            invocation,
            PassthroughInvocation {
                program: OsString::from("/toolchain/bin/rustc"),
                args:    vec![OsString::from("-vV")],
            }
        );
        Ok(())
    }

    #[test]
    fn passthrough_invocation_chains_existing_wrapper_before_rustc() -> Result<()> {
        let wrapper_args = wrapper_args();

        let invocation =
            PassthroughInvocation::new(&wrapper_args, Some(OsString::from("/cache/kache")))?;

        assert_eq!(
            invocation,
            PassthroughInvocation {
                program: OsString::from("/cache/kache"),
                args:    vec![
                    OsString::from("/toolchain/bin/rustc"),
                    OsString::from("-vV")
                ],
            }
        );
        Ok(())
    }

    fn wrapper_args() -> Vec<OsString> {
        vec![
            OsString::from("/target/debug/cargo-mend"),
            OsString::from("/toolchain/bin/rustc"),
            OsString::from("-vV"),
        ]
    }
}
