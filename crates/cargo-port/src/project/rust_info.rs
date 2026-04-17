use std::ops::Deref;
use std::ops::DerefMut;

use super::cargo::ExampleGroup;
use super::cargo::ProjectType;
use super::info::ProjectInfo;
use super::package::Package;
use super::paths::AbsolutePath;
use crate::lint::LintRuns;

/// Rust-specific project data shared by both `WorkspaceProject` and
/// `PackageProject`. Derefs to `ProjectInfo` for uniform metadata access.
#[derive(Clone)]
pub(crate) struct RustInfo {
    pub(super) info:                      ProjectInfo,
    pub(super) cargo:                     Cargo,
    pub(super) vendored:                  Vec<Package>,
    pub(super) worktree_name:             Option<String>,
    pub(super) worktree_primary_abs_path: Option<AbsolutePath>,
    pub(super) lint_runs:                 LintRuns,
    pub(super) crates_version:            Option<String>,
    pub(super) crates_downloads:          Option<u64>,
}

impl RustInfo {
    pub(crate) const fn cargo(&self) -> &Cargo { &self.cargo }

    pub(crate) fn vendored(&self) -> &[Package] { &self.vendored }

    pub(crate) const fn vendored_mut(&mut self) -> &mut Vec<Package> { &mut self.vendored }

    pub(crate) fn worktree_name(&self) -> Option<&str> { self.worktree_name.as_deref() }

    pub(crate) const fn worktree_primary_abs_path(&self) -> Option<&AbsolutePath> {
        self.worktree_primary_abs_path.as_ref()
    }

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
#[derive(Clone, Debug)]
pub(crate) struct Cargo {
    version:     Option<String>,
    description: Option<String>,
    types:       Vec<ProjectType>,
    examples:    Vec<ExampleGroup>,
    benches:     Vec<String>,
    test_count:  usize,
    publishable: bool,
}

impl Cargo {
    pub(crate) const fn new(
        version: Option<String>,
        description: Option<String>,
        types: Vec<ProjectType>,
        examples: Vec<ExampleGroup>,
        benches: Vec<String>,
        test_count: usize,
        publishable: bool,
    ) -> Self {
        Self {
            version,
            description,
            types,
            examples,
            benches,
            test_count,
            publishable,
        }
    }

    pub(crate) fn types(&self) -> &[ProjectType] { &self.types }

    pub(crate) fn examples(&self) -> &[ExampleGroup] { &self.examples }

    pub(crate) fn benches(&self) -> &[String] { &self.benches }

    pub(crate) fn version(&self) -> Option<&str> { self.version.as_deref() }

    pub(crate) fn description(&self) -> Option<&str> { self.description.as_deref() }

    pub(crate) const fn test_count(&self) -> usize { self.test_count }

    pub(crate) fn example_count(&self) -> usize {
        self.examples.iter().map(|g| g.names.len()).sum()
    }

    pub(crate) fn is_binary(&self) -> bool {
        self.types.iter().any(|t| matches!(t, ProjectType::Binary))
    }

    pub(crate) const fn publishable(&self) -> bool { self.publishable }
}
