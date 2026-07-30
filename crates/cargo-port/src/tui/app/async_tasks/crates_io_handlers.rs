use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;

use super::constants::CRATES_IO_NEW_RELEASE_TITLE;
use crate::tui::app::App;

impl App {
    /// Insert `name` into the live crates.io fetch tracker. Drives the
    /// "Fetching crates.io info" running toast — single toast that grows
    /// as fetches enqueue and finishes when the tracker drains. Mirrors
    /// the GitHub repo-fetch lifecycle.
    pub(super) fn handle_crates_io_fetch_queued(&mut self, name: String) {
        let now = Instant::now();
        // Grow the crates.io denominator for as long as the startup panel
        // is open — mirrors `handle_repo_fetch_queued`. A fetch queued
        // outside the upfront plan (a submodule, the priority fetch, a
        // recovery refetch) joins the row, and a re-fetch of an
        // already-seen name un-marks it, so the row cannot read done
        // while this fetch is in flight. Once the panel has closed, the
        // fetch only drives the steady-state toast.
        if self.startup.is_collecting() {
            self.startup.crates_io.expected.insert(name.clone());
            self.startup.crates_io.seen.remove(&name);
            // The name is now expected-but-unseen, so any earlier row
            // completion no longer holds.
            self.startup.crates_io.complete_at = None;
        } else if let Some(scan_complete_at) = self.startup.scan_complete_at {
            // Steady state owns the standalone "Fetching crates.io info"
            // toast, so this queue creates it. A cluster of these at a small
            // `since_scan_complete_ms` is the post-startup refetch leak — a
            // recovery refetch that landed after the panel handed off. A
            // genuine steady-state fetch (user refresh, file change) logs a
            // large elapsed and is easy to tell apart.
            tracing::trace!(
                target: PERF_LOG_TARGET,
                crate_name = %name,
                since_scan_complete_ms =
                    tui_pane::perf_log_ms(now.duration_since(scan_complete_at).as_millis()),
                "crates_io_steady_state_fetch_queued"
            );
        }
        self.net.crates_io_running_mut().insert(name, now);
        self.sync_running_crates_io_toast();
    }

    /// Remove `name` from the live tracker; the toast finishes when
    /// the tracker drains. Also advances the startup panel's crates.io row
    /// (`seen` marked here, denominator seeded upfront at startup).
    pub(super) fn handle_crates_io_fetch_complete(&mut self, name: &str) {
        self.net.crates_io_running_mut().remove(name);
        self.startup.crates_io.seen.insert(name.to_string());
        // Every toast-bearing fetch path — startup, watcher discovery,
        // priority fetch — lands here, so this is where the periodic
        // refresh learns a name is already fresh. Without the stamp the
        // first refresh would fire a minute after startup instead of an
        // hour after the name was last queried.
        self.crates_io_checked_at
            .insert(name.to_string(), Instant::now());
        self.maybe_log_startup_phase_completions();
        self.sync_running_crates_io_toast();
    }

    /// A background refresh found `name` published at a version other
    /// than the one on display. The new version is already applied by
    /// the accompanying `BackgroundMsg::CratesIoVersion` messages; this
    /// raises the persistent toast that says so, since the refresh is
    /// otherwise silent and a version that changes while the user is
    /// looking elsewhere would go unnoticed.
    pub(super) fn handle_crates_io_new_release(
        &mut self,
        name: &str,
        previous_version: &str,
        version: &str,
    ) {
        self.framework.toasts.push(
            CRATES_IO_NEW_RELEASE_TITLE,
            format!("{name} {previous_version} → {version}"),
        );
    }
}
