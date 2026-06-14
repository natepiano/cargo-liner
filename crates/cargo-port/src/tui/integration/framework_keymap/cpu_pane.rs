use super::App;
use super::AppPaneId;
use super::CPU_TAB_ORDER;
use super::Pane;
use super::TabStop;
use super::cpu_is_tabbable;

/// `Pane<App>` host for the Cpu pane. No pane-local actions — see
/// `LangPane`.
pub(super) struct CpuPane;

impl Pane<App> for CpuPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Cpu;

    fn tab_stop() -> TabStop<App> { TabStop::ordered(CPU_TAB_ORDER, cpu_is_tabbable) }
}
