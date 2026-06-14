use crate::tui::app::App;

impl App {
    pub(super) fn begin_startup_phase(&mut self, lint_registered: usize) {
        self.begin_startup_phase_from_scan(lint_registered);
    }
}
