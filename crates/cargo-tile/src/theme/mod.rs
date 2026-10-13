//! cargo-tile's themes: the built-in variants [`builtins`] hands to
//! [`tui_pane::install_theme`] at startup, and the colours a family of
//! invocations is tied together by.
//!
//! The palettes themselves are this app's — `tui_pane` ships the
//! machinery and no colors.

pub(crate) mod builtins;

use ratatui::style::Color;
use tui_pane::Theme;
use tui_pane::label_color;

use crate::constants::FAMILY_BLUE_ROLE;
use crate::constants::FAMILY_CHROMA_SCALE;
use crate::constants::FAMILY_COLOR_CLEARANCE;
use crate::constants::FAMILY_CYAN_ROLE;
use crate::constants::FAMILY_GREEN_ROLE;
use crate::constants::FAMILY_HUE_SECTOR;
use crate::constants::FAMILY_MAGENTA_ROLE;
use crate::constants::FAMILY_ORANGE_ROLE;
use crate::constants::FAMILY_RED_ROLE;
use crate::constants::FAMILY_VIOLET_ROLE;
use crate::constants::FAMILY_YELLOW_ROLE;

/// Theme roles tying a parent's pid to the invocations beneath it.
const FAMILY_ROLES: [&str; 8] = [
    FAMILY_BLUE_ROLE,
    FAMILY_GREEN_ROLE,
    FAMILY_MAGENTA_ROLE,
    FAMILY_CYAN_ROLE,
    FAMILY_ORANGE_ROLE,
    FAMILY_VIOLET_ROLE,
    FAMILY_YELLOW_ROLE,
    FAMILY_RED_ROLE,
];

/// HSL hue and saturation with lightness omitted from identity comparisons.
enum FamilyChroma {
    TerminalDefault,
    Hue { horizontal: f64, vertical: f64 },
}

impl FamilyChroma {
    fn from_color(color: Color) -> Self {
        let Color::Rgb(red, green, blue) = tui_pane::blend_color(color, Color::Black, 0) else {
            return Self::TerminalDefault;
        };
        let maximum_channel = red.max(green).max(blue);
        let minimum_channel = red.min(green).min(blue);
        if maximum_channel == minimum_channel {
            return Self::Hue {
                horizontal: 0.0,
                vertical:   0.0,
            };
        }
        let maximum = f64::from(maximum_channel) / f64::from(u8::MAX);
        let minimum = f64::from(minimum_channel) / f64::from(u8::MAX);
        let chroma = maximum - minimum;
        let saturation = chroma / (1.0 - (maximum + minimum - 1.0).abs());
        let sector = if maximum_channel == red {
            (f64::from(green) - f64::from(blue)) / f64::from(u8::MAX) / chroma
        } else if maximum_channel == green {
            (f64::from(blue) - f64::from(red)) / f64::from(u8::MAX) / chroma + 2.0
        } else {
            (f64::from(red) - f64::from(green)) / f64::from(u8::MAX) / chroma + 4.0
        };
        let hue = (sector * FAMILY_HUE_SECTOR).to_radians();
        let radius = saturation * FAMILY_CHROMA_SCALE;
        Self::Hue {
            horizontal: radius * hue.cos(),
            vertical:   radius * hue.sin(),
        }
    }
}

/// Distance on the hue circle, whose radius is the colour's HSL saturation.
/// Neutral inks sit at its centre; lightness cannot hide a hue collision.
/// A terminal default has no measurable hue and does not exclude an ink.
fn family_colour_distance(color: Color, other: Color) -> f64 {
    match (
        FamilyChroma::from_color(color),
        FamilyChroma::from_color(other),
    ) {
        (
            FamilyChroma::Hue {
                horizontal,
                vertical,
            },
            FamilyChroma::Hue {
                horizontal: other_horizontal,
                vertical: other_vertical,
            },
        ) => (horizontal - other_horizontal).hypot(vertical - other_vertical),
        _ => f64::INFINITY,
    }
}

/// Inks used for uncoloured pids, errors, directory headings and progress.
const fn reserved_colors(theme: &Theme) -> [Color; 5] {
    [
        theme.semantic.label.color,
        theme.semantic.error.color,
        theme.semantic.accent.color,
        theme.semantic.success.color,
        theme.text.default.color,
    ]
}

/// Palette entries clear of reserved inks and every previously selected hue.
///
/// Split out from [`family_palette`] so a test can aim it at a theme by
/// name. The colours that matter are the ones this app ships, and the
/// theme a test process runs under is the framework's fallback, whose
/// label colour is a plain grey none of ours is.
fn palette_clear_of<'theme>(
    theme: &'theme Theme,
    reserved: &'theme [Color],
) -> impl Iterator<Item = Color> + 'theme {
    let mut selected = Vec::new();
    FAMILY_ROLES
        .into_iter()
        .zip(builtins::DARK_FAMILY_COLORS)
        .map(|(role, fallback)| theme.roles.get(role).map_or(fallback, |spec| spec.color))
        .filter(move |color| {
            reserved
                .iter()
                .all(|taken| family_colour_distance(*color, *taken) >= FAMILY_COLOR_CLEARANCE)
        })
        .filter(move |color| {
            let clear = selected
                .iter()
                .all(|taken| family_colour_distance(*color, *taken) >= FAMILY_COLOR_CLEARANCE);
            if clear {
                selected.push(*color);
            }
            clear
        })
}

