mod orchestrator;
mod start;
mod toast_bodies;
mod tracker;

pub use orchestrator::Startup;

use crate::tui::app::App;

impl App {
    pub(super) fn begin_startup_phase_from_scan(&mut self, lint_registered: usize) {
        start::begin_startup_phase_from_scan(self, lint_registered);
    }
}
