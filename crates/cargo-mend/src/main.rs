use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use cargo_metadata::{Metadata, MetadataCommand, Package};
use clap::Parser;
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Deserialize;
use serde::Serialize;
use walkdir::WalkDir;

static RE_PUB_CRATE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bpub\s*\(\s*crate\s*\)").unwrap());
static RE_PUB_IN_CRATE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\bpub\s*\(\s*in\s+crate::").unwrap());
static RE_PUB_MOD: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:;|\{)").unwrap());
static RE_PUBLIC_USE_CHILD_ITEM: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*pub(?:\s*\([^)]*\))?\s+use\s+(.+)$").unwrap()
});

static RE_PUB_FN: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_STRUCT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+struct\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_ENUM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+enum\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_TRAIT: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+trait\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_TYPE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+type\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_CONST: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*pub\s+const\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
static RE_PUB_STATIC: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*pub\s+static(?:\s+mut)?\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
});

#[derive(Parser, Debug)]
#[command(name = "vischeck")]
#[command(about = "Audit Rust visibility patterns against a stricter house style")]
struct Cli {
    #[arg(long)]
    manifest_path: Option<PathBuf>,

    #[arg(long)]
    config: Option<PathBuf>,

    #[arg(long)]
    json: bool,

    #[arg(long)]
    fail_on_warn: bool,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    visibility: VisibilityConfig,
}

#[derive(Debug, Clone, Deserialize)]
struct VisibilityConfig {
    #[serde(default)]
    allow_pub_mod: Vec<String>,
    #[serde(default)]
    allow_pub_items: Vec<String>,
}

impl Default for VisibilityConfig {
    fn default() -> Self {
        Self {
            allow_pub_mod: Vec::new(),
            allow_pub_items: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy)]
struct DiagnosticSpec {
    code:        &'static str,
    headline:    &'static str,
    inline_help: Option<&'static str>,
    help_anchor: &'static str,
}

const DIAGNOSTICS: &[DiagnosticSpec] = &[
    DiagnosticSpec {
        code:        "forbidden_pub_crate",
        headline:    "use of `pub(crate)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-crate",
    },
    DiagnosticSpec {
        code:        "forbidden_pub_in_crate",
        headline:    "use of `pub(in crate::...)` is forbidden by policy",
        inline_help: None,
        help_anchor: "forbidden-pub-in-crate",
    },
    DiagnosticSpec {
        code:        "review_pub_mod",
        headline:    "`pub mod` requires explicit review or allowlisting",
        inline_help: None,
        help_anchor: "review-pub-mod",
    },
    DiagnosticSpec {
        code:        "suspicious_bare_pub",
        headline:    "bare `pub` is not publicly re-exported by its parent module",
        inline_help: Some("consider using: `pub(super)`"),
        help_anchor: "suspicious-bare-pub",
    },
];

fn diagnostic_spec(code: &str) -> &'static DiagnosticSpec {
    DIAGNOSTICS
        .iter()
        .find(|spec| spec.code == code)
        .unwrap_or_else(|| panic!("unknown diagnostic code: {code}"))
}

#[derive(Debug, Clone, Serialize)]
struct Finding {
    severity: Severity,
    code: String,
    path: String,
    line: usize,
    column: usize,
    highlight_len: usize,
    source_line: String,
    item: Option<String>,
    message: String,
}

#[derive(Debug, Default, Serialize)]
struct Report {
    root: String,
    findings: Vec<Finding>,
}

impl Report {
    fn has_errors(&self) -> bool { self.findings.iter().any(|f| f.severity == Severity::Error) }

    fn has_warnings(&self) -> bool { self.findings.iter().any(|f| f.severity == Severity::Warning) }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("vischeck: {err:#}");
            ExitCode::from(2)
        },
    }
}

