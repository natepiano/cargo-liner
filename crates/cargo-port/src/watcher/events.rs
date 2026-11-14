use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::mpsc::Receiver as StdReceiver;
use std::time::Instant;

use notify::Event;
use tui_pane::PERF_LOG_TARGET;

use super::ProjectEntry;
use super::WatchDrainResult;
use super::WatchState;
use super::WatcherLoopState;
use super::probe;
use super::refresh;
use super::roots;
use super::roots::RegisteredRoots;
use super::roots::Subscription;
use super::roots::WatchedFolderRole;
use crate::channel::Sender;
use crate::config::WatcherConfig;
use crate::constants::GIT_DIR;
use crate::lint;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::GitEventRefresh;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;

pub(super) struct WatcherBackgroundSinks<'a> {
    pub(super) background_tx:     &'a Sender<BackgroundMsg>,
    pub(super) lint_runtime:      Option<&'a RuntimeHandle>,
    pub(super) metadata_dispatch: Option<&'a MetadataDispatchContext>,
}

/// Default-policy routing for existing watcher test harnesses.
#[cfg(test)]
pub(super) fn route_folder_event(
    watcher: &mut impl notify::Watcher,
    registered: &mut RegisteredRoots,
    event: &Event,
    state: &mut WatcherLoopState,
    background_tx: &Sender<BackgroundMsg>,
) -> Vec<Event> {
    route_folder_event_with_config(
        watcher,
        registered,
        event,
        state,
        background_tx,
        &WatcherConfig::default(),
    )
}

/// Default-policy replay for existing watcher test harnesses.
#[cfg(test)]
pub(super) fn replay_buffered_events(
    events: &[Event],
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    replay_buffered_events_with_config(
        events,
        ctx,
        pending_disk,
        pending_git,
        pending_new,
        &WatcherConfig::default(),
    );
}

/// Route with the subscription role before running project-specific handlers.
pub(super) fn route_folder_event_with_config(
    watcher: &mut impl notify::Watcher,
    registered: &mut RegisteredRoots,
    event: &Event,
    state: &mut WatcherLoopState,
    background_tx: &Sender<BackgroundMsg>,
    watcher_config: &WatcherConfig,
) -> Vec<Event> {
    if event.need_rescan() {
        return vec![event.clone()];
    }
    let roles: Vec<_> = event
        .paths
        .iter()
        .map(|path| registered.event_role(path))
        .collect();
    roots::update_directory_watches(watcher, registered, event);
    let mut project_event = event.clone();
    project_event.paths.clear();
    for (path, old_role) in event.paths.iter().zip(roles) {
        if registered.skips(path) {
            continue;
        }
        // An ancestor removal must retire every tracked child, not just the first
        // project-level directory selected by the discovery walk-up.
        if !path.exists() {
            for root in state.retire_projects_under(path) {
                let _ = background_tx.send(BackgroundMsg::DiskUsage {
                    path:  root,
                    bytes: 0,
                });
            }
        }
        let subscription = match registered.event_role(path) {
            Subscription::Unwatched => old_role,
            Subscription::Watched(role) => Subscription::Watched(role),
        };
        match subscription {
            Subscription::Watched(WatchedFolderRole::Project) => {
                project_event.paths.push(path.clone());
            },
            Subscription::Watched(WatchedFolderRole::ConfigSource) => {
                // Drop other Cargo folder files here, such as the Cargo home cache journal.
                if registered.config_source_event_matters(path) {
                    project_event.paths.push(path.clone());
                }
            },
            Subscription::Watched(WatchedFolderRole::Discovery | WatchedFolderRole::Candidate) => {
                // A Cargo config folder can also lead to a known project.
                // Preserve metadata refresh when discovery owns its subscription.
                if matches!(
                    lint::classify_cargo_metadata_basename(path),
                    Some(lint::CargoMetadataTriggerKind::Config)
                ) {
                    project_event.paths.push(path.clone());
                    continue;
                }
                // The event belongs to the watched folder itself or one immediate
                // child. Never descend into an unassigned or candidate subtree.
                let candidate = if path.file_name().is_some_and(|name| name == GIT_DIR)
                    && let Some(parent) = path.parent()
                {
                    parent
                } else if path.is_dir() || !path.exists() {
                    path.as_path()
                } else if let Some(parent) = path.parent() {
                    parent
                } else {
                    continue;
                };
                if matches!(
                    registered.role(candidate),
                    Subscription::Watched(
                        WatchedFolderRole::Discovery | WatchedFolderRole::Candidate
                    )
                ) || path.parent().is_some_and(|parent| {
                    registered.role(parent) == Subscription::Watched(WatchedFolderRole::Discovery)
                }) {
                    let deadline = Instant::now() + watcher_config.new_project_debounce();
                    state
                        .pending_new
                        .entry(AbsolutePath::from(candidate))
                        .or_insert(deadline);
                    if candidate != path.as_path()
                        && registered.role(path)
                            == Subscription::Watched(WatchedFolderRole::Candidate)
                    {
                        state
                            .pending_new
                            .entry(AbsolutePath::from(path.as_path()))
                            .or_insert(deadline);
                    }
                }
            },
            Subscription::Unwatched => {},
        }
    }
    if project_event.paths.is_empty() {
        Vec::new()
    } else {
        vec![project_event]
    }
}

