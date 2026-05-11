//! Framework-side keymap scaffolding.
//!
//! Phase 14.2 introduced the `tui_pane`-driven keymap path beside the
//! legacy `src/keymap.rs` path. The two coexist through Phase 18: the
//! framework keymap now owns targeted structural lookups, while broad
//! key dispatch remains on the legacy path until Phase 19.
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
//!   builder typestate requires. Dispatcher fns route through the framework keymap after the Phase
//!   19 cutover.
//! - [`build_framework_keymap`]: assembles a [`tui_pane::Keymap<App>`] using the canonical builder
//!   chain. Called once at startup.

use std::rc::Rc;

use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::BarRegion;
use tui_pane::BarSlot;
use tui_pane::Bindings;
use tui_pane::Configuring;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapBuilder;
use tui_pane::KeymapError;
use tui_pane::Mode;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::VimMode;
use tui_pane::Visibility;

use super::app::App;
use super::finder;
use super::input;
use super::panes;
use super::panes::DetailField;
use super::panes::GitRow;
use super::panes::PaneId;
use crate::config::NavigationKeys;
use crate::keymap::CiRunsAction;
use crate::keymap::FinderAction;
use crate::keymap::GitAction;
use crate::keymap::LintsAction;
use crate::keymap::OutputAction;
use crate::keymap::PackageAction;
use crate::keymap::ProjectListAction;
use crate::keymap::TargetsAction;

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

pub(super) const fn vim_mode_from_config(navigation_keys: NavigationKeys) -> VimMode {
    match navigation_keys {
        NavigationKeys::ArrowsOnly => VimMode::Disabled,
        NavigationKeys::ArrowsAndVim => VimMode::Enabled,
    }
}

const PROJECT_LIST_TAB_ORDER: i16 = 0;
const PACKAGE_TAB_ORDER: i16 = 1;
const GIT_TAB_ORDER: i16 = 2;
const LANG_TAB_ORDER: i16 = 3;
const CPU_TAB_ORDER: i16 = 4;
const TARGETS_TAB_ORDER: i16 = 5;
const LINTS_TAB_ORDER: i16 = 6;
const CI_RUNS_TAB_ORDER: i16 = 7;
const OUTPUT_TAB_ORDER: i16 = 8;

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

    pub(crate) const fn from_legacy(pane: PaneId) -> Option<Self> {
        match pane {
            PaneId::ProjectList => Some(Self::ProjectList),
            PaneId::Package => Some(Self::Package),
            PaneId::Lang => Some(Self::Lang),
            PaneId::Cpu => Some(Self::Cpu),
            PaneId::Git => Some(Self::Git),
            PaneId::Targets => Some(Self::Targets),
            PaneId::Lints => Some(Self::Lints),
            PaneId::CiRuns => Some(Self::CiRuns),
            PaneId::Output => Some(Self::Output),
            PaneId::Finder => Some(Self::Finder),
            PaneId::Settings | PaneId::Keymap | PaneId::Toasts => None,
        }
    }
}

fn project_list_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::ProjectList) }

fn package_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Package) }

fn git_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Git) }

fn lang_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Lang) }

fn cpu_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Cpu) }

fn targets_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Targets) }

fn lints_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Lints) }

fn ci_runs_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::CiRuns) }

fn output_is_tabbable(app: &App) -> bool { app.is_pane_tabbable(PaneId::Output) }

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
        Find         => ("find",          "find",     "Open finder");
        OpenEditor   => ("open_editor",   "editor",   "Open in editor");
        OpenTerminal => ("open_terminal", "terminal", "Open terminal");
        Rescan       => ("rescan",        "rescan",   "Rescan projects");
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }

    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        self.framework.set_focused(focus);
        if let FocusedPane::App(id) = focus {
            self.visited_panes.insert(id);
        }
    }
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
        panes::dispatch_navigation_action
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    fn render_order() -> &'static [Self::Actions] { Self::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            '/'                  => Self::Find,
            'e'                  => Self::OpenEditor,
            't'                  => Self::OpenTerminal,
            KeyBind::ctrl('r')   => Self::Rescan,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch_app_global }
}

fn dispatch_app_global(action: AppGlobalAction, app: &mut App) {
    match action {
        AppGlobalAction::Find => input::open_finder(app),
        AppGlobalAction::OpenEditor => input::open_in_editor(app),
        AppGlobalAction::OpenTerminal => input::open_terminal(app),
        AppGlobalAction::Rescan => app.rescan(),
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Package detail pane.
pub(crate) struct PackagePane;

impl Pane<App> for PackagePane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Package;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(PACKAGE_TAB_ORDER, package_is_tabbable) }
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

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_package_action }
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
    let fields = panes::package_fields_from_data(pkg);
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

    fn tab_stop() -> TabStop<App> { TabStop::ordered(GIT_TAB_ORDER, git_is_tabbable) }
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

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_git_action }
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
    if let Some(GitRow::Remote(remote)) = panes::git_row_at(git, pos)
        && remote.full_url.is_some()
    {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}

