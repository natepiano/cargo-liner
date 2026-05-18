//! Compiled-in default themes.
//!
//! Values mirror the Phase 1 audit in `docs/themes.md`. The starter
//! TOML templates in `tui_pane/themes/` round-trip to these
//! constructors; the test in `tui_pane/tests/themes.rs` locks it.

use ratatui::style::Color;

use super::DiskUsageTheme;
use super::FinderTheme;
use super::FocusTheme;
use super::GitTheme;
use super::Modifiers;
use super::PaneChromeTheme;
use super::SemanticTheme;
use super::StatusTheme;
use super::StyleSpec;
use super::TextTheme;
use super::Theme;

/// Built-in dark variant. Matches the pre-theme constant values
/// audited in `docs/themes.md` so the migration is behavior-preserving.
#[must_use]
pub const fn default_dark() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   StyleSpec::from_color(Color::Yellow),
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
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::White),
            secondary: StyleSpec::from_color(Color::Gray),
            dim:       StyleSpec::from_color(Color::DarkGray),
            bright:    StyleSpec::from_color(Color::Cyan),
            bg_focus:  StyleSpec::from_color(Color::Black),
        },
        git:         GitTheme {
            ignored:   StyleSpec::from_color(Color::DarkGray),
            modified:  StyleSpec::from_color(Color::Indexed(208)),
            untracked: StyleSpec::from_color(Color::Green),
        },
        status:      StatusTheme {
            bar:           StyleSpec::from_color(Color::DarkGray),
            target_bench:  StyleSpec::from_color(Color::Magenta),
            column_header: StyleSpec {
                color:     Color::Rgb(150, 190, 180),
                modifiers: Modifiers {
                    bold:      true,
                    italic:    false,
                    dim:       false,
                    underline: false,
                },
            },
        },
        finder:      FinderTheme {
            match_bg:          StyleSpec::from_color(Color::Rgb(0, 90, 100)),
            discovery_shimmer: StyleSpec::from_color(Color::Rgb(150, 210, 255)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::from_color(Color::Rgb(100, 220, 100)),
            mid:  StyleSpec::from_color(Color::Rgb(255, 255, 255)),
            high: StyleSpec::from_color(Color::Rgb(255, 100, 100)),
        },
    }
}

/// Built-in light variant. Picks each value for legibility on a white
/// terminal background per the `docs/themes.md` design table.
#[must_use]
pub const fn default_light() -> Theme {
    Theme {
        pane_chrome: PaneChromeTheme {
            active_border:   StyleSpec::from_color(Color::Rgb(180, 120, 0)),
            inactive_border: StyleSpec::from_color(Color::Gray),
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
        },
        text:        TextTheme {
            default:   StyleSpec::from_color(Color::Black),
            secondary: StyleSpec::from_color(Color::Rgb(70, 70, 70)),
            dim:       StyleSpec::from_color(Color::Rgb(130, 130, 130)),
            bright:    StyleSpec::from_color(Color::Rgb(0, 95, 135)),
            bg_focus:  StyleSpec::from_color(Color::White),
        },
        git:         GitTheme {
            ignored:   StyleSpec::from_color(Color::Rgb(150, 150, 150)),
            modified:  StyleSpec::from_color(Color::Indexed(208)),
            untracked: StyleSpec::from_color(Color::Rgb(0, 120, 0)),
        },
        status:      StatusTheme {
            bar:           StyleSpec::from_color(Color::Rgb(220, 220, 220)),
            target_bench:  StyleSpec::from_color(Color::Rgb(140, 0, 140)),
            column_header: StyleSpec {
                color:     Color::Rgb(60, 100, 90),
                modifiers: Modifiers {
                    bold:      true,
                    italic:    false,
                    dim:       false,
                    underline: false,
                },
            },
        },
        finder:      FinderTheme {
            match_bg:          StyleSpec::from_color(Color::Rgb(255, 245, 180)),
            discovery_shimmer: StyleSpec::from_color(Color::Rgb(120, 140, 200)),
        },
        disk_usage:  DiskUsageTheme {
            low:  StyleSpec::from_color(Color::Rgb(0, 140, 0)),
            mid:  StyleSpec::from_color(Color::Rgb(90, 90, 90)),
            high: StyleSpec::from_color(Color::Rgb(200, 0, 0)),
        },
    }
}
