//! A selected word and its locations in the last painted frame.

use std::collections::HashMap;
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use ratatui::style::Color;
use tui_pane::Theme;
use unicode_width::UnicodeWidthStr;

use crate::footer_colors;
use crate::footer_colors::constants::MINIMUM_CONTRAST;
use crate::theme::WordMatchGrounds;

/// A non-empty token with its leading and trailing punctuation removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct HighlightWord(String);

/// The word under a press, or a painted position without a word.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum WordHit {
    Empty,
    Word(HighlightWord),
}

impl HighlightWord {
    /// Read one visible token without retaining punctuation at its ends.
    pub(crate) fn from_token(token: &str) -> WordHit {
        let token = token.trim_matches(|c: char| is_edge_punctuation(c));
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            WordHit::Empty
        } else {
            WordHit::Word(Self(token.to_owned()))
        }
    }

    pub(crate) fn text(&self) -> &str { &self.0 }
}

/// Read one word directly from a painted buffer.
#[cfg(test)]
fn painted_word(buffer: &Buffer, position: Position) -> WordHit {
    PaintedWords::capture(buffer, vec![buffer.area]).hit(position)
}

/// The single word selected across all visible tiles.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum WordHighlight {
    #[default]
    Off,
    Word(HighlightWord),
}

impl WordHighlight {
    pub(crate) fn press(&mut self, hit: WordHit) {
        *self = match hit {
            WordHit::Word(word) if *self != Self::Word(word.clone()) => Self::Word(word),
            WordHit::Empty | WordHit::Word(_) => Self::Off,
        };
    }

    pub(crate) fn clear(&mut self) { *self = Self::Off; }

    #[cfg(test)]
    fn apply(&self, buffer: &mut Buffer, background: Color) {
        if matches!(self, Self::Off) {
            return;
        }
        let painted = PaintedWords::capture(buffer, vec![buffer.area]);
        painted.apply(self, buffer, background);
    }
}

/// Word locations read from painted cells, limited to content and title regions.
#[derive(Clone, Debug, Default)]
pub(crate) enum PaintedWords {
    #[default]
    Uncaptured,
    Captured(Vec<PaintedToken>),
}

/// A token has one press target and one punctuation-free match area.
#[derive(Clone, Debug)]
pub(crate) struct PaintedToken {
    word:    HighlightWord,
    target:  Rect,
    matched: Rect,
}

fn is_edge_punctuation(c: char) -> bool { !c.is_alphanumeric() }

const fn is_token_separator(c: char) -> bool {
    c.is_whitespace() || matches!(c, '\u{2500}'..='\u{257f}' | '▶' | '▼' | '▷' | '▽')
}

/// Read tokens from one painted row without storing a row-wide token list.
fn row_tokens(buffer: &Buffer, row: Rect) -> impl Iterator<Item = PaintedToken> + '_ {
    let mut x = row.x;
    std::iter::from_fn(move || {
        while x < row.right() {
            let start = x;
            let mut token = String::new();
            while x < row.right() {
                let symbol = buffer[(x, row.y)].symbol();
                if symbol.chars().any(is_token_separator) {
                    break;
                }
                let width = u16::try_from(symbol.width()).unwrap_or(u16::MAX).max(1);
                if x.saturating_add(width) > row.right() {
                    break;
                }
                token.push_str(symbol);
                x = x.saturating_add(width);
            }
            let end = x;
            x = x.saturating_add(1);
            if let WordHit::Word(word) = HighlightWord::from_token(&token) {
                let prefix = token.len() - token.trim_start_matches(is_edge_punctuation).len();
                let left = u16::try_from(token[..prefix].width()).unwrap_or(u16::MAX);
                let width = u16::try_from(word.text().width()).unwrap_or(u16::MAX);
                return Some(PaintedToken {
                    word,
                    target: Rect::new(start, row.y, end - start, 1),
                    matched: Rect::new(start.saturating_add(left), row.y, width, 1),
                });
            }
        }
        None
    })
}

impl PaintedWords {
    /// An idle Off frame retains no tokens and requests no regions.
    pub(crate) fn refresh(
        &mut self,
        highlight: &WordHighlight,
        buffer: &Buffer,
        regions: impl FnOnce() -> Vec<Rect>,
    ) {
        *self = match highlight {
            WordHighlight::Off => Self::Uncaptured,
            WordHighlight::Word(_) => Self::capture(buffer, regions()),
        };
    }

