use std::time::Instant;

use crate::tui::app::App;

impl App {
    /// Insert `name` into the live crates.io fetch tracker. Drives the
    /// "Fetching crates.io info" running toast — single toast that grows
    /// as fetches enqueue and finishes when the tracker drains. Mirrors
    /// the GitHub repo-fetch lifecycle.
    pub(super) fn handle_crates_io_fetch_queued(&mut self, name: String) {
        self.net
            .crates_io
            .running_mut()
            .insert(name, Instant::now());
        self.sync_running_crates_io_toast();
    }

    /// Remove `name` from the live tracker; the toast finishes when
    /// the tracker drains.
    pub(super) fn handle_crates_io_fetch_complete(&mut self, name: &str) {
        self.net.crates_io.running_mut().remove(name);
        self.sync_running_crates_io_toast();
    }
}
