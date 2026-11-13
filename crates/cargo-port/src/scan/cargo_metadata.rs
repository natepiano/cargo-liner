use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt::Formatter;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use ansi_to_tui::IntoText;
use cargo_metadata::Error;
use cargo_metadata::Metadata;
use cargo_metadata::MetadataCommand;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use tui_pane::PERF_LOG_TARGET;

use super::BackgroundMsg;
use super::SourceMeasurements;
use super::background_workers;
use super::constants::CARGO_OFFLINE_FLAG;
use super::discovery;
use super::disk_usage;
use super::exclude::ExcludeDirs;
use super::language_stats;
use super::pending_work::NextWork;
use super::pending_work::PendingWork;
use super::pending_work::PendingWorker;
use super::pending_work::WorkSubmission;
use super::test_counts;
use super::tree;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::config;
use crate::config::NonRustInclusion;
use crate::constants::CARGO_TOML;
use crate::http::HttpClient;
use crate::project::AbsolutePath;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::RootItem;
use crate::project::TargetRecord;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;

/// Structured failure for a `cargo metadata` invocation. Held inside
/// [`BackgroundMsg::CargoMetadata`] so the main loop can distinguish a
/// rejected manifest's row diagnostic from an operational error toast.
#[derive(Clone, Debug)]
pub(crate) enum CargoMetadataError {
    /// Workspace root no longer exists on disk between dispatch and run.
    /// Common when a worktree is deleted while a refresh is in flight.
    /// Logged at debug; never shown to the user.
    WorkspaceMissing,
    /// Cargo ran and rejected this manifest's metadata request with a nonzero exit.
    /// Discovery has already accepted its TOML; retain the diagnostic on its row.
    ManifestRejected { first_line: String },
    /// Launch, I/O, timeout and output-decoding failures retain their timed toast.
    Other(String),
}

impl From<Error> for CargoMetadataError {
    fn from(error: Error) -> Self {
        if let Error::CargoMetadata { stderr } = &error {
            let diagnostic = stderr
                .as_str()
                .into_text()
                .map_or_else(|_| stderr.clone(), |text| text.to_string());
            // exec discards exit status, and Cargo and launchers can both use 101.
            // Cargo's manifest diagnostic identifies this failure at the process boundary.
            if let Some(evidence) = diagnostic.lines().find(|line| {
                let trimmed = line.trim_start();
                trimmed
                    .strip_prefix("error: ")
                    .unwrap_or(trimmed)
                    .starts_with("failed to parse manifest at")
            }) {
                let first_line = diagnostic
                    .lines()
                    .find(|line| line.starts_with("error:"))
                    .unwrap_or(evidence)
                    .to_string();
                return Self::ManifestRejected { first_line };
            }
        }
        Self::Other(format_cargo_metadata_error(&error))
    }
}

#[derive(Clone)]
pub(super) struct StreamingScanContext {
    pub(super) source_measurements: Arc<SourceMeasurements>,
    pub(super) client:              HttpClient,
    pub(super) sender:              Sender<BackgroundMsg>,
    pub(super) disk_limit:          Arc<Semaphore>,
    pub(super) metadata_store:      Arc<Mutex<WorkspaceMetadataStore>>,
    pub(super) metadata_limit:      Arc<Semaphore>,
}

