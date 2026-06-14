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
use super::NO_REMOTE_SYNC;
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
use super::package_data;
use super::project;

pub fn git_fields_from_data(data: &GitData) -> Vec<DetailField> {
    let mut fields = Vec::new();
    if data.head.is_some() {
        fields.push(DetailField::Head);
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
    if data.vs_local.is_some() {
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
    if data.inception.is_some() {
        fields.push(DetailField::Inception);
    }
    if data.last_commit.is_some() {
        fields.push(DetailField::LastCommit);
    }
    if data.last_fetched.is_some() {
        fields.push(DetailField::LastFetched);
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
/// How the current `HEAD` relates to the repo, rendered as a qualifier
/// after the branch name in the Git pane's Branch row (`main · default`,
/// `feature/x · feature`, `feature/x · worktree`). `None` for detached and
/// unborn checkouts, whose value (`detached @ <sha>`, `unborn`) already
/// describes the state without a qualifier.
#[derive(Clone, Copy)]
pub enum HeadRelation {
    /// On the repo's default branch.
    Default,
    /// On a non-default branch in the primary checkout.
    Feature,
    /// On a branch in a linked worktree checkout.
    Worktree,
}

impl HeadRelation {
    /// Classify a branch `HEAD` against the repo's default branch and the
    /// checkout's worktree status. `None` when `HEAD` is not on a branch.
    fn classify(
        head: &HeadState,
        default_branch: Option<&str>,
        worktree_status: Option<&WorktreeStatus>,
    ) -> Option<Self> {
        let HeadState::Branch(name) = head else {
            return None;
        };
        Some(
            if worktree_status.is_some_and(WorktreeStatus::is_linked_worktree) {
                Self::Worktree
            } else if default_branch == Some(name.as_str()) {
                Self::Default
            } else {
                Self::Feature
            },
        )
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Feature => "feature",
            Self::Worktree => "worktree",
        }
    }
}

/// Per-pane data for the Git detail panel.
#[derive(Clone, Default)]
pub struct GitData {
    pub head:               Option<HeadState>,
    pub head_relation:      Option<HeadRelation>,
    pub bisect:             Option<BisectProgress>,
    pub status:             Option<GitStatus>,
    pub vs_local:           Option<String>,
    pub stars:              Option<u64>,
    pub description:        Option<String>,
    pub inception:          Option<String>,
    pub last_commit:        Option<String>,
    pub last_fetched:       Option<String>,
    pub rate_limit_core:    Option<RateLimitQuota>,
    pub rate_limit_graphql: Option<RateLimitQuota>,
    pub github_status:      AvailabilityStatus,
    pub pull_requests:      PullRequestSection,
    pub remotes:            Vec<RemoteRow>,
    pub worktrees:          Vec<WorktreeInfo>,
    /// Submodule-specific overlay. `Some` only when this `GitData` is
    /// built for a submodule pane — the renderer reads this to decide
    /// whether to emit the `Tracks` / `Pinned` rows. Submodule identity
    /// is conveyed by the project-list `(s)` marker and the pane's
    /// "Submodule — \<name\>" title, not by an About-section line.
    pub submodule_ctx:      Option<SubmoduleContext>,
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
    const fn from_polling(is_polling: bool) -> Self {
        if is_polling { Self::Active } else { Self::Idle }
    }

    pub const fn is_polling(&self) -> bool { matches!(self, Self::Active) }
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
    pub pinned_commit: String,
}

impl GitData {
    /// Whether the repo has no remotes — drives the `(📁 local)` branch
    /// annotation in the git pane.
    pub const fn is_local(&self) -> bool { self.remotes.is_empty() }
}

/// Per-remote row rendered in the Git pane's Remotes table. Pre-formatted
/// for display — status and `tracked_ref` already reduce to rendered text.
#[derive(Clone)]
pub struct RemoteRow {
    pub name:            String,
    pub icon:            &'static str,
    pub display_url:     String,
    /// Local branch (current `HEAD`) compared against `tracked_ref` —
    /// the source side of the `status` ahead/behind delta.
    pub branch:          String,
    pub tracked_ref:     String,
    pub status:          String,
    pub full_url:        Option<String>,
    /// Pre-formatted push-disabled annotation (e.g. `"↛ push disabled"`
    /// or `"↛ push disabled (DISABLED)"`). `None` when push is enabled.
    pub push_annotation: Option<String>,
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
    pub ahead_behind: Option<(usize, usize)>,
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
    if pos < data.remotes.len() {
        return data.remotes.get(pos).map(GitRow::Remote);
    }
    let pos = pos - data.remotes.len();
    data.worktrees.get(pos).map(GitRow::Worktree)
}
pub(super) struct GitDetailFields {
    pub(super) head:               Option<HeadState>,
    pub(super) head_relation:      Option<HeadRelation>,
    pub(super) bisect:             Option<BisectProgress>,
    pub(super) path:               Option<GitStatus>,
    pub(super) vs_local:           Option<String>,
    pub(super) stars:              Option<u64>,
    pub(super) description:        Option<String>,
    pub(super) inception:          Option<String>,
    pub(super) last_commit:        Option<String>,
    pub(super) last_fetched:       Option<String>,
    pub(super) rate_limit_core:    Option<RateLimitQuota>,
    pub(super) rate_limit_graphql: Option<RateLimitQuota>,
    pub(super) github_status:      AvailabilityStatus,
    pub(super) pull_requests:      PullRequestSection,
    pub(super) remotes:            Vec<RemoteRow>,
}

pub(super) fn build_git_detail_fields(app: &App, abs_path: &Path) -> GitDetailFields {
    let git_repo = app.project_list.git_repo_for(abs_path);
    let repo_info = git_repo.and_then(|repo| repo.repo_info.as_ref());
    let checkout = app.project_list.git_info_for(abs_path);

    let head = checkout.map(|info| info.head.clone());
    let bisect = checkout.and_then(|info| info.bisect.clone());
    let local_main_branch = repo_info.and_then(|repo| repo.local_main_branch.clone());
    let head_relation = head.as_ref().and_then(|head| {
        HeadRelation::classify(
            head,
            local_main_branch.as_deref(),
            app.project_list.worktree_status_for(abs_path),
        )
    });
    let local_main_label = local_main_branch
        .as_deref()
        .unwrap_or_else(|| app.config.current().tui.main_branch.as_str());
    let vs_local = checkout
        .and_then(|info| info.ahead_behind_local)
        .map(|ahead_behind| {
            formatting::format_ahead_behind_against(ahead_behind, local_main_label)
        });
    let github = git_repo.and_then(|repo| repo.github_info.as_ref());
    let stars = github.map(|g| g.stars);
    let description = github.and_then(|g| g.description.clone());
    let inception = repo_info
        .and_then(|repo| repo.first_commit.as_deref())
        .map(formatting::format_timestamp);
    let last_commit = checkout
        .and_then(|info| info.last_commit.as_deref())
        .map(formatting::format_timestamp);
    let last_fetched = repo_info
        .and_then(|repo| repo.last_fetched.as_deref())
        .map(formatting::format_timestamp);
    let default_host = app.config.current().tui.default_remote_host_url.clone();
    let current_branch = head
        .as_ref()
        .and_then(HeadState::branch_name)
        .unwrap_or("-");
    let remotes = repo_info.map_or_else(Vec::new, |repo| {
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
        head,
        head_relation,
        bisect,
        path: app.project_list.git_status_for(abs_path),
        vs_local,
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
        polling:     PullRequestPolling::from_polling(is_polling),
        branch:      info.branch_label(default_branch),
        base:        info.base.clone(),
    }
}

/// Convert each `RemoteInfo` into a render-ready `RemoteRow`, shortening
/// the URL when it begins with `default_host` and collapsing missing
/// tracked refs / ahead-behind values to placeholder runes.
fn build_remote_rows(repo: &RepoInfo, default_host: &str, current_branch: &str) -> Vec<RemoteRow> {
    repo.remotes
        .iter()
        .map(|remote| {
            let icon = match remote.kind {
                RemoteKind::Fork => GIT_FORK,
                RemoteKind::Clone => GIT_CLONE,
            };
            let display_url = remote
                .url
                .as_deref()
                .map_or_else(String::new, |raw| shorten_remote_url(raw, default_host));
            let tracked_ref = remote
                .tracked_ref
                .clone()
                .unwrap_or_else(|| NO_REMOTE_SYNC.to_string());
            let status = formatting::format_ahead_behind(remote.ahead_behind);
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
        .collect()
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
                (None, None)
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
pub(super) fn resolve_worktrees(app: &App, wt_item: Option<&RootItem>) -> Vec<WorktreeInfo> {
    wt_item.map_or_else(Vec::new, |item| {
        app.panes
            .git
            .worktree_summary_or_compute(item.path().as_path(), || worktrees_from_item(app, item))
    })
}
/// Build the submodule render overlay (`tracks`, `pinned_commit`).
/// Returns `None` when the parent has no pinned commit recorded —
/// without it there's nothing meaningful to render in the overlay.
pub(super) fn build_submodule_context(submodule: &Submodule) -> Option<SubmoduleContext> {
    let pinned_commit = submodule.commit.clone()?;
    Some(SubmoduleContext {
        tracks: submodule.branch.clone(),
        pinned_commit,
    })
}
