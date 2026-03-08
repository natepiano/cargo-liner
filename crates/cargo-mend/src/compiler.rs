use std::env;
use std::ffi::OsString;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
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
use rustc_span::RealFileName;
use rustc_span::Span;
use rustc_span::def_id::CRATE_DEF_ID;
use rustc_span::def_id::LocalDefId;
use serde::Deserialize;
use serde::Serialize;
use super::config::LoadedConfig;
use super::config::VisibilityConfig;
use super::diagnostics::Finding;
use super::diagnostics::Report;
use super::diagnostics::Severity;
use super::selection::Selection;

const DRIVER_ENV: &str = "VISCHECK_DRIVER";
const CONFIG_ROOT_ENV: &str = "VISCHECK_CONFIG_ROOT";
const CONFIG_JSON_ENV: &str = "VISCHECK_CONFIG_JSON";
const FINDINGS_DIR_ENV: &str = "VISCHECK_FINDINGS_DIR";
const PACKAGE_ROOT_ENV: &str = "CARGO_MANIFEST_DIR";
const FINDINGS_SCHEMA_VERSION: u32 = 2;

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
            env::var_os(CONFIG_ROOT_ENV)
                .context("missing VISCHECK_CONFIG_ROOT for compiler driver")?,
        );
        let config = serde_json::from_str(
            &env::var(CONFIG_JSON_ENV)
                .context("missing VISCHECK_CONFIG_JSON for compiler driver")?,
        )
        .context("failed to parse VISCHECK_CONFIG_JSON")?;
        let findings_dir = PathBuf::from(
            env::var_os(FINDINGS_DIR_ENV)
                .context("missing VISCHECK_FINDINGS_DIR for compiler driver")?,
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
    fn new(settings: DriverSettings) -> Self {
        Self {
            settings,
            error: None,
        }
    }
}

impl Callbacks for AnalysisCallbacks {
    fn after_analysis<'tcx>(
        &mut self,
        _compiler: &rustc_interface::interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        match collect_and_store_findings(tcx, &self.settings) {
            Ok(true) => return Compilation::Continue,
            Ok(false) => return Compilation::Continue,
            Err(err) => {
                self.error = Some(err);
                return Compilation::Stop;
            },
        }
    }
}

pub(super) fn run_selection(selection: &Selection, loaded_config: &LoadedConfig) -> Result<Report> {
    let findings_dir = selection.target_directory.join("vischeck-findings");
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create persistent findings directory {}",
            findings_dir.display()
        )
    })?;

    let status = run_cargo_check(selection, loaded_config, &findings_dir)?;

    if !status.success() {
        anyhow::bail!("cargo check failed during vischeck analysis");
    }

    let missing_packages = selection
        .packages
        .iter()
        .filter(|package| !cache_is_current_for(&findings_dir, &package.root))
        .collect::<Vec<_>>();

    if !missing_packages.is_empty() {
        for package in missing_packages {
            let status = run_cargo_rustc_for_package(package, loaded_config, &findings_dir)?;
            if !status.success() {
                anyhow::bail!(
                    "cargo rustc refresh failed during vischeck analysis for package {}",
                    package.name
                );
            }
        }
    }

    let report = load_report(&findings_dir, selection)?;

    Ok(report)
}

pub(super) fn driver_main() -> ExitCode {
    match driver_main_impl() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vischeck: {err:#}");
            ExitCode::from(1)
        },
    }
}

