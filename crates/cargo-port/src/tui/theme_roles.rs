use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::style::Style;
use tui_pane::Appearance;
use tui_pane::ColorMode;
use tui_pane::DIM_VALUE_ROLE;
use tui_pane::PaneSelectionState;
use tui_pane::StyleSpec;
use tui_pane::Theme;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;

use super::constants::COLUMN_HEADER;
use super::constants::CPU_BAR_LABEL;
use super::constants::DISCOVERY_SHIMMER;
use super::constants::GIT_IGNORED;
use super::constants::GIT_MODIFIED;
use super::constants::GIT_UNTRACKED;
use super::constants::LANGUAGE_SUBTOTAL;
use super::constants::ROLE_KEYS;
use super::constants::SELECTION_TEXT;
use super::constants::TARGET_BENCH;
use crate::constants::DEFAULT_HC_DARK_THEME;
use crate::constants::DEFAULT_HC_LIGHT_THEME;

#[derive(Clone, Copy)]
enum RolePalette {
    DefaultDark,
    DefaultLight,
    HighContrastDark,
    HighContrastLight,
}

pub(super) fn apply_role_defaults_to_registry(theme_registry: &mut ThemeRegistry) {
    theme_registry.update_themes(|theme_id, appearance, theme| {
        apply_role_defaults_to_theme(theme, Some(theme_id), appearance);
    });
}

pub(super) fn apply_role_defaults_to_theme(
    theme: &mut Theme,
    theme_id: Option<&ThemeId>,
    appearance: Appearance,
) {
    let role_palette = palette_for(theme_id, appearance);
    for role in ROLE_KEYS {
        theme
            .roles
            .entry(String::from(role))
            .or_insert_with(|| default_role(role, role_palette));
    }
}

pub(super) fn cpu_bar_label_color() -> Color { role_color(CPU_BAR_LABEL) }

pub(super) fn column_header_color() -> Color { role_color(COLUMN_HEADER) }

pub(super) fn discovery_shimmer_color() -> Color { role_color(DISCOVERY_SHIMMER) }

pub(super) fn git_ignored_color() -> Color { role_color(GIT_IGNORED) }

pub(super) fn git_modified_color() -> Color { role_color(GIT_MODIFIED) }

pub(super) fn git_untracked_color() -> Color { role_color(GIT_UNTRACKED) }

pub(super) fn language_subtotal_color() -> Color { role_color(LANGUAGE_SUBTOTAL) }

pub(super) fn target_bench_color() -> Color { role_color(TARGET_BENCH) }

pub(super) fn selection_style(selection: PaneSelectionState, mode: ColorMode) -> Style {
    let style = selection.overlay_style(mode);
    match selection {
        PaneSelectionState::Active
        | PaneSelectionState::Remembered
        | PaneSelectionState::Hovered => style.fg(role_color(SELECTION_TEXT)),
        PaneSelectionState::Unselected => style,
    }
}

/// Apply selection after the row's text and cell styles.
pub(super) fn paint_selection(
    frame: &mut Frame,
    area: Rect,
    selection: PaneSelectionState,
    mode: ColorMode,
) {
    if !matches!(selection, PaneSelectionState::Unselected) {
        frame
            .buffer_mut()
            .set_style(area, selection_style(selection, mode));
    }
}

fn role_color(role: &str) -> Color {
    tui_pane::role_color(role, default_role(role, RolePalette::DefaultDark))
}

fn palette_for(theme_id: Option<&ThemeId>, appearance: Appearance) -> RolePalette {
    match theme_id.map(ThemeId::as_str) {
        Some(DEFAULT_HC_DARK_THEME) => RolePalette::HighContrastDark,
        Some(DEFAULT_HC_LIGHT_THEME) => RolePalette::HighContrastLight,
        _ => match appearance {
            Appearance::Dark => RolePalette::DefaultDark,
            Appearance::Light => RolePalette::DefaultLight,
        },
    }
}