pub(super) fn process_notify_events(
    tick: u64,
    watch_drain: &WatchDrainResult,
    notify_events: Vec<Event>,
    watch_roots: &[AbsolutePath],
    sinks: &WatcherBackgroundSinks<'_>,
    state: &mut WatcherLoopState,
    watcher_config: &WatcherConfig,
) {
    for path in notify_events.iter().flat_map(|event| &event.paths) {
        if !path.exists() {
            for root in state.retire_projects_under(path) {
                let _ = sinks.background_tx.send(BackgroundMsg::DiskUsage {
                    path:  root,
                    bytes: 0,
                });
            }
        }
    }
    let notify_count = notify_events.len();
    if watch_drain.registration_progress.is_completed() {
        tracing::trace!(
            target: PERF_LOG_TARGET,
            tick,
            buffered = state.buffered_events.len(),
            notify_count,
            initializing = state.registration.is_initializing(),
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
            background_tx:     sinks.background_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
        };
        replay_buffered_events_with_config(
            &state.buffered_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
            watcher_config,
        );
        state.buffered_events.clear();
    }
    if state.registration.is_initializing() {
        if notify_count > 0 {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                tick,
                notify_count,
                buffered_total = state.buffered_events.len() + notify_count,
                "watcher_loop_buffering_while_initializing"
            );
        }
        state.buffered_events.extend(notify_events);
    } else {
        if notify_count > 0 {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                tick,
                notify_count,
                "watcher_loop_processing_events"
            );
        }
        let dispatch = WatcherDispatchContext {
            event:             EventContext {
                watch_roots,
                projects: &state.projects,
                project_parents: &state.project_parents,
                discovered: &state.discovered,
            },
            background_tx:     sinks.background_tx,
            lint_runtime:      sinks.lint_runtime,
            metadata_dispatch: sinks.metadata_dispatch,
        };
        replay_buffered_events_with_config(
            &notify_events,
            &dispatch,
            &mut state.pending_disk,
            &mut state.pending_git,
            &mut state.pending_new,
            watcher_config,
        );
    }
}

