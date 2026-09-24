use std::io;
use std::io::BufRead;
use std::io::BufReader;
use std::io::ErrorKind;
use std::io::Read;
use std::mem;
use std::process::ChildStderr;

use anyhow::Result;

use super::BuildOutputMode;
use super::progress::CargoProgress;
use super::progress::ProgressDisplay;
use super::progress::UnitCounter;
use crate::compiler::constants::CARGO_PROGRESS_BAR_CLOSE;
use crate::compiler::constants::CARGO_PROGRESS_COUNTER_SEPARATOR;
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ProgressStatus {
    Active,
    #[default]
    Inactive,
}

impl From<bool> for ProgressStatus {
    fn from(value: bool) -> Self { if value { Self::Active } else { Self::Inactive } }
}

/// Cargo's stderr split into lines the way `BufRead::read_line` splits it, with
/// the counter out of each progress-bar frame handed on the moment it lands.
///
/// Cargo ends a bar frame with `\r` and no newline, then draws the next frame
/// over it, so a build that prints nothing else holds one unfinished line for as
/// long as it runs. Reading the counter only once that line ends would hold the
/// status line at wherever cargo stood the last time it printed a status.
struct StderrReader<R> {
    reader:  BufReader<R>,
    /// Bytes read but not yet returned as a line.
    pending: Vec<u8>,
}

impl<R: Read> From<R> for StderrReader<R> {
    fn from(stderr: R) -> Self {
        Self {
            reader:  BufReader::new(stderr),
            pending: Vec::new(),
        }
    }
}

impl<R: Read> StderrReader<R> {
    /// The next line, ending in `\n` unless the stream ended first, or `None`
    /// once the stream is spent. Bytes that are not UTF-8 are replaced rather
    /// than failing the run.
    fn next_line(&mut self, progress: &mut impl ProgressDisplay) -> io::Result<Option<String>> {
        loop {
            if let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
                let line: Vec<u8> = self.pending.drain(..=end).collect();
                return Ok(Some(String::from_utf8_lossy(&line).into_owned()));
            }
            let chunk = match self.reader.fill_buf() {
                Ok(chunk) => chunk,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            if chunk.is_empty() {
                let rest = mem::take(&mut self.pending);
                return Ok((!rest.is_empty()).then(|| String::from_utf8_lossy(&rest).into_owned()));
            }
            let read = chunk.len();
            self.pending.extend_from_slice(chunk);
            self.reader.consume(read);
            if let Some(unit_counter) = latest_unit_counter(&self.pending) {
                progress.record_unit_counter(unit_counter);
            }
        }
    }
}

