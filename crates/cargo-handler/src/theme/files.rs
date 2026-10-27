//! Handler theme files require their own five-entry session palette.

use ratatui::style::Color;
use serde::Deserialize;
use serde::Deserializer;
use serde::de::Error;
use tui_pane::AppIdentity;
use tui_pane::AppearanceConfig;
use tui_pane::StyleSpec;
use tui_pane::ThemeFamily;
use tui_pane::ThemeRegistry;
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

/// Install handler files before the appearance watcher supplies its first observation.
pub(crate) fn install_theme<I: AppIdentity>(
    appearance: &AppearanceConfig<I>,
    builtins: Vec<ThemeVariant>,
) -> Option<String> {
    let registry = match I::themes_dir() {
        Some(dir) => ThemeRegistry::from_dir_with_parser(Some(&dir), builtins, parse_family),
        None => ThemeRegistry::new_with_builtins(builtins),
    };
    let resolved = registry.resolve_active(
        &appearance.mode,
        &appearance.light_theme,
        &appearance.dark_theme,
        None,
    );
    let note = resolved
        .miss
        .as_ref()
        .map(|missing| format!("theme `{missing}` not found — using a built-in"));
    tui_pane::install_theme_state(ThemeState::with_registry(
        registry,
        (*resolved.theme).clone(),
    ));
    tui_pane::set_transparent_background(appearance.transparent);
    note
}
