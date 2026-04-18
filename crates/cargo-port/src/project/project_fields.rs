use super::git::GitInfo;
use super::info::ProjectInfo;
use super::info::Visibility;
use super::info::WorktreeHealth;
use super::paths::AbsolutePath;
use super::paths::DisplayPath;
use super::paths::RootDirectoryName;

/// Whether to scan languages for an entry during enrichment.
pub(crate) enum LanguageScan {
    Run,
    Skip,
}

/// Whether to fetch CI runs for an entry during enrichment.
pub(crate) enum CiFetch {
    Run,
    Skip,
}

/// Whether to fetch the first-commit date for an entry during enrichment.
pub(crate) enum FirstCommitFetch {
    Run,
    Skip,
}

/// Whether to fetch GitHub repo metadata (stars, description) for an entry.
pub(crate) enum RepoMetadataFetch {
    Run,
    Skip,
}

/// Shared field access for every concrete project-list node.
///
/// Once a type implements this trait it participates in all generic
/// disk/git/visibility/enrichment logic — there is no looser tier.
pub(crate) trait ProjectFields {
    fn path(&self) -> &AbsolutePath;
    fn name(&self) -> Option<&str>;
    fn visibility(&self) -> Visibility;
    fn worktree_health(&self) -> WorktreeHealth;
    fn disk_usage_bytes(&self) -> Option<u64>;
    fn git_info(&self) -> Option<&GitInfo>;
    fn info(&self) -> &ProjectInfo;
    fn info_mut(&mut self) -> &mut ProjectInfo;
    fn display_path(&self) -> DisplayPath;
    fn root_directory_name(&self) -> RootDirectoryName;

    /// Crates.io package name to query, when the entry corresponds to a
    /// publishable crate. Default `None` — opt in by overriding.
    fn crates_io_name(&self) -> Option<&str> { None }

    /// Whether the enrichment funnel should run a language scan.
    fn language_scan(&self) -> LanguageScan { LanguageScan::Run }

    /// Whether the enrichment funnel should fetch CI runs. Default keys
    /// off the entry's primary git remote URL.
    fn ci_fetch(&self) -> CiFetch {
        match self.git_info().and_then(GitInfo::primary_url) {
            Some(_) => CiFetch::Run,
            None => CiFetch::Skip,
        }
    }

    /// Whether the enrichment funnel should fetch the first-commit date.
    /// Default keys off whether git info has been resolved.
    fn first_commit(&self) -> FirstCommitFetch {
        match self.git_info() {
            Some(_) => FirstCommitFetch::Run,
            None => FirstCommitFetch::Skip,
        }
    }

    /// Whether the enrichment funnel should fetch GitHub repo metadata.
    /// Default tracks `ci_fetch` because today's repo-metadata fetch
    /// piggybacks on the same upstream resolution.
    fn repo_metadata(&self) -> RepoMetadataFetch {
        match self.ci_fetch() {
            CiFetch::Run => RepoMetadataFetch::Run,
            CiFetch::Skip => RepoMetadataFetch::Skip,
        }
    }
}
