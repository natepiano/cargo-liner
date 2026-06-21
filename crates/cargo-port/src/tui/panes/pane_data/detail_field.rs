use super::copy_data;
use super::formatting;
use super::git_data::GitData;
use super::package_data;
use super::package_data::PackageData;
use super::package_data::PackageRow;
use crate::project::GitStatus;
use crate::project::HeadState;
use crate::tui::render;

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
            Self::Edition => package_data::or_dash(data.edition.as_deref()),
            Self::License => package_data::or_dash(data.license.as_deref()),
            Self::Homepage => package_data::or_dash(data.homepage.as_deref()),
            Self::Repository => package_data::or_dash(data.repository.as_deref()),
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
                .map_or_else(String::new, formatting::format_bisect_progress),
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
            Self::RateLimitCore => formatting::format_rate_limit_bucket(data.rate_limit_core),
            Self::RateLimitGraphQl => formatting::format_rate_limit_bucket(data.rate_limit_graphql),
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

pub fn strip_ansi(raw: &str) -> String { copy_data::strip_ansi(raw) }

pub fn sanitize_ansi_for_output(raw: &str) -> String { copy_data::sanitize_ansi_for_output(raw) }

pub const fn github_stars_is_unreachable_placeholder(data: &GitData) -> bool {
    package_data::github_stars_is_unreachable_placeholder(data)
}

#[cfg(test)]
pub fn package_fields_from_data(data: &PackageData) -> Vec<DetailField> {
    package_data::package_fields_from_data(data)
}

pub fn package_rows_from_data(data: &PackageData) -> Vec<PackageRow> {
    package_data::package_rows_from_data(data)
}

pub const fn package_row_is_selectable(row: &PackageRow) -> bool {
    package_data::package_row_is_selectable(row)
}

pub fn package_first_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    package_data::package_first_selectable_row(rows)
}

pub fn package_last_selectable_row(rows: &[PackageRow]) -> Option<usize> {
    package_data::package_last_selectable_row(rows)
}

pub fn package_selectable_row_at_or_after(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_data::package_selectable_row_at_or_after(rows, pos)
}

pub fn package_selectable_row_at_or_before(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_data::package_selectable_row_at_or_before(rows, pos)
}

pub fn package_nearest_selectable_row(rows: &[PackageRow], pos: usize) -> Option<usize> {
    package_data::package_nearest_selectable_row(rows, pos)
}

#[cfg(test)]
mod tests {
    use crate::tui::panes::pane_data::RateLimitQuota;
    use crate::tui::panes::pane_data::formatting;

    #[test]
    fn rate_limit_bucket_empty_without_quota() {
        assert!(formatting::format_rate_limit_bucket(None).is_empty());
    }

    #[test]
    fn rate_limit_bucket_without_reset_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      42,
            remaining: 4958,
            reset_at:  None,
        };
        assert_eq!(
            formatting::format_rate_limit_bucket(Some(quota)),
            "4958/5000"
        );
    }

    #[test]
    fn rate_limit_bucket_fully_unused_omits_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      0,
            remaining: 5000,
            reset_at:  Some(u64::MAX),
        };
        assert_eq!(
            formatting::format_rate_limit_bucket(Some(quota)),
            "5000/5000"
        );
    }

    #[test]
    fn rate_limit_bucket_with_past_reset_renders_zero_countdown() {
        let quota = RateLimitQuota {
            limit:     5000,
            used:      100,
            remaining: 4900,
            reset_at:  Some(0),
        };
        assert_eq!(
            formatting::format_rate_limit_bucket(Some(quota)),
            "4900/5000 resets 0s"
        );
    }
}
