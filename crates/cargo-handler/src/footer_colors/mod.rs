//! The footer's named colors, shared by account labels and management roles.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::SystemTime;

use ratatui::style::Color;
use ratatui::style::Modifier;
use ratatui::style::Style;
use tui_pane::Theme;

use self::constants::ACCOUNT_KEY_PREFIX;
use self::constants::BLUE_LUMINANCE_WEIGHT;
use self::constants::CONTRAST_LUMINANCE_OFFSET;
use self::constants::FOOTER_CONFIG_PATH;
use self::constants::GREEN_LUMINANCE_WEIGHT;
use self::constants::LIGHTNESS_SEARCH_ITERATIONS;
use self::constants::MINIMUM_CONTRAST;
use self::constants::RED_LUMINANCE_WEIGHT;
use self::constants::SRGB_ENCODED_OFFSET;
use self::constants::SRGB_ENCODED_SCALE;
use self::constants::SRGB_LINEAR_DIVISOR;
use self::constants::SRGB_LINEAR_THRESHOLD;
use self::constants::SRGB_POWER;
use crate::census::HierarchyRole;

pub(crate) mod constants;

/// A color name understood by the Claude Code footer.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FooterColor {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    BrightBlack,
    BrightRed,
    BrightGreen,
    BrightYellow,
    BrightBlue,
    BrightMagenta,
    BrightCyan,
    BrightWhite,
    Orange,
    Purple,
    Pink,
    Teal,
    Brown,
}

/// A name outside the footer's color table.
#[derive(Debug)]
pub(crate) struct UnknownFooterColor;

impl FromStr for FooterColor {
    type Err = UnknownFooterColor;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Ok(match name {
            "black" => Self::Black,
            "red" => Self::Red,
            "green" => Self::Green,
            "yellow" => Self::Yellow,
            "blue" => Self::Blue,
            "magenta" => Self::Magenta,
            "cyan" => Self::Cyan,
            "white" => Self::White,
            "bright-black" => Self::BrightBlack,
            "bright-red" => Self::BrightRed,
            "bright-green" => Self::BrightGreen,
            "bright-yellow" => Self::BrightYellow,
            "bright-blue" => Self::BrightBlue,
            "bright-magenta" => Self::BrightMagenta,
            "bright-cyan" => Self::BrightCyan,
            "bright-white" => Self::BrightWhite,
            "orange" => Self::Orange,
            "purple" => Self::Purple,
            "pink" => Self::Pink,
            "teal" => Self::Teal,
            "brown" => Self::Brown,
            _ => return Err(UnknownFooterColor),
        })
    }
}

impl From<FooterColor> for Color {
    fn from(color: FooterColor) -> Self {
        match color {
            FooterColor::Black => Self::Black,
            FooterColor::Red => Self::Red,
            FooterColor::Green => Self::Green,
            FooterColor::Yellow => Self::Yellow,
            FooterColor::Blue => Self::Blue,
            FooterColor::Magenta => Self::Magenta,
            FooterColor::Cyan => Self::Cyan,
            FooterColor::White => Self::Gray,
            FooterColor::BrightBlack => Self::DarkGray,
            FooterColor::BrightRed => Self::LightRed,
            FooterColor::BrightGreen => Self::LightGreen,
            FooterColor::BrightYellow => Self::LightYellow,
            FooterColor::BrightBlue => Self::LightBlue,
            FooterColor::BrightMagenta => Self::LightMagenta,
            FooterColor::BrightCyan => Self::LightCyan,
            FooterColor::BrightWhite => Self::White,
            FooterColor::Orange => Self::Indexed(208),
            FooterColor::Purple => Self::Indexed(141),
            FooterColor::Pink => Self::Indexed(211),
            FooterColor::Teal => Self::Indexed(30),
            FooterColor::Brown => Self::Indexed(137),
        }
    }
}

/// A responsibility with a configurable footer color.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum FooterRole {
    Showrunner,
    UnitDirector,
}

/// Whether the footer supplies a foreground or leaves the theme's style intact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FooterTint {
    Footer(FooterColor),
    /// Management ink reserves contrast for antialiased glyph edges.
    Management(FooterColor),
    #[default]
    Plain,
}

