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
//! - **Fan-out flavor** — see [`TreeMutation`] (this module). The guard currently borrows `&mut
//!   App` and clears `Panes`-owned tree-derived caches via
//!   [`super::panes::Panes::clear_for_tree_change`]. Phase 6 will rewrite the guard to borrow `Scan
//!   + Panes + Selection` directly so the type signature declares the dependency.
//! - **Self-only flavor** — lands in Phase 3 with `SelectionMutation` (stub: see `docs/app-api.md`
//!   § "Phase 3 (Selection)").
//!
//! ## Cross-subsystem orchestrator on App
//! Operations that touch multiple subsystems and have no single
//! subsystem where they naturally live stay as named methods on `App`.
//! Their doc comments name every subsystem they touch and instruct
//! future maintainers that new side-effects of the same event MUST be
//! added here, not scattered.
//!
//! - Lands in Phase 4 with `App::apply_lint_config_change`. See `docs/app-api.md` § "Methods that
//!   stay on App" for the template.
//!
//! ## Generic primitive plus bespoke state
//! When two subsystems need the same lifecycle but carry different
//! bespoke state, write the lifecycle as a generic struct and have
//! each subsystem compose it.
//!
//! - Lands in Phase 5 with `tui::watched_file::WatchedFile<T>`, composed by `Config` (with edit
//!   buffer) and `Keymap` (with diagnostics-toast id).

mod async_tasks;
mod ci;
mod construct;
mod dismiss;
mod focus;
mod lint;
mod navigation;
mod phase_state;
mod query;
mod service_state;
mod snapshots;
mod target_index;
mod types;

use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use ratatui::layout::Position;

use self::service_state::CratesIoState;
use self::service_state::GitHubState;
use super::pane::PaneManager;
use super::panes::LayoutCache;
use super::panes::PaneDataStore;
use super::panes::PaneId;
use super::panes::Panes;
use crate::ci::CiRun;
use crate::ci::OwnerRepo;
use crate::config::CargoPortConfig;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::keymap::ResolvedKeymap;
use crate::lint::CacheUsage;
use crate::lint::LintRuns;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::WorkspaceMetadataHandle;
use crate::project::WorkspaceMetadataStore;
use crate::project::WorkspaceSnapshot;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::RepoCache;
use crate::watcher::WatcherMsg;

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
mod tests;

pub(super) use dismiss::DismissTarget;
pub(super) use service_state::AvailabilityStatus;
pub(super) use target_index::CleanSelection;
pub(super) use target_index::MemberKind;
pub(super) use types::CiFetchTracker;
pub(super) use types::CiRunDisplayMode;
pub(super) use types::ConfirmAction;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::ExpandKey;
pub(super) use types::HoveredPaneRow;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
pub(super) use types::VisibleRow;

