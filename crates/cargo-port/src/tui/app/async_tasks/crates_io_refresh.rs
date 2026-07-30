use std::time::Duration;
use std::time::Instant;

use super::constants::CRATES_IO_REFRESH_INTERVAL_SECS;
use super::constants::CRATES_IO_REFRESH_MIN_AGE_SECS;
use crate::project::AbsolutePath;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;

impl App {
    /// Re-query one publishable crate on crates.io per
    /// [`CRATES_IO_REFRESH_INTERVAL_SECS`], oldest-checked first, so a
    /// release published while cargo-port is running stops reading as the
    /// version on display. Startup fetches every name once and never
    /// looks again; without this a session left open for hours shows
    /// whatever crates.io held at launch.
    ///
    /// Two clocks give the whole policy: `crates_io_refresh_at` spaces
    /// requests apart, and `crates_io_checked_at` holds each name back
    /// until its last query is [`CRATES_IO_REFRESH_MIN_AGE_SECS`] old. No
    /// cursor and no queue — a name discovered mid-session enters as
    /// never-queried and wins the next slot, and a removed one is simply
    /// never picked. When more than one name is stale the per-crate
    /// period stretches to one minute times the number of names rather
    /// than the request rate rising.
    pub(super) fn refresh_crates_io_if_due(&mut self, now: Instant) {
        if !crates_io_refresh_due(self.crates_io_refresh_at, now) {
            return;
        }
        // An outage or a tripped rate limit already has a recovery path
        // (`refetch_missing_crates_io_targets`); adding a request a
        // minute on top of it would only deepen the limit.
        if !self.net.crates_io_status().is_available() {
            return;
        }
        let Some((name, paths)) = self.collect_crates_io_fetch_plan().take_stalest(
            &self.crates_io_checked_at,
            now,
            Duration::from_secs(CRATES_IO_REFRESH_MIN_AGE_SECS),
        ) else {
            return;
        };
        // Read the displayed version before the fetch so the worker can
        // tell a genuine release from a first fill.
        let previous_version = paths
            .first()
            .and_then(|path| self.displayed_crates_io_version(path))
            .map(str::to_string);
        self.crates_io_refresh_at = now;
        self.crates_io_checked_at.insert(name.clone(), now);
        self.spawn_crates_io_refresh(name, paths, previous_version);
    }

    /// Fetch `name` on a rayon worker and write the result to every path
    /// that displays it.
    ///
    /// No `CratesIoFetchQueued` / `CratesIoFetchComplete` pair: those
    /// drive the "Fetching crates.io info" running toast and the startup
    /// panel's crates.io row, and a refresh that fires every minute for
    /// the life of the session must raise neither. The one toast this
    /// path can produce is [`BackgroundMsg::CratesIoNewRelease`], sent
    /// only when the fetched version differs from `previous_version`.
    fn spawn_crates_io_refresh(
        &self,
        name: String,
        paths: Vec<AbsolutePath>,
        previous_version: Option<String>,
    ) {
        let sender = self.background.background_sender();
        let client = self.net.http_client();
        rayon::spawn(move || {
            let (info, signal) = client.fetch_crates_io_info(&name);
            scan::emit_service_signal(&sender, signal);
            let Some(info) = info else {
                return;
            };
            for path in paths {
                let _ = sender.send(BackgroundMsg::CratesIoVersion {
                    path,
                    version: info.version.clone(),
                    prerelease: info.prerelease.clone(),
                    downloads: info.downloads,
                });
            }
            if let Some(previous_version) = previous_version
                && previous_version != info.version
            {
                let _ = sender.send(BackgroundMsg::CratesIoNewRelease {
                    name,
                    previous_version,
                    version: info.version,
                });
            }
        });
    }
}

/// Whether enough time has passed since the last background crates.io
/// request to issue the next one.
fn crates_io_refresh_due(last_refresh: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_refresh)
        >= Duration::from_secs(CRATES_IO_REFRESH_INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;
    use std::time::Instant;

    use super::CRATES_IO_REFRESH_INTERVAL_SECS;
    use super::crates_io_refresh_due;

    #[test]
    fn refresh_is_due_only_after_the_full_interval() {
        let start = Instant::now();
        let interval = Duration::from_secs(CRATES_IO_REFRESH_INTERVAL_SECS);

        assert!(!crates_io_refresh_due(
            start,
            start + interval.saturating_sub(Duration::from_millis(1))
        ));
        assert!(crates_io_refresh_due(start, start + interval));
    }
}
