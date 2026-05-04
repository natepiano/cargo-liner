use std::path::Path;
use std::time::Instant;

use crate::project::AbsolutePath;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::types::PollBackgroundStats;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts::TrackedItem;

impl App {
    pub fn poll_background(&mut self) -> PollBackgroundStats {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.background.bg_rx().try_recv() else {
                break;
            };
            Self::record_background_msg_kind(&mut stats, &msg);
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }
        stats.bg_msgs = msg_count;
        Self::log_saturated_background_batch(&stats);
        stats.ci_msgs = self.poll_ci_fetches();
        stats.example_msgs = self.poll_example_msgs();
        self.poll_clean_msgs();

        stats.tree_results = 0;
        stats.fit_results = 0;
        stats.disk_results = 0;

        if needs_rebuild {
            self.refresh_derived_state();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        let elapsed = started.elapsed();
        if elapsed.as_millis() >= crate::perf_log::SLOW_BG_BATCH_MS {
            tracing::info!(
                elapsed_ms = crate::perf_log::ms(elapsed.as_millis()),
                bg_msgs = stats.bg_msgs,
                ci_msgs = stats.ci_msgs,
                example_msgs = stats.example_msgs,
                tree_results = stats.tree_results,
                fit_results = stats.fit_results,
                disk_results = stats.disk_results,
                needs_rebuild = stats.needs_rebuild,
                items = self.projects().len(),
                "poll_background"
            );
        }
        stats
    }
    pub(super) const fn record_background_msg_kind(
        stats: &mut PollBackgroundStats,
        msg: &BackgroundMsg,
    ) {
        match msg {
            BackgroundMsg::DiskUsage { .. } | BackgroundMsg::DiskUsageBatch { .. } => {
                stats.disk_usage_msgs += 1;
            },
            BackgroundMsg::CheckoutInfo { .. }
            | BackgroundMsg::RepoInfo { .. }
            | BackgroundMsg::GitFirstCommit { .. } => {
                stats.git_info_msgs += 1;
            },
            BackgroundMsg::LintStatus { .. } | BackgroundMsg::LintStartupStatus { .. } => {
                stats.lint_status_msgs += 1;
            },
            BackgroundMsg::CiRuns { .. }
            | BackgroundMsg::RepoFetchQueued { .. }
            | BackgroundMsg::RepoFetchComplete { .. }
            | BackgroundMsg::CratesIoVersion { .. }
            | BackgroundMsg::RepoMeta { .. }
            | BackgroundMsg::Submodules { .. }
            | BackgroundMsg::ScanResult { .. }
            | BackgroundMsg::ProjectDiscovered { .. }
            | BackgroundMsg::ProjectRefreshed { .. }
            | BackgroundMsg::LintCachePruned { .. }
            | BackgroundMsg::ServiceReachable { .. }
            | BackgroundMsg::ServiceRecovered { .. }
            | BackgroundMsg::ServiceUnreachable { .. }
            | BackgroundMsg::ServiceRateLimited { .. }
            | BackgroundMsg::LanguageStatsBatch { .. }
            | BackgroundMsg::CargoMetadata { .. }
            | BackgroundMsg::OutOfTreeTargetSize { .. } => {},
        }
    }
    pub(super) fn log_saturated_background_batch(stats: &PollBackgroundStats) {
        const MAX_MSGS_PER_FRAME: usize = 50;
        if stats.bg_msgs != MAX_MSGS_PER_FRAME {
            return;
        }

        tracing::info!(
            bg_msgs = stats.bg_msgs,
            disk_usage_msgs = stats.disk_usage_msgs,
            git_info_msgs = stats.git_info_msgs,
            lint_status_msgs = stats.lint_status_msgs,
            "poll_background_saturated"
        );
    }
    pub(super) fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.background.ci_fetch_rx().try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    let before = self
                        .projects()
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    self.handle_ci_fetch_complete(&path, result, kind);
                    let after = self
                        .projects()
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    let new_runs = after.saturating_sub(before);
                    if let Some(task_id) = self.ci.take_fetch_toast() {
                        let empty: std::collections::HashSet<String> =
                            std::collections::HashSet::new();
                        self.toasts.complete_missing_items(task_id, &empty);
                        let label = if new_runs > 0 {
                            format!("{new_runs} new runs fetched")
                        } else {
                            "no new runs".to_string()
                        };
                        let result_item = TrackedItem {
                            label,
                            key: AbsolutePath::from(format!("{path}:result")).into(),
                            started_at: None,
                            completed_at: None,
                        };
                        let linger = std::time::Duration::from_secs_f64(
                            self.config.current().tui.task_linger_secs,
                        );
                        self.toasts
                            .add_new_tracked_items(task_id, &[result_item], linger);
                        self.finish_task_toast(task_id);
                    }
                },
            }
            count += 1;
        }
        count
    }
    pub(super) fn poll_example_msgs(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.background.example_rx().try_recv() {
            match msg {
                ExampleMsg::Output(line) => self.inflight.example_output_mut().push(line),
                ExampleMsg::Progress(line) => self.apply_example_progress(line),
                ExampleMsg::Finished => self.finish_example_run(),
            }
            count += 1;
        }
        count
    }
    pub(super) fn apply_example_progress(&mut self, line: String) {
        if let Some(last) = self.inflight.example_output_mut().last_mut() {
            *last = line;
        } else {
            self.inflight.example_output_mut().push(line);
        }
    }
    pub(super) fn finish_example_run(&mut self) {
        self.inflight.set_example_running(None);
        self.inflight
            .example_output_mut()
            .push("── done ──".to_string());
        self.scan.mark_terminal_dirty();
    }
    pub(super) fn poll_clean_msgs(&mut self) {
        while let Ok(msg) = self.background.clean_rx().try_recv() {
            match msg {
                CleanMsg::Finished(abs_path) => {
                    // Normally `handle_disk_usage` removes the path
                    // first (filesystem watcher sees target/ shrink).
                    // This is the safety-net terminator if no disk
                    // update arrives.
                    if self
                        .inflight
                        .clean_mut()
                        .remove(abs_path.as_path())
                        .is_some()
                    {
                        self.sync_running_clean_toast();
                    }
                },
            }
        }
    }
}
