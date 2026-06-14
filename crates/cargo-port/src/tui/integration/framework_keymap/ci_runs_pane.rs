use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CI_RUNS_TAB_ORDER;
use super::CiRunDisplayMode;
use super::CiRunsAction;
use super::CopySelection;
use super::CopySelectionResult;
use super::Pane;
use super::Shortcuts;
use super::TabStop;
use super::Visibility;
use super::ci_runs_is_tabbable;
use super::panes;

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
pub(super) fn ci_runs_activate_visibility(ctx: &App) -> Visibility {
    let run_count = ctx.ci.content().map_or(0, |data| data.runs.len());
    if ctx.ci.viewport.pos() >= run_count {
        Visibility::Hidden
    } else {
        Visibility::Visible
    }
}

/// `ShowBranch` is visible only when the current project is in `All`
/// mode (i.e., pressing it switches to the destination, `BranchOnly`).
pub(super) fn ci_runs_show_branch_visibility(ctx: &App) -> Visibility {
    ci_runs_destination_visibility(ctx, CiRunDisplayMode::All)
}

/// `ShowAll` is visible only when the current project is in
/// `BranchOnly` mode (i.e., pressing it switches to `All`).
pub(super) fn ci_runs_show_all_visibility(ctx: &App) -> Visibility {
    ci_runs_destination_visibility(ctx, CiRunDisplayMode::BranchOnly)
}

/// Visible when the selected project's current mode is `current_mode`
/// — the slot points at the destination state, so it shows only when
/// the user is on the opposite side of the toggle. Hidden when no
/// project is selected, or when the branch is unpublished (no
/// upstream, not the default): there's no branch-scoped run set to
/// filter to, so the all/branch toggle doesn't apply.
pub(super) fn ci_runs_destination_visibility(
    ctx: &App,
    current_mode: CiRunDisplayMode,
) -> Visibility {
    let Some(path) = ctx.project_list.selected_project_path() else {
        return Visibility::Hidden;
    };
    if !ctx.ci_toggle_available_for(path) {
        return Visibility::Hidden;
    }
    let owner = ctx.project_list.ci_branch_owner_path(path);
    if ctx.ci.display_mode_for(owner.as_path()) == current_mode {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}
