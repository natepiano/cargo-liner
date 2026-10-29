#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;
use std::time::Instant;

use notify::Config;
use notify::Event;
use notify::EventKind;
use notify::RecursiveMode;
use notify::Watcher;
use notify::WatcherKind;
use notify::event::CreateKind;
use notify::event::RemoveKind;

use super::refresh;
use super::roots;
use super::roots::WatchRegistration;
use super::runtime;
use super::runtime::ProjectEntry;
use crate::channel;
use crate::channel::Sender;
use crate::project::AbsolutePath;
use crate::project::FileStamp;
use crate::project::ManifestFingerprint;
use crate::project::WorkspaceMetadata;
use crate::project::WorkspaceMetadataStore;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;
use crate::test_support;

#[derive(Default)]
struct WatchCalls {
    watched:   Vec<(PathBuf, RecursiveMode)>,
    unwatched: Vec<PathBuf>,
}

impl WatchCalls {
    fn covers(&self, path: &Path) -> bool {
        self.watched.iter().any(|(watched, mode)| {
            watched == path || (*mode == RecursiveMode::Recursive && path.starts_with(watched))
        })
    }
}

impl Watcher for WatchCalls {
    fn new<F: notify::EventHandler>(_: F, _: Config) -> notify::Result<Self> { Ok(Self::default()) }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        self.watched.push((path.to_path_buf(), mode));
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.unwatched.push(path.to_path_buf());
        Ok(())
    }

    fn kind() -> WatcherKind { WatcherKind::NullWatcher }
}

fn create_folder_event(path: &Path) -> Event {
    Event::new(EventKind::Create(CreateKind::Folder)).add_path(path.to_path_buf())
}

fn metadata_dispatch(sender: Sender<BackgroundMsg>) -> MetadataDispatchContext {
    MetadataDispatchContext {
        handle: test_support::test_runtime().handle().clone(),
        sender,
        metadata_store: Arc::new(Mutex::new(WorkspaceMetadataStore::new())),
        metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
    }
}

fn record_target_directory(
    dispatch: &MetadataDispatchContext,
    project_path: AbsolutePath,
    target_directory: AbsolutePath,
) {
    let mut store = dispatch.metadata_store.lock().expect("metadata store");
    store.upsert(WorkspaceMetadata {
        declared_checkout_root: project_path,
        target_directory,
        packages: HashMap::new(),
        fingerprint: ManifestFingerprint {
            manifest:       FileStamp {
                content_hash: [0_u8; 32],
            },
            lockfile:       None,
            rust_toolchain: None,
            configs:        BTreeMap::new(),
        },
        out_of_tree_target_bytes: None,
    });
}

#[test]
fn initial_registration_skips_target_and_git_objects() {
    let scratch = tempfile::tempdir().expect("scratch root");
    for directory in [
        "src",
        "target/debug/deps",
        ".git/objects/ab",
        ".git/refs/heads",
    ] {
        std::fs::create_dir_all(scratch.path().join(directory)).expect("fixture directory");
    }
    let mut watcher = WatchCalls::default();
    let (_, failures) =
        roots::register_watch_roots(&mut watcher, &[AbsolutePath::from(scratch.path())]);

    assert!(failures.is_empty());
    assert!(
        watcher
            .watched
            .iter()
            .all(|(_, mode)| *mode == RecursiveMode::NonRecursive)
    );
    assert!(!watcher.covers(&scratch.path().join("target")));
    assert!(!watcher.covers(&scratch.path().join("target/debug/deps")));
    assert!(!watcher.covers(&scratch.path().join(".git/objects")));
    assert!(!watcher.covers(&scratch.path().join(".git/objects/ab")));
    for directory in ["", "src", ".git", ".git/refs", ".git/refs/heads"] {
        assert!(
            watcher
                .watched
                .contains(&(scratch.path().join(directory), RecursiveMode::NonRecursive,)),
            "missing watch: {directory}"
        );
    }
}

#[test]
fn created_source_folder_and_its_children_get_watches() {
    let scratch = tempfile::tempdir().expect("scratch root");
    std::fs::create_dir(scratch.path().join("src")).expect("source directory");
    let watch_roots = [AbsolutePath::from(scratch.path())];
    let mut watcher = WatchCalls::default();
    let (mut registered, failures) = roots::register_watch_roots(&mut watcher, &watch_roots);
    assert!(failures.is_empty());
    watcher.watched.clear();
    let created = scratch.path().join("src/sub");
    std::fs::create_dir_all(created.join("nested")).expect("new source directories");

    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &create_folder_event(&created),
    );

    for directory in [&created, &created.join("nested")] {
        assert!(
            watcher
                .watched
                .contains(&(directory.clone(), RecursiveMode::NonRecursive,)),
            "missing created-folder watch: {}",
            directory.display()
        );
    }
}

#[test]
fn created_folder_inside_target_gets_no_watch() {
    let scratch = tempfile::tempdir().expect("scratch root");
    std::fs::create_dir(scratch.path().join("target")).expect("build directory");
    let watch_roots = [AbsolutePath::from(scratch.path())];
    let mut watcher = WatchCalls::default();
    let (mut registered, failures) = roots::register_watch_roots(&mut watcher, &watch_roots);
    assert!(failures.is_empty());
    let initial_calls = watcher.watched.len();
    let created = scratch.path().join("target/debug/incremental/new-build");
    std::fs::create_dir_all(&created).expect("new build directory");

    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &create_folder_event(&created),
    );

    assert_eq!(
        watcher.watched.len(),
        initial_calls,
        "build output registered additional folders"
    );
    assert!(!watcher.covers(&scratch.path().join("target")));
    assert!(
        !watcher.covers(&created),
        "created build output acquired a watch"
    );
}

