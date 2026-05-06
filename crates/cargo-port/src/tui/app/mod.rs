//! # Recurring patterns
//!
//! `App` and the subsystems it owns follow a few patterns that recur
//! across the codebase. New code that fits one of these patterns MUST
//! follow the named pattern, not invent a variant. The
//! `docs/app-api.md` plan is the design source of truth; this index
//! is the in-code map a maintainer hits when reading the code.
//!
//! ## Mutation guard (RAII)
//! Gate mutating methods through a temporary handle whose `Drop` runs
//! the recompute that derived caches need. The only way to call the
//! mutating methods is via the handle; the only way to drop the handle
//! is to let the recompute fire. Type-enforced; no convention to
//! remember.
//!
//! - **Fan-out flavor** — see [`TreeMutation`] (this module). The guard borrows `&mut ProjectList +
//!   &mut Panes` directly so its `Drop` can fan out across both subsystems with the dependency
//!   declared at the type level. On drop it clears [`super::panes::Panes::clear_for_tree_change`]
//!   and rebuilds [`crate::tui::project_list::ProjectList::recompute_visibility`].
//!   `App::mutate_tree` constructs the guard via destructuring so the two subsystem borrows are
//!   disjoint.
//! - **Self-only flavor** — see [`crate::tui::project_list::SelectionMutation`].
//!   Visibility-changing mutations on `ProjectList` (`toggle_expand`, `apply_finder`) are only
//!   callable through the guard; `Drop` recomputes `cached_visible_rows`.
//!
//! ## Cross-subsystem orchestrator on App
//! Operations that touch multiple subsystems and have no single
//! subsystem where they naturally live stay as named methods on `App`.
//! Their doc comments name every subsystem they touch and instruct
//! future maintainers that new side-effects of the same event MUST be
//! added here, not scattered.
//!
//! - See [`App::apply_lint_config_change`]. Touches Inflight (respawn lint runtime, clear in-flight
//!   paths, sync toast), Scan (clear lint state, refresh from disk, bump `data_generation`), and
//!   `ProjectList` (recompute fit widths). New side-effects of a lint-config change MUST be added
//!   there.
//!
//! ## Generic primitive plus bespoke state
//! When two subsystems need the same lifecycle but carry different
//! bespoke state, write the lifecycle as a generic struct and have
//! each subsystem compose it.
//!
//! - See [`super::watched_file::WatchedFile<T>`], composed by [`super::config_state::Config`] (with
//!   the `SettingsEditBuffer` edit buffer) and [`super::keymap_state::Keymap`] (with the
//!   diagnostics-toast id). The primitive captures the load-on-disk-change contract once; the two
//!   subsystems add their bespoke state on top.

mod async_tasks;
mod ci;
mod construct;
mod dismiss;
mod navigation;
mod phase_state;
mod query;
mod startup;

pub(super) use phase_state::CountedPhase;
pub(super) use phase_state::KeyedPhase;
mod target_index;
mod types;

use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use ratatui::layout::Position;
use strum::IntoEnumIterator;

use super::background::Background;
use super::columns;
use super::columns::LintCell;
use super::columns::StyledSegment;
use super::config_state::Config;
use super::focus::Focus;
use super::inflight::Inflight;
use super::keymap_state::Keymap;
use super::overlays::Overlays;
use super::panes::LayoutCache;
use super::panes::PaneId;
use super::panes::Panes;
use super::project_list::ProjectList;
use super::scan_state::Scan;
use super::toasts::ToastStyle::Warning;
use super::toasts::ToastView;
use super::toasts::TrackedItem;
use crate::ci::CiRun;
use crate::ci::Conclusion;
use crate::ci::OwnerRepo;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::lint::LintRuns;
use crate::lint::LintStatus;
use crate::project::AbsolutePath;
use crate::project::GitStatus;
use crate::project::Package;
use crate::project::ProjectFields;
use crate::project::RustProject;
use crate::project::Workspace;
use crate::project::WorkspaceMetadataStore;
use crate::project::WorktreeGroup;
use crate::scan;
use crate::scan::BackgroundMsg;

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
#[allow(clippy::panic, reason = "tests should panic on unexpected values")]
#[allow(
    clippy::unreachable,
    reason = "tests should panic on unexpected values"
)]
mod tests;

use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;

use async_tasks::Startup;
pub(super) use dismiss::DismissTarget;
pub(super) use target_index::CleanSelection;
pub(super) use target_index::TargetDirIndex;
pub(super) use types::CiRunDisplayMode;
pub(super) use types::ConfirmAction;
pub(super) use types::DirtyState;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::DiscoveryShimmer;
pub(super) use types::FinderState;
pub(super) use types::HoveredPaneRow;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
#[cfg(test)]
pub(super) use types::RetrySpawnMode;
pub(super) use types::ScanState;
pub(super) use types::SelectionPaths;
pub(super) use types::SelectionSync;

