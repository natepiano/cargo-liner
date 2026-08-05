mod event_loop;
mod frame_metrics;
mod processes;
mod run;
mod tree_state;

pub(super) use processes::signal_owned_run;
pub use run::run;

use super::app::App;
pub(super) use super::messages::CiFetchMsg;
pub(super) use super::messages::CleanMsg;
pub(super) use super::messages::OwnedRunEvent;
use super::project_list::ExpandTarget;
use crate::project::AbsolutePath;

pub(super) fn spawn_priority_fetch(app: &App, path: &str, abs_path: &str, name: Option<&String>) {
    processes::spawn_priority_fetch(app, path, abs_path, name);
}

pub(super) fn rearm_input_modes() -> std::io::Result<()> { run::rearm_input_modes() }

pub(super) fn load_tree_state() -> (Option<AbsolutePath>, Vec<ExpandTarget>) {
    tree_state::load_tree_state()
}
