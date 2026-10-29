//! Filesystem metadata helpers.

use std::fs;
use std::path::Path;

/// Stat `target_path`, returning zero on every metadata error.
#[must_use]
pub fn file_size_or_zero(target_path: &Path) -> u64 {
    fs::metadata(target_path).map_or(0, |meta| meta.len())
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use super::*;

    #[test]
    fn metadata_errors_return_zero_and_files_report_their_length() {
        let directory = tempfile::tempdir().expect("tempdir");
        let file = directory.path().join("payload");
        assert_eq!(file_size_or_zero(&file), 0);
        fs::write(&file, b"payload").expect("write");
        assert_eq!(file_size_or_zero(&file), 7);
        assert_eq!(file_size_or_zero(&file.join("not-a-directory")), 0);
    }

    #[cfg(unix)]
    #[test]
    fn metadata_follows_symbolic_links() {
        let directory = tempfile::tempdir().expect("tempdir");
        let file = directory.path().join("payload");
        let link = directory.path().join("link");
        fs::write(&file, b"payload").expect("write");
        std::os::unix::fs::symlink(&file, &link).expect("symlink");
        assert_eq!(file_size_or_zero(&link), file_size_or_zero(&file));
    }
}
