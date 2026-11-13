use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use notify::Config;
use notify::RecursiveMode;
use notify::Watcher;
use notify::WatcherKind;

use super::ProjectEntry;
use super::WatchState;
use super::events::EventContext;
use super::runtime::GitRefreshDemand;
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
pub(super) struct RecordedFolderWatches {
    pub(super) watched:   Vec<PathBuf>,
    pub(super) unwatched: Vec<PathBuf>,
}

impl Watcher for RecordedFolderWatches {
    fn new<F: notify::EventHandler>(_: F, _: Config) -> notify::Result<Self> { Ok(Self::default()) }

    fn watch(&mut self, path: &Path, mode: RecursiveMode) -> notify::Result<()> {
        assert_eq!(mode, RecursiveMode::NonRecursive);
        self.watched.push(path.to_path_buf());
        Ok(())
    }

    fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
        self.unwatched.push(path.to_path_buf());
        Ok(())
    }

    fn kind() -> WatcherKind { WatcherKind::NullWatcher }
}

pub(super) fn target_metadata(
    sender: Sender<BackgroundMsg>,
    project_path: AbsolutePath,
    target_directory: AbsolutePath,
) -> MetadataDispatchContext {
    let mut store = WorkspaceMetadataStore::new();
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
    MetadataDispatchContext {
        handle: test_support::test_runtime().handle().clone(),
        sender,
        metadata_store: Arc::new(Mutex::new(store)),
        metadata_limit: Arc::new(tokio::sync::Semaphore::new(1)),
    }
}

pub(super) fn assert_pending_disk(states: &HashMap<String, WatchState>, project_path: &str) {
    assert!(matches!(
        states.get(project_path),
        Some(WatchState::Pending { .. })
    ));
}

pub(super) fn tracked_file_event_context<'a>(
    watch_roots: &'a [AbsolutePath],
    projects: &'a HashMap<AbsolutePath, ProjectEntry>,
    project_parents: &'a HashSet<AbsolutePath>,
    discovered: &'a HashSet<AbsolutePath>,
) -> EventContext<'a> {
    EventContext {
        watch_roots,
        projects,
        project_parents,
        discovered,
    }
}

pub(super) fn tracked_file_projects(project_dir: &Path) -> HashMap<AbsolutePath, ProjectEntry> {
    let mut projects = HashMap::new();
    projects.insert(
        AbsolutePath::from(project_dir.to_path_buf()),
        ProjectEntry {
            git_event_visibility: std::sync::Mutex::new(
                crate::project::GitEventVisibilityCache::default(),
            ),
            git_refresh_demand:   GitRefreshDemand::default(),
            label:                "~/demo".to_string(),
            abs_path:             AbsolutePath::from(project_dir.to_path_buf()),
            repo_root:            Some(AbsolutePath::from(project_dir.to_path_buf())),
            git_dir:              Some(AbsolutePath::from(project_dir.join(".git"))),
            common_git_dir:       Some(AbsolutePath::from(project_dir.join(".git"))),
        },
    );
    projects
}

#[allow(clippy::expect_used, reason = "tests panic on fixture errors")]
pub(super) fn write_tracked_file(project_dir: &Path, contents: &str) {
    std::fs::create_dir_all(project_dir.join("src")).expect("create src");
    std::fs::write(project_dir.join("Cargo.toml"), b"[package]").expect("write Cargo.toml");
    std::fs::write(project_dir.join("src").join("main.rs"), contents).expect("write main.rs");
}
