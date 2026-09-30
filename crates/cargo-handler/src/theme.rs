//! cargo-handler's compiled-in theme variants.
//!
//! `tui_pane` supplies the theme machinery — the types, the registry,
//! the resolver, the directory watch — and none of the colors. Every
//! palette this app ships lives here, so retuning cargo-handler's grid
//! cannot move another app's panes.
//!
//! The summary's own colors are theme roles: [`Role`] names each one,
//! and every built-in variant carries a value for each. Among them is
//! the rainbow the agent cells take in turn, one [`RainbowHue`] a cell,
//! which draws both the cell's title and its agent's name in the
//! summary.
//!
//! [`builtins`] is what startup hands to [`tui_pane::install_theme`];
//! user `themes/*.toml` variants layer on top, replacing a built-in
//! when the names match. The `cargo-handler/themes/*.toml` templates
//! mirror these constructors as copyable documentation, locked against
//! drift by the tests at the bottom of this file.

use std::collections::BTreeMap;

use ratatui::style::Color;
use ratatui::style::Style;
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

use crate::constants::BUSY_ROLE;
use crate::constants::CLAUDE_ROLE;
use crate::constants::CODEX_ROLE;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_HC_DARK_THEME;
use crate::constants::DEFAULT_HC_LIGHT_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::IDLE_ROLE;
use crate::constants::RAINBOW_BLUE_ROLE;
use crate::constants::RAINBOW_CYAN_ROLE;
use crate::constants::RAINBOW_GREEN_ROLE;
use crate::constants::RAINBOW_ORANGE_ROLE;
use crate::constants::RAINBOW_RED_ROLE;
use crate::constants::RAINBOW_VIOLET_ROLE;
use crate::constants::RAINBOW_YELLOW_ROLE;
use crate::constants::SHELL_ROLE;
use crate::constants::UNREACHABLE_ROLE;

/// The summary's colors, each read from the active variant's
/// `[variants.roles]` under [`Role::key`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Role {
    /// The `claude` agent label.
    Claude,
    /// The `codex` agent label.
    Codex,
    /// A `busy` status.
    Busy,
    /// A `shell` status.
    Shell,
    /// An `idle` status, a status the summary does not know, and a
    /// missing one.
    Idle,
    /// Why a remote machine gave no answer.
    Unreachable,
    /// An agent cell's title and its agent's name in the summary.
    Rainbow(RainbowHue),
}

impl Role {
    /// Every role, in the order [`RolePalette`] lists them.
    const ALL: [Self; 13] = [
        Self::Claude,
        Self::Codex,
        Self::Busy,
        Self::Shell,
        Self::Idle,
        Self::Unreachable,
        Self::Rainbow(RainbowHue::Red),
        Self::Rainbow(RainbowHue::Orange),
        Self::Rainbow(RainbowHue::Yellow),
        Self::Rainbow(RainbowHue::Green),
        Self::Rainbow(RainbowHue::Cyan),
        Self::Rainbow(RainbowHue::Blue),
        Self::Rainbow(RainbowHue::Violet),
    ];

    /// The role's key in `[variants.roles]`.
    const fn key(self) -> &'static str {
        match self {
            Self::Claude => CLAUDE_ROLE,
            Self::Codex => CODEX_ROLE,
            Self::Busy => BUSY_ROLE,
            Self::Shell => SHELL_ROLE,
            Self::Idle => IDLE_ROLE,
            Self::Unreachable => UNREACHABLE_ROLE,
            Self::Rainbow(hue) => hue.key(),
        }
    }

    /// The role's style in the active variant. A variant without the
    /// role gets [`DEFAULT_DARK_ROLES`]' value.
    pub(crate) fn style(self) -> Style {
        tui_pane::role_style(self.key(), DEFAULT_DARK_ROLES.spec(self))
    }
}

