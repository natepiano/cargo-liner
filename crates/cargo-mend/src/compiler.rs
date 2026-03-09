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

use anyhow::Context;
use anyhow::Result;
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

use super::config::LoadedConfig;
use super::config::VisibilityConfig;
use super::diagnostics::Finding;
use super::diagnostics::FixKind;
use super::diagnostics::Report;
use super::diagnostics::ReportSummary;
use super::diagnostics::Severity;
use super::selection::Selection;

const DRIVER_ENV: &str = "MEND_DRIVER";
const CONFIG_ROOT_ENV: &str = "MEND_CONFIG_ROOT";
const CONFIG_JSON_ENV: &str = "MEND_CONFIG_JSON";
const FINDINGS_DIR_ENV: &str = "MEND_FINDINGS_DIR";
const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
const FINDINGS_SCHEMA_VERSION: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildOutputMode {
    Full,
    SuppressUnusedImportWarnings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticBlockKind {
    SuppressedUnusedImport,
    ForwardedDiagnostic,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredReport {
    version:      u32,
    package_root: String,
    findings:     Vec<StoredFinding>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredFinding {
    severity:      Severity,
    code:          String,
    path:          String,
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
    item:          Option<String>,
    message:       String,
    suggestion:    Option<String>,
    #[serde(default)]
    fix_kind:      Option<FixKind>,
    #[serde(default)]
    related:       Option<String>,
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
    config_root:  PathBuf,
    config:       VisibilityConfig,
    findings_dir: PathBuf,
    package_root: PathBuf,
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
        let findings_dir = PathBuf::from(
            env::var_os(FINDINGS_DIR_ENV)
                .context("missing MEND_FINDINGS_DIR for compiler driver")?,
        );
        let package_root = PathBuf::from(
            env::var_os(PACKAGE_ROOT_ENV)
                .context("missing CARGO_MANIFEST_DIR for compiler driver")?,
        );

        Ok(Self {
            config_root,
            config,
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

pub fn run_selection(
    selection: &Selection,
    loaded_config: &LoadedConfig,
    output_mode: BuildOutputMode,
) -> Result<Report> {
    let findings_dir = selection.target_directory.join("mend-findings");
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create persistent findings directory {}",
            findings_dir.display()
        )
    })?;

    let status = run_cargo_check(selection, loaded_config, &findings_dir, output_mode)?;

    if !status.success() {
        anyhow::bail!("cargo check failed during mend analysis");
    }

    let missing_packages = selection
        .packages
        .iter()
        .filter(|package| !cache_is_current_for(&findings_dir, &package.root))
        .collect::<Vec<_>>();

    if !missing_packages.is_empty() {
        for package in missing_packages {
            let status =
                run_cargo_rustc_for_package(package, loaded_config, &findings_dir, output_mode)?;
            if !status.success() {
                anyhow::bail!(
                    "cargo rustc refresh failed during mend analysis for package {}",
                    package.name
                );
            }
        }
    }

    let report = load_report(&findings_dir, selection)?;

    Ok(report)
}

pub fn driver_main() -> ExitCode {
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
    });

    let exit_code = callbacks.error.map_or(compiler_exit_code, |err| {
        eprintln!("mend: {err:#}");
        1
    });

    Ok(exit_code_from_i32(exit_code))
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

fn run_cargo_check(
    selection: &Selection,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
    output_mode: BuildOutputMode,
) -> Result<std::process::ExitStatus> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new("cargo");
    command.arg("check");

    if selection.workspace_selected {
        command.arg("--workspace");
    } else {
        command
            .arg("--manifest-path")
            .arg(selection.manifest_path.as_os_str());
    }

    command
        .env("RUSTC_WORKSPACE_WRAPPER", &current_exe)
        .env(DRIVER_ENV, "1")
        .env(CONFIG_ROOT_ENV, &loaded_config.root)
        .env(
            CONFIG_JSON_ENV,
            serde_json::to_string(&loaded_config.config)
                .context("failed to serialize mend config for compiler driver")?,
        )
        .env(FINDINGS_DIR_ENV, findings_dir)
        .stdin(Stdio::inherit());

    run_cargo_command(&mut command, output_mode).context("failed to run cargo check for mend")
}

fn run_cargo_rustc_for_package(
    package: &super::selection::SelectedPackage,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
    output_mode: BuildOutputMode,
) -> Result<std::process::ExitStatus> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new("cargo");
    command.arg("rustc");
    command
        .arg("--manifest-path")
        .arg(package.manifest_path.as_os_str());

    for arg in package.target.cargo_args() {
        command.arg(arg);
    }
    for arg in refresh_rustc_args() {
        command.arg(arg);
    }

    command
        .env("RUSTC_WORKSPACE_WRAPPER", &current_exe)
        .env(DRIVER_ENV, "1")
        .env(CONFIG_ROOT_ENV, &loaded_config.root)
        .env(
            CONFIG_JSON_ENV,
            serde_json::to_string(&loaded_config.config)
                .context("failed to serialize mend config for compiler driver")?,
        )
        .env(FINDINGS_DIR_ENV, findings_dir)
        .stdin(Stdio::inherit());

    run_cargo_command(&mut command, output_mode).with_context(|| {
        format!(
            "failed to run cargo rustc refresh for package {}",
            package.name
        )
    })
}

