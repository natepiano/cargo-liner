use std::path::Path;

use crate::ci;
use crate::constants::IN_SYNC;
use crate::constants::NO_REMOTE_SYNC;
use crate::constants::SYNC_DOWN;
use crate::constants::SYNC_UP;
use crate::project::CheckoutInfo;
use crate::project::GitStatus;
use crate::project::ProjectFields;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::project::WorktreeGroup;
use crate::tui::app::App;

impl App {
    pub fn git_info_for(&self, path: &Path) -> Option<&CheckoutInfo> {
        self.projects()
            .at_path(path)
            .and_then(|project| project.local_git_state.info())
    }

    /// Per-repo info (remotes, workflows, default branch, ...) for the
    /// entry containing `path`. `None` means either the path isn't in a
    /// known entry, the entry isn't in a git repo, or the background
    /// `LocalGitInfo::get` call hasn't completed yet.
    pub fn repo_info_for(&self, path: &Path) -> Option<&RepoInfo> {
        self.projects()
            .entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref()?.repo_info.as_ref())
    }

    /// Convenience: the primary remote's URL for the checkout at `path`,
    /// looked up against its containing entry's `RepoInfo`.
    pub fn primary_url_for(&self, path: &Path) -> Option<&str> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_url(repo)
    }

    /// Convenience: the primary remote's ahead/behind for the checkout
    /// at `path`.
    pub(super) fn primary_ahead_behind_for(&self, path: &Path) -> Option<(usize, usize)> {
        let checkout = self.git_info_for(path)?;
        let repo = self.repo_info_for(path)?;
        checkout.primary_ahead_behind(repo)
    }

    /// Pick a remote URL to drive the GitHub fetch for the entry
    /// containing `path`. Independent of the current checkout's
    /// upstream tracking: a worktree on a branch without an upstream
    /// still belongs to the repo and should fetch repo-level metadata.
    /// Preference order: `upstream`, then `origin`, then the first
    /// remote with a parseable owner/repo URL.
    pub fn fetch_url_for(&self, path: &Path) -> Option<String> {
        let repo = self.repo_info_for(path)?;
        let parseable = |name: &str| {
            repo.remotes
                .iter()
                .find(|r| r.name == name)
                .and_then(|r| r.url.as_deref())
                .filter(|url| ci::parse_owner_repo(url).is_some())
        };
        parseable("upstream")
            .or_else(|| parseable("origin"))
            .or_else(|| {
                repo.remotes.iter().find_map(|r| {
                    let url = r.url.as_deref()?;
                    ci::parse_owner_repo(url).map(|_| url)
                })
            })
            .map(String::from)
    }

    pub fn git_status_for(&self, path: &Path) -> Option<GitStatus> {
        self.git_info_for(path).map(|info| info.status)
    }

    /// Roll up the worst git path state across all **visible** children of a
    /// `RootItem`.  For worktree groups, checks primary + non-dismissed linked
    /// entries.  For everything else, returns the state for the single path.
    pub fn git_status_for_item(&self, item: &RootItem) -> Option<GitStatus> {
        match item {
            RootItem::Worktrees(g) => {
                let states: Box<dyn Iterator<Item = Option<GitStatus>>> = match g {
                    WorktreeGroup::Workspaces {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                    WorktreeGroup::Packages {
                        primary, linked, ..
                    } => Box::new(
                        std::iter::once(self.git_status_for(primary.path())).chain(
                            linked
                                .iter()
                                .filter(|l| l.visibility() == Visibility::Visible)
                                .map(|l| self.git_status_for(l.path())),
                        ),
                    ),
                };
                worst_git_status(states)
            },
            _ => self.git_status_for(item.path()),
        }
    }

    /// Formatted ahead/behind sync status for the project list columns.
    pub fn git_sync(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match self.primary_ahead_behind_for(path) {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            // No upstream tracking branch: render a flat placeholder in the O column.
            None => NO_REMOTE_SYNC.to_string(),
        }
    }

    pub fn git_main(&self, path: &Path) -> String {
        let Some(info) = self.git_info_for(path) else {
            return String::new();
        };
        if matches!(info.status, GitStatus::Untracked | GitStatus::Ignored) {
            return String::new();
        }
        match info.ahead_behind_local {
            Some((0, 0)) => IN_SYNC.to_string(),
            Some((a, 0)) => format!("{SYNC_UP}{a}"),
            Some((0, b)) => format!("{SYNC_DOWN}{b}"),
            Some((a, b)) => format!("{SYNC_UP}{a}{SYNC_DOWN}{b}"),
            None => String::new(),
        }
    }
}

/// Return the most severe git path state from an iterator.
/// Severity: `Modified` > `Untracked` > `Clean` > `Ignored`.
fn worst_git_status(states: impl Iterator<Item = Option<GitStatus>>) -> Option<GitStatus> {
    const fn severity(state: GitStatus) -> u8 {
        match state {
            GitStatus::Modified => 4,
            GitStatus::Untracked => 3,
            GitStatus::Clean => 2,
            GitStatus::Ignored => 1,
        }
    }
    states.flatten().max_by_key(|s| severity(*s))
}
