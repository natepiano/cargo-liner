//! Wrapping a styled line to the width of the column it is drawn in.
//!
//! Wrapping in ratatui belongs to [`ratatui::widgets::Paragraph`], and a
//! [`ratatui::widgets::Table`] cell draws the [`Text`] it is handed as
//! it stands. Callers use this wrapper when a cell or other region
//! needs already-wrapped text.

use std::mem::take;

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;

use crate::constants::ELISION;
use crate::constants::WRAP_BREAK_AFTER;
use crate::constants::WRAP_HYPHEN;
use crate::toasts::RunBreaks;
use crate::toasts::ToastRun;

/// One word or indivisible run being wrapped, carrying its drawing style.
struct Word {
    /// Whitespace that precedes the word.
    separator: WordSeparator,
    /// Contiguous styled text, with each run's permitted breaks.
    runs:      Vec<ToastRun>,
}

/// Whitespace before a word and whether wrapping may consume it.
enum WordSeparator {
    /// The word directly follows the preceding text.
    None,
    /// Leading indentation, which is part of the line's visible contents.
    Indentation(String, Style),
    /// Whitespace between words, which disappears when it becomes a break.
    BreakPoint(String, Style),
}

impl WordSeparator {
    /// Cells this separator occupies when it is drawn on the current line.
    fn width(&self) -> usize {
        match self {
            Self::None => 0,
            Self::Indentation(text, _) | Self::BreakPoint(text, _) => cell_count(text),
        }
    }

    /// Whether this separator may be consumed to start the word on a new line.
    const fn is_break_point(&self) -> bool { matches!(self, Self::BreakPoint(..)) }
}

/// How input whitespace appears when no wrap consumes it.
#[derive(Clone, Copy)]
enum WhitespaceHandling {
    /// Replace every run between words with one space.
    Collapse,
    /// Draw leading indentation and runs between words exactly as supplied.
    Preserve,
}

/// The lines already broken off and the one still being filled.
struct BoundaryWrap {
    /// Cells one line holds.
    width: usize,
    /// Lines the wrap has finished.
    lines: Vec<Line<'static>>,
    /// Spans of the line being filled.
    spans: Vec<Span<'static>>,
    /// Cells those spans occupy.
    used:  usize,
}

