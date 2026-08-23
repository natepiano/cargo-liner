use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use ratatui::widgets::Block;
use ratatui::widgets::Borders;

use super::constants::PANE_TINT_ALPHA_WHOLE;
use super::constants::PANE_TINT_BRIGHTNESS_MIDPOINT;
use super::constants::PANE_TINT_DARK_FOCUSED_ALPHA;
use super::constants::PANE_TINT_DARK_OVERLAY;
use super::constants::PANE_TINT_DARK_UNFOCUSED_ALPHA;
use super::constants::PANE_TINT_LIGHT_FOCUSED_ALPHA;
use super::constants::PANE_TINT_LIGHT_OVERLAY;
use super::constants::PANE_TINT_LIGHT_UNFOCUSED_ALPHA;
use crate::active_border_color;
use crate::focused_pane_tint_enabled;
use crate::inactive_border_color;
use crate::inactive_title_color;
use crate::theme;
use crate::title_color;

/// Pane chrome styling bundle: border and title styles for the
/// focused / unfocused render paths of a bordered pane.
#[derive(Clone, Copy)]
pub struct PaneChrome {
    /// Border style for the focused pane, where each pane draws its
    /// own box.
    ///
    /// Only [`PaneBorders::Separate`] reads it; see
    /// [`GridLines::render`].
    ///
    /// [`PaneBorders::Separate`]: crate::PaneBorders::Separate
    /// [`GridLines::render`]: crate::GridLines::render
    pub active_border:   Style,
    /// Border style for unfocused panes, and for every pane where
    /// neighbours share their border cells.
    pub inactive_border: Style,
    /// Title style when the pane is focused.
    pub active_title:    Style,
    /// Title style when the pane is unfocused.
    pub inactive_title:  Style,
}

impl PaneChrome {
    /// Build a bordered ratatui [`Block`] using this chrome.
    #[must_use]
    pub fn block(self, title: String, focused: bool) -> Block<'static> {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(self.title_style(focused))
            .border_style(self.border_style(focused));
        if let Some(fill) = pane_fill(focused) {
            block.style(fill)
        } else {
            block
        }
    }

    /// The border style this chrome applies given focus.
    ///
    /// A pane drawing its own [`Block`] owns every cell of its border,
    /// so focus can light it. A pane drawn into a shared grid asks
    /// [`GridLines`] instead, which decides per line rather than per
    /// pane.
    ///
    /// [`GridLines`]: crate::GridLines
    #[must_use]
    pub const fn border_style(self, focused: bool) -> Style {
        if focused {
            self.active_border
        } else {
            self.inactive_border
        }
    }

    /// The title style this chrome applies given focus.
    #[must_use]
    pub const fn title_style(self, focused: bool) -> Style {
        if focused {
            self.active_title
        } else {
            self.inactive_title
        }
    }
}

/// Default pane chrome.
///
/// Which of the two border shades a line takes is the layout's call,
/// not the theme's. Where each pane draws its own box the focused one
/// lights up; where neighbours share a border cell every line stays on
/// `pane_chrome.inactive_border`, because lighting a shared cell makes
/// the boundary belong to neither pane. Driving both shades from the
/// theme (rather than `Style::default()`) keeps every pane the same,
/// regardless of how a given terminal profile renders its "default
/// foreground" colour.
///
/// Titles always take focus: focused gets the bold accent, unfocused
/// the dim shade.
#[must_use]
pub fn default_pane_chrome() -> PaneChrome {
    PaneChrome {
        active_border:   Style::default().fg(active_border_color()),
        inactive_border: Style::default().fg(inactive_border_color()),
        active_title:    Style::default()
            .fg(title_color())
            .add_modifier(Modifier::BOLD),
        inactive_title:  Style::default().fg(inactive_title_color()),
    }
}