impl FooterTint {
    /// Keep all theme attributes, replacing only a configured foreground.
    pub(crate) fn style(self, theme: Style) -> Style {
        match self {
            Self::Footer(color) => theme.fg(resolved(color).foreground),
            Self::Management(color) => theme.fg(resolved_management(color).foreground),
            Self::Plain => theme,
        }
    }

    /// Subdue a label without delegating its contrast to terminal dimming.
    pub(crate) fn subdued_style(self, theme: Style) -> Style {
        let foreground = match self {
            Self::Footer(color) => resolved(color).subdued,
            Self::Management(color) => resolved_management(color).subdued,
            Self::Plain => PALETTE.with(|palette| palette.borrow_mut().current().plain.subdued),
        };
        theme.remove_modifier(Modifier::DIM).fg(foreground)
    }
}

/// Footer ink resolved against the theme's ground, including a readable muted form.
#[derive(Clone, Copy, Debug)]
struct ReadableInk {
    foreground: Color,
    subdued:    Color,
}

/// One palette belongs to one active theme; drawing only looks up its colors.
struct ReadablePalette {
    theme:      Arc<Theme>,
    colors:     BTreeMap<FooterColor, ReadableInk>,
    management: BTreeMap<FooterColor, ReadableInk>,
    plain:      ReadableInk,
    grounds:    InkGrounds,
    glyph_inks: Vec<(Color, Color)>,
}

thread_local! {
    static PALETTE: RefCell<ReadablePalette> = RefCell::new(ReadablePalette::new(tui_pane::theme()));
}

impl ReadablePalette {
    fn new(theme: Arc<Theme>) -> Self {
        let grounds = InkGrounds::current(&theme);
        let colors = FooterColor::ALL
            .into_iter()
            .map(|color| {
                (
                    color,
                    ReadableInk::new(
                        InkPurpose::Account.source(color),
                        grounds,
                        InkCoverage::Antialiased,
                    ),
                )
            })
            .collect();
        let management = FooterColor::ALL
            .into_iter()
            .map(|color| {
                (
                    color,
                    ReadableInk::new(
                        InkPurpose::Management.source(color),
                        grounds,
                        InkCoverage::Antialiased,
                    ),
                )
            })
            .collect();
        let plain = ReadableInk::new(theme.text.default.color, grounds, InkCoverage::Opaque);
        Self {
            theme,
            colors,
            management,
            plain,
            grounds,
            glyph_inks: Vec::new(),
        }
    }

    fn current(&mut self) -> &Self {
        let theme = tui_pane::theme();
        if !Arc::ptr_eq(&self.theme, &theme) || self.grounds != InkGrounds::current(&theme) {
            *self = Self::new(theme);
        }
        self
    }
}

impl FooterColor {
    const ALL: [Self; 21] = [
        Self::Black,
        Self::Red,
        Self::Green,
        Self::Yellow,
        Self::Blue,
        Self::Magenta,
        Self::Cyan,
        Self::White,
        Self::BrightBlack,
        Self::BrightRed,
        Self::BrightGreen,
        Self::BrightYellow,
        Self::BrightBlue,
        Self::BrightMagenta,
        Self::BrightCyan,
        Self::BrightWhite,
        Self::Orange,
        Self::Purple,
        Self::Pink,
        Self::Teal,
        Self::Brown,
    ];
}

fn resolved(color: FooterColor) -> ReadableInk {
    PALETTE.with(|palette| palette.borrow_mut().current().colors[&color])
}

fn resolved_management(color: FooterColor) -> ReadableInk {
    PALETTE.with(|palette| palette.borrow_mut().current().management[&color])
}

/// Read named channels through `tui_pane`'s xterm approximation.
fn rgb(color: Color) -> (u8, u8, u8) {
    match tui_pane::blend_color(color, Color::Black, 0) {
        Color::Rgb(red, green, blue) => (red, green, blue),
        _ => (0, 0, 0),
    }
}

