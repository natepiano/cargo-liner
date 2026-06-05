use std::collections::hash_map::DefaultHasher;
use std::env;
use std::hash::Hash;
use std::hash::Hasher;
use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::process::ChildStderr;
use std::process::Command;
use std::process::ExitStatus;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde_json::to_string;

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

// binary names
pub(crate) const CARGO_BIN: &str = "cargo";
pub(crate) const RUSTC_BIN: &str = "rustc";

// cargo cli flags
pub(crate) const CARGO_FLAG_ALL_TARGETS: &str = "--all-targets";
pub(crate) const CARGO_FLAG_ALLOW_DIRTY: &str = "--allow-dirty";
pub(crate) const CARGO_FLAG_ALLOW_STAGED: &str = "--allow-staged";
pub(crate) const CARGO_FLAG_EXCLUDE: &str = "--exclude";
pub(crate) const CARGO_FLAG_MANIFEST_PATH: &str = "--manifest-path";
pub(crate) const CARGO_FLAG_PACKAGE: &str = "--package";
pub(crate) const CARGO_FLAG_TESTS: &str = "--tests";
pub(crate) const CARGO_FLAG_WORKSPACE: &str = "--workspace";

// cargo manifest filename
pub(crate) const CARGO_MANIFEST_FILE: &str = "Cargo.toml";

// cargo output protocol
pub(crate) const CARGO_PROGRESS_PREFIX_BLOCKING: &str = "Blocking waiting for file lock";
pub(crate) const CARGO_PROGRESS_PREFIX_BUILDING: &str = "Building ";
pub(crate) const CARGO_PROGRESS_PREFIX_CHECKING: &str = "Checking ";
pub(crate) const CARGO_PROGRESS_PREFIX_COMPILING: &str = "Compiling ";
pub(crate) const CARGO_PROGRESS_PREFIX_FINISHED: &str = "Finished ";
pub(crate) const CARGO_PROGRESS_PREFIX_FRESH: &str = "Fresh ";
pub(crate) const CARGO_UNUSED_IMPORTS_WARNING: &str = "warning: unused imports:";
pub(crate) const CARGO_UNUSED_IMPORT_WARNING: &str = "warning: unused import:";
pub(crate) const CARGO_WARNING_SUMMARY_PREFIX: &str = "warning: `";
pub(crate) const CARGO_WARNING_SUMMARY_TOKEN_GENERATED: &str = " generated ";
pub(crate) const CARGO_WARNING_SUMMARY_TOKEN_TO_APPLY: &str = "to apply ";

// cargo subcommands
pub(crate) const CARGO_SUBCOMMAND_CHECK: &str = "check";
pub(crate) const CARGO_SUBCOMMAND_FIX: &str = "fix";
pub(crate) const CARGO_SUBCOMMAND_MEND: &str = "mend";

// diagnostic severity prefixes
pub(crate) const DIAGNOSTIC_SEVERITY_ERROR_PREFIX: &str = "error:";
pub(crate) const DIAGNOSTIC_SEVERITY_WARNING_PREFIX: &str = "warning:";

const PROGRESS_INTERVAL: Duration = Duration::from_millis(120);
const PROGRESS_FRAMES: [&str; 4] = ["|", "/", "-", "\\"];

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
    Forwarded,
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
    report.facts.compiler_warnings = command_outcome.compiler_warning_facts;
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
/// the project's build cache. See git history for the prior `ToolchainOverride` approach.
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

