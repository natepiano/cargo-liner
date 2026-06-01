use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use tui_pane::TrackedItem;
use tui_pane::TrackedItemKey;

use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::types::PollBackgroundStats;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PendingCiFetch;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;

impl App {
    pub fn poll_background(&mut self) -> PollBackgroundStats {
        const MAX_MSGS_PER_FRAME: usize = 50;
        let mut needs_rebuild = false;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.background.background_receiver().try_recv() else {
                break;
            };
            record_background_msg_kind(&mut stats, &msg);
            msg_count += 1;
            needs_rebuild |= self.handle_bg_msg(msg);
        }
        stats.bg_msgs = msg_count;
        log_saturated_background_batch(&stats);
        stats.ci_msgs = self.poll_ci_fetches();
        stats.example_msgs = self.poll_example_msgs();
        self.poll_clean_msgs();

        stats.tree_results = 0;
        stats.fit_results = 0;
        stats.disk_results = 0;

        if needs_rebuild {
            self.scan.bump_generation();
            self.maybe_priority_fetch();
        }
        stats.needs_rebuild = needs_rebuild;

        let elapsed = started.elapsed();
        if elapsed.as_millis() >= tui_pane::SLOW_BG_BATCH_MS {
            tracing::info!(
                elapsed_ms = tui_pane::perf_log_ms(elapsed.as_millis()),
                bg_msgs = stats.bg_msgs,
                ci_msgs = stats.ci_msgs,
                example_msgs = stats.example_msgs,
                tree_results = stats.tree_results,
                fit_results = stats.fit_results,
                disk_results = stats.disk_results,
                needs_rebuild = stats.needs_rebuild,
                items = self.project_list.len(),
                "poll_background"
            );
        }
        stats
    }
    pub(super) fn poll_ci_fetches(&mut self) -> usize {
        let mut count = 0;
        while let Ok(msg) = self.background.ci_fetch_rx().try_recv() {
            match msg {
                CiFetchMsg::Complete { path, result, kind } => {
                    let before = self
                        .project_list
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    let chain_older = self.handle_ci_fetch_complete(&path, result, kind);
                    let after = self
                        .project_list
                        .ci_info_for(Path::new(&path))
                        .map_or(0, |info| info.runs.len());
                    let new_runs = after.saturating_sub(before);
                    if chain_older {
                        // Sync turned up nothing; schedule the follow-up
                        // FetchOlder using the (unchanged) cached tail as
                        // the cursor. Preserve the existing toast so the
                        // user sees one continuous "Fetching CI" task.
                        let oldest_created_at = self
                            .project_list
                            .ci_info_for(Path::new(&path))
                            .and_then(|info| info.runs.last().map(|r| r.created_at.clone()));
                        if let Some(oldest_created_at) = oldest_created_at {
                            self.inflight.set_pending_ci_fetch(PendingCiFetch {
                                project_path:      path.clone(),
                                ci_run_count:      self.config.ci_run_count(),
                                oldest_created_at: Some(oldest_created_at),
                                kind:              CiFetchKind::FetchOlder,
                            });
                            count += 1;
                            continue;
                        }
                    }
                    if let Some(task_id) = self.ci.take_fetch_toast() {
                        let empty: HashSet<TrackedItemKey> = HashSet::new();
                        self.framework
                            .toasts
                            .complete_missing_items(task_id, &empty);
                        let label = if new_runs > 0 {
                            format!("{new_runs} new runs fetched")
                        } else {
                            match kind {
                                CiFetchKind::FetchOlder => "no older runs found".to_string(),
                                CiFetchKind::Sync => "no new runs found".to_string(),
                            }
                        };
                        let result_item = TrackedItem {
                            label,
                            key: TrackedItemKey::new(format!("{path}:result")),
                            started_at: None,
                            completed_at: None,
                        };
                        self.framework
                            .toasts
                            .add_new_tracked_items(task_id, &[result_item]);
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
                ExampleMsg::Progress(line) => self.inflight.apply_example_progress(line),
                ExampleMsg::Finished => self.finish_example_run(),
            }
            count += 1;
        }
        count
    }
    pub(super) fn finish_example_run(&mut self) {
        self.inflight.set_example_running(None);
        self.inflight.append_done_marker();
        // Process exit resumes following the tail so the final output is
        // visible — unless a selection is holding the view.
        self.panes.output.on_process_exit();
        self.scan.mark_terminal_dirty();
    }
    pub(super) fn poll_clean_msgs(&mut self) {
        while let Ok(msg) = self.background.clean_rx().try_recv() {
            match msg {
                CleanMsg::Finished(abs_path) => {
                    // The process exit is the completion signal. Disk
                    // usage updates may arrive earlier while `cargo
                    // clean` is still deleting files.
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
        BackgroundMsg::LintStatus { .. }
        | BackgroundMsg::LintStartupStatus { .. }
        | BackgroundMsg::LintHistoryLoaded { .. } => {
            stats.lint_status_msgs += 1;
        },
        BackgroundMsg::CiRuns { .. }
        | BackgroundMsg::PullRequests { .. }
        | BackgroundMsg::PullRequestCheckPollStopped { .. }
        | BackgroundMsg::PullRequestDisappeared { .. }
        | BackgroundMsg::RepoFetchQueued { .. }
        | BackgroundMsg::RepoFetchComplete { .. }
        | BackgroundMsg::CratesIoVersion { .. }
        | BackgroundMsg::CratesIoFetchQueued { .. }
        | BackgroundMsg::CratesIoFetchComplete { .. }
        | BackgroundMsg::RepoMeta { .. }
        | BackgroundMsg::Submodules { .. }
        | BackgroundMsg::ScanResult { .. }
        | BackgroundMsg::ProjectDiscovered { .. }
        | BackgroundMsg::ProjectRefreshed { .. }
        | BackgroundMsg::LintCachePruned { .. }
        | BackgroundMsg::LintCacheUsage { .. }
        | BackgroundMsg::ServiceReachable { .. }
        | BackgroundMsg::ServiceRecovered { .. }
        | BackgroundMsg::ServiceUnreachable { .. }
        | BackgroundMsg::ServiceUnreachableConfirmed { .. }
        | BackgroundMsg::ServiceRateLimited { .. }
        | BackgroundMsg::LanguageStatsBatch { .. }
        | BackgroundMsg::TestCountsBatch { .. }
        | BackgroundMsg::SccacheStats { .. }
        | BackgroundMsg::CargoMetadata { .. }
        | BackgroundMsg::OutOfTreeTargetSize { .. }
        | BackgroundMsg::AppearanceChanged(_) => {},
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
