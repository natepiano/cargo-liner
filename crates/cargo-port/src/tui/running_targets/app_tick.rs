//! `App` glue for shared process refresh and Running Targets attribution.

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::hash_map::Entry;
use std::time::Duration;
use std::time::Instant;

use cargo_metadata::TargetKind;

use super::ExactRunningTargetOwnerEvidence;
use super::RunningTargetProjectAttribution;
use super::constants::BENCHES_DIR;
use super::constants::EXAMPLES_DIR;
use super::constants::SOURCE_DIR;
use crate::process_observation::ProcessRefreshDeadline;
use crate::process_observation::ProcessRefreshDispatchOutcome;
use crate::process_observation::ProcessRefreshExecutionOutcome;
use crate::process_observation::ProcessRefreshResultPoll;
use crate::process_observation::ProcessRefreshResultReceiver;
use crate::project::AbsolutePath;
use crate::project::CanonicalPathResolution;
use crate::project::CargoWorkspaceIndex;
use crate::project::CargoWorkspaceView;
use crate::project::VisibleTargetWorkspaceOwnership;
use crate::tui::app::App;
use crate::tui::messages::ProcessRefreshMsg;
use crate::tui::panes;
use crate::tui::panes::RunTargetKind;
use crate::tui::panes::TargetEntry;
use crate::tui::startup_services::StartupEffect;
use crate::tui::workspace_index::WorkspaceIndexReadiness;

/// Whether the foreground tick received one completed observer refresh and
/// therefore has an observer duration to instrument.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum ObserverRefreshTiming {
    #[default]
    NoCompletedRefresh,
    Completed(Duration),
}

/// Owned Running Targets attribution for one project or workspace. The value
/// lives across one view rebuild so [`RunningTargetProjectAttribution`] can borrow
/// its fields.
struct OwnedRunningTargetProjectAttribution {
    /// Target-directory path used for executable matching. Canonical when
    /// live resolution succeeds, otherwise the declared path.
    executable_match_target_directory: AbsolutePath,
    fallback_owner_root:               AbsolutePath,
    bench_names:                       HashSet<String>,
    bin_names:                         HashSet<String>,
    /// Declared owner evidence for each exact `(RunTargetKind, name)` identity.
    exact_target_owner_evidence: HashMap<(RunTargetKind, String), ExactRunningTargetOwnerEvidence>,
}

/// An indexed workspace paired with the Running Targets attribution that
/// receives visible targets owned by that exact [`CargoWorkspaceView`].
struct IndexedRunningTargetProjectAttribution<'a> {
    workspace:           &'a CargoWorkspaceView,
    project_attribution: OwnedRunningTargetProjectAttribution,
}

impl App {
    /// Dispatch due process work and reconcile completed immutable results.
    pub fn process_refresh_tick(&mut self, now: Instant) -> ObserverRefreshTiming {
        let mut observer_refresh_timing = ObserverRefreshTiming::NoCompletedRefresh;
        match self.process_refresh_executor.poll_result() {
            ProcessRefreshResultPoll::Ready(process_refresh_execution) => {
                observer_refresh_timing =
                    self.apply_process_refresh_execution(now, *process_refresh_execution);
            },
            ProcessRefreshResultPoll::Pending => {},
        }

        let running_targets_polling_effect = self.startup_services.running_targets_polling_effect();
        if running_targets_polling_effect == StartupEffect::Suppressed {
            self.startup_services
                .record_running_targets_polling(running_targets_polling_effect);
            return observer_refresh_timing;
        }

        match self.process_refresh_executor.refresh_due(now) {
            ProcessRefreshDispatchOutcome::Finished(process_refresh_execution) => {
                self.startup_services
                    .record_running_targets_polling(running_targets_polling_effect);
                observer_refresh_timing =
                    self.apply_process_refresh_execution(now, *process_refresh_execution);
            },
            ProcessRefreshDispatchOutcome::AwaitingWorker(_) => {
                self.startup_services
                    .record_running_targets_polling(running_targets_polling_effect);
            },
            ProcessRefreshDispatchOutcome::NotDue => {},
        }
        observer_refresh_timing
    }

    pub fn process_refresh_next_deadline(&self) -> ProcessRefreshDeadline {
        self.process_refresh_executor.next_deadline()
    }

