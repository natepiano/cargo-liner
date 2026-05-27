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

use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;
use std::time::Instant;

use ratatui::Frame;
use ratatui::layout::Position;
use ratatui::layout::Rect;
use tui_pane::Hittable;
use tui_pane::RenderFocus;
use tui_pane::Renderable;
use tui_pane::RunningTracker;
use tui_pane::ToastTaskId;
use tui_pane::TrackedItem;
use tui_pane::Viewport;

use super::Config;
use crate::constants::LINT_NO_LOG;
use crate::lint::CacheUsage;
use crate::lint::LintStatus;
use crate::lint::RuntimeHandle;
use crate::project;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::Visibility;
use crate::tui::columns::LintCell;
use crate::tui::integration;
use crate::tui::pane::HoverTarget;
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
    /// Presentation-only state for the single sticky "N lints running" toast.
    /// The running paths are reconciled from `ProjectList` by
    /// `Self::toast_items_from_project_model`; this tracker must not become a
    /// second source of truth for lint status.
    running:         RunningTracker<AbsolutePath>,
    /// Bytes used by the on-disk lint-log cache (`~/.cache/cargo-port/lints/`).
    /// Refreshed by `App::refresh_lint_cache_usage_from_disk`,
    /// displayed in the Settings popup.
    pub cache_usage: CacheUsage,
    /// Per-pane cursor for the Lints pane.
    pub viewport:    Viewport,
    /// Per-pane focus snapshot stamped before the render loop.
    pub focus:       RenderFocus,
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
            focus: RenderFocus::inactive(),
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

    // ── running toast projection ────────────────────────────────

    pub fn toast_items_from_project_model(
        &mut self,
        projects: &ProjectList,
    ) -> (Option<ToastTaskId>, Vec<TrackedItem>) {
        let running_paths = projects.running_lint_paths();
        let running_set: HashSet<AbsolutePath> = running_paths.iter().cloned().collect();
        self.running
            .running
            .retain(|path, _| running_set.contains(path));
        let now = Instant::now();
        for path in running_paths {
            self.running.running.entry(path).or_insert(now);
        }
        self.running.items_for_toast(
            |p| project::home_relative_path(p.as_path()),
            integration::path_key,
        )
    }

    pub const fn set_running_toast(&mut self, toast: Option<ToastTaskId>) {
        self.running.toast = toast;
    }

    #[cfg(test)]
    pub fn running_toast_is_empty(&self) -> bool { self.running.is_empty() }

    #[cfg(test)]
    pub const fn running_toast_id(&self) -> Option<ToastTaskId> { self.running.toast }

    pub fn running_toast_path_count(&self) -> usize { self.running.running.len() }

    #[cfg(test)]
    pub fn running_toast_contains_path(&self, path: &Path) -> bool {
        self.running.running.contains_key(path)
    }

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
                        .iter_entries()
                        .filter(|entry| entry.visibility() == Visibility::Visible)
                        .map(|entry| Self::run_count_at(projects, entry.path().as_path()))
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
        // A first, in-progress run has no completed history yet
        // (count == 0). Still surface it as `Runs` so the detail pane
        // shows the spinner — matching the project-list column and
        // toast, which key off `status` directly. The renderer omits
        // the `0` count while it's zero.
        if count == 0 && !matches!(status, LintStatus::Running(_)) {
            LintDisplay::NoRuns
        } else {
            LintDisplay::Runs { count, status }
        }
    }
}

/// Resolve a [`LintStatus`] to the [`LintCell`] (icon + style
/// pair) rendered in the Lint column. Free fn so renderers can
/// call it from `Pane::render` with typed refs (no `&App`).
pub fn lint_cell_for(
    status: &LintStatus,
    config: &Config,
    animation_elapsed: Duration,
) -> LintCell {
    if !config.lint_enabled() {
        return LintCell::from_parts(LINT_NO_LOG, ratatui::style::Style::default());
    }
    let icon = integration::lint_icon_for(status.kind()).frame_at(animation_elapsed);
    let style = if matches!(status, LintStatus::Running(_)) {
        ratatui::style::Style::default().fg(tui_pane::accent_color())
    } else {
        ratatui::style::Style::default()
    };
    LintCell::from_parts(icon, style)
}

impl Renderable<PaneRenderCtx<'_>> for Lint {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &PaneRenderCtx<'_>) {
        panes::render_lints_pane_body(frame, area, self, ctx);
    }
}

impl Hittable<HoverTarget> for Lint {
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
    use super::*;

    #[test]
    fn new_starts_with_no_runtime_and_empty_inflight() {
        let lint = Lint::new(None);
        assert!(lint.runtime().is_none());
        assert!(lint.running_toast_is_empty());
        assert!(lint.running_toast_id().is_none());
    }

    #[test]
    fn running_toast_round_trip() {
        let mut lint = Lint::new(None);
        lint.set_running_toast(Some(ToastTaskId(7)));
        assert_eq!(lint.running_toast_id(), Some(tui_pane::ToastTaskId(7)));
        lint.set_running_toast(None);
        assert!(lint.running_toast_id().is_none());
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
