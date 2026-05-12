//! The `Ci` subsystem.
//!
//! Owns the CI field cluster: `fetch_tracker` (paths with
//! in-flight fetches), `fetch_toast` (the fire-once
//! "Retrieving CI runs" toast slot), and `display_modes`
//! (per-project `BranchOnly` vs `All`).
//!
//! `package_display` returns a typed [`CiDisplay`] enum for the
//! Ci row in the Package detail pane. The renderer at
//! `panes/package.rs` matches on enum variants directly, so
//! `PackageData.ci_display` carries `CiDisplay` rather than a
//! pre-rendered string.
//!
//! Pattern: typed display values, not pre-rendered strings (see
//! "Recurring patterns" in `docs/app-api.md`). Mirrors
//! `LintDisplay`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::ToastTaskId;

use super::app::CiRunDisplayMode;
use super::pane::Hittable;
use super::pane::HoverTarget;
use super::pane::Pane;
use super::pane::PaneRenderCtx;
use super::pane::Viewport;
use super::panes;
use super::panes::CiData;
use super::panes::PaneId;
#[cfg(test)]
use crate::ci::CiRun;
use crate::ci::CiStatus;
use crate::project::AbsolutePath;
use crate::project::CheckoutInfo;
use crate::project::ProjectCiInfo;
use crate::project::RepoInfo;

/// Display value for the Ci row in the Package detail pane.
///
/// - `NoWorkflow` — repo has no CI workflows configured. Default for partial / placeholder
///   `PackageData`. Renders as greyed-out "no workflow" text (matching today's `NO_CI_WORKFLOW`
///   styling at `panes/package.rs:121-125`), not vanish — CI rows show for non-Rust projects too,
///   unlike Lint's `NotRust` which excludes the row entirely via `package_fields_from_data`.
/// - `UnpublishedBranch` — branch has no upstream tracking and isn't the repo's default branch; the
///   parent repo's CI doesn't apply to this checkout.
/// - `NoRuns` — workflows present, branch published, but zero local runs and zero `github_total`.
/// - `Runs { ci_status, local, github_total }` — at least one run is known. `ci_status` is the
///   latest run's outcome (renderer applies `CiStatus::icon()` at render time); `local` is the
///   count of runs after display-mode filtering; `github_total` drives the "/ github N" suffix when
///   > 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CiDisplay {
    #[default]
    NoWorkflow,
    UnpublishedBranch,
    NoRuns,
    Runs {
        ci_status:    Option<CiStatus>,
        local:        usize,
        github_total: u32,
    },
}

/// The `Ci` subsystem owns three fields:
///
/// - `fetch_tracker` (`HashSet<AbsolutePath>`-backed [`CiFetchTracker`]) — paths with an in-flight
///   CI fetch. Bespoke type rather than `RunningTracker<K>`: no toast slot, no started-at
///   timestamp.
/// - `fetch_toast` (`Option<ToastTaskId>`) — fire-once toast slot consumed via `take_fetch_toast`
///   at fetch completion. Different lifecycle from `Lint::running` / `Github::running` (which are
///   sticky-during-flight), so it's a plain field rather than wrapped in `RunningTracker`.
/// - `display_modes` (`HashMap<AbsolutePath, CiRunDisplayMode>`) — per-project `BranchOnly` vs
///   `All` selection. Treated as domain state (which CI runs are surfaced for this project), not UI
///   state.
pub struct Ci {
    pub fetch_tracker: CiFetchTracker,
    fetch_toast:       Option<ToastTaskId>,
    display_modes:     HashMap<AbsolutePath, CiRunDisplayMode>,
    /// Per-pane cursor for the CI runs pane.
    pub viewport:      Viewport,
    /// Cached CI table content built per-frame in
    /// `panes::build_ci_data`.
    content:           Option<CiData>,
}

impl Ci {
    pub fn new() -> Self {
        Self {
            fetch_tracker: CiFetchTracker::default(),
            fetch_toast:   None,
            display_modes: HashMap::new(),
            viewport:      Viewport::new(),
            content:       None,
        }
    }

    // ── viewport ────────────────────────────────────────────────

    // ── content ─────────────────────────────────────────────────

    pub const fn content(&self) -> Option<&CiData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: CiData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    #[cfg(test)]
    pub fn override_runs_for_test(&mut self, runs: Vec<CiRun>) {
        if let Some(ci) = self.content.as_mut() {
            ci.runs = runs;
            ci.mode_label = None;
        }
    }

