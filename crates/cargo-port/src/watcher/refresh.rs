use std::collections::HashMap;

#[cfg(test)]
#[path = "disk_burst_tests.rs"]
mod disk_burst_tests;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender as StdSender;
use std::time::Duration;
use std::time::Instant;

use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tui_pane::PERF_LOG_TARGET;

use super::ProjectEntry;
use super::WatchState;
use super::events::EventContext;
use crate::channel::Sender;
use crate::config::WatcherConfig;
use crate::constants::CARGO_TOML;
use crate::constants::GIT_DIR;
use crate::constants::TARGET_DIR;
use crate::lint;
use crate::process_observation::DiskReadBytes;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::RepoInfo;
use crate::project::ResolvedBranch;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::DirSizes;
use crate::scan::DiskUpdateScope;
use crate::scan::DiskUsageTracker;
#[cfg(test)]
use crate::scan::DiskUsageTree;
use crate::scan::MetadataDispatchContext;

pub(super) const DISK_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// The watcher calls `schedule_disk_poll_with_config` to detect deletions without tree walks.
pub(super) fn schedule_disk_poll_with_config(
    now: Instant,
    last_poll: &mut Instant,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_disk: &mut HashMap<String, WatchState>,
    watcher_config: &WatcherConfig,
) {
    if now.saturating_duration_since(*last_poll) < DISK_POLL_INTERVAL {
        return;
    }
    *last_poll = now;
    for entry in projects.values() {
        if entry.is_alive() || entry.abs_path.is_dir() {
            continue;
        }
        let state = pending_disk
            .entry(entry.label.clone())
            .or_insert(WatchState::Idle);
        if matches!(state, WatchState::Idle) {
            *state = WatchState::pending(now, true, watcher_config);
        }
    }
}

/// Default-policy entry point for existing watcher test harnesses.
#[cfg(test)]
pub(super) fn schedule_disk_poll(
    now: Instant,
    last_poll: &mut Instant,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_disk: &mut HashMap<String, WatchState>,
) {
    schedule_disk_poll_with_config(
        now,
        last_poll,
        projects,
        pending_disk,
        &WatcherConfig::default(),
    );
}

enum DiskProbeScope {
    ProjectTree,
    TreeAndLinkedTarget {
        target_dir: AbsolutePath,
    },
    TreeAndExternalTarget {
        workspace_root: AbsolutePath,
        target_dir:     AbsolutePath,
    },
}

fn disk_probe_scope(dispatch: &MetadataDispatchContext, path: &AbsolutePath) -> DiskProbeScope {
    if let Ok(store) = dispatch.metadata_store.lock()
        && let Some(root) = store.containing_checkout_root(path)
        && let Some(target) = store.resolved_target_dir(path)
    {
        if !target.starts_with(root) {
            return DiskProbeScope::TreeAndExternalTarget {
                workspace_root: root.clone(),
                target_dir:     target.clone(),
            };
        }
        return linked_target_scope(path, target);
    }
    linked_target_scope(path, &AbsolutePath::from(path.join(TARGET_DIR)))
}

fn linked_target_scope(path: &AbsolutePath, target: &AbsolutePath) -> DiskProbeScope {
    if target.starts_with(path)
        && let Ok(resolved_target) = target.canonicalize()
        && let Ok(resolved_project) = path.canonicalize()
        && !resolved_target.starts_with(resolved_project)
    {
        return DiskProbeScope::TreeAndLinkedTarget {
            target_dir: AbsolutePath::from(resolved_target),
        };
    }
    DiskProbeScope::ProjectTree
}

#[cfg(test)]
fn project_disk_sizes(path: &AbsolutePath, scope: &DiskProbeScope) -> DirSizes {
    let tree = DiskUsageTree {
        root_abs_path: path.clone(),
        entries:       vec![path.clone()],
    };
    let mut sizes = scan::dir_sizes_for_tree(&tree)
        .into_iter()
        .next()
        .map_or_else(DirSizes::default, |(_, sizes)| sizes);
    if path.is_dir()
        && let DiskProbeScope::TreeAndLinkedTarget { target_dir } = scope
    {
        let target = scan::dir_size(target_dir);
        sizes.total = sizes.total.saturating_add(target);
        sizes.in_project_target = sizes.in_project_target.saturating_add(target);
    }
    sizes
}

