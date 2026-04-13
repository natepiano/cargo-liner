use std::env;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;

use super::persistence::load_report;
use super::persistence::prepare_findings_dir;
use crate::config::LoadedConfig;
use crate::constants::CONFIG_FINGERPRINT_ENV;
use crate::constants::CONFIG_JSON_ENV;
use crate::constants::CONFIG_ROOT_ENV;
use crate::constants::DRIVER_ENV;
use crate::constants::FINDINGS_DIR_ENV;
use crate::constants::SCOPE_FINGERPRINT_ENV;
use crate::diagnostics::CompilerWarningFacts;
use crate::diagnostics::Report;
use crate::outcome::AnalysisFailure;
use crate::outcome::CompilerFailureCause;
use crate::outcome::MendFailure;
use crate::render::ColorMode;
use crate::selection::CargoCheckPlan;
use crate::selection::Selection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildOutputMode {
    Full,
    Json,
    SuppressUnusedImportWarnings,
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DiagnosticBlockKind {
    SuppressedUnusedImport,
    CompilerWarningSummary {
        warning_count: usize,
        fixable_count: usize,
    },
    ForwardedDiagnostic,
}

#[derive(Debug, Clone, Copy)]
struct CommandOutcome {
    status:                 std::process::ExitStatus,
    compiler_warnings:      CompilerWarningFacts,
    duration:               Duration,
    compiler_warning_count: usize,
    compiler_fixable_count: usize,
}

pub(crate) struct SelectionResult {
    pub report:                 Report,
    pub check_duration:         Duration,
    pub compiler_warning_count: usize,
    pub compiler_fixable_count: usize,
}

pub(crate) fn run_selection(
    selection: &Selection,
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    output_mode: BuildOutputMode,
    color: ColorMode,
) -> Result<SelectionResult, MendFailure> {
    let findings_dir =
        prepare_findings_dir(cargo_plan.target_directory.as_path()).map_err(|err| {
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
        color,
    )
    .map_err(|err| {
        MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::DriverSetup(err),
        })
    })?;

    if !command_outcome.status.success() {
        return Err(MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::CargoCheck,
        }));
    }

    let report = load_report(
        &findings_dir,
        selection,
        &loaded_config.fingerprint,
        &scope_fingerprint,
    )
    .map_err(|err| {
        MendFailure::Analysis(AnalysisFailure {
            cause: CompilerFailureCause::DriverExecution(err),
        })
    })?;

    let mut report = report;
    report.facts.compiler_warnings = command_outcome.compiler_warnings;
    Ok(SelectionResult {
        report,
        check_duration: command_outcome.duration,
        compiler_warning_count: command_outcome.compiler_warning_count,
        compiler_fixable_count: command_outcome.compiler_fixable_count,
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
/// the project's build cache. See git history for the prior `ToolchainOverride` approach.
fn run_cargo_check(
    cargo_plan: &CargoCheckPlan,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
    scope_fingerprint: &str,
    output_mode: BuildOutputMode,
    color: ColorMode,
) -> Result<CommandOutcome> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new("cargo");
    command.arg("check");
    command.args(&cargo_plan.cargo_args);

    command
        .env("RUSTC_WORKSPACE_WRAPPER", &current_exe)
        .env(DRIVER_ENV, "1")
        .env(CONFIG_ROOT_ENV, &loaded_config.root)
        .env(
            CONFIG_JSON_ENV,
            serde_json::to_string(&loaded_config.config)
                .context("failed to serialize mend config for compiler driver")?,
        )
        .env(CONFIG_FINGERPRINT_ENV, &loaded_config.fingerprint)
        .env(FINDINGS_DIR_ENV, findings_dir)
        .env(SCOPE_FINGERPRINT_ENV, scope_fingerprint)
        .stdin(Stdio::inherit());

    run_cargo_command(&mut command, output_mode, color)
        .context("failed to run cargo check for mend")
}

fn scope_fingerprint_for(cargo_plan: &CargoCheckPlan) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    cargo_plan.manifest_path.hash(&mut hasher);
    for arg in &cargo_plan.cargo_args {
        arg.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

pub(crate) fn run_cargo_fix(cargo_plan: &CargoCheckPlan, color: ColorMode) -> Result<Duration> {
    let start = Instant::now();
    let mut command = Command::new("cargo");
    command
        .arg("fix")
        .arg("--allow-dirty")
        .arg("--allow-staged")
        .args(&cargo_plan.cargo_args);

    if color.is_enabled() {
        command.env("CARGO_TERM_COLOR", "always");
    }

    let status = command
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .context("failed to run cargo fix")?;

    if !status.success() {
        anyhow::bail!("cargo fix failed");
    }

    Ok(start.elapsed())
}

fn run_cargo_command(
    command: &mut Command,
    output_mode: BuildOutputMode,
    color: ColorMode,
) -> Result<CommandOutcome> {
    if color.is_enabled() {
        command.env("CARGO_TERM_COLOR", "always");
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
    let status = child.wait().context("failed to wait for cargo command")?;
    let duration = start.elapsed();
    Ok(CommandOutcome {
        status,
        compiler_warnings: stderr_outcome.warnings,
        duration,
        compiler_warning_count: stderr_outcome.warning_count,
        compiler_fixable_count: stderr_outcome.fixable_count,
    })
}

#[derive(Debug, Clone, Copy, Default)]
struct StderrObservation {
    warnings:      CompilerWarningFacts,
    warning_count: usize,
    fixable_count: usize,
}

fn stream_cargo_stderr(
    stderr: std::process::ChildStderr,
    output_mode: BuildOutputMode,
) -> Result<StderrObservation> {
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();
    let mut block = Vec::new();
    let mut printed_suppression_notice = false;
    let mut compiler_warnings = CompilerWarningFacts::None;
    let mut compiler_warning_count: usize = 0;
    let mut compiler_fixable_count: usize = 0;

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            flush_diagnostic_block(
                &mut block,
                &mut printed_suppression_notice,
                &mut compiler_warnings,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
            );
            break;
        }

        let current = line.clone();
        if is_progress_line(&current) {
            flush_diagnostic_block(
                &mut block,
                &mut printed_suppression_notice,
                &mut compiler_warnings,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
            );
            // Suppress "Finished" lines and all progress in Quiet mode
            if !is_finished_line(&current)
                && !matches!(output_mode, BuildOutputMode::Json | BuildOutputMode::Quiet)
            {
                eprint!("{current}");
            }
            continue;
        }

        if current.trim().is_empty() {
            block.push(current);
            flush_diagnostic_block(
                &mut block,
                &mut printed_suppression_notice,
                &mut compiler_warnings,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
            );
        } else {
            block.push(current);
        }
    }

    Ok(StderrObservation {
        warnings:      compiler_warnings,
        warning_count: compiler_warning_count,
        fixable_count: compiler_fixable_count,
    })
}

pub(super) fn is_progress_line(line: &str) -> bool {
    let sanitized = sanitize_for_match(line);
    let trimmed = sanitized.trim_start();
    if trimmed.contains("warning:") || trimmed.contains("error:") {
        return false;
    }
    trimmed.starts_with("Blocking waiting for file lock")
        || trimmed.starts_with("Building ")
        || trimmed.starts_with("Checking ")
        || trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Finished ")
        || trimmed.starts_with("Fresh ")
}

fn is_finished_line(line: &str) -> bool {
    let sanitized = sanitize_for_match(line);
    sanitized.trim_start().starts_with("Finished ")
}

pub(super) fn sanitize_for_match(line: &str) -> String {
    let mut sanitized = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek().copied() == Some('[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            continue;
        }

        sanitized.push(ch);
    }

    sanitized
}

