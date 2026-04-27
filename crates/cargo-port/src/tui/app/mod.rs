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
//! - **Fan-out flavor** — see [`TreeMutation`] (this module). The guard borrows `&mut Scan + &mut
//!   Panes + &mut Selection` directly so its `Drop` can fan out across the three subsystems with
//!   the dependency declared at the type level. On drop it clears
//!   [`super::panes::Panes::clear_for_tree_change`] and rebuilds
//!   [`super::selection::Selection::recompute_visibility`]. `App::mutate_tree` constructs the guard
//!   via destructuring so the three subsystem borrows are disjoint.
//! - **Self-only flavor** — see [`super::selection::SelectionMutation`]. Visibility-changing
//!   mutations on `Selection` (`toggle_expand`, `apply_finder`) are only callable through the
//!   guard; `Drop` recomputes `cached_visible_rows`.
//!
//! ## Cross-subsystem orchestrator on App
//! Operations that touch multiple subsystems and have no single
//! subsystem where they naturally live stay as named methods on `App`.
//! Their doc comments name every subsystem they touch and instruct
//! future maintainers that new side-effects of the same event MUST be
//! added here, not scattered.
//!
//! - See [`App::apply_lint_config_change`] (Phase 4). Touches Inflight (respawn lint runtime, clear
//!   in-flight paths, sync toast), the Scan-shaped state on App (clear lint state, refresh from
//!   disk, bump `data_generation`), and Selection (recompute fit widths). New side-effects of a
//!   lint-config change MUST be added there.
//!
//! ## Generic primitive plus bespoke state
//! When two subsystems need the same lifecycle but carry different
//! bespoke state, write the lifecycle as a generic struct and have
//! each subsystem compose it.
//!
//! - See [`super::watched_file::WatchedFile<T>`] (Phase 5), composed by
//!   [`super::config_state::Config`] (with the `SettingsEditBuffer` edit buffer) and
//!   [`super::keymap_state::Keymap`] (with the diagnostics-toast id). The primitive captures the
//!   load-on-disk-change contract once; the two subsystems add their bespoke state on top.

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

pub(super) use snapshots::build_visible_rows;
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
use super::background::Background;
use super::config_state::Config;
use super::inflight::Inflight;
use super::keymap_state::Keymap;
use super::pane::PaneManager;
use super::panes::LayoutCache;
use super::panes::PaneDataStore;
use super::panes::PaneId;
use super::panes::Panes;
use super::scan_state::Scan;
use super::selection::Selection;
use crate::ci::CiRun;
use crate::ci::OwnerRepo;
use crate::config::CargoPortConfig;
use crate::http::GitHubRateLimit;
use crate::http::HttpClient;
use crate::keymap::ResolvedKeymap;
use crate::lint::CacheUsage;
use crate::lint::LintRuns;
use crate::project::AbsolutePath;
use crate::project::ProjectCiData;
use crate::project::WorkspaceMetadataHandle;
use crate::project::WorkspaceMetadataStore;
use crate::project::WorkspaceSnapshot;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::RepoCache;

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
pub(super) use target_index::TargetDirIndex;
pub(super) use types::CiFetchTracker;
pub(super) use types::CiRunDisplayMode;
pub(super) use types::ConfirmAction;
pub(super) use types::DirtyState;
pub(super) use types::DiscoveryRowKind;
pub(super) use types::DiscoveryShimmer;
pub(super) use types::ExpandKey;
pub(super) use types::FinderState;
pub(super) use types::HoveredPaneRow;
pub(super) use types::PendingClean;
pub(super) use types::PollBackgroundStats;
#[cfg(test)]
pub(super) use types::RetrySpawnMode;
pub(super) use types::ScanState;
pub(super) use types::SelectionPaths;
pub(super) use types::SelectionSync;
pub(super) use types::VisibleRow;