fn luminance(color: Color) -> f64 {
    let channel = |value| {
        let value = f64::from(value) / 255.0;
        if value <= SRGB_LINEAR_THRESHOLD {
            value / SRGB_LINEAR_DIVISOR
        } else {
            ((value + SRGB_ENCODED_OFFSET) / SRGB_ENCODED_SCALE).powf(SRGB_POWER)
        }
    };
    let (red, green, blue) = rgb(color);
    RED_LUMINANCE_WEIGHT.mul_add(
        channel(red),
        GREEN_LUMINANCE_WEIGHT.mul_add(channel(green), BLUE_LUMINANCE_WEIGHT * channel(blue)),
    )
}

fn contrast(ink: Color, ground: Color) -> f64 {
    let ink = luminance(ink);
    let ground = luminance(ground);
    (ink.max(ground) + CONTRAST_LUMINANCE_OFFSET) / (ink.min(ground) + CONTRAST_LUMINANCE_OFFSET)
}

/// The actual pane grounds on which one foreground must remain readable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InkGrounds {
    base:      Color,
    unfocused: Color,
    focused:   Color,
}

impl InkGrounds {
    fn current(theme: &Theme) -> Self {
        Self {
            base:      theme.text.bg_focus.color,
            unfocused: tui_pane::pane_background(false),
            focused:   tui_pane::pane_background(true),
        }
    }

    fn contrast(self, ink: Color) -> f64 {
        [self.base, self.unfocused, self.focused]
            .into_iter()
            .map(|ground| contrast(ink, ground))
            .fold(f64::INFINITY, f64::min)
    }
}

/// Accounts reserve azure blue, keeping it apart from management purple after tinting.
#[derive(Clone, Copy)]
enum InkPurpose {
    Account,
    Management,
}

impl InkPurpose {
    fn source(self, color: FooterColor) -> Color {
        match (self, color) {
            (Self::Account, FooterColor::Blue | FooterColor::BrightBlue) => {
                constants::ACCOUNT_BLUE_SEED
            },
            (Self::Account, FooterColor::Teal) => constants::ACCOUNT_TEAL_SEED,
            (Self::Account | Self::Management, _) => color.into(),
        }
    }
}

/// Foreground share of a covered glyph stem, out of 255.
/// Live xterm stems measured about 216; 208 leaves a small rasterization margin.
const ANTIALIASED_GLYPH_COVERAGE: u8 = 208;

/// The minimum glyph coverage for which an ink must retain the contrast floor.
#[derive(Clone, Copy)]
enum InkCoverage {
    Opaque,
    /// Measured stem coverage with a small margin for the terminal rasterizer.
    Antialiased,
}

impl InkCoverage {
    fn contrast(self, grounds: InkGrounds, ink: Color) -> f64 {
        match self {
            Self::Opaque => grounds.contrast(ink),
            Self::Antialiased => [grounds.base, grounds.unfocused, grounds.focused]
                .into_iter()
                .map(|ground| {
                    contrast(
                        tui_pane::blend_color(ink, ground, 255 - ANTIALIASED_GLYPH_COVERAGE),
                        ground,
                    )
                })
                .fold(f64::INFINITY, f64::min),
        }
    }
}

/// Status foregrounds share the footer's contrast policy and palette lifetime.
pub(crate) fn readable_status_style(style: Style) -> Style { readable_style(style) }

/// Session hues retain readable glyph coverage on every drawn pane ground.
pub(crate) fn readable_session_style(style: Style) -> Style { readable_style(style) }

/// Solve each source ink once per theme and set of pane grounds.
fn readable_style(style: Style) -> Style {
    let Some(color) = style.fg else { return style };
    let foreground = PALETTE.with(|palette| {
        let mut palette = palette.borrow_mut();
        palette.current();
        let grounds = palette.grounds;
        let inks = &mut palette.glyph_inks;
        if let Some((_, foreground)) = inks.iter().find(|(ink, _)| *ink == color) {
            return *foreground;
        }
        let foreground = ReadableInk::new(color, grounds, InkCoverage::Antialiased).foreground;
        inks.push((color, foreground));
        foreground
    });
    style.fg(foreground)
}