fn run() -> Result<ExitCode> {
    let cli = Cli::parse_from(normalized_args());
    let selection = resolve_cargo_selection(cli.manifest_path.as_deref())?;
    let config = load_config(
        selection.manifest_dir.as_path(),
        selection.workspace_root.as_path(),
        cli.config.as_deref(),
    )?;
    let report = scan_selection(&selection, &config)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print!("{}", render_human_report(&report, std::io::stdout().is_terminal()));
    }

    if report.has_errors() {
        return Ok(ExitCode::from(1));
    }

    if cli.fail_on_warn && report.has_warnings() {
        return Ok(ExitCode::from(2));
    }

    Ok(ExitCode::SUCCESS)
}

fn normalized_args() -> Vec<std::ffi::OsString> {
    let mut args: Vec<_> = std::env::args_os().collect();
    if args.get(1).is_some_and(|arg| arg == "vischeck") {
        args.remove(1);
    }
    args
}

#[derive(Debug)]
struct LoadedConfig {
    config: VisibilityConfig,
    root: PathBuf,
}

fn load_config(manifest_dir: &Path, workspace_root: &Path, explicit: Option<&Path>) -> Result<LoadedConfig> {
    let candidates = if let Some(path) = explicit {
        vec![path.to_path_buf()]
    } else {
        let mut result = Vec::new();
        for root in [manifest_dir, workspace_root] {
            result.push(root.join("vischeck.toml"));
            result.push(root.join("visibility_audit.toml"));
            result.push(root.join(".visibility-audit.toml"));
        }
        result
    };

    for path in candidates {
        if path.exists() {
            let text = fs::read_to_string(&path)
                .with_context(|| format!("failed to read config {}", path.display()))?;
            let file: ConfigFile = toml::from_str(&text)
                .with_context(|| format!("failed to parse config {}", path.display()))?;
            let root = path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| manifest_dir.to_path_buf());
            return Ok(LoadedConfig {
                config: file.visibility,
                root,
            });
        }
    }

    Ok(LoadedConfig {
        config: VisibilityConfig::default(),
        root: manifest_dir.to_path_buf(),
    })
}

#[derive(Debug)]
struct Selection {
    manifest_dir: PathBuf,
    workspace_root: PathBuf,
    analysis_root: PathBuf,
    packages: Vec<Package>,
}

fn scan_selection(selection: &Selection, loaded_config: &LoadedConfig) -> Result<Report> {
    let mut findings = Vec::new();
    for package in &selection.packages {
        let crate_root = package
            .manifest_path
            .parent()
            .context("package manifest path had no parent directory")?;
        findings.extend(scan_crate(
            selection.analysis_root.as_path(),
            crate_root.as_std_path(),
            &loaded_config.root,
            &loaded_config.config,
        )?);
    }

    findings.sort_by(|a, b| {
        (a.severity, &a.path, a.line, &a.code).cmp(&(b.severity, &b.path, b.line, &b.code))
    });

    Ok(Report {
        root: selection.analysis_root.display().to_string(),
        findings,
    })
}

fn resolve_cargo_selection(explicit_manifest_path: Option<&Path>) -> Result<Selection> {
    let manifest_path = match explicit_manifest_path {
        Some(path) => path.canonicalize().with_context(|| format!("failed to canonicalize {}", path.display()))?,
        None => find_nearest_manifest(&std::env::current_dir()?)?,
    };

    let metadata = cargo_metadata_for(&manifest_path)?;
    let workspace_root = metadata.workspace_root.clone().into_std_path_buf();
    let manifest_dir = manifest_path
        .parent()
        .context("manifest path had no parent directory")?
        .to_path_buf();
    let workspace_manifest = workspace_root.join("Cargo.toml");

    let packages = if manifest_path == workspace_manifest {
        select_packages(&metadata, &metadata.workspace_members)?
    } else {
        let package = metadata
            .packages
            .iter()
            .find(|pkg| pkg.manifest_path.as_std_path() == manifest_path)
            .cloned()
            .with_context(|| format!("manifest {} not found in cargo metadata", manifest_path.display()))?;
        vec![package]
    };

    let analysis_root = if manifest_path == workspace_manifest {
        workspace_root.clone()
    } else {
        manifest_dir.clone()
    };

    Ok(Selection {
        manifest_dir,
        workspace_root,
        analysis_root,
        packages,
    })
}