#[derive(Debug, Clone, Copy, Default)]
struct StderrObservation {
    compiler_warning_facts: CompilerWarningFacts,
    warning_count:          usize,
    fixable_count:          usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SuppressionNotice {
    #[default]
    Pending,
    Printed,
}

fn stream_cargo_stderr(
    stderr: ChildStderr,
    output_mode: BuildOutputMode,
) -> Result<StderrObservation> {
    let mut reader = BufReader::new(stderr);
    let mut progress = CargoProgress::start(output_mode);
    let mut line = String::new();
    let mut block = Vec::new();
    let mut suppression_notice = SuppressionNotice::Pending;
    let mut compiler_warning_facts = CompilerWarningFacts::None;
    let mut compiler_warning_count: usize = 0;
    let mut compiler_fixable_count: usize = 0;

    loop {
        line.clear();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            flush_diagnostic_block(
                &mut block,
                &mut suppression_notice,
                &mut compiler_warning_facts,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
                &mut progress,
            );
            break;
        }

        let current = line.clone();
        if is_progress_line(&current) {
            flush_diagnostic_block(
                &mut block,
                &mut suppression_notice,
                &mut compiler_warning_facts,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
                &mut progress,
            );
            if should_forward_progress_line(&current, output_mode, progress.is_active().into()) {
                eprint!("{current}");
            }
            continue;
        }

        if current.trim().is_empty() {
            block.push(current);
            flush_diagnostic_block(
                &mut block,
                &mut suppression_notice,
                &mut compiler_warning_facts,
                &mut compiler_warning_count,
                &mut compiler_fixable_count,
                output_mode,
                &mut progress,
            );
        } else {
            block.push(current);
        }
    }

