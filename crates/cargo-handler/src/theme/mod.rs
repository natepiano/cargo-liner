//! cargo-handler's compiled-in theme variants.
//!
//! `tui_pane` supplies the theme machinery — the types, the registry,
//! the resolver, the directory watch — and none of the colors. Every
//! palette this app ships lives here, so retuning cargo-handler's grid
//! cannot move another app's panes.
//!
//! The summary's own colors are theme roles: [`Role`] names each one,
//! and every built-in variant carries a value for each. Among them is
//! the session palette the agent cells take in turn, one [`SessionHue`] a cell,
//! which draws both the cell's title and its agent's name in the
//! summary.
//!
//! [`builtins`] is what startup hands to [`install_theme`];
//! user `themes/*.toml` variants layer on top, replacing a built-in
//! when the names match. The `cargo-handler/themes/*.toml` templates
//! mirror these constructors as copyable documentation, locked against
//! drift by the tests at the bottom of this file.

mod files;
#[cfg(test)]
mod files_tests;

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

pub(crate) use self::files::install_theme;
use crate::constants::BUSY_ROLE;
use crate::constants::CLAUDE_ROLE;
use crate::constants::CODEX_ROLE;
use crate::constants::DEFAULT_DARK_THEME;
use crate::constants::DEFAULT_HC_DARK_THEME;
use crate::constants::DEFAULT_HC_LIGHT_THEME;
use crate::constants::DEFAULT_LIGHT_THEME;
use crate::constants::IDLE_ROLE;
use crate::constants::SHELL_ROLE;
use crate::constants::UNREACHABLE_ROLE;
use crate::footer_colors;

/// A chronological rank constructed only within its set of distinct reset times.
pub(crate) struct ResetRank {
    position: usize,
    last:     usize,
}

impl ResetRank {
    /// Enumerate exactly the valid positions, including a single nearest reset.
    pub(crate) fn positions(distinct: usize) -> impl Iterator<Item = Self> {
        (0..distinct).map(move |position| Self {
            position,
            last: distinct.saturating_sub(1),
        })
    }

    /// Blend in the active theme's color space, retaining exact endpoints.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "positions constructs ranks at most last, so the rounded alpha is at most 255"
    )]
    pub(crate) fn color(self) -> Color {
        if self.position == 0 {
            return tui_pane::success_color();
        }
        if self.position == self.last {
            return tui_pane::error_color();
        }
        let intervals = self.last as u128;
        let alpha =
            ((self.position as u128 * u128::from(u8::MAX) + intervals / 2) / intervals) as u8;
        tui_pane::blend_color(tui_pane::success_color(), tui_pane::error_color(), alpha)
    }
}

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
    Session(SessionHue),
}

impl Role {
    /// Every role, in the order [`RolePalette`] lists them.
    const ALL: [Self; 11] = [
        Self::Claude,
        Self::Codex,
        Self::Busy,
        Self::Shell,
        Self::Idle,
        Self::Unreachable,
        Self::Session(SessionHue::First),
        Self::Session(SessionHue::Second),
        Self::Session(SessionHue::Third),
        Self::Session(SessionHue::Fourth),
        Self::Session(SessionHue::Fifth),
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
            Self::Session(hue) => hue.key(),
        }
    }

    /// The role's style in the active variant. A variant without the
    /// role gets [`DEFAULT_DARK_ROLES`]' value.
    pub(crate) fn style(self) -> Style {
        let style = tui_pane::role_style(self.key(), DEFAULT_DARK_ROLES.spec(self));
        match self {
            Self::Session(_) => footer_colors::readable_session_style(style),
            Self::Claude
            | Self::Codex
            | Self::Busy
            | Self::Shell
            | Self::Idle
            | Self::Unreachable => footer_colors::readable_status_style(style),
        }
    }
}

/// One color of the session palette the agent cells take in turn, in cell
/// order: a cell's title and its agent's name in the summary are drawn
/// in its hue, so the two can be paired at a glance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionHue {
    /// The first cell's hue.
    First,
    /// The second cell's hue.
    Second,
    /// The third cell's hue.
    Third,
    /// The fourth cell's hue.
    Fourth,
    /// The fifth cell's hue.
    Fifth,
}

