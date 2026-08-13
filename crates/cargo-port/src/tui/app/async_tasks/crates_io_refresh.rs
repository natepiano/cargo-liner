use std::time::Duration;
use std::time::Instant;

use super::background_services::CratesIoRefreshTarget;
use super::constants::CRATES_IO_REFRESH_INTERVAL_SECS;
use super::constants::CRATES_IO_REFRESH_MIN_AGE_SECS;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::CratesIoInfo;
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
        let Some(target) = self.collect_crates_io_fetch_plan().take_stalest(
            &self.crates_io_checked_at,
            now,
            Duration::from_secs(CRATES_IO_REFRESH_MIN_AGE_SECS),
        ) else {
            return;
        };
        // Read the displayed version before the fetch so the worker can
        // tell a genuine release from a first fill.
        let previous_version = target
            .paths
            .first()
            .and_then(|path| self.displayed_crates_io_version(path))
            .map(str::to_string);
        self.crates_io_refresh_at = now;
        self.crates_io_checked_at.insert(target.name.clone(), now);
        self.spawn_crates_io_refresh(target, previous_version);
    }

    /// Fetch `target.name` on a rayon worker and write the result to every
    /// path that displays it.
    ///
    /// No `CratesIoFetchQueued` / `CratesIoFetchComplete` pair: those
    /// drive the "Fetching crates.io info" running toast and the startup
    /// panel's crates.io row, and a refresh that fires every minute for
    /// the life of the session must raise neither. The one toast this
    /// path can produce is [`BackgroundMsg::CratesIoNewRelease`], sent
    /// only when the fetched version differs from `previous_version`.
    fn spawn_crates_io_refresh(
        &self,
        target: CratesIoRefreshTarget,
        previous_version: Option<String>,
    ) {
        let sender = self.background.background_sender();
        let client = self.net.http_client();
        rayon::spawn(move || {
            let (info, signal) = client.fetch_crates_io_info(&target.name);
            scan::emit_service_signal(&sender, signal);
            let Some(info) = info else {
                return;
            };
            for message in crates_io_refresh_messages(target, previous_version, info) {
                let _ = sender.send(message);
            }
        });
    }
}

fn crates_io_refresh_messages(
    target: CratesIoRefreshTarget,
    previous_version: Option<String>,
    info: CratesIoInfo,
) -> Vec<BackgroundMsg> {
    let CratesIoRefreshTarget {
        name,
        paths,
        notification,
    } = target;
    let mut messages = Vec::with_capacity(paths.len() + 1);
    for path in paths {
        messages.push(BackgroundMsg::CratesIoVersion {
            path,
            version: info.version.clone(),
            prerelease: info.prerelease.clone(),
            downloads: info.downloads,
        });
    }
    if let Some(previous_version) = previous_version
        && previous_version != info.version
        && let Some(display_name) = notification.into_name(name)
    {
        messages.push(BackgroundMsg::CratesIoNewRelease {
            display_name,
            previous_version,
            version: info.version,
        });
    }
    messages
}

/// Whether enough time has passed since the last background crates.io
/// request to issue the next one.
fn crates_io_refresh_due(last_refresh: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_refresh)
        >= Duration::from_secs(CRATES_IO_REFRESH_INTERVAL_SECS)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::Duration;
    use std::time::Instant;

    use super::CRATES_IO_REFRESH_INTERVAL_SECS;
    use super::CratesIoInfo;
    use super::crates_io_refresh_due;
    use super::crates_io_refresh_messages;
    use crate::config::CargoPortConfig;
    use crate::config::CratesIoReleaseGroupConfig;
    use crate::config::WorkspaceMemberInclusion;
    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::scan::BackgroundMsg;
    use crate::tui::app::async_tasks::constants::CRATES_IO_NEW_RELEASE_TITLE;
    use crate::tui::test_support::make_app_with_config;

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

    #[test]
    fn coordinated_workspace_refresh_updates_every_crate_and_emits_one_toast() {
        const PREVIOUS_VERSION: &str = "0.19.0";
        const RELEASE_VERSION: &str = "0.20.0";

        let root_path = AbsolutePath::from("/rust/bevy");
        let ecs_path = AbsolutePath::from("/rust/bevy/crates/bevy_ecs");
        let settings_path = AbsolutePath::from("/rust/bevy/crates/bevy_settings");
        let workspace = Workspace {
            path: root_path.clone(),
            name: Some("bevy".to_string()),
            ..Workspace::default()
        };
        let ecs = Package {
            path: ecs_path.clone(),
            name: Some("bevy_ecs".to_string()),
            ..Package::default()
        };
        let settings = Package {
            path: settings_path.clone(),
            name: Some("bevy-settings".to_string()),
            ..Package::default()
        };
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config
            .crates_io
            .release_groups
            .push(CratesIoReleaseGroupConfig {
                representative:    "bevy".to_string(),
                label:             "Bevy".to_string(),
                workspace_members: WorkspaceMemberInclusion::Include,
                members:           Vec::new(),
            });
        let mut app = make_app_with_config(
            &[
                RootItem::Rust(RustProject::Workspace(workspace)),
                RootItem::Rust(RustProject::Package(ecs)),
                RootItem::Rust(RustProject::Package(settings)),
            ],
            &cargo_port_config,
        );
        let paths = [root_path, ecs_path, settings_path];
        for path in &paths {
            app.handle_bg_msg(BackgroundMsg::CratesIoVersion {
                path:       path.clone(),
                version:    PREVIOUS_VERSION.to_string(),
                prerelease: None,
                downloads:  1,
            });
        }

        let now = Instant::now();
        let mut checked_at = HashMap::new();
        let mut targets = Vec::new();
        for _ in &paths {
            if let Some(target) =
                app.collect_crates_io_fetch_plan()
                    .take_stalest(&checked_at, now, Duration::ZERO)
            {
                checked_at.insert(target.name.clone(), now);
                targets.push(target);
            }
        }
        assert_eq!(targets.len(), paths.len());
        for target in targets {
            for message in crates_io_refresh_messages(
                target,
                Some(PREVIOUS_VERSION.to_string()),
                CratesIoInfo {
                    version:    RELEASE_VERSION.to_string(),
                    prerelease: None,
                    downloads:  2,
                },
            ) {
                app.handle_bg_msg(message);
            }
        }

        for path in &paths {
            assert_eq!(
                app.displayed_crates_io_version(path),
                Some(RELEASE_VERSION),
                "release-group path {} receives the fetched version",
                path.as_path().display()
            );
        }
        let release_toasts = app
            .framework
            .toasts
            .active_now()
            .into_iter()
            .filter(|toast| toast.title() == CRATES_IO_NEW_RELEASE_TITLE)
            .collect::<Vec<_>>();
        assert_eq!(release_toasts.len(), 1);
        assert_eq!(
            release_toasts[0].body(),
            format!("Bevy {PREVIOUS_VERSION} → {RELEASE_VERSION}")
        );
    }
}