    Ok(StderrObservation {
        compiler_warning_facts,
        warning_count: compiler_warning_count,
        fixable_count: compiler_fixable_count,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProgressStatus {
    Active,
    Inactive,
}

impl From<bool> for ProgressStatus {
    fn from(value: bool) -> Self { if value { Self::Active } else { Self::Inactive } }
}

fn should_forward_progress_line(
    line: &str,
    output_mode: BuildOutputMode,
    progress_status: ProgressStatus,
) -> bool {
    matches!(progress_status, ProgressStatus::Inactive)
        && !is_finished_line(line)
        && !matches!(output_mode, BuildOutputMode::Json | BuildOutputMode::Quiet)
}

pub(super) fn is_progress_line(line: &str) -> bool {
    let sanitized = sanitize_for_match(line);
    let trimmed = sanitized.trim_start();
    if trimmed.contains(DIAGNOSTIC_SEVERITY_WARNING_PREFIX)
        || trimmed.contains(DIAGNOSTIC_SEVERITY_ERROR_PREFIX)
    {
        return false;
    }
    trimmed.starts_with(CARGO_PROGRESS_PREFIX_BLOCKING)
        || trimmed.starts_with(CARGO_PROGRESS_PREFIX_BUILDING)
        || trimmed.starts_with(CARGO_PROGRESS_PREFIX_CHECKING)
        || trimmed.starts_with(CARGO_PROGRESS_PREFIX_COMPILING)
        || trimmed.starts_with(CARGO_PROGRESS_PREFIX_FINISHED)
        || trimmed.starts_with(CARGO_PROGRESS_PREFIX_FRESH)
}

fn is_finished_line(line: &str) -> bool {
    let sanitized = sanitize_for_match(line);
    sanitized
        .trim_start()
        .starts_with(CARGO_PROGRESS_PREFIX_FINISHED)
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
    if !trimmed.starts_with(CARGO_WARNING_SUMMARY_PREFIX)
        || !trimmed.contains(CARGO_WARNING_SUMMARY_TOKEN_GENERATED)
    {
        return None;
    }

    let after_generated = trimmed
        .split(CARGO_WARNING_SUMMARY_TOKEN_GENERATED)
        .nth(1)?;
    let warning_count: usize = after_generated.split_whitespace().next()?.parse().ok()?;

    let fixable_count = trimmed
        .split(CARGO_WARNING_SUMMARY_TOKEN_TO_APPLY)
        .nth(1)
        .map_or(0, |after_apply| {
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
    first_non_empty.map_or(DiagnosticBlockKind::Forwarded, |line| {
        let sanitized = sanitize_for_match(line);
        let trimmed = sanitized.trim_start();

        // Check for "generated N warnings" summary line first — always suppress
        if let Some((warning_count, fixable_count)) = parse_compiler_warning_summary(trimmed) {
            DiagnosticBlockKind::CompilerWarningSummary {
                warning_count,
                fixable_count,
            }
        } else {
            let contains_unused_import_warning = trimmed.contains(CARGO_UNUSED_IMPORT_WARNING)
                || trimmed.contains(CARGO_UNUSED_IMPORTS_WARNING);
            if contains_unused_import_warning {
                DiagnosticBlockKind::SuppressedUnusedImport
            } else {
                DiagnosticBlockKind::Forwarded
            }
        }
    })
}

fn flush_diagnostic_block(
    block: &mut Vec<String>,
    suppression_notice: &mut SuppressionNotice,
    compiler_warnings: &mut CompilerWarningFacts,
    compiler_warning_count: &mut usize,
    compiler_fixable_count: &mut usize,
    output_mode: BuildOutputMode,
    progress: &mut impl ProgressDisplay,
) {
    if block.is_empty() {
        return;
    }

    match classify_diagnostic_block(block) {
        DiagnosticBlockKind::SuppressedUnusedImport => {
            *compiler_warnings = CompilerWarningFacts::UnusedImportWarnings;
            match output_mode {
                BuildOutputMode::SuppressUnusedImportWarnings
                    if *suppression_notice == SuppressionNotice::Pending =>
                {
                    progress.write_status_notice(
                        "mend: suppressing `unused import` warning during `--fix-pub-use` \
                         discovery",
                    );
                    *suppression_notice = SuppressionNotice::Printed;
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
        DiagnosticBlockKind::Forwarded => {
            // Always forward except in JSON mode. Quiet mode used to drop
            // these too, which hid post-fix validation errors — leaving
            // users with the opaque "compiler failed" message and no way
            // to see what cargo actually complained about.
            if !matches!(output_mode, BuildOutputMode::Json) {
                progress.stop_for_forwarded_output();
                for line in block.iter() {
                    eprint!("{line}");
                }
            }
        },
    }

    block.clear();
}

trait ProgressDisplay {
    fn is_active(&self) -> bool;

    fn write_status_notice(&mut self, notice: &str);

    fn stop_for_forwarded_output(&mut self);
}

struct CargoProgress {
    state: Option<CargoProgressState>,
}

struct CargoProgressState {
    active:      Arc<AtomicBool>,
    output_lock: Arc<Mutex<()>>,
    handle:      Option<JoinHandle<()>>,
    line_width:  usize,
}

impl CargoProgress {
    fn start(output_mode: BuildOutputMode) -> Self {
        let Some(message) = progress_message_for(output_mode) else {
            return Self { state: None };
        };
        if !io::stderr().is_terminal() {
            return Self { state: None };
        }

        let active = Arc::new(AtomicBool::new(true));
        let output_lock = Arc::new(Mutex::new(()));
        let thread_active = Arc::clone(&active);
        let thread_lock = Arc::clone(&output_lock);
        let line_width = progress_line_width(message);
        let handle = thread::spawn(move || {
            let mut frame_index = 0;
            while thread_active.load(Ordering::Relaxed) {
                if let Ok(_guard) = thread_lock.lock() {
                    eprint!("{}", progress_frame(message, frame_index));
                    let _ = io::stderr().flush();
                }
                frame_index = (frame_index + 1) % PROGRESS_FRAMES.len();
                thread::sleep(PROGRESS_INTERVAL);
            }
        });

        Self {
            state: Some(CargoProgressState {
                active,
                output_lock,
                handle: Some(handle),
                line_width,
            }),
        }
    }

    fn stop(&mut self) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        state.active.store(false, Ordering::Relaxed);
        if let Some(handle) = state.handle.take() {
            let _ = handle.join();
        }
        state.clear_line();
        self.state = None;
    }
}

impl Drop for CargoProgress {
    fn drop(&mut self) { self.stop(); }
}

impl ProgressDisplay for CargoProgress {
    fn is_active(&self) -> bool { self.state.is_some() }

    fn write_status_notice(&mut self, notice: &str) {
        if let Some(state) = self.state.as_ref() {
            state.write_status_notice(notice);
        } else {
            eprintln!("{notice}");
        }
    }

    fn stop_for_forwarded_output(&mut self) { self.stop(); }
}

impl CargoProgressState {
    fn clear_line(&self) {
        if let Ok(_guard) = self.output_lock.lock() {
            eprint!("{}", clear_progress_line(self.line_width));
            let _ = io::stderr().flush();
        }
    }

    fn write_status_notice(&self, notice: &str) {
        if let Ok(_guard) = self.output_lock.lock() {
            eprint!("{}", clear_progress_line(self.line_width));
            eprintln!("{notice}");
            let _ = io::stderr().flush();
        }
    }
}

const fn progress_message_for(output_mode: BuildOutputMode) -> Option<&'static str> {
    match output_mode {
        BuildOutputMode::SuppressUnusedImportWarnings => Some("checking for fix candidates"),
        BuildOutputMode::Quiet => Some("validating applied fixes"),
        BuildOutputMode::Full | BuildOutputMode::Json => None,
    }
}

fn progress_frame(message: &str, frame_index: usize) -> String {
    let frame = PROGRESS_FRAMES[frame_index % PROGRESS_FRAMES.len()];
    format!("\rmend: {frame} {message}")
}

fn progress_line_width(message: &str) -> usize { progress_frame(message, 0).chars().count() - 1 }

fn clear_progress_line(width: usize) -> String { format!("\r{}\r", " ".repeat(width)) }

#[cfg(test)]
mod tests {
    use super::BuildOutputMode;
    use super::CompilerWarningFacts;
    use super::DiagnosticBlockKind;
    use super::ProgressDisplay;
    use super::ProgressStatus;
    use super::SuppressionNotice;
    use super::classify_diagnostic_block;
    use super::clear_progress_line;
    use super::flush_diagnostic_block;
    use super::is_progress_line;
    use super::progress_frame;
    use super::progress_line_width;
    use super::progress_message_for;
    use super::should_forward_progress_line;

    #[derive(Default)]
    struct ProgressRecorder {
        active:  bool,
        notices: Vec<String>,
        stops:   usize,
    }

    impl ProgressRecorder {
        const fn active() -> Self {
            Self {
                active:  true,
                notices: Vec::new(),
                stops:   0,
            }
        }
    }

    impl ProgressDisplay for ProgressRecorder {
        fn is_active(&self) -> bool { self.active }

        fn write_status_notice(&mut self, notice: &str) { self.notices.push(notice.to_string()); }

        fn stop_for_forwarded_output(&mut self) {
            self.stops += 1;
            self.active = false;
        }
    }

    #[test]
    fn plain_building_progress_line_is_treated_as_progress() {
        let line = "    Building [                             ] 0/1: cli_json_clean_fixture      \r    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.16s\n";
        assert!(is_progress_line(line));
    }

    #[test]
    fn progress_line_with_embedded_warning_is_not_treated_as_progress() {
        let line = "    Building [                             ] 0/1: fixture...warning: unused import: `child::SpawnStats`\n";
        assert!(!is_progress_line(line));
    }

    #[test]
    fn classify_suppresses_unused_import_when_warning_follows_progress_prefix() {
        let block = vec![
            "    Building [                             ] 0/1: fixture...warning: unused import: `child::SpawnStats`\n"
                .to_string(),
            " --> src/actor/mod.rs:2:9\n".to_string(),
            "  |\n".to_string(),
            "2 | pub use child::SpawnStats;\n".to_string(),
            "  |         ^^^^^^^^^^^^^^^^^\n".to_string(),
            "\n".to_string(),
        ];

        assert!(matches!(
            classify_diagnostic_block(&block),
            DiagnosticBlockKind::SuppressedUnusedImport
        ));
    }

    #[test]
    fn quiet_builds_do_not_accumulate_compiler_warning_summary_counts() {
        let mut block = vec![
            "warning: `fixture` (lib) generated 3 warnings (1 duplicate) (run `cargo fix --lib -p fixture` to apply 1 suggestion)\n"
                .to_string(),
            "\n".to_string(),
        ];
        let mut suppression_notice = SuppressionNotice::Pending;
        let mut compiler_warning_facts = CompilerWarningFacts::None;
        let mut compiler_warning_count = 0;
        let mut compiler_fixable_count = 0;

        flush_diagnostic_block(
            &mut block,
            &mut suppression_notice,
            &mut compiler_warning_facts,
            &mut compiler_warning_count,
            &mut compiler_fixable_count,
            BuildOutputMode::Quiet,
            &mut ProgressRecorder::default(),
        );

        assert_eq!(compiler_warning_count, 0);
        assert_eq!(compiler_fixable_count, 0);
    }

    #[test]
    fn quiet_mode_uses_validation_status_message() {
        assert_eq!(
            progress_message_for(BuildOutputMode::Quiet),
            Some("validating applied fixes")
        );
    }

    #[test]
    fn json_mode_has_no_progress_status() {
        assert_eq!(progress_message_for(BuildOutputMode::Json), None);
    }

    #[test]
    fn progress_frame_and_clear_line_use_carriage_return() {
        let frame = progress_frame("validating applied fixes", 1);
        let width = progress_line_width("validating applied fixes");

        assert_eq!(frame, "\rmend: / validating applied fixes");
        assert_eq!(
            clear_progress_line(width),
            format!("\r{}\r", " ".repeat(width))
        );
    }

    #[test]
    fn progress_lines_are_hidden_while_progress_status_is_active() {
        let line = "    Checking fixture v0.1.0\n";

        assert!(!should_forward_progress_line(
            line,
            BuildOutputMode::SuppressUnusedImportWarnings,
            ProgressStatus::Active
        ));
        assert!(should_forward_progress_line(
            line,
            BuildOutputMode::SuppressUnusedImportWarnings,
            ProgressStatus::Inactive
        ));
    }

    #[test]
    fn forwarded_diagnostic_stops_progress_before_printing() {
        let mut block = vec!["error: expected item\n".to_string(), "\n".to_string()];
        let mut suppression_notice = SuppressionNotice::Pending;
        let mut compiler_warning_facts = CompilerWarningFacts::None;
        let mut compiler_warning_count = 0;
        let mut compiler_fixable_count = 0;
        let mut progress = ProgressRecorder::active();

        flush_diagnostic_block(
            &mut block,
            &mut suppression_notice,
            &mut compiler_warning_facts,
            &mut compiler_warning_count,
            &mut compiler_fixable_count,
            BuildOutputMode::Quiet,
            &mut progress,
        );

        assert_eq!(progress.stops, 1);
        assert!(progress.notices.is_empty());
        assert!(!progress.active);
    }

    #[test]
    fn suppression_notice_writes_progress_status_notice_without_stopping() {
        let mut block = vec![
            "warning: unused import: `child::SpawnStats`\n".to_string(),
            "\n".to_string(),
        ];
        let mut suppression_notice = SuppressionNotice::Pending;
        let mut compiler_warning_facts = CompilerWarningFacts::None;
        let mut compiler_warning_count = 0;
        let mut compiler_fixable_count = 0;
        let mut progress = ProgressRecorder::active();

        flush_diagnostic_block(
            &mut block,
            &mut suppression_notice,
            &mut compiler_warning_facts,
            &mut compiler_warning_count,
            &mut compiler_fixable_count,
            BuildOutputMode::SuppressUnusedImportWarnings,
            &mut progress,
        );

        assert_eq!(
            progress.notices,
            vec![
                "mend: suppressing `unused import` warning during `--fix-pub-use` discovery"
                    .to_string()
            ]
        );
        assert_eq!(progress.stops, 0);
        assert!(progress.active);
    }
}
