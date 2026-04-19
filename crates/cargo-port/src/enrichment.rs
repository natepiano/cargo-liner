//! Per-discovery enrichment funnel.
//!
//! Every site that *discovers* a new project-like path at runtime — the
//! filesystem watcher, the per-submodule loop in `scan::fetch_project_details`
//! — calls [`enrich`] with the entry as `&dyn ProjectFields`. The funnel
//! emits git info, disk usage, language stats, and (when the entry's
//! `crates_io_name` returns `Some`) crates.io metadata.
//!
//! Initial bulk scanning keeps its own paths (`spawn_initial_language_stats`
//! for tree-batched language scans, and the post-scan schedulers in
//! `tui::app::async_tasks`) — those have aggregate context the funnel
//! cannot replicate per-entry without regressing performance.
//!
//! CI runs and GitHub repo metadata cascade off the central
//! `BackgroundMsg::GitInfo` handler in `async_tasks.rs`. That handler
//! consults `ProjectList::is_submodule_path` to suppress the cascade for
//! submodule paths, since CI/metadata is shown on the parent project.

use std::sync::mpsc::Sender;

use crate::project::AbsolutePath;
use crate::project::DetectedGit;
use crate::project::ProjectFields;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FetchContext;

/// Run enrichment for a single discovered entry.
///
/// Emits git info, disk usage, language stats, and crates.io metadata
/// according to the entry's capability methods.
pub(crate) fn enrich(entry: &dyn ProjectFields, tx: &Sender<BackgroundMsg>, ctx: &FetchContext) {
    let path: AbsolutePath = entry.path().clone();

    emit_git(&path, tx);
    emit_disk(&path, tx);
    spawn_language_scan(path.clone(), tx.clone());
    if let Some(name) = entry.crates_io_name() {
        emit_crates_io(name, &path, tx, ctx);
    }
}

fn emit_git(path: &AbsolutePath, tx: &Sender<BackgroundMsg>) {
    let Some(info) = DetectedGit::detect_fast(path.as_path()) else {
        return;
    };
    let _ = tx.send(BackgroundMsg::GitInfo {
        path: path.clone(),
        info,
    });
}

fn emit_disk(path: &AbsolutePath, tx: &Sender<BackgroundMsg>) {
    let bytes = scan::dir_size(path.as_path());
    let _ = tx.send(BackgroundMsg::DiskUsage {
        path: path.clone(),
        bytes,
    });
}

pub(crate) fn spawn_language_scan(path: AbsolutePath, tx: Sender<BackgroundMsg>) {
    rayon::spawn(move || {
        let stats = scan::collect_language_stats_single(path.as_path());
        if !stats.entries.is_empty() {
            let _ = tx.send(BackgroundMsg::LanguageStatsBatch {
                entries: vec![(path, stats)],
            });
        }
    });
}

fn emit_crates_io(name: &str, path: &AbsolutePath, tx: &Sender<BackgroundMsg>, ctx: &FetchContext) {
    let (info, signal) = ctx.client.fetch_crates_io_info(name);
    scan::emit_service_signal(tx, signal);
    if let Some(info) = info {
        let _ = tx.send(BackgroundMsg::CratesIoVersion {
            path:      path.clone(),
            version:   info.version,
            downloads: info.downloads,
        });
    }
}
