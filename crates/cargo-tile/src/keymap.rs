//! Keymap assembly — the `cargo-tile` counterpart of cargo-port's
//! `build_framework_keymap`.
//!
//! The framework globals — `s` settings, ctrl-k keymap, `?` shortcuts,
//! `q` quit, `R` restart, `x` dismiss, Tab pane cycling — come from
//! [`tui_pane::GlobalAction`]'s defaults and need no registration.
//! `register_globals` folds in this app's own globals scope (empty in
//! the template, see [`crate::globals`]), `register_overlay` the
//! overlay-local scope, and `register_pane` installs each app pane in
//! the framework's pane registry. `keymap.toml`, when present,
//! overrides any of it.

use std::path::PathBuf;

use tui_pane::CycleDirection;
use tui_pane::Framework;
use tui_pane::Keymap;
use tui_pane::KeymapError;
use tui_pane::Pane;

use crate::app::App;
use crate::app::AppPaneId;
use crate::globals::AppGlobalAction;

/// `Pane<App>` host for the main content pane. No pane-local shortcuts
/// yet, so it registers through `register_pane` rather than `register`.
struct MainPane;

impl Pane<App> for MainPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Main;

    /// Tab walks the tile grid rather than the framework's pane cycle.
    /// The app registers one pane and puts every command in a cell
    /// inside it, so the cells are what a developer means by "the next
    /// one" -- and the step never falls through, because there is no
    /// second pane behind the grid to fall through to.
    fn cycle_step() -> Option<fn(&mut App, CycleDirection) -> bool> {
        Some(|app, direction| app.tiles.cycle_focus(direction))
    }
}

/// Assemble the keymap and install its pane registry on `framework`.
///
/// Built in [`ignore_unknown_entries`](tui_pane::KeymapBuilder::ignore_unknown_entries)
/// mode so a stale `keymap.toml` entry is skipped rather than failing
/// startup.
pub(crate) fn build_keymap(
    framework: &mut Framework<App>,
    keymap_path: Option<PathBuf>,
) -> Result<Keymap<App>, KeymapError> {
    let mut builder = Keymap::builder().ignore_unknown_entries();
    if let Some(path) = keymap_path {
        builder = builder.config_path(path.clone());
        if path.is_file() {
            builder = builder.load_toml(path)?;
        }
    }
    builder
        .register_globals::<AppGlobalAction>()?
        .register_overlay()?
        .register_pane::<MainPane>()
        .build_into(framework)
}
