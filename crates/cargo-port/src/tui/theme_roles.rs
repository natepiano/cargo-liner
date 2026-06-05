use ratatui::style::Color;
use tui_pane::Appearance;
use tui_pane::StyleSpec;
use tui_pane::Theme;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;

const COLUMN_HEADER: &str = "cargo-port.column.header";
const DISCOVERY_SHIMMER: &str = "cargo-port.discovery.shimmer";
const GIT_IGNORED: &str = "cargo-port.git.ignored";
const GIT_MODIFIED: &str = "cargo-port.git.modified";
const GIT_UNTRACKED: &str = "cargo-port.git.untracked";
const TARGET_BENCH: &str = "cargo-port.target.bench";

const ROLE_KEYS: [&str; 6] = [
    COLUMN_HEADER,
    DISCOVERY_SHIMMER,
    GIT_IGNORED,
    GIT_MODIFIED,
    GIT_UNTRACKED,
    TARGET_BENCH,
];

#[derive(Clone, Copy)]
enum RolePalette {
    DefaultDark,
    DefaultLight,
    HighContrastDark,
    HighContrastLight,
}

pub(crate) fn apply_role_defaults_to_registry(registry: &mut ThemeRegistry) {
    registry.update_themes(|id, appearance, theme| {
        apply_role_defaults_to_theme(theme, Some(id), appearance);
    });
}

pub(crate) fn apply_role_defaults_to_theme(
    theme: &mut Theme,
    id: Option<&ThemeId>,
    appearance: Appearance,
) {
    let palette = palette_for(id, appearance);
    for role in ROLE_KEYS {
        theme
            .roles
            .entry(String::from(role))
            .or_insert_with(|| default_role(role, palette));
    }
}

pub(crate) fn column_header_color() -> Color { role_color(COLUMN_HEADER) }

pub(crate) fn discovery_shimmer_color() -> Color { role_color(DISCOVERY_SHIMMER) }

pub(crate) fn git_ignored_color() -> Color { role_color(GIT_IGNORED) }

pub(crate) fn git_modified_color() -> Color { role_color(GIT_MODIFIED) }

pub(crate) fn git_untracked_color() -> Color { role_color(GIT_UNTRACKED) }

pub(crate) fn target_bench_color() -> Color { role_color(TARGET_BENCH) }

fn role_color(role: &str) -> Color {
    tui_pane::role_color(role, default_role(role, RolePalette::DefaultDark))
}

fn palette_for(id: Option<&ThemeId>, appearance: Appearance) -> RolePalette {
    match id.map(ThemeId::as_str) {
        Some(tui_pane::BUILTIN_HC_DARK_NAME) => RolePalette::HighContrastDark,
        Some(tui_pane::BUILTIN_HC_LIGHT_NAME) => RolePalette::HighContrastLight,
        _ => match appearance {
            Appearance::Dark => RolePalette::DefaultDark,
            Appearance::Light => RolePalette::DefaultLight,
        },
    }
}

fn default_role(role: &str, palette: RolePalette) -> StyleSpec {
    match (role, palette) {
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
        (TARGET_BENCH, RolePalette::DefaultDark) => StyleSpec::from_color(Color::Magenta),
        (TARGET_BENCH, RolePalette::DefaultLight) => StyleSpec::from_color(Color::Rgb(140, 0, 140)),
        (TARGET_BENCH, RolePalette::HighContrastDark) => StyleSpec::bold(Color::LightMagenta),
        (TARGET_BENCH, RolePalette::HighContrastLight) => StyleSpec::bold(Color::Rgb(140, 0, 140)),
        _ => StyleSpec::from_color(Color::Reset),
    }
}

const fn bold(color: Color) -> StyleSpec { StyleSpec::bold(color) }