impl ReadableInk {
    fn new(color: Color, grounds: InkGrounds, coverage: InkCoverage) -> Self {
        let foreground = if coverage.contrast(grounds, color) >= MINIMUM_CONTRAST {
            color
        } else {
            readable_lightness(color, grounds, coverage)
        };
        // `subdued` approaches the contrast boundary along the same hue.
        let subdued = readable_lightness(foreground, grounds, coverage);
        Self {
            foreground,
            subdued,
        }
    }
}

/// Keep HSL hue and saturation, choosing the closest readable lightness.
fn readable_lightness(color: Color, grounds: InkGrounds, coverage: InkCoverage) -> Color {
    let channels = <[u8; 3]>::from(rgb(color)).map(|value| f64::from(value) / 255.0);
    let maximum = channels.into_iter().fold(0.0, f64::max);
    let minimum = channels.into_iter().fold(1.0, f64::min);
    let lightness = maximum.midpoint(minimum);
    let chroma = maximum - minimum;
    let saturation = if chroma == 0.0 {
        0.0
    } else {
        chroma / (1.0 - lightness.mul_add(2.0, -1.0).abs())
    };
    let at = |light: f64| {
        let new_chroma = (1.0 - light.mul_add(2.0, -1.0).abs()) * saturation;
        let shifted_minimum = light - new_chroma / 2.0;
        let values = channels.map(|value| {
            let relative = if chroma == 0.0 {
                0.0
            } else {
                (value - minimum) / chroma
            };
            rounded_channel(relative.mul_add(new_chroma, shifted_minimum) * 255.0)
        });
        Color::Rgb(values[0], values[1], values[2])
    };
    // Each side of `ground` has a monotone readable interval. Find its boundary.
    let mut best = (f64::INFINITY, at(lightness));
    for endpoint in [0.0, 1.0] {
        if coverage.contrast(grounds, at(endpoint)) < MINIMUM_CONTRAST {
            continue;
        }
        let mut readable = endpoint;
        let mut unreadable = if coverage.contrast(grounds, color) >= MINIMUM_CONTRAST {
            // Subdued ink chooses the boundary nearest `ground`'s lightness.
            let (red, green, blue) = rgb(grounds.base);
            (f64::from(red.max(green).max(blue)) + f64::from(red.min(green).min(blue))) / 510.0
        } else {
            lightness
        };
        for _ in 0..LIGHTNESS_SEARCH_ITERATIONS {
            let middle = readable.midpoint(unreadable);
            if coverage.contrast(grounds, at(middle)) >= MINIMUM_CONTRAST {
                readable = middle;
            } else {
                unreadable = middle;
            }
        }
        let distance = (readable - lightness).abs();
        if distance < best.0 {
            best = (distance, at(readable));
        }
    }
    best.1
}

/// Quantize to the nearest channel byte, saturating at the representable endpoints.
fn rounded_channel(value: f64) -> u8 {
    let mut lower = u8::MIN;
    let mut upper = u8::MAX;
    while lower < upper {
        let middle = lower.midpoint(upper);
        if value >= f64::from(middle) + 0.5 {
            lower = middle + 1;
        } else {
            upper = middle;
        }
    }
    lower
}

/// Valid role and account colors parsed from the footer's configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FooterColors {
    roles:    BTreeMap<FooterRole, FooterColor>,
    accounts: BTreeMap<String, FooterColor>,
}

impl FooterColors {
    /// Read assignments as the footer does: comments, whitespace, then the first `=`.
    pub(crate) fn parse(text: &str) -> Self {
        let mut colors = Self::default();
        for line in text.lines() {
            let assignment = line.split('#').next().unwrap_or_default().trim();
            let Some((key, value)) = assignment.split_once('=') else {
                continue;
            };
            let color = value.trim().parse::<FooterColor>();
            let role = match key.trim() {
                "role.showrunner" => FooterRole::Showrunner,
                "role.unit-director" => FooterRole::UnitDirector,
                key => {
                    if let Some(account) = key.strip_prefix(ACCOUNT_KEY_PREFIX) {
                        match color {
                            Ok(color) => {
                                colors.accounts.insert(account.to_string(), color);
                            },
                            Err(_) => {
                                colors.accounts.remove(account);
                            },
                        }
                    }
                    continue;
                },
            };
            match color {
                Ok(color) => {
                    colors.roles.insert(role, color);
                },
                Err(_) => {
                    colors.roles.remove(&role);
                },
            }
        }
        colors
    }

