//! cargo-tile's compiled-in theme variants.
//!
//! `tui_pane` supplies the theme machinery — the types, the registry,
//! the resolver, the directory watch — and none of the colors. Every
//! palette this app ships lives here, so retuning cargo-tile's grid
//! cannot move cargo-port's panes.
//!
//! [`builtins`] is what startup hands to [`tui_pane::install_theme`];
//! user `themes/*.toml` variants layer on top, replacing a built-in
//! when the names match. The `cargo-tile/themes/*.toml` templates
//! mirror these constructors as copyable documentation, locked against
//! drift by the tests at the bottom of this file.

use std::collections::BTreeMap;

use ratatui::style::Color;
use tui_pane::Appearance;
use tui_pane::DiskUsageTheme;
use tui_pane::FinderTheme;
use tui_pane::FocusTheme;
use tui_pane::PaneChromeTheme;
use tui_pane::SemanticTheme;
use tui_pane::StatusTheme;
use tui_pane::StyleSpec;
use tui_pane::TextTheme;
use tui_pane::Theme;
use tui_pane::ThemeId;
use tui_pane::ThemeVariant;

use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_HC_DARK_THEME;
use crate::constants::DEFAULT_HC_LIGHT_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;

/// Family hues readable over the dark theme's focused and unfocused panes.
pub(super) const DARK_FAMILY_COLORS: [Color; 8] = [
    Color::Rgb(140, 150, 255),
    Color::Rgb(148, 210, 0),
    Color::Rgb(255, 110, 207),
    Color::Rgb(0, 205, 205),
    Color::Rgb(255, 185, 45),
    Color::Rgb(209, 130, 255),
    Color::Rgb(230, 205, 0),
    Color::Rgb(255, 115, 115),
];

/// Family hues readable over the light theme's focused and unfocused panes.
const LIGHT_FAMILY_COLORS: [Color; 8] = [
    Color::Rgb(0, 15, 170),
    Color::Rgb(70, 105, 0),
    Color::Rgb(150, 0, 100),
    Color::Rgb(0, 105, 105),
    Color::Rgb(135, 89, 0),
    Color::Rgb(105, 0, 160),
    Color::Rgb(120, 95, 0),
    Color::Rgb(175, 25, 40),
];

fn family_roles(colors: [Color; 8]) -> BTreeMap<String, StyleSpec> {
    super::FAMILY_ROLES
        .into_iter()
        .zip(colors)
        .map(|(role, color)| (role.to_owned(), StyleSpec::from_color(color)))
        .collect()
}

/// The variants cargo-tile compiles in, in the order the settings
/// stepper offers them.
pub(crate) fn builtins() -> Vec<ThemeVariant> {
    vec![
        ThemeVariant {
            id:         ThemeId::new(DEFAULT_DARK_THEME),
            appearance: Appearance::Dark,
            theme:      default_dark(),
        },
        ThemeVariant {
            id:         ThemeId::new(DEFAULT_LIGHT_THEME),
            appearance: Appearance::Light,
            theme:      default_light(),
        },
        ThemeVariant {
            id:         ThemeId::new(DEFAULT_HC_DARK_THEME),
            appearance: Appearance::Dark,
            theme:      high_contrast_dark(),
        },
        ThemeVariant {
            id:         ThemeId::new(DEFAULT_HC_LIGHT_THEME),
            appearance: Appearance::Light,
            theme:      high_contrast_light(),
        },
    ]
}