use super::ci_state::Ci;
pub(super) use super::columns::ProjectListWidths;
use super::interaction;
use super::lint_state::Lint;
pub(super) use super::net_state::AvailabilityStatus;
use super::net_state::Net;
use super::panes;
use super::panes::BottomRow;
pub(super) use super::project_list::ExpandKey;
pub(super) use super::project_list::VisibleRow;
use super::settings::SettingOption;
use super::shortcuts::InputContext;
use super::toasts::ToastManager;
use super::toasts::ToastTaskId;
use crate::project;
use crate::project::RootItem;
use crate::scan::MetadataDispatchContext;
pub(super) struct App {
    /// Net subsystem. Owns the shared `HttpClient`, the GitHub
    /// sub-state (availability, repo-fetch cache, in-flight set,
    /// running tracker), and the crates.io sub-state
    /// (availability). App orchestration that touches Net plus
    /// other subsystems (toast push/dismiss, retry spawn) stays
    /// as named methods on `App`.
    pub(super) net:          Net,
    /// Panes subsystem. Owns `pane_manager`, `pane_data`,
    /// `hovered_pane_row`, and `cpu_poller`. App's
    /// impl-files reach pane state through this handle.
    pub(super) panes:        Panes,
    /// Background subsystem. Owns the four mpsc channel pairs plus
    /// `watch_tx`. The `bg_*` pair is replaced wholesale on every
    /// rescan via [`Background::swap_bg_channel`]; the others outlive
    /// any single rescan.
    pub(super) background:   Background,
    /// Inflight subsystem. Owns the running-paths maps, toast
    /// slots, pending queues, and example-runner state.
    pub(super) inflight:     Inflight,
    /// Lint subsystem. Owns the lint runtime, in-flight lint
    /// state, the disk cache stat counter, and the startup-pass
    /// trackers.
    pub(super) lint:         Lint,
    /// Ci subsystem. Owns `fetch_tracker`, `fetch_toast`, and
    /// per-project `display_modes`, plus `Ci::package_display`
    /// which returns the typed [`CiDisplay`] for the package
    /// detail row.
    pub(super) ci:           Ci,
    /// Config subsystem. Owns `current_config`, `config_path`,
    /// `config_last_seen`, plus the in-app settings editor's
    /// `SettingsEditBuffer`. Composes
    /// `WatchedFile<CargoPortConfig>`.
    pub(super) config:       Config,
    /// Keymap subsystem. Owns `current_keymap`, `keymap_path`,
    /// `keymap_last_seen`, `keymap_diagnostics_id`. Composes
    /// `WatchedFile<ResolvedKeymap>`.
    pub(super) keymap:       Keymap,
    /// The central per-project data store. Lint runs, CI info, git
    /// info, language stats, package/workspace fields, and disk usage
    /// all live inside the tree. Every subsystem that produces
    /// per-project data writes into it.
    pub(super) project_list: ProjectList,
    /// Scan subsystem. Owns `scan` (`ScanState`),
    /// `dirty`, `data_generation`, `discovery_shimmers`,
    /// `pending_git_first_commit`, `metadata_store`,
    /// `target_dir_index`, `priority_fetch_path`,
    /// `confirm_verifying`, `lint_cache_usage`, and (test-only)
    /// `retry_spawn_mode`.
    pub(super) scan:         Scan,
    /// Startup-phase orchestrator. Owns the per-phase trackers
    /// (`disk`, `git`, `repo`, `metadata`, `lint_phase`,
    /// `lint_count`) plus the `scan_complete_at`, `toast`, and
    /// `complete_at` slots that drive the umbrella "Startup" toast
    /// and its detail toasts.
    pub(super) startup:      Startup,
    pub(super) focus:        Focus,
    /// Overlays subsystem. Owns the four overlay-mode enums
    /// (`FinderMode`, `SettingsMode`, `KeymapMode`, `ExitMode`),
    /// the transient `inline_error` UI feedback, and the
    /// `status_flash` slot.
    pub(super) overlays:     Overlays,
    confirm:                 Option<ConfirmAction>,
    animation_started:       Instant,
    mouse_pos:               Option<Position>,
    pub(super) toasts:       ToastManager,
    /// Layout coordination cache. Computed once per draw and shared
    /// across the render path: tile layout, project-list body rect,
    /// and the row-hitbox map for click/hover dispatch. Lives on
    /// App-shell because it's coordination state, not pane state —
    /// it describes what rect each pane occupies.
    pub(super) layout_cache: LayoutCache,
}

impl App {
    /// Constructor entry — declared here in `mod.rs` so `pub(super)`
    /// reaches `tui` (its parent module), satisfying callers in
    /// sibling modules `tui::terminal` and `tui::interaction`. The
    /// real construction logic is the `AppBuilder<S>` typestate
    /// pipeline in `construct.rs`; this shim drives the chain
    /// end-to-end and is visibility-anchored to `tui::app::mod`.
    pub(super) fn new(
        projects: &[RootItem],
        bg_tx: Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        construct::AppBuilder::new(
            projects,
            bg_tx,
            bg_rx,
            cfg,
            http_client,
            scan_started_at,
            metadata_store,
        )
        .open_channels()
        .run_startup()
        .build()
    }

    /// Whether the currently selected row is a lint-owning node.
    /// Only roots and worktree entries own lint state. Members,
    /// vendored packages, and group headers do not — the match is
    /// exhaustive so new variants must be classified.
    ///
    /// Declared in `mod.rs` (not `lint.rs`) so `pub(super)` reaches
    /// `tui` and satisfies the caller in `tui/panes/actions.rs`.
    pub(super) fn selected_row_owns_lint(&self) -> bool {
        match self.selected_row() {
            Some(
                VisibleRow::Root { .. }
                | VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. },
            ) => true,
            Some(
                VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::Submodule { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeVendored { .. },
            )
            | None => false,
        }
    }