fn run_cargo_command(
    command: &mut Command,
    output_mode: BuildOutputMode,
) -> Result<std::process::ExitStatus> {
    command.stdin(Stdio::inherit());
    match output_mode {
        BuildOutputMode::Full => {
            command.stdout(Stdio::inherit()).stderr(Stdio::inherit());
            command.status().context("failed to run cargo command")
        },
        BuildOutputMode::SuppressUnusedImportWarnings => {
            command.stdout(Stdio::null()).stderr(Stdio::piped());
            let mut child = command.spawn().context("failed to spawn cargo command")?;
            let stderr = child
                .stderr
                .take()
                .context("failed to capture cargo stderr")?;
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            let mut block = Vec::new();
            let mut printed_suppression_notice = false;

            loop {
                line.clear();
                let bytes = reader.read_line(&mut line)?;
                if bytes == 0 {
                    flush_diagnostic_block(&mut block, &mut printed_suppression_notice);
                    break;
                }

                let current = line.clone();
                if is_progress_line(&current) {
                    flush_diagnostic_block(&mut block, &mut printed_suppression_notice);
                    eprint!("{current}");
                    continue;
                }

                if current.trim().is_empty() {
                    block.push(current);
                    flush_diagnostic_block(&mut block, &mut printed_suppression_notice);
                } else {
                    block.push(current);
                }
            }

            child.wait().context("failed to wait for cargo command")
        },
    }
}

fn is_progress_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("Blocking waiting for file lock")
        || trimmed.starts_with("Checking ")
        || trimmed.starts_with("Compiling ")
        || trimmed.starts_with("Finished ")
        || trimmed.starts_with("Fresh ")
}

fn classify_diagnostic_block(
    block: &[String],
    printed_suppression_notice: bool,
) -> DiagnosticBlockKind {
    let first_non_empty = block.iter().find(|line| !line.trim().is_empty());
    first_non_empty.map_or(DiagnosticBlockKind::ForwardedDiagnostic, |line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("warning: unused import:")
            || (printed_suppression_notice
                && trimmed.starts_with("warning: `")
                && (trimmed.contains(" generated 1 warning ")
                    || trimmed.contains("to apply 1 suggestion")))
        {
            DiagnosticBlockKind::SuppressedUnusedImport
        } else {
            DiagnosticBlockKind::ForwardedDiagnostic
        }
    })
}

fn flush_diagnostic_block(block: &mut Vec<String>, printed_suppression_notice: &mut bool) {
    if block.is_empty() {
        return;
    }

    match classify_diagnostic_block(block, *printed_suppression_notice) {
        DiagnosticBlockKind::SuppressedUnusedImport => {
            if !*printed_suppression_notice {
                eprintln!(
                    "mend: suppressing `unused import` warning during `--fix-pub-use` discovery"
                );
                *printed_suppression_notice = true;
            }
        },
        DiagnosticBlockKind::ForwardedDiagnostic => {
            for line in block.iter() {
                eprint!("{line}");
            }
        },
    }

    block.clear();
}

fn refresh_rustc_args() -> Vec<String> {
    vec![
        "--".to_string(),
        format!("--cfg=mend_refresh_{}", std::process::id()),
    ]
}