    #[cfg(test)]
    pub(crate) const fn is_uncaptured(&self) -> bool { matches!(self, Self::Uncaptured) }

    /// Read only the pressed row within its painted content or title region.
    pub(crate) fn word_at(buffer: &Buffer, regions: Vec<Rect>, position: Position) -> WordHit {
        let Some(region) = regions
            .into_iter()
            .map(|region| region.intersection(buffer.area))
            .find(|region| region.contains(position))
        else {
            return WordHit::Empty;
        };
        let row = Rect::new(region.x, position.y, region.width, 1);
        row_tokens(buffer, row)
            .find(|token| token.target.contains(position))
            .map_or(WordHit::Empty, |token| WordHit::Word(token.word))
    }

    pub(crate) fn capture(buffer: &Buffer, regions: Vec<Rect>) -> Self {
        let mut words = Vec::new();
        for region in regions {
            let region = region.intersection(buffer.area);
            for y in region.y..region.bottom() {
                words.extend(row_tokens(buffer, Rect::new(region.x, y, region.width, 1)));
            }
        }
        Self::Captured(words)
    }

    #[cfg(test)]
    pub(crate) fn hit(&self, position: Position) -> WordHit {
        let Self::Captured(words) = self else {
            return WordHit::Empty;
        };
        words
            .iter()
            .find(|token| token.target.contains(position))
            .map_or(WordHit::Empty, |token| WordHit::Word(token.word.clone()))
    }

    #[cfg(test)]
    fn apply(&self, highlight: &WordHighlight, buffer: &mut Buffer, background: Color) {
        self.apply_with_inks(highlight, buffer, background, &mut MatchInkCache::default());
    }

    /// Apply the match background and the cached ink for each original cell ground.
    pub(crate) fn apply_with_inks(
        &self,
        highlight: &WordHighlight,
        buffer: &mut Buffer,
        background: Color,
        cache: &mut MatchInkCache,
    ) {
        cache.prepare(highlight, background);
        if let (WordHighlight::Word(selected), MatchPaletteState::Active(palette)) =
            (highlight, &mut cache.state)
        {
            let Self::Captured(words) = self else { return };
            for token in words {
                if &token.word == selected {
                    for position in token.matched.positions() {
                        let cell = &mut buffer[position];
                        let surface = palette.grounds.surface(cell.bg);
                        let match_ground = palette.grounds.background(surface);
                        let ink = palette
                            .inks
                            .entry((cell.fg, cell.bg))
                            .or_insert_with(|| MatchInk::resolve(cell.fg, cell.bg, match_ground));
                        cell.set_fg(ink.foreground()).set_bg(match_ground);
                    }
                }
            }
        }
    }
}

/// A matched ink either keeps its exact color or changes its HSL lightness.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MatchInk {
    Kept(Color),
    Lifted(Color),
}

impl MatchInk {
    /// Preserve hue and saturation while finding the nearest readable lightness.
    pub(crate) fn resolve(ink: Color, unmatched_ground: Color, match_ground: Color) -> Self {
        let bound =
            MINIMUM_CONTRAST.min(footer_colors::antialiased_contrast(ink, unmatched_ground));
        if footer_colors::antialiased_contrast(ink, match_ground) >= bound {
            return Self::Kept(ink);
        }
        let path = InkLightness::from(ink);
        let mut readable: f64 = if footer_colors::antialiased_contrast(Color::White, match_ground)
            >= footer_colors::antialiased_contrast(Color::Black, match_ground)
        {
            1.0
        } else {
            0.0
        };
        let mut unreadable = path.lightness;
        for _ in 0..32 {
            let middle = readable.midpoint(unreadable);
            if footer_colors::antialiased_contrast(path.at(middle), match_ground) >= bound {
                readable = middle;
            } else {
                unreadable = middle;
            }
        }
        Self::Lifted(path.at(readable))
    }

    /// Both HSL lightness endpoints bound the contrast available to any matched ink.
    pub(crate) fn capacity(ground: Color) -> f64 {
        footer_colors::antialiased_contrast(Color::White, ground)
            .max(footer_colors::antialiased_contrast(Color::Black, ground))
    }