pub(super) fn drain_notify_events(notify_rx: &StdReceiver<notify::Result<Event>>) -> Vec<Event> {
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

fn replay_buffered_events_with_config(
    events: &[Event],
    ctx: &WatcherDispatchContext<'_>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
    watcher_config: &WatcherConfig,
) {
    for event in events {
        if event.need_rescan() {
            if let Some(dispatch) = ctx.metadata_dispatch {
                dispatch.request_disk_rescan();
            }
            for entry in ctx.event.projects.values() {
                refresh::schedule_disk_refresh(
                    pending_disk,
                    &entry.label,
                    Instant::now(),
                    watcher_config,
                );
            }
            continue;
        }
        for event_path in &event.paths {
            handle_notify_event(
                event_path,
                Some(event),
                &ctx.event,
                ctx.background_tx,
                ctx.lint_runtime,
                ctx.metadata_dispatch,
                pending_disk,
                pending_git,
                pending_new,
                watcher_config,
            );
        }
    }
}

pub(super) fn drain_completed_refreshes(
    disk_done_rx: &StdReceiver<String>,
    git_done_rx: &StdReceiver<AbsolutePath>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    watcher_config: &WatcherConfig,
) {
    while let Ok(project_path) = disk_done_rx.try_recv() {
        refresh::handle_disk_completion(pending_disk, &project_path, watcher_config);
    }

    while let Ok(repo_root) = git_done_rx.try_recv() {
        refresh::handle_git_completion(pending_git, &repo_root, watcher_config);
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
    pub(super) background_tx:     &'a Sender<BackgroundMsg>,
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
    background_tx: &Sender<BackgroundMsg>,
    lint_runtime: Option<&RuntimeHandle>,
    metadata_dispatch: Option<&MetadataDispatchContext>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
    watcher_config: &WatcherConfig,
) {
    let now = Instant::now();

    refresh::try_dispatch_out_of_tree_cargo_config_refresh(event_path, ctx, metadata_dispatch);

    if queue_git_metadata_refresh(event_path, ctx.projects, pending_git, now, watcher_config) {
        return;
    }

    // Match `event_path` against the known root keys in `ctx.projects`.
    if let Some((_, entry)) = ctx
        .projects
        .iter()
        .find(|(root, _)| event_path.starts_with(root))
    {
        if refresh::is_internal_git_path(event_path, entry) {
            return;
        }
        if let Some(lint_runtime) = lint_runtime
            && entry.is_alive()
            && let Some(event) = event
        {
            dispatch_project_lint(lint_runtime, entry, event, event_path);
        }
        if let Some(dispatch) = metadata_dispatch
            && let Some(kind) =
                lint::classify_cargo_metadata_event_path(entry.abs_path.as_path(), event_path)
            && entry.is_alive()
        {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                workspace_root = %entry.abs_path.display(),
                event_path = %event_path.display(),
                ?kind,
                "watcher_cargo_metadata_refresh"
            );
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), entry.abs_path.clone());
        }
        if refresh::is_target_metadata_event(event_path, entry.abs_path.as_path())
            && entry.is_alive()
        {
            probe::spawn_project_refresh(background_tx.clone(), entry.abs_path.clone());
        }
        let resolved_target =
            metadata_dispatch.and_then(|dispatch| dispatch.resolved_target_dir(&entry.abs_path));
        let is_target_event = refresh::is_target_event_for(
            event_path,
            entry.abs_path.as_path(),
            resolved_target.as_deref(),
        );
        if let Some(dispatch) = metadata_dispatch {
            let change = if event.is_some_and(Event::need_rescan) {
                ProjectDiskChange::Rescan
            } else {
                ProjectDiskChange::ChangedPath
            };
            record_project_disk_change(dispatch, entry, event_path, change);
        }
        refresh::schedule_disk_refresh(pending_disk, &entry.label, now, watcher_config);
        let git_event_refresh = if is_target_event {
            GitEventRefresh::Ignored
        } else {
            match (&entry.repo_root, &entry.git_dir) {
                (Some(root), Some(git_dir)) => entry
                    .git_event_visibility
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .requires_refresh(root, git_dir, event_path),
                _ => GitEventRefresh::Required,
            }
        };
        tracing::trace!(
            target: PERF_LOG_TARGET,
            event_path = %event_path.display(),
            project = %entry.abs_path.display(),
            git_refresh = matches!(git_event_refresh, GitEventRefresh::Required) && refresh::git_refresh_key(entry).is_some(),
            "watcher_project_event"
        );
        if matches!(git_event_refresh, GitEventRefresh::Required)
            && let Some(refresh_key) = refresh::git_refresh_key(entry)
        {
            request_project_git_refresh(ctx.projects, entry, event_path);
            refresh::enqueue_git_refresh(
                pending_git,
                refresh_key,
                now,
                false,
                "project_event",
                watcher_config,
            );
        }
        return;
    }

    schedule_discovery_probe(event_path, ctx, pending_new, now, watcher_config);
}

fn request_project_git_refresh(
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    entry: &ProjectEntry,
    event_path: &Path,
) {
    for affected in projects.values().filter(|candidate| {
        candidate.repo_root == entry.repo_root
            && (event_path.starts_with(&candidate.abs_path)
                || candidate.abs_path.starts_with(event_path))
    }) {
        affected.git_refresh_demand.request();
    }
}

#[derive(Clone, Copy)]
enum GitEventKind {
    Fast,
    Internal,
    Unrelated,
    Noise,
}

