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

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::style::Color;
    use ratatui::widgets::Block;
    use tui_pane::ColorMode;
    use tui_pane::PaneFocusState;
    use tui_pane::Renderable;
    use tui_pane::ToastStyle;
    use tui_pane::ToastsRenderCtx;

    use crate::tui::app::App;
    use crate::tui::test_support;

    #[test]
    fn startup_page_footer_meets_contrast_in_every_registered_theme() -> anyhow::Result<()> {
        for toast_style in [
            ToastStyle::Normal,
            ToastStyle::Warning,
            ToastStyle::Error,
            ToastStyle::Success,
        ] {
            let mut app = test_support::project_app();
            let lines: Vec<_> = (0..20)
                .map(|index| format!("Startup task {index}"))
                .collect();
            if toast_style == ToastStyle::Normal {
                app.framework.toasts.push_colored_persistent(
                    "Startup",
                    lines,
                    vec![Color::Green; 20],
                );
            } else {
                app.framework.toasts.push_timed_styled(
                    "Startup",
                    lines.join("\n"),
                    Duration::from_secs(30),
                    1,
                    toast_style,
                );
            }
            let now = Instant::now() + Duration::from_secs(1);
            for variant in crate::themes::builtins() {
                let mut theme = variant.theme;
                super::apply_role_defaults_to_theme(
                    &mut theme,
                    Some(&variant.id),
                    variant.appearance,
                );
                let _theme = test_support::ThemeOverride::install(theme);
                for transparent in [false, true] {
                    tui_pane::set_transparent_background(transparent);
                    for mode in [ColorMode::Colored, ColorMode::NoColor] {
                        for width in [80, 95] {
                            let buffer = draw_startup_toast(&mut app, width, mode, now)?;
                            let mark = test_support::position(&buffer, "1 of")?;
                            let body = test_support::position(&buffer, "Startup task")?;
                            if mode == ColorMode::Colored && !transparent {
                                let failures = test_support::contrast_failures(
                                    &buffer,
                                    &["1 of", "▼"],
                                    variant.id.as_str(),
                                );
                                assert!(failures.is_empty(), "{}", failures.join("\n"));
                                assert_eq!(buffer[mark].fg, tui_pane::text_default());
                                assert_eq!(buffer[mark].bg, tui_pane::pane_background(false));
                                assert_eq!(buffer[body].bg, tui_pane::pane_background(false));
                                let body_color = match toast_style {
                                    ToastStyle::Normal => Color::Green,
                                    ToastStyle::Warning => tui_pane::warning_color(),
                                    ToastStyle::Error => tui_pane::error_color(),
                                    ToastStyle::Success => tui_pane::success_color(),
                                };
                                assert_eq!(buffer[body].fg, body_color);
                                if toast_style != ToastStyle::Normal {
                                    let countdown = test_support::position(&buffer, "Closing in")?;
                                    assert_eq!(buffer[countdown].fg, tui_pane::text_default());
                                }
                            } else if mode == ColorMode::Colored {
                                let footer_color = match toast_style {
                                    ToastStyle::Normal => tui_pane::text_default(),
                                    ToastStyle::Warning => Color::Yellow,
                                    ToastStyle::Error => Color::Red,
                                    ToastStyle::Success => Color::Green,
                                };
                                assert_eq!(buffer[mark].fg, footer_color);
                                assert_eq!(buffer[mark].bg, Color::Reset);
                                assert_eq!(buffer[body].bg, Color::Reset);
                            } else {
                                assert_eq!(buffer[mark].fg, Color::Reset);
                                assert_eq!(buffer[mark].bg, Color::Reset);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn draw_startup_toast(
        app: &mut App,
        width: u16,
        mode: ColorMode,
        now: Instant,
    ) -> anyhow::Result<Buffer> {
        app.framework.color_mode = mode;
        let mut terminal = Terminal::new(TestBackend::new(width, 13))?;
        terminal.draw(|frame| {
            frame.render_widget(
                Block::default().style(ratatui::style::Style::default().bg(Color::Black)),
                frame.area(),
            );
            Renderable::render(
                &mut app.framework.toasts,
                frame,
                frame.area(),
                &ToastsRenderCtx {
                    now,
                    pane_focus_state: PaneFocusState::Inactive,
                },
            );
            mode.clear_colors(frame.buffer_mut());
        })?;
        Ok(terminal.backend().buffer().clone())
    }
}
