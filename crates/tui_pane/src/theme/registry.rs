//! Registry of theme variants — built-ins plus user-loaded.
//!
//! The registry is the single source of truth for "what themes exist
//! right now." The client app seeds it with the variants it compiles
//! in ([`ThemeRegistry::new_with_builtins`]) and adds whatever the
//! user's themes directory yields ([`ThemeRegistry::register`]); the
//! resolver then matches config theme names against it and the
//! settings UI lists it.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;
use std::path::PathBuf;
use std::sync::Arc;

use super::Appearance;
use super::Theme;

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
/// an existing one with the same id — client scan code can record
/// overrides so the user can see them.
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

    /// Seed the registry with the variants the app compiles in.
    ///
    /// The framework contributes no colors: `builtins` is entirely the
    /// app's own set, listed in the order it wants the settings UI to
    /// offer them. User `themes/*.toml` variants layer on top via
    /// [`register`](Self::register), overriding by id.
    #[must_use]
    pub fn new_with_builtins(builtins: Vec<ThemeVariant>) -> Self {
        Self {
            variants: builtins,
            status:   RegistryStatus::default(),
        }
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

    /// Mutate every registered theme while preserving registry order
    /// and status metadata.
    pub fn update_themes(&mut self, mut update: impl FnMut(&ThemeId, Appearance, &mut Theme)) {
        for variant in &mut self.variants {
            update(&variant.id, variant.appearance, &mut variant.theme);
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
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;
    use crate::theme;

    /// Stand-in for an app-supplied variant. The id is stamped into
    /// `roles` so two variants of the same appearance stay
    /// distinguishable by value.
    fn dummy_variant(id: &str, appearance: Appearance) -> ThemeVariant {
        let mut theme = theme::fallback_theme(appearance);
        theme.roles.insert(id.to_owned(), theme.text.default);
        ThemeVariant {
            id: ThemeId::new(id),
            appearance,
            theme,
        }
    }

    #[test]
    fn new_with_builtins_takes_the_app_supplied_set() {
        let registry = ThemeRegistry::new_with_builtins(vec![
            dummy_variant("App Dark", Appearance::Dark),
            dummy_variant("App Light", Appearance::Light),
        ]);
        assert_eq!(registry.len(), 2);
        assert!(registry.find(&ThemeId::new("App Dark")).is_some());
        assert!(registry.find(&ThemeId::new("App Light")).is_some());
        assert!(
            registry.status().overridden.is_empty(),
            "seeding is not an override"
        );
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
        let mut registry = ThemeRegistry::new_with_builtins(vec![
            dummy_variant("App Dark", Appearance::Dark),
            dummy_variant("App Light", Appearance::Light),
        ]);
        let replacement = dummy_variant("Overridden", Appearance::Dark);
        let replacement = ThemeVariant {
            id: ThemeId::new("App Dark"),
            ..replacement
        };
        let expected = replacement.theme.clone();
        let outcome = registry.register(replacement);
        assert_eq!(outcome, RegisterOutcome::Overrode(ThemeId::new("App Dark")));
        assert_eq!(registry.len(), 2, "override must replace in place");
        assert_eq!(
            registry
                .find(&ThemeId::new("App Dark"))
                .expect("replacement findable")
                .theme,
            expected,
            "override took effect"
        );
        assert_eq!(registry.status().overridden, vec![ThemeId::new("App Dark")]);
    }

    #[test]
    fn variants_by_appearance_filters() {
        let registry = ThemeRegistry::new_with_builtins(vec![
            dummy_variant("App Dark", Appearance::Dark),
            dummy_variant("App Light", Appearance::Light),
            dummy_variant("App HC Dark", Appearance::Dark),
        ]);
        let darks: Vec<_> = registry
            .variants_by_appearance(Appearance::Dark)
            .map(|v| v.id.as_str())
            .collect();
        let lights: Vec<_> = registry
            .variants_by_appearance(Appearance::Light)
            .map(|v| v.id.as_str())
            .collect();
        assert_eq!(darks, vec!["App Dark", "App HC Dark"]);
        assert_eq!(lights, vec!["App Light"]);
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
