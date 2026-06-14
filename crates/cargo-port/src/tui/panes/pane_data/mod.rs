mod constants;
mod copy_data;
mod detail_data;
mod formatting;
mod git_data;
mod package_data;
mod pending;

use std::collections::HashSet;
use std::path::Path;

pub use copy_data::copy_payload_for_ci;
pub use copy_data::copy_payload_for_git;
pub use copy_data::copy_payload_for_lints;
pub use copy_data::copy_payload_for_output;
pub use copy_data::copy_payload_for_package;
pub use copy_data::copy_payload_for_targets;
pub use detail_data::DetailPaneData;
pub use detail_data::build_pane_data;
pub use detail_data::build_pane_data_for_member;
pub use detail_data::build_pane_data_for_submodule;
pub use detail_data::build_pane_data_for_vendored;
pub use detail_data::build_pane_data_for_workspace_ref;
pub use detail_data::max_top_pane_inner_height;
pub use formatting::format_ahead_behind;
#[cfg(test)]
pub use formatting::format_ahead_behind_against;
use formatting::format_bisect_progress;
pub use formatting::format_date;
pub use formatting::format_duration;
use formatting::format_rate_limit_bucket;
pub use formatting::format_time;
pub use formatting::format_timestamp;
pub use git_data::GitData;
pub use git_data::GitRow;
#[cfg(test)]
pub use git_data::PullRequestPolling;
pub use git_data::PullRequestRow;
pub use git_data::PullRequestSection;
pub use git_data::PullRequestSectionState;
pub use git_data::RemoteRow;
pub use git_data::WorktreeInfo;
pub use git_data::git_fields_from_data;
pub use git_data::git_has_description_row;
pub use git_data::git_row_at;
pub use package_data::PackageData;
#[cfg(test)]
pub use package_data::PackagePresence;
pub use package_data::PackageRow;
pub use package_data::PackageSection;
use package_data::or_dash;
pub use pending::CiFetchKind;
pub use pending::PendingCiFetch;
pub use pending::PendingExampleRun;
use ratatui::layout::Rect;
use tui_pane::CopyLabel;
use tui_pane::CopyPayload;
use tui_pane::CopySelectionResult;

pub(super) use self::constants::CRATES_IO_UNREACHABLE;
use self::constants::PROJECT_LIBS_LABEL;
use self::constants::PROJECT_MEMBERS_LABEL;
use self::constants::PROJECT_PROC_MACROS_LABEL;
use self::constants::PROJECT_SUBMODULES_LABEL;
use self::constants::PROJECT_VENDORED_LABEL;
use self::constants::TESTS_DOC_LABEL;
use self::constants::TESTS_INTEGRATION_LABEL;
use self::constants::TESTS_UNIT_LABEL;
use super::EmptyDescriptionBehavior;
pub use super::ci::CiData;
#[cfg(test)]
pub use super::ci::CiEmptyState;
use super::constants::TESTS_IGNORED_LABEL;
use super::constants::TESTS_TOTAL_LABEL;
use super::git;
pub use super::lints::LintsData;
#[cfg(test)]
pub use super::lints::LintsProjectKind;
use super::package;
pub use super::targets::BuildMode;
pub use super::targets::RunTargetKind;
pub use super::targets::TargetEntry;
#[cfg(test)]
pub use super::targets::TargetSource;
pub use super::targets::TargetsData;
use crate::ci;
use crate::ci::CiStatus;
use crate::constants::GIT_CLONE;
use crate::constants::GIT_DIR;
use crate::constants::GIT_FORK;
use crate::constants::NO_REMOTE_SYNC;
use crate::http::RateLimitQuota;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::BisectProgress;
use crate::project::Cargo;
use crate::project::GitStatus;
use crate::project::HeadState;
use crate::project::NonRustProject;
use crate::project::Package;
use crate::project::PackageRecord;
use crate::project::ProjectPrData;
use crate::project::ProjectPrInfo;
use crate::project::ProjectType;
use crate::project::PullRequestCompleteness;
use crate::project::PullRequestInfo;
use crate::project::PullRequestUnavailableReason;
use crate::project::PushDisabledReason;
use crate::project::PushState;
use crate::project::RemoteKind;
use crate::project::RepoInfo;
use crate::project::RootItem;
use crate::project::RustInfo;
use crate::project::RustProject;
use crate::project::Submodule;
use crate::project::TestCounts;
use crate::project::VendoredPackage;
use crate::project::Visibility;
use crate::project::Workspace;
use crate::project::WorktreeStatus;
use crate::tui::app::App;
use crate::tui::app::AvailabilityStatus;
use crate::tui::constants::TARGET_KIND_BENCH_LABEL;
use crate::tui::constants::TARGET_KIND_BIN_LABEL;
use crate::tui::constants::TARGET_KIND_EXAMPLE_LABEL;
use crate::tui::project_list::ProjectList;
use crate::tui::render;
use crate::tui::state::ServiceStatus;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailField {
    Worktrees,
    DeletedWorktrees,
    Path,
    Targets,
    Disk,
    /// Submodule overlay: `.gitmodules` tracking branch (the `branch =` line).
    Tracks,
    /// Submodule overlay: parent repo's pinned commit (`git ls-tree HEAD`).
    Pinned,
    /// Bytes consumed by the `target/` subtree rooted at the project.
    /// Shown alongside Disk when the walker has reported a breakdown.
    DiskTarget,
    /// Bytes under the project root that are *not* inside a `target/`
    /// subtree (source, docs, .git, etc.).
    DiskNonTarget,
    /// Sharer target: the workspace's `target_directory` lives outside
    /// `workspace_root` (e.g. redirected by `CARGO_TARGET_DIR` or a
    /// `.cargo/config.toml`). Byte total is filled by the cached
    /// out-of-tree walk (`BackgroundMsg::OutOfTreeTargetSize`) since the
    /// per-project walker never reaches there.
    DiskOutOfTreeTarget,
    Lint,
    Ci,
    Head,
    Bisect,
    GitStatus,
    VsLocal,
    Stars,
    Inception,
    LastCommit,
    LastFetched,
    RateLimitCore,
    RateLimitGraphQl,
    WorktreeError,
    Version,
    Edition,
    License,
    Homepage,
    Repository,
}

