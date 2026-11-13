use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::fs::Metadata;
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use walkdir::WalkDir;

use super::DirSizes;
use super::DiskUsageTree;
use super::disk_usage::DiskSizeAggregation;
use crate::project::AbsolutePath;

/// `DiskUsageMeasurements` shares `DiskUsageTracker` values between startup scans and watcher
/// updates.
#[derive(Debug, Default)]
pub(crate) struct DiskUsageMeasurements {
    trees: HashMap<AbsolutePath, TrackedDiskTree>,
}

#[derive(Debug)]
struct TrackedDiskTree {
    tracker: DiskUsageTracker,
    owners:  HashSet<AbsolutePath>,
}

impl DiskUsageMeasurements {
    pub(crate) fn for_tree(&mut self, tree: DiskUsageTree) -> DiskUsageTracker {
        if let Some(tracker) = self.trees.get(&tree.root_abs_path)
            && tracker.tracker.tree.entries == tree.entries
        {
            return tracker.tracker.clone();
        }
        let tracker = DiskUsageTracker {
            tree:     Arc::new(tree),
            snapshot: Arc::default(),
            changes:  Arc::default(),
        };
        self.trees.insert(
            tracker.tree.root_abs_path.clone(),
            TrackedDiskTree {
                owners:  tracker.tree.entries.iter().cloned().collect(),
                tracker: tracker.clone(),
            },
        );
        tracker
    }

    pub(crate) fn for_project(&mut self, project: &AbsolutePath) -> DiskUsageTracker {
        if let Some(tracker) = self
            .trees
            .values()
            .find(|tree| tree.tracker.tree.entries.contains(project))
        {
            return tracker.tracker.clone();
        }
        self.for_tree(DiskUsageTree {
            root_abs_path: project.clone(),
            entries:       vec![project.clone()],
        })
    }

    pub(crate) fn for_target(
        &mut self,
        project: &AbsolutePath,
        target: &AbsolutePath,
    ) -> DiskUsageTracker {
        let tracker = self.for_project(target);
        if let Some(tree) = self.trees.get_mut(&tracker.tree.root_abs_path) {
            tree.owners.insert(project.clone());
        }
        tracker
    }

    pub(crate) fn request_rescan(&self) {
        for tree in self.trees.values() {
            tree.tracker.request_rescan();
        }
    }

    pub(crate) fn retain_projects(&mut self, projects: &HashSet<AbsolutePath>) {
        self.trees.retain(|_, tree| {
            tree.owners.retain(|owner| projects.contains(owner));
            !tree.owners.is_empty()
        });
    }
}

#[derive(Debug, Default)]
enum DiskSnapshotState {
    #[default]
    Unmeasured,
    Measured(DiskUsageIndex),
}

#[derive(Debug, Default)]
enum DiskChanges {
    #[default]
    Unchanged,
    Paths(HashSet<PathBuf>),
    Rescan,
}

/// `DiskUsageTracker` retains a tree snapshot for startup scans and watcher updates.
#[derive(Clone, Debug)]
pub(crate) struct DiskUsageTracker {
    tree:     Arc<DiskUsageTree>,
    snapshot: Arc<Mutex<DiskSnapshotState>>,
    changes:  Arc<Mutex<DiskChanges>>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum DiskUpdateScope {
    WholeTree,
    ChangedPaths,
}

pub(crate) struct DiskUsageUpdate {
    pub(crate) entries:                   Vec<(AbsolutePath, DirSizes)>,
    pub(crate) visited_entries:           usize,
    pub(crate) identity_entries_examined: usize,
    pub(crate) scope:                     DiskUpdateScope,
}

impl DiskUsageTracker {
    pub(crate) fn tree(&self) -> &DiskUsageTree { &self.tree }

    pub(crate) fn record_change(&self, path: &Path) {
        self.changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record_path(path);
    }

    pub(crate) fn request_rescan(&self) {
        *self
            .changes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = DiskChanges::Rescan;
    }

