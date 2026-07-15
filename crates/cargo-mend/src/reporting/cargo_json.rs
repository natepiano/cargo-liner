use std::fs;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use serde::Serialize;
use serde_json::Value;
use serde_json::to_string;

use super::constants::CARGO_MESSAGE_TYPE_DIAGNOSTIC;
use super::constants::CARGO_REASON_BUILD_FINISHED;
use super::constants::CARGO_REASON_COMPILER_MESSAGE;
use super::constants::RUSTC_LEVEL_HELP;
use super::constants::RUSTC_LEVEL_NOTE;
use super::diagnostics;
use super::diagnostics::BuildOutcome;
use super::diagnostics::Finding;
use super::diagnostics::Report;
use super::diagnostics::Severity;
use crate::compiler::SOURCE_DIR_SRC;
use crate::selection::CARGO_TARGET_KIND_BIN;
use crate::selection::CARGO_TARGET_KIND_LIB;
use crate::selection::PackageMetadata;
use crate::selection::Selection;
use crate::selection::TargetMetadata;
use crate::selection::TargetSupport;

#[derive(Serialize)]
struct CompilerMessage<'a> {
    reason:        &'static str,
    package_id:    &'a str,
    manifest_path: String,
    target:        CargoTarget,
    message:       RustcDiagnostic,
}

#[derive(Serialize)]
struct CargoTarget {
    kind:              Vec<String>,
    crate_types:       Vec<String>,
    name:              String,
    src_path:          String,
    edition:           String,
    #[serde(rename = "required-features", skip_serializing_if = "Vec::is_empty")]
    required_features: Vec<String>,
    doc:               TargetSupport,
    doctest:           TargetSupport,
    test:              TargetSupport,
}

#[derive(Serialize)]
struct RustcDiagnostic {
    rendered:     String,
    #[serde(rename = "$message_type")]
    message_type: &'static str,
    children:     Vec<RustcDiagnosticChild>,
    level:        &'static str,
    message:      String,
    spans:        Vec<RustcSpan>,
    code:         RustcCode,
}

#[derive(Serialize)]
struct RustcDiagnosticChild {
    children: Vec<Self>,
    code:     Option<RustcCode>,
    level:    &'static str,
    message:  String,
    rendered: Option<String>,
    spans:    Vec<RustcSpan>,
}

#[derive(Serialize)]
struct RustcCode {
    code:        String,
    explanation: Option<String>,
}

#[derive(Serialize, Clone)]
struct RustcSpan {
    byte_end:                 usize,
    byte_start:               usize,
    column_end:               usize,
    column_start:             usize,
    expansion:                Option<Value>,
    file_name:                String,
    is_primary:               bool,
    label:                    Option<String>,
    line_end:                 usize,
    line_start:               usize,
    suggested_replacement:    Option<String>,
    suggestion_applicability: Option<&'static str>,
    text:                     Vec<RustcSpanText>,
}

#[derive(Serialize, Clone)]
struct RustcSpanText {
    highlight_end:   usize,
    highlight_start: usize,
    text:            String,
}

#[derive(Serialize)]
struct BuildFinished {
    reason:  &'static str,
    success: BuildOutcome,
}

pub(crate) fn render_report(report: &Report, selection: &Selection) -> Result<String> {
    let mut output = String::new();
    for finding in &report.findings {
        let message = compiler_message(finding, selection)?;
        output.push_str(&to_string(&message)?);
        output.push('\n');
    }
    output.push_str(&to_string(&BuildFinished {
        reason:  CARGO_REASON_BUILD_FINISHED,
        success: report.outcome(),
    })?);
    output.push('\n');
    Ok(output)
}

