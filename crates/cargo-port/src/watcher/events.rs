use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Instant;

use notify::Event;

use super::ProjectEntry;
use super::WatchDrainResult;
use super::WatchState;
use super::WatcherLoopState;
use super::probe;
use super::refresh;
use crate::constants::NEW_PROJECT_DEBOUNCE;
use crate::lint;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;

pub(super) struct WatcherBackgroundSinks<'a> {
    pub(super) bg_tx:             &'a Sender<BackgroundMsg>,
    pub(super) lint_runtime:      Option<&'a RuntimeHandle>,
    pub(super) metadata_dispatch: Option<&'a MetadataDispatchContext>,
}

pub(super) fn process_notify_events(
    tick: u64,
    watch_drain: &WatchDrainResult,
    notify_events: Vec<Event>,
    watch_roots: &[AbsolutePath],
    sinks: &WatcherBackgroundSinks<'_>,
    state: &mut WatcherLoopState,
) {
    let notify_count = notify_events.len();
    if watch_drain.registration_completed {
        tracing::info!(
            tick,
            buffered = state.buffered_events.len(),
            notify_count,
            initializing = state.initializing,
            projects = state.projects.len(),
            "watcher_loop_registration_completed"
        );
        let dispatch = WatcherDispatchContext {
            event:             EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx:             sinks.bg_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
        };
        replay_buffered_events(
            &state.buffered_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
        );
        state.buffered_events.clear();
    }
    if state.initializing {
        if notify_count > 0 {
            tracing::info!(
                tick,
                notify_count,
                buffered_total = state.buffered_events.len() + notify_count,
                "watcher_loop_buffering_while_initializing"
            );
        }
        state.buffered_events.extend(notify_events);
    } else {
        if notify_count > 0 {
            tracing::info!(tick, notify_count, "watcher_loop_processing_events");
        }
        let dispatch = WatcherDispatchContext {
            event:             EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            bg_tx:             sinks.bg_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
        };
        replay_buffered_events(
            &notify_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
        );
    }
}

pub(super) fn drain_notify_events(notify_rx: &Receiver<notify::Result<Event>>) -> Vec<Event> {
    let mut events = Vec::new();
    while let Ok(result) = notify_rx.try_recv() {
        match result {
            Ok(event) => events.push(event),
            Err(err) => {
                tracing::warn!(error = %err, "watcher_notify_error");
            },
        }
    }
    events
}

pub(super) fn replay_buffered_events(
    events: &[Event],
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    for event in events {
        for event_path in &event.paths {
            handle_notify_event(
                event_path,
                Some(event),
                &ctx.event,
                ctx.bg_tx,
                ctx.lint_runtime,
                ctx.metadata_dispatch,
                pending_disk,
                pending_git,
                pending_new,
            );
        }
    }
}

pub(super) fn drain_completed_refreshes(
    disk_done_rx: &Receiver<String>,
    git_done_rx: &Receiver<AbsolutePath>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
) {
    while let Ok(project_path) = disk_done_rx.try_recv() {
        refresh::handle_disk_completion(pending_disk, &project_path);
    }

    while let Ok(repo_root) = git_done_rx.try_recv() {
        refresh::handle_git_completion(pending_git, &repo_root);
    }
}

/// Immutable state needed to classify a filesystem event.
pub(super) struct EventContext<'a> {
    pub(super) watch_roots:     &'a [AbsolutePath],
    pub(super) projects:        &'a HashMap<AbsolutePath, ProjectEntry>,
    pub(super) project_parents: &'a HashSet<AbsolutePath>,
    pub(super) discovered:      &'a HashSet<AbsolutePath>,
}

pub(super) struct WatcherDispatchContext<'a> {
    pub(super) event:             EventContext<'a>,
    pub(super) bg_tx:             &'a Sender<BackgroundMsg>,
    pub(super) lint_runtime:      Option<&'a RuntimeHandle>,
    /// `None` in test harnesses that do not provide a tokio runtime;
    /// disables the metadata refresh branch rather than panicking.
    pub(super) metadata_dispatch: Option<&'a MetadataDispatchContext>,
}