fn cache_is_current_for(findings_dir: &Path, package_root: &Path) -> bool {
    let cache_path = findings_dir.join(cache_filename_for(package_root));
    let Ok(text) = fs::read_to_string(&cache_path) else {
        return false;
    };
    let Ok(cache_metadata) = fs::metadata(&cache_path) else {
        return false;
    };
    let Ok(cache_modified) = cache_metadata.modified() else {
        return false;
    };
    let Ok(stored) = serde_json::from_str::<StoredReport>(&text) else {
        return false;
    };
    stored.version == FINDINGS_SCHEMA_VERSION
        && stored.package_root == package_root.to_string_lossy()
        && !package_sources_newer_than(package_root, cache_modified)
}

fn package_sources_newer_than(package_root: &Path, reference: std::time::SystemTime) -> bool {
    let manifest = package_root.join("Cargo.toml");
    if file_is_newer_than(&manifest, reference) {
        return true;
    }

    let src = package_root.join("src");
    rust_sources_newer_than(&src, reference)
}

fn file_is_newer_than(path: &Path, reference: std::time::SystemTime) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified > reference)
}

fn rust_sources_newer_than(dir: &Path, reference: std::time::SystemTime) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if rust_sources_newer_than(&path, reference) {
                return true;
            }
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs")
            && file_is_newer_than(&path, reference)
        {
            return true;
        }
    }

    false
}

fn load_report(findings_dir: &Path, selection: &Selection) -> Result<Report> {
    let mut findings = Vec::new();
    let selected_roots: Vec<String> = selection
        .package_roots
        .iter()
        .map(|root| root.to_string_lossy().into_owned())
        .collect();
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
        if stored.version != FINDINGS_SCHEMA_VERSION {
            continue;
        }
        let matches_selected_root = selected_roots
            .iter()
            .any(|root| root == &stored.package_root)
            || (stored.package_root.is_empty() && selected_roots.len() == 1);
        if !matches_selected_root {
            continue;
        }
        for finding in stored.findings {
            findings.push(Finding {
                severity:      finding.severity,
                code:          finding.code,
                path:          relativize_path(&finding.path, selection.analysis_root.as_path()),
                line:          finding.line,
                column:        finding.column,
                highlight_len: finding.highlight_len,
                source_line:   finding.source_line,
                item:          finding.item,
                message:       finding.message,
                suggestion:    finding.suggestion,
                fix_kind:      finding.fix_kind,
                related:       finding.related,
            });
        }
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
    })
}

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    absolute.strip_prefix(analysis_root).map_or_else(
        |_| path.to_string(),
        |relative| relative.to_string_lossy().replace('\\', "/"),
    )
}

fn exit_code_from_i32(code: i32) -> ExitCode {
    let normalized_code = u8::try_from(code).unwrap_or(1);
    ExitCode::from(normalized_code)
}

fn collect_and_store_findings(tcx: TyCtxt<'_>, settings: &DriverSettings) -> Result<bool> {
    let crate_root_file = real_file_path(tcx, tcx.def_span(CRATE_DEF_ID))
        .context("failed to determine local crate root file")?;
    let Some(src_root) = crate_root_file
        .parent()
        .filter(|parent| parent.file_name().and_then(|name| name.to_str()) == Some("src"))
    else {
        return Ok(false);
    };
    let src_root = src_root.to_path_buf();

    let mut findings = Vec::new();
    let crate_items = tcx.hir_crate_items(());
    let effective_visibilities = tcx.effective_visibilities(());

    for item_id in crate_items.free_items() {
        let item = tcx.hir_item(item_id);
        analyze_item(
            tcx,
            settings,
            &src_root,
            &crate_root_file,
            effective_visibilities,
            item,
            &mut findings,
        )?;
    }

    for item_id in crate_items.impl_items() {
        let item = tcx.hir_impl_item(item_id);
        analyze_impl_item(
            tcx,
            settings,
            &src_root,
            &crate_root_file,
            effective_visibilities,
            item,
            &mut findings,
        )?;
    }

    for item_id in crate_items.foreign_items() {
        let item = tcx.hir_foreign_item(item_id);
        analyze_foreign_item(
            tcx,
            settings,
            &src_root,
            &crate_root_file,
            effective_visibilities,
            item,
            &mut findings,
        )?;
    }

    let output_path = settings
        .findings_dir
        .join(cache_filename_for(&settings.package_root));
    if !findings.is_empty() {
        findings.sort_by(|a, b| {
            (&a.path, a.line, a.column, &a.code, &a.item, &a.message)
                .cmp(&(&b.path, b.line, b.column, &b.code, &b.item, &b.message))
        });
        findings.dedup_by(|a, b| {
            a.code == b.code
                && a.path == b.path
                && a.line == b.line
                && a.column == b.column
                && a.message == b.message
                && a.item == b.item
        });
    }

    let report = StoredReport {
        version: FINDINGS_SCHEMA_VERSION,
        package_root: settings.package_root.to_string_lossy().into_owned(),
        findings,
    };
    fs::write(&output_path, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("failed to write findings file {}", output_path.display()))?;
    Ok(true)
}

