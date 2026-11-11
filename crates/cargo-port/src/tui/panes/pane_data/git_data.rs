use super::AbsolutePath;
use super::App;
use super::AvailabilityStatus;
use super::BisectProgress;
use super::DetailField;
use super::GIT_CLONE;
use super::GIT_DIR;
use super::GIT_FORK;
use super::GitReadHealth;
use super::GitStatus;
use super::HashSet;
use super::HeadState;
use super::Path;
use super::ProjectPrData;
use super::ProjectPrInfo;
use super::PullRequestCompleteness;
use super::PullRequestInfo;
use super::PullRequestUnavailableReason;
use super::PushDisabledReason;
use super::PushState;
use super::RateLimitQuota;
use super::RemoteKind;
use super::RepoInfo;
use super::RootItem;
use super::Submodule;
use super::Visibility;
use super::WorktreeStatus;
use super::ci;
use super::formatting;
use super::formatting::GitReadFailures;
use super::project;
use crate::constants::GIT_UNREADABLE_LABEL;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::GitHubInfo;
use crate::project::PinnedCommit;
use crate::project::RemoteList;
use crate::project::RepositoryProbe;
use crate::project::ResolvedBranch;
use crate::project::TrackedRef;
use crate::project::WorktreeHealth;
use crate::tui::panes::DescriptionState;
use crate::tui::panes::git::GitDescription;
use crate::tui::project_list::ProjectPathLookup::Found;
use crate::tui::project_list::ProjectPathLookup::StaleRoute;
use crate::tui::project_list::ProjectPathLookup::UnknownPath;
use crate::tui::state::RepoBranchProbe;

/// How a branch relates to the repo, rendered as a qualifier after its
/// name in the Git pane's Branch row.
#[derive(Clone, Copy)]
pub enum HeadRelation {
    /// On the repo's default branch.
    Default,
    /// On a non-default branch in the primary checkout.
    Feature,
    /// On a branch in a linked worktree checkout.
    Worktree,
    Unreadable,
    AwaitingRepoProbe,
}

/// Evidence that this checkout is a linked worktree.
#[derive(Clone, Copy)]
enum WorktreePlacement {
    Linked,
    NotLinked,
}

impl From<Option<&WorktreeStatus>> for WorktreePlacement {
    fn from(status: Option<&WorktreeStatus>) -> Self {
        match status {
            Some(WorktreeStatus::Linked { .. }) => Self::Linked,
            Some(WorktreeStatus::Primary { .. } | WorktreeStatus::NotGit) | None => Self::NotLinked,
        }
    }
}

/// Whether the checkout's first HEAD probe has arrived.
enum HeadProbe<'a> {
    Awaiting,
    Probed(&'a HeadState),
}

impl HeadRelation {
    /// Classify a branch against the default branch and linked-worktree evidence.
    fn classify(
        branch: &str,
        local_main_branch: RepoBranchProbe<'_>,
        placement: WorktreePlacement,
    ) -> Self {
        match (placement, local_main_branch) {
            (WorktreePlacement::Linked, RepoBranchProbe::Awaiting | RepoBranchProbe::Probed(_)) => {
                Self::Worktree
            },
            (WorktreePlacement::NotLinked, RepoBranchProbe::Awaiting) => Self::AwaitingRepoProbe,
            (
                WorktreePlacement::NotLinked,
                RepoBranchProbe::Probed(ResolvedBranch::Named(name)),
            ) => {
                if name == branch {
                    Self::Default
                } else {
                    Self::Feature
                }
            },
            (WorktreePlacement::NotLinked, RepoBranchProbe::Probed(ResolvedBranch::NotFound)) => {
                Self::Feature
            },
            (WorktreePlacement::NotLinked, RepoBranchProbe::Probed(ResolvedBranch::Unreadable)) => {
                Self::Unreadable
            },
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Feature => "feature",
            Self::Worktree => "worktree",
            Self::Unreadable => GIT_UNREADABLE_LABEL,
            Self::AwaitingRepoProbe => "awaiting repository probe",
        }
    }
}

/// A value that is absent only until its git probe completes.
#[derive(Clone, Copy, Debug, Default)]
pub enum GitProbe<T> {
    #[default]
    Awaiting,
    Probed(T),
}

/// GitHub applicability and the current Stars measurement.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GithubStars {
    #[default]
    NotGithub,
    Pending,
    Measured(u64),
    Unreachable,
}

/// `QuotaObservation` records whether a GitHub bucket has a measured quota.
/// `QuotaObservation::Observed` keeps the last received `RateLimitQuota`.
#[derive(Clone, Copy, Debug, Default)]
pub enum QuotaObservation {
    #[default]
    Unobserved,
    Observed(RateLimitQuota),
}

/// Per-pane data for the Git detail panel.
#[derive(Clone, Default)]
pub struct GitData {
    pub checkout:                    GitCheckout,
    pub(in crate::tui) failed_reads: GitReadFailures,
    pub bisect:                      Option<BisectProgress>,
    pub status:                      Option<GitStatus>,
    pub vs_local:                    GitProbe<Comparison>,
    pub local_main_branch:           GitProbe<ResolvedBranch>,
    pub stars:                       GithubStars,
    pub description:                 DescriptionState,
    pub inception:                   FirstCommit,
    pub last_commit:                 GitProbe<CommitDate>,
    pub last_fetched:                Option<String>,
    pub rate_limit_core:             QuotaObservation,
    pub rate_limit_graphql:          QuotaObservation,
    pub github_status:               AvailabilityStatus,
    pub pull_requests:               PullRequestSection,
    pub remotes:                     RemoteRows,
    pub worktrees:                   Vec<WorktreeInfo>,
    /// Submodule-specific overlay. `Some` only when this `GitData` is
    /// built for a submodule pane — the renderer reads this to decide
    /// whether to emit the `Tracks` / `Pinned` rows. Submodule identity
    /// is conveyed by the project-list `(s)` marker and the pane's
    /// "Submodule — \<name\>" title, not by an About-section line.
    pub submodule_ctx:               Option<SubmoduleContext>,
}