pub(super) use super::columns::ResolvedWidths;
use super::panes::PendingCiFetch;
use super::panes::PendingExampleRun;
use super::panes::WorktreeInfo;
use super::terminal::CiFetchMsg;
use super::terminal::CleanMsg;
use super::terminal::ExampleMsg;
use super::toasts::ToastManager;
use super::toasts::ToastTaskId;
use crate::project::RootItem;
pub(super) struct App {
    current_config:           CargoPortConfig,
    http_client:              HttpClient,
    github:                   GitHubState,
    crates_io:                CratesIoState,
    projects:                 ProjectList,
    ci_fetch_tracker:         CiFetchTracker,
    lint_cache_usage:         CacheUsage,
    discovery_shimmers:       HashMap<AbsolutePath, types::DiscoveryShimmer>,
    pending_git_first_commit: HashMap<AbsolutePath, String>,
    /// Panes subsystem (Phase 1 of the App-API carve, see
    /// `docs/app-api.md`). Owns `pane_manager`, `pane_data`,
    /// `visited_panes`, `layout_cache`, `worktree_summary_cache`,
    /// `hovered_pane_row`, `ci_display_modes`, `cpu_poller`. App's
    /// impl-files reach pane state through this handle.
    panes:                    Panes,
    bg_tx:                    mpsc::Sender<BackgroundMsg>,
    bg_rx:                    mpsc::Receiver<BackgroundMsg>,
    priority_fetch_path:      Option<AbsolutePath>,
    expanded:                 HashSet<ExpandKey>,
    settings_edit_buf:        String,
    settings_edit_cursor:     usize,
    focused_pane:             PaneId,
    return_focus:             Option<PaneId>,
    pending_example_run:      Option<PendingExampleRun>,
    pending_ci_fetch:         Option<PendingCiFetch>,
    pending_cleans:           VecDeque<PendingClean>,
    confirm:                  Option<ConfirmAction>,
    animation_started:        Instant,
    ci_fetch_tx:              mpsc::Sender<CiFetchMsg>,
    ci_fetch_rx:              mpsc::Receiver<CiFetchMsg>,
    clean_tx:                 mpsc::Sender<CleanMsg>,
    clean_rx:                 mpsc::Receiver<CleanMsg>,
    example_running:          Option<String>,
    example_child:            Arc<Mutex<Option<u32>>>,
    example_output:           Vec<String>,
    example_tx:               mpsc::Sender<ExampleMsg>,
    example_rx:               mpsc::Receiver<ExampleMsg>,
    running_clean_paths:      HashMap<AbsolutePath, Instant>,
    clean_toast:              Option<ToastTaskId>,
    running_lint_paths:       HashMap<AbsolutePath, Instant>,
    lint_toast:               Option<ToastTaskId>,
    ci_fetch_toast:           Option<ToastTaskId>,
    watch_tx:                 mpsc::Sender<WatcherMsg>,
    lint_runtime:             Option<RuntimeHandle>,
    selection_paths:          types::SelectionPaths,
    finder:                   types::FinderState,
    cached_visible_rows:      Vec<VisibleRow>,
    cached_root_sorted:       Vec<u64>,
    cached_child_sorted:      HashMap<usize, Vec<u64>>,
    cached_fit_widths:        ResolvedWidths,
    data_generation:          u64,
    mouse_pos:                Option<Position>,
    status_flash:             Option<(String, std::time::Instant)>,
    toasts:                   ToastManager,
    config_path:              Option<AbsolutePath>,
    config_last_seen:         Option<types::ConfigFileStamp>,
    current_keymap:           ResolvedKeymap,
    keymap_path:              Option<AbsolutePath>,
    keymap_last_seen:         Option<types::ConfigFileStamp>,
    keymap_diagnostics_id:    Option<u64>,
    inline_error:             Option<String>,
    ui_modes:                 types::UiModes,
    dirty:                    types::DirtyState,
    scan:                     types::ScanState,
    selection:                types::SelectionSync,
    metadata_store:           Arc<Mutex<WorkspaceMetadataStore>>,
    target_dir_index:         target_index::TargetDirIndex,
    /// Step 6e: workspace root whose manifest fingerprint drifted
    /// between the last snapshot and the moment the user asked for
    /// confirm (`c`). While `Some`, the confirm popup renders
    /// "Verifying target dir…" and 'y' is ignored; the matching
    /// `CargoMetadata` arrival in `handle_cargo_metadata_msg` clears
    /// the slot so the popup transitions to Ready.
    confirm_verifying:        Option<AbsolutePath>,
    #[cfg(test)]
    retry_spawn_mode:         types::RetrySpawnMode,
}

impl App {
    pub(super) const fn current_config(&self) -> &CargoPortConfig { &self.current_config }

    pub(super) const fn current_keymap(&self) -> &ResolvedKeymap { &self.current_keymap }

    pub(super) const fn current_keymap_mut(&mut self) -> &mut ResolvedKeymap {
        &mut self.current_keymap
    }

    pub(super) fn resolved_dirs(&self) -> Vec<AbsolutePath> {
        scan::resolve_include_dirs(&self.current_config.tui.include_dirs)
    }

    pub(super) const fn projects(&self) -> &ProjectList { &self.projects }

    #[cfg(test)]
    pub(super) const fn projects_mut(&mut self) -> &mut ProjectList { &mut self.projects }

    pub(super) const fn repo_fetch_cache(&self) -> &RepoCache { &self.github.fetch_cache }

    /// GitHub availability — `Reachable`, `Unreachable` (network
    /// failure), or `RateLimited`. Used by the Git pane to color the
    /// rate-limit rows and choose the right unavailability suffix.
    pub(super) const fn github_status(&self) -> AvailabilityStatus {
        self.github.availability.status()
    }

