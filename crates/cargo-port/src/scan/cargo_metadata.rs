use std::collections::HashSet;
use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;

use cargo_metadata::Error;
use cargo_metadata::Metadata;
use tokio::runtime::Handle;
use tokio::sync::Semaphore;
use walkdir::WalkDir;

use super::BackgroundMsg;
use super::discovery::phase1_discover;
use super::disk_usage::spawn_initial_disk_usage;
use super::language_stats::spawn_initial_language_stats;
use super::tree::build_tree;
use crate::config::NonRustInclusion;
use crate::constants::CARGO_METADATA_TIMEOUT;
use crate::constants::SCAN_DISK_CONCURRENCY;
use crate::constants::SCAN_METADATA_CONCURRENCY;
use crate::http::HttpClient;
use crate::project::AbsolutePath;
use crate::project::ManifestFingerprint;
use crate::project::PackageRecord;
use crate::project::PublishPolicy;
use crate::project::RootItem;
use crate::project::TargetRecord;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;

/// Structured failure for a `cargo metadata` invocation. Held inside
/// [`BackgroundMsg::CargoMetadata`] so the main loop can raise a keyed
/// error toast and leave the affected rows in fallback state.
///
/// Variant chosen deliberately so the handler dispatches on cause rather
/// than string-matching: `WorkspaceMissing` is the expected race when the
/// user just deleted a worktree (no toast — the workspace is gone), and
/// `Other` is a real failure surface that needs to be visible.
#[derive(Clone, Debug)]
pub(crate) enum CargoMetadataError {
    /// Workspace root no longer exists on disk between dispatch and run.
    /// Common when a worktree is deleted while a refresh is in flight.
    /// Logged at debug; never shown to the user.
    WorkspaceMissing,
    /// All other failures: cargo subprocess errors, timeouts, parse
    /// failures. Shown verbatim in a timed error toast.
    Other(String),
}

impl CargoMetadataError {
    /// Message body for the error toast — only meaningful for `Other`.
    pub(crate) const fn user_facing_message(&self) -> Option<&str> {
        match self {
            Self::WorkspaceMissing => None,
            Self::Other(message) => Some(message.as_str()),
        }
    }
}

#[derive(Clone)]
pub(super) struct StreamingScanContext {
    pub(super) client:         HttpClient,
    pub(super) tx:             Sender<BackgroundMsg>,
    pub(super) disk_limit:     Arc<Semaphore>,
    pub(super) metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    pub(super) metadata_limit: Arc<Semaphore>,
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
    client: HttpClient,
    metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
) -> (Sender<BackgroundMsg>, Receiver<BackgroundMsg>) {
    let (tx, rx) = mpsc::channel();
    let inline_dirs = inline_dirs.to_vec();

    let scan_tx = tx.clone();
    thread::spawn(move || {
        let scan_context = StreamingScanContext {
            client,
            tx: scan_tx.clone(),
            disk_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_DISK_CONCURRENCY)),
            metadata_store,
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_METADATA_CONCURRENCY)),
        };

        let phase1_started = std::time::Instant::now();
        let phase1 = phase1_discover(&scan_dirs, non_rust);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(phase1_started.elapsed().as_millis()),
            scan_dirs = scan_dirs.len(),
            visited_dirs = phase1.stats.visited_dirs,
            manifests = phase1.stats.manifests,
            projects = phase1.stats.projects,
            non_rust_projects = phase1.stats.non_rust_projects,
            disk_entries = phase1.disk_entries.len(),
            "phase1_discover_total"
        );

        let tree_started = std::time::Instant::now();
        let projects = build_tree(&phase1.items, &inline_dirs);
        tracing::info!(
            elapsed_ms = crate::perf_log::ms(tree_started.elapsed().as_millis()),
            input_items = phase1.items.len(),
            tree_items = projects.len(),
            "scan_tree_build"
        );

        let workspace_roots = collect_cargo_metadata_roots(&projects);
        let _ = scan_tx.send(BackgroundMsg::ScanResult {
            projects,
            disk_entries: phase1.disk_entries.clone(),
        });
        spawn_initial_disk_usage(&scan_context, &phase1.disk_entries);
        spawn_initial_language_stats(&scan_context, &phase1.disk_entries);
        spawn_cargo_metadata_tree(&scan_context, workspace_roots);
    });

    (tx, rx)
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

fn cargo_metadata_roots_for_item(item: &RootItem) -> Vec<AbsolutePath> {
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
            tx:             scan_context.tx.clone(),
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
    pub tx:             Sender<BackgroundMsg>,
    pub metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    pub metadata_limit: Arc<Semaphore>,
}

impl MetadataDispatchContext {
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
    let MetadataDispatchContext {
        handle,
        tx,
        metadata_store: store,
        metadata_limit: limit,
    } = dispatch;

