//! The `Ci` subsystem.
//!
//! Phase 13 of the App-API extraction (see `docs/app-api.md`).
//! Phase 13.1 introduces the type and the read-side display API;
//! the field cluster (`ci_fetch_tracker`, `ci_fetch_toast`,
//! `display_modes`) moves in Phase 13.2. Until then `Ci` is a
//! marker struct with no fields, parallel to Phase 11.2's
//! pre-field-move `Lint`.
//!
//! `package_display` returns a typed [`CiDisplay`] enum for the
//! Ci row in the Package detail pane. Phase 13.3 (capstone) flips
//! `PackageData.ci_display` from `String` to `CiDisplay`,
//! updates `panes/package.rs` to match on enum variants instead
//! of string-comparing the `NO_CI_*` constants, and deletes
//! `resolve_ci_display` from `panes/support.rs`.
//!
//! Pattern: typed display values, not pre-rendered strings (see
//! "Recurring patterns" in `docs/app-api.md`). Mirrors
//! `LintDisplay` (Phase 11.3).

use crate::ci::Conclusion;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::ProjectCiInfo;
use crate::project::RepoInfo;
use crate::tui::app::CiRunDisplayMode;

/// Display value for the Ci row in the Package detail pane.
///
/// - `NoWorkflow` — repo has no CI workflows configured. Default for partial / placeholder
///   `PackageData`. Renders as greyed-out "no workflow" text (matching today's `NO_CI_WORKFLOW`
///   styling at `panes/package.rs:121-125`), not vanish — CI rows show for non-Rust projects too,
///   unlike Lint's `NotRust` which excludes the row entirely via `package_fields_from_data`.
/// - `UnpublishedBranch` — branch has no upstream tracking and isn't the repo's default branch; the
///   parent repo's CI doesn't apply to this checkout.
/// - `NoRuns` — workflows present, branch published, but zero local runs and zero `github_total`.
/// - `Runs { conclusion, local, github_total }` — at least one run is known. `conclusion` is the
///   latest run's outcome (renderer applies `Conclusion::icon()` at render time); `local` is the
///   count of runs after display-mode filtering; `github_total` drives the "/ github N" suffix when
///   > 0.
#[allow(
    dead_code,
    reason = "wired in Phase 13.3 capstone; ships in 13.1 alongside the old string API"
)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CiDisplay {
    #[default]
    NoWorkflow,
    UnpublishedBranch,
    NoRuns,
    Runs {
        conclusion:   Option<Conclusion>,
        local:        usize,
        github_total: u32,
    },
}

/// The `Ci` subsystem.
///
/// Phase 13.1 marker struct: holds no fields. Phase 13.2 absorbs
/// the field cluster (`ci_fetch_tracker`, `ci_fetch_toast`,
/// `display_modes`) from `Inflight` and `CiPane`.
pub struct Ci;

impl Ci {
    pub const fn new() -> Self { Self }