pub(super) use super::columns::ProjectListWidths;
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
    http_client:       HttpClient,
    github:            GitHubState,
    crates_io:         CratesIoState,
    /// Panes subsystem (Phase 1 of the App-API carve, see
    /// `docs/app-api.md`). Owns `pane_manager`, `pane_data`,
    /// `visited_panes`, `layout_cache`, `worktree_summary_cache`,
    /// `hovered_pane_row`, `ci_display_modes`, `cpu_poller`. App's
    /// impl-files reach pane state through this handle.
    panes:             Panes,
    /// Selection subsystem (Phase 3 of the App-API carve, see
    /// `docs/app-api.md`). Owns `selection_paths`, `selection`
    /// (`SelectionSync`), `expanded`, `finder`,
    /// `cached_visible_rows`, `cached_root_sorted`,
    /// `cached_child_sorted`, `cached_fit_widths` (now
    /// `ProjectListWidths`).
    selection:         Selection,
    /// Background subsystem (Phase 4 of the App-API carve, see
    /// `docs/app-api.md`). Owns the four mpsc channel pairs plus
    /// `watch_tx`. The `bg_*` pair is replaced wholesale on every
    /// rescan via [`Background::swap_bg_channel`]; the others outlive
    /// any single rescan.
    background:        Background,
    /// Inflight subsystem (Phase 4 of the App-API carve). Owns the
    /// running-paths maps, toast slots, ci-fetch tracker, pending
    /// queues, example-runner state, and `lint_runtime`.
    inflight:          Inflight,
    /// Config subsystem (Phase 5 of the App-API carve, see
    /// `docs/app-api.md`). Owns `current_config`, `config_path`,
    /// `config_last_seen`, plus the in-app settings editor's
    /// `SettingsEditBuffer` (the previous `settings_edit_buf` and
    /// `settings_edit_cursor` collapsed into one typed pair).
    /// Composes `WatchedFile<CargoPortConfig>`.
    config:            Config,
    /// Keymap subsystem (Phase 5 of the App-API carve). Owns
    /// `current_keymap`, `keymap_path`, `keymap_last_seen`,
    /// `keymap_diagnostics_id`. Composes
    /// `WatchedFile<ResolvedKeymap>`.
    keymap:            Keymap,
    /// Scan subsystem (Phase 6 of the App-API carve, see
    /// `docs/app-api.md`). Owns `projects`, `scan`
    /// (`ScanState`), `dirty`, `data_generation`,
    /// `discovery_shimmers`, `pending_git_first_commit`,
    /// `metadata_store`, `target_dir_index`, `priority_fetch_path`,
    /// `confirm_verifying`, `lint_cache_usage`, and (test-only)
    /// `retry_spawn_mode`.
    scan:              Scan,
    focused_pane:      PaneId,
    return_focus:      Option<PaneId>,
    confirm:           Option<ConfirmAction>,
    animation_started: Instant,
    mouse_pos:         Option<Position>,
    status_flash:      Option<(String, std::time::Instant)>,
    toasts:            ToastManager,
    inline_error:      Option<String>,
    ui_modes:          types::UiModes,
}

impl App {
    pub(super) const fn current_config(&self) -> &CargoPortConfig { self.config.current() }

    /// Test-only mutable access to the active config. Production
    /// paths route through [`Self::apply_config`] so derived state
    /// (panes, selection, scan-shaped fields) stays in sync.
    #[cfg(test)]
    pub(super) const fn current_config_mut(&mut self) -> &mut CargoPortConfig {
        self.config.current_mut()
    }

    pub(super) const fn current_keymap(&self) -> &ResolvedKeymap { self.keymap.current() }

    pub(super) const fn current_keymap_mut(&mut self) -> &mut ResolvedKeymap {
        self.keymap.current_mut()
    }

    /// Test-only — production paths reach Config sub-fields via
    /// the top-level App accessors (`current_config`, `config_path`,
    /// `settings_edit_*`).
    #[cfg(test)]
    pub(super) const fn config_mut(&mut self) -> &mut Config { &mut self.config }

    pub(super) fn resolved_dirs(&self) -> Vec<AbsolutePath> {
        scan::resolve_include_dirs(&self.config.current().tui.include_dirs)
    }