    /// Snapshot of GitHub's REST + GraphQL rate-limit buckets. Rebuilt
    /// from the shared `HttpClient` state every frame — not persisted.
    pub(super) fn rate_limit(&self) -> GitHubRateLimit { self.http_client.rate_limit_snapshot() }

    pub(in super::super) fn complete_ci_fetch_for(&mut self, path: &Path) -> bool {
        self.ci_fetch_tracker.complete(path)
    }

    pub(in super::super) fn replace_ci_data_for_path(
        &mut self,
        path: &Path,
        ci_data: ProjectCiData,
    ) {
        if let Some(repo) = self
            .projects
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut())
        {
            repo.ci_data = ci_data;
        }
    }

    pub(in super::super) fn start_ci_fetch_for(&mut self, path: AbsolutePath) {
        self.ci_fetch_tracker.start(path);
    }

    pub(super) const fn lint_cache_usage(&self) -> &CacheUsage { &self.lint_cache_usage }

    pub(super) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        self.projects.lint_at_path(path)
    }

    pub(super) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        self.projects.lint_at_path_mut(path)
    }

    pub(super) fn clear_all_lint_state(&mut self) {
        let mut paths = Vec::new();
        self.projects.for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            if let Some(lr) = self.projects.lint_at_path_mut(path) {
                lr.clear_runs();
            }
        }
    }

    pub(super) const fn layout_cache(&self) -> &LayoutCache { self.panes.layout_cache() }

    pub(super) const fn layout_cache_mut(&mut self) -> &mut LayoutCache {
        self.panes.layout_cache_mut()
    }

    pub(super) const fn pane_data(&self) -> &PaneDataStore { self.panes.pane_data() }

    pub(super) const fn pane_data_mut(&mut self) -> &mut PaneDataStore {
        self.panes.pane_data_mut()
    }

    pub(super) const fn panes_mut(&mut self) -> &mut Panes { &mut self.panes }

    pub(super) const fn mouse_pos(&self) -> Option<Position> { self.mouse_pos }

    pub(super) const fn set_mouse_pos(&mut self, pos: Option<Position>) { self.mouse_pos = pos; }

    pub(super) const fn set_hovered_pane_row(
        &mut self,
        hovered_pane_row: Option<types::HoveredPaneRow>,
    ) {
        self.panes.set_hover(hovered_pane_row);
    }

    pub(super) fn apply_hovered_pane_row(&mut self) { self.panes.apply_hovered_pane_row(); }

    pub(super) const fn cached_fit_widths(&self) -> &ResolvedWidths { &self.cached_fit_widths }

    pub(super) fn cached_root_sorted(&self) -> &[u64] { &self.cached_root_sorted }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        &self.cached_child_sorted
    }

    pub(super) const fn focused_pane(&self) -> PaneId { self.focused_pane }

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { &self.expanded }

    #[cfg(test)]
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> { &mut self.expanded }

    pub(super) const fn pane_manager(&self) -> &PaneManager { self.panes.pane_manager() }

    pub(super) const fn pane_manager_mut(&mut self) -> &mut PaneManager {
        self.panes.pane_manager_mut()
    }

    pub(super) const fn finder(&self) -> &types::FinderState { &self.finder }

    pub(super) const fn finder_mut(&mut self) -> &mut types::FinderState { &mut self.finder }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.selection_paths.last_selected.as_ref()
    }

    pub(super) fn set_pending_example_run(&mut self, run: PendingExampleRun) {
        self.pending_example_run = Some(run);
    }

    pub(super) const fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        self.pending_example_run.take()
    }

    pub(super) fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.pending_ci_fetch = Some(fetch);
    }

    pub(super) const fn set_ci_fetch_toast(&mut self, task_id: ToastTaskId) {
        self.ci_fetch_toast = Some(task_id);
    }

    pub(super) const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.pending_ci_fetch.take()
    }

    pub(super) const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        &mut self.pending_cleans
    }

    #[cfg(test)]
    pub(super) fn set_confirm(&mut self, action: ConfirmAction) { self.confirm = Some(action); }

    /// Whether the currently-open confirm is still waiting for a
    /// `cargo metadata` refresh to land (design plan → "Per-worktree
    /// clean, Step 6e"). Callers that gate `y` on a settled plan
    /// consult this.
    pub(in super::super) const fn confirm_verifying(&self) -> Option<&AbsolutePath> {
        self.confirm_verifying.as_ref()
    }

    /// Open a Clean confirm popup for `project_path`, first checking
    /// whether the project's workspace manifest has drifted since the
    /// last snapshot. On drift: dispatch a `cargo metadata` refresh,
    /// mark the confirm as verifying (popup blocks `y` until the
    /// refresh lands). On match: open the confirm Ready immediately.
    pub(in super::super) fn request_clean_confirm(&mut self, project_path: AbsolutePath) {
        if self.should_verify_before_clean(&project_path) {
            let dispatch = self.clean_metadata_dispatch();
            scan::spawn_cargo_metadata_refresh(dispatch, project_path.clone());
            self.confirm_verifying = Some(project_path.clone());
        } else {
            self.confirm_verifying = None;
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
    pub(in super::super) fn request_clean_group_confirm(
        &mut self,
        primary: AbsolutePath,
        linked: Vec<AbsolutePath>,
    ) {
        if self.should_verify_before_clean(&primary) {
            let dispatch = self.clean_metadata_dispatch();
            scan::spawn_cargo_metadata_refresh(dispatch, primary.clone());
            self.confirm_verifying = Some(primary.clone());
        } else {
            self.confirm_verifying = None;
        }
        self.confirm = Some(ConfirmAction::CleanGroup { primary, linked });
    }

    /// Does the workspace covering `project_path` need a re-fetch
    /// before the confirm opens? True when the on-disk manifest
    /// fingerprint differs from the stored snapshot's fingerprint
    /// (a `.cargo/config.toml` edit, a manifest save, etc.), OR when
    /// no snapshot covers `project_path` at all.
    fn should_verify_before_clean(&self, project_path: &AbsolutePath) -> bool {
        let Ok(store) = self.metadata_store.lock() else {
            return false;
        };
        let Some(workspace_root) = store.containing_workspace_root(project_path) else {
            // No snapshot covers this path — nothing to verify against.
            return true;
        };
        let Some(snapshot) = store.get(workspace_root) else {
            return true;
        };
        let Ok(current) = crate::project::ManifestFingerprint::capture(workspace_root.as_path())
        else {
            return false;
        };
        current != snapshot.fingerprint
    }

    /// The scan's `MetadataDispatchContext` refreshed from the current
    /// App state. Used by `request_clean_confirm` to re-dispatch on
    /// fingerprint drift.
    fn clean_metadata_dispatch(&self) -> scan::MetadataDispatchContext {
        scan::MetadataDispatchContext {
            handle:         self.http_client.handle.clone(),
            tx:             self.bg_tx.clone(),
            metadata_store: Arc::clone(&self.metadata_store),
            // Use the shared scan-concurrency cap so confirm-triggered
            // refreshes can't monopolize the metadata blocking pool.
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(
                crate::constants::SCAN_METADATA_CONCURRENCY,
            )),
        }
    }

    /// Clear the verifying flag — called by `handle_cargo_metadata_msg`
    /// when a refresh for the pending workspace lands.
    pub(in super::super) fn clear_confirm_verifying_for(&mut self, workspace_root: &AbsolutePath) {
        if self
            .confirm_verifying
            .as_ref()
            .is_some_and(|pending| pending == workspace_root)
        {
            self.confirm_verifying = None;
        }
    }

    pub(super) const fn confirm(&self) -> Option<&ConfirmAction> { self.confirm.as_ref() }

    pub(super) fn settings_edit_buf(&self) -> &str { &self.settings_edit_buf }

    pub(super) const fn settings_edit_cursor(&self) -> usize { self.settings_edit_cursor }

    pub(super) const fn settings_edit_parts_mut(&mut self) -> (&mut String, &mut usize) {
        (&mut self.settings_edit_buf, &mut self.settings_edit_cursor)
    }

    pub(super) fn set_settings_edit_state(&mut self, value: String, cursor: usize) {
        self.settings_edit_buf = value;
        self.settings_edit_cursor = cursor;
    }

    pub(super) const fn inline_error(&self) -> Option<&String> { self.inline_error.as_ref() }

    pub(super) fn set_inline_error(&mut self, error: impl Into<String>) {
        self.inline_error = Some(error.into());
    }

    pub(super) fn clear_inline_error(&mut self) { self.inline_error = None; }

    pub(super) fn bg_tx(&self) -> mpsc::Sender<BackgroundMsg> { self.bg_tx.clone() }

    pub(super) fn http_client(&self) -> HttpClient { self.http_client.clone() }

    pub(super) fn ci_fetch_tx(&self) -> mpsc::Sender<CiFetchMsg> { self.ci_fetch_tx.clone() }

    pub(super) fn clean_tx(&self) -> mpsc::Sender<CleanMsg> { self.clean_tx.clone() }

    pub(super) fn example_tx(&self) -> mpsc::Sender<ExampleMsg> { self.example_tx.clone() }

    pub(super) fn example_child(&self) -> Arc<Mutex<Option<u32>>> {
        Arc::clone(&self.example_child)
    }

    pub(super) fn example_output(&self) -> &[String] { &self.example_output }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        let was_empty = self.example_output.is_empty();
        self.example_output = output;
        if was_empty && !self.example_output.is_empty() {
            self.focus_pane(PaneId::Output);
        }
    }

    pub(super) const fn example_output_mut(&mut self) -> &mut Vec<String> {
        &mut self.example_output
    }

    pub(super) fn example_running(&self) -> Option<&str> { self.example_running.as_deref() }

    pub(super) fn set_example_running(&mut self, running: Option<String>) {
        self.example_running = running;
    }

    pub(super) const fn increment_data_generation(&mut self) { self.data_generation += 1; }

    /// Delegates to `Panes::worktree_summary_or_compute`. Kept on App
    /// so existing call sites (e.g. `panes/support.rs`) need no
    /// rewrite this phase.
    pub(super) fn worktree_summary_or_compute(
        &self,
        group_root: &Path,
        compute: impl FnOnce() -> Vec<WorktreeInfo>,
    ) -> Vec<WorktreeInfo> {
        self.panes.worktree_summary_or_compute(group_root, compute)
    }

    /// Borrow `App` for a structural mutation of the project tree. The
    /// returned guard is the **only** way (in production code) to call
    /// methods that change which projects are present, where they live,
    /// or how they are grouped. On drop, the guard clears all
    /// tree-derived caches (currently `worktree_summary_cache`), so
    /// invalidation cannot drift out of sync with mutation: the
    /// borrow checker forces every structural change through this entry
    /// point, and `Drop` runs unconditionally — even on early return or
    /// panic.
    pub(super) const fn mutate_tree(&mut self) -> TreeMutation<'_> { TreeMutation { app: self } }

    pub(super) const fn config_path(&self) -> Option<&AbsolutePath> { self.config_path.as_ref() }

    pub(super) const fn keymap_path(&self) -> Option<&AbsolutePath> { self.keymap_path.as_ref() }

    pub(super) const fn ui_modes(&self) -> &types::UiModes { &self.ui_modes }

    pub(super) const fn take_confirm(&mut self) -> Option<ConfirmAction> { self.confirm.take() }

    #[cfg(test)]
    pub(super) fn set_projects(&mut self, projects: ProjectList) { self.projects = projects; }

    #[cfg(test)]
    pub(super) const fn toasts_mut(&mut self) -> &mut ToastManager { &mut self.toasts }

    pub(super) fn dismiss_target_for_row(&self, row: VisibleRow) -> Option<DismissTarget> {
        self.dismiss_target_for_row_inner(row)
    }

    pub(super) fn owner_repo_for_path(&self, path: &std::path::Path) -> Option<OwnerRepo> {
        self.owner_repo_for_path_inner(path)
    }

    pub(super) fn ci_display_mode_label_for(&self, path: &std::path::Path) -> &'static str {
        self.ci_display_mode_label_for_inner(path)
    }

    pub(super) fn ci_toggle_available_for(&self, path: &std::path::Path) -> bool {
        self.ci_toggle_available_for_inner(path)
    }

    pub(super) fn toggle_ci_display_mode_for(&mut self, path: &std::path::Path) {
        self.toggle_ci_display_mode_for_inner(path);
    }

    pub(super) fn ci_runs_for_display(&self, path: &std::path::Path) -> Vec<CiRun> {
        self.ci_runs_for_display_inner(path)
    }

    pub(super) fn poll_cpu_if_due(&mut self, now: Instant) { self.panes.cpu_tick(now); }

    pub(super) fn reset_cpu_placeholder(&mut self) {
        self.panes.reset_cpu(&self.current_config.cpu);
    }

    /// Clone of the process-wide cargo-metadata store. The scan thread and
    /// future refresh paths stamp dispatches with a generation pulled from
    /// this handle, and the main loop merges arrivals back into it.
    pub(in super::super) fn metadata_store_handle(&self) -> Arc<Mutex<WorkspaceMetadataStore>> {
        Arc::clone(&self.metadata_store)
    }

    /// Borrow the [`target_index::TargetDirIndex`] for read-only
    /// lookups (e.g. confirm-dialog "also affects" listings). Mutation
    /// flows only through the metadata-arrival handler.
    pub(in super::super) const fn target_dir_index_ref(&self) -> &target_index::TargetDirIndex {
        &self.target_dir_index
    }

    /// Resolve a [`WorkspaceMetadataHandle`] to a cloned snapshot, or `None`
    /// when the workspace has no snapshot yet. Callers get the snapshot by
    /// value; the store lock is released before this returns.
    #[allow(
        dead_code,
        reason = "consumed in later steps (5/6); kept now so WorkspaceMetadataHandle \
                  has a resolve path in place before handle-carrying RustInfo lands"
    )]
    pub(in super::super) fn resolve_metadata(
        &self,
        handle: &WorkspaceMetadataHandle,
    ) -> Option<WorkspaceSnapshot> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.get(&handle.workspace_root).cloned())
    }

    /// Resolve the owning workspace's `target_directory` for any path inside
    /// a known workspace. Accepts project roots, members, worktree entries,
    /// vendored crate roots — the store walks ancestors internally. Returns
    /// `None` when no snapshot covers `path` yet; callers should fall back
    /// to `<project>/target`.
    pub(in super::super) fn resolve_target_dir(&self, path: &AbsolutePath) -> Option<AbsolutePath> {
        self.metadata_store
            .lock()
            .ok()
            .and_then(|store| store.resolved_target_dir(path).cloned())
    }
}