/// `GitEventClassification` reports the manifest probes that Git event routing performs.
pub(super) struct GitEventClassification {
    git_event_kind:             GitEventKind,
    pub(super) manifest_probes: usize,
}

pub(super) fn classify_project_git_event(
    path: &Path,
    entry: &ProjectEntry,
) -> GitEventClassification {
    let mut manifest_probes = 0;
    let internal_path = refresh::is_internal_git_path(path, entry);
    let possible_fast = internal_path || path.file_name().is_some_and(|name| name == ".gitignore");
    if internal_path
        && path
            .extension()
            .is_some_and(|extension| extension == "lock")
    {
        return GitEventClassification {
            git_event_kind: GitEventKind::Noise,
            manifest_probes,
        };
    }
    let source_change = match (&entry.repo_root, &entry.git_dir, &entry.common_git_dir) {
        (Some(root), Some(git_dir), Some(common)) => entry
            .git_event_visibility
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_rule_source(root, git_dir, common, path),
        _ => false,
    };
    let fast = source_change
        || possible_fast && refresh::is_fast_git_refresh_event(path, entry) && {
            manifest_probes += 1;
            entry.is_alive()
        };
    let internal = internal_path && refresh::is_internal_git_refresh_event(path, entry);
    GitEventClassification {
        git_event_kind: if fast {
            GitEventKind::Fast
        } else if internal {
            GitEventKind::Internal
        } else {
            GitEventKind::Unrelated
        },
        manifest_probes,
    }
}

fn queue_git_metadata_refresh(
    event_path: &Path,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    now: Instant,
    watcher_config: &WatcherConfig,
) -> bool {
    let mut matched_git = false;
    let mut manifest_probes = 0;
    for entry in projects.values() {
        let classification = classify_project_git_event(event_path, entry);
        entry
            .git_event_visibility
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .invalidate(event_path);
        manifest_probes += classification.manifest_probes;
        let fast_git = matches!(classification.git_event_kind, GitEventKind::Fast);
        if matches!(classification.git_event_kind, GitEventKind::Noise) {
            matched_git = true;
            continue;
        }
        if !matches!(classification.git_event_kind, GitEventKind::Unrelated)
            && let Some(refresh_key) = refresh::git_refresh_key(entry)
        {
            matched_git = true;
            entry.git_refresh_demand.request();
            tracing::trace!(
                target: PERF_LOG_TARGET,
                refresh_key = %refresh_key.display(),
                event_path = %event_path.display(),
                fast_git,
                "watcher_git_event"
            );
            let cause = if fast_git { "fast_git" } else { "git_internal" };
            refresh::enqueue_git_refresh(
                pending_git,
                refresh_key,
                now,
                fast_git,
                cause,
                watcher_config,
            );
        }
    }
    tracing::trace!(
        target: PERF_LOG_TARGET,
        event_path = %event_path.display(),
        manifest_probes,
        registered_projects = projects.len(),
        "watcher_git_event_probes"
    );
    matched_git
}

fn dispatch_project_lint(
    runtime: &RuntimeHandle,
    entry: &ProjectEntry,
    event: &Event,
    path: &Path,
) {
    let Some(trigger) = lint::classify_event_path(&entry.abs_path, event.kind, path) else {
        return;
    };
    tracing::debug!(
        project_root = %entry.abs_path.display(),
        event_path = %path.display(),
        trigger = ?trigger.trigger,
        event_kind = ?trigger.event_kind,
        removal = trigger.is_removal(),
        "watcher_lint_trigger"
    );
    runtime.lint_trigger(trigger);
}

#[derive(Clone, Copy)]
enum ProjectDiskChange {
    ChangedPath,
    Rescan,
}

fn record_project_disk_change(
    dispatch: &MetadataDispatchContext,
    entry: &ProjectEntry,
    path: &Path,
    change: ProjectDiskChange,
) {
    let tracker = dispatch.disk_tracker(&entry.abs_path);
    match change {
        ProjectDiskChange::Rescan => tracker.request_rescan(),
        ProjectDiskChange::ChangedPath => tracker.record_change(path),
    }
}

