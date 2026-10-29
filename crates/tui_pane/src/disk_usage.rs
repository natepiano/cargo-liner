//! Filesystem size walks with hard-link accounting.

#[cfg(unix)]
use std::collections::HashSet;
use std::fs::Metadata;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use walkdir::WalkDir;

/// Per-walk record of the hard-linked files a size walk has already counted.
///
/// cargo hard-links each binary, example, and test executable it builds in
/// `deps/` into the profile directory (`target/debug/hana` and
/// `target/debug/deps/hana-<hash>` are one file), and rustc hard-links
/// unchanged artifacts from one `incremental/` session directory into the
/// next. Summing `Metadata::len` per path counted those files once per link:
/// a `target/` that `du` measures at 96 GiB was reported as 150 GiB.
#[derive(Default)]
pub struct HardLinks {
    /// `(st_dev, st_ino)` of every multiply-linked file already counted.
    #[cfg(unix)]
    counted: HashSet<(u64, u64)>,
}

impl HardLinks {
    /// Bytes `metadata`'s file adds to the walk: its length the first time the
    /// walk reaches its inode, zero at every later link to that inode. Non-unix
    /// `Metadata` carries no inode identity, so there every path counts.
    pub fn unseen_bytes(&mut self, metadata: &Metadata) -> u64 {
        #[cfg(unix)]
        if metadata.nlink() > 1 && !self.counted.insert((metadata.dev(), metadata.ino())) {
            return 0;
        }
        metadata.len()
    }
}

/// Bytes held by the files under `path`, each hard-linked file counted once.
#[must_use]
pub fn dir_size(path: &Path) -> u64 {
    let mut hard_links = HardLinks::default();
    WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file())
        .filter_map(|entry| entry.metadata().ok())
        .map(|metadata| hard_links.unseen_bytes(&metadata))
        .sum()
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn walks_sum_nested_files_skip_missing_paths_and_start_fresh() {
        let directory = tempfile::tempdir().expect("tempdir");
        let nested = directory.path().join("nested");
        fs::create_dir(&nested).expect("mkdir");
        fs::write(directory.path().join("a"), b"abc").expect("write");
        fs::write(nested.join("b"), b"defgh").expect("write");
        assert_eq!(dir_size(&directory.path().join("missing")), 0);
        assert_eq!(dir_size(directory.path()), 8);
        assert_eq!(dir_size(directory.path()), 8);
    }

    #[cfg(unix)]
    #[test]
    fn counters_deduplicate_linked_inodes_but_independent_counters_do_not() {
        let directory = tempfile::tempdir().expect("tempdir");
        let file = directory.path().join("a");
        let link = directory.path().join("b");
        fs::write(&file, b"payload").expect("write");
        fs::hard_link(&file, &link).expect("hard link");
        let metadata = fs::metadata(&file).expect("metadata");
        let linked_metadata = fs::metadata(&link).expect("metadata");
        let mut counter = HardLinks::default();
        assert_eq!(counter.unseen_bytes(&metadata), 7);
        assert_eq!(counter.unseen_bytes(&linked_metadata), 0);
        assert_eq!(counter.unseen_bytes(&metadata), 0);
        assert_eq!(HardLinks::default().unseen_bytes(&linked_metadata), 7);
        assert_eq!(dir_size(directory.path()), 7);
    }

    #[cfg(unix)]
    #[test]
    fn walks_do_not_follow_child_symbolic_links() {
        let directory = tempfile::tempdir().expect("tempdir");
        let target = tempfile::tempdir().expect("tempdir");
        fs::write(target.path().join("payload"), b"payload").expect("write");
        std::os::unix::fs::symlink(target.path(), directory.path().join("link")).expect("symlink");
        assert_eq!(dir_size(directory.path()), 0);
    }

    #[cfg(not(unix))]
    #[test]
    fn counters_count_every_metadata_visit_without_inode_identity() {
        let directory = tempfile::tempdir().expect("tempdir");
        let file = directory.path().join("payload");
        fs::write(&file, b"payload").expect("write");
        let metadata = fs::metadata(file).expect("metadata");
        let mut counter = HardLinks::default();
        assert_eq!(counter.unseen_bytes(&metadata), 7);
        assert_eq!(counter.unseen_bytes(&metadata), 7);
    }
}