fn cargo_metadata_for(manifest_path: &Path) -> Result<Metadata> {
    let mut command = MetadataCommand::new();
    command.no_deps();
    command.manifest_path(manifest_path);
    command.exec().context("failed to run cargo metadata")
}

fn find_nearest_manifest(start: &Path) -> Result<PathBuf> {
    for dir in start.ancestors() {
        let candidate = dir.join("Cargo.toml");
        if candidate.is_file() {
            return candidate
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", candidate.display()));
        }
    }

    bail!("could not find Cargo.toml in current directory or any parent")
}

fn select_packages(metadata: &Metadata, ids: &[cargo_metadata::PackageId]) -> Result<Vec<Package>> {
    let id_set: BTreeSet<_> = ids.iter().collect();
    let mut packages: Vec<_> = metadata
        .packages
        .iter()
        .filter(|pkg| id_set.contains(&pkg.id))
        .cloned()
        .collect();
    packages.sort_by(|a, b| a.manifest_path.cmp(&b.manifest_path));
    Ok(packages)
}

fn scan_crate(
    analysis_root: &Path,
    crate_root: &Path,
    config_root: &Path,
    config: &VisibilityConfig,
) -> Result<Vec<Finding>> {
    let src_root = crate_root.join("src");
    if !src_root.is_dir() {
        return Ok(Vec::new());
    }
    let root_module = if src_root.join("lib.rs").is_file() {
        src_root.join("lib.rs")
    } else {
        src_root.join("main.rs")
    };

    let mut source_files = Vec::new();
    let walker = WalkDir::new(&src_root).into_iter().filter_entry(|entry| {
        let name = entry.file_name().to_string_lossy();
        name != "target"
    });

    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_file() || entry.path().extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(entry.path())
            .with_context(|| format!("failed to read source file {}", entry.path().display()))?;
        source_files.push(SourceFile {
            path: entry.path().to_path_buf(),
            text,
        });
    }

    let mut findings = Vec::new();
    for source in &source_files {
        findings.extend(scan_file(
            analysis_root,
            crate_root,
            &src_root,
            &root_module,
            config_root,
            &source.path,
            &source.text,
            &source_files,
            config,
        )?);
    }

    Ok(findings)
}

struct SourceFile {
    path: PathBuf,
    text: String,
}

