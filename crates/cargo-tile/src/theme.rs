//! Theme installation.
//!
//! Builds a [`ThemeRegistry`] from the compiled-in built-ins plus every
//! `*.toml` in the user's themes directory, resolves the `[appearance]`
//! selection against it, and publishes the result process-wide so the
//! `tui_pane` color helpers (`active_border_color`, `label_color`, …)
//! read it.

use std::path::Path;

use tui_pane::ThemeRegistry;
use tui_pane::ThemeState;

use crate::config::Config;

/// Install the theme `config` selects. Returns a note when the
/// configured theme id matched nothing and a built-in was substituted.
pub(crate) fn install(config: &Config, themes_dir: Option<&Path>) -> Option<String> {
    let registry = ThemeRegistry::from_dir_with_builtins(themes_dir);
    let resolved = registry.resolve_active(
        &config.appearance.mode,
        &config.appearance.light_theme,
        &config.appearance.dark_theme,
        None,
    );
    let note = resolved
        .miss
        .as_ref()
        .map(|missing| format!("theme `{missing}` not found — using a built-in"));
    let initial_theme = (*resolved.theme).clone();
    tui_pane::install_theme_state(ThemeState::with_registry(registry, initial_theme));
    note
}