fn driver_main_impl() -> Result<ExitCode> {
    let wrapper_args: Vec<OsString> = env::args_os().collect();
    if wrapper_args.len() < 2 {
        anyhow::bail!("compiler driver expected rustc wrapper arguments");
    }
    let settings = match DriverSettings::from_env() {
        Ok(settings) => settings,
        Err(_) => return passthrough_to_rustc(&wrapper_args),
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
    let mut exit_code = rustc_driver::catch_with_exit_code(|| {
        rustc_driver::run_compiler(&rustc_args, &mut callbacks);
    });

    if let Some(err) = callbacks.error {
        eprintln!("vischeck: {err:#}");
        exit_code = 1;
    }

    Ok(ExitCode::from(exit_code as u8))
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
        .context("failed to invoke rustc passthrough from vischeck wrapper")?;
    Ok(ExitCode::from(status.code().unwrap_or(1) as u8))
}

fn run_cargo_check(
    selection: &Selection,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
) -> Result<std::process::ExitStatus> {
    let current_exe = env::current_exe().context("failed to determine current executable path")?;
    let mut command = Command::new("cargo");
    command.arg("check");

    if selection.is_workspace_selection {
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
                .context("failed to serialize vischeck config for compiler driver")?,
        )
        .env(FINDINGS_DIR_ENV, findings_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    command
        .status()
        .context("failed to run cargo check for vischeck")
}

fn run_cargo_rustc_for_package(
    package: &super::selection::SelectedPackage,
    loaded_config: &LoadedConfig,
    findings_dir: &Path,
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
                .context("failed to serialize vischeck config for compiler driver")?,
        )
        .env(FINDINGS_DIR_ENV, findings_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    command
        .status()
        .with_context(|| format!("failed to run cargo rustc refresh for package {}", package.name))
}

fn refresh_rustc_args() -> Vec<String> {
    vec![
        "--".to_string(),
        format!("--cfg=vischeck_refresh_{}", std::process::id()),
    ]
}

fn cache_is_current_for(findings_dir: &Path, package_root: &Path) -> bool {
    let cache_path = findings_dir.join(cache_filename_for(package_root));
    let Ok(text) = fs::read_to_string(&cache_path) else {
        return false;
    };
    let Ok(stored) = serde_json::from_str::<StoredReport>(&text) else {
        return false;
    };
    stored.version == FINDINGS_SCHEMA_VERSION
        && stored.package_root == package_root.to_string_lossy()
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
        let matches_selected_root = selected_roots.iter().any(|root| root == &stored.package_root)
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
        summary: Default::default(),
        findings,
    })
}

fn selection_root_string(root: &Path) -> String { root.display().to_string() }

