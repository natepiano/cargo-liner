//! Retain file measurements while their project owns them. Walks still apply
//! current ignore rules; unchanged files reuse counts instead of parser buffers.

use std::collections::HashMap;
use std::collections::HashSet;
use std::fs;
use std::io;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::SystemTime;

use super::language_stats::LanguageMeasurements;
use super::test_counts::TestMeasurements;
use crate::project::AbsolutePath;

#[derive(Default)]
pub(crate) struct SourceMeasurements {
    submodule_owners: Mutex<HashMap<AbsolutePath, AbsolutePath>>,
    languages:        Mutex<HashMap<AbsolutePath, Arc<Mutex<LanguageMeasurements>>>>,
    tests:            Mutex<HashMap<AbsolutePath, Arc<Mutex<TestMeasurements>>>>,
}

impl SourceMeasurements {
    pub(super) fn languages(&self, root: &AbsolutePath) -> Arc<Mutex<LanguageMeasurements>> {
        self.languages.lock().map_or_else(
            |_| Arc::default(),
            |mut roots| Arc::clone(roots.entry(root.clone()).or_default()),
        )
    }

    pub(super) fn tests(&self, root: &AbsolutePath) -> Arc<Mutex<TestMeasurements>> {
        self.tests.lock().map_or_else(
            |_| Arc::default(),
            |mut roots| Arc::clone(roots.entry(root.clone()).or_default()),
        )
    }

    pub(crate) fn replace_submodules(&self, parent: &AbsolutePath, children: &[AbsolutePath]) {
        let mut owners = self
            .submodule_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let removed: Vec<_> = owners
            .iter()
            .filter(|(child, owner)| *owner == parent && !children.contains(child))
            .map(|(child, _)| child.clone())
            .collect();
        owners.retain(|_, owner| owner != parent);
        for child in children {
            owners.insert(child.clone(), parent.clone());
        }
        if let Ok(mut languages) = self.languages.lock() {
            for child in &removed {
                languages.remove(child);
            }
        }
        if let Ok(mut tests) = self.tests.lock() {
            for child in &removed {
                tests.remove(child);
            }
        }
        drop(owners);
    }

    pub(crate) fn retain_projects(&self, mut owns: impl FnMut(&AbsolutePath) -> bool) {
        let mut owners = self
            .submodule_owners
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owners.retain(|_, parent| owns(parent));
        if let Ok(mut languages) = self.languages.lock() {
            languages.retain(|root, _| owns(root) || owners.contains_key(root));
        }
        if let Ok(mut tests) = self.tests.lock() {
            tests.retain(|root, _| owns(root) || owners.contains_key(root));
        }
        drop(owners);
    }

    #[cfg(test)]
    pub(crate) fn observe_test_owner_for_test(&self, root: &AbsolutePath) { self.tests(root); }

    #[cfg(test)]
    pub(crate) fn has_test_owner_for_test(&self, root: &AbsolutePath) -> bool {
        self.tests
            .lock()
            .is_ok_and(|roots| roots.contains_key(root))
    }

    #[cfg(test)]
    pub(crate) fn has_language_owner_for_test(&self, root: &AbsolutePath) -> bool {
        self.languages
            .lock()
            .is_ok_and(|roots| roots.contains_key(root))
    }
}

#[derive(Clone, Eq, PartialEq)]
struct SourceRevision {
    modified: SystemTime,
    length:   u64,
    #[cfg(unix)]
    identity: FileIdentity,
}

#[cfg(unix)]
#[derive(Clone, Eq, PartialEq)]
struct FileIdentity {
    device:              u64,
    inode:               u64,
    changed_seconds:     i64,
    changed_nanoseconds: i64,
}

impl SourceRevision {
    fn read(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        Ok(Self {
            modified:              metadata.modified()?,
            length:                metadata.len(),
            #[cfg(unix)]
            identity:              {
                FileIdentity {
                    device:              metadata.dev(),
                    inode:               metadata.ino(),
                    changed_seconds:     metadata.ctime(),
                    changed_nanoseconds: metadata.ctime_nsec(),
                }
            },
        })
    }
}

struct MeasuredSource<T> {
    revision: SourceRevision,
    counts:   T,
}

