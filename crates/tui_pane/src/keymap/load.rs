//! Keymap loader error type.
//!
//! [`KeymapError`] is the variant set the TOML loader and the builder
//! emit. Parsing logic — file read, table walk, per-scope merge — lives
//! alongside `Keymap<Ctx>`.

use thiserror::Error;

use super::key_bind::KeyParseError;

/// Failures returned by the keymap loader.
///
/// Variants split into three groups by the producer that emits them:
///
/// - [`Self::Io`], [`Self::Parse`] — `?` propagation from the filesystem and `toml` deserializer.
/// - [`Self::InArrayDuplicate`], [`Self::CrossActionCollision`], [`Self::InvalidBinding`],
///   [`Self::UnknownAction`], [`Self::UnknownScope`] — semantic checks the loader runs after the
///   TOML parses but before the [`ScopeMap`](super::scope_map::ScopeMap) indexes are built.
///
/// `Display` impls are user-facing strings; the binary's startup path
/// renders them directly into the terminal on a config error.
#[derive(Debug, Error)]
pub enum KeymapError {
    /// `std::io::Error` opening the keymap file. A missing file is
    /// **not** an error — the loader treats it as "use defaults" and
    /// returns `Ok`.
    #[error("I/O error reading keymap config")]
    Io(#[from] std::io::Error),

    /// Top-level TOML parse failure.
    #[error("TOML parse error in keymap config")]
    Parse(#[from] toml::de::Error),

    /// A key string failed [`KeyBind::parse`](super::key_bind::KeyBind::parse)
    /// in a context with no scope/action attribution available — used
    /// for direct `?` propagation. Loader paths that already know their
    /// scope/action prefer [`Self::InvalidBinding`].
    #[error("invalid key binding")]
    KeyParse(#[from] KeyParseError),

    /// Two TOML keys in the same array refer to the same physical key.
    #[error("duplicate key '{key}' in {scope}.{action}")]
    InArrayDuplicate {
        /// TOML scope (table name) the array belongs to.
        scope:  String,
        /// Action whose key array contains the duplicate.
        action: String,
        /// The repeated key string.
        key:    String,
    },

    /// Two actions in the same scope bind to the same physical key.
    #[error(
        "key '{key}' bound to both {first} and {second} in [{scope}]",
        first = actions.0,
        second = actions.1,
    )]
    CrossActionCollision {
        /// TOML scope the collision occurred in.
        scope:   String,
        /// The colliding key string.
        key:     String,
        /// Pair of action TOML keys that fired on the same `key`.
        actions: (String, String),
    },

    /// A TOML key string failed [`KeyBind::parse`](super::key_bind::KeyBind::parse).
    #[error("invalid binding for {scope}.{action}")]
    InvalidBinding {
        /// TOML scope the bad binding belongs to.
        scope:  String,
        /// Action whose binding failed to parse.
        action: String,
        /// Underlying parse error, chained via `Display`/`source`.
        #[source]
        source: KeyParseError,
    },

    /// TOML referenced an unknown action in a known scope. The loader
    /// constructs this when `A::from_toml_key(key)` returns `None`,
    /// attaching the scope name from its current context.
    #[error("unknown action '{action}' in [{scope}]")]
    UnknownAction {
        /// TOML scope the unknown action appeared in.
        scope:  String,
        /// The unrecognized action TOML key.
        action: String,
    },

    /// TOML referenced an unknown scope name (top-level table).
    #[error("unknown scope [{scope}]")]
    UnknownScope {
        /// The unrecognized scope name.
        scope: String,
    },

    /// `KeymapBuilder::build` / `build_into` was called without
    /// [`KeymapBuilder::register_navigation`](super::KeymapBuilder::register_navigation).
    #[error("no Navigation impl registered on the keymap builder")]
    NavigationMissing,

    /// `KeymapBuilder::build` / `build_into` was called without
    /// [`KeymapBuilder::register_globals`](super::KeymapBuilder::register_globals).
    #[error("no Globals impl registered on the keymap builder")]
    GlobalsMissing,

    /// Two distinct types registered on the builder share the same
    /// [`AppPaneId`](crate::AppContext::AppPaneId). The second
    /// [`KeymapBuilder::register`](super::KeymapBuilder::register) call
    /// is the one that surfaces the error; `type_name` is the
    /// `core::any::type_name::<P>()` of the offender for diagnostics.
    #[error("duplicate scope registered for AppPaneId (offender: {type_name})")]
    DuplicateScope {
        /// `core::any::type_name::<P>()` of the second registrant.
        type_name: &'static str,
    },
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn display_in_array_duplicate() {
        let err = KeymapError::InArrayDuplicate {
            scope:  "package".to_string(),
            action: "activate".to_string(),
            key:    "Enter".to_string(),
        };
        assert_eq!(err.to_string(), "duplicate key 'Enter' in package.activate");
    }

    #[test]
    fn display_cross_action_collision() {
        let err = KeymapError::CrossActionCollision {
            scope:   "global".to_string(),
            key:     "q".to_string(),
            actions: ("quit".to_string(), "find".to_string()),
        };
        assert_eq!(
            err.to_string(),
            "key 'q' bound to both quit and find in [global]",
        );
    }

    #[test]
    fn display_invalid_binding_chains_source() {
        let err = KeymapError::InvalidBinding {
            scope:  "package".to_string(),
            action: "activate".to_string(),
            source: KeyParseError::Empty,
        };
        assert_eq!(err.to_string(), "invalid binding for package.activate");

        let source = std::error::Error::source(&err).expect("source must be set");
        assert_eq!(source.to_string(), "empty key string");
    }

    #[test]
    fn display_unknown_action() {
        let err = KeymapError::UnknownAction {
            scope:  "package".to_string(),
            action: "explode".to_string(),
        };
        assert_eq!(err.to_string(), "unknown action 'explode' in [package]");
    }

    #[test]
    fn display_unknown_scope() {
        let err = KeymapError::UnknownScope {
            scope: "frobnicate".to_string(),
        };
        assert_eq!(err.to_string(), "unknown scope [frobnicate]");
    }

    #[test]
    fn from_io_error() {
        let io = std::io::Error::other("disk on fire");
        let err: KeymapError = io.into();
        assert!(matches!(err, KeymapError::Io(_)));
        assert_eq!(err.to_string(), "I/O error reading keymap config");
    }

    #[test]
    fn from_key_parse_error_via_invalid_binding() {
        let source = KeyParseError::UnknownKey("Bogus".to_string());
        let err = KeymapError::InvalidBinding {
            scope: "package".to_string(),
            action: "activate".to_string(),
            source,
        };
        let chained = std::error::Error::source(&err).expect("source must be set");
        assert!(chained.to_string().contains("Bogus"));
    }

    #[test]
    fn from_key_parse_error_propagates_via_question_mark() {
        let source = KeyParseError::UnknownKey("Bogus".to_string());
        let err: KeymapError = source.into();
        assert!(matches!(err, KeymapError::KeyParse(_)));
        assert_eq!(err.to_string(), "invalid key binding");
        let chained = std::error::Error::source(&err).expect("source must be set");
        assert!(chained.to_string().contains("Bogus"));
    }
}