pub(super) fn disk_refresh_message(path: &AbsolutePath, sizes: DirSizes) -> BackgroundMsg {
    BackgroundMsg::DiskUsageBatch {
        root_path: path.clone(),
        entries:   vec![(path.clone(), sizes)],
    }
}

pub(super) fn schedule_disk_refresh(
    pending_disk: &mut HashMap<String, WatchState>,
    project_label: &str,
    now: Instant,
    watcher_config: &WatcherConfig,
) {
    match pending_disk
        .entry(project_label.to_string())
        .or_insert(WatchState::Idle)
    {
        state @ WatchState::Idle => {
            *state = WatchState::pending(now, false, watcher_config);
        },
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = now + watcher_config.debounce();
        },
        state @ WatchState::Running => *state = WatchState::RunningDirty,
        WatchState::RunningDirty => {},
    }
}

pub(super) fn handle_disk_completion(
    pending_disk: &mut HashMap<String, WatchState>,
    project_label: &str,
    watcher_config: &WatcherConfig,
) {
    let now = Instant::now();
    let Some(state) = pending_disk.get_mut(project_label) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false, watcher_config),
        WatchState::Idle | WatchState::Pending { .. } => return,
    };
}

pub(super) fn handle_git_completion(
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    repo_root: &AbsolutePath,
    watcher_config: &WatcherConfig,
) {
    let now = Instant::now();
    let Some(state) = pending_git.get_mut(repo_root) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false, watcher_config),
        WatchState::Idle | WatchState::Pending { .. } => return,
    };
}

pub(super) fn is_fast_git_refresh_event(event_path: &Path, entry: &ProjectEntry) -> bool {
    let Some(repo_root) = entry.repo_root.as_deref() else {
        return false;
    };
    let Some(git_dir) = entry.git_dir.as_deref() else {
        return false;
    };
    let Some(common_git_dir) = entry.common_git_dir.as_deref() else {
        return false;
    };
    event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("info").join("exclude")
        || event_path == git_dir.join("HEAD")
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
        || is_worktree_git_fallback_event(event_path, git_dir)
}

pub(super) fn is_internal_git_refresh_event(event_path: &Path, entry: &ProjectEntry) -> bool {
    let Some(git_dir) = entry.git_dir.as_deref() else {
        return false;
    };
    let Some(common_git_dir) = entry.common_git_dir.as_deref() else {
        return false;
    };
    let Some(repo_root) = entry.repo_root.as_deref() else {
        return false;
    };
    event_path == repo_root.join(".gitignore")
        || event_path == git_dir.join("index")
        || event_path == git_dir.join("info").join("exclude")
        || event_path == git_dir.join("HEAD")
        || event_path == git_dir.join("FETCH_HEAD")
        || event_path == git_dir.join("ORIG_HEAD")
        || event_path == git_dir.join("config")
        || event_path == git_dir.join("packed-refs")
        || event_path.starts_with(git_dir.join("refs").join("heads"))
        || event_path.starts_with(git_dir.join("refs").join("remotes"))
        || event_path == common_git_dir.join("packed-refs")
        || event_path.starts_with(common_git_dir.join("refs").join("heads"))
        || event_path.starts_with(common_git_dir.join("refs").join("remotes"))
        || is_worktree_git_fallback_event(event_path, git_dir)
}

pub(super) fn is_worktree_git_fallback_event(event_path: &Path, git_dir: &Path) -> bool {
    let logs_dir = git_dir.join("logs");
    event_path == git_dir || event_path == logs_dir || event_path.starts_with(&logs_dir)
}

/// Key used to dedup git refreshes in `pending_git`. Prefers the
/// shared `common_git_dir` so primary + linked worktrees of the same
/// repo collapse into a single pending refresh; falls back to the
/// per-entry `repo_root` when the common-git-dir lookup is missing
/// (degenerate case — e.g. a worktree whose `.git` file points at a
/// path we couldn't resolve).
pub(super) fn git_refresh_key(entry: &ProjectEntry) -> Option<AbsolutePath> {
    entry
        .common_git_dir
        .clone()
        .or_else(|| entry.repo_root.clone())
}

