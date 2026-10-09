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

use crate::constants::WRAP_BREAK_AFTER;

/// One word of the line being wrapped, carrying the style of the span it
/// came out of so a line broken between two spans keeps both.
struct Word {
    /// Whitespace that precedes the word.
    separator: WordSeparator,
    /// The word, with no whitespace in it.
    text:      String,
    /// The style the span it came from draws in.
    style:     Style,
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
    /// The break falls before the word when whitespace is the best break.
    /// A boundary that leaves at least half a line filled may split the word;
    /// otherwise a word no line can hold is written across as many lines as
    /// it takes.
    fn push(&mut self, word: &Word) {
        let mut rest = word.text.as_str();
        let mut separator = &word.separator;
        while !rest.is_empty() {
            let room = self.room(separator);
            if cell_count(rest) <= room {
                self.write(separator, rest, word.style);
                return;
            }
            if room == 0 && (self.used > 0 || separator.is_break_point()) {
                self.wrap();
                separator = &WordSeparator::None;
                continue;
            }
            let candidate = split_at_cells(rest, room).0;
            let boundary = candidate
                .char_indices()
                .rev()
                .find(|(_, character)| WRAP_BREAK_AFTER.contains(*character))
                .map(|(index, character)| index.saturating_add(character.len_utf8()))
                .filter(|&index| cell_count(&rest[..index]) >= self.width.saturating_add(1) / 2);
            let Some(boundary) = boundary else {
                if separator.is_break_point() {
                    self.wrap();
                    separator = &WordSeparator::None;
                    continue;
                }
                let (head, tail) = split_at_cells(rest, room);
                self.write(separator, head, word.style);
                rest = tail;
                separator = &WordSeparator::None;
                if !rest.is_empty() {
                    self.wrap();
                }
                continue;
            };
            let (head, tail) = rest.split_at(boundary);
            self.write(separator, head, word.style);
            rest = tail;
            separator = &WordSeparator::None;
            if !rest.is_empty() {
                self.wrap();
            }
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
/// Breaks fall at whitespace wherever whitespace will do, and runs of it
/// come back as the one space that separates two words. A `width` of
/// nought leaves no wrap to make, so the spans come back as the single
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
                        text: text.to_owned(),
                        style: span.style,
                    }
                })
                .collect::<Vec<Word>>()
        })
        .collect()
}

/// Words with their original leading and inter-word whitespace.
fn preserved_words(spans: Vec<Span<'static>>) -> Vec<Word> {
    let mut words = Vec::new();
    let mut pending_whitespace = String::new();
    let mut whitespace_style = Style::default();
    let mut saw_word = false;

    for span in spans {
        let mut run = String::new();
        let mut run_is_whitespace = None;
        for character in span.content.chars() {
            let is_whitespace = character.is_whitespace();
            if run_is_whitespace.is_some_and(|kind| kind != is_whitespace) {
                push_preserved_run(
                    &mut words,
                    &mut pending_whitespace,
                    &mut whitespace_style,
                    &mut saw_word,
                    &run,
                    run_is_whitespace.unwrap_or(false),
                    span.style,
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
            span.style,
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
    style: Style,
) {
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
    words.push(Word {
        separator,
        text: run.to_owned(),
        style,
    });
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
                spans("long-ancestor …76jzc-tmux-3.6a/bin/tmux new-session"),
                38,
            )),
            vec!["long-ancestor …76jzc-tmux-3.6a/bin/", "tmux new-session",]
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
