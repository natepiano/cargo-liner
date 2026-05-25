//! Framework-side keymap scaffolding.
//!
//! The `tui_pane`-driven keymap path coexists with the legacy
//! `src/keymap.rs` path: the framework keymap owns targeted structural
//! lookups while broad key dispatch remains on the legacy path.
//!
//! Surface:
//!
//! - [`AppPaneId`]: every app-side pane id the framework keys on.
//! - [`NavigationAction`]: directional nav enum the [`Navigation`] singleton routes through.
//! - [`AppGlobalAction`]: app-extension globals scope. Currently ships a single placeholder variant
//!   ([`AppGlobalAction::Find`]); grows to cover the rest of the binary's non-framework globals.
//! - [`AppNavigation`] / [`PackagePane`]: the `Navigation` and `Pane` + `Shortcuts` impls the
//!   builder typestate requires.
//! - [`build_framework_keymap`]: assembles a [`tui_pane::Keymap<App>`] using the canonical builder
//!   chain. Called once at startup.

use std::rc::Rc;

use tui_pane::Action;
use tui_pane::AppContext;
use tui_pane::BarRegion;
use tui_pane::BarSlot;
use tui_pane::Bindings;
use tui_pane::Configuring;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelection;
use tui_pane::CopySelectionResult;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Globals;
use tui_pane::KeyBind;
use tui_pane::KeyOutcome;
use tui_pane::Keymap;
use tui_pane::KeymapBuilder;
use tui_pane::KeymapError;
use tui_pane::KeymapUiContext;
use tui_pane::Mode;
use tui_pane::Navigation;
use tui_pane::Pane;
use tui_pane::PaneFocusState;
use tui_pane::ShortcutState;
use tui_pane::Shortcuts;
use tui_pane::TabStop;
use tui_pane::TrackedItemKey;
use tui_pane::VimMode;
use tui_pane::Visibility;

use crate::ci::OwnerRepo;
use crate::config::NavigationKeys;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::CargoPortToastAction;
use crate::tui::app::CiRunDisplayMode;
use crate::tui::finder;
use crate::tui::input;
use crate::tui::keymap::CiRunsAction;
use crate::tui::keymap::FinderAction;
use crate::tui::keymap::GitAction;
use crate::tui::keymap::LintsAction;
use crate::tui::keymap::OutputAction;
use crate::tui::keymap::PackageAction;
use crate::tui::keymap::ProjectListAction;
use crate::tui::keymap::TargetsAction;
use crate::tui::panes;
use crate::tui::panes::DetailField;
use crate::tui::panes::GitRow;
use crate::tui::panes::PaneId;

/// Stable identifier for every app-side pane the framework keys its
/// per-pane registries on.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum AppPaneId {
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