    /// Resolve a [`LintStatus`] to the [`LintCell`] (icon + style
    /// pair) rendered in the Lint column. Single source of truth:
    /// the icon and style cannot drift because both derive from
    /// the same status here. Returns the `NoLog` cell when lint
    /// is disabled.
    ///
    /// Declared in `mod.rs` (not `lint.rs`) so `pub(super)` reaches
    /// `tui` and satisfies callers in `tui/panes/project_list.rs`.
    pub(super) fn lint_cell(&self, status: &LintStatus) -> LintCell {
        if !self.config.lint_enabled() {
            return LintCell::from_parts(
                crate::constants::LINT_NO_LOG,
                ratatui::style::Style::default(),
            );
        }
        let icon = status.icon().frame_at(self.animation_elapsed());
        let style = if matches!(status, LintStatus::Running(_)) {
            ratatui::style::Style::default().fg(crate::tui::constants::ACCENT_COLOR)
        } else {
            ratatui::style::Style::default()
        };
        LintCell::from_parts(icon, style)
    }

    pub(super) fn resolved_dirs(&self) -> Vec<AbsolutePath> {
        scan::resolve_include_dirs(&self.config.current().tui.include_dirs)
    }

    fn toast_timeout(&self) -> Duration {
        Duration::from_secs_f64(self.config.current().tui.status_flash_secs)
    }

    pub(super) fn focused_toast_id(&self) -> Option<u64> {
        let active = self.toasts.active_now();
        active.get(self.toasts.viewport().pos()).map(ToastView::id)
    }

    pub(super) fn prune_toasts(&mut self) {
        let now = Instant::now();
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.prune_tracked_items(now, linger);
        self.toasts.prune(now);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
        if self.focus.base() == PaneId::Toasts && self.toasts.active_now().is_empty() {
            self.focus.set(PaneId::ProjectList);
        }
    }

