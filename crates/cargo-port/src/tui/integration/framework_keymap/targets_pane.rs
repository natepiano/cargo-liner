use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::Pane;
use super::Shortcuts;
use super::TARGETS_TAB_ORDER;
use super::TabStop;
use super::TargetsAction;
use super::Visibility;
use super::panes;
use super::targets_is_tabbable;

/// `Pane<App>` + `Shortcuts<App>` host for the Targets pane.
pub(super) struct TargetsPane;

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

    fn visibility(&self, action: TargetsAction, ctx: &App) -> Visibility {
        match action {
            TargetsAction::Activate | TargetsAction::ReleaseBuild => targets_run_visibility(ctx),
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { panes::dispatch_targets_action }
}

impl CopySelection<App> for TargetsPane {
    fn copy_selection(ctx: &App) -> CopySelectionResult {
        let Some(targets) = ctx.panes.targets.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_targets(targets, ctx.panes.targets.viewport.pos())
    }
}

/// Run actions are visible when the table has a selected target.
pub(super) fn targets_run_visibility(ctx: &App) -> Visibility {
    if ctx
        .panes
        .targets
        .content()
        .is_some_and(panes::TargetsData::has_targets)
    {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}