/// Spawn a streaming scan using a hybrid approach:
///
/// - **Discovery (scan thread):** Walk the directory tree, discover projects, and emit rows
///   quickly.
/// - **Local enrichment (tokio blocking pool):** Git info runs behind its own semaphore so it does
///   not block discovery.
/// - **Disk usage (tokio blocking pool):** `dir_size()` runs behind its own semaphore so disk walks
///   cannot monopolize startup.
/// - **HTTP (tokio):** CI runs, repo metadata, crates.io info, and connectivity checks run on the
///   async runtime behind a shared semaphore.
///
/// `ScanResult` is sent after discovery/local work has finished, containing the complete tree
/// and flat entries. Disk and HTTP results may continue to stream in afterward.
pub(crate) fn spawn_streaming_scan(
    scan_dirs: Vec<AbsolutePath>,
    inline_dirs: &[String],
    non_rust: NonRustInclusion,
    exclude_dirs: ExcludeDirs,
    client: HttpClient,
    metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    source_measurements: Arc<SourceMeasurements>,
) -> (Sender<BackgroundMsg>, Receiver<BackgroundMsg>) {
    let (sender, receiver) = channel::unbounded();
    let inline_dirs = inline_dirs.to_vec();

    let scan_sender = sender.clone();
    background_workers::spawn_discovery_for_roots(scan_dirs.clone(), move || {
        let scan_context = StreamingScanContext {
            source_measurements,
            client,
            sender: scan_sender.clone(),
            disk_limit: Arc::new(tokio::sync::Semaphore::new(
                config::active_config().scan.disk_concurrency.get(),
            )),
            metadata_store,
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(
                config::active_config().scan.metadata_concurrency.get(),
            )),
        };

        let phase1_started = std::time::Instant::now();
        let phase1 = discovery::phase1_discover(&scan_dirs, non_rust, &exclude_dirs);
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(phase1_started.elapsed().as_millis()),
            scan_dirs = scan_dirs.len(),
            visited_dirs = phase1.stats.visited_dirs,
            manifests = phase1.stats.manifests,
            projects = phase1.stats.projects,
            non_rust_projects = phase1.stats.non_rust_projects,
            disk_entries = phase1.disk_entries.len(),
            "phase1_discover_total"
        );

        let tree_started = std::time::Instant::now();
        let projects = tree::build_tree(&phase1.items, &inline_dirs);
        tracing::trace!(
            target: PERF_LOG_TARGET,
            elapsed_ms = tui_pane::perf_log_ms(tree_started.elapsed().as_millis()),
            input_items = phase1.items.len(),
            tree_items = projects.len(),
            "scan_tree_build"
        );

        let workspace_roots = collect_cargo_metadata_roots(&projects);
        let _ = scan_sender.send(BackgroundMsg::ScanResult {
            projects,
            disk_entries: phase1.disk_entries.clone(),
        });
        disk_usage::spawn_initial_disk_usage(&scan_context, &phase1.disk_entries);
        language_stats::spawn_initial_language_stats(&scan_context, &phase1.disk_entries);
        test_counts::spawn_initial_test_counts(&scan_context, &phase1.disk_entries);
        spawn_cargo_metadata_tree(&scan_context, workspace_roots);
    });

    (sender, receiver)
}

/// Collect distinct workspace roots that warrant a `cargo metadata`
/// dispatch — every Rust leaf project (workspace or standalone package),
/// worktree members included. Non-Rust roots are skipped.
fn collect_cargo_metadata_roots(projects: &[RootItem]) -> Vec<AbsolutePath> {
    let mut seen: HashSet<AbsolutePath> = HashSet::new();
    let mut roots = Vec::new();
    for item in projects {
        for root in cargo_metadata_roots_for_item(item) {
            if seen.insert(root.clone()) {
                roots.push(root);
            }
        }
    }
    roots
}

/// The `cargo metadata` workspace roots a single `RootItem` owns: a
/// standalone Rust project's own path, every checkout of a worktree
/// group, or nothing for a non-Rust project. Incremental insertion
/// paths dispatch a refresh per root so a project never lands in the
/// list without its metadata scheduled.
pub(crate) fn cargo_metadata_roots_for_item(item: &RootItem) -> Vec<AbsolutePath> {
    match item {
        RootItem::Rust(rust) => vec![rust.path().clone()],
        RootItem::Worktrees(group) => group.iter_paths().cloned().collect(),
        RootItem::NonRust(_) => Vec::new(),
    }
}

fn spawn_cargo_metadata_tree(scan_context: &StreamingScanContext, roots: Vec<AbsolutePath>) {
    for workspace_root in roots {
        let dispatch = MetadataDispatchContext {
            handle:         scan_context.client.handle.clone(),
            sender:         scan_context.sender.clone(),
            metadata_store: Arc::clone(&scan_context.metadata_store),
            metadata_limit: Arc::clone(&scan_context.metadata_limit),
        };
        spawn_cargo_metadata_refresh(dispatch, workspace_root);
    }
}

/// Context shared by any caller that wants to kick off a
/// `cargo metadata --no-deps --offline` task for a single workspace root.
/// The scan thread uses this to do initial dispatch; the watcher uses it
/// to re-run on manifest/config edits.
#[derive(Clone)]
pub(crate) struct MetadataDispatchContext {
    pub handle:         Handle,
    pub sender:         Sender<BackgroundMsg>,
    pub metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    pub metadata_limit: Arc<Semaphore>,
}