    pub(super) const fn projects(&self) -> &ProjectList { self.scan.projects() }

    pub(super) const fn projects_mut(&mut self) -> &mut ProjectList { self.scan.projects_mut() }

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

    pub fn complete_ci_fetch_for(&mut self, path: &Path) -> bool {
        self.inflight.ci_fetch_tracker_mut().complete(path)
    }

    pub fn replace_ci_data_for_path(&mut self, path: &Path, ci_data: ProjectCiData) {
        if let Some(repo) = self
            .scan
            .projects_mut()
            .entry_containing_mut(path)
            .and_then(|entry| entry.git_repo.as_mut())
        {
            repo.ci_data = ci_data;
        }
    }

    pub fn start_ci_fetch_for(&mut self, path: AbsolutePath) {
        self.inflight.ci_fetch_tracker_mut().start(path);
    }

    pub(super) const fn lint_cache_usage(&self) -> &CacheUsage { self.scan.lint_cache_usage() }

    pub(super) fn lint_at_path(&self, path: &Path) -> Option<&LintRuns> {
        self.projects().lint_at_path(path)
    }

    pub(super) fn lint_at_path_mut(&mut self, path: &Path) -> Option<&mut LintRuns> {
        self.projects_mut().lint_at_path_mut(path)
    }

    pub(super) fn clear_all_lint_state(&mut self) {
        let mut paths = Vec::new();
        self.projects().for_each_leaf_path(|path, is_rust| {
            if is_rust {
                paths.push(path.to_path_buf());
            }
        });
        for path in &paths {
            if let Some(lr) = self.projects_mut().lint_at_path_mut(path) {
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

    pub(super) const fn cached_fit_widths(&self) -> &ProjectListWidths {
        self.selection.fit_widths()
    }

    pub(super) fn cached_root_sorted(&self) -> &[u64] { self.selection.cached_root_sorted() }

    pub(super) const fn cached_child_sorted(&self) -> &HashMap<usize, Vec<u64>> {
        self.selection.cached_child_sorted()
    }

    pub(super) const fn focused_pane(&self) -> PaneId { self.focused_pane }

    pub(super) const fn expanded(&self) -> &HashSet<ExpandKey> { self.selection.expanded() }

    #[cfg(test)]
    pub(super) const fn expanded_mut(&mut self) -> &mut HashSet<ExpandKey> {
        self.selection.expanded_mut()
    }

    pub(super) const fn pane_manager(&self) -> &PaneManager { self.panes.pane_manager() }

    pub(super) const fn pane_manager_mut(&mut self) -> &mut PaneManager {
        self.panes.pane_manager_mut()
    }

    pub(super) const fn finder(&self) -> &types::FinderState { self.selection.finder() }

    pub(super) const fn finder_mut(&mut self) -> &mut types::FinderState {
        self.selection.finder_mut()
    }

    /// Read-only handle to the [`Selection`] subsystem. Used by
    /// callers that need access to multiple sub-fields in one
    /// Test-only — production paths reach individual sub-fields
    /// through the existing top-level App accessors.
    #[cfg(test)]
    pub(super) const fn selection(&self) -> &Selection { &self.selection }

    /// Test-only — production paths use the documented top-level
    /// accessors. Tests use this to drive `Selection::mutate(...)`
    /// and to inspect inflight-only Selection state.
    #[cfg(test)]
    pub(super) const fn selection_mut(&mut self) -> &mut Selection { &mut self.selection }

    pub(super) const fn last_selected_path(&self) -> Option<&AbsolutePath> {
        self.selection.paths().last_selected.as_ref()
    }

    pub(super) fn set_pending_example_run(&mut self, run: PendingExampleRun) {
        self.inflight.set_pending_example_run(run);
    }

    pub(super) const fn take_pending_example_run(&mut self) -> Option<PendingExampleRun> {
        self.inflight.take_pending_example_run()
    }

    pub(super) fn set_pending_ci_fetch(&mut self, fetch: PendingCiFetch) {
        self.inflight.set_pending_ci_fetch(fetch);
    }

    pub(super) const fn set_ci_fetch_toast(&mut self, task_id: ToastTaskId) {
        self.inflight.set_ci_fetch_toast(Some(task_id));
    }

    pub(super) const fn take_pending_ci_fetch(&mut self) -> Option<PendingCiFetch> {
        self.inflight.take_pending_ci_fetch()
    }

    pub(super) const fn pending_cleans_mut(&mut self) -> &mut VecDeque<PendingClean> {
        self.inflight.pending_cleans_mut()
    }

    /// Test-only — production paths reach background channels via
    /// the per-channel accessors below.
    #[cfg(test)]
    pub(super) const fn background_mut(&mut self) -> &mut Background { &mut self.background }

    /// Read-only handle to the [`Inflight`] subsystem. Test-only —
    /// production paths reach individual sub-fields through the
    /// existing top-level App accessors.
    #[cfg(test)]
    pub(super) const fn inflight(&self) -> &Inflight { &self.inflight }

    #[cfg(test)]
    pub(super) fn set_confirm(&mut self, action: ConfirmAction) { self.confirm = Some(action); }

    /// Whether the currently-open confirm is still waiting for a
    /// `cargo metadata` refresh to land (design plan → "Per-worktree
    /// clean, Step 6e"). Callers that gate `y` on a settled plan
    /// consult this.
    pub const fn confirm_verifying(&self) -> Option<&AbsolutePath> { self.scan.confirm_verifying() }

    /// Open a Clean confirm popup for `project_path`, first checking
    /// whether the project's workspace manifest has drifted since the
    /// last snapshot. On drift: dispatch a `cargo metadata` refresh,
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
    /// fingerprint differs from the stored snapshot's fingerprint
    /// (a `.cargo/config.toml` edit, a manifest save, etc.), OR when
    /// no snapshot covers `project_path` at all.
    fn should_verify_before_clean(&self, project_path: &AbsolutePath) -> bool {
        let Ok(store) = self.scan.metadata_store().lock() else {
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
            tx:             self.background.bg_sender(),
            metadata_store: Arc::clone(self.scan.metadata_store()),
            // Use the shared scan-concurrency cap so confirm-triggered
            // refreshes can't monopolize the metadata blocking pool.
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(
                crate::constants::SCAN_METADATA_CONCURRENCY,
            )),
        }
    }

    /// Clear the verifying flag — called by `handle_cargo_metadata_msg`
    /// when a refresh for the pending workspace lands.
    pub fn clear_confirm_verifying_for(&mut self, workspace_root: &AbsolutePath) {
        if self
            .scan
            .confirm_verifying()
            .is_some_and(|pending| pending == workspace_root)
        {
            self.scan.set_confirm_verifying(None);
        }
    }

    pub(super) const fn confirm(&self) -> Option<&ConfirmAction> { self.confirm.as_ref() }

    pub(super) fn settings_edit_buf(&self) -> &str { self.config.edit_buffer().buf() }

    pub(super) const fn settings_edit_cursor(&self) -> usize { self.config.edit_buffer().cursor() }

    pub(super) const fn settings_edit_parts_mut(&mut self) -> (&mut String, &mut usize) {
        self.config.edit_buffer_mut().parts_mut()
    }

    pub(super) fn set_settings_edit_state(&mut self, value: String, cursor: usize) {
        self.config.edit_buffer_mut().set(value, cursor);
    }

    pub(super) const fn inline_error(&self) -> Option<&String> { self.inline_error.as_ref() }

    pub(super) fn set_inline_error(&mut self, error: impl Into<String>) {
        self.inline_error = Some(error.into());
    }

    pub(super) fn clear_inline_error(&mut self) { self.inline_error = None; }

    pub(super) fn bg_tx(&self) -> mpsc::Sender<BackgroundMsg> { self.background.bg_sender() }

    pub(super) fn http_client(&self) -> HttpClient { self.http_client.clone() }

    pub(super) fn ci_fetch_tx(&self) -> mpsc::Sender<CiFetchMsg> {
        self.background.ci_fetch_sender()
    }

    pub(super) fn clean_tx(&self) -> mpsc::Sender<CleanMsg> { self.background.clean_sender() }

    pub(super) fn example_tx(&self) -> mpsc::Sender<ExampleMsg> { self.background.example_sender() }

    pub(super) fn example_child(&self) -> Arc<Mutex<Option<u32>>> { self.inflight.example_child() }

    pub(super) fn example_output(&self) -> &[String] { self.inflight.example_output() }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        let was_empty = self.inflight.example_output_is_empty();
        self.inflight.set_example_output(output);
        if was_empty && !self.inflight.example_output_is_empty() {
            self.focus_pane(PaneId::Output);
        }
    }

    pub(super) const fn example_output_mut(&mut self) -> &mut Vec<String> {
        self.inflight.example_output_mut()
    }

    pub(super) fn example_running(&self) -> Option<&str> { self.inflight.example_running() }

    pub(super) fn set_example_running(&mut self, running: Option<String>) {
        self.inflight.set_example_running(running);
    }

    pub(super) const fn increment_data_generation(&mut self) { self.scan.bump_generation(); }

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

    /// Borrow `App` for a structural mutation of the project tree.
    /// The returned guard borrows `Scan + Panes + Selection`
    /// directly so its `Drop` can fan out across the three
    /// subsystems with the dependency declared at the type level.
    /// `mutate_tree` stays on `App` (rather than on `Scan`) so
    /// callers can split-borrow the three disjoint App fields:
    /// putting it on `Scan` would force callers to hold
    /// `&mut self.scan` while also passing `&mut self.panes` and
    /// `&mut self.selection`, which the borrow checker rejects
    /// because method-call syntax reborrows the receiver.
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
            scan,
            panes,
            selection,
            ..
        } = self;
        TreeMutation {
            scan,
            panes,
            selection,
            include_non_rust,
        }
    }

    pub(super) fn config_path(&self) -> Option<&Path> { self.config.path() }

    pub(super) fn keymap_path(&self) -> Option<&Path> { self.keymap.path() }

    pub(super) const fn ui_modes(&self) -> &types::UiModes { &self.ui_modes }

    pub(super) const fn take_confirm(&mut self) -> Option<ConfirmAction> { self.confirm.take() }

    #[cfg(test)]
    pub(super) fn set_projects(&mut self, projects: ProjectList) {
        *self.scan.projects_mut() = projects;
    }

    #[cfg(test)]
    pub(super) const fn set_retry_spawn_mode_for_test(&mut self, mode: types::RetrySpawnMode) {
        self.scan.set_retry_spawn_mode(mode);
    }

    /// Test-only — production paths reach Scan sub-fields via the
    /// top-level App accessors (`projects`, `current_config`, etc.).
    #[cfg(test)]
    pub(super) const fn scan(&self) -> &Scan { &self.scan }

    #[cfg(test)]
    pub(super) const fn scan_mut(&mut self) -> &mut Scan { &mut self.scan }

    #[cfg(test)]
    pub(super) const fn scan_state(&self) -> &types::ScanState { self.scan.scan_state() }

    #[cfg(test)]
    pub(super) const fn scan_state_mut(&mut self) -> &mut types::ScanState {
        self.scan.scan_state_mut()
    }

    #[cfg(test)]
    pub(super) const fn data_generation_for_test(&self) -> u64 { self.scan.generation() }

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
        self.panes.reset_cpu(&self.config.current().cpu);
    }

    /// Clone of the process-wide cargo-metadata store. The scan thread and
    /// future refresh paths stamp dispatches with a generation pulled from
    /// this handle, and the main loop merges arrivals back into it.
    pub fn metadata_store_handle(&self) -> Arc<Mutex<WorkspaceMetadataStore>> {
        Arc::clone(self.scan.metadata_store())
    }

    /// Borrow the [`TargetDirIndex`] for read-only lookups (e.g.
    /// confirm-dialog "also affects" listings). Mutation flows only
    /// through the metadata-arrival handler.
    pub const fn target_dir_index_ref(&self) -> &TargetDirIndex { self.scan.target_dir_index() }

    /// Resolve a [`WorkspaceMetadataHandle`] to a cloned snapshot, or `None`
    /// when the workspace has no snapshot yet. Callers get the snapshot by
    /// value; the store lock is released before this returns.
    #[allow(
        dead_code,
        reason = "consumed in later steps (5/6); kept now so WorkspaceMetadataHandle \
                  has a resolve path in place before handle-carrying RustInfo lands"
    )]
    pub fn resolve_metadata(&self, handle: &WorkspaceMetadataHandle) -> Option<WorkspaceSnapshot> {
        self.scan
            .metadata_store()
            .lock()
            .ok()
            .and_then(|store| store.get(&handle.workspace_root).cloned())
    }

    /// Resolve the owning workspace's `target_directory` for any path inside
    /// a known workspace. Accepts project roots, members, worktree entries,
    /// vendored crate roots — the store walks ancestors internally. Returns
    /// `None` when no snapshot covers `path` yet; callers should fall back
    /// to `<project>/target`.
    pub fn resolve_target_dir(&self, path: &AbsolutePath) -> Option<AbsolutePath> {
        self.scan
            .metadata_store()
            .lock()
            .ok()
            .and_then(|store| store.resolved_target_dir(path).cloned())
    }
}

