//! Painted tokens keep terminal cell boundaries and exact text.

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;
const WIDTH: u16 = 100;
const MATCH_BACKGROUND: Color = Color::Rgb(31, 47, 63);

fn painted(text: &str) -> Buffer {
    let mut buffer = Buffer::empty(Rect::new(0, 0, WIDTH, 2));
    buffer.set_string(0, 0, text, Style::default());
    buffer
}

#[test]
fn punctuation_at_token_ends_does_not_enter_the_highlight_word() -> TestResult {
    let buffer = painted("(claude1), [claude1] claude1's /claude1/ ---");
    for column in [0, 1, 8, 11, 12, 19, 31, 32, 39] {
        let WordHit::Word(word) = painted_word(&buffer, Position::new(column, 0)) else {
            return Err(format!("column {column} must resolve a word").into());
        };
        assert_eq!(word.text(), "claude1");
    }
    for column in [10, 20, 30, 40, 41, 42, 43, 50] {
        assert_eq!(
            painted_word(&buffer, Position::new(column, 0)),
            WordHit::Empty
        );
    }
    Ok(())
}

#[test]
fn matches_keep_whole_words_and_exact_case_without_changing_ink() {
    let mut buffer = painted("claude1 claude12 Claude1 (claude1), claude1's");
    buffer.set_style(buffer.area, Style::default().fg(Color::Cyan));
    let mut highlight = WordHighlight::Off;
    highlight.press(painted_word(&buffer, Position::new(2, 0)));
    let before = buffer.clone();
    highlight.apply(&mut buffer, MATCH_BACKGROUND);
    for range in [0..7, 26..33] {
        for x in range {
            assert_eq!(buffer[(x, 0)].bg, MATCH_BACKGROUND, "match column {x}");
        }
    }
    for range in [8..16, 17..24, 36..45] {
        for x in range {
            assert_eq!(buffer[(x, 0)].bg, before[(x, 0)].bg, "nonmatch column {x}");
        }
    }
    for (before, after) in before.content.iter().zip(&buffer.content) {
        assert_eq!(after.fg, before.fg);
        assert_eq!(after.symbol(), before.symbol());
    }
}

#[test]
fn punctuation_and_guides_without_letters_cannot_construct_a_highlight_word() {
    let buffer = painted("   ├── ▼ ▶ ... !!! ()");
    for x in 0..WIDTH {
        assert_eq!(painted_word(&buffer, Position::new(x, 0)), WordHit::Empty);
    }
    assert_eq!(
        painted_word(&buffer, Position::new(WIDTH, 0)),
        WordHit::Empty
    );
}

#[test]
fn every_cell_of_a_wide_glyph_resolves_its_visible_token() -> TestResult {
    for text in [
        "界面",
        "e\u{301}clair",
        "👩\u{200d}💻work",
        "✈\u{fe0f}flight",
        "resources…",
    ] {
        let buffer = painted(text);
        let first = painted_word(&buffer, Position::new(0, 0));
        let WordHit::Word(word) = &first else {
            return Err(format!("visible token {text} must resolve").into());
        };
        let width = unicode_width::UnicodeWidthStr::width(text);
        for x in 0..u16::try_from(width)? {
            assert_eq!(
                painted_word(&buffer, Position::new(x, 0)),
                first,
                "{text} cell {x}"
            );
        }
        assert_ne!(word.text(), "");
    }
    Ok(())
}

#[test]
fn an_off_highlight_leaves_every_painted_cell_unchanged() {
    let mut buffer = painted("claude1 claude1");
    let before = buffer.clone();
    WordHighlight::Off.apply(&mut buffer, MATCH_BACKGROUND);
    assert_eq!(buffer, before);
}
