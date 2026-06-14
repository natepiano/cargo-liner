use super::App;
use super::AppPaneId;
use super::LANG_TAB_ORDER;
use super::Pane;
use super::TabStop;
use super::lang_is_tabbable;

/// `Pane<App>` host for the Lang detail pane. No pane-local actions —
/// `Clean` lives on `AppGlobalAction`, and the pane has no
/// row-conditional dispatch.
pub(super) struct LangPane;

impl Pane<App> for LangPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Lang;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(LANG_TAB_ORDER, lang_is_tabbable) }
}
