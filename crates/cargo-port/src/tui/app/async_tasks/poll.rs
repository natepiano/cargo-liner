use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;
use tui_pane::SLOW_BG_BATCH_MS;
use tui_pane::TrackedItem;
use tui_pane::TrackedItemActivity;
use tui_pane::TrackedItemKey;

use super::constants::MAX_MSGS_PER_FRAME;
use crate::scan::BackgroundMsg;
use crate::tui::app::App;
use crate::tui::app::poll_background_stats::PollBackgroundStats;
use crate::tui::app::poll_background_stats::RebuildStatus;
use crate::tui::panes::CiFetchKind;
use crate::tui::panes::PendingCiFetch;
use crate::tui::state::OwnedRunMessageUpdate;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::OwnedRunEvent;

impl App {
    pub fn poll_background(&mut self) -> PollBackgroundStats {
        let mut rebuild_status = RebuildStatus::NotNeeded;
        let mut msg_count = 0;
        let started = Instant::now();
        let mut stats = PollBackgroundStats::default();
        let project_list_revision_before = self.project_list.revision();

        while msg_count < MAX_MSGS_PER_FRAME {
            let Ok(msg) = self.background.background_receiver().try_recv() else {
                break;
            };
            record_background_msg_kind(&mut stats, &msg);
            msg_count += 1;
            rebuild_status.merge_needed(self.handle_bg_msg(msg));
        }
        stats.bg_msgs = msg_count;
        log_saturated_background_batch(&stats);
        stats.ci_msgs = self.poll_ci_fetches();
        self.background.poll_process_termination_worker_readiness();
        stats.example_msgs = self.poll_example_msgs();
        self.poll_clean_msgs();
        let now = Instant::now();
        self.refresh_project_storage_if_due(now);
        self.refresh_crates_io_if_due(now);

        stats.tree_results = 0;
        stats.fit_results = 0;
        stats.disk_results = 0;

        if rebuild_status.needs_rebuild() {
            self.scan.bump_generation();
            self.maybe_priority_fetch();
        }
        stats.rebuild_status = rebuild_status;

        // Background handlers can advance `ProjectList::revision` without
        // routing through `sync_selected_project` (discovery, refresh, disk,
        // and submodule mutations), so re-resolve the enabled monitor scope
        // whenever this batch changed visible ownership.
        if self.project_list.revision() != project_list_revision_before {
            // The scope is resolved from the cached rows and termination
            // authority reads it, so the cache is brought current first. Every
            // handler that changes the rows already rebuilds them, which makes
            // this a guard rather than the rebuild those paths depend on.
            self.ensure_visible_rows_cached();
            self.refresh_compile_monitor_scope_if_on();
        }

        let elapsed = started.elapsed();
        if elapsed.as_millis() >= SLOW_BG_BATCH_MS {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                elapsed_ms = tui_pane::perf_log_ms(elapsed.as_millis()),
                bg_msgs = stats.bg_msgs,
                ci_msgs = stats.ci_msgs,
                example_msgs = stats.example_msgs,
                tree_results = stats.tree_results,
                fit_results = stats.fit_results,
                disk_results = stats.disk_results,
                needs_rebuild = stats.rebuild_status.needs_rebuild(),
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
                        // Schedule `CiFetchKind::Older` using the
                        // (unchanged) cached tail as the cursor. Preserve
                        // the existing toast so the user sees one
                        // continuous "Fetching CI" task.
                        let oldest_created_at = self
                            .project_list
                            .ci_info_for(Path::new(&path))
                            .and_then(|info| info.runs.last().map(|r| r.created_at.clone()));
                        if let Some(oldest_created_at) = oldest_created_at {
                            self.inflight.set_pending_ci_fetch(PendingCiFetch {
                                project_path:      path.clone(),
                                ci_run_count:      self.config.ci_run_count(),
                                oldest_created_at: Some(oldest_created_at),
                                ci_fetch_kind:     CiFetchKind::Older,
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
                                CiFetchKind::Older => "no older runs found".to_string(),
                                CiFetchKind::Sync => "no new runs found".to_string(),
                            }
                        };
                        let result_item = TrackedItem {
                            label,
                            key: TrackedItemKey::new(format!("{path}:result")),
                            started_at: None,
                            completed_at: None,
                            activity: TrackedItemActivity::Progressing,
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
                OwnedRunEvent::Started { owned_run_id } => {
                    let _ = self.inflight.acknowledge_owned_run_started(owned_run_id);
                },
                OwnedRunEvent::Output { owned_run_id, line } => {
                    let _ = self.inflight.record_owned_run_output(owned_run_id, line);
                },
                OwnedRunEvent::Progress { owned_run_id, line } => {
                    let _ = self.inflight.record_owned_run_progress(owned_run_id, line);
                },
                OwnedRunEvent::TerminationOutcome(owned_run_termination_outcome) => {
                    let _ = self
                        .inflight
                        .reconcile_owned_run_termination(owned_run_termination_outcome);
                },
                OwnedRunEvent::Finished { owned_run_id } => {
                    if self.inflight.finish_owned_run(owned_run_id)
                        == OwnedRunMessageUpdate::Applied
                    {
                        // Process exit resumes following the tail so the final output is
                        // visible — unless a selection is holding the view.
                        self.panes.output.on_process_exit();
                    }
                },
            }
            count += 1;
        }
        count
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
        BackgroundMsg::LanguageStatsProgressPlan { .. } => {
            stats.language_progress_msgs += 1;
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
        | BackgroundMsg::CratesIoNewRelease { .. }
        | BackgroundMsg::RepoMeta { .. }
        | BackgroundMsg::ProjectDetailsDeclared { .. }
        | BackgroundMsg::ProjectStorage { .. }
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
    if stats.bg_msgs != MAX_MSGS_PER_FRAME {
        return;
    }

    tracing::trace!(
        target: PERF_LOG_TARGET,
        bg_msgs = stats.bg_msgs,
        disk_usage_msgs = stats.disk_usage_msgs,
        git_info_msgs = stats.git_info_msgs,
        lint_status_msgs = stats.lint_status_msgs,
        language_progress_msgs = stats.language_progress_msgs,
        "poll_background_saturated"
    );
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::project::AbsolutePath;
    use crate::project::Package;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::scan::BackgroundMsg;
    use crate::tui::app::App;
    use crate::tui::state::OwnedProcessGroupSignalOutcome;
    use crate::tui::state::OwnedRunCompletionMarker;
    use crate::tui::state::OwnedRunId;
    use crate::tui::state::OwnedRunTermination;
    use crate::tui::state::OwnedRunTerminationOutcome;
    use crate::tui::state::OwnedRunTerminationSubmission;
    use crate::tui::terminal::OwnedRunEvent;
    use crate::tui::test_support::make_app;

    fn make_package(path: &Path) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(path),
            name: path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            ..Package::default()
        }))
    }

    fn prepare_pending_owned_run(app: &mut App) -> Result<OwnedRunId, Box<dyn std::error::Error>> {
        app.inflight
            .set_example_running(Some("ordered event fixture".to_string()));
        let OwnedRunTermination::Available {
            owned_run_id,
            owned_run_termination_token,
        } = app.inflight.owned_run_termination()
        else {
            return Err("the fixture should expose owned-run termination authority".into());
        };
        let submission = app
            .inflight
            .submit_owned_run_termination(owned_run_termination_token);
        if submission != OwnedRunTerminationSubmission::Submitted(owned_run_id) {
            return Err(format!(
                "the fixture termination request was not accepted: {submission:?}"
            )
            .into());
        }
        Ok(owned_run_id)
    }

    fn queue_termination_outcome_and_completion(
        app: &App,
        owned_run_id: OwnedRunId,
        owned_run_termination_outcome: OwnedRunTerminationOutcome,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let owned_run_event_sender = app.background.example_sender();
        owned_run_event_sender
            .send(OwnedRunEvent::TerminationOutcome(
                owned_run_termination_outcome,
            ))
            .map_err(|_| "the owned-run event channel should accept the outcome")?;
        owned_run_event_sender
            .send(OwnedRunEvent::Finished { owned_run_id })
            .map_err(|_| "the owned-run event channel should accept completion")?;
        Ok(())
    }

    #[test]
    fn queued_successful_signal_outcome_precedes_completion_and_labels_the_run_killed()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = make_app(&[]);
        let owned_run_id = prepare_pending_owned_run(&mut app)?;
        queue_termination_outcome_and_completion(
            &app,
            owned_run_id,
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::Sent,
            },
        )?;

        assert_eq!(app.poll_example_msgs(), 2);
        assert_eq!(
            app.inflight.owned_run().completion_marker(),
            OwnedRunCompletionMarker::Killed
        );
        Ok(())
    }

    #[test]
    fn failed_signal_followed_by_natural_completion_labels_the_run_done()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = make_app(&[]);
        let owned_run_id = prepare_pending_owned_run(&mut app)?;
        queue_termination_outcome_and_completion(
            &app,
            owned_run_id,
            OwnedRunTerminationOutcome::Honored {
                owned_run_id,
                signal: OwnedProcessGroupSignalOutcome::SignalFailed,
            },
        )?;

        assert_eq!(app.poll_example_msgs(), 2);
        assert_eq!(
            app.inflight.owned_run().completion_marker(),
            OwnedRunCompletionMarker::Done
        );
        Ok(())
    }

    #[test]
    fn refused_signal_followed_by_natural_completion_labels_the_run_done()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = make_app(&[]);
        let owned_run_id = prepare_pending_owned_run(&mut app)?;
        queue_termination_outcome_and_completion(
            &app,
            owned_run_id,
            OwnedRunTerminationOutcome::Refused { owned_run_id },
        )?;

        assert_eq!(app.poll_example_msgs(), 2);
        assert_eq!(
            app.inflight.owned_run().completion_marker(),
            OwnedRunCompletionMarker::Done
        );
        Ok(())
    }

    /// `disk_handlers` flips a project to deleted and advances the project-list
    /// revision without routing through `sync_selected_project`, so the batch
    /// that ends in such a flip has to re-resolve the monitor scope itself. The
    /// revision is stamped into the scope key, so a batch that skipped the
    /// re-resolve would leave the enabled monitor — and the termination
    /// authority that reads its scope — holding a scope the very next resolve
    /// declares non-current, tearing classification down every frame.
    #[test]
    fn a_background_revision_bump_leaves_the_monitor_holding_a_current_scope()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        // Never created on disk: `apply_disk_usage` only flips a project to
        // deleted when a zero-byte report meets a path that no longer exists.
        let removed_root = temp_dir.path().join("removed");
        let retained_root = temp_dir.path().join("retained");
        std::fs::create_dir_all(&retained_root)?;
        let mut app = make_app(&[make_package(&removed_root), make_package(&retained_root)]);
        app.ensure_visible_rows_cached();
        app.project_list
            .set_cursor(app.project_list.visible_rows().len().saturating_sub(1));
        app.toggle_compile_visibility(std::time::Instant::now());
        let scope_before_flip = app.resolve_compile_monitor_scope();

        app.background
            .background_sender()
            .send(BackgroundMsg::DiskUsage {
                path:  AbsolutePath::from(removed_root.as_path()),
                bytes: 0,
            })?;
        app.poll_background();

        assert!(
            app.project_list.is_deleted(&removed_root),
            "the zero-byte report against a missing path flipped the project to deleted"
        );
        let scope_after_flip = app.resolve_compile_monitor_scope();
        assert_ne!(
            scope_before_flip.monitor_scope_resolution(),
            scope_after_flip.monitor_scope_resolution(),
            "the flip stamped a new project-list revision into the resolved scope"
        );
        assert_eq!(
            scope_before_flip.monitor_selected_row(),
            scope_after_flip.monitor_selected_row(),
            "a deleted project keeps its row, so the cursor still names the same one"
        );
        assert!(
            !app.compile_visibility_state
                .requires_scope_replacement(&scope_after_flip),
            "the batch already moved the monitor onto the scope the flip resolves to"
        );
        Ok(())
    }
}