impl SessionHue {
    /// Every hue, in the order the cells take them.
    const ALL: [Self; 5] = [
        Self::First,
        Self::Second,
        Self::Third,
        Self::Fourth,
        Self::Fifth,
    ];

    /// The hue of the agent at `position` in cell order, counted from
    /// zero and starting over after the fifth entry.
    pub(crate) const fn of_agent(position: usize) -> Self { Self::ALL[position % Self::ALL.len()] }

    /// The ordinal's position in the theme's five-color array.
    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
            Self::Third => 2,
            Self::Fourth => 3,
            Self::Fifth => 4,
        }
    }

    /// Private renderer keys; theme files expose only the five-entry list.
    const fn key(self) -> &'static str {
        match self {
            Self::First => "session_1",
            Self::Second => "session_2",
            Self::Third => "session_3",
            Self::Fourth => "session_4",
            Self::Fifth => "session_5",
        }
    }
}

/// One built-in variant's value for each [`Role`].
struct RolePalette {
    /// [`Role::Claude`].
    claude:       StyleSpec,
    /// [`Role::Codex`].
    codex:        StyleSpec,
    /// [`Role::Busy`].
    busy:         StyleSpec,
    /// [`Role::Shell`].
    shell:        StyleSpec,
    /// [`Role::Idle`].
    idle:         StyleSpec,
    /// [`Role::Unreachable`].
    unreachable:  StyleSpec,
    /// [`Role::Session`], one value per hue.
    session_hues: [Color; SessionHue::ALL.len()],
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
            Role::Session(hue) => StyleSpec::from_color(self.session_hues[hue.index()]),
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
    claude:       StyleSpec::from_color(Color::Rgb(217, 119, 87)),
    codex:        StyleSpec::from_color(Color::Rgb(175, 140, 255)),
    busy:         StyleSpec::from_color(Color::Rgb(100, 220, 100)),
    // Pale cyan stays apart from the account teal after focused-ground solving.
    shell:        StyleSpec::from_color(Color::Rgb(140, 230, 240)),
    idle:         StyleSpec::from_color(Color::Rgb(140, 140, 140)),
    unreachable:  StyleSpec::from_color(Color::Rgb(255, 100, 100)),
    session_hues: [
        Color::Rgb(245, 186, 38),
        Color::Rgb(218, 251, 142),
        Color::Rgb(73, 249, 195),
        Color::Rgb(41, 189, 172),
        Color::Rgb(249, 193, 253),
    ],
};

/// [`default_light`]'s roles: the same hues, darker, for a white
/// background.
const DEFAULT_LIGHT_ROLES: RolePalette = RolePalette {
    claude:       StyleSpec::from_color(Color::Rgb(190, 85, 50)),
    codex:        StyleSpec::from_color(Color::Rgb(110, 60, 200)),
    busy:         StyleSpec::from_color(Color::Rgb(0, 130, 0)),
    // Green teal keeps the covered timer ink distinct from the account teal.
    shell:        StyleSpec::from_color(Color::Rgb(0, 120, 100)),
    idle:         StyleSpec::from_color(Color::Rgb(110, 110, 110)),
    unreachable:  StyleSpec::from_color(Color::Rgb(190, 0, 0)),
    session_hues: [
        Color::Rgb(113, 0, 43),
        Color::Rgb(180, 20, 110),
        Color::Rgb(168, 92, 168),
        Color::Rgb(69, 14, 100),
        Color::Rgb(29, 42, 98),
    ],
};

/// [`high_contrast_dark`]'s roles: bright and bold, save `idle`, which
/// stays quiet.
const HIGH_CONTRAST_DARK_ROLES: RolePalette = RolePalette {
    claude:       StyleSpec::bold(Color::Rgb(255, 150, 90)),
    codex:        StyleSpec::bold(Color::LightMagenta),
    busy:         StyleSpec::bold(Color::LightGreen),
    shell:        StyleSpec::bold(Color::LightCyan),
    idle:         StyleSpec::from_color(Color::Gray),
    unreachable:  StyleSpec::bold(Color::LightRed),
    session_hues: [
        Color::Rgb(245, 186, 38),
        Color::Rgb(205, 246, 140),
        Color::Rgb(85, 185, 69),
        Color::Rgb(7, 229, 182),
        Color::Rgb(249, 193, 253),
    ],
};

