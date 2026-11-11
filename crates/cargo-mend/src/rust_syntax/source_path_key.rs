use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use rustc_hash::FxHashMap;

use super::module_map;

/// A source identity key resolves symlinks in its existing path prefix.
/// Missing path components retain their normalized spelling.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SourcePathKey(PathBuf);

impl SourcePathKey {
    fn new(path: &Path) -> Self {
        if let Ok(resolved) = fs::canonicalize(path) {
            return Self(resolved);
        }
        let normalized = module_map::lexically_normalized(path);
        for ancestor in normalized.ancestors() {
            if let Ok(resolved) = fs::canonicalize(ancestor)
                && let Ok(suffix) = normalized.strip_prefix(ancestor)
            {
                return Self(resolved.join(suffix));
            }
        }
        Self(normalized)
    }

    pub(crate) fn as_path(&self) -> &Path { &self.0 }

    pub(crate) fn into_path_buf(self) -> PathBuf { self.0 }
}

/// Path spellings share one resolved identity during a source snapshot.
#[derive(Default)]
pub(super) struct SourcePathKeys {
    keys: RefCell<FxHashMap<PathBuf, SourcePathKey>>,
}

impl SourcePathKeys {
    pub(super) fn key(&self, path: &Path) -> SourcePathKey {
        if let Some(key) = self.keys.borrow().get(path) {
            return key.clone();
        }
        let key = SourcePathKey::new(path);
        let mut keys = self.keys.borrow_mut();
        keys.insert(key.as_path().to_path_buf(), key.clone());
        keys.insert(path.to_path_buf(), key.clone());
        key
    }
}