impl BoundaryWrap {
    /// An empty wrap onto lines of `width` cells.
    const fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            spans: Vec::new(),
            used: 0,
        }
    }

    /// Cells the line being filled has left after `separator` is drawn.
    fn room(&self, separator: &WordSeparator) -> usize {
        self.width
            .saturating_sub(self.used)
            .saturating_sub(separator.width())
    }

    /// Break the line being filled and start the next one.
    fn wrap(&mut self) {
        self.lines.push(Line::from(take(&mut self.spans)));
        self.used = 0;
    }

    /// Put `word` on the line being filled.
    ///
    /// A word that fits a fresh line moves there whole. Longer words use
    /// a punctuation boundary or the width; indivisible runs carry a cut mark.
    fn push(&mut self, word: &Word) {
        let cells = word
            .runs
            .iter()
            .map(|run| cell_count(&run.text))
            .sum::<usize>();
        let mut separator = &word.separator;
        if cells <= self.width && cells > self.room(separator) && self.used > 0 {
            self.wrap();
            separator = &WordSeparator::None;
        }
        if cells <= self.room(separator) {
            self.write_runs(separator, &word.runs, 0, usize::MAX);
            return;
        }
        let mut index = 0;
        while index < word.runs.len() {
            let run = &word.runs[index];
            if matches!(run.breaks, RunBreaks::AtSpaces) {
                let end = word.runs[index..]
                    .iter()
                    .position(|run| matches!(run.breaks, RunBreaks::Never))
                    .map_or(word.runs.len(), |offset| index + offset);
                self.push_breakable(&word.runs[index..end], separator);
                index = end;
            } else {
                let suffix = word.runs[index + 1..]
                    .iter()
                    .take_while(|run| matches!(run.breaks, RunBreaks::AtSpaces))
                    .map(|run| cell_count(&run.text))
                    .sum::<usize>();
                let reserved = if suffix < self.width { suffix } else { 0 };
                if self.used > 0 && cell_count(&run.text) + reserved > self.room(separator) {
                    self.wrap();
                    separator = &WordSeparator::None;
                }
                let room = self.room(separator).saturating_sub(reserved);
                if cell_count(&run.text) <= room {
                    self.write(separator, &run.text, run.style);
                } else {
                    let (head, _) =
                        split_at_cells(&run.text, room.saturating_sub(cell_count(ELISION)));
                    self.write(separator, &format!("{head}{ELISION}"), run.style);
                }
                index += 1;
            }
            separator = &WordSeparator::None;
        }
    }

    /// Wrap breakable text while retaining styles across cuts.
    fn push_breakable(&mut self, runs: &[ToastRun], mut separator: &WordSeparator) {
        let text = runs.iter().map(|run| run.text.as_str()).collect::<String>();
        let mut rest = text.as_str();
        let mut offset = 0;
        while !rest.is_empty() {
            let room = self.room(separator);
            if cell_count(rest) <= room {
                self.write_runs(separator, runs, offset, rest.len());
                return;
            }
            if self.used > 0 && cell_count(rest) <= self.width {
                self.wrap();
                separator = &WordSeparator::None;
                continue;
            }
            if room == 0 && (self.used > 0 || separator.is_break_point()) {
                self.wrap();
                separator = &WordSeparator::None;
                continue;
            }
            let candidate = split_at_cells(rest, room).0;
            let hyphen = candidate
                .char_indices()
                .rev()
                .filter(|(_, character)| *character == WRAP_HYPHEN)
                .map(|(index, character)| index.saturating_add(character.len_utf8()))
                .find(|&index| rest[..index].chars().any(char::is_alphanumeric));
            let boundary = hyphen.or_else(|| {
                candidate
                    .char_indices()
                    .rev()
                    .find(|(_, character)| WRAP_BREAK_AFTER.contains(*character))
                    .map(|(index, character)| index.saturating_add(character.len_utf8()))
                    .filter(|&index| cell_count(&rest[..index]) >= self.width.saturating_add(1) / 2)
            });
            let Some(boundary) = boundary else {
                if separator.is_break_point() {
                    self.wrap();
                    separator = &WordSeparator::None;
                    continue;
                }
                let (head, tail) = split_at_cells(rest, room);
                self.write_runs(separator, runs, offset, head.len());
                offset += head.len();
                rest = tail;
                separator = &WordSeparator::None;
                if !rest.is_empty() {
                    self.wrap();
                }
                continue;
            };
            let (head, tail) = rest.split_at(boundary);
            self.write_runs(separator, runs, offset, head.len());
            offset += head.len();
            rest = tail;
            separator = &WordSeparator::None;
            if !rest.is_empty() {
                self.wrap();
            }
        }
    }

    /// Draw a byte range of contiguous runs in their original styles.
    fn write_runs(
        &mut self,
        separator: &WordSeparator,
        runs: &[ToastRun],
        start: usize,
        len: usize,
    ) {
        let mut offset: usize = 0;
        let end = start.saturating_add(len);
        let mut separator = separator;
        for run in runs {
            let run_end = offset.saturating_add(run.text.len());
            let left = start.max(offset);
            let right = end.min(run_end);
            if left < right {
                self.write(
                    separator,
                    &run.text[left - offset..right - offset],
                    run.style,
                );
                separator = &WordSeparator::None;
            }
            offset = run_end;
        }
    }

    /// Add `text` and its preceding separator to the line being filled.
    fn write(&mut self, separator: &WordSeparator, text: &str, style: Style) {
        match separator {
            WordSeparator::None => {},
            WordSeparator::Indentation(text, style) | WordSeparator::BreakPoint(text, style) => {
                self.spans.push(Span::styled(text.clone(), *style));
            },
        }
        self.used = self
            .used
            .saturating_add(separator.width())
            .saturating_add(cell_count(text));
        self.spans.push(Span::styled(text.to_owned(), style));
    }

    /// Every line, the one still being filled last.
    fn finish(mut self) -> Text<'static> {
        self.lines.push(Line::from(self.spans));
        Text::from(self.lines)
    }
}

/// `spans` broken into lines no wider than `width`.
///
/// Each span contributes its own whitespace-separated words: a span boundary
/// is always a word boundary, even without whitespace in the text. Words are
/// joined by one unstyled space, consumed when a line breaks there. A `width`
/// of nought leaves no wrap to make, so the spans come back as the single
/// line they arrived as.
#[must_use]
pub fn wrapped(spans: Vec<Span<'static>>, width: u16) -> Text<'static> {
    wrapped_with_whitespace(spans, width, WhitespaceHandling::Collapse)
}

/// `spans` wrapped while retaining whitespace that is not used as a break.
pub(crate) fn wrapped_preserving_whitespace(
    spans: Vec<Span<'static>>,
    width: u16,
) -> Text<'static> {
    wrapped_with_whitespace(spans, width, WhitespaceHandling::Preserve)
}

