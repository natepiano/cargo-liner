//! `Action` trait + `action_enum!` macro: vocabulary every action
//! enum implements so it can flow through `ScopeMap`, the bar, and the
//! TOML loader behind a single bound.

use core::fmt::Debug;
use core::fmt::Display;
use core::hash::Hash;

/// Marker plus minimal vocabulary every action enum implements.
///
/// Implemented automatically by the [`action_enum!`](crate::action_enum)
/// macro. Hand-rolled impls are allowed but unusual; the framework's
/// own [`GlobalAction`](crate::keymap::GlobalAction) is the one
/// hand-rolled case.
///
/// Super-traits chosen so generic code (`ScopeMap<A: Action>`,
/// keymap-overlay rendering, TOML round-trip) needs only one bound,
/// not five.
pub trait Action: Copy + Eq + Hash + Debug + Display + 'static {
    /// Every variant of `Self`, in declaration order. Stable across runs.
    const ALL: &'static [Self];

    /// Identifier used in TOML config keys (e.g. `"activate"`,
    /// `"expand_all"`). Must be stable — TOML files are user-edited.
    fn toml_key(self) -> &'static str;

    /// Default short label rendered in the bar (e.g. `"activate"`,
    /// `"clean"`). The pane's `Shortcuts::label` returns this by
    /// default; overrides only fire when the label is state-dependent.
    fn bar_label(self) -> &'static str;

    /// Human-readable description used by the keymap-overlay help.
    /// `Display::fmt` delegates to this.
    fn description(self) -> &'static str;

    /// Inverse of [`Self::toml_key`]. Returns `None` for unknown
    /// identifiers; the TOML loader attaches scope context and surfaces
    /// a `KeymapError::UnknownAction`.
    fn from_toml_key(key: &str) -> Option<Self>;
}

/// Declares an action enum and implements [`Action`] +
/// [`Display`](core::fmt::Display) for it.
///
/// Grammar:
///
/// ```text
/// action_enum! {
///     #[derive(...)]
///     pub enum Name {
///         Variant => ("toml_key", "bar_label", "description");
///         ...
///     }
/// }
/// ```
///
/// At least one variant is required — empty bodies are rejected at
/// expansion time. The caller supplies
/// `#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]`; those are
/// super-trait requirements of [`Action`] that the macro does not
/// inject silently.
///
/// Example:
///
/// ```ignore
/// tui_pane::action_enum! {
///     #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
///     pub enum NavAction {
///         Up   => ("up",   "up",   "Move up");
///         Down => ("down", "down", "Move down");
///     }
/// }
/// ```
#[macro_export]
macro_rules! action_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Name:ident {
            $( $Variant:ident => ( $toml_key:literal , $bar:literal , $desc:literal ) ; )+
        }
    ) => {
        $(#[$meta])*
        $vis enum $Name {
            $( $Variant, )+
        }

        impl $crate::Action for $Name {
            const ALL: &'static [Self] = &[ $( Self::$Variant, )+ ];

            fn toml_key(self) -> &'static str {
                match self {
                    $( Self::$Variant => $toml_key, )+
                }
            }

            fn bar_label(self) -> &'static str {
                match self {
                    $( Self::$Variant => $bar, )+
                }
            }

            fn description(self) -> &'static str {
                match self {
                    $( Self::$Variant => $desc, )+
                }
            }

            fn from_toml_key(key: &str) -> ::core::option::Option<Self> {
                match key {
                    $( $toml_key => ::core::option::Option::Some(Self::$Variant), )+
                    _ => ::core::option::Option::None,
                }
            }
        }

        impl ::core::fmt::Display for $Name {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                f.write_str(<Self as $crate::Action>::description(*self))
            }
        }
    };
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::Action;

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum Foo {
            A => ("a", "alpha", "alpha-desc");
            B => ("b", "beta",  "beta-desc");
        }
    }

    #[test]
    fn all_in_declaration_order() {
        assert_eq!(Foo::ALL, &[Foo::A, Foo::B]);
    }

    #[test]
    fn toml_keys_match_declaration() {
        assert_eq!(Foo::A.toml_key(), "a");
        assert_eq!(Foo::B.toml_key(), "b");
    }

    #[test]
    fn bar_labels_match_declaration() {
        assert_eq!(Foo::A.bar_label(), "alpha");
        assert_eq!(Foo::B.bar_label(), "beta");
    }

    #[test]
    fn descriptions_match_declaration() {
        assert_eq!(Foo::A.description(), "alpha-desc");
        assert_eq!(Foo::B.description(), "beta-desc");
    }

    #[test]
    fn from_toml_key_round_trips_every_variant() {
        for variant in Foo::ALL {
            assert_eq!(Foo::from_toml_key(variant.toml_key()), Some(*variant));
        }
    }

    #[test]
    fn from_toml_key_unknown_returns_none() {
        assert_eq!(Foo::from_toml_key("zzz"), None);
        assert_eq!(Foo::from_toml_key(""), None);
    }

    #[test]
    fn display_delegates_to_description() {
        assert_eq!(format!("{}", Foo::A), "alpha-desc");
        assert_eq!(format!("{}", Foo::B), "beta-desc");
    }
}