fn default_role(role: &str, role_palette: RolePalette) -> StyleSpec {
    match (role, role_palette) {
        (DIM_VALUE_ROLE, RolePalette::DefaultDark) => {
            StyleSpec::from_color(Color::Rgb(160, 160, 160))
        },
        (DIM_VALUE_ROLE, RolePalette::DefaultLight) => {
            StyleSpec::from_color(Color::Rgb(98, 98, 98))
        },
        (DIM_VALUE_ROLE, RolePalette::HighContrastDark) => {
            StyleSpec::from_color(Color::Rgb(229, 229, 229))
        },
        (DIM_VALUE_ROLE, RolePalette::HighContrastLight) => {
            StyleSpec::from_color(Color::Rgb(90, 90, 90))
        },
        (SELECTION_TEXT, RolePalette::DefaultDark | RolePalette::HighContrastDark) => {
            StyleSpec::from_color(Color::Rgb(255, 255, 255))
        },
        (SELECTION_TEXT, RolePalette::DefaultLight | RolePalette::HighContrastLight) => {
            StyleSpec::from_color(Color::Black)
        },
        (CPU_BAR_LABEL, _) => StyleSpec::from_color(Color::Black),
        (COLUMN_HEADER, RolePalette::DefaultDark) => bold(Color::Rgb(150, 190, 180)),
        (COLUMN_HEADER, RolePalette::DefaultLight) => bold(Color::Rgb(60, 100, 90)),
        (COLUMN_HEADER | DISCOVERY_SHIMMER, RolePalette::HighContrastDark) => {
            StyleSpec::bold(Color::LightCyan)
        },
        (COLUMN_HEADER, RolePalette::HighContrastLight) => StyleSpec::bold(Color::Rgb(0, 0, 140)),
        (DISCOVERY_SHIMMER, RolePalette::DefaultDark) => {
            StyleSpec::from_color(Color::Rgb(150, 210, 255))
        },
        (DISCOVERY_SHIMMER, RolePalette::DefaultLight) => {
            StyleSpec::from_color(Color::Rgb(120, 140, 200))
        },
        (DISCOVERY_SHIMMER, RolePalette::HighContrastLight) => {
            StyleSpec::bold(Color::Rgb(0, 0, 140))
        },
        (GIT_IGNORED, RolePalette::DefaultDark) => StyleSpec::from_color(Color::DarkGray),
        (GIT_IGNORED, RolePalette::DefaultLight) => {
            StyleSpec::from_color(Color::Rgb(150, 150, 150))
        },
        (GIT_IGNORED, RolePalette::HighContrastDark) => StyleSpec::from_color(Color::Gray),
        (GIT_IGNORED, RolePalette::HighContrastLight) => {
            StyleSpec::from_color(Color::Rgb(80, 80, 80))
        },
        (GIT_MODIFIED, RolePalette::DefaultDark | RolePalette::DefaultLight) => {
            StyleSpec::from_color(Color::Indexed(208))
        },
        (GIT_MODIFIED, RolePalette::HighContrastDark) => StyleSpec::bold(Color::LightYellow),
        (GIT_MODIFIED, RolePalette::HighContrastLight) => StyleSpec::bold(Color::Rgb(140, 60, 0)),
        (GIT_UNTRACKED, RolePalette::DefaultDark) => StyleSpec::from_color(Color::Green),
        (GIT_UNTRACKED, RolePalette::DefaultLight) => StyleSpec::from_color(Color::Rgb(0, 120, 0)),
        (GIT_UNTRACKED, RolePalette::HighContrastDark) => StyleSpec::bold(Color::LightGreen),
        (GIT_UNTRACKED, RolePalette::HighContrastLight) => StyleSpec::bold(Color::Rgb(0, 100, 0)),
        (LANGUAGE_SUBTOTAL, RolePalette::DefaultDark) => {
            StyleSpec::from_color(Color::Rgb(120, 190, 210))
        },
        (LANGUAGE_SUBTOTAL, RolePalette::DefaultLight) => {
            StyleSpec::from_color(Color::Rgb(35, 110, 130))
        },
        (LANGUAGE_SUBTOTAL, RolePalette::HighContrastDark) => {
            StyleSpec::from_color(Color::LightCyan)
        },
        (LANGUAGE_SUBTOTAL, RolePalette::HighContrastLight) => {
            StyleSpec::bold(Color::Rgb(0, 85, 120))
        },
        (TARGET_BENCH, RolePalette::DefaultDark) => StyleSpec::from_color(Color::Magenta),
        (TARGET_BENCH, RolePalette::DefaultLight) => StyleSpec::from_color(Color::Rgb(140, 0, 140)),
        (TARGET_BENCH, RolePalette::HighContrastDark) => StyleSpec::bold(Color::LightMagenta),
        (TARGET_BENCH, RolePalette::HighContrastLight) => StyleSpec::bold(Color::Rgb(140, 0, 140)),
        _ => StyleSpec::from_color(Color::Reset),
    }
}

const fn bold(color: Color) -> StyleSpec { StyleSpec::bold(color) }
