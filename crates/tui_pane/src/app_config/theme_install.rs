//! [`install_theme`]: build the theme registry an app's `[appearance]`
//! table is resolved against, and install it process-wide.

use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;

use super::appearance::AppearanceConfig;
use super::identity::AppIdentity;
use crate::ResolvedTheme;
use crate::Theme;
use crate::ThemeFamily;
use crate::ThemeId;
use crate::ThemeLoadError;
use crate::ThemeRegistry;
use crate::ThemeSource;
use crate::ThemeState;
use crate::ThemeVariant;
use crate::install_theme_state;
use crate::set_transparent_background;
use crate::theme;
use crate::theme::RememberedAppearance;

/// Whether the configured theme name resolved during installation.
#[derive(Debug, Default)]
pub enum ActiveThemeOutcome {
    /// The registry contains the configured theme.
    #[default]
    Found,
    /// The resolver used a fallback because the registry lacks this `ThemeId`.
    MissingName(ThemeId),
}

/// One theme file that the loader could not read or parse.
#[derive(Debug)]
pub struct RejectedThemeFile {
    /// Path to the rejected theme file.
    pub path:  PathBuf,
    /// The loader error for this file.
    pub error: ThemeLoadError,
}

/// The active theme outcome and the separate file diagnostics.
#[derive(Debug, Default)]
pub struct ThemeInstallation {
    /// The result of resolving the configured theme name.
    pub active:   ActiveThemeOutcome,
    /// Every theme file the registry loader rejected.
    pub rejected: Vec<RejectedThemeFile>,
}

/// Controls whether theme installation uses the remembered system appearance.
#[derive(Clone, Copy)]
enum RememberedAppearancePolicy {
    /// Read the process-wide observation after loading the registry.
    Use,
    /// Resolve as if the process has no system appearance observation.
    Ignore,
}

struct InstalledTheme {
    resolved_theme: ResolvedTheme,
    rejected:       Vec<RejectedThemeFile>,
}

/// Install the theme `appearance` selects, process-wide.
///
/// Builds the theme registry from the app's `builtins` plus every
/// `*.toml` in [`AppIdentity::themes_dir`], resolves `appearance`
/// against it, and installs both so the color helpers
/// ([`active_border_color`](crate::active_border_color),
/// [`label_color`](crate::label_color), …) read them.
///
/// Returns the note shown when a configured theme id matched nothing
/// and another variant was substituted.
#[must_use]
pub fn install_theme<I: AppIdentity>(
    appearance: &AppearanceConfig<I>,
    builtins: Vec<ThemeVariant>,
) -> Option<String> {
    theme::resolution_notice(
        &install_from_source(appearance, RememberedAppearancePolicy::Use, || {
            ThemeRegistry::from_dir_with_builtins(I::themes_dir().as_deref(), builtins)
        })
        .resolved_theme,
    )
}

/// Install a theme with an app's parser.
///
/// [`ThemeSource::Directory`] uses `parse` for files in the directory.
/// [`ThemeSource::BuiltinsOnly`] uses the app's built-in variants.
/// The `auto` mode resolves without the remembered system appearance.
/// The `ThemeInstallation` reports a missing name and any rejected files.
#[must_use]
pub fn install_theme_with_parser<I: AppIdentity, E: Display>(
    appearance: &AppearanceConfig<I>,
    builtins: Vec<ThemeVariant>,
    source: ThemeSource<'_>,
    parse: impl Fn(&str) -> Result<ThemeFamily, E>,
) -> ThemeInstallation {
    let InstalledTheme {
        resolved_theme,
        rejected,
    } = install_from_source(appearance, RememberedAppearancePolicy::Ignore, || {
        ThemeRegistry::from_dir_with_parser(source, builtins, parse)
    });
    let active = resolved_theme
        .miss
        .map_or(ActiveThemeOutcome::Found, ActiveThemeOutcome::MissingName);
    ThemeInstallation { active, rejected }
}

fn install_from_source<I: AppIdentity>(
    appearance: &AppearanceConfig<I>,
    remembered_appearance_policy: RememberedAppearancePolicy,
    load_registry: impl FnOnce() -> ThemeRegistry,
) -> InstalledTheme {
    let theme_registry = load_registry();
    let remembered_appearance = match remembered_appearance_policy {
        RememberedAppearancePolicy::Use => theme::remembered_system_appearance(),
        RememberedAppearancePolicy::Ignore => RememberedAppearance::NotObserved,
    };
    let resolved_theme = resolve_appearance_for(&theme_registry, appearance, remembered_appearance);
    let rejected = theme_registry
        .status()
        .failed_files
        .iter()
        .map(|(path, error)| RejectedThemeFile {
            path:  path.clone(),
            error: error.clone(),
        })
        .collect();
    install_theme_state(ThemeState::with_registry(
        theme_registry,
        (*resolved_theme.theme).clone(),
    ));
    set_transparent_background(appearance.transparent);
    InstalledTheme {
        resolved_theme,
        rejected,
    }
}