/// One color of the rainbow the agent cells take in turn, in cell
/// order: a cell's title and its agent's name in the summary are drawn
/// in its hue, so the two can be paired at a glance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RainbowHue {
    /// The first cell's hue.
    Red,
    /// The second cell's hue.
    Orange,
    /// The third cell's hue.
    Yellow,
    /// The fourth cell's hue.
    Green,
    /// The fifth cell's hue.
    Cyan,
    /// The sixth cell's hue.
    Blue,
    /// The seventh cell's hue; the eighth starts over at [`Self::Red`].
    Violet,
}

impl RainbowHue {
    /// Every hue, in the order the cells take them.
    const ALL: [Self; 7] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Cyan,
        Self::Blue,
        Self::Violet,
    ];

    /// The hue of the agent at `position` in cell order, counted from
    /// zero and starting over at red past violet.
    pub(crate) const fn of_agent(position: usize) -> Self { Self::ALL[position % Self::ALL.len()] }

    /// The hue's key in `[variants.roles]`.
    const fn key(self) -> &'static str {
        match self {
            Self::Red => RAINBOW_RED_ROLE,
            Self::Orange => RAINBOW_ORANGE_ROLE,
            Self::Yellow => RAINBOW_YELLOW_ROLE,
            Self::Green => RAINBOW_GREEN_ROLE,
            Self::Cyan => RAINBOW_CYAN_ROLE,
            Self::Blue => RAINBOW_BLUE_ROLE,
            Self::Violet => RAINBOW_VIOLET_ROLE,
        }
    }
}

/// One built-in variant's value for each [`RainbowHue`].
struct RainbowPalette {
    /// [`RainbowHue::Red`].
    red:    StyleSpec,
    /// [`RainbowHue::Orange`].
    orange: StyleSpec,
    /// [`RainbowHue::Yellow`].
    yellow: StyleSpec,
    /// [`RainbowHue::Green`].
    green:  StyleSpec,
    /// [`RainbowHue::Cyan`].
    cyan:   StyleSpec,
    /// [`RainbowHue::Blue`].
    blue:   StyleSpec,
    /// [`RainbowHue::Violet`].
    violet: StyleSpec,
}

impl RainbowPalette {
    /// The value for `hue`.
    const fn spec(&self, hue: RainbowHue) -> StyleSpec {
        match hue {
            RainbowHue::Red => self.red,
            RainbowHue::Orange => self.orange,
            RainbowHue::Yellow => self.yellow,
            RainbowHue::Green => self.green,
            RainbowHue::Cyan => self.cyan,
            RainbowHue::Blue => self.blue,
            RainbowHue::Violet => self.violet,
        }
    }
}

/// One built-in variant's value for each [`Role`].
struct RolePalette {
    /// [`Role::Claude`].
    claude:      StyleSpec,
    /// [`Role::Codex`].
    codex:       StyleSpec,
    /// [`Role::Busy`].
    busy:        StyleSpec,
    /// [`Role::Shell`].
    shell:       StyleSpec,
    /// [`Role::Idle`].
    idle:        StyleSpec,
    /// [`Role::Unreachable`].
    unreachable: StyleSpec,
    /// [`Role::Rainbow`], one value per hue.
    rainbow:     RainbowPalette,
}

impl RolePalette {
    /// The value for `role`.
    const fn spec(&self, role: Role) -> StyleSpec {
        match role {
            Role::Claude => self.claude,
            Role::Codex => self.codex,
            Role::Busy => self.busy,
            Role::Shell => self.shell,
            Role::Idle => self.idle,
            Role::Unreachable => self.unreachable,
            Role::Rainbow(hue) => self.rainbow.spec(hue),
        }
    }

    /// Every role's value, keyed as `[variants.roles]` keys it.
    fn map(&self) -> BTreeMap<String, StyleSpec> {
        Role::ALL
            .into_iter()
            .map(|role| (role.key().to_string(), self.spec(role)))
            .collect()
    }
}

