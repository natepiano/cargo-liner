use crate::ci;
use crate::ci::CiRun;
use crate::project::GitOrigin;
use crate::tui::app::App;

#[derive(Clone)]
pub enum CiEmptyState {
    BranchScopedOnly,
    Fetching,
    Loading,
    NoRuns,
    NoRunsForBranch(String),
    NoWorkflowConfigured,
    NotGitRepo,
    RequiresGithubRemote,
}

impl CiEmptyState {
    pub fn title(&self) -> String {
        match self {
            Self::BranchScopedOnly => " CI Runs — shown on branch/worktree rows ".to_string(),
            Self::Fetching | Self::Loading => " CI Runs — loading… ".to_string(),
            Self::NoRuns => " No CI Runs ".to_string(),
            Self::NoRunsForBranch(branch) => format!(" No CI runs for branch {branch} "),
            Self::NoWorkflowConfigured => " No CI workflow configured ".to_string(),
            Self::NotGitRepo => " CI Runs — not a git repository ".to_string(),
            Self::RequiresGithubRemote => " CI Runs — requires a GitHub origin remote ".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct CiData {
    pub runs:           Vec<CiRun>,
    pub mode_label:     Option<String>,
    pub current_branch: Option<String>,
    pub empty_state:    CiEmptyState,
}

impl CiData {
    pub const fn has_runs(&self) -> bool { !self.runs.is_empty() }
}

pub fn build_ci_data(app: &App) -> CiData {
    let selected_path = app.project_list.selected_project_path();
    let has_ci_owner = app.project_list.selected_ci_path().is_some();
    // CI branch, toggle, and display mode resolve to the checkout root
    // that owns them, so a workspace member reads the same values as its
    // parent row.
    let ci_owner = selected_path.map(|path| app.project_list.ci_branch_owner_path(path));
    let git_info = ci_owner
        .as_ref()
        .and_then(|owner| app.project_list.git_info_for(owner.as_path()));
    let repo_info = selected_path.and_then(|path| app.project_list.repo_info_for(path));
    let ci_info = selected_path.and_then(|path| app.project_list.ci_info_for(path));
    let current_branch = selected_path
        .and_then(|path| app.project_list.current_branch_for(path))
        .map(str::to_string);
    let runs = app
        .project_list
        .selected_project_path()
        .map_or_else(Vec::new, |path| {
            app.project_list.ci_runs_for_ci_pane(path, &app.ci)
        });
    let is_fetching = selected_path.is_some_and(|path| app.ci.fetch_tracker.is_fetching(path));
    let branch_filtered_empty = selected_path.is_some_and(|path| {
        app.ci_toggle_available_for(path)
            && ci_owner
                .as_ref()
                .is_some_and(|owner| app.ci.display_mode_label_for(owner.as_path()) == "branch")
    }) && ci_info.is_some_and(|info| !info.runs.is_empty())
        && runs.is_empty();
    // "Do we have a GitHub-parseable remote?" is a per-repo question and
    // must not depend on whether the current branch has an upstream — a
    // checkout on a branch without upstream tracking still belongs to
    // the repo.
    let has_github_remote = repo_info.is_some_and(|r| {
        r.remotes
            .iter()
            .filter_map(|r| r.url.as_deref())
            .any(|url| ci::parse_owner_repo(url).is_some())
    });
    let empty_state = if selected_path.is_some() && !has_ci_owner {
        CiEmptyState::BranchScopedOnly
    } else if git_info.is_none() {
        CiEmptyState::NotGitRepo
    } else if has_ci_owner
        && (repo_info.is_none_or(|r| r.origin_kind() == GitOrigin::Local) || !has_github_remote)
    {
        CiEmptyState::RequiresGithubRemote
    } else if repo_info.is_some_and(|r| !r.workflows.is_present()) {
        CiEmptyState::NoWorkflowConfigured
    } else if is_fetching {
        CiEmptyState::Fetching
    } else if ci_info.is_none() || !app.scan.is_complete() {
        CiEmptyState::Loading
    } else if branch_filtered_empty {
        CiEmptyState::NoRunsForBranch(
            current_branch
                .clone()
                .unwrap_or_else(|| "current".to_string()),
        )
    } else {
        CiEmptyState::NoRuns
    };

    CiData {
        runs,
        mode_label: ci_owner.as_ref().and_then(|owner| {
            selected_path
                .is_some_and(|path| app.ci_toggle_available_for(path))
                .then(|| app.ci.display_mode_label_for(owner.as_path()).to_string())
        }),
        current_branch,
        empty_state,
    }
}