    pub(super) fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.toasts.push_timed(title, body, self.toast_timeout(), 1);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
    }

    pub(super) fn show_timed_warning_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.toasts
            .push_timed_styled(title, body, self.toast_timeout(), 1, Warning);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
    }

    pub(super) fn start_task_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) -> ToastTaskId {
        let task_id = self.toasts.push_task(title, body, 1);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
        task_id
    }

    pub(super) fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        let linger = if self.toasts.tracked_item_count(task_id) > 0 {
            Duration::from_secs_f64(self.config.current().tui.task_linger_secs)
        } else {
            Duration::ZERO
        };
        self.toasts.finish_task(task_id, linger);
        self.prune_toasts();
    }

    pub(super) fn set_task_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) {
        let linger = Duration::from_secs_f64(self.config.current().tui.task_linger_secs);
        self.toasts.set_tracked_items(task_id, items, linger);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
    }

    pub(super) fn mark_tracked_item_completed(&mut self, task_id: ToastTaskId, key: &str) {
        self.toasts.mark_item_completed(task_id, key);
        let toast_len = self.toasts.active_now().len();
        self.toasts.viewport_mut().set_len(toast_len);
    }

    /// Begin a clean for `project_path`. Returns `true` if a cargo clean
    /// should be spawned; `false` when the project is already clean,
    /// in which case a timed "Already clean" toast is shown and no
    /// spinner is started.
    pub(super) fn start_clean(&mut self, project_path: &AbsolutePath) -> bool {
        let target_dir = self
            .scan
            .resolve_target_dir(project_path)
            .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join("target")));
        if !target_dir.as_path().exists() {
            let name = project::home_relative_path(project_path.as_path());
            self.show_timed_toast("Already clean", name);
            return false;
        }
        self.inflight
            .clean_mut()
            .insert(project_path.clone(), Instant::now());
        self.sync_running_clean_toast();
        true
    }

    pub(super) fn clean_spawn_failed(&mut self, project_path: &AbsolutePath) {
        self.inflight.clean_mut().remove(project_path.as_path());
        self.sync_running_clean_toast();
    }

    pub(super) fn dismiss_toast(&mut self, id: u64) {
        self.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub(super) fn selected_ci_path(&self) -> Option<AbsolutePath> {
        let path = self.selected_project_path()?;
        let entry = self.project_list.entry_containing(path)?;
        Some(entry.item.path().clone())
    }

    pub(super) fn selected_ci_runs(&self) -> Vec<CiRun> {
        self.selected_project_path()
            .map_or_else(Vec::new, |path| self.ci_runs_for_display(path))
    }

    pub(super) fn ci_for(&self, path: &Path) -> Option<Conclusion> {
        // A branch with no upstream tracking can't have CI runs — don't
        // show the parent repo's result for an unpushed worktree branch.
        if self.project_list.unpublished_ci_branch_name(path).is_some() {
            return None;
        }
        self.project_list
            .ci_info_for(path)
            .and_then(|_| self.latest_ci_run_for_path(path))
            .map(|run| run.conclusion)
    }

    pub(super) fn ci_is_fetching(&self, path: &Path) -> bool {
        self.project_list.entry_containing(path).is_some_and(|entry| {
            self.ci
                .fetch_tracker()
                .is_fetching(entry.item.path().as_path())
        })
    }

    /// All absolute paths for a `RootItem` (root + worktrees).
    fn unique_item_paths(item: &RootItem) -> Vec<AbsolutePath> {
        let mut paths = Vec::new();
        paths.push(item.path().clone());
        match item {
            RootItem::Worktrees(WorktreeGroup::Workspaces { linked, .. }) => {
                for l in linked {
                    let p = l.path().clone();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            RootItem::Worktrees(WorktreeGroup::Packages { linked, .. }) => {
                for l in linked {
                    let p = l.path().clone();
                    if !paths.contains(&p) {
                        paths.push(p);
                    }
                }
            },
            _ => {},
        }
        paths
    }

    /// Aggregate CI for a `RootItem`.
    pub(super) fn ci_for_item(&self, item: &RootItem) -> Option<Conclusion> {
        let paths = Self::unique_item_paths(item);
        if paths.len() == 1 {
            return self.ci_for(&paths[0]);
        }
        let mut any_red = false;
        let mut all_green = true;
        let mut any_data = false;
        for path in &paths {
            if let Some(run) = self.latest_ci_run_for_path(path) {
                any_data = true;
                if run.conclusion.is_failure() {
                    any_red = true;
                    all_green = false;
                } else if !run.conclusion.is_success() {
                    all_green = false;
                }
            }
        }
        if !any_data {
            None
        } else if any_red {
            Some(Conclusion::Failure)
        } else if all_green {
            Some(Conclusion::Success)
        } else {
            None
        }
    }

    pub(super) fn animation_elapsed(&self) -> Duration { self.animation_started.elapsed() }

    pub(super) fn register_discovery_shimmer(&mut self, path: &Path) {
        if !self.scan.is_complete() || !self.config.discovery_shimmer_enabled() {
            return;
        }
        let shimmer =
            types::DiscoveryShimmer::new(Instant::now(), self.config.discovery_shimmer_duration());
        self.scan
            .discovery_shimmers_mut()
            .insert(AbsolutePath::from(path), shimmer);
    }

    pub(super) fn discovery_name_segments_for_path(
        &self,
        row_path: &Path,
        name: &str,
        git_status: Option<GitStatus>,
        row_kind: DiscoveryRowKind,
    ) -> Option<Vec<StyledSegment>> {
        if !self.config.discovery_shimmer_enabled() {
            return None;
        }
        let now = Instant::now();
        let (session_path, shimmer) =
            self.discovery_shimmer_session_for_path(row_path, now, row_kind)?;
        let char_count = name.chars().count();
        if char_count == 0 {
            return None;
        }

        let base_style = columns::project_name_style(git_status);
        let accent_style = columns::project_name_shimmer_style(git_status);
        let window = discovery_shimmer_window_len(char_count);
        let elapsed_ms = usize::try_from(now.duration_since(shimmer.started_at).as_millis())
            .unwrap_or(usize::MAX);
        let step = elapsed_ms / discovery_shimmer_step_millis();
        let head = (step
            + discovery_shimmer_phase_offset(
                session_path.as_path(),
                row_path,
                row_kind,
                char_count,
            ))
            % char_count;

        Some(columns::build_shimmer_segments(
            name,
            base_style,
            accent_style,
            head,
            window,
        ))
    }

    fn discovery_shimmer_session_for_path(
        &self,
        row_path: &Path,
        now: Instant,
        row_kind: DiscoveryRowKind,
    ) -> Option<(AbsolutePath, DiscoveryShimmer)> {
        self.scan
            .discovery_shimmers()
            .iter()
            .filter(|(session_path, shimmer)| {
                shimmer.is_active_at(now)
                    && self.discovery_shimmer_session_matches(
                        session_path.as_path(),
                        row_path,
                        row_kind,
                    )
            })
            .max_by_key(|(_, shimmer)| shimmer.started_at)
            .map(|(session_path, shimmer)| (session_path.clone(), *shimmer))
    }

    fn discovery_shimmer_session_matches(
        &self,
        session_path: &Path,
        row_path: &Path,
        row_kind: DiscoveryRowKind,
    ) -> bool {
        self.discovery_scope_contains(session_path, row_path)
            || self
                .discovery_parent_row(session_path)
                .is_some_and(|parent| {
                    parent.path.as_path() == row_path && row_kind.allows_parent_kind(parent.kind)
                })
    }

    fn discovery_scope_contains(&self, session_path: &Path, row_path: &Path) -> bool {
        self.project_list
            .iter()
            .any(|item| root_item_scope_contains(item, session_path, row_path))
    }

    fn discovery_parent_row(&self, session_path: &Path) -> Option<DiscoveryParentRow> {
        self.project_list
            .iter()
            .find_map(|item| root_item_parent_row(item, session_path))
    }

    pub(super) fn selected_project_is_deleted(&self) -> bool {
        self.selected_project_path()
            .is_some_and(|path| self.project_list.is_deleted(path))
    }

    pub(super) fn prune_inactive_project_state(&mut self) {
        let mut all_paths: HashSet<AbsolutePath> = HashSet::new();
        self.project_list.for_each_leaf_path(|path, _| {
            all_paths.insert(AbsolutePath::from(path));
        });
        self.scan
            .pending_git_first_commit_mut()
            .retain(|path, _| all_paths.contains(path));
        self.ci
            .fetch_tracker_mut()
            .retain(|path| all_paths.contains(path));
    }

    pub(super) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        self.project_list.lint_at_path(path)
    }

    pub(super) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        self.project_list.lint_at_path_mut(path)
    }

    pub(super) fn clear_all_lint_state(&mut self) {
        let mut paths = Vec::new();
        self.project_list.for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            if let Some(lr) = self.project_list.lint_at_path_mut(path) {
                lr.clear_runs();
            }
        }
    }

    /// Split-borrow accessor for per-pane render dispatch.
    /// Returns `(&mut Panes, &mut LayoutCache, &Config, &Selection,
    /// &ProjectList)` — the refs the dispatcher passes through to
    /// construct `PaneRenderCtx`. All disjoint `App` fields, so holding
    /// them simultaneously is sound.
    pub(super) const fn split_panes_for_render(
        &mut self,
    ) -> (&mut Panes, &mut LayoutCache, &Config, &ProjectList) {
        (
            &mut self.panes,
            &mut self.layout_cache,
            &self.config,
            &self.project_list,
        )
    }

    /// Split-borrow accessor for the CI pane render path. CI content
    /// lives on the `Ci` subsystem (not `Panes`), so it has its own
    /// split.
    pub(super) const fn split_ci_for_render(&mut self) -> (&mut Ci, &Config, &ProjectList) {
        (&mut self.ci, &self.config, &self.project_list)
    }

    /// Split-borrow accessor for the Lints pane render path. Lints
    /// content lives on the `Lint` subsystem (not `Panes`).
    pub(super) const fn split_lint_for_render(&mut self) -> (&mut Lint, &Config, &ProjectList) {
        (&mut self.lint, &self.config, &self.project_list)
    }

    /// Compute `selected_project_path` once for the current frame
    /// and hand it to per-pane dispatchers via `DispatchArgs`. It
    /// requires both `&Selection` and `&Scan` (resolves a row to
    /// a path), so panes can't recompute it from disjoint borrows
    /// after the dispatcher has split them.
    pub(super) fn selected_project_path_for_render(&self) -> Option<&Path> {
        self.selected_project_path()
    }

    pub(super) const fn mouse_pos(&self) -> Option<Position> { self.mouse_pos }

    pub(super) const fn set_mouse_pos(&mut self, pos: Option<Position>) { self.mouse_pos = pos; }

    pub(super) const fn apply_hovered_pane_row(&mut self) {
        interaction::apply_hovered_pane_row(self);
    }

    pub(super) const fn cached_fit_widths(&self) -> &ProjectListWidths {
        self.project_list.fit_widths()
    }

    pub(super) fn cached_root_sorted(&self) -> &[u64] { self.project_list.cached_root_sorted() }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        self.project_list.cached_child_sorted()
    }

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { self.project_list.expanded() }

    #[cfg(test)]
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        self.project_list.expanded_mut()
    }

    pub(super) const fn finder(&self) -> &FinderState { self.project_list.finder() }

    pub(super) const fn finder_mut(&mut self) -> &mut FinderState { self.project_list.finder_mut() }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.project_list.paths().last_selected.as_ref()
    }

    #[cfg(test)]
    pub(super) fn set_confirm(&mut self, action: ConfirmAction) { self.confirm = Some(action); }

    /// Whether the currently-open confirm is still waiting for a
    /// `cargo metadata` refresh to land (design plan → "Per-worktree
    /// clean, Step 6e"). Callers that gate `y` on a settled plan
    /// consult this.
    /// Open a Clean confirm popup for `project_path`, first checking
    /// whether the project's workspace manifest has drifted since the
    /// last `cargo metadata` run. On drift: dispatch a `cargo metadata` refresh,
    /// mark the confirm as verifying (popup blocks `y` until the
    /// refresh lands). On match: open the confirm Ready immediately.
    pub fn request_clean_confirm(&mut self, project_path: AbsolutePath) {
        if self.should_verify_before_clean(&project_path) {
            let dispatch = self.clean_metadata_dispatch();
            scan::spawn_cargo_metadata_refresh(dispatch, project_path.clone());
            self.scan.set_confirm_verifying(Some(project_path.clone()));
        } else {
            self.scan.set_confirm_verifying(None);
        }
        self.confirm = Some(ConfirmAction::Clean(project_path));
    }

    /// Open the confirm dialog for a group-level clean — fans out to
    /// primary + every linked worktree. The Verifying gate re-uses the
    /// primary's workspace fingerprint; linked worktrees typically share
    /// the same workspace manifest chain (same project, different
    /// branches), so a single-primary re-fetch covers the drift window
    /// for the group. If a linked worktree has diverged independently
    /// (different `.cargo/config.toml`, etc.), its own re-dispatch will
    /// still land before `start_clean` resolves its target dir.
    pub fn request_clean_group_confirm(
        &mut self,
        primary: AbsolutePath,
        linked: Vec<AbsolutePath>,
    ) {
        if self.should_verify_before_clean(&primary) {
            let dispatch = self.clean_metadata_dispatch();
            scan::spawn_cargo_metadata_refresh(dispatch, primary.clone());
            self.scan.set_confirm_verifying(Some(primary.clone()));
        } else {
            self.scan.set_confirm_verifying(None);
        }
        self.confirm = Some(ConfirmAction::CleanGroup { primary, linked });
    }

    /// Does the workspace covering `project_path` need a re-fetch
    /// before the confirm opens? True when the on-disk manifest
    /// fingerprint differs from the stored metadata's fingerprint
    /// (a `.cargo/config.toml` edit, a manifest save, etc.), OR when
    /// no metadata covers `project_path` at all.
    fn should_verify_before_clean(&self, project_path: &AbsolutePath) -> bool {
        let Ok(store) = self.scan.metadata_store().lock() else {
            return false;
        };
        let Some(workspace_root) = store.containing_workspace_root(project_path) else {
            // No metadata covers this path — nothing to verify against.
            return true;
        };
        let Some(metadata) = store.get(workspace_root) else {
            return true;
        };
        let Ok(current) = crate::project::ManifestFingerprint::capture(workspace_root.as_path())
        else {
            return false;
        };
        current != metadata.fingerprint
    }

    /// The scan's `MetadataDispatchContext` refreshed from the current
    /// App state. Used by `request_clean_confirm` to re-dispatch on
    /// fingerprint drift.
    fn clean_metadata_dispatch(&self) -> MetadataDispatchContext {
        MetadataDispatchContext {
            handle:         self.net.http_client_ref().handle.clone(),
            tx:             self.background.bg_sender(),
            metadata_store: Arc::clone(self.scan.metadata_store()),
            // Use the shared scan-concurrency cap so confirm-triggered
            // refreshes can't monopolize the metadata blocking pool.
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(
                crate::constants::SCAN_METADATA_CONCURRENCY,
            )),
        }
    }

    pub(super) const fn confirm(&self) -> Option<&ConfirmAction> { self.confirm.as_ref() }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        let was_empty = self.inflight.example_output_is_empty();
        self.inflight.set_example_output(output);
        if was_empty && !self.inflight.example_output_is_empty() {
            self.focus.set(PaneId::Output);
        }
    }

    /// Borrow `App` for a structural mutation of the project tree.
    /// The returned guard borrows `&mut ProjectList + &mut Panes +
    /// &mut Selection` directly so its `Drop` can fan out across the
    /// three subsystems with the dependency declared at the type
    /// level. `mutate_tree` stays on `App` so callers can split-borrow
    /// the three disjoint fields.
    ///
    /// Mutation guard (RAII) — fan-out flavor. See "Recurring
    /// patterns" in this module.
    pub(super) const fn mutate_tree(&mut self) -> TreeMutation<'_> {
        let include_non_rust = self
            .config
            .current()
            .tui
            .include_non_rust
            .includes_non_rust();
        let Self {
            project_list: projects,
            panes,
            ..
        } = self;
        TreeMutation {
            projects,
            panes,
            include_non_rust,
        }
    }

    pub(super) const fn take_confirm(&mut self) -> Option<ConfirmAction> { self.confirm.take() }

    #[cfg(test)]
    pub(super) fn set_projects(&mut self, projects: ProjectList) {
        self.project_list.replace_roots_from(projects);
    }

    pub(super) fn dismiss_target_for_row(&self, row: VisibleRow) -> Option<DismissTarget> {
        self.dismiss_target_for_row_inner(row)
    }

    pub(super) fn owner_repo_for_path(&self, path: &Path) -> Option<OwnerRepo> {
        self.owner_repo_for_path_inner(path)
    }

    pub(super) fn ci_display_mode_label_for(&self, path: &Path) -> &'static str {
        self.ci_display_mode_label_for_inner(path)
    }

    pub(super) fn ci_toggle_available_for(&self, path: &Path) -> bool {
        self.ci_toggle_available_for_inner(path)
    }

    pub(super) fn toggle_ci_display_mode_for(&mut self, path: &Path) {
        self.toggle_ci_display_mode_for_inner(path);
    }

    pub(super) fn ci_runs_for_display(&self, path: &Path) -> Vec<CiRun> {
        self.ci_runs_for_display_inner(path)
    }

    pub(super) fn reset_cpu_placeholder(&mut self) {
        self.panes.reset_cpu(&self.config.current().cpu);
    }

    // ── Group 3 cross-subsystem orchestrators (post-Phase-9) ────────
    //
    // These read or mutate two or more subsystems and have no single
    // subsystem they belong to. They live in `mod.rs` so `pub(super)`
    // reaches `crate::tui` and external callers (`tui/input.rs`,
    // `tui/render.rs`, `tui/finder.rs`, etc.) can reach them.

    /// Open the settings overlay and position the cursor on `IncludeDirs`
    /// when no include directories are configured. Touches Config +
    /// Focus + Overlays + Panes + `inline_error`.
    pub(super) fn force_settings_if_unconfigured(&mut self) {
        if !self.config.current().tui.include_dirs.is_empty() {
            return;
        }
        self.focus.open_overlay(PaneId::Settings);
        self.overlays.open_settings();
        if let Some(idx) = crate::tui::settings::SettingOption::iter()
            .position(|s| s == SettingOption::IncludeDirs)
        {
            self.overlays
                .settings_pane_mut()
                .viewport_mut()
                .set_pos(idx);
        }
        self.overlays
            .set_inline_error("Configure at least one include directory before continuing");
    }

    /// Derive the current input context from app state. Reads
    /// Overlays + Focus + Panes (via `panes::behavior`).
    pub(super) const fn input_context(&self) -> InputContext {
        use super::panes::PaneBehavior;
        use super::shortcuts::InputContext;
        let overlays = &self.overlays;
        if overlays.keymap_is_awaiting() && overlays.inline_error().is_some() {
            InputContext::KeymapConflict
        } else if overlays.keymap_is_awaiting() {
            InputContext::KeymapAwaiting
        } else if overlays.is_keymap_open() {
            InputContext::Keymap
        } else if overlays.is_finder_open() {
            InputContext::Finder
        } else if overlays.is_settings_editing() {
            InputContext::SettingsEditing
        } else if overlays.is_settings_open() {
            InputContext::Settings
        } else {
            match panes::behavior(self.focus.current()) {
                PaneBehavior::ProjectList | PaneBehavior::Overlay => InputContext::ProjectList,
                PaneBehavior::DetailFields => InputContext::DetailFields,
                PaneBehavior::DetailTargets | PaneBehavior::Cpu => InputContext::DetailTargets,
                PaneBehavior::Lints => InputContext::Lints,
                PaneBehavior::CiRuns => InputContext::CiRuns,
                PaneBehavior::Output => InputContext::Output,
                PaneBehavior::Toasts => InputContext::Toasts,
            }
        }
    }

    /// Whether `pane` is reachable via Tab/Shift-Tab in the current
    /// app state. Reads Selection (cursor → project), Scan (project
    /// data), Panes (pane content), Inflight (example output).
    pub(super) fn is_pane_tabbable(&self, pane: PaneId) -> bool {
        use super::panes;
        use super::panes::PaneBehavior;
        match panes::behavior(pane) {
            PaneBehavior::ProjectList => true,
            PaneBehavior::DetailFields => match pane {
                PaneId::Package => self.selected_project_path().is_some(),
                PaneId::Lang => self.selected_project_path().is_some_and(|path| {
                    self.project_list
                        .at_path(path)
                        .and_then(|p| p.language_stats.as_ref())
                        .is_some_and(|ls| !ls.entries.is_empty())
                }),
                PaneId::Git => self.panes.git().content().is_some_and(|g| {
                    g.branch.is_some() || !g.remotes.is_empty() || !g.worktrees.is_empty()
                }),
                _ => false,
            },
            PaneBehavior::Cpu => self.panes.cpu().content().is_some(),
            PaneBehavior::DetailTargets => self
                .panes
                .targets()
                .content()
                .is_some_and(panes::TargetsData::has_targets),
            PaneBehavior::Lints => {
                self.inflight.example_output_is_empty()
                    && self.lint.content().is_some_and(panes::LintsData::has_runs)
            },
            PaneBehavior::CiRuns => {
                self.inflight.example_output_is_empty()
                    && self.ci.content().is_some_and(panes::CiData::has_runs)
            },
            PaneBehavior::Output => !self.inflight.example_output_is_empty(),
            PaneBehavior::Toasts => !self.toasts.active_now().is_empty(),
            PaneBehavior::Overlay => false,
        }
    }

    /// All currently-tabbable panes, in tab order.
    pub(super) fn tabbable_panes(&self) -> Vec<PaneId> {
        use super::panes;
        panes::tab_order(if self.inflight.example_output_is_empty() {
            BottomRow::Diagnostics
        } else {
            BottomRow::Output
        })
        .into_iter()
        .filter(|pane| self.is_pane_tabbable(*pane))
        .chain(
            self.is_pane_tabbable(PaneId::Toasts)
                .then_some(PaneId::Toasts),
        )
        .collect()
    }

    pub(super) fn focus_next_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.focus.base();
        if current == PaneId::Toasts
            && self.toasts.viewport().pos() + 1 < self.toasts.active_now().len()
        {
            self.toasts.viewport_mut().down();
            self.focus.set(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let next = panes[(index + 1) % panes.len()];
        self.focus.set(next);
        if next == PaneId::Toasts {
            self.toasts.viewport_mut().home();
        }
    }

    pub(super) fn focus_previous_pane(&mut self) {
        self.prune_toasts();
        let panes = self.tabbable_panes();
        if panes.is_empty() {
            return;
        }
        let current = self.focus.base();
        if current == PaneId::Toasts && self.toasts.viewport().pos() > 0 {
            self.toasts.viewport_mut().up();
            self.focus.set(PaneId::Toasts);
            return;
        }
        let index = panes.iter().position(|pane| *pane == current).unwrap_or(0);
        let prev = panes[(index + panes.len() - 1) % panes.len()];
        self.focus.set(prev);
        if prev == PaneId::Toasts {
            let last_index = self.toasts.active_now().len().saturating_sub(1);
            self.toasts.viewport_mut().set_pos(last_index);
        }
    }

    pub(super) fn reset_project_panes(&mut self) {
        self.panes.package_mut().viewport_mut().home();
        self.panes.git_mut().viewport_mut().home();
        self.panes.targets_mut().viewport_mut().home();
        self.ci.viewport_mut().home();
        self.lint.viewport_mut().home();
        self.toasts.viewport_mut().home();
        self.focus.unvisit(PaneId::Package);
        self.focus.unvisit(PaneId::Git);
        self.focus.unvisit(PaneId::Targets);
        self.focus.unvisit(PaneId::CiRuns);
    }
}

