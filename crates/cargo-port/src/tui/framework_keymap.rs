//! Framework-side keymap scaffolding.
//!
//! Phase 14.2 introduces the `tui_pane`-driven keymap path beside the
//! legacy `src/keymap.rs` path. The two coexist through Phases 14–17;
//! Phase 18 retires the legacy path and folds dispatch onto the
//! framework. Until then every entry here is purely additive — the
//! binary's existing keymap remains authoritative, and the framework
//! keymap is built but not consulted for dispatch.
//!
//! Surface:
//!
//! - [`AppPaneId`]: every app-side pane id the framework will key on. Defined in full now to avoid
//!   a churn-rename on every later chunk.
//! - [`NavigationAction`]: directional nav enum the [`Navigation`] singleton routes through.
//! - [`AppGlobalAction`]: app-extension globals scope. Phase 14.2 ships a single placeholder
//!   variant ([`AppGlobalAction::Find`]); Phase 14.7 grows it to cover the rest of the binary's
//!   non-framework globals.
//! - [`AppNavigation`] / [`PackagePane`]: the `Navigation` and `Pane` + `Shortcuts` impls the
//!   builder typestate requires. Dispatcher fns are no-ops for now — the legacy path still owns key
//!   dispatch.
//! - [`build_framework_keymap`]: assembles a [`tui_pane::Keymap<App>`] using the canonical builder
//!   chain. Called once at startup.

#![allow(
    dead_code,
    reason = "Phase 14.2 introduces these types; later chunks (14.3–14.6) plug each pane in. \
              Variants/methods stay unconstructed in the binary path until Phase 18 swaps over."
)]

use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::Bindings;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Globals;
use tui_pane::Keymap;
use tui_pane::KeymapError;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;

use crate::keymap::GitAction;
use crate::keymap::PackageAction;
use crate::tui::app::App;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitRow;
use crate::tui::panes::PaneId;
use crate::tui::panes::git_row_at;
use crate::tui::panes::package_fields_from_data;

/// Stable identifier for every app-side pane the framework keys its
/// per-pane registries on. Defined in full at 14.2 so later chunks
/// (14.3–14.6) plug each pane in without renaming variants.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AppPaneId {
    ProjectList,
    Package,
    Lang,
    Cpu,
    Git,
    Targets,
    Lints,
    CiRuns,
    Output,
    Finder,
}

impl AppPaneId {
    /// Translation to the legacy [`PaneId`] enum so the parallel-path
    /// cutover bridges the new id back to the old. App-only variants
    /// only — framework panes (Toasts, Settings, Keymap) are not part
    /// of [`AppPaneId`].
    pub(crate) const fn to_legacy(self) -> PaneId {
        match self {
            Self::ProjectList => PaneId::ProjectList,
            Self::Package => PaneId::Package,
            Self::Lang => PaneId::Lang,
            Self::Cpu => PaneId::Cpu,
            Self::Git => PaneId::Git,
            Self::Targets => PaneId::Targets,
            Self::Lints => PaneId::Lints,
            Self::CiRuns => PaneId::CiRuns,
            Self::Output => PaneId::Output,
            Self::Finder => PaneId::Finder,
        }
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum NavigationAction {
        Up    => ("up",    "up",    "Move up");
        Down  => ("down",  "down",  "Move down");
        Left  => ("left",  "left",  "Move left");
        Right => ("right", "right", "Move right");
        Home  => ("home",  "home",  "Jump to start");
        End   => ("end",   "end",   "Jump to end");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum AppGlobalAction {
        Find => ("find", "find", "Open finder");
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }
}

/// `Navigation<App>` host. Zero-sized because the framework only needs
/// the type; navigation defaults / dispatch are static methods.
pub(crate) struct AppNavigation;

impl Navigation<App> for AppNavigation {
    type Actions = NavigationAction;

    const DOWN: Self::Actions = NavigationAction::Down;
    const END: Self::Actions = NavigationAction::End;
    const HOME: Self::Actions = NavigationAction::Home;
    const LEFT: Self::Actions = NavigationAction::Left;
    const RIGHT: Self::Actions = NavigationAction::Right;
    const UP: Self::Actions = NavigationAction::Up;

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Up    => NavigationAction::Up,
            crossterm::event::KeyCode::Down  => NavigationAction::Down,
            crossterm::event::KeyCode::Left  => NavigationAction::Left,
            crossterm::event::KeyCode::Right => NavigationAction::Right,
            crossterm::event::KeyCode::Home  => NavigationAction::Home,
            crossterm::event::KeyCode::End   => NavigationAction::End,
        }
    }

    fn dispatcher() -> fn(Self::Actions, FocusedPane<AppPaneId>, &mut App) {
        |_action, _focused, _ctx| {
            // No-op through Phase 17. The legacy navigation path
            // (handle_detail_key etc.) remains authoritative.
        }
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    fn render_order() -> &'static [Self::Actions] { Self::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            '/' => Self::Find,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) {
        |_action, _ctx| {
            // No-op through Phase 17. The legacy global dispatcher in
            // src/tui/input.rs remains authoritative.
        }
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Package detail pane.
pub(crate) struct PackagePane;

impl Pane<App> for PackagePane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Package;
}

impl Shortcuts<App> for PackagePane {
    type Actions = PackageAction;