#[allow(
    clippy::too_many_arguments,
    reason = "watcher dispatch needs the raw event plus debounce maps and background contexts"
)]
pub(super) fn handle_notify_event(
    event_path: &Path,
    event: Option<&Event>,
    ctx: &EventContext<'_>,
    bg_tx: &Sender<BackgroundMsg>,
    lint_runtime: Option<&RuntimeHandle>,
    metadata_dispatch: Option<&MetadataDispatchContext>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    let now = Instant::now();

    refresh::try_dispatch_out_of_tree_cargo_config_refresh(event_path, ctx, metadata_dispatch);

    let mut matched_fast_git = false;
    for entry in ctx.projects.values() {
        if refresh::is_fast_git_refresh_event(event_path, entry)
            && let Some(refresh_key) = refresh::git_refresh_key(entry)
        {
            matched_fast_git = true;
            tracing::info!(
                refresh_key = %refresh_key.display(),
                event_path = %event_path.display(),
                "watcher_fast_git_event"
            );
            refresh::emit_root_git_info_refresh(bg_tx, entry);
            refresh::enqueue_git_refresh(pending_git, refresh_key, now, false, "fast_git");
        }
    }
    if matched_fast_git {
        return;
    }

    // Try to match the event to a known project.
    if let Some((_, entry)) = ctx
        .projects
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
    {
        if let Some(lint_runtime) = lint_runtime
            && let Some(event) = event
            && let Some(lint_trigger) =
                lint::classify_event_path(&entry.abs_path, event.kind, event_path)
        {
            lint_runtime.lint_trigger(lint_trigger);
        }
        if let Some(dispatch) = metadata_dispatch
            && let Some(kind) =
                lint::classify_cargo_metadata_event_path(entry.abs_path.as_path(), event_path)
        {
            tracing::info!(
                workspace_root = %entry.abs_path.display(),
                event_path = %event_path.display(),
                ?kind,
                "watcher_cargo_metadata_refresh"
            );
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), entry.abs_path.clone());
        }
        if refresh::is_target_metadata_event(event_path, entry.abs_path.as_path()) {
            probe::spawn_project_refresh(bg_tx.clone(), entry.abs_path.clone());
        }
        if refresh::is_internal_git_path(event_path, entry) {
            if let Some(refresh_key) = refresh::git_refresh_key(entry)
                && refresh::is_internal_git_refresh_event(event_path, entry)
            {
                refresh::enqueue_git_refresh(pending_git, refresh_key, now, false, "git_internal");
            }
            return;
        }
        let resolved_target =
            metadata_dispatch.and_then(|dispatch| dispatch.resolved_target_dir(&entry.abs_path));
        let is_target_event = refresh::is_target_event_for(
            event_path,
            entry.abs_path.as_path(),
            resolved_target.as_deref(),
        );
        refresh::schedule_disk_refresh(pending_disk, &entry.project_label, now);
        if !is_target_event && let Some(refresh_key) = refresh::git_refresh_key(entry) {
            refresh::enqueue_git_refresh(pending_git, refresh_key, now, false, "project_event");
        }
        return;
    }

    // Not a known project — walk up from the event path to find the
    // directory at the same level as existing projects. A "project parent"
    // is any directory that already contains a known project (e.g. `~/rust/`).
    let Some(candidate) =
        probe::project_level_dir(event_path, ctx.watch_roots, ctx.project_parents)
    else {
        return;
    };
    // Canonicalize so symlinked notify paths match existing project keys.
    let candidate = AbsolutePath::from(
        candidate
            .to_path_buf()
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf()),
    );
    // Always enqueue removals (dir gone); for creations, skip already-discovered.
    if !candidate.is_dir() || !ctx.discovered.contains(&candidate) {
        pending_new
            .entry(candidate)
            .or_insert_with(|| now + NEW_PROJECT_DEBOUNCE);
    }
}

#[cfg(test)]
pub(super) fn handle_event(
    event_path: &Path,
    ctx: &EventContext<'_>,
    bg_tx: &Sender<BackgroundMsg>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    // Tests skip the metadata-refresh branch; no tokio runtime is
    // provided so the `None` arm is the safe default.
    handle_notify_event(
        event_path,
        None,
        ctx,
        bg_tx,
        None,
        None,
        pending_disk,
        pending_git,
        pending_new,
    );
}
