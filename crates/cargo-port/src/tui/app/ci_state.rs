use std::collections::HashSet;

use super::types::App;
use super::types::CiState;
use crate::ci;
use crate::ci::CiRun;
use crate::scan;
use crate::scan::CiFetchResult;
use crate::tui::detail::CiFetchKind;

impl App {
    /// Insert CI runs from the initial scan for a CI owner path.
    pub(super) fn insert_ci_runs(&mut self, path: String, runs: Vec<CiRun>) {
        if !self.is_cargo_active_path(&path) {
            self.ci_state.remove(&path);
            return;
        }
        let exhausted = self
            .git_info
            .get(&path)
            .and_then(|git| {
                git.url.as_ref().and_then(|url| {
                    ci::parse_owner_repo(url).map(|(owner, repo)| {
                        scan::is_exhausted(&owner, &repo, git.branch.as_deref())
                    })
                })
            })
            .unwrap_or(false);
        self.ci_state
            .insert(path, CiState::Loaded { runs, exhausted });
    }

    /// Process a completed CI fetch: merge runs and detect exhaustion.
    pub(super) fn handle_ci_fetch_complete(
        &mut self,
        path: String,
        result: CiFetchResult,
        kind: CiFetchKind,
    ) {
        let new_runs = match result {
            CiFetchResult::Loaded(runs) | CiFetchResult::CacheOnly(runs) => runs,
        };

        let prev_count = self
            .ci_state
            .get(&path)
            .map_or(0, |state| state.runs().len());

        let existing = self
            .ci_state
            .remove(&path)
            .map(|state| match state {
                CiState::Fetching { runs, .. } | CiState::Loaded { runs, .. } => runs,
            })
            .unwrap_or_default();

        let mut seen = HashSet::new();
        let mut merged = Vec::new();
        for run in new_runs {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        for run in existing {
            if seen.insert(run.run_id) {
                merged.push(run);
            }
        }
        merged.sort_by(|left, right| right.run_id.cmp(&left.run_id));

        let found_new = merged.len() > prev_count;
        let exhausted = if found_new {
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = ci::parse_owner_repo(url)
            {
                scan::clear_exhausted(&owner, &repo, git.branch.as_deref());
            }
            false
        } else {
            if let Some(git) = self.git_info.get(&path)
                && let Some(ref url) = git.url
                && let Some((owner, repo)) = ci::parse_owner_repo(url)
            {
                scan::mark_exhausted(&owner, &repo, git.branch.as_deref());
            }
            if matches!(kind, CiFetchKind::Refresh) {
                self.status_flash =
                    Some(("no new runs found".to_string(), std::time::Instant::now()));
                self.show_timed_toast("CI", "No new runs found".to_string());
            }
            true
        };

        self.ci_pane.set_pos(merged.len());
        self.ci_state.insert(
            path,
            CiState::Loaded {
                runs: merged,
                exhausted,
            },
        );
        self.data_generation += 1;
    }

    pub(super) fn is_ci_owner_path(&self, path: &str) -> bool {
        self.nodes.iter().any(|node| {
            node.project.path == path
                || node
                    .worktrees
                    .iter()
                    .any(|worktree| worktree.project.path == path)
        })
    }

    pub(super) fn latest_ci_run_for_path(&self, path: &str) -> Option<&CiRun> {
        let branch = self
            .git_info
            .get(path)
            .and_then(|git| git.branch.as_deref());
        let state = self.ci_state.get(path)?;
        branch.map_or_else(
            || state.runs().first(),
            |branch| state.runs().iter().find(|run| run.branch == branch),
        )
    }
}