    const SCOPE_NAME: &'static str = "package";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => PackageAction::Activate,
            'c' => PackageAction::Clean,
        }
    }

    fn state(&self, action: PackageAction, ctx: &App) -> ShortcutState {
        match action {
            PackageAction::Activate => activate_state(ctx),
            PackageAction::Clean => ShortcutState::Enabled,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) {
        |_action, _ctx| {
            // No-op through Phase 17. The legacy detail-key path in
            // src/tui/panes/actions.rs remains authoritative.
        }
    }
}

/// `Activate` on the Package pane is enabled only when the cursor is
/// on a row whose dispatch has a real effect. Today the legacy path
/// only opens a URL on the `CratesIo` row (see
/// `src/tui/panes/actions.rs::handle_detail_enter`); every other row
/// is a no-op. The `CratesIo` row itself is rendered only when
/// `crates_version` is known (see `package_fields_from_data`), so
/// `Activate` is implicitly disabled on packages without a crates.io
/// version too.
fn activate_state(ctx: &App) -> ShortcutState {
    let Some(pkg) = ctx.panes.package.content() else {
        return ShortcutState::Disabled;
    };
    let fields = package_fields_from_data(pkg);
    let pos = ctx.panes.package.viewport.pos();
    if matches!(fields.get(pos), Some(DetailField::CratesIo)) {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Git detail pane.
pub(crate) struct GitPane;

impl Pane<App> for GitPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Git;
}

impl Shortcuts<App> for GitPane {
    type Actions = GitAction;

    const SCOPE_NAME: &'static str = "git";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => GitAction::Activate,
            'c' => GitAction::Clean,
        }
    }

    fn state(&self, action: GitAction, ctx: &App) -> ShortcutState {
        match action {
            GitAction::Activate => git_activate_state(ctx),
            GitAction::Clean => ShortcutState::Enabled,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) {
        |_action, _ctx| {
            // No-op through Phase 17. The legacy detail-key path in
            // src/tui/panes/actions.rs remains authoritative.
        }
    }
}

/// `Activate` on the Git pane is enabled only when the cursor is on a
/// row whose dispatch has a real effect. Today the legacy path only
/// opens a URL when the cursor sits on a `GitRow::Remote` whose
/// `full_url` is `Some` (see
/// `src/tui/panes/actions.rs::handle_detail_enter`); every other row
/// — flat fields, worktrees, and remotes without a URL — is a no-op.
fn git_activate_state(ctx: &App) -> ShortcutState {
    let Some(git) = ctx.panes.git.content() else {
        return ShortcutState::Disabled;
    };
    let pos = ctx.panes.git.viewport.pos();
    if let Some(GitRow::Remote(remote)) = git_row_at(git, pos)
        && remote.full_url.is_some()
    {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}

/// Assemble the framework keymap. Called once during App construction.
/// Errors propagate so the caller can surface them through the
/// existing keymap-diagnostics toast plumbing.
pub(crate) fn build_framework_keymap(
    framework: &mut Framework<App>,
) -> Result<Keymap<App>, KeymapError> {
    Keymap::<App>::builder()
        .register_navigation::<AppNavigation>()?
        .register_globals::<AppGlobalAction>()?
        .register::<PackagePane>(PackagePane)
        .register::<GitPane>(GitPane)
        .build_into(framework)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use tui_pane::Action;

    use super::*;

    #[test]
    fn nav_action_count_is_six() {
        assert_eq!(NavigationAction::ALL.len(), 6);
    }

    #[test]
    fn app_pane_id_round_trips_to_legacy() {
        for (app_id, legacy) in [
            (AppPaneId::Package, PaneId::Package),
            (AppPaneId::Git, PaneId::Git),
            (AppPaneId::Output, PaneId::Output),
            (AppPaneId::Finder, PaneId::Finder),
        ] {
            assert_eq!(app_id.to_legacy(), legacy);
        }
    }

    #[test]
    fn package_action_inherent_facade_matches_action_trait() {
        assert_eq!(
            <PackageAction as Action>::ALL.len(),
            PackageAction::ALL.len(),
        );
        assert_eq!(
            <PackageAction as Action>::toml_key(PackageAction::Activate),
            PackageAction::Activate.toml_key(),
        );
    }
}

/// Bridge module: helpers that need crate-internal access on App. Held
/// here so the parent module of [`crate::tui::app`] does not need to
/// know about the framework keymap.
pub(crate) mod bridge {
    use super::App;
    use super::Keymap;
    use super::KeymapError;
    use super::build_framework_keymap;

    /// Build the framework keymap against `app.framework`. Public to
    /// the binary so the construct.rs builder can wire it during
    /// startup without exposing any other internals.
    pub(crate) fn build_for_app(app: &mut App) -> Result<Keymap<App>, KeymapError> {
        build_framework_keymap(&mut app.framework)
    }
}