pub(super) fn enqueue_git_refresh(
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    repo_root: AbsolutePath,
    now: Instant,
    immediate: bool,
    cause: &str,
    watcher_config: &WatcherConfig,
) {
    let pending_count = pending_git
        .iter()
        .filter(|(path, _)| path.as_path() != repo_root.as_path())
        .filter(|(_, state)| matches!(state, WatchState::Pending { .. }))
        .count()
        + usize::from(!matches!(
            pending_git.get(&repo_root),
            Some(WatchState::Pending { .. })
        ));
    tracing::trace!(
        target: PERF_LOG_TARGET,
        repo_root = %repo_root.display(),
        immediate,
        cause,
        pending_git = pending_count,
        "watcher_enqueue_git_refresh"
    );
    match pending_git.entry(repo_root).or_insert(WatchState::Idle) {
        state @ WatchState::Idle => *state = WatchState::pending(now, immediate, watcher_config),
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = if immediate {
                now
            } else {
                now + watcher_config.debounce()
            };
        },
        state @ WatchState::Running => *state = WatchState::RunningDirty,
        WatchState::RunningDirty => {},
    }
}

fn is_ready_to_launch(state: &WatchState, now: Instant) -> bool {
    matches!(
        state,
        WatchState::Pending {
            debounce_deadline,
            max_deadline,
        } if now >= *debounce_deadline || now >= *max_deadline
    )
}

const fn mark_running(state: &mut WatchState) {
    if matches!(state, WatchState::Pending { .. }) {
        *state = WatchState::Running;
    }
}

pub(super) fn is_internal_git_path(event_path: &Path, entry: &ProjectEntry) -> bool {
    let repo_root = entry.repo_root.as_deref();
    let git_dir = entry.git_dir.as_deref();
    let common_git_dir = entry.common_git_dir.as_deref();
    // `is_internal_git_path` matches `git_dir` for worktrees, `common_git_dir` for shared
    // refs/heads updates outside the worktree Git directory, and `repo_root.join(GIT_DIR)` for
    // normal repositories where git_dir is repo_root/.git.
    git_dir.is_some_and(|d| event_path.starts_with(d))
        || common_git_dir.is_some_and(|d| event_path.starts_with(d))
        || repo_root.is_some_and(|r| event_path.starts_with(r.join(GIT_DIR)))
}

/// Cargo's `target-directory` may be redirected by an out-of-tree
/// `<dir>/.cargo/config[.toml]` (typically `~/.cargo/config.toml`,
/// the cargo home). Edits to such a config affect every project
/// nested under `<dir>`, none of which contains the event path —
/// so the per-project `classify_cargo_metadata_event_path` gate at
/// the bottom of `handle_notify_event` will not fire for them.
///
/// When the event basename matches a cargo-metadata trigger AND the
/// path looks like `<dir>/.cargo/config[.toml]`, fan a metadata
/// refresh out to every project whose root is descendant of `<dir>`.
pub(super) fn try_dispatch_out_of_tree_cargo_config_refresh(
    event_path: &Path,
    ctx: &EventContext<'_>,
    metadata_dispatch: Option<&MetadataDispatchContext>,
) {
    let Some(dispatch) = metadata_dispatch else {
        return;
    };
    if !matches!(
        lint::classify_cargo_metadata_basename(event_path),
        Some(lint::CargoMetadataTriggerKind::Config)
    ) {
        return;
    }
    let Some(cargo_dir) = event_path.parent() else {
        return;
    };
    let Some(host_dir) = cargo_dir.parent() else {
        return;
    };
    for project_root in ctx.projects.keys() {
        if project_root.as_path().starts_with(host_dir) {
            scan::spawn_cargo_metadata_refresh(dispatch.clone(), project_root.clone());
        }
    }
}

/// Does `event_path` sit under the workspace's resolved target
/// directory? `resolved_target = None` means we don't yet have
/// workspace metadata — fall back to `<project_root>/target`, which is
/// what cargo uses by default.
///
/// When the metadata *is* available (e.g. target is redirected via
/// `CARGO_TARGET_DIR` or `.cargo/config.toml`), events under the real
/// target dir are suppressed and events under the in-tree `target/`
/// decoy are treated as ordinary project events. The design doc
/// (call-site migrations → step 2) calls this out explicitly.
pub(super) fn is_target_event_for(
    event_path: &Path,
    project_root: &Path,
    resolved_target: Option<&Path>,
) -> bool {
    let fallback = project_root.join(TARGET_DIR);
    let dir = resolved_target.unwrap_or(fallback.as_path());
    event_path.starts_with(dir)
}

