//! Handler theme files require their own five-entry session palette.

use std::path::PathBuf;

use ratatui::style::Color;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::StyleSpec;
use tui_pane::ThemeFamily;
use tui_pane::ThemeId;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeSource;
use tui_pane::ThemeState;
use tui_pane::ThemeVariant;
use tui_pane::ThemeVariantFile;

use super::SessionHue;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HandlerThemeFamily {
    schema:   u32,
    name:     String,
    variants: Vec<HandlerThemeVariant>,
}

/// Parsing establishes the palette's cardinality before it enters the domain.
struct HandlerThemeVariant {
    session_hues: [Color; SessionHue::ALL.len()],
    shared:       ThemeVariantFile,
}

impl<'de> Deserialize<'de> for HandlerThemeVariant {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut fields = toml::Table::deserialize(deserializer)?;
        let hues = fields
            .remove("session_hues")
            .ok_or_else(|| D::Error::missing_field("session_hues"))?;
        let styles: [StyleSpec; SessionHue::ALL.len()] =
            hues.try_into().map_err(D::Error::custom)?;
        let shared = fields.try_into().map_err(D::Error::custom)?;
        Ok(Self {
            session_hues: styles.map(|style| style.color),
            shared,
        })
    }
}

/// Lower only at the shared renderer boundary; the file schema stays handler-owned.
pub(super) fn parse_family(contents: &str) -> Result<ThemeFamily, toml::de::Error> {
    let family: HandlerThemeFamily = toml::from_str(contents)?;
    Ok(ThemeFamily {
        schema:   family.schema,
        name:     family.name,
        variants: family
            .variants
            .into_iter()
            .map(|variant| {
                let mut shared = variant.shared;
                for (hue, color) in SessionHue::ALL.into_iter().zip(variant.session_hues) {
                    shared
                        .roles
                        .insert(hue.key().to_owned(), StyleSpec::from_color(color));
                }
                shared
            })
            .collect(),
    })
}

/// Whether the configured active theme was found at installation.
#[derive(Debug, Default)]
enum ActiveThemeOutcome {
    #[default]
    Found,
    BuiltinForMissingName(ThemeId),
}

/// One file rejected by the handler schema or filesystem loader.
#[derive(Debug)]
struct RejectedThemeFile {
    path:   PathBuf,
    reason: String,
}

/// The active-theme outcome and the independent file-loading diagnostics.
#[derive(Debug, Default)]
pub(crate) struct ThemeInstallation {
    active:   ActiveThemeOutcome,
    rejected: Vec<RejectedThemeFile>,
}

/// The latest response from the framework appearance resolver.
#[derive(Debug, Default)]
pub(crate) enum AppearanceDiagnostic {
    #[default]
    NotObserved,
    Clear,
    Notice(String),
}

impl From<Option<String>> for AppearanceDiagnostic {
    fn from(note: Option<String>) -> Self { note.map_or(Self::Clear, Self::Notice) }
}

/// Installation diagnostics persist independently of appearance changes.
#[derive(Debug, Default)]
pub(crate) struct StartupThemeNotes {
    installation:          ThemeInstallation,
    pub(crate) appearance: AppearanceDiagnostic,
}

impl StartupThemeNotes {
    /// Combine the installation report with the first appearance answer.
    pub(crate) const fn new(
        installation: ThemeInstallation,
        appearance: AppearanceDiagnostic,
    ) -> Self {
        Self {
            installation,
            appearance,
        }
    }

    /// Convert only for framework APIs that require an optional diagnostic.
    pub(crate) fn appearance_note(&self) -> Option<String> {
        match &self.appearance {
            AppearanceDiagnostic::Notice(note) => Some(note.clone()),
            AppearanceDiagnostic::NotObserved | AppearanceDiagnostic::Clear => None,
        }
    }

    /// File failures outlive any later successful theme resolution.
    pub(crate) fn messages(&self) -> Vec<String> {
        let mut messages: Vec<_> = self
            .installation
            .rejected
            .iter()
            .map(|file| {
                format!(
                    "theme file `{}` rejected: {}",
                    file.path.display(),
                    file.reason
                )
            })
            .collect();
        match &self.appearance {
            AppearanceDiagnostic::Notice(note) => messages.push(note.clone()),
            AppearanceDiagnostic::NotObserved => {
                if let ActiveThemeOutcome::BuiltinForMissingName(name) = &self.installation.active {
                    messages.push(format!("theme `{name}` not found — using a built-in"));
                }
            },
            AppearanceDiagnostic::Clear => {},
        }
        messages
    }
}

/// Install handler files before the appearance watcher supplies its first observation.
pub(crate) fn install_theme<I: AppIdentity>(
    appearance: &AppearanceConfig<I>,
    builtins: Vec<ThemeVariant>,
) -> ThemeInstallation {
    let registry = match I::themes_dir() {
        Some(dir) => ThemeRegistry::from_dir_with_parser(
            ThemeSource::Directory(&dir),
            builtins,
            parse_family,
        ),
        None => ThemeRegistry::new_with_builtins(builtins),
    };
    let resolved = registry.resolve_active(
        &appearance.mode,
        &appearance.light_theme,
        &appearance.dark_theme,
        None,
    );
    let active = resolved.miss.map_or(
        ActiveThemeOutcome::Found,
        ActiveThemeOutcome::BuiltinForMissingName,
    );
    let rejected = registry
        .status()
        .failed_files
        .iter()
        .map(|(path, error)| RejectedThemeFile {
            path:   path.clone(),
            reason: error.message().to_owned(),
        })
        .collect();
    tui_pane::install_theme_state(ThemeState::with_registry(
        registry,
        (*resolved.theme).clone(),
    ));
    tui_pane::set_transparent_background(appearance.transparent);
    ThemeInstallation { active, rejected }
}