impl MetadataDispatchContext {
    pub(crate) fn request_disk_rescan(&self) {
        self.metadata_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .disk_usage
            .request_rescan();
    }

    pub(crate) fn disk_tracker(&self, project: &AbsolutePath) -> super::DiskUsageTracker {
        self.metadata_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .disk_usage
            .for_project(project)
    }

    pub(crate) fn target_disk_tracker(
        &self,
        project: &AbsolutePath,
        target: &AbsolutePath,
    ) -> super::DiskUsageTracker {
        self.metadata_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .disk_usage
            .for_target(project, target)
    }
    /// Lock the store briefly and clone the resolved `target_directory`
    /// for any `path` inside a known workspace. Callers that hold a live
    /// `App` should use `App::resolve_target_dir` instead; this shim
    /// exists for the watcher, which holds the dispatch context but has
    /// no direct App handle.
    pub(crate) fn resolved_target_dir(&self, path: &AbsolutePath) -> Option<AbsolutePath> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.resolved_target_dir(path).cloned())
    }
}

/// Queue a `cargo metadata` invocation for `workspace_root` on the shared
/// tokio handle. Captures the fingerprint and bumps the store's dispatch
/// generation before the blocking `exec()` fires; arrivals round-trip
/// through `BackgroundMsg::CargoMetadata` so the main loop can gate on
/// the latest generation.
pub(crate) fn spawn_cargo_metadata_refresh(
    dispatch: MetadataDispatchContext,
    workspace_root: AbsolutePath,
) {
    let owner = workspace_root;
    let handle = dispatch.handle.clone();
    let limit = Arc::clone(&dispatch.metadata_limit);
    let pending = {
        let store = dispatch
            .metadata_store
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(&store.metadata_refreshes.pending)
    };
    let submission = {
        let mut pending = pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = dispatch
            .metadata_store
            .lock()
            .map_or(0, |mut store| store.next_generation(&owner));
        pending.submit(
            owner.clone(),
            MetadataRefresh {
                dispatch,
                generation,
            },
        )
    };
    if matches!(submission, WorkSubmission::JoinedWorker) {
        return;
    }
    let mut worker = PendingWorker::new(pending, owner);
    handle.spawn(async move {
        while let NextWork::Run(refresh) = worker.next() {
            let Ok(permit) = Arc::clone(&limit).acquire_owned().await else {
                return;
            };
            execute_metadata_refresh(worker.owner(), refresh).await;
            drop(permit);
        }
    });
}

/// Own pending refreshes for one metadata store and its runtime.
#[derive(Default)]
pub(crate) struct MetadataRefreshQueue {
    pending: Arc<Mutex<PendingWork<AbsolutePath, MetadataRefresh>>>,
}

impl std::fmt::Debug for MetadataRefreshQueue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MetadataRefreshQueue")
            .finish_non_exhaustive()
    }
}

struct MetadataRefresh {
    dispatch:   MetadataDispatchContext,
    generation: u64,
}

async fn execute_metadata_refresh(workspace_root: &AbsolutePath, refresh: MetadataRefresh) {
    let metadata_timeout = config::active_config().scan.metadata_timeout();
    let MetadataRefresh {
        dispatch,
        generation,
    } = refresh;
    let workspace_root_for_task = workspace_root.clone();
    let blocking = tokio::task::spawn_blocking(move || {
        background_workers::run_metadata(|| {
            run_cargo_metadata_for_root(&workspace_root_for_task, generation)
        })
    });
    let task_result = match tokio::time::timeout(metadata_timeout, blocking).await {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => {
            tracing::warn!(workspace_root = %workspace_root.display(), "cargo_metadata_task_join_failed");
            return;
        },
        Err(_) => {
            let fingerprint = ManifestFingerprint::capture(workspace_root.as_path())
                .unwrap_or_else(|_| synthetic_fingerprint());
            CargoMetadataTaskOutput {
                generation,
                fingerprint,
                result: Err(CargoMetadataError::Other(format!(
                    "cargo metadata timed out after {}s",
                    metadata_timeout.as_secs()
                ))),
            }
        },
    };
    let CargoMetadataTaskOutput {
        generation,
        fingerprint,
        result,
    } = task_result;
    let _ = dispatch.sender.send(BackgroundMsg::CargoMetadata {
        workspace_root: workspace_root.clone(),
        generation,
        fingerprint,
        result,
    });
}

