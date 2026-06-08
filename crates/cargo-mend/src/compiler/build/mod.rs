mod progress;
mod stderr;

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde_json::to_string;

use self::stderr::stream_cargo_stderr;
use super::constants::CARGO_BIN;
use super::constants::CARGO_FLAG_ALL_TARGETS;
use super::constants::CARGO_FLAG_ALLOW_DIRTY;
use super::constants::CARGO_FLAG_ALLOW_STAGED;
use super::constants::CARGO_FLAG_TESTS;
use super::constants::CARGO_SUBCOMMAND_CHECK;
use super::constants::CARGO_SUBCOMMAND_FIX;
use super::constants::CONFIG_FINGERPRINT_ENV;
use super::constants::CONFIG_JSON_ENV;
use super::constants::CONFIG_ROOT_ENV;
use super::constants::DRIVER_ENV;
use super::constants::DRIVER_ENV_ENABLED;
use super::constants::FINDINGS_DIR_ENV;
use super::constants::RUSTC_WORKSPACE_WRAPPER_ENV;
use super::constants::SCOPE_FINGERPRINT_ENV;
use super::persistence;
use crate::config::LoadedConfig;
use crate::reporting::AnalysisFailure;
use crate::reporting::CARGO_TERM_COLOR_ALWAYS;
use crate::reporting::CARGO_TERM_COLOR_ENV;
use crate::reporting::ColorMode;
use crate::reporting::CompilerFailureCause;
use crate::reporting::CompilerWarningFacts;
use crate::reporting::MendFailure;
use crate::reporting::Report;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

// cargo manifest filename
pub(crate) const CARGO_MANIFEST_FILE: &str = "Cargo.toml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildOutputMode {
    Full,
    Json,
    SuppressUnusedImportWarnings,
    Quiet,
}

#[derive(Debug, Clone, Copy)]
struct CommandOutcome {
    exit_status:            ExitStatus,
    compiler_warning_facts: CompilerWarningFacts,
    duration:               Duration,
    compiler_warnings:      usize,
    compiler_fixable:       usize,
}

pub(crate) struct SelectionResult {
    pub report:            Report,
    pub check_duration:    Duration,
    pub compiler_warnings: usize,
    pub compiler_fixable:  usize,
}

pub(crate) fn run_selection(
    selection: &Selection,
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<SelectionResult, MendFailure> {
    let findings_dir = persistence::prepare_findings_dir(cargo_plan.target_directory.as_path())
        .map_err(|err| {
            MendFailure::Analysis(AnalysisFailure {
                cause: CompilerFailureCause::DriverSetup(err),
            })
        })?;
    let scope_fingerprint = scope_fingerprint_for(cargo_plan);

    let command_outcome = run_cargo_check(
        cargo_plan,
        loaded_config,
        &findings_dir,
        &scope_fingerprint,
        output_mode,
        color_mode,
    )
    .map_err(|err| {
        MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::DriverSetup(err),
        })
    })?;

    if !command_outcome.exit_status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck,
        }));
    }

    let report = persistence::load_report(&findings_dir, selection, &loaded_config.fingerprint)
        .map_err(|err| {
            MendFailure::Analysis(AnalysisFailure {
                cause: CompilerFailureCause::DriverExecution(err),
            })
        })?;

    let mut report = report;
    report.facts.compiler_warning_facts = command_outcome.compiler_warning_facts;
    Ok(SelectionResult {
        report,
        check_duration: command_outcome.duration,
        compiler_warnings: command_outcome.compiler_warnings,
        compiler_fixable: command_outcome.compiler_fixable,
    })
}