impl DetailField {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Worktrees => "Worktrees",
            Self::DeletedWorktrees => "Deleted",
            Self::Path => "Path",
            Self::Targets => "Type",
            Self::Disk => "Disk",
            Self::DiskTarget => "  target/",
            Self::DiskNonTarget => "  other",
            Self::DiskOutOfTreeTarget => "  target/ (out of tree)",
            Self::Lint => "Lint",
            Self::Ci => "CI",
            Self::Head => "Branch",
            Self::Bisect => "Bisect",
            Self::Tracks => "Tracks",
            Self::Pinned => "Pinned",
            Self::GitStatus => "Status",
            Self::VsLocal => "Ahead/Behind",
            Self::Stars => "Stars",
            Self::Inception => "Incept",
            Self::LastCommit => "Latest",
            Self::LastFetched => "Fetched",
            Self::RateLimitCore => "Rate limit core",
            Self::RateLimitGraphQl => "Rate limit GraphQL",
            Self::WorktreeError => "Error",
            Self::Version => "Version",
            Self::Edition => "Edition",
            Self::License => "License",
            Self::Homepage => "Homepage",
            Self::Repository => "Repository",
        }
    }

    /// Get the display value for a package field from `PackageData`.
    /// All values are pure-on-data. The Lint and Ci rows are *not*
    /// handled here — the package renderer matches on
    /// `data.lint_display` / `data.ci_display` (typed enums)
    /// directly and frames the icon at render time. Calling this
    /// with `Self::Lint` or `Self::Ci` returns an empty string.
    pub fn package_value(self, data: &PackageData) -> String {
        match self {
            Self::Worktrees => data
                .worktree_group_summary
                .as_ref()
                .map_or_else(String::new, |summary| summary.worktrees.to_string()),
            Self::DeletedWorktrees => data
                .worktree_group_summary
                .as_ref()
                .map_or_else(String::new, |summary| summary.deleted.to_string()),
            Self::Path => data.path.clone(),
            Self::Disk => data.disk.map_or_else(String::new, render::format_bytes),
            Self::Targets => match &data.types {
                None => String::new(),
                Some(types) if types.is_empty() => "-".to_string(),
                Some(types) => types
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", "),
            },
            Self::Version => data.version.clone().unwrap_or_else(|| "-".to_string()),
            Self::DiskTarget => data
                .in_project_target
                .map_or_else(String::new, render::format_bytes),
            Self::DiskNonTarget => data
                .in_project_non_target
                .map_or_else(String::new, render::format_bytes),
            Self::DiskOutOfTreeTarget => data
                .out_of_tree_target_bytes
                .map_or_else(String::new, render::format_bytes),
            Self::Edition => or_dash(data.edition.as_deref()),
            Self::License => or_dash(data.license.as_deref()),
            Self::Homepage => or_dash(data.homepage.as_deref()),
            Self::Repository => or_dash(data.repository.as_deref()),
            Self::WorktreeError => "broken .git — gitdir target missing".to_string(),
            // Git fields, Lint, and Ci — should not be called with
            // package_value. Lint and Ci are rendered directly from
            // their typed-enum fields (`PackageData.lint_display` /
            // `ci_display`) at render time.
            Self::Head
            | Self::Bisect
            | Self::Tracks
            | Self::Pinned
            | Self::GitStatus
            | Self::VsLocal
            | Self::Stars
            | Self::Inception
            | Self::LastCommit
            | Self::LastFetched
            | Self::RateLimitCore
            | Self::RateLimitGraphQl
            | Self::Lint
            | Self::Ci => String::new(),
        }
    }

    /// Get the display value for a git field from `GitData`.
    pub fn git_value(self, data: &GitData) -> String {
        match self {
            Self::Head => match data.head.as_ref() {
                None | Some(HeadState::Unborn) => "unborn".to_string(),
                Some(HeadState::Detached { short_sha }) => format!("detached @ {short_sha}"),
                Some(HeadState::Branch(name)) => data.head_relation.map_or_else(
                    || name.clone(),
                    |relation| format!("{name} · {}", relation.label()),
                ),
            },
            Self::Bisect => data
                .bisect
                .as_ref()
                .map_or_else(String::new, format_bisect_progress),
            Self::GitStatus => data
                .status
                .map_or_else(String::new, GitStatus::label_with_icon),
            Self::VsLocal => data.vs_local.as_deref().unwrap_or("").to_string(),
            Self::Stars => data
                .stars
                .map_or_else(String::new, |count| format!("⭐ {count}")),
            Self::Inception => data.inception.as_deref().unwrap_or("").to_string(),
            Self::LastCommit => data.last_commit.as_deref().unwrap_or("").to_string(),
            Self::LastFetched => data.last_fetched.as_deref().unwrap_or("").to_string(),
            Self::RateLimitCore => format_rate_limit_bucket(data.rate_limit_core),
            Self::RateLimitGraphQl => format_rate_limit_bucket(data.rate_limit_graphql),
            Self::Tracks => data
                .submodule_ctx
                .as_ref()
                .and_then(|context| context.tracks.as_deref())
                .map_or_else(String::new, |t| format!("{t}  (from .gitmodules)")),
            Self::Pinned => data
                .submodule_ctx
                .as_ref()
                .map(|ctx| format!("{}  (parent HEAD)", ctx.pinned_commit))
                .unwrap_or_default(),
            // Package fields — should not be called with git_value.
            Self::Worktrees
            | Self::DeletedWorktrees
            | Self::Path
            | Self::Disk
            | Self::DiskTarget
            | Self::DiskNonTarget
            | Self::DiskOutOfTreeTarget
            | Self::Targets
            | Self::Lint
            | Self::Ci
            | Self::Version
            | Self::Edition
            | Self::License
            | Self::Homepage
            | Self::Repository
            | Self::WorktreeError => String::new(),
        }
    }
}