    /// Build the [`CiDisplay`] for the Ci row in the Package
    /// detail pane at the selected project (or worktree-group
    /// row).
    ///
    /// Inputs are pre-resolved by the caller
    /// (`panes/support.rs:build_pane_data_common`) so this
    /// function stays pure over its parameters and does not
    /// reach into the project tree itself:
    ///
    /// - `repo_info` via `App::repo_info_for(path)` — workflow presence is repo-level, identical
    ///   for every checkout in a worktree group.
    /// - `git_info` via `App::git_info_for(path)` — primary checkout's branch / upstream, used for
    ///   the unpublished-branch detection (parallel to `App::unpublished_ci_branch_name`).
    /// - `ci_info` via `App::ci_info_for(path)` — primary checkout's local CI runs and
    ///   `github_total`. Run counts are NOT aggregated across worktree-group checkouts (matches
    ///   today's `resolve_ci_display` behavior, which reads `ci_data_for(abs_path)` for the primary
    ///   only).
    /// - `latest_conclusion` via `App::ci_for(path)` for single-project rows /
    ///   `App::ci_for_item(item)` for worktree-group rollup rows. The aggregator at
    ///   `app/query.rs:424-452` walks all worktree paths and returns `Failure` if any-red,
    ///   `Success` if all-green, else `None`. The rollup is the only group-level distinction;
    ///   everything else is primary-checkout data.
    /// - `display_mode` — temporary parameter shipped in 13.1 while `display_modes` still lives on
    ///   `CiPane`. Phase 13.2 drops this parameter and reads `self.display_mode_for(abs)` from the
    ///   absorbed field on `Ci`.
    /// - `is_worktree_group` — kept for signature symmetry with `Lint::package_display`. Today's CI
    ///   display logic doesn't branch on it (the caller's pre-resolution of `latest_conclusion`
    ///   already handles the rollup); reserved in case future variants need group-aware text.
    #[allow(
        dead_code,
        reason = "wired in Phase 13.3 capstone; ships in 13.1 alongside the old string API"
    )]
    #[allow(
        clippy::unused_self,
        reason = "Phase 13.2 reads display_modes from self; the parameter goes away then"
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "wide CI dependency surface (Q6 in docs/app-api.md); display_mode arg drops in 13.2 to 7 — the absolute limit"
    )]
    pub fn package_display(
        &self,
        abs: &AbsolutePath,
        repo_info: Option<&RepoInfo>,
        git_info: Option<&CheckoutInfo>,
        ci_info: Option<&ProjectCiInfo>,
        latest_conclusion: Option<Conclusion>,
        display_mode: CiRunDisplayMode,
        is_worktree_group: bool,
    ) -> CiDisplay {
        let _ = (abs, is_worktree_group);
        let has_workflows = repo_info.is_some_and(|r| r.workflows.is_present());
        if !has_workflows {
            return CiDisplay::NoWorkflow;
        }
        if Self::is_unpublished_branch(git_info, repo_info) {
            return CiDisplay::UnpublishedBranch;
        }
        let Some(info) = ci_info else {
            return CiDisplay::NoRuns;
        };
        let local = Self::filtered_run_count(info, git_info, display_mode);
        let github_total = info.github_total;
        if local == 0 && github_total == 0 {
            CiDisplay::NoRuns
        } else {
            CiDisplay::Runs {
                conclusion: latest_conclusion,
                local,
                github_total,
            }
        }
    }

    /// True when the checkout's branch has no upstream tracking
    /// and is not the repo's default branch — i.e. the parent
    /// repo's CI doesn't apply to this checkout. Mirrors
    /// `App::unpublished_ci_branch_name` returning `Some`.
    fn is_unpublished_branch(
        git_info: Option<&CheckoutInfo>,
        repo_info: Option<&RepoInfo>,
    ) -> bool {
        let Some(git) = git_info else {
            return false;
        };
        let default_branch = repo_info.and_then(|r| r.default_branch.as_deref());
        git.primary_tracked_ref().is_none() && git.branch.as_deref() != default_branch
    }

    /// Count `info.runs` after applying the display-mode
    /// filter. `BranchOnly` keeps only runs matching the
    /// current branch (when available); `All` keeps every
    /// run. Mirrors the filtering in
    /// `App::ci_runs_for_display_inner`.
    fn filtered_run_count(
        info: &ProjectCiInfo,
        git_info: Option<&CheckoutInfo>,
        display_mode: CiRunDisplayMode,
    ) -> usize {
        let Some(branch) = git_info.and_then(|g| g.branch.as_deref()) else {
            return info.runs.len();
        };
        if matches!(display_mode, CiRunDisplayMode::All) {
            return info.runs.len();
        }
        info.runs.iter().filter(|run| run.branch == branch).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_workflow_when_repo_info_missing() {
        let ci = Ci::new();
        let abs = AbsolutePath::from(std::path::Path::new("/abs/x"));
        let display = ci.package_display(
            &abs,
            None,
            None,
            None,
            None,
            CiRunDisplayMode::default(),
            false,
        );
        assert_eq!(display, CiDisplay::NoWorkflow);
    }

    #[test]
    fn default_is_no_workflow() {
        assert_eq!(CiDisplay::default(), CiDisplay::NoWorkflow);
    }
}
