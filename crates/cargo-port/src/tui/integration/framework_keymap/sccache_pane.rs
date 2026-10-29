use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::Pane;
use super::SccacheAction;
use super::Shortcuts;
use super::TabStop;
use super::sccache;

/// Registered shortcuts and copy selection for the modal Sccache stats pane.
pub(super) struct SccachePane;

impl Pane<App> for SccachePane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Sccache;

    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for SccachePane {
    type Actions = SccacheAction;

    const SCOPE_NAME: &'static str = "sccache";
    const SECTION_NAME: &'static str = "Sccache";

    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            crossterm::event::KeyCode::Esc => SccacheAction::Cancel,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { sccache::dispatch_sccache_action }
}

impl CopySelection<App> for SccachePane {
    fn copy_selection(ctx: &App) -> CopySelectionResult { sccache::copy_selection(ctx) }
}