/// Default dark variant, named by [`DEFAULT_DARK_THEME`].
///
/// `inactive_border` is the shade every tile draws in while the tint is
/// painted: a border is a cell two tiles share, so focus is carried by
/// the tint under a tile's contents. On a transparent screen there is
/// no tint, and the focused tile's border lights instead.
#[must_use]
fn default_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            // No colour of its own: where no tint is painted to carry
            // focus, the focused tile's border takes the focused title's.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::DarkGray),
            active_title:    StyleSpec::bold(Color::Yellow),
            inactive_title:  StyleSpec::from_color(Color::White),
        },
        focus:       FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(125, 125, 125)),
            hover:      StyleSpec::from_color(Color::Rgb(80, 80, 80)),
            remembered: StyleSpec::from_color(Color::Rgb(40, 40, 40)),
        },
        semantic:    SemanticTheme {
            accent:       StyleSpec::from_color(Color::Cyan),
            error:        StyleSpec::from_color(Color::Rgb(255, 115, 115)),
            on_error:     StyleSpec::from_color(Color::White),
            inline_error: StyleSpec::from_color(Color::Yellow),
            success:      StyleSpec::from_color(Color::Green),
            label:        StyleSpec::from_color(Color::Rgb(150, 190, 180)),
            warning:      StyleSpec::from_color(Color::Yellow),
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::White),
            secondary: StyleSpec::from_color(Color::Gray),
            dim:       StyleSpec::from_color(Color::DarkGray),
            bright:    StyleSpec::from_color(Color::Cyan),
            bg_focus:  StyleSpec::from_color(Color::Black),
        },
        status:      StatusTheme {
            bar: StyleSpec::from_color(Color::Rgb(58, 58, 58)),
        },
        finder:      FinderTheme {
            match_bg: StyleSpec::from_color(Color::Rgb(0, 90, 100)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::from_color(Color::Rgb(100, 220, 100)),
            mid:  StyleSpec::from_color(Color::Rgb(255, 255, 255)),
            high: StyleSpec::from_color(Color::Rgb(255, 100, 100)),
        },
        roles:       family_roles(DARK_FAMILY_COLORS),
    }
}

/// Default light variant, named by [`DEFAULT_LIGHT_THEME`]. Each
/// value is picked for legibility on a white terminal background.
#[must_use]
fn default_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            // No colour of its own: where no tint is painted to carry
            // focus, the focused tile's border takes the focused title's.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::Rgb(140, 140, 140)),
            active_title:    StyleSpec::bold(Color::Rgb(137, 86, 0)),
            inactive_title:  StyleSpec::from_color(Color::Black),
        },
        focus:       FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(200, 200, 200)),
            hover:      StyleSpec::from_color(Color::Rgb(220, 220, 220)),
            remembered: StyleSpec::from_color(Color::Rgb(235, 235, 235)),
        },
        semantic:    SemanticTheme {
            accent:       StyleSpec::from_color(Color::Rgb(0, 95, 135)),
            error:        StyleSpec::from_color(Color::Rgb(170, 0, 0)),
            on_error:     StyleSpec::from_color(Color::White),
            inline_error: StyleSpec::from_color(Color::Rgb(180, 95, 0)),
            success:      StyleSpec::from_color(Color::Rgb(0, 116, 0)),
            label:        StyleSpec::from_color(Color::Rgb(60, 100, 90)),
            warning:      StyleSpec::from_color(Color::Rgb(145, 76, 0)),
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::Black),
            secondary: StyleSpec::from_color(Color::Rgb(70, 70, 70)),
            dim:       StyleSpec::from_color(Color::Rgb(130, 130, 130)),
            bright:    StyleSpec::from_color(Color::Rgb(0, 95, 135)),
            bg_focus:  StyleSpec::from_color(Color::White),
        },
        status:      StatusTheme {
            bar: StyleSpec::from_color(Color::Rgb(220, 220, 220)),
        },
        finder:      FinderTheme {
            match_bg: StyleSpec::from_color(Color::Rgb(255, 245, 180)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::from_color(Color::Rgb(0, 140, 0)),
            mid:  StyleSpec::from_color(Color::Rgb(90, 90, 90)),
            high: StyleSpec::from_color(Color::Rgb(200, 0, 0)),
        },
        roles:       family_roles(LIGHT_FAMILY_COLORS),
    }
}