fn scan_file(
    root: &Path,
    crate_root: &Path,
    src_root: &Path,
    root_module: &Path,
    config_root: &Path,
    file: &Path,
    text: &str,
    source_files: &[SourceFile],
    config: &VisibilityConfig,
) -> Result<Vec<Finding>> {
    let rel_path = path_relative_to(file, root)?;
    let rel_path_string = rel_path.to_string_lossy().replace('\\', "/");

    let mut findings = Vec::new();
    let module_context = ModuleContext::for_file(crate_root, src_root, root_module, file);
    let config_rel_path = path_relative_to(file, config_root)
        .ok()
        .map(|p| p.to_string_lossy().replace('\\', "/"));

    let mut depth = 0usize;
    for (idx, line) in text.lines().enumerate() {
        let line_no = idx + 1;
        let sanitized = sanitize_for_visibility_checks(line);

        if let Some(matched) = RE_PUB_CRATE.find(&sanitized) {
            let (column, highlight_len) =
                highlight_from_range(line, matched.start(), matched.end());
            findings.push(Finding {
                severity: Severity::Error,
                code: "forbidden_pub_crate".to_string(),
                path: rel_path_string.clone(),
                line: line_no,
                column,
                highlight_len,
                source_line: line.to_string(),
                item: None,
                message: "use of `pub(crate)` is forbidden by policy".to_string(),
            });
        }

        if let Some(matched) = RE_PUB_IN_CRATE.find(&sanitized) {
            let (column, highlight_len) =
                highlight_from_range(line, matched.start(), matched.end());
            findings.push(Finding {
                severity: Severity::Error,
                code: "forbidden_pub_in_crate".to_string(),
                path: rel_path_string.clone(),
                line: line_no,
                column,
                highlight_len,
                source_line: line.to_string(),
                item: None,
                message: "use of `pub(in crate::...)` is forbidden by policy".to_string(),
            });
        }

        if let Some(captures) = RE_PUB_MOD.captures(&sanitized) {
            let allowlisted = config_rel_path
                .as_ref()
                .is_some_and(|config_rel| config.allow_pub_mod.iter().any(|allowed| allowed == config_rel));
            if !allowlisted {
                let module_name = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
                let start = sanitized.find("pub mod").unwrap_or(0);
                let end = sanitized
                    .find(module_name)
                    .map(|index| index + module_name.len())
                    .unwrap_or_else(|| start + "pub mod".len());
                let (column, highlight_len) = highlight_from_range(line, start, end);
                findings.push(Finding {
                    severity: Severity::Error,
                    code: "review_pub_mod".to_string(),
                    path: rel_path_string.clone(),
                    line: line_no,
                    column,
                    highlight_len,
                    source_line: line.to_string(),
                    item: captures.get(1).map(|m| m.as_str().to_string()),
                    message: "`pub mod` requires explicit review or allowlisting".to_string(),
                });
            }
        }

        let sanitized_trimmed = sanitized.trim_start();
        if depth == 0
            && sanitized_trimmed.starts_with("pub ")
            && !sanitized_trimmed.starts_with("pub mod")
            && !sanitized_trimmed.starts_with("pub use")
        {
            if let Some((kind, name)) = bare_pub_item(sanitized_trimmed) {
                let item_key = config_rel_path.as_ref().map(|path| format!("{path}::{name}"));
                let allowlisted = item_key
                    .as_ref()
                    .is_some_and(|key| config.allow_pub_items.iter().any(|allowed| allowed == key));
                if !allowlisted {
                    if let Some(reason) = suspicious_pub_reason(
                        &module_context,
                        &name,
                        file,
                        source_files,
                    )? {
                        let start = sanitized.find("pub").unwrap_or(0);
                        let end = sanitized
                            .find(&name)
                            .map(|index| index + name.len())
                            .unwrap_or_else(|| line.len());
                        let (column, highlight_len) = highlight_from_range(line, start, end);
                        findings.push(Finding {
                            severity: Severity::Warning,
                            code: "suspicious_bare_pub".to_string(),
                            path: rel_path_string.clone(),
                            line: line_no,
                            column,
                            highlight_len,
                            source_line: line.to_string(),
                            item: Some(format!("{kind} {name}")),
                            message: reason,
                        });
                    }
                }
            }
        }

        depth = update_brace_depth(depth, line);
    }

    Ok(findings)
}

fn suspicious_pub_reason(
    module_context: &ModuleContext,
    item_name: &str,
    file: &Path,
    source_files: &[SourceFile],
) -> Result<Option<String>> {
    if module_context.is_root_or_boundary_file {
        return Ok(None);
    }

    let used_elsewhere = item_name_used_elsewhere(item_name, file, source_files)?;
    if !module_context.parent_module_is_public
        && !module_context.parent_publicly_reexports(item_name)?
        && !used_elsewhere
    {
        return Ok(Some(
            "bare `pub` item lives in a non-root child module whose parent module is private, is not publicly re-exported by that parent, and appears unused outside its defining file"
                .to_string(),
        ));
    }

    Ok(None)
}

fn item_name_used_elsewhere(item_name: &str, file: &Path, source_files: &[SourceFile]) -> Result<bool> {
    let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(item_name)))?;
    Ok(source_files
        .iter()
        .filter(|source| source.path != file)
        .any(|source| pattern.is_match(&source.text)))
}

