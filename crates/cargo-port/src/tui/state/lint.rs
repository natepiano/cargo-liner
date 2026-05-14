//! The `Lint` subsystem.
//!
//! Owns the lint runtime, in-flight paths, the running-toast slot,
//! and the on-disk cache stat counter. Startup-pass trackers live on
//! the [`Startup`] subsystem.
//!
//! The four lookup functions (`status_for_path`, `status_for_root`,
//! `status_for_worktree`, `run_count_at`) return unframed
//! `LintStatus` — callers apply `animation_elapsed` to
//! `status.icon()` at render time.
//!
//! `package_display` returns a typed [`LintDisplay`] enum for the
//! Lint row in the Package detail pane. The renderer matches on
//! variants directly, so `PackageData.lint_display` carries
//! `LintDisplay` rather than a pre-rendered string.

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::RunningTracker;
use tui_pane::Viewport;

use crate::lint::CacheUsage;
use crate::lint::LintStatus;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::tui::pane::Hittable;
use crate::tui::pane::HoverTarget;
use crate::tui::pane::Pane;
use crate::tui::pane::PaneRenderCtx;
use crate::tui::panes;
use crate::tui::panes::LintsData;
use crate::tui::panes::PaneId;
use crate::tui::project_list::ProjectList;

/// Display value for the Lint row in the Package detail pane.
///
/// Pattern: typed display values, not pre-rendered strings (see
/// "Recurring patterns" in `docs/app-api.md`). The Package
/// renderer matches on variants and applies `animation_elapsed`
/// to `status.icon()` at render time.
///
/// - `NotRust` — selected project is not a Rust project; no cargo lint applies.
/// - `NoRuns` — Rust project, but no lint history (zero runs).
/// - `Runs { count, status }` — Rust project with at least one lint run. `status` is unframed; the
///   renderer frames the icon at render time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum LintDisplay {
    /// Default for partial / placeholder `PackageData` (e.g.,
    /// submodules and other non-Rust contexts where the Lint row
    /// is excluded by `package_fields_from_data` anyway).
    #[default]
    NotRust,
    NoRuns,
    Runs {
        count:  usize,
        status: LintStatus,
    },
}

/// The `Lint` subsystem. Owns the lint runtime, in-flight
/// paths, running-toast slot, and the disk cache stat counter.
pub struct Lint {
    /// Tokio runtime handle that runs cargo lint commands. Spawned
    /// at startup; replaced by [`Self::set_runtime`] when lint
    /// config (`lint.enabled`, `lint.parallel`, `lint.cache_root`)
    /// changes. `None` when lint is disabled.
    runtime:         Option<RuntimeHandle>,
    /// Paths with a lint run currently in flight, keyed by the
    /// time the run was launched, paired with the single sticky
    /// "N lints running" toast slot. Synced each tick by
    /// `App::sync_running_lint_toast`.
    running:         RunningTracker<AbsolutePath>,
    /// Bytes used by the on-disk lint-log cache (`~/.cache/cargo-port/lints/`).
    /// Refreshed by `App::refresh_lint_cache_usage_from_disk`,
    /// displayed in the Settings popup.
    pub cache_usage: CacheUsage,
    /// Per-pane cursor for the Lints pane.
    pub viewport:    Viewport,
    /// Cached Lints table content built per-frame in
    /// `panes::build_lints_data`.
    content:         Option<LintsData>,
}

impl Lint {
    /// Construct a fresh `Lint` carrying the runtime handle. The
    /// handle is initialized once at app startup; subsequent
    /// config-driven respawns flow through [`Self::set_runtime`].
    pub fn new(runtime: Option<RuntimeHandle>) -> Self {
        Self {
            runtime,
            running: RunningTracker::new(),
            cache_usage: CacheUsage::default(),
            viewport: Viewport::new(),
            content: None,
        }
    }

    // ── viewport ────────────────────────────────────────────────

    // ── content ─────────────────────────────────────────────────

    pub const fn content(&self) -> Option<&LintsData> { self.content.as_ref() }

    pub fn set_content(&mut self, data: LintsData) { self.content = Some(data); }

    pub fn clear_content(&mut self) { self.content = None; }

    // ── runtime ─────────────────────────────────────────────────

    /// The lint runtime handle, if lint is enabled.
    pub const fn runtime(&self) -> Option<&RuntimeHandle> { self.runtime.as_ref() }

    /// Clone the runtime handle. Used by spawn paths that want an
    /// owned handle (e.g., [`crate::tui::app::App::reload_lint_history`]).
    pub fn runtime_clone(&self) -> Option<RuntimeHandle> { self.runtime.clone() }

    /// Replace the runtime handle. Called by the config-reload
    /// path when lint settings change.
    pub fn set_runtime(&mut self, handle: Option<RuntimeHandle>) { self.runtime = handle; }

    // ── running tracker ─────────────────────────────────────────

    pub const fn running(&self) -> &RunningTracker<AbsolutePath> { &self.running }

    pub const fn running_mut(&mut self) -> &mut RunningTracker<AbsolutePath> { &mut self.running }

