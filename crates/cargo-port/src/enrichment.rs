//! Per-discovery enrichment funnel.
//!
//! Every site that *discovers* a new project-like path at runtime — the
//! filesystem watcher, the per-submodule loop in `scan::fetch_project_details`
//! — calls [`enrich`] with the entry as `&dyn ProjectFields`. The funnel
//! emits the same set of `BackgroundMsg::*` values today's hand-wired
//! call sites emit, and consults the entry's capability methods to decide
//! which signals to fire.
//!
//! Initial bulk scanning keeps its own paths (`spawn_initial_language_stats`
//! for tree-batched language scans, and the post-scan schedulers in
//! `tui::app::async_tasks`) — those have aggregate context the funnel
//! cannot replicate per-entry without regressing performance.
//!
//! The capability methods `ci_fetch`, `first_commit`, and `repo_metadata`
//! are declared on `ProjectFields` but not yet honoured here. Today CI and
//! GitHub repo metadata cascade off the central `BackgroundMsg::GitInfo`
//! handler, and first-commit is scheduled batched-per-repo post-scan.
//! Honouring the `Skip` variants of those capabilities (e.g. for
//! submodules whose upstream we do not own) requires gating the cascade
//! and lives in a follow-up.

use std::sync::mpsc::Sender;

use crate::project::AbsolutePath;
use crate::project::GitInfo;
use crate::project::LanguageScan;
use crate::project::ProjectFields;
use crate::scan::BackgroundMsg;
use crate::scan::FetchContext;
use crate::scan::collect_language_stats_single;
use crate::scan::dir_size;
use crate::scan::emit_service_signal;

/// Run enrichment for a single discovered entry.
///
/// Emits git info, disk usage, language stats, and crates.io metadata
/// according to the entry's capability methods.
pub(crate) fn enrich(entry: &dyn ProjectFields, tx: &Sender<BackgroundMsg>, ctx: &FetchContext) {
    let path: AbsolutePath = entry.path().clone();

    emit_git(&path, tx);
    emit_disk(&path, tx);

    match entry.language_scan() {
        LanguageScan::Run => spawn_language_scan(path.clone(), tx.clone()),
        LanguageScan::Skip => {},
    }

    if let Some(name) = entry.crates_io_name() {
        emit_crates_io(name, &path, tx, ctx);
    }
}

fn emit_git(path: &AbsolutePath, tx: &Sender<BackgroundMsg>) {
    let Some(info) = GitInfo::detect_fast(path.as_path()) else {
        return;
    };
    let _ = tx.send(BackgroundMsg::GitInfo {
        path: path.clone(),
        info,
    });
}

fn emit_disk(path: &AbsolutePath, tx: &Sender<BackgroundMsg>) {
    let bytes = dir_size(path.as_path());
    let _ = tx.send(BackgroundMsg::DiskUsage {
        path: path.clone(),
        bytes,
    });
}

pub(crate) fn spawn_language_scan(path: AbsolutePath, tx: Sender<BackgroundMsg>) {
    rayon::spawn(move || {
        let stats = collect_language_stats_single(path.as_path());
        if !stats.entries.is_empty() {
            let _ = tx.send(BackgroundMsg::LanguageStatsBatch {
                entries: vec![(path, stats)],
            });
        }
    });
}

fn emit_crates_io(name: &str, path: &AbsolutePath, tx: &Sender<BackgroundMsg>, ctx: &FetchContext) {
    let (info, signal) = ctx.client.fetch_crates_io_info(name);
    emit_service_signal(tx, signal);
    if let Some(info) = info {
        let _ = tx.send(BackgroundMsg::CratesIoVersion {
            path:      path.clone(),
            version:   info.version,
            downloads: info.downloads,
        });
    }
}