/// [`default_dark`]'s roles, and the value a variant without a role
/// falls back to.
const DEFAULT_DARK_ROLES: RolePalette = RolePalette {
    claude:      StyleSpec::from_color(Color::Rgb(217, 119, 87)),
    codex:       StyleSpec::from_color(Color::Rgb(175, 140, 255)),
    busy:        StyleSpec::from_color(Color::Rgb(100, 220, 100)),
    shell:       StyleSpec::from_color(Color::Rgb(90, 200, 220)),
    idle:        StyleSpec::from_color(Color::Rgb(140, 140, 140)),
    unreachable: StyleSpec::from_color(Color::Rgb(255, 100, 100)),
    rainbow:     RainbowPalette {
        red:    StyleSpec::from_color(Color::Rgb(255, 95, 95)),
        orange: StyleSpec::from_color(Color::Rgb(255, 165, 70)),
        yellow: StyleSpec::from_color(Color::Rgb(240, 220, 80)),
        green:  StyleSpec::from_color(Color::Rgb(110, 220, 110)),
        cyan:   StyleSpec::from_color(Color::Rgb(80, 210, 230)),
        blue:   StyleSpec::from_color(Color::Rgb(100, 150, 255)),
        violet: StyleSpec::from_color(Color::Rgb(190, 130, 255)),
    },
};

/// [`default_light`]'s roles: the same hues, darker, for a white
/// background.
const DEFAULT_LIGHT_ROLES: RolePalette = RolePalette {
    claude:      StyleSpec::from_color(Color::Rgb(190, 85, 50)),
    codex:       StyleSpec::from_color(Color::Rgb(110, 60, 200)),
    busy:        StyleSpec::from_color(Color::Rgb(0, 130, 0)),
    shell:       StyleSpec::from_color(Color::Rgb(0, 120, 150)),
    idle:        StyleSpec::from_color(Color::Rgb(120, 120, 120)),
    unreachable: StyleSpec::from_color(Color::Rgb(190, 0, 0)),
    rainbow:     RainbowPalette {
        red:    StyleSpec::from_color(Color::Rgb(200, 30, 30)),
        orange: StyleSpec::from_color(Color::Rgb(180, 85, 0)),
        yellow: StyleSpec::from_color(Color::Rgb(135, 105, 0)),
        green:  StyleSpec::from_color(Color::Rgb(0, 135, 0)),
        cyan:   StyleSpec::from_color(Color::Rgb(0, 120, 140)),
        blue:   StyleSpec::from_color(Color::Rgb(30, 80, 210)),
        violet: StyleSpec::from_color(Color::Rgb(130, 50, 200)),
    },
};

/// [`high_contrast_dark`]'s roles: bright and bold, save `idle`, which
/// stays quiet.
const HIGH_CONTRAST_DARK_ROLES: RolePalette = RolePalette {
    claude:      StyleSpec::bold(Color::Rgb(255, 150, 90)),
    codex:       StyleSpec::bold(Color::LightMagenta),
    busy:        StyleSpec::bold(Color::LightGreen),
    shell:       StyleSpec::bold(Color::LightCyan),
    idle:        StyleSpec::from_color(Color::Gray),
    unreachable: StyleSpec::bold(Color::LightRed),
    rainbow:     RainbowPalette {
        red:    StyleSpec::bold(Color::LightRed),
        orange: StyleSpec::bold(Color::Rgb(255, 170, 60)),
        yellow: StyleSpec::bold(Color::LightYellow),
        green:  StyleSpec::bold(Color::LightGreen),
        cyan:   StyleSpec::bold(Color::LightCyan),
        blue:   StyleSpec::bold(Color::LightBlue),
        violet: StyleSpec::bold(Color::LightMagenta),
    },
};

/// [`high_contrast_light`]'s roles: deep and bold, save `idle`, which
/// stays quiet.
const HIGH_CONTRAST_LIGHT_ROLES: RolePalette = RolePalette {
    claude:      StyleSpec::bold(Color::Rgb(160, 60, 0)),
    codex:       StyleSpec::bold(Color::Rgb(100, 0, 160)),
    busy:        StyleSpec::bold(Color::Rgb(0, 100, 0)),
    shell:       StyleSpec::bold(Color::Rgb(0, 80, 120)),
    idle:        StyleSpec::from_color(Color::Rgb(80, 80, 80)),
    unreachable: StyleSpec::bold(Color::Rgb(180, 0, 0)),
    rainbow:     RainbowPalette {
        red:    StyleSpec::bold(Color::Rgb(180, 0, 0)),
        orange: StyleSpec::bold(Color::Rgb(150, 60, 0)),
        yellow: StyleSpec::bold(Color::Rgb(110, 85, 0)),
        green:  StyleSpec::bold(Color::Rgb(0, 100, 0)),
        cyan:   StyleSpec::bold(Color::Rgb(0, 95, 115)),
        blue:   StyleSpec::bold(Color::Rgb(0, 0, 170)),
        violet: StyleSpec::bold(Color::Rgb(100, 0, 160)),
    },
};