    pub const fn process_refresh_result_receiver(&self) -> ProcessRefreshResultReceiver<'_> {
        self.process_refresh_executor.result_receiver()
    }

    fn apply_process_refresh_execution(
        &mut self,
        now: Instant,
        process_refresh_execution: ProcessRefreshMsg,
    ) -> ObserverRefreshTiming {
        let demand = process_refresh_execution.demand();
        let completed_process_refresh_execution = match process_refresh_execution.into_outcome() {
            ProcessRefreshExecutionOutcome::Completed(completed_process_refresh_execution) => {
                completed_process_refresh_execution
            },
            ProcessRefreshExecutionOutcome::Failed(failure) => {
                tracing::warn!(?failure, "process_refresh_execution_failed");
                return ObserverRefreshTiming::NoCompletedRefresh;
            },
        };
        let observer_refresh_timing =
            ObserverRefreshTiming::Completed(completed_process_refresh_execution.elapsed());
        if !demand.includes_running_targets() {
            return observer_refresh_timing;
        }
        let process_observation_snapshot = completed_process_refresh_execution.into_snapshot();

        let owned_project_attributions = self.collect_running_target_attributions();
        let project_attributions: Vec<RunningTargetProjectAttribution<'_>> =
            owned_project_attributions
                .iter()
                .map(|entry| RunningTargetProjectAttribution {
                    executable_match_target_directory: &entry.executable_match_target_directory,
                    fallback_owner_root:               &entry.fallback_owner_root,
                    bench_names:                       &entry.bench_names,
                    bin_names:                         &entry.bin_names,
                    exact_target_owner_evidence:       &entry.exact_target_owner_evidence,
                })
                .collect();
        self.panes.running_targets.apply_observation(
            now,
            &process_observation_snapshot,
            &project_attributions,
        );
        observer_refresh_timing
    }

    fn collect_running_target_attributions(&mut self) -> Vec<OwnedRunningTargetProjectAttribution> {
        #[cfg(test)]
        {
            self.running_target_attribution_collection_count += 1;
        }
        let visible_entries = self
            .panes
            .targets
            .content()
            .map(panes::build_target_list_from_data)
            .unwrap_or_default();
        let workspace_index_readiness = self.workspace_index_readiness();
        Self::collect_running_target_attributions_with_index_readiness(
            visible_entries,
            workspace_index_readiness,
        )
    }

    fn collect_running_target_attributions_with_index_readiness(
        visible_entries: Vec<TargetEntry>,
        workspace_index_readiness: WorkspaceIndexReadiness<'_>,
    ) -> Vec<OwnedRunningTargetProjectAttribution> {
        match workspace_index_readiness {
            WorkspaceIndexReadiness::Current {
                cargo_workspace_index,
            }
            | WorkspaceIndexReadiness::RetainedLastAccepted {
                cargo_workspace_index,
            } => {
                collect_indexed_running_target_attributions(cargo_workspace_index, visible_entries)
            },
            WorkspaceIndexReadiness::Uninitialized => {
                collect_unindexed_visible_target_attributions(visible_entries)
            },
        }
    }
}

fn collect_indexed_running_target_attributions(
    cargo_workspace_index: &CargoWorkspaceIndex,
    visible_entries: Vec<TargetEntry>,
) -> Vec<OwnedRunningTargetProjectAttribution> {
    let mut indexed_project_attributions: Vec<_> = cargo_workspace_index
        .workspaces()
        .map(|workspace| IndexedRunningTargetProjectAttribution {
            workspace,
            project_attribution: OwnedRunningTargetProjectAttribution::from_workspace(workspace),
        })
        .collect();
    let mut unindexed_project_attributions = Vec::new();

    for entry in visible_entries {
        match cargo_workspace_index
            .workspace_for_visible_target(&entry.src_path, &entry.project_path)
        {
            VisibleTargetWorkspaceOwnership::Indexed(workspace) => {
                if let Some(indexed_project_attribution) = indexed_project_attributions
                    .iter_mut()
                    .find(|candidate| std::ptr::eq(candidate.workspace, workspace))
                {
                    indexed_project_attribution
                        .project_attribution
                        .add_visible_target(entry);
                } else {
                    let mut project_attribution =
                        OwnedRunningTargetProjectAttribution::from_workspace(workspace);
                    project_attribution.add_visible_target(entry);
                    indexed_project_attributions.push(IndexedRunningTargetProjectAttribution {
                        workspace,
                        project_attribution,
                    });
                }
            },
            VisibleTargetWorkspaceOwnership::Ambiguous
            | VisibleTargetWorkspaceOwnership::NotIndexed => {
                add_unindexed_visible_target(&mut unindexed_project_attributions, entry);
            },
        }
    }

    indexed_project_attributions
        .into_iter()
        .map(|indexed_project_attribution| indexed_project_attribution.project_attribution)
        .chain(unindexed_project_attributions)
        .collect()
}

impl OwnedRunningTargetProjectAttribution {
    fn from_workspace(workspace: &CargoWorkspaceView) -> Self {
        let mut bench_names = HashSet::new();
        let mut bin_names = HashSet::new();
        let mut exact_target_owner_evidence = HashMap::new();
        for package in workspace.packages() {
            for target_identity in package.targets() {
                for (cargo_kind, kind) in [
                    (TargetKind::Bin, RunTargetKind::Binary),
                    (TargetKind::Example, RunTargetKind::Example),
                    (TargetKind::Bench, RunTargetKind::Bench),
                ] {
                    if target_identity.kinds().contains(&cargo_kind) {
                        include_exact_target_owner(
                            &mut exact_target_owner_evidence,
                            kind,
                            target_identity.name().to_string(),
                            package.declared_member_root_path().clone(),
                        );
                    }
                }
                if target_identity.kinds().contains(&TargetKind::Bench) {
                    bench_names.insert(target_identity.name().to_string());
                }
                if target_identity.kinds().contains(&TargetKind::Bin) {
                    bin_names.insert(target_identity.name().to_string());
                }
            }
        }
        Self {
            executable_match_target_directory: indexed_workspace_running_target_directory_path(
                workspace,
            ),
            fallback_owner_root: workspace.declared_checkout_root_path().clone(),
            bench_names,
            bin_names,
            exact_target_owner_evidence,
        }
    }