fn compiler_message<'a>(
    finding: &Finding,
    selection: &'a Selection,
) -> Result<CompilerMessage<'a>> {
    let absolute_path = absolute_finding_path(finding, selection);
    let package = package_for_path(selection, &absolute_path)
        .context("selection did not include packages")?;
    let target =
        target_for_path(package, &absolute_path).context("package did not include targets")?;
    let span = rustc_span(finding, selection, &absolute_path);
    let message = rustc_diagnostic(finding, span);

    Ok(CompilerMessage {
        reason: CARGO_REASON_COMPILER_MESSAGE,
        package_id: &package.id,
        manifest_path: package.manifest_path.display().to_string(),
        target: cargo_target(target),
        message,
    })
}

fn rustc_diagnostic(finding: &Finding, span: RustcSpan) -> RustcDiagnostic {
    let mut children = Vec::new();
    if !finding.message.is_empty() {
        children.push(child(RUSTC_LEVEL_NOTE, finding.message.clone(), Vec::new()));
    }
    if let Some(related) = &finding.related {
        children.push(child(RUSTC_LEVEL_NOTE, related.clone(), Vec::new()));
    }
    if let Some(help) = diagnostics::inline_help_text(finding) {
        children.push(child(
            RUSTC_LEVEL_HELP,
            help.to_string(),
            vec![span.clone()],
        ));
    }
    if let Some(help) = diagnostics::custom_inline_help_text(finding) {
        children.push(child(
            RUSTC_LEVEL_HELP,
            help.to_string(),
            vec![span.clone()],
        ));
    }
    if let Some(note) = diagnostics::effective_fixability(finding).note() {
        children.push(child(RUSTC_LEVEL_HELP, note.to_string(), Vec::new()));
    }

    let level = severity_level(finding.severity);
    RustcDiagnostic {
        rendered: render_diagnostic(finding, &span, level),
        message_type: CARGO_MESSAGE_TYPE_DIAGNOSTIC,
        children,
        level,
        message: diagnostics::finding_headline(finding),
        spans: vec![span],
        code: RustcCode {
            code:        finding.diagnostic_code.as_str().to_string(),
            explanation: None,
        },
    }
}

const fn child(
    level: &'static str,
    message: String,
    spans: Vec<RustcSpan>,
) -> RustcDiagnosticChild {
    RustcDiagnosticChild {
        children: Vec::new(),
        code: None,
        level,
        message,
        rendered: None,
        spans,
    }
}

fn render_diagnostic(finding: &Finding, span: &RustcSpan, level: &str) -> String {
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let marker_pad = " ".repeat(span.column_start.saturating_sub(1));
    let marker_len = span.column_end.saturating_sub(span.column_start).max(1);
    let marker = "^".repeat(marker_len);
    let inline_help = diagnostics::inline_help_text(finding)
        .or_else(|| diagnostics::custom_inline_help_text(finding))
        .map_or_else(String::new, |help| format!(" help: {help}"));
    let mut rendered = format!(
        "{level}: {}\n --> {}:{}:{}\n{gutter_pad}|\n{line_label} | {}\n{gutter_pad}| {marker_pad}{marker}{inline_help}\n",
        diagnostics::finding_headline(finding),
        span.file_name,
        finding.line,
        finding.column,
        finding.source_line,
    );
    if !finding.message.is_empty() {
        rendered.push_str("  = note: ");
        rendered.push_str(&finding.message);
        rendered.push('\n');
    }
    if let Some(related) = &finding.related {
        rendered.push_str("  = note: ");
        rendered.push_str(related);
        rendered.push('\n');
    }
    if let Some(note) = diagnostics::effective_fixability(finding).note() {
        rendered.push_str("  = help: ");
        rendered.push_str(note);
        rendered.push('\n');
    }
    rendered.push('\n');
    rendered
}