pub const fn vim_mode_from_config(navigation_keys: NavigationKeys) -> VimMode {
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
    /// Translation to the legacy [`PaneId`] enum so the framework's
    /// `AppPaneId` bridges back to the legacy id. App-only variants
    /// only — framework panes (Toasts, Settings, Keymap) are not part
    /// of [`AppPaneId`].
    pub const fn to_legacy(self) -> PaneId {
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

    pub const fn from_legacy(pane: PaneId) -> Option<Self> {
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
    pub enum NavigationAction {
        Up    => ("up",    "up",    "Move up");
        Down  => ("down",  "down",  "Move down");
        Left  => ("left",  "left",  "Move left");
        Right => ("right", "right", "Move right");
        Home  => ("home",  "home",  "Jump to start");
        End   => ("end",   "end",   "Jump to end");
        PageUp       => ("page_up",       "page up",       "Page up");
        PageDown     => ("page_down",     "page down",     "Page down");
        HalfPageUp   => ("half_page_up",   "half up",       "Half-page up");
        HalfPageDown => ("half_page_down", "half down",     "Half-page down");
    }
}

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum AppGlobalAction {
        Copy         => ("copy",          "copy",     "Copy selection");
        Find         => ("find",          "find",     "Open finder");
        OpenEditor   => ("open_editor",   "editor",   "Open in editor");
        OpenTerminal => ("open_terminal", "terminal", "Open terminal");
        Rescan       => ("rescan",        "rescan",   "Rescan projects");
        Clean        => ("clean",         "clean",    "Clean project");
    }
}

impl AppContext for App {
    type AppPaneId = AppPaneId;
    type ToastAction = CargoPortToastAction;

    fn framework(&self) -> &Framework<Self> { &self.framework }

    fn framework_mut(&mut self) -> &mut Framework<Self> { &mut self.framework }

    fn handle_toast_action(&mut self, action: Self::ToastAction) {
        match action {
            CargoPortToastAction::OpenPath(path) => {
                if let Err(err) =
                    input::open_paths_in_editor(self.config.editor(), [path.as_path()])
                {
                    self.show_timed_toast("Toast action failed", err.to_string());
                }
            },
        }
    }

    fn set_focus(&mut self, focus: FocusedPane<Self::AppPaneId>) {
        self.framework.set_focused(focus);
        if let FocusedPane::App(id) = focus {
            self.visited_panes.insert(id);
        }
    }
}

/// Display ordering for the keymap-help overlay's per-pane sections.
/// Mirrors the prior `push_app_pane_rows` hardcoded order so the
/// overlay still surfaces sections in the cargo-port-preferred
/// sequence.
const KEYMAP_OVERLAY_PANE_ORDER: &[AppPaneId] = &[
    AppPaneId::ProjectList,
    AppPaneId::Package,
    AppPaneId::Git,
    AppPaneId::Targets,
    AppPaneId::Lints,
    AppPaneId::CiRuns,
    AppPaneId::Output,
    AppPaneId::Finder,
];

impl KeymapUiContext for App {
    fn keymap_inline_error(&self) -> Option<&str> {
        self.overlays.inline_error().map(String::as_str)
    }

    fn keymap_pane_focus_state(&self) -> PaneFocusState { self.pane_focus_state(PaneId::Keymap) }

    fn keymap_pane_sort_priority(&self, scope: &str, toml_key: &str) -> u8 {
        if scope == "project_list" {
            match toml_key {
                "clean" => 0,
                "collapse_all" => 1,
                "expand_all" => 2,
                "collapse_row" => 3,
                "expand_row" => 4,
                _ => u8::MAX,
            }
        } else {
            u8::MAX
        }
    }

    fn keymap_pane_display_order(&self) -> &[AppPaneId] { KEYMAP_OVERLAY_PANE_ORDER }
}

/// `Navigation<App>` host. Zero-sized because the framework only needs
/// the type; navigation defaults / dispatch are static methods.
pub struct AppNavigation;

impl Navigation<App> for AppNavigation {
    type Actions = NavigationAction;

    const SECTION_NAME: &'static str = "List Navigation";

    const DOWN: Self::Actions = NavigationAction::Down;
    const END: Self::Actions = NavigationAction::End;
    const HOME: Self::Actions = NavigationAction::Home;
    const HALF_PAGE_DOWN: Self::Actions = NavigationAction::HalfPageDown;
    const HALF_PAGE_UP: Self::Actions = NavigationAction::HalfPageUp;
    const LEFT: Self::Actions = NavigationAction::Left;
    const PAGE_DOWN: Self::Actions = NavigationAction::PageDown;
    const PAGE_UP: Self::Actions = NavigationAction::PageUp;
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
            crossterm::event::KeyCode::PageUp   => NavigationAction::PageUp,
            crossterm::event::KeyCode::PageDown => NavigationAction::PageDown,
        }
    }

    fn dispatcher() -> fn(Self::Actions, FocusedPane<AppPaneId>, &mut App) {
        panes::dispatch_navigation_action
    }
}

impl Globals<App> for AppGlobalAction {
    type Actions = Self;

    const SECTION_NAME: &'static str = "Global Shortcuts";

    fn render_order() -> &'static [Self::Actions] { Self::ALL }

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            'y'                  => Self::Copy,
            '/'                  => Self::Find,
            'e'                  => Self::OpenEditor,
            't'                  => Self::OpenTerminal,
            KeyBind::ctrl('r')   => Self::Rescan,
            'c'                  => Self::Clean,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch_app_global }
}

