//! Per-discovery enrichment funnel.
//!
//! Every site that *discovers* a new project-like path at runtime — the
//! filesystem watcher, the per-submodule loop in `scan::fetch_project_details`
//! — creates an [`enrichment_job`] for the entry. The funnel
//! emits git info, disk usage, language stats, and (when the entry's
//! `crates_io_name` returns `Some`) crates.io metadata.
//!
//! Initial bulk scanning keeps its own paths (`spawn_initial_language_stats`
//! for tree-batched language scans, and the post-scan schedulers in
//! `tui::app::async_tasks`) — those have aggregate context the funnel
//! cannot replicate per-entry without regressing performance.
//!
//! `src/tui/app/async_tasks/dispatch.rs` routes `BackgroundMsg::CheckoutInfo` and
//! `BackgroundMsg::RepoInfo` to `handle_checkout_info` and `handle_repo_info` in
//! `src/tui/app/async_tasks/repo_handlers.rs`. These handlers start CI and GitHub repository
//! metadata requests. If a parent and submodule share a repository, the `OwnerRepo` key prevents
//! duplicate requests while a request is queued or running.

use crate::channel::Sender;
use crate::project::AbsolutePath;
use crate::project::ProjectFields;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FetchContext;

/// Capture source caches before enrichment can wait on other work.
pub(crate) fn enrichment_job(
    entry: &dyn ProjectFields,
    sender: &Sender<BackgroundMsg>,
    ctx: &FetchContext,
) -> impl FnOnce() + Send + use<> {
    let path: AbsolutePath = entry.path().clone();
    let language_job =
        scan::language_enrichment_job(path.clone(), sender.clone(), &ctx.source_measurements);
    let name = entry.crates_io_name().map(str::to_string);
    let sender = sender.clone();
    let ctx = ctx.clone();
    move || {
        scan::emit_git_info(&sender, &path);
        emit_disk(&path, &sender);
        language_job();
        spawn_test_count_scan(path.clone(), sender.clone());
        if let Some(name) = name {
            emit_crates_io(&name, &path, &sender, &ctx);
        }
    }
}

fn emit_disk(path: &AbsolutePath, sender: &Sender<BackgroundMsg>) {
    let bytes = scan::dir_size(path.as_path());
    let _ = sender.send(BackgroundMsg::DiskUsage {
        path: path.clone(),
        bytes,
    });
}

fn spawn_test_count_scan(path: AbsolutePath, sender: Sender<BackgroundMsg>) {
    rayon::spawn(move || {
        let counts = scan::collect_test_counts_single(path.as_path());
        let _ = sender.send(BackgroundMsg::TestCountsBatch {
            entries: vec![(path, counts)],
        });
    });
}

fn emit_crates_io(
    name: &str,
    path: &AbsolutePath,
    sender: &Sender<BackgroundMsg>,
    ctx: &FetchContext,
) {
    let _ = sender.send(BackgroundMsg::CratesIoFetchQueued {
        name: name.to_string(),
    });
    let (info, signal) = ctx.client.fetch_crates_io_info(name);
    scan::emit_service_signal(sender, signal);
    if let Some(info) = info {
        let _ = sender.send(BackgroundMsg::CratesIoVersion {
            path:    path.clone(),
            release: info,
        });
    }
    let _ = sender.send(BackgroundMsg::CratesIoFetchComplete {
        name: name.to_string(),
    });
}