fn schedule_discovery_probe(
    event_path: &Path,
    ctx: &EventContext<'_>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
    now: Instant,
    watcher_config: &WatcherConfig,
) {
    if ctx
        .projects
        .values()
        .any(|entry| refresh::is_internal_git_path(event_path, entry))
        || matches!(
            lint::classify_cargo_metadata_basename(event_path),
            Some(lint::CargoMetadataTriggerKind::Config)
        )
    {
        return;
    }

    // With no match in `ctx.projects`, `probe::project_level_dir` walks up from `event_path` to
    // the level of existing projects. `ctx.project_parents` contains directories already owning a
    // known project, such as ~/rust/.
    let Some(candidate) =
        probe::project_level_dir(event_path, ctx.watch_roots, ctx.project_parents)
    else {
        return;
    };
    // Canonicalize `candidate` so symlinked notify paths match keys in `ctx.discovered`.
    let candidate = AbsolutePath::from(
        candidate
            .to_path_buf()
            .canonicalize()
            .unwrap_or_else(|_| candidate.to_path_buf()),
    );
    // Always enqueue missing `candidate` directories in `pending_new`; skip creations in
    // `ctx.discovered`.
    if !candidate.is_dir() || !ctx.discovered.contains(&candidate) {
        pending_new
            .entry(candidate)
            .or_insert_with(|| now + watcher_config.new_project_debounce());
    }
}