    pub(crate) fn refresh(&self) -> DiskUsageUpdate {
        let changes = std::mem::take(
            &mut *self
                .changes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let update = match (&mut *snapshot, changes) {
            (DiskSnapshotState::Unmeasured, _) | (_, DiskChanges::Rescan) => {
                let index = DiskUsageIndex::scan(&self.tree.root_abs_path);
                let update = index.update(&self.tree, DiskUpdateScope::WholeTree);
                *snapshot = DiskSnapshotState::Measured(index);
                update
            },
            (DiskSnapshotState::Measured(index), changes) => {
                index.visited_entries = 0;
                index.identity_entries_examined = 0;
                if let DiskChanges::Paths(paths) = changes {
                    for path in paths {
                        index.refresh_path(&path);
                    }
                }
                index.update(&self.tree, DiskUpdateScope::ChangedPaths)
            },
        };
        drop(snapshot);
        update
    }
}

impl DiskChanges {
    fn record_path(&mut self, path: &Path) {
        match self {
            Self::Rescan => {},
            Self::Unchanged => {
                *self = Self::Paths(HashSet::from([path.to_path_buf()]));
            },
            Self::Paths(paths) => {
                paths.insert(path.to_path_buf());
            },
        }
    }
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct DiskFileIdentity {
    device: u64,
    inode:  u64,
}

#[derive(Clone, Copy, Debug)]
enum DiskModificationTime {
    Available(SystemTime),
    Unavailable,
}

#[derive(Clone, Copy, Debug)]
struct DiskFile {
    bytes:    u64,
    modified: DiskModificationTime,
    #[cfg(unix)]
    identity: DiskFileIdentity,
}

/// `DiskUsageIndex` retains file metadata for `DiskUsageTracker` path updates.
#[derive(Debug, Default)]
struct DiskUsageIndex {
    files:                     BTreeMap<PathBuf, DiskFile>,
    directories:               HashSet<PathBuf>,
    #[cfg(unix)]
    identities:                HashMap<DiskFileIdentity, HashSet<PathBuf>>,
    visited_entries:           usize,
    identity_entries_examined: usize,
}

impl DiskUsageIndex {
    fn scan(root: &Path) -> Self {
        let mut index = Self::default();
        index.scan_subtree(root);
        index
    }

    fn scan_subtree(&mut self, root: &Path) {
        for entry in WalkDir::new(root).into_iter().flatten() {
            self.visited_entries += 1;
            if entry.file_type().is_dir() {
                self.directories.insert(entry.into_path());
            } else if entry.file_type().is_file()
                && let Ok(metadata) = entry.metadata()
            {
                self.replace_file(entry.into_path(), &metadata);
            }
        }
    }

    fn forget_path(&mut self, path: &Path) {
        let removed: Vec<_> = self
            .files
            .range(path.to_path_buf()..)
            .take_while(|(file, _)| file.starts_with(path))
            .map(|(file, _)| file.clone())
            .collect();
        for file in removed {
            self.remove_file(&file);
        }
        self.directories
            .retain(|directory| !directory.starts_with(path));
    }

    fn refresh_path(&mut self, path: &Path) {
        self.visited_entries += 1;
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() => self.replace_file(path.to_path_buf(), &metadata),
            Ok(metadata) if metadata.is_dir() => self.refresh_directory(path),
            Ok(_) => self.forget_path(path),
            Err(error) if error.kind() == ErrorKind::NotFound => self.forget_path(path),
            Err(_) => {},
        }
    }

    fn refresh_directory(&mut self, path: &Path) {
        if !self.directories.contains(path) {
            self.forget_path(path);
            self.scan_subtree(path);
            return;
        }
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        let children: HashSet<_> = entries.flatten().map(|entry| entry.path()).collect();
        let removed: Vec<_> = self
            .files
            .keys()
            .chain(&self.directories)
            .filter(|child| child.parent() == Some(path) && !children.contains(*child))
            .cloned()
            .collect();
        for child in removed {
            self.forget_path(&child);
        }
        for child in children {
            self.visited_entries += 1;
            match fs::symlink_metadata(&child) {
                Ok(metadata) if metadata.is_file() => self.replace_file(child, &metadata),
                Ok(metadata) if metadata.is_dir() && !self.directories.contains(&child) => {
                    self.forget_path(&child);
                    self.scan_subtree(&child);
                },
                Ok(metadata) if metadata.is_dir() => {},
                Ok(_) => self.forget_path(&child),
                Err(error) if error.kind() == ErrorKind::NotFound => self.forget_path(&child),
                Err(_) => {},
            }
        }
    }

    fn replace_file(&mut self, path: PathBuf, metadata: &Metadata) {
        if self.directories.contains(&path) {
            self.forget_path(&path);
        }
        let file = DiskFile::from(metadata);
        #[cfg(unix)]
        if let Some(paths) = self.identities.get(&file.identity) {
            for path in paths {
                self.identity_entries_examined += 1;
                if let Some(existing) = self.files.get_mut(path) {
                    *existing = file;
                }
            }
        }
        self.remove_file(&path);
        #[cfg(unix)]
        self.identities
            .entry(file.identity)
            .or_default()
            .insert(path.clone());
        self.files.insert(path, file);
    }

    fn remove_file(&mut self, path: &Path) {
        #[cfg(unix)]
        if let Some(file) = self.files.remove(path)
            && let Some(paths) = self.identities.get_mut(&file.identity)
        {
            paths.remove(path);
            if paths.is_empty() {
                self.identities.remove(&file.identity);
            }
        }
        #[cfg(not(unix))]
        self.files.remove(path);
    }

    fn update(&self, tree: &DiskUsageTree, scope: DiskUpdateScope) -> DiskUsageUpdate {
        DiskUsageUpdate {
            entries: self.sizes(tree),
            visited_entries: self.visited_entries,
            identity_entries_examined: self.identity_entries_examined,
            scope,
        }
    }

