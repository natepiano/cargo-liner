use std::collections::HashMap;
use std::time::Instant;

use crate::project;
use crate::project::AbsolutePath;
use crate::project::GitRepoPresence;
use crate::project::ProjectFields;
use crate::project::RootItem;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::FetchContext;
use crate::scan::ProjectDetailRequest;
use crate::tui::app::App;
use crate::watcher::WatchRequest;
use crate::watcher::WatcherMsg;

impl App {
    /// Register file-system watchers for every item in the tree after a
    /// single-pass scan delivers the complete tree.
    pub(super) fn register_background_services_for_tree(&self) {
        let started = Instant::now();
        let mut count = 0usize;
        self.projects().for_each_leaf(|item| {
            self.register_item_background_services(item);
            count += 1;
        });
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            count,
            "register_background_services_for_tree"
        );
    }
    pub(super) fn register_item_background_services(&self, item: &RootItem) {
        let started = Instant::now();
        let abs_path = AbsolutePath::from(item.path().to_path_buf());
        let repo_root = project::git_repo_root(&abs_path);
        let has_repo_root = repo_root.is_some();
        let _ = self
            .background
            .send_watcher(WatcherMsg::Register(WatchRequest {
                project_label: abs_path.to_string_lossy().to_string(),
                abs_path: abs_path.clone(),
                repo_root,
            }));
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
            path = %item.display_path(),
            has_repo_root,
            "app_register_project_background_services"
        );
    }
    pub(super) fn schedule_startup_project_details(&self) {
        let tx = self.background.bg_sender();
        let fetch_context = std::sync::Arc::new(FetchContext {
            client: self.net.http_client(),
        });
        self.projects().for_each_leaf(|item| {
            let abs_path = item.path().to_path_buf();
            let display_path = item.display_path().into_string();
            let project_name = item
                .is_rust()
                .then(|| item.name().map(str::to_string))
                .flatten()
                .filter(|_| {
                    self.projects()
                        .rust_info_at_path(item.path())
                        .is_some_and(|r| r.cargo().publishable())
                });
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
                    project_name: project_name.as_deref(),
                    repo_presence,
                };
                scan::fetch_project_details(&request);
            });
        });
        self.schedule_member_crates_io_fetches();
    }
    /// Fire crates.io fetches for publishable workspace members and vendored
    /// crates.
    ///
    /// `schedule_startup_project_details` only iterates leaf-level projects
    /// (workspace roots), not individual workspace members or vendored
    /// crates. This method supplements it by iterating both and fetching
    /// crates.io data for each publishable one.
    pub(super) fn schedule_member_crates_io_fetches(&self) {
        let tx = self.background.bg_sender();
        let client = self.net.http_client();
        let mut targets: Vec<(AbsolutePath, String)> = Vec::new();
        for entry in self.projects() {
            collect_publishable_children(&entry.item, &mut targets);
        }
        if targets.is_empty() {
            return;
        }
        rayon::spawn(move || {
            for (path, name) in targets {
                let (info, signal) = client.fetch_crates_io_info(&name);
                scan::emit_service_signal(&tx, signal);
                if let Some(info) = info {
                    let _ = tx.send(BackgroundMsg::CratesIoVersion {
                        path,
                        version: info.version,
                        downloads: info.downloads,
                    });
                }
            }
        });
    }
    pub(super) fn schedule_git_first_commit_refreshes(&self) {
        let tx = self.background.bg_sender();
        let mut projects_by_repo: HashMap<AbsolutePath, Vec<AbsolutePath>> = HashMap::new();
        self.projects().for_each_leaf_path(|path, _| {
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
                tracing::info!(
                    elapsed_ms = crate::perf_log::ms(started.elapsed().as_millis()),
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

/// Collect publishable workspace members and vendored crates into a flat
/// `(path, crates.io name)` list for crates.io scheduling.
fn collect_publishable_children(item: &RootItem, out: &mut Vec<(AbsolutePath, String)>) {
    use crate::project::Package;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::project::WorktreeGroup;

    pub(super) fn push_workspace(ws: &Workspace, out: &mut Vec<(AbsolutePath, String)>) {
        for group in ws.groups() {
            for member in group.members() {
                if let Some(name) = member.crates_io_name() {
                    out.push((member.path().clone(), name.to_string()));
                }
            }
        }
        for vendored in ws.vendored() {
            if let Some(name) = vendored.crates_io_name() {
                out.push((vendored.path().clone(), name.to_string()));
            }
        }
    }
    pub(super) fn push_package_vendored(pkg: &Package, out: &mut Vec<(AbsolutePath, String)>) {
        for vendored in pkg.vendored() {
            if let Some(name) = vendored.crates_io_name() {
                out.push((vendored.path().clone(), name.to_string()));
            }
        }
    }

    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => push_workspace(ws, out),
        RootItem::Rust(RustProject::Package(pkg)) => push_package_vendored(pkg, out),
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            push_workspace(primary, out);
            for ws in linked {
                push_workspace(ws, out);
            }
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            push_package_vendored(primary, out);
            for pkg in linked {
                push_package_vendored(pkg, out);
            }
        },
        RootItem::NonRust(_) => {},
    }
}