/// Wrap styled toast runs, retaining their whitespace and hard line breaks.
pub(crate) fn wrapped_runs(runs: &[ToastRun], width: u16, style: Style) -> Text<'static> {
    let mut paragraphs = vec![Vec::new()];
    for run in runs {
        for (index, text) in run.text.split('\n').enumerate() {
            if index > 0 {
                paragraphs.push(Vec::new());
            }
            if let Some(paragraph) = paragraphs.last_mut() {
                paragraph.push(ToastRun::new(text, style.patch(run.style), run.breaks));
            }
        }
    }
    Text::from(
        paragraphs
            .into_iter()
            .flat_map(|runs| {
                if width == 0 {
                    return vec![Line::from(
                        runs.into_iter()
                            .map(|run| Span::styled(run.text, run.style))
                            .collect::<Vec<_>>(),
                    )];
                }
                let mut wrap = BoundaryWrap::new(usize::from(width));
                for word in preserved_run_words(runs) {
                    wrap.push(&word);
                }
                wrap.finish().lines
            })
            .collect::<Vec<_>>(),
    )
}

/// Wrap `spans` according to the requested whitespace behavior.
fn wrapped_with_whitespace(
    spans: Vec<Span<'static>>,
    width: u16,
    whitespace: WhitespaceHandling,
) -> Text<'static> {
    if width == 0 {
        return Text::from(Line::from(spans));
    }
    let mut wrap = BoundaryWrap::new(usize::from(width));
    for word in words(spans, whitespace) {
        wrap.push(&word);
    }
    wrap.finish()
}

/// The words of `spans`, each keeping the style and whitespace it arrived with.
fn words(spans: Vec<Span<'static>>, whitespace: WhitespaceHandling) -> Vec<Word> {
    match whitespace {
        WhitespaceHandling::Collapse => collapsed_words(spans),
        WhitespaceHandling::Preserve => preserved_words(spans),
    }
}

/// Words separated by one unstyled space.
fn collapsed_words(spans: Vec<Span<'static>>) -> Vec<Word> {
    let mut first = true;
    spans
        .into_iter()
        .flat_map(|span| {
            span.content
                .split_whitespace()
                .map(|text| {
                    let separator = if first {
                        first = false;
                        WordSeparator::None
                    } else {
                        WordSeparator::BreakPoint(" ".to_owned(), Style::default())
                    };
                    Word {
                        separator,
                        runs: vec![ToastRun::new(text, span.style, RunBreaks::AtSpaces)],
                    }
                })
                .collect::<Vec<Word>>()
        })
        .collect()
}

/// Words with their original leading and inter-word whitespace.
fn preserved_words(spans: Vec<Span<'static>>) -> Vec<Word> {
    preserved_run_words(
        spans
            .into_iter()
            .map(|span| ToastRun::new(span.content.into_owned(), span.style, RunBreaks::AtSpaces))
            .collect(),
    )
}

fn preserved_run_words(runs: Vec<ToastRun>) -> Vec<Word> {
    let mut words = Vec::new();
    let mut pending_whitespace = String::new();
    let mut whitespace_style = Style::default();
    let mut saw_word = false;

    for span in runs {
        if matches!(span.breaks, RunBreaks::Never) {
            let leading = span.text.len() - span.text.trim_start().len();
            let trailing = span.text.trim_end().len().max(leading);
            for (text, is_whitespace) in [
                (&span.text[..leading], true),
                (&span.text[leading..trailing], false),
                (&span.text[trailing..], true),
            ] {
                push_preserved_run(
                    &mut words,
                    &mut pending_whitespace,
                    &mut whitespace_style,
                    &mut saw_word,
                    text,
                    is_whitespace,
                    &span,
                );
            }
            continue;
        }
        let mut run = String::new();
        let mut run_is_whitespace = None;
        for character in span.text.chars() {
            let is_whitespace = character.is_whitespace();
            if run_is_whitespace.is_some_and(|kind| kind != is_whitespace) {
                push_preserved_run(
                    &mut words,
                    &mut pending_whitespace,
                    &mut whitespace_style,
                    &mut saw_word,
                    &run,
                    run_is_whitespace.unwrap_or(false),
                    &span,
                );
                run.clear();
            }
            run_is_whitespace = Some(is_whitespace);
            run.push(character);
        }
        push_preserved_run(
            &mut words,
            &mut pending_whitespace,
            &mut whitespace_style,
            &mut saw_word,
            &run,
            run_is_whitespace.unwrap_or(false),
            &span,
        );
    }
    words
}

