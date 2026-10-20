use super::AbsolutePath;
use super::App;
use super::AvailabilityStatus;
use super::BisectProgress;
use super::DetailField;
use super::GIT_CLONE;
use super::GIT_DIR;
use super::GIT_FORK;
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
use super::package_data;
use super::project;
use crate::constants::GIT_UNREADABLE_LABEL;
use crate::project::CommitDate;
use crate::project::Comparison;
use crate::project::FirstCommit;
use crate::project::PinnedCommit;
use crate::project::ResolvedBranch;
use crate::project::TrackedRef;
use crate::project::WorktreeHealth;
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

impl WorktreePlacement {
    const fn of(status: Option<&WorktreeStatus>) -> Self {
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

/// Per-pane data for the Git detail panel.
#[derive(Clone, Default)]
pub struct GitData {
    pub checkout:                    GitCheckout,
    pub(in crate::tui) failed_reads: GitReadFailures,
    pub bisect:                      Option<BisectProgress>,
    pub status:                      Option<GitStatus>,
    pub vs_local:                    GitProbe<Comparison>,
    pub local_main_branch:           GitProbe<ResolvedBranch>,
    pub stars:                       Option<u64>,
    pub description:                 Option<String>,
    pub inception:                   FirstCommit,
    pub last_commit:                 GitProbe<CommitDate>,
    pub last_fetched:                Option<String>,
    pub rate_limit_core:             Option<RateLimitQuota>,
    pub rate_limit_graphql:          Option<RateLimitQuota>,
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
    pub const fn head_value_is_unreadable(&self) -> bool {
        matches!(
            self,
            Self::UnreadableHead
                | Self::Branch {
                    relation: HeadRelation::Unreadable,
                    ..
                }
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

impl PullRequestPolling {
    pub const fn is_polling(&self) -> bool { matches!(self, Self::Active) }
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

impl GitData {
    /// Whether the repo has no remotes — drives the `(📁 local)` branch
    /// annotation in the git pane.
    pub const fn is_local(&self) -> bool {
        matches!(&self.remotes, RemoteRows::Listed(rows) if rows.is_empty())
    }
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
    pub const fn is_unreadable(&self) -> bool { matches!(self, Self::Unreadable) }
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
        GitCheckout::AwaitingProbe => {},
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
    // Show the Stars row when:
    //   - the count has landed (real value), OR
    //   - GitHub is confirmed unreachable / rate-limited (placeholder in warning color, mirrors the
    //     crates.io unreachable row behavior on the Package pane).
    if data.stars.is_some() || package_data::github_stars_is_unreachable_placeholder(data) {
        fields.push(DetailField::Stars);
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
    if data.remotes.is_unreadable() {
        fields.push(DetailField::RemotesUnreadable);
    }
    for field in data.failed_reads.fields() {
        if !(fields.contains(&field)
            || field == DetailField::Head && matches!(data.checkout, GitCheckout::Broken))
        {
            fields.push(field);
        }
    }
    // Rate-limit rows are always shown so the section structure stays
    // stable across fetch state; rendering handles the empty-quota
    // case.
    fields.push(DetailField::RateLimitCore);
    fields.push(DetailField::RateLimitGraphQl);
    if !data.worktrees.is_empty() {
        // Worktree count is appended by the render function, not as a field.
    }
    fields
}

pub fn git_has_description_row(data: &GitData) -> bool {
    data.description
        .as_deref()
        .map(str::trim)
        .is_some_and(|description| !description.is_empty())
}

pub fn git_row_at(data: &GitData, pos: usize) -> Option<GitRow<'_>> {
    let description_rows = usize::from(git_has_description_row(data));
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
    pub(super) stars:              Option<u64>,
    pub(super) description:        Option<String>,
    pub(super) inception:          FirstCommit,
    pub(super) last_commit:        GitProbe<CommitDate>,
    pub(super) last_fetched:       Option<String>,
    pub(super) rate_limit_core:    Option<RateLimitQuota>,
    pub(super) rate_limit_graphql: Option<RateLimitQuota>,
    pub(super) github_status:      AvailabilityStatus,
    pub(super) pull_requests:      PullRequestSection,
    pub(super) remotes:            RemoteRows,
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
    let stars = github.map(|g| g.stars);
    let description = github.and_then(|g| g.description.clone());
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
    let pr_check_polls = app
        .project_list
        .fetch_url_for(abs_path)
        .and_then(|url| ci::parse_owner_repo(&url))
        .map(|repo| app.net.github.pr_check_poll_numbers(&repo))
        .unwrap_or_default();
    let pull_requests = git_repo
        .map(|repo| build_pull_request_section(&repo.pr_data, &pr_check_polls))
        .unwrap_or_default();
    let rate_limit = app.net.rate_limit();
    GitDetailFields {
        failed_reads: GitReadFailures::of(checkout, repo_info),
        checkout: GitCheckout::from_probe(
            app.project_list
                .at_path(abs_path)
                .map_or(WorktreeHealth::Normal, |info| info.worktree_health),
            head,
            local_main_probe,
            WorktreePlacement::of(app.project_list.worktree_status_for(abs_path)),
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
        rate_limit_core: rate_limit.core,
        rate_limit_graphql: rate_limit.graphql,
        github_status: app.net.github_status(),
        pull_requests,
        remotes,
    }
}

fn build_pull_request_section(
    data: &ProjectPrData,
    pr_check_polls: &HashSet<u32>,
) -> PullRequestSection {
    match data {
        ProjectPrData::Unfetched => PullRequestSection::default(),
        ProjectPrData::Loading(_) => PullRequestSection {
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
                pr_check_polls.contains(&pull_request.number),
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
    is_polling: bool,
) -> PullRequestRow {
    PullRequestRow {
        number:      info.number,
        title:       info.title.clone(),
        url:         info.url.clone(),
        state_label: info.state.label(),
        polling:     PullRequestPolling::from(is_polling),
        branch:      info.branch_label(default_branch),
        base:        info.base.clone(),
    }
}

/// Convert each `RemoteInfo` into a render-ready `RemoteRow`, shortening
/// the URL when it begins with `default_host` and collapsing missing
/// tracked refs / ahead-behind values to placeholder runes.
fn build_remote_rows(repo: &RepoInfo, default_host: &str, current_branch: &str) -> RemoteRows {
    let failures = GitReadFailures::of(None, Some(repo));
    if failures.field_failed(DetailField::RemotesUnreadable) {
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
                let tracked_ref = if failures.remote_tracked_failed(index) {
                    TrackedRef::Unreadable
                } else {
                    remote.tracked_ref.clone()
                };
                let status = if failures.remote_sync_failed(index) {
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

/// Check whether a `RootItem` currently renders as a worktree group.
pub(super) fn is_worktree_group(item: &RootItem) -> bool {
    matches!(item, RootItem::Worktrees(group) if group.renders_as_group())
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
    use crate::project::ResolvedBranch;
    use crate::tui::panes::pane_data;
    use crate::tui::panes::pane_data::DetailField;
    use crate::tui::panes::pane_data::constants::HEAD_UNREADABLE_LABEL;
    use crate::tui::state::AvailabilityStatus;

    fn git_data() -> GitData {
        GitData {
            checkout:           GitCheckout::AwaitingProbe,
            failed_reads:       GitReadFailures::default(),
            bisect:             None,
            submodule_ctx:      None,
            status:             None,
            vs_local:           crate::tui::panes::GitProbe::Probed(Comparison::NotCompared),
            local_main_branch:  crate::tui::panes::GitProbe::Probed(ResolvedBranch::NotFound),
            stars:              None,
            description:        None,
            inception:          FirstCommit::AwaitingProbe,
            last_commit:        crate::tui::panes::GitProbe::Probed(CommitDate::NoCommits),
            last_fetched:       None,
            rate_limit_core:    None,
            rate_limit_graphql: None,
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
            WorktreePlacement::of(None),
            WorktreePlacement::NotLinked
        ));
    }

    #[test]
    fn stars_row_hidden_when_github_reachable_and_no_data() {
        // Pre-fetch state on a reachable GitHub: no stars yet, suppress
        // the row - same UX as the crates.io row pre-data on a healthy
        // service. The placeholder helper must report false.
        let mut data = git_data();
        data.stars = None;
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
        // Outage state with no stars yet: the row surfaces with the
        // "github unreachable" placeholder so the user knows why the
        // value is empty. Mirrors `crates_io_row_shows_warning_...`.
        let mut data = git_data();
        data.stars = None;
        data.github_status = AvailabilityStatus::Unreachable;
        let fields = git_fields_from_data(&data);
        assert!(
            fields.contains(&DetailField::Stars),
            "Stars row must surface during outage so the user sees the placeholder"
        );
        assert!(pane_data::github_stars_is_unreachable_placeholder(&data));
        assert!(
            DetailField::Stars.git_value(&data).is_empty(),
            "git_value stays empty - the placeholder is added by the renderer overlay"
        );
    }

    #[test]
    fn stars_row_shows_warning_when_github_rate_limited_and_no_data() {
        // Rate-limit collapses to the same UX as unreachable on the
        // render side - the value cell isn't going to land, so warn the
        // user instead of leaving an invisible gap.
        let mut data = git_data();
        data.stars = None;
        data.github_status = AvailabilityStatus::RateLimited;
        let fields = git_fields_from_data(&data);
        assert!(fields.contains(&DetailField::Stars));
        assert!(pane_data::github_stars_is_unreachable_placeholder(&data));
    }

    #[test]
    fn stars_row_shows_real_value_when_data_present_during_outage() {
        // Stars landed before the outage (or during a brief recovery).
        // Even with GitHub currently unreachable, the row renders the
        // real value - not the warning placeholder.
        let mut data = git_data();
        data.stars = Some(42);
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
        data.stars = None;
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
        assert!(data.checkout.head_value_is_unreadable());
    }
}