#[cfg(test)]
pub(super) fn handle_event(
    event_path: &Path,
    ctx: &EventContext<'_>,
    background_tx: &Sender<BackgroundMsg>,
    pending_disk: &mut HashMap<String, WatchState>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    pending_new: &mut HashMap<AbsolutePath, Instant>,
) {
    // `handle_event` passes None to `handle_notify_event` to skip metadata refresh in tests:
    // no Tokio runtime is provided, so this avoids spawning without one.
    handle_notify_event(
        event_path,
        None,
        ctx,
        background_tx,
        None,
        None,
        pending_disk,
        pending_git,
        pending_new,
        &WatcherConfig::default(),
    );
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests panic on fixture errors")]
#[allow(clippy::unwrap_used, reason = "tests panic on fixture errors")]
#[allow(clippy::panic, reason = "tests panic on unexpected values")]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::Mutex;

    use super::*;
    use crate::channel;
    use crate::test_support;
    use crate::watcher::events;
    use crate::watcher::watch_test_support;
    #[test]
    fn ignored_worktree_events_keep_disk_sizes_without_git_refresh() {
        let tmp = tempfile::tempdir().expect("fixture");
        let project_dir = ignored_git_project(tmp.path());
        let root = AbsolutePath::from(project_dir.as_path());
        let projects = watch_test_support::tracked_file_projects(&project_dir);
        let watch_roots = vec![AbsolutePath::from(tmp.path())];
        let project_parents = HashSet::from([AbsolutePath::from(tmp.path())]);
        let discovered = HashSet::new();
        let context = watch_test_support::tracked_file_event_context(
            &watch_roots,
            &projects,
            &project_parents,
            &discovered,
        );
        let (sender, receiver) = channel::unbounded();
        let dispatch = dispatch_with_git_lock_check(&sender);
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        crate::project::GIT_COMMANDS.with(|commands| commands.set(0));
        for (relative, visible) in [
            ("target/artifact.o", false),
            ("session.log", false),
            ("nested/session.log", false),
            ("nested/visible.log", true),
            ("ignored/new.txt", false),
            ("scratch.scratch", false),
            ("new.txt", true),
            ("src/main.rs", true),
            ("ignored", true),
        ] {
            let path = project_dir.join(relative);
            std::fs::create_dir_all(path.parent().expect("parent")).expect("parent");
            if path.is_dir() {
                std::fs::remove_dir_all(&path).expect("delete tracked directory");
            } else {
                std::fs::write(&path, "changed").expect("change");
            }
            events::handle_notify_event(
                &path,
                None,
                &context,
                &sender,
                None,
                Some(&dispatch),
                &mut pending_disk,
                &mut pending_git,
                &mut pending_new,
                &WatcherConfig::default(),
            );
            if relative == "target/artifact.o" {
                crate::project::GIT_COMMANDS.with(|commands| {
                    assert_eq!(
                        commands.get(),
                        0,
                        "target artifacts must not launch Git commands"
                    );
                });
            }
            watch_test_support::assert_pending_disk(&pending_disk, "~/demo");
            assert_eq!(!pending_git.is_empty(), visible, "{relative}");
            assert!(pending_new.is_empty(), "{relative}");
            assert!(receiver.try_recv().is_err(), "{relative}");
            pending_disk.clear();
            pending_git.clear();
        }
        crate::project::GIT_COMMANDS.with(|commands| {
            assert!(
                commands.get() > 0,
                "the visibility cache must start Git commands"
            );
        });
        std::fs::write(root.join(".gitignore"), "").expect("remove ignore rule");
        projects[&root]
            .git_event_visibility
            .lock()
            .expect("watcher cache")
            .invalidate(&root.join(".gitignore"));
        let path = root.join("session.log");
        events::handle_notify_event(
            &path,
            None,
            &context,
            &sender,
            None,
            Some(&dispatch),
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
            &WatcherConfig::default(),
        );
        assert!(
            !pending_git.is_empty(),
            "an ignore rule change must refresh the filter"
        );
        assert_external_rule_changes(tmp.path(), &project_dir, &context, &sender, &dispatch);
    }

    fn dispatch_with_git_lock_check(sender: &Sender<BackgroundMsg>) -> MetadataDispatchContext {
        let dispatch = MetadataDispatchContext {
            handle:         test_support::test_runtime().handle().clone(),
            sender:         sender.clone(),
            metadata_store: Arc::new(Mutex::new(crate::project::WorkspaceMetadataStore::new())),
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
        };
        crate::project::GIT_COMMAND_METADATA_STORE.with(|store| {
            *store.borrow_mut() = Arc::downgrade(&dispatch.metadata_store);
        });
        dispatch
    }

    fn assert_external_rule_changes(
        temporary: &Path,
        project_dir: &Path,
        context: &EventContext<'_>,
        sender: &Sender<BackgroundMsg>,
        dispatch: &MetadataDispatchContext,
    ) {
        let root = AbsolutePath::from(project_dir);
        let mut pending_disk = HashMap::new();
        let mut pending_git = HashMap::new();
        let mut pending_new = HashMap::new();
        let external = temporary.join("external-ignore");
        std::fs::write(&external, "*.log\n").expect("external rules");
        let configured = std::process::Command::new("git")
            .current_dir(project_dir)
            .args(["config", "core.excludesfile"])
            .arg(&external)
            .status()
            .expect("external config");
        assert!(configured.success());
        context.projects[&root]
            .git_event_visibility
            .lock()
            .expect("watcher cache")
            .invalidate(&root.join(".git/config"));
        handle_notify_event(
            &root.join("session.log"),
            None,
            context,
            sender,
            None,
            Some(dispatch),
            &mut pending_disk,
            &mut pending_git,
            &mut pending_new,
            &WatcherConfig::default(),
        );
        for contents in ["", "*.log\n"] {
            pending_git.clear();
            std::fs::write(&external, contents).expect("change external rules");
            handle_notify_event(
                &external,
                None,
                context,
                sender,
                None,
                Some(dispatch),
                &mut pending_disk,
                &mut pending_git,
                &mut pending_new,
                &WatcherConfig::default(),
            );
            assert!(
                !pending_git.is_empty(),
                "each external rule change must refresh the checkout"
            );
        }
    }

    fn ignored_git_project(temporary: &Path) -> PathBuf {
        let project_dir = temporary.join("demo");
        watch_test_support::write_tracked_file(&project_dir, "fn main() {}\n");
        std::fs::create_dir_all(project_dir.join("ignored")).expect("tracked ignored directory");
        std::fs::write(project_dir.join("ignored/kept.txt"), "tracked")
            .expect("tracked ignored file");
        test_support::init_git_repo(&project_dir);
        std::fs::write(project_dir.join(".gitignore"), "*.log\n*.rs\nignored/\n")
            .expect("ignore rules");
        std::fs::create_dir_all(project_dir.join("nested")).expect("nested rules");
        std::fs::write(project_dir.join("nested/.gitignore"), "!visible.log\n")
            .expect("nested rules");
        std::fs::create_dir_all(project_dir.join(".git/info")).expect("exclude directory");
        std::fs::write(project_dir.join(".git/info/exclude"), "*.scratch\n").expect("exclude rule");
        project_dir
    }
}