pub(super) fn strip_ansi(raw: &str) -> String { copy_data::strip_ansi(raw) }

pub(super) fn sanitize_ansi_for_output(raw: &str) -> String {
    copy_data::sanitize_ansi_for_output(raw)
}

pub(super) const fn github_stars_is_unreachable_placeholder(data: &GitData) -> bool {
    package_data::github_stars_is_unreachable_placeholder(data)
}

#[cfg(test)]
pub(super) fn package_fields_from_data(data: &PackageData) -> Vec<DetailField> {
    package_data::package_fields_from_data(data)
}

pub(super) fn package_rows_from_data(data: &PackageData) -> Vec<PackageRow> {
    package_data::package_rows_from_data(data)
}

pub(super) const fn package_row_is_selectable(row: &PackageRow) -> bool {
    package_data::package_row_is_selectable(row)
}

pub(super) fn package_first_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    package_data::package_first_selectable_row(rows)
}

pub(super) fn package_last_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    package_data::package_last_selectable_row(rows)
}

pub(super) fn package_selectable_row_at_or_after(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_data::package_selectable_row_at_or_after(rows, pos)
}

pub(super) fn package_selectable_row_at_or_before(
    rows: &[PackageRow],
    pos: usize,
) -> Option<usize> {
    package_data::package_selectable_row_at_or_before(rows, pos)
}

pub(super) fn package_nearest_selectable_row(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_data::package_nearest_selectable_row(rows, pos)
}

#[cfg(test)]
mod tests {
    use super::RateLimitQuota;
    use super::format_rate_limit_bucket;

    #[test]
    fn rate_limit_bucket_empty_without_quota() {
        assert!(format_rate_limit_bucket(None).is_empty());
    }

    #[test]
    fn rate_limit_bucket_without_reset_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      42,
            remaining: 4958,
            reset_at:  None,
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "4958/5000");
    }

    #[test]
    fn rate_limit_bucket_fully_unused_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      0,
            remaining: 5000,
            reset_at:  Some(u64::MAX),
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "5000/5000");
    }

    #[test]
    fn rate_limit_bucket_with_past_reset_renders_zero_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      100,
            remaining: 4900,
            reset_at:  Some(0),
        };
        assert_eq!(format_rate_limit_bucket(Some(quota)), "4900/5000 resets 0s");
    }
}