/// The theme `appearance` selects from `registry`, and the note to show
/// when a configured theme id matched nothing and another variant was
/// substituted.
pub(crate) fn resolve_appearance<I: AppIdentity>(
    theme_registry: &ThemeRegistry,
    appearance: &AppearanceConfig<I>,
) -> (Arc<Theme>, Option<String>) {
    let resolved_theme = resolve_appearance_for(
        theme_registry,
        appearance,
        theme::remembered_system_appearance(),
    );
    let note = theme::resolution_notice(&resolved_theme);
    (resolved_theme.theme, note)
}

fn resolve_appearance_for<I: AppIdentity>(
    theme_registry: &ThemeRegistry,
    appearance: &AppearanceConfig<I>,
    remembered_appearance: RememberedAppearance,
) -> ResolvedTheme {
    theme_registry.resolve_active(
        &appearance.mode,
        &appearance.light_theme,
        &appearance.dark_theme,
        remembered_appearance.observed(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use tempfile::TempDir;

    use super::ActiveThemeOutcome;
    use super::RememberedAppearancePolicy;
    use super::install_from_source;
    use super::install_theme_with_parser;
    use crate::AppIdentity;
    use crate::Appearance;
    use crate::AppearanceConfig;
    use crate::ThemeId;
    use crate::ThemeRegistry;
    use crate::ThemeSource;
    use crate::ThemeVariant;
    use crate::apply_system_appearance;
    use crate::fallback_theme;

    const CUSTOM_THEME_NAME: &str = "Sample Dark";
    const CUSTOM_THEME_PREFIX: &str = "custom theme:\n";
    const MISSING_THEME_NAME: &str = "Missing Dark";
    const REJECTED_THEME_REASON: &str = "custom theme prefix required";

    struct TestApp;

    impl AppIdentity for TestApp {
        const BINARY_NAME: &'static str = "theme-install-test";
        const CONFIG_DIRNAME: &'static str = "theme-install-test";
        const DEFAULT_LIGHT_THEME: &'static str = "App Light";
        const DEFAULT_DARK_THEME: &'static str = "App Dark";
    }

    #[test]
    fn auto_theme_uses_the_appearance_observed_during_registry_load() {
        let appearance = AppearanceConfig::<TestApp>::default();
        let _ = apply_system_appearance(Appearance::Dark, &appearance);

        let installation =
            install_from_source(&appearance, RememberedAppearancePolicy::Use, || {
                let _ = apply_system_appearance(Appearance::Light, &appearance);
                ThemeRegistry::empty()
            });

        assert_eq!(installation.resolved_theme.appearance, Appearance::Light);
    }

    #[test]
    fn custom_theme_install_reports_rejected_file_and_missing_name() -> io::Result<()> {
        let directory = TempDir::new()?;
        let rejected_path = directory.path().join("rejected.toml");
        let family = include_str!("../theme/testdata/sample_family.toml");
        fs::write(
            directory.path().join("custom.toml"),
            format!("{CUSTOM_THEME_PREFIX}{family}"),
        )?;
        fs::write(&rejected_path, family)?;
        let mut appearance = AppearanceConfig::<TestApp>::default();
        appearance.dark_theme = MISSING_THEME_NAME.to_owned();
        let builtin = ThemeVariant {
            id:         ThemeId::new(TestApp::DEFAULT_DARK_THEME),
            appearance: Appearance::Dark,
            theme:      fallback_theme(Appearance::Dark),
        };

        let installation = install_theme_with_parser(
            &appearance,
            vec![builtin],
            ThemeSource::Directory(directory.path()),
            |contents| {
                contents
                    .strip_prefix(CUSTOM_THEME_PREFIX)
                    .ok_or_else(|| REJECTED_THEME_REASON.to_owned())
                    .and_then(|family| toml::from_str(family).map_err(|error| error.to_string()))
            },
        );

        assert!(matches!(
            installation.active,
            ActiveThemeOutcome::MissingName(name) if name.as_str() == MISSING_THEME_NAME
        ));
        let rejected = installation
            .rejected
            .iter()
            .map(|file| (&file.path, file.error.message()))
            .collect::<Vec<_>>();
        let expected_reason = format!("parse error: {REJECTED_THEME_REASON}");
        assert_eq!(rejected, [(&rejected_path, expected_reason.as_str())]);
        // Nextest gives this test its own process-wide `ThemeState`.
        assert!(
            crate::registry()
                .find(&ThemeId::new(CUSTOM_THEME_NAME))
                .is_some()
        );
        Ok(())
    }
}