/// RAII guard for structural mutations of the project tree. Obtained via
/// `App::mutate_tree`, dropped at end of scope (or earlier via `drop`),
/// at which point all tree-derived caches are invalidated.
///
/// **Type-level invariant:** the only methods that change tree shape live
/// on this guard, and the guard cannot be constructed outside `App`. New
/// tree-mutation paths must be added here, which forces the cache-clear
/// to fire — there is no way to forget invalidation. `Drop` runs on
/// every exit path, including panics and early returns.
pub(super) struct TreeMutation<'a> {
    app: &'a mut App,
}

impl TreeMutation<'_> {
    /// Replace the entire project list (used by tree-build paths).
    pub(super) fn replace_all(&mut self, projects: ProjectList) { self.app.projects = projects; }

    /// Insert a discovered project into the existing tree, returning
    /// `true` if the insertion changed the tree.
    pub(super) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        self.app.projects.insert_into_hierarchy(item)
    }

    /// Replace a single leaf at `path` with `item`. Returns the previous
    /// item if one was found.
    pub(super) fn replace_leaf_by_path(&mut self, path: &Path, item: RootItem) -> Option<RootItem> {
        self.app.projects.replace_leaf_by_path(path, item)
    }

    /// Re-bucket workspace members under inline-dir groups.
    pub(super) fn regroup_members(&mut self, inline_dirs: &[String]) {
        self.app.projects.regroup_members(inline_dirs);
    }

    /// Re-detect worktree groupings at the top level after a structural
    /// change (insert / replace / remove).
    pub(super) fn regroup_top_level_worktrees(&mut self) {
        self.app.projects.regroup_top_level_worktrees();
    }
}

impl Drop for TreeMutation<'_> {
    /// Phase 1 staging: temporarily routes the existing tree-mutation
    /// invalidation through `Panes::clear_for_tree_change` so the
    /// `worktree_summary_cache` (now owned by `Panes`) is still
    /// cleared. Phase 6 will rewrite `TreeMutation` as a fan-out
    /// guard borrowing `Scan + Panes + Selection` directly; this
    /// per-field nudge will go away then.
    fn drop(&mut self) { self.app.panes.clear_for_tree_change(); }
}