    pub(crate) const fn foreground(self) -> Color {
        match self {
            Self::Kept(ink) | Self::Lifted(ink) => ink,
        }
    }
}

/// Each cached palette belongs to one theme, selected word, and match ground.
#[derive(Default)]
pub(crate) struct MatchInkCache {
    state:   MatchPaletteState,
    grounds: MatchGroundsCache,
}

#[derive(Default)]
enum MatchPaletteState {
    #[default]
    Off,
    Active(MatchPalette),
}

/// Selected grounds remain cached when the selected word changes.
#[derive(Default)]
enum MatchGroundsCache {
    #[default]
    Unset,
    Ready {
        theme:      Arc<Theme>,
        background: Color,
        grounds:    Arc<WordMatchGrounds>,
    },
}

impl MatchGroundsCache {
    fn current(&mut self, theme: &Arc<Theme>, background: Color) -> Arc<WordMatchGrounds> {
        if let Self::Ready {
            theme: cached,
            background: previous,
            grounds,
        } = self
            && Arc::ptr_eq(cached, theme)
            && *previous == background
        {
            return Arc::clone(grounds);
        }
        let grounds = Arc::new(crate::theme::WordMatchGrounds::new(background));
        *self = Self::Ready {
            theme: Arc::clone(theme),
            background,
            grounds: Arc::clone(&grounds),
        };
        grounds
    }
}

struct MatchPalette {
    theme:      Arc<Theme>,
    word:       HighlightWord,
    background: Color,
    grounds:    Arc<WordMatchGrounds>,
    inks:       HashMap<(Color, Color), MatchInk>,
}

impl MatchInkCache {
    fn prepare(&mut self, highlight: &WordHighlight, background: Color) {
        let WordHighlight::Word(word) = highlight else {
            self.state = MatchPaletteState::Off;
            return;
        };
        let theme = tui_pane::theme();
        if let MatchPaletteState::Active(palette) = &self.state
            && Arc::ptr_eq(&palette.theme, &theme)
            && palette.word == *word
            && palette.background == background
        {
            return;
        }
        let grounds = self.grounds.current(&theme, background);
        self.state = MatchPaletteState::Active(MatchPalette {
            theme,
            word: word.clone(),
            background,
            grounds,
            inks: HashMap::new(),
        });
    }
}

/// HSL hue and saturation stay fixed while the lightness changes.
struct InkLightness {
    relative:   [f64; 3],
    saturation: f64,
    lightness:  f64,
}

impl From<Color> for InkLightness {
    fn from(ink: Color) -> Self {
        let channels = <[u8; 3]>::from(footer_colors::rgb(ink))
            .map(f64::from)
            .map(|channel| channel / 255.0);
        let maximum = channels.into_iter().fold(0.0, f64::max);
        let minimum = channels.into_iter().fold(1.0, f64::min);
        let lightness = maximum.midpoint(minimum);
        let chroma = maximum - minimum;
        let (relative, saturation) = if chroma == 0.0 {
            ([0.0; 3], 0.0)
        } else {
            (
                channels.map(|channel| (channel - minimum) / chroma),
                chroma / (1.0 - lightness.mul_add(2.0, -1.0).abs()),
            )
        };
        Self {
            relative,
            saturation,
            lightness,
        }
    }
}

impl InkLightness {
    fn at(&self, lightness: f64) -> Color {
        let chroma = (1.0 - lightness.mul_add(2.0, -1.0).abs()) * self.saturation;
        let minimum = lightness - chroma / 2.0;
        let [red, green, blue] = self
            .relative
            .map(|channel| channel_byte(channel.mul_add(chroma, minimum) * 255.0));
        Color::Rgb(red, green, blue)
    }
}

fn channel_byte(value: f64) -> u8 {
    let mut lower = u8::MIN;
    let mut upper = u8::MAX;
    while lower < upper {
        let middle = lower.midpoint(upper);
        if value >= f64::from(middle) + 0.5 {
            lower = middle + 1;
        } else {
            upper = middle;
        }
    }
    lower
}

#[cfg(test)]
#[path = "word_highlight_tests.rs"]
mod tests;
