use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use tokio::runtime::Handle;
use tokio::sync::Semaphore;

use super::ProjectEntry;
use super::WatchState;
use super::events::EventContext;
use crate::constants::DEBOUNCE_DURATION;
use crate::lint;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::RepoInfo;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;

pub(super) fn schedule_disk_refresh(
    pending_disk: &mut HashMap<String, WatchState>,
    project_label: &str,
    now: Instant,
) {
    match pending_disk
        .entry(project_label.to_string())
        .or_insert(WatchState::Idle)
    {
        state @ WatchState::Idle => {
            *state = WatchState::pending(now, false);
        },
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = now + DEBOUNCE_DURATION;
        },
        state @ WatchState::Running => *state = WatchState::RunningDirty,
        WatchState::RunningDirty => {},
    }
}

pub(super) fn handle_disk_completion(
    pending_disk: &mut HashMap<String, WatchState>,
    project_label: &str,
) {
    let now = Instant::now();
    let Some(state) = pending_disk.get_mut(project_label) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false),
        WatchState::Idle | WatchState::Pending { .. } => return,
    };
}

pub(super) fn handle_git_completion(
    pending_git: &mut HashMap<AbsolutePath, WatchState>,
    repo_root: &AbsolutePath,
) {
    let now = Instant::now();
    let Some(state) = pending_git.get_mut(repo_root) else {
        return;
    };
    *state = match *state {
        WatchState::Running => WatchState::Idle,
        WatchState::RunningDirty => WatchState::pending(now, false),
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
        || event_path == git_dir.join("index.lock")
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
    tracing::info!(
        repo_root = %repo_root.display(),
        immediate,
        cause,
        pending_git = pending_count,
        "watcher_enqueue_git_refresh"
    );
    match pending_git.entry(repo_root).or_insert(WatchState::Idle) {
        state @ WatchState::Idle => *state = WatchState::pending(now, immediate),
        WatchState::Pending {
            debounce_deadline, ..
        } => {
            *debounce_deadline = if immediate {
                now
            } else {
                now + DEBOUNCE_DURATION
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
    // Match events under the resolved git dir (handles worktrees) or
    // under repo_root/.git (handles normal repos where git_dir ==
    // repo_root/.git, but also catches events like refs/heads updates
    // that live in the common git dir rather than the worktree git dir).
    git_dir.is_some_and(|d| event_path.starts_with(d))
        || common_git_dir.is_some_and(|d| event_path.starts_with(d))
        || repo_root.is_some_and(|r| event_path.starts_with(r.join(".git")))
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
        Some(lint::CargoMetadataTriggerKind::CargoConfig)
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
    let fallback = project_root.join("target");
    let dir = resolved_target.unwrap_or(fallback.as_path());
    event_path.starts_with(dir)
}

pub(super) fn is_target_metadata_event(event_path: &Path, project_root: &Path) -> bool {
    let cargo_toml = project_root.join("Cargo.toml");
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

pub(super) fn emit_root_git_info_refresh(bg_tx: &Sender<BackgroundMsg>, entry: &ProjectEntry) {
    let started = Instant::now();
    let Some(repo) = RepoInfo::get(entry.abs_path.as_path()) else {
        return;
    };
    let checkout = CheckoutInfo::get(entry.abs_path.as_path(), repo.local_main_branch.as_deref());
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
        path = %entry.project_label,
        git_status = checkout.as_ref().map_or("unknown", |c| c.status.label()),
        "watcher_root_git_info_refresh"
    );
    let _ = bg_tx.send(BackgroundMsg::RepoInfo {
        path: entry.abs_path.clone(),
        info: repo,
    });
    if let Some(checkout) = checkout {
        let _ = bg_tx.send(BackgroundMsg::CheckoutInfo {
            path: entry.abs_path.clone(),
            info: checkout,
        });
    }
}

pub(super) fn fire_git_updates(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: &Sender<AbsolutePath>,
    bg_tx: &Sender<BackgroundMsg>,
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
        // Affected = every entry whose refresh-key matches; for the
        // common-git-dir case that's primary + all linked siblings of
        // the same repo. Each entry needs its own `CheckoutInfo`
        // probe because branch/HEAD/status differ per worktree.
        let affected: Vec<AbsolutePath> = projects
            .values()
            .filter(|entry| git_refresh_key(entry).as_ref() == Some(&refresh_key))
            .map(|entry| entry.abs_path.clone())
            .collect();
        if affected.is_empty() {
            if let Some(state) = pending_git.get_mut(&refresh_key) {
                *state = WatchState::Idle;
            }
            continue;
        }
        // Identify the primary checkout: the one whose own `.git` is
        // the common git dir (i.e., its working tree is `<git_dir>/..`).
        // Falls back to the first affected entry when no clear primary
        // is visible (e.g., entry registered without `common_git_dir`).
        let primary_path = projects
            .values()
            .find(|entry| {
                entry.git_dir.as_deref() == Some(refresh_key.as_path())
                    || entry.common_git_dir.as_deref() == Some(refresh_key.as_path())
                        && entry.abs_path.as_path().join(".git").is_dir()
            })
            .map_or_else(|| affected[0].clone(), |entry| entry.abs_path.clone());
        if let Some(state) = pending_git.get_mut(&refresh_key) {
            mark_running(state);
        }
        spawn_git_refresh(
            handle,
            git_limit,
            git_done_tx.clone(),
            bg_tx.clone(),
            refresh_key,
            primary_path,
            affected,
        );
    }
}

pub(super) fn spawn_git_refresh(
    handle: &Handle,
    git_limit: &Arc<Semaphore>,
    git_done_tx: Sender<AbsolutePath>,
    bg_tx: Sender<BackgroundMsg>,
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
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_started.elapsed().as_millis()),
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
        let local_main_branch = repo_info.as_ref().and_then(|r| r.local_main_branch.clone());
        if let Some(repo_info) = repo_info {
            let _ = bg_tx.send(BackgroundMsg::RepoInfo {
                path: primary_path.clone(),
                info: repo_info,
            });
        }

        // Probe the per-checkout half for each affected path. These are
        // cheap (no per-remote loop); each yields the worktree's own
        // branch/HEAD/status.
        for checkout_path in affected {
            let probe_path = checkout_path.clone();
            let lmb = local_main_branch.clone();
            let checkout = tokio::task::spawn_blocking(move || {
                CheckoutInfo::get(probe_path.as_path(), lmb.as_deref())
            })
            .await
            .ok()
            .flatten();
            if let Some(checkout) = checkout {
                let _ = bg_tx.send(BackgroundMsg::CheckoutInfo {
                    path: checkout_path,
                    info: checkout,
                });
            }
        }

        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            refresh_key = %refresh_key.display(),
            "watcher_git_refresh"
        );
        let _ = git_done_tx.send(refresh_key);
    });
}

