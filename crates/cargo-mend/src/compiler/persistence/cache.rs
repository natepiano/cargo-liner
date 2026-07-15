use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;

use crate::constants::FINGERPRINT_HEX_WIDTH;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum CacheBuildKind {
    Library,
    Test,
}

pub fn cache_filename_for(
    package_root: &Path,
    crate_root_file: &Path,
    build_kind: CacheBuildKind,
) -> String {
    let mut hasher = DefaultHasher::new();
    package_root.hash(&mut hasher);
    crate_root_file.hash(&mut hasher);
    build_kind.hash(&mut hasher);
    format!(
        "{:0width$x}.json",
        hasher.finish(),
        width = FINGERPRINT_HEX_WIDTH
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::CacheBuildKind;
    use super::cache_filename_for;

    #[test]
    fn cache_filename_for_separates_library_and_test_builds() {
        let package_root = Path::new("/package");
        let crate_root = Path::new("/package/src/lib.rs");

        assert_ne!(
            cache_filename_for(package_root, crate_root, CacheBuildKind::Library),
            cache_filename_for(package_root, crate_root, CacheBuildKind::Test)
        );
    }
}
