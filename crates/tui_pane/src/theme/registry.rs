//! Registry of theme variants — built-ins plus user-loaded.
//!
//! The registry is the single source of truth for "what themes exist
//! right now." Phase 2 owns its construction (built-ins seeded by
//! [`ThemeRegistry::new_with_builtins`]; user themes registered via
//! [`ThemeRegistry::register`] from cargo-port-side scan code). Phase
//! 3 resolves config theme names against this registry; Phase 4
//! populates settings dropdowns from it.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::sync::Arc;

use super::Appearance;
use super::Theme;
use super::builtins;

/// Name of the built-in dark variant. Stable identifier used by config.
pub const BUILTIN_DARK_NAME: &str = "Default Dark";
/// Name of the built-in light variant. Stable identifier used by config.
pub const BUILTIN_LIGHT_NAME: &str = "Default Light";

/// Cheaply cloneable identifier for a theme variant. Backed by an
/// `Arc<str>` so the registry, config, and runtime references share
/// one allocation per name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThemeId(Arc<str>);

impl ThemeId {
    /// Build a [`ThemeId`] from any string-like value.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self { Self(name.into()) }

    /// Borrow the underlying name.
    #[must_use]
    pub fn as_str(&self) -> &str { &self.0 }
}

impl Display for ThemeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { f.write_str(&self.0) }
}

impl From<&str> for ThemeId {
    fn from(value: &str) -> Self { Self::new(value) }
}

impl From<String> for ThemeId {
    fn from(value: String) -> Self { Self::new(value) }
}

/// A registered theme variant — id, appearance target, and the [`Theme`]
/// itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeVariant {
    /// Unique identifier for this variant.
    pub id:         ThemeId,
    /// Whether the variant is designed for a light or dark terminal.
    pub appearance: Appearance,
    /// The palette consumed by the render layer.
    pub theme:      Theme,
}

/// Outcome of [`ThemeRegistry::register`].
///
/// Tracks whether a register call inserted a fresh variant or replaced
/// an existing one with the same id — the cargo-port-side scan code
/// records overrides so the user can see them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterOutcome {
    /// New variant added.
    Inserted,
    /// Replaced an existing variant with the same id. Carries the id
    /// that was overridden.
    Overrode(ThemeId),
}

/// Single-line message describing why a theme file failed to load
/// (io error, parse error, schema mismatch). Stored in
/// [`RegistryStatus::failed_files`] so the UI can surface it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeLoadError {
    message: String,
}

impl ThemeLoadError {
    /// Wrap a message string.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// Borrow the underlying message.
    #[must_use]
    pub fn message(&self) -> &str { &self.message }
}

impl Display for ThemeLoadError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result { f.write_str(&self.message) }
}

/// Diagnostic side-data carried by the registry: which files failed to
/// load and which built-in ids were overridden by user variants. Both
/// are surfaced through the settings UI and startup toasts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistryStatus {
    /// Files that failed to load, paired with the reason.
    pub failed_files: Vec<(PathBuf, ThemeLoadError)>,
    /// Built-in ids that were overridden by a user variant.
    pub overridden:   Vec<ThemeId>,
}

/// Ordered list of theme variants plus diagnostic [`RegistryStatus`].
///
/// Lookups are linear in the number of variants — fine for the
/// expected single-digit-to-low-tens range. If a real user ever ships
/// hundreds of variants, swap the `Vec` for an `IndexMap` without
/// changing the public API.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeRegistry {
    variants: Vec<ThemeVariant>,
    status:   RegistryStatus,
}