struct CargoMetadataTaskOutput {
    generation:  u64,
    fingerprint: ManifestFingerprint,
    result:      Result<WorkspaceMetadata, CargoMetadataError>,
}

/// Walk `target_dir` on a blocking thread and emit its total byte size via
/// [`BackgroundMsg::OutOfTreeTargetSize`]. Used when workspace metadata
/// reports a `target_directory` outside its `workspace_root`; the scan-time
/// walker's per-project breakdown doesn't reach there, so this fills in the
/// sharer target size for the detail pane.
pub(crate) fn spawn_out_of_tree_target_walk(
    handle: &Handle,
    sender: Sender<BackgroundMsg>,
    workspace_root: AbsolutePath,
    target_dir: AbsolutePath,
) {
    handle.spawn(async move {
        let walk_target = target_dir.clone();
        let bytes =
            tokio::task::spawn_blocking(move || disk_usage::dir_size(walk_target.as_path())).await;
        let bytes = match bytes {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    target_dir = %target_dir.display(),
                    error = %err,
                    "out_of_tree_target_walk_join_failed"
                );
                return;
            },
        };
        tracing::debug!(
            workspace_root = %workspace_root.display(),
            target_dir = %target_dir.display(),
            bytes,
            "out_of_tree_target_walk_done"
        );
        let _ = sender.send(BackgroundMsg::OutOfTreeTargetSize {
            workspace_root,
            target_dir,
            bytes,
        });
    });
}

fn run_cargo_metadata_for_root(
    workspace_root: &AbsolutePath,
    generation: u64,
) -> CargoMetadataTaskOutput {
    let fingerprint = match ManifestFingerprint::capture(workspace_root.as_path()) {
        Ok(fp) => fp,
        Err(err) => {
            // `NotFound` here means the workspace root vanished between
            // dispatch and run — the user just deleted a worktree, or a
            // similar race. Classify it as `WorkspaceMissing` so the
            // handler can suppress the toast at the type level. All other
            // I/O errors (permissions, etc.) flow into `Other`.
            let result = if err.kind() == ErrorKind::NotFound {
                Err(CargoMetadataError::WorkspaceMissing)
            } else {
                Err(CargoMetadataError::Other(format!(
                    "fingerprint capture failed: {err}"
                )))
            };
            return CargoMetadataTaskOutput {
                generation,
                fingerprint: synthetic_fingerprint(),
                result,
            };
        },
    };

    let manifest_path = workspace_root.as_path().join(CARGO_TOML);
    let started_at = std::time::Instant::now();
    let result = match execute_cargo_metadata(&manifest_path) {
        Ok(metadata) => Ok(build_workspace_metadata(
            workspace_root.clone(),
            &metadata,
            fingerprint.clone(),
        )),
        Err(err) => Err(err),
    };
    tracing::trace!(
        target: PERF_LOG_TARGET,
        elapsed_ms = tui_pane::perf_log_ms(started_at.elapsed().as_millis()),
        workspace_root = %workspace_root.display(),
        ok = result.is_ok(),
        "cargo_metadata_exec"
    );

    CargoMetadataTaskOutput {
        generation,
        fingerprint,
        result,
    }
}

fn execute_cargo_metadata(manifest_path: &Path) -> Result<Metadata, CargoMetadataError> {
    // Wall-clock cap lives on the caller via `tokio::time::timeout`;
    // `MetadataCommand::exec` itself has no timeout knob.
    let mut command = cargo_metadata::MetadataCommand::new();
    command.manifest_path(manifest_path).no_deps();
    command.other_options(vec![CARGO_OFFLINE_FLAG.to_string()]);
    execute_metadata_command(&command)
}

fn execute_metadata_command(command: &MetadataCommand) -> Result<Metadata, CargoMetadataError> {
    command.exec().map_err(CargoMetadataError::from)
}

fn format_cargo_metadata_error(err: &Error) -> String {
    let text = err.to_string();
    text.lines().next().unwrap_or(&text).to_string()
}