pub(super) fn is_target_metadata_event(event_path: &Path, project_root: &Path) -> bool {
    let cargo_toml = project_root.join(CARGO_TOML);
    let build_rs = project_root.join("build.rs");
    let src_main = project_root.join("src").join("main.rs");
    let src_bin = project_root.join("src").join("bin");
    let examples = project_root.join("examples");
    let benches = project_root.join("benches");
    let tests = project_root.join("tests");

    event_path == cargo_toml
        || event_path == build_rs
        || event_path == src_main
        || event_path.starts_with(src_bin)
        || event_path.starts_with(examples)
        || event_path.starts_with(benches)
        || event_path.starts_with(tests)
}

pub(super) fn fire_git_updates(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: &StdSender<AbsolutePath>,
    background_tx: &Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
) {
    let now = Instant::now();
    let ready: Vec<AbsolutePath> = pending_git
        .iter()
        .filter(|(_, state)| is_ready_to_launch(state, now))
        .map(|(refresh_key, _)| refresh_key.clone())
        .collect();

    for refresh_key in ready {
        // `affected` collects each `ProjectEntry` sharing `git_refresh_key`; `common_git_dir`
        // combines primary and linked siblings in one repository. Each needs its own
        // `CheckoutInfo` probe because branch, HEAD and status differ per worktree.
        let affected = requested_git_checkouts(projects, &refresh_key);
        if affected.is_empty() {
            if let Some(state) = pending_git.get_mut(&refresh_key) {
                *state = WatchState::Idle;
            }
            continue;
        }
        // `primary_path` identifies the entry whose own .git is the shared `refresh_key` directory
        // (the working tree is <git_dir>/..). Fall back to `affected[0]` when no clear primary is
        // visible, for example when an entry was registered without `common_git_dir`.
        let primary_path = projects
            .values()
            .find(|entry| {
                entry.common_git_dir.as_deref() == Some(refresh_key.as_path())
                    && entry.abs_path.as_path().join(GIT_DIR).is_dir()
            })
            .map_or_else(|| affected[0].clone(), |entry| entry.abs_path.clone());
        if let Some(state) = pending_git.get_mut(&refresh_key) {
            mark_running(state);
        }
        spawn_git_refresh(
            handle,
            git_limit,
            git_done_tx.clone(),
            background_tx.clone(),
            refresh_key,
            primary_path,
            affected,
        );
    }
}

pub(super) fn requested_git_checkouts(
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    refresh_key: &AbsolutePath,
) -> Vec<AbsolutePath> {
    let candidates: Vec<_> = projects
        .values()
        .filter(|entry| git_refresh_key(entry).as_ref() == Some(refresh_key))
        .collect();
    let requested: Vec<_> = candidates
        .iter()
        .filter(|entry| {
            matches!(
                entry.git_refresh_demand.take(),
                super::runtime::GitRefreshRequest::Pending
            )
        })
        .map(|entry| entry.abs_path.clone())
        .collect();
    if requested.is_empty() {
        candidates
            .iter()
            .map(|entry| entry.abs_path.clone())
            .collect()
    } else {
        requested
    }
}

pub(super) fn spawn_git_refresh(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: StdSender<AbsolutePath>,
    background_tx: Sender<BackgroundMsg>,
    refresh_key: AbsolutePath,
    primary_path: AbsolutePath,
    affected: Vec<AbsolutePath>,
) {
    let handle = handle.clone();
    let git_limit = Arc::clone(git_limit);
    handle.spawn(async move {
        let queue_started = Instant::now();
        let Ok(_permit) = git_limit.acquire_owned().await else {
            return;
        };
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(queue_started.elapsed().as_millis()),
            refresh_key = %refresh_key.display(),
            affected_rows = affected.len(),
            "watcher_git_queue_wait"
        );

        // Probe the per-repo half once at the primary's path. Linked
        // siblings reuse this RepoInfo via the entry's `git_repo` slot
        // (the primary-only write policy in `handle_repo_info` keeps
        // just this copy).
        let started = Instant::now();
        let primary_for_repo = primary_path.clone();
        let repo_info =
            tokio::task::spawn_blocking(move || RepoInfo::get(primary_for_repo.as_path()))
                .await
                .ok()
                .flatten();
        let local_main_branch = repo_info
            .as_ref()
            .map_or(ResolvedBranch::Unreadable, |r| r.local_main_branch.clone());
        if let Some(repo_info) = repo_info {
            let _ = background_tx.send(BackgroundMsg::RepoInfo {
                path: primary_path.clone(),
                info: repo_info,
            });
        }

        // Probe `CheckoutInfo` for every `checkout_path` in `affected`: no per-remote loop makes
        // these probes cheap, and each reports its worktree's own branch, HEAD and status.
        for checkout_path in affected {
            let probe_path = checkout_path.clone();
            let lmb = local_main_branch.clone();
            let checkout =
                tokio::task::spawn_blocking(move || CheckoutInfo::get(probe_path.as_path(), &lmb))
                    .await
                    .ok()
                    .flatten();
            if let Some(checkout) = checkout {
                let _ = background_tx.send(BackgroundMsg::CheckoutInfo {
                    path: checkout_path,
                    info: checkout,
                });
            }
        }

        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            refresh_key = %refresh_key.display(),
            "watcher_git_refresh"
        );
        let _ = git_done_tx.send(refresh_key);
    });
}