impl ThemeRegistry {
    /// Empty registry. Tests use this to verify behavior without the
    /// built-in seeds present.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            variants: Vec::new(),
            status:   RegistryStatus::default(),
        }
    }

    /// Seed the registry with the two compiled-in variants
    /// ([`BUILTIN_DARK_NAME`] and [`BUILTIN_LIGHT_NAME`]).
    #[must_use]
    pub fn new_with_builtins() -> Self {
        let mut registry = Self::empty();
        registry.variants.push(ThemeVariant {
            id:         ThemeId::new(BUILTIN_DARK_NAME),
            appearance: Appearance::Dark,
            theme:      builtins::default_dark(),
        });
        registry.variants.push(ThemeVariant {
            id:         ThemeId::new(BUILTIN_LIGHT_NAME),
            appearance: Appearance::Light,
            theme:      builtins::default_light(),
        });
        registry
    }

    /// Register a variant. If an existing variant shares the id, it is
    /// replaced in place (preserving the registry's relative order
    /// for non-overridden entries) and the override is recorded in
    /// [`RegistryStatus::overridden`].
    pub fn register(&mut self, variant: ThemeVariant) -> RegisterOutcome {
        if let Some(slot) = self.variants.iter_mut().find(|v| v.id == variant.id) {
            let overridden_id = slot.id.clone();
            *slot = variant;
            self.status.overridden.push(overridden_id.clone());
            RegisterOutcome::Overrode(overridden_id)
        } else {
            self.variants.push(variant);
            RegisterOutcome::Inserted
        }
    }

    /// Record a file that failed to load. Surfaces in the settings UI.
    pub fn record_failed_file(&mut self, path: PathBuf, error: ThemeLoadError) {
        self.status.failed_files.push((path, error));
    }

    /// Look up a variant by id.
    #[must_use]
    pub fn find(&self, id: &ThemeId) -> Option<&ThemeVariant> {
        self.variants.iter().find(|v| &v.id == id)
    }

    /// Iterate every registered variant in insertion order.
    pub fn all(&self) -> impl Iterator<Item = &ThemeVariant> { self.variants.iter() }

    /// Iterate only variants whose `appearance` matches.
    pub fn variants_by_appearance(
        &self,
        appearance: Appearance,
    ) -> impl Iterator<Item = &ThemeVariant> {
        self.variants
            .iter()
            .filter(move |v| v.appearance == appearance)
    }

    /// Borrow the diagnostic status block (failed files + overrides).
    #[must_use]
    pub const fn status(&self) -> &RegistryStatus { &self.status }

    /// Count of registered variants.
    #[must_use]
    pub const fn len(&self) -> usize { self.variants.len() }

    /// True when no variants are registered.
    #[must_use]
    pub const fn is_empty(&self) -> bool { self.variants.is_empty() }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    fn dummy_variant(id: &str, appearance: Appearance) -> ThemeVariant {
        ThemeVariant {
            id: ThemeId::new(id),
            appearance,
            theme: builtins::default_dark(),
        }
    }

    #[test]
    fn new_with_builtins_seeds_two_named_variants() {
        let registry = ThemeRegistry::new_with_builtins();
        assert_eq!(registry.len(), 2);
        assert!(registry.find(&ThemeId::new(BUILTIN_DARK_NAME)).is_some());
        assert!(registry.find(&ThemeId::new(BUILTIN_LIGHT_NAME)).is_some());
    }

    #[test]
    fn register_inserts_new_variant() {
        let mut registry = ThemeRegistry::empty();
        let outcome = registry.register(dummy_variant("Catppuccin Mocha", Appearance::Dark));
        assert_eq!(outcome, RegisterOutcome::Inserted);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn register_replaces_existing_variant_with_same_id() {
        let mut registry = ThemeRegistry::new_with_builtins();
        let outcome = registry.register(dummy_variant(BUILTIN_DARK_NAME, Appearance::Dark));
        assert_eq!(
            outcome,
            RegisterOutcome::Overrode(ThemeId::new(BUILTIN_DARK_NAME))
        );
        assert_eq!(registry.len(), 2, "override must replace in place");
        assert_eq!(
            registry.status().overridden,
            vec![ThemeId::new(BUILTIN_DARK_NAME)]
        );
    }

    #[test]
    fn variants_by_appearance_filters() {
        let registry = ThemeRegistry::new_with_builtins();
        let darks: Vec<_> = registry
            .variants_by_appearance(Appearance::Dark)
            .map(|v| v.id.as_str())
            .collect();
        let lights: Vec<_> = registry
            .variants_by_appearance(Appearance::Light)
            .map(|v| v.id.as_str())
            .collect();
        assert_eq!(darks, vec![BUILTIN_DARK_NAME]);
        assert_eq!(lights, vec![BUILTIN_LIGHT_NAME]);
    }

    #[test]
    fn record_failed_file_accumulates_status() {
        let mut registry = ThemeRegistry::empty();
        registry.record_failed_file(
            PathBuf::from("/tmp/bad.toml"),
            ThemeLoadError::new("invalid color"),
        );
        assert_eq!(registry.status().failed_files.len(), 1);
        assert_eq!(
            registry.status().failed_files[0].1.message(),
            "invalid color"
        );
    }

    #[test]
    fn theme_id_round_trips_from_and_to_str() {
        let id = ThemeId::from("Catppuccin Mocha");
        assert_eq!(id.as_str(), "Catppuccin Mocha");
        assert_eq!(id.to_string(), "Catppuccin Mocha");
    }
}