// ── Lang / Cpu action enums (framework-only) ─────────────────────────
//
// Lang and Cpu have no row-conditional dispatch in the legacy path
// (Lang fall-throughs to PackageAction; Cpu's `handle_detail_key` arm
// is empty). Each gets its own minimal action enum so the framework
// keymap can register a real scope; the dispatcher fns are no-ops
// through Phase 17. No facade required — these enums are not consumed
// by `src/keymap.rs`'s `ResolvedKeymap` or any legacy call site.

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum LangAction {
        Clean => ("clean", "Clean project");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum CpuAction {
        Clean => ("clean", "Clean project");
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Lang detail pane.
pub(crate) struct LangPane;

impl Pane<App> for LangPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Lang;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(LANG_TAB_ORDER, lang_is_tabbable) }
}

impl Shortcuts<App> for LangPane {
    type Actions = LangAction;

    const SCOPE_NAME: &'static str = "lang";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            'c' => LangAction::Clean,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_lang_action }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Cpu pane.
pub(crate) struct CpuPane;

impl Pane<App> for CpuPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Cpu;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(CPU_TAB_ORDER, cpu_is_tabbable) }
}

impl Shortcuts<App> for CpuPane {
    type Actions = CpuAction;

    const SCOPE_NAME: &'static str = "cpu";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            'c' => CpuAction::Clean,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_cpu_action }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Targets pane.
pub(crate) struct TargetsPane;

impl Pane<App> for TargetsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Targets;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(TARGETS_TAB_ORDER, targets_is_tabbable) }
}

impl Shortcuts<App> for TargetsPane {
    type Actions = TargetsAction;

    const SCOPE_NAME: &'static str = "targets";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => TargetsAction::Activate,
            'r' => TargetsAction::ReleaseBuild,
            'c' => TargetsAction::Clean,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_targets_action }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Lints pane.
pub(crate) struct LintsPane;

impl Pane<App> for LintsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Lints;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(LINTS_TAB_ORDER, lints_is_tabbable) }
}

impl Shortcuts<App> for LintsPane {
    type Actions = LintsAction;

    const SCOPE_NAME: &'static str = "lints";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => LintsAction::Activate,
            'd' => LintsAction::ClearHistory,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_lints_action }
}

/// `Pane<App>` + `Shortcuts<App>` host for the `CiRuns` pane.
pub(crate) struct CiRunsPane;

impl Pane<App> for CiRunsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::CiRuns;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(CI_RUNS_TAB_ORDER, ci_runs_is_tabbable) }
}

impl Shortcuts<App> for CiRunsPane {
    type Actions = CiRunsAction;

    const SCOPE_NAME: &'static str = "ci_runs";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => CiRunsAction::Activate,
            'f' => CiRunsAction::FetchMore,
            'v' => CiRunsAction::ToggleView,
            'd' => CiRunsAction::ClearCache,
        }
    }

    fn visibility(&self, action: CiRunsAction, ctx: &App) -> Visibility {
        match action {
            CiRunsAction::Activate => ci_runs_activate_visibility(ctx),
            _ => Visibility::Visible,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_ci_runs_action }
}

/// `Activate` on the `CiRuns` pane is hidden when the cursor sits at
/// or beyond the end of the visible runs list. The legacy
/// `handle_ci_enter` path indexes `ci.content().runs.get(pos)`; an
/// out-of-range cursor is a no-op. Hiding the slot (rather than
/// disabling it) matches the Phase 14 plan's distinction:
/// `Visibility::Hidden` drops the slot from the bar entirely.
fn ci_runs_activate_visibility(ctx: &App) -> Visibility {
    let run_count = ctx.ci.content().map_or(0, |data| data.runs.len());
    if ctx.ci.viewport.pos() >= run_count {
        Visibility::Hidden
    } else {
        Visibility::Visible
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the `ProjectList` pane.
pub(crate) struct ProjectListPane;

impl Pane<App> for ProjectListPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::ProjectList;

    fn tab_stop() -> TabStop<App> {
        TabStop::ordered(PROJECT_LIST_TAB_ORDER, project_list_is_tabbable)
    }
}

impl Shortcuts<App> for ProjectListPane {
    type Actions = ProjectListAction;

    const SCOPE_NAME: &'static str = "project_list";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Right => ProjectListAction::ExpandRow,
            crossterm::event::KeyCode::Left => ProjectListAction::CollapseRow,
            '=' => ProjectListAction::ExpandAll,
            '-' => ProjectListAction::CollapseAll,
            'c' => ProjectListAction::Clean,
        }
    }

    fn bar_slots(&self, _ctx: &App) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        vec![
            (
                BarRegion::Nav,
                BarSlot::Paired(
                    ProjectListAction::CollapseRow,
                    ProjectListAction::ExpandRow,
                    "expand",
                ),
            ),
            (
                BarRegion::Nav,
                BarSlot::Paired(
                    ProjectListAction::ExpandAll,
                    ProjectListAction::CollapseAll,
                    "all",
                ),
            ),
            (
                BarRegion::PaneAction,
                BarSlot::Single(ProjectListAction::Clean),
            ),
        ]
    }

    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &PROJECT_LIST_VIM_EXTRAS }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_project_list_action }
}