const fn synthetic_fingerprint() -> ManifestFingerprint {
    ManifestFingerprint {
        manifest:       FileStamp {
            content_hash: [0_u8; 32],
        },
        lockfile:       None,
        rust_toolchain: None,
        configs:        BTreeMap::new(),
    }
}

fn build_workspace_metadata(
    declared_checkout_root: AbsolutePath,
    metadata: &Metadata,
    fingerprint: ManifestFingerprint,
) -> WorkspaceMetadata {
    let target_directory =
        AbsolutePath::from(PathBuf::from(metadata.target_directory.as_std_path()));
    let packages = metadata
        .packages
        .iter()
        .map(|pkg| {
            let record = PackageRecord {
                name:          pkg.name.to_string(),
                version:       pkg.version.clone(),
                edition:       pkg.edition.to_string(),
                description:   pkg.description.clone(),
                license:       pkg.license.clone(),
                homepage:      pkg.homepage.clone(),
                repository:    pkg.repository.clone(),
                manifest_path: AbsolutePath::from(PathBuf::from(pkg.manifest_path.as_std_path())),
                targets:       pkg
                    .targets
                    .iter()
                    .map(|target| TargetRecord {
                        name:              target.name.clone(),
                        kinds:             target.kind.clone(),
                        src_path:          AbsolutePath::from(PathBuf::from(
                            target.src_path.as_std_path(),
                        )),
                        required_features: target.required_features.clone(),
                    })
                    .collect(),
                publish:       PublishPolicy::from_cargo_publish(pkg.publish.as_deref()),
            };
            (pkg.id.clone(), record)
        })
        .collect();
    WorkspaceMetadata {
        declared_checkout_root,
        target_directory,
        packages,
        fingerprint,
        out_of_tree_target_bytes: None,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tokio::runtime::Runtime;

    use super::*;
    use crate::constants::SCAN_METADATA_CONCURRENCY;
    use crate::project::Package;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::project::WorktreeStatus;
    use crate::scan::tree;

    #[test]
    fn metadata_bursts_keep_one_pending_refresh_and_the_latest_generation() -> anyhow::Result<()> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let directory = tempfile::tempdir()?;
        let root = AbsolutePath::from(directory.path());
        let (sender, receiver) = channel::unbounded();
        let store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
        let dispatch = MetadataDispatchContext {
            handle: runtime.handle().clone(),
            sender,
            metadata_store: Arc::clone(&store),
            metadata_limit: Arc::new(Semaphore::new(SCAN_METADATA_CONCURRENCY)),
        };
        for _ in 0..128 {
            spawn_cargo_metadata_refresh(dispatch.clone(), root.clone());
        }
        assert_eq!(
            runtime.metrics().num_alive_tasks(),
            1,
            "one checkout must have one pending refresh"
        );
        let message = runtime
            .block_on(async { tokio::task::spawn_blocking(move || receiver.recv()).await })??;
        let BackgroundMsg::CargoMetadata { generation, .. } = message else {
            anyhow::bail!("metadata refresh returned a different message");
        };
        assert_eq!(generation, 128);
        competing_checkout_runs_during_metadata_bursts(&runtime)?;
        Ok(())
    }

    fn competing_checkout_runs_during_metadata_bursts(runtime: &Runtime) -> anyhow::Result<()> {
        let directory = tempfile::tempdir()?;
        let mut busy_roots = Vec::new();
        for index in 0..SCAN_METADATA_CONCURRENCY {
            let path = directory.path().join(format!("busy-{index}"));
            std::fs::create_dir_all(path.join("src"))?;
            std::fs::write(path.join("src/lib.rs"), "pub fn measured() {}")?;
            std::fs::write(
                path.join(CARGO_TOML),
                format!(
                    "[package]\nname = \"busy-{index}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
                ),
            )?;
            busy_roots.push(AbsolutePath::from(path));
        }
        let competitor = AbsolutePath::from(directory.path().join("competitor"));
        let (sender, receiver) = channel::unbounded();
        let dispatch = MetadataDispatchContext {
            handle: runtime.handle().clone(),
            sender,
            metadata_store: Arc::new(Mutex::new(WorkspaceMetadataStore::new())),
            metadata_limit: Arc::new(Semaphore::new(SCAN_METADATA_CONCURRENCY)),
        };
        runtime.block_on(async move {
            for root in &busy_roots {
                spawn_cargo_metadata_refresh(dispatch.clone(), root.clone());
            }
            tokio::task::yield_now().await;
            for root in &busy_roots {
                spawn_cargo_metadata_refresh(dispatch.clone(), root.clone());
            }
            spawn_cargo_metadata_refresh(dispatch.clone(), competitor.clone());
            let replenish = tokio::spawn(async move {
                loop {
                    for root in &busy_roots {
                        spawn_cargo_metadata_refresh(dispatch.clone(), root.clone());
                    }
                    tokio::task::yield_now().await;
                }
            });
            let result = tokio::task::spawn_blocking(move || {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    let message = receiver.recv_deadline(deadline)?;
                    if let BackgroundMsg::CargoMetadata { workspace_root, .. } = message
                        && workspace_root == competitor
                    {
                        return Ok::<_, crate::channel::RecvTimeoutError>(());
                    }
                }
            })
            .await;
            replenish.abort();
            result??;
            Ok::<_, anyhow::Error>(())
        })
    }

    #[cfg(unix)]
    #[test]
    fn manifest_rejection_normalizes_colored_diagnostic_after_preamble() -> std::io::Result<()> {
        let scratch = tempfile::tempdir()?;
        let launcher = scratch.path().join("cargo-launcher");
        std::fs::write(
            &launcher,
            "#!/bin/sh\nprintf '\\033[33mlauncher preamble\\033[0m\\n\\033[33mwarning: example warning\\033[0m\\n\\033[31merror: failed to parse manifest at `/project/Cargo.toml`\\033[0m\\n\\nCaused by:\\n  inherited version has no workspace\\n' >&2\nexit 101\n",
        )?;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))?;
        let mut command = MetadataCommand::new();
        command.cargo_path(&launcher);
        assert!(matches!(execute_metadata_command(&command),
            Err(CargoMetadataError::ManifestRejected { first_line })
                if first_line == "error: failed to parse manifest at `/project/Cargo.toml`"));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn failing_cargo_launcher_remains_an_operational_error() -> std::io::Result<()> {
        let scratch = tempfile::tempdir()?;
        let launcher = scratch.path().join("cargo-launcher");
        for status in [1, 101] {
            std::fs::write(
                &launcher,
                format!("#!/bin/sh\necho 'error: toolchain is not installed' >&2\nexit {status}\n"),
            )?;
            std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))?;
            let mut command = cargo_metadata::MetadataCommand::new();
            command.cargo_path(&launcher);
            let failure = execute_metadata_command(&command);
            assert!(
                matches!(failure, Err(CargoMetadataError::Other(message)) if message.contains("toolchain is not installed")),
                "launcher failure must retain its toast"
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn missing_rustup_toolchain_remains_an_operational_error() -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        let scratch = tempfile::tempdir()?;
        let launcher = scratch.path().join("cargo-launcher");
        std::fs::write(
            &launcher,
            "#!/bin/sh\nexec rustup run cargo-port-test-missing-toolchain cargo \"$@\"\n",
        )?;
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o700))?;
        let mut command = cargo_metadata::MetadataCommand::new();
        command.cargo_path(&launcher);
        assert_eq!(
            command.cargo_command().output()?.status.code(),
            Some(1),
            "rustup rejects an uninstalled named toolchain before Cargo runs"
        );
        assert!(matches!(execute_metadata_command(&command),
            Err(CargoMetadataError::Other(message)) if message.contains("cargo-port-test-missing-toolchain") && message.contains("not installed")));
        Ok(())
    }

    #[test]
    fn metadata_errors_without_process_evidence_remain_operational() {
        let rejected = CargoMetadataError::from(Error::CargoMetadata {
            stderr: "error: cannot inherit version without a workspace\nmore context".into(),
        });
        assert!(matches!(rejected, CargoMetadataError::Other(message)
            if message == "`cargo metadata` exited with an error: error: cannot inherit version without a workspace"));
        let io = CargoMetadataError::from(Error::Io(std::io::Error::new(
            ErrorKind::PermissionDenied,
            "permission denied",
        )));
        assert!(matches!(io, CargoMetadataError::Other(message)
            if message == "failed to start `cargo metadata`: permission denied"));
        let decoding = CargoMetadataError::from(Error::NoJson);
        assert!(matches!(decoding, CargoMetadataError::Other(_)));
    }

    #[test]
    fn orphan_workspace_manifest_is_typed_as_rejected() -> std::io::Result<()> {
        let scratch = tempfile::tempdir()?;
        std::fs::create_dir(scratch.path().join("src"))?;
        std::fs::write(scratch.path().join("src/lib.rs"), "")?;
        let manifest = scratch.path().join("Cargo.toml");
        std::fs::write(
            &manifest,
            "[package]\nname = \"orphan\"\nversion.workspace = true\nedition = \"2024\"\n",
        )?;
        assert!(matches!(execute_cargo_metadata(&manifest),
            Err(CargoMetadataError::ManifestRejected { first_line }) if !first_line.is_empty()));
        Ok(())
    }

    fn status_for(is_linked_worktree: bool, primary_abs: Option<&str>) -> WorktreeStatus {
        match (is_linked_worktree, primary_abs) {
            (_, None) => WorktreeStatus::NotGit,
            (true, Some(p)) => WorktreeStatus::Linked {
                primary: AbsolutePath::from(p.to_string()),
            },
            (false, Some(p)) => WorktreeStatus::Primary {
                root: AbsolutePath::from(p.to_string()),
            },
        }
    }

    fn make_workspace(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Workspace::default()
        }))
    }

    fn make_package(
        name: Option<&str>,
        abs_path: &str,
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from(abs_path),
            name: name.map(String::from),
            worktree_status: status_for(is_linked_worktree, primary_abs),
            ..Package::default()
        }))
    }

    #[test]
    fn collect_cargo_metadata_roots_yields_one_root_per_rust_leaf() {
        let ws = make_workspace(Some("ws"), "/ws", false, Some("/ws"));
        let pkg = make_package(Some("pkg"), "/pkg", false, Some("/pkg"));
        let roots = collect_cargo_metadata_roots(&[ws, pkg]);

        assert_eq!(
            roots,
            vec![AbsolutePath::from("/ws"), AbsolutePath::from("/pkg"),],
            "each Rust leaf produces exactly one metadata root, preserving input order"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_skips_non_rust_projects() {
        let non_rust = RootItem::NonRust(crate::project::NonRustProject::new(
            AbsolutePath::from("/notes"),
            Some("notes".into()),
        ));
        let pkg = make_package(Some("pkg"), "/pkg", false, Some("/pkg"));

        let roots = collect_cargo_metadata_roots(&[non_rust, pkg]);

        assert_eq!(
            roots,
            vec![AbsolutePath::from("/pkg")],
            "non-rust leaves never receive a metadata dispatch"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_unions_primary_and_linked_worktrees() {
        // `tree::merge_worktrees_new` combines `primary`, `linked_a` and
        // `linked_b`; `collect_cargo_metadata_roots` must retain a metadata
        // root for every checkout.
        let primary = make_workspace(Some("ws"), "/ws", false, Some("/ws"));
        let linked_a = make_workspace(Some("ws_feat"), "/ws_feat", true, Some("/ws"));
        let linked_b = make_workspace(Some("ws_bug"), "/ws_bug", true, Some("/ws"));
        let mut items = vec![primary, linked_a, linked_b];
        // `tree::merge_worktrees_new` is also used by the production scan.
        tree::merge_worktrees_new(&mut items);
        assert_eq!(items.len(), 1, "merged into one worktree group");

        let mut roots = collect_cargo_metadata_roots(&items);
        roots.sort_by(|a, b| a.as_path().cmp(b.as_path()));
        assert_eq!(
            roots,
            vec![
                AbsolutePath::from("/ws"),
                AbsolutePath::from("/ws_bug"),
                AbsolutePath::from("/ws_feat"),
            ],
            "primary + every linked worktree gets its own metadata root"
        );
    }

    #[test]
    fn collect_cargo_metadata_roots_dedupes_repeated_paths() {
        // Projects normally have unique paths; `collect_cargo_metadata_roots`
        // uses its `seen` set to cheaply deduplicate repeated paths from
        // future callers.
        let pkg_a = make_package(Some("a"), "/pkg", false, Some("/pkg"));
        let pkg_b = make_package(Some("b"), "/pkg", false, Some("/pkg"));

        let roots = collect_cargo_metadata_roots(&[pkg_a, pkg_b]);
        assert_eq!(roots, vec![AbsolutePath::from("/pkg")]);
    }
}
