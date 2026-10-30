use std::collections::BTreeMap;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use notify::Config;
use notify::RecursiveMode;
use notify::Watcher;
use notify::WatcherKind;

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
