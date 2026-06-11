mod background_services;
mod config;
mod constants;
mod crates_io_handlers;
mod disk_handlers;
mod dispatch;
mod lint_handlers;
mod lint_runtime;
mod metadata_handlers;
mod poll;
mod priority_fetch;
mod recovery;
mod repo_handlers;
mod running_toasts;
mod service_handlers;
mod startup_phase;
mod tree;

pub use startup_phase::Startup;

use super::App;

impl App {
    pub(super) fn begin_startup_phase(&mut self, lint_registered: usize) {
        self.begin_startup_phase_from_scan(lint_registered);
    }
}