/// High-contrast dark variant, named by [`DEFAULT_HC_DARK_THEME`].
///
/// Pure white on pure black with bold modifiers throughout; accent
/// fields use the bright ANSI palette (`LightYellow`, `LightCyan`,
/// `LightGreen`, `LightRed`, `LightMagenta`) for maximum legibility
/// under reduced-vision or glare conditions.
#[must_use]
fn high_contrast_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            // No colour of its own: where no tint is painted to carry
            // focus, the focused tile's border takes the focused title's.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::White),
            active_title:    StyleSpec::bold(Color::LightYellow),
            inactive_title:  StyleSpec::from_color(Color::White),
        },
        focus:       FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(0, 60, 100)),
            hover:      StyleSpec::from_color(Color::Rgb(0, 40, 70)),
            remembered: StyleSpec::from_color(Color::Rgb(0, 25, 50)),
        },
        semantic:    SemanticTheme {
            accent:       StyleSpec::bold(Color::LightCyan),
            error:        StyleSpec::bold(Color::Rgb(255, 115, 115)),
            on_error:     StyleSpec::from_color(Color::White),
            inline_error: StyleSpec::bold(Color::LightYellow),
            success:      StyleSpec::bold(Color::LightGreen),
            label:        StyleSpec::from_color(Color::White),
            warning:      StyleSpec::bold(Color::LightYellow),
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::White),
            secondary: StyleSpec::from_color(Color::White),
            dim:       StyleSpec::from_color(Color::Gray),
            bright:    StyleSpec::bold(Color::LightYellow),
            bg_focus:  StyleSpec::from_color(Color::Black),
        },
        status:      StatusTheme {
            bar: StyleSpec::from_color(Color::Rgb(60, 60, 60)),
        },
        finder:      FinderTheme {
            match_bg: StyleSpec::from_color(Color::LightYellow),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::bold(Color::LightGreen),
            mid:  StyleSpec::from_color(Color::White),
            high: StyleSpec::bold(Color::LightRed),
        },
        roles:       family_roles(DARK_FAMILY_COLORS),
    }
}