    // ── fetch tracker ───────────────────────────────────────────

    // ── fetch toast ─────────────────────────────────────────────

    pub const fn set_fetch_toast(&mut self, task_id: Option<ToastTaskId>) {
        self.fetch_toast = task_id;
    }

    pub const fn take_fetch_toast(&mut self) -> Option<ToastTaskId> { self.fetch_toast.take() }

    // ── display modes ───────────────────────────────────────────

    pub fn display_mode_for(&self, path: &Path) -> CiRunDisplayMode {
        self.display_modes.get(path).copied().unwrap_or_default()
    }

    pub(super) fn display_mode_label_for(&self, path: &Path) -> &'static str {
        match self.display_mode_for(path) {
            CiRunDisplayMode::BranchOnly => "branch",
            CiRunDisplayMode::All => "all",
        }
    }

    pub fn set_display_mode(&mut self, path: AbsolutePath, mode: CiRunDisplayMode) {
        self.display_modes.insert(path, mode);
    }

    pub fn remove_display_mode(&mut self, path: &Path) { self.display_modes.remove(path); }

    pub fn clear_display_modes(&mut self) { self.display_modes.clear(); }

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
    /// - `latest_ci_status` via `ProjectList::ci_status_for(path, ci)` for single-project rows /
    ///   `RootItem::ci_status(resolver)` for worktree-group rollup rows. The aggregator walks all
    ///   worktree paths and returns `Failed` if any-red, `Passed` if all-green, else `None`. The
    ///   rollup is the only group-level distinction; everything else is primary-checkout data.
    /// - `is_worktree_group` — kept for signature symmetry with `Lint::package_display`. Today's CI
    ///   display logic doesn't branch on it (the caller's pre-resolution of `latest_conclusion`
    ///   already handles the rollup); reserved in case future variants need group-aware text.
    #[allow(
        clippy::too_many_arguments,
        reason = "wide CI dependency surface (Q6 in docs/app-api.md)"
    )]
    pub fn package_display(
        &self,
        abs: &AbsolutePath,
        repo_info: Option<&RepoInfo>,
        git_info: Option<&CheckoutInfo>,
        ci_info: Option<&ProjectCiInfo>,
        latest_conclusion: Option<CiStatus>,
        is_worktree_group: bool,
    ) -> CiDisplay {
        let _ = is_worktree_group;
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
        let display_mode = self.display_mode_for(abs.as_path());
        let local = Self::filtered_run_count(info, git_info, display_mode);
        let github_total = info.github_total;
        if local == 0 && github_total == 0 {
            CiDisplay::NoRuns
        } else {
            CiDisplay::Runs {
                ci_status: latest_conclusion,
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
    /// `ProjectList::ci_runs_for_ci_pane`.
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

/// Runtime-only CI fetch tracking. Persistent CI data lives on the project
/// hierarchy; this only records which owner paths currently have a request
/// in flight.
#[derive(Default)]
pub(super) struct CiFetchTracker {
    inner: HashSet<AbsolutePath>,
}

impl CiFetchTracker {
    pub(super) fn start(&mut self, path: AbsolutePath) { self.inner.insert(path); }

    pub(super) fn complete(&mut self, path: &Path) -> bool { self.inner.remove(path) }

    pub(super) fn is_fetching(&self, path: &Path) -> bool { self.inner.contains(path) }

    pub(super) fn clear(&mut self) { self.inner.clear(); }

    pub(super) fn retain(&mut self, mut keep: impl FnMut(&AbsolutePath) -> bool) {
        self.inner.retain(|path| keep(path));
    }
}

impl Pane for Ci {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        panes::render_ci_pane_body(frame, area, self, ctx);
    }
}

impl Hittable for Ci {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = panes::hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::CiRuns,
            row,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_workflow_when_repo_info_missing() {
        let ci = Ci::new();
        let abs = AbsolutePath::from(std::path::Path::new("/abs/x"));
        let display = ci.package_display(&abs, None, None, None, None, false);
        assert_eq!(display, CiDisplay::NoWorkflow);
    }

    #[test]
    fn default_is_no_workflow() {
        assert_eq!(CiDisplay::default(), CiDisplay::NoWorkflow);
    }
}