    handle.spawn(async move {
        let Ok(_permit) = limit.acquire_owned().await else {
            return;
        };

        let workspace_root_for_task = workspace_root.clone();
        let blocking = tokio::task::spawn_blocking(move || {
            run_cargo_metadata_for_root(&workspace_root_for_task, &store)
        });
        let task_result = match tokio::time::timeout(CARGO_METADATA_TIMEOUT, blocking).await {
            Ok(Ok(output)) => output,
            Ok(Err(_)) => {
                tracing::warn!(
                    workspace_root = %workspace_root.display(),
                    "cargo_metadata_task_join_failed"
                );
                return;
            },
            Err(_) => {
                let fingerprint = ManifestFingerprint::capture(workspace_root.as_path())
                    .unwrap_or_else(|_| synthetic_fingerprint());
                CargoMetadataTaskOutput {
                    generation: 0,
                    fingerprint,
                    result: Err(CargoMetadataError::Other(format!(
                        "cargo metadata timed out after {}s",
                        CARGO_METADATA_TIMEOUT.as_secs()
                    ))),
                }
            },
        };

        let CargoMetadataTaskOutput {
            generation,
            fingerprint,
            result,
        } = task_result;
        let _ = tx.send(BackgroundMsg::CargoMetadata {
            workspace_root,
            generation,
            fingerprint,
            result,
        });
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
    tx: Sender<BackgroundMsg>,
    workspace_root: AbsolutePath,
    target_dir: AbsolutePath,
) {
    handle.spawn(async move {
        let walk_target = target_dir.clone();
        let bytes = tokio::task::spawn_blocking(move || sum_dir_bytes(walk_target.as_path())).await;
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
        let _ = tx.send(BackgroundMsg::OutOfTreeTargetSize {
            workspace_root,
            target_dir,
            bytes,
        });
    });
}

fn sum_dir_bytes(dir: &Path) -> u64 {
    WalkDir::new(dir)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok().map(|meta| meta.len()))
        .sum()
}

fn run_cargo_metadata_for_root(
    workspace_root: &AbsolutePath,
    store: &Arc<Mutex<WorkspaceMetadataStore>>,
) -> CargoMetadataTaskOutput {
    let generation = store
        .lock()
        .map_or(0, |mut guard| guard.next_generation(workspace_root));
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

    let manifest_path = workspace_root.as_path().join("Cargo.toml");
    let started_at = std::time::Instant::now();
    let result = match execute_cargo_metadata(&manifest_path) {
        Ok(metadata) => Ok(build_workspace_metadata(
            workspace_root.clone(),
            &metadata,
            fingerprint.clone(),
        )),
        Err(err) => Err(err),
    };
    tracing::info!(
        elapsed_ms = crate::perf_log::ms(started_at.elapsed().as_millis()),
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
    let mut cmd = cargo_metadata::MetadataCommand::new();
    cmd.manifest_path(manifest_path).no_deps();
    cmd.other_options(vec!["--offline".to_string()]);
    cmd.exec()
        .map_err(|err| CargoMetadataError::Other(format_cargo_metadata_error(&err)))
}

fn format_cargo_metadata_error(err: &Error) -> String {
    let text = err.to_string();
    text.lines().next().unwrap_or(&text).to_string()
}

const fn synthetic_fingerprint() -> ManifestFingerprint {
    use std::collections::BTreeMap;

    use crate::project::FileStamp;
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
    workspace_root: AbsolutePath,
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
                        name:     target.name.clone(),
                        kinds:    target.kind.clone(),
                        src_path: AbsolutePath::from(PathBuf::from(target.src_path.as_std_path())),
                    })
                    .collect(),
                publish:       PublishPolicy::from_cargo_publish(pkg.publish.as_deref()),
            };
            (pkg.id.clone(), record)
        })
        .collect();
    WorkspaceMetadata {
        workspace_root,
        target_directory,
        packages,
        fingerprint,
        out_of_tree_target_bytes: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::Package;
    use crate::project::RustProject;
    use crate::project::Workspace;
    use crate::scan::tree::merge_worktrees_new;

    fn status_for(
        is_linked_worktree: bool,
        primary_abs: Option<&str>,
    ) -> crate::project::WorktreeStatus {
        match (is_linked_worktree, primary_abs) {
            (_, None) => crate::project::WorktreeStatus::NotGit,
            (true, Some(p)) => crate::project::WorktreeStatus::Linked {
                primary: AbsolutePath::from(p.to_string()),
            },
            (false, Some(p)) => crate::project::WorktreeStatus::Primary {
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
        // Merge a primary + two linked worktrees into a group, then assert
        // every worktree gets its own metadata root.
        let primary = make_workspace(Some("ws"), "/ws", false, Some("/ws"));
        let linked_a = make_workspace(Some("ws_feat"), "/ws_feat", true, Some("/ws"));
        let linked_b = make_workspace(Some("ws_bug"), "/ws_bug", true, Some("/ws"));
        let mut items = vec![primary, linked_a, linked_b];
        // Use the merge logic the production scan uses.
        merge_worktrees_new(&mut items);
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
        // Shouldn't happen in practice (each project has a unique path),
        // but the deduping logic is cheap and catches any future caller
        // that accidentally feeds the same root twice.
        let pkg_a = make_package(Some("a"), "/pkg", false, Some("/pkg"));
        let pkg_b = make_package(Some("b"), "/pkg", false, Some("/pkg"));

        let roots = collect_cargo_metadata_roots(&[pkg_a, pkg_b]);
        assert_eq!(roots, vec![AbsolutePath::from("/pkg")]);
    }
}
