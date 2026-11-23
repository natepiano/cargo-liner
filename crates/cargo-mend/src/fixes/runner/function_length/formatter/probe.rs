use std::path::PathBuf;
use std::process::Command;
use std::process::Output;
use std::sync::OnceLock;

use crate::fixes::runner::function_length::constants::NIGHTLY_RUSTFMT_ARGUMENTS;
use crate::fixes::runner::function_length::constants::NIGHTLY_RUSTFMT_LOOKUP_ARGUMENTS;
use crate::fixes::runner::function_length::constants::RUSTUP_COMMAND;
use crate::fixes::runner::function_length::constants::VERSION_ARGUMENT;

#[derive(Debug)]
pub(super) enum FormatterProbe {
    Available,
    Unavailable(String),
}

enum NightlyExecutable {
    Direct(PathBuf),
    Rustup,
}

fn nightly_executable() -> &'static NightlyExecutable {
    static EXECUTABLE: OnceLock<NightlyExecutable> = OnceLock::new();
    EXECUTABLE.get_or_init(|| {
        match Command::new(RUSTUP_COMMAND)
            .args(NIGHTLY_RUSTFMT_LOOKUP_ARGUMENTS)
            .output()
        {
            Ok(output) if output.status.success() => {
                let path = String::from_utf8_lossy(&output.stdout);
                if path.trim().is_empty() {
                    NightlyExecutable::Rustup
                } else {
                    NightlyExecutable::Direct(PathBuf::from(path.trim()))
                }
            },
            _ => NightlyExecutable::Rustup,
        }
    })
}

pub(super) fn nightly_command() -> Command {
    match nightly_executable() {
        NightlyExecutable::Direct(path) => Command::new(path),
        NightlyExecutable::Rustup => {
            let mut command = Command::new(RUSTUP_COMMAND);
            command.args(NIGHTLY_RUSTFMT_ARGUMENTS);
            command
        },
    }
}

/// `rustup which` names the nightly `rustfmt` file only when the toolchain
/// has the component, so an existing file needs no `--version` run. Each
/// `rustfmt` start loads the compiler library, which is the cost this saves.
pub(super) fn nightly_probe() -> FormatterProbe {
    match nightly_executable() {
        NightlyExecutable::Direct(path) if path.is_file() => FormatterProbe::Available,
        NightlyExecutable::Direct(_) | NightlyExecutable::Rustup => probe_result(
            nightly_command().arg(VERSION_ARGUMENT).output(),
            "nightly rustfmt",
            "nightly rustfmt",
        ),
    }
}

pub(super) fn probe(program: &str, arguments: &[&str]) -> FormatterProbe {
    probe_result(
        Command::new(program)
            .args(arguments)
            .arg(VERSION_ARGUMENT)
            .output(),
        &format!("{program} {}", arguments.join(" ")),
        program,
    )
}

fn probe_result(
    output: std::io::Result<Output>,
    failure_label: &str,
    error_label: &str,
) -> FormatterProbe {
    match output {
        Ok(output) if output.status.success() => FormatterProbe::Available,
        Ok(output) => FormatterProbe::Unavailable(format!(
            "{failure_label}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(error) => FormatterProbe::Unavailable(format!("{error_label}: {error}")),
    }
}