/// `'l'` / `'h'` extend the `ProjectList` scope with vim-style row
/// expand/collapse when `VimMode::Enabled`. Append-only — the keymap
/// builder's vim overlay skips letters already bound on the full
/// `KeyBind` (code + mods), so a TOML rebind to `'l'` for a different
/// action wins over this default.
static PROJECT_LIST_VIM_EXTRAS: [(ProjectListAction, KeyBind); 2] = [
    (ProjectListAction::ExpandRow, KeyBind::from_char('l')),
    (ProjectListAction::CollapseRow, KeyBind::from_char('h')),
];

/// `Pane<App>` + `Shortcuts<App>` host for the Output pane.
///
/// First non-`Navigable` pane: `Mode::Static` suppresses the `Nav`
/// region so the bar shows only `PaneAction` (the `Esc close` slot)
/// plus the global strip.
pub(crate) struct OutputPane;

impl Pane<App> for OutputPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Output;

    fn mode() -> fn(&App) -> Mode<App> { |_ctx| Mode::Static }

    fn tab_stop() -> TabStop<App> { TabStop::ordered(OUTPUT_TAB_ORDER, output_is_tabbable) }
}

impl Shortcuts<App> for OutputPane {
    type Actions = OutputAction;

    const SCOPE_NAME: &'static str = "output";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Esc => OutputAction::Cancel,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_output_action }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Finder overlay.
///
/// Mode flips with `app.overlays.is_finder_open()`:
/// - open  → `Mode::TextInput(finder_keys)` — character keys go to the embedded handler, which
///   dispatches Finder actions through the framework keymap before falling back to text entry.
/// - closed → `Mode::Navigable` (default Browse-style behaviour, though while closed the Finder
///   pane never actually has focus).
pub(crate) struct FinderPane;

impl Pane<App> for FinderPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Finder;

    fn mode() -> fn(&App) -> Mode<App> {
        |ctx| {
            if ctx.overlays.is_finder_open() {
                Mode::TextInput(finder_keys)
            } else {
                Mode::Navigable
            }
        }
    }

    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for FinderPane {
    type Actions = FinderAction;

    const SCOPE_NAME: &'static str = "finder";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => FinderAction::Activate,
            crossterm::event::KeyCode::Esc   => FinderAction::Cancel,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { finder::dispatch_finder_action }
}

/// `Mode::TextInput` handler for the Finder. Routes a single keypress
/// through the Finder pane scope first, then falls back to text entry
/// and result-list navigation for keys that are not Finder actions.
fn finder_keys(bind: KeyBind, app: &mut App) {
    let keymap = Rc::clone(&app.framework_keymap);
    match keymap.dispatch_app_pane(FinderPane::APP_PANE_ID, &bind, app) {
        KeyOutcome::Consumed => {},
        KeyOutcome::Unhandled => finder::handle_finder_text_key(app, bind.code),
    }
}

/// Assemble the framework keymap from a configured builder. Called
/// once during App construction after the builder has loaded the
/// production keymap TOML, if any. Errors propagate so the caller can
/// surface them through the existing keymap-diagnostics toast
/// plumbing.
pub(super) fn build_framework_keymap(
    builder: KeymapBuilder<App, Configuring>,
    framework: &mut Framework<App>,
) -> Result<Keymap<App>, KeymapError> {
    builder
        .dismiss_fallback(dismiss_fallback)
        .register_navigation::<AppNavigation>()?
        .register_globals::<AppGlobalAction>()?
        .register_settings_overlay()?
        .register_keymap_overlay()?
        .register::<ProjectListPane>(ProjectListPane)
        .register::<PackagePane>(PackagePane)
        .register::<LangPane>(LangPane)
        .register::<CpuPane>(CpuPane)
        .register::<GitPane>(GitPane)
        .register::<TargetsPane>(TargetsPane)
        .register::<LintsPane>(LintsPane)
        .register::<CiRunsPane>(CiRunsPane)
        .register::<OutputPane>(OutputPane)
        .register::<FinderPane>(FinderPane)
        .build_into(framework)
}

fn dismiss_fallback(app: &mut App) -> bool {
    let Some(target) = app.focused_dismiss_target() else {
        return false;
    };
    app.dismiss(target);
    true
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
            <PackageAction as Action>::ALL.len(),
        );
        assert_eq!(
            <PackageAction as Action>::toml_key(PackageAction::Activate),
            PackageAction::Activate.toml_key(),
        );
    }
}