/// RAII guard for structural mutations of the project tree.
/// Obtained via [`App::mutate_tree`]; dropped at end of scope (or
/// earlier via `drop`), at which point all tree-derived caches are
/// invalidated.
///
/// **Type-level invariant:** the guard borrows `&mut ProjectList +
/// &mut Panes` simultaneously. New tree-mutation paths added here
/// force the cache-clear to fire on `Drop` — there is no way to
/// forget invalidation. `Drop` runs on every exit path, including
/// panics and early returns.
///
/// Mutation guard (RAII), fan-out flavor. See "Recurring patterns"
/// in [`crate::tui::app`] for the pattern.
pub(super) struct TreeMutation<'a> {
    projects:         &'a mut ProjectList,
    panes:            &'a mut Panes,
    include_non_rust: bool,
}

impl TreeMutation<'_> {
    /// Replace the entire project list (used by tree-build paths).
    pub(super) fn replace_all(&mut self, projects: ProjectList) {
        self.projects.replace_roots_from(projects);
    }

    /// Insert a discovered project into the existing tree, returning
    /// `true` if the insertion changed the tree.
    pub(super) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        self.projects.insert_into_hierarchy(item)
    }

    /// Replace a single leaf at `path` with `item`. Returns the previous
    /// item if one was found.
    pub(super) fn replace_leaf_by_path(&mut self, path: &Path, item: RootItem) -> Option<RootItem> {
        self.projects.replace_leaf_by_path(path, item)
    }

    /// Re-bucket workspace members under inline-dir groups.
    pub(super) fn regroup_members(&mut self, inline_dirs: &[String]) {
        self.projects.regroup_members(inline_dirs);
    }

    /// Re-detect worktree groupings at the top level after a structural
    /// change (insert / replace / remove).
    pub(super) fn regroup_top_level_worktrees(&mut self) {
        self.projects.regroup_top_level_worktrees();
    }
}

