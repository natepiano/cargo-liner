use std::io::BufRead;
use std::io::BufReader;
use std::process::ChildStderr;

use anyhow::Result;

use super::BuildOutputMode;
use super::progress::CargoProgress;
use super::progress::ProgressDisplay;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_BLOCKING;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_BUILDING;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_CHECKING;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_COMPILING;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_FINISHED;
use crate::compiler::constants::CARGO_PROGRESS_PREFIX_FRESH;
use crate::compiler::constants::CARGO_UNUSED_IMPORT_WARNING;
use crate::compiler::constants::CARGO_UNUSED_IMPORTS_WARNING;
use crate::compiler::constants::CARGO_WARNING_SUMMARY_PREFIX;
use crate::compiler::constants::CARGO_WARNING_SUMMARY_TOKEN_GENERATED;
use crate::compiler::constants::CARGO_WARNING_SUMMARY_TOKEN_TO_APPLY;
use crate::compiler::constants::DIAGNOSTIC_SEVERITY_ERROR_PREFIX;
use crate::compiler::constants::DIAGNOSTIC_SEVERITY_WARNING_PREFIX;
use crate::reporting::CompilerWarningFacts;

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct StderrObservation {
    pub(super) compiler_warning_facts: CompilerWarningFacts,
    pub(super) warning_count:          usize,
    pub(super) fixable_count:          usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiagnosticBlockKind {
    SuppressedUnusedImport,
    CompilerWarningSummary {
        warning_count: usize,
        fixable_count: usize,
    },
    Forwarded,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum SuppressionNotice {
    #[default]
    Pending,
    Printed,
}

pub(super) fn stream_cargo_stderr(
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ProgressStatus {
    Active,
    #[default]
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

fn is_progress_line(line: &str) -> bool {
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

fn classify_diagnostic_block(block: &[String]) -> DiagnosticBlockKind {
    let first_non_empty = block.iter().find(|line| !line.trim().is_empty());
    first_non_empty.map_or(DiagnosticBlockKind::Forwarded, |line| {
        let sanitized = sanitize_for_match(line);
        let trimmed = sanitized.trim_start();

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
        },
        DiagnosticBlockKind::Forwarded => {
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

#[cfg(test)]
mod tests {
    use super::DiagnosticBlockKind;
    use super::ProgressStatus;
    use super::SuppressionNotice;
    use super::classify_diagnostic_block;
    use super::flush_diagnostic_block;
    use super::is_progress_line;
    use super::should_forward_progress_line;
    use crate::compiler::build::BuildOutputMode;
    use crate::compiler::build::progress::ProgressDisplay;
    use crate::reporting::CompilerWarningFacts;

    #[derive(Default)]
    struct ProgressRecorder {
        progress_status: ProgressStatus,
        notices:         Vec<String>,
        stops:           usize,
    }

    impl ProgressRecorder {
        const fn active() -> Self {
            Self {
                progress_status: ProgressStatus::Active,
                notices:         Vec::new(),
                stops:           0,
            }
        }
    }

    impl ProgressDisplay for ProgressRecorder {
        fn is_active(&self) -> bool { matches!(self.progress_status, ProgressStatus::Active) }

        fn write_status_notice(&mut self, notice: &str) { self.notices.push(notice.to_string()); }

        fn stop_for_forwarded_output(&mut self) {
            self.stops += 1;
            self.progress_status = ProgressStatus::Inactive;
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
        assert_eq!(progress.progress_status, ProgressStatus::Inactive);
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
        assert_eq!(progress.progress_status, ProgressStatus::Active);
    }
}