/// The variants cargo-handler compiles in, in the order the settings
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
        roles:       DEFAULT_DARK_ROLES.map(),
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
        roles:       DEFAULT_LIGHT_ROLES.map(),
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
            error:        StyleSpec::bold(Color::LightRed),
            inline_error: StyleSpec::bold(Color::LightYellow),
            success:      StyleSpec::bold(Color::LightGreen),
            label:        StyleSpec::from_color(Color::Rgb(175, 215, 200)),
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
        roles:       HIGH_CONTRAST_DARK_ROLES.map(),
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
            inline_error: StyleSpec::bold(Color::Rgb(140, 60, 0)),
            success:      StyleSpec::bold(Color::Rgb(0, 100, 0)),
            label:        StyleSpec::from_color(Color::Rgb(40, 85, 75)),
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
        roles:       HIGH_CONTRAST_LIGHT_ROLES.map(),
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

    const DARK_TEMPLATE: &str = include_str!("../themes/default_dark.toml");
    const HC_TEMPLATE: &str = include_str!("../themes/high_contrast.toml");
    const LIGHT_TEMPLATE: &str = include_str!("../themes/default_light.toml");
    const STARTER_TEMPLATE: &str = include_str!("../themes/starter.toml");

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

    /// The agents take the rainbow red to violet, and the eighth starts
    /// over at red.
    #[test]
    fn agents_take_the_rainbow_in_order_and_start_over_past_violet() {
        let hues: Vec<RainbowHue> = (0..9).map(RainbowHue::of_agent).collect();

        assert_eq!(
            hues,
            [
                RainbowHue::Red,
                RainbowHue::Orange,
                RainbowHue::Yellow,
                RainbowHue::Green,
                RainbowHue::Cyan,
                RainbowHue::Blue,
                RainbowHue::Violet,
                RainbowHue::Red,
                RainbowHue::Orange,
            ]
        );
    }

    /// Every built-in variant gives each hue a color of its own, so no
    /// two cells within one turn of the rainbow share a color.
    #[test]
    fn every_builtin_draws_each_hue_in_its_own_color() {
        for palette in [
            &DEFAULT_DARK_ROLES,
            &DEFAULT_LIGHT_ROLES,
            &HIGH_CONTRAST_DARK_ROLES,
            &HIGH_CONTRAST_LIGHT_ROLES,
        ] {
            let colors: Vec<Color> = RainbowHue::ALL
                .into_iter()
                .map(|hue| palette.rainbow.spec(hue).color)
                .collect();
            for (index, color) in colors.iter().enumerate() {
                assert!(
                    !colors[index + 1..].contains(color),
                    "{color:?} is drawn for two hues"
                );
            }
        }
    }

    /// A label is told from what it labels by color alone, so in every
    /// built-in the label color is its own: no text, accent or role
    /// shares it.
    #[test]
    fn every_builtin_keeps_the_label_color_for_labels() {
        for variant in builtins() {
            let theme = &variant.theme;
            let label = theme.semantic.label.color;
            let others = [
                ("text", theme.text.default.color),
                ("secondary", theme.text.secondary.color),
                ("bright", theme.text.bright.color),
                ("accent", theme.semantic.accent.color),
            ]
            .into_iter()
            .chain(
                theme
                    .roles
                    .iter()
                    .map(|(key, spec)| (key.as_str(), spec.color)),
            );
            for (name, color) in others {
                assert_ne!(
                    color, label,
                    "{:?} draws {name} in its label color",
                    variant.id
                );
            }
        }
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