/// High-contrast light variant, named by [`DEFAULT_HC_LIGHT_THEME`].
///
/// Pure black on pure white with bold modifiers throughout; accent
/// fields use saturated dark colors (deep red, deep green, deep blue,
/// deep orange) chosen for AAA-grade contrast against a white canvas.
#[must_use]
fn high_contrast_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            // No colour of its own: where no tint is painted to carry
            // focus, the focused tile's border takes the focused title's.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::Black),
            active_title:    StyleSpec::bold(Color::Rgb(140, 60, 0)),
            inactive_title:  StyleSpec::from_color(Color::Black),
        },
        focus:       FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(255, 230, 100)),
            hover:      StyleSpec::from_color(Color::Rgb(255, 245, 180)),
            remembered: StyleSpec::from_color(Color::Rgb(255, 250, 220)),
        },
        semantic:    SemanticTheme {
            accent:       StyleSpec::bold(Color::Rgb(0, 0, 140)),
            error:        StyleSpec::bold(Color::Rgb(180, 0, 0)),
            on_error:     StyleSpec::from_color(Color::White),
            inline_error: StyleSpec::bold(Color::Rgb(140, 60, 0)),
            success:      StyleSpec::bold(Color::Rgb(0, 100, 0)),
            label:        StyleSpec::from_color(Color::Black),
            warning:      StyleSpec::bold(Color::Rgb(140, 60, 0)),
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::Black),
            secondary: StyleSpec::from_color(Color::Black),
            dim:       StyleSpec::from_color(Color::Rgb(80, 80, 80)),
            bright:    StyleSpec::bold(Color::Rgb(140, 60, 0)),
            bg_focus:  StyleSpec::from_color(Color::White),
        },
        status:      StatusTheme {
            bar: StyleSpec::from_color(Color::Rgb(210, 210, 210)),
        },
        finder:      FinderTheme {
            match_bg: StyleSpec::from_color(Color::Rgb(255, 230, 100)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::bold(Color::Rgb(0, 100, 0)),
            mid:  StyleSpec::from_color(Color::Black),
            high: StyleSpec::bold(Color::Rgb(180, 0, 0)),
        },
        roles:       family_roles(LIGHT_FAMILY_COLORS),
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tui_pane::ThemeFamily;
    use tui_pane::ThemeVariantFile;

    use super::*;
    use crate::constants::MIN_STATUS_LINE_TEXT_CONTRAST;

    const DARK_TEMPLATE: &str = include_str!("../../themes/default_dark.toml");
    const HC_TEMPLATE: &str = include_str!("../../themes/high_contrast.toml");
    const LIGHT_TEMPLATE: &str = include_str!("../../themes/default_light.toml");
    const STARTER_TEMPLATE: &str = include_str!("../../themes/starter.toml");
    const XTERM_ANSI_COLORS: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];

    /// Parse one template and return its variants, asserting the schema
    /// version and the expected variant count.
    fn variants(template: &str, expected: usize) -> Vec<ThemeVariantFile> {
        let family: ThemeFamily = toml::from_str(template).expect("template should parse");
        assert_eq!(family.schema, 1);
        assert_eq!(family.variants.len(), expected);
        family.variants
    }

    #[test]
    fn dark_template_matches_constructor() {
        let variant = variants(DARK_TEMPLATE, 1).remove(0);
        assert_eq!(variant.name, DEFAULT_DARK_THEME);
        assert_eq!(variant.appearance, Appearance::Dark);
        assert_eq!(variant.into_theme(), default_dark());
    }

    #[test]
    fn light_template_matches_constructor() {
        let variant = variants(LIGHT_TEMPLATE, 1).remove(0);
        assert_eq!(variant.name, DEFAULT_LIGHT_THEME);
        assert_eq!(variant.appearance, Appearance::Light);
        assert_eq!(variant.into_theme(), default_light());
    }

    #[test]
    fn hc_template_matches_constructors() {
        let mut both = variants(HC_TEMPLATE, 2);
        let light = both.remove(1);
        let dark = both.remove(0);
        assert_eq!(dark.name, DEFAULT_HC_DARK_THEME);
        assert_eq!(dark.appearance, Appearance::Dark);
        assert_eq!(dark.into_theme(), high_contrast_dark());
        assert_eq!(light.name, DEFAULT_HC_LIGHT_THEME);
        assert_eq!(light.appearance, Appearance::Light);
        assert_eq!(light.into_theme(), high_contrast_light());
    }

    /// The starter is a copy-me template, not a mirror, so it only has
    /// to parse and to carry a name that will not collide with a
    /// built-in when a user drops it in.
    #[test]
    fn starter_template_parses_under_its_own_name() {
        let variant = variants(STARTER_TEMPLATE, 1).remove(0);
        assert_eq!(variant.appearance, Appearance::Dark);
        assert!(
            !builtins()
                .iter()
                .any(|builtin| builtin.id.as_str() == variant.name),
            "starter must not shadow a built-in on drop-in"
        );
    }

    #[test]
    fn builtins_are_the_four_named_variants() {
        let ids: Vec<_> = builtins()
            .into_iter()
            .map(|v| v.id.as_str().to_owned())
            .collect();
        assert_eq!(
            ids,
            vec![
                DEFAULT_DARK_THEME,
                DEFAULT_LIGHT_THEME,
                DEFAULT_HC_DARK_THEME,
                DEFAULT_HC_LIGHT_THEME,
            ]
        );
    }

    fn linear_channel(channel: u8) -> f64 {
        let channel = f64::from(channel) / 255.0;
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn relative_luminance(red: u8, green: u8, blue: u8) -> f64 {
        0.2126f64.mul_add(
            linear_channel(red),
            0.7152f64.mul_add(linear_channel(green), 0.0722 * linear_channel(blue)),
        )
    }

    fn xterm_rgb(color: Color) -> (u8, u8, u8) {
        let named = match color {
            Color::Black | Color::Reset => 0,
            Color::Red => 1,
            Color::Green => 2,
            Color::Yellow => 3,
            Color::Blue => 4,
            Color::Magenta => 5,
            Color::Cyan => 6,
            Color::Gray => 7,
            Color::DarkGray => 8,
            Color::LightRed => 9,
            Color::LightGreen => 10,
            Color::LightYellow => 11,
            Color::LightBlue => 12,
            Color::LightMagenta => 13,
            Color::LightCyan => 14,
            Color::White => 15,
            Color::Rgb(red, green, blue) => return (red, green, blue),
            Color::Indexed(index) => return xterm_indexed_rgb(index),
        };
        XTERM_ANSI_COLORS[named]
    }

    fn xterm_indexed_rgb(index: u8) -> (u8, u8, u8) {
        if let Some(rgb) = XTERM_ANSI_COLORS.get(usize::from(index)) {
            return *rgb;
        }
        if index < 232 {
            const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
            let offset = index - 16;
            return (
                LEVELS[usize::from(offset / 36)],
                LEVELS[usize::from((offset % 36) / 6)],
                LEVELS[usize::from(offset % 6)],
            );
        }
        let gray = 8 + 10 * (index - 232);
        (gray, gray, gray)
    }

    fn contrast_ratio(foreground: Color, background: Color) -> f64 {
        let (foreground_red, foreground_green, foreground_blue) = xterm_rgb(foreground);
        let (background_red, background_green, background_blue) = xterm_rgb(background);
        let foreground = relative_luminance(foreground_red, foreground_green, foreground_blue);
        let background = relative_luminance(background_red, background_green, background_blue);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    fn for_each_pane_ground(theme: &Theme, mut check: impl FnMut(bool, bool, Color)) {
        let previous_theme = tui_pane::theme();
        let previous_transparency = tui_pane::transparent_background();
        let previous_tint = tui_pane::focused_pane_tint_enabled();
        tui_pane::set_active_theme(std::sync::Arc::new(theme.clone()));
        tui_pane::set_focused_pane_tint(true);
        for transparent in [false, true] {
            tui_pane::set_transparent_background(transparent);
            for focused in [false, true] {
                check(transparent, focused, tui_pane::pane_background(focused));
            }
        }
        tui_pane::set_active_theme(previous_theme);
        tui_pane::set_transparent_background(previous_transparency);
        tui_pane::set_focused_pane_tint(previous_tint);
    }

    #[test]
    fn every_family_colour_meets_contrast_on_both_grounds_in_every_built_in_theme() {
        for (name, theme) in super::super::tests::shipped_themes() {
            for_each_pane_ground(&theme, |transparent, focused, ground| {
                for index in 0..super::super::family_color_count() {
                    let ink = super::super::family_color(index);
                    let contrast = contrast_ratio(ink, ground);
                    assert!(
                        contrast >= MIN_STATUS_LINE_TEXT_CONTRAST,
                        "{name} family {index} {ink:?} on {ground:?}: {contrast:.2}:1, transparent={transparent}, focused={focused}"
                    );
                }
            });
        }
    }

    #[test]
    fn the_readout_count_meets_contrast_on_both_grounds_in_every_built_in_theme() {
        for variant in builtins() {
            for_each_pane_ground(&variant.theme, |transparent, focused, ground| {
                let ink = tui_pane::error_color();
                let contrast = contrast_ratio(ink, ground);
                assert!(
                    contrast >= MIN_STATUS_LINE_TEXT_CONTRAST,
                    "{} readout count and width {ink:?} on {ground:?}: {contrast:.2}:1, transparent={transparent}, focused={focused}",
                    variant.id.as_str()
                );
            });
        }
    }

    #[test]
    fn table_inks_meet_contrast_on_both_grounds_in_every_built_in_theme() {
        for variant in builtins() {
            for_each_pane_ground(&variant.theme, |transparent, focused, ground| {
                let inks = [
                    ("text", tui_pane::text_default()),
                    ("label", tui_pane::label_color()),
                    ("success", tui_pane::success_color()),
                    ("warning", tui_pane::warning_color()),
                    ("secondary text", tui_pane::secondary_text_color()),
                ];
                for (name, ink) in inks {
                    let contrast = contrast_ratio(ink, ground);
                    assert!(
                        contrast >= MIN_STATUS_LINE_TEXT_CONTRAST,
                        "{} table {name} {ink:?} on {ground:?}: {contrast:.2}:1, transparent={transparent}, focused={focused}",
                        variant.id.as_str()
                    );
                }
            });
        }
    }

    #[test]
    fn status_line_text_meets_contrast_in_every_built_in_theme() {
        for variant in builtins() {
            let theme = &variant.theme;
            let background = theme.status.bar.color;
            // These are exactly the lookups in `BarPalette::themed`:
            // fill and values, labels, activity/enabled keys, then
            // disabled keys and labels.
            let inks = [
                ("status fill and value", theme.text.default.color),
                ("status label", theme.pane_chrome.active_title.color),
                (
                    "status activity and enabled key",
                    theme.semantic.accent.color,
                ),
                ("status disabled key and label", theme.text.secondary.color),
            ];
            for (name, ink) in inks {
                let contrast = contrast_ratio(ink, background);
                assert!(
                    contrast >= MIN_STATUS_LINE_TEXT_CONTRAST,
                    "{} {name} contrast is {contrast:.2}:1",
                    variant.id.as_str()
                );
            }
        }
    }
}