fn dispatch_app_global(action: AppGlobalAction, app: &mut App) {
    match action {
        AppGlobalAction::Copy => app.copy_focused_selection(),
        AppGlobalAction::Find => input::open_finder(app),
        AppGlobalAction::OpenEditor => input::open_in_editor(app),
        AppGlobalAction::OpenTerminal => input::open_terminal(app),
        AppGlobalAction::Rescan => app.rescan(),
        AppGlobalAction::Clean => panes::request_clean(app),
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Package detail pane.
pub struct PackagePane;

impl Pane<App> for PackagePane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Package;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(PACKAGE_TAB_ORDER, package_is_tabbable) }
}

impl Shortcuts<App> for PackagePane {
    type Actions = PackageAction;

    const SCOPE_NAME: &'static str = "package";
    const SECTION_NAME: &'static str = "Package";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => PackageAction::Activate,
        }
    }

    fn state(&self, action: PackageAction, ctx: &App) -> ShortcutState {
        match action {
            PackageAction::Activate => activate_state(ctx),
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_package_action }
}

impl CopySelection<App> for PackagePane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        let Some(pkg) = ctx.panes.package.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_package(pkg, ctx.panes.package.viewport.pos())
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
    let pos = ctx.panes.package.viewport.pos();
    if matches!(
        panes::package_field_at(pkg, pos),
        Some(DetailField::CratesIo)
    ) {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Git detail pane.
pub struct GitPane;

impl Pane<App> for GitPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Git;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(GIT_TAB_ORDER, git_is_tabbable) }
}

impl Shortcuts<App> for GitPane {
    type Actions = GitAction;

    const SCOPE_NAME: &'static str = "git";
    const SECTION_NAME: &'static str = "Git";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => GitAction::Activate,
        }
    }

    fn state(&self, action: GitAction, ctx: &App) -> ShortcutState {
        match action {
            GitAction::Activate => git_activate_state(ctx),
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_git_action }
}

impl CopySelection<App> for GitPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        let Some(git) = ctx.panes.git.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_git(git, ctx.panes.git.viewport.pos())
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
    if let Some(GitRow::Remote(remote)) = panes::git_row_at(git, pos)
        && remote.full_url.is_some()
    {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}

/// `Pane<App>` host for the Lang detail pane. No pane-local actions —
/// `Clean` lives on [`AppGlobalAction`], and the pane has no
/// row-conditional dispatch.
pub struct LangPane;

impl Pane<App> for LangPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Lang;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(LANG_TAB_ORDER, lang_is_tabbable) }
}

/// `Pane<App>` host for the Cpu pane. No pane-local actions — see
/// [`LangPane`].
pub struct CpuPane;

impl Pane<App> for CpuPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Cpu;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(CPU_TAB_ORDER, cpu_is_tabbable) }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Targets pane.
pub struct TargetsPane;

impl Pane<App> for TargetsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Targets;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(TARGETS_TAB_ORDER, targets_is_tabbable) }
}

impl Shortcuts<App> for TargetsPane {
    type Actions = TargetsAction;

    const SCOPE_NAME: &'static str = "targets";
    const SECTION_NAME: &'static str = "Targets";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => TargetsAction::Activate,
            'r' => TargetsAction::ReleaseBuild,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_targets_action }
}

impl CopySelection<App> for TargetsPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        let Some(targets) = ctx.panes.targets.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_targets(targets, ctx.panes.targets.viewport.pos(), &|entry| {
            ctx.target_is_running(entry)
        })
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the Lints pane.
pub struct LintsPane;

impl Pane<App> for LintsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Lints;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(LINTS_TAB_ORDER, lints_is_tabbable) }
}

impl Shortcuts<App> for LintsPane {
    type Actions = LintsAction;

    const SCOPE_NAME: &'static str = "lints";
    const SECTION_NAME: &'static str = "Lints";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => LintsAction::Activate,
            'd' => LintsAction::ClearHistory,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_lints_action }
}

impl CopySelection<App> for LintsPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        if !ctx.selected_row_owns_lint() {
            return CopySelectionResult::Nothing;
        }
        let Some(project_root) = ctx.project_list.selected_project_path() else {
            return CopySelectionResult::Nothing;
        };
        let Some(lints) = ctx.lint.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_lints(lints, ctx.lint.viewport.pos(), project_root)
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the `CiRuns` pane.
pub struct CiRunsPane;