/// [`high_contrast_light`]'s roles: deep and bold, save `idle`, which
/// stays quiet.
const HIGH_CONTRAST_LIGHT_ROLES: RolePalette = RolePalette {
    claude:       StyleSpec::bold(Color::Rgb(160, 60, 0)),
    codex:        StyleSpec::bold(Color::Rgb(100, 0, 160)),
    busy:         StyleSpec::bold(Color::Rgb(0, 100, 0)),
    shell:        StyleSpec::bold(Color::Rgb(0, 80, 65)),
    idle:         StyleSpec::from_color(Color::Rgb(80, 80, 80)),
    unreachable:  StyleSpec::bold(Color::Rgb(180, 0, 0)),
    session_hues: [
        Color::Rgb(113, 0, 43),
        Color::Rgb(250, 23, 154),
        Color::Rgb(95, 4, 76),
        Color::Rgb(141, 86, 172),
        Color::Rgb(40, 42, 113),
    ],
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
            // PaneChromeTheme::active_border is None, so without focus
            // tint the focused border uses active_title's color.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::DarkGray),
            active_title:    StyleSpec::bold(Color::Rgb(255, 210, 160)),
            inactive_title:  StyleSpec::from_color(Color::White),
        },
        focus:       FocusTheme {
            active:     StyleSpec::from_color(Color::Rgb(86, 86, 86)),
            hover:      StyleSpec::from_color(Color::Rgb(80, 80, 80)),
            remembered: StyleSpec::from_color(Color::Rgb(46, 46, 46)),
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
            bar: StyleSpec::from_color(Color::Rgb(60, 60, 60)),
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
            // PaneChromeTheme::active_border is None, so without focus
            // tint the focused border uses active_title's color.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::Rgb(140, 140, 140)),
            active_title:    StyleSpec::bold(Color::Rgb(54, 58, 61)),
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
            bar: StyleSpec::from_color(Color::Rgb(250, 250, 250)),
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
            // PaneChromeTheme::active_border is None, so without focus
            // tint the focused border uses active_title's color.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::White),
            active_title:    StyleSpec::bold(Color::Rgb(255, 210, 160)),
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
            // PaneChromeTheme::active_border is None, so without focus
            // tint the focused border uses active_title's color.
            active_border:   None,
            inactive_border: StyleSpec::from_color(Color::Black),
            active_title:    StyleSpec::bold(Color::Rgb(54, 58, 61)),
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
        let family: ThemeFamily = files::parse_family(template).expect("template should parse");
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

    /// The five session hues repeat after the fifth entry without using role hues.
    #[test]
    fn agents_take_session_entries_in_order_and_cycle_after_the_last() {
        let hues: Vec<SessionHue> = (0..9).map(SessionHue::of_agent).collect();
        assert_eq!(
            hues,
            [
                SessionHue::First,
                SessionHue::Second,
                SessionHue::Third,
                SessionHue::Fourth,
                SessionHue::Fifth,
                SessionHue::First,
                SessionHue::Second,
                SessionHue::Third,
                SessionHue::Fourth,
            ]
        );
    }

    /// Every built-in variant gives each hue a color of its own, so no
    /// two cells within one turn of the session palette share a color.
    #[test]
    fn every_builtin_draws_each_hue_in_its_own_color() {
        for palette in [
            &DEFAULT_DARK_ROLES,
            &DEFAULT_LIGHT_ROLES,
            &HIGH_CONTRAST_DARK_ROLES,
            &HIGH_CONTRAST_LIGHT_ROLES,
        ] {
            let colors: Vec<Color> = SessionHue::ALL
                .into_iter()
                .map(|hue| palette.session_hues[hue.index()])
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
    fn the_readout_count_meets_contrast_on_both_grounds_in_every_built_in_theme() {
        for variant in builtins() {
            for_each_pane_ground(&variant.theme, |transparent, focused, ground| {
                let ink = tui_pane::error_color();
                let contrast = contrast_ratio(ink, ground);
                assert!(
                    contrast >= 4.5,
                    "{} readout count and width {ink:?} on {ground:?}: {contrast:.2}:1, transparent={transparent}, focused={focused}",
                    variant.id.as_str()
                );
            });
        }
    }
}
