//! [`install_theme`]: build the theme registry an app's `[appearance]`
//! table is resolved against, and install it process-wide.

use std::sync::Arc;

use super::appearance::AppearanceConfig;
use super::identity::AppIdentity;
use crate::Theme;
use crate::ThemeRegistry;
use crate::ThemeState;
use crate::ThemeVariant;
use crate::install_theme_state;

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
    let themes_dir = I::themes_dir();
    let registry = ThemeRegistry::from_dir_with_builtins(themes_dir.as_deref(), builtins);
    let (theme, note) = resolve_appearance(&registry, appearance);
    let initial_theme = (*theme).clone();
    install_theme_state(ThemeState::with_registry(registry, initial_theme));
    note
}

/// The theme `appearance` selects from `registry`, and the note to show
/// when a configured theme id matched nothing and another variant was
/// substituted.
pub(crate) fn resolve_appearance<I: AppIdentity>(
    registry: &ThemeRegistry,
    appearance: &AppearanceConfig<I>,
) -> (Arc<Theme>, Option<String>) {
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
    (resolved.theme, note)
}