fn relativize_path(path: &str, analysis_root: &Path) -> String {
    let absolute = Path::new(path);
    if let Ok(relative) = absolute.strip_prefix(analysis_root) {
        relative.to_string_lossy().replace('\\', "/")
    } else {
        path.to_string()
    }
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

    if findings.is_empty() {
        return Ok(true);
    }

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

    let output_path = settings
        .findings_dir
        .join(cache_filename_for(&settings.package_root));
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
        item.kind.ident().map(|ident| ident.to_string()),
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

    record_visibility_findings(
        tcx,
        settings,
        src_root,
        root_module,
        effective_visibilities,
        item.owner_id.def_id,
        &file_path,
        &vis_text,
        impl_item_kind_label(item.kind),
        Some(item.ident.to_string()),
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

    record_visibility_findings(
        tcx,
        settings,
        src_root,
        root_module,
        effective_visibilities,
        item.owner_id.def_id,
        &file_path,
        &vis_text,
        foreign_item_kind_label(item.kind),
        Some(item.ident.to_string()),
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
    item_name: Option<String>,
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
        && tcx.parent_module_from_def_id(parent_module.to_local_def_id())
            .to_local_def_id()
            == CRATE_DEF_ID;
    let module_location = module_location(parent_is_crate_root, grandparent_is_crate_root);

    if matches!(vis_text, "pub(crate)") {
        if !allow_pub_crate_by_policy(crate_kind, module_location, parent_is_public) {
            findings.push(build_finding(
                tcx,
                file_path,
                highlight_span,
                Severity::Error,
                "forbidden_pub_crate",
                None,
                "use of `pub(crate)` is forbidden by policy".to_string(),
                Some(forbidden_pub_crate_help(module_location).to_string()),
            )?);
        }
    }

    if vis_text.starts_with("pub(in crate::") {
        findings.push(build_finding(
            tcx,
            file_path,
            highlight_span,
            Severity::Error,
            "forbidden_pub_in_crate",
            None,
            "use of `pub(in crate::...)` is forbidden by policy".to_string(),
            None,
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
                Severity::Error,
                "review_pub_mod",
                item_name.clone(),
                "`pub mod` requires explicit review or allowlisting".to_string(),
                None,
            )?);
        }
    }

    if vis_text == "pub"
        && let Some(kind_label) = kind_label
        && !is_boundary_file(src_root, root_module, file_path)
    {
        let item_key = config_rel_path
            .as_ref()
            .and_then(|path| item_name.as_ref().map(|name| format!("{path}::{name}")));
        let allowlisted = item_key.as_ref().is_some_and(|key| {
            settings
                .config
                .allow_pub_items
                .iter()
                .any(|allowed| allowed == key)
        });
        let reachable = effective_visibilities.is_public_at_level(def_id, Level::Reachable);

        if !allowlisted
            && !parent_is_public
            && !matches!(module_location, ModuleLocation::TopLevelPrivateModule)
            && !reachable
        {
            findings.push(build_finding(
                tcx,
                file_path,
                highlight_span,
                Severity::Warning,
                "suspicious_pub",
                item_name
                    .as_ref()
                    .map(|name| format!("{kind_label} {name}")),
                "it is not reachable from the crate's public API after analysis".to_string(),
                None,
            )?);
        }
    }
    Ok(())
}

fn build_finding(
    tcx: TyCtxt<'_>,
    file_path: &Path,
    highlight_span: Span,
    severity: Severity,
    code: &str,
    item: Option<String>,
    message: String,
    suggestion: Option<String>,
) -> Result<StoredFinding> {
    let display = line_display(tcx, file_path, highlight_span)?;
    Ok(StoredFinding {
        severity,
        code: code.to_string(),
        path: file_path.to_string_lossy().into_owned(),
        line: display.line,
        column: display.column,
        highlight_len: display.highlight_len,
        source_line: display.source_line,
        item,
        message,
        suggestion,
    })
}

fn module_location(
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

fn allow_pub_crate_by_policy(
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

fn forbidden_pub_crate_help(module_location: ModuleLocation) -> &'static str {
    if matches!(
        module_location,
        ModuleLocation::CrateRoot | ModuleLocation::TopLevelPrivateModule
    ) {
        "consider using just `pub` or removing `pub(crate)` entirely"
    } else {
        "consider using `pub(super)` or removing `pub(crate)` entirely"
    }
}

#[derive(Debug)]
struct LineDisplay {
    line:          usize,
    column:        usize,
    highlight_len: usize,
    source_line:   String,
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
        FileName::Real(RealFileName::LocalPath(path)) => Some(path),
        _ => None,
    }
}

fn highlight_span(vis_span: Span, ident_span: Option<Span>) -> Span {
    ident_span.map_or(vis_span, |ident_span| vis_span.to(ident_span))
}

fn item_kind_label(kind: ItemKind<'_>) -> Option<&'static str> {
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
        ItemKind::Use(..) => None,
        ItemKind::ExternCrate(..)
        | ItemKind::ForeignMod { .. }
        | ItemKind::GlobalAsm { .. }
        | ItemKind::Impl(..)
        | ItemKind::Macro(..) => None,
    }
}

fn impl_item_kind_label(kind: ImplItemKind<'_>) -> Option<&'static str> {
    match kind {
        ImplItemKind::Const(..) => Some("const"),
        ImplItemKind::Fn(..) => Some("fn"),
        ImplItemKind::Type(..) => Some("type"),
    }
}

fn foreign_item_kind_label(kind: ForeignItemKind<'_>) -> Option<&'static str> {
    match kind {
        ForeignItemKind::Fn(..) => Some("fn"),
        ForeignItemKind::Static(..) => Some("static"),
        ForeignItemKind::Type => Some("type"),
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
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::CrateKind;
    use super::FINDINGS_SCHEMA_VERSION;
    use super::ModuleLocation;
    use super::allow_pub_crate_by_policy;
    use super::cache_filename_for;
    use super::cache_is_current_for;
    use super::forbidden_pub_crate_help;
    use super::module_location;
    use super::refresh_rustc_args;
    use super::Severity;
    use super::StoredFinding;
    use super::StoredReport;

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
    fn refresh_rustc_args_adds_vischeck_cfg() {
        let args = refresh_rustc_args();
        assert_eq!(args.first().map(String::as_str), Some("--"));
        assert!(
            args.get(1)
                .is_some_and(|arg| arg.starts_with("--cfg=vischeck_refresh_"))
        );
    }

    #[test]
    fn cache_is_current_requires_matching_schema_version() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("vischeck-cache-test-{unique}"));
        fs::create_dir_all(&temp_dir).unwrap();

        let package_root = Path::new("/tmp/example-crate");
        let cache_path = temp_dir.join(cache_filename_for(package_root));
        let stale = StoredReport {
            version: FINDINGS_SCHEMA_VERSION - 1,
            package_root: package_root.to_string_lossy().into_owned(),
            findings: vec![StoredFinding {
                severity: Severity::Warning,
                code: "suspicious_pub".to_string(),
                path: "src/lib.rs".to_string(),
                line: 1,
                column: 1,
                highlight_len: 3,
                source_line: "pub fn x() {}".to_string(),
                item: None,
                message: String::new(),
                suggestion: None,
            }],
        };
        fs::write(&cache_path, serde_json::to_vec(&stale).unwrap()).unwrap();

        assert!(!cache_is_current_for(&temp_dir, package_root));

        fs::remove_dir_all(&temp_dir).unwrap();
    }
}
