//! Wrapping a styled line to the width of the column it is drawn in.
//!
//! Wrapping in ratatui belongs to [`ratatui::widgets::Paragraph`], and a
//! [`ratatui::widgets::Table`] cell draws the [`Text`] it is handed as
//! it stands. The command column is the one cell that outruns its
//! width, so the wrap happens before the cell is built and the row is
//! made as tall as the lines that came back.

use std::mem::take;

use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::text::Span;
use ratatui::text::Text;

/// One word of the line being wrapped, carrying the style of the span it
/// came out of so a line broken between two spans keeps both.
struct Word {
    /// The word, with no whitespace in it.
    text:  String,
    /// The style the span it came from draws in.
    style: Style,
}

/// The lines already broken off and the one still being filled.
struct Wrap {
    /// Cells one line holds.
    width: usize,
    /// Lines the wrap has finished.
    lines: Vec<Line<'static>>,
    /// Spans of the line being filled.
    spans: Vec<Span<'static>>,
    /// Cells those spans occupy.
    used:  usize,
}

impl Wrap {
    /// An empty wrap onto lines of `width` cells.
    const fn new(width: usize) -> Self {
        Self {
            width,
            lines: Vec::new(),
            spans: Vec::new(),
            used: 0,
        }
    }

    /// Cells the line being filled has left for a word, with the space
    /// that would separate it from the word already there taken off.
    fn room(&self) -> usize {
        self.width
            .saturating_sub(self.used)
            .saturating_sub(self.separator())
    }

    /// The one cell a word costs to be told from the word before it, or
    /// nothing at the start of a line.
    fn separator(&self) -> usize { usize::from(!self.spans.is_empty()) }

    /// Break the line being filled and start the next one.
    fn wrap(&mut self) {
        self.lines.push(Line::from(take(&mut self.spans)));
        self.used = 0;
    }

    /// Put `word` on the line being filled.
    ///
    /// The break falls before the word when the line cannot hold it and
    /// an empty one could. A word no line could hold is written out
    /// across as many lines as it takes, which is the only break here
    /// that lands anywhere but whitespace.
    fn push(&mut self, word: &Word) {
        if cell_count(&word.text) <= self.width {
            if cell_count(&word.text) > self.room() {
                self.wrap();
            }
            self.write(&word.text, word.style);
            return;
        }
        let mut rest = word.text.as_str();
        while !rest.is_empty() {
            if self.room() == 0 {
                self.wrap();
            }
            let (head, tail) = split_at_cells(rest, self.room());
            self.write(head, word.style);
            rest = tail;
        }
    }

    /// Add `text` to the line being filled, after the space that tells
    /// it from the word already there.
    fn write(&mut self, text: &str, style: Style) {
        let separator = self.separator();
        if separator > 0 {
            self.spans.push(Span::raw(" "));
        }
        self.used = self
            .used
            .saturating_add(separator)
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
pub(crate) fn wrapped(spans: Vec<Span<'static>>, width: u16) -> Text<'static> {
    if width == 0 {
        return Text::from(Line::from(spans));
    }
    let mut wrap = Wrap::new(usize::from(width));
    for word in words(spans) {
        wrap.push(&word);
    }
    wrap.finish()
}

/// The words of `spans`, each keeping the style of its span. Whitespace
/// is where a break may fall rather than something to draw, so it is not
/// a word.
fn words(spans: Vec<Span<'static>>) -> Vec<Word> {
    spans
        .into_iter()
        .flat_map(|span| {
            span.content
                .split_whitespace()
                .map(|text| Word {
                    text:  text.to_owned(),
                    style: span.style,
                })
                .collect::<Vec<Word>>()
        })
        .collect()
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
            lines(&wrapped(spans("--features=aaaaaaaaaa"), 8)),
            vec!["--featur", "es=aaaaa", "aaaaa"]
        );
    }

    #[test]
    fn a_word_too_long_finishes_the_line_it_started_on() {
        assert_eq!(
            lines(&wrapped(spans("run aaaaaaaaaaaa"), 8)),
            vec!["run aaaa", "aaaaaaaa"]
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