/// Parse cargo's "generated N warnings" summary line.
/// Returns `(warning_count, fixable_count)` if the line matches.
fn parse_compiler_warning_summary(line: &str) -> Option<(usize, usize)> {
    let sanitized = sanitize_for_match(line);
    let trimmed = sanitized.trim_start();

    // Match: warning: `pkg` (target) generated N warning(s)
    if !trimmed.starts_with("warning: `") || !trimmed.contains(" generated ") {
        return None;
    }

    let after_generated = trimmed.split(" generated ").nth(1)?;
    let warning_count: usize = after_generated.split_whitespace().next()?.parse().ok()?;

    let fixable_count = trimmed.split("to apply ").nth(1).map_or(0, |after_apply| {
        after_apply
            .split_whitespace()
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or(0)
    });

    Some((warning_count, fixable_count))
}

pub(super) fn classify_diagnostic_block(block: &[String]) -> DiagnosticBlockKind {
    let first_non_empty = block.iter().find(|line| !line.trim().is_empty());
    first_non_empty.map_or(DiagnosticBlockKind::ForwardedDiagnostic, |line| {
        let sanitized = sanitize_for_match(line);
        let trimmed = sanitized.trim_start();

        // Check for "generated N warnings" summary line first — always suppress
        if let Some((warning_count, fixable_count)) = parse_compiler_warning_summary(trimmed) {
            return DiagnosticBlockKind::CompilerWarningSummary {
                warning_count,
                fixable_count,
            };
        }

        let contains_unused_import_warning = trimmed.contains("warning: unused import:")
            || trimmed.contains("warning: unused imports:");
        if contains_unused_import_warning {
            DiagnosticBlockKind::SuppressedUnusedImport
        } else {
            DiagnosticBlockKind::ForwardedDiagnostic
        }
    })
}

pub(super) fn flush_diagnostic_block(
    block: &mut Vec<String>,
    printed_suppression_notice: &mut bool,
    compiler_warnings: &mut CompilerWarningFacts,
    compiler_warning_count: &mut usize,
    compiler_fixable_count: &mut usize,
    output_mode: BuildOutputMode,
) {
    if block.is_empty() {
        return;
    }

    match classify_diagnostic_block(block) {
        DiagnosticBlockKind::SuppressedUnusedImport => {
            *compiler_warnings = CompilerWarningFacts::UnusedImportWarnings;
            match output_mode {
                BuildOutputMode::SuppressUnusedImportWarnings if !*printed_suppression_notice => {
                    eprintln!(
                        "mend: suppressing `unused import` warning during `--fix-pub-use` \
                         discovery"
                    );
                    *printed_suppression_notice = true;
                },
                BuildOutputMode::Full => {
                    for line in block.iter() {
                        eprint!("{line}");
                    }
                },
                BuildOutputMode::Json
                | BuildOutputMode::SuppressUnusedImportWarnings
                | BuildOutputMode::Quiet => {},
            }
        },
        DiagnosticBlockKind::CompilerWarningSummary {
            warning_count,
            fixable_count,
        } => {
            if !matches!(output_mode, BuildOutputMode::Quiet) {
                *compiler_warning_count += warning_count;
                *compiler_fixable_count += fixable_count;
            }
            // Suppressed — will be rendered in the unified summary
        },
        DiagnosticBlockKind::ForwardedDiagnostic => {
            if !matches!(output_mode, BuildOutputMode::Json | BuildOutputMode::Quiet) {
                for line in block.iter() {
                    eprint!("{line}");
                }
            }
        },
    }

    block.clear();
}