pub(super) fn fire_disk_updates(
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: &StdSender<String>,
    background_tx: &Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_disk: &mut HashMap<String, WatchState>,
    metadata_dispatch: &MetadataDispatchContext,
) {
    let now = Instant::now();
    let ready: Vec<String> = pending_disk
        .iter()
        .filter(|(_, state)| is_ready_to_launch(state, now))
        .map(|(key, _)| key.clone())
        .collect();

    let mut requests: HashMap<AbsolutePath, DiskRefreshRequest> = HashMap::new();
    for project_label in ready {
        let Some(entry) = projects.values().find(|entry| entry.label == project_label) else {
            if let Some(state) = pending_disk.get_mut(&project_label) {
                *state = WatchState::Idle;
            }
            continue;
        };
        if let Some(state) = pending_disk.get_mut(&project_label) {
            mark_running(state);
        }
        let tracker = metadata_dispatch.disk_tracker(&entry.abs_path);
        if !entry.abs_path.is_dir() {
            tracker.record_change(&entry.abs_path);
        }
        let root = tracker.tree().root_abs_path.clone();
        requests
            .entry(root.clone())
            .or_insert_with(|| DiskRefreshRequest {
                root,
                labels: Vec::new(),
                inputs: DiskUpdateInputs::new(metadata_dispatch, &entry.abs_path),
            })
            .labels
            .push(project_label);
    }
    for request in requests.into_values() {
        spawn_disk_update(
            handle,
            disk_limit,
            disk_done_tx.clone(),
            background_tx.clone(),
            request,
        );
    }
}

struct DiskRefreshRequest {
    root:   AbsolutePath,
    labels: Vec<String>,
    inputs: DiskUpdateInputs,
}

struct DiskUpdateInputs {
    project: DiskUsageTracker,
    targets: Vec<(AbsolutePath, TargetDiskUpdate)>,
}

enum TargetDiskUpdate {
    InProject,
    Linked(DiskUsageTracker),
    External {
        workspace_root: AbsolutePath,
        target_dir:     AbsolutePath,
        tracker:        DiskUsageTracker,
    },
}

impl DiskUpdateInputs {
    fn new(dispatch: &MetadataDispatchContext, project: &AbsolutePath) -> Self {
        let tracker = dispatch.disk_tracker(project);
        let targets = tracker
            .tree()
            .entries
            .iter()
            .map(|path| {
                let target = match disk_probe_scope(dispatch, path) {
                    DiskProbeScope::ProjectTree => TargetDiskUpdate::InProject,
                    DiskProbeScope::TreeAndLinkedTarget { target_dir } => {
                        TargetDiskUpdate::Linked(dispatch.target_disk_tracker(path, &target_dir))
                    },
                    DiskProbeScope::TreeAndExternalTarget {
                        workspace_root,
                        target_dir,
                    } => TargetDiskUpdate::External {
                        tracker: dispatch.target_disk_tracker(path, &target_dir),
                        workspace_root,
                        target_dir,
                    },
                };
                (path.clone(), target)
            })
            .collect();
        Self {
            project: tracker,
            targets,
        }
    }