    // ── cache usage ─────────────────────────────────────────────

    pub const fn set_cache_usage(&mut self, usage: CacheUsage) { self.cache_usage = usage; }

    // ── read-side lookups ───────────────────────────────────────

    /// Lint status of the project at `path`. Returns
    /// [`LintStatus::NoLog`] when the path has no lint history.
    pub fn status_for_path(projects: &ProjectList, path: &Path) -> LintStatus {
        projects
            .lint_at_path(path)
            .map_or(LintStatus::NoLog, |lr| lr.status().clone())
    }

    /// Lint status of a `RootItem` (single project or worktree
    /// group), aggregated across the group's checkouts when
    /// applicable. Delegates to [`RootItem::lint_rollup_status`].
    pub fn status_for_root(item: &RootItem) -> LintStatus { item.lint_rollup_status() }

    /// Lint status of a single worktree entry within a worktree
    /// group; `worktree_index` 0 is the primary checkout.
    pub fn status_for_worktree(item: &RootItem, worktree_index: usize) -> LintStatus {
        match item {
            RootItem::Worktrees(group) => group.lint_status_for_worktree(worktree_index),
            _ => LintStatus::NoLog,
        }
    }

    /// Run count at `path`, or 0 when no lint history exists.
    pub fn run_count_at(projects: &ProjectList, path: &Path) -> usize {
        projects.lint_at_path(path).map_or(0, |lr| lr.runs().len())
    }

    /// Build the [`LintDisplay`] for the Package detail pane row
    /// at the selected project.
    ///
    /// `is_worktree_group` is true when the selected row's
    /// `package_title` is "Worktree Group" — i.e., the detail
    /// pane is showing a worktree-group rollup. In that case the
    /// status aggregates across the group's checkouts and the run
    /// count sums across them. Otherwise the lookup is per-path.
    pub fn package_display(
        projects: &ProjectList,
        abs: &AbsolutePath,
        is_worktree_group: bool,
        is_rust: bool,
    ) -> LintDisplay {
        if !is_rust {
            return LintDisplay::NotRust;
        }
        let path = abs.as_path();
        let (status, count) = if is_worktree_group {
            let group_item = projects.iter().find(|entry| {
                entry.item.path() == abs && matches!(&entry.item, RootItem::Worktrees(_))
            });
            match group_item.map(|entry| &entry.item) {
                Some(item @ RootItem::Worktrees(group)) => {
                    let status = Self::status_for_root(item);
                    let count: usize = group
                        .iter_paths()
                        .map(|p| Self::run_count_at(projects, p.as_path()))
                        .sum();
                    (status, count)
                },
                _ => (
                    Self::status_for_path(projects, path),
                    Self::run_count_at(projects, path),
                ),
            }
        } else {
            (
                Self::status_for_path(projects, path),
                Self::run_count_at(projects, path),
            )
        };
        if count == 0 {
            LintDisplay::NoRuns
        } else {
            LintDisplay::Runs { count, status }
        }
    }
}

impl Pane for Lint {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        panes::render_lints_pane_body(frame, area, self, ctx);
    }
}

impl Hittable for Lint {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
        let row = panes::hit_test_table_row(&self.viewport, pos)?;
        Some(HoverTarget::PaneRow {
            pane: PaneId::Lints,
            row,
        })
    }
}

#[cfg(test)]
mod tests {
    use tui_pane::ToastTaskId;

    use super::*;

    #[test]
    fn new_starts_with_no_runtime_and_empty_inflight() {
        let lint = Lint::new(None);
        assert!(lint.runtime().is_none());
        assert!(lint.running().is_empty());
        assert!(lint.running().toast.is_none());
    }

    #[test]
    fn running_toast_round_trip() {
        let mut lint = Lint::new(None);
        lint.running_mut().toast = Some(ToastTaskId(7));
        assert_eq!(lint.running().toast, Some(tui_pane::ToastTaskId(7)));
        lint.running_mut().toast = None;
        assert!(lint.running().toast.is_none());
    }

    #[test]
    fn package_display_returns_not_rust_when_is_rust_false() {
        let projects = ProjectList::default();
        let abs = AbsolutePath::from(Path::new("/abs/x"));
        assert_eq!(
            Lint::package_display(&projects, &abs, false, false),
            LintDisplay::NotRust,
        );
    }

    #[test]
    fn package_display_returns_no_runs_when_rust_with_zero_runs() {
        let projects = ProjectList::default();
        let abs = AbsolutePath::from(Path::new("/abs/x"));
        assert_eq!(
            Lint::package_display(&projects, &abs, false, true),
            LintDisplay::NoRuns,
        );
    }

    #[test]
    fn run_count_at_returns_zero_for_unknown_path() {
        let projects = ProjectList::default();
        assert_eq!(Lint::run_count_at(&projects, Path::new("/abs/missing")), 0);
    }

    #[test]
    fn status_for_path_returns_no_log_for_unknown_path() {
        let projects = ProjectList::default();
        assert_eq!(
            Lint::status_for_path(&projects, Path::new("/abs/missing")),
            LintStatus::NoLog,
        );
    }
}
