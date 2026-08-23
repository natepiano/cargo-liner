//! Last-resort palette for when no app variant is reachable.
//!
//! Theme *content* belongs to the app, not to the framework: each app
//! compiles in its own variants and hands them to
//! [`ThemeRegistry::new_with_builtins`](super::ThemeRegistry::new_with_builtins).
//! Nothing here is a designed theme, and no app should select it. It
//! exists for the two paths where the registry has nothing to offer —
//! a [`ThemeState`](super::ThemeState) installed before any app
//! variants are registered (render tests that skip startup), and a
//! configured theme id that matches nothing with no variant of the
//! right appearance to stand in.
//!
//! Deliberately plain: grays and the base ANSI palette, legible on any
//! terminal, distinctive enough that landing on it looks like the
//! misconfiguration it is.

use std::collections::BTreeMap;

use ratatui::style::Color;

use super::Appearance;
use super::DiskUsageTheme;
use super::FinderTheme;
use super::FocusTheme;
use super::PaneChromeTheme;
use super::SemanticTheme;
use super::StatusTheme;
use super::StyleSpec;
use super::TextTheme;
use super::Theme;

/// Neutral palette for `appearance`, used only where no registered
/// variant applies. See the module docs for the paths that reach it.
#[must_use]
pub const fn fallback_theme(appearance: Appearance) -> Theme {
    match appearance {
        Appearance::Dark => fallback_dark(),
        Appearance::Light => fallback_light(),
    }
}

/// Neutral dark palette: white on the terminal's own background.
const fn fallback_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::from_color(Color::White)),
            inactive_border: StyleSpec::from_color(Color::DarkGray),
            active_title:    StyleSpec::bold(Color::White),
            inactive_title:  StyleSpec::from_color(Color::Gray),
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
            label:        StyleSpec::from_color(Color::Gray),
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
            low:  StyleSpec::from_color(Color::Green),
            mid:  StyleSpec::from_color(Color::White),
            high: StyleSpec::from_color(Color::Red),
        },
        roles:       BTreeMap::new(),
    }
}

/// Neutral light palette: black on the terminal's own background.
const fn fallback_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   Some(StyleSpec::from_color(Color::Black)),
            inactive_border: StyleSpec::from_color(Color::Rgb(140, 140, 140)),
            active_title:    StyleSpec::bold(Color::Black),
            inactive_title:  StyleSpec::from_color(Color::Rgb(70, 70, 70)),
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
            label:        StyleSpec::from_color(Color::Rgb(90, 90, 90)),
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
