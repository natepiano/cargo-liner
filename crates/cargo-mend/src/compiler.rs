use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::io::BufRead;
use std::io::BufReader;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::ExitCode;
use std::process::Stdio;
use std::time::Duration;
use std::time::Instant;

use anyhow::Context;
use anyhow::Result;
use quote::ToTokens;
use rustc_driver::Callbacks;
use rustc_driver::Compilation;
use rustc_hir::ForeignItem;
use rustc_hir::ForeignItemKind;
use rustc_hir::ImplItem;
use rustc_hir::ImplItemKind;
use rustc_hir::Item;
use rustc_hir::ItemKind;
use rustc_middle::middle::privacy::Level;
use rustc_middle::ty::TyCtxt;
use rustc_span::FileName;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;
use serde::Deserialize;
use serde::Serialize;
use syn::ItemUse;
use syn::UseTree;
use syn::visit::Visit;

use super::config::DiagnosticCode;
use super::config::LoadedConfig;
use super::config::VisibilityConfig;
use super::constants::CONFIG_FINGERPRINT_ENV;
use super::constants::CONFIG_JSON_ENV;
use super::constants::CONFIG_ROOT_ENV;
use super::constants::DRIVER_ENV;
use super::constants::EXIT_CODE_ERROR;
use super::constants::FINDINGS_DIR_ENV;
use super::constants::FINDINGS_SCHEMA_VERSION;
use super::constants::PACKAGE_ROOT_ENV;
use super::constants::SCOPE_FINGERPRINT_ENV;
use super::diagnostics::CompilerWarningFacts;
use super::diagnostics::Finding;
use super::diagnostics::PubUseFixFact;
use super::diagnostics::PubUseFixFacts;
use super::diagnostics::Report;
use super::diagnostics::ReportFacts;
use super::diagnostics::ReportSummary;
use super::diagnostics::Severity;
use super::fix_support::FixSupport;
use super::module_paths;
use super::outcome::AnalysisFailure;
use super::outcome::CompilerFailureCause;
use super::outcome::MendFailure;
use super::render::ColorMode;
use super::selection::CargoCheckPlan;
use super::selection::Selection;
fn current_analysis_fingerprint() -> String {
    let version = env!("CARGO_PKG_VERSION");
    let git_hash = option_env!("MEND_GIT_HASH").unwrap_or("nogit");
    let build_id = option_env!("MEND_BUILD_ID").unwrap_or("nobuild");
    format!("{version}+{git_hash}+{build_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildOutputMode {
    Full,
    Json,
    SuppressUnusedImportWarnings,
    Quiet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticBlockKind {
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

#[derive(Debug, Serialize, Deserialize)]
struct StoredReport {
    version:              u32,
    #[serde(default)]
    analysis_fingerprint: String,
    #[serde(default)]
    scope_fingerprint:    String,
    package_root:         String,
    #[serde(default)]
    crate_root_file:      String,
    config_fingerprint:   String,
    findings:             Vec<StoredFinding>,
    #[serde(default)]
    pub_use_fix_facts:    Vec<StoredPubUseFixFact>,
    #[serde(default)]
    compiler_warnings:    CompilerWarningFacts,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredFinding {
    severity:      Severity,
    code:          DiagnosticCode,
    path:          String,
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
    item:          Option<String>,
    message:       String,
    suggestion:    Option<String>,
    #[serde(default)]
    fixability:    FixSupport,
    #[serde(default)]
    related:       Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredPubUseFixFact {
    child_path:      String,
    child_line:      usize,
    child_item_name: String,
    parent_path:     String,
    parent_line:     usize,
    child_module:    String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CrateKind {
    Binary,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModuleLocation {
    CrateRoot,
    TopLevelPrivateModule,
    NestedModule,
}

#[derive(Debug, Clone)]
struct DriverSettings {
    config_root:          PathBuf,
    config:               VisibilityConfig,
    config_fingerprint:   String,
    analysis_fingerprint: String,
    scope_fingerprint:    String,
    findings_dir:         PathBuf,
    package_root:         PathBuf,
}

impl DriverSettings {
    fn from_env() -> Result<Self> {
        let config_root = PathBuf::from(
            env::var_os(CONFIG_ROOT_ENV).context("missing MEND_CONFIG_ROOT for compiler driver")?,
        );
        let config = serde_json::from_str(
            &env::var(CONFIG_JSON_ENV).context("missing MEND_CONFIG_JSON for compiler driver")?,
        )
        .context("failed to parse MEND_CONFIG_JSON")?;
        let config_fingerprint =
            env::var(CONFIG_FINGERPRINT_ENV).context("missing MEND_CONFIG_FINGERPRINT")?;
        let findings_dir = PathBuf::from(
            env::var_os(FINDINGS_DIR_ENV)
                .context("missing MEND_FINDINGS_DIR for compiler driver")?,
        );
        let scope_fingerprint =
            env::var(SCOPE_FINGERPRINT_ENV).context("missing MEND_SCOPE_FINGERPRINT")?;
        let package_root = PathBuf::from(
            env::var_os(PACKAGE_ROOT_ENV)
                .context("missing CARGO_MANIFEST_DIR for compiler driver")?,
        );

        Ok(Self {
            config_root,
            config,
            config_fingerprint,
            analysis_fingerprint: current_analysis_fingerprint(),
            scope_fingerprint,
            findings_dir,
            package_root,
        })
    }
}

#[derive(Debug)]
struct AnalysisCallbacks {
    settings: DriverSettings,
    error:    Option<anyhow::Error>,
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
    fn after_analysis(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'_>,
    ) -> Compilation {
        match collect_and_store_findings(tcx, &self.settings) {
            Ok(true | false) => Compilation::Continue,
            Err(err) => {
                self.error = Some(err);
                Compilation::Stop
            },
        }
    }
}

pub(crate) struct SelectionResult {
    pub report:                 Report,
    pub check_duration:         Duration,
    pub compiler_warning_count: usize,
    pub compiler_fixable_count: usize,
}

fn prepare_findings_dir(target_directory: &Path) -> Result<PathBuf> {
    let findings_dir = target_directory.join("mend-findings");
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create findings directory {}",
            findings_dir.display()
        )
    })?;
    Ok(findings_dir)
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

pub(crate) fn driver_main() -> ExitCode {
    match driver_main_impl() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("mend: {err:#}");
            ExitCode::from(1)
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
    Ok(exit_code_from_i32(status.code().unwrap_or(1)))
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

fn is_progress_line(line: &str) -> bool {
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

fn sanitize_for_match(line: &str) -> String {
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

fn classify_diagnostic_block(block: &[String]) -> DiagnosticBlockKind {
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

fn flush_diagnostic_block(
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

fn load_report(
    findings_dir: &Path,
    selection: &Selection,
    config_fingerprint: &str,
    scope_fingerprint: &str,
) -> Result<Report> {
    let selected_roots: Vec<PathBuf> = selection.package_roots.clone();
    let selected_root_strings: Vec<String> = selected_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
    let selected_canonical_roots: Vec<PathBuf> = selected_roots
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .collect();
    let mut findings = Vec::new();
    let mut pub_use_fix_facts = Vec::new();

    for entry in fs::read_dir(findings_dir).with_context(|| {
        format!(
            "failed to read findings directory {}",
            findings_dir.display()
        )
    })? {
        let entry = entry?;
        if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read findings file {}", entry.path().display()))?;
        let Ok(stored) = serde_json::from_str::<StoredReport>(&text) else {
            continue;
        };
        if !stored_report_matches_selection(
            &stored,
            &selected_roots,
            &selected_root_strings,
            &selected_canonical_roots,
            config_fingerprint,
            scope_fingerprint,
        ) {
            continue;
        }
        extend_report_from_stored(
            &mut findings,
            &mut pub_use_fix_facts,
            stored,
            selection.analysis_root.as_path(),
        );
    }

    findings.sort_by(|a, b| {
        (
            a.severity, &a.path, a.line, a.column, &a.code, &a.item, &a.message,
        )
            .cmp(&(
                b.severity, &b.path, b.line, b.column, &b.code, &b.item, &b.message,
            ))
    });
    findings.dedup_by(|a, b| {
        a.severity == b.severity
            && a.code == b.code
            && a.path == b.path
            && a.line == b.line
            && a.column == b.column
            && a.message == b.message
            && a.item == b.item
    });

    Ok(Report {
        root: selection_root_string(selection.analysis_root.as_path()),
        summary: ReportSummary::default(),
        findings,
        facts: ReportFacts {
            pub_use:           PubUseFixFacts::from_vec(pub_use_fix_facts),
            compiler_warnings: CompilerWarningFacts::None,
        },
    })
}

fn stored_report_matches_selection(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
    config_fingerprint: &str,
    scope_fingerprint: &str,
) -> bool {
    stored.version == FINDINGS_SCHEMA_VERSION
        && stored.analysis_fingerprint == current_analysis_fingerprint()
        && stored.config_fingerprint == config_fingerprint
        && stored.scope_fingerprint == scope_fingerprint
        && stored_crate_root_exists(stored)
        && stored_matches_selected_root(
            stored,
            selected_roots,
            selected_root_strings,
            selected_canonical_roots,
        )
}

fn stored_crate_root_exists(stored: &StoredReport) -> bool {
    stored.crate_root_file.is_empty() || {
        let crate_root = Path::new(&stored.crate_root_file);
        if crate_root.is_absolute() {
            crate_root.exists()
        } else {
            Path::new(&stored.package_root).join(crate_root).exists()
        }
    }
}

fn stored_matches_selected_root(
    stored: &StoredReport,
    selected_roots: &[PathBuf],
    selected_root_strings: &[String],
    selected_canonical_roots: &[PathBuf],
) -> bool {
    selected_root_strings
        .iter()
        .any(|root| root == &stored.package_root)
        || fs::canonicalize(Path::new(&stored.package_root))
            .ok()
            .is_some_and(|stored_root| {
                selected_canonical_roots
                    .iter()
                    .any(|selected_root| selected_root == &stored_root)
            })
        || (stored.package_root.is_empty() && selected_roots.len() == 1)
}

fn extend_report_from_stored(
    findings: &mut Vec<Finding>,
    pub_use_fix_facts: &mut Vec<PubUseFixFact>,
    stored: StoredReport,
    analysis_root: &Path,
) {
    for finding in stored.findings {
        findings.push(Finding {
            severity:      finding.severity,
            code:          finding.code,
            path:          relativize_path(&finding.path, analysis_root),
            line:          finding.line,
            column:        finding.column,
            highlight_len: finding.highlight_len,
            source_line:   finding.source_line,
            item:          finding.item,
            message:       finding.message,
            suggestion:    finding.suggestion,
            fixability:    finding.fixability,
            related:       finding
                .related
                .map(|related| relativize_path(&related, analysis_root)),
        });
    }
    for fact in stored.pub_use_fix_facts {
        pub_use_fix_facts.push(PubUseFixFact {
            child_path:      relativize_path(&fact.child_path, analysis_root),
            child_line:      fact.child_line,
            child_item_name: fact.child_item_name,
            parent_path:     relativize_path(&fact.parent_path, analysis_root),
            parent_line:     fact.parent_line,
            child_module:    fact.child_module,
        });
    }
}

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    absolute.strip_prefix(analysis_root).map_or_else(
        |_| path.to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

fn config_relative_path(file_path: &Path, config_root: &Path) -> Option<String> {
    file_path
        .strip_prefix(config_root)
        .ok()
        .map(normalize_relative_path)
        .or_else(|| {
            let canonical_file = fs::canonicalize(file_path).ok()?;
            let canonical_root = fs::canonicalize(config_root).ok()?;
            canonical_file
                .strip_prefix(canonical_root)
                .ok()
                .map(normalize_relative_path)
        })
}

fn config_relative_path_for_settings(
    file_path: &Path,
    settings: &DriverSettings,
) -> Option<String> {
    if file_path.is_relative() {
        let workspace_relative = normalize_relative_path(file_path);
        if settings.config_root.join(file_path).exists() {
            return Some(workspace_relative);
        }

        let package_relative = settings.package_root.join(file_path);
        return config_relative_path(&package_relative, &settings.config_root)
            .or(Some(workspace_relative));
    }

    config_relative_path(file_path, &settings.config_root)
}

fn normalize_relative_path(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }

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

fn collect_and_store_findings(tcx: TyCtxt<'_>, settings: &DriverSettings) -> Result<bool> {
    let crate_root_file = real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(src_root) = analysis_source_root_for(&crate_root_file, &settings.package_root) else {
        return Ok(false);
    };

    let mut sink = FindingsSink::default();
    let crate_items = tcx.hir_crate_items(());
    let cache_roots: Vec<&Path> = if settings.config_root == settings.package_root {
        vec![&src_root]
    } else {
        vec![&src_root, &settings.config_root]
    };
    let source_cache = SourceCache::build(&cache_roots)?;
    let ctx = VisibilityContext {
        tcx,
        settings,
        src_root: &src_root,
        root_module: &crate_root_file,
        effective_visibilities: tcx.effective_visibilities(()),
        source_cache: &source_cache,
    };

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        analyze_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.impl_items() {
        let item = tcx.hir_impl_item(item_id);
        analyze_impl_item(&ctx, item, &mut sink)?;
    }

    for item_id in crate_items.foreign_items() {
        let item = tcx.hir_foreign_item(item_id);
        analyze_foreign_item(&ctx, item, &mut sink)?;
    }

    let output_path = settings
        .findings_dir
        .join(cache_filename_for(&settings.package_root, &crate_root_file));
    let stored_crate_root = if crate_root_file.is_absolute() {
        crate_root_file.clone()
    } else {
        settings.config_root.join(&crate_root_file)
    };
    if !sink.findings.is_empty() {
        sink.findings.sort_by(|a, b| {
            (&a.path, a.line, a.column, &a.code, &a.item, &a.message)
                .cmp(&(&b.path, b.line, b.column, &b.code, &b.item, &b.message))
        });
        sink.findings.dedup_by(|a, b| {
            a.code == b.code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
        });
    }

    let report = StoredReport {
        version:              FINDINGS_SCHEMA_VERSION,
        analysis_fingerprint: settings.analysis_fingerprint.clone(),
        scope_fingerprint:    settings.scope_fingerprint.clone(),
        package_root:         settings.package_root.to_string_lossy().into_owned(),
        crate_root_file:      stored_crate_root.to_string_lossy().into_owned(),
        config_fingerprint:   settings.config_fingerprint.clone(),
        findings:             sink.findings,
        pub_use_fix_facts:    sink.pub_use_fix_facts,
        compiler_warnings:    CompilerWarningFacts::None,
    };
    fs::write(&output_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write findings file {}", output_path.display()))?;
    Ok(true)
}

fn analysis_source_root_for(crate_root_file: &Path, package_root: &Path) -> Option<PathBuf> {
    let source_root = crate_root_file.parent()?.to_path_buf();
    let canonical_crate_root =
        fs::canonicalize(crate_root_file).unwrap_or_else(|_| crate_root_file.to_path_buf());
    let canonical_package_root =
        fs::canonicalize(package_root).unwrap_or_else(|_| package_root.to_path_buf());
    let relative = canonical_crate_root
        .strip_prefix(&canonical_package_root)
        .ok()?;
    let first_component = relative.components().next()?.as_os_str().to_str()?;
    matches!(first_component, "src" | "examples" | "tests" | "benches").then_some(source_root)
}

#[derive(Default)]
struct FindingsSink {
    findings:          Vec<StoredFinding>,
    pub_use_fix_facts: Vec<StoredPubUseFixFact>,
}

/// Pre-loaded source file contents and parsed ASTs, built once before the analysis
/// loops to avoid re-reading and re-parsing the same `.rs` files hundreds of times
/// during visibility analysis.
struct ExtractedPaths {
    /// Flattened use-tree paths with their origin (`Relative`/`Crate`).
    use_paths:   Vec<(Vec<String>, PathOrigin)>,
    /// All `syn::Path` nodes found via AST visit, as raw segment strings with origin.
    expr_paths:  Vec<(Vec<String>, PathOrigin)>,
    /// Module-level renames (`use path::to::module as alias`): maps alias → original path.
    use_renames: Vec<UseRename>,
}

struct UseRename {
    alias:         String,
    original_path: Vec<String>,
}

struct SourceCache {
    contents:        HashMap<PathBuf, String>,
    files_by_dir:    HashMap<PathBuf, Vec<PathBuf>>,
    parsed:          HashMap<PathBuf, syn::File>,
    extracted_paths: HashMap<PathBuf, ExtractedPaths>,
}

impl SourceCache {
    fn build(roots: &[&Path]) -> Result<Self> {
        let mut contents = HashMap::new();
        for root in roots {
            for file in rust_source_files(root)? {
                contents
                    .entry(file.clone())
                    .or_insert(fs::read_to_string(&file).with_context(|| {
                        format!("failed to pre-read source file {}", file.display())
                    })?);
            }
        }
        let mut files_by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for path in contents.keys() {
            if let Some(parent) = path.parent() {
                files_by_dir
                    .entry(parent.to_path_buf())
                    .or_default()
                    .push(path.clone());
            }
        }
        let mut parsed = HashMap::new();
        for (path, source) in &contents {
            if let Ok(ast) = syn::parse_file(source) {
                parsed.insert(path.clone(), ast);
            }
        }
        let mut extracted_paths = HashMap::new();
        for (path, ast) in &parsed {
            extracted_paths.insert(path.clone(), extract_paths(ast));
        }
        Ok(Self {
            contents,
            files_by_dir,
            parsed,
            extracted_paths,
        })
    }

    fn source_files_under(&self, dir: &Path) -> Vec<&Path> {
        self.files_by_dir
            .iter()
            .filter(|(d, _)| d.starts_with(dir))
            .flat_map(|(_, files)| files.iter().map(PathBuf::as_path))
            .collect()
    }

    fn read_source(&self, path: &Path) -> Result<&str> {
        self.contents
            .get(path)
            .map(String::as_str)
            .with_context(|| format!("source file not in cache: {}", path.display()))
    }

    fn parsed_file(&self, path: &Path) -> Option<&syn::File> { self.parsed.get(path) }

    fn extracted_paths(&self, path: &Path) -> Option<&ExtractedPaths> {
        self.extracted_paths.get(path)
    }
}

struct VisibilityContext<'a, 'tcx> {
    tcx:                    TyCtxt<'tcx>,
    settings:               &'a DriverSettings,
    src_root:               &'a Path,
    root_module:            &'a Path,
    effective_visibilities: &'a rustc_middle::middle::privacy::EffectiveVisibilities,
    source_cache:           &'a SourceCache,
}

struct ItemInfo<'a> {
    def_id:         LocalDefId,
    file_path:      &'a Path,
    vis_text:       &'a str,
    kind_label:     Option<&'static str>,
    item_name:      Option<&'a str>,
    highlight_span: Span,
    is_module_item: bool,
    impl_self_name: Option<String>,
}

struct SuspiciousPubInput<'a> {
    def_id:           LocalDefId,
    file_path:        &'a Path,
    config_rel_path:  Option<&'a str>,
    parent_is_public: bool,
    module_location:  ModuleLocation,
    crate_kind:       CrateKind,
    kind_label:       Option<&'static str>,
    item_name:        Option<&'a str>,
    highlight_span:   Span,
}

fn cache_filename_for(package_root: &Path, crate_root_file: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    package_root.hash(&mut hasher);
    crate_root_file.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}

fn analyze_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &Item<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.kind.ident().map(|ident| ident.to_string());

    if vis_text == "pub"
        && is_boundary_file(ctx.src_root, ctx.root_module, &file_path)
        && matches!(item.kind, ItemKind::Use(..))
        && use_item_contains_glob(ctx.tcx, item.span)?
    {
        sink.findings.push(build_finding(
            ctx.tcx,
            &file_path,
            item.span,
            FindingParams {
                severity:   Severity::Warning,
                code:       DiagnosticCode::WildcardParentPubUse,
                item:       None,
                message:    String::new(),
                suggestion: None,
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     item_kind_label(item.kind),
            item_name:      item_name.as_deref(),
            highlight_span: highlight_span(
                item.vis_span,
                item.kind.ident().map(|ident| ident.span),
            ),
            is_module_item: matches!(item.kind, ItemKind::Mod(..)),
            impl_self_name: None,
        },
        sink,
    )
}

fn analyze_impl_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ImplItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(vis_span) = item.vis_span() else {
        return Ok(());
    };
    if item.span.from_expansion() || vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    let impl_self_name = impl_self_type_name_from_tcx(ctx.tcx, item.owner_id.def_id);

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id: item.owner_id.def_id,
            file_path: &file_path,
            vis_text: &vis_text,
            kind_label: Some(impl_item_kind_label(item.kind)),
            item_name: Some(item_name.as_str()),
            highlight_span: highlight_span(vis_span, Some(item.ident.span)),
            is_module_item: false,
            impl_self_name,
        },
        sink,
    )
}

fn analyze_foreign_item(
    ctx: &VisibilityContext<'_, '_>,
    item: &ForeignItem<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(ctx.tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(ctx.tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    record_visibility_findings(
        ctx,
        &ItemInfo {
            def_id:         item.owner_id.def_id,
            file_path:      &file_path,
            vis_text:       &vis_text,
            kind_label:     Some(foreign_item_kind_label(item.kind)),
            item_name:      Some(item_name.as_str()),
            highlight_span: highlight_span(item.vis_span, Some(item.ident.span)),
            is_module_item: false,
            impl_self_name: None,
        },
        sink,
    )
}

fn record_visibility_findings(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let crate_kind = if ctx.root_module.file_name().and_then(|name| name.to_str()) == Some("lib.rs")
    {
        CrateKind::Library
    } else {
        CrateKind::Binary
    };
    let config_rel_path = config_relative_path_for_settings(item.file_path, ctx.settings);
    let parent_module = ctx.tcx.parent_module_from_def_id(item.def_id);
    let parent_is_public = ctx
        .tcx
        .local_visibility(parent_module.to_local_def_id())
        .is_public();
    let module_location = resolve_module_location(ctx.tcx, parent_module.to_local_def_id());

    if matches!(item.vis_text, "pub(crate)")
        && !allow_pub_crate_by_policy(crate_kind, module_location, parent_is_public)
    {
        sink.findings.push(build_finding(
            ctx.tcx,
            item.file_path,
            item.highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       DiagnosticCode::ForbiddenPubCrate,
                item:       None,
                message:    "use of `pub(crate)` is forbidden by policy".to_string(),
                suggestion: Some(forbidden_pub_crate_help(module_location).to_string()),
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    if item.vis_text.starts_with("pub(in crate::") {
        sink.findings.push(build_finding(
            ctx.tcx,
            item.file_path,
            item.highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       DiagnosticCode::ForbiddenPubInCrate,
                item:       None,
                message:    "use of `pub(in crate::...)` is forbidden by policy".to_string(),
                suggestion: None,
                fixability: FixSupport::None,
                related:    None,
            },
        )?);
    }

    if item.is_module_item && item.vis_text.starts_with("pub") {
        let allowlisted = config_rel_path.as_ref().is_some_and(|path| {
            ctx.settings
                .config
                .allow_pub_mod
                .iter()
                .any(|allowed| allowed == path)
        });
        if !allowlisted {
            sink.findings.push(build_finding(
                ctx.tcx,
                item.file_path,
                item.highlight_span,
                FindingParams {
                    severity:   Severity::Error,
                    code:       DiagnosticCode::ReviewPubMod,
                    item:       item.item_name.map(str::to_owned),
                    message:    "`pub mod` requires explicit review or allowlisting".to_string(),
                    suggestion: None,
                    fixability: FixSupport::None,
                    related:    None,
                },
            )?);
        }
    }

    if item.vis_text == "pub"
        && !parent_is_public
        && is_top_level_module_file(ctx.src_root, ctx.root_module, item.file_path)
    {
        maybe_record_narrow_to_pub_crate(ctx, item, sink)?;
    }

    if item.vis_text == "pub" && !is_boundary_file(ctx.src_root, ctx.root_module, item.file_path) {
        maybe_record_suspicious_pub(
            ctx,
            &SuspiciousPubInput {
                def_id: item.def_id,
                file_path: item.file_path,
                config_rel_path: config_rel_path.as_deref(),
                parent_is_public,
                module_location,
                crate_kind,
                kind_label: item.kind_label,
                item_name: item.item_name,
                highlight_span: item.highlight_span,
            },
            sink,
        )?;
    }
    Ok(())
}

fn maybe_record_narrow_to_pub_crate(
    ctx: &VisibilityContext<'_, '_>,
    item: &ItemInfo<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let (Some(item_name), Some(kind_label)) = (item.item_name, item.kind_label) else {
        return Ok(());
    };
    // Check if the item itself is re-exported by the crate root.
    if root_module_exports_item(ctx.source_cache, ctx.root_module, item.file_path, item_name) {
        return Ok(());
    }
    // For impl items (methods, consts, types), also check if the self type
    // is re-exported — pub methods on re-exported types must stay pub.
    if let Some(self_name) = &item.impl_self_name
        && root_module_exports_item(ctx.source_cache, ctx.root_module, item.file_path, self_name)
    {
        return Ok(());
    }
    sink.findings.push(build_finding(
        ctx.tcx,
        item.file_path,
        item.highlight_span,
        FindingParams {
            severity:   Severity::Warning,
            code:       DiagnosticCode::NarrowToPubCrate,
            item:       Some(format!("{kind_label} {item_name}")),
            message:    String::from(
                "item is not re-exported by the crate root — use `pub(crate)`",
            ),
            suggestion: Some(String::from("consider using: `pub(crate)`")),
            fixability: FixSupport::NarrowToPubCrate,
            related:    None,
        },
    )?);
    Ok(())
}

fn maybe_record_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
    sink: &mut FindingsSink,
) -> Result<()> {
    let Some(kind_label) = input.kind_label else {
        return Ok(());
    };

    match classify_suspicious_pub(ctx, input)? {
        SuspiciousPubAssessment::Allowed(_) => {},
        SuspiciousPubAssessment::ReviewInternalParentFacade { related } => {
            let Some(status) = input
                .item_name
                .map(|name| {
                    parent_facade_export_status(
                        ctx.source_cache,
                        ctx.settings,
                        ctx.src_root,
                        input.file_path,
                        name,
                    )
                })
                .transpose()?
                .flatten()
            else {
                return Ok(());
            };
            sink.findings.push(build_line_finding(
                ctx.source_cache,
                &status.parent_path,
                status.parent_line,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::InternalParentPubUseFacade,
                    item: input.item_name.map(|name| format!("pub use {name}")),
                    message: String::from(
                        "this `pub use` is used inside its parent module subtree",
                    ),
                    suggestion: None,
                    fixability: FixSupport::InternalParentFacade,
                    related,
                },
            )?);
        },
        SuspiciousPubAssessment::Warn {
            fixability,
            related,
            stale_parent_pub_use,
        } => {
            sink.findings.push(build_finding(
                ctx.tcx,
                input.file_path,
                input.highlight_span,
                FindingParams {
                    severity: Severity::Warning,
                    code: DiagnosticCode::SuspiciousPub,
                    item: input.item_name.map(|name| format!("{kind_label} {name}")),
                    message: suspicious_pub_note(input.crate_kind, kind_label),
                    suggestion: None,
                    fixability,
                    related,
                },
            )?);
            if let (Some(status), Some(item_name)) = (stale_parent_pub_use, input.item_name)
                && fixability == FixSupport::FixPubUse
            {
                let display = line_display(ctx.tcx, input.file_path, input.highlight_span)?;
                let Some(child_module) = input
                    .file_path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .filter(|stem| *stem != "mod")
                    .map(str::to_string)
                else {
                    return Ok(());
                };
                sink.pub_use_fix_facts.push(StoredPubUseFixFact {
                    child_path: input.file_path.to_string_lossy().into_owned(),
                    child_line: display.line,
                    child_item_name: item_name.to_string(),
                    parent_path: status.parent_path.to_string_lossy().into_owned(),
                    parent_line: status.parent_line,
                    child_module,
                });
            }
        },
    }
    Ok(())
}

fn classify_suspicious_pub(
    ctx: &VisibilityContext<'_, '_>,
    input: &SuspiciousPubInput<'_>,
) -> Result<SuspiciousPubAssessment> {
    if let Some(allowance) = basic_suspicious_pub_allowance(
        ctx.settings,
        ctx.effective_visibilities,
        input.def_id,
        input.config_rel_path,
        input.parent_is_public,
        input.item_name,
    ) {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let parent_facade_export = input
        .item_name
        .map(|name| {
            parent_facade_export_status(
                ctx.source_cache,
                ctx.settings,
                ctx.src_root,
                input.file_path,
                name,
            )
        })
        .transpose()?
        .flatten();

    if let Some(assessment) = assess_parent_facade_usage(parent_facade_export.as_ref()) {
        return Ok(assessment);
    }

    if let Some(allowance) = assess_signature_exposure_allowance(
        ctx.source_cache,
        ctx.settings,
        ctx.src_root,
        input.file_path,
        input.item_name,
    )? {
        return Ok(SuspiciousPubAssessment::Allowed(allowance));
    }

    let stale_result = parent_facade_export.as_ref().and_then(|status| {
        let message = match status.usage {
            ParentFacadeUsage::Unused => format!(
                "parent module also has an `unused import` warning for this `pub use` at {}:{}",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
            | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport => format!(
                "parent `pub use` at {}:{} is only used through crate-relative paths inside its own subtree",
                status.parent_rel_path, status.parent_line
            ),
            ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport
            | ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
            | ParentFacadeUsage::UsedOutsideParentSubtree => return None,
        };
        Some((message, status))
    });

    if matches!(input.module_location, ModuleLocation::TopLevelPrivateModule)
        && stale_result.is_none()
    {
        return Ok(SuspiciousPubAssessment::Allowed(
            AllowanceReason::TopLevelPrivateModulePolicy,
        ));
    }

    let (related, fixability, stale_parent_pub_use) = match stale_result {
        Some((message, status)) => {
            let fix = if status.fix_supported {
                FixSupport::FixPubUse
            } else {
                FixSupport::NeedsManualPubUseCleanup
            };
            (Some(message), fix, Some(status.clone()))
        },
        None => (None, FixSupport::None, None),
    };

    Ok(SuspiciousPubAssessment::Warn {
        fixability,
        related,
        stale_parent_pub_use,
    })
}

fn basic_suspicious_pub_allowance(
    settings: &DriverSettings,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    def_id: LocalDefId,
    config_rel_path: Option<&str>,
    parent_is_public: bool,
    item_name: Option<&str>,
) -> Option<AllowanceReason> {
    let item_key = config_rel_path.and_then(|path| item_name.map(|name| format!("{path}::{name}")));
    let allowlisted = item_key.as_ref().is_some_and(|key| {
        settings
            .config
            .allow_pub_items
            .iter()
            .any(|allowed| allowed == key)
    });
    if allowlisted {
        return Some(AllowanceReason::Allowlist);
    }
    if parent_is_public {
        return Some(AllowanceReason::ParentIsPublic);
    }
    if effective_visibilities.is_public_at_level(def_id, Level::Reachable) {
        return Some(AllowanceReason::ReachablePublicApi);
    }
    None
}

fn assess_parent_facade_usage(
    parent_facade_export: Option<&ParentFacadeExportStatus>,
) -> Option<SuspiciousPubAssessment> {
    let status = parent_facade_export?;
    if status.visibility == ParentFacadeVisibility::Super
        && !matches!(status.usage, ParentFacadeUsage::Unused)
    {
        return Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::InternalParentFacadeBoundary,
        ));
    }
    match status.usage {
        ParentFacadeUsage::UsedOutsideParentSubtree => Some(SuspiciousPubAssessment::Allowed(
            AllowanceReason::ParentFacadeUsedOutsideParent,
        )),
        ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport => {
            let related = Some(format!(
                "parent module uses this item as an internal facade at {}:{}",
                status.parent_rel_path, status.parent_line
            ));
            Some(SuspiciousPubAssessment::ReviewInternalParentFacade { related })
        },
        ParentFacadeUsage::UsedInsideParentSubtreeByCratePath
        | ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport
        | ParentFacadeUsage::Unused => None,
    }
}

fn assess_signature_exposure_allowance(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    file_path: &Path,
    item_name: Option<&str>,
) -> Result<Option<AllowanceReason>> {
    let Some(item_name) = item_name else {
        return Ok(None);
    };
    if child_item_is_exposed_by_other_crate_visible_signature(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || impl_item_is_exposed_by_exported_self_type(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || child_item_is_exposed_by_sibling_boundary_signature(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? || parent_boundary_public_signature_exposes_child_used_outside_parent(
        source_cache,
        settings,
        src_root,
        file_path,
        item_name,
    )? {
        return Ok(Some(AllowanceReason::ExposedByOtherCrateVisibleSignature));
    }
    Ok(None)
}

struct FindingParams {
    severity:   Severity,
    code:       DiagnosticCode,
    item:       Option<String>,
    message:    String,
    suggestion: Option<String>,
    fixability: FixSupport,
    related:    Option<String>,
}

fn build_finding(
    tcx: TyCtxt<'_>,
    file_path: &Path,
    highlight_span: Span,
    params: FindingParams,
) -> Result<StoredFinding> {
    let display = line_display(tcx, file_path, highlight_span)?;
    Ok(StoredFinding {
        severity:      params.severity,
        code:          params.code,
        path:          file_path.to_string_lossy().into_owned(),
        line:          display.line,
        column:        display.column,
        highlight_len: display.highlight_len,
        source_line:   display.source_line,
        item:          params.item,
        message:       params.message,
        suggestion:    params.suggestion,
        fixability:    params.fixability,
        related:       params.related,
    })
}

fn build_line_finding(
    source_cache: &SourceCache,
    file_path: &Path,
    line: usize,
    params: FindingParams,
) -> Result<StoredFinding> {
    let text = source_cache.read_source(file_path)?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();
    let trimmed = source_line.trim_start();
    let column = source_line.len().saturating_sub(trimmed.len()) + 1;
    let highlight_len = trimmed
        .find(char::is_whitespace)
        .unwrap_or(trimmed.len())
        .max(1);

    Ok(StoredFinding {
        severity: params.severity,
        code: params.code,
        path: file_path.to_string_lossy().into_owned(),
        line,
        column,
        highlight_len,
        source_line,
        item: params.item,
        message: params.message,
        suggestion: params.suggestion,
        fixability: params.fixability,
        related: params.related,
    })
}

fn resolve_module_location(tcx: TyCtxt<'_>, parent_def: LocalDefId) -> ModuleLocation {
    if parent_def == CRATE_DEF_ID {
        return ModuleLocation::CrateRoot;
    }

    let grandparent = tcx.parent_module_from_def_id(parent_def).to_local_def_id();
    if grandparent == CRATE_DEF_ID {
        return ModuleLocation::TopLevelPrivateModule;
    }

    let great_grandparent = tcx.parent_module_from_def_id(grandparent).to_local_def_id();
    if great_grandparent == CRATE_DEF_ID {
        return ModuleLocation::TopLevelPrivateModule;
    }

    ModuleLocation::NestedModule
}

/// Check whether `root_module` (lib.rs / main.rs) re-exports `item_name`
/// from the child module that `child_file` belongs to.
fn root_module_exports_item(
    source_cache: &SourceCache,
    root_module: &Path,
    child_file: &Path,
    item_name: &str,
) -> bool {
    let Some(child_module_name) = module_paths::module_name_for_child_boundary_file(child_file)
    else {
        return false;
    };
    let Some(file) = source_cache.parsed_file(root_module) else {
        return false;
    };
    let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);
    !exports.explicit.is_empty()
}

fn parent_facade_export_status(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<Option<ParentFacadeExportStatus>> {
    let Some(initial_boundary) = parent_boundary_for_child(src_root, child_file) else {
        return Ok(None);
    };

    // Walk up from the immediate parent through ancestors until we find a
    // boundary that re-exports `item_name`, or run out of ancestors.
    let mut current_child: PathBuf = child_file.to_path_buf();
    let mut parent_boundary = initial_boundary;

    let exported_names = loop {
        let Some(child_module_name) =
            module_paths::module_name_for_child_boundary_file(&current_child)
        else {
            return Ok(None);
        };

        let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
            return Ok(None);
        };
        let exports = exported_names_from_parent_boundary(file, child_module_name, item_name);

        if !exports.explicit.is_empty() {
            break exports;
        }

        // Not found at this level — walk up to the next ancestor.
        current_child.clone_from(&parent_boundary.boundary_file);
        let Some(next_boundary) = parent_of_boundary(src_root, &current_child) else {
            return Ok(None);
        };
        parent_boundary = next_boundary;
    };

    let parent_rel_path = parent_boundary
        .boundary_file
        .strip_prefix(src_root)
        .unwrap_or(&parent_boundary.boundary_file)
        .to_string_lossy()
        .replace('\\', "/");
    let parent_source = source_cache.read_source(&parent_boundary.boundary_file)?;
    let parent_line = first_line_matching(parent_source, item_name).unwrap_or(1);

    let usage = scan_facade_usage(
        source_cache,
        settings,
        src_root,
        &parent_boundary,
        &exported_names,
    )?;

    Ok(Some(ParentFacadeExportStatus {
        usage,
        fix_supported: exported_names.fix_supported,
        visibility: exported_names
            .visibility
            .unwrap_or(ParentFacadeVisibility::Public),
        parent_path: parent_boundary.boundary_file,
        parent_rel_path,
        parent_line,
    }))
}

fn scan_facade_usage(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    parent_boundary: &ParentBoundary,
    exported_names: &ParentFacadeExports,
) -> Result<ParentFacadeUsage> {
    let mut usage = ParentFacadeUsage::Unused;
    for source_path in source_cache.source_files_under(src_root) {
        if source_path == parent_boundary.boundary_file {
            continue;
        }
        let Some(current_module_path) = module_path_from_source_file(src_root, source_path) else {
            continue;
        };
        let Some(extracted) = source_cache.extracted_paths(source_path) else {
            continue;
        };
        match source_references_parent_export(
            extracted,
            &current_module_path,
            &parent_boundary.module_path,
            &exported_names.explicit,
        ) {
            ParentFacadeReferenceUsage::None => {},
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative) => {
                if matches!(usage, ParentFacadeUsage::Unused)
                    && source_path.starts_with(&parent_boundary.subtree_root)
                {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByRelativeImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate) => {
                if matches!(usage, ParentFacadeUsage::Unused)
                    && source_path.starts_with(&parent_boundary.subtree_root)
                {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByCrateImport;
                } else if !source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByRelativePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate) => {
                if source_path.starts_with(&parent_boundary.subtree_root) {
                    usage = ParentFacadeUsage::UsedInsideParentSubtreeByCratePath;
                } else {
                    usage = ParentFacadeUsage::UsedOutsideParentSubtree;
                    break;
                }
            },
        }
    }

    if !matches!(usage, ParentFacadeUsage::UsedOutsideParentSubtree)
        && workspace_source_mentions_parent_export_literal(
            source_cache,
            settings,
            parent_boundary,
            &exported_names.explicit,
        )?
    {
        usage = ParentFacadeUsage::UsedOutsideParentSubtree;
    }

    Ok(usage)
}

fn workspace_source_mentions_parent_export_literal(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    parent_boundary: &ParentBoundary,
    exported_names: &[String],
) -> Result<bool> {
    if settings.config_root == settings.package_root {
        return Ok(false);
    }

    if parent_boundary.module_path.is_empty() {
        return Ok(false);
    }

    let module_prefix = format!("crate::{}", parent_boundary.module_path.join("::"));
    let findings_root = settings
        .findings_dir
        .parent()
        .map_or_else(|| settings.findings_dir.clone(), Path::to_path_buf);

    for file in source_cache.source_files_under(&settings.config_root) {
        if file.starts_with(&settings.package_root)
            || file.starts_with(&settings.findings_dir)
            || file.starts_with(&findings_root)
        {
            continue;
        }
        let source = source_cache.read_source(file)?;
        if exported_names.iter().any(|name| {
            let pattern = format!("{module_prefix}::{name}");
            source.contains(&pattern)
        }) {
            return Ok(true);
        }
    }

    Ok(false)
}

fn parent_boundary_for_child(src_root: &Path, child_file: &Path) -> Option<ParentBoundary> {
    let parent_dir = child_file.parent()?;
    let parent_mod_rs = parent_dir.join("mod.rs");
    if parent_mod_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_mod_rs,
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   module_path_from_dir(src_root, parent_dir)?,
        });
    }

    let parent_file = parent_dir.with_extension("rs");
    if parent_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: parent_file.clone(),
            subtree_root:  parent_dir.to_path_buf(),
            module_path:   module_path_from_boundary_file(src_root, &parent_file)?,
        });
    }

    None
}

/// Find the parent boundary of an existing boundary file itself.
///
/// `parent_boundary_for_child` cannot be called on a `mod.rs` file because it
/// would find itself.  This helper handles both `mod.rs` and named boundary
/// files (e.g. `tools.rs`).
fn parent_of_boundary(src_root: &Path, boundary_file: &Path) -> Option<ParentBoundary> {
    if boundary_file.file_name()?.to_str() != Some("mod.rs") {
        return parent_boundary_for_child(src_root, boundary_file);
    }

    // For mod.rs the enclosing directory IS the module, so go up one more
    // level to reach the parent module's directory.
    let container_dir = boundary_file.parent()?.parent()?;

    let mod_rs = container_dir.join("mod.rs");
    if mod_rs.is_file() {
        return Some(ParentBoundary {
            boundary_file: mod_rs,
            subtree_root:  container_dir.to_path_buf(),
            module_path:   module_path_from_dir(src_root, container_dir)?,
        });
    }

    let named_file = container_dir.with_extension("rs");
    if named_file.is_file() {
        return Some(ParentBoundary {
            boundary_file: named_file.clone(),
            subtree_root:  container_dir.to_path_buf(),
            module_path:   module_path_from_boundary_file(src_root, &named_file)?,
        });
    }

    for name in ["lib.rs", "main.rs"] {
        let root = container_dir.join(name);
        if root.is_file() {
            return Some(ParentBoundary {
                boundary_file: root,
                subtree_root:  container_dir.to_path_buf(),
                module_path:   Vec::new(),
            });
        }
    }

    None
}

fn module_path_from_boundary_file(src_root: &Path, boundary_file: &Path) -> Option<Vec<String>> {
    let relative = boundary_file.strip_prefix(src_root).ok()?;
    let mut components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let last = components.last_mut()?;
    *last = last.strip_suffix(".rs")?.to_string();
    if matches!(components.as_slice(), [name] if name == "lib" || name == "main") {
        Some(Vec::new())
    } else {
        Some(components)
    }
}

fn module_path_from_source_file(src_root: &Path, source_file: &Path) -> Option<Vec<String>> {
    if source_file.file_name().and_then(|name| name.to_str()) == Some("mod.rs") {
        module_path_from_dir(src_root, source_file.parent()?)
    } else {
        module_path_from_boundary_file(src_root, source_file)
    }
}

fn exported_names_from_parent_boundary(
    file: &syn::File,
    child_module_name: &str,
    item_name: &str,
) -> ParentFacadeExports {
    let mut exported = ParentFacadeExports::default();
    for item in &file.items {
        let syn::Item::Use(item_use) = item else {
            continue;
        };
        let Some(visibility) = parent_facade_visibility(&item_use.vis) else {
            continue;
        };
        exported.visibility = Some(exported.visibility.map_or(visibility, |existing| existing));
        collect_matching_pub_use_exports(item_use, child_module_name, item_name, &mut exported);
    }
    exported.explicit.sort();
    exported.explicit.dedup();
    exported
}

fn collect_matching_pub_use_exports(
    item_use: &ItemUse,
    child_module_name: &str,
    item_name: &str,
    exported: &mut ParentFacadeExports,
) {
    if pub_use_is_fix_supported(&item_use.tree, child_module_name, item_name) {
        exported.fix_supported = true;
    }
    let mut paths = Vec::new();
    flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
    for path in paths {
        let normalized = if path.first().is_some_and(|segment| segment == "self") {
            &path[1..]
        } else {
            &path[..]
        };
        if normalized.len() >= 2
            && normalized[0] == child_module_name
            && normalized[1..].iter().any(|segment| segment == item_name)
            && let Some(export_name) = normalized.last()
        {
            exported.explicit.push(export_name.clone());
        }
    }
}

fn pub_use_is_fix_supported(tree: &UseTree, child_module_name: &str, item_name: &str) -> bool {
    pub_use_is_fix_supported_with_prefix(Vec::new(), tree, child_module_name, item_name)
}

fn pub_use_is_fix_supported_with_prefix(
    prefix: Vec<String>,
    tree: &UseTree,
    child_module_name: &str,
    item_name: &str,
) -> bool {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            pub_use_is_fix_supported_with_prefix(next, &path.tree, child_module_name, item_name)
        },
        UseTree::Name(name) => {
            let normalized = if prefix.first().is_some_and(|segment| segment == "self") {
                &prefix[1..]
            } else {
                &prefix[..]
            };
            normalized.len() == 1 && normalized[0] == child_module_name && name.ident == item_name
        },
        UseTree::Group(group) => group.items.iter().any(|item| {
            pub_use_is_fix_supported_with_prefix(prefix.clone(), item, child_module_name, item_name)
        }),
        UseTree::Rename(_) | UseTree::Glob(_) => false,
    }
}

fn parent_facade_visibility(vis: &syn::Visibility) -> Option<ParentFacadeVisibility> {
    match vis {
        syn::Visibility::Public(_) => Some(ParentFacadeVisibility::Public),
        syn::Visibility::Restricted(restricted)
            if restricted.path.segments.len() == 1
                && restricted.path.segments[0].ident == "super" =>
        {
            Some(ParentFacadeVisibility::Super)
        },
        _ => None,
    }
}

fn flatten_use_tree(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<Vec<String>>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            flatten_use_tree(next, &path.tree, out);
        },
        UseTree::Name(name) => {
            let mut next = prefix;
            next.push(name.ident.to_string());
            out.push(next);
        },
        UseTree::Rename(rename) => {
            let mut next = prefix;
            next.push(rename.ident.to_string());
            next.push(rename.rename.to_string());
            out.push(next);
        },
        UseTree::Group(group) => {
            for item in &group.items {
                flatten_use_tree(prefix.clone(), item, out);
            }
        },
        UseTree::Glob(_) => {
            let mut next = prefix;
            next.push("*".to_string());
            out.push(next);
        },
    }
}

fn use_item_contains_glob(tcx: TyCtxt<'_>, span: Span) -> Result<bool> {
    let snippet = tcx.sess.source_map().span_to_snippet(span).map_err(|err| {
        anyhow::anyhow!("failed to extract use item snippet for span {span:?}: {err:?}")
    })?;
    Ok(snippet.contains('*'))
}

fn first_line_matching(source: &str, needle: &str) -> Option<usize> {
    source
        .lines()
        .position(|line| line.contains(needle))
        .map(|index| index + 1)
}

fn module_path_from_dir(src_root: &Path, module_dir: &Path) -> Option<Vec<String>> {
    let relative = module_dir.strip_prefix(src_root).ok()?;
    let components = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    (!components.is_empty()).then_some(components)
}

fn rust_source_files(src_root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    collect_rust_source_files(src_root, &mut files)?;
    Ok(files)
}

fn collect_rust_source_files(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read source directory {}", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_source_files(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            files.push(path);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathOrigin {
    Relative,
    Crate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFacadeReferenceUsage {
    None,
    Import(PathOrigin),
    DirectPath(PathOrigin),
}

fn source_references_parent_export(
    extracted: &ExtractedPaths,
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> ParentFacadeReferenceUsage {
    for (raw, origin) in &extracted.expr_paths {
        if matching_origin_indexed(
            raw,
            *origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(*origin);
        }
        if let Some(resolved) = resolve_alias_expr_path(raw, &extracted.use_renames)
            && matching_origin_indexed(
                &resolved,
                *origin,
                current_module_path,
                module_path,
                exported_names,
            )
            .is_some()
        {
            return ParentFacadeReferenceUsage::DirectPath(*origin);
        }
    }

    let mut import_usage = ParentFacadeReferenceUsage::None;
    for (raw, origin) in &extracted.use_paths {
        if matching_origin_indexed(
            raw,
            *origin,
            current_module_path,
            module_path,
            exported_names,
        )
        .is_some()
        {
            import_usage =
                merge_reference_usage(import_usage, ParentFacadeReferenceUsage::Import(*origin));
        }
    }

    import_usage
}

/// Resolves the first segment of an `expr_path` through module aliases.
///
/// Given `["test_utils", "assert_test_case"]` and a rename mapping
/// `test_utils → ["crate", "test_support"]`, returns
/// `["crate", "test_support", "assert_test_case"]`.
fn resolve_alias_expr_path(raw: &[String], renames: &[UseRename]) -> Option<Vec<String>> {
    let first = raw.first()?;
    let rename = renames.iter().find(|rename| rename.alias == *first)?;
    let mut resolved = rename.original_path.clone();
    resolved.extend(raw[1..].iter().cloned());
    Some(resolved)
}

fn matching_origin_indexed(
    raw: &[String],
    origin: PathOrigin,
    current_module_path: &[String],
    module_path: &[String],
    exported_names: &[String],
) -> Option<PathOrigin> {
    resolve_module_relative_paths(raw, current_module_path)
        .into_iter()
        .find(|segments| {
            segments.len() == module_path.len() + 1
                && segments[..module_path.len()] == *module_path
                && exported_names
                    .iter()
                    .any(|name| name == &segments[module_path.len()])
        })
        .map(|_| origin)
}

fn resolve_module_relative_paths(
    raw: &[String],
    current_module_path: &[String],
) -> Vec<Vec<String>> {
    if raw.is_empty() {
        return Vec::new();
    }

    if raw.first().map(String::as_str) == Some("crate") {
        return vec![raw[1..].to_vec()];
    }

    if raw.first().map(String::as_str) == Some("self") {
        let mut resolved = current_module_path.to_vec();
        resolved.extend(raw[1..].iter().cloned());
        return vec![resolved];
    }

    if raw.first().map(String::as_str) == Some("super") {
        let mut index = 0usize;
        let mut resolved = current_module_path.to_vec();
        while raw.get(index).is_some_and(|segment| segment == "super") {
            if resolved.pop().is_none() {
                return Vec::new();
            }
            index += 1;
        }
        if raw.get(index).is_some_and(|segment| segment == "self") {
            index += 1;
        }
        resolved.extend(raw[index..].iter().cloned());
        return vec![resolved];
    }

    (0..=current_module_path.len())
        .map(|prefix_len| {
            let mut resolved = current_module_path[..prefix_len].to_vec();
            resolved.extend(raw.iter().cloned());
            resolved
        })
        .collect()
}

fn child_item_is_exposed_by_other_crate_visible_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let Some(exposing_item_name) = public_item_name(item) else {
            continue;
        };
        if exposing_item_name == item_name {
            continue;
        }
        if !public_item_surface_mentions_name(item, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            &exposing_item_name,
        )? {
            return Ok(true);
        }
    }

    for item in &file.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = impl_self_type_name(item_impl) else {
            continue;
        };
        if self_type_name == item_name {
            continue;
        }
        if !outward_impl_surface_mentions_name(item_impl, item_name) {
            continue;
        }
        if type_is_exposed_outside_parent(
            source_cache,
            settings,
            src_root,
            child_file,
            &self_type_name,
        )? {
            return Ok(true);
        }
    }

    Ok(false)
}

fn child_item_is_exposed_by_sibling_boundary_signature(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = parent_boundary_for_child(src_root, child_file) else {
        return Ok(false);
    };

    for candidate_file in source_cache.source_files_under(&parent_boundary.subtree_root) {
        if candidate_file == child_file || candidate_file == parent_boundary.boundary_file {
            continue;
        }

        let Some(file) = source_cache.parsed_file(candidate_file) else {
            continue;
        };

        for item in &file.items {
            let Some(exposing_item_name) = public_item_name(item) else {
                continue;
            };
            if exposing_item_name == item_name {
                continue;
            }
            if !public_item_surface_mentions_name(item, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                src_root,
                candidate_file,
                &exposing_item_name,
            )? {
                return Ok(true);
            }
        }

        for item in &file.items {
            let syn::Item::Impl(item_impl) = item else {
                continue;
            };
            let Some(self_type_name) = impl_self_type_name(item_impl) else {
                continue;
            };
            if self_type_name == item_name {
                continue;
            }
            if !outward_impl_surface_mentions_name(item_impl, item_name) {
                continue;
            }
            if type_is_exposed_outside_parent(
                source_cache,
                settings,
                src_root,
                candidate_file,
                &self_type_name,
            )? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn impl_item_is_exposed_by_exported_self_type(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(file) = source_cache.parsed_file(child_file) else {
        return Ok(false);
    };

    for item in &file.items {
        let syn::Item::Impl(item_impl) = item else {
            continue;
        };
        let Some(self_type_name) = impl_self_type_name(item_impl) else {
            continue;
        };
        for impl_item in &item_impl.items {
            let outward = item_impl.trait_.is_some();
            let is_target = match impl_item {
                syn::ImplItem::Fn(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.sig.ident == item_name =>
                {
                    true
                },
                syn::ImplItem::Const(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                syn::ImplItem::Type(item)
                    if (outward || matches!(item.vis, syn::Visibility::Public(_)))
                        && item.ident == item_name =>
                {
                    true
                },
                _ => false,
            };

            if is_target {
                let definition_file =
                    find_type_definition_file(source_cache, child_file, &self_type_name);
                let check_file = definition_file.as_deref().unwrap_or(child_file);
                if type_is_exposed_outside_parent(
                    source_cache,
                    settings,
                    src_root,
                    check_file,
                    &self_type_name,
                )? {
                    return Ok(true);
                }
            }
        }
    }

    Ok(false)
}

/// When an `impl` block for a type lives in a different child module than the
/// type definition (e.g. `impl App` in `focus.rs` while `struct App` is in
/// `types.rs`), the exposure check must use the definition file — not the impl
/// file — so that `parent_facade_export_status` resolves the correct child
/// module name.
///
/// Returns `Some(path)` if the type is defined in a sibling file, `None` if it
/// is defined in `child_file` itself or cannot be located.
fn find_type_definition_file(
    source_cache: &SourceCache,
    child_file: &Path,
    type_name: &str,
) -> Option<PathBuf> {
    if file_defines_type(source_cache, child_file, type_name) {
        return None;
    }

    let parent_dir = child_file.parent()?;
    for path in source_cache.source_files_under(parent_dir) {
        if path == child_file {
            continue;
        }
        if file_defines_type(source_cache, path, type_name) {
            return Some(path.to_path_buf());
        }
    }

    None
}

fn file_defines_type(source_cache: &SourceCache, path: &Path, type_name: &str) -> bool {
    let Some(file) = source_cache.parsed_file(path) else {
        return false;
    };
    for item in &file.items {
        let name = match item {
            syn::Item::Struct(item) => &item.ident,
            syn::Item::Enum(item) => &item.ident,
            syn::Item::Type(item) => &item.ident,
            syn::Item::Union(item) => &item.ident,
            _ => continue,
        };
        if name == type_name {
            return true;
        }
    }
    false
}

fn parent_boundary_public_signature_exposes_child_used_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = parent_boundary_for_child(src_root, child_file) else {
        return Ok(false);
    };

    let Some(file) = source_cache.parsed_file(&parent_boundary.boundary_file) else {
        return Ok(false);
    };

    let mut exposing_names = Vec::new();
    for item in &file.items {
        let Some(exposing_item_name) = public_item_name(item) else {
            continue;
        };
        if public_item_surface_mentions_name(item, item_name) {
            exposing_names.push(exposing_item_name);
        }
    }

    if exposing_names.is_empty() {
        return Ok(false);
    }

    for source_file in source_cache.source_files_under(src_root) {
        if source_file == parent_boundary.boundary_file
            || source_file.starts_with(&parent_boundary.subtree_root)
        {
            continue;
        }
        let Some(current_module_path) = module_path_from_source_file(src_root, source_file) else {
            continue;
        };
        let Some(extracted) = source_cache.extracted_paths(source_file) else {
            continue;
        };
        if !matches!(
            source_references_parent_export(
                extracted,
                &current_module_path,
                &parent_boundary.module_path,
                &exposing_names,
            ),
            ParentFacadeReferenceUsage::None
        ) {
            return Ok(true);
        }
    }

    if workspace_source_mentions_parent_export_literal(
        source_cache,
        settings,
        &parent_boundary,
        &exposing_names,
    )? {
        return Ok(true);
    }

    Ok(false)
}

fn path_origin(raw: &[String]) -> PathOrigin {
    if raw.first().map(String::as_str) == Some("crate") {
        PathOrigin::Crate
    } else {
        PathOrigin::Relative
    }
}

struct PathExtractor {
    use_paths:       Vec<(Vec<String>, PathOrigin)>,
    expr_paths:      Vec<(Vec<String>, PathOrigin)>,
    use_renames:     Vec<UseRename>,
    inside_use_item: bool,
}

impl<'ast> Visit<'ast> for PathExtractor {
    fn visit_item_use(&mut self, item_use: &'ast ItemUse) {
        let mut flat = Vec::new();
        flatten_use_tree(Vec::new(), &item_use.tree, &mut flat);
        for raw in flat {
            let origin = path_origin(&raw);
            self.use_paths.push((raw, origin));
        }
        extract_use_renames(Vec::new(), &item_use.tree, &mut self.use_renames);
        self.inside_use_item = true;
        syn::visit::visit_item_use(self, item_use);
        self.inside_use_item = false;
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        if !self.inside_use_item {
            let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
            let origin = path_origin(&segments);
            self.expr_paths.push((segments, origin));
        }
        syn::visit::visit_path(self, path);
    }
}

fn extract_paths(file: &syn::File) -> ExtractedPaths {
    let mut extractor = PathExtractor {
        use_paths:       Vec::new(),
        expr_paths:      Vec::new(),
        use_renames:     Vec::new(),
        inside_use_item: false,
    };
    extractor.visit_file(file);

    ExtractedPaths {
        use_paths:   extractor.use_paths,
        expr_paths:  extractor.expr_paths,
        use_renames: extractor.use_renames,
    }
}

fn extract_use_renames(prefix: Vec<String>, tree: &UseTree, out: &mut Vec<UseRename>) {
    match tree {
        UseTree::Path(path) => {
            let mut next = prefix;
            next.push(path.ident.to_string());
            extract_use_renames(next, &path.tree, out);
        },
        UseTree::Rename(rename) => {
            let mut original_path = prefix;
            original_path.push(rename.ident.to_string());
            out.push(UseRename {
                alias: rename.rename.to_string(),
                original_path,
            });
        },
        UseTree::Group(group) => {
            for item in &group.items {
                extract_use_renames(prefix.clone(), item, out);
            }
        },
        UseTree::Name(_) | UseTree::Glob(_) => {},
    }
}

const fn merge_reference_usage(
    current: ParentFacadeReferenceUsage,
    next: ParentFacadeReferenceUsage,
) -> ParentFacadeReferenceUsage {
    match (current, next) {
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Relative), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Relative)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Relative)
        },
        (ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::DirectPath(PathOrigin::Crate)
        },
        (ParentFacadeReferenceUsage::Import(PathOrigin::Crate), _)
        | (_, ParentFacadeReferenceUsage::Import(PathOrigin::Crate)) => {
            ParentFacadeReferenceUsage::Import(PathOrigin::Crate)
        },
        _ => ParentFacadeReferenceUsage::None,
    }
}

fn public_item_name(item: &syn::Item) -> Option<String> {
    match item {
        syn::Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.sig.ident.to_string())
        },
        syn::Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        syn::Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            Some(item.ident.to_string())
        },
        _ => None,
    }
}

