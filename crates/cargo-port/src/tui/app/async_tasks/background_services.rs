use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::btree_map::Entry;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use tui_pane::PERF_LOG_TARGET;

use super::constants::CRATES_IO_FETCH_WORKERS;
use crate::config::CratesIoReleaseGroupConfig;
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
use crate::tui::startup_services::StartupEffect;

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
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
            count,
            "register_background_services_for_tree"
        );
    }
    /// Dispatch the startup project-detail workers and the crates.io fetch
    /// plan that was already installed into the startup ledger.
    pub(super) fn schedule_startup_project_details(&self, crates_io_plan: CratesIoFetchPlan) {
        let effect = self.startup_services.startup_project_details_effect();
        self.startup_services.record_startup_project_details(effect);
        if effect == StartupEffect::Suppressed {
            return;
        }

        let sender = self.background.background_sender();
        let fetch_context = std::sync::Arc::new(FetchContext {
            client: self.net.http_client(),
        });
        self.project_list.for_each_leaf(|item| {
            let abs_path = item.path().to_path_buf();
            let repo_presence = if project::git_repo_root(&abs_path).is_some() {
                GitRepoPresence::InRepo
            } else {
                GitRepoPresence::OutsideRepo
            };
            let sender = sender.clone();
            let fetch_context = std::sync::Arc::clone(&fetch_context);
            rayon::spawn(move || {
                let request = ProjectDetailRequest {
                    sender: &sender,
                    fetch_context: fetch_context.as_ref(),
                    abs_path: &abs_path,
                    // Startup crates.io fetches flow through the fetch
                    // plan below, not the per-leaf detail task; only the
                    // watcher probe path passes a name here.
                    name: None,
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
        let release_groups = &self.config.current().crates_io.release_groups;
        for entry in &self.project_list {
            collect_plan_children(&entry.root_item, &mut plan, release_groups);
        }
        plan
    }
    /// Re-fire crates.io fetches for publishable projects whose
    /// version data didn't land during a prior outage. Called from the
    /// service-recovery path so the warning placeholder rows fill in
    /// once the network is back.
    pub(super) fn refetch_missing_crates_io_targets(&self) {
        let mut plan = self.collect_crates_io_fetch_plan();
        plan.retain_paths(|path| self.displayed_crates_io_version(path).is_none());
        self.dispatch_crates_io_fetches(plan);
    }
    /// The crates.io version currently on display for `path`, if one has
    /// landed. Looks the project up via either the rust-info or vendored
    /// accessor; `None` for either resolution means "no version yet."
    pub(super) fn displayed_crates_io_version(&self, path: &AbsolutePath) -> Option<&str> {
        if let Some(rust) = self.project_list.rust_info_at_path(path.as_path()) {
            return rust.crates_version();
        }
        self.project_list
            .vendored_at_path(path.as_path())
            .and_then(|vendored| vendored.crates_version())
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
            let sender = self.background.background_sender();
            let client = self.net.http_client();
            rayon::spawn(move || {
                for (name, paths) in bucket {
                    let _ = sender.send(BackgroundMsg::CratesIoFetchQueued { name: name.clone() });
                    let (info, signal) = client.fetch_crates_io_info(&name);
                    scan::emit_service_signal(&sender, signal);
                    if let Some(info) = info {
                        for path in paths {
                            let _ = sender.send(BackgroundMsg::CratesIoVersion {
                                path,
                                version: info.version.clone(),
                                prerelease: info.prerelease.clone(),
                                downloads: info.downloads,
                            });
                        }
                    }
                    let _ = sender.send(BackgroundMsg::CratesIoFetchComplete { name });
                }
            });
        }
    }
    pub(super) fn schedule_git_first_commit_refreshes(&self) {
        let effect = self.startup_services.startup_git_first_commit_effect();
        self.startup_services
            .record_startup_git_first_commit(effect);
        if effect == StartupEffect::Suppressed {
            return;
        }

        let sender = self.background.background_sender();
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
                    target: PERF_LOG_TARGET,
                    elapsed_ms = tui_pane::perf_log_ms(started.elapsed().as_millis()),
                    repo_root = %repo_root.display(),
                    rows = paths.len(),
                    found = first_commit.is_some(),
                    "git_first_commit_fetch"
                );
                for path in paths {
                    let _ = sender.send(BackgroundMsg::GitFirstCommit {
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
    by_name: BTreeMap<String, CratesIoFetchTarget>,
}

struct CratesIoFetchTarget {
    paths:        Vec<AbsolutePath>,
    notification: ReleaseNotification,
}

pub(super) struct CratesIoRefreshTarget {
    pub(super) name:         String,
    pub(super) paths:        Vec<AbsolutePath>,
    pub(super) notification: ReleaseNotification,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum ReleaseNotification {
    Crate,
    Group { label: String },
    Suppress,
}

impl ReleaseNotification {
    fn merged(self, incoming: Self) -> Self {
        match (self, incoming) {
            (group @ Self::Group { .. }, _) | (_, group @ Self::Group { .. }) => group,
            (Self::Suppress, _) | (_, Self::Suppress) => Self::Suppress,
            (Self::Crate, Self::Crate) => Self::Crate,
        }
    }

    pub(super) fn into_name(self, crate_name: String) -> Option<String> {
        match self {
            Self::Crate => Some(crate_name),
            Self::Group { label } => Some(label),
            Self::Suppress => None,
        }
    }
}

impl CratesIoFetchPlan {
    fn insert(&mut self, name: &str, path: &AbsolutePath, notification: ReleaseNotification) {
        match self.by_name.entry(name.to_string()) {
            Entry::Vacant(entry) => {
                entry.insert(CratesIoFetchTarget {
                    paths: vec![path.clone()],
                    notification,
                });
            },
            Entry::Occupied(mut entry) => {
                let target = entry.get_mut();
                if !target.paths.contains(path) {
                    target.paths.push(path.clone());
                }
                target.notification = target.notification.clone().merged(notification);
            },
        }
    }

    /// The deduplicated name set — the startup row's denominator.
    pub(super) fn names(&self) -> HashSet<String> { self.by_name.keys().cloned().collect() }

    /// The crates.io name published from `path`. Callers that already hold
    /// a path and need the name to query for it must resolve it here, so
    /// the pair always comes from one project — a version fetched under
    /// one name is written back to that name's paths and no others.
    pub(super) fn name_for_path(&self, path: &Path) -> Option<&str> {
        self.by_name
            .iter()
            .find(|(_, target)| {
                target
                    .paths
                    .iter()
                    .any(|candidate| candidate.as_path() == path)
            })
            .map(|(name, _)| name.as_str())
    }

    /// Drop every path failing `keep`, then every name left with no
    /// paths. The recovery refetch uses this to re-dispatch only the
    /// projects whose version never landed.
    fn retain_paths(&mut self, mut keep: impl FnMut(&AbsolutePath) -> bool) {
        self.by_name.retain(|_, target| {
            target.paths.retain(&mut keep);
            !target.paths.is_empty()
        });
    }

    /// Remove and return the name whose last crates.io query is oldest,
    /// provided that query is at least `min_age` old. A name missing from
    /// `checked_at` has never been queried and sorts oldest. `None` means
    /// the plan is empty or every name was queried more recently than
    /// `min_age` — the steady state once the whole set has been covered.
    pub(super) fn take_stalest(
        mut self,
        checked_at: &HashMap<String, Instant>,
        now: Instant,
        min_age: Duration,
    ) -> Option<CratesIoRefreshTarget> {
        let name = self
            .by_name
            .keys()
            .max_by_key(|name| query_age(checked_at, name, now))?
            .clone();
        if query_age(checked_at, &name, now) < min_age {
            return None;
        }
        self.by_name
            .remove(&name)
            .map(|target| CratesIoRefreshTarget {
                name,
                paths: target.paths,
                notification: target.notification,
            })
    }

    /// Split the plan into at most `workers` non-empty round-robin
    /// buckets, one per dispatch worker. Round-robin keeps each worker's
    /// share even regardless of where slow names cluster alphabetically.
    fn into_worker_buckets(self, workers: usize) -> Vec<Vec<(String, Vec<AbsolutePath>)>> {
        let bucket_count = workers.max(1);
        let mut buckets: Vec<Vec<(String, Vec<AbsolutePath>)>> = vec![Vec::new(); bucket_count];
        for (index, (name, target)) in self.by_name.into_iter().enumerate() {
            buckets[index % bucket_count].push((name, target.paths));
        }
        buckets.retain(|bucket| !bucket.is_empty());
        buckets
    }
}

/// How long ago `name` was last queried on crates.io. A name that has
/// never been queried reports [`Duration::MAX`] so it outranks every
/// stamped entry.
fn query_age(checked_at: &HashMap<String, Instant>, name: &str, now: Instant) -> Duration {
    checked_at
        .get(name)
        .map_or(Duration::MAX, |at| now.saturating_duration_since(*at))
}

/// Collect one root item's publishable crates — the root package itself,
/// workspace members, and vendored crates — into the fetch plan.
fn collect_plan_children(
    item: &RootItem,
    plan: &mut CratesIoFetchPlan,
    release_groups: &[CratesIoReleaseGroupConfig],
) {
    fn notification_for_entry(
        name: &str,
        workspace_group: Option<&CratesIoReleaseGroupConfig>,
        release_groups: &[CratesIoReleaseGroupConfig],
    ) -> ReleaseNotification {
        if let Some(release_group) = workspace_group {
            return if name == release_group.representative {
                ReleaseNotification::Group {
                    label: release_group.toast_label().to_string(),
                }
            } else {
                ReleaseNotification::Suppress
            };
        }

        for release_group in release_groups {
            if release_group.members.iter().any(|member| member == name) {
                return ReleaseNotification::Suppress;
            }
            if !release_group.members.is_empty() && name == release_group.representative {
                return ReleaseNotification::Group {
                    label: release_group.toast_label().to_string(),
                };
            }
        }

        ReleaseNotification::Crate
    }

    fn push_entry(
        entry: &dyn ProjectFields,
        plan: &mut CratesIoFetchPlan,
        workspace_group: Option<&CratesIoReleaseGroupConfig>,
        release_groups: &[CratesIoReleaseGroupConfig],
    ) {
        if let Some(name) = entry.crates_io_name() {
            plan.insert(
                name,
                entry.path(),
                notification_for_entry(name, workspace_group, release_groups),
            );
        }
    }

    fn push_workspace(
        workspace: &Workspace,
        plan: &mut CratesIoFetchPlan,
        release_groups: &[CratesIoReleaseGroupConfig],
    ) {
        let workspace_group = workspace.crates_io_name().and_then(|name| {
            release_groups.iter().find(|release_group| {
                release_group.includes_workspace_members() && release_group.representative == name
            })
        });
        push_entry(workspace, plan, workspace_group, release_groups);
        for group in workspace.groups() {
            for member in group.members() {
                push_package(member, plan, workspace_group, release_groups);
            }
        }
        for vendored in workspace.vendored() {
            push_entry(vendored, plan, None, release_groups);
        }
    }

    fn push_package(
        package: &Package,
        plan: &mut CratesIoFetchPlan,
        workspace_group: Option<&CratesIoReleaseGroupConfig>,
        release_groups: &[CratesIoReleaseGroupConfig],
    ) {
        push_entry(package, plan, workspace_group, release_groups);
        for vendored in package.vendored() {
            push_entry(vendored, plan, None, release_groups);
        }
    }

    match item {
        RootItem::Rust(RustProject::Workspace(workspace)) => {
            push_workspace(workspace, plan, release_groups);
        },
        RootItem::Rust(RustProject::Package(package)) => {
            push_package(package, plan, None, release_groups);
        },
        RootItem::Worktrees(group) => {
            for entry in group.iter_entries() {
                match entry {
                    RustProject::Workspace(workspace) => {
                        push_workspace(workspace, plan, release_groups);
                    },
                    RustProject::Package(package) => {
                        push_package(package, plan, None, release_groups);
                    },
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
mod tests {
    use super::*;
    use crate::config::WorkspaceMemberInclusion;
    use crate::project::MemberGroup;
    use crate::project::RustInfo;
    use crate::project::VendoredPackage;
    use crate::tui::app::async_tasks::constants::CRATES_IO_REFRESH_MIN_AGE_SECS;

    fn abs(raw: &str) -> AbsolutePath { AbsolutePath::from(raw) }

    #[test]
    fn plan_fans_duplicate_names_out_to_distinct_paths() {
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("serde", &abs("/a/serde"), ReleaseNotification::Crate);
        plan.insert("serde", &abs("/b/serde"), ReleaseNotification::Crate);
        plan.insert("serde", &abs("/a/serde"), ReleaseNotification::Crate);
        assert_eq!(plan.names().len(), 1, "one name means one query");
        assert_eq!(
            plan.by_name["serde"].paths.len(),
            2,
            "both paths fan out; the repeated (name, path) pair dedups"
        );
    }

    #[test]
    fn retain_paths_drops_emptied_names() {
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("serde", &abs("/a/serde"), ReleaseNotification::Crate);
        plan.insert("tokio", &abs("/a/tokio"), ReleaseNotification::Crate);
        plan.insert("tokio", &abs("/b/tokio"), ReleaseNotification::Crate);
        plan.retain_paths(|path| path.as_path().starts_with("/b"));
        assert!(
            !plan.names().contains("serde"),
            "a name with no surviving paths leaves the plan"
        );
        assert_eq!(
            plan.by_name["tokio"].paths,
            vec![abs("/b/tokio")],
            "surviving paths stay under their name"
        );
    }

    #[test]
    fn take_stalest_picks_the_oldest_query_past_the_age_gate() {
        let min_age = Duration::from_secs(CRATES_IO_REFRESH_MIN_AGE_SECS);
        let base = Instant::now();
        let now = base + min_age * 2;
        let mut plan = CratesIoFetchPlan::default();
        for name in ["fresh", "stale", "stalest"] {
            plan.insert(
                name,
                &abs(&format!("/x/{name}")),
                ReleaseNotification::Crate,
            );
        }
        let checked_at = HashMap::from([
            ("fresh".to_string(), now),
            ("stale".to_string(), base + min_age),
            ("stalest".to_string(), base),
        ]);

        let target = plan
            .take_stalest(&checked_at, now, min_age)
            .expect("one name is past the age gate");
        assert_eq!(target.name, "stalest", "the oldest query goes first");
        assert_eq!(target.paths, vec![abs("/x/stalest")]);
    }

    #[test]
    fn take_stalest_prefers_a_never_queried_name() {
        let min_age = Duration::from_secs(CRATES_IO_REFRESH_MIN_AGE_SECS);
        let base = Instant::now();
        let now = base + min_age * 2;
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("queried", &abs("/x/queried"), ReleaseNotification::Crate);
        plan.insert("new", &abs("/x/new"), ReleaseNotification::Crate);
        let checked_at = HashMap::from([("queried".to_string(), base)]);

        let target = plan
            .take_stalest(&checked_at, now, min_age)
            .expect("the unstamped name is eligible");
        assert_eq!(
            target.name, "new",
            "a name discovered mid-session outranks every stamped one"
        );
    }

    #[test]
    fn take_stalest_holds_back_until_the_age_gate_passes() {
        let min_age = Duration::from_secs(CRATES_IO_REFRESH_MIN_AGE_SECS);
        let base = Instant::now();
        let now = base + min_age.saturating_sub(Duration::from_millis(1));
        let mut plan = CratesIoFetchPlan::default();
        plan.insert("recent", &abs("/x/recent"), ReleaseNotification::Crate);
        let checked_at = HashMap::from([("recent".to_string(), base)]);

        assert!(
            plan.take_stalest(&checked_at, now, min_age).is_none(),
            "a name queried inside the window is not re-queried"
        );
        assert!(
            CratesIoFetchPlan::default()
                .take_stalest(&HashMap::new(), now, min_age)
                .is_none(),
            "an empty plan yields nothing to refresh"
        );
    }

    #[test]
    fn worker_buckets_round_robin_and_drop_empties() {
        let mut plan = CratesIoFetchPlan::default();
        for name in ["a", "b", "c", "d", "e"] {
            plan.insert(
                name,
                &abs(&format!("/x/{name}")),
                ReleaseNotification::Crate,
            );
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
        small.insert("only", &abs("/x/only"), ReleaseNotification::Crate);
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

    #[test]
    fn workspace_release_group_uses_one_representative_notification() {
        let release_group = CratesIoReleaseGroupConfig {
            representative:    "bevy".to_string(),
            label:             "Bevy".to_string(),
            workspace_members: WorkspaceMemberInclusion::Include,
            members:           Vec::new(),
        };
        let workspace = Workspace {
            path: abs("/rust/bevy"),
            name: Some("bevy".to_string()),
            rust: RustInfo {
                vendored: vec![VendoredPackage {
                    path: abs("/rust/bevy/vendor/serde"),
                    name: Some("serde".to_string()),
                    ..VendoredPackage::default()
                }],
                ..RustInfo::default()
            },
            groups: vec![MemberGroup::Inline {
                members: vec![
                    Package {
                        path: abs("/rust/bevy/crates/bevy_ecs"),
                        name: Some("bevy_ecs".to_string()),
                        ..Package::default()
                    },
                    Package {
                        path: abs("/rust/bevy/crates/bevy_settings"),
                        name: Some("bevy-settings".to_string()),
                        ..Package::default()
                    },
                ],
            }],
            ..Workspace::default()
        };
        let root_item = RootItem::Rust(RustProject::Workspace(workspace));
        let mut plan = CratesIoFetchPlan::default();

        collect_plan_children(&root_item, &mut plan, &[release_group]);

        assert_eq!(
            plan.by_name["bevy"].notification,
            ReleaseNotification::Group {
                label: "Bevy".to_string(),
            }
        );
        assert_eq!(
            plan.by_name["bevy_ecs"].notification,
            ReleaseNotification::Suppress
        );
        assert_eq!(
            plan.by_name["bevy-settings"].notification,
            ReleaseNotification::Suppress
        );
        assert_eq!(
            plan.by_name["serde"].notification,
            ReleaseNotification::Crate,
            "vendored crates do not inherit the workspace release group"
        );
    }

    #[test]
    fn explicit_release_group_applies_across_standalone_packages() {
        let release_group = CratesIoReleaseGroupConfig {
            representative: "suite".to_string(),
            label: "Suite".to_string(),
            members: vec!["suite-core".to_string()],
            ..CratesIoReleaseGroupConfig::default()
        };
        let representative = RootItem::Rust(RustProject::Package(Package {
            path: abs("/rust/suite"),
            name: Some("suite".to_string()),
            ..Package::default()
        }));
        let member = RootItem::Rust(RustProject::Package(Package {
            path: abs("/rust/suite-core"),
            name: Some("suite-core".to_string()),
            ..Package::default()
        }));
        let mut plan = CratesIoFetchPlan::default();

        collect_plan_children(
            &representative,
            &mut plan,
            std::slice::from_ref(&release_group),
        );
        collect_plan_children(&member, &mut plan, &[release_group]);

        assert_eq!(
            plan.by_name["suite"].notification,
            ReleaseNotification::Group {
                label: "Suite".to_string(),
            }
        );
        assert_eq!(
            plan.by_name["suite-core"].notification,
            ReleaseNotification::Suppress
        );
    }
}
