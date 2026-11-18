mod event_loop;
mod frame_metrics;
#[cfg(test)]
mod no_color_tests;
mod processes;
mod run;
mod tree_state;

pub(super) use processes::CiFetchMsg;
pub(super) use processes::CleanMsg;
pub(super) use processes::signal_owned_run;
pub use run::run;
#[cfg(test)]
use toml::ser::Error;
pub(super) use tree_state::RestoredTreeState;

use super::app::App;
#[cfg(test)]
use super::state::OwnedRunLaunchStart;

pub(super) fn spawn_priority_fetch(app: &App, path: &str, abs_path: &str, name: Option<&String>) {
    processes::spawn_priority_fetch(app, path, abs_path, name);
}

pub(super) fn rearm_mouse_capture() -> std::io::Result<()> { run::rearm_mouse_capture() }

#[cfg(test)]
pub(super) fn start_queued_target_for_test(app: &mut App) {
    if let OwnedRunLaunchStart::Starting(id) = app.inflight.begin_owned_run_launch() {
        processes::spawn_owned_run_process(app, id);
    }
}

pub(super) fn load_tree_state() -> RestoredTreeState { tree_state::load_tree_state() }

#[cfg(test)]
pub(super) fn serialize_tree_state_for_test(app: &App) -> Result<String, Error> {
    tree_state::serialize_tree_state_for_test(app)
}

#[cfg(test)]
pub(super) fn parse_tree_state_for_test(text: &str) -> RestoredTreeState {
    tree_state::parse_tree_state_for_test(text)
}