fn cache_filename_for(package_root: &Path) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    package_root.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
}

fn analyze_item(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
    src_root: &Path,
    root_module: &Path,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    item: &Item<'_>,
    findings: &mut Vec<StoredFinding>,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.kind.ident().map(|ident| ident.to_string());

    record_visibility_findings(
        tcx,
        settings,
        src_root,
        root_module,
        effective_visibilities,
        item.owner_id.def_id,
        &file_path,
        &vis_text,
        item_kind_label(item.kind),
        item_name.as_deref(),
        highlight_span(item.vis_span, item.kind.ident().map(|ident| ident.span)),
        matches!(item.kind, ItemKind::Mod(..)),
        findings,
    )
}

fn analyze_impl_item(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
    src_root: &Path,
    root_module: &Path,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    item: &ImplItem<'_>,
    findings: &mut Vec<StoredFinding>,
) -> Result<()> {
    let Some(vis_span) = item.vis_span() else {
        return Ok(());
    };
    if item.span.from_expansion() || vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(tcx, vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(tcx, vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    record_visibility_findings(
        tcx,
        settings,
        src_root,
        root_module,
        effective_visibilities,
        item.owner_id.def_id,
        &file_path,
        &vis_text,
        Some(impl_item_kind_label(item.kind)),
        Some(item_name.as_str()),
        highlight_span(vis_span, Some(item.ident.span)),
        false,
        findings,
    )
}

fn analyze_foreign_item(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
    src_root: &Path,
    root_module: &Path,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    item: &ForeignItem<'_>,
    findings: &mut Vec<StoredFinding>,
) -> Result<()> {
    if item.span.from_expansion() || item.vis_span.from_expansion() {
        return Ok(());
    }
    let Some(file_path) = real_file_path(tcx, item.vis_span) else {
        return Ok(());
    };
    let Some(vis_text) = visibility_text(tcx, item.vis_span)? else {
        return Ok(());
    };

    let item_name = item.ident.to_string();

    record_visibility_findings(
        tcx,
        settings,
        src_root,
        root_module,
        effective_visibilities,
        item.owner_id.def_id,
        &file_path,
        &vis_text,
        Some(foreign_item_kind_label(item.kind)),
        Some(item_name.as_str()),
        highlight_span(item.vis_span, Some(item.ident.span)),
        false,
        findings,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_visibility_findings(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
    src_root: &Path,
    root_module: &Path,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    def_id: LocalDefId,
    file_path: &Path,
    vis_text: &str,
    kind_label: Option<&'static str>,
    item_name: Option<&str>,
    highlight_span: Span,
    is_module_item: bool,
    findings: &mut Vec<StoredFinding>,
) -> Result<()> {
    let crate_kind = if root_module.file_name().and_then(|name| name.to_str()) == Some("lib.rs") {
        CrateKind::Library
    } else {
        CrateKind::Binary
    };
    let config_rel_path = file_path
        .strip_prefix(&settings.config_root)
        .ok()
        .map(|path| path.to_string_lossy().replace('\\', "/"));
    let parent_module = tcx.parent_module_from_def_id(def_id);
    let parent_is_public = tcx
        .local_visibility(parent_module.to_local_def_id())
        .is_public();
    let parent_is_crate_root = parent_module.to_local_def_id() == CRATE_DEF_ID;
    let grandparent_is_crate_root = !parent_is_crate_root
        && tcx
            .parent_module_from_def_id(parent_module.to_local_def_id())
            .to_local_def_id()
            == CRATE_DEF_ID;
    let module_location = module_location(parent_is_crate_root, grandparent_is_crate_root);

    if matches!(vis_text, "pub(crate)")
        && !allow_pub_crate_by_policy(crate_kind, module_location, parent_is_public)
    {
        findings.push(build_finding(
            tcx,
            file_path,
            highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       "forbidden_pub_crate",
                item:       None,
                message:    "use of `pub(crate)` is forbidden by policy".to_string(),
                suggestion: Some(forbidden_pub_crate_help(module_location).to_string()),
                fix_kind:   None,
                related:    None,
            },
        )?);
    }

    if vis_text.starts_with("pub(in crate::") {
        findings.push(build_finding(
            tcx,
            file_path,
            highlight_span,
            FindingParams {
                severity:   Severity::Error,
                code:       "forbidden_pub_in_crate",
                item:       None,
                message:    "use of `pub(in crate::...)` is forbidden by policy".to_string(),
                suggestion: None,
                fix_kind:   None,
                related:    None,
            },
        )?);
    }

    if is_module_item && vis_text.starts_with("pub") {
        let allowlisted = config_rel_path.as_ref().is_some_and(|path| {
            settings
                .config
                .allow_pub_mod
                .iter()
                .any(|allowed| allowed == path)
        });
        if !allowlisted {
            findings.push(build_finding(
                tcx,
                file_path,
                highlight_span,
                FindingParams {
                    severity:   Severity::Error,
                    code:       "review_pub_mod",
                    item:       item_name.map(str::to_owned),
                    message:    "`pub mod` requires explicit review or allowlisting".to_string(),
                    suggestion: None,
                    fix_kind:   None,
                    related:    None,
                },
            )?);
        }
    }

    if vis_text == "pub" && !is_boundary_file(src_root, root_module, file_path) {
        maybe_record_suspicious_pub(
            tcx,
            settings,
            src_root,
            effective_visibilities,
            def_id,
            file_path,
            config_rel_path.as_deref(),
            parent_is_public,
            module_location,
            kind_label,
            item_name,
            highlight_span,
            findings,
            crate_kind,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn maybe_record_suspicious_pub(
    tcx: TyCtxt<'_>,
    settings: &DriverSettings,
    src_root: &Path,
    effective_visibilities: &rustc_middle::middle::privacy::EffectiveVisibilities,
    def_id: LocalDefId,
    file_path: &Path,
    config_rel_path: Option<&str>,
    parent_is_public: bool,
    module_location: ModuleLocation,
    kind_label: Option<&'static str>,
    item_name: Option<&str>,
    highlight_span: Span,
    findings: &mut Vec<StoredFinding>,
    crate_kind: CrateKind,
) -> Result<()> {
    let Some(kind_label) = kind_label else {
        return Ok(());
    };

    let item_key = config_rel_path.and_then(|path| item_name.map(|name| format!("{path}::{name}")));
    let allowlisted = item_key.as_ref().is_some_and(|key| {
        settings
            .config
            .allow_pub_items
            .iter()
            .any(|allowed| allowed == key)
    });
    let reachable = effective_visibilities.is_public_at_level(def_id, Level::Reachable);
    let parent_facade_export = item_name
        .map(|name| parent_facade_export_status(src_root, file_path, name))
        .transpose()?
        .flatten();
    let parent_facade_export_used_outside_parent = parent_facade_export
        .as_ref()
        .is_some_and(|status| status.used_outside_parent);

    if allowlisted
        || parent_is_public
        || matches!(module_location, ModuleLocation::TopLevelPrivateModule)
        || reachable
        || parent_facade_export_used_outside_parent
    {
        return Ok(());
    }

    let stale_parent_pub_use = parent_facade_export
        .as_ref()
        .filter(|status| !status.used_outside_parent);
    let related = stale_parent_pub_use.map(|status| {
        format!(
            "parent module also has an `unused import` warning for this `pub use` at {}:{}",
            status.parent_rel_path, status.parent_line
        )
    });
    findings.push(build_finding(
        tcx,
        file_path,
        highlight_span,
        FindingParams {
            severity: Severity::Warning,
            code: "suspicious_pub",
            item: item_name.map(|name| format!("{kind_label} {name}")),
            message: suspicious_pub_note(crate_kind, kind_label),
            suggestion: None,
            fix_kind: stale_parent_pub_use.map(|_| FixKind::ParentPubUse),
            related,
        },
    )?);
    Ok(())
}

struct FindingParams {
    severity:   Severity,
    code:       &'static str,
    item:       Option<String>,
    message:    String,
    suggestion: Option<String>,
    fix_kind:   Option<FixKind>,
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
        code:          params.code.to_string(),
        path:          file_path.to_string_lossy().into_owned(),
        line:          display.line,
        column:        display.column,
        highlight_len: display.highlight_len,
        source_line:   display.source_line,
        item:          params.item,
        message:       params.message,
        suggestion:    params.suggestion,
        fix_kind:      params.fix_kind,
        related:       params.related,
    })
}

const fn module_location(
    parent_is_crate_root: bool,
    grandparent_is_crate_root: bool,
) -> ModuleLocation {
    if parent_is_crate_root {
        ModuleLocation::CrateRoot
    } else if grandparent_is_crate_root {
        ModuleLocation::TopLevelPrivateModule
    } else {
        ModuleLocation::NestedModule
    }
}

fn parent_facade_export_status(
    src_root: &Path,
    child_file: &Path,
    item_name: &str,
) -> Result<Option<ParentFacadeExportStatus>> {
    let Some(parent_dir) = child_file.parent() else {
        return Ok(None);
    };
    let parent_mod_rs = parent_dir.join("mod.rs");
    if !parent_mod_rs.is_file() {
        return Ok(None);
    }

    let child_module_name = child_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| *stem != "mod")
        .context("child file for facade check must not be mod.rs")?;

    let parent_source = fs::read_to_string(&parent_mod_rs).with_context(|| {
        format!(
            "failed to read parent module file {}",
            parent_mod_rs.display()
        )
    })?;
    let exported_names =
        exported_names_from_parent_mod(&parent_source, child_module_name, item_name)?;
    if exported_names.is_empty() {
        return Ok(None);
    }

    let Some(module_path) = module_path_from_dir(src_root, parent_dir) else {
        return Ok(None);
    };
    let parent_rel_path = parent_mod_rs
        .strip_prefix(src_root)
        .unwrap_or(&parent_mod_rs)
        .to_string_lossy()
        .replace('\\', "/");
    let parent_line = first_line_matching(&parent_source, item_name).unwrap_or(1);

    for file in rust_source_files(src_root)? {
        if file.starts_with(parent_dir) {
            continue;
        }
        let source = fs::read_to_string(&file)
            .with_context(|| format!("failed to read source file {}", file.display()))?;
        if use_tree_references_parent_export(&source, &module_path, &exported_names) {
            return Ok(Some(ParentFacadeExportStatus {
                used_outside_parent: true,
                parent_rel_path,
                parent_line,
            }));
        }
    }

    Ok(Some(ParentFacadeExportStatus {
        used_outside_parent: false,
        parent_rel_path,
        parent_line,
    }))
}

fn exported_names_from_parent_mod(
    parent_source: &str,
    child_module_name: &str,
    item_name: &str,
) -> Result<Vec<String>> {
    let file = syn::parse_file(parent_source).context("failed to parse parent mod.rs")?;
    let mut exported = Vec::new();
    for item in file.items {
        let syn::Item::Use(item_use) = item else {
            continue;
        };
        if !matches!(item_use.vis, syn::Visibility::Public(_)) {
            continue;
        }
        collect_matching_pub_use_exports(&item_use, child_module_name, item_name, &mut exported);
    }
    exported.sort();
    exported.dedup();
    Ok(exported)
}

fn collect_matching_pub_use_exports(
    item_use: &ItemUse,
    child_module_name: &str,
    item_name: &str,
    exported: &mut Vec<String>,
) {
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
            && normalized[1] == item_name
            && let Some(export_name) = normalized.last()
        {
            exported.push(export_name.clone());
        }
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
        UseTree::Glob(_) => {},
    }
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

fn use_tree_references_parent_export(
    source: &str,
    module_path: &[String],
    exported_names: &[String],
) -> bool {
    let Ok(file) = syn::parse_file(source) else {
        return false;
    };
    for item in file.items {
        let syn::Item::Use(item_use) = item else {
            continue;
        };
        let mut paths = Vec::new();
        flatten_use_tree(Vec::new(), &item_use.tree, &mut paths);
        for path in paths {
            let normalized = match path.first().map(String::as_str) {
                Some("crate" | "self" | "super") => &path[1..],
                _ => &path[..],
            };
            if normalized.len() != module_path.len() + 1 {
                continue;
            }
            if normalized[..module_path.len()] == *module_path
                && exported_names
                    .iter()
                    .any(|name| name == &normalized[module_path.len()])
            {
                return true;
            }
        }
    }
    false
}

const fn allow_pub_crate_by_policy(
    crate_kind: CrateKind,
    module_location: ModuleLocation,
    parent_is_public: bool,
) -> bool {
    match (crate_kind, module_location) {
        (CrateKind::Library, ModuleLocation::CrateRoot) => true,
        (CrateKind::Library, ModuleLocation::TopLevelPrivateModule) => !parent_is_public,
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

#[derive(Debug, Clone)]
struct ParentFacadeExportStatus {
    used_outside_parent: bool,
    parent_rel_path:     String,
    parent_line:         usize,
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
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::SystemTime;
    use std::time::UNIX_EPOCH;

    use super::CrateKind;
    use super::FINDINGS_SCHEMA_VERSION;
    use super::ModuleLocation;
    use super::Severity;
    use super::StoredFinding;
    use super::StoredReport;
    use super::allow_pub_crate_by_policy;
    use super::cache_filename_for;
    use super::cache_is_current_for;
    use super::forbidden_pub_crate_help;
    use super::module_location;
    use super::refresh_rustc_args;
    use super::suspicious_pub_note;

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
    fn module_location_handles_crate_root() {
        assert_eq!(module_location(true, false), ModuleLocation::CrateRoot);
    }

    #[test]
    fn module_location_handles_top_level_private_module() {
        assert_eq!(
            module_location(false, true),
            ModuleLocation::TopLevelPrivateModule
        );
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
    fn refresh_rustc_args_adds_mend_cfg() {
        let args = refresh_rustc_args();
        assert_eq!(args.first().map(String::as_str), Some("--"));
        assert!(
            args.get(1)
                .is_some_and(|arg| arg.starts_with("--cfg=mend_refresh_"))
        );
    }

    #[test]
    fn cache_is_current_requires_matching_schema_version() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("mend-cache-test-{unique}"));
        fs::create_dir_all(&temp_dir)?;

        let package_root = Path::new("/tmp/example-crate");
        let cache_path = temp_dir.join(cache_filename_for(package_root));
        let stale = StoredReport {
            version:      FINDINGS_SCHEMA_VERSION - 1,
            package_root: package_root.to_string_lossy().into_owned(),
            findings:     vec![StoredFinding {
                severity:      Severity::Warning,
                code:          "suspicious_pub".to_string(),
                path:          "src/lib.rs".to_string(),
                line:          1,
                column:        1,
                highlight_len: 3,
                source_line:   "pub fn x() {}".to_string(),
                item:          None,
                message:       String::new(),
                suggestion:    None,
                fix_kind:      None,
                related:       None,
            }],
        };
        fs::write(&cache_path, serde_json::to_vec(&stale)?)?;

        assert!(!cache_is_current_for(&temp_dir, package_root));

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }

    #[test]
    fn cache_is_current_rejects_stale_cache_when_sources_changed() -> anyhow::Result<()> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("mend-cache-source-test-{unique}"));
        let package_root = temp_dir.join("crate");
        let src_dir = package_root.join("src");
        fs::create_dir_all(&src_dir)?;
        fs::write(
            package_root.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )?;
        fs::write(src_dir.join("lib.rs"), "pub fn demo() {}\n")?;

        let findings_dir = temp_dir.join("findings");
        fs::create_dir_all(&findings_dir)?;
        let cache_path = findings_dir.join(cache_filename_for(&package_root));
        let report = StoredReport {
            version:      FINDINGS_SCHEMA_VERSION,
            package_root: package_root.to_string_lossy().into_owned(),
            findings:     Vec::new(),
        };
        fs::write(&cache_path, serde_json::to_vec(&report)?)?;

        assert!(cache_is_current_for(&findings_dir, &package_root));

        std::thread::sleep(std::time::Duration::from_secs(1));
        fs::write(
            src_dir.join("lib.rs"),
            "pub fn demo() {}\npub fn newer() {}\n",
        )?;

        assert!(!cache_is_current_for(&findings_dir, &package_root));

        fs::remove_dir_all(&temp_dir)?;
        Ok(())
    }
}