fn sanitize_for_visibility_checks(line: &str) -> String {
    let mut result = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if !in_string && ch == '/' && chars.peek() == Some(&'/') {
            break;
        }

        if in_string {
            if escaped {
                escaped = false;
                result.push(' ');
                continue;
            }

            match ch {
                '\\' => {
                    escaped = true;
                    result.push(' ');
                },
                '"' => {
                    in_string = false;
                    result.push(' ');
                },
                _ => result.push(' '),
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(' ');
            continue;
        }

        result.push(ch);
    }

    result
}

fn highlight_from_range(line: &str, start: usize, end: usize) -> (usize, usize) {
    let safe_start = start.min(line.len());
    let safe_end = end.max(safe_start).min(line.len());
    let column = line[..safe_start].chars().count() + 1;
    let highlight_len = line[safe_start..safe_end].chars().count().max(1);
    (column, highlight_len)
}

fn bare_pub_item(trimmed: &str) -> Option<(&'static str, String)> {
    let candidates = [
        ("fn", &*RE_PUB_FN),
        ("struct", &*RE_PUB_STRUCT),
        ("enum", &*RE_PUB_ENUM),
        ("trait", &*RE_PUB_TRAIT),
        ("type", &*RE_PUB_TYPE),
        ("const", &*RE_PUB_CONST),
        ("static", &*RE_PUB_STATIC),
    ];

    for (kind, re) in candidates {
        if let Some(captures) = re.captures(trimmed) {
            return captures.get(1).map(|m| (kind, m.as_str().to_string()));
        }
    }

    None
}

fn update_brace_depth(mut depth: usize, line: &str) -> usize {
    for ch in line.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ => {},
        }
    }
    depth
}

fn path_relative_to<'a>(path: &'a Path, root: &Path) -> Result<&'a Path> {
    path.strip_prefix(root)
        .with_context(|| format!("failed to make {} relative to {}", path.display(), root.display()))
}

#[derive(Debug, Clone)]
struct ModuleContext {
    parent_file: Option<PathBuf>,
    child_module_name: Option<String>,
    parent_module_is_public: bool,
    is_root_or_boundary_file: bool,
}

impl ModuleContext {
    fn for_file(crate_root: &Path, src_root: &Path, root_module: &Path, file: &Path) -> Self {
        let rel_to_src = file.strip_prefix(src_root).unwrap();
        let is_root_file = file == root_module;
        let is_mod_rs = file.file_name().and_then(|n| n.to_str()) == Some("mod.rs");
        let is_top_level_file = rel_to_src.components().count() == 1;

        if is_root_file || is_mod_rs || is_top_level_file {
            return Self {
                parent_file: None,
                child_module_name: None,
                parent_module_is_public: false,
                is_root_or_boundary_file: true,
            };
        }

        let child_module_name = file.file_stem().and_then(|s| s.to_str()).unwrap().to_string();
        let parent_dir = file.parent().unwrap();
        let parent_file = if parent_dir == src_root {
            root_module.to_path_buf()
        } else {
            let mod_rs = parent_dir.join("mod.rs");
            if mod_rs.is_file() {
                mod_rs
            } else {
                let parent_module_name = parent_dir.file_name().and_then(|s| s.to_str()).unwrap();
                crate_root.join("src").join(format!("{parent_module_name}.rs"))
            }
        };

        let parent_text = fs::read_to_string(&parent_file).unwrap_or_default();
        let parent_module_is_public = parent_declares_public_module(&parent_text, &child_module_name);

        Self {
            parent_file: Some(parent_file),
            child_module_name: Some(child_module_name),
            parent_module_is_public,
            is_root_or_boundary_file: false,
        }
    }