/// RAII guard for structural mutations of the project tree.
/// Obtained via [`App::mutate_tree`]; dropped at end of scope (or
/// earlier via `drop`), at which point all tree-derived caches are
/// invalidated.
///
/// **Type-level invariant:** the guard borrows `&mut Scan + &mut
/// Panes + &mut Selection` simultaneously. New tree-mutation paths
/// added here force the cache-clear to fire on `Drop` — there is
/// no way to forget invalidation. `Drop` runs on every exit path,
/// including panics and early returns.
///
/// Mutation guard (RAII), fan-out flavor. See "Recurring patterns"
/// in [`crate::tui::app`] for the pattern.
pub(super) struct TreeMutation<'a> {
    scan:             &'a mut Scan,
    panes:            &'a mut Panes,
    selection:        &'a mut Selection,
    include_non_rust: bool,
}

impl TreeMutation<'_> {
    /// Replace the entire project list (used by tree-build paths).
    pub(super) fn replace_all(&mut self, projects: ProjectList) {
        *self.scan.projects_mut() = projects;
    }

    /// Insert a discovered project into the existing tree, returning
    /// `true` if the insertion changed the tree.
    pub(super) fn insert_into_hierarchy(&mut self, item: RootItem) -> bool {
        self.scan.projects_mut().insert_into_hierarchy(item)
    }

    /// Replace a single leaf at `path` with `item`. Returns the previous
    /// item if one was found.
    pub(super) fn replace_leaf_by_path(&mut self, path: &Path, item: RootItem) -> Option<RootItem> {
        self.scan.projects_mut().replace_leaf_by_path(path, item)
    }

    /// Re-bucket workspace members under inline-dir groups.
    pub(super) fn regroup_members(&mut self, inline_dirs: &[String]) {
        self.scan.projects_mut().regroup_members(inline_dirs);
    }

    /// Re-detect worktree groupings at the top level after a structural
    /// change (insert / replace / remove).
    pub(super) fn regroup_top_level_worktrees(&mut self) {
        self.scan.projects_mut().regroup_top_level_worktrees();
    }
}

impl Drop for TreeMutation<'_> {
    /// Fan out across the three subsystems whose derived state
    /// depends on tree shape:
    /// 1. [`Panes::clear_for_tree_change`] drops `worktree_summary_cache`.
    /// 2. [`Selection::recompute_visibility`] rebuilds `cached_visible_rows` against the new tree.
    fn drop(&mut self) {
        self.panes.clear_for_tree_change();
        self.selection
            .recompute_visibility(self.scan.projects(), self.include_non_rust);
    }
}