/// The checkout as the Git pane draws it. `Broken` (a linked worktree
/// whose gitdir is missing) wins over whatever the HEAD probe returned.
#[derive(Clone, Default)]
pub enum GitCheckout {
    Broken,
    #[default]
    AwaitingProbe,
    /// The scan found no git checkout at the project path.
    NotRepository,
    UnreadableHead,
    Unborn,
    Detached {
        short_sha: String,
    },
    Branch {
        name:     String,
        relation: HeadRelation,
    },
}

impl GitCheckout {
    pub(in crate::tui::panes) const fn head_read_health(&self) -> GitReadHealth {
        match self {
            Self::UnreadableHead
            | Self::Branch {
                relation: HeadRelation::Unreadable,
                ..
            } => GitReadHealth::Unreadable,
            Self::Broken
            | Self::AwaitingProbe
            | Self::NotRepository
            | Self::Unborn
            | Self::Detached { .. }
            | Self::Branch {
                relation:
                    HeadRelation::Default
                    | HeadRelation::Feature
                    | HeadRelation::Worktree
                    | HeadRelation::AwaitingRepoProbe,
                ..
            } => GitReadHealth::Readable,
        }
    }

    fn for_project(
        status: Option<&WorktreeStatus>,
        health: WorktreeHealth,
        head: HeadProbe<'_>,
        default_branch: RepoBranchProbe<'_>,
    ) -> Self {
        if matches!(
            (status, &head),
            (Some(WorktreeStatus::NotGit), HeadProbe::Awaiting)
        ) {
            return Self::NotRepository;
        }
        Self::from_probe(
            health,
            head,
            default_branch,
            WorktreePlacement::from(status),
        )
    }

    fn from_probe(
        health: WorktreeHealth,
        head: HeadProbe<'_>,
        default_branch: RepoBranchProbe<'_>,
        placement: WorktreePlacement,
    ) -> Self {
        match (health, head) {
            (WorktreeHealth::Broken, HeadProbe::Awaiting | HeadProbe::Probed(_)) => Self::Broken,
            (WorktreeHealth::Normal, HeadProbe::Awaiting) => Self::AwaitingProbe,
            (WorktreeHealth::Normal, HeadProbe::Probed(head)) => match head {
                HeadState::Unreadable => Self::UnreadableHead,
                HeadState::Unborn => Self::Unborn,
                HeadState::Detached { short_sha } => Self::Detached {
                    short_sha: short_sha.clone(),
                },
                HeadState::Branch(name) => Self::Branch {
                    name:     name.clone(),
                    relation: HeadRelation::classify(name, default_branch, placement),
                },
            },
        }
    }
}

