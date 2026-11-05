mod event_loop;
mod frame_metrics;
#[cfg(test)]
mod no_color_tests;
mod processes;
mod run;
mod tree_state;

pub(super) use processes::signal_owned_run;
pub use run::run;
#[cfg(test)]
use toml::ser::Error;

use super::app::App;
use super::project_list::ExpandTarget;
use crate::project::AbsolutePath;

pub(super) fn spawn_priority_fetch(app: &App, path: &str, abs_path: &str, name: Option<&String>) {
    processes::spawn_priority_fetch(app, path, abs_path, name);
}

pub(super) fn rearm_mouse_capture() -> std::io::Result<()> { run::rearm_mouse_capture() }

pub(super) fn load_tree_state() -> (Option<AbsolutePath>, Vec<ExpandTarget>) {
    tree_state::load_tree_state()
}

#[cfg(test)]
pub(super) fn serialize_tree_state_for_test(app: &App) -> Result<String, Error> {
    tree_state::serialize_tree_state_for_test(app)
}

#[cfg(test)]
pub(super) fn parse_tree_state_for_test(text: &str) -> (Option<AbsolutePath>, Vec<ExpandTarget>) {
    tree_state::parse_tree_state_for_test(text)
}
