use crate::tui::app::App;

pub(super) fn begin_startup_phase_from_scan(app: &mut App, lint_registered: usize) {
    app.begin_startup_phase_tracker(lint_registered);
}
