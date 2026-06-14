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
            'K' => TargetsAction::Kill,
        }
    }

    fn visibility(&self, action: TargetsAction, ctx: &App) -> Visibility {
        match action {
            TargetsAction::Kill => targets_kill_visibility(ctx),
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

/// `Kill` on the Targets pane is hidden unless the highlight sits on a
/// Running row: `running_cursor_pid` is `Some` exactly then. Hiding is
/// presentational — dispatch never consults `visibility()`, and
/// `handle_target_kill` already no-ops on table rows
/// (`resolve_kill_request` returns `None`).
pub(super) const fn targets_kill_visibility(ctx: &App) -> Visibility {
    if ctx.panes.targets.running_cursor_pid().is_some() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}

/// `Activate`/`ReleaseBuild` belong to the targets table: hidden while
/// the highlight sits in the Running list (any row at or past the
/// table's length), where only `Kill` applies. Hiding is presentational
/// — `handle_target_action` already runs nothing on Running rows, and
/// Enter on the `cargo` header still toggles the group.
pub(super) fn targets_run_visibility(ctx: &App) -> Visibility {
    let table_len = ctx
        .panes
        .targets
        .content()
        .map_or(0, panes::TargetsData::target_count);
    if ctx.panes.targets.viewport.pos() < table_len {
        Visibility::Visible
    } else {
        Visibility::Hidden
    }
}