/// Add one same-kind run to whitespace pending before a word or to the words.
fn push_preserved_run(
    words: &mut Vec<Word>,
    pending_whitespace: &mut String,
    whitespace_style: &mut Style,
    saw_word: &mut bool,
    run: &str,
    is_whitespace: bool,
    styled_run: &ToastRun,
) {
    let style = styled_run.style;
    if run.is_empty() {
        return;
    }
    if is_whitespace {
        if pending_whitespace.is_empty() {
            *whitespace_style = style;
        }
        pending_whitespace.push_str(run);
        return;
    }
    let separator = if pending_whitespace.is_empty() {
        WordSeparator::None
    } else if *saw_word {
        WordSeparator::BreakPoint(take(pending_whitespace), *whitespace_style)
    } else {
        WordSeparator::Indentation(take(pending_whitespace), *whitespace_style)
    };
    let run = ToastRun::new(run, style, styled_run.breaks);
    if matches!(separator, WordSeparator::None)
        && let Some(word) = words.last_mut()
    {
        word.runs.push(run);
    } else {
        words.push(Word {
            separator,
            runs: vec![run],
        });
    }
    *saw_word = true;
}

/// The cells `text` draws in.
fn cell_count(text: &str) -> usize { text.chars().count() }

/// `text` split where it has drawn `cells` cells.
fn split_at_cells(text: &str, cells: usize) -> (&str, &str) {
    let byte = text
        .char_indices()
        .nth(cells)
        .map_or(text.len(), |(index, _)| index);
    text.split_at(byte)
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use ratatui::style::Color;

    use super::*;

    /// The plain text of each line, which is what the breaks are about.
    fn lines(text: &Text<'static>) -> Vec<String> {
        text.lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect()
    }

    fn spans(text: &str) -> Vec<Span<'static>> { vec![Span::raw(text.to_owned())] }

    #[test]
    fn adjacent_spans_are_separate_words_joined_by_one_space() {
        let program = Style::default().fg(Color::Green);
        let argument = Style::default().fg(Color::Cyan);
        let text = wrapped(
            vec![
                Span::styled("cargo", program),
                Span::styled("check", argument),
            ],
            20,
        );
        assert_eq!(lines(&text), ["cargo check"]);
        assert_eq!(
            text.lines[0].spans,
            vec![
                Span::styled("cargo", program),
                Span::raw(" "),
                Span::styled("check", argument),
            ]
        );
    }

    #[test]
    fn a_word_that_crosses_a_style_boundary_moves_whole_to_a_fresh_line() {
        let first = Style::default().fg(Color::Green);
        let second = Style::default().fg(Color::Cyan);
        let runs = vec![
            ToastRun::new("prefix foo", first, RunBreaks::AtSpaces),
            ToastRun::new("bar", second, RunBreaks::AtSpaces),
        ];
        for text in [
            wrapped_runs(&runs, 10, Style::default()),
            wrapped_preserving_whitespace(
                vec![
                    Span::styled("prefix foo", first),
                    Span::styled("bar", second),
                ],
                10,
            ),
        ] {
            assert_eq!(lines(&text), ["prefix", "foobar"]);
            assert_eq!(text.lines[1].spans[0].style, first);
            assert_eq!(text.lines[1].spans[1].style, second);
        }
        assert_eq!(
            lines(&wrapped_runs(&runs, 4, Style::default())),
            ["pref", "ix", "foob", "ar"],
        );
    }

    #[test]
    fn text_glued_to_a_nonbreaking_run_stays_on_its_line() {
        let command_style = Style::default().fg(Color::Cyan);
        let runs = vec![
            ToastRun::new("Run ", Style::default(), RunBreaks::AtSpaces),
            ToastRun::new("cargo tile uninstall", command_style, RunBreaks::Never),
            ToastRun::new(". Done", Style::default(), RunBreaks::AtSpaces),
        ];
        for (width, expected) in [
            (24, vec!["Run", "cargo tile uninstall.", "Done"]),
            (20, vec!["Run", "cargo tile uninsta….", "Done"]),
            (12, vec!["Run", "cargo tile….", "Done"]),
        ] {
            let text = wrapped_runs(&runs, width, Style::default());
            assert_eq!(lines(&text), expected);
            assert!(
                text.lines
                    .iter()
                    .all(|line| line.width() <= usize::from(width))
            );
            let command = &text.lines[1];
            assert_eq!(command.spans[0].style, command_style);
            assert_eq!(
                command.spans.last().expect("punctuation span").style,
                Style::default()
            );
        }
    }

    #[test]
    fn a_hyphenated_word_moves_whole_when_it_fits_a_fresh_line() {
        let toolchain = "nightly-2026-09-03-x86_64-unknown-linux-gnu.";
        assert_eq!(
            lines(&wrapped(spans(&format!("Capturing for {toolchain}")), 56)),
            ["Capturing for", toolchain],
        );
        assert_eq!(
            lines(&wrapped(spans(toolchain), 24)),
            ["nightly-2026-09-03-", "x86_64-unknown-linux-", "gnu."],
        );
    }

    #[test]
    fn a_line_that_fits_is_left_alone() {
        assert_eq!(
            lines(&wrapped(spans("cargo build"), 20)),
            vec!["cargo build"]
        );
    }

    #[test]
    fn a_long_line_breaks_at_whitespace() {
        assert_eq!(
            lines(&wrapped(spans("cargo build --release"), 12)),
            vec!["cargo build", "--release"]
        );
    }

    #[test]
    fn a_line_breaks_as_many_times_as_it_takes() {
        assert_eq!(
            lines(&wrapped(spans("cargo nextest run --workspace --all"), 12)),
            vec!["cargo", "nextest run", "--workspace", "--all"]
        );
    }

    #[test]
    fn no_line_is_wider_than_the_column() {
        let width = 15;
        let text = wrapped(
            spans("cargo build --features one,two,three --release"),
            width,
        );
        for line in &text.lines {
            assert!(line.width() <= usize::from(width), "{line:?}");
        }
    }

    #[test]
    fn a_word_no_line_could_hold_breaks_where_it_runs_out() {
        assert_eq!(
            lines(&wrapped(spans("abcdefghijklmnopqrst"), 8)),
            vec!["abcdefgh", "ijklmnop", "qrst"]
        );
    }

    #[test]
    fn an_overlong_word_breaks_after_the_last_boundary_that_fits() {
        assert_eq!(
            lines(&wrapped(spans("src/long-file_name.rs"), 10)),
            vec!["src/long-", "file_name.", "rs"]
        );
    }

    #[test]
    fn a_comma_list_breaks_after_a_comma() {
        assert_eq!(
            lines(&wrapped(spans("one,two,three,four"), 10)),
            vec!["one,two,", "three,four"]
        );
    }

    #[test]
    fn a_broken_words_tail_shares_its_continuation_with_following_words() {
        assert_eq!(
            lines(&wrapped(
                spans("long-ancestor abcdefghijklmnopqrstuvwxyz0123456789-long tail"),
                38,
            )),
            vec![
                "long-ancestor",
                "abcdefghijklmnopqrstuvwxyz0123456789-",
                "long tail"
            ]
        );
    }

    #[test]
    fn a_flag_keeps_its_dashes_with_its_name() {
        assert_eq!(
            lines(&wrapped(spans("--workspace"), 8)),
            vec!["--worksp", "ace"]
        );
    }

    #[test]
    fn a_flag_value_breaks_without_a_stranded_boundary() {
        assert_eq!(
            lines(&wrapped(spans("--features=aaaaaaaaaa"), 8)),
            vec!["--featur", "es=aaaaa", "aaaaa"]
        );
    }

    #[test]
    fn an_overlong_word_waits_for_a_whole_line_to_reach_a_boundary() {
        assert_eq!(
            lines(&wrapped(spans("run abcdef/ghij"), 8)),
            vec!["run", "abcdef/", "ghij"]
        );
    }

    #[test]
    fn an_overlong_word_without_a_boundary_waits_for_a_whole_line() {
        assert_eq!(
            lines(&wrapped(spans("run aaaaaaaaaaaa"), 8)),
            vec!["run", "aaaaaaaa", "aaaa"]
        );
    }

    #[test]
    fn runs_of_whitespace_come_back_as_one_space() {
        assert_eq!(
            lines(&wrapped(spans("cargo   build"), 20)),
            vec!["cargo build"]
        );
    }

    #[test]
    fn a_break_between_two_spans_keeps_both_styles() {
        let program = Style::default().add_modifier(ratatui::style::Modifier::BOLD);
        let text = wrapped(
            vec![
                Span::styled("cargo".to_owned(), program),
                Span::raw("build --release".to_owned()),
            ],
            11,
        );
        assert_eq!(lines(&text), vec!["cargo build", "--release"]);
        let first = text.lines.first().expect("a first line");
        assert_eq!(
            first.spans.first().expect("the program span").style,
            program
        );
        assert_eq!(
            first.spans.last().expect("the argument span").style,
            Style::default()
        );
    }

    #[test]
    fn a_column_with_no_width_is_left_unwrapped() {
        assert_eq!(
            lines(&wrapped(spans("cargo build"), 0)),
            vec!["cargo build"]
        );
    }

    #[test]
    fn an_empty_line_still_stands_one_line_tall() {
        assert_eq!(wrapped(spans(""), 20).height(), 1);
    }
}
