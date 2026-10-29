//! Lexical path component normalization.

use std::path::Component;
use std::path::Path;
use std::path::PathBuf;

/// Remove current-directory components and pop on parent-directory components,
/// discarding parents that cannot pop. Does not access the filesystem.
#[must_use]
pub fn normalize_path_components(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {},
            Component::ParentDir => {
                normalized.pop();
            },
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            },
        }
    }
    normalized
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_components_are_normalized_without_filesystem_access() {
        for (input, expected) in [
            ("", ""),
            (".", ""),
            ("./a/./b", "a/b"),
            ("a/b/../c", "a/c"),
            ("../../a", "a"),
            ("a/../../b", "b"),
            ("missing/../uncreated", "uncreated"),
        ] {
            assert_eq!(
                normalize_path_components(Path::new(input)),
                PathBuf::from(expected)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn absolute_roots_survive_failed_parent_pops() {
        assert_eq!(
            normalize_path_components(Path::new("/../../a/../b")),
            PathBuf::from("/b")
        );
    }

    #[cfg(windows)]
    #[test]
    fn drive_and_unc_prefixes_are_retained() {
        assert_eq!(
            normalize_path_components(Path::new(r"C:\a\..\b")),
            PathBuf::from(r"C:\b")
        );
        assert_eq!(
            normalize_path_components(Path::new(r"\\server\share\a\..\b")),
            PathBuf::from(r"\\server\share\b")
        );
    }
}
