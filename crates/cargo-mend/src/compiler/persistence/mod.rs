mod caller_aware;
mod intersection;
mod load;
mod schema;
mod visibility_priority;

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hash;
use std::hash::Hasher;
use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
pub(super) use load::load_report;
pub(super) use schema::StoredFinding;
pub(super) use schema::StoredPubUseFixFact;
pub(super) use schema::StoredReport;
pub(super) use schema::UseSite;

use super::constants::FINDINGS_DIR_NAME;

#[derive(Default)]
pub(super) struct FindingsSink {
    pub findings:          Vec<StoredFinding>,
    pub pub_use_fix_facts: Vec<StoredPubUseFixFact>,
    pub use_sites:         Vec<UseSite>,
}

pub(super) fn prepare_findings_dir(target_directory: &Path) -> Result<PathBuf> {
    let findings_dir = target_directory.join(FINDINGS_DIR_NAME);
    fs::create_dir_all(&findings_dir).with_context(|| {
        format!(
            "failed to create findings directory {}",
            findings_dir.display()
        )
    })?;
    Ok(findings_dir)
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub(super) enum CacheBuildKind {
    Library,
    Test,
}

pub(super) fn cache_filename_for(
    package_root: &Path,
    crate_root_file: &Path,
    build_kind: CacheBuildKind,
) -> String {
    let mut hasher = DefaultHasher::new();
    package_root.hash(&mut hasher);
    crate_root_file.hash(&mut hasher);
    build_kind.hash(&mut hasher);
    format!("{:016x}.json", hasher.finish())
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
