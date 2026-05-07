//! `GlobalAction`: framework-owned global actions.
//!
//! Covers pane management, lifecycle, and overlay focus. The app's own
//! globals (Find, Rescan, etc.) live in a separate `Globals<Ctx>` impl
//! and share the `[global]` TOML table at load time.

use core::fmt::Display;
use core::fmt::Formatter;

use crossterm::event::KeyCode;

use super::action_enum::Action;
use super::bindings::Bindings;
use super::key_bind::KeyBind;

/// Framework-owned global actions.
///
/// The framework owns dispatch for every variant. [`Self::Quit`] and
/// [`Self::Restart`] set lifecycle flags on the framework; the binary
/// can register `on_quit` / `on_restart` hooks to observe them.
/// [`Self::Dismiss`] runs the framework's overlay/toast chain first
/// and then bubbles to an optional `dismiss_fallback` hook for app-
/// owned dismissables (e.g. collapsing a deleted-row placeholder).
/// The four pane-focus variants ([`Self::NextPane`], [`Self::PrevPane`],
/// [`Self::OpenKeymap`], [`Self::OpenSettings`]) are dispatched
/// entirely by the framework, which owns the registered pane set.
///
/// [`Action`] is implemented by hand here rather than through
/// [`action_enum!`](crate::action_enum) because the strings are
/// framework-canonical and the variant set is closed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum GlobalAction {
    /// Quit the application. Sets the framework's `quit_requested` flag.
    Quit,
    /// Restart the application. Sets the framework's `restart_requested` flag.
    Restart,
    /// Move focus to the next registered pane.
    NextPane,
    /// Move focus to the previous registered pane.
    PrevPane,
    /// Focus the framework-provided keymap overlay.
    OpenKeymap,
    /// Focus the framework-provided settings overlay.
    OpenSettings,
    /// Close the current overlay, dismiss a focused toast, or — if no
    /// framework dismissable matches — bubble to the binary's optional
    /// `dismiss_fallback` hook.
    Dismiss,
}

impl GlobalAction {
    /// Canonical default key bindings for the framework's globals.
    ///
    /// Loaded into the `[global]` scope's [`ScopeMap`](super::scope_map::ScopeMap)
    /// at builder time, then merged with any user overrides from the
    /// TOML loader.
    #[must_use]
    pub fn defaults() -> Bindings<Self> {
        crate::bindings! {
            'q' => Self::Quit,
            'R' => Self::Restart,
            KeyCode::Tab => Self::NextPane,
            KeyBind::shift(KeyCode::Tab) => Self::PrevPane,
            KeyBind::ctrl('k') => Self::OpenKeymap,
            's' => Self::OpenSettings,
            'x' => Self::Dismiss,
        }
    }
}

impl Action for GlobalAction {
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

    fn bar_label(self) -> &'static str {
        match self {
            Self::Quit => "quit",
            Self::Restart => "restart",
            Self::NextPane => "next",
            Self::PrevPane => "prev",
            Self::OpenKeymap => "keymap",
            Self::OpenSettings => "settings",
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

impl Display for GlobalAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result { f.write_str(self.description()) }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;
    use crossterm::event::KeyModifiers;

    use super::GlobalAction;
    use super::KeyBind;
    use crate::Action;

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
    fn bar_labels_are_non_empty_for_every_variant() {
        for variant in GlobalAction::ALL {
            assert!(!variant.bar_label().is_empty());
        }
    }

    #[test]
    fn bar_labels_match_canonical_form() {
        assert_eq!(GlobalAction::Quit.bar_label(), "quit");
        assert_eq!(GlobalAction::NextPane.bar_label(), "next");
        assert_eq!(GlobalAction::OpenKeymap.bar_label(), "keymap");
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

    #[test]
    fn defaults_produce_canonical_seven_bindings() {
        let map = GlobalAction::defaults().into_scope_map();

        let cases: [(KeyBind, GlobalAction); 7] = [
            (KeyBind::from('q'), GlobalAction::Quit),
            (KeyBind::from('R'), GlobalAction::Restart),
            (KeyBind::from(KeyCode::Tab), GlobalAction::NextPane),
            (
                KeyBind {
                    code: KeyCode::Tab,
                    mods: KeyModifiers::SHIFT,
                },
                GlobalAction::PrevPane,
            ),
            (KeyBind::ctrl('k'), GlobalAction::OpenKeymap),
            (KeyBind::from('s'), GlobalAction::OpenSettings),
            (KeyBind::from('x'), GlobalAction::Dismiss),
        ];

        for (key, expected) in cases {
            assert_eq!(
                map.action_for(&key),
                Some(expected),
                "expected {key:?} → {expected:?}",
            );
        }

        for variant in GlobalAction::ALL {
            assert!(
                !map.display_keys_for(*variant).is_empty(),
                "every default variant must be bound: {variant:?}",
            );
        }
    }
}