/// The background a pane sits on, or `None` when the tint is switched
/// off and panes are left to the terminal's own background.
///
/// Both states are painted, not just the focused one. A pane with no
/// background of its own is the terminal's *default* background, which
/// a transparent window treats differently from a painted cell, so
/// leaving unfocused panes bare would make focus a difference in
/// opacity rather than a difference in colour. Painting both puts them
/// on the same footing and lets the window's transparency apply to the
/// grid evenly.
///
/// A pane drawing a [`Block`] hands this to the block as its style. A
/// pane drawn into a shared frame has no block to carry it, so
/// [`crate::draw_clipped`] lays it down under the contents instead.
pub(super) fn pane_fill(focused: bool) -> Option<Style> {
    focused_pane_tint_enabled().then(|| Style::default().bg(pane_tint(focused)))
}

/// The colour a pane's contents are drawn over.
///
/// `pane_fill` in the one form a caller outside this crate can work
/// against: text carried toward the ground it stands on -- a row fading
/// out of a closing tile -- has to name that ground, and a pane whose
/// tint is switched off still stands on the appearance the theme was
/// written for.
#[must_use]
pub fn pane_background(focused: bool) -> Color {
    if focused_pane_tint_enabled() {
        pane_tint(focused)
    } else {
        theme().text.bg_focus.color
    }
}

/// Background tint behind a pane's contents.
///
/// Derived from `text.bg_focus` so it tracks the active appearance: a
/// dark background is lifted toward a cool white, a light one settled
/// toward the same hue at the other end. A focused pane is carried
/// further along that same line than an unfocused one, so focus reads
/// as more of the one shift rather than as a second colour.
///
/// A background this cannot read -- a named colour that is neither
/// black nor white, or `Reset` -- is handed back untouched, because
/// there is nothing to blend against.
fn pane_tint(focused: bool) -> Color {
    let (red, green, blue) = match theme().text.bg_focus.color {
        Color::Black => (u8::MIN, u8::MIN, u8::MIN),
        Color::White => (u8::MAX, u8::MAX, u8::MAX),
        Color::Rgb(red, green, blue) => (red, green, blue),
        other => return other,
    };
    let average = (u16::from(red) + u16::from(green) + u16::from(blue)) / 3;
    let (overlay, focused_alpha, unfocused_alpha) = if average < PANE_TINT_BRIGHTNESS_MIDPOINT {
        (
            PANE_TINT_DARK_OVERLAY,
            PANE_TINT_DARK_FOCUSED_ALPHA,
            PANE_TINT_DARK_UNFOCUSED_ALPHA,
        )
    } else {
        (
            PANE_TINT_LIGHT_OVERLAY,
            PANE_TINT_LIGHT_FOCUSED_ALPHA,
            PANE_TINT_LIGHT_UNFOCUSED_ALPHA,
        )
    };
    let alpha = if focused {
        focused_alpha
    } else {
        unfocused_alpha
    };
    Color::Rgb(
        blend(red, overlay.0, alpha),
        blend(green, overlay.1, alpha),
        blend(blue, overlay.2, alpha),
    )
}

/// One channel of `base` carried `alpha` of the way toward `overlay`,
/// where `alpha` is read against [`PANE_TINT_ALPHA_WHOLE`].
///
/// This is the composite an alpha channel would have done. A terminal
/// cell's background is three opaque bytes with nowhere to put a
/// fourth, so the blend happens here and only its result is written.
/// A transparent terminal window composites that result again against
/// whatever lies behind it, which is the second half of the same idea
/// and the half this crate does not control.
fn blend(base: u8, overlay: u8, alpha: u16) -> u8 {
    let rest = PANE_TINT_ALPHA_WHOLE.saturating_sub(alpha);
    let mixed = (u16::from(base) * rest + u16::from(overlay) * alpha) / PANE_TINT_ALPHA_WHOLE;
    u8::try_from(mixed).unwrap_or(u8::MAX)
}

/// Bordered empty-state block.
///
/// Used for panes that have no content to render (no data yet, no
/// selection, etc.). Matches the unfocused chrome of
/// [`default_pane_chrome`] so empty and populated panes draw the
/// same border shade.
#[must_use]
pub fn empty_pane_block(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .title(title.into())
        .title_style(Style::default().fg(inactive_border_color()))
        .border_style(Style::default().fg(inactive_border_color()))
}