    fn parent_publicly_reexports(&self, item_name: &str) -> Result<bool> {
        let (Some(parent_file), Some(child_module_name)) = (&self.parent_file, &self.child_module_name) else {
            return Ok(false);
        };

        let text = fs::read_to_string(parent_file)
            .with_context(|| format!("failed to read parent module {}", parent_file.display()))?;

        for line in text.lines() {
            let Some(captures) = RE_PUBLIC_USE_CHILD_ITEM.captures(line) else {
                continue;
            };

            let body = captures.get(1).map(|m| m.as_str()).unwrap_or_default();
            if !body.contains(child_module_name) {
                continue;
            }

            if body.contains(&format!("{child_module_name}::*")) {
                return Ok(true);
            }

            if body.contains(&format!("{child_module_name}::{item_name}")) {
                return Ok(true);
            }

            if body.contains(&format!("{child_module_name}::{{")) && body.contains(item_name) {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

fn parent_declares_public_module(parent_text: &str, child_module_name: &str) -> bool {
    let exact = format!("pub mod {child_module_name}");
    parent_text.lines().any(|line| line.contains(&exact))
}

fn render_human_report(report: &Report, color: bool) -> String {
    if report.findings.is_empty() {
        return "No findings.\n".to_string();
    }

    let mut output = String::new();
    for finding in &report.findings {
        render_finding(&mut output, finding, color);
    }

    let error_count = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warn_count = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let _ = writeln!(output, "{}", summary_line(error_count, warn_count, color));
    output
}

fn render_finding(output: &mut String, finding: &Finding, color: bool) {
    let severity = severity_label(finding.severity, color);
    let headline = finding_headline(finding);
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let arrow_pad = " ".repeat(gutter_width);
    let _ = writeln!(output, "{severity} {headline}");
    let _ = writeln!(
        output,
        "{}{} {}:{}:{}",
        arrow_pad,
        blue_bold("-->", color),
        finding.path,
        finding.line,
        finding.column
    );
    let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
    let _ = writeln!(
        output,
        "{:>width$} {} {}",
        blue_bold(&line_label, color),
        blue_bold("|", color),
        finding.source_line,
        width = gutter_width
    );
    let _ = writeln!(
        output,
        "{}{} {}",
        gutter_pad,
        blue_bold("|", color),
        severity_marker(finding.severity, finding.column, finding.highlight_len, color)
    );
    if let Some(inline_help) = inline_help_text(finding) {
        let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
        let _ = writeln!(
            output,
            "{}{} {}",
            gutter_pad,
            blue_bold("|", color),
            blue_bold(&format!("help: {inline_help}"), color)
        );
    }

    let reasons = detail_reasons(finding);
    if inline_help_text(finding).is_some() || !reasons.is_empty() {
        let _ = writeln!(output, "{}{}", gutter_pad, blue_bold("|", color));
    }
    if !reasons.is_empty() {
        for reason in reasons {
            let _ = writeln!(output, "{}{} {}", gutter_pad, diagnostic_label("note", color), reason);
        }
    }
    if let Some(help_url) = finding_help_url(finding) {
        let _ = writeln!(
            output,
            "{}{} for further information visit {help_url}",
            gutter_pad,
            diagnostic_label("help", color)
        );
    }
    let _ = writeln!(output);
}

fn split_message(message: &str) -> Vec<String> {
    message
        .split(", and ")
        .flat_map(|part| part.split("; "))
        .flat_map(|part| part.split(", "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn finding_headline(finding: &Finding) -> String {
    diagnostic_spec(&finding.code).headline.to_string()
}

fn detail_reasons(finding: &Finding) -> Vec<String> {
    match finding.code.as_str() {
        "suspicious_bare_pub" => {
            let reasons = split_message(&finding.message);
            if reasons.iter().any(|reason| reason == "appears unused outside its defining file") {
                vec!["it appears unused outside its defining file".to_string()]
            } else {
                Vec::new()
            }
        },
        _ => Vec::new(),
    }
}

fn inline_help_text(finding: &Finding) -> Option<&'static str> {
    diagnostic_spec(&finding.code).inline_help
}

fn finding_help_url(finding: &Finding) -> Option<String> {
    Some(format!(
        "https://github.com/natepiano/cargo-vischeck#{}",
        diagnostic_spec(&finding.code).help_anchor
    ))
}

fn summary_line(error_count: usize, warn_count: usize, color: bool) -> String {
    format!("{} {} error(s), {} warning(s)", dim("summary:", color), error_count, warn_count)
}

fn severity_label(severity: Severity, color: bool) -> String {
    match severity {
        Severity::Error => paint("error:", "1;31", color),
        Severity::Warning => paint("warn:", "1;33", color),
    }
}

fn dim(text: &str, color: bool) -> String { paint(text, "2", color) }

fn blue_bold(text: &str, color: bool) -> String { paint(text, "1;34", color) }

fn severity_marker(severity: Severity, column: usize, highlight_len: usize, color: bool) -> String {
    let indent = " ".repeat(column.saturating_sub(1));
    let carets = "^".repeat(highlight_len.max(1));
    let code = match severity {
        Severity::Error => "1;31",
        Severity::Warning => "1;33",
    };
    format!("{indent}{}", paint(&carets, code, color))
}

fn diagnostic_label(kind: &str, color: bool) -> String {
    let prefix = blue_bold("=", color);
    let label = match kind {
        "help" => paint("help", "1", color),
        "note" => paint("note", "1", color),
        other => other.to_string(),
    };
    format!("{prefix} {label}:")
}

fn paint(text: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;

    use tempfile::tempdir;

    #[test]
    fn every_diagnostic_has_a_unique_readme_anchor() {
        let readme = include_str!("../README.md");
        let mut seen_codes = BTreeSet::new();
        let mut seen_anchors = BTreeSet::new();

        for spec in DIAGNOSTICS {
            assert!(seen_codes.insert(spec.code), "duplicate diagnostic code: {}", spec.code);
            assert!(
                seen_anchors.insert(spec.help_anchor),
                "duplicate README anchor: {}",
                spec.help_anchor
            );
            let anchor = format!(r#"<a id="{}"></a>"#, spec.help_anchor);
            assert!(
                readme.contains(&anchor),
                "README is missing anchor for {}: {}",
                spec.code,
                spec.help_anchor
            );
        }
    }

    #[test]
    fn fixture_renders_every_current_diagnostic() -> Result<()> {
        let temp = tempdir()?;
        fs::create_dir_all(temp.path().join("src/private_parent"))?;

        fs::write(
            temp.path().join("Cargo.toml"),
            r#"[package]
name = "fixture"
version = "0.1.0"
edition = "2024"
"#,
        )?;
        fs::write(
            temp.path().join("src/main.rs"),
            r#"pub(crate) fn crate_only() {}
pub(in crate::private_parent) fn subtree_only() {}
pub mod review_mod;
mod private_parent;

fn main() {}
"#,
        )?;
        fs::write(temp.path().join("src/review_mod.rs"), "\n")?;
        fs::write(temp.path().join("src/private_parent.rs"), "mod child;\n")?;
        fs::write(
            temp.path().join("src/private_parent/child.rs"),
            "pub struct Suspicious;\n",
        )?;

        let manifest_path = temp.path().join("Cargo.toml");
        let selection = resolve_cargo_selection(Some(&manifest_path))?;
        let loaded_config = load_config(
            selection.manifest_dir.as_path(),
            selection.workspace_root.as_path(),
            None,
        )?;
        let report = scan_selection(&selection, &loaded_config)?;

        let rendered = render_human_report(&report, false);
        let codes: BTreeSet<_> = report.findings.iter().map(|finding| finding.code.as_str()).collect();
        let expected_codes: BTreeSet<_> = DIAGNOSTICS.iter().map(|spec| spec.code).collect();

        assert_eq!(codes, expected_codes, "fixture should trigger every diagnostic exactly once");
        assert_eq!(
            report.findings.len(),
            DIAGNOSTICS.len(),
            "fixture should trigger one finding per diagnostic"
        );

        for spec in DIAGNOSTICS {
            assert!(
                rendered.contains(spec.headline),
                "rendered output is missing headline for {}",
                spec.code
            );
            let help_url = format!(
                "https://github.com/natepiano/cargo-vischeck#{}",
                spec.help_anchor
            );
            assert!(
                rendered.contains(&help_url),
                "rendered output is missing help URL for {}",
                spec.code
            );
        }

        assert!(rendered.contains("help: consider using: `pub(super)`"));
        Ok(())
    }
}
