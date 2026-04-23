use std::ops::Deref;
use std::ops::DerefMut;

use super::cargo::ExampleGroup;
use super::cargo::ProjectType;
use super::info::ProjectInfo;
use super::vendored_package::VendoredPackage;
use crate::lint::LintRuns;

/// Rust-specific project data shared by both `Workspace` and `Package`.
/// Derefs to `ProjectInfo` for uniform metadata access.
#[derive(Clone, Default)]
pub(crate) struct RustInfo {
    pub(crate) info:             ProjectInfo,
    pub(crate) cargo:            Cargo,
    pub(crate) vendored:         Vec<VendoredPackage>,
    pub(crate) lint_runs:        LintRuns,
    pub(crate) crates_version:   Option<String>,
    pub(crate) crates_downloads: Option<u64>,
}

impl RustInfo {
    pub(crate) const fn cargo(&self) -> &Cargo { &self.cargo }

    pub(crate) fn vendored(&self) -> &[VendoredPackage] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<VendoredPackage> { &mut self.vendored }

    pub(crate) const fn info(&self) -> &ProjectInfo { &self.info }

    pub(crate) const fn info_mut(&mut self) -> &mut ProjectInfo { &mut self.info }

    pub(crate) const fn lint_runs(&self) -> &LintRuns { &self.lint_runs }

    pub(crate) const fn lint_runs_mut(&mut self) -> &mut LintRuns { &mut self.lint_runs }

    pub(crate) fn crates_version(&self) -> Option<&str> { self.crates_version.as_deref() }

    pub(crate) const fn crates_downloads(&self) -> Option<u64> { self.crates_downloads }

    pub(crate) fn set_crates_io(&mut self, version: String, downloads: u64) {
        self.crates_version = Some(version);
        self.crates_downloads = Some(downloads);
    }
}

impl Deref for RustInfo {
    type Target = ProjectInfo;

    fn deref(&self) -> &ProjectInfo { &self.info }
}

impl DerefMut for RustInfo {
    fn deref_mut(&mut self) -> &mut ProjectInfo { &mut self.info }
}

/// Shared Cargo fields extracted from `Cargo.toml`.
///
/// Step 3b partial retirement: `version` and `description` are no longer
/// hand-parsed here — the detail pane reads them from the authoritative
/// `PackageRecord` via `App::resolve_metadata`. The remaining fields
/// (types / examples / benches / `test_count` / publishable) still back
/// the finder index, `ProjectCounts` stats, and `crates_io_name`
/// publishability gating; migrating those to the snapshot is the next
/// follow-up.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cargo {
    pub(crate) types:       Vec<ProjectType>,
    pub(crate) examples:    Vec<ExampleGroup>,
    pub(crate) benches:     Vec<String>,
    pub(crate) test_count:  usize,
    pub(crate) publishable: bool,
}

impl Cargo {
    pub(crate) fn types(&self) -> &[ProjectType] { &self.types }

    pub(crate) fn examples(&self) -> &[ExampleGroup] { &self.examples }

    pub(crate) fn benches(&self) -> &[String] { &self.benches }

    pub(crate) const fn test_count(&self) -> usize { self.test_count }

    pub(crate) fn example_count(&self) -> usize {
        self.examples.iter().map(|g| g.names.len()).sum()
    }

    #[allow(
        dead_code,
        reason = "Step 3b: no more production callers; previously drove \
                  `TargetsData.primary_binary` in the cold-start fallback, \
                  which now defaults to None. Kept for future reuse against \
                  PackageRecord.targets."
    )]
    pub(crate) fn is_binary(&self) -> bool {
        self.types.iter().any(|t| matches!(t, ProjectType::Binary))
    }

    pub(crate) const fn publishable(&self) -> bool { self.publishable }
}
