use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;

use crate::project;
use crate::project::AbsolutePath;
use crate::project::GitRepoPresence;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;
use crate::tui::app::App;

/// Number of parallel crates.io fetch workers. Each worker runs its share
/// of the plan sequentially, so peak request concurrency against crates.io
/// is capped at this count while the fetch wall-clock drops to roughly
/// 1/N of the serial chain. A tripped limiter surfaces as a 429 →
/// `ServiceSignal::RateLimited` and the recovery path refetches the
/// misses.
const CRATES_IO_FETCH_WORKERS: usize = 10;

impl App {
    /// Register file-system watchers for every item in the tree after a
    /// single-pass scan delivers the complete tree.
    pub(super) fn register_background_services_for_tree(&self) {
        let started = Instant::now();
        let mut count = 0usize;
        self.project_list.for_each_leaf(|item| {
            self.background.register_item_background_services(item);
            count += 1;
        });
        tracing::trace!(
            target: tui_pane::PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            count,
            "register_background_services_for_tree"
        );
    }
    /// Dispatch the startup project-detail workers and the crates.io fetch
    /// plan that was already installed into the startup ledger.
    pub(super) fn schedule_startup_project_details(&self, crates_io_plan: CratesIoFetchPlan) {
        let tx = self.background.background_sender();
        let fetch_context = std::sync::Arc::new(FetchContext {
            client: self.net.http_client(),
        });
        self.project_list.for_each_leaf(|item| {
            let abs_path = item.path().to_path_buf();
            let display_path = item.display_path().into_string();
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let tx = tx.clone();
            let fetch_context = std::sync::Arc::clone(&fetch_context);
            rayon::spawn(move || {
                let request = ProjectDetailRequest {
                    tx: &tx,
                    fetch_context: fetch_context.as_ref(),
                    _project_path: display_path.as_str(),
                    abs_path: &abs_path,
                    // Startup crates.io fetches flow through the fetch
                    // plan below, not the per-leaf detail task; only the
                    // watcher probe path passes a name here.
                    project_name: None,
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
        });
        self.dispatch_crates_io_fetches(crates_io_plan);
    }
    /// Walk every project root and collect the crates.io fetch plan:
    /// each publishable crate name mapped to every project path that
    /// displays its version. Root packages, workspace members, and
    /// vendored crates all land in one plan; a worktree copy of a
    /// workspace contributes the same names under different paths — one
    /// query each, fanned out to every path.
    pub(super) fn collect_crates_io_fetch_plan(&self) -> CratesIoFetchPlan {
        let mut plan = CratesIoFetchPlan::default();
        for entry in &self.project_list {
            collect_plan_children(&entry.item, &mut plan);
        }
        plan
    }
    /// Re-fire crates.io fetches for publishable projects whose
    /// version data didn't land during a prior outage. Called from the
    /// service-recovery path so the warning placeholder rows fill in
    /// once the network is back.
    pub(super) fn refetch_missing_crates_io_targets(&self) {
        let mut plan = self.collect_crates_io_fetch_plan();
        plan.retain_paths(|path| !self.has_crates_io_version(path));
        self.dispatch_crates_io_fetches(plan);
    }
    /// Whether `path` has a cached crates.io version already. Looks
    /// the project up via either the rust-info or vendored accessor;
    /// `None` for either resolution counts as "no version yet."
    fn has_crates_io_version(&self, path: &AbsolutePath) -> bool {
        if let Some(rust) = self.project_list.rust_info_at_path(path.as_path()) {
            return rust.crates_version().is_some();
        }
        self.project_list
            .vendored_at_path(path.as_path())
            .is_some_and(|v| v.crates_version().is_some())
    }
    /// Fan the plan out to [`CRATES_IO_FETCH_WORKERS`] rayon workers, each
    /// driving its share through the crates.io fetch lifecycle — queued
    /// toast, one network call per name, a version write to every path
    /// bearing that name, complete toast. Every name's `Queued` precedes
    /// its `Complete` within its worker, so the startup row's
    /// registration ordering holds regardless of cross-worker
    /// interleaving. An empty plan spawns nothing.
    fn dispatch_crates_io_fetches(&self, plan: CratesIoFetchPlan) {
        for bucket in plan.into_worker_buckets(CRATES_IO_FETCH_WORKERS) {
            let tx = self.background.background_sender();
            let client = self.net.http_client();
            rayon::spawn(move || {
                for (name, paths) in bucket {
                    let _ = tx.send(BackgroundMsg::CratesIoFetchQueued { name: name.clone() });
                    let (info, signal) = client.fetch_crates_io_info(&name);
                    scan::emit_service_signal(&tx, signal);
                    if let Some(info) = info {
                        for path in paths {
                            let _ = tx.send(BackgroundMsg::CratesIoVersion {
                                path,
                                version: info.version.clone(),
                                prerelease: info.prerelease.clone(),
                                downloads: info.downloads,
                            });
                        }
                    }
                    let _ = tx.send(BackgroundMsg::CratesIoFetchComplete { name });
                }
            });
        }
    }
    pub(super) fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.background.background_sender();
        let mut projects_by_repo: HashMap<AbsolutePath, Vec<AbsolutePath>> = HashMap::new();
        self.project_list.for_each_leaf_path(|path, _| {
            let abs_path = AbsolutePath::from(path);
            let Some(repo_root) = project::git_repo_root(&abs_path) else {
                return;
            };
            projects_by_repo
                .entry(repo_root)
                .or_default()
                .push(abs_path);
        });
        std::thread::spawn(move || {
            for (repo_root, paths) in projects_by_repo {
                let started = Instant::now();
                let first_commit = project::get_first_commit(&repo_root);
                tracing::trace!(
                    target: tui_pane::PERF_LOG_TARGET,
                    elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
                    repo_root = %repo_root.display(),
                    rows = paths.len(),
                    found = first_commit.is_some(),
                    "git_first_commit_fetch"
                );
                for path in paths {
                    let _ = tx.send(BackgroundMsg::GitFirstCommit {
                        path,
                        first_commit: first_commit.clone(),
                    });
                }
            }
        });
    }
}

/// The startup crates.io fetch plan: every publishable crate name mapped
/// to the project paths that display its version. The same value seeds
/// the startup panel's crates.io denominator and drives the dispatcher,
/// so the row reads done only when every query the dispatcher will issue
/// has completed. Worktree copies of a workspace land as extra paths
/// under one name — one query, fanned out to every path.
#[derive(Default)]
pub(super) struct CratesIoFetchPlan {
    by_name: BTreeMap<String, Vec<AbsolutePath>>,
}

impl CratesIoFetchPlan {
    fn insert(&mut self, name: &str, path: &AbsolutePath) {
        let paths = self.by_name.entry(name.to_string()).or_default();
        if !paths.contains(path) {
            paths.push(path.clone());
        }
    }

    /// The deduplicated name set — the startup row's denominator.
    pub(super) fn names(&self) -> HashSet<String> { self.by_name.keys().cloned().collect() }

    /// Drop every path failing `keep`, then every name left with no
    /// paths. The recovery refetch uses this to re-dispatch only the
    /// projects whose version never landed.
    fn retain_paths(&mut self, mut keep: impl FnMut(&AbsolutePath) -> bool) {
        self.by_name.retain(|_, paths| {
            paths.retain(&mut keep);
            !paths.is_empty()
        });
    }

    /// Split the plan into at most `workers` non-empty round-robin
    /// buckets, one per dispatch worker. Round-robin keeps each worker's
    /// share even regardless of where slow names cluster alphabetically.
    fn into_worker_buckets(self, workers: usize) -> Vec<Vec<(String, Vec<AbsolutePath>)>> {
        let bucket_count = workers.max(1);
        let mut buckets: Vec<Vec<(String, Vec<AbsolutePath>)>> = vec![Vec::new(); bucket_count];
        for (index, entry) in self.by_name.into_iter().enumerate() {
            buckets[index % bucket_count].push(entry);
        }
        buckets.retain(|bucket| !bucket.is_empty());
        buckets
    }
}

/// Collect one root item's publishable crates — the root package itself,
/// workspace members, and vendored crates — into the fetch plan.
fn collect_plan_children(item: &RootItem, plan: &mut CratesIoFetchPlan) {
    fn push_entry(entry: &dyn ProjectFields, plan: &mut CratesIoFetchPlan) {
        if let Some(name) = entry.crates_io_name() {
            plan.insert(name, entry.path());
        }
    }
    fn push_workspace(ws: &Workspace, plan: &mut CratesIoFetchPlan) {
        push_entry(ws, plan);
        for group in ws.groups() {
            for member in group.members() {
                push_package(member, plan);
            }
        }
        for vendored in ws.vendored() {
            push_entry(vendored, plan);
        }
    }
    fn push_package(pkg: &Package, plan: &mut CratesIoFetchPlan) {
        push_entry(pkg, plan);
        for vendored in pkg.vendored() {
            push_entry(vendored, plan);
        }
    }

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => push_workspace(ws, plan),
        RootItem::Rust(RustProject::Package(pkg)) => push_package(pkg, plan),
        RootItem::Worktrees(group) => {
            for entry in group.iter_entries() {
                match entry {
                    RustProject::Workspace(ws) => push_workspace(ws, plan),
                    RustProject::Package(pkg) => push_package(pkg, plan),
                }
            }
        },
        RootItem::NonRust(_) => {},
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
mod tests {
    use super::*;

    fn abs(raw: &str) -> AbsolutePath { AbsolutePath::from(raw) }

    #[test]
    fn plan_fans_duplicate_names_out_to_distinct_paths() {
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("serde", &abs("/a/serde"));
        plan.insert("serde", &abs("/b/serde"));
        plan.insert("serde", &abs("/a/serde"));
        assert_eq!(plan.names().len(), 1, "one name means one query");
        assert_eq!(
            plan.by_name["serde"].len(),
            2,
            "both paths fan out; the repeated (name, path) pair dedups"
        );
    }

    #[test]
    fn retain_paths_drops_emptied_names() {
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("serde", &abs("/a/serde"));
        plan.insert("tokio", &abs("/a/tokio"));
        plan.insert("tokio", &abs("/b/tokio"));
        plan.retain_paths(|path| path.as_path().starts_with("/b"));
        assert!(
            !plan.names().contains("serde"),
            "a name with no surviving paths leaves the plan"
        );
        assert_eq!(
            plan.by_name["tokio"],
            vec![abs("/b/tokio")],
            "surviving paths stay under their name"
        );
    }

    #[test]
    fn worker_buckets_round_robin_and_drop_empties() {
        let mut plan = CratesIoFetchPlan::default();
        for name in ["a", "b", "c", "d", "e"] {
            plan.insert(name, &abs(&format!("/x/{name}")));
        }
        let buckets = plan.into_worker_buckets(2);
        assert_eq!(buckets.len(), 2);
        let names: Vec<Vec<&str>> = buckets
            .iter()
            .map(|bucket| bucket.iter().map(|(name, _)| name.as_str()).collect())
            .collect();
        assert_eq!(
            names,
            vec![vec!["a", "c", "e"], vec!["b", "d"]],
            "names alternate across buckets in order"
        );

        let mut small = CratesIoFetchPlan::default();
        small.insert("only", &abs("/x/only"));
        assert_eq!(
            small.into_worker_buckets(4).len(),
            1,
            "empty buckets are dropped, not spawned"
        );
        assert!(
            CratesIoFetchPlan::default()
                .into_worker_buckets(4)
                .is_empty(),
            "an empty plan yields no buckets"
        );
    }
}