    fn add_visible_target(&mut self, entry: TargetEntry) {
        match entry.run_target_kind {
            RunTargetKind::Bench => {
                self.bench_names.insert(entry.name.clone());
            },
            RunTargetKind::Binary => {
                self.bin_names.insert(entry.name.clone());
            },
            RunTargetKind::Example => {},
        }
        let target_owner_root = target_owner_root_for_entry(&entry);
        include_exact_target_owner(
            &mut self.exact_target_owner_evidence,
            entry.run_target_kind,
            entry.name,
            target_owner_root,
        );
    }
}

fn include_exact_target_owner(
    exact_target_owner_evidence: &mut HashMap<
        (RunTargetKind, String),
        ExactRunningTargetOwnerEvidence,
    >,
    run_target_kind: RunTargetKind,
    name: String,
    declared_member_root: AbsolutePath,
) {
    match exact_target_owner_evidence.entry((run_target_kind, name)) {
        Entry::Occupied(mut entry) => entry.get_mut().include(&declared_member_root),
        Entry::Vacant(entry) => {
            entry.insert(ExactRunningTargetOwnerEvidence::Unique(
                declared_member_root,
            ));
        },
    }
}

fn collect_unindexed_visible_target_attributions(
    visible_entries: Vec<TargetEntry>,
) -> Vec<OwnedRunningTargetProjectAttribution> {
    let mut project_attributions = Vec::new();
    for entry in visible_entries {
        add_unindexed_visible_target(&mut project_attributions, entry);
    }
    project_attributions
}

fn add_unindexed_visible_target(
    project_attributions: &mut Vec<OwnedRunningTargetProjectAttribution>,
    entry: TargetEntry,
) {
    let fallback_owner_root = entry.project_path.clone();
    let declared_target_directory =
        AbsolutePath::from(fallback_owner_root.as_path().join("target"));
    let executable_match_target_directory =
        unindexed_visible_running_target_directory_path(&declared_target_directory);
    if let Some(project_attribution) = project_attributions.iter_mut().find(|candidate| {
        candidate.executable_match_target_directory == executable_match_target_directory
            && candidate.fallback_owner_root == fallback_owner_root
    }) {
        project_attribution.add_visible_target(entry);
    } else {
        let mut project_attribution = OwnedRunningTargetProjectAttribution {
            executable_match_target_directory,
            fallback_owner_root,
            bench_names: HashSet::new(),
            bin_names: HashSet::new(),
            exact_target_owner_evidence: HashMap::new(),
        };
        project_attribution.add_visible_target(entry);
        project_attributions.push(project_attribution);
    }
}

fn indexed_workspace_running_target_directory_path(workspace: &CargoWorkspaceView) -> AbsolutePath {
    match workspace.target_directory_resolution() {
        CanonicalPathResolution::Resolved(target_directory) => target_directory.path().clone(),
        CanonicalPathResolution::Unresolved => workspace.declared_target_directory_path().clone(),
    }
}

fn unindexed_visible_running_target_directory_path(
    declared_target_directory: &AbsolutePath,
) -> AbsolutePath {
    declared_target_directory
        .as_path()
        .canonicalize()
        .map_or_else(|_| declared_target_directory.clone(), AbsolutePath::from)
}

fn target_owner_root_for_entry(entry: &TargetEntry) -> AbsolutePath {
    let dir_name = match entry.run_target_kind {
        RunTargetKind::Binary => SOURCE_DIR,
        RunTargetKind::Example => EXAMPLES_DIR,
        RunTargetKind::Bench => BENCHES_DIR,
    };
    entry
        .src_path
        .as_path()
        .ancestors()
        .find(|path| path.file_name().and_then(|name| name.to_str()) == Some(dir_name))
        .and_then(|target_dir| target_dir.parent())
        .map_or_else(|| entry.project_path.clone(), AbsolutePath::from)
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "tests should fail on invalid fixtures")]
mod tests {
    use std::collections::BTreeMap;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::Path;
    use std::time::Duration;

    use cargo_metadata::PackageId;
    use cargo_metadata::semver::Version;

    use super::*;
    use crate::process_observation::CompileMonitorRefreshSchedule;
    use crate::process_observation::ProcessRefreshExecution;
    use crate::process_observation::ProcessRefreshExecutionBackendSelection;
    use crate::process_observation::ProcessRefreshExecutor;
    use crate::process_observation::RunningTargetsRefreshSchedule;
    use crate::process_observation::snapshot::ProcessRefreshExecutionFailure;
    use crate::project::FileStamp;
    use crate::project::ManifestFingerprint;
    use crate::project::Package;
    use crate::project::PackageRecord;
    use crate::project::PublishPolicy;
    use crate::project::RootItem;
    use crate::project::RustProject;
    use crate::project::TargetRecord;
    use crate::project::Visibility;
    use crate::project::Workspace;
    use crate::project::WorkspaceMetadata;
    use crate::tui::panes::TargetSource;
    use crate::tui::panes::TargetsData;
    use crate::tui::project_list::ProjectList;
    use crate::tui::startup_services::StartupServices;

    fn path(path: impl AsRef<Path>) -> AbsolutePath {
        AbsolutePath::from(path.as_ref().to_path_buf())
    }