    /// The configured role foreground, or the theme's plain text style.
    pub(crate) fn role(&self, role: FooterRole) -> FooterTint {
        self.roles
            .get(&role)
            .copied()
            .map_or(FooterTint::Plain, FooterTint::Management)
    }

    /// A specialist can name the same management responsibilities as a session.
    pub(crate) fn named_role(&self, word: &str) -> FooterTint {
        match word {
            "showrunner" => self.role(FooterRole::Showrunner),
            "unit-director" | "unit" => self.role(FooterRole::UnitDirector),
            _ => FooterTint::Plain,
        }
    }

    /// Management responsibilities use configured ink; worker roles remain plain.
    pub(crate) fn hierarchy_role(&self, role: HierarchyRole) -> FooterTint {
        match role {
            HierarchyRole::Showrunner => self.role(FooterRole::Showrunner),
            HierarchyRole::Unit => self.role(FooterRole::UnitDirector),
            HierarchyRole::Impl
            | HierarchyRole::Test
            | HierarchyRole::Review
            | HierarchyRole::Fix => FooterTint::Plain,
        }
    }

    /// The configured foreground of an account label.
    pub(crate) fn account(&self, label: &str) -> FooterTint {
        self.accounts
            .get(label)
            .copied()
            .map_or(FooterTint::Plain, FooterTint::Footer)
    }
}

/// The last observed availability and modification time of the footer file.
#[derive(Debug, Default, Eq, PartialEq)]
enum FooterFileRevision {
    #[default]
    Unavailable,
    Modified(SystemTime),
}

/// A footer source, disabled for apps constructed without filesystem access.
#[derive(Debug, Default)]
enum FooterColorSource {
    #[default]
    Disabled,
    File(PathBuf),
}

/// The app's parsed palette and file revision, refreshed on census scans.
#[derive(Debug, Default)]
pub(crate) struct FooterColorFile {
    source:          FooterColorSource,
    revision:        FooterFileRevision,
    colors:          FooterColors,
    #[cfg(test)]
    revision_checks: usize,
}

impl FooterColorFile {
    /// Load `$HOME/.claude/config/statusline.conf`, or an uncolored palette.
    pub(crate) fn load() -> Self {
        let source = std::env::var_os("HOME").map_or(FooterColorSource::Disabled, |home| {
            FooterColorSource::File(PathBuf::from(home).join(FOOTER_CONFIG_PATH))
        });
        let mut file = Self {
            source,
            ..Self::default()
        };
        file.refresh();
        file
    }

    /// The palette already in memory; drawing never opens the file.
    pub(crate) const fn colors(&self) -> &FooterColors { &self.colors }

    /// Observe real file checks without using the user's footer in terminal tests.
    #[cfg(test)]
    pub(crate) fn fixture(path: PathBuf) -> Self {
        Self {
            source: FooterColorSource::File(path),
            ..Self::default()
        }
    }

    /// Number of file revision checks, including unchanged revisions.
    #[cfg(test)]
    pub(crate) const fn revision_checks(&self) -> usize { self.revision_checks }

    /// Check once per census scan, rereading only when the revision changes.
    pub(crate) fn refresh(&mut self) -> bool {
        let FooterColorSource::File(path) = &self.source else {
            return false;
        };
        #[cfg(test)]
        {
            self.revision_checks += 1;
        }
        let revision = std::fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .map_or(
                FooterFileRevision::Unavailable,
                FooterFileRevision::Modified,
            );
        if revision == self.revision {
            return false;
        }
        let colors = std::fs::read_to_string(path).map_or_else(
            |_| FooterColors::default(),
            |text| FooterColors::parse(&text),
        );
        self.revision = revision;
        let changed = self.colors != colors;
        self.colors = colors;
        changed
    }
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod ground_tests;

#[cfg(test)]
mod account_seed_tests;