#[derive(Clone, Default)]
pub struct PullRequestSection {
    pub state:              PullRequestSectionState,
    pub rows:               Vec<PullRequestRow>,
    pub fetched_at:         Option<String>,
    pub unavailable_reason: Option<PullRequestUnavailableReason>,
    pub completeness:       Option<PullRequestCompleteness>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PullRequestSectionState {
    #[default]
    HiddenConfirmedEmpty,
    Loading,
    Loaded,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullRequestPolling {
    Active,
    Idle,
}

impl From<bool> for PullRequestPolling {
    fn from(is_polling: bool) -> Self { if is_polling { Self::Active } else { Self::Idle } }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestRow {
    pub number:      u32,
    pub title:       String,
    pub url:         String,
    pub state_label: &'static str,
    pub polling:     PullRequestPolling,
    pub branch:      String,
    pub base:        String,
}

/// Submodule-only render context: facts the parent repo provides about
/// the submodule that no normal repo has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubmoduleContext {
    /// Tracking branch from `.gitmodules` (the `branch =` line). `None`
    /// when `.gitmodules` doesn't specify one.
    pub tracks:        Option<String>,
    /// Pinned commit SHA from `git ls-tree HEAD` in the parent repo.
    /// Always present when `SubmoduleContext` is built — without it
    /// there's no reason to render the overlay.
    pub pinned_commit: PinnedCommit,
}

/// Remote table rows retain whether the underlying listing failed.
#[derive(Clone, Default)]
pub enum RemoteRows {
    #[default]
    AwaitingProbe,
    Listed(Vec<RemoteRow>),
    Unreadable,
}

impl RemoteRows {
    pub fn listed(&self) -> &[RemoteRow] {
        match self {
            Self::Listed(rows) => rows,
            Self::AwaitingProbe | Self::Unreadable => &[],
        }
    }
}

/// Per-remote row: identity text plus typed tracking and comparison evidence.
#[derive(Clone)]
pub struct RemoteRow {
    pub name:            String,
    pub icon:            &'static str,
    pub display_url:     String,
    /// Local branch (current `HEAD`) compared against `tracked_ref` —
    /// the source side of the `status` ahead/behind delta.
    pub branch:          String,
    pub tracked_ref:     TrackedRef,
    pub status:          Comparison,
    pub full_url:        Option<String>,
    /// Pre-formatted push-disabled annotation (e.g. `"↛ push disabled"`
    /// or `"↛ push disabled (DISABLED)"`). `None` when push is enabled.
    pub push_annotation: Option<String>,
}

impl RemoteRow {
    pub fn tracked_text(&self) -> String { formatting::format_tracked_ref(&self.tracked_ref) }
    pub fn status_text(&self) -> String { formatting::format_ahead_behind(self.status) }
}

/// Per-worktree info rendered in the Git pane's Worktrees table.
///
/// `ahead_behind` is relative to the primary worktree's HEAD commit.
#[derive(Clone)]
pub struct WorktreeInfo {
    pub name:         String,
    pub path:         String,
    pub branch:       Option<String>,
    /// The primary worktree's branch this entry is measured against —
    /// the target side of the `ahead_behind` delta. `None` for the
    /// primary entry itself, which is the baseline (nothing to compare).
    pub tracked:      Option<String>,
    pub ahead_behind: Comparison,
}

/// What the Git pane cursor selects at a given `pos()`. Single source of
/// truth shared by the renderer and the Enter-key handler so neither side
/// can drift from the other's row layout.
#[allow(
    dead_code,
    reason = "Field/Worktree payloads exist for exhaustiveness; callers may match only Remote"
)]
pub enum GitRow<'a> {
    Description(&'a str),
    Field(DetailField),
    PullRequest(&'a PullRequestRow),
    Remote(&'a RemoteRow),
    Worktree(&'a WorktreeInfo),
}

pub fn git_fields_from_data(data: &GitData) -> Vec<DetailField> {
    let mut fields = Vec::new();
    match data.checkout {
        GitCheckout::Broken => fields.push(DetailField::WorktreeError),
        GitCheckout::UnreadableHead
        | GitCheckout::Unborn
        | GitCheckout::Detached { .. }
        | GitCheckout::Branch { .. } => fields.push(DetailField::Head),
        GitCheckout::AwaitingProbe | GitCheckout::NotRepository => {},
    }
    if data.bisect.is_some() {
        fields.push(DetailField::Bisect);
    }
    if let Some(ctx) = data.submodule_ctx.as_ref() {
        if ctx.tracks.is_some() {
            fields.push(DetailField::Tracks);
        }
        fields.push(DetailField::Pinned);
    }
    if data.status.is_some() {
        fields.push(DetailField::GitStatus);
    }
    if matches!(
        data.vs_local,
        GitProbe::Probed(Comparison::Counted(_) | Comparison::Unreadable)
    ) {
        fields.push(DetailField::VsLocal);
    }
    if matches!(data.stars, GithubStars::Measured(_)) {
        fields.push(DetailField::Stars);
    }
    if matches!(
        data.github_status,
        AvailabilityStatus::Unreachable | AvailabilityStatus::RateLimited
    ) {
        fields.push(DetailField::GithubStatus);
    }
    // Repo description is rendered separately in the About section by
    // `render_git_about_section`, so it is intentionally not a flat field.
    if matches!(
        data.inception,
        FirstCommit::Probed(CommitDate::At(_) | CommitDate::Unreadable)
    ) {
        fields.push(DetailField::Inception);
    }
    if matches!(
        data.last_commit,
        GitProbe::Probed(CommitDate::At(_) | CommitDate::Unreadable)
    ) {
        fields.push(DetailField::LastCommit);
    }
    if data.last_fetched.is_some() {
        fields.push(DetailField::LastFetched);
    }
    match &data.remotes {
        RemoteRows::Unreadable => fields.push(DetailField::RemotesUnreadable),
        RemoteRows::AwaitingProbe | RemoteRows::Listed(_) => {},
    }
    for field in data.failed_reads.fields() {
        if !(fields.contains(&field)
            || field == DetailField::Head && matches!(data.checkout, GitCheckout::Broken))
        {
            fields.push(field);
        }
    }
    if matches!(data.rate_limit_core, QuotaObservation::Observed(_)) {
        fields.push(DetailField::RateLimitCore);
    }
    if matches!(data.rate_limit_graphql, QuotaObservation::Observed(_)) {
        fields.push(DetailField::RateLimitGraphQl);
    }
    if !data.worktrees.is_empty() {
        // Worktree count is appended by the render function, not as a field.
    }
    fields
}

pub fn git_has_description_row(data: &GitData) -> GitDescription {
    match data.description.as_deref().map(str::trim) {
        Some(description) if !description.is_empty() => GitDescription::Present,
        Some(_) | None => GitDescription::Absent,
    }
}

pub fn git_row_at(data: &GitData, pos: usize) -> Option<GitRow<'_>> {
    let description_rows = git_has_description_row(data).row_count();
    if description_rows > 0 && pos == 0 {
        return data.description.as_deref().map(GitRow::Description);
    }
    let pos = pos.checked_sub(description_rows)?;
    let fields = git_fields_from_data(data);
    let flat_len = fields.len();
    if pos < flat_len {
        return fields.get(pos).copied().map(GitRow::Field);
    }
    let pos = pos - flat_len;
    if pos < data.pull_requests.rows.len() {
        return data.pull_requests.rows.get(pos).map(GitRow::PullRequest);
    }
    let pos = pos - data.pull_requests.rows.len();
    if pos < data.remotes.listed().len() {
        return data.remotes.listed().get(pos).map(GitRow::Remote);
    }
    let pos = pos - data.remotes.listed().len();
    data.worktrees.get(pos).map(GitRow::Worktree)
}
pub(super) struct GitDetailFields {
    pub(super) checkout:           GitCheckout,
    pub(super) failed_reads:       GitReadFailures,
    pub(super) bisect:             Option<BisectProgress>,
    pub(super) path:               Option<GitStatus>,
    pub(super) vs_local:           GitProbe<Comparison>,
    pub(super) local_main_branch:  GitProbe<ResolvedBranch>,
    pub(super) stars:              GithubStars,
    pub(super) description:        DescriptionState,
    pub(super) inception:          FirstCommit,
    pub(super) last_commit:        GitProbe<CommitDate>,
    pub(super) last_fetched:       Option<String>,
    pub(super) rate_limit_core:    QuotaObservation,
    pub(super) rate_limit_graphql: QuotaObservation,
    pub(super) github_status:      AvailabilityStatus,
    pub(super) pull_requests:      PullRequestSection,
    pub(super) remotes:            RemoteRows,
}

/// The repository source must resolve before an empty GitHub description is known.
enum GithubDescriptionSource<'a> {
    AwaitingRemotes,
    NoGithubRemote,
    AwaitingReply,
    Reply(&'a GitHubInfo),
}

impl<'a> GithubDescriptionSource<'a> {
    fn from_repository(probe: &RepositoryProbe, github: Option<&'a GitHubInfo>) -> Self {
        match probe {
            RepositoryProbe::Awaiting => Self::AwaitingRemotes,
            RepositoryProbe::NoRepository => Self::NoGithubRemote,
            RepositoryProbe::Repository(repo) => match &repo.remotes {
                RemoteList::Unreadable => Self::AwaitingRemotes,
                RemoteList::Listed(remotes) => {
                    if remotes.iter().any(|remote| {
                        remote
                            .url
                            .as_deref()
                            .and_then(ci::parse_owner_repo)
                            .is_some()
                    }) {
                        github.map_or(Self::AwaitingReply, Self::Reply)
                    } else {
                        Self::NoGithubRemote
                    }
                },
            },
        }
    }

    fn description(self) -> DescriptionState {
        match self {
            Self::AwaitingRemotes | Self::AwaitingReply => DescriptionState::Pending,
            Self::NoGithubRemote => DescriptionState::KnownEmpty,
            Self::Reply(info) => DescriptionState::from(info.description.clone()),
        }
    }
}

pub(super) fn build_git_detail_fields(app: &App, abs_path: &Path) -> GitDetailFields {
    let git_repo = app.project_list.git_repo_for(abs_path);
    let repo_info = git_repo.and_then(|repo| repo.repo_info.as_ref());
    let checkout = app.project_list.git_info_for(abs_path);

    let head = checkout.map_or(HeadProbe::Awaiting, |info| HeadProbe::Probed(&info.head));
    let bisect = checkout.and_then(|info| info.bisect.clone());
    let local_main_probe = repo_info.map_or(RepoBranchProbe::Awaiting, |repo| {
        RepoBranchProbe::Probed(&repo.local_main_branch)
    });
    let local_main_branch = repo_info.map_or(GitProbe::Awaiting, |repo| {
        GitProbe::Probed(repo.local_main_branch.clone())
    });
    let vs_local = checkout.map_or(GitProbe::Awaiting, |info| {
        GitProbe::Probed(info.ahead_behind_local)
    });
    let github = git_repo.and_then(|repo| repo.github_info.as_ref());
    let stars = match github {
        Some(info) => GithubStars::Measured(info.stars),
        None if app
            .project_list
            .fetch_url_for(abs_path)
            .is_some_and(|url| ci::parse_owner_repo(&url).is_some()) =>
        {
            if matches!(
                app.net.github_status(),
                AvailabilityStatus::Unreachable | AvailabilityStatus::RateLimited
            ) {
                GithubStars::Unreachable
            } else {
                GithubStars::Pending
            }
        },
        None => GithubStars::NotGithub,
    };
    let repository_probe = git_repo.map_or(&RepositoryProbe::Awaiting, |repo| &repo.repo_info);
    let source = GithubDescriptionSource::from_repository(repository_probe, github);
    let description = source.description();
    let inception = repo_info.map_or(FirstCommit::AwaitingProbe, |repo| repo.first_commit.clone());
    let last_commit = checkout.map_or(GitProbe::Awaiting, |info| {
        GitProbe::Probed(info.last_commit.clone())
    });
    let last_fetched = repo_info
        .and_then(|repo| repo.last_fetched.as_deref())
        .map(formatting::format_timestamp);
    let default_host = app.config.current().tui.default_remote_host_url.clone();
    let current_branch = checkout
        .and_then(|info| info.head.branch_name())
        .unwrap_or("-");
    let remotes = repo_info.map_or_else(RemoteRows::default, |repo| {
        build_remote_rows(repo, &default_host, current_branch)
    });
    let rate_limit = app.net.rate_limit();
    GitDetailFields {
        failed_reads: GitReadFailures::of(checkout, repo_info),
        checkout: GitCheckout::for_project(
            app.project_list.worktree_status_for(abs_path),
            match app.project_list.at_path(abs_path) {
                Found(info) => info.worktree_health,
                UnknownPath | StaleRoute => WorktreeHealth::Normal,
            },
            head,
            local_main_probe,
        ),
        bisect,
        path: app.project_list.git_status_for(abs_path),
        vs_local,
        local_main_branch,
        stars,
        description,
        inception,
        last_commit,
        last_fetched,
        rate_limit_core: rate_limit
            .core
            .map_or(QuotaObservation::Unobserved, QuotaObservation::Observed),
        rate_limit_graphql: rate_limit
            .graphql
            .map_or(QuotaObservation::Unobserved, QuotaObservation::Observed),
        github_status: app.net.github_status(),
        pull_requests: project_pull_request_section(app, abs_path),
        remotes,
    }
}

fn project_pull_request_section(app: &App, path: &Path) -> PullRequestSection {
    let repository = app.project_list.git_repo_for(path);
    let can_fetch = match repository.map_or(&RepositoryProbe::Awaiting, |repo| &repo.repo_info) {
        RepositoryProbe::Awaiting => true,
        RepositoryProbe::NoRepository => false,
        RepositoryProbe::Repository(repo) => repo.remotes.listed().iter().any(|remote| {
            remote
                .url
                .as_deref()
                .and_then(ci::parse_owner_repo)
                .is_some()
        }),
    };
    if !can_fetch && repository.is_none_or(|repo| repo.pr_data.info().is_none()) {
        return PullRequestSection::default();
    }
    let check_polls = app
        .project_list
        .fetch_url_for(path)
        .and_then(|url| ci::parse_owner_repo(&url))
        .map(|repo| app.net.github.pr_check_poll_numbers(&repo))
        .unwrap_or_default();
    repository.map_or_else(
        || PullRequestSection {
            state: PullRequestSectionState::Loading,
            ..PullRequestSection::default()
        },
        |repo| match &repo.pr_data {
            ProjectPrData::Loading(Some(info)) if !can_fetch => {
                section_from_pr_info(info, PullRequestSectionState::Stale, &check_polls)
            },
            data => build_pull_request_section(data, &check_polls),
        },
    )
}

fn build_pull_request_section(
    data: &ProjectPrData,
    pr_check_polls: &HashSet<u32>,
) -> PullRequestSection {
    match data {
        ProjectPrData::Unfetched | ProjectPrData::Loading(_) => PullRequestSection {
            state: PullRequestSectionState::Loading,
            ..PullRequestSection::default()
        },
        ProjectPrData::Loaded(info) => {
            section_from_pr_info(info, PullRequestSectionState::Loaded, pr_check_polls)
        },
        ProjectPrData::Unavailable(unavailable) => unavailable.stale.as_ref().map_or_else(
            || PullRequestSection {
                state: PullRequestSectionState::Unavailable,
                unavailable_reason: Some(unavailable.reason),
                fetched_at: unavailable.fetched_at.clone(),
                ..PullRequestSection::default()
            },
            |info| {
                let mut section =
                    section_from_pr_info(info, PullRequestSectionState::Stale, pr_check_polls);
                section.unavailable_reason = Some(unavailable.reason);
                section
            },
        ),
    }
}

fn section_from_pr_info(
    info: &ProjectPrInfo,
    state: PullRequestSectionState,
    pr_check_polls: &HashSet<u32>,
) -> PullRequestSection {
    let rows = info
        .open
        .iter()
        .map(|pull_request| {
            pull_request_row(
                pull_request,
                &info.default_branch,
                PullRequestPolling::from(pr_check_polls.contains(&pull_request.number)),
            )
        })
        .collect();
    PullRequestSection {
        state: if info.open.is_empty() {
            PullRequestSectionState::HiddenConfirmedEmpty
        } else {
            state
        },
        rows,
        fetched_at: Some(info.fetched_at.clone()),
        unavailable_reason: None,
        completeness: Some(info.completeness),
    }
}

fn pull_request_row(
    info: &PullRequestInfo,
    default_branch: &str,
    polling: PullRequestPolling,
) -> PullRequestRow {
    PullRequestRow {
        number: info.number,
        title: info.title.clone(),
        url: info.url.clone(),
        state_label: info.state.label(),
        polling,
        branch: info.branch_label(default_branch),
        base: info.base.clone(),
    }
}

/// Convert each `RemoteInfo` into a render-ready `RemoteRow`, shortening
/// the URL when it begins with `default_host` and collapsing missing
/// tracked refs / ahead-behind values to placeholder runes.
fn build_remote_rows(repo: &RepoInfo, default_host: &str, current_branch: &str) -> RemoteRows {
    let failures = GitReadFailures::of(None, Some(repo));
    if failures.field_health(DetailField::RemotesUnreadable) == GitReadHealth::Unreadable {
        return RemoteRows::Unreadable;
    }
    RemoteRows::Listed(
        repo.remotes
            .listed()
            .iter()
            .enumerate()
            .map(|(index, remote)| {
                let icon = match remote.kind {
                    RemoteKind::Fork => GIT_FORK,
                    RemoteKind::Clone => GIT_CLONE,
                };
                let display_url = remote
                    .url
                    .as_deref()
                    .map_or_else(String::new, |raw| shorten_remote_url(raw, default_host));
                let tracked_ref =
                    if failures.remote_tracked_health(index) == GitReadHealth::Unreadable {
                        TrackedRef::Unreadable
                    } else {
                        remote.tracked_ref.clone()
                    };
                let status = if failures.remote_sync_health(index) == GitReadHealth::Unreadable {
                    Comparison::Unreadable
                } else {
                    remote.ahead_behind
                };
                let push_annotation = format_push_annotation(&remote.push);
                RemoteRow {
                    name: remote.name.clone(),
                    icon,
                    display_url,
                    branch: current_branch.to_string(),
                    tracked_ref,
                    status,
                    full_url: remote.url.clone(),
                    push_annotation,
                }
            })
            .collect(),
    )
}

/// Pre-format the `↛ push disabled` annotation rendered after the
/// status column in the Remotes table. Returns `None` for enabled
/// remotes — rendering then leaves the slot empty.
fn format_push_annotation(push: &PushState) -> Option<String> {
    let PushState::Disabled { reason } = push else {
        return None;
    };
    let suffix = match reason {
        PushDisabledReason::KnownSentinel(s) => Some(s.label()),
        PushDisabledReason::NoPushUrl => None,
    };
    Some(suffix.map_or_else(
        || "\u{21A0} push disabled".to_string(),
        |label| format!("\u{21A0} push disabled ({label})"),
    ))
}

/// If `url` starts with `default_host`, return `owner/repo` (stripping
/// `.git` suffix); otherwise return the full URL.
fn shorten_remote_url(url: &str, default_host: &str) -> String {
    let stripped = url.strip_prefix(default_host).unwrap_or(url);
    stripped
        .strip_suffix(GIT_DIR)
        .unwrap_or(stripped)
        .to_string()
}

/// Collect worktree info from a worktree group item.
///
/// Branch is read from cached `CheckoutInfo` populated by the watcher (no
/// shell-out). Ahead/behind is computed via git shell-out — this is the
/// expensive part — and is the reason the caller wraps this in
/// `App::worktree_summary_or_compute` so each `(group, data_generation)`
/// pair pays at most once.
fn worktrees_from_item(app: &App, item: &RootItem) -> Vec<WorktreeInfo> {
    let (paths_and_names, primary_path) = match item {
        RootItem::Worktrees(group) => {
            let primary_path = group.primary.path().clone();
            let entries: Vec<(AbsolutePath, String)> = group
                .iter_entries()
                .filter(|p| p.visibility() != Visibility::Dismissed)
                .map(|p| (p.path().clone(), p.root_directory_name().into_string()))
                .collect();
            (entries, primary_path)
        },
        _ => return Vec::new(),
    };

    // The branch each linked worktree is measured against — the primary's
    // current branch. `None` if the primary's HEAD isn't on a branch.
    let primary_branch = app
        .project_list
        .git_info_for(primary_path.as_path())
        .and_then(|info| info.head.branch_name().map(str::to_string));

    paths_and_names
        .into_iter()
        .map(|(path, name)| {
            let branch = app
                .project_list
                .git_info_for(path.as_path())
                .and_then(|info| info.head.branch_name().map(str::to_string));
            // The primary is the baseline: nothing to track against and no
            // delta. Linked worktrees compare their HEAD to the primary's.
            let is_primary = path.as_path() == primary_path.as_path();
            let (tracked, ahead_behind) = if is_primary {
                (None, Comparison::NotCompared)
            } else {
                (
                    primary_branch.clone(),
                    project::worktree_ahead_behind_primary(path.as_path(), primary_path.as_path()),
                )
            };
            WorktreeInfo {
                name,
                path: path.display().to_string(),
                branch,
                tracked,
                ahead_behind,
            }
        })
        .collect()
}
pub(super) fn resolve_worktrees(app: &App, worktree_item: Option<&RootItem>) -> Vec<WorktreeInfo> {
    worktree_item.map_or_else(Vec::new, |item| {
        app.panes
            .git
            .worktree_summary_or_compute(item.path().as_path(), || worktrees_from_item(app, item))
    })
}
/// Build the submodule render overlay (`tracks`, `pinned_commit`).
/// Returns `None` when the parent has no pinned commit recorded —
/// without it there's nothing meaningful to render in the overlay.
pub(super) fn build_submodule_context(submodule: &Submodule) -> Option<SubmoduleContext> {
    let pinned_commit = match &submodule.commit {
        PinnedCommit::Absent => return None,
        PinnedCommit::Sha(_) | PinnedCommit::Unreadable => submodule.commit.clone(),
    };
    Some(SubmoduleContext {
        tracks: submodule.branch.clone(),
        pinned_commit,
    })
}

#[cfg(test)]
mod git_field_tests {
    use super::*;
    use crate::project::CommitDate;
    use crate::project::Comparison;
    use crate::project::FirstCommit;
    use crate::project::ProjectPrUnavailable;
    use crate::project::ResolvedBranch;
    use crate::tui::panes::DescriptionState;
    use crate::tui::panes::pane_data;
    use crate::tui::panes::pane_data::DetailField;
    use crate::tui::panes::pane_data::constants::HEAD_UNREADABLE_LABEL;
    use crate::tui::state::AvailabilityStatus;

    fn pull_request_fixture() -> ProjectPrInfo {
        ProjectPrInfo {
            open:           vec![PullRequestInfo {
                number:     7,
                title:      "Keep the page stable".to_string(),
                url:        "https://github.com/acme/proof/pull/7".to_string(),
                state:      crate::project::PullRequestState::Ready,
                head:       "topic".to_string(),
                head_owner: None,
                head_repo:  None,
                base:       "main".to_string(),
            }],
            default_branch: "main".to_string(),
            fetched_at:     "2026-11-08T12:00:00Z".to_string(),
            completeness:   PullRequestCompleteness::Complete,
            viewer_login:   "viewer".to_string(),
            owner_repo:     crate::ci::OwnerRepo::new("acme", "proof"),
        }
    }

    fn completed_pull_requests() -> [(ProjectPrData, PullRequestSectionState); 4] {
        let info = pull_request_fixture();
        let empty = ProjectPrInfo {
            open: Vec::new(),
            ..info.clone()
        };
        let unavailable = ProjectPrUnavailable {
            reason:     PullRequestUnavailableReason::Network,
            stale:      None,
            fetched_at: None,
        };
        [
            (
                ProjectPrData::Loaded(empty),
                PullRequestSectionState::HiddenConfirmedEmpty,
            ),
            (
                ProjectPrData::Loaded(info.clone()),
                PullRequestSectionState::Loaded,
            ),
            (
                ProjectPrData::Unavailable(unavailable.clone()),
                PullRequestSectionState::Unavailable,
            ),
            (
                ProjectPrData::Unavailable(ProjectPrUnavailable {
                    stale: Some(info),
                    ..unavailable
                }),
                PullRequestSectionState::Stale,
            ),
        ]
    }

    fn pull_request_app(mode: tui_pane::ColorMode) -> anyhow::Result<App> {
        use anyhow::Context;

        use crate::tui::test_support;
        let mut app = test_support::project_app();
        app.framework.color_mode = mode;
        app.project_list
            .ensure_git_repo_for(Path::new(test_support::PROJECT_PATH))
            .context("repo")?
            .repo_info = RepositoryProbe::Repository(RepoInfo {
            remotes: RemoteList::Listed(vec![crate::project::RemoteInfo {
                name:         "origin".to_string(),
                url:          Some("https://github.com/acme/proof".to_string()),
                owner:        Some("acme".to_string()),
                repo:         Some("proof".to_string()),
                tracked_ref:  TrackedRef::Untracked,
                ahead_behind: Comparison::NotCompared,
                kind:         RemoteKind::Clone,
                push:         PushState::Enabled {
                    push_url: "https://github.com/acme/proof".to_string(),
                },
            }]),
            ..RepoInfo::default()
        });
        Ok(app)
    }

    fn draw_pull_request_frame(
        app: &mut App,
        state: ProjectPrData,
        expected: PullRequestSectionState,
        dimensions: (u16, u16),
    ) -> anyhow::Result<(usize, String)> {
        use anyhow::Context;

        use crate::tui::panes::PaneId;
        use crate::tui::test_support;
        let path = Path::new(test_support::PROJECT_PATH);
        app.project_list
            .ensure_git_repo_for(path)
            .context("repo")?
            .pr_data = state;
        let section = build_git_detail_fields(app, path).pull_requests;
        assert_eq!(section.state, expected);
        app.panes.git.set_content(GitData {
            pull_requests: section,
            checkout: GitCheckout::Branch {
                name:     "main".to_string(),
                relation: HeadRelation::Default,
            },
            ..git_data()
        });
        let buffer = test_support::draw_pane_in_window_at_size(
            app,
            PaneId::Git,
            dimensions.0,
            dimensions.1,
            12,
        )?;
        if expected == PullRequestSectionState::HiddenConfirmedEmpty {
            assert!(
                !test_support::rows(&buffer)
                    .join("\n")
                    .contains("Pull Requests")
            );
        }
        Ok((
            app.panes.git.viewport.content_height(),
            test_support::rows(&buffer)
                .last()
                .cloned()
                .context("bottom border")?,
        ))
    }

    #[test]
    fn pull_request_loading_keeps_the_first_frame_height_and_page_count() -> anyhow::Result<()> {
        use anyhow::Context;
        use tui_pane::ColorMode;

        use crate::tui::test_support;
        for dimensions in [(80, 17), (95, 24)] {
            for mode in [ColorMode::Colored, ColorMode::NoColor] {
                let mut app = pull_request_app(mode)?;
                let repository = app
                    .project_list
                    .ensure_git_repo_for(Path::new(test_support::PROJECT_PATH))
                    .context("repo")?
                    .repo_info
                    .clone();
                let unfetched = draw_pull_request_frame(
                    &mut app,
                    ProjectPrData::Unfetched,
                    PullRequestSectionState::Loading,
                    dimensions,
                )?;
                let loading = draw_pull_request_frame(
                    &mut app,
                    ProjectPrData::Loading(None),
                    PullRequestSectionState::Loading,
                    dimensions,
                )?;
                assert_eq!(
                    unfetched, loading,
                    "{dimensions:?}: unfetched and loading pages"
                );
                for (finished, expected) in completed_pull_requests() {
                    app.project_list
                        .ensure_git_repo_for(Path::new(test_support::PROJECT_PATH))
                        .context("repo")?
                        .repo_info = repository.clone();
                    app.panes.git = crate::tui::panes::GitPane::new();
                    let finished_frame =
                        draw_pull_request_frame(&mut app, finished.clone(), expected, dimensions)?;
                    if matches!(
                        expected,
                        PullRequestSectionState::Loaded | PullRequestSectionState::Stale
                    ) {
                        app.project_list
                            .ensure_git_repo_for(Path::new(test_support::PROJECT_PATH))
                            .context("repo")?
                            .repo_info = RepositoryProbe::Repository(RepoInfo {
                            remotes: RemoteList::Unreadable,
                            ..RepoInfo::default()
                        });
                        let after = draw_pull_request_frame(
                            &mut app,
                            finished.clone(),
                            expected,
                            dimensions,
                        )?;
                        assert_eq!(
                            finished_frame, after,
                            "unreadable remotes retain the finished frame"
                        );
                        let buffer =
                            test_support::draw_pane(&mut app, crate::tui::panes::PaneId::Git, 70)?;
                        test_support::position(&buffer, "Keep the page stable")?;
                    }
                    for remotes in [RemoteList::Listed(Vec::new()), RemoteList::Unreadable] {
                        app.project_list
                            .ensure_git_repo_for(Path::new(test_support::PROJECT_PATH))
                            .context("repo")?
                            .repo_info = RepositoryProbe::Repository(RepoInfo {
                            remotes,
                            ..RepoInfo::default()
                        });
                        for state in [ProjectPrData::Unfetched, ProjectPrData::Loading(None)] {
                            draw_pull_request_frame(
                                &mut app,
                                state,
                                PullRequestSectionState::HiddenConfirmedEmpty,
                                dimensions,
                            )?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn submodule_pull_requests_finish_in_the_submodule_pane() -> anyhow::Result<()> {
        use anyhow::Context;

        use crate::project::AbsolutePath;
        use crate::project::GitRepo;
        use crate::project::PinnedCommit;
        use crate::project::ProjectInfo;
        use crate::project::Submodule;
        use crate::tui::panes::PaneId;
        use crate::tui::test_support;
        let mut app = pull_request_app(tui_pane::ColorMode::Colored)?;
        let parent = Path::new(test_support::PROJECT_PATH);
        let path = AbsolutePath::from(parent.join("vendor/child"));
        let repository = app
            .project_list
            .git_repo_for(parent)
            .context("parent repository")?
            .repo_info
            .clone();
        app.project_list.replace_submodules(
            parent,
            vec![Submodule {
                name:          "child".to_string(),
                path:          path.clone(),
                relative_path: "vendor/child".to_string(),
                url:           Some("https://github.com/acme/proof".to_string()),
                branch:        None,
                commit:        PinnedCommit::Absent,
                project_info:  ProjectInfo::default(),
                git_repo:      Some(GitRepo {
                    repo_info: repository,
                    ..GitRepo::default()
                }),
            }],
        );
        app.project_list.rebuild_path_index();
        let owner =
            crate::ci::parse_owner_repo("https://github.com/acme/proof").context("owner")?;
        for (state, expected, visible) in [
            (
                ProjectPrData::Unfetched,
                PullRequestSectionState::Loading,
                "loading pull requests",
            ),
            (
                ProjectPrData::Loaded(pull_request_fixture()),
                PullRequestSectionState::Loaded,
                "Keep the page stable",
            ),
        ] {
            app.project_list.replace_pr_data_for_repo(&owner, &state);
            let section = build_git_detail_fields(&app, path.as_path()).pull_requests;
            assert_eq!(section.state, expected);
            app.panes.git.set_content(GitData {
                pull_requests: section,
                checkout: GitCheckout::Branch {
                    name:     "main".to_string(),
                    relation: HeadRelation::Default,
                },
                ..git_data()
            });
            let buffer = test_support::draw_pane(&mut app, PaneId::Git, 70)?;
            test_support::position(&buffer, visible)?;
        }
        Ok(())
    }

    fn git_data() -> GitData {
        GitData {
            checkout:           GitCheckout::AwaitingProbe,
            failed_reads:       GitReadFailures::default(),
            bisect:             None,
            submodule_ctx:      None,
            status:             None,
            vs_local:           crate::tui::panes::GitProbe::Probed(Comparison::NotCompared),
            local_main_branch:  crate::tui::panes::GitProbe::Probed(ResolvedBranch::NotFound),
            stars:              GithubStars::NotGithub,
            description:        DescriptionState::KnownEmpty,
            inception:          FirstCommit::AwaitingProbe,
            last_commit:        crate::tui::panes::GitProbe::Probed(CommitDate::NoCommits),
            last_fetched:       None,
            rate_limit_core:    super::QuotaObservation::Unobserved,
            rate_limit_graphql: super::QuotaObservation::Unobserved,
            github_status:      AvailabilityStatus::Reachable,
            pull_requests:      PullRequestSection::default(),
            remotes:            RemoteRows::default(),
            worktrees:          Vec::new(),
        }
    }

    #[test]
    fn broken_worktree_error_precedes_head_and_normal_worktree_omits_it() {
        let mut data = GitData {
            checkout: GitCheckout::from_probe(
                WorktreeHealth::Broken,
                HeadProbe::Probed(&HeadState::Unreadable),
                RepoBranchProbe::Awaiting,
                WorktreePlacement::NotLinked,
            ),
            ..git_data()
        };
        let fields = git_fields_from_data(&data);
        assert_eq!(fields[0], DetailField::WorktreeError);
        assert!(!fields.contains(&DetailField::Head));

        data.checkout = GitCheckout::AwaitingProbe;
        assert!(!git_fields_from_data(&data).contains(&DetailField::WorktreeError));
        data.checkout = GitCheckout::Branch {
            name:     "main".to_string(),
            relation: HeadRelation::Default,
        };
        let fields = git_fields_from_data(&data);
        assert_eq!(fields[0], DetailField::Head);
        assert!(!fields.contains(&DetailField::WorktreeError));
    }

    #[test]
    fn unreadable_head_shows_branch_row_as_head_unreadable() {
        let data = GitData {
            checkout: GitCheckout::from_probe(
                WorktreeHealth::Normal,
                HeadProbe::Probed(&HeadState::Unreadable),
                RepoBranchProbe::Awaiting,
                WorktreePlacement::NotLinked,
            ),
            ..git_data()
        };
        assert!(matches!(data.checkout, GitCheckout::UnreadableHead));
        let fields = git_fields_from_data(&data);
        assert_eq!(fields[0], DetailField::Head);
        assert!(!fields.contains(&DetailField::WorktreeError));
        let value = DetailField::Head.git_value(&data);
        assert_eq!(value, HEAD_UNREADABLE_LABEL);
        assert_ne!(value, "unborn");
        assert!(matches!(
            GitCheckout::from_probe(
                WorktreeHealth::Normal,
                HeadProbe::Awaiting,
                RepoBranchProbe::Awaiting,
                WorktreePlacement::NotLinked,
            ),
            GitCheckout::AwaitingProbe,
        ));
        let head = HeadState::Branch("main".into());
        assert!(matches!(
            GitCheckout::from_probe(
                WorktreeHealth::Normal,
                HeadProbe::Probed(&head),
                RepoBranchProbe::Probed(&ResolvedBranch::Named("main".into())),
                WorktreePlacement::NotLinked,
            ),
            GitCheckout::Branch {
                relation: HeadRelation::Default,
                ..
            },
        ));
        assert!(matches!(
            GitCheckout::from_probe(
                WorktreeHealth::Normal,
                HeadProbe::Probed(&head),
                RepoBranchProbe::Probed(&ResolvedBranch::Named("main".into())),
                WorktreePlacement::Linked,
            ),
            GitCheckout::Branch {
                relation: HeadRelation::Worktree,
                ..
            },
        ));
        assert!(matches!(
            WorktreePlacement::from(None),
            WorktreePlacement::NotLinked
        ));
    }

    #[test]
    fn stars_row_hidden_when_github_reachable_and_no_data() {
        // Pre-fetch state on a reachable GitHub: no stars yet, suppress
        // the row - same UX as the crates.io row pre-data on a healthy
        // service. The placeholder helper must report false.
        let mut data = git_data();
        data.stars = GithubStars::Pending;
        data.github_status = AvailabilityStatus::Reachable;
        let fields = git_fields_from_data(&data);
        assert!(
            !fields.contains(&DetailField::Stars),
            "no Stars row before data lands while GitHub is reachable"
        );
        assert!(!pane_data::github_stars_is_unreachable_placeholder(&data));
    }

    #[test]
    fn stars_row_shows_warning_when_github_unreachable_and_no_data() {
        let mut data = git_data();
        data.stars = GithubStars::Unreachable;
        data.github_status = AvailabilityStatus::Unreachable;
        let fields = git_fields_from_data(&data);
        assert!(!fields.contains(&DetailField::Stars));
        assert_eq!(
            fields
                .iter()
                .filter(|field| **field == DetailField::GithubStatus)
                .count(),
            1
        );
        assert!(pane_data::github_stars_is_unreachable_placeholder(&data));
        assert_eq!(DetailField::Stars.git_value(&data), "");
    }

    #[test]
    fn stars_row_shows_warning_when_github_rate_limited_and_no_data() {
        let mut data = git_data();
        data.stars = GithubStars::Unreachable;
        data.github_status = AvailabilityStatus::RateLimited;
        let fields = git_fields_from_data(&data);
        assert!(!fields.contains(&DetailField::Stars));
        assert_eq!(
            fields
                .iter()
                .filter(|field| **field == DetailField::GithubStatus)
                .count(),
            1
        );
        assert!(pane_data::github_stars_is_unreachable_placeholder(&data));
    }

    #[test]
    fn stars_row_shows_real_value_when_data_present_during_outage() {
        // Stars landed before the outage (or during a brief recovery).
        // Even with GitHub currently unreachable, the row renders the
        // real value - not the warning placeholder.
        let mut data = git_data();
        data.stars = GithubStars::Measured(42);
        data.github_status = AvailabilityStatus::Unreachable;
        let fields = git_fields_from_data(&data);
        assert!(fields.contains(&DetailField::Stars));
        assert_eq!(DetailField::Stars.git_value(&data), "⭐ 42");
        assert!(
            !pane_data::github_stars_is_unreachable_placeholder(&data),
            "real value present - no placeholder"
        );
    }

    #[test]
    fn stars_row_hidden_when_github_unauthenticated() {
        // Unauthenticated is not a service outage - don't surface the
        // "github unreachable" placeholder on the Stars row. The rate-limit
        // rows and the startup toast carry the `gh auth login` hint instead.
        let mut data = git_data();
        data.stars = GithubStars::Pending;
        data.github_status = AvailabilityStatus::Unauthenticated;
        let fields = git_fields_from_data(&data);
        assert!(
            !fields.contains(&DetailField::Stars),
            "no Stars placeholder when merely unauthenticated"
        );
        assert!(!pane_data::github_stars_is_unreachable_placeholder(&data));
    }

    #[test]
    fn branch_relation_with_unreadable_default_branch_is_unreadable_not_feature() {
        let unreadable = ResolvedBranch::Unreadable;
        assert!(matches!(
            HeadRelation::classify(
                "main",
                RepoBranchProbe::Probed(&unreadable),
                WorktreePlacement::NotLinked
            ),
            HeadRelation::Unreadable
        ));
        assert!(matches!(
            HeadRelation::classify(
                "main",
                RepoBranchProbe::Probed(&unreadable),
                WorktreePlacement::Linked
            ),
            HeadRelation::Worktree
        ));
        assert!(matches!(
            HeadRelation::classify(
                "main",
                RepoBranchProbe::Probed(&ResolvedBranch::NotFound),
                WorktreePlacement::NotLinked
            ),
            HeadRelation::Feature
        ));
        let data = GitData {
            checkout: GitCheckout::Branch {
                name:     "main".into(),
                relation: HeadRelation::Unreadable,
            },
            ..git_data()
        };
        assert_eq!(
            DetailField::Head.git_value(&data),
            format!("main · {GIT_UNREADABLE_LABEL}")
        );
        assert!(
            !DetailField::Head
                .git_value(&data)
                .contains(HeadRelation::Feature.label())
        );
        assert!(matches!(
            data.checkout.head_read_health(),
            GitReadHealth::Unreadable
        ));
    }
}
