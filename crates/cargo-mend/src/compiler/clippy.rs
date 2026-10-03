use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::process::ExitCode;

use anyhow::Result;
use rustc_interface::interface::Config;
use serde::Serialize;

use super::constants::CLIPPY_STATUS_UNKNOWN_RUSTC;
use super::constants::RUSTC_BIN;
use super::constants::RUSTC_ENV;
use super::constants::RUSTC_FLAG_VERSION;
use crate::reporting::EXIT_CODE_ERROR;

/// The `rustc -V` line of the compiler this build links, recorded by
/// `build.rs`. Linked clippy lints were compiled against the same compiler.
const BUILD_RUSTC_VERSION: Option<&str> = option_env!("MEND_BUILD_RUSTC_VERSION");

/// Clippy's lints, linked into a build of cargo-mend by the `cargo-mend-clippy`
/// package.
///
/// cargo-mend cannot depend on rust-clippy itself: rust-clippy is published
/// only as git tags, and crates.io refuses a package with a git dependency.
/// The compiler driver calls these two functions for every workspace member it
/// analyzes, which mirrors what `clippy-driver` does for the same member.
#[derive(Clone, Copy, Debug)]
pub struct ClippyLints {
    /// Appends `--cfg clippy` and the flags in `CLIPPY_ARGS` to the member's
    /// rustc arguments.
    pub extend_rustc_args: fn(&mut Vec<String>),
    /// Registers clippy's lint passes on the compiler configuration and sets
    /// the MIR options clippy's MIR lints need.
    pub configure:         fn(&mut Config),
}

/// Whether a run checks workspace members with clippy's lints, as reported by
/// `cargo mend --clippy-status`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case", tag = "clippy")]
pub(crate) enum ClippyStatus {
    /// This build links clippy's lints, compiled against the host rustc.
    Active { rustc: String },
    /// This build does not link clippy's lints.
    Absent,
    /// This build links clippy's lints, compiled against a rustc other than
    /// the host's, so runs skip them.
    RustcMismatch {
        built_for: String,
        host:      String,
    },
}

impl ClippyStatus {
    /// Compares the rustc that `clippy_lints` was compiled against with the
    /// host rustc that cargo will run.
    pub(crate) fn detect(clippy_lints: Option<ClippyLints>) -> Self {
        if clippy_lints.is_none() {
            return Self::Absent;
        }
        let built_for = BUILD_RUSTC_VERSION.unwrap_or(CLIPPY_STATUS_UNKNOWN_RUSTC);
        match host_rustc_version() {
            Some(host) if host == built_for => Self::Active { rustc: host },
            host => Self::RustcMismatch {
                built_for: built_for.to_string(),
                host:      host.unwrap_or_else(|| CLIPPY_STATUS_UNKNOWN_RUSTC.to_string()),
            },
        }
    }

    pub(crate) const fn is_active(&self) -> bool { matches!(self, Self::Active { .. }) }

    pub(crate) fn to_json(&self) -> Result<String> { Ok(serde_json::to_string(self)?) }

    pub(crate) fn exit_code(&self) -> ExitCode {
        if self.is_active() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(EXIT_CODE_ERROR)
        }
    }

    /// Prints to stderr that the run skips the clippy lints it links, when the
    /// rustc they were built for is not the host's.
    pub(crate) fn print_skipped_notice(&self) {
        match self {
            Self::RustcMismatch { built_for, host } => {
                eprintln!("mend: skipping clippy lints: built for `{built_for}`, host is `{host}`");
            },
            Self::Active { .. } | Self::Absent => {},
        }
    }
}

fn host_rustc_version() -> Option<String> {
    let rustc = env::var_os(RUSTC_ENV).unwrap_or_else(|| OsString::from(RUSTC_BIN));
    let output = Command::new(rustc).arg(RUSTC_FLAG_VERSION).output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|stdout| stdout.trim().to_string())
}
