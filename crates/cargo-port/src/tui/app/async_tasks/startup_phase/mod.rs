mod orchestrator;
mod start;
mod toast_bodies;

pub use orchestrator::Startup;
pub(in crate::tui::app) use orchestrator::StartupFetchDetail;
pub(in crate::tui::app) use orchestrator::StartupPlan;
pub(in crate::tui::app) use orchestrator::StartupReadiness;
pub(in crate::tui::app) use orchestrator::StartupToast;
pub(in crate::tui::app) use orchestrator::StartupToastLayout;
pub(in crate::tui::app) use toast_bodies::startup_panel_body;

use crate::tui::app::App;

impl App {
    pub(super) fn begin_startup_phase_from_scan(&mut self, lint_registered: usize) {
        start::begin_startup_phase_from_scan(self, lint_registered);
    }
}
