mod orchestrator;
mod toast_bodies;
mod tracker;

pub use orchestrator::Startup;

use crate::tui::app::App;

impl App {
    pub(super) fn begin_startup_phase_from_scan(&mut self, lint_registered: usize) {
        self.begin_startup_phase_tracker(lint_registered);
    }
}