impl Pane<App> for CiRunsPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::CiRuns;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(CI_RUNS_TAB_ORDER, ci_runs_is_tabbable) }
}

impl Shortcuts<App> for CiRunsPane {
    type Actions = CiRunsAction;

    const SCOPE_NAME: &'static str = "ci_runs";
    const SECTION_NAME: &'static str = "CI Runs";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Enter => CiRunsAction::Activate,
            'f' => CiRunsAction::FetchMore,
            'b' => CiRunsAction::ShowBranch,
            'a' => CiRunsAction::ShowAll,
            'd' => CiRunsAction::ClearCache,
        }
    }

    fn visibility(&self, action: CiRunsAction, ctx: &App) -> Visibility {
        match action {
            CiRunsAction::Activate => ci_runs_activate_visibility(ctx),
            CiRunsAction::ShowBranch => ci_runs_show_branch_visibility(ctx),
            CiRunsAction::ShowAll => ci_runs_show_all_visibility(ctx),
            _ => Visibility::Visible,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_ci_runs_action }
}

impl CopySelection<App> for CiRunsPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        let Some(ci) = ctx.ci.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_ci(ci, ctx.ci.viewport.pos())
    }
}

/// `Activate` on the `CiRuns` pane is hidden when the cursor sits at
/// or beyond the end of the visible runs list. The legacy
/// `handle_ci_enter` path indexes `ci.content().runs.get(pos)`; an
/// out-of-range cursor is a no-op. Hiding the slot (rather than
/// disabling it) drops it from the bar entirely.
fn ci_runs_activate_visibility(ctx: &App) -> Visibility {
    let run_count = ctx.ci.content().map_or(0, |data| data.runs.len());
    if ctx.ci.viewport.pos() >= run_count {
        Visibility::Hidden
    } else {
        Visibility::Visible
    }
}

/// `ShowBranch` is visible only when the current project is in `All`
/// mode (i.e., pressing it switches to the destination, `BranchOnly`).
fn ci_runs_show_branch_visibility(ctx: &App) -> Visibility {
    ci_runs_destination_visibility(ctx, CiRunDisplayMode::All)
}

/// `ShowAll` is visible only when the current project is in
/// `BranchOnly` mode (i.e., pressing it switches to `All`).
fn ci_runs_show_all_visibility(ctx: &App) -> Visibility {
    ci_runs_destination_visibility(ctx, CiRunDisplayMode::BranchOnly)
}

/// Visible when the selected project's current mode is `current_mode`
/// — the slot points at the destination state, so it shows only when
/// the user is on the opposite side of the toggle. Hidden when no
/// project is selected. The dispatcher itself is a no-op when the
/// project has no current branch, so we don't gate on
/// `ci_toggle_available_for` here.
fn ci_runs_destination_visibility(ctx: &App, current_mode: CiRunDisplayMode) -> Visibility {
    let Some(path) = ctx.project_list.selected_project_path() else {
        return Visibility::Hidden;
    };
    if ctx.ci.display_mode_for(path) == current_mode {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}

/// `Pane<App>` + `Shortcuts<App>` host for the `ProjectList` pane.
pub struct ProjectListPane;

impl Pane<App> for ProjectListPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::ProjectList;

    fn tab_stop() -> TabStop<App> {
        TabStop::ordered(PROJECT_LIST_TAB_ORDER, project_list_is_tabbable)
    }
}

impl Shortcuts<App> for ProjectListPane {
    type Actions = ProjectListAction;

    const SCOPE_NAME: &'static str = "project_list";
    const SECTION_NAME: &'static str = "Project List";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Right => ProjectListAction::ExpandRow,
            crossterm::event::KeyCode::Left => ProjectListAction::CollapseRow,
            '=' => ProjectListAction::ExpandAll,
            '-' => ProjectListAction::CollapseAll,
        }
    }

    fn bar_slots(&self, _ctx: &App) -> Vec<(BarRegion, BarSlot<Self::Actions>)> {
        vec![(
            BarRegion::Nav,
            BarSlot::Paired(
                ProjectListAction::ExpandAll,
                ProjectListAction::CollapseAll,
                "all",
            ),
        )]
    }

    fn vim_extras() -> &'static [(Self::Actions, KeyBind)] { &PROJECT_LIST_VIM_EXTRAS }

    fn dispatcher() -> fn(Self::Actions, &mut App) { input::dispatch_project_list_action }
}

