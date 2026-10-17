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
    Worker,
    Unassigned,
}

/// Whether the footer supplies a foreground or leaves the theme's style intact.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum FooterTint {
    Footer(FooterColor),
    #[default]
    Plain,
}

impl FooterTint {
    /// Keep all theme attributes, replacing only a configured foreground.
    pub(crate) fn style(self, theme: Style) -> Style {
        match self {
            Self::Footer(color) => theme.fg(resolved(color).foreground),
            Self::Plain => theme,
        }
    }

    /// Subdue a label without delegating its contrast to terminal dimming.
    pub(crate) fn subdued_style(self, theme: Style) -> Style {
        let foreground = match self {
            Self::Footer(color) => resolved(color).subdued,
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
    theme:  Arc<Theme>,
    colors: BTreeMap<FooterColor, ReadableInk>,
    plain:  ReadableInk,
}

thread_local! {
    static PALETTE: RefCell<ReadablePalette> = RefCell::new(ReadablePalette::new(tui_pane::theme()));
}

impl ReadablePalette {
    fn new(theme: Arc<Theme>) -> Self {
        let ground = theme.text.bg_focus.color;
        let colors = FooterColor::ALL
            .into_iter()
            .map(|color| (color, ReadableInk::new(color.into(), ground)))
            .collect();
        let plain = ReadableInk::new(theme.text.default.color, ground);
        Self {
            theme,
            colors,
            plain,
        }
    }

    fn current(&mut self) -> &Self {
        let theme = tui_pane::theme();
        if !Arc::ptr_eq(&self.theme, &theme) {
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

impl ReadableInk {
    fn new(color: Color, ground: Color) -> Self {
        let foreground = if contrast(color, ground) >= MINIMUM_CONTRAST {
            color
        } else {
            readable_lightness(color, ground)
        };
        // A muted label approaches the contrast boundary along the same hue.
        let subdued = readable_lightness(foreground, ground);
        Self {
            foreground,
            subdued,
        }
    }
}

/// Keep HSL hue and saturation, choosing the closest readable lightness.
fn readable_lightness(color: Color, ground: Color) -> Color {
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
    // Each side of the ground has a monotone readable interval. Find its boundary.
    let mut best = (f64::INFINITY, at(lightness));
    for endpoint in [0.0, 1.0] {
        if contrast(at(endpoint), ground) < MINIMUM_CONTRAST {
            continue;
        }
        let mut readable = endpoint;
        let mut unreadable = if contrast(color, ground) >= MINIMUM_CONTRAST {
            // Subdued ink chooses the boundary nearest the ground's lightness.
            let (red, green, blue) = rgb(ground);
            (f64::from(red.max(green).max(blue)) + f64::from(red.min(green).min(blue))) / 510.0
        } else {
            lightness
        };
        for _ in 0..LIGHTNESS_SEARCH_ITERATIONS {
            let middle = readable.midpoint(unreadable);
            if contrast(at(middle), ground) >= MINIMUM_CONTRAST {
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
                "role.worker" => FooterRole::Worker,
                "role.unassigned" => FooterRole::Unassigned,
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
            .map_or(FooterTint::Plain, FooterTint::Footer)
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
