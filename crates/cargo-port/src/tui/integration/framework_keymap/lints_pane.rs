use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::LINTS_TAB_ORDER;
use super::LintsAction;
use super::Pane;
use super::Shortcuts;
use super::TabStop;
use super::lints_is_tabbable;
use super::panes;

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
        let Some(lints) = ctx.lint.content() else {
            return CopySelectionResult::Nothing;
        };
        panes::copy_payload_for_lints(lints, ctx.lint.viewport.pos())
    }
}