#[test]
fn removed_source_folder_gets_watches_when_recreated() {
    let scratch = tempfile::tempdir().expect("scratch root");
    let source = scratch.path().join("src/sub");
    let nested = source.join("nested");
    std::fs::create_dir_all(&nested).expect("source directories");
    let mut watcher = WatchCalls::default();
    let (mut registered, failures) =
        roots::register_watch_roots(&mut watcher, &[AbsolutePath::from(scratch.path())]);
    assert!(failures.is_empty());
    watcher.watched.clear();

    std::fs::remove_dir_all(&source).expect("remove source tree");
    let removed = Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(source.clone());
    roots::update_directory_watches(&mut watcher, &mut registered, &removed);
    assert!(watcher.unwatched.contains(&source));
    assert!(watcher.unwatched.contains(&nested));
    std::fs::create_dir_all(&nested).expect("recreate source tree");
    roots::update_directory_watches(&mut watcher, &mut registered, &create_folder_event(&source));

    for directory in [source, nested] {
        assert!(
            watcher
                .watched
                .contains(&(directory, RecursiveMode::NonRecursive))
        );
    }
}

#[test]
fn resolved_custom_target_is_skipped_at_registration_and_creation() {
    let scratch = tempfile::tempdir().expect("scratch root");
    for directory in [
        "workspace/src",
        "build-cache/debug/deps",
        "build-cache-old/src",
    ] {
        std::fs::create_dir_all(scratch.path().join(directory)).expect("fixture directory");
    }
    let target = AbsolutePath::from(scratch.path().join("build-cache"));
    let (background_tx, _background_rx) = channel::unbounded();
    let dispatch = metadata_dispatch(background_tx);
    let project_path = AbsolutePath::from(scratch.path().join("workspace"));
    record_target_directory(&dispatch, project_path.clone(), target.clone());
    let skips = runtime::known_target_skips(&dispatch, std::iter::once(&project_path));
    let mut watcher = WatchCalls::default();
    let (mut registered, failures) = roots::register_watch_roots_with_skips(
        &mut watcher,
        &[AbsolutePath::from(scratch.path())],
        skips,
        WatchRegistration::PerFolder,
    );

    assert!(failures.is_empty());
    assert!(!watcher.covers(&target));
    assert!(!watcher.covers(&target.join("debug/deps")));
    assert!(watcher.covers(&scratch.path().join("workspace/src")));
    assert!(watcher.covers(&scratch.path().join("build-cache-old/src")));
    let initial_calls = watcher.watched.len();
    let created = target.join("release/new-build");
    std::fs::create_dir_all(&created).expect("new custom build output");
    roots::update_directory_watches(
        &mut watcher,
        &mut registered,
        &create_folder_event(&created),
    );
    assert_eq!(watcher.watched.len(), initial_calls);
    assert!(!watcher.covers(&created));
}

#[test]
fn target_size_reaches_project_without_filesystem_events() {
    let scratch = tempfile::tempdir().expect("scratch root");
    std::fs::write(scratch.path().join("Cargo.toml"), "[workspace]\n").expect("manifest");
    let target = scratch.path().join("target/debug");
    std::fs::create_dir_all(&target).expect("build directory");
    std::fs::write(target.join("artifact"), [7_u8; 4096]).expect("build artifact");
    let project_path = AbsolutePath::from(scratch.path());
    let projects = HashMap::from([(
        project_path.clone(),
        ProjectEntry {
            label:          "scratch-project".to_string(),
            abs_path:       project_path.clone(),
            repo_root:      None,
            git_dir:        None,
            common_git_dir: None,
        },
    )]);
    let (background_tx, background_rx) = channel::unbounded();
    let (done_tx, _done_rx) = mpsc::channel();
    let runtime = test_support::test_runtime();
    let metadata_dispatch = metadata_dispatch(background_tx.clone());
    let now = Instant::now();
    let mut last_poll = now.checked_sub(Duration::from_secs(30)).unwrap();
    let mut pending_disk = HashMap::new();
    refresh::schedule_disk_poll(
        now.checked_sub(Duration::from_secs(1)).unwrap(),
        &mut last_poll,
        &projects,
        &mut pending_disk,
    );
    assert!(pending_disk.is_empty(), "slow poll ran before its interval");
    refresh::schedule_disk_poll(now, &mut last_poll, &projects, &mut pending_disk);
    refresh::fire_disk_updates(
        runtime.handle(),
        &Arc::new(tokio::sync::Semaphore::new(1)),
        &done_tx,
        &background_tx,
        &projects,
        &mut pending_disk,
        &metadata_dispatch,
    );

    let message = background_rx
        .recv_timeout(Duration::from_millis(250))
        .expect("poll publishes project disk usage without an event");
    assert!(
        matches!(message, BackgroundMsg::DiskUsage { path, bytes } if path == project_path && bytes == 4096 + 12)
    );
}