impl Drop for TreeMutation<'_> {
    /// Fan out across the two subsystems whose derived state depends
    /// on tree structure:
    /// 1. [`Panes::clear_for_tree_change`] drops `worktree_summary_cache`.
    /// 2. [`ProjectList::recompute_visibility`] rebuilds `cached_visible_rows` against the new
    ///    tree.
    fn drop(&mut self) {
        self.panes.clear_for_tree_change();
        self.projects.recompute_visibility(self.include_non_rust);
    }
}

// ── Discovery shimmer helpers (moved from query/discovery_shimmer.rs) ──

const fn discovery_shimmer_window_len(char_count: usize) -> usize {
    match char_count {
        0 => 0,
        1..=2 => 1,
        3..=5 => 2,
        6..=8 => 3,
        _ => 4,
    }
}

const fn discovery_shimmer_step_millis() -> usize { 85 }

fn discovery_shimmer_phase_offset(
    session_path: &Path,
    row_path: &Path,
    row_kind: DiscoveryRowKind,
    char_count: usize,
) -> usize {
    if char_count == 0 {
        return 0;
    }
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let key = format!(
        "{}|{}|{}",
        session_path.to_string_lossy(),
        row_path.to_string_lossy(),
        row_kind.discriminant()
    );
    for byte in key.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    usize::try_from(hash % u64::try_from(char_count).unwrap_or(1)).unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DiscoveryParentRow {
    path: AbsolutePath,
    kind: DiscoveryRowKind,
}

fn package_contains_path(pkg: &Package, row_path: &Path) -> bool {
    pkg.path() == row_path
        || pkg
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn workspace_contains_path(ws: &Workspace, row_path: &Path) -> bool {
    ws.path() == row_path
        || ws.groups().iter().any(|group| {
            group
                .members()
                .iter()
                .any(|member| package_contains_path(member, row_path))
        })
        || ws
            .vendored()
            .iter()
            .any(|vendored| vendored.path() == row_path)
}

fn root_item_scope_contains(item: &RootItem, session_path: &Path, row_path: &Path) -> bool {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_scope_contains(ws, session_path, row_path)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_scope_contains(pkg, session_path, row_path)
        },
        RootItem::NonRust(project) => project.path() == session_path && project.path() == row_path,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            workspace_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| workspace_scope_contains(l, session_path, row_path))
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            package_scope_contains(primary, session_path, row_path)
                || linked
                    .iter()
                    .any(|l| package_scope_contains(l, session_path, row_path))
        },
    }
}