    fn sizes(&self, tree: &DiskUsageTree) -> Vec<(AbsolutePath, DirSizes)> {
        let mut totals = DiskSizeAggregation::new(tree);
        #[cfg(unix)]
        let mut identities = HashSet::new();
        for (path, metadata) in &self.files {
            #[cfg(unix)]
            let bytes = if identities.insert(metadata.identity) {
                metadata.bytes
            } else {
                0
            };
            #[cfg(not(unix))]
            let bytes = metadata.bytes;
            let modified = match metadata.modified {
                DiskModificationTime::Available(time) => Some(time),
                DiskModificationTime::Unavailable => None,
            };
            totals.add_file(bytes, path, modified);
        }
        totals.finish()
    }
}

impl From<&Metadata> for DiskFile {
    fn from(metadata: &Metadata) -> Self {
        Self {
            bytes:                 metadata.len(),
            modified:              metadata.modified().map_or(
                DiskModificationTime::Unavailable,
                DiskModificationTime::Available,
            ),
            #[cfg(unix)]
            identity:              DiskFileIdentity {
                device: metadata.dev(),
                inode:  metadata.ino(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_log_updates_only_changed_paths() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        fs::write(directory.path().join("Cargo.toml"), "[workspace]\n")?;
        let root = AbsolutePath::from(directory.path());
        assert_session_log_updates(&root)
    }

    fn assert_session_log_updates(root: &AbsolutePath) -> std::io::Result<()> {
        let unchanged = root.join("unchanged");
        fs::create_dir(&unchanged)?;
        fs::write(unchanged.join("history"), [0_u8; 8192])?;
        let changed = root.join("session.log");
        fs::write(&changed, "initial")?;
        let tracker = DiskUsageMeasurements::default().for_tree(DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone()],
        });
        tracker.request_rescan();
        let initial = tracker.refresh();
        assert_eq!(initial.entries[0].1.total, 8192 + 12 + 7);
        for content in ["one update", "a longer session update", ""] {
            fs::write(&changed, content)?;
            tracker.record_change(&changed);
            let update = tracker.refresh();
            assert_eq!(
                update.visited_entries, 1,
                "an event scanned an unchanged tree"
            );
            assert_eq!(update.entries[0].1.total, 8192 + 12 + content.len() as u64);
            assert_eq!(
                update.entries[0].1.in_project_non_target,
                update.entries[0].1.total
            );
            assert_eq!(update.entries[0].1.in_project_target, 0);
        }
        #[cfg(unix)]
        {
            let alias = root.join("session-alias.log");
            fs::hard_link(&changed, &alias)?;
            tracker.record_change(&alias);
            assert_eq!(tracker.refresh().entries[0].1.total, 8192 + 12);
            fs::write(&changed, "linked update")?;
            tracker.record_change(&changed);
            let update = tracker.refresh();
            assert_eq!(update.visited_entries, 1);
            assert_eq!(update.identity_entries_examined, 2);
            assert_eq!(update.entries[0].1.total, 8192 + 12 + 13);
            fs::remove_file(&changed)?;
            tracker.record_change(&changed);
            assert_eq!(tracker.refresh().entries[0].1.total, 8192 + 12 + 13);
            fs::remove_file(&alias)?;
            tracker.record_change(&alias);
            assert_eq!(tracker.refresh().entries[0].1.total, 8192 + 12);
        }
        let added = root.join("new-session");
        fs::create_dir(&added)?;
        fs::write(added.join("output.log"), "child")?;
        tracker.record_change(&added);
        let update = tracker.refresh();
        assert_eq!(update.visited_entries, 3);
        assert_eq!(update.entries[0].1.total, 8192 + 12 + 5);
        fs::remove_dir_all(&added)?;
        tracker.record_change(&added);
        let update = tracker.refresh();
        assert_eq!(update.visited_entries, 1);
        assert_eq!(update.entries[0].1.total, 8192 + 12);

        Ok(())
    }

    #[test]
    fn parent_events_keep_nested_file_changes() -> std::io::Result<()> {
        let directory = tempfile::tempdir()?;
        let root = AbsolutePath::from(directory.path());
        let parent = root.join("a/deep");
        fs::create_dir_all(&parent)?;
        let file = parent.join("log");
        fs::write(&file, "old")?;
        let tracker = DiskUsageMeasurements::default().for_tree(DiskUsageTree {
            root_abs_path: root.clone(),
            entries:       vec![root.clone()],
        });
        assert_eq!(tracker.refresh().entries[0].1.total, 3);
        for (content, paths) in [
            ("longer log", [root.as_path(), file.as_path()]),
            ("short", [file.as_path(), root.as_path()]),
        ] {
            fs::write(&file, content)?;
            for path in paths {
                tracker.record_change(path);
            }
            let update = tracker.refresh();
            assert_eq!(update.entries[0].1.total, content.len() as u64);
            assert_eq!(
                update.visited_entries, 3,
                "the parent event walked a subtree"
            );
        }
        Ok(())
    }
}
