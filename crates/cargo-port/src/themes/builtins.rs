//! cargo-port's compiled-in theme variants.
//!
//! `tui_pane` supplies the theme machinery — the types, the registry,
//! the resolver, the directory watch — and none of the colors. Every
//! palette the app ships lives here, so a change to one app's look
//! cannot move another's.
//!
//! [`builtins`] is what startup hands to
//! [`ThemeRegistry::from_dir_with_builtins`](tui_pane::ThemeRegistry::from_dir_with_builtins);
//! user `themes/*.toml`
//! variants layer on top, replacing a built-in when the names match.
//! The `cargo-port/themes/*.toml` templates mirror these constructors
//! as copyable documentation, locked against drift by the tests at the
//! bottom of this file.

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

/// The variants cargo-port compiles in, in the order the settings UI
/// offers them.
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

/// Default dark variant — cargo-port's shipped palette for dark
/// terminals, and the value [`DEFAULT_DARK_THEME`] names.
#[must_use]
const fn default_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::from_color(Color::Yellow)),
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
            error:        StyleSpec::from_color(Color::Red),
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
            bar: StyleSpec::from_color(Color::DarkGray),
        },
        finder:      FinderTheme {
            match_bg: StyleSpec::from_color(Color::Rgb(0, 90, 100)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::from_color(Color::Rgb(100, 220, 100)),
            mid:  StyleSpec::from_color(Color::Rgb(255, 255, 255)),
            high: StyleSpec::from_color(Color::Rgb(255, 100, 100)),
        },
        roles:       BTreeMap::new(),
    }
}

/// Default light variant — each value picked for legibility on a
/// white terminal background.
#[must_use]
const fn default_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::from_color(Color::Rgb(180, 120, 0))),
            inactive_border: StyleSpec::from_color(Color::Rgb(140, 140, 140)),
            active_title:    StyleSpec::bold(Color::Rgb(160, 100, 0)),
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
            inline_error: StyleSpec::from_color(Color::Rgb(180, 95, 0)),
            success:      StyleSpec::from_color(Color::Rgb(0, 120, 0)),
            label:        StyleSpec::from_color(Color::Rgb(60, 100, 90)),
            warning:      StyleSpec::from_color(Color::Rgb(180, 95, 0)),
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
        roles:       BTreeMap::new(),
    }
}

/// High-contrast dark variant, named by [`DEFAULT_HC_DARK_THEME`].
///
/// Pure white on pure black with bold modifiers throughout; accent
/// fields use the bright ANSI palette (`LightYellow`, `LightCyan`,
/// `LightGreen`, `LightRed`, `LightMagenta`) for maximum legibility
/// under reduced-vision or glare conditions.
#[must_use]
const fn high_contrast_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::bold(Color::LightYellow)),
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
            error:        StyleSpec::bold(Color::LightRed),
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
        roles:       BTreeMap::new(),
    }
}

/// High-contrast light variant, named by [`DEFAULT_HC_LIGHT_THEME`].
///
/// Pure black on pure white with bold modifiers throughout; accent
/// fields use saturated dark colors (deep red, deep green, deep blue,
/// deep orange) chosen for AAA-grade contrast against a white canvas.
#[must_use]
const fn high_contrast_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::bold(Color::Rgb(140, 60, 0))),
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
        roles:       BTreeMap::new(),
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

    const DARK_TEMPLATE: &str = include_str!("../../themes/default_dark.toml");
    const HC_TEMPLATE: &str = include_str!("../../themes/high_contrast.toml");
    const LIGHT_TEMPLATE: &str = include_str!("../../themes/default_light.toml");

    /// Parse one template and return its variants, asserting the
    /// schema version and the expected variant count.
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
}