    fn measure(self) -> ProjectDiskUpdate {
        let reads = crate::process_observation::DiskReadMeasurement::start();
        let mut update = self.project.refresh();
        let mut target_bytes = HashMap::new();
        let mut external_targets = Vec::new();
        for (path, target) in self.targets {
            match target {
                TargetDiskUpdate::InProject => {},
                TargetDiskUpdate::Linked(tracker) => {
                    let bytes = measure_target(&tracker, update.scope, &mut target_bytes);
                    if let Some((_, sizes)) =
                        update.entries.iter_mut().find(|(entry, _)| *entry == path)
                    {
                        sizes.total = sizes.total.saturating_add(bytes);
                        sizes.in_project_target = sizes.in_project_target.saturating_add(bytes);
                    }
                },
                TargetDiskUpdate::External {
                    workspace_root,
                    target_dir,
                    tracker,
                } => {
                    let bytes = measure_target(&tracker, update.scope, &mut target_bytes);
                    external_targets.push(BackgroundMsg::OutOfTreeTargetSize {
                        workspace_root,
                        target_dir,
                        bytes,
                    });
                },
            }
        }
        ProjectDiskUpdate {
            read_bytes: reads.finish(),
            entries: update.entries,
            external_targets,
            visited_entries: update.visited_entries,
            identity_entries_examined: update.identity_entries_examined,
            aggregation_entries_examined: update.aggregation_entries_examined,
        }
    }
}

fn measure_target(
    tracker: &DiskUsageTracker,
    scope: DiskUpdateScope,
    measured: &mut HashMap<AbsolutePath, u64>,
) -> u64 {
    *measured
        .entry(tracker.tree().root_abs_path.clone())
        .or_insert_with(|| {
            if matches!(scope, DiskUpdateScope::WholeTree) {
                tracker.request_rescan();
            }
            tracker
                .refresh()
                .entries
                .first()
                .map_or(0, |(_, sizes)| sizes.total)
        })
}

struct ProjectDiskUpdate {
    read_bytes:                   DiskReadBytes,
    entries:                      Vec<(AbsolutePath, DirSizes)>,
    external_targets:             Vec<BackgroundMsg>,
    visited_entries:              usize,
    identity_entries_examined:    usize,
    aggregation_entries_examined: usize,
}

fn spawn_disk_update(
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: StdSender<String>,
    background_tx: Sender<BackgroundMsg>,
    request: DiskRefreshRequest,
) {
    let handle = handle.clone();
    let disk_limit = Arc::clone(disk_limit);
    handle.spawn(async move {
        let queue_started = Instant::now();
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(queue_started.elapsed().as_millis()),
            abs_path = %request.root.display(),
            "watcher_disk_queue_wait"
        );
        let started = Instant::now();
        if let Ok(update) = tokio::task::spawn_blocking(move || request.inputs.measure()).await {
            tracing::trace!(
                target: PERF_LOG_TARGET,
                elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
                abs_path = %request.root.display(),
                visited_entries = update.visited_entries,
                identity_entries_examined = update.identity_entries_examined,
                aggregation_entries_examined = update.aggregation_entries_examined,
                read_bytes = %update.read_bytes,
                "watcher_disk_usage"
            );
            for (path, sizes) in update.entries {
                let _ = background_tx.send(disk_refresh_message(&path, sizes));
            }
            for message in update.external_targets {
                let _ = background_tx.send(message);
            }
        }
        for label in request.labels {
            let _ = disk_done_tx.send(label);
        }
    });
}

#[cfg(test)]
#[path = "linked_target_poll_tests.rs"]
mod linked_target_tests;

#[cfg(test)]
mod disk_render_tests {
    use super::*;

    #[test]
    fn watcher_disk_refresh_updates_the_package_total_and_parts() -> anyhow::Result<()> {
        let scratch = tempfile::tempdir()?;
        let path = AbsolutePath::from(scratch.path());
        std::fs::create_dir(path.join("target"))?;
        for (target, other, labels) in [
            (4096, 74, ["4.1 KiB", "4.0 KiB", "74 B"]),
            (8192, 90, ["8.1 KiB", "8.0 KiB", "90 B"]),
        ] {
            std::fs::write(path.join("source.rs"), vec![0_u8; other])?;
            std::fs::write(path.join("target/artifact"), vec![0_u8; target])?;
            let sizes = project_disk_sizes(&path, &DiskProbeScope::ProjectTree);
            let buffer = crate::tui::test_support::draw_disk_refresh_for_test(
                &path,
                disk_refresh_message(&path, sizes),
            )?;
            let text: String = buffer
                .content
                .iter()
                .map(ratatui::buffer::Cell::symbol)
                .collect();
            for label in labels {
                assert!(text.contains(label), "missing {label}: {text}");
            }
        }
        Ok(())
    }
}
