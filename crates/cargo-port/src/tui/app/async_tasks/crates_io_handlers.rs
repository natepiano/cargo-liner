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
        // While startup.is_collecting, startup.crates_io.expected gains name and seen loses it,
        // including submodule/priority/recovery requests outside the initial plan. This prevents
        // done while a refetch is active; after panel closure sync_running_crates_io_toast owns the
        // standalone toast. This matches handle_repo_fetch_queued.
        if self.startup.is_collecting() {
            self.startup.crates_io.expected.insert(name.clone());
            self.startup.crates_io.seen.remove(&name);
            // Reopening `startup.crates_io.lifecycle` invalidates any earlier completion because
            // `name` is now expected but unseen.
            self.startup.crates_io.lifecycle.reopen();
        } else if let Some(scan_complete_at) = self.startup.scan_complete_at {
            // sync_running_crates_io_toast owns the standalone toast after startup;
            // since_scan_complete_ms distinguishes a recovery refetch leak just after handoff from
            // later user/file-change fetches.
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
        // `handle_crates_io_fetch_complete` stamps `crates_io_checked_at` for startup, watcher
        // discovery and priority fetches. Otherwise `refresh_crates_io_if_due` would query a minute
        // after startup rather than an hour after the name's last query.
        self.crates_io_checked_at
            .insert(name.to_string(), Instant::now());
        self.maybe_log_startup_phase_completions();
        self.sync_running_crates_io_toast();
    }

    /// A background refresh found a crate published at a version other than
    /// the one on display. The new version is already applied by the
    /// accompanying `BackgroundMsg::CratesIoVersion` messages; this raises
    /// the persistent toast using either the crate name or its configured
    /// release-group label.
    pub(super) fn handle_crates_io_new_release(
        &mut self,
        display_name: &str,
        previous_version: &str,
        version: &str,
    ) {
        self.framework.toasts.push(
            CRATES_IO_NEW_RELEASE_TITLE,
            format!("{display_name} {previous_version} → {version}"),
        );
    }
}
