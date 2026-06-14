use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::GIT_TAB_ORDER;
use super::GitAction;
use super::GitRow;
use super::Pane;
use super::ShortcutState;
use super::Shortcuts;
use super::TabStop;
use super::git_is_tabbable;
use super::panes;

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
/// row whose dispatch opens a URL: pull requests or remotes with a
/// known full URL.
pub(super) fn git_activate_state(ctx: &App) -> ShortcutState {
    let Some(git) = ctx.panes.git.content() else {
        return ShortcutState::Disabled;
    };
    let pos = ctx.panes.git.viewport.pos();
    match panes::git_row_at(git, pos) {
        Some(GitRow::PullRequest(_)) => ShortcutState::Enabled,
        Some(GitRow::Remote(remote)) if remote.full_url.is_some() => ShortcutState::Enabled,
        _ => ShortcutState::Disabled,
    }
}
