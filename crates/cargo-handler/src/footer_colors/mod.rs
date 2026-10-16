//! The footer's named colors, shared by account labels and management roles.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::time::SystemTime;

use ratatui::style::Color;
use ratatui::style::Style;

/// A color name understood by the Claude Code footer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
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
            Self::Footer(color) => theme.fg(color.into()),
            Self::Plain => theme,
        }
    }
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
                    if let Some(account) = key.strip_prefix("account.") {
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
            FooterColorSource::File(PathBuf::from(home).join(".claude/config/statusline.conf"))
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
mod tests;
