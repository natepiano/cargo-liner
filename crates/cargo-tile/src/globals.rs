//! The app-globals scope: this template's extension point for global
//! shortcuts that are not the framework's own.
//!
//! [`tui_pane::GlobalAction`] owns quit, restart, pane cycling, and the
//! settings / keymap / shortcut overlays — those need no registration
//! here. This scope is for the shortcuts a *particular* app adds on top,
//! and it starts empty: [`AppGlobalAction`] has no variants, so the
//! status line shows only framework globals and `[global]` in
//! `keymap.toml` accepts only framework action names.
//!
//! To add one, give the enum a variant through
//! [`tui_pane::action_enum!`], list it in [`Globals::render_order`], bind
//! a default key in [`Globals::defaults`], and handle it in
//! [`dispatch`]. The framework picks up the rest — TOML loading, the
//! status-line slot, and the row in the keymap overlay.

use std::fmt;
use std::fmt::Display;
use std::fmt::Formatter;

use tui_pane::Action;
use tui_pane::Bindings;
use tui_pane::Globals;

use crate::app::App;
use crate::constants::APP_GLOBALS_SECTION;

/// This app's global actions. Uninhabited until the app adds one, which
/// is what makes every method below trivially exhaustive.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppGlobalAction {}

impl Display for AppGlobalAction {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.description())
    }
}

impl Action for AppGlobalAction {
    const ALL: &'static [Self] = &[];

    fn toml_key(self) -> &'static str { match self {} }

    fn bar_label(self) -> &'static str { match self {} }

    fn description(self) -> &'static str { match self {} }

    fn from_toml_key(_key: &str) -> Option<Self> { None }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    const SECTION_NAME: &'static str = APP_GLOBALS_SECTION;

    fn render_order() -> &'static [Self::Actions] { Self::ALL }

    fn defaults() -> Bindings<Self::Actions> { Bindings::new() }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one app-global action. Unreachable while [`AppGlobalAction`] has
/// no variants, and the place to grow a `match` once it does.
const fn dispatch(action: AppGlobalAction, _app: &mut App) { match action {} }