pub(super) fn stream_cargo_stderr(
    stderr: ChildStderr,
    output_mode: BuildOutputMode,
    mut progress: CargoProgress,
) -> Result<StderrObservation> {
    let mut reader = StderrReader::from(stderr);
    let mut block = Vec::new();
    let mut suppression_notice = SuppressionNotice::Pending;
    let mut compiler_warning_facts = CompilerWarningFacts::None;
    let mut compiler_warning_count: usize = 0;
    let mut compiler_fixable_count: usize = 0;

    loop {
        let Some(line) = reader.next_line(&mut progress)? else {
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
        };

        let current = collapse_carriage_returns(&line);
        // Cargo erases the progress frame it just drew and writes the next
        // diagnostic onto the same physical line, so one read can carry both.
        // Once the carriage returns are applied a frame may leave nothing
        // behind, which is a spent frame rather than the blank line that ends a
        // diagnostic block.
        if is_blank_after_sanitize(&current) && !is_blank_after_sanitize(&line) {
            continue;
        }

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
                // Through the progress display rather than straight to stderr:
                // when the spinner is up it owns the last line, and a bare
                // `eprint!` would interleave with the frame it is redrawing.
                progress.write_status_notice(current.trim_end());
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

fn should_forward_progress_line(
    line: &str,
    output_mode: BuildOutputMode,
    progress_status: ProgressStatus,
) -> bool {
    if is_finished_line(line) {
        return false;
    }
    match output_mode {
        BuildOutputMode::Json | BuildOutputMode::Quiet => false,
        // The main pass shows cargo's status lines even while the spinner runs:
        // there the spinner reports which target mend is analyzing, which is
        // additional to cargo's output rather than a replacement for it.
        BuildOutputMode::Full => true,
        // The fix-candidate pass replaces cargo's output with the spinner, so a
        // forwarded line would duplicate what the spinner already says.
        BuildOutputMode::SuppressUnusedImportWarnings => {
            matches!(progress_status, ProgressStatus::Inactive)
        },
    }
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

/// The counter in the newest complete bar frame in `pending`: text a `\r` has
/// ended, since the text after the last `\r` may still be arriving.
fn latest_unit_counter(pending: &[u8]) -> Option<UnitCounter> {
    let frames_end = pending.iter().rposition(|&byte| byte == b'\r')?;
    String::from_utf8_lossy(pending.get(..frames_end)?)
        .rsplit(['\r', '\n'])
        .find_map(parse_unit_counter)
}

/// Cargo's unit counter out of one progress-bar frame,
/// `    Building [=======>        ] 149/403: globset, regex-automata`.
fn parse_unit_counter(frame: &str) -> Option<UnitCounter> {
    let sanitized = sanitize_for_match(frame);
    let bar = sanitized
        .trim_start()
        .strip_prefix(CARGO_PROGRESS_PREFIX_BUILDING)?;
    let (_, counter) = bar.split_once(CARGO_PROGRESS_BAR_CLOSE)?;
    let (done, rest) = counter.split_once(CARGO_PROGRESS_COUNTER_SEPARATOR)?;
    let total = rest
        .split(|character: char| !character.is_ascii_digit())
        .next()?;
    let unit_counter = UnitCounter {
        done:  done.parse().ok()?,
        total: total.parse().ok()?,
    };
    (unit_counter.total > 0).then_some(unit_counter)
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

/// Apply carriage returns the way a terminal does: `\r` returns the cursor to
/// the start of the line, so only the text written after the last one is
/// visible. A trailing `\r` belongs to a line ending and is dropped instead.
fn collapse_carriage_returns(line: &str) -> String {
    let (content, newline) = line
        .strip_suffix('\n')
        .map_or((line, ""), |rest| (rest, "\n"));
    let content = content.strip_suffix('\r').unwrap_or(content);
    let visible = content.rsplit('\r').next().unwrap_or(content);
    format!("{visible}{newline}")
}

fn is_blank_after_sanitize(line: &str) -> bool { sanitize_for_match(line).trim().is_empty() }

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
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::DiagnosticBlockKind;
    use super::ProgressStatus;
    use super::StderrReader;
    use super::SuppressionNotice;
    use super::classify_diagnostic_block;
    use super::collapse_carriage_returns;
    use super::flush_diagnostic_block;
    use super::is_progress_line;
    use super::parse_unit_counter;
    use super::should_forward_progress_line;
    use crate::compiler::build::BuildOutputMode;
    use crate::compiler::build::progress::ProgressDisplay;
    use crate::compiler::build::progress::UnitCounter;
    use crate::reporting::CompilerWarningFacts;

    const CAPTURED_FRAME: &str = "\u{1b}[1m\u{1b}[96m    Building\u{1b}[0m \
                                  [========>                ] 149/403: globset, regex-automata\r";

    #[derive(Default)]
    struct ProgressRecorder {
        progress_status: ProgressStatus,
        notices:         Vec<String>,
        stops:           usize,
        unit_counters:   Vec<UnitCounter>,
    }

    impl ProgressRecorder {
        const fn active() -> Self {
            Self {
                progress_status: ProgressStatus::Active,
                notices:         Vec::new(),
                stops:           0,
                unit_counters:   Vec::new(),
            }
        }
    }

    impl ProgressDisplay for ProgressRecorder {
        fn is_active(&self) -> bool { matches!(self.progress_status, ProgressStatus::Active) }

        fn record_unit_counter(&mut self, unit_counter: UnitCounter) {
            self.unit_counters.push(unit_counter);
        }

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

    #[test]
    fn collapse_carriage_returns_keeps_only_the_visible_tail() {
        assert_eq!(collapse_carriage_returns("frame\rreal\n"), "real\n");
        assert_eq!(collapse_carriage_returns("one\rtwo\rthree"), "three");
        assert_eq!(collapse_carriage_returns("plain\n"), "plain\n");
        assert_eq!(collapse_carriage_returns("windows\r\n"), "windows\n");
    }

    #[test]
    fn warning_summary_glued_to_a_progress_frame_is_counted_not_forwarded() {
        let frame = "\u{1b}[1m\u{1b}[96m    Building\u{1b}[0m [    ] 0/2: fixture\r\u{1b}[K";
        let summary = "\u{1b}[1m\u{1b}[33mwarning\u{1b}[0m: `fixture` (bin \"fixture\") generated 2 \
                       warnings (run `cargo fix --bin \"fixture\"` to apply 1 suggestion)\n";
        let mut block = vec![collapse_carriage_returns(&format!("{frame}{summary}"))];
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

        assert_eq!(compiler_warning_count, 2);
        assert_eq!(compiler_fixable_count, 1);
        assert!(progress.notices.is_empty());
    }

    #[test]
    fn a_captured_bar_frame_yields_cargos_counter() {
        assert_eq!(
            parse_unit_counter(CAPTURED_FRAME),
            Some(UnitCounter {
                done:  149,
                total: 403,
            })
        );
    }

    #[test]
    fn only_a_building_frame_over_a_nonzero_total_is_a_counter() {
        assert_eq!(parse_unit_counter("    Checking fixture v0.1.0\n"), None);
        assert_eq!(
            parse_unit_counter("    Downloading [=====>   ] 3/10: serde\r"),
            None
        );
        assert_eq!(parse_unit_counter("    Building [   ] 0/0: fixture"), None);
    }

    #[test]
    fn reader_reports_each_frame_and_splits_lines_as_read_line_does() {
        let second_frame = CAPTURED_FRAME.replace("149/403", "150/403");
        let stderr = format!(
            "    Checking fixture v0.1.0\n{CAPTURED_FRAME}{second_frame}\u{1b}[Kwarning: unused\n\
             trailing"
        );
        let mut reader = StderrReader::from(stderr.as_bytes());
        let mut progress = ProgressRecorder::default();

        let mut lines = Vec::new();
        while let Some(line) = reader.next_line(&mut progress).expect("read stderr") {
            lines.push(line);
        }

        assert_eq!(
            lines,
            vec![
                "    Checking fixture v0.1.0\n".to_string(),
                format!("{CAPTURED_FRAME}{second_frame}\u{1b}[Kwarning: unused\n"),
                "trailing".to_string(),
            ]
        );
        assert_eq!(
            progress.unit_counters.last(),
            Some(&UnitCounter {
                done:  150,
                total: 403,
            })
        );
    }
}