impl CopySelection<App> for ProjectListPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        ctx.project_list
            .selected_project_path()
            .map_or(CopySelectionResult::Nothing, |path| {
                CopySelectionResult::Payload(CopyPayload::new(
                    path.display().to_string(),
                    CopyLabel::Path,
                ))
            })
    }
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
pub struct OutputPane;

impl Pane<App> for OutputPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Output;

    fn mode() -> fn(&App) -> Mode<App> { |_ctx| Mode::Static }

    fn tab_stop() -> TabStop<App> { TabStop::ordered(OUTPUT_TAB_ORDER, output_is_tabbable) }
}

impl Shortcuts<App> for OutputPane {
    type Actions = OutputAction;

    const SCOPE_NAME: &'static str = "output";
    const SECTION_NAME: &'static str = "Output";

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
pub struct FinderPane;

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
    const SECTION_NAME: &'static str = "Finder";

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
pub fn build_framework_keymap(
    builder: KeymapBuilder<App, Configuring>,
    framework: &mut Framework<App>,
) -> Result<Keymap<App>, KeymapError> {
    builder
        .dismiss_fallback(dismiss_fallback)
        .register_navigation::<AppNavigation>()?
        .register_globals::<AppGlobalAction>()?
        .register_overlay()?
        .register::<ProjectListPane>(ProjectListPane)
        .register_copy_selection::<ProjectListPane>()
        .register::<PackagePane>(PackagePane)
        .register_copy_selection::<PackagePane>()
        .register_pane::<LangPane>()
        .register_pane::<CpuPane>()
        .register::<GitPane>(GitPane)
        .register_copy_selection::<GitPane>()
        .register::<TargetsPane>(TargetsPane)
        .register_copy_selection::<TargetsPane>()
        .register::<LintsPane>(LintsPane)
        .register_copy_selection::<LintsPane>()
        .register::<CiRunsPane>(CiRunsPane)
        .register_copy_selection::<CiRunsPane>()
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

pub fn path_key(path: &AbsolutePath) -> TrackedItemKey { TrackedItemKey::new(path.to_string()) }

pub fn owner_repo_key(repo: &OwnerRepo) -> TrackedItemKey { TrackedItemKey::new(repo.to_string()) }

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
    fn path_key_uses_cargo_port_absolute_path_string() {
        let path = AbsolutePath::from("/tmp/cargo-port");

        assert_eq!(path_key(&path).as_str(), "/tmp/cargo-port");
    }

    #[test]
    fn owner_repo_key_uses_cargo_port_owner_repo_string() {
        let repo = OwnerRepo::new("natepiano", "cargo-port");

        assert_eq!(owner_repo_key(&repo).as_str(), "natepiano/cargo-port");
    }

    #[test]
    fn nav_action_count_includes_paging_actions() {
        assert_eq!(NavigationAction::ALL.len(), 10);
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

    #[test]
    fn ci_runs_branch_and_all_defaults() {
        let defaults = CiRunsPane::defaults().into_scope_map();

        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('b')),
            Some(CiRunsAction::ShowBranch),
        );
        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('a')),
            Some(CiRunsAction::ShowAll),
        );
        assert_eq!(defaults.action_for(&tui_pane::KeyBind::from('v')), None);
    }

    #[test]
    fn app_global_copy_defaults_to_y_without_terminal_copy_keys() {
        let defaults = AppGlobalAction::defaults().into_scope_map();

        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::from('y')),
            Some(AppGlobalAction::Copy),
        );
        assert_eq!(defaults.action_for(&tui_pane::KeyBind::ctrl('c')), None,);
        assert_eq!(
            defaults.action_for(&tui_pane::KeyBind::ctrl(tui_pane::KeyBind::shift('c'))),
            None,
        );
    }
}