fn workspace_scope_contains(ws: &Workspace, session_path: &Path, row_path: &Path) -> bool {
    if ws.path() == session_path {
        return workspace_contains_path(ws, row_path);
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
    {
        return true;
    }
    ws.groups().iter().any(|group| {
        group
            .members()
            .iter()
            .any(|member| package_scope_contains(member, session_path, row_path))
    })
}

fn package_scope_contains(pkg: &Package, session_path: &Path, row_path: &Path) -> bool {
    if pkg.path() == session_path {
        return package_contains_path(pkg, row_path);
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path && vendored.path() == row_path)
}

fn root_item_parent_row(item: &RootItem, session_path: &Path) -> Option<DiscoveryParentRow> {
    match item {
        RootItem::Rust(RustProject::Workspace(ws)) => {
            workspace_parent_row(ws, session_path, DiscoveryRowKind::Root)
        },
        RootItem::Rust(RustProject::Package(pkg)) => {
            package_parent_row(pkg, session_path, DiscoveryRowKind::Root)
        },
        RootItem::NonRust(_) => None,
        RootItem::Worktrees(WorktreeGroup::Workspaces {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            workspace_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        workspace_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
        RootItem::Worktrees(WorktreeGroup::Packages {
            primary, linked, ..
        }) => {
            if primary.path() == session_path {
                return None;
            }
            if linked.iter().any(|l| l.path() == session_path) {
                return Some(DiscoveryParentRow {
                    path: primary.path().clone(),
                    kind: DiscoveryRowKind::Root,
                });
            }
            package_parent_row(primary, session_path, DiscoveryRowKind::WorktreeEntry).or_else(
                || {
                    linked.iter().find_map(|l| {
                        package_parent_row(l, session_path, DiscoveryRowKind::WorktreeEntry)
                    })
                },
            )
        },
    }
}

fn workspace_parent_row(
    ws: &Workspace,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if ws.path() == session_path {
        return None;
    }
    if ws
        .vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
    {
        return Some(DiscoveryParentRow {
            path: ws.path().clone(),
            kind: parent_kind,
        });
    }
    for group in ws.groups() {
        for member in group.members() {
            if member.path() == session_path {
                return Some(DiscoveryParentRow {
                    path: ws.path().clone(),
                    kind: parent_kind,
                });
            }
            if let Some(parent) =
                package_parent_row(member, session_path, DiscoveryRowKind::PathOnly)
            {
                return Some(parent);
            }
        }
    }
    None
}

fn package_parent_row(
    pkg: &Package,
    session_path: &Path,
    parent_kind: DiscoveryRowKind,
) -> Option<DiscoveryParentRow> {
    if pkg.path() == session_path {
        return None;
    }
    pkg.vendored()
        .iter()
        .any(|vendored| vendored.path() == session_path)
        .then(|| DiscoveryParentRow {
            path: pkg.path().clone(),
            kind: parent_kind,
        })
}
