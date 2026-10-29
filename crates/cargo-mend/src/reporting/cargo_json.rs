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

/// Source coordinates remain available even when the file cannot be resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceCoordinates {
    line:          usize,
    column:        usize,
    highlight_len: usize,
}

/// Only checked, ordered byte endpoints may describe a resolved span. Missing
/// source must not masquerade as byte zero. A highlight can span several lines
/// or count Unicode bytes rather than characters; keep available line endpoints
/// so Cargo can apply its legacy clipping policy at serialization instead.
#[derive(Debug, PartialEq, Eq)]
enum SpanResolution {
    Resolved(ResolvedSpan),
    ClippedToLine(ClippedLineSpan),
    Unavailable(SourceCoordinates),
}

/// The line is known, but a requested endpoint is past it. Exact byte positions
/// remain checked; the encoder decides how a past-line endpoint is represented.
#[derive(Debug, PartialEq, Eq)]
struct ClippedLineSpan {
    coordinates: SourceCoordinates,
    start:       LineEndpoint,
    end:         LineEndpoint,
    line_end:    usize,
}

#[derive(Debug, PartialEq, Eq)]
enum LineEndpoint {
    Exact(usize),
    PastLineEnd,
}

impl LineEndpoint {
    const fn from_offset(offset: Option<usize>, line_start: usize) -> Self {
        match offset {
            Some(byte) => Self::Exact(line_start + byte),
            None => Self::PastLineEnd,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedSpan {
    coordinates: SourceCoordinates,
    byte_start:  usize,
    byte_end:    usize,
}

impl SpanResolution {
    fn for_path(path: &Path, coordinates: SourceCoordinates) -> Self {
        fs::read_to_string(path).map_or(Self::Unavailable(coordinates), |text| {
            Self::in_source(&text, coordinates)
        })
    }

    fn in_source(text: &str, coordinates: SourceCoordinates) -> Self {
        let Some(start_column) = coordinates.column.checked_sub(1) else {
            return Self::Unavailable(coordinates);
        };
        let Some(end_column) = start_column.checked_add(coordinates.highlight_len) else {
            return Self::Unavailable(coordinates);
        };
        let mut offset = 0;
        for (index, source_line) in text.lines().enumerate() {
            if index + 1 == coordinates.line {
                let column_offset = |column| {
                    source_line
                        .char_indices()
                        .map(|(byte, _)| byte)
                        .chain(std::iter::once(source_line.len()))
                        .nth(column)
                };
                return match (column_offset(start_column), column_offset(end_column)) {
                    (Some(start), Some(end)) => Self::Resolved(ResolvedSpan {
                        coordinates,
                        byte_start: offset + start,
                        byte_end: offset + end,
                    }),
                    (start @ None, end @ (Some(_) | None)) | (start @ Some(_), end @ None) => {
                        Self::ClippedToLine(ClippedLineSpan {
                            coordinates,
                            start: LineEndpoint::from_offset(start, offset),
                            end: LineEndpoint::from_offset(end, offset),
                            line_end: offset + source_line.len(),
                        })
                    },
                };
            }
            offset += source_line.len() + 1;
        }
        Self::Unavailable(coordinates)
    }
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

enum FindingAnnotation<'a> {
    Note(&'a str),
    InlineHelp(&'a str),
    FixabilityHelp(&'static str),
}

struct FindingAnnotations<'a>(Vec<FindingAnnotation<'a>>);

impl<'a> From<&'a Finding> for FindingAnnotations<'a> {
    fn from(finding: &'a Finding) -> Self {
        let mut entries = Vec::new();
        if let Some(message) = diagnostics::finding_message_not_in_headline(finding) {
            entries.push(FindingAnnotation::Note(message));
        }
        if let Some(related) = &finding.related {
            entries.push(FindingAnnotation::Note(related));
        }
        if let Some(help) = diagnostics::resolved_inline_help_text(finding) {
            entries.push(FindingAnnotation::InlineHelp(help));
        }
        if let Some(note) = diagnostics::fixability_note(finding) {
            entries.push(FindingAnnotation::FixabilityHelp(note));
        }
        Self(entries)
    }
}

enum FindingPath<'a> {
    Absolute(&'a Path),
    Relative(&'a Path),
}

impl<'a> From<&'a str> for FindingPath<'a> {
    fn from(spelling: &'a str) -> Self {
        let path = Path::new(spelling);
        if path.is_absolute() {
            Self::Absolute(path)
        } else {
            Self::Relative(path)
        }
    }
}

impl FindingPath<'_> {
    fn absolute(&self, analysis_root: &Path) -> PathBuf {
        match self {
            Self::Absolute(path) => path.to_path_buf(),
            Self::Relative(path) => analysis_root.join(path),
        }
    }

    fn display(&self, spelling: &str, analysis_root: &Path) -> String {
        match self {
            Self::Absolute(path) => path
                .strip_prefix(analysis_root)
                .map_or_else(|_| spelling.to_string(), normalize_path),
            Self::Relative(_) => spelling.to_string(),
        }
    }
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
    let path = FindingPath::from(finding.path.as_str());
    let absolute_path = path.absolute(&selection.analysis_root);
    let package = package_for_path(selection, &absolute_path)
        .context("selection did not include packages")?;
    let target =
        target_for_path(package, &absolute_path).context("package did not include targets")?;
    let display_path = path.display(&finding.path, &selection.analysis_root);
    let span = rustc_span(finding, display_path, &absolute_path);
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
    let annotations = FindingAnnotations::from(finding);
    let children = annotations
        .0
        .iter()
        .map(|annotation| match annotation {
            FindingAnnotation::Note(text) => {
                child(RUSTC_LEVEL_NOTE, (*text).to_string(), Vec::new())
            },
            FindingAnnotation::InlineHelp(text) => {
                child(RUSTC_LEVEL_HELP, (*text).to_string(), vec![span.clone()])
            },
            FindingAnnotation::FixabilityHelp(text) => {
                child(RUSTC_LEVEL_HELP, (*text).to_string(), Vec::new())
            },
        })
        .collect();

    let level = severity_level(finding.severity());
    RustcDiagnostic {
        rendered: render_diagnostic(finding, &span, level, &annotations),
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

fn render_diagnostic(
    finding: &Finding,
    span: &RustcSpan,
    level: &str,
    annotations: &FindingAnnotations<'_>,
) -> String {
    let line_label = finding.line.to_string();
    let gutter_width = line_label.len();
    let gutter_pad = " ".repeat(gutter_width + 1);
    let marker_pad = " ".repeat(span.column_start.saturating_sub(1));
    let marker_len = span.column_end.saturating_sub(span.column_start).max(1);
    let marker = "^".repeat(marker_len);
    let inline_help = annotations
        .0
        .iter()
        .filter_map(|annotation| match annotation {
            FindingAnnotation::InlineHelp(text) => Some(format!(" help: {text}")),
            FindingAnnotation::Note(_) | FindingAnnotation::FixabilityHelp(_) => None,
        })
        .collect::<String>();
    let mut rendered = format!(
        concat!(
            "{level}: {}\n --> {}:{}:{}\n{gutter_pad}|\n{line_label} | {}\n",
            "{gutter_pad}| {marker_pad}{marker}{inline_help}\n",
        ),
        diagnostics::finding_headline(finding),
        span.file_name,
        finding.line,
        finding.column,
        finding.source_line,
        gutter_pad = gutter_pad,
        line_label = line_label,
        marker_pad = marker_pad,
        marker = marker,
        inline_help = inline_help,
        level = level,
    );
    for annotation in &annotations.0 {
        let (label, text) = match annotation {
            FindingAnnotation::Note(text) => ("  = note: ", *text),
            FindingAnnotation::FixabilityHelp(text) => ("  = help: ", *text),
            FindingAnnotation::InlineHelp(_) => continue,
        };
        rendered.push_str(label);
        rendered.push_str(text);
        rendered.push('\n');
    }
    rendered.push('\n');
    rendered
}

fn rustc_span(finding: &Finding, display_path: String, absolute_path: &Path) -> RustcSpan {
    let coordinates = SourceCoordinates {
        line:          finding.line,
        column:        finding.column,
        highlight_len: finding.highlight_len,
    };
    // Cargo requires byte fields even when the source is unavailable. Keep its
    // legacy fallback here, at the serialized-span boundary, not in resolution.
    let (coordinates, byte_start, byte_end) =
        match SpanResolution::for_path(absolute_path, coordinates) {
            SpanResolution::Resolved(span) => (span.coordinates, span.byte_start, span.byte_end),
            SpanResolution::ClippedToLine(span) => {
                let clip = |endpoint| match endpoint {
                    LineEndpoint::Exact(byte) => byte,
                    LineEndpoint::PastLineEnd => span.line_end,
                };
                (span.coordinates, clip(span.start), clip(span.end))
            },
            SpanResolution::Unavailable(coordinates) => (coordinates, 0, coordinates.highlight_len),
        };
    let column_end = coordinates.column + coordinates.highlight_len;
    RustcSpan {
        byte_end,
        byte_start,
        column_end,
        column_start: coordinates.column,
        expansion: None,
        file_name: display_path,
        is_primary: true,
        label: finding.item.clone(),
        line_end: coordinates.line,
        line_start: coordinates.line,
        suggested_replacement: None,
        suggestion_applicability: None,
        text: vec![RustcSpanText {
            highlight_end:   column_end,
            highlight_start: coordinates.column,
            text:            finding.source_line.clone(),
        }],
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::FindingPath;
    use super::rustc_diagnostic;
    use super::rustc_span;
    use crate::config::DiagnosticCode;
    use crate::reporting::constants::HINT_WARNING_FIXABLE_WITH_FIX;
    use crate::reporting::diagnostics::Finding;
    use crate::reporting::diagnostics::FixSupport;
    use crate::reporting::diagnostics::ItemVisibility;
    use crate::reporting::diagnostics::Severity;

    #[test]
    fn span_resolution_checks_both_endpoints_and_preserves_unicode_offsets() {
        let text = "first\nαbeta\n";
        let coordinates = super::SourceCoordinates {
            line:          2,
            column:        2,
            highlight_len: 3,
        };
        assert_eq!(
            super::SpanResolution::in_source(text, coordinates),
            super::SpanResolution::Resolved(super::ResolvedSpan {
                coordinates,
                byte_start: 8,
                byte_end: 11,
            })
        );
        for (line, column, highlight_len) in [(0, 1, 1), (9, 1, 1), (1, 0, 1)] {
            let coordinates = super::SourceCoordinates {
                line,
                column,
                highlight_len,
            };
            assert_eq!(
                super::SpanResolution::in_source(text, coordinates),
                super::SpanResolution::Unavailable(coordinates)
            );
        }
        let coordinates = super::SourceCoordinates {
            line:          1,
            column:        1,
            highlight_len: 4,
        };
        assert_eq!(
            super::SpanResolution::for_path(Path::new(""), coordinates),
            super::SpanResolution::Unavailable(coordinates)
        );
    }

    #[test]
    fn cargo_spans_clip_available_lines_without_losing_the_real_start() -> anyhow::Result<()> {
        let cases = [
            (
                "fn main() {\n    use std::{\n        fmt,\n    };\n}\n",
                "    use std::{",
                5,
                30,
                16,
                26,
            ),
            (
                "fn main() {\n    use crate::αβ;\n}\n",
                "    use crate::αβ;",
                9,
                "crate::αβ".len(),
                20,
                32,
            ),
        ];
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("lib.rs");
        for (text, source_line, column, highlight_len, byte_start, byte_end) in cases {
            std::fs::write(&path, text)?;
            let finding: Finding = serde_json::from_value(serde_json::json!({
                "severity": "warning", "diagnostic_code": "imports_at_top",
                "fixability": "fix_imports_at_top", "path": "lib.rs", "line": 2,
                "column": column, "highlight_len": highlight_len, "source_line": source_line,
                "message": "message",
            }))?;
            let span = rustc_span(&finding, "lib.rs".to_string(), &path);
            let column_end = column + highlight_len;
            assert_eq!(finding.highlight_len, highlight_len);
            assert_eq!(
                serde_json::to_string(&span)?,
                format!(
                    concat!(
                        "{{\"byte_end\":{},\"byte_start\":{},\"column_end\":{},",
                        "\"column_start\":{},\"expansion\":null,\"file_name\":\"lib.rs\",",
                        "\"is_primary\":true,\"label\":null,\"line_end\":2,\"line_start\":2,",
                        "\"suggested_replacement\":null,\"suggestion_applicability\":null,",
                        "\"text\":[{{\"highlight_end\":{},\"highlight_start\":{},",
                        "\"text\":\"{}\"}}]}}"
                    ),
                    byte_end, byte_start, column_end, column, column_end, column, source_line
                )
            );
        }
        Ok(())
    }

    #[test]
    fn clipped_coordinates_keep_line_evidence_for_both_endpoints() {
        let coordinates = super::SourceCoordinates {
            line:          2,
            column:        7,
            highlight_len: 0,
        };
        assert_eq!(
            super::SpanResolution::in_source("first\nαbeta\n", coordinates),
            super::SpanResolution::ClippedToLine(super::ClippedLineSpan {
                coordinates,
                start: super::LineEndpoint::PastLineEnd,
                end: super::LineEndpoint::PastLineEnd,
                line_end: 12,
            })
        );
    }

    #[test]
    fn finding_paths_preserve_relative_spelling_and_absolute_root_stripping() {
        let root = Path::new("/repo");
        for (spelling, absolute, display) in [
            ("src/./lib.rs", "/repo/src/./lib.rs", "src/./lib.rs"),
            ("/repo/src/lib.rs", "/repo/src/lib.rs", "src/lib.rs"),
            (
                "/elsewhere/lib.rs",
                "/elsewhere/lib.rs",
                "/elsewhere/lib.rs",
            ),
        ] {
            let path = FindingPath::from(spelling);
            assert_eq!(path.absolute(root), Path::new(absolute));
            assert_eq!(path.display(spelling, root), display);
        }
    }

    #[test]
    fn shared_annotations_keep_child_order_spans_and_rendered_bytes() {
        let finding = Finding {
            disposition:     Finding::disposition(
                DiagnosticCode::InlinePathQualifiedType,
                Severity::Error,
                FixSupport::InlinePathQualifiedType,
            ),
            diagnostic_code: DiagnosticCode::InlinePathQualifiedType,
            path:            "src/lib.rs".to_string(),
            line:            1,
            column:          1,
            highlight_len:   4,
            source_line:     "type".to_string(),
            item:            None,
            message:         "message".to_string(),
            suggestion:      Some("replace".to_string()),
            related:         Some("related".to_string()),
            item_visibility: ItemVisibility::default(),
        };
        let span = rustc_span(&finding, finding.path.clone(), Path::new(""));
        assert_eq!((span.byte_start, span.byte_end), (0, 4));
        let diagnostic = rustc_diagnostic(&finding, span);
        let children = diagnostic
            .children
            .iter()
            .map(|child| (child.level, child.message.as_str(), child.spans.len()))
            .collect::<Vec<_>>();
        assert_eq!(
            children,
            vec![
                ("note", "message", 0),
                ("note", "related", 0),
                ("help", "replace", 1),
                ("help", HINT_WARNING_FIXABLE_WITH_FIX, 0),
            ]
        );
        assert_eq!(
            diagnostic.rendered,
            format!(
                concat!(
                    "warning: inline path-qualified type should use a `use` import\n",
                    " --> src/lib.rs:1:1\n  |\n1 | type\n  | ^^^^ help: replace\n",
                    "  = note: message\n  = note: related\n  = help: {}\n\n",
                ),
                HINT_WARNING_FIXABLE_WITH_FIX
            )
        );
    }
}
