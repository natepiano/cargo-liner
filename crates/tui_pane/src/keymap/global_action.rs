//! `GlobalAction`: framework-owned global actions.
//!
//! Covers pane management, lifecycle, and overlay focus. The app's own
//! globals (Find, Rescan, etc.) live in a separate `Globals<Ctx>` impl
//! and share the `[global]` TOML table at load time.

use core::fmt;

use super::action_enum::ActionEnum;

/// Framework-owned global actions.
///
/// Defaults: `q` / `R` / `Tab` / `Shift+Tab` / `Ctrl+K` / `s` / `x`
/// (resolved at builder time in Phase 8).
///
/// Dispatch for [`Self::Quit`], [`Self::Restart`], and [`Self::Dismiss`]
/// is supplied by the binary as the three positional
/// `Keymap::builder(quit, restart, dismiss)` arguments. The four
/// pane-focus variants ([`Self::NextPane`], [`Self::PrevPane`],
/// [`Self::OpenKeymap`], [`Self::OpenSettings`]) are dispatched
/// entirely by the framework, which owns the registered pane set.
///
/// [`ActionEnum`] is implemented by hand here rather than through
/// [`action_enum!`](crate::action_enum) because the strings are
/// framework-canonical and the variant set is closed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlobalAction {
    /// Quit the application. Dispatched by the binary-supplied `quit` fn.
    Quit,
    /// Restart the application. Dispatched by the binary-supplied
    /// `restart` fn.
    Restart,
    /// Move focus to the next registered pane.
    NextPane,
    /// Move focus to the previous registered pane.
    PrevPane,
    /// Focus the framework-provided keymap overlay.
    OpenKeymap,
    /// Focus the framework-provided settings overlay.
    OpenSettings,
    /// Close the current overlay or dismiss the focused dismissable
    /// item. Dispatched by the binary-supplied `dismiss` fn.
    Dismiss,
}

impl ActionEnum for GlobalAction {
    const ALL: &'static [Self] = &[
        Self::Quit,
        Self::Restart,
        Self::NextPane,
        Self::PrevPane,
        Self::OpenKeymap,
        Self::OpenSettings,
        Self::Dismiss,
    ];

    fn toml_key(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Restart => "restart",
            Self::NextPane => "next_pane",
            Self::PrevPane => "prev_pane",
            Self::OpenKeymap => "open_keymap",
            Self::OpenSettings => "open_settings",
            Self::Dismiss => "dismiss",
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::Quit => "Quit",
            Self::Restart => "Restart",
            Self::NextPane => "Next pane",
            Self::PrevPane => "Previous pane",
            Self::OpenKeymap => "Open keymap viewer",
            Self::OpenSettings => "Open settings",
            Self::Dismiss => "Dismiss overlay / output",
        }
    }

    fn from_toml_key(key: &str) -> Option<Self> {
        match key {
            "quit" => Some(Self::Quit),
            "restart" => Some(Self::Restart),
            "next_pane" => Some(Self::NextPane),
            "prev_pane" => Some(Self::PrevPane),
            "open_keymap" => Some(Self::OpenKeymap),
            "open_settings" => Some(Self::OpenSettings),
            "dismiss" => Some(Self::Dismiss),
            _ => None,
        }
    }
}

impl fmt::Display for GlobalAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.description()) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::GlobalAction;
    use crate::ActionEnum;

    #[test]
    fn all_has_seven_variants_in_declaration_order() {
        assert_eq!(
            GlobalAction::ALL,
            &[
                GlobalAction::Quit,
                GlobalAction::Restart,
                GlobalAction::NextPane,
                GlobalAction::PrevPane,
                GlobalAction::OpenKeymap,
                GlobalAction::OpenSettings,
                GlobalAction::Dismiss,
            ]
        );
    }

    #[test]
    fn toml_keys_round_trip_for_every_variant() {
        for variant in GlobalAction::ALL {
            assert_eq!(
                GlobalAction::from_toml_key(variant.toml_key()),
                Some(*variant),
            );
        }
    }

    #[test]
    fn descriptions_are_non_empty_for_every_variant() {
        for variant in GlobalAction::ALL {
            assert!(!variant.description().is_empty());
        }
    }

    #[test]
    fn display_delegates_to_description() {
        assert_eq!(format!("{}", GlobalAction::Quit), "Quit");
        assert_eq!(format!("{}", GlobalAction::OpenSettings), "Open settings");
    }

    #[test]
    fn from_toml_key_unknown_returns_none() {
        assert!(GlobalAction::from_toml_key("nope").is_none());
        assert!(GlobalAction::from_toml_key("").is_none());
    }
}
