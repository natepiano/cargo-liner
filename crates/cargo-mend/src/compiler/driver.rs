use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;

use anyhow::Context;
use anyhow::Error;
use anyhow::Result;
use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

use super::settings::DriverSettings;
use super::visibility;
use crate::reporting::EXIT_CODE_ERROR;

#[derive(Debug)]
struct AnalysisCallbacks {
    settings: DriverSettings,
    error:    Option<Error>,
}

impl AnalysisCallbacks {
    const fn new(settings: DriverSettings) -> Self {
        Self {
            settings,
            error: None,
        }
    }
}

impl Callbacks for AnalysisCallbacks {
    fn after_analysis(&mut self, _: &Compiler, tcx: TyCtxt<'_>) -> Compilation {
        match visibility::collect_and_store_findings(tcx, &self.settings) {
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
        anyhow::bail!("compiler driver expected rustc wrapper arguments");
    }
    let Ok(settings) = DriverSettings::from_env() else {
        return passthrough_to_rustc(&wrapper_args);
    };

    let rustc_args: Vec<String> = std::iter::once("rustc".to_string())
        .chain(
            wrapper_args
                .into_iter()
                .skip(2)
                .map(|arg| arg.to_string_lossy().into_owned()),
        )
        .collect();

    let mut callbacks = AnalysisCallbacks::new(settings);
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

fn passthrough_to_rustc(wrapper_args: &[OsString]) -> Result<ExitCode> {
    let rustc = wrapper_args
        .get(1)
        .context("compiler driver expected rustc path in wrapper arguments")?;
    let status = Command::new(rustc)
        .args(wrapper_args.iter().skip(2))
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to invoke rustc passthrough from mend wrapper")?;
    Ok(exit_code_from_i32(
        status.code().unwrap_or_else(|| i32::from(EXIT_CODE_ERROR)),
    ))
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