/// Distinct family hues standing clear of [`reserved_colors`].
///
/// Read from the theme on each call rather than worked out once: the
/// appearance can change under a running display, and a palette fixed
/// at startup would go on handing out a colour the headers or the
/// errors had since moved onto. A colour with no channels to read is
/// kept -- there is nothing to measure it against, and emptying the
/// palette over one unreadable colour is the worse failure.
fn family_palette() -> impl Iterator<Item = Color> {
    let theme = tui_pane::theme();
    let reserved = reserved_colors(&theme);
    palette_clear_of(&theme, &reserved)
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

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn shipped_themes() -> Vec<(String, Theme)> {
        let mut themes: Vec<_> = builtins::builtins()
            .into_iter()
            .map(|variant| (variant.id.as_str().to_owned(), variant.theme))
            .collect();
        let starter =
            toml::from_str::<tui_pane::ThemeFamily>(include_str!("../../themes/starter.toml"));
        assert!(starter.is_ok(), "starter must parse: {starter:?}");
        let Ok(starter) = starter else {
            return themes;
        };
        themes.extend(
            starter
                .variants
                .into_iter()
                .map(|variant| (variant.name.clone(), variant.into_theme())),
        );
        themes
    }

    #[test]
    fn every_builtin_family_palette_has_five_colours_and_light_matches_dark() {
        let mut dark_count = 0;
        let mut light_count = 0;
        for (name, theme) in shipped_themes() {
            let reserved = reserved_colors(&theme);
            let palette: Vec<_> = palette_clear_of(&theme, &reserved).collect();
            assert!(
                palette.len() >= crate::constants::MIN_FAMILY_COLORS,
                "{name}: {palette:?}"
            );
            if name == crate::constants::DEFAULT_DARK_THEME {
                dark_count = palette.len();
            }
            if name == crate::constants::DEFAULT_LIGHT_THEME {
                light_count = palette.len();
            }
        }
        assert_eq!(light_count, dark_count);
    }

    #[test]
    fn every_builtin_family_palette_separates_each_pair_of_hues() {
        for (name, theme) in shipped_themes() {
            let reserved = reserved_colors(&theme);
            let palette: Vec<_> = palette_clear_of(&theme, &reserved).collect();
            for (index, color) in palette.iter().enumerate() {
                for other in &palette[index + 1..] {
                    let distance = family_colour_distance(*color, *other);
                    assert!(
                        distance >= crate::constants::FAMILY_COLOR_CLEARANCE,
                        "{name}: {color:?} and {other:?} are {distance:.2} apart"
                    );
                }
            }
        }
    }

    #[test]
    fn every_builtin_family_colour_clears_the_table_semantic_inks() {
        for (name, theme) in shipped_themes() {
            let reserved = reserved_colors(&theme);
            for color in palette_clear_of(&theme, &reserved) {
                for taken in [
                    theme.semantic.label.color,
                    theme.semantic.error.color,
                    theme.semantic.accent.color,
                    theme.semantic.success.color,
                    theme.text.default.color,
                ] {
                    let distance = family_colour_distance(color, taken);
                    assert!(
                        distance >= crate::constants::FAMILY_COLOR_CLEARANCE,
                        "{name}: {color:?} and {taken:?} are {distance:.2} apart"
                    );
                }
            }
        }
    }

    /// The whole point of the colour is that it says this pid has
    /// cargo running under it. Near the headers it says the opposite,
    /// and near the error colour it says something worse.
    #[test]
    fn no_family_colour_is_drawn_near_one_that_already_means_something() {
        for index in 0..family_color_count() {
            let color = family_color(index);

            for taken in reserved_colors(&tui_pane::theme()) {
                let apart = family_colour_distance(color, taken);

                assert!(
                    apart >= FAMILY_COLOR_CLEARANCE,
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
            let reserved = reserved_colors(&variant.theme);
            let clear: Vec<Color> = palette_clear_of(&variant.theme, &reserved).collect();

            assert!(clear.len() > 1, "{:?} leaves {clear:?}", variant.id);
            for color in clear {
                for taken in reserved {
                    let apart = family_colour_distance(color, taken);

                    assert!(
                        apart >= FAMILY_COLOR_CLEARANCE,
                        "{:?}: {color:?} stands {apart:?} from {taken:?}",
                        variant.id
                    );
                }
            }
        }
    }
}
