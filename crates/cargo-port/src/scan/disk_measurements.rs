use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ffi::OsString;
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

use totals::ContributionChange;
use totals::DiskTreeTotals;
use walkdir::WalkDir;

use super::DirSizes;
use super::DiskUsageTree;
use crate::project::AbsolutePath;

#[path = "disk_totals.rs"]
mod totals;

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
    Measured(Box<DiskUsageIndex>),
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
    pub(crate) entries:                      Vec<(AbsolutePath, DirSizes)>,
    pub(crate) visited_entries:              usize,
    pub(crate) identity_entries_examined:    usize,
    pub(crate) aggregation_entries_examined: usize,
    pub(crate) scope:                        DiskUpdateScope,
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
                let index = DiskUsageIndex::scan(&self.tree);
                let update = index.update(&self.tree, DiskUpdateScope::WholeTree);
                *snapshot = DiskSnapshotState::Measured(Box::new(index));
                update
            },
            (DiskSnapshotState::Measured(index), changes) => {
                index.visited_entries = 0;
                index.identity_entries_examined = 0;
                index.aggregation_entries_examined = 0;
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
#[derive(Debug)]
struct DiskUsageIndex {
    files:                        BTreeMap<PathBuf, DiskFile>,
    directories:                  BTreeSet<PathBuf>,
    directory_children:           BTreeMap<PathBuf, HashSet<OsString>>,
    #[cfg(unix)]
    identities:                   HashMap<DiskFileIdentity, HashSet<PathBuf>>,
    visited_entries:              usize,
    identity_entries_examined:    usize,
    aggregation_entries_examined: usize,
    totals:                       DiskTreeTotals,
}

impl DiskUsageIndex {
    fn scan(tree: &DiskUsageTree) -> Self {
        let mut index = Self {
            files:                        BTreeMap::new(),
            directories:                  BTreeSet::new(),
            directory_children:           BTreeMap::new(),
            #[cfg(unix)]
            identities:                   HashMap::new(),
            visited_entries:              0,
            identity_entries_examined:    0,
            aggregation_entries_examined: 0,
            totals:                       DiskTreeTotals::new(tree),
        };
        index.scan_subtree(&tree.root_abs_path);
        index
    }

    fn scan_subtree(&mut self, root: &Path) {
        for entry in WalkDir::new(root).into_iter().flatten() {
            self.visited_entries += 1;
            if entry.file_type().is_dir() {
                let directory = entry.into_path();
                self.remember_child(&directory);
                self.directories.insert(directory);
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
        let directories: Vec<_> = self
            .directories
            .range(path.to_path_buf()..)
            .take_while(|directory| directory.starts_with(path))
            .cloned()
            .collect();
        for directory in directories {
            self.directories.remove(&directory);
        }
        let parents: Vec<_> = self
            .directory_children
            .range(path.to_path_buf()..)
            .take_while(|(parent, _)| parent.starts_with(path))
            .map(|(parent, _)| parent.clone())
            .collect();
        for parent in parents {
            self.directory_children.remove(&parent);
        }
        self.forget_child(path);
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
            .directory_children
            .get(path)
            .into_iter()
            .flat_map(|names| names.iter())
            .map(|name| path.join(Path::new(name)))
            .filter(|child| !children.contains(child))
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
        let affected = self
            .file_aliases(&path)
            .into_iter()
            .chain(
                self.identities
                    .get(&file.identity)
                    .into_iter()
                    .flatten()
                    .cloned(),
            )
            .collect();
        #[cfg(not(unix))]
        let affected = HashSet::from([path.clone()]);
        self.change_contributions(&affected, ContributionChange::Removed);
        #[cfg(unix)]
        if let Some(paths) = self.identities.get(&file.identity) {
            for path in paths {
                self.identity_entries_examined += 1;
                if let Some(existing) = self.files.get_mut(path) {
                    *existing = file;
                }
            }
        }
        self.remove_file_index(&path);
        #[cfg(unix)]
        self.identities
            .entry(file.identity)
            .or_default()
            .insert(path.clone());
        self.remember_child(&path);
        self.files.insert(path, file);
        self.change_contributions(&affected, ContributionChange::Added);
    }

    #[cfg(unix)]
    fn file_aliases(&self, path: &Path) -> HashSet<PathBuf> {
        self.files
            .get(path)
            .and_then(|file| self.identities.get(&file.identity))
            .cloned()
            .unwrap_or_else(|| HashSet::from([path.to_path_buf()]))
    }

    fn change_contributions(&mut self, paths: &HashSet<PathBuf>, change: ContributionChange) {
        for path in paths {
            let Some(file) = self.files.get(path).copied() else {
                continue;
            };
            self.aggregation_entries_examined += 1;
            #[cfg(unix)]
            let bytes = if self
                .identities
                .get(&file.identity)
                .and_then(|aliases| aliases.iter().min())
                == Some(path)
            {
                file.bytes
            } else {
                0
            };
            #[cfg(not(unix))]
            let bytes = file.bytes;
            self.totals.change_file(path, bytes, file.modified, change);
        }
    }

    fn remove_file(&mut self, path: &Path) {
        #[cfg(unix)]
        let affected = self.file_aliases(path);
        #[cfg(not(unix))]
        let affected = HashSet::from([path.to_path_buf()]);
        self.change_contributions(&affected, ContributionChange::Removed);
        self.remove_file_index(path);
        self.change_contributions(&affected, ContributionChange::Added);
    }

    fn remove_file_index(&mut self, path: &Path) {
        self.forget_child(path);
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

    fn remember_child(&mut self, path: &Path) {
        if let Some(parent) = path.parent()
            && let Some(name) = path.file_name()
        {
            self.directory_children
                .entry(parent.to_path_buf())
                .or_default()
                .insert(name.to_os_string());
        }
    }

    fn forget_child(&mut self, path: &Path) {
        if let Some(parent) = path.parent()
            && let Some(name) = path.file_name()
            && let Some(children) = self.directory_children.get_mut(parent)
        {
            children.remove(name);
        }
    }

    fn update(&self, tree: &DiskUsageTree, scope: DiskUpdateScope) -> DiskUsageUpdate {
        DiskUsageUpdate {
            entries: self.sizes(tree),
            visited_entries: self.visited_entries,
            identity_entries_examined: self.identity_entries_examined,
            aggregation_entries_examined: self.aggregation_entries_examined,
            scope,
        }
    }

    fn sizes(&self, tree: &DiskUsageTree) -> Vec<(AbsolutePath, DirSizes)> {
        self.totals.sizes(tree)
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
        assert_source_time_changes(root, &tracker)?;
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
        #[cfg(unix)]
        assert_canonical_alias_removal(root, &tracker)?;
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

    fn assert_source_time_changes(
        root: &AbsolutePath,
        tracker: &DiskUsageTracker,
    ) -> std::io::Result<()> {
        let manifest = root.join("Cargo.toml");
        for seconds in [1_700_000_090, 1_700_000_000] {
            let modified = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
            fs::File::open(&manifest)?.set_times(fs::FileTimes::new().set_modified(modified))?;
            tracker.record_change(&manifest);
            let update = tracker.refresh();
            assert_eq!(update.entries[0].1.max_source_mtime, Some(modified));
            assert!(
                update.aggregation_entries_examined <= 2,
                "a source time change read unrelated file records"
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    fn assert_canonical_alias_removal(
        root: &AbsolutePath,
        tracker: &DiskUsageTracker,
    ) -> std::io::Result<()> {
        let original = root.join("linked.bin");
        let target = root.join("target");
        fs::create_dir(&target)?;
        fs::write(&original, [0_u8; 11])?;
        fs::hard_link(&original, target.join("linked.bin"))?;
        tracker.record_change(&original);
        tracker.record_change(&target);
        let before = tracker.refresh().entries[0].1;
        assert_eq!(before.in_project_target, 0);
        fs::remove_file(&original)?;
        tracker.record_change(&original);
        let update = tracker.refresh();
        assert_eq!(update.entries[0].1.total, before.total);
        assert_eq!(update.entries[0].1.in_project_target, 11);
        assert_eq!(update.entries[0].1.in_project_non_target, before.total - 11);
        assert!(
            update.aggregation_entries_examined <= 3,
            "an alias removal read unrelated file records"
        );
        fs::remove_dir_all(&target)?;
        tracker.record_change(&target);
        assert_eq!(tracker.refresh().entries[0].1.total, before.total - 11);
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