    fn package_root(project_path: impl AsRef<Path>) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: path(project_path),
            ..Package::default()
        }))
    }

    fn workspace_root(project_path: impl AsRef<Path>) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: path(project_path),
            ..Workspace::default()
        }))
    }

    fn example_entry(
        project_path: impl AsRef<Path>,
        member_path: impl AsRef<Path>,
        name: &str,
    ) -> TargetEntry {
        TargetEntry {
            name:              name.to_string(),
            display_name:      name.to_string(),
            run_target_kind:   RunTargetKind::Example,
            source:            TargetSource::member("fake_widgets".to_string()),
            project_path:      path(project_path),
            package_name:      "fake_widgets".to_string(),
            src_path:          path(
                member_path
                    .as_ref()
                    .join("examples")
                    .join(format!("{name}.rs")),
            ),
            required_features: Vec::new(),
        }
    }

    fn worktree_example_entry(project_path: &Path, member_path: &Path, name: &str) -> TargetEntry {
        let mut entry = example_entry(project_path, member_path, name);
        entry.source = TargetSource::worktree("member".to_string());
        entry.package_name = "member".to_string();
        entry
    }

    fn create_example_fixture(source_path: &Path, target_directory: &Path) {
        std::fs::create_dir_all(source_path.parent().expect("source parent should exist"))
            .expect("create member source directory");
        std::fs::create_dir_all(target_directory).expect("create target directory");
        std::fs::write(source_path, "fn main() {}").expect("write target source fixture");
    }

    fn metadata(
        checkout_root: impl AsRef<Path>,
        target_directory: impl AsRef<Path>,
    ) -> WorkspaceMetadata {
        WorkspaceMetadata {
            declared_checkout_root:   path(checkout_root.as_ref()),
            cargo_workspace_root:     path(checkout_root.as_ref()),
            target_directory:         path(target_directory),
            packages:                 HashMap::new(),
            fingerprint:              ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            },
            out_of_tree_target_bytes: None,
        }
    }

    fn metadata_with_example(
        checkout_root: &Path,
        target_directory: &Path,
        member_root: &Path,
        target_name: &str,
        source_path: &Path,
    ) -> WorkspaceMetadata {
        let mut workspace_metadata = metadata(checkout_root, target_directory);
        workspace_metadata.packages.insert(
            PackageId {
                repr: format!("member 0.1.0 (path+file://{})", checkout_root.display()),
            },
            example_package_record("member", member_root, target_name, source_path),
        );
        workspace_metadata
    }

    fn example_package_record(
        package_name: &str,
        member_root: &Path,
        target_name: &str,
        source_path: &Path,
    ) -> PackageRecord {
        PackageRecord {
            name:          package_name.to_string(),
            version:       Version::new(0, 1, 0),
            edition:       "2024".to_string(),
            description:   None,
            license:       None,
            homepage:      None,
            repository:    None,
            manifest_path: path(member_root.join("Cargo.toml")),
            targets:       vec![TargetRecord {
                name:              target_name.to_string(),
                kinds:             vec![TargetKind::Example],
                src_path:          path(source_path),
                required_features: Vec::new(),
            }],
            publish:       PublishPolicy::Any,
        }
    }

    fn upsert_two_workspace_metadata(
        app: &App,
        first_metadata: WorkspaceMetadata,
        second_metadata: WorkspaceMetadata,
    ) {
        let metadata_store_handle = app.scan.metadata_store_handle();
        let mut metadata_store = metadata_store_handle
            .lock()
            .expect("metadata store lock should be available");
        metadata_store.upsert(first_metadata);
        metadata_store.upsert(second_metadata);
    }

    fn poison_metadata_store(app: &App) {
        let metadata_store_handle = app.scan.metadata_store_handle();
        let poisoning_thread = std::thread::spawn(move || {
            let _metadata_store = metadata_store_handle
                .lock()
                .expect("metadata store lock should be available before poisoning");
            std::panic::resume_unwind(Box::new("poison metadata store"));
        });
        assert!(poisoning_thread.join().is_err());
    }

    fn attribution_for_target_directory<'a>(
        project_attributions: &'a [OwnedRunningTargetProjectAttribution],
        target_directory: &Path,
    ) -> &'a OwnedRunningTargetProjectAttribution {
        let canonical_target_directory = path(
            target_directory
                .canonicalize()
                .expect("canonicalize target directory"),
        );
        project_attributions
            .iter()
            .find(|attribution| {
                attribution.executable_match_target_directory == canonical_target_directory
            })
            .expect("workspace attribution should exist")
    }

    #[test]
    fn visible_targets_supply_attribution_before_metadata_lands() {
        let mut app = crate::tui::test_support::make_app(&[]);
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![example_entry(
                "/tmp/hana",
                "/tmp/hana/crates/fake_widgets",
                "oit_resize_repro",
            )],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let project_attribution = &project_attributions[0];
        let key = (RunTargetKind::Example, "oit_resize_repro".to_string());

        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attribution.executable_match_target_directory,
            path("/tmp/hana/target")
        );
        assert_eq!(
            project_attribution.exact_target_owner_evidence.get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(
                "/tmp/hana/crates/fake_widgets"
            )))
        );
    }

    #[test]
    fn subsecond_app_ticks_skip_attribution_collection_until_due() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let poll_interval = Duration::from_secs(1);
        let first_poll = Instant::now();
        app.startup_services = StartupServices::production();
        app.process_refresh_executor = ProcessRefreshExecutor::new(
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Every(poll_interval),
            CompileMonitorRefreshSchedule::NotScheduled,
            first_poll,
        );

        assert!(matches!(
            app.process_refresh_tick(first_poll),
            ObserverRefreshTiming::Completed(_)
        ));
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        assert_eq!(app.running_target_attribution_collection_count, 1);
        assert_eq!(
            app.process_refresh_next_deadline(),
            ProcessRefreshDeadline::At(first_poll + poll_interval)
        );

        app.process_refresh_tick(first_poll + poll_interval / 4);
        app.process_refresh_tick(
            first_poll + poll_interval.saturating_sub(Duration::from_millis(1)),
        );

        assert_eq!(app.running_target_attribution_collection_count, 1);
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);

        app.process_refresh_tick(first_poll + poll_interval);

        assert_eq!(app.running_target_attribution_collection_count, 2);
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);
    }

    #[test]
    fn request_channel_failure_has_no_completed_observer_timing() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::failed_for_test(
            crate::process_observation::ProcessRefreshConsumerDemand::RunningTargets,
            ProcessRefreshExecutionFailure::RequestChannelDisconnected,
        );

        assert_eq!(
            app.apply_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::NoCompletedRefresh
        );
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    #[test]
    fn result_channel_failure_has_no_completed_observer_timing() {
        let mut app = crate::tui::test_support::make_app(&[]);
        let process_refresh_execution = ProcessRefreshExecution::failed_for_test(
            crate::process_observation::ProcessRefreshConsumerDemand::RunningTargets,
            ProcessRefreshExecutionFailure::ResultChannelDisconnected,
        );

        assert_eq!(
            app.apply_process_refresh_execution(Instant::now(), process_refresh_execution),
            ObserverRefreshTiming::NoCompletedRefresh
        );
        assert_eq!(app.running_target_attribution_collection_count, 0);
    }

    #[test]
    fn moving_selection_between_existing_rows_keeps_content_revision_and_index() {
        let first_path = Path::new("/tmp/first-project");
        let second_path = Path::new("/tmp/second-project");
        let mut app = crate::tui::test_support::make_app(&[
            package_root(first_path),
            package_root(second_path),
        ]);
        let _ = app.collect_running_target_attributions();
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        let project_list_revision = app.project_list.revision();

        app.project_list.select_project_path(path(second_path));
        let _ = app.collect_running_target_attributions();
        app.project_list.select_project_path(path(first_path));
        let _ = app.collect_running_target_attributions();
        app.project_list.clear_selected_project();
        let _ = app.collect_running_target_attributions();

        assert_eq!(app.project_list.revision(), project_list_revision);
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);
    }

    #[test]
    fn visible_row_revision_rebuilds_with_unchanged_selected_path() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let first_path = temp_dir.path().join("first");
        let second_path = temp_dir.path().join("second");
        let first = package_root(&first_path);
        let second = package_root(&second_path);
        let mut app = crate::tui::test_support::make_app(&[first, second]);
        let _ = app.collect_running_target_attributions();
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        let selected_path = app
            .project_list
            .selected_project_path()
            .map(AbsolutePath::from);

        app.handle_disk_usage(second_path.as_path(), 0);
        app.ensure_visible_rows_cached();
        assert_eq!(
            app.project_list
                .at_path(second_path.as_path())
                .expect("second project should exist")
                .visibility,
            Visibility::Deleted
        );
        assert_eq!(
            app.project_list
                .selected_project_path()
                .map(AbsolutePath::from),
            selected_path
        );

        let _ = app.collect_running_target_attributions();

        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count + 1);
    }

    #[test]
    fn same_path_package_to_workspace_replacement_rebuilds_the_index_once() {
        let mut app = crate::tui::test_support::make_app(&[package_root("/tmp/project")]);
        let _ = app.collect_running_target_attributions();
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        let project_list_revision = app.project_list.revision();

        app.mutate_tree()
            .replace_all(ProjectList::new(vec![workspace_root("/tmp/project")]));
        app.sync_selected_project();

        assert!(app.project_list.revision() > project_list_revision);
        assert_eq!(
            app.project_list.selected_project_path(),
            Some(Path::new("/tmp/project"))
        );
        let _ = app.collect_running_target_attributions();
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count + 1);
        let _ = app.collect_running_target_attributions();
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count + 1);
    }

    #[test]
    fn visible_targets_augment_a_matching_metadata_slice() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let checkout_root = temp_dir.path().join("hana");
        let member_root = checkout_root.join("crates/fake_widgets");
        let target_directory = checkout_root.join("custom-target");
        std::fs::create_dir_all(&member_root).expect("create member directory");
        std::fs::create_dir_all(&target_directory).expect("create target directory");
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(metadata(&checkout_root, &target_directory));
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![example_entry(
                &checkout_root,
                &member_root,
                "oit_resize_repro",
            )],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let key = (RunTargetKind::Example, "oit_resize_repro".to_string());

        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attributions[0]
                .exact_target_owner_evidence
                .get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(&member_root)))
        );
    }

    #[test]
    fn duplicate_package_target_names_produce_ambiguous_member_evidence() {
        let checkout_root = Path::new("/tmp/duplicate-workspace");
        let target_directory = checkout_root.join("target");
        let first_member_root = checkout_root.join("crates/first");
        let second_member_root = checkout_root.join("crates/second");
        let mut workspace_metadata = metadata(checkout_root, &target_directory);
        workspace_metadata.packages.insert(
            PackageId {
                repr: "first-package-id".to_string(),
            },
            example_package_record(
                "first",
                &first_member_root,
                "duplicate",
                &first_member_root.join("examples/duplicate.rs"),
            ),
        );
        workspace_metadata.packages.insert(
            PackageId {
                repr: "second-package-id".to_string(),
            },
            example_package_record(
                "second",
                &second_member_root,
                "duplicate",
                &second_member_root.join("examples/duplicate.rs"),
            ),
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(workspace_metadata);

        let project_attributions = app.collect_running_target_attributions();
        let key = (RunTargetKind::Example, "duplicate".to_string());

        assert_eq!(
            project_attributions[0]
                .exact_target_owner_evidence
                .get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Ambiguous)
        );
    }

    #[test]
    fn visible_target_agreement_keeps_unique_member_evidence() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let checkout_root = temp_dir.path().join("workspace");
        let member_root = checkout_root.join("crates/member");
        let source_path = member_root.join("examples/agreed.rs");
        let target_directory = checkout_root.join("target");
        std::fs::create_dir_all(source_path.parent().expect("source parent should exist"))
            .expect("create member source directory");
        std::fs::create_dir_all(&target_directory).expect("create target directory");
        std::fs::write(&source_path, "fn main() {}").expect("write target source fixture");
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(metadata_with_example(
                &checkout_root,
                &target_directory,
                &member_root,
                "agreed",
                &source_path,
            ));
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![example_entry(&checkout_root, &member_root, "agreed")],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let key = (RunTargetKind::Example, "agreed".to_string());

        assert_eq!(
            project_attributions[0]
                .exact_target_owner_evidence
                .get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(&member_root)))
        );
    }

    #[test]
    fn retained_visible_target_keeps_its_indexed_member_owner() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let workspace_root = temp_dir.path().join("workspace");
        let member_root = workspace_root.join("crates/member");
        let current_source = member_root.join("examples/current.rs");
        let removed_source = member_root.join("examples/removed.rs");
        let shared_target = temp_dir.path().join("shared-target");
        std::fs::create_dir_all(current_source.parent().expect("source parent should exist"))
            .expect("create member source directory");
        std::fs::create_dir_all(&shared_target).expect("create shared target directory");
        std::fs::write(&current_source, "fn main() {}")
            .expect("write current target source fixture");

        let mut workspace_metadata = metadata(&workspace_root, &shared_target);
        workspace_metadata.packages.insert(
            PackageId {
                repr: "member 0.1.0 (path+file:///member)".to_string(),
            },
            PackageRecord {
                name:          "member".to_string(),
                version:       Version::new(0, 1, 0),
                edition:       "2024".to_string(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: path(member_root.join("Cargo.toml")),
                targets:       vec![TargetRecord {
                    name:              "current".to_string(),
                    kinds:             vec![TargetKind::Example],
                    src_path:          path(&current_source),
                    required_features: Vec::new(),
                }],
                publish:       PublishPolicy::Any,
            },
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(workspace_metadata);
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![TargetEntry {
                name:              "removed".to_string(),
                display_name:      "removed".to_string(),
                run_target_kind:   RunTargetKind::Example,
                source:            TargetSource::member("member".to_string()),
                project_path:      path(&member_root),
                package_name:      "member".to_string(),
                src_path:          path(&removed_source),
                required_features: Vec::new(),
            }],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let key = (RunTargetKind::Example, "removed".to_string());
        let declared_member_root = path(&member_root);

        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attributions[0]
                .executable_match_target_directory
                .as_path(),
            shared_target
                .canonicalize()
                .expect("canonicalize shared target directory")
        );
        assert_eq!(
            project_attributions[0].fallback_owner_root.as_path(),
            workspace_root.as_path()
        );
        assert_eq!(
            project_attributions[0]
                .exact_target_owner_evidence
                .get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(
                declared_member_root
            ))
        );
    }

    #[test]
    fn ambiguous_workspace_ownership_uses_the_unindexed_visible_target_fallback() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let first_checkout_root = temp_dir.path().join("first-checkout");
        let second_checkout_root = temp_dir.path().join("second-checkout");
        let visible_project_root = temp_dir.path().join("visible-project");
        let shared_source = temp_dir.path().join("shared-source/examples/demo.rs");
        std::fs::create_dir_all(&first_checkout_root).expect("create first checkout directory");
        std::fs::create_dir_all(&second_checkout_root).expect("create second checkout directory");
        std::fs::create_dir_all(&visible_project_root).expect("create visible project directory");
        std::fs::create_dir_all(
            shared_source
                .parent()
                .expect("shared source parent should exist"),
        )
        .expect("create shared source directory");
        std::fs::write(&shared_source, "fn main() {}").expect("write shared source fixture");

        let first_metadata = metadata_with_example(
            &first_checkout_root,
            &first_checkout_root.join("target"),
            &first_checkout_root,
            "demo",
            &shared_source,
        );
        let second_metadata = metadata_with_example(
            &second_checkout_root,
            &second_checkout_root.join("target"),
            &second_checkout_root,
            "demo",
            &shared_source,
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        {
            let metadata_store_handle = app.scan.metadata_store_handle();
            let mut metadata_store = metadata_store_handle
                .lock()
                .expect("metadata store lock should be available");
            metadata_store.upsert(first_metadata);
            metadata_store.upsert(second_metadata);
        }
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![TargetEntry {
                name:              "demo".to_string(),
                display_name:      "demo".to_string(),
                run_target_kind:   RunTargetKind::Example,
                source:            TargetSource::member("member".to_string()),
                project_path:      path(&visible_project_root),
                package_name:      "member".to_string(),
                src_path:          path(&shared_source),
                required_features: Vec::new(),
            }],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let fallback_attribution = project_attributions
            .iter()
            .find(|attribution| attribution.fallback_owner_root == path(&visible_project_root))
            .expect("ambiguous target should use visible project fallback");

        assert_eq!(
            fallback_attribution.executable_match_target_directory,
            path(visible_project_root.join("target"))
        );
    }

    #[test]
    fn visible_worktree_targets_join_exact_indexed_workspace_attributions() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let first_checkout_root = temp_dir.path().join("first-checkout");
        let first_member_root = first_checkout_root.join("crates/member");
        let first_source = first_member_root.join("examples/first-visible.rs");
        let first_target_directory = temp_dir.path().join("first-custom-target");
        let second_checkout_root = temp_dir.path().join("second-checkout");
        let second_member_root = second_checkout_root.join("crates/member");
        let second_current_source = second_member_root.join("examples/current.rs");
        let second_target_directory = temp_dir.path().join("second-custom-target");
        create_example_fixture(&first_source, &first_target_directory);
        create_example_fixture(&second_current_source, &second_target_directory);

        let first_metadata = metadata_with_example(
            &first_checkout_root,
            &first_target_directory,
            &first_member_root,
            "first-visible",
            &first_source,
        );
        let second_metadata = metadata_with_example(
            &second_checkout_root,
            &second_target_directory,
            &second_member_root,
            "current",
            &second_current_source,
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        upsert_two_workspace_metadata(&app, first_metadata, second_metadata);
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![
                worktree_example_entry(&first_checkout_root, &first_member_root, "first-visible"),
                worktree_example_entry(&second_checkout_root, &second_member_root, "retained"),
            ],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let first_attribution =
            attribution_for_target_directory(&project_attributions, &first_target_directory);
        let second_attribution =
            attribution_for_target_directory(&project_attributions, &second_target_directory);
        let first_key = (RunTargetKind::Example, "first-visible".to_string());
        let retained_key = (RunTargetKind::Example, "retained".to_string());

        assert_eq!(project_attributions.len(), 2);
        assert_eq!(
            first_attribution.fallback_owner_root,
            path(&first_checkout_root)
        );
        assert_eq!(
            first_attribution
                .exact_target_owner_evidence
                .get(&first_key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(
                &first_member_root
            )))
        );
        assert!(
            !first_attribution
                .exact_target_owner_evidence
                .contains_key(&retained_key)
        );
        assert_eq!(
            second_attribution.fallback_owner_root,
            path(&second_checkout_root)
        );
        assert_eq!(
            second_attribution
                .exact_target_owner_evidence
                .get(&retained_key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(
                &second_member_root
            )))
        );
        assert!(
            !second_attribution
                .exact_target_owner_evidence
                .contains_key(&first_key)
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_checkout_keeps_declared_running_target_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let canonical_checkout_root = temp_dir.path().join("canonical-checkout");
        let declared_checkout_root = temp_dir.path().join("declared-checkout");
        let target_directory = canonical_checkout_root.join("target");
        fs::create_dir_all(&target_directory)?;
        symlink(&canonical_checkout_root, &declared_checkout_root)?;
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(metadata(
                &declared_checkout_root,
                declared_checkout_root.join("target"),
            ));
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![TargetEntry {
                name:              "stale".to_string(),
                display_name:      "stale".to_string(),
                run_target_kind:   RunTargetKind::Example,
                source:            TargetSource::member("removed".to_string()),
                project_path:      path(&declared_checkout_root),
                package_name:      "removed".to_string(),
                src_path:          path(declared_checkout_root.join("examples/stale.rs")),
                required_features: Vec::new(),
            }],
            benches:  Vec::new(),
        });

        let project_attributions = app.collect_running_target_attributions();
        let workspace = app
            .cargo_workspace_index
            .workspaces()
            .next()
            .ok_or("indexed workspace should exist")?;

        assert!(matches!(
            workspace.checkout_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == canonical_checkout_root.canonicalize()?
        ));
        assert_eq!(
            workspace.declared_checkout_root_path().as_path(),
            declared_checkout_root.as_path()
        );
        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attributions[0].fallback_owner_root.as_path(),
            declared_checkout_root.as_path()
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn symlink_member_keeps_canonical_identity_and_declared_running_target_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let workspace_root = temp_dir.path().join("workspace");
        let canonical_member_root = temp_dir.path().join("canonical-member");
        let declared_member_root = workspace_root.join("crates/member");
        let declared_source = declared_member_root.join("examples/demo.rs");
        let canonical_source = canonical_member_root.join("examples/demo.rs");
        let target_directory = workspace_root.join("target");
        fs::create_dir_all(&workspace_root)?;
        fs::create_dir_all(
            declared_member_root
                .parent()
                .ok_or("declared member parent should exist")?,
        )?;
        fs::create_dir_all(
            canonical_source
                .parent()
                .ok_or("canonical source parent should exist")?,
        )?;
        fs::create_dir_all(&target_directory)?;
        fs::write(&canonical_source, "fn main() {}")?;
        symlink(&canonical_member_root, &declared_member_root)?;

        let mut workspace_metadata = metadata(&workspace_root, &target_directory);
        workspace_metadata.packages.insert(
            PackageId {
                repr: "member 0.1.0 (path+file:///declared-member)".to_string(),
            },
            PackageRecord {
                name:          "member".to_string(),
                version:       Version::new(0, 1, 0),
                edition:       "2024".to_string(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: path(declared_member_root.join("Cargo.toml")),
                targets:       vec![TargetRecord {
                    name:              "demo".to_string(),
                    kinds:             vec![TargetKind::Example],
                    src_path:          path(&declared_source),
                    required_features: Vec::new(),
                }],
                publish:       PublishPolicy::Any,
            },
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(workspace_metadata);

        let project_attributions = app.collect_running_target_attributions();
        let package = app
            .cargo_workspace_index
            .workspaces()
            .flat_map(CargoWorkspaceView::packages)
            .next()
            .ok_or("indexed package should exist")?;
        let key = (RunTargetKind::Example, "demo".to_string());

        assert!(matches!(
            package.member_root_resolution(),
            CanonicalPathResolution::Resolved(root)
                if root.path().as_path() == canonical_member_root.canonicalize()?
        ));
        assert_eq!(
            package.declared_member_root_path().as_path(),
            declared_member_root
        );
        assert_eq!(
            project_attributions[0]
                .exact_target_owner_evidence
                .get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(
                &declared_member_root
            )))
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn target_directory_symlink_retargets_without_index_rebuild()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = tempfile::tempdir()?;
        let workspace_root = temp_dir.path().join("workspace");
        let first_target = temp_dir.path().join("first-target");
        let second_target = temp_dir.path().join("second-target");
        let target_link = temp_dir.path().join("target-link");
        fs::create_dir_all(&workspace_root)?;
        fs::create_dir_all(&first_target)?;
        fs::create_dir_all(&second_target)?;
        symlink(&first_target, &target_link)?;
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(metadata(&workspace_root, &target_link));

        let first_attributions = app.collect_running_target_attributions();
        let rebuild_count = app.cargo_workspace_index.rebuild_count();
        assert_eq!(
            first_attributions[0]
                .executable_match_target_directory
                .as_path(),
            first_target.canonicalize()?
        );

        fs::remove_file(&target_link)?;
        symlink(&second_target, &target_link)?;
        let second_attributions = app.collect_running_target_attributions();

        assert_eq!(
            second_attributions[0]
                .executable_match_target_directory
                .as_path(),
            second_target.canonicalize()?
        );
        assert_eq!(app.cargo_workspace_index.rebuild_count(), rebuild_count);
        Ok(())
    }

    #[test]
    fn lock_failure_retains_last_accepted_workspace_index() {
        let temp_dir = tempfile::tempdir().expect("create temporary directory");
        let checkout_root = temp_dir.path().join("workspace");
        let member_root = checkout_root.join("crates/member");
        let source_path = member_root.join("examples/metadata-only.rs");
        let target_directory = temp_dir.path().join("custom-target");
        create_example_fixture(&source_path, &target_directory);
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(metadata_with_example(
                &checkout_root,
                &target_directory,
                &member_root,
                "metadata-only",
                &source_path,
            ));
        let _ = app.collect_running_target_attributions();
        poison_metadata_store(&app);

        assert!(matches!(
            app.workspace_index_readiness(),
            WorkspaceIndexReadiness::RetainedLastAccepted { .. }
        ));
        let project_attributions = app.collect_running_target_attributions();
        let project_attribution =
            attribution_for_target_directory(&project_attributions, &target_directory);
        let key = (RunTargetKind::Example, "metadata-only".to_string());

        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attribution.fallback_owner_root,
            path(&checkout_root)
        );
        assert_eq!(
            project_attribution.exact_target_owner_evidence.get(&key),
            Some(&ExactRunningTargetOwnerEvidence::Unique(path(&member_root)))
        );
    }

    #[test]
    fn uninitialized_workspace_index_uses_only_the_default_visible_slice() {
        let visible_entry = example_entry(
            "/tmp/visible-project",
            "/tmp/stale-checkout/crates/fake_widgets",
            "example",
        );
        let mut stale_metadata = metadata("/tmp/stale-checkout", "/tmp/stale-target");
        stale_metadata.packages.insert(
            PackageId {
                repr: "stale-package-id".to_string(),
            },
            PackageRecord {
                name:          "fake_widgets".to_string(),
                version:       Version::new(0, 1, 0),
                edition:       "2024".to_string(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: path("/tmp/stale-checkout/crates/fake_widgets/Cargo.toml"),
                targets:       vec![TargetRecord {
                    name:              visible_entry.name.clone(),
                    kinds:             vec![TargetKind::Example],
                    src_path:          visible_entry.src_path.clone(),
                    required_features: Vec::new(),
                }],
                publish:       PublishPolicy::Any,
            },
        );
        let mut app = crate::tui::test_support::make_app(&[]);
        app.scan
            .metadata_store_handle()
            .lock()
            .expect("metadata store lock should be available")
            .upsert(stale_metadata);
        app.panes.targets.set_content(TargetsData {
            binaries: Vec::new(),
            examples: vec![visible_entry],
            benches:  Vec::new(),
        });
        app.cargo_workspace_index = crate::project::CargoWorkspaceIndex::default();
        poison_metadata_store(&app);

        assert!(matches!(
            app.workspace_index_readiness(),
            WorkspaceIndexReadiness::Uninitialized
        ));
        let project_attributions = app.collect_running_target_attributions();

        assert_eq!(project_attributions.len(), 1);
        assert_eq!(
            project_attributions[0].executable_match_target_directory,
            path("/tmp/visible-project/target")
        );
        assert_eq!(
            project_attributions[0].fallback_owner_root,
            path("/tmp/visible-project")
        );
    }
}
