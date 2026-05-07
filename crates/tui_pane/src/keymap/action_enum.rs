//! `ActionEnum` trait + `action_enum!` macro: vocabulary every action
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
/// Super-traits chosen so generic code (`ScopeMap<A: ActionEnum>`,
/// keymap-overlay rendering, TOML round-trip) needs only one bound,
/// not five.
pub trait ActionEnum: Copy + Eq + Hash + Debug + Display + 'static {
    /// Every variant of `Self`, in declaration order. Stable across runs.
    const ALL: &'static [Self];

    /// Identifier used in TOML config keys (e.g. `"activate"`,
    /// `"expand_all"`). Must be stable — TOML files are user-edited.
    fn toml_key(self) -> &'static str;

    /// Human-readable label rendered in the bar and the keymap overlay.
    /// `Display::fmt` delegates to this.
    fn description(self) -> &'static str;

    /// Inverse of [`Self::toml_key`]. Returns `None` for unknown
    /// identifiers; the TOML loader attaches scope context and surfaces
    /// a `KeymapError::UnknownAction`.
    fn from_toml_key(key: &str) -> Option<Self>;
}

/// Declares an action enum and implements [`ActionEnum`] +
/// [`Display`](core::fmt::Display) for it.
///
/// Grammar:
///
/// ```text
/// action_enum! {
///     #[derive(...)]
///     pub enum Name {
///         Variant => "toml_key", "description";
///         ...
///     }
/// }
/// ```
///
/// At least one variant is required — empty bodies are rejected at
/// expansion time. The caller supplies
/// `#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]`; those are
/// super-trait requirements of [`ActionEnum`] that the macro does not
/// inject silently.
///
/// Example:
///
/// ```ignore
/// tui_pane::action_enum! {
///     #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
///     pub enum NavAction {
///         Up   => "up",   "Move up";
///         Down => "down", "Move down";
///     }
/// }
/// ```
#[macro_export]
macro_rules! action_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $Name:ident {
            $( $Variant:ident => $toml_key:literal, $desc:literal ; )+
        }
    ) => {
        $(#[$meta])*
        $vis enum $Name {
            $( $Variant, )+
        }

        impl $crate::ActionEnum for $Name {
            const ALL: &'static [Self] = &[ $( Self::$Variant, )+ ];

            fn toml_key(self) -> &'static str {
                match self {
                    $( Self::$Variant => $toml_key, )+
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
                f.write_str(<Self as $crate::ActionEnum>::description(*self))
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
    use super::ActionEnum;

    crate::action_enum! {
        #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
        pub enum Foo {
            A => "a", "alpha";
            B => "b", "beta";
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
    fn descriptions_match_declaration() {
        assert_eq!(Foo::A.description(), "alpha");
        assert_eq!(Foo::B.description(), "beta");
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
        assert_eq!(format!("{}", Foo::A), "alpha");
        assert_eq!(format!("{}", Foo::B), "beta");
    }
}