/// Runs `cargo check` with `RUSTC_WORKSPACE_WRAPPER` pointing to the mend binary.
///
/// The wrapper uses nightly's `rustc_driver::run_compiler` to analyze workspace members,
/// while dependencies are compiled by the project's default toolchain (typically stable).
/// This relies on nightly's `rustc_driver` being able to read `.rmeta` files produced by
/// stable — which works across close toolchain versions but is not guaranteed by rustc.
///
/// If a future rustc update breaks `.rmeta` compatibility between the mend binary's
/// toolchain and the project's default, this function would need to force the mend
/// binary's toolchain for the entire `cargo check` (via `RUSTUP_TOOLCHAIN`) and isolate
/// artifacts in a separate target directory (via `CARGO_TARGET_DIR`) to avoid corrupting
/// the project's build cache. See git history for the prior toolchain-selection design.
fn run_cargo_check(
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
    scope_fingerprint: &str,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<CommandOutcome> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new(CARGO_BIN);
    command.arg(CARGO_SUBCOMMAND_CHECK);
    command.args(&cargo_plan.cargo_args);

    command
        .env(RUSTC_WORKSPACE_WRAPPER_ENV, &current_exe)
        .env(DRIVER_ENV, DRIVER_ENV_ENABLED)
        .env(CONFIG_ROOT_ENV, &loaded_config.root)
        .env(
            CONFIG_JSON_ENV,
            to_string(&loaded_config.visibility_config)
                .context("failed to serialize mend config for compiler driver")?,
        )
        .env(CONFIG_FINGERPRINT_ENV, &loaded_config.fingerprint)
        .env(FINDINGS_DIR_ENV, findings_dir)
        .env(SCOPE_FINGERPRINT_ENV, scope_fingerprint)
        .stdin(Stdio::inherit());

    run_cargo_command(&mut command, output_mode, color_mode)
        .context("failed to run cargo check for mend")
}

fn scope_fingerprint_for(cargo_plan: &CargoCheckPlan) -> String {
    let mut hasher = DefaultHasher::new();
    cargo_plan.manifest_path.hash(&mut hasher);
    for arg in &cargo_plan.cargo_args {
        arg.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub(crate) fn run_cargo_fix(
    cargo_plan: &CargoCheckPlan,
    color_mode: ColorMode,
) -> Result<Duration> {
    let start = Instant::now();
    let mut command = Command::new(CARGO_BIN);
    command
        .arg(CARGO_SUBCOMMAND_FIX)
        .arg(CARGO_FLAG_ALLOW_DIRTY)
        .arg(CARGO_FLAG_ALLOW_STAGED);

    // Replace `--all-targets` with `--tests` for the fix pass. Rationale:
    // `cargo fix --all-targets` runs the lib (non-test) compilation
    // alongside others; that compilation strips `#[cfg(test)]` blocks and
    // emits `unused_imports` warnings for items reached only from test
    // code. Cargo fix then deletes those imports, breaking the test build
    // (E0425 cascade — the original bug). Running with `--tests` only
    // compiles each target in test mode, where every cfg(test)-protected
    // call site is live, so genuinely-needed imports are never removed.
    // Trade-off: imports that are unused in BOTH lib and test mode (rare)
    // won't be pruned by `--fix-compiler`; users can clean those manually.
    for arg in &cargo_plan.cargo_args {
        if arg == CARGO_FLAG_ALL_TARGETS {
            command.arg(CARGO_FLAG_TESTS);
        } else {
            command.arg(arg);
        }
    }

    if color_mode.is_enabled() {
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
    }

    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run cargo fix")?;

    if !status.success() {
        bail!("cargo fix failed");
    }

    Ok(start.elapsed())
}

fn run_cargo_command(
    command: &mut Command,
    output_mode: BuildOutputMode,
    color_mode: ColorMode,
) -> Result<CommandOutcome> {
    if color_mode.is_enabled() {
        command.env(CARGO_TERM_COLOR_ENV, CARGO_TERM_COLOR_ALWAYS);
    }
    command.stdin(Stdio::inherit());
    command.stderr(Stdio::piped());
    match output_mode {
        BuildOutputMode::Full => command.stdout(Stdio::inherit()),
        BuildOutputMode::Json
        | BuildOutputMode::SuppressUnusedImportWarnings
        | BuildOutputMode::Quiet => command.stdout(Stdio::null()),
    };
    let start = Instant::now();
    let mut child = command.spawn().context("failed to spawn cargo command")?;
    let stderr = child
        .stderr
        .take()
        .context("failed to capture cargo stderr")?;
    let stderr_outcome = stream_cargo_stderr(stderr, output_mode)?;
    let exit_status = child.wait().context("failed to wait for cargo command")?;
    let duration = start.elapsed();
    Ok(CommandOutcome {
        exit_status,
        compiler_warning_facts: stderr_outcome.compiler_warning_facts,
        duration,
        compiler_warnings: stderr_outcome.warning_count,
        compiler_fixable: stderr_outcome.fixable_count,
    })
}
