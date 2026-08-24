//! Theme installation.
//!
//! Builds a [`ThemeRegistry`] from cargo-tile's own [`builtins`] plus
//! every `*.toml` in the user's themes directory, resolves the
//! `[appearance]` selection against it, and publishes the result
//! process-wide so the `tui_pane` color helpers
//! (`active_border_color`, `label_color`, …) read it.
//!
//! The palettes themselves are this app's — `tui_pane` ships the
//! machinery and no colors.

mod builtins;

use std::path::Path;

use ratatui::style::Color;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeState;
use tui_pane::color_distance;
use tui_pane::error_color;
use tui_pane::label_color;

use crate::config::Config;

/// The colours a family of invocations is tied together by: the pid of
/// a command that has cargo running under it, and the parent pid on
/// each of the rows underneath.
///
/// The solarized accent set, chosen because legibility against a light
/// ground and a dark one is the property it was built for -- these are
/// the eight hues that palette holds constant across both of its
/// backgrounds. A tie that only reads in one appearance would be no
/// tie at all in the other, and this one colour has to work in a
/// theme this app did not ship.
const FAMILY_COLORS: [Color; 8] = [
    Color::Rgb(0x26, 0x8B, 0xD2), // blue
    Color::Rgb(0x85, 0x99, 0x00), // green
    Color::Rgb(0xD3, 0x36, 0x82), // magenta
    Color::Rgb(0x2A, 0xA1, 0x98), // cyan
    Color::Rgb(0xCB, 0x4B, 0x16), // orange
    Color::Rgb(0x6C, 0x71, 0xC4), // violet
    Color::Rgb(0xB5, 0x89, 0x00), // yellow
    Color::Rgb(0xDC, 0x32, 0x2F), // red
];

/// How near a family colour may come to one the display already
/// spends, before it is dropped from the palette. On the scale
/// [`color_distance`] works in.
///
/// Set where it clears both of the colours in [`reserved_colors`] by a
/// comfortable margin under every theme this app ships, and still
/// leaves four families to tell apart -- more than the screen has held
/// at once.
const FAMILY_COLOR_CLEARANCE: u16 = 200;

/// The colours the display already spends on a meaning of their own,
/// which a family colour must not be read as.
///
/// The column headers, because a pid drawn near that colour reads as
/// an uncoloured pid -- and an uncoloured pid is exactly what a row
/// with nothing under it gets, the one thing a family colour says it
/// is not.
///
/// And whatever the theme draws an error in, because a pid in that
/// colour reads as something having gone wrong when the only thing
/// that has happened is that a command started another one.
fn reserved_colors() -> [Color; 2] { [label_color(), error_color()] }

/// The palette entries standing clear of every colour in `reserved`.
///
/// Split out from [`family_palette`] so a test can aim it at a theme by
/// name. The colours that matter are the ones this app ships, and the
/// theme a test process runs under is the framework's fallback, whose
/// label colour is a plain grey none of ours is.
fn palette_clear_of(reserved: &[Color]) -> impl Iterator<Item = Color> + use<'_> {
    FAMILY_COLORS.into_iter().filter(move |color| {
        reserved.iter().all(|taken| {
            color_distance(*color, *taken).is_none_or(|apart| apart >= FAMILY_COLOR_CLEARANCE)
        })
    })
}

/// The palette entries a family may be handed, which is every one
/// standing clear of [`reserved_colors`].
///
/// Read from the theme on each call rather than worked out once: the
/// appearance can change under a running display, and a palette fixed
/// at startup would go on handing out a colour the headers or the
/// errors had since moved onto. A colour with no channels to read is
/// kept -- there is nothing to measure it against, and emptying the
/// palette over one unreadable colour is the worse failure.
fn family_palette() -> impl Iterator<Item = Color> {
    let reserved = reserved_colors();
    palette_clear_of(&reserved)
        .collect::<Vec<Color>>()
        .into_iter()
}

/// How many families can be told apart at once.
pub(crate) fn family_color_count() -> usize { family_palette().count() }

/// The colour a family holds, by the index the roster handed it.
///
/// Handed out against the families on screen rather than computed from
/// the pid: a colour is only a tie if no two families in view share
/// one, and any function of the pid alone will collide sooner or later
/// -- it did so on the first two families it was shown. The roster
/// keeps the assignment, which is also what makes the colour the same
/// in the summary and in the command's own cell.
///
/// A theme that leaves the palette with nothing clear of what it
/// already spends gets the label colour back, which is what an
/// uncoloured pid draws in: no tie to read, rather than a tie that
/// would be read as something else.
pub(crate) fn family_color(index: usize) -> Color {
    let count = family_color_count();
    if count == 0 {
        return label_color();
    }
    family_palette()
        .nth(index % count)
        .unwrap_or_else(label_color)
}

/// Install the theme `config` selects. Returns a note when the
/// configured theme id matched nothing and another variant was
/// substituted.
pub(crate) fn install(config: &Config, themes_dir: Option<&Path>) -> Option<String> {
    let registry = ThemeRegistry::from_dir_with_builtins(themes_dir, builtins::builtins());
    let resolved = registry.resolve_active(
        &config.appearance.mode,
        &config.appearance.light_theme,
        &config.appearance.dark_theme,
        None,
    );
    let note = resolved
        .miss
        .as_ref()
        .map(|missing| format!("theme `{missing}` not found — using a built-in"));
    let initial_theme = (*resolved.theme).clone();
    tui_pane::install_theme_state(ThemeState::with_registry(registry, initial_theme));
    note
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point of the colour is that it says this pid has
    /// cargo running under it. Near the headers it says the opposite,
    /// and near the error colour it says something worse.
    #[test]
    fn no_family_colour_is_drawn_near_one_that_already_means_something() {
        for index in 0..family_color_count() {
            let color = family_color(index);

            for taken in reserved_colors() {
                let apart = color_distance(color, taken);

                assert!(
                    apart.is_none_or(|apart| apart >= FAMILY_COLOR_CLEARANCE),
                    "{color:?} stands {apart:?} from {taken:?}"
                );
            }
        }
    }

    /// A palette the reserved colours had emptied would leave every
    /// family unmarked, which is the state the colours were added to
    /// end.
    #[test]
    fn what_the_display_already_spends_never_takes_the_whole_palette() {
        assert!(family_color_count() > 1, "{}", family_color_count());
    }

    /// The test above reads whatever theme the process is running
    /// under, which is the framework's fallback and not one of ours.
    /// These are the palettes that actually ship.
    #[test]
    fn every_theme_this_app_ships_leaves_families_to_tell_apart() {
        for variant in builtins::builtins() {
            let reserved = [
                variant.theme.semantic.label.color,
                variant.theme.semantic.error.color,
            ];
            let clear: Vec<Color> = palette_clear_of(&reserved).collect();

            assert!(clear.len() > 1, "{:?} leaves {clear:?}", variant.id);
            for color in clear {
                for taken in reserved {
                    let apart = color_distance(color, taken);

                    assert!(
                        apart.is_none_or(|apart| apart >= FAMILY_COLOR_CLEARANCE),
                        "{:?}: {color:?} stands {apart:?} from {taken:?}",
                        variant.id
                    );
                }
            }
        }
    }
}