pub(super) struct SourceFileMeasurements<T> {
    files:             HashMap<PathBuf, MeasuredSource<T>>,
    pub(super) reads:  usize,
    pub(super) reuses: usize,
}

impl<T> SourceFileMeasurements<T> {
    pub(super) fn files_len(&self) -> usize { self.files.len() }
}

impl<T> Default for SourceFileMeasurements<T> {
    fn default() -> Self {
        Self {
            files:  HashMap::new(),
            reads:  0,
            reuses: 0,
        }
    }
}

impl<T: Clone> SourceFileMeasurements<T> {
    pub(super) fn measure(
        &mut self,
        path: &Path,
        read: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        let revision = SourceRevision::read(path)?;
        if let Some(measured) = self.files.get(path)
            && measured.revision == revision
        {
            self.reuses += 1;
            return Ok(measured.counts.clone());
        }
        self.reads += 1;
        let counts = read()?;
        // A concurrent save must not attach older counts to the newer revision.
        if SourceRevision::read(path).is_ok_and(|after| after == revision) {
            self.files.insert(
                path.to_path_buf(),
                MeasuredSource {
                    revision,
                    counts: counts.clone(),
                },
            );
        } else {
            self.files.remove(path);
        }
        Ok(counts)
    }

    pub(super) fn retain_seen(&mut self, paths: &HashSet<PathBuf>) {
        self.files.retain(|path, _| paths.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submodule_counts_follow_the_declared_parent_until_details_resolve() {
        let parent = AbsolutePath::from("/tmp/source-owner");
        let children = [parent.join("sub-a").into(), parent.join("sub-b").into()];
        let measurements = SourceMeasurements::default();
        measurements.replace_submodules(&parent, &children);
        for child in &children {
            measurements.tests(child);
        }
        measurements.retain_projects(|root| root == &parent);
        for child in &children {
            assert!(measurements.has_test_owner_for_test(child));
        }
        measurements.replace_submodules(&parent, &children[1..]);
        assert!(!measurements.has_test_owner_for_test(&children[0]));
        assert!(measurements.has_test_owner_for_test(&children[1]));
        measurements.retain_projects(|_| false);
        assert!(!measurements.has_test_owner_for_test(&children[1]));
    }

    #[test]
    fn unchanged_files_reuse_counts_and_only_edits_are_read() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let a = directory.path().join("a.rs");
        let b = directory.path().join("b.rs");
        fs::write(&a, "a")?;
        fs::write(&b, "b")?;
        let mut measurements = SourceFileMeasurements::default();
        for _ in 0..128 {
            for path in [&a, &b] {
                measurements.measure(path, || fs::read_to_string(path))?;
            }
        }
        assert_eq!(measurements.reads, 2);
        assert_eq!(measurements.reuses, 254);
        fs::write(&a, "edited source")?;
        assert_eq!(
            measurements.measure(&a, || fs::read_to_string(&a))?,
            "edited source"
        );
        measurements.measure(&b, || fs::read_to_string(&b))?;
        assert_eq!(measurements.reads, 3);
        assert_eq!(measurements.files.len(), 2);
        Ok(())
    }

    #[test]
    fn removed_files_and_project_owners_do_not_accumulate() -> io::Result<()> {
        let directory = tempfile::tempdir()?;
        let mut measurements = SourceFileMeasurements::default();
        let owners = SourceMeasurements::default();
        for index in 0..128 {
            let path = directory.path().join(format!("{index}.rs"));
            fs::write(&path, "source")?;
            measurements.measure(&path, || fs::read_to_string(&path))?;
            measurements.retain_seen(&HashSet::from([path.clone()]));
            assert_eq!(measurements.files.len(), 1);
            let root = AbsolutePath::from(directory.path().join(index.to_string()));
            owners.languages(&root);
            owners.tests(&root);
            owners.retain_projects(|path| path == &root);
            fs::remove_file(path)?;
        }
        assert_eq!(
            owners
                .languages
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?
                .len(),
            1
        );
        assert_eq!(
            owners
                .tests
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?
                .len(),
            1
        );
        owners.retain_projects(|_| false);
        assert!(
            owners
                .languages
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?
                .is_empty()
        );
        assert!(
            owners
                .tests
                .lock()
                .map_err(|error| io::Error::other(error.to_string()))?
                .is_empty()
        );
        Ok(())
    }
}
