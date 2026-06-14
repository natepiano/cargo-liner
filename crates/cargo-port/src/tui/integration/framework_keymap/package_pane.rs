use super::App;
use super::AppPaneId;
use super::Bindings;
use super::CopySelection;
use super::CopySelectionResult;
use super::PACKAGE_TAB_ORDER;
use super::PackageAction;
use super::PackageRow;
use super::Pane;
use super::ShortcutState;
use super::Shortcuts;
use super::TabStop;
use super::package_is_tabbable;
use super::panes;

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
/// only opens a URL on a crates.io row (see
/// `src/tui/panes/actions.rs::handle_detail_enter`); every other row
/// is a no-op. The crates.io rows are rendered only for publishable
/// projects with data (see `build_crates_io_rows`), so `Activate` is
/// implicitly disabled on packages without crates.io data too.
pub(super) fn activate_state(ctx: &App) -> ShortcutState {
    let Some(pkg) = ctx.panes.package.content() else {
        return ShortcutState::Disabled;
    };
    let pos = ctx.panes.package.viewport.pos();
    if matches!(
        panes::package_rows_from_data(pkg).get(pos),
        Some(PackageRow::CratesIo(_))
    ) {
        ShortcutState::Enabled
    } else {
        ShortcutState::Disabled
    }
}