fn rustc_span(finding: &Finding, selection: &Selection, absolute_path: &Path) -> RustcSpan {
    let byte_start =
        byte_offset_for_position(absolute_path, finding.line, finding.column).unwrap_or_default();
    let byte_end = byte_offset_for_position(
        absolute_path,
        finding.line,
        finding.column + finding.highlight_len,
    )
    .unwrap_or(byte_start + finding.highlight_len);
    let column_end = finding.column + finding.highlight_len;
    RustcSpan {
        byte_end,
        byte_start,
        column_end,
        column_start: finding.column,
        expansion: None,
        file_name: path_for_display(finding, selection),
        is_primary: true,
        label: finding.item.clone(),
        line_end: finding.line,
        line_start: finding.line,
        suggested_replacement: None,
        suggestion_applicability: None,
        text: vec![RustcSpanText {
            highlight_end:   column_end,
            highlight_start: finding.column,
            text:            finding.source_line.clone(),
        }],
    }
}

fn path_for_display(finding: &Finding, selection: &Selection) -> String {
    let path = Path::new(&finding.path);
    if path.is_absolute() {
        path.strip_prefix(selection.analysis_root.as_path())
            .map_or_else(|_| finding.path.clone(), normalize_path)
    } else {
        finding.path.clone()
    }
}

fn byte_offset_for_position(path: &Path, line: usize, column: usize) -> Option<usize> {
    let text = fs::read_to_string(path).ok()?;
    let mut offset = 0;
    for (index, source_line) in text.lines().enumerate() {
        if index + 1 == line {
            let column_offset = source_line
                .char_indices()
                .nth(column.saturating_sub(1))
                .map_or(source_line.len(), |(byte, _)| byte);
            return Some(offset + column_offset);
        }
        offset += source_line.len() + 1;
    }
    None
}

fn absolute_finding_path(finding: &Finding, selection: &Selection) -> PathBuf {
    let path = Path::new(&finding.path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        selection.analysis_root.join(path)
    }
}

fn package_for_path<'a>(selection: &'a Selection, path: &Path) -> Option<&'a PackageMetadata> {
    selection
        .packages
        .iter()
        .filter(|package| path.starts_with(package.root.as_path()))
        .max_by_key(|package| package.root.components().count())
        .or_else(|| selection.packages.first())
}

fn target_for_path<'a>(package: &'a PackageMetadata, path: &Path) -> Option<&'a TargetMetadata> {
    package
        .targets
        .iter()
        .find(|target| target.src_path == path)
        .or_else(|| preferred_package_target(package, path))
        .or_else(|| package.targets.first())
}

fn preferred_package_target<'a>(
    package: &'a PackageMetadata,
    path: &Path,
) -> Option<&'a TargetMetadata> {
    let relative = path.strip_prefix(package.root.as_path()).ok()?;
    if relative.starts_with(SOURCE_DIR_SRC) {
        if let Some(target) = package.targets.iter().find(|target| {
            target.kind.iter().any(|kind| kind == CARGO_TARGET_KIND_LIB)
                && target.src_path.file_name().and_then(|name| name.to_str()) == Some("lib.rs")
                && target
                    .src_path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    == Some(SOURCE_DIR_SRC)
        }) {
            return Some(target);
        }
        return package.targets.iter().find(|target| {
            target.kind.iter().any(|kind| kind == CARGO_TARGET_KIND_BIN)
                && target.src_path.file_name().and_then(|name| name.to_str()) == Some("main.rs")
                && target
                    .src_path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    == Some(SOURCE_DIR_SRC)
        });
    }

    package.targets.iter().find(|target| {
        target
            .src_path
            .parent()
            .is_some_and(|parent| path.starts_with(parent))
    })
}

fn cargo_target(target: &TargetMetadata) -> CargoTarget {
    CargoTarget {
        kind:              target.kind.clone(),
        crate_types:       target.crate_types.clone(),
        name:              target.name.clone(),
        src_path:          target.src_path.display().to_string(),
        edition:           target.edition.clone(),
        required_features: target.required_features.clone(),
        doc:               target.doc,
        doctest:           target.doctest,
        test:              target.test,
    }
}

const fn severity_level(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn normalize_path(path: &Path) -> String { path.to_string_lossy().replace('\\', "/") }
