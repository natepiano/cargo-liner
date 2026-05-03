//! The `Lint` subsystem.
//!
//! Phase 11 of the App-API extraction (see `docs/app-api.md`). Phase
//! 11.2 introduces the type and the read-side lookup API; the field
//! cluster (runtime, running paths, toasts, cache usage, phase
//! tracking) moves in Phase 11.4. Until then `Lint` is a marker
//! struct holding only function definitions.
//!
//! The four lookup functions (`status_for_path`, `status_for_root`,
//! `status_for_worktree`, `run_count_at`) replace the four icon
//! resolvers that previously sat on `App` (`lint_icon`,
//! `lint_icon_for_root`, `lint_icon_for_worktree`,
//! `selected_lint_icon`). They return unframed `LintStatus` —
//! callers apply `animation_elapsed` to `status.icon()` at render
//! time. Animation framing is no longer threaded through the lookup
//! API.
//!
//! `package_display` returns a typed [`LintDisplay`] enum for the
//! Lint row in the Package detail pane. Phase 11.3 flips
//! `PackageData.lint_display` from `String` to `LintDisplay` and
//! deletes the `resolve_lint_display` / `lint_run_count_for`
//! stringifiers in `panes/support.rs`.

use std::path::Path;

use crate::lint::CacheUsage;
use crate::lint::LintStatus;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project_list::ProjectList;
use super::app::CountedPhase;
use super::app::KeyedPhase;
use super::running_tracker::RunningTracker;

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

/// The `Lint` subsystem.
///
/// Phase 11.4a absorbed the in-flight lint state from
/// [`Inflight`](super::inflight::Inflight): the lint runtime
/// handle, the running-paths map, and the running-toast slot.
/// Subsequent slices (11.4b, 11.4c) absorb the disk cache stat
/// counter from `Scan` and the lint-specific phase trackers from
/// `ScanState::startup_phases`.
pub struct Lint {
    /// Tokio runtime handle that runs cargo lint commands. Spawned
    /// at startup; replaced by [`Self::set_runtime`] when lint
    /// config (`lint.enabled`, `lint.parallel`, `lint.cache_root`)
    /// changes. `None` when lint is disabled.
    runtime:           Option<RuntimeHandle>,
    /// Paths with a lint run currently in flight, keyed by the
    /// time the run was launched, paired with the single sticky
    /// "N lints running" toast slot. Synced each tick by
    /// `App::sync_running_lint_toast`. Phase 12.1 swapped the prior
    /// `running_paths` + `running_toast` field pair for the generic
    /// [`RunningTracker`] primitive.
    running:           RunningTracker<AbsolutePath>,
    /// Bytes used by the on-disk lint-log cache (`~/.cache/cargo-port/lints/`).
    /// Refreshed by `App::refresh_lint_cache_usage_from_disk`,
    /// displayed in the Settings popup. Phase 11.4b moved this
    /// off [`Scan`](super::scan_state::Scan).
    cache_usage:       CacheUsage,
    /// Tracks terminal lint events (`Passed` / `Failed`) keyed
    /// on project path; `seen` counts only terminal arrivals.
    /// Phase 11.4c moved this off `ScanState::startup_phases`.
    pub phase:         KeyedPhase<AbsolutePath>,
    /// Counts startup-time lint completions across the project
    /// tree. Used by `App::maybe_complete_startup_lints` to decide
    /// when the startup-lint pass is done. Phase 11.4c moved this
    /// off `ScanState::startup_phases`.
    pub startup_phase: CountedPhase,
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
            phase: KeyedPhase::default(),
            startup_phase: CountedPhase::default(),
        }
    }

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

    pub const fn cache_usage(&self) -> &CacheUsage { &self.cache_usage }

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
    /// at the selected project. Phase 11.3 wires this in as the
    /// replacement for `resolve_lint_display`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::toasts::ToastTaskId;

    #[test]
    fn new_starts_with_no_runtime_and_empty_inflight() {
        let lint = Lint::new(None);
        assert!(lint.runtime().is_none());
        assert!(lint.running().is_empty());
        assert!(lint.running().toast().is_none());
    }

    #[test]
    fn running_toast_round_trip() {
        let mut lint = Lint::new(None);
        lint.running_mut()
            .set_toast(Some(ToastTaskId(7)));
        assert_eq!(
            lint.running().toast(),
            Some(crate::tui::toasts::ToastTaskId(7))
        );
        lint.running_mut().set_toast(None);
        assert!(lint.running().toast().is_none());
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