pub(super) fn fire_disk_updates(
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: &Sender<String>,
    bg_tx: &Sender<BackgroundMsg>,
    projects: &HashMap<AbsolutePath, ProjectEntry>,
    pending_disk: &mut HashMap<String, WatchState>,
) {
    let now = Instant::now();
    let ready: Vec<String> = pending_disk
        .iter()
        .filter(|(_, state)| is_ready_to_launch(state, now))
        .map(|(key, _)| key.clone())
        .collect();

    for project_label in ready {
        let Some(entry) = projects.values().find(|e| e.project_label == project_label) else {
            if let Some(state) = pending_disk.get_mut(&project_label) {
                *state = WatchState::Idle;
            }
            continue;
        };
        if let Some(state) = pending_disk.get_mut(&project_label) {
            mark_running(state);
        }
        spawn_disk_update(
            handle,
            disk_limit,
            disk_done_tx.clone(),
            bg_tx.clone(),
            project_label.clone(),
            entry.abs_path.clone(),
        );
    }
}

pub(super) fn spawn_disk_update(
    handle: &Handle,
    disk_limit: &Arc<Semaphore>,
    disk_done_tx: Sender<String>,
    bg_tx: Sender<BackgroundMsg>,
    project_label: String,
    abs_path: AbsolutePath,
) {
    let handle = handle.clone();
    let disk_limit = Arc::clone(disk_limit);
    handle.spawn(async move {
        let queue_started = Instant::now();
        let Ok(_permit) = disk_limit.acquire_owned().await else {
            return;
        };
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(queue_started.elapsed().as_millis()),
            path = %project_label,
            abs_path = %abs_path.display(),
            "watcher_disk_queue_wait"
        );

        let started = Instant::now();
        let abs_for_msg = abs_path.clone();
        let bytes = tokio::task::spawn_blocking(move || scan::dir_size(&abs_path))
            .await
            .ok()
            .unwrap_or(0);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %project_label,
            bytes,
            "watcher_disk_usage"
        );
        let _ = bg_tx.send(BackgroundMsg::DiskUsage {
            path: abs_for_msg,
            bytes,
        });
        let _ = disk_done_tx.send(project_label);
    });
}