fn public_item_surface_mentions_name(item: &syn::Item, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    match item {
        syn::Item::Const(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        syn::Item::Enum(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for variant in &item.variants {
                match &variant.fields {
                    syn::Fields::Named(fields) => {
                        for field in &fields.named {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    syn::Fields::Unnamed(fields) => {
                        for field in &fields.unnamed {
                            visitor.visit_type(&field.ty);
                        }
                    },
                    syn::Fields::Unit => {},
                }
            }
        },
        syn::Item::Fn(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_signature(&item.sig);
        },
        syn::Item::Static(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        syn::Item::Struct(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            match &item.fields {
                syn::Fields::Named(fields) => {
                    for field in &fields.named {
                        visitor.visit_type(&field.ty);
                    }
                },
                syn::Fields::Unnamed(fields) => {
                    for field in &fields.unnamed {
                        visitor.visit_type(&field.ty);
                    }
                },
                syn::Fields::Unit => {},
            }
        },
        syn::Item::Trait(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            for trait_item in &item.items {
                match trait_item {
                    syn::TraitItem::Fn(item) => visitor.visit_signature(&item.sig),
                    syn::TraitItem::Type(item) => {
                        if let Some((_, ty)) = &item.default {
                            visitor.visit_type(ty);
                        }
                    },
                    syn::TraitItem::Const(item) => visitor.visit_type(&item.ty),
                    _ => {},
                }
            }
        },
        syn::Item::Type(item) if matches!(item.vis, syn::Visibility::Public(_)) => {
            if attributes_mention_name(&item.attrs, item_name) {
                return true;
            }
            visitor.visit_type(&item.ty);
        },
        _ => {},
    }
    visitor.found
}

fn impl_self_type_name(item_impl: &syn::ItemImpl) -> Option<String> {
    let syn::Type::Path(type_path) = item_impl.self_ty.as_ref() else {
        return None;
    };
    if type_path.qself.is_some() {
        return None;
    }
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn outward_impl_surface_mentions_name(item_impl: &syn::ItemImpl, item_name: &str) -> bool {
    let mut visitor = ItemSurfaceReferenceVisitor::new(item_name);
    let mut found_public_surface = false;
    let outward = item_impl.trait_.is_some();

    for impl_item in &item_impl.items {
        match impl_item {
            syn::ImplItem::Fn(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_signature(&item.sig);
                found_public_surface = true;
            },
            syn::ImplItem::Const(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            syn::ImplItem::Type(item)
                if outward || matches!(item.vis, syn::Visibility::Public(_)) =>
            {
                if attributes_mention_name(&item.attrs, item_name) {
                    return true;
                }
                visitor.visit_type(&item.ty);
                found_public_surface = true;
            },
            _ => {},
        }
    }

    found_public_surface && visitor.found
}

fn type_is_exposed_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    Ok(
        parent_facade_export_status(source_cache, settings, src_root, child_file, item_name)?
            .is_some_and(|status| status.usage == ParentFacadeUsage::UsedOutsideParentSubtree)
            || public_reexport_exists_outside_parent(
                source_cache,
                settings,
                src_root,
                child_file,
                item_name,
            )?
            || child_item_is_exposed_by_other_crate_visible_signature(
                source_cache,
                settings,
                src_root,
                child_file,
                item_name,
            )?
            || child_item_is_exposed_by_sibling_boundary_signature(
                source_cache,
                settings,
                src_root,
                child_file,
                item_name,
            )?
            || parent_boundary_public_signature_exposes_child_used_outside_parent(
                source_cache,
                settings,
                src_root,
                child_file,
                item_name,
            )?,
    )
}

fn public_reexport_exists_outside_parent(
    source_cache: &SourceCache,
    settings: &DriverSettings,
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<bool> {
    let Some(parent_boundary) = parent_boundary_for_child(src_root, child_file) else {
        return Ok(false);
    };
    let Some(child_module_path) = module_path_from_source_file(src_root, child_file) else {
        return Ok(false);
    };

    for source_file in source_cache.source_files_under(src_root) {
        if source_file.starts_with(&parent_boundary.subtree_root) {
            continue;
        }
        let Some(file) = source_cache.parsed_file(source_file) else {
            continue;
        };
        let Some(current_module_path) = module_path_from_source_file(src_root, source_file) else {
            continue;
        };

        for item in &file.items {
            let syn::Item::Use(item_use) = item else {
                continue;
            };
            let Some(_visibility) = parent_facade_visibility(&item_use.vis) else {
                continue;
            };
            let mut paths = Vec::new();
            flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
            for path in paths {
                for resolved in resolve_module_relative_paths(&path, &current_module_path) {
                    if resolved.len() != child_module_path.len() + 1 {
                        continue;
                    }
                    if resolved[..child_module_path.len()] == *child_module_path
                        && resolved[child_module_path.len()] == item_name
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }

    if settings.config_root != settings.package_root {
        let module_prefix = format!("crate::{}", child_module_path.join("::"));
        let findings_root = settings
            .findings_dir
            .parent()
            .map_or_else(|| settings.findings_dir.clone(), Path::to_path_buf);

        for file in source_cache.source_files_under(&settings.config_root) {
            if file.starts_with(&settings.package_root)
                || file.starts_with(&settings.findings_dir)
                || file.starts_with(&findings_root)
            {
                continue;
            }
            let source = source_cache.read_source(file)?;
            let pattern = format!("{module_prefix}::{item_name}");
            if source.contains(&pattern) {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn attributes_mention_name(attrs: &[syn::Attribute], item_name: &str) -> bool {
    attrs
        .iter()
        .any(|attr| attribute_tokens_mention_name(attr, item_name))
}

fn attribute_tokens_mention_name(attr: &syn::Attribute, item_name: &str) -> bool {
    fn token_tree_mentions_name(tree: &proc_macro2::TokenTree, item_name: &str) -> bool {
        match tree {
            proc_macro2::TokenTree::Group(group) => group
                .stream()
                .into_iter()
                .any(|tree| token_tree_mentions_name(&tree, item_name)),
            proc_macro2::TokenTree::Ident(ident) => ident == item_name,
            proc_macro2::TokenTree::Literal(literal) => {
                literal
                    .to_string()
                    .trim_matches('"')
                    .trim_matches('r')
                    .trim_matches('#')
                    == item_name
            },
            proc_macro2::TokenTree::Punct(_) => false,
        }
    }

    attr.meta
        .to_token_stream()
        .into_iter()
        .any(|tree| token_tree_mentions_name(&tree, item_name))
}

struct ItemSurfaceReferenceVisitor<'a> {
    item_name: &'a str,
    found:     bool,
}

impl<'a> ItemSurfaceReferenceVisitor<'a> {
    const fn new(item_name: &'a str) -> Self {
        Self {
            item_name,
            found: false,
        }
    }
}

impl<'ast> Visit<'ast> for ItemSurfaceReferenceVisitor<'_> {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if self.found {
            return;
        }
        if path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == self.item_name)
        {
            self.found = true;
            return;
        }
        syn::visit::visit_path(self, path);
    }
}

const fn allow_pub_crate_by_policy(
    crate_kind: CrateKind,
    module_location: ModuleLocation,
    parent_is_public: bool,
) -> bool {
    match (crate_kind, module_location) {
        (CrateKind::Library, ModuleLocation::CrateRoot) => true,
        (_, ModuleLocation::TopLevelPrivateModule) => !parent_is_public,
        _ => false,
    }
}

const fn forbidden_pub_crate_help(module_location: ModuleLocation) -> &'static str {
    if matches!(
        module_location,
        ModuleLocation::CrateRoot | ModuleLocation::TopLevelPrivateModule
    ) {
        "consider using just `pub` or removing `pub(crate)` entirely"
    } else {
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    }
}

fn suspicious_pub_note(crate_kind: CrateKind, kind_label: &str) -> String {
    match crate_kind {
        CrateKind::Library => {
            format!("{kind_label} is not reachable from the crate's public API")
        },
        CrateKind::Binary => {
            format!("{kind_label} is not used outside its parent module subtree")
        },
    }
}

#[derive(Debug)]
struct LineDisplay {
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParentFacadeExportStatus {
    usage:           ParentFacadeUsage,
    fix_supported:   bool,
    visibility:      ParentFacadeVisibility,
    parent_path:     PathBuf,
    parent_rel_path: String,
    parent_line:     usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFacadeUsage {
    Unused,
    UsedInsideParentSubtreeByRelativeImport,
    UsedInsideParentSubtreeByRelativePath,
    UsedInsideParentSubtreeByCrateImport,
    UsedInsideParentSubtreeByCratePath,
    UsedOutsideParentSubtree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ParentFacadeVisibility {
    Public,
    Super,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AllowanceReason {
    Allowlist,
    ParentIsPublic,
    TopLevelPrivateModulePolicy,
    ReachablePublicApi,
    ParentFacadeUsedOutsideParent,
    InternalParentFacadeBoundary,
    ExposedByOtherCrateVisibleSignature,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SuspiciousPubAssessment {
    Allowed(AllowanceReason),
    ReviewInternalParentFacade {
        related: Option<String>,
    },
    Warn {
        fixability:           FixSupport,
        related:              Option<String>,
        stale_parent_pub_use: Option<ParentFacadeExportStatus>,
    },
}

#[derive(Debug, Clone)]
struct ParentBoundary {
    boundary_file: PathBuf,
    subtree_root:  PathBuf,
    module_path:   Vec<String>,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ParentFacadeExports {
    explicit:      Vec<String>,
    fix_supported: bool,
    visibility:    Option<ParentFacadeVisibility>,
}

fn line_display(tcx: TyCtxt<'_>, file_path: &Path, span: Span) -> Result<LineDisplay> {
    let source_map = tcx.sess.source_map();
    let start = source_map.lookup_char_pos(span.lo());
    let end = source_map.lookup_char_pos(span.hi());
    let line = start.line;
    let column = start.col_display + 1;
    let highlight_len = if start.line == end.line {
        (end.col_display.saturating_sub(start.col_display)).max(1)
    } else {
        1
    };
    let text = fs::read_to_string(file_path)
        .with_context(|| format!("failed to read source file {}", file_path.display()))?;
    let source_line = text
        .lines()
        .nth(line.saturating_sub(1))
        .unwrap_or_default()
        .to_string();

    Ok(LineDisplay {
        line,
        column,
        highlight_len,
        source_line,
    })
}

fn visibility_text(tcx: TyCtxt<'_>, vis_span: Span) -> Result<Option<String>> {
    if vis_span.is_dummy() {
        return Ok(None);
    }
    Ok(Some(
        tcx.sess
            .source_map()
            .span_to_snippet(vis_span)
            .map_err(|err| {
                anyhow::anyhow!(
                    "failed to extract visibility snippet for span {vis_span:?}: {err:?}"
                )
            })?
            .trim()
            .to_string(),
    ))
}

fn real_file_path(tcx: TyCtxt<'_>, span: Span) -> Option<PathBuf> {
    let source_map = tcx.sess.source_map();
    let file = source_map.lookup_char_pos(span.lo()).file;
    real_file_path_from_name(file.name.clone())
}

fn real_file_path_from_name(name: FileName) -> Option<PathBuf> {
    match name {
        FileName::Real(real) => real.local_path().map(Path::to_path_buf),
        _ => None,
    }
}

fn highlight_span(vis_span: Span, ident_span: Option<Span>) -> Span {
    ident_span.map_or(vis_span, |ident_span| vis_span.to(ident_span))
}

const fn item_kind_label(kind: ItemKind<'_>) -> Option<&'static str> {
    match kind {
        ItemKind::Const(..) => Some("const"),
        ItemKind::Enum(..) => Some("enum"),
        ItemKind::Fn { .. } => Some("fn"),
        ItemKind::Static(..) => Some("static"),
        ItemKind::Struct(..) => Some("struct"),
        ItemKind::Trait(..) | ItemKind::TraitAlias(..) => Some("trait"),
        ItemKind::TyAlias(..) => Some("type"),
        ItemKind::Union(..) => Some("union"),
        ItemKind::Mod(..) => Some("mod"),
        ItemKind::Use(..)
        | ItemKind::ExternCrate(..)
        | ItemKind::ForeignMod { .. }
        | ItemKind::GlobalAsm { .. }
        | ItemKind::Impl(..)
        | ItemKind::Macro(..) => None,
    }
}

const fn impl_item_kind_label(kind: ImplItemKind<'_>) -> &'static str {
    match kind {
        ImplItemKind::Const(..) => "const",
        ImplItemKind::Fn(..) => "fn",
        ImplItemKind::Type(..) => "type",
    }
}

const fn foreign_item_kind_label(kind: ForeignItemKind<'_>) -> &'static str {
    match kind {
        ForeignItemKind::Fn(..) => "fn",
        ForeignItemKind::Static(..) => "static",
        ForeignItemKind::Type => "type",
    }
}

/// Extract the self type name for an impl item via the compiler.
///
/// Given an impl item's `LocalDefId`, walks up to the parent impl block
/// and returns the last path segment of the self type (e.g., `"MyStruct"`).
fn impl_self_type_name_from_tcx(tcx: TyCtxt<'_>, impl_item_def: LocalDefId) -> Option<String> {
    let hir_id = tcx.local_def_id_to_hir_id(impl_item_def);
    let parent_id = tcx.hir_get_parent_item(hir_id);
    let parent_node = tcx.hir_node_by_def_id(parent_id.def_id);
    let rustc_hir::Node::Item(parent_item) = parent_node else {
        return None;
    };
    let ItemKind::Impl(impl_block) = parent_item.kind else {
        return None;
    };
    let rustc_hir::TyKind::Path(rustc_hir::QPath::Resolved(_, path)) = impl_block.self_ty.kind
    else {
        return None;
    };
    path.segments.last().map(|seg| seg.ident.to_string())
}

/// True when `file` is part of a top-level module — either `src/foo.rs` or
/// `src/foo/mod.rs` — but NOT the root module itself (lib.rs / main.rs).
fn is_top_level_module_file(src_root: &Path, root_module: &Path, file: &Path) -> bool {
    if file == root_module {
        return false;
    }
    let Ok(relative) = file.strip_prefix(src_root) else {
        return false;
    };
    let count = relative.components().count();
    // src/foo.rs → 1 component
    if count == 1 {
        return true;
    }
    // src/foo/mod.rs → 2 components, last is "mod.rs"
    count == 2 && relative.file_name().and_then(|name| name.to_str()) == Some("mod.rs")
}

fn is_boundary_file(src_root: &Path, root_module: &Path, file: &Path) -> bool {
    let is_root_file = file == root_module;
    let is_mod_rs = file.file_name().and_then(|name| name.to_str()) == Some("mod.rs");
    let is_top_level_file = file
        .strip_prefix(src_root)
        .ok()
        .is_some_and(|path| path.components().count() == 1);
    is_root_file || is_mod_rs || is_top_level_file
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::path::PathBuf;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use super::BuildOutputMode;
    use super::CrateKind;
    use super::DiagnosticBlockKind;
    use super::DriverSettings;
    use super::ModuleLocation;
    use super::ParentFacadeExports;
    use super::ParentFacadeVisibility;
    use super::allow_pub_crate_by_policy;
    use super::analysis_source_root_for;
    use super::classify_diagnostic_block;
    use super::config_relative_path;
    use super::config_relative_path_for_settings;
    use super::current_analysis_fingerprint;
    use super::exported_names_from_parent_boundary;
    use super::flush_diagnostic_block;
    use super::forbidden_pub_crate_help;
    use super::is_progress_line;
    use super::module_path_from_source_file;
    use super::suspicious_pub_note;
    use crate::config::VisibilityConfig;
    use crate::diagnostics::CompilerWarningFacts;

    #[test]
    fn allow_pub_crate_allows_library_crate_root_items() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::CrateRoot,
            true
        ));
    }

    #[test]
    fn allow_pub_crate_allows_top_level_private_library_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::TopLevelPrivateModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Library,
            ModuleLocation::NestedModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_crate_root_items() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::CrateRoot,
            true
        ));
    }

    #[test]
    fn allow_pub_crate_allows_top_level_private_binary_modules() {
        assert!(allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::TopLevelPrivateModule,
            false
        ));
    }

    #[test]
    fn allow_pub_crate_rejects_binary_nested_modules() {
        assert!(!allow_pub_crate_by_policy(
            CrateKind::Binary,
            ModuleLocation::NestedModule,
            false
        ));
    }

    #[test]
    fn forbidden_pub_crate_help_handles_crate_root_items() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::CrateRoot),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_top_level_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::TopLevelPrivateModule),
            "consider using just `pub` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn forbidden_pub_crate_help_handles_nested_private_modules() {
        assert_eq!(
            forbidden_pub_crate_help(ModuleLocation::NestedModule),
            "consider using `pub(super)` or removing `pub(crate)` entirely"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_public_api_wording_for_libraries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Library, "struct"),
            "struct is not reachable from the crate's public API"
        );
    }

    #[test]
    fn suspicious_pub_note_uses_subtree_wording_for_binaries() {
        assert_eq!(
            suspicious_pub_note(CrateKind::Binary, "function"),
            "function is not used outside its parent module subtree"
        );
    }

    #[test]
    fn config_relative_path_handles_nested_workspace_paths() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let workspace_root = std::env::temp_dir().join(format!("mend-config-root-test-{unique}"));
        let file_path = workspace_root.join("mcp/src/brp_tools/tools/mod.rs");
        let parent = file_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("test path must have a parent directory"))?;
        fs::create_dir_all(parent)?;
        fs::write(&file_path, "pub mod world_query;\n")?;

        assert_eq!(
            config_relative_path(&file_path, &workspace_root).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }

    #[test]
    fn config_relative_path_for_settings_handles_package_relative_workspace_paths() {
        let settings = DriverSettings {
            config_root:          PathBuf::from("/workspace/root"),
            config:               VisibilityConfig::default(),
            config_fingerprint:   "test".to_string(),
            scope_fingerprint:    "scope".to_string(),
            findings_dir:         PathBuf::from("/workspace/root/target/mend-findings"),
            package_root:         PathBuf::from("/workspace/root/mcp"),
            analysis_fingerprint: current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("src/brp_tools/tools/mod.rs");

        assert_eq!(
            config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );
    }

    #[test]
    fn config_relative_path_for_settings_handles_workspace_relative_paths() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let config_root = temp.path().join("workspace");
        let package_root = config_root.join("mcp");
        std::fs::create_dir_all(package_root.join("src/brp_tools/tools"))?;
        std::fs::write(
            package_root.join("src/brp_tools/tools/mod.rs"),
            "pub mod child;\n",
        )?;
        let settings = DriverSettings {
            config_root,
            config: VisibilityConfig::default(),
            config_fingerprint: "test".to_string(),
            scope_fingerprint: "scope".to_string(),
            findings_dir: temp.path().join("workspace/target/mend-findings"),
            package_root,
            analysis_fingerprint: current_analysis_fingerprint(),
        };
        let file_path = PathBuf::from("mcp/src/brp_tools/tools/mod.rs");

        assert_eq!(
            config_relative_path_for_settings(&file_path, &settings).as_deref(),
            Some("mcp/src/brp_tools/tools/mod.rs")
        );

        Ok(())
    }

    #[test]
    fn analysis_source_root_ignores_build_scripts() {
        let package_root = Path::new("/tmp/example-crate");

        assert_eq!(
            analysis_source_root_for(&package_root.join("src/lib.rs"), package_root),
            Some(package_root.join("src"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("src/bin/demo.rs"), package_root),
            Some(package_root.join("src/bin"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("examples/demo.rs"), package_root),
            Some(package_root.join("examples"))
        );
        assert_eq!(
            analysis_source_root_for(&package_root.join("build.rs"), package_root),
            None
        );
    }

    #[test]
    fn grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use report_writer::{ReportDefinition, ReportWriter};\n";
        let file = syn::parse_file(source).unwrap();
        let exports =
            exported_names_from_parent_boundary(&file, "report_writer", "ReportDefinition");
        assert_eq!(exports.explicit, vec!["ReportDefinition".to_string()]);
        assert!(exports.fix_supported);
    }

    #[test]
    fn multiline_grouped_parent_pub_use_is_fix_supported() {
        let source = "pub use child::{\n    Thing,\n    Other,\n};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(exports.explicit, vec!["Thing".to_string()]);
        assert!(exports.fix_supported);
    }

    #[test]
    fn module_path_from_source_file_treats_main_rs_as_crate_root() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("mend-main-root-test-{unique}"));
        let src_dir = temp_dir.join("src");
        fs::create_dir_all(&src_dir)?;
        let main_rs = src_dir.join("main.rs");
        fs::write(&main_rs, "fn main() {}\n")?;

        assert_eq!(
            module_path_from_source_file(&src_dir, &main_rs),
            Some(Vec::new())
        );

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }

    #[test]
    fn grouped_parent_pub_use_with_rename_is_manual_only() {
        let source = "pub use child::{Thing as RenamedThing, Other};\n";
        let file = syn::parse_file(source).unwrap();
        let exports = exported_names_from_parent_boundary(&file, "child", "Thing");
        assert_eq!(
            exports,
            ParentFacadeExports {
                explicit:      vec!["RenamedThing".to_string()],
                fix_supported: false,
                visibility:    Some(ParentFacadeVisibility::Public),
            }
        );

        let exports = exported_names_from_parent_boundary(&file, "child", "Other");
        assert_eq!(exports.explicit, vec!["Other".to_string()]);
        assert!(exports.fix_supported);
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
        let mut printed_suppression_notice = false;
        let mut compiler_warnings = CompilerWarningFacts::None;
        let mut compiler_warning_count = 0;
        let mut compiler_fixable_count = 0;

        flush_diagnostic_block(
            &mut block,
            &mut printed_suppression_notice,
            &mut compiler_warnings,
            &mut compiler_warning_count,
            &mut compiler_fixable_count,
            BuildOutputMode::Quiet,
        );

        assert_eq!(compiler_warning_count, 0);
        assert_eq!(compiler_fixable_count, 0);
    }
}
