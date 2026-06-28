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
//! - **Self-only flavor** — see `SelectionMutation` in the project-list module. Visibility-changing
//!   mutations on `ProjectList` (`toggle_expand`, `apply_finder`) are only callable through the
//!   guard; `Drop` recomputes `cached_visible_rows`.
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
//! - See [`tui_pane::WatchedFile<T>`], composed by [`super::state::Config`] and
//!   [`super::state::Keymap`] (with the diagnostics-toast id). The primitive captures the
//!   load-on-disk-change contract once; the two subsystems add their bespoke state on top.

mod async_tasks;
mod ci;
mod confirm_action;
mod constants;
mod construct;
mod discovery;
mod discovery_shimmer;
mod dismiss;
mod finder_state;
mod hovered_pane_row;
mod lint_registration;
mod navigation;
mod pending_clean;
mod phase_state;
mod poll_background_stats;
mod render_registry;
mod scan_state;
mod selection_paths;
mod startup;
mod target_index;
mod toast_action;
mod tree_mutation;

use std::collections::HashSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use anyhow::Error;
use async_tasks::Startup;
pub(super) use ci::CiRunDisplayMode;
pub(crate) use confirm_action::ConfirmAction;
pub(super) use discovery::DiscoveryRowKind;
pub(super) use discovery::DiscoveryShimmer;
pub(super) use discovery_shimmer::discovery_name_segments_for_path_with_refs;
pub(super) use finder_state::FinderState;
pub(super) use hovered_pane_row::HoveredPaneRow;
pub(crate) use pending_clean::PendingClean;
pub(super) use phase_state::CountedPhase;
pub(super) use phase_state::KeyedPhase;
pub(super) use phase_state::LanguagePhase;
pub(super) use poll_background_stats::PollBackgroundStats;
use ratatui::layout::Position;
#[cfg(test)]
pub(super) use scan_state::RetrySpawnMode;
pub(super) use scan_state::ScanState;
pub(super) use selection_paths::SelectionPaths;
pub(super) use selection_paths::SelectionSync;
pub(crate) use target_index::CleanSelection;
pub(crate) use target_index::TargetDirIndex;
pub(super) use toast_action::CargoPortToastAction;
pub(super) use tree_mutation::TreeMutation;
use tui_pane::AppContext;
use tui_pane::ClipboardBackend;
use tui_pane::CopyOutcome;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::FrameworkFocusId;
use tui_pane::GlobalAction;
use tui_pane::KeyBind;
use tui_pane::Keymap as FrameworkKeymap;
use tui_pane::PaneFocusState;
use tui_pane::SystemClipboard;
use tui_pane::ThemeRuntime;
use tui_pane::ToastId;
use tui_pane::ToastStyle::Success;
use tui_pane::ToastStyle::Warning;
use tui_pane::ToastTaskId;
use tui_pane::TrackedItem;

use self::constants::ANIMATION_TICK;
use self::constants::LINT_CANCELLED_TOAST_TITLE;
use self::constants::LINT_PAUSED_TOAST_BODY;
use self::constants::LINT_PAUSED_TOAST_TITLE;
use self::constants::LINT_RESUMED_TOAST_BODY;
use self::constants::LINT_RESUMED_TOAST_TITLE;
pub(super) use super::app_render_state::FinderSplit;
pub(super) use super::app_render_state::OverlayRenderInputs;
pub(super) use super::app_render_state::RenderBorrows;
pub(super) use super::app_render_state::RenderRegistry;
use super::background::Background;
#[cfg(test)]
use super::columns::LintCell;
pub(super) use super::columns::ProjectListWidths;
#[cfg(test)]
use super::columns::StyledSegment;
use super::integration;
use super::integration::AppPaneId;
use super::interaction;
use super::keymap;
use super::overlays::Overlays;
use super::panes;
use super::panes::BottomRow;
use super::panes::PaneBehavior;
use super::panes::PaneId;
use super::panes::Panes;
use super::panes::SyncedDescriptionHeight;
pub(super) use super::project_list::ExpandKey;
use super::project_list::ProjectList;
pub(super) use super::project_list::VisibleRow;
use super::render_context::PaneRenderCtx;
use super::settings;
use super::settings::SettingOption;
use super::settings::StartupSettings;
#[cfg(test)]
use super::startup_services::StartupEffectCounts;
use super::startup_services::StartupEnvironment;
use super::startup_services::StartupServices;
pub(super) use super::state::AvailabilityStatus;
use super::state::Ci;
use super::state::CiStatusLookup;
use super::state::Config;
use super::state::GitStatusTracker;
use super::state::Inflight;
use super::state::Keymap;
use super::state::Lint;
use super::state::Net;
use super::state::Scan;
use super::state::SyncTracker;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::ci::OwnerRepo;
#[cfg(test)]
use crate::constants::LINT_NO_LOG;
use crate::constants::SCAN_METADATA_CONCURRENCY;
use crate::constants::TARGET_DIR;
use crate::http::HttpClient;
use crate::lint;
use crate::lint::LintRuns;
#[cfg(test)]
use crate::lint::LintStatus;
use crate::project;
use crate::project::AbsolutePath;
#[cfg(test)]
use crate::project::GitStatus;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::MetadataDispatchContext;

pub(super) struct App {
    /// Net subsystem. Owns the shared `HttpClient`, the GitHub
    /// sub-state (availability, repo-fetch cache, in-flight set,
    /// running tracker), and the crates.io sub-state
    /// (availability). App orchestration that touches Net plus
    /// other subsystems (toast push/dismiss, retry spawn) stays
    /// as named methods on `App`.
    pub(super) net:                Net,
    /// Panes subsystem. Owns `pane_manager`, `pane_data`,
    /// `hovered_pane_row`, and `cpu_poller`. App's
    /// impl-files reach pane state through this handle.
    pub(super) panes:              Panes,
    /// Background subsystem. Owns the four mpsc channel pairs plus
    /// the watcher handle. The `background_*` pair is replaced wholesale on every
    /// rescan via [`Background::swap_background_channel`]; the others outlive
    /// any single rescan.
    pub(super) background:         Background,
    /// Inflight subsystem. Owns the running-paths maps, toast
    /// slots, pending queues, and example-runner state.
    pub(super) inflight:           Inflight,
    /// Lint subsystem. Owns the lint runtime, in-flight lint
    /// state, the disk cache stat counter, and the startup-pass
    /// trackers.
    pub(super) lint:               Lint,
    /// Ci subsystem. Owns `fetch_tracker`, `fetch_toast`, and
    /// per-project `display_modes`, plus `Ci::package_display`
    /// which returns the typed [`CiDisplay`](crate::tui::state::CiDisplay) for the package
    /// detail row.
    pub(super) ci:                 Ci,
    /// Config subsystem. Owns `current_config`, `config_path`,
    /// and `config_last_seen`. Composes `WatchedFile<CargoPortConfig>`.
    pub(super) config:             Config,
    /// Keymap subsystem. Owns `current_keymap`, `keymap_path`,
    /// `keymap_last_seen`, `keymap_diagnostics_id`. Composes
    /// `WatchedFile<ResolvedKeymap>`.
    pub(super) keymap:             Keymap,
    /// Themes subsystem. Owns the user-themes directory watch and the
    /// parse-error toast slot used to dismiss prior diagnostics when
    /// the registry reloads cleanly. The active theme + registry
    /// themselves live in `tui_pane`'s `THEME_STATE`.
    pub(super) themes:             ThemeRuntime,
    /// Per-project ahead/behind tracker. Holds the eligibility flag,
    /// last-seen value, and the in-flight "Sync changes" task-toast
    /// id used to accumulate transitions within the linger window.
    pub(super) sync_tracker:       SyncTracker,
    /// Per-project `GitStatus` tracker. Holds the last-seen value and
    /// the in-flight "Git status changes" task-toast id used to
    /// accumulate transitions within the linger window.
    pub(super) git_status_tracker: GitStatusTracker,
    /// The central per-project data store. Lint runs, CI info, git
    /// info, language stats, package/workspace fields, and disk usage
    /// all live inside the tree. Every subsystem that produces
    /// per-project data writes into it.
    pub(super) project_list:       ProjectList,
    /// Scan subsystem. Owns `scan` (`ScanState`),
    /// `dirty`, `data_generation`, `discovery_shimmers`,
    /// `pending_git_first_commit`, `metadata_store`,
    /// `target_dir_index`, `priority_fetch_path`,
    /// `confirm_verifying`, `lint_cache_usage`, and (test-only)
    /// `retry_spawn_mode`.
    pub(super) scan:               Scan,
    /// Startup-phase orchestrator. Owns the per-phase trackers
    /// (`disk`, `git`, `repo`, `metadata`, `lint_phase`,
    /// `lint_count`) plus the phase state that decides when the
    /// umbrella "Startup" toast may enter its close countdown.
    pub(super) startup:            Startup,
    pub(super) startup_services:   StartupServices,
    pub(super) visited_panes:      HashSet<AppPaneId>,
    /// Overlays subsystem. Owns the overlay-mode enums
    /// (`FinderMode`, `KeymapMode`),
    /// the transient `inline_error` UI feedback, and the
    /// `status_flash` slot.
    pub(super) overlays:           Overlays,
    confirm:                       Option<ConfirmAction>,
    pub(super) animation_started:  Instant,
    pub(super) mouse_pos:          Option<Position>,
    /// Framework aggregator from `tui_pane`. Owns the focused-pane id,
    /// quit/restart flags, the per-pane mode-query registry, and the
    /// framework-side `Toasts`/`KeymapPane`/`SettingsPane` overlays.
    /// Stored alongside the legacy keymap path; dispatch routes
    /// through it for targeted structural lookups.
    pub(super) framework:          Framework<Self>,
    /// Framework keymap built at startup from
    /// [`tui_pane::Keymap::builder`]. Held in parallel with the legacy
    /// `keymap` field; the legacy path remains authoritative for broad
    /// key dispatch.
    pub(super) framework_keymap:   Rc<FrameworkKeymap<Self>>,
    pub(super) pending_nav_chord:  Vec<KeyBind>,
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
        background_tx: Sender<BackgroundMsg>,
        background_rx: Receiver<BackgroundMsg>,
        startup_settings: StartupSettings,
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Result<Self, Error> {
        construct::AppBuilder::new(
            projects,
            background_tx,
            background_rx,
            startup_settings,
            StartupEnvironment::production(http_client, scan_started_at, metadata_store),
        )
        .open_channels()
        .run_startup()
        .build()
    }

    #[cfg(test)]
    pub(super) fn new_with_startup_environment(
        projects: &[RootItem],
        background_tx: Sender<BackgroundMsg>,
        background_rx: Receiver<BackgroundMsg>,
        startup_settings: StartupSettings,
        startup_environment: StartupEnvironment,
    ) -> Result<Self, Error> {
        construct::AppBuilder::new(
            projects,
            background_tx,
            background_rx,
            startup_settings,
            startup_environment,
        )
        .open_channels()
        .run_startup()
        .build()
    }

    #[cfg(test)]
    pub(super) fn startup_effect_counts(&self) -> StartupEffectCounts {
        self.startup_services.counts()
    }

    /// Whether the currently selected row is a lint-owning node.
    /// Only roots and worktree entries own lint state. Members,
    /// vendored packages, and group headers do not — the match is
    /// exhaustive so new variants must be classified.
    ///
    /// Declared in `mod.rs` (not `lint.rs`) so `pub(super)` reaches
    /// `tui` and satisfies the caller in `tui/panes/actions.rs`.
    pub(super) fn selected_row_owns_lint(&self) -> bool {
        match self.project_list.selected_row() {
            Some(
                VisibleRow::Root { .. }
                | VisibleRow::WorktreeEntry { .. }
                | VisibleRow::WorktreeGroupHeader { .. },
            ) => true,
            Some(
                VisibleRow::GroupHeader { .. }
                | VisibleRow::Member { .. }
                | VisibleRow::MemberVendored { .. }
                | VisibleRow::Vendored { .. }
                | VisibleRow::Submodule { .. }
                | VisibleRow::WorktreeMember { .. }
                | VisibleRow::WorktreeMemberVendored { .. }
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
    /// Test-only thin delegator to
    /// [`crate::tui::state::lint_cell_for`]. Production callers
    /// in `tui/panes/project_list.rs` use the free fn directly
    /// because `Pane::render` has no `&App` to call methods on.
    #[cfg(test)]
    pub(super) fn lint_cell(&self, status: &LintStatus) -> LintCell {
        if !self.config.lint_enabled() {
            return LintCell::from_parts(LINT_NO_LOG, ratatui::style::Style::default());
        }
        let icon =
            integration::lint_icon_for(status.kind()).frame_at(self.animation_started.elapsed());
        let style = if matches!(status, LintStatus::Running(_)) {
            ratatui::style::Style::default().fg(tui_pane::accent_color())
        } else {
            ratatui::style::Style::default()
        };
        LintCell::from_parts(icon, style)
    }

    pub(super) fn prune_toasts(&mut self) {
        let now = Instant::now();
        self.framework.toasts.prune_tracked_items(now);
        self.framework.toasts.prune(now);
        if self.base_focus() == PaneId::Toasts && self.framework.toasts.active_now().is_empty() {
            self.set_focus_to_pane(PaneId::ProjectList);
        }
    }

    /// Wake interval for the event loop — always bounded, never
    /// block-forever. Returns the animation cadence (~80 ms) while any
    /// on-screen animation is live, otherwise a ~1 s idle heartbeat
    /// floor. The floor keeps the mtime-polled config/keymap/theme
    /// reload (`maybe_reload_*_from_disk`) and the 1 s running-targets
    /// poll alive when idle, since the loop drains them on every wake
    /// (PD1) and no filesystem watcher covers those files.
    ///
    /// [`is_animating`](Self::is_animating) must mirror the render-time
    /// spinner/shimmer/toast checks; a new animated element added at
    /// render time without a matching predicate here will not advance
    /// while the screen is otherwise idle.
    pub(super) fn animation_timeout(&self) -> Duration {
        const IDLE_HEARTBEAT: Duration = Duration::from_secs(1);
        if self.is_animating() {
            ANIMATION_TICK
        } else {
            IDLE_HEARTBEAT
        }
    }

    /// Whether any on-screen animation is currently live: scan discovery
    /// shimmers, the in-flight lint / clean / example-run spinners, or an
    /// active toast. Composed from per-subsystem predicates so each owns
    /// the definition of "animating" for its own state.
    fn is_animating(&self) -> bool {
        self.scan.needs_animation()
            || self.project_list.has_running_lints()
            || self.inflight.needs_animation()
            || self.net.github.has_pr_check_polls()
            || !self.framework.toasts.active_now().is_empty()
    }

    pub(super) fn show_timed_toast(&mut self, title: impl Into<String>, body: impl Into<String>) {
        self.framework.toasts.push_status(title, body);
    }

    pub(super) fn copy_focused_selection(&mut self) {
        let mut backend = SystemClipboard::new();
        self.copy_focused_selection_with_backend(&mut backend);
    }

    pub(super) fn copy_focused_selection_with_backend<B>(&mut self, backend: &mut B)
    where
        B: ClipboardBackend,
    {
        let outcome = self.framework.copy_selection(self, backend);
        // A deliberate output-pane yank reports the line count and
        // collapses the selection back to following the tail. The generic
        // CopyOutcome only carries the label, so the count is read here.
        if matches!(outcome, CopyOutcome::Copied { .. }) && self.focus_is(PaneId::Output) {
            let live = self.inflight.example_output().to_vec();
            let count = self.panes.output.selection_line_count(&live);
            self.panes.output.collapse_to_tail();
            let lines = if count == 1 { "line" } else { "lines" };
            self.show_timed_toast("Copy", format!("Copied {count} {lines}"));
            return;
        }
        self.show_copy_outcome(outcome);
    }

    fn show_copy_outcome(&mut self, outcome: CopyOutcome) {
        match outcome {
            CopyOutcome::Copied { label } => {
                self.show_timed_toast("Copy", format!("Copied {}", label.noun()));
            },
            CopyOutcome::NothingToCopy => self.show_timed_toast("Copy", "Nothing to copy"),
            CopyOutcome::Unavailable { reason } => {
                self.show_timed_toast("Clipboard unavailable", reason.to_string());
            },
            CopyOutcome::Failed { reason } => {
                self.show_timed_toast("Copy failed", reason.to_string());
            },
        }
    }

    pub(super) fn show_timed_warning_toast(
        &mut self,
        title: impl Into<String>,
        body: impl Into<String>,
    ) {
        self.framework
            .toasts
            .push_status_styled(title, body, Warning);
    }

    pub(super) fn finish_task_toast(&mut self, task_id: ToastTaskId) {
        self.framework.toasts.finish_task(task_id);
        self.prune_toasts();
    }

    pub(super) fn set_task_tracked_items(&mut self, task_id: ToastTaskId, items: &[TrackedItem]) {
        self.framework.toasts.set_tracked_items(task_id, items);
    }

    /// Begin a clean for `project_path`. Returns `true` if a cargo clean
    /// should be spawned; `false` when the project is already clean,
    /// in which case a timed "Already clean" toast is shown and no
    /// spinner is started.
    pub(super) fn start_clean(&mut self, project_path: &AbsolutePath) -> bool {
        let target_dir = self
            .scan
            .resolve_target_dir(project_path)
            .unwrap_or_else(|| AbsolutePath::from(project_path.as_path().join(TARGET_DIR)));
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

    pub(super) fn dismiss_toast(&mut self, id: ToastId) {
        self.framework.toasts.dismiss(id);
        self.prune_toasts();
    }

    pub(super) fn register_discovery_shimmer(&mut self, path: &Path) {
        if !self.scan.is_complete() || !self.config.discovery_shimmer_enabled() {
            return;
        }
        let shimmer =
            DiscoveryShimmer::new(Instant::now(), self.config.discovery_shimmer_duration());
        self.scan
            .discovery_shimmers_mut()
            .insert(AbsolutePath::from(path), shimmer);
    }

    /// Test-only thin delegator to
    /// [`discovery_name_segments_for_path_with_refs`]. Production
    /// callers use the free fn directly.
    #[cfg(test)]
    pub(super) fn discovery_name_segments_for_path(
        &self,
        row_path: &Path,
        name: &str,
        git_status: Option<GitStatus>,
        row_kind: DiscoveryRowKind,
    ) -> Option<Vec<StyledSegment>> {
        discovery_name_segments_for_path_with_refs(
            &self.scan,
            &self.config,
            &self.project_list,
            row_path,
            name,
            git_status,
            row_kind,
        )
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
            .fetch_tracker
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
            self.lint.clear_running_path(path);
        }
    }

    /// Split-borrow accessor for per-pane render dispatch.
    /// Returns the `&mut Panes` plus the read-only refs the
    /// dispatcher passes through to construct `PaneRenderCtx`. All
    /// Split-borrow accessor for the tiled render loop. Returns the
    /// `&mut` registry plus a fully-built `PaneRenderCtx` whose
    /// borrows are disjoint from the registry's. The single split
    /// pattern lets [`tui_pane::render_panes`] dispatch every tile
    /// pane through one `PaneRegistry` impl, including Lint and Ci
    /// which were on their own split paths before this phase.
    ///
    /// `selected_project_path`, `animation_elapsed`, and
    /// `ci_status_lookup` are passed in because their construction
    /// requires `&self` (multi-subsystem queries / a clone of CI
    /// display-mode state), which has to be released before the
    /// `&mut` split here.
    pub(super) const fn split_for_render<'a>(
        &'a mut self,
        selected_project_path: Option<&'a Path>,
        animation_elapsed: Duration,
        ci_status_lookup: &'a CiStatusLookup,
        overlay_inputs: OverlayRenderInputs<'a>,
        synced_description_height: SyncedDescriptionHeight,
    ) -> RenderBorrows<'a> {
        let Self {
            panes,
            lint,
            ci,
            config,
            project_list,
            inflight,
            scan,
            framework,
            ..
        } = self;
        let running_targets = panes.running_targets.snapshot();
        let registry = RenderRegistry {
            package: &mut panes.package,
            lang: &mut panes.lang,
            cpu: &mut panes.cpu,
            git: &mut panes.git,
            targets: &mut panes.targets,
            project_list: &mut panes.project_list,
            output: &mut panes.output,
            lint,
            ci,
            settings_pane: &mut framework.settings_pane,
        };
        let pane_render_context = PaneRenderCtx {
            animation_elapsed,
            config,
            project_list,
            selected_project_path,
            inflight,
            scan,
            ci_status_lookup,
            settings_render_inputs: overlay_inputs.settings,
            synced_description_height,
            running_targets,
        };
        RenderBorrows {
            registry,
            pane_render_ctx: pane_render_context,
        }
    }

    /// Split-borrow accessor for the app-modal Finder overlay
    /// render path. The finder pane lives on `overlays`; the
    /// disjoint borrow of `&self.config` and `&self.project_list`
    /// is sound. Finder sits outside the tiled render loop because
    /// the popup sizes itself off the whole frame area.
    pub(super) const fn split_finder_for_render(&mut self) -> FinderSplit<'_> {
        FinderSplit {
            finder_pane:     &mut self.overlays.finder_pane,
            config:          &self.config,
            project_list:    &self.project_list,
            inflight:        &self.inflight,
            scan:            &self.scan,
            running_targets: self.panes.running_targets.snapshot(),
        }
    }

    /// Compute `selected_project_path` once for the current frame
    /// and hand it to per-pane dispatchers via `DispatchArgs`. It
    /// requires both `&Selection` and `&Scan` (resolves a row to
    /// a path), so panes can't recompute it from disjoint borrows
    /// after the dispatcher has split them.
    pub(super) fn selected_project_path_for_render(&self) -> Option<&Path> {
        self.project_list.selected_project_path()
    }

    pub(super) fn apply_hovered_pane_row(&mut self) { interaction::apply_hovered_pane_row(self); }

    #[cfg(test)]
    pub(super) fn set_confirm(&mut self, action: ConfirmAction) { self.confirm = Some(action); }

    /// Whether the currently-open confirm is still waiting for a
    /// `cargo metadata` refresh to land. Callers that gate `y` on a
    /// settled plan consult this.
    /// Open a Clean confirm popup for `project_path`, first checking
    /// whether the project's workspace manifest has drifted since the
    /// last `cargo metadata` run. On drift: dispatch a `cargo metadata` refresh,
    /// mark the confirm as verifying (popup blocks `y` until the
    /// refresh lands). On match: open the confirm Ready immediately.
    pub fn request_clean_confirm(&mut self, project_path: AbsolutePath) {
        if self.scan.should_verify_before_clean(&project_path) {
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
        if self.scan.should_verify_before_clean(&primary) {
            let dispatch = self.clean_metadata_dispatch();
            scan::spawn_cargo_metadata_refresh(dispatch, primary.clone());
            self.scan.set_confirm_verifying(Some(primary.clone()));
        } else {
            self.scan.set_confirm_verifying(None);
        }
        self.confirm = Some(ConfirmAction::CleanGroup { primary, linked });
    }

    /// Open a confirm dialog to `SIGTERM` the running instance named by
    /// `label`. The PID is verified against `create_time` immediately
    /// before the signal so a reused PID is never killed.
    pub fn request_kill_confirm(&mut self, label: String, pid: u32, create_time: u64) {
        self.confirm = Some(ConfirmAction::KillTarget {
            label,
            pid,
            create_time,
        });
    }

    /// Toggle the lint pause state (bound to Space). Pausing opens a confirm
    /// dialog (like Clean); resuming is immediate. A no-op when lint is
    /// disabled — there is nothing to pause.
    pub fn toggle_lint_pause(&mut self) {
        if self.lint.runtime().is_none() {
            return;
        }
        if self.lint.is_paused() {
            self.resume_lints();
        } else {
            self.confirm = Some(ConfirmAction::PauseLint);
        }
    }

    /// Pause all lint operations: kill in-flight runs and hold new runs in the
    /// runtime. Surfaces a sticky warning toast that stays until resume.
    /// Invoked from the `PauseLint` confirm on `y`.
    pub(super) fn pause_lints(&mut self) {
        let Some(runtime) = self.lint.runtime() else {
            return;
        };
        runtime.pause();
        lint::record_paused(self.config.current());
        // Count the runs being killed before their terminal statuses drain the
        // running-lint toast, so the cancellation notice can name how many.
        let cancelled = self.lint.running_toast_path_count();
        let id = self.framework.toasts.push_styled(
            LINT_PAUSED_TOAST_TITLE,
            LINT_PAUSED_TOAST_BODY,
            Warning,
        );
        self.lint.set_pause_toast(id);
        if cancelled > 0 {
            let plural = if cancelled == 1 { "lint" } else { "lints" };
            self.framework.toasts.push_status(
                LINT_CANCELLED_TOAST_TITLE,
                format!("Stopped {cancelled} running {plural}."),
            );
        }
    }

    /// Resume lint operations: dismiss the sticky warning toast, tell the
    /// runtime to re-dispatch the catch-up runs accumulated while paused, and
    /// flash a green confirmation toast.
    pub(super) fn resume_lints(&mut self) {
        if let Some(runtime) = self.lint.runtime() {
            runtime.resume();
        }
        lint::record_resumed(self.config.current());
        if let Some(id) = self.lint.take_pause_toast() {
            self.framework.toasts.dismiss(id);
        }
        self.framework.toasts.push_status_styled(
            LINT_RESUMED_TOAST_TITLE,
            LINT_RESUMED_TOAST_BODY,
            Success,
        );
    }

    /// Re-apply the pause state to a freshly spawned runtime after a lint
    /// config change. The new runtime starts unpaused, so a paused session
    /// re-pauses it; if lint became disabled, the sticky toast is cleared
    /// because a missing runtime can't be paused.
    pub(super) fn reapply_lint_pause_after_runtime_swap(&mut self) {
        if !self.lint.is_paused() {
            return;
        }
        if let Some(runtime) = self.lint.runtime() {
            runtime.pause();
        } else if let Some(id) = self.lint.take_pause_toast() {
            self.framework.toasts.dismiss(id);
        }
    }

    /// Resume a paused session paused after a restart. When the persisted pause
    /// marker is set, pause the freshly spawned runtime and re-raise the sticky
    /// warning toast — without the confirm dialog a live pause shows. Runs
    /// before project registration and the startup staleness sweep, so the
    /// triggers those produce are held and shown as `Stale` rather than briefly
    /// starting and then being killed.
    pub(super) fn restore_persisted_lint_pause(&mut self) {
        if self.lint.is_paused() || !lint::is_set(self.config.current()) {
            return;
        }
        let Some(runtime) = self.lint.runtime() else {
            return;
        };
        runtime.pause();
        let id = self.framework.toasts.push_styled(
            LINT_PAUSED_TOAST_TITLE,
            LINT_PAUSED_TOAST_BODY,
            Warning,
        );
        self.lint.set_pause_toast(id);
    }

    /// A `MetadataDispatchContext` built from the current App state.
    /// Any path that admits a Rust project into the list (discovery,
    /// refresh) builds one and hands it to the insertion method, which
    /// dispatches the `cargo metadata` refresh — so a project can't
    /// enter the list without its metadata scheduled.
    pub(super) fn metadata_dispatch(&self) -> MetadataDispatchContext {
        MetadataDispatchContext {
            handle:         self.net.http_client.handle.clone(),
            sender:         self.background.background_sender(),
            metadata_store: Arc::clone(self.scan.metadata_store()),
            // Bound by the scan-concurrency cap so these refreshes can't
            // monopolize the metadata blocking pool.
            metadata_limit: Arc::new(tokio::sync::Semaphore::new(SCAN_METADATA_CONCURRENCY)),
        }
    }

    /// The dispatch context `request_clean_confirm` uses to re-run
    /// `cargo metadata` on fingerprint drift.
    fn clean_metadata_dispatch(&self) -> MetadataDispatchContext { self.metadata_dispatch() }

    pub(super) const fn confirm(&self) -> Option<&ConfirmAction> { self.confirm.as_ref() }

    pub(super) fn set_example_output(&mut self, output: Vec<String>) {
        let was_empty = self.inflight.example_output_is_empty();
        self.inflight.set_example_output(output);
        if was_empty && !self.inflight.example_output_is_empty() {
            self.panes.output.reset_for_open();
            self.set_focus_to_pane(PaneId::Output);
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
        let non_rust = self.config.current().tui.include_non_rust;
        let Self {
            project_list: projects,
            panes,
            ..
        } = self;
        TreeMutation {
            projects,
            panes,
            non_rust,
        }
    }

    pub(super) const fn take_confirm(&mut self) -> Option<ConfirmAction> { self.confirm.take() }

    pub(super) fn owner_repo_for_path(&self, path: &Path) -> Option<OwnerRepo> {
        self.project_list.owner_repo_for_path_inner(path)
    }

    pub(super) fn ci_toggle_available_for(&self, path: &Path) -> bool {
        self.project_list.ci_toggle_available_for_inner(path)
    }

    pub(super) fn set_ci_display_mode_for(&mut self, path: &Path, mode: CiRunDisplayMode) {
        self.set_ci_display_mode_for_inner(path, mode);
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
        self.dispatch_framework_global_action(GlobalAction::OpenSettings);
        if let Some(idx) = settings::selection_index_for_setting(self, SettingOption::IncludeDirs) {
            self.framework.settings_pane.viewport_mut().set_pos(idx);
        }
        self.overlays
            .set_inline_error("Configure at least one include directory before continuing");
    }

    fn dispatch_framework_global_action(&mut self, action: GlobalAction) {
        let keymap = Rc::clone(&self.framework_keymap);
        keymap.dispatch_framework_global(action, self);
    }

    pub(super) fn rebuild_framework_keymap_from_disk(&mut self) -> Result<(), String> {
        let framework_builder = FrameworkKeymap::<Self>::builder().vim_mode(
            integration::vim_mode_from_config(self.config.current().tui.navigation_keys),
        );
        let framework_builder = if let Some(path) = self.keymap.path().map(Path::to_path_buf) {
            let display_path = path.display().to_string();
            keymap::migrate_removed_action_keys_on_disk(&path).map_err(|err| {
                format!("migrating removed keymap actions in {display_path}: {err}")
            })?;
            framework_builder
                .load_toml(path)
                .map_err(|err| format!("loading keymap from {display_path}: {err}"))?
        } else {
            framework_builder
        };
        let framework_keymap =
            integration::build_framework_keymap(framework_builder, &mut self.framework)
                .map_err(|err| format!("building framework keymap: {err}"))?;
        self.framework_keymap = Rc::new(framework_keymap);
        Ok(())
    }

    pub(super) fn close_framework_overlay_if_open(&mut self) {
        if self.framework.overlay().is_some() {
            self.dispatch_framework_global_action(GlobalAction::Dismiss);
        }
    }

    pub(super) const fn focused_pane_id(&self) -> PaneId {
        Self::pane_id_for_focus(*self.framework.focused())
    }

    pub(super) fn focus_is(&self, pane: PaneId) -> bool { self.focused_pane_id() == pane }

    /// Keep focus off whichever bottom-row pane the layout currently
    /// hides. The bottom row shows the full-width Output pane while
    /// example output is present and the Lints/CiRuns diagnostics panes
    /// otherwise — the two layouts are never on screen together. Focus
    /// is tracked separately from that choice, so a stale focus can
    /// point at the hidden pane (e.g. starting a second run while the
    /// previous run's buffer is still shown leaves focus on `CiRuns`).
    /// Redirect focus to the visible counterpart so the status bar and
    /// key dispatch match what the user sees.
    pub(super) fn reconcile_bottom_row_focus(&mut self) {
        let output_active = !self.inflight.example_output_is_empty();
        match (output_active, self.focused_pane_id()) {
            (true, PaneId::Lints | PaneId::CiRuns) => self.set_focus_to_pane(PaneId::Output),
            (false, PaneId::Output) => self.set_focus_to_pane(PaneId::Targets),
            _ => {},
        }
    }

    pub(super) fn base_focus(&self) -> PaneId {
        if self.overlays.is_finder_open() && self.focus_is(PaneId::Finder) {
            return self
                .overlays
                .finder_return()
                .map_or(PaneId::ProjectList, Self::pane_id_for_focus);
        }
        self.focused_pane_id()
    }

    pub(super) fn pane_focus_state(&self, pane: PaneId) -> PaneFocusState {
        if self.focus_is(pane) {
            return PaneFocusState::Active;
        }
        AppPaneId::from_legacy(pane).map_or(PaneFocusState::Inactive, |id| {
            if self.visited_panes.contains(&id) {
                PaneFocusState::Remembered
            } else {
                PaneFocusState::Inactive
            }
        })
    }

    pub(super) fn set_focus_to_pane(&mut self, pane: PaneId) {
        match AppPaneId::from_legacy(pane) {
            Some(id) => self.set_focus(FocusedPane::App(id)),
            None if pane == PaneId::Toasts => {
                self.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
            },
            None => {},
        }
    }

    const fn pane_id_for_focus(focus: FocusedPane<AppPaneId>) -> PaneId {
        match focus {
            FocusedPane::App(id) => id.to_legacy(),
            FocusedPane::Framework(FrameworkFocusId::Toasts) => PaneId::Toasts,
        }
    }

    /// Whether `pane` is reachable via Tab/Shift-Tab in the current
    /// app state. Reads Selection (cursor → project), Scan (project
    /// data), Panes (pane content), Inflight (example output).
    pub(super) fn is_pane_tabbable(&self, pane: PaneId) -> bool {
        match panes::behavior(pane) {
            PaneBehavior::ProjectList => true,
            PaneBehavior::DetailFields => match pane {
                PaneId::Package => self.project_list.selected_project_path().is_some(),
                PaneId::Lang => self
                    .project_list
                    .selected_project_path()
                    .is_some_and(|path| {
                        self.project_list
                            .at_path(path)
                            .and_then(|p| p.language_stats.as_ref())
                            .is_some_and(|ls| !ls.entries.is_empty())
                    }),
                PaneId::Git => self.panes.git.content().is_some_and(|g| {
                    g.head.is_some() || !g.remotes.is_empty() || !g.worktrees.is_empty()
                }),
                _ => false,
            },
            PaneBehavior::Cpu => self.panes.cpu.content().is_some(),
            // The Running list is global, so the pane stays reachable
            // while anything runs even when the project has no targets.
            PaneBehavior::DetailTargets => {
                self.panes
                    .targets
                    .content()
                    .is_some_and(panes::TargetsData::has_targets)
                    || self.panes.running_targets.snapshot().has_instances()
            },
            PaneBehavior::Lints => {
                self.inflight.example_output_is_empty()
                    && self.lint.content().is_some_and(panes::LintsData::has_runs)
            },
            PaneBehavior::CiRuns => {
                self.inflight.example_output_is_empty()
                    && self.ci.content().is_some_and(panes::CiData::has_runs)
            },
            PaneBehavior::Output => !self.inflight.example_output_is_empty(),
            PaneBehavior::Toasts => !self.framework.toasts.active_now().is_empty(),
            PaneBehavior::Overlay => false,
        }
    }

    /// All currently-tabbable panes, in tab order.
    pub(super) fn tabbable_panes(&self) -> Vec<PaneId> {
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

    pub(super) fn reset_project_panes(&mut self) {
        self.panes.package.viewport.home();
        self.panes.git.viewport.home();
        self.panes.targets.viewport.home();
        self.ci.viewport.home();
        self.lint.viewport.home();
        self.framework.toasts.viewport.home();
        self.visited_panes.remove(&AppPaneId::Package);
        self.visited_panes.remove(&AppPaneId::Git);
        self.visited_panes.remove(&AppPaneId::Targets);
        self.visited_panes.remove(&AppPaneId::CiRuns);
    }

    pub fn sync_selected_project(&mut self) {
        self.ensure_visible_rows_cached();
        let current = self
            .project_list
            .selected_project_path()
            .map(AbsolutePath::from);
        if self
            .project_list
            .paths
            .collapsed_anchor
            .as_ref()
            .is_some_and(|anchor| current.as_ref() != Some(anchor))
        {
            self.project_list.paths.collapsed_selected = None;
            self.project_list.paths.collapsed_anchor = None;
        }
        if self.project_list.paths.selected_project == current {
            return;
        }

        self.project_list
            .paths
            .selected_project
            .clone_from(&current);
        self.reset_project_panes();

        let panes = self.tabbable_panes();
        if !panes.contains(&self.base_focus()) {
            self.set_focus_to_pane(PaneId::ProjectList);
        }

        if let Some(return_target) = self.overlays.finder_return()
            && !panes.contains(&Self::pane_id_for_focus(return_target))
        {
            self.overlays
                .set_finder_return(FocusedPane::App(AppPaneId::ProjectList));
        }

        if let Some(abs_path) = current
            && self.project_list.paths.last_selected.as_ref() != Some(&abs_path)
        {
            self.scan.bump_generation();
            self.project_list.paths.last_selected = Some(abs_path);
            self.project_list.mark_sync_changed();
            self.maybe_priority_fetch();
        }
    }
}

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
mod tests {
    use std::collections::HashSet;
    use std::path::Path;
    use std::path::PathBuf;
    use std::process::Command;

    use chrono::DateTime;
    use chrono::FixedOffset;
    use crossterm::event::KeyCode;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;
    use ratatui::style::Modifier;
    use ratatui::style::Style;
    use ratatui::widgets::List;
    use ratatui::widgets::Widget;
    use tui_pane::PaneFocusState;
    use tui_pane::RenderFocus;

    pub(super) use super::App;
    use super::CiRunDisplayMode;
    use super::DiscoveryRowKind;
    use super::DiscoveryShimmer;
    use super::scan_state::ScanPhase;
    use crate::ci::CiRun;
    use crate::ci::CiStatus;
    use crate::ci::FetchStatus;
    use crate::config::CargoPortConfig;
    use crate::config::NonRustInclusion;
    use crate::config::ScrollDirection;
    use crate::constants::WORKTREE;
    use crate::http::ServiceKind;
    use crate::lint::LintRunOrigin;
    use crate::lint::LintStatus;
    use crate::project;
    use crate::project::AbsolutePath;
    use crate::project::CheckoutInfo;
    use crate::project::CiPagination;
    use crate::project::GitStatus;
    use crate::project::HeadState;
    use crate::project::MemberGroup;
    use crate::project::NonRustProject;
    use crate::project::Package;
    use crate::project::ProjectCiData;
    use crate::project::ProjectCiInfo;
    use crate::project::ProjectFields;
    use crate::project::RemoteInfo;
    use crate::project::RemoteKind;
    use crate::project::RepoInfo;
    use crate::project::RootItem;
    use crate::project::RustInfo;
    use crate::project::RustProject;
    use crate::project::VendoredPackage;
    use crate::project::Visibility::Deleted;
    use crate::project::Visibility::Dismissed;
    use crate::project::WorkflowPresence;
    use crate::project::Workspace;
    use crate::project::WorktreeGroup;
    use crate::project::WorktreeStatus;
    use crate::scan;
    use crate::scan::BackgroundMsg;
    use crate::scan::CiFetchResult;
    use crate::tui::columns::COL_NAME;
    use crate::tui::columns::ProjectListWidths;
    use crate::tui::dismiss_target::DismissTarget;
    use crate::tui::panes::CiFetchKind;
    use crate::tui::panes::PREFIX_ROOT_COLLAPSED;
    use crate::tui::panes::PREFIX_ROOT_LEAF;
    use crate::tui::panes::PREFIX_WORKTREE_FLAT;
    use crate::tui::panes::PaneId;
    pub(super) use crate::tui::project_list::ExpandKey;
    use crate::tui::project_list::ProjectList;
    pub(super) use crate::tui::project_list::VisibleRow;
    use crate::tui::render_context::PaneRenderCtx;
    use crate::tui::state::CiStatusLookup;
    use crate::tui::test_support as tui_test_support;
    use crate::tui::test_support::TestApp;

    mod background {
        use scan::CachedRepoData;
        use scan::RepoMetaInfo;

        use super::*;
        use crate::channel;
        use crate::project::AbsolutePath;
        use crate::project::ProjectPrData;
        use crate::tui::project_list::ExpandTarget;
        use crate::tui::startup_services::WatcherHandle;
        use crate::watcher::WatcherMsg;

        #[test]
        fn scan_result_registers_linked_worktrees_with_watcher() {
            let primary = make_workspace_raw_with_primary(
                Some("bevy_window_manager"),
                "~/rust/bevy_window_manager",
                vec![inline_group(vec![Package {
                    path: test_path("~/rust/bevy_window_manager/crates/bevy_window_manager"),
                    name: Some("bevy_window_manager".to_string()),
                    ..Package::default()
                }])],
                None,
                None,
            );
            let linked = make_workspace_raw_with_primary(
                Some("bevy_window_manager_style_fix"),
                "~/rust/bevy_window_manager_style_fix",
                vec![inline_group(vec![Package {
                    path: test_path(
                        "~/rust/bevy_window_manager_style_fix/crates/bevy_window_manager",
                    ),
                    name: Some("bevy_window_manager".to_string()),
                    worktree_status: WorktreeStatus::Linked {
                        primary: test_path("~/rust/bevy_window_manager"),
                    },
                    ..Package::default()
                }])],
                Some("bevy_window_manager_style_fix"),
                Some("~/rust/bevy_window_manager"),
            );
            let mut app = make_app(&[]);
            let (watch_tx, watch_rx) = channel::unbounded();
            app.background
                .replace_watcher(WatcherHandle::active(watch_tx));

            apply_bg_msg(
                &mut app,
                BackgroundMsg::ScanResult {
                    projects:     vec![make_workspace_worktrees_item(
                        primary.clone(),
                        vec![linked.clone()],
                    )],
                    disk_entries: Vec::new(),
                },
            );

            let messages: Vec<_> = watch_rx.try_iter().collect();
            let watched_paths: HashSet<AbsolutePath> = messages
                .iter()
                .filter_map(|msg| match msg {
                    WatcherMsg::Register(req) => Some(req.abs_path.clone()),
                    WatcherMsg::InitialRegistrationComplete => None,
                })
                .collect();
            let completion_count = messages
                .iter()
                .filter(|msg| matches!(msg, WatcherMsg::InitialRegistrationComplete))
                .count();

            assert!(
                watched_paths.contains(primary.path().as_path()),
                "primary worktree root should be registered with watcher"
            );
            assert!(
                watched_paths.contains(linked.path().as_path()),
                "linked worktree root should be registered with watcher"
            );
            assert_eq!(
                completion_count, 1,
                "scan result should finish the watcher registration batch"
            );
        }

        #[test]
        fn empty_scan_result_finishes_watcher_registration_batch() {
            let mut app = make_app(&[]);
            let (watch_tx, watch_rx) = channel::unbounded();
            app.background
                .replace_watcher(WatcherHandle::active(watch_tx));

            apply_bg_msg(
                &mut app,
                BackgroundMsg::ScanResult {
                    projects:     Vec::new(),
                    disk_entries: Vec::new(),
                },
            );

            let messages: Vec<_> = watch_rx.try_iter().collect();
            assert_eq!(messages.len(), 1);
            assert!(matches!(
                messages[0],
                WatcherMsg::InitialRegistrationComplete
            ));
        }

        #[test]
        fn quiet_scan_result_does_not_start_startup_workers_or_wait_for_lint_history() {
            let mut app = make_app(&[]);

            apply_bg_msg(
                &mut app,
                BackgroundMsg::ScanResult {
                    projects:     vec![make_project(Some("demo"), "~/demo")],
                    disk_entries: Vec::new(),
                },
            );

            assert_eq!(
                app.startup_effect_counts().real_total(),
                0,
                "quiet ScanResult handling must not start real startup effects"
            );
            assert_eq!(
                app.startup.disk.expected_len(),
                0,
                "quiet ScanResult has no disk producer"
            );
            assert_eq!(
                app.startup.git.expected_len(),
                0,
                "quiet ScanResult has no project-detail git producer"
            );
            assert_eq!(
                app.startup.metadata.expected_len(),
                0,
                "quiet ScanResult has no metadata producer"
            );
            assert!(
                app.startup.crates_io.expected.is_unknown(),
                "quiet ScanResult must not declare crates.io fetches"
            );
            assert!(
                app.startup.lint_phase.expected.is_unknown(),
                "suppressed lint-history hydration must not leave expected paths"
            );
            assert_eq!(
                app.startup.details_declared.expected_len(),
                0,
                "quiet ScanResult must not wait for project-detail declarations"
            );
            assert!(
                app.startup.details_declared.complete_at.is_some(),
                "empty declaration phase should complete immediately"
            );
        }

        #[test]
        fn quiet_rescan_uses_noop_scan_without_real_startup_effects() {
            let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);

            app.rescan();

            assert_eq!(
                app.startup_effect_counts().real_total(),
                0,
                "quiet rescan must not start scan, watcher, lint, network, or git work"
            );
            assert!(
                app.project_list.is_empty(),
                "quiet rescan applies a deterministic empty scan result"
            );
            assert_eq!(app.scan.state.phase, ScanPhase::Complete);
            assert_eq!(
                app.startup.metadata.expected_len(),
                0,
                "quiet rescan must not wait for suppressed metadata work"
            );
            assert!(
                app.startup.lint_phase.expected.is_unknown(),
                "quiet rescan must not wait for suppressed lint history"
            );
        }

        #[test]
        fn scan_result_reapplies_pending_expansion_targets_then_drains_them() {
            let mut app = make_app(&[]);
            // Seed the pending targets the way startup-load and rescan both do.
            app.project_list.paths.pending_expanded = vec![
                ExpandTarget::Root(test_path("~/ws")),
                ExpandTarget::WorktreeGroup(test_path("~/ws_feat"), "crates".to_string()),
            ];

            let primary = make_workspace_raw(
                None,
                "~/ws",
                vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
                None,
            );
            let linked = make_workspace_raw(
                None,
                "~/ws_feat",
                vec![named_group(
                    "crates",
                    vec![make_member(Some("a"), "~/ws_feat/a")],
                )],
                Some("ws_feat"),
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ScanResult {
                    projects:     vec![make_workspace_worktrees_item(primary, vec![linked])],
                    disk_entries: Vec::new(),
                },
            );

            assert!(
                app.project_list.expanded.contains(&ExpandKey::Node(0)),
                "the root re-expands from its pending target"
            );
            assert!(
                app.project_list
                    .expanded
                    .contains(&ExpandKey::WorktreeGroup(0, 1, 0)),
                "the linked worktree's named group re-expands at depth"
            );
            assert!(
                app.project_list.paths.pending_expanded.is_empty(),
                "pending targets are drained once applied, so the next scan starts clean"
            );
        }

        #[test]
        fn external_config_reload_applies_valid_changes() {
            let mut app = make_app(&[]);
            let dir = tempfile::tempdir().expect("create test tempdir");
            let path = dir.path().join("config.toml");

            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.editor = "helix".to_string();
            cargo_port_config.tui.ci_run_count = 9;
            cargo_port_config.cpu.poll_ms = 1500;
            cargo_port_config.mouse.invert_scroll = ScrollDirection::Normal;
            std::fs::write(
                &path,
                toml::to_string_pretty(&cargo_port_config).expect("serialize test config"),
            )
            .expect("write test file");

            app.config.force_reload_from(path);
            app.maybe_reload_config_from_disk();

            assert_eq!(app.config.editor(), "helix");
            assert_eq!(app.config.ci_run_count(), 9);
            assert_eq!(app.config.current().cpu.poll_ms, 1500);
            assert_eq!(app.config.invert_scroll(), ScrollDirection::Normal);
            assert_eq!(app.config.current().tui.editor, "helix");
            assert_eq!(app.config.current().tui.ci_run_count, 9);
            assert_eq!(
                app.framework.settings_store().table()["tui"]["editor"].as_str(),
                Some("helix")
            );
        }

        #[test]
        fn external_config_reload_keeps_last_good_config_on_parse_error() {
            let mut app = make_app(&[]);
            let dir = tempfile::tempdir().expect("create test tempdir");
            let path = dir.path().join("config.toml");

            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.editor = "zed".to_string();
            std::fs::write(
                &path,
                toml::to_string_pretty(&cargo_port_config).expect("serialize test config"),
            )
            .expect("write test file");

            app.config.force_reload_from(path.clone());
            app.maybe_reload_config_from_disk();

            std::fs::write(&path, "[tui\neditor = \"vim\"\n").expect("write test file");
            app.config.force_reload_from(path);
            app.maybe_reload_config_from_disk();

            assert_eq!(app.config.editor(), "zed");
            assert_eq!(app.config.current().tui.editor, "zed");
            assert!(matches!(
                app.overlays.status_flash(),
                Some((msg, _)) if msg.contains("Config reload failed")
            ));
        }

        #[test]
        fn external_config_reload_keeps_last_good_config_on_validation_error() {
            let mut app = make_app(&[]);
            let dir = tempfile::tempdir().expect("create test tempdir");
            let path = dir.path().join("config.toml");

            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.editor = "zed".to_string();
            std::fs::write(
                &path,
                toml::to_string_pretty(&cargo_port_config).expect("serialize test config"),
            )
            .expect("write test file");

            app.config.force_reload_from(path.clone());
            app.maybe_reload_config_from_disk();
            let last_good_table = app.framework.settings_store().table().clone();

            std::fs::write(&path, "[tui]\neditor = \"vim\"\nmain_branch = \"\"\n")
                .expect("write test file");
            app.config.force_reload_from(path);
            app.maybe_reload_config_from_disk();

            assert_eq!(app.config.editor(), "zed");
            assert_eq!(app.config.current().tui.editor, "zed");
            assert_eq!(app.framework.settings_store().table(), &last_good_table);
            assert!(matches!(
                app.overlays.status_flash(),
                Some((msg, _)) if msg.contains("Config reload failed")
            ));
        }

        #[test]
        fn completed_scan_hides_and_restores_cached_non_rust_projects_without_rescan() {
            let rust_project = make_project(Some("rust"), "~/rust");
            let non_rust_project = make_non_rust_project(Some("js"), "~/js");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.include_non_rust = NonRustInclusion::Include;
            cargo_port_config.tui.include_dirs = vec!["/tmp/test".to_string()];
            let mut app =
                make_app_with_config(&[rust_project, non_rust_project], &cargo_port_config);
            app.scan.state.phase = ScanPhase::Complete;

            assert_eq!(app.project_list.len(), 2);

            let mut hide_config = cargo_port_config.clone();
            hide_config.tui.include_non_rust = NonRustInclusion::Exclude;
            app.apply_config(&hide_config);
            wait_for_tree_build(&mut app);

            assert!(app.scan.is_complete());
            assert_eq!(app.project_list.len(), 2);
            app.ensure_visible_rows_cached();
            let visible: Vec<_> = app
                .visible_rows()
                .iter()
                .filter_map(|row| match row {
                    VisibleRow::Root { node_index } => {
                        Some(app.project_list[*node_index].path().clone())
                    },
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0], test_path("~/rust"));

            app.apply_config(&cargo_port_config);
            wait_for_tree_build(&mut app);

            assert!(app.scan.is_complete());
            assert_eq!(app.project_list.len(), 2);
            assert!(
                app.project_list
                    .iter()
                    .any(|entry| entry.root_item.path() == test_path("~/js").as_path())
            );
        }

        #[test]
        fn quiet_completed_scan_applies_noop_rescan_when_enabling_non_rust_without_cached_projects()
        {
            let rust_project = make_project(Some("rust"), "~/rust");
            let mut app = make_app(&[rust_project]);
            app.scan.state.phase = ScanPhase::Complete;

            let mut cargo_port_config = app.config.current().clone();
            cargo_port_config.tui.include_non_rust = NonRustInclusion::Include;
            app.apply_config(&cargo_port_config);

            assert!(app.project_list.is_empty());
            assert!(app.scan.is_complete());
            assert_eq!(app.startup_effect_counts().real_total(), 0);
        }

        #[test]
        fn service_reachability_tracks_background_messages() {
            let mut app = make_app(&[]);

            assert!(!app.net.github.availability.is_unavailable());
            assert!(!app.net.crates_io.availability.is_unavailable());

            assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            }));
            assert!(app.net.github.availability.is_unavailable());

            assert!(!app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::CratesIo,
            }));
            assert!(app.net.crates_io.availability.is_unavailable());

            assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
                service: ServiceKind::GitHub,
            }));
            assert!(!app.net.github.availability.is_unavailable());
            assert!(app.net.crates_io.availability.is_unavailable());

            assert!(!app.handle_bg_msg(BackgroundMsg::ServiceReachable {
                service: ServiceKind::CratesIo,
            }));
            assert!(!app.net.github.availability.is_unavailable());
            assert!(!app.net.crates_io.availability.is_unavailable());
        }

        #[test]
        fn successful_request_dismisses_stuck_unreachable_toast() {
            // Regression: `Reachable` signals used to be no-ops when the
            // service was already marked unavailable. That left the
            // persistent toast stuck whenever the retry probe couldn't
            // complete (tight 1s HEAD timeout on a slow link, graphql quota
            // quirks, etc.) even while real data fetches were succeeding.
            // A successful request is authoritative evidence the service
            // works — it must clear the toast.
            //
            // Under the grace-period flow the toast only surfaces once
            // `ServiceUnreachableConfirmed` arrives — `ServiceUnreachable`
            // alone is silent. Drive that explicitly here so the regression
            // assertion still applies to a *surfaced* toast.
            let mut app = make_app(&[]);

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            });
            assert!(
                app.net.github.availability.toast_id().is_none(),
                "Unreachable alone must not surface a toast — grace window first"
            );
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::GitHub,
            });
            let toast_id = app
                .net
                .github
                .availability
                .toast_id()
                .expect("confirmed signal pushes the toast");
            assert!(app.framework.toasts.is_alive(toast_id));
            assert!(app.net.github.availability.is_unavailable());

            app.handle_bg_msg(BackgroundMsg::ServiceReachable {
                service: ServiceKind::GitHub,
            });
            assert!(
                !app.net.github.availability.is_unavailable(),
                "reachable signal should flip status back to available"
            );
            assert!(
                !app.framework.toasts.is_alive(toast_id),
                "reachable signal must dismiss the persistent unreachable toast"
            );
        }

        #[test]
        fn unreachable_toast_reappears_after_user_dismissal() {
            // Regression: dismissing the persistent unreachable toast by hand
            // left `ServiceAvailability.unavailable_toast` holding a stale id.
            // Subsequent confirmed unreachable signals saw the stale id and
            // silently did nothing, so the user had no visible indicator
            // that GitHub was still unreachable.
            let mut app = make_app(&[]);

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::GitHub,
            });
            let toast_id = app
                .net
                .github
                .availability
                .toast_id()
                .expect("confirmed signal pushes a toast");

            // User dismisses the toast. Prune at a synthetic future time so
            // the exit animation has completed without sleeping in the test.
            app.dismiss_toast(toast_id);
            let after_exit = std::time::Instant::now() + std::time::Duration::from_secs(1);
            app.framework.toasts.prune(after_exit);
            assert!(
                !app.framework.toasts.is_alive(toast_id),
                "dismissed toast should no longer be alive after exit animation"
            );

            // Another confirmed signal (the retry probe reports still down)
            // must re-push a fresh toast instead of silently doing nothing.
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::GitHub,
            });
            let new_id = app
                .net
                .github
                .availability
                .toast_id()
                .expect("second confirmed signal should retain a toast id");
            assert_ne!(
                new_id, toast_id,
                "a fresh toast should be pushed with a new id"
            );
            assert!(
                app.framework.toasts.is_alive(new_id),
                "the new toast should be visible"
            );
        }

        #[test]
        fn transient_unreachable_then_reachable_surfaces_no_toast() {
            // Single timeout in a stream of fetches: the retry thread starts
            // its grace sleep, but a real fetch lands `Reachable` before
            // confirmation. Neither the "unreachable" nor "back online"
            // toast should ever surface — that's the whole point of the
            // grace window.
            let mut app = make_app(&[]);
            let baseline_toast_count = app.framework.toasts.active().len();

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::CratesIo,
            });
            assert!(
                app.net.crates_io.availability.toast_id().is_none(),
                "no toast id during the grace window"
            );

            app.handle_bg_msg(BackgroundMsg::ServiceReachable {
                service: ServiceKind::CratesIo,
            });
            assert!(
                !app.net.crates_io.availability.is_unavailable(),
                "state must flip back to reachable"
            );
            assert_eq!(
                app.framework.toasts.active().len(),
                baseline_toast_count,
                "no toasts surfaced — neither unreachable nor back-online"
            );
        }

        #[test]
        fn confirm_after_recovered_during_grace_does_not_resurface_toast() {
            // The retry thread slept the grace window, then probed and
            // failed, then emitted `ServiceUnreachableConfirmed`. But during
            // that gap a successful real fetch already marked the service
            // reachable. The stale confirm must NOT push a toast.
            let mut app = make_app(&[]);
            let baseline_toast_count = app.framework.toasts.active().len();

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::CratesIo,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceReachable {
                service: ServiceKind::CratesIo,
            });
            // Late confirm arrives after state already recovered.
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::CratesIo,
            });
            assert!(
                app.net.crates_io.availability.toast_id().is_none(),
                "no toast id should be set — state was already reachable"
            );
            assert_eq!(
                app.framework.toasts.active().len(),
                baseline_toast_count,
                "stale confirm must be a no-op"
            );
        }

        #[test]
        fn recovered_without_confirm_suppresses_back_online_toast() {
            // The grace-window happy path: brief blip, retry thread's first
            // probe succeeds, `ServiceRecovered` arrives. Since we never
            // pushed an "unreachable" toast, we must not push the matching
            // "back online" toast either.
            let mut app = make_app(&[]);
            let baseline_toast_count = app.framework.toasts.active().len();

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::CratesIo,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceRecovered {
                service: ServiceKind::CratesIo,
            });
            assert!(
                !app.net.crates_io.availability.is_unavailable(),
                "state must flip back to reachable"
            );
            assert_eq!(
                app.framework.toasts.active().len(),
                baseline_toast_count,
                "no back-online toast because no unreachable toast ever surfaced"
            );
        }

        #[test]
        fn recovery_invalidates_failed_github_cache_entries() {
            // The repo cache stores both successful and failed fetches; the
            // failed ones are flagged by `meta.is_none()` (a successful
            // GraphQL call always returns a meta payload). On recovery, the
            // refetch sweep must drop the failed entries so the next fetch
            // actually runs against the network, while leaving successful
            // entries in place to avoid burning quota on data we already have.
            let mut app = make_app(&[]);
            let success = crate::ci::OwnerRepo::new("acme", "good");
            let failure = crate::ci::OwnerRepo::new("acme", "bad");
            scan::store_cached_repo_data(
                &app.net.github.fetch_cache,
                &success,
                CachedRepoData {
                    runs:         Vec::new(),
                    meta:         Some(RepoMetaInfo {
                        stars:       7,
                        description: Some("ok".to_string()),
                    }),
                    github_total: 0,
                    pr_data:      ProjectPrData::Unfetched,
                },
            );
            scan::store_cached_repo_data(
                &app.net.github.fetch_cache,
                &failure,
                CachedRepoData {
                    runs:         Vec::new(),
                    meta:         None,
                    github_total: 0,
                    pr_data:      ProjectPrData::Unfetched,
                },
            );

            // Drive a confirmed-then-recovered cycle so the recovery hook
            // actually fires (NoTransition would short-circuit before the
            // refetch dispatch).
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::GitHub,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::GitHub,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceRecovered {
                service: ServiceKind::GitHub,
            });

            assert!(
                scan::load_cached_repo_data(&app.net.github.fetch_cache, &success).is_some(),
                "successful entry must stay cached so the recovery sweep doesn't refetch known-good data"
            );
            assert!(
                scan::load_cached_repo_data(&app.net.github.fetch_cache, &failure).is_none(),
                "meta.is_none() entry was a failed outage-time fetch — must be dropped on recovery"
            );
        }

        #[test]
        fn confirmed_then_recovered_shows_back_online_toast() {
            // Full sustained-outage path: confirmed unreachable surfaces a
            // toast, later recovery dismisses it and pushes a "back online"
            // toast. This is the user-visible flow we want for a real outage.
            let mut app = make_app(&[]);

            app.handle_bg_msg(BackgroundMsg::ServiceUnreachable {
                service: ServiceKind::CratesIo,
            });
            app.handle_bg_msg(BackgroundMsg::ServiceUnreachableConfirmed {
                service: ServiceKind::CratesIo,
            });
            let unreachable_id = app
                .net
                .crates_io
                .availability
                .toast_id()
                .expect("confirmed signal pushes the unreachable toast");
            let entries_after_confirm = app.framework.toasts.active().len();

            app.handle_bg_msg(BackgroundMsg::ServiceRecovered {
                service: ServiceKind::CratesIo,
            });
            assert!(
                !app.framework.toasts.is_alive(unreachable_id),
                "unreachable toast must be dismissed on recovery"
            );
            assert!(
                app.framework.toasts.active().len() > entries_after_confirm,
                "a fresh `back online` toast must be pushed"
            );
            assert!(
                app.net.crates_io.availability.toast_id().is_none(),
                "availability state cleared after recovery"
            );
        }
    }
    mod discovery_shimmer {
        use std::time::Duration;
        use std::time::Instant;

        use super::*;

        #[test]
        fn discovery_shimmer_is_not_registered_before_scan_completes() {
            let mut app = make_app(&[]);

            assert!(app.handle_project_discovered(make_project(Some("demo"), "~/rust/demo",)));
            assert!(app.scan.discovery_shimmers_mut().is_empty());
        }

        #[test]
        fn discovery_shimmer_registers_and_allows_multiple_concurrent_roots() {
            let mut app = make_app(&[]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(app.handle_project_discovered(make_project(Some("alpha"), "~/rust/alpha",)));
            assert!(app.handle_project_discovered(make_project(Some("beta"), "~/rust/beta",)));

            assert!(
                app.scan
                    .discovery_shimmers()
                    .contains_key(test_path("~/rust/alpha").as_path())
            );
            assert!(
                app.scan
                    .discovery_shimmers()
                    .contains_key(test_path("~/rust/beta").as_path())
            );
            assert_eq!(app.scan.discovery_shimmers_mut().len(), 2);
        }

        #[test]
        fn expanded_workspace_members_use_the_parent_shimmer_owner() {
            let member = make_member(Some("crate_a"), "~/rust/ws/crates/crate_a");
            let workspace = make_workspace_with_members(
                Some("ws"),
                "~/rust/ws",
                vec![inline_group(vec![member.clone()])],
            );
            let mut app = make_app(&[]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(app.handle_project_discovered(workspace));
            app.project_list.set_cursor(0);
            assert!(app.expand());
            app.ensure_visible_rows_cached();

            assert!(
                app.visible_rows().iter().any(|row| matches!(
                    row,
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    }
                )),
                "expanded workspace should render its member row during an active shimmer"
            );
            assert!(
                app.scan
                    .discovery_shimmers()
                    .contains_key(test_path("~/rust/ws").as_path()),
                "expanded member shimmer should be owned by the parent workspace session"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    member.path(),
                    "crate_a",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some(),
                "member should inherit the parent shimmer while expanded and active"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/ws").as_path(),
                    "ws",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::Root,
                )
                .is_some(),
                "parent workspace row should also shimmer while the discovered member is active"
            );

            assert!(app.project_list.collapse(false));
            app.ensure_visible_rows_cached();
            assert!(
                !app.visible_rows()
                    .iter()
                    .any(|row| matches!(row, VisibleRow::Member { .. })),
                "collapsed workspace should stop rendering member rows"
            );
        }

        #[test]
        fn newly_discovered_member_keeps_its_own_shimmer_owner() {
            let workspace =
                make_workspace_with_members(Some("ws"), "~/rust/ws", vec![inline_group(vec![])]);
            let mut app = make_app(&[workspace]);
            app.scan.state.phase = ScanPhase::Complete;

            let member_path = test_path("~/rust/ws/crates/crate_a");
            assert!(app.handle_project_discovered(make_project(
                Some("crate_a"),
                "~/rust/ws/crates/crate_a",
            )));

            assert!(
                app.scan
                    .discovery_shimmers_mut()
                    .contains_key(member_path.as_path()),
                "newly discovered member should keep its own shimmer session"
            );
        }

        #[test]
        fn discovered_workspace_member_shimmers_parent_and_self_but_not_siblings() {
            let workspace = make_workspace_with_members(
                Some("ws"),
                "~/rust/ws",
                vec![inline_group(vec![make_member(
                    Some("crate_existing"),
                    "~/rust/ws/crates/crate_existing",
                )])],
            );
            let mut app = make_app(&[workspace]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(
                app.handle_project_discovered(RootItem::Rust(RustProject::Package(
                    make_package_with_vendored(
                        Some("crate_new"),
                        "~/rust/ws/crates/crate_new",
                        vec![super::make_vendored(
                            Some("helper_new"),
                            "~/rust/ws/crates/crate_new/vendor/helper_new",
                        )],
                    )
                )))
            );

            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/ws").as_path(),
                    "ws",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::Root,
                )
                .is_some()
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/ws/crates/crate_new").as_path(),
                    "crate_new",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some()
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/ws/crates/crate_new/vendor/helper_new").as_path(),
                    "helper_new",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some(),
                "children of the discovered member should inherit the shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/ws/crates/crate_existing").as_path(),
                    "crate_existing",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_none(),
                "existing siblings should not inherit the discovered member shimmer"
            );
        }

        #[test]
        fn discovered_linked_worktree_shimmers_parent_and_subtree_but_not_existing_sibling() {
            let primary = make_workspace_raw_with_primary(
                Some("app"),
                "~/rust/app",
                Vec::new(),
                None,
                Some("/canonical/app"),
            );
            let linked = make_workspace_raw_with_primary(
                Some("app_feat"),
                "~/rust/app_feat",
                vec![inline_group(vec![make_member(
                    Some("crate_a"),
                    "~/rust/app_feat/crates/crate_a",
                )])],
                Some("app_feat"),
                Some("/canonical/app"),
            );
            let primary_item = RootItem::Rust(RustProject::Workspace(primary));
            let mut app = make_app(&[primary_item]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(app.handle_project_discovered(RootItem::Rust(RustProject::Workspace(linked))));

            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app").as_path(),
                    "app",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::Root,
                )
                .is_some(),
                "top-level parent row should shimmer for a discovered linked worktree"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat").as_path(),
                    "app_feat",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_some(),
                "discovered linked worktree row should shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat/crates/crate_a").as_path(),
                    "crate_a",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some(),
                "children of the discovered linked worktree should shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app").as_path(),
                    "app",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_none(),
                "existing sibling worktree entry should not shimmer"
            );
        }

        #[test]
        fn discovered_package_worktree_shimmers_parent_and_self_but_not_existing_sibling() {
            let root = make_package_worktrees_item(
                make_package_raw_with_primary(
                    Some("cargo-port"),
                    "~/rust/cargo-port",
                    None,
                    Some("/canonical/cargo-port"),
                ),
                vec![make_package_raw_with_primary(
                    Some("cargo-port"),
                    "~/rust/cargo-port-feat",
                    Some("cargo-port-feat"),
                    Some("/canonical/cargo-port"),
                )],
            );
            let mut app = make_app(&[root]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(
                app.handle_project_discovered(RootItem::Rust(RustProject::Package(
                    make_package_raw_with_primary(
                        Some("cargo-port"),
                        "~/rust/cargo-port-test",
                        Some("cargo-port-test"),
                        Some("/canonical/cargo-port"),
                    )
                )))
            );

            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/cargo-port").as_path(),
                    "cargo-port",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::Root,
                )
                .is_some(),
                "top-level parent row should shimmer for a discovered package worktree"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/cargo-port-test").as_path(),
                    "cargo-port-test",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_some(),
                "discovered package worktree row should shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/cargo-port-feat").as_path(),
                    "cargo-port-feat",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_none(),
                "existing sibling worktree should not shimmer"
            );
        }

        #[test]
        fn refreshed_stale_package_worktree_keeps_shimmer_after_regroup() {
            let root = make_package_worktrees_item(
                make_package_raw_with_primary(
                    Some("cargo-port"),
                    "~/rust/cargo-port",
                    None,
                    Some("/canonical/cargo-port"),
                ),
                vec![make_package_raw_with_primary(
                    Some("cargo-port"),
                    "~/rust/cargo-port-feat",
                    Some("cargo-port-feat"),
                    Some("/canonical/cargo-port"),
                )],
            );
            let mut app = make_app(&[root]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(app.handle_project_discovered(make_project(
                Some("cargo-port"),
                "~/rust/cargo-port-test",
            )));
            assert!(
                app.handle_project_refreshed(RootItem::Rust(RustProject::Package(
                    make_package_raw_with_primary(
                        Some("cargo-port"),
                        "~/rust/cargo-port-test",
                        Some("cargo-port-test"),
                        Some("/canonical/cargo-port"),
                    )
                )))
            );

            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/cargo-port-test").as_path(),
                    "cargo-port-test",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_some(),
                "refreshed package worktree should keep its discovery shimmer after regroup"
            );
        }

        #[test]
        fn discovered_worktree_member_shimmers_parent_self_and_children_but_not_siblings() {
            let root = make_workspace_worktrees_item(
                make_workspace_raw_with_primary(
                    Some("app"),
                    "~/rust/app",
                    vec![inline_group(vec![make_member(
                        Some("crate_root"),
                        "~/rust/app/crates/crate_root",
                    )])],
                    None,
                    Some("/canonical/app"),
                ),
                vec![make_workspace_raw_with_primary(
                    Some("app_feat"),
                    "~/rust/app_feat",
                    vec![inline_group(vec![make_member(
                        Some("crate_existing"),
                        "~/rust/app_feat/crates/crate_existing",
                    )])],
                    Some("app_feat"),
                    Some("/canonical/app"),
                )],
            );
            let mut app = make_app(&[root]);
            app.scan.state.phase = ScanPhase::Complete;

            assert!(
                app.handle_project_discovered(RootItem::Rust(RustProject::Package(
                    make_package_with_vendored(
                        Some("crate_new"),
                        "~/rust/app_feat/crates/crate_new",
                        vec![super::make_vendored(
                            Some("helper_new"),
                            "~/rust/app_feat/crates/crate_new/vendor/helper_new",
                        )],
                    )
                )))
            );

            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat").as_path(),
                    "app_feat",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_some(),
                "the containing worktree entry should shimmer as the discovered member's parent"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat/crates/crate_new").as_path(),
                    "crate_new",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some(),
                "the discovered member should shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat/crates/crate_new/vendor/helper_new").as_path(),
                    "helper_new",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_some(),
                "children of the discovered member should shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app_feat/crates/crate_existing").as_path(),
                    "crate_existing",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::PathOnly,
                )
                .is_none(),
                "existing siblings in the same worktree should not shimmer"
            );
            assert!(
                app.discovery_name_segments_for_path(
                    test_path("~/rust/app").as_path(),
                    "app",
                    Some(GitStatus::Clean),
                    DiscoveryRowKind::WorktreeEntry,
                )
                .is_none(),
                "peer worktree entries should not shimmer"
            );
        }

        #[test]
        fn prune_discovery_shimmers_removes_expired_entries() {
            let mut app = make_app(&[]);
            let path = test_path("~/rust/demo");
            app.scan.discovery_shimmers_mut().insert(
                crate::project::AbsolutePath::from(path.as_path()),
                DiscoveryShimmer::new(
                    Instant::now()
                        .checked_sub(Duration::from_secs(5))
                        .unwrap_or_else(Instant::now),
                    Duration::from_secs(1),
                ),
            );

            app.scan.prune_shimmers(Instant::now());

            assert!(
                !app.scan
                    .discovery_shimmers_mut()
                    .contains_key(path.as_path())
            );
        }
    }
    mod framework_keymap {
        //! Tests for the framework-keymap path.
        //!
        //! - Bar snapshots assert that `tui_pane::render_status_bar` produces the expected
        //!   pane-action labels when Package or Git is focused. They read `bar.pane_action` only —
        //!   the global and nav regions are covered separately by the `AppGlobalAction` snapshots
        //!   below.
        //! - The `state` tests pin the `Shortcuts::state` rules that gray out `Activate` when the
        //!   cursor sits on a row whose dispatch has no effect (Package's non-`CratesIo` rows;
        //!   Git's flat fields and any remote without a URL).

        use std::fs;
        use std::ops::Deref;
        use std::ops::DerefMut;
        use std::path::Path;
        use std::rc::Rc;

        use crossterm::event::Event;
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyModifiers;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::text::Span;
        use tempfile::TempDir;
        use toml::Table;
        use tui_pane::Action;
        use tui_pane::AppContext;
        use tui_pane::BarPalette;
        use tui_pane::FocusedPane;
        use tui_pane::FrameworkFocusId;
        use tui_pane::FrameworkOverlayId;
        use tui_pane::GlobalAction as FrameworkGlobalAction;
        use tui_pane::KeyBind;
        use tui_pane::Mode;
        use tui_pane::Pane;
        use tui_pane::ShortcutState;
        use tui_pane::Shortcuts;
        use tui_pane::Visibility;
        use tui_pane::render_status_bar;

        use super::App;
        use super::make_app;
        use crate::ci::CiRun;
        use crate::ci::CiStatus;
        use crate::ci::FetchStatus;
        use crate::config::CargoPortConfig;
        use crate::config::NavigationKeys;
        use crate::lint::LintRun;
        use crate::lint::LintRunStatus;
        use crate::project::HeadState;
        use crate::project::ProjectType;
        use crate::project::RootItem;
        use crate::project::Submodule;
        use crate::test_support;
        use crate::tui::app::CargoPortToastAction;
        use crate::tui::input;
        use crate::tui::integration::AppGlobalAction;
        use crate::tui::integration::AppPaneId;
        use crate::tui::integration::CiRunsPane;
        use crate::tui::integration::FinderPane;
        use crate::tui::integration::GitPane;
        use crate::tui::integration::NavAction;
        use crate::tui::integration::PackagePane;
        use crate::tui::integration::TargetsPane;
        use crate::tui::keymap;
        use crate::tui::keymap::CiRunsAction;
        use crate::tui::keymap::GitAction;
        use crate::tui::keymap::OutputAction;
        use crate::tui::keymap::PackageAction;
        use crate::tui::keymap::TargetsAction;
        use crate::tui::keymap_ui;
        use crate::tui::panes;
        use crate::tui::panes::CiData;
        use crate::tui::panes::CiEmptyState;
        use crate::tui::panes::GitData;
        use crate::tui::panes::LintsData;
        use crate::tui::panes::LintsProjectKind;
        use crate::tui::panes::PackageData;
        use crate::tui::panes::PackagePresence;
        use crate::tui::panes::PaneId;
        use crate::tui::panes::RemoteRow;
        use crate::tui::panes::TargetsData;
        use crate::tui::render;
        use crate::tui::settings::SettingOption;

        const TAB_WALK_STEPS: usize = 6;
        const SINGLE_RUN_COUNT: usize = 1;
        const GLOBAL_SHORTCUTS_TEST_WIDTH: u16 = 100;
        const GLOBAL_SHORTCUTS_TEST_HEIGHT: u16 = 40;

        fn focus_app_pane_in_framework(app: &mut App, id: AppPaneId) {
            app.set_focus(FocusedPane::App(id));
        }

        fn flatten(spans: &[Span<'static>]) -> String {
            let mut out = String::new();
            for span in spans {
                out.push_str(&span.content);
            }
            out
        }

        fn assert_contains_in_order(text: &str, labels: &[&str]) {
            let mut start = 0;
            for label in labels {
                let Some(offset) = text[start..].find(label) else {
                    panic!("{label:?} missing or out of order in {text:?}");
                };
                start += offset + label.len();
            }
        }

        struct KeymapFixture<Guard> {
            app:                        Option<App>,
            keymap_path_override_guard: Option<Guard>,
            temp_dir:                   Option<TempDir>,
        }

        impl<Guard> KeymapFixture<Guard> {
            fn app(&self) -> &App {
                self.app
                    .as_ref()
                    .expect("keymap fixture app should be live")
            }

            fn app_mut(&mut self) -> &mut App {
                self.app
                    .as_mut()
                    .expect("keymap fixture app should be live")
            }

            fn keymap_path(&self) -> &Path {
                self.app()
                    .keymap
                    .path()
                    .expect("keymap fixture should use an on-disk keymap path")
            }
        }

        impl<Guard> Deref for KeymapFixture<Guard> {
            type Target = App;

            fn deref(&self) -> &Self::Target { self.app() }
        }

        impl<Guard> DerefMut for KeymapFixture<Guard> {
            fn deref_mut(&mut self) -> &mut Self::Target { self.app_mut() }
        }

        impl<Guard> Drop for KeymapFixture<Guard> {
            fn drop(&mut self) {
                drop(self.app.take());
                drop(self.keymap_path_override_guard.take());
                drop(self.temp_dir.take());
            }
        }

        fn keymap_fixture_with_config(
            projects: &[RootItem],
            cargo_port_config: &CargoPortConfig,
            toml: &str,
        ) -> KeymapFixture<impl Sized + use<>> {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            fs::write(&toml_path, toml).expect("write keymap toml");
            let keymap_path_override_guard = keymap::override_keymap_path_for_test(toml_path);
            let app = super::make_app_with_config(projects, cargo_port_config);
            KeymapFixture {
                app:                        Some(app),
                keymap_path_override_guard: Some(keymap_path_override_guard),
                temp_dir:                   Some(temp_dir),
            }
        }

        fn make_app_with_keymap_toml(
            projects: &[RootItem],
            toml: &str,
        ) -> KeymapFixture<impl Sized + use<>> {
            keymap_fixture_with_config(projects, &CargoPortConfig::default(), toml)
        }

        fn make_app_with_config_and_keymap_toml(
            projects: &[RootItem],
            cargo_port_config: &CargoPortConfig,
            toml: &str,
        ) -> KeymapFixture<impl Sized + use<>> {
            keymap_fixture_with_config(projects, cargo_port_config, toml)
        }

        fn app_returned_from_keymap_helper() -> KeymapFixture<impl Sized + use<>> {
            let project = super::make_project(Some("demo"), "~/demo");
            make_app_with_keymap_toml(&[project], "[output]\ncancel = \"Esc\"\n")
        }

        #[test]
        fn helper_returned_keymap_fixture_reloads_from_app_path() {
            let mut app = app_returned_from_keymap_helper();
            let toml_path = app.keymap_path().to_path_buf();

            assert!(
                toml_path.exists(),
                "fixture-owned keymap file should exist after helper return",
            );

            fs::write(&toml_path, "[output]\ncancel = \"q\"\n").expect("rewrite keymap toml");
            app.maybe_reload_keymap_from_disk();

            assert_eq!(
                app.framework_keymap
                    .key_for_toml_key(AppPaneId::Output, OutputAction::Cancel.toml_key()),
                Some(tui_pane::KeySequence::from(KeyBind {
                    code: KeyCode::Char('q'),
                    mods: KeyModifiers::NONE,
                })),
            );
        }

        fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
            let event = Event::Key(KeyEvent::new(code, modifiers));
            input::handle_event(app, &event);
        }

        fn open_framework_overlay(app: &mut App, action: FrameworkGlobalAction) {
            let keymap = Rc::clone(&app.framework_keymap);
            keymap.dispatch_framework_global(action, app);
        }

        fn buffer_text_sized(app: &mut App, width: u16, height: u16) -> String {
            app.ensure_visible_rows_cached();
            app.ensure_detail_cached();
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| render::ui(frame, app))
                .expect("draw test frame");
            let area = terminal.size().expect("read test terminal size");
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    text.push_str(buffer[(x, y)].symbol());
                }
                text.push('\n');
            }
            text
        }

        fn make_app_with_git_tabbable() -> App {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.git.set_content(GitData {
                head: Some(HeadState::Branch("main".to_string())),
                ..GitData::default()
            });
            app
        }

        fn package_data_no_version() -> PackageData {
            PackageData {
                title:                    "Package".to_string(),
                name:                     "demo".to_string(),
                worktree_group_summary:   None,
                primary_section:          None,
                path:                     "~/demo".to_string(),
                version:                  Some("0.1.0".to_string()),
                description:              None,
                crates_io_rows:           Vec::new(),
                types:                    Some(vec![ProjectType::Library]),
                disk:                     Some(1_048_576),
                stats_rows:               Vec::new(),
                test_rows:                Vec::new(),
                package_presence:         PackagePresence::Present,
                edition:                  None,
                license:                  None,
                homepage:                 None,
                repository:               None,
                in_project_target:        None,
                in_project_non_target:    None,
                out_of_tree_target_bytes: None,
                lint_display:             crate::tui::panes::LintDisplay::default(),
                ci_display:               crate::tui::panes::CiDisplay::default(),
            }
        }

        #[test]
        fn focused_app_panes_render_expected_pane_action_labels() {
            type Setup = fn(&mut App);
            let cases: &[(AppPaneId, &[&str], Setup)] = &[
                (AppPaneId::Package, &["activate"], |app| {
                    app.panes.package.set_content(package_data_no_version());
                }),
                (AppPaneId::Git, &["activate"], |app| {
                    app.panes.git.set_content(GitData::default());
                }),
                // The targets shortcuts split by highlight zone: run/release
                // show on table rows (Kill hidden), Kill shows on Running rows
                // (run/release hidden — the anchor pid is `Some` exactly then).
                (AppPaneId::Targets, &["run", "release"], |app| {
                    app.panes.targets.set_content(targets_data_with_binary());
                }),
                (AppPaneId::Targets, &["kill"], |app| {
                    app.panes.targets.set_running_cursor_pid(Some(4242));
                }),
                (AppPaneId::Lints, &["open", "del history"], |_| {}),
                // No git branch on this fixture, so the all/branch toggle is
                // hidden — only the always-on CiRuns actions render.
                (
                    AppPaneId::CiRuns,
                    &["open", "fetch more", "del cache"],
                    |app| {
                        app.ci.set_content(ci_data_with_runs(2));
                        app.ci.viewport.set_pos(0);
                    },
                ),
                (AppPaneId::Finder, &["go to", "close"], |_| {}),
            ];

            for (pane, expected_labels, setup) in cases {
                let project = super::make_project(Some("demo"), "~/demo");
                let mut app = make_app(&[project]);
                setup(&mut app);
                focus_app_pane_in_framework(&mut app, *pane);

                let palette = BarPalette::default();
                let bar = render_status_bar(
                    &FocusedPane::App(*pane),
                    &app,
                    &app.framework_keymap,
                    app.framework(),
                    &palette,
                );
                let pane_action = flatten(&bar.pane_action);

                for label in *expected_labels {
                    assert!(
                        pane_action.contains(label),
                        "{pane:?} bar must show label {label:?} (got {pane_action:?})",
                    );
                }
            }
        }

        #[test]
        fn package_activate_state_disabled_when_no_crates_version() {
            // `package_fields_from_data` omits the CratesIo row when
            // `crates_version` is `None`, so no cursor position lands on a
            // row whose Activate dispatch does anything — the state must be
            // Disabled regardless of where the cursor sits.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            let data = package_data_no_version();
            app.panes.package.set_content(data);
            app.panes.package.viewport.set_pos(0);

            let pane = PackagePane;
            assert_eq!(
                pane.state(PackageAction::Activate, &app),
                ShortcutState::Disabled,
                "Activate must be Disabled with no crates.io rows — no actionable row exists",
            );
        }

        #[test]
        fn package_activate_state_enabled_on_crates_io_with_version() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            let mut data = package_data_no_version();
            data.crates_io_rows = vec![("version", "0.1.0".to_string())];
            let rows = panes::package_rows_from_data(&data);
            let crates_io_pos = rows
                .iter()
                .position(|row| matches!(row, panes::PackageRow::CratesIo(_)))
                .expect("crates.io row must appear for a Rust package with crates.io data");
            app.panes.package.set_content(data);
            app.panes.package.viewport.set_pos(crates_io_pos);

            let pane = PackagePane;
            assert_eq!(
                pane.state(PackageAction::Activate, &app),
                ShortcutState::Enabled,
                "Activate is Enabled on CratesIo when crates_version is known",
            );
        }

        fn git_remote_with_url(url: &str) -> RemoteRow {
            RemoteRow {
                name:            "origin".to_string(),
                icon:            "",
                display_url:     url.to_string(),
                branch:          "main".to_string(),
                tracked_ref:     String::new(),
                status:          String::new(),
                full_url:        Some(url.to_string()),
                push_annotation: None,
            }
        }

        #[test]
        fn git_activate_state_disabled_when_cursor_not_on_remote() {
            // Default GitData has only the two rate-limit flat fields and no
            // remotes — the cursor at position 0 lands on a flat field whose
            // Activate dispatch is a no-op, so the state must be Disabled.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.git.set_content(GitData::default());
            app.panes.git.viewport.set_pos(0);

            let pane = GitPane;
            assert_eq!(
                pane.state(GitAction::Activate, &app),
                ShortcutState::Disabled,
                "Activate must be Disabled on a flat field row — only Remote rows dispatch",
            );
        }

        #[test]
        fn git_activate_state_enabled_on_remote_with_url() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            let mut data = GitData::default();
            data.remotes
                .push(git_remote_with_url("https://github.com/natepiano/demo"));
            // Default GitData carries two flat rate-limit rows, so the first
            // remote row sits at index 2.
            let remote_pos = 2;
            app.panes.git.set_content(data);
            app.panes.git.viewport.set_pos(remote_pos);

            let pane = GitPane;
            assert_eq!(
                pane.state(GitAction::Activate, &app),
                ShortcutState::Enabled,
                "Activate is Enabled on a Remote row whose full_url is Some",
            );
        }

        fn ci_data_with_runs(count: usize) -> CiData {
            let runs = (0..count)
                .map(|i| CiRun {
                    run_id:          1 + i as u64,
                    created_at:      "2026-04-01T21:00:00-04:00".to_string(),
                    branch:          "main".to_string(),
                    url:             format!("https://example.com/run/{}", 1 + i),
                    ci_status:       CiStatus::Passed,
                    jobs:            Vec::new(),
                    wall_clock_secs: Some(17),
                    commit_title:    Some("commit".to_string()),
                    updated_at:      None,
                    fetched:         FetchStatus::Fetched,
                })
                .collect();
            CiData {
                runs,
                mode_label: None,
                current_branch: None,
                empty_state: CiEmptyState::NoRuns,
            }
        }

        fn lints_data_with_runs(count: usize) -> LintsData {
            let runs = (0..count)
                .map(|i| LintRun {
                    run_id:        format!("lint-{i}"),
                    started_at:    "2026-04-01T21:00:00-04:00".to_string(),
                    finished_at:   None,
                    duration_ms:   None,
                    status:        LintRunStatus::Passed,
                    commands:      Vec::new(),
                    archive_bytes: 0,
                })
                .collect();
            LintsData {
                runs,
                sizes: Vec::new(),
                owner_paths: Vec::new(),
                owner_of: Vec::new(),
                project_kind: LintsProjectKind::Rust,
            }
        }

        fn targets_data_with_binary() -> TargetsData {
            TargetsData {
                binaries: vec![crate::tui::panes::TargetEntry {
                    name:              "demo".to_string(),
                    display_name:      "demo".to_string(),
                    run_target_kind:   crate::tui::panes::RunTargetKind::Binary,
                    source:            crate::tui::panes::TargetSource::workspace_root(
                        "demo".into(),
                    ),
                    project_path:      crate::project::AbsolutePath::from("/tmp/demo"),
                    package_name:      "demo".to_string(),
                    src_path:          crate::project::AbsolutePath::from("/tmp/demo/src/main.rs"),
                    required_features: Vec::new(),
                }],
                examples: Vec::new(),
                benches:  Vec::new(),
            }
        }

        #[test]
        fn ci_runs_activate_visibility_hidden_at_eol() {
            // CiRuns `pane.visibility(Activate, ctx)` returns
            // `Visibility::Hidden` when the cursor is at or beyond the end of
            // the visible runs.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.ci.set_content(ci_data_with_runs(2));
            // Cursor at index == runs.len() — past the last run.
            app.ci.viewport.set_pos(2);

            let pane = CiRunsPane;
            assert_eq!(
                pane.visibility(CiRunsAction::Activate, &app),
                Visibility::Hidden,
                "Activate must be Hidden when cursor is past the visible runs",
            );
            assert_eq!(
                pane.visibility(CiRunsAction::FetchMore, &app),
                Visibility::Visible,
                "FetchMore stays Visible regardless of cursor position",
            );
        }

        #[test]
        fn ci_runs_activate_visibility_visible_on_run_row() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.ci.set_content(ci_data_with_runs(2));
            app.ci.viewport.set_pos(0);

            let pane = CiRunsPane;
            assert_eq!(
                pane.visibility(CiRunsAction::Activate, &app),
                Visibility::Visible,
                "Activate is Visible when cursor sits on a real run row",
            );
        }

        #[test]
        fn targets_kill_visibility_hidden_without_running_anchor() {
            // `running_cursor_pid` is `None` whenever the highlight is on a
            // table row (or no Running rows exist), so Kill drops from the bar.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.targets.set_content(targets_data_with_binary());

            let pane = TargetsPane;
            assert_eq!(
                pane.visibility(TargetsAction::Kill, &app),
                Visibility::Hidden,
                "Kill must be Hidden while the highlight is on a table row",
            );
            assert_eq!(
                pane.visibility(TargetsAction::Activate, &app),
                Visibility::Visible,
                "Activate is Visible while the highlight is on a table row",
            );
            assert_eq!(
                pane.visibility(TargetsAction::ReleaseBuild, &app),
                Visibility::Visible,
                "ReleaseBuild is Visible while the highlight is on a table row",
            );
        }

        #[test]
        fn targets_kill_visibility_visible_with_running_anchor() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.targets.set_running_cursor_pid(Some(4242));

            let pane = TargetsPane;
            assert_eq!(
                pane.visibility(TargetsAction::Kill, &app),
                Visibility::Visible,
                "Kill is Visible while the highlight sits on a Running row",
            );
        }

        #[test]
        fn targets_run_visibility_hidden_in_the_running_list() {
            // The run shortcuts belong to the targets table: a highlight past
            // the table's rows sits in the Running list, where only Kill
            // applies.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.targets.set_content(targets_data_with_binary());
            let table_len = targets_data_with_binary().target_count();
            app.panes.targets.viewport.set_len(table_len + 1);
            app.panes.targets.viewport.set_pos(table_len);

            let pane = TargetsPane;
            assert_eq!(
                pane.visibility(TargetsAction::Activate, &app),
                Visibility::Hidden,
                "Activate must be Hidden while the highlight is in the Running list",
            );
            assert_eq!(
                pane.visibility(TargetsAction::ReleaseBuild, &app),
                Visibility::Hidden,
                "ReleaseBuild must be Hidden while the highlight is in the Running list",
            );
        }

        #[test]
        fn focused_project_list_bar_renders_pane_action_and_nav_slots() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            focus_app_pane_in_framework(&mut app, AppPaneId::ProjectList);

            let palette = BarPalette::default();
            let bar = render_status_bar(
                &FocusedPane::App(AppPaneId::ProjectList),
                &app,
                &app.framework_keymap,
                app.framework(),
                &palette,
            );
            let pane_action = flatten(&bar.pane_action);
            let nav = flatten(&bar.nav);

            // ProjectList keeps row expand/collapse keys active, but does not
            // spend bar space advertising them. Only the all pair lands in
            // the Nav region; no pane-local actions remain after Clean moved
            // to the global scope.
            assert!(
                pane_action.is_empty(),
                "ProjectList has no pane-local actions (got {pane_action:?})",
            );
            assert!(
                !nav.contains(" expand"),
                "ProjectList nav region must not show row expand help (got {nav:?})",
            );
            assert!(
                nav.contains("=/- all"),
                "ProjectList nav region must include the paired all row (got {nav:?})",
            );
            assert_contains_in_order(&nav, &["nav", "all"]);
            assert!(
                !nav.contains(" home") && !nav.contains(" end"),
                "ProjectList nav region must stay compact and omit Home/End rows (got {nav:?})",
            );
        }

        // ── Output (Mode::Navigable) ──────────────────────────────────────

        #[test]
        fn focused_output_bar_renders_select_all_and_close_labels() {
            // OutputPane is Navigable: the Nav region shows (the cursor scrolls
            // the buffer) alongside PaneAction, which carries the
            // OutputAction::SelectAll label "select all" and the Cancel label
            // "close". The vim visual-line toggle (`V`) is a built-in, not a
            // rebindable action, so it has no bar slot.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            focus_app_pane_in_framework(&mut app, AppPaneId::Output);

            let palette = BarPalette::default();
            let bar = render_status_bar(
                &FocusedPane::App(AppPaneId::Output),
                &app,
                &app.framework_keymap,
                app.framework(),
                &palette,
            );
            let pane_action = flatten(&bar.pane_action);
            let nav = flatten(&bar.nav);

            assert!(
                pane_action.contains("close"),
                "Output bar must show the Cancel label \"close\" (got {pane_action:?})",
            );
            assert!(
                pane_action.contains("select all"),
                "Output bar must show the SelectAll label \"select all\" (got {pane_action:?})",
            );
            assert!(
                !nav.is_empty(),
                "Navigable Output must surface the Nav region (got {nav:?})",
            );
        }

        #[test]
        fn output_cancel_label_tracks_state() {
            fn output_pane_action(app: &App) -> String {
                let palette = BarPalette::default();
                let bar = render_status_bar(
                    &FocusedPane::App(AppPaneId::Output),
                    app,
                    &app.framework_keymap,
                    app.framework(),
                    &palette,
                );
                flatten(&bar.pane_action)
            }

            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            // Stream output and render so the output pane is laid out and its
            // action bar populates, then focus it.
            app.inflight
                .set_example_output(vec!["line one".to_string()]);
            let _ = buffer_text_sized(&mut app, 120, 40);
            focus_app_pane_in_framework(&mut app, AppPaneId::Output);

            // Idle (no run): Esc closes the pane.
            let idle = output_pane_action(&app);
            assert!(
                idle.contains("close"),
                "idle cancel label is close (got {idle:?})"
            );

            // A run is streaming: Esc stops it.
            app.inflight.set_example_running(Some("demo".to_string()));
            let running = output_pane_action(&app);
            assert!(
                running.contains("stop") && !running.contains("close"),
                "running cancel label is stop (got {running:?})",
            );

            app.inflight.set_example_running(None);
            let idle_again = output_pane_action(&app);
            assert!(
                idle_again.contains("close"),
                "with no run the cancel label returns to close (got {idle_again:?})",
            );

            // A visual selection is active: Esc collapses it, so the label reads
            // "done" — taking priority over stop/close (it is the first thing Esc
            // does). The bar tracks the title's "(y copy · Esc done)".
            let live = app.inflight.example_output().to_vec();
            app.panes.output.toggle_visual(&live);
            assert!(app.panes.output.selection().is_visual());
            let selecting = output_pane_action(&app);
            assert!(
                selecting.contains("done") && !selecting.contains("close"),
                "with a visual selection the cancel label is done (got {selecting:?})",
            );
        }

        // ── Finder (Mode::TextInput when open) ────────────────────────────

        #[test]
        fn finder_pane_mode_navigable_when_closed() {
            let project = super::make_project(Some("demo"), "~/demo");
            let app = make_app(&[project]);
            let mode_fn = <FinderPane as Pane<App>>::mode();
            assert!(
                matches!(mode_fn(&app), Mode::Navigable),
                "Finder mode must be Navigable when overlay is closed",
            );
        }

        #[test]
        fn finder_text_input_inserts_char_into_query() {
            // When Finder is open, a typed letter goes through the framework's
            // TextInput handler and into the search query — vim mode is bypassed
            // by Mode::TextInput. We exercise the handler directly via the `fn`
            // pointer carried inside `Mode::TextInput(...)`.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.overlays.open_finder();

            let mode = <FinderPane as Pane<App>>::mode()(&app);
            let Mode::TextInput(handler) = mode else {
                panic!("expected Mode::TextInput when finder is open");
            };
            handler(KeyBind::from('k'), &mut app);

            assert_eq!(
                app.project_list.finder.query, "k",
                "TextInput handler must insert the typed character into the query",
            );
        }

        #[test]
        fn focused_finder_open_bar_suppresses_all_regions() {
            // Open Finder → Mode::TextInput suppresses every bar region.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.overlays.open_finder();
            focus_app_pane_in_framework(&mut app, AppPaneId::Finder);

            let palette = BarPalette::default();
            let bar = render_status_bar(
                &FocusedPane::App(AppPaneId::Finder),
                &app,
                &app.framework_keymap,
                app.framework(),
                &palette,
            );

            assert!(
                flatten(&bar.nav).is_empty(),
                "Mode::TextInput must suppress Nav (got {:?})",
                flatten(&bar.nav),
            );
            assert!(
                flatten(&bar.pane_action).is_empty(),
                "Mode::TextInput must suppress PaneAction (got {:?})",
                flatten(&bar.pane_action),
            );
            assert!(
                flatten(&bar.global).is_empty(),
                "Mode::TextInput must suppress Global (got {:?})",
                flatten(&bar.global),
            );
            let cargo_port_right = render::cargo_port_right_text_for_test(&app, &bar.global);
            assert!(
                cargo_port_right.is_empty(),
                "cargo-port global override must preserve TextInput global suppression (got {cargo_port_right:?})",
            );
        }

        // ── AppGlobalAction four-variant bar snapshots ────────────────────

        #[test]
        fn focused_package_bar_renders_every_app_global() {
            // Walks `AppGlobalAction::ALL` so adding a variant automatically
            // extends the assertion. Previously hardcoded `{find, editor,
            // terminal, rescan}` and silently stayed green after `Clean` was
            // added but missed from the status-line strip — that is the bug
            // class this test now guards against, paired with the const checks
            // in `tui::render`.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.package.set_content(package_data_no_version());
            focus_app_pane_in_framework(&mut app, AppPaneId::Package);

            let palette = BarPalette::default();
            let bar = render_status_bar(
                &FocusedPane::App(AppPaneId::Package),
                &app,
                &app.framework_keymap,
                app.framework(),
                &palette,
            );
            let global = flatten(&bar.global);

            for variant in AppGlobalAction::ALL {
                let label = variant.bar_label();
                assert!(
                    global.contains(label),
                    "Global region must include AppGlobalAction::{variant:?} \
                     label {label:?} (got {global:?})",
                );
            }
        }

        #[test]
        fn focused_package_status_line_collapses_globals_to_shortcuts_help() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.package.set_content(package_data_no_version());
            focus_app_pane_in_framework(&mut app, AppPaneId::Package);

            let global = render::cargo_port_global_text_for_test(&app);

            assert_contains_in_order(&global, &["?", "shortcuts"]);
            assert!(
                !global.contains("finder")
                    && !global.contains("editor")
                    && !global.contains("quit"),
                "normal app-pane global strip should advertise only the shortcut viewer (got {global:?})",
            );
        }

        // ── Base-pane navigation routed through framework keymap ──────────

        /// `Ctrl-f` / `Ctrl-b` page the project list when vim mode is on. They
        /// are vim-only motions (not keymappable), so the test enables
        /// `ArrowsAndVim`; with vim off these keys do nothing. Validates that
        /// `handle_normal_key` consults the framework keymap's navigation scope
        /// after the legacy pane-scope match.
        #[test]
        fn ctrl_b_and_ctrl_f_page_the_project_list() {
            let projects: Vec<_> = (0..40)
                .map(|i| super::make_project(Some("p"), &format!("~/p{i}")))
                .collect();
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let mut app = make_app_with_config_and_keymap_toml(&projects, &cargo_port_config, "");
            let _ = buffer_text_sized(&mut app, 120, 30);
            assert_eq!(app.project_list.cursor(), 0);

            press(&mut app, KeyCode::Char('f'), KeyModifiers::CONTROL);
            let after_ctrl_f = app.project_list.cursor();
            assert!(after_ctrl_f > 0, "Ctrl-f paged down (got {after_ctrl_f})");

            press(&mut app, KeyCode::Char('b'), KeyModifiers::CONTROL);
            assert!(
                app.project_list.cursor() < after_ctrl_f,
                "Ctrl-b paged up from {after_ctrl_f}",
            );
        }

        #[test]
        fn navigation_action_rebound_to_j_moves_cursor_down() {
            let projects = vec![
                super::make_project(Some("alpha"), "~/alpha"),
                super::make_project(Some("beta"), "~/beta"),
            ];
            let mut app = make_app_with_keymap_toml(&projects, "[navigation]\ndown = \"j\"\n");
            let baseline = app.project_list.cursor();

            let event = Event::Key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
            input::handle_event(&mut app, &event);

            assert_eq!(
                app.project_list.cursor(),
                baseline + 1,
                "cursor must advance after `'j'` resolves to NavAction::Down",
            );
        }

        /// A stale on-disk keymap with `home = ""` must NOT unbind Home — the
        /// framework owns the navigation defaults and an empty value keeps the
        /// compiled default. This is the original live bug: an empty entry left
        /// over from an older keymap silently disabled Home/End at startup.
        #[test]
        fn empty_navigation_entry_keeps_the_compiled_default() {
            let projects = vec![
                super::make_project(Some("alpha"), "~/alpha"),
                super::make_project(Some("beta"), "~/beta"),
                super::make_project(Some("gamma"), "~/gamma"),
            ];
            let mut app = make_app_with_keymap_toml(&projects, "[navigation]\nhome = \"\"\n");

            // Move down so Home has somewhere to return from.
            for _ in 0..2 {
                input::handle_event(
                    &mut app,
                    &Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
                );
            }
            assert!(app.project_list.cursor() > 0, "cursor moved down");

            input::handle_event(
                &mut app,
                &Event::Key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            );

            assert_eq!(
                app.project_list.cursor(),
                0,
                "Home stays bound to its compiled default despite the empty TOML entry",
            );
        }

        /// Reloading exactly what the TOML writer emits — named page keys and
        /// empty half-page entries — with vim mode on must layer the vim Ctrl
        /// motions back on without tripping the build-time cross-action
        /// collision check. The page keys keep their named default; `Ctrl-b/f`
        /// and `Ctrl-u/d` arrive only as vim extras; empty half-page entries
        /// keep the (empty) compiled default rather than erroring.
        #[test]
        fn generated_navigation_defaults_round_trip_without_collision() {
            let projects = vec![super::make_project(Some("alpha"), "~/alpha")];
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let app = make_app_with_config_and_keymap_toml(
                &projects,
                &cargo_port_config,
                "[navigation]\n\
                 page_up = \"pageup\"\n\
                 page_down = \"pagedown\"\n\
                 half_page_up = \"\"\n\
                 half_page_down = \"\"\n",
            );

            let nav = app
                .framework_keymap
                .navigation()
                .expect("navigation scope is registered");

            assert_eq!(
                nav.action_for(&KeyBind::from(KeyCode::PageUp)),
                Some(NavAction::PageUp),
            );
            assert_eq!(nav.action_for(&KeyBind::ctrl('b')), Some(NavAction::PageUp));
            assert_eq!(
                nav.action_for(&KeyBind::ctrl('f')),
                Some(NavAction::PageDown)
            );
            assert_eq!(
                nav.action_for(&KeyBind::ctrl('u')),
                Some(NavAction::HalfPageUp)
            );
            assert_eq!(
                nav.action_for(&KeyBind::ctrl('d')),
                Some(NavAction::HalfPageDown),
            );
        }

        /// Vim navigation keys drive the output pane through the shared viewport
        /// navigation — `k` scrolls up off the tail (freezing the view), `j` and
        /// `G` return to the tail (resuming follow) — with no output-specific
        /// motion code.
        #[test]
        fn output_pane_navigates_with_vim_keys() {
            let projects = vec![super::make_project(Some("alpha"), "~/alpha")];
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let mut app = make_app_with_config_and_keymap_toml(&projects, &cargo_port_config, "");
            app.set_example_output((0..30).map(|i| format!("line {i}")).collect());
            // Render once so the viewport learns its length and visible rows.
            let _ = buffer_text_sized(&mut app, 120, 20);

            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert!(
                app.panes.output.is_following(),
                "the view opens following the streaming tail",
            );

            press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
            let _ = buffer_text_sized(&mut app, 120, 20);
            assert!(
                !app.panes.output.is_following(),
                "`k` scrolls up off the tail and freezes the view",
            );

            press(&mut app, KeyCode::Char('j'), KeyModifiers::NONE);
            let _ = buffer_text_sized(&mut app, 120, 20);
            assert!(
                app.panes.output.is_following(),
                "`j` back at the tail resumes following",
            );

            // Scroll up again, then `G` jumps to the tail and follows.
            press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
            press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);
            let _ = buffer_text_sized(&mut app, 120, 20);
            assert!(!app.panes.output.is_following());
            press(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
            let _ = buffer_text_sized(&mut app, 120, 20);
            assert!(
                app.panes.output.is_following(),
                "`G` jumps to the tail and resumes following",
            );
        }

        #[test]
        fn generated_home_end_entries_do_not_disable_vim_home_end_navigation() {
            let projects = vec![
                super::make_project(Some("alpha"), "~/alpha"),
                super::make_project(Some("beta"), "~/beta"),
                super::make_project(Some("gamma"), "~/gamma"),
            ];
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let mut app = make_app_with_config_and_keymap_toml(
                &projects,
                &cargo_port_config,
                "[navigation]\nhome = \"home\"\nend = \"end\"\n",
            );

            app.project_list.set_cursor(2);
            press(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
            assert_eq!(app.project_list.cursor(), 2);
            press(&mut app, KeyCode::Char('g'), KeyModifiers::NONE);
            assert_eq!(app.project_list.cursor(), 0);

            press(&mut app, KeyCode::Char('G'), KeyModifiers::NONE);
            assert_eq!(app.project_list.cursor(), 2);
        }

        /// Rebinding `ProjectListAction::ExpandRow` to `Tab` (with
        /// `GlobalAction::NextPane` rebound away) expands the current row.
        /// Validates that the legacy pane-scope match in `handle_normal_key`
        /// drives `ExpandRow` through its match arm.
        #[test]
        fn project_list_action_expand_row_rebound_to_tab_expands() {
            let tmp = tempfile::tempdir().expect("tempdir");
            let root_dir = tmp.path().join("repo");
            let sub_dir = root_dir.join("submod");
            fs::create_dir_all(&sub_dir).expect("create_dir_all");
            let root_path = root_dir.to_string_lossy().to_string();
            let sub_path = sub_dir.to_string_lossy().to_string();

            let project = super::make_project(Some("repo"), &root_path);
            let mut app = make_app_with_keymap_toml(
                &[project],
                "[global]\nnext_pane = \"F12\"\n[project_list]\nexpand_row = \"Tab\"\n",
            );

            let root_info = app
                .project_list
                .at_path_mut(Path::new(&root_path))
                .expect("root info");
            root_info.submodules.push(Submodule {
                name:          "submod".to_string(),
                path:          crate::project::AbsolutePath::from(sub_path),
                relative_path: "submod".to_string(),
                url:           None,
                branch:        None,
                commit:        None,
                project_info:  crate::project::ProjectInfo::default(),
                git_repo:      None,
            });
            app.ensure_visible_rows_cached();
            app.project_list.set_cursor(0);
            let baseline_rows = app.project_list.row_count();

            let event = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
            input::handle_event(&mut app, &event);
            app.ensure_visible_rows_cached();

            assert!(
                app.project_list.row_count() > baseline_rows,
                "expanding the parent must reveal additional rows (was {baseline_rows}, now {})",
                app.project_list.row_count(),
            );
        }

        // ── Output structural cancel uses framework keymap ────────────────

        fn assert_output_cancel_binding(
            keymap_toml: &str,
            key: KeyCode,
            starting_focus: Option<PaneId>,
            expected_focus: Option<PaneId>,
        ) {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(&[project], keymap_toml);
            if let Some(focus) = starting_focus {
                app.set_focus_to_pane(focus);
            }
            let focus_before = app.focused_pane_id();
            app.inflight.example_output_mut().push("line".to_string());

            let event = Event::Key(KeyEvent::new(key, KeyModifiers::NONE));
            input::handle_event(&mut app, &event);

            assert!(app.inflight.example_output().is_empty());
            assert_eq!(
                app.focused_pane_id(),
                expected_focus.unwrap_or(focus_before),
                "unexpected focus after structural output cancel",
            );
        }

        #[test]
        fn output_cancel_bindings_clear_output_and_handle_focus() {
            for (toml, key, starting_focus, expected_focus) in [
                ("[output]\ncancel = \"q\"\n", KeyCode::Char('q'), None, None),
                (
                    "[output]\ncancel = \"q\"\n",
                    KeyCode::Char('q'),
                    Some(PaneId::Output),
                    Some(PaneId::Targets),
                ),
                (
                    "[output]\ncancel = [\"Esc\", \"q\"]\n",
                    KeyCode::Esc,
                    None,
                    None,
                ),
                (
                    "[output]\ncancel = [\"Esc\", \"q\"]\n",
                    KeyCode::Char('q'),
                    None,
                    None,
                ),
            ] {
                assert_output_cancel_binding(toml, key, starting_focus, expected_focus);
            }
        }

        #[test]
        fn output_stop_consumes_esc_even_if_global_quit_is_esc() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(&[project], "[global]\nquit = \"Esc\"\n");
            app.inflight
                .set_example_output(vec!["line one".to_string()]);
            app.inflight.set_example_running(Some("demo".to_string()));

            press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

            assert!(
                !app.framework.quit_requested(),
                "stopping output must not also request cargo-port quit",
            );
            assert!(app.inflight.example_running().is_none());
            assert_eq!(
                app.inflight.example_output().last().map(String::as_str),
                Some("── killed ──"),
            );
        }

        // ── Keymap UI backed by framework keymap ──────────────────────────

        #[test]
        fn framework_keymap_template_matches_golden_file() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
            let project = super::make_project(Some("demo"), "~/demo");
            let app = make_app(&[project]);
            let generated = keymap_ui::current_keymap_toml(&app);
            let expected = include_str!("../../../tests/assets/default-keymap.toml");

            assert_eq!(
                test_support::normalize_line_endings(&generated),
                test_support::normalize_line_endings(expected),
            );
        }

        #[test]
        fn keymap_template_omits_generated_vim_bindings() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let app = make_app_with_config_and_keymap_toml(&[project], &cargo_port_config, "");

            let generated = keymap_ui::current_keymap_toml(&app);

            assert!(generated.contains("down           = \"down\""));
            assert!(generated.contains("left           = \"left\""));
            assert!(generated.contains("collapse_row = \"left\""));
            assert!(generated.contains("expand_row   = \"right\""));
            assert!(!generated.contains("[\"down\", \"j\"]"));
            assert!(!generated.contains("[\"left\", \"h\"]"));
            assert!(!generated.contains("[\"shift-left\", \"h\"]"));
            assert!(!generated.contains("[\"shift-right\", \"l\"]"));
        }

        #[test]
        fn startup_warns_for_ignored_reserved_vim_keymap_bindings() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let app = make_app_with_config_and_keymap_toml(
                &[project],
                &cargo_port_config,
                "[project_list]\ncollapse_row = [\"shift-left\", \"h\"]\nexpand_row = [\"shift-right\", \"l\"]\n",
            );

            let warnings = app
                .framework
                .toasts
                .active_now()
                .into_iter()
                .filter(|toast| toast.title() == "Keymap warnings")
                .collect::<Vec<_>>();

            assert_eq!(warnings.len(), 1);
            assert!(warnings[0].body().contains("project_list.expand_row"));
            assert!(warnings[0].body().contains("project_list.collapse_row"));
            assert!(!warnings[0].body().contains("using defaults"));
        }

        #[test]
        fn keymap_ui_save_preserves_framework_owned_scopes() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            fs::write(
                &toml_path,
                "[output]\ncancel = \"q\"\n\
                 [finder]\nactivate = \"Tab\"\n\
                 [overlay]\nstart_edit = \"F2\"\n",
            )
            .expect("write keymap toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            keymap_ui::save_current_keymap_to_disk(&mut app);
            let saved = fs::read_to_string(&toml_path).expect("read keymap toml");

            assert!(saved.contains("[finder]"));
            assert!(saved.contains("activate = \"tab\""));
            assert!(saved.contains("[output]"));
            // The output scope now aligns its keys (select_linewise is wider), so
            // match the preserved cancel binding without depending on padding.
            assert!(
                saved
                    .lines()
                    .any(|line| line.starts_with("cancel") && line.contains("\"q\"")),
                "custom output cancel binding must be preserved (got {saved:?})",
            );
            assert!(saved.contains("[overlay]"));
            assert!(saved.contains("start_edit = \"f2\""));
        }

        #[test]
        fn external_keymap_reload_updates_framework_owned_scope() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            fs::write(&toml_path, "[output]\ncancel = \"Esc\"\n").expect("write keymap toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            fs::write(
                &toml_path,
                "[output]\ncancel = \"q\"\n[finder]\nactivate = \"Tab\"\n",
            )
            .expect("rewrite keymap toml");
            app.maybe_reload_keymap_from_disk();

            assert_eq!(
                app.framework_keymap
                    .key_for_toml_key(AppPaneId::Output, OutputAction::Cancel.toml_key()),
                Some(tui_pane::KeySequence::from(KeyBind {
                    code: KeyCode::Char('q'),
                    mods: KeyModifiers::NONE,
                })),
            );
        }

        #[test]
        fn external_keymap_reload_missing_actions_does_not_rewrite_file() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            let keymap_path_guard = keymap::override_keymap_path_for_test(toml_path.clone());
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            keymap_ui::save_current_keymap_to_disk(&mut app);
            let edited = "[output]\n# cancel = \"Esc\"\n";
            fs::write(&toml_path, edited).expect("rewrite keymap toml");

            app.maybe_reload_keymap_from_disk();

            let saved = fs::read_to_string(&toml_path).expect("read keymap toml");
            assert_eq!(saved, edited);
            assert!(
                app.framework
                    .toasts
                    .active_now()
                    .iter()
                    .any(|toast| toast.title() == "Keymap warnings"),
                "missing entries should warn without rewriting the user's in-progress edit"
            );
            drop(keymap_path_guard);
        }

        #[test]
        fn legacy_project_list_removed_actions_migrate_before_framework_load() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            fs::write(
                &toml_path,
                "[project_list]\nopen_editor = \"E\"\nrescan = \"Ctrl+r\"\n",
            )
            .expect("write keymap toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path.clone());
            let project = super::make_project(Some("demo"), "~/demo");
            let app = make_app(&[project]);

            let globals = app
                .framework_keymap
                .globals::<AppGlobalAction>()
                .expect("app globals registered");
            assert_eq!(
                globals.action_for(&KeyBind::from('E')),
                Some(AppGlobalAction::OpenEditor),
            );
            assert_eq!(
                globals.action_for(&KeyBind::ctrl('r')),
                Some(AppGlobalAction::Rescan),
            );

            let saved = fs::read_to_string(&toml_path).expect("read migrated keymap toml");
            let table: Table = saved.parse().expect("parse migrated keymap toml");
            let project_list = table
                .get("project_list")
                .and_then(toml::Value::as_table)
                .expect("project_list table");
            assert!(!project_list.contains_key("open_editor"));
            assert!(!project_list.contains_key("rescan"));
            let global = table
                .get("global")
                .and_then(toml::Value::as_table)
                .expect("global table");
            assert_eq!(
                global.get("open_editor").and_then(toml::Value::as_str),
                Some("E"),
            );
            assert_eq!(
                global.get("rescan").and_then(toml::Value::as_str),
                Some("ctrl-r"),
            );
        }

        #[test]
        fn legacy_project_list_removed_action_does_not_override_framework_global() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            fs::write(
                &toml_path,
                "[global]\nopen_editor = \"E\"\n[project_list]\nopen_editor = \"Enter\"\n",
            )
            .expect("write keymap toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
            let project = super::make_project(Some("demo"), "~/demo");
            let app = make_app(&[project]);

            let globals = app
                .framework_keymap
                .globals::<AppGlobalAction>()
                .expect("app globals registered");
            assert_eq!(
                globals.action_for(&KeyBind::from('E')),
                Some(AppGlobalAction::OpenEditor),
            );
            assert_ne!(
                globals.action_for(&KeyBind::from(KeyCode::Enter)),
                Some(AppGlobalAction::OpenEditor),
            );
        }

        #[test]
        fn keymap_popup_keeps_legacy_global_shortcuts_layout() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

            let text = buffer_text_sized(&mut app, 120, 80);

            assert_contains_in_order(
                &text,
                &[
                    "Global Navigation:",
                    "Next pane",
                    "Global Shortcuts:",
                    "Dismiss overlay / output",
                    "Open finder",
                    "Open keymap viewer",
                    "Show global shortcuts",
                    "Project List:",
                ],
            );
            assert!(
                !text.contains("App Global Shortcuts:"),
                "app-owned globals must stay merged into the legacy Global Shortcuts section",
            );
        }

        #[test]
        fn global_shortcuts_overlay_opens_with_question_mark_and_esc_closes() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            press(&mut app, KeyCode::Char('?'), KeyModifiers::NONE);

            assert_eq!(
                app.framework.overlay(),
                Some(tui_pane::FrameworkOverlayId::GlobalShortcuts)
            );

            press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

            assert_eq!(app.framework.overlay(), None);
        }

        #[test]
        fn global_shortcuts_overlay_renders_all_global_shortcuts() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            open_framework_overlay(&mut app, FrameworkGlobalAction::OpenGlobalShortcuts);

            let text = buffer_text_sized(
                &mut app,
                GLOBAL_SHORTCUTS_TEST_WIDTH,
                GLOBAL_SHORTCUTS_TEST_HEIGHT,
            );

            assert_contains_in_order(
                &text,
                &[
                    "Global Shortcuts",
                    "Global Navigation:",
                    "Next pane",
                    "Global Shortcuts:",
                    "Open finder",
                    "Quit",
                    "Show global shortcuts",
                ],
            );
        }

        #[test]
        fn keymap_popup_renders_framework_overflow_affordance() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let toml_path = temp_dir.path().join("keymap.toml");
            let _keymap_path = keymap::override_keymap_path_for_test(toml_path);
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

            let text = buffer_text_sized(&mut app, 120, 18);

            assert!(text.contains("Keymap"));
            assert!(
                text.contains("1 of"),
                "keymap overlay should render the framework-owned overflow marker"
            );
        }

        // ── Framework-owned live tab cycle ────────────────────────────────

        #[test]
        fn tab_from_package_lands_on_git_when_lang_is_unavailable() {
            let mut app = make_app_with_git_tabbable();
            app.set_focus(FocusedPane::App(AppPaneId::Package));

            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
        }

        #[test]
        fn repeated_tab_never_lands_on_unavailable_lang() {
            let mut app = make_app_with_git_tabbable();
            app.set_focus(FocusedPane::App(AppPaneId::Package));

            for step in 0..TAB_WALK_STEPS {
                press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
                assert_ne!(app.focused_pane_id(), PaneId::Lang, "step {step}");
            }
        }

        #[test]
        fn shift_tab_skips_unavailable_panes_in_reverse() {
            let mut app = make_app_with_git_tabbable();
            app.set_focus(FocusedPane::App(AppPaneId::Cpu));

            press(&mut app, KeyCode::Tab, KeyModifiers::SHIFT);

            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
        }

        #[test]
        fn output_active_excludes_diagnostics_and_reaches_output() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.targets.set_content(targets_data_with_binary());
            app.lint.set_content(lints_data_with_runs(SINGLE_RUN_COUNT));
            app.ci.set_content(ci_data_with_runs(SINGLE_RUN_COUNT));
            app.inflight.example_output_mut().push("line".to_string());
            app.set_focus(FocusedPane::App(AppPaneId::Targets));

            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert_eq!(
                app.framework().focused(),
                &FocusedPane::App(AppPaneId::Output),
            );
        }

        #[test]
        fn rebound_next_pane_uses_framework_filtered_tab_cycle() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(&[project], "[global]\nnext_pane = \"F8\"\n");
            app.panes.git.set_content(GitData {
                head: Some(HeadState::Branch("main".to_string())),
                ..GitData::default()
            });
            app.set_focus(FocusedPane::App(AppPaneId::Package));

            press(&mut app, KeyCode::F(8), KeyModifiers::NONE);

            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.framework().focused(), &FocusedPane::App(AppPaneId::Git),);
        }

        #[test]
        fn settings_text_input_esc_wins_over_output_cancel_preflight() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            open_framework_overlay(&mut app, FrameworkGlobalAction::OpenSettings);
            app.framework
                .settings_pane
                .viewport_mut()
                .set_pos(SettingOption::CiRunCount as usize);
            press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
            app.inflight.example_output_mut().push("line".to_string());

            press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

            assert!(
                !app.inflight.example_output().is_empty(),
                "settings edit cancel must not clear example output",
            );
            assert!(
                !app.framework.settings_pane.is_editing(),
                "Esc must still leave settings edit mode",
            );
        }

        #[test]
        fn framework_overlay_esc_wins_over_output_cancel_preflight() {
            let overlays = [
                (
                    FrameworkGlobalAction::OpenSettings,
                    FrameworkOverlayId::Settings,
                ),
                (
                    FrameworkGlobalAction::OpenKeymap,
                    FrameworkOverlayId::Keymap,
                ),
                (
                    FrameworkGlobalAction::OpenGlobalShortcuts,
                    FrameworkOverlayId::GlobalShortcuts,
                ),
            ];

            for (action, overlay) in overlays {
                let project = super::make_project(Some("demo"), "~/demo");
                let mut app = make_app(&[project]);
                app.inflight.example_output_mut().push("line".to_string());
                open_framework_overlay(&mut app, action);
                assert_eq!(app.framework.overlay(), Some(overlay));

                press(&mut app, KeyCode::Esc, KeyModifiers::NONE);

                assert_eq!(app.framework.overlay(), None);
                assert_eq!(app.inflight.example_output().len(), 1);
                assert_eq!(
                    app.inflight.example_output().first().map(String::as_str),
                    Some("line"),
                );
            }
        }

        // ── Overlay input/render ownership ────────────────────────────────

        #[test]
        fn finder_cancel_rebind_closes_finder_through_production_input() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(&[project], "[finder]\ncancel = \"q\"\n");
            input::open_finder(&mut app);

            press(&mut app, KeyCode::Char('q'), KeyModifiers::NONE);

            assert!(!app.overlays.is_finder_open());
            assert!(app.project_list.finder.query.is_empty());
        }

        #[test]
        fn finder_text_input_keeps_vim_k_as_query_text() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let mut app = super::make_app_with_config(&[project], &cargo_port_config);
            input::open_finder(&mut app);

            press(&mut app, KeyCode::Char('k'), KeyModifiers::NONE);

            assert_eq!(app.project_list.finder.query, "k");
        }

        #[test]
        fn finder_activate_rebind_wins_over_global_tab_while_finder_is_open() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(
                &[project],
                "[global]\nnext_pane = \"Tab\"\n[finder]\nactivate = \"Tab\"\n",
            );
            input::open_finder(&mut app);
            app.project_list.finder.results = vec![0];
            app.project_list.finder.total = 1;
            let base_before = app.base_focus();

            press(&mut app, KeyCode::Tab, KeyModifiers::NONE);

            assert!(!app.overlays.is_finder_open());
            assert_eq!(
                app.focused_pane_id(),
                base_before,
                "finder Activate must consume Tab before global pane cycling",
            );
        }

        #[test]
        fn keymap_capture_rejects_navigation_key_through_production_input() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            open_framework_overlay(&mut app, FrameworkGlobalAction::OpenKeymap);

            press(&mut app, KeyCode::Enter, KeyModifiers::NONE);
            press(&mut app, KeyCode::Up, KeyModifiers::NONE);

            assert!(app.framework.keymap_pane.is_capturing());
            assert!(
                app.overlays
                    .inline_error()
                    .is_some_and(|error| error.contains("reserved for navigation")),
            );
        }

        /// The `App::set_focus` override updates framework focus and records
        /// app-pane visits for render selection styling.
        #[test]
        fn set_focus_override_updates_framework_focus_and_visits() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            app.set_focus(FocusedPane::App(AppPaneId::Targets));
            assert!(matches!(
                app.framework().focused(),
                FocusedPane::App(AppPaneId::Targets)
            ));
            assert_eq!(app.focused_pane_id(), panes::PaneId::Targets);

            app.set_focus(FocusedPane::App(AppPaneId::Git));
            assert!(matches!(
                app.framework().focused(),
                FocusedPane::App(AppPaneId::Git)
            ));
            assert_eq!(app.focused_pane_id(), panes::PaneId::Git);
            assert_eq!(
                app.pane_focus_state(panes::PaneId::Targets),
                tui_pane::PaneFocusState::Remembered
            );

            app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));
            assert!(matches!(
                app.framework().focused(),
                FocusedPane::Framework(FrameworkFocusId::Toasts),
            ));
            assert_eq!(app.focused_pane_id(), panes::PaneId::Toasts);
        }

        #[test]
        fn focused_toasts_without_action_falls_through_to_app_globals() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app_with_keymap_toml(&[project], "[global]\nfind = \"Enter\"\n");
            let _ = app.framework.toasts.push("Build done", "ok");
            app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));

            press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

            assert!(app.overlays.is_finder_open());
        }

        #[test]
        fn enter_on_focused_toast_with_action_dispatches() {
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.config.current_mut().tui.editor =
                "/definitely/missing/cargo-port-editor".to_string();
            let action_path = crate::project::AbsolutePath::from(std::path::PathBuf::from(
                "/tmp/cargo-port-keymap.toml",
            ));
            let _ = app.framework.toasts.push_with_action(
                "Keymap errors",
                "bad binding",
                CargoPortToastAction::OpenPath(action_path),
            );
            app.set_focus(FocusedPane::Framework(FrameworkFocusId::Toasts));

            press(&mut app, KeyCode::Enter, KeyModifiers::NONE);

            assert!(
                app.framework
                    .toasts
                    .active_now()
                    .iter()
                    .any(|toast| toast.title() == "Toast action failed"),
                "Enter on a focused toast with an action should dispatch the cargo-port toast action"
            );
        }

        #[test]
        fn focused_package_bar_nav_region_renders_arrow_keys() {
            // Lock the framework's nav-region rendering for a focused
            // Mode::Navigable pane. The nav region surfaces the pane-cycle row
            // plus the navigation defaults; the keymap's default for
            // `NavAction::Up` is `↑` so we look for that glyph as a
            // stable anchor.
            let project = super::make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.panes.package.set_content(package_data_no_version());
            focus_app_pane_in_framework(&mut app, AppPaneId::Package);

            let palette = BarPalette::default();
            let bar = render_status_bar(
                &FocusedPane::App(AppPaneId::Package),
                &app,
                &app.framework_keymap,
                app.framework(),
                &palette,
            );
            let nav = flatten(&bar.nav);

            assert_contains_in_order(&nav, &["↑/↓", "nav", "tab", "pane"]);
        }
    }
    mod interaction {
        use std::path::Path;
        use std::rc::Rc;
        use std::time::Duration;
        use std::time::Instant;

        use cargo_metadata::PackageId;
        use cargo_metadata::TargetKind;
        use cargo_metadata::semver::Version;
        use crossterm::event::Event;
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyEventKind;
        use crossterm::event::KeyModifiers;
        use crossterm::event::MouseButton;
        use crossterm::event::MouseEvent;
        use crossterm::event::MouseEventKind;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Position;
        use tempfile::TempDir;
        use tui_pane::AppContext;
        use tui_pane::ClipboardBackend;
        use tui_pane::ClipboardError;
        use tui_pane::FocusedPane;
        use tui_pane::FrameworkFocusId;
        use tui_pane::GlobalAction as FrameworkGlobalAction;
        use tui_pane::PaneFocusState;
        use tui_pane::PaneSelectionState;
        use tui_pane::RenderFocus;
        use tui_pane::ToastId;
        use tui_pane::ToastStyle;
        use tui_pane::Viewport;

        use crate::ci::CiJob;
        use crate::ci::CiRun;
        use crate::ci::CiStatus;
        use crate::ci::FetchStatus;
        use crate::config::CargoPortConfig;
        use crate::config::EdgeScroll;
        use crate::config::NavigationKeys;
        use crate::lint::LintCommand;
        use crate::lint::LintCommandStatus;
        use crate::lint::LintRun;
        use crate::lint::LintRunStatus;
        use crate::project;
        use crate::project::AbsolutePath;
        use crate::project::Cargo;
        use crate::project::CheckoutInfo;
        use crate::project::ExampleGroup;
        use crate::project::FileStamp;
        use crate::project::GitStatus;
        use crate::project::HeadState;
        use crate::project::ManifestFingerprint;
        use crate::project::MemberGroup;
        use crate::project::Package;
        use crate::project::PackageRecord;
        use crate::project::ProjectType;
        use crate::project::PublishPolicy;
        use crate::project::PublishStatus;
        use crate::project::RemoteInfo;
        use crate::project::RemoteKind;
        use crate::project::RepoInfo;
        use crate::project::RootItem;
        use crate::project::RustInfo;
        use crate::project::RustProject;
        use crate::project::Submodule;
        use crate::project::TargetRecord;
        use crate::project::Visibility;
        use crate::project::WorkflowPresence;
        use crate::project::Workspace;
        use crate::project::WorkspaceMetadata;
        use crate::project::WorktreeGroup;
        use crate::project::WorktreeStatus;
        use crate::scan::BackgroundMsg;
        use crate::scan::DirSizes;
        use crate::tui::app::App;
        use crate::tui::app::ConfirmAction;
        use crate::tui::app::ExpandKey;
        use crate::tui::app::HoveredPaneRow;
        use crate::tui::app::OverlayRenderInputs;
        use crate::tui::dismiss_target::DismissTarget;
        use crate::tui::finder;
        use crate::tui::hit_test::HoverTarget;
        use crate::tui::input;
        use crate::tui::integration::AppPaneId;
        use crate::tui::integration::NavAction;
        use crate::tui::interaction;
        use crate::tui::panes;
        use crate::tui::panes::LintsData;
        use crate::tui::panes::LintsProjectKind;
        use crate::tui::panes::PaneId;
        use crate::tui::panes::RunTargetKind;
        use crate::tui::panes::SyncedDescriptionHeight;
        use crate::tui::panes::TargetsData;
        use crate::tui::project_list::ProjectList;
        use crate::tui::render;
        use crate::tui::running_targets::RunProfile;
        use crate::tui::running_targets::RunningInstance;
        use crate::tui::running_targets::RunningKey;
        use crate::tui::running_targets::RunningTargets;
        use crate::tui::settings;
        use crate::tui::settings::SettingOption;
        use crate::tui::test_support as tui_test_support;

        fn open_settings_overlay(app: &mut App) {
            let keymap = Rc::clone(&app.framework_keymap);
            keymap.dispatch_framework_global(FrameworkGlobalAction::OpenSettings, app);
        }

        fn open_keymap_overlay(app: &mut App) {
            let keymap = Rc::clone(&app.framework_keymap);
            keymap.dispatch_framework_global(FrameworkGlobalAction::OpenKeymap, app);
        }

        fn make_package(name: &str, path: &Path) -> RootItem {
            make_package_with_cargo(name, path, Cargo::default())
        }

        fn make_package_with_cargo(name: &str, path: &Path, cargo: Cargo) -> RootItem {
            RootItem::Rust(RustProject::Package(Package {
                path: AbsolutePath::from(path),
                name: Some(name.to_string()),
                rust: RustInfo {
                    cargo,
                    ..RustInfo::default()
                },
                ..Package::default()
            }))
        }

        fn make_package_worktree(
            name: &str,
            path: &Path,
            is_linked_worktree: bool,
            primary_abs_path: Option<&Path>,
        ) -> Package {
            let worktree_status = match (is_linked_worktree, primary_abs_path) {
                (true, Some(p)) => WorktreeStatus::Linked {
                    primary: AbsolutePath::from(p),
                },
                (false, Some(p)) => WorktreeStatus::Primary {
                    root: AbsolutePath::from(p),
                },
                _ => WorktreeStatus::NotGit,
            };
            Package {
                path: AbsolutePath::from(path),
                name: Some(name.to_string()),
                worktree_status,
                ..Package::default()
            }
        }

        fn inline_group(members: Vec<Package>) -> MemberGroup { MemberGroup::Inline { members } }

        fn make_member(name: &str, path: &Path) -> Package {
            Package {
                path: AbsolutePath::from(path),
                name: Some(name.to_string()),
                ..Package::default()
            }
        }

        fn make_workspace_with_members(
            name: &str,
            path: &Path,
            groups: Vec<MemberGroup>,
        ) -> RootItem {
            RootItem::Rust(RustProject::Workspace(Workspace {
                path: AbsolutePath::from(path),
                name: Some(name.to_string()),
                groups,
                ..Workspace::default()
            }))
        }

        fn make_git_info(url: Option<&str>) -> (CheckoutInfo, RepoInfo) {
            let checkout = CheckoutInfo {
                status:              GitStatus::Clean,
                head:                HeadState::Branch("main".to_string()),
                last_commit:         Some("2024-01-02T00:00:00Z".to_string()),
                ahead_behind_local:  Some((0, 0)),
                primary_tracked_ref: Some("origin/main".to_string()),
                bisect:              None,
            };
            let repo = RepoInfo {
                remotes:           vec![RemoteInfo {
                    name:         "origin".to_string(),
                    url:          url.map(str::to_string),
                    owner:        Some("natepiano".to_string()),
                    repo:         Some("demo".to_string()),
                    tracked_ref:  Some("origin/main".to_string()),
                    ahead_behind: Some((0, 0)),
                    kind:         RemoteKind::Clone,
                    push:         crate::project::PushState::Enabled {
                        push_url: String::new(),
                    },
                }],
                workflows:         WorkflowPresence::Present,
                first_commit:      Some("2024-01-01T00:00:00Z".to_string()),
                last_fetched:      None,
                default_branch:    Some("main".to_string()),
                local_main_branch: Some("main".to_string()),
            };
            (checkout, repo)
        }

        fn make_ci_run(run_id: u64, conclusion: CiStatus) -> CiRun {
            CiRun {
                run_id,
                created_at: "2024-01-01T00:00:00Z".to_string(),
                branch: "main".to_string(),
                url: format!("https://github.com/natepiano/demo/actions/runs/{run_id}"),
                ci_status: conclusion,
                jobs: vec![CiJob {
                    name:          "build".to_string(),
                    ci_status:     conclusion,
                    duration:      "1m".to_string(),
                    duration_secs: Some(60),
                }],
                wall_clock_secs: Some(60),
                commit_title: Some("commit".to_string()),
                updated_at: None,
                fetched: FetchStatus::Fetched,
            }
        }

        fn make_lint_run(run_id: &str, status: LintRunStatus) -> LintRun {
            LintRun {
                run_id: run_id.to_string(),
                started_at: "2024-01-01T00:00:00Z".to_string(),
                finished_at: Some("2024-01-01T00:01:00Z".to_string()),
                duration_ms: Some(60_000),
                status,
                commands: vec![LintCommand {
                    name:        "clippy".to_string(),
                    command:     "cargo clippy".to_string(),
                    status:      LintCommandStatus::Passed,
                    duration_ms: Some(1_000),
                    exit_code:   Some(0),
                    log_file:    "clippy.log".to_string(),
                }],
                archive_bytes: 0,
            }
        }

        fn make_app(projects: &[RootItem]) -> App { tui_test_support::make_app(projects) }

        /// Build an app with vim navigation enabled so the output pane's `V`
        /// toggles the visual-line sub-mode (it is inert with vim off). The
        /// config is passed through the constructor so the built keymap matches,
        /// and vim mode is re-asserted on the app's own config afterward so the
        /// runtime `V` check does not depend on the process-wide active-config
        /// singleton that concurrent tests mutate.
        fn make_app_vim(projects: &[RootItem]) -> App {
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            let mut app = tui_test_support::make_app_with_config(projects, &cargo_port_config);
            app.config.current_mut().tui.navigation_keys = NavigationKeys::ArrowsAndVim;
            app
        }

        /// The output pane's inclusive selection range against the live buffer.
        fn output_range(app: &App) -> Option<(usize, usize)> {
            app.panes
                .output
                .selected_range(app.inflight.example_output())
        }

        /// The output pane's selection line count against the live buffer.
        fn output_count(app: &App) -> usize {
            app.panes
                .output
                .selection_line_count(app.inflight.example_output())
        }

        fn render_ui(app: &mut App) {
            app.ensure_visible_rows_cached();
            app.ensure_detail_cached();
            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| render::ui(frame, app))
                .expect("draw test frame");
        }

        fn render_lints_panel(app: &mut App, runs: &[LintRun]) {
            app.ensure_detail_cached();
            app.lint.set_content(LintsData {
                runs:         runs.to_vec(),
                sizes:        vec![Some(0); runs.len()],
                owner_paths:  Vec::new(),
                owner_of:     Vec::new(),
                project_kind: LintsProjectKind::Rust,
            });
            let backend = TestBackend::new(120, 20);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            let focus = RenderFocus {
                pane_focus_state: app.pane_focus_state(PaneId::Lints),
            };
            app.lint.focus = focus;
            let animation_elapsed = app.animation_started.elapsed();
            let selected_path = app
                .selected_project_path_for_render()
                .map(std::path::Path::to_path_buf);
            let ci_status_lookup = app.ci.status_lookup();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let split = app.split_for_render(
                        selected_path.as_deref(),
                        animation_elapsed,
                        &ci_status_lookup,
                        OverlayRenderInputs::none(),
                        SyncedDescriptionHeight::default(),
                    );
                    tui_pane::Renderable::render(
                        split.registry.lint,
                        frame,
                        area,
                        &split.pane_render_ctx,
                    );
                })
                .expect("draw test frame");
        }

        fn render_ci_panel(app: &mut App, runs: &[CiRun]) {
            app.ensure_detail_cached();
            app.ci.override_runs_for_test(runs.to_vec());
            let backend = TestBackend::new(120, 20);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            let focus = RenderFocus {
                pane_focus_state: app.pane_focus_state(PaneId::CiRuns),
            };
            app.ci.focus = focus;
            let animation_elapsed = app.animation_started.elapsed();
            let selected_path = app
                .selected_project_path_for_render()
                .map(std::path::Path::to_path_buf);
            let ci_status_lookup = app.ci.status_lookup();
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    let split = app.split_for_render(
                        selected_path.as_deref(),
                        animation_elapsed,
                        &ci_status_lookup,
                        OverlayRenderInputs::none(),
                        SyncedDescriptionHeight::default(),
                    );
                    tui_pane::Renderable::render(
                        split.registry.ci,
                        frame,
                        area,
                        &split.pane_render_ctx,
                    );
                })
                .expect("draw test frame");
        }

        fn click(app: &mut App, column: u16, row: u16) {
            input::handle_event(
                app,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }

        fn move_mouse(app: &mut App, column: u16, row: u16) {
            input::handle_event(
                app,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }

        fn scroll_down(app: &mut App, column: u16, row: u16) {
            input::handle_event(
                app,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::ScrollDown,
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }

        fn drag(app: &mut App, column: u16, row: u16) {
            input::handle_event(
                app,
                &Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Drag(MouseButton::Left),
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                }),
            );
        }

        fn press_key(app: &mut App, code: KeyCode) {
            input::handle_event(
                app,
                &Event::Key(KeyEvent {
                    code,
                    modifiers: KeyModifiers::NONE,
                    kind: KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                }),
            );
        }

        fn press_shift_key(app: &mut App, code: KeyCode) {
            input::handle_event(
                app,
                &Event::Key(KeyEvent {
                    code,
                    modifiers: KeyModifiers::SHIFT,
                    kind: KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                }),
            );
        }

        fn press_ctrl_shift_key(app: &mut App, code: KeyCode) {
            input::handle_event(
                app,
                &Event::Key(KeyEvent {
                    code,
                    modifiers: KeyModifiers::CONTROL | KeyModifiers::SHIFT,
                    kind: KeyEventKind::Press,
                    state: crossterm::event::KeyEventState::NONE,
                }),
            );
        }

        fn focus_gained(app: &mut App) { input::handle_event(app, &Event::FocusGained); }

        fn row_body_point(app: &App, row_index: usize) -> (u16, u16) {
            let area = app.panes.project_list.body_rect;
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
            )
        }

        fn row_dismiss_point(app: &App, row_index: usize) -> (u16, u16) {
            let area = app.panes.project_list.body_rect;
            (
                area.x.saturating_add(area.width.saturating_sub(2)),
                area.y
                    .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
            )
        }

        fn pane_row_point(pane: &Viewport, row_index: usize) -> (u16, u16) {
            let area = pane.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
            )
        }

        fn package_metadata_row_point(app: &App, row_index: usize) -> (u16, u16) {
            let area = app.panes.package.viewport.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(2)
                    .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
            )
        }

        fn pane_row_hit_point(app: &App, pane: PaneId, row: usize) -> (u16, u16) {
            let area = app
                .panes
                .tiled_layout
                .panes
                .iter()
                .find_map(|resolved| (resolved.pane == pane).then_some(resolved.area))
                .expect("pane must be laid out");
            for y in area.y..area.bottom() {
                for x in area.x..area.right() {
                    if interaction::hovered_pane_row_at(app, Position::new(x, y))
                        == Some(HoveredPaneRow { pane, row })
                    {
                        return (x, y);
                    }
                }
            }
            panic!("row {row} in pane {pane:?} was not hit-testable");
        }

        fn framework_pane_row_point(pane: &Viewport, row_index: usize) -> (u16, u16) {
            let area = pane.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(u16::try_from(row_index).unwrap_or(u16::MAX)),
            )
        }

        fn settings_point_for_setting(app: &App, setting: SettingOption) -> (u16, u16) {
            let row = settings::selection_index_for_setting_for_test(app, setting)
                .expect("setting must be visible");
            let pane = &app.framework.settings_pane;
            let height = usize::from(pane.viewport().content_area().height);
            let line = (0..height)
                .find(|line| pane.line_target(*line) == Some(row))
                .expect("setting must have a rendered hit target");
            framework_pane_row_point(pane.viewport(), line)
        }

        fn keymap_point_for_row_after(app: &App, min_row: usize) -> (u16, u16, usize) {
            let pane = &app.framework.keymap_pane;
            let height = usize::from(pane.viewport().content_area().height);
            let (line, row) = (0..height)
                .filter_map(|line| pane.line_target(line).map(|row| (line, row)))
                .find(|(_, row)| *row > min_row)
                .expect("keymap row must have a rendered hit target");
            let (x, y) = framework_pane_row_point(pane.viewport(), line);
            (x, y, row)
        }

        fn finder_result_point(app: &App, result_index: usize) -> (u16, u16) {
            let area = app.overlays.finder_pane.viewport.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(1)
                    .saturating_add(u16::try_from(result_index).unwrap_or(u16::MAX)),
            )
        }

        fn lint_run_point(app: &App, run_index: usize) -> (u16, u16) {
            let area = app.lint.viewport.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(1)
                    .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
            )
        }

        fn ci_run_point(app: &App, run_index: usize) -> (u16, u16) {
            let area = app.ci.viewport.content_area();
            (
                area.x.saturating_add(1),
                area.y
                    .saturating_add(1)
                    .saturating_add(u16::try_from(run_index).unwrap_or(u16::MAX)),
            )
        }

        /// Screen point for output row `row`. The output pane has no header, so
        /// rows start at the top of the content area; `content_area` is already
        /// the inner rect inside the border.
        fn output_point(app: &App, row: usize) -> (u16, u16) {
            let area = app.panes.output.viewport.content_area();
            (
                area.x,
                area.y
                    .saturating_add(u16::try_from(row).unwrap_or(u16::MAX)),
            )
        }

        fn toast_close_point(app: &App, toast_id: ToastId) -> (u16, u16) {
            let Some(rect) = app
                .framework
                .toasts
                .hits()
                .iter()
                .find(|h| h.id == toast_id)
                .map(|h| h.close_rect)
            else {
                panic!("toast close hit should be rendered for test toast");
            };
            (
                rect.x.saturating_add(rect.width.saturating_sub(1) / 2),
                rect.y.saturating_add(rect.height.saturating_sub(1) / 2),
            )
        }

        fn toast_body_point(app: &App, toast_id: ToastId) -> (u16, u16) {
            let Some(rect) = app
                .framework
                .toasts
                .hits()
                .iter()
                .find(|h| h.id == toast_id)
                .map(|h| h.card_rect)
            else {
                panic!("toast body hit should be rendered for test toast");
            };
            (
                rect.x.saturating_add(rect.width.saturating_sub(1) / 2),
                rect.y.saturating_add(rect.height.saturating_sub(1) / 2),
            )
        }

        fn mark_deleted(app: &mut App, path: &Path) {
            let project = app
                .project_list
                .at_path_mut(path)
                .expect("test project should exist in project list");
            project.disk_usage_bytes = Some(0);
            project.visibility = Visibility::Deleted;
        }

        #[test]
        fn deleted_project_row_mouse_click_dismisses_it() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let deleted_dir = tmp.path().join("deleted");
            std::fs::create_dir_all(&deleted_dir).expect("create test directory");

            let mut app = make_app(&[make_package("deleted", &deleted_dir)]);
            mark_deleted(&mut app, &deleted_dir);
            render_ui(&mut app);

            let (x, y) = row_dismiss_point(&app, 0);
            click(&mut app, x, y);
            render_ui(&mut app);

            assert!(
                app.visible_rows().is_empty(),
                "clicking deleted row [x] should stop rendering that row"
            );
        }

        #[test]
        fn mouse_and_keyboard_dismiss_resolve_same_deleted_project_target() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let deleted_dir = tmp.path().join("deleted");
            std::fs::create_dir_all(&deleted_dir).expect("create test directory");

            let mut app = make_app(&[make_package("deleted", &deleted_dir)]);
            mark_deleted(&mut app, &deleted_dir);
            app.project_list.set_cursor(0);
            render_ui(&mut app);

            let keyboard_target = app
                .focused_dismiss_target()
                .expect("deleted project should have a focused dismiss target");
            let (x, y) = row_dismiss_point(&app, 0);
            let Some(hit) = interaction::hit_test_at(&app, Position::new(x, y)) else {
                panic!("deleted row dismiss point should hit a target");
            };
            let HoverTarget::Dismiss(mouse_target) = hit else {
                unreachable!("deleted row dismiss point should hit dismiss target");
            };

            let DismissTarget::DeletedProject(lhs) = keyboard_target else {
                unreachable!("keyboard dismiss target should be deleted project");
            };
            let DismissTarget::DeletedProject(rhs) = mouse_target else {
                unreachable!("mouse dismiss target should be deleted project");
            };
            assert_eq!(lhs, rhs);
        }

        #[test]
        fn row_body_click_selects_clicked_project() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let first = tmp.path().join("first");
            let second = tmp.path().join("second");
            std::fs::create_dir_all(&first).expect("create test directory");
            std::fs::create_dir_all(&second).expect("create test directory");

            let mut app = make_app(&[
                make_package("first", &first),
                make_package("second", &second),
            ]);
            render_ui(&mut app);

            let (x, y) = row_body_point(&app, 1);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::ProjectList);
            assert_eq!(app.project_list.cursor(), 1);
            assert_eq!(
                app.project_list
                    .selected_project_path()
                    .map(Path::to_path_buf),
                Some(second),
            );
        }

        #[test]
        fn expandable_project_row_click_toggles_children() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let root_dir = tmp.path().join("demo");
            let sub_dir = root_dir.join("vendor").join("dep");
            std::fs::create_dir_all(&sub_dir).expect("create test directory");

            let root = make_package("demo", &root_dir);
            let mut app = make_app(&[root]);
            app.project_list
                .at_path_mut(&root_dir)
                .expect("test project should exist in project list")
                .submodules
                .push(Submodule {
                    name:          "vendor/dep".to_string(),
                    path:          AbsolutePath::from(sub_dir),
                    relative_path: "vendor/dep".to_string(),
                    url:           None,
                    branch:        None,
                    commit:        None,
                    project_info:  crate::project::ProjectInfo::default(),
                    git_repo:      None,
                });
            render_ui(&mut app);

            let (x, y) = pane_row_hit_point(&app, PaneId::ProjectList, 0);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::ProjectList);
            assert_eq!(app.project_list.cursor(), 0);
            assert_eq!(app.visible_rows().len(), 2);

            render_ui(&mut app);
            let (x, y) = pane_row_hit_point(&app, PaneId::ProjectList, 0);
            click(&mut app, x, y);

            assert_eq!(app.project_list.cursor(), 0);
            assert_eq!(app.visible_rows().len(), 1);
        }

        #[test]
        fn focus_gained_on_project_row_selects_without_toggling_children() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let root_dir = tmp.path().join("demo");
            let sub_dir = root_dir.join("vendor").join("dep");
            std::fs::create_dir_all(&sub_dir).expect("create test directory");

            let root = make_package("demo", &root_dir);
            let mut app = make_app(&[root]);
            app.project_list
                .at_path_mut(&root_dir)
                .expect("test project should exist in project list")
                .submodules
                .push(Submodule {
                    name:          "vendor/dep".to_string(),
                    path:          AbsolutePath::from(sub_dir),
                    relative_path: "vendor/dep".to_string(),
                    url:           None,
                    branch:        None,
                    commit:        None,
                    project_info:  crate::project::ProjectInfo::default(),
                    git_repo:      None,
                });
            render_ui(&mut app);

            app.set_focus(FocusedPane::App(AppPaneId::Package));
            let (x, y) = pane_row_hit_point(&app, PaneId::ProjectList, 0);
            input::set_last_mouse_pos_for_test(Some((x, y)));
            focus_gained(&mut app);

            assert_eq!(app.focused_pane_id(), PaneId::ProjectList);
            assert_eq!(app.project_list.cursor(), 0);
            assert_eq!(app.visible_rows().len(), 1);

            render_ui(&mut app);
            let (x, y) = pane_row_hit_point(&app, PaneId::ProjectList, 0);
            click(&mut app, x, y);

            assert_eq!(app.visible_rows().len(), 2);
        }

        // The "overlay surface beats content surface" priority is now
        // encoded by the order of `HITTABLE_Z_ORDER` in
        // `panes::dispatch`. The strum-backed
        // `z_order_covers_every_hittable_id` test pins coverage; the
        // ordering itself is enforced by the literal constant value.

        #[test]
        fn hovered_pane_row_resolves_project_list_rows() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let first = tmp.path().join("first");
            let second = tmp.path().join("second");
            std::fs::create_dir_all(&first).expect("create test directory");
            std::fs::create_dir_all(&second).expect("create test directory");

            let mut app = make_app(&[
                make_package("first", &first),
                make_package("second", &second),
            ]);
            render_ui(&mut app);

            let (x, y) = row_body_point(&app, 1);
            assert_eq!(
                interaction::hovered_pane_row_at(&app, Position::new(x, y)),
                Some(HoveredPaneRow {
                    pane: PaneId::ProjectList,
                    row:  1,
                }),
            );
        }

        #[test]
        fn finder_row_click_uses_result_index_not_visual_table_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let alpha = tmp.path().join("alpha");
            let beta = tmp.path().join("beta");
            std::fs::create_dir_all(&alpha).expect("create test directory");
            std::fs::create_dir_all(&beta).expect("create test directory");

            let mut app = make_app(&[make_package("alpha", &alpha), make_package("beta", &beta)]);
            let (index, col_widths) = finder::build_finder_index(&app.project_list);
            let finder = &mut app.project_list.finder;
            finder.index = index;
            finder.col_widths = col_widths;
            finder.results = vec![0, 1];
            finder.total = 2;
            app.overlays
                .set_finder_return(FocusedPane::App(AppPaneId::ProjectList));
            app.set_focus(FocusedPane::App(AppPaneId::Finder));
            app.overlays.open_finder();
            render_ui(&mut app);

            let (x, y) = finder_result_point(&app, 1);
            click(&mut app, x, y);

            assert_eq!(
                app.overlays.finder_pane.viewport.pos(),
                1,
                "clicking the second rendered finder result should select result index 1, not the header-offset visual row"
            );
        }

        #[test]
        fn git_hover_uses_owner_backed_pane_surface_for_workspace_member() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let workspace = tmp.path().join("ws");
            let member = workspace.join("core");
            std::fs::create_dir_all(&member).expect("create test directory");

            let root = make_workspace_with_members(
                "ws",
                &workspace,
                vec![inline_group(vec![make_member("core", &member)])],
            );
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();
            app.project_list.move_down();
            let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
            app.handle_repo_info(&workspace, repo);
            app.handle_checkout_info(&workspace, checkout);

            render_ui(&mut app);

            let (x, y) = pane_row_point(&app.panes.git.viewport, 0);
            assert_eq!(
                interaction::hovered_pane_row_at(&app, Position::new(x, y)),
                Some(HoveredPaneRow {
                    pane: PaneId::Git,
                    row:  0,
                }),
            );
        }

        #[test]
        fn settings_row_click_uses_setting_index_not_visual_line() {
            let mut app = make_app(&[]);
            open_settings_overlay(&mut app);
            render_ui(&mut app);
            let ci_run_count_row =
                settings::selection_index_for_setting_for_test(&app, SettingOption::CiRunCount)
                    .expect("CI run count row");

            let (x, y) = settings_point_for_setting(&app, SettingOption::CiRunCount);
            click(&mut app, x, y);

            assert_eq!(
                app.framework.settings_pane.viewport().pos(),
                ci_run_count_row,
                "clicking a rendered settings option should select the logical setting, not the visual line index including spacer/header rows"
            );
        }

        #[test]
        fn keymap_row_click_uses_keymap_line_targets() {
            let mut app = make_app(&[]);
            open_keymap_overlay(&mut app);
            render_ui(&mut app);

            let (x, y, row) = keymap_point_for_row_after(&app, 0);
            click(&mut app, x, y);

            assert_eq!(
                app.framework.keymap_pane.viewport().pos(),
                row,
                "clicking a keymap row should select the logical keymap entry, not the visual line including spacer/header rows"
            );
        }

        fn assert_overlay_blocks_underlying_project_list_mouse(
            overlay_name: &str,
            open_overlay: fn(&mut App),
        ) {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let first = tmp.path().join("first");
            let second = tmp.path().join("second");
            std::fs::create_dir_all(&first).expect("create test directory");
            std::fs::create_dir_all(&second).expect("create test directory");
            let mut app = make_app(&[
                make_package("first", &first),
                make_package("second", &second),
            ]);
            render_ui(&mut app);
            open_overlay(&mut app);
            render_ui(&mut app);

            let (x, y) = row_body_point(&app, 1);
            click(&mut app, x, y);
            scroll_down(&mut app, x, y);

            assert_eq!(
                app.project_list.cursor(),
                0,
                "project-list mouse input must not pass through an open {overlay_name} overlay"
            );
        }

        #[test]
        fn overlays_block_underlying_project_list_mouse() {
            for (overlay_name, open_overlay) in [
                ("keymap", open_keymap_overlay as fn(&mut App)),
                ("finder", input::open_finder as fn(&mut App)),
                ("settings", open_settings_overlay as fn(&mut App)),
            ] {
                assert_overlay_blocks_underlying_project_list_mouse(overlay_name, open_overlay);
            }
        }

        #[test]
        fn keyboard_navigation_clears_stale_settings_hover() {
            let mut app = make_app(&[]);
            open_settings_overlay(&mut app);
            render_ui(&mut app);

            let hovered_row =
                settings::selection_index_for_setting_for_test(&app, SettingOption::CiRunCount)
                    .expect("CI run count row");
            let (x, y) = settings_point_for_setting(&app, SettingOption::CiRunCount);
            move_mouse(&mut app, x, y);
            render_ui(&mut app);

            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    hovered_row,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Hovered,
            );

            press_key(&mut app, KeyCode::Down);
            render_ui(&mut app);

            assert_eq!(app.framework.settings_pane.viewport().pos(), 1);
            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    hovered_row,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Unselected,
            );
            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    1,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Active,
            );
        }

        #[test]
        fn mouse_move_restores_hover_after_keyboard_navigation() {
            let mut app = make_app(&[]);
            open_settings_overlay(&mut app);
            render_ui(&mut app);

            let hovered_row =
                settings::selection_index_for_setting_for_test(&app, SettingOption::CiRunCount)
                    .expect("CI run count row");
            let (x, y) = settings_point_for_setting(&app, SettingOption::CiRunCount);
            move_mouse(&mut app, x, y);
            render_ui(&mut app);
            press_key(&mut app, KeyCode::Down);
            render_ui(&mut app);

            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    hovered_row,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Unselected,
            );

            move_mouse(&mut app, x, y);
            render_ui(&mut app);

            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    hovered_row,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Hovered,
            );
        }

        #[test]
        fn focus_gained_restores_selection_from_last_mouse_position() {
            let mut app = make_app(&[]);
            open_settings_overlay(&mut app);
            render_ui(&mut app);

            let hovered_row =
                settings::selection_index_for_setting_for_test(&app, SettingOption::CiRunCount)
                    .expect("CI run count row");
            let (x, y) = settings_point_for_setting(&app, SettingOption::CiRunCount);
            input::set_last_mouse_pos_for_test(Some((x, y)));
            focus_gained(&mut app);
            render_ui(&mut app);

            assert_eq!(app.framework.settings_pane.viewport().pos(), hovered_row);
            assert_eq!(
                tui_pane::selection_state(
                    app.framework.settings_pane.viewport(),
                    hovered_row,
                    PaneFocusState::Active,
                ),
                PaneSelectionState::Active,
            );
        }

        #[test]
        fn lint_row_click_uses_run_index_not_header_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            let runs = vec![
                make_lint_run("run-1", LintRunStatus::Passed),
                make_lint_run("run-2", LintRunStatus::Failed),
            ];
            render_lints_panel(&mut app, &runs);

            let (x, y) = lint_run_point(&app, 1);
            click(&mut app, x, y);

            assert_eq!(
                app.lint.viewport.pos(),
                1,
                "clicking the second rendered lint run should select run index 1, not the header-offset visual row"
            );
        }

        #[test]
        fn ci_row_click_uses_run_index_not_header_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package_with_cargo(
                "demo",
                &project_dir,
                Cargo {
                    types: vec![ProjectType::Binary],
                    examples: vec![ExampleGroup {
                        category: String::new(),
                        names:    vec!["example".to_string()],
                    }],
                    ..Cargo::default()
                },
            )]);
            let runs = vec![
                make_ci_run(1, CiStatus::Passed),
                make_ci_run(2, CiStatus::Failed),
            ];
            render_ci_panel(&mut app, &runs);

            let (x, y) = ci_run_point(&app, 1);
            click(&mut app, x, y);

            assert_eq!(
                app.ci.viewport.pos(),
                1,
                "clicking the second rendered CI run should select run index 1, not the header-offset visual row"
            );
        }

        #[test]
        fn expanded_tree_rebuild_refreshes_clickable_rows() {
            let primary: AbsolutePath = "/abs/app".into();
            let linked: AbsolutePath = "/abs/app_feat".into();
            let mut app = make_app(&[RootItem::Rust(RustProject::Package(make_package_worktree(
                "app",
                &primary,
                false,
                Some(primary.as_path()),
            )))]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            render_ui(&mut app);

            app.project_list
                .replace_roots_from(ProjectList::new(vec![RootItem::Worktrees(
                    WorktreeGroup::new(
                        RustProject::Package(make_package_worktree(
                            "app",
                            &primary,
                            false,
                            Some(primary.as_path()),
                        )),
                        vec![RustProject::Package(make_package_worktree(
                            "app",
                            &linked,
                            true,
                            Some(primary.as_path()),
                        ))],
                    ),
                )]));
            render_ui(&mut app);

            let (x, y) = row_body_point(&app, 2);
            click(&mut app, x, y);

            assert_eq!(
                app.project_list.selected_project_path(),
                Some(linked.as_path()),
                "clicking the linked worktree row after regroup should select it"
            );
        }

        #[test]
        fn old_dismiss_click_location_does_not_dismiss_surviving_row_after_rerender() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let deleted_dir = tmp.path().join("deleted");
            let live_dir = tmp.path().join("live");
            std::fs::create_dir_all(&deleted_dir).expect("create test directory");
            std::fs::create_dir_all(&live_dir).expect("create test directory");

            let mut app = make_app(&[
                make_package("deleted", &deleted_dir),
                make_package("live", &live_dir),
            ]);
            mark_deleted(&mut app, &deleted_dir);
            render_ui(&mut app);
            let stale_click = row_dismiss_point(&app, 0);

            app.project_list.set_cursor(0);
            let target = app
                .focused_dismiss_target()
                .expect("deleted project should have a focused dismiss target");
            app.dismiss(target);
            render_ui(&mut app);

            click(&mut app, stale_click.0, stale_click.1);
            render_ui(&mut app);

            assert!(
                app.project_list
                    .at_path(&live_dir)
                    .is_some_and(|info| info.visibility == Visibility::Visible),
                "clicking the old dismiss location after rerender must not dismiss the surviving row"
            );
            assert_eq!(
                app.project_list
                    .selected_project_path()
                    .map(Path::to_path_buf),
                Some(live_dir),
                "the surviving row may be selected, but it must not be dismissed by stale geometry"
            );
        }

        #[test]
        fn toast_close_click_dismisses_toast() {
            let mut app = make_app(&[]);
            let toast_id = app.framework.toasts.push_persistent(
                "Error",
                "toast body",
                ToastStyle::Error,
                None,
                1,
            );
            let toast_len = app.framework.toasts.active_now().len();
            app.framework.toasts.viewport.set_len(toast_len);
            render_ui(&mut app);

            let (x, y) = toast_close_point(&app, toast_id);
            click(&mut app, x, y);
            let after_exit = Instant::now() + Duration::from_secs(1);
            app.framework.toasts.prune(after_exit);

            assert!(
                app.framework
                    .toasts
                    .active_views(after_exit)
                    .iter()
                    .all(|toast| toast.id() != toast_id),
                "clicking the toast close affordance should start dismissal and let the toast exit"
            );
        }

        #[test]
        fn toast_body_click_focuses_toast_over_underlying_content() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            let toast_id = app.framework.toasts.push_persistent(
                "Error",
                "toast body",
                ToastStyle::Error,
                None,
                1,
            );
            let toast_len = app.framework.toasts.active_now().len();
            app.framework.toasts.viewport.set_len(toast_len);
            render_ui(&mut app);

            let (x, y) = toast_body_point(&app, toast_id);
            click(&mut app, x, y);

            assert_eq!(
                app.focused_pane_id(),
                PaneId::Toasts,
                "toast body click should focus the toast surface over underlying content"
            );
        }

        #[test]
        fn package_pane_row_click_selects_field() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            render_ui(&mut app);

            let (x, y) = pane_row_hit_point(&app, PaneId::Package, 1);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Package);
            assert_eq!(app.panes.package.viewport.pos(), 1);
        }

        #[test]
        fn edge_scroll_down_past_bottom_advances_to_next_pane() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            app.config.current_mut().tui.edge_scroll = EdgeScroll::AdvancesPane;
            render_ui(&mut app);
            app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
            app.project_list.move_to_bottom();

            panes::dispatch_navigation_action(
                NavAction::Down,
                FocusedPane::App(AppPaneId::ProjectList),
                &mut app,
            );

            assert_eq!(
                app.focused_pane_id(),
                PaneId::Package,
                "Down at the bottom row should roll focus to the next pane in tab order",
            );
        }

        #[test]
        fn edge_scroll_down_past_last_toast_advances_to_next_pane() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            app.config.current_mut().tui.edge_scroll = EdgeScroll::AdvancesPane;
            let _ = app
                .framework
                .toasts
                .push_persistent("First", "", ToastStyle::Normal, None, 1);
            let _ = app
                .framework
                .toasts
                .push_persistent("Second", "", ToastStyle::Normal, None, 1);
            app.set_focus_to_pane(PaneId::Toasts);
            app.framework.toasts.reset_to_last();

            panes::dispatch_navigation_action(
                NavAction::Down,
                FocusedPane::Framework(FrameworkFocusId::Toasts),
                &mut app,
            );

            assert_eq!(
                app.focused_pane_id(),
                PaneId::ProjectList,
                "Down at the last toast should roll focus to the next pane in tab order",
            );
        }

        #[test]
        fn edge_scroll_off_holds_focus_at_list_edge() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            render_ui(&mut app);
            app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
            app.project_list.move_to_bottom();

            panes::dispatch_navigation_action(
                NavAction::Down,
                FocusedPane::App(AppPaneId::ProjectList),
                &mut app,
            );

            assert_eq!(
                app.focused_pane_id(),
                PaneId::ProjectList,
                "with edge scroll off, focus stays at the list edge",
            );
        }

        #[test]
        fn package_pane_description_row_click_selects_first_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            app.panes.package.viewport.set_pos(1);
            render_ui(&mut app);

            let (x, y) = pane_row_hit_point(&app, PaneId::Package, 0);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Package);
            assert_eq!(app.panes.package.viewport.pos(), 0);
        }

        #[test]
        fn package_pane_section_row_click_is_ignored() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary = tmp.path().join("demo");
            let linked = tmp.path().join("demo_fix");
            std::fs::create_dir_all(&primary).expect("create test directory");
            std::fs::create_dir_all(&linked).expect("create test directory");

            let mut app = make_app(&[RootItem::Worktrees(WorktreeGroup::new(
                RustProject::Package(make_package_worktree(
                    "demo",
                    &primary,
                    false,
                    Some(&primary),
                )),
                vec![RustProject::Package(make_package_worktree(
                    "demo",
                    &linked,
                    true,
                    Some(&primary),
                ))],
            ))]);
            app.set_focus_to_pane(PaneId::Package);
            app.panes.package.viewport.set_pos(1);
            render_ui(&mut app);

            let package = app.panes.package.content().expect("package pane content");
            assert!(matches!(
                panes::package_rows_from_data(package).get(1),
                Some(panes::PackageRow::Section(_))
            ));

            let pos_before = app.panes.package.viewport.pos();
            let (x, y) = package_metadata_row_point(&app, 0);
            assert_eq!(
                interaction::hovered_pane_row_at(&app, Position::new(x, y)),
                None
            );
            click(&mut app, x, y);

            assert_eq!(app.panes.package.viewport.pos(), pos_before);
        }

        #[test]
        fn package_pane_keyboard_navigation_skips_section_rows() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary = tmp.path().join("demo");
            let linked = tmp.path().join("demo_fix");
            std::fs::create_dir_all(&primary).expect("create test directory");
            std::fs::create_dir_all(&linked).expect("create test directory");

            let mut app = make_app(&[RootItem::Worktrees(WorktreeGroup::new(
                RustProject::Package(make_package_worktree(
                    "demo",
                    &primary,
                    false,
                    Some(&primary),
                )),
                vec![RustProject::Package(make_package_worktree(
                    "demo",
                    &linked,
                    true,
                    Some(&primary),
                ))],
            ))]);
            app.set_focus_to_pane(PaneId::Package);
            render_ui(&mut app);

            let package = app.panes.package.content().expect("package pane content");
            let rows = panes::package_rows_from_data(package);
            assert!(matches!(rows.get(1), Some(panes::PackageRow::Section(_))));
            assert!(matches!(rows.get(5), Some(panes::PackageRow::Section(_))));
            assert_eq!(app.panes.package.viewport.pos(), 0);

            press_key(&mut app, KeyCode::Up);
            assert_eq!(app.panes.package.viewport.pos(), 0);
            press_key(&mut app, KeyCode::Down);
            assert_eq!(app.panes.package.viewport.pos(), 2);
            for _ in 0..3 {
                press_key(&mut app, KeyCode::Down);
            }
            assert_eq!(app.panes.package.viewport.pos(), 6);

            press_key(&mut app, KeyCode::Down);
            assert_eq!(app.panes.package.viewport.pos(), 7);

            press_key(&mut app, KeyCode::Up);
            assert_eq!(app.panes.package.viewport.pos(), 6);
        }

        #[test]
        fn package_pane_structure_rows_are_clickable_after_metadata_rows() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project = tmp.path().join("app");
            std::fs::create_dir_all(&project).expect("create test directory");
            let cargo = Cargo {
                types:          vec![ProjectType::Library],
                examples:       vec![ExampleGroup {
                    category: String::new(),
                    names:    vec!["demo".to_string()],
                }],
                benches:        Vec::new(),
                publish_status: PublishStatus::Publishable,
            };
            let root = make_package_with_cargo("app", &project, cargo);
            let mut app = make_app(&[root]);
            app.set_focus_to_pane(PaneId::Package);
            render_ui(&mut app);

            let package = app.panes.package.content().expect("package pane content");
            let rows = panes::package_rows_from_data(package);
            let structure_row = rows
                .iter()
                .position(|row| matches!(row, panes::PackageRow::Structure(0)))
                .expect("first structure row");
            let before_structure =
                panes::package_selectable_row_at_or_before(&rows, structure_row.saturating_sub(1))
                    .expect("selectable row before structure");

            app.panes.package.viewport.set_pos(before_structure);
            press_key(&mut app, KeyCode::Down);
            assert_eq!(app.panes.package.viewport.pos(), structure_row);

            let (x, y) = pane_row_hit_point(&app, PaneId::Package, structure_row);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Package);
            assert_eq!(app.panes.package.viewport.pos(), structure_row);
        }

        #[test]
        fn targets_pane_row_click_selects_target() {
            // The Targets pane sources its data from the `cargo metadata`
            // result. Populate two Example targets via a CargoMetadata
            // arrival so the pane has at least two rows to click on.
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            let make_target = |name: &str| TargetRecord {
                name:              name.to_string(),
                kinds:             vec![TargetKind::Example],
                required_features: vec![],
                src_path:          AbsolutePath::from(
                    project_dir.join(format!("examples/{name}.rs")),
                ),
            };
            let pkg_id = PackageId {
                repr: "demo-id".into(),
            };
            let pkg = PackageRecord {
                name:          "demo".into(),
                version:       Version::new(0, 1, 0),
                edition:       "2021".into(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: AbsolutePath::from(project_dir.join("Cargo.toml")),
                targets:       vec![make_target("example_a"), make_target("example_b")],
                publish:       PublishPolicy::Any,
            };
            let mut packages = std::collections::HashMap::new();
            packages.insert(pkg_id, pkg);
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .upsert(WorkspaceMetadata {
                    workspace_root: AbsolutePath::from(project_dir.clone()),
                    target_directory: AbsolutePath::from(project_dir.join("target")),
                    packages,
                    fingerprint: ManifestFingerprint {
                        manifest:       FileStamp {
                            content_hash: [0_u8; 32],
                        },
                        lockfile:       None,
                        rust_toolchain: None,
                        configs:        std::collections::BTreeMap::new(),
                    },
                    out_of_tree_target_bytes: None,
                });
            render_ui(&mut app);

            let (x, y) = pane_row_point(&app.panes.targets.viewport, 1);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Targets);
            assert_eq!(app.panes.targets.viewport.pos(), 1);
        }

        #[test]
        fn running_outline_parent_click_toggles_children() {
            let mut app = make_app(&[make_package("demo", Path::new("/tmp/demo"))]);
            let key = RunningKey {
                target_dir:      AbsolutePath::from("/tmp/demo/target"),
                run_target_kind: RunTargetKind::Binary,
                name:            "demo".into(),
            };
            app.panes
                .running_targets
                .set_snapshot_for_test(RunningTargets::from_pairs(vec![(
                    key,
                    vec![
                        RunningInstance::for_test(10, RunProfile::Debug),
                        RunningInstance::for_test(20, RunProfile::Debug).with_parent(10),
                    ],
                )]));
            render_ui(&mut app);

            let (x, y) = pane_row_hit_point(&app, PaneId::Targets, 0);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Targets);
            assert_eq!(app.panes.targets.viewport.pos(), 0);
            assert!(app.panes.targets.expanded_parents().contains(&10));

            render_ui(&mut app);
            let (x, y) = pane_row_hit_point(&app, PaneId::Targets, 0);
            click(&mut app, x, y);

            assert_eq!(app.panes.targets.viewport.pos(), 0);
            assert!(!app.panes.targets.expanded_parents().contains(&10));
        }

        #[test]
        fn focus_gained_on_running_outline_selects_without_toggling_children() {
            let mut app = make_app(&[make_package("demo", Path::new("/tmp/demo"))]);
            let key = RunningKey {
                target_dir:      AbsolutePath::from("/tmp/demo/target"),
                run_target_kind: RunTargetKind::Binary,
                name:            "demo".into(),
            };
            app.panes
                .running_targets
                .set_snapshot_for_test(RunningTargets::from_pairs(vec![(
                    key,
                    vec![
                        RunningInstance::for_test(10, RunProfile::Debug),
                        RunningInstance::for_test(20, RunProfile::Debug).with_parent(10),
                    ],
                )]));
            render_ui(&mut app);

            app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
            let (x, y) = pane_row_hit_point(&app, PaneId::Targets, 0);
            input::set_last_mouse_pos_for_test(Some((x, y)));
            focus_gained(&mut app);

            assert_eq!(app.focused_pane_id(), PaneId::Targets);
            assert_eq!(app.panes.targets.viewport.pos(), 0);
            assert!(!app.panes.targets.expanded_parents().contains(&10));

            render_ui(&mut app);
            let (x, y) = pane_row_hit_point(&app, PaneId::Targets, 0);
            click(&mut app, x, y);

            assert!(app.panes.targets.expanded_parents().contains(&10));
        }

        /// `Right`/`Left` expand and collapse the Running list's `cargo` group —
        /// the same keys the project list's rows use — falling through to
        /// ordinary row moves everywhere else.
        #[test]
        fn arrow_keys_expand_and_collapse_the_running_cargo_group() {
            let mut app = make_app(&[make_package("demo", Path::new("/tmp/demo"))]);
            app.panes.targets.set_content(TargetsData {
                binaries: vec![panes::TargetEntry {
                    name:              "demo".to_string(),
                    display_name:      "demo".to_string(),
                    run_target_kind:   panes::RunTargetKind::Binary,
                    source:            panes::TargetSource::workspace_root("demo".into()),
                    project_path:      AbsolutePath::from("/tmp/demo"),
                    package_name:      "demo".to_string(),
                    src_path:          AbsolutePath::from("/tmp/demo/src/main.rs"),
                    required_features: Vec::new(),
                }],
                examples: Vec::new(),
                benches:  Vec::new(),
            });
            let key = |name: &str| RunningKey {
                target_dir:      AbsolutePath::from(format!("/tmp/{name}/target")),
                run_target_kind: RunTargetKind::Binary,
                name:            name.into(),
            };
            app.panes
                .running_targets
                .set_snapshot_for_test(RunningTargets::from_pairs(vec![
                    (
                        key("cargo-port"),
                        vec![
                            RunningInstance::for_test(7, RunProfile::Installed),
                            RunningInstance::for_test(8, RunProfile::Installed),
                        ],
                    ),
                    (
                        key("worker"),
                        vec![RunningInstance::for_test(9, RunProfile::Debug)],
                    ),
                ]));
            app.set_focus_to_pane(PaneId::Targets);
            // One table row + the collapsed list (header, debug instance).
            app.panes.targets.viewport.set_len(3);
            app.panes.targets.viewport.set_pos(1);

            press_key(&mut app, KeyCode::Right);
            assert_eq!(
                app.panes.targets.cargo_group(),
                panes::CargoGroup::Expanded,
                "Right on the collapsed header expands the group",
            );
            assert_eq!(
                app.panes.targets.viewport.pos(),
                1,
                "the highlight stays on the header",
            );

            // On the expanded header, Right falls through to a row move — into
            // the first grouped instance, anchoring its PID.
            press_key(&mut app, KeyCode::Right);
            assert_eq!(app.panes.targets.viewport.pos(), 2);
            assert_eq!(app.panes.targets.running_cursor_pid(), Some(7));

            // Left on a grouped instance collapses the group and hands the
            // highlight back to the header.
            press_key(&mut app, KeyCode::Left);
            assert_eq!(
                app.panes.targets.cargo_group(),
                panes::CargoGroup::Collapsed
            );
            assert_eq!(app.panes.targets.viewport.pos(), 1);
            assert_eq!(app.panes.targets.running_cursor_pid(), None);

            // Left on the collapsed header falls through to a row-up move, back
            // into the table.
            press_key(&mut app, KeyCode::Left);
            assert_eq!(app.panes.targets.viewport.pos(), 0);
        }

        #[test]
        fn git_pane_row_click_selects_field() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
            app.handle_repo_info(&project_dir, repo);
            app.handle_checkout_info(&project_dir, checkout);
            render_ui(&mut app);

            let (x, y) = pane_row_point(&app.panes.git.viewport, 1);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.panes.git.viewport.pos(), 1);
        }

        #[test]
        fn git_pane_description_row_click_selects_first_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let mut app = make_app(&[make_package("demo", &project_dir)]);
            let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
            app.handle_repo_info(&project_dir, repo);
            app.handle_checkout_info(&project_dir, checkout);
            app.project_list.handle_repo_meta(
                &project_dir,
                7,
                Some("A useful demo repo".to_string()),
            );
            app.panes.git.viewport.set_pos(1);
            render_ui(&mut app);

            let git = app.panes.git.content().expect("git pane content");
            assert!(matches!(
                panes::git_row_at(git, 0),
                Some(panes::GitRow::Description(_))
            ));

            let (x, y) = pane_row_hit_point(&app, PaneId::Git, 0);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.panes.git.viewport.pos(), 0);

            press_key(&mut app, KeyCode::Down);
            assert_eq!(app.panes.git.viewport.pos(), 1);
        }

        // ── Confirm popup renders resolved target dir (Step 2) ─────────

        fn buffer_text(app: &mut App) -> String { buffer_text_sized(app, 120, 40) }

        fn buffer_text_sized(app: &mut App, width: u16, height: u16) -> String {
            app.ensure_visible_rows_cached();
            app.ensure_detail_cached();
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| render::ui(frame, app))
                .expect("draw test frame");
            let area = terminal.size().expect("read test terminal size");
            let buffer = terminal.backend().buffer();
            let mut text = String::new();
            for y in 0..area.height {
                for x in 0..area.width {
                    text.push_str(buffer[(x, y)].symbol());
                }
                text.push('\n');
            }
            text
        }

        fn make_many_packages(tmp: &TempDir, count: usize) -> Vec<RootItem> {
            (0..count)
                .map(|index| {
                    let name = format!("project-{index:02}");
                    let dir = tmp.path().join(&name);
                    std::fs::create_dir_all(&dir).expect("create test directory");
                    make_package(&name, &dir)
                })
                .collect()
        }

        #[test]
        fn project_list_renders_framework_overflow_affordance() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let projects = make_many_packages(&tmp, 40);
            let mut app = make_app(&projects);

            let rendered = buffer_text_sized(&mut app, 100, 18);

            assert!(
                rendered.contains("1 of"),
                "project list should render the framework-owned overflow marker"
            );
        }

        #[test]
        fn finder_results_render_framework_overflow_affordance() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let projects = make_many_packages(&tmp, 40);
            let mut app = make_app(&projects);
            input::open_finder(&mut app);
            let result_count = app.project_list.finder.index.len();
            app.project_list.finder.results = (0..result_count).collect();
            app.project_list.finder.total = result_count;

            let rendered = buffer_text_sized(&mut app, 100, 20);

            assert!(rendered.contains("Find Anything"));
            assert!(
                rendered.contains("1 of"),
                "finder should render the framework-owned overflow marker"
            );
        }

        #[test]
        fn settings_popup_renders_framework_overflow_affordance() {
            let mut app = make_app(&[]);
            open_settings_overlay(&mut app);

            let rendered = buffer_text_sized(&mut app, 100, 18);

            assert!(rendered.contains("Settings"));
            assert!(
                rendered.contains("1 of"),
                "settings should render the framework-owned overflow marker"
            );
        }

        #[test]
        fn clean_confirm_popup_shows_resolved_out_of_tree_target_dir() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);

            let custom_target = tmp.path().join("out-of-tree-target");
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .upsert(WorkspaceMetadata {
                    workspace_root:           AbsolutePath::from(project_dir.clone()),
                    target_directory:         AbsolutePath::from(custom_target.clone()),
                    packages:                 std::collections::HashMap::new(),
                    fingerprint:              ManifestFingerprint {
                        manifest:       FileStamp {
                            content_hash: [0_u8; 32],
                        },
                        lockfile:       None,
                        rust_toolchain: None,
                        configs:        std::collections::BTreeMap::new(),
                    },
                    out_of_tree_target_bytes: None,
                });

            app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(project_dir)));
            let rendered = buffer_text(&mut app);

            assert!(
                rendered.contains("Run cargo clean?"),
                "prompt line still renders"
            );
            let expected = project::home_relative_path(custom_target.as_path());
            assert!(
                rendered.contains(&expected),
                "resolved out-of-tree target dir is shown in the popup (expected {expected:?})"
            );
        }

        fn upsert_fake_package_metadata(
            app: &App,
            project_dir: &Path,
            license: Option<&str>,
            homepage: Option<&str>,
            repository: Option<&str>,
        ) {
            let root = AbsolutePath::from(project_dir);
            let manifest = AbsolutePath::from(project_dir.join("Cargo.toml"));
            let pkg_id = PackageId {
                repr: "demo-id".into(),
            };
            let pkg = PackageRecord {
                name:          "demo".into(),
                version:       Version::new(0, 1, 0),
                edition:       "2021".into(),
                description:   None,
                license:       license.map(String::from),
                homepage:      homepage.map(String::from),
                repository:    repository.map(String::from),
                manifest_path: manifest,
                targets:       Vec::new(),
                publish:       PublishPolicy::Any,
            };
            let mut packages = std::collections::HashMap::new();
            packages.insert(pkg_id, pkg);
            let workspace_metadata = WorkspaceMetadata {
                workspace_root: root,
                target_directory: AbsolutePath::from(project_dir.join("target")),
                packages,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            };
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .upsert(workspace_metadata);
        }

        #[test]
        fn package_pane_renders_metadata_edition_license_homepage_repository() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);
            upsert_fake_package_metadata(
                &app,
                &project_dir,
                Some("MIT"),
                Some("a.test/hp"),
                Some("a.test/rp"),
            );
            // Use a taller backend so the package pane's full field list
            // fits without scrolling — recent steps added rows (Targets /
            // Lint / CI / Disk breakdown) ahead of Edition..Repository.
            let rendered = buffer_text_sized(&mut app, 120, 80);

            // All four Step-4 field labels must be present when their
            // corresponding value is populated (edition is always set by
            // the fake metadata). Value fragments are kept short to fit
            // the test backend's 120-column layout once the package pane
            // has split off its allotted share.
            for label in ["Edition", "License", "Homepage", "Repository"] {
                assert!(
                    rendered.contains(label),
                    "{label} label missing from rendered package pane"
                );
            }
            assert!(rendered.contains("2021"), "edition value (2021) missing");
            assert!(rendered.contains("MIT"), "license value missing");
            assert!(rendered.contains("a.test/hp"), "homepage value missing");
            assert!(rendered.contains("a.test/rp"), "repository value missing");
        }

        #[test]
        fn package_pane_renders_em_dash_for_missing_metadata_fields() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);
            upsert_fake_package_metadata(&app, &project_dir, None, None, None);
            // Taller backend so the full field list fits — see the sibling
            // test for why the default 120×40 isn't enough here.
            let rendered = buffer_text_sized(&mut app, 120, 80);

            // Absent manifest fields render as `—`. Count dashes in the
            // rendered screen — license / homepage / repository are all
            // None here, so at least three should show.
            let dash_count = rendered.matches('—').count();
            assert!(
                dash_count >= 3,
                "expected at least 3 em-dash placeholders for missing \
                 license/homepage/repository, got {dash_count}"
            );
        }

        #[test]
        fn package_pane_renders_target_and_non_target_disk_breakdown() {
            // When the walker has reported the breakdown, the Package
            // pane shows two rows beneath `Disk` — `target/` and `other` —
            // so the user can see at a glance which half of their disk is
            // build artifact vs source. Uses the bytes reported by
            // handle_bg_msg::DiskUsageBatch.
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);

            // Stage a disk-usage batch with a clearly-split breakdown.
            // 10 MiB target, 2 MiB source: assert both lines render with
            // distinct byte formatting.
            let abs_path = AbsolutePath::from(project_dir);
            let sizes = DirSizes {
                total:                 12 * 1024 * 1024,
                in_project_target:     10 * 1024 * 1024,
                in_project_non_target: 2 * 1024 * 1024,
                max_source_mtime:      None,
            };
            app.handle_bg_msg(BackgroundMsg::DiskUsageBatch {
                root_path: abs_path.clone(),
                entries:   vec![(abs_path, sizes)],
            });

            let rendered = buffer_text(&mut app);
            assert!(
                rendered.contains("target/"),
                "detail pane must surface the target/ breakdown label"
            );
            assert!(
                rendered.contains("other"),
                "detail pane must surface the non-target (other) breakdown label"
            );
            assert!(
                rendered.contains("10.0 MiB"),
                "in-target value renders using format_bytes"
            );
            assert!(
                rendered.contains("2.0 MiB"),
                "non-target value renders using format_bytes"
            );
        }

        #[test]
        fn package_pane_renders_out_of_tree_target_size_for_sharer() {
            // When the workspace's target_directory sits outside
            // workspace_root (e.g. redirected via CARGO_TARGET_DIR or an
            // ancestor .cargo/config.toml), the per-project walker can't
            // reach it. The cached walk fills in the sharer target size,
            // which shows up beneath Disk as `target/ (out of tree)`.
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            let shared_target = tmp.path().join("shared-target");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);

            let root = AbsolutePath::from(project_dir);
            let target = AbsolutePath::from(shared_target);
            {
                let store = app.scan.metadata_store_handle();
                let mut guard = store.lock().expect("lock test store");
                guard.upsert(WorkspaceMetadata {
                    workspace_root:           root,
                    target_directory:         target,
                    packages:                 std::collections::HashMap::new(),
                    fingerprint:              ManifestFingerprint {
                        manifest:       FileStamp {
                            content_hash: [0_u8; 32],
                        },
                        lockfile:       None,
                        rust_toolchain: None,
                        configs:        std::collections::BTreeMap::new(),
                    },
                    out_of_tree_target_bytes: Some(42 * 1024 * 1024),
                });
            }

            let rendered = buffer_text(&mut app);
            assert!(
                rendered.contains("out of tree"),
                "sharer detail pane must surface the out-of-tree target label"
            );
            assert!(
                rendered.contains("42.0 MiB"),
                "out-of-tree target size renders using format_bytes"
            );
        }

        /// Helper for the shared-target popup tests: stage two project
        /// metadata "arrivals" pointing at the same `target_directory`,
        /// so the `TargetDirIndex` reports sibling B when we confirm a
        /// clean on A.
        fn upsert_shared_target_metadata(
            app: &mut App,
            primary_dir: &Path,
            sibling_dirs: &[&Path],
            target_dir: &Path,
        ) {
            for dir in std::iter::once(primary_dir).chain(sibling_dirs.iter().copied()) {
                let root = AbsolutePath::from(dir);
                let manifest = AbsolutePath::from(dir.join("Cargo.toml"));
                let pkg_name = dir
                    .file_name()
                    .map_or_else(|| "demo".to_string(), |n| n.to_string_lossy().into_owned());
                let pkg_id = PackageId {
                    repr: format!("{pkg_name}-id"),
                };
                let pkg = PackageRecord {
                    name:          pkg_name,
                    version:       Version::new(0, 1, 0),
                    edition:       "2021".into(),
                    description:   None,
                    license:       None,
                    homepage:      None,
                    repository:    None,
                    manifest_path: manifest,
                    targets:       Vec::new(),
                    publish:       PublishPolicy::Any,
                };
                let mut packages = std::collections::HashMap::new();
                packages.insert(pkg_id, pkg);
                let workspace_metadata = WorkspaceMetadata {
                    workspace_root: root.clone(),
                    target_directory: AbsolutePath::from(target_dir),
                    packages,
                    fingerprint: ManifestFingerprint {
                        manifest:       FileStamp {
                            content_hash: [0_u8; 32],
                        },
                        lockfile:       None,
                        rust_toolchain: None,
                        configs:        std::collections::BTreeMap::new(),
                    },
                    out_of_tree_target_bytes: None,
                };
                // Route through handle_bg_msg so the TargetDirIndex gets
                // refreshed alongside the store (Step 6c handler path).
                let store = app.scan.metadata_store_handle();
                let generation = store
                    .lock()
                    .expect("lock test store")
                    .next_generation(&root);
                app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                    workspace_root: root,
                    generation,
                    fingerprint: workspace_metadata.fingerprint.clone(),
                    result: Ok(workspace_metadata),
                });
            }
        }

        #[test]
        fn clean_confirm_popup_lists_affected_siblings_on_shared_target() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("main");
            let sibling_dir = tmp.path().join("feat");
            let target_dir = tmp.path().join("shared-target");
            for dir in [&primary_dir, &sibling_dir] {
                std::fs::create_dir_all(dir).expect("create test directory");
            }
            std::fs::create_dir_all(&target_dir).expect("create test directory");

            let mut app = make_app(&[
                make_package("main", &primary_dir),
                make_package("feat", &sibling_dir),
            ]);
            upsert_shared_target_metadata(
                &mut app,
                &primary_dir,
                &[sibling_dir.as_path()],
                &target_dir,
            );

            app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(primary_dir)));
            let rendered = buffer_text(&mut app);

            assert!(
                rendered.contains("Also affects:"),
                "shared-target popup should label the collateral list"
            );
            let sibling_label = project::home_relative_path(sibling_dir.as_path());
            assert!(
                rendered.contains(&sibling_label),
                "sibling path should appear in the affected list (expected {sibling_label:?})"
            );
        }

        #[test]
        fn clean_confirm_popup_falls_back_to_in_tree_target_without_metadata() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");
            let mut app = make_app(&[make_package("demo", &project_dir)]);

            app.set_confirm(ConfirmAction::Clean(AbsolutePath::from(
                project_dir.clone(),
            )));
            let rendered = buffer_text(&mut app);

            let fallback_target = project_dir.join("target");
            let expected = project::home_relative_path(fallback_target.as_path());
            assert!(
                rendered.contains(&expected),
                "without metadata, popup shows the default <project>/target (expected {expected:?})"
            );
        }

        #[test]
        fn clean_group_confirm_popup_lists_all_checkouts() {
            // Selecting Clean on a worktree-group root should open the
            // confirm popup with every checkout listed — the UX regression
            // was that the WorktreeGroup arm was stubbed out so the popup
            // never appeared at all.
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary = tmp.path().join("main");
            let linked_a = tmp.path().join("feat-a");
            let linked_b = tmp.path().join("feat-b");
            for dir in [&primary, &linked_a, &linked_b] {
                std::fs::create_dir_all(dir).expect("create test directory");
            }

            let mut app = make_app(&[]);
            app.set_confirm(ConfirmAction::CleanGroup {
                primary: AbsolutePath::from(primary.clone()),
                linked:  vec![
                    AbsolutePath::from(linked_a.clone()),
                    AbsolutePath::from(linked_b.clone()),
                ],
            });
            let rendered = buffer_text_sized(&mut app, 160, 40);

            assert!(
                rendered.contains("Run cargo clean on all checkouts?"),
                "group confirm uses the fan-out prompt"
            );
            assert!(
                rendered.contains("Checkouts:"),
                "group confirm labels the checkout list"
            );
            for dir in [&primary, &linked_a, &linked_b] {
                let label = project::home_relative_path(dir.as_path());
                assert!(
                    rendered.contains(&label),
                    "every checkout appears in the popup (expected {label:?})"
                );
            }
        }

        // ── Output-pane linewise yank ─────────────────────────────────────

        /// Test clipboard backend that records the most recent write.
        #[derive(Default)]
        struct RecordingClipboard {
            written: Option<String>,
        }

        impl ClipboardBackend for RecordingClipboard {
            fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError> {
                self.written = Some(text.to_string());
                Ok(())
            }
        }

        /// Open the output pane with `lines` and render once so the viewport
        /// syncs to the streaming tail (the realistic open state).
        fn open_output(app: &mut App, lines: &[&str]) {
            app.set_example_output(lines.iter().map(|line| (*line).to_string()).collect());
            let _ = buffer_text_sized(app, 120, 48);
        }

        #[test]
        fn output_row_click_selects_clicked_line() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Opening follows the tail, so the cursor starts on the last row.
            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert!(app.panes.output.is_following());

            let (x, y) = output_point(&app, 1);
            click(&mut app, x, y);

            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert_eq!(
                app.panes.output.viewport.pos(),
                1,
                "clicking the second output line selects row index 1",
            );
            assert!(
                !app.panes.output.is_following(),
                "selecting an interior row freezes the view off the tail",
            );
        }

        #[test]
        fn output_drag_selects_the_line_range_and_yanks_it() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Press on "beta" (row 1) positions the cursor; dragging to "delta"
            // (row 3) enters visual mode anchored at the press row and grows the
            // range to cover the pointer.
            let (x1, y1) = output_point(&app, 1);
            click(&mut app, x1, y1);
            let (x3, y3) = output_point(&app, 3);
            drag(&mut app, x3, y3);

            assert!(app.panes.output.selection().is_visual());
            assert_eq!(output_range(&app), Some((1, 3)));

            // Dragging back up past the anchor flips the range without losing it.
            let (x0, y0) = output_point(&app, 0);
            drag(&mut app, x0, y0);
            assert_eq!(output_range(&app), Some((0, 1)));

            // Drag down again, then yank the selected lines.
            drag(&mut app, x3, y3);
            assert_eq!(output_range(&app), Some((1, 3)));

            let mut clipboard = RecordingClipboard::default();
            app.copy_focused_selection_with_backend(&mut clipboard);
            assert_eq!(clipboard.written.as_deref(), Some("beta\ngamma\ndelta"));
        }

        #[test]
        fn output_click_after_drag_clears_the_selection_to_the_clicked_line() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Drag out a multi-line range.
            let (x1, y1) = output_point(&app, 1);
            click(&mut app, x1, y1);
            let (x3, y3) = output_point(&app, 3);
            drag(&mut app, x3, y3);
            assert!(app.panes.output.selection().is_visual());
            assert_eq!(output_range(&app), Some((1, 3)));

            // A fresh click (no drag) collapses the range to just the clicked
            // line and leaves visual mode, so it does not extend from the old
            // anchor.
            let (x0, y0) = output_point(&app, 0);
            click(&mut app, x0, y0);
            assert!(!app.panes.output.selection().is_visual());
            assert_eq!(output_range(&app), Some((0, 0)));

            // Dragging again anchors at the new click, not the stale one.
            let (x2, y2) = output_point(&app, 2);
            drag(&mut app, x2, y2);
            assert_eq!(output_range(&app), Some((0, 2)));
        }

        #[test]
        fn output_drag_ignored_when_output_not_focused() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);

            // Focus elsewhere; a stray drag must not start an output selection.
            app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
            let (x, y) = output_point(&app, 0);
            drag(&mut app, x, y);

            assert!(!app.panes.output.selection().is_visual());
        }

        /// Regression: with the diagnostics panes shown first (recording their
        /// content area), switching to Output and clicking must land on Output —
        /// the hidden Lints/CiRuns rects are reset each frame so they cannot
        /// claim the click.
        #[test]
        fn output_click_does_not_hit_stale_diagnostics_rect() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);

            // Render once with output empty so the bottom row shows Lints/CiRuns
            // and they record a content area over the bottom strip.
            let _ = buffer_text_sized(&mut app, 120, 40);

            // Now show output (hiding Lints/CiRuns) and click an output line.
            open_output(&mut app, &["alpha", "beta", "gamma"]);
            let (x, y) = output_point(&app, 0);
            click(&mut app, x, y);

            assert_eq!(
                app.focused_pane_id(),
                PaneId::Output,
                "the click must focus Output, not a hidden diagnostics pane",
            );
            assert_eq!(app.panes.output.viewport.pos(), 0);
        }

        #[test]
        fn output_toggle_visual_enters_and_leaves_visual_mode() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);

            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert!(app.panes.output.is_following());

            // There is always a selection — at rest the single cursor row.
            assert_eq!(output_count(&app), 1);

            // Entering vim visual-line mode anchors on the cursor row.
            let live = app.inflight.example_output().to_vec();
            app.panes.output.toggle_visual(&live);
            assert!(app.panes.output.selection().is_visual());
            assert_eq!(output_count(&app), 1);

            // Toggling again leaves visual mode, collapsing to the cursor row.
            let live = app.inflight.example_output().to_vec();
            app.panes.output.toggle_visual(&live);
            assert!(!app.panes.output.selection().is_visual());
            assert_eq!(output_count(&app), 1);
        }

        #[test]
        fn output_v_is_inert_without_vim() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);

            // `V` is the vim affordance; with vim navigation off it does nothing.
            press_key(&mut app, KeyCode::Char('V'));
            assert!(!app.panes.output.selection().is_visual());
            assert!(app.panes.output.is_following());
            assert_eq!(output_count(&app), 1);
        }

        #[test]
        fn output_selection_extends_and_yanks_against_snapshot() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            press_shift_key(&mut app, KeyCode::Up); // extend up one row, freezing
            press_shift_key(&mut app, KeyCode::Up); // extend up another row

            assert_eq!(output_range(&app), Some((2, 4)));

            // Streaming output after the snapshot must not drift the frozen range.
            app.inflight
                .apply_example_progress("epsilon-updated".to_string());
            app.inflight.example_output_mut().push("zeta".to_string());

            let mut clipboard = RecordingClipboard::default();
            app.copy_focused_selection_with_backend(&mut clipboard);

            assert_eq!(clipboard.written.as_deref(), Some("gamma\ndelta\nepsilon"));
            assert!(
                app.panes.output.is_following(),
                "a yank collapses back to following the tail",
            );
            assert_eq!(output_count(&app), 1);
        }

        #[test]
        fn output_ctrl_a_selects_all_lines_and_yanks_them() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta"]);

            input::handle_event(
                &mut app,
                &Event::Key(KeyEvent {
                    code:      KeyCode::Char('a'),
                    modifiers: KeyModifiers::CONTROL,
                    kind:      KeyEventKind::Press,
                    state:     crossterm::event::KeyEventState::NONE,
                }),
            );

            assert_eq!(
                output_range(&app),
                Some((0, 3)),
                "Ctrl-A selects every line",
            );
            assert_eq!(output_count(&app), 4, "the selection spans every line");

            let mut clipboard = RecordingClipboard::default();
            app.copy_focused_selection_with_backend(&mut clipboard);
            assert_eq!(
                clipboard.written.as_deref(),
                Some("alpha\nbeta\ngamma\ndelta"),
            );
        }

        #[test]
        fn output_esc_collapses_vim_visual_to_the_cursor_row() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app_vim(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Enter visual mode at the tail, then extend up one row.
            press_key(&mut app, KeyCode::Char('V'));
            press_key(&mut app, KeyCode::Up);
            let _ = buffer_text_sized(&mut app, 120, 40);
            assert_eq!(output_range(&app), Some((3, 4)));

            // Esc leaves visual mode, collapsing the selection back to the single
            // cursor row where the user was reading — not snapping to the tail.
            press_key(&mut app, KeyCode::Esc);
            assert!(!app.panes.output.selection().is_visual());
            assert_eq!(output_count(&app), 1);
            assert_eq!(
                app.panes.output.viewport.pos(),
                3,
                "collapse leaves the cursor where the visual range ended",
            );
            assert!(
                !app.panes.output.is_following(),
                "the view stays where the user was reading, not at the tail",
            );
        }

        #[test]
        fn output_shift_arrows_grow_the_selection_from_the_cursor_row() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Opening follows the tail; the at-rest selection is the cursor row.
            assert!(app.panes.output.is_following());
            assert_eq!(output_count(&app), 1);

            // Shift+Up grows the selection upward from the anchor (no vim needed).
            press_shift_key(&mut app, KeyCode::Up);
            assert_eq!(
                output_range(&app),
                Some((3, 4)),
                "the selection spans the anchor row and the row above",
            );
            assert!(
                !app.panes.output.is_following(),
                "extending the selection freezes the view off the tail",
            );

            // Shift+Down shrinks it back toward the anchor.
            press_shift_key(&mut app, KeyCode::Down);
            assert_eq!(output_range(&app), Some((4, 4)));
        }

        #[test]
        fn shift_arrows_do_nothing_outside_the_output_pane() {
            let mut app = make_app(&[
                make_package("first", Path::new("/tmp/first")),
                make_package("second", Path::new("/tmp/second")),
            ]);
            app.set_focus(FocusedPane::App(AppPaneId::ProjectList));
            let _ = buffer_text_sized(&mut app, 120, 40);
            app.project_list.move_down();
            assert_eq!(app.project_list.cursor(), 1);

            // Shift+arrows are an output-only gesture; in other panes they are
            // inert (they would only duplicate the plain arrow navigation).
            press_shift_key(&mut app, KeyCode::Up);
            assert_eq!(
                app.project_list.cursor(),
                1,
                "Shift+Up is inert outside Output",
            );
            press_shift_key(&mut app, KeyCode::Down);
            assert_eq!(
                app.project_list.cursor(),
                1,
                "Shift+Down is inert outside Output",
            );
        }

        #[test]
        fn output_ctrl_shift_up_selects_from_the_cursor_to_the_top() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // Park the cursor on an interior row; the at-rest selection follows it.
            press_key(&mut app, KeyCode::Up);
            press_key(&mut app, KeyCode::Up);
            assert_eq!(app.panes.output.viewport.pos(), 2);
            assert_eq!(output_count(&app), 1);

            // Ctrl+Shift+Up extends the selection from here to row 0.
            press_ctrl_shift_key(&mut app, KeyCode::Up);
            assert_eq!(output_range(&app), Some((0, 2)));
        }

        #[test]
        fn output_ctrl_shift_down_selects_from_the_cursor_to_the_bottom() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            press_key(&mut app, KeyCode::Up);
            press_key(&mut app, KeyCode::Up);
            assert_eq!(app.panes.output.viewport.pos(), 2);

            // Ctrl+Shift+Down extends the selection from here to the last row.
            press_ctrl_shift_key(&mut app, KeyCode::Down);
            assert_eq!(output_range(&app), Some((2, 4)));
        }

        #[test]
        fn output_shift_arrows_extend_and_shrink_an_active_selection() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma", "delta", "epsilon"]);

            // The at-rest selection is the tail row; grow it upward with Shift+Up.
            assert_eq!(output_range(&app), Some((4, 4)));

            press_shift_key(&mut app, KeyCode::Up);
            press_shift_key(&mut app, KeyCode::Up);
            assert_eq!(
                output_range(&app),
                Some((2, 4)),
                "Shift+Up extends the selection from the anchor",
            );

            // Shift+Down shrinks it back toward the anchor.
            press_shift_key(&mut app, KeyCode::Down);
            assert_eq!(
                output_range(&app),
                Some((3, 4)),
                "Shift+Down shrinks the selection",
            );
        }

        #[test]
        fn output_esc_collapses_vim_visual_before_stopping_the_run() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app_vim(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);
            // A process is still streaming while the user enters visual mode.
            app.inflight.set_example_running(Some("demo".to_string()));

            press_key(&mut app, KeyCode::Char('V'));
            assert!(app.panes.output.selection().is_visual());

            // First Esc leaves visual mode — it must NOT kill the run.
            press_key(&mut app, KeyCode::Esc);
            assert!(!app.panes.output.selection().is_visual());
            assert!(
                app.inflight.example_running().is_some(),
                "leaving visual mode must not stop the running process",
            );

            // Second Esc stops the run and records a single kill marker.
            press_key(&mut app, KeyCode::Esc);
            assert!(app.inflight.example_running().is_none());
            assert_eq!(
                app.inflight.example_output().last().map(String::as_str),
                Some("── killed ──"),
            );
        }

        #[test]
        fn output_title_shows_visual_hint_even_while_running() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app_vim(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);
            app.inflight.set_example_running(Some("demo".to_string()));

            // At rest (collapsed, following), the title advertises the run.
            let running = buffer_text_sized(&mut app, 120, 40);
            assert!(
                running.contains("Running: demo"),
                "title shows the running process before entering visual mode",
            );

            press_key(&mut app, KeyCode::Char('V'));
            let visual = buffer_text_sized(&mut app, 120, 40);
            assert!(
                visual.contains("visual") && visual.contains("y copy"),
                "pressing V switches the title to the visual hint even while running",
            );
        }

        #[test]
        fn output_title_keeps_target_path_after_run_finishes() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha"]);
            app.inflight
                .set_example_title(Some("workspace/member/smoke".to_string()));
            app.inflight
                .set_example_running(Some("workspace/member/smoke (dev)".to_string()));

            let running = buffer_text_sized(&mut app, 120, 40);
            assert!(
                running.contains("Running: workspace/member/smoke"),
                "running title includes target path",
            );

            app.inflight.set_example_running(None);
            let finished = buffer_text_sized(&mut app, 120, 40);
            assert!(
                finished.contains("Output: workspace/member/smoke"),
                "finished output title keeps target path",
            );
        }

        #[test]
        fn output_yank_strips_ansi_from_selection() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["\u{1b}[31mred line\u{1b}[0m"]);

            // The at-rest selection is the cursor row; yank copies it ANSI-stripped.
            let mut clipboard = RecordingClipboard::default();
            app.copy_focused_selection_with_backend(&mut clipboard);

            assert_eq!(clipboard.written.as_deref(), Some("red line"));
        }

        #[test]
        fn output_render_drops_non_sgr_escape_sequences() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(
                &mut app,
                &["before \u{1b}[6nafter", "start \u{1b}Pignored\u{1b}\\end"],
            );

            let text = buffer_text_sized(&mut app, 120, 40);
            assert!(text.contains("before after"));
            assert!(text.contains("start end"));
            assert!(!text.contains('\u{1b}'));
            assert!(!text.contains("[6n"));
            assert!(!text.contains("ignored"));
        }

        #[test]
        fn output_vim_esc_collapses_then_a_second_esc_closes_the_pane() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app_vim(&[project]);
            open_output(&mut app, &["alpha", "beta"]);

            // Enter visual mode, then extend so the selection spans two rows.
            press_key(&mut app, KeyCode::Char('V'));
            press_key(&mut app, KeyCode::Up);
            assert!(app.panes.output.selection().is_visual());

            // First Esc leaves visual mode without closing the pane.
            press_key(&mut app, KeyCode::Esc);
            assert!(!app.panes.output.selection().is_visual());
            assert!(
                !app.inflight.example_output().is_empty(),
                "the first Esc only leaves visual mode, not the pane",
            );
            assert_eq!(app.focused_pane_id(), PaneId::Output);

            // Second Esc closes the pane.
            press_key(&mut app, KeyCode::Esc);
            assert!(
                app.inflight.example_output().is_empty(),
                "the second Esc closes the pane",
            );
            assert_eq!(app.focused_pane_id(), PaneId::Targets);
        }

        #[test]
        fn focused_output_selection_row_highlight_fills_full_width() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta", "gamma"]);

            assert_eq!(app.focused_pane_id(), PaneId::Output);
            assert!(
                app.panes.output.is_following(),
                "cursor sits on the tail row"
            );

            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| render::ui(frame, &mut app))
                .expect("draw test frame");
            let buffer = terminal.backend().buffer().clone();
            let area = buffer.area;

            // Find the tail row ("gamma") — the one-line selection while following.
            let mut cursor_row = None;
            for y in 0..area.height {
                let row: String = (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                if let Some(col) = row.find("gamma") {
                    cursor_row = Some((u16::try_from(col).unwrap_or(0), y));
                    break;
                }
            }
            let (text_col, y) = cursor_row.expect("the tail row is rendered");

            // A cell well past the 5-char text must carry the selection
            // background — the cursor row is a one-line selection, drawn in the
            // single selection color — so the highlight spans the full width.
            let probe = buffer[(text_col + 30, y)].bg;
            assert_eq!(
                probe,
                tui_pane::finder_match_bg(),
                "selection row highlight should fill the full pane width",
            );
        }

        #[test]
        fn selection_row_highlight_covers_ansi_colored_log_text() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            // A green ANSI segment followed by plain text — the colored span
            // carries its own background, which must not punch a hole in the
            // row highlight.
            open_output(&mut app, &["\u{1b}[32mINFO\u{1b}[0m starting up"]);

            assert_eq!(app.focused_pane_id(), PaneId::Output);

            let backend = TestBackend::new(120, 40);
            let mut terminal = Terminal::new(backend).expect("create test terminal");
            terminal
                .draw(|frame| render::ui(frame, &mut app))
                .expect("draw test frame");
            let buffer = terminal.backend().buffer().clone();
            let area = buffer.area;

            let mut info_cell = None;
            for y in 0..area.height {
                let row: String = (0..area.width)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect();
                if let Some(col) = row.find("INFO") {
                    info_cell = Some((u16::try_from(col).unwrap_or(0), y));
                    break;
                }
            }
            let (col, y) = info_cell.expect("the colored log line is rendered");

            // The 'I' of the green "INFO" must carry the selection-row background
            // (the cursor row is a one-line selection), not the bare default
            // behind the colored glyph.
            assert_eq!(
                buffer[(col, y)].bg,
                tui_pane::finder_match_bg(),
                "the highlight must cover the ANSI-colored text, not just the padding",
            );
            // And the green foreground survives the highlight.
            assert_eq!(buffer[(col, y)].fg, ratatui::style::Color::Green);
        }

        #[test]
        fn overlaid_output_steals_focus_from_hidden_diagnostics_pane() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta"]);

            // Simulate the stale focus left when a pane that the Output layout
            // hides was focused before output appeared.
            app.set_focus_to_pane(PaneId::CiRuns);
            assert_eq!(app.focused_pane_id(), PaneId::CiRuns);

            // Rendering reconciles focus to the visible bottom-row pane.
            let _ = buffer_text_sized(&mut app, 120, 40);
            assert_eq!(
                app.focused_pane_id(),
                PaneId::Output,
                "focus must not stay on a pane the Output overlay hides",
            );
        }

        #[test]
        fn closing_output_releases_focus_to_a_visible_pane() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta"]);
            assert_eq!(app.focused_pane_id(), PaneId::Output);

            // Output emptied without the Esc-close path redirecting focus.
            app.inflight.example_output_mut().clear();

            let _ = buffer_text_sized(&mut app, 120, 40);
            assert_eq!(
                app.focused_pane_id(),
                PaneId::Targets,
                "focus must not stay on the Output pane once it is hidden",
            );
        }

        #[test]
        fn output_yank_copies_the_cursor_row_by_default() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["alpha", "beta"]);

            // The cursor row is always a one-line selection, so a yank copies it
            // (here the followed tail row) rather than copying nothing.
            let mut clipboard = RecordingClipboard::default();
            app.copy_focused_selection_with_backend(&mut clipboard);

            assert_eq!(clipboard.written.as_deref(), Some("beta"));
        }

        #[test]
        fn output_scroll_up_freezes_and_end_resumes_follow() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["a", "b", "c", "d", "e"]);

            assert!(app.panes.output.is_following());
            press_key(&mut app, KeyCode::Up);
            assert!(
                !app.panes.output.is_following(),
                "scrolling up freezes the view",
            );
            press_key(&mut app, KeyCode::End);
            assert!(
                app.panes.output.is_following(),
                "End resumes following the tail",
            );
        }

        #[test]
        fn output_process_exit_holds_a_range_but_resumes_when_collapsed() {
            let project = make_package("demo", Path::new("/tmp/demo"));
            let mut app = make_app(&[project]);
            open_output(&mut app, &["a", "b", "c"]);

            // Collapsed but scrolled up: a process exit snaps back to the tail so
            // the final output shows.
            press_key(&mut app, KeyCode::Up);
            assert!(!app.panes.output.is_following());
            app.panes.output.on_process_exit();
            assert!(
                app.panes.output.is_following(),
                "exit resumes follow when the selection is a single collapsed row",
            );

            // A multi-row selection (the user is copying): exit must leave it put.
            let live = app.inflight.example_output().to_vec();
            app.panes.output.select_extend_up(&live);
            assert_eq!(output_count(&app), 2);
            app.panes.output.on_process_exit();
            assert!(
                output_count(&app) >= 2,
                "exit must not collapse a range the user is selecting",
            );
            assert!(
                !app.panes.output.is_following(),
                "exit must not resume follow while a range holds the view",
            );
        }
    }
    mod panes {
        use cargo_metadata::PackageId;
        use cargo_metadata::TargetKind;
        use cargo_metadata::semver::Version;
        use crossterm::event::Event;
        use crossterm::event::KeyCode;
        use crossterm::event::KeyEvent;
        use crossterm::event::KeyModifiers;
        use tui_pane::GlobalAction;

        use super::*;
        use crate::config::LintIndicator;
        use crate::lint::LintRun;
        use crate::lint::LintRunStatus;
        use crate::project::Cargo;
        use crate::project::ExampleGroup;
        use crate::project::FileStamp;
        use crate::project::ManifestFingerprint;
        use crate::project::PackageRecord;
        use crate::project::ProjectInfo;
        use crate::project::ProjectType;
        use crate::project::PublishPolicy;
        use crate::project::PublishStatus;
        use crate::project::Submodule;
        use crate::project::TargetRecord;
        use crate::project::WorkspaceMetadata;
        use crate::project::WorktreeHealth::Normal;
        use crate::tui::app::startup;
        use crate::tui::columns;
        use crate::tui::columns::ProjectRow;
        use crate::tui::columns::RowLifecycle;
        use crate::tui::input;

        fn test_submodule(name: &str, path: &str) -> Submodule {
            Submodule {
                name:          name.to_string(),
                path:          test_path(path),
                relative_path: name.to_string(),
                url:           None,
                branch:        None,
                commit:        None,
                project_info:  ProjectInfo::default(),
                git_repo:      None,
            }
        }

        fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
            input::handle_event(app, &Event::Key(KeyEvent::new(code, modifiers)));
        }

        #[test]
        fn collapse_all_anchors_member_selection_to_root() {
            let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
            let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
            let root = make_workspace_with_members(
                Some("hana"),
                "~/rust/hana",
                vec![inline_group(vec![make_member(
                    Some("hana_core"),
                    "~/rust/hana/crates/hana_core",
                )])],
            );

            let mut app = make_app(&[workspace, member.clone()]);
            apply_items(&mut app, &[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.project_list
                .select_project_in_tree(member.path(), false);

            app.project_list.collapse_all(false);

            assert_eq!(
                app.project_list.selected_row(),
                Some(VisibleRow::Root { node_index: 0 })
            );
        }

        #[test]
        fn expand_all_preserves_selected_project_path() {
            let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
            let member = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
            let root = make_workspace_with_members(
                Some("hana"),
                "~/rust/hana",
                vec![inline_group(vec![make_member(
                    Some("hana_core"),
                    "~/rust/hana/crates/hana_core",
                )])],
            );

            let mut app = make_app(&[workspace, member.clone()]);
            apply_items(&mut app, &[root]);
            app.project_list
                .select_project_in_tree(member.path(), false);
            app.project_list.collapse_all(false);

            app.project_list.expand_all(false);

            assert_eq!(
                app.project_list.selected_project_path(),
                Some(member.path().as_path())
            );
        }

        #[test]
        fn name_width_with_gutter_reserves_space_before_lint() {
            assert_eq!(crate::tui::panes::name_width_with_gutter(0), 1);
            assert_eq!(crate::tui::panes::name_width_with_gutter(42), 43);
        }

        #[test]
        fn workspace_structure_counts_tree_children_and_cargo_targets() {
            let mut core = make_package_with_vendored(
                Some("core"),
                "~/ws/crates/core",
                vec![make_vendored(
                    Some("helper"),
                    "~/ws/crates/core/vendor/helper",
                )],
            );
            core.rust.cargo = Cargo {
                types:          vec![ProjectType::Library],
                examples:       vec![ExampleGroup {
                    category: String::new(),
                    names:    vec!["demo".to_string()],
                }],
                benches:        Vec::new(),
                publish_status: PublishStatus::NotPublishable,
            };
            let mut cli = make_member(Some("cli"), "~/ws/crates/cli");
            cli.rust.cargo = Cargo {
                types:          vec![ProjectType::Binary],
                examples:       Vec::new(),
                benches:        Vec::new(),
                publish_status: PublishStatus::NotPublishable,
            };

            let mut root = make_workspace_with_members(
                Some("ws"),
                "~/ws",
                vec![inline_group(vec![core, cli])],
            );
            let RootItem::Rust(RustProject::Workspace(ws)) = &mut root else {
                unreachable!("test root should be a workspace");
            };
            ws.rust.cargo = Cargo {
                types:          vec![ProjectType::Workspace],
                examples:       Vec::new(),
                benches:        vec!["smoke".to_string()],
                publish_status: PublishStatus::NotPublishable,
            };
            ws.rust.vendored = vec![make_vendored(
                Some("root-helper"),
                "~/ws/vendor/root-helper",
            )];
            ws.rust.project_info.submodules = vec![
                test_submodule("native", "~/ws/native"),
                test_submodule("assets", "~/ws/assets"),
            ];

            let mut app = make_app(std::slice::from_ref(&root));
            apply_items(&mut app, &[root]);
            app.ensure_detail_cached();

            let package = app
                .panes
                .package
                .content()
                .expect("pane should have rendered test content");
            assert_eq!(
                package.stats_rows,
                vec![
                    ("members", 2),
                    ("vendored", 2),
                    ("submodules", 2),
                    ("lib", 1),
                    ("bin", 1),
                    ("example", 1),
                    ("bench", 1),
                ]
            );
        }

        #[test]
        fn package_structure_counts_direct_vendored_children() {
            let mut package = make_package_with_vendored(
                Some("app"),
                "~/app",
                vec![
                    make_vendored(Some("helper-a"), "~/app/vendor/helper-a"),
                    make_vendored(Some("helper-b"), "~/app/vendor/helper-b"),
                ],
            );
            package.rust.cargo = Cargo {
                types:          vec![ProjectType::Binary],
                examples:       Vec::new(),
                benches:        Vec::new(),
                publish_status: PublishStatus::NotPublishable,
            };
            let root = RootItem::Rust(RustProject::Package(package));

            let mut app = make_app(std::slice::from_ref(&root));
            apply_items(&mut app, &[root]);
            app.ensure_detail_cached();

            let package = app
                .panes
                .package
                .content()
                .expect("pane should have rendered test content");
            assert_eq!(package.stats_rows, vec![("vendored", 2), ("bin", 1)]);
        }

        /// Upsert minimal `WorkspaceMetadata` into `app`'s metadata store
        /// for `project_path`, naming a single Example target so the Targets
        /// pane becomes tabbable. Keeps the per-test setup out of line when
        /// the test's focus is pane behavior, not metadata plumbing.
        fn seed_single_example_metadata(
            app: &App,
            project_path: &AbsolutePath,
            example_name: &str,
        ) {
            let pkg_id = PackageId {
                repr: "demo-id".into(),
            };
            let pkg = PackageRecord {
                name:          "demo".into(),
                version:       Version::new(0, 1, 0),
                edition:       "2021".into(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: AbsolutePath::from(project_path.as_path().join("Cargo.toml")),
                targets:       vec![crate::project::TargetRecord {
                    name:              example_name.to_string(),
                    kinds:             vec![TargetKind::Example],
                    required_features: vec![],
                    src_path:          AbsolutePath::from(
                        project_path
                            .as_path()
                            .join(format!("examples/{example_name}.rs")),
                    ),
                }],
                publish:       PublishPolicy::Any,
            };
            let mut packages = std::collections::HashMap::new();
            packages.insert(pkg_id, pkg);
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .upsert(WorkspaceMetadata {
                    workspace_root: project_path.clone(),
                    target_directory: AbsolutePath::from(project_path.as_path().join("target")),
                    packages,
                    fingerprint: ManifestFingerprint {
                        manifest:       FileStamp {
                            content_hash: [0_u8; 32],
                        },
                        lockfile:       None,
                        rust_toolchain: None,
                        configs:        std::collections::BTreeMap::new(),
                    },
                    out_of_tree_target_bytes: None,
                });
        }

        #[test]
        fn tabbable_panes_follow_canonical_order() {
            // Targets pane requires workspace metadata.
            let project_path = test_path("~/demo");
            let project = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".to_string()),
                ..Package::default()
            }));
            let mut app = make_app(std::slice::from_ref(&project));
            seed_single_example_metadata(&app, &project_path, "example");
            app.framework.toasts = tui_pane::Toasts::default();
            app.framework.toasts.viewport.set_len(0);
            app.scan.state.phase = ScanPhase::Complete;
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Unborn,
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: None,
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        None,
                            repo:         Some("demo".to_string()),
                            tracked_ref:  None,
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    None,
                        local_main_branch: None,
                    },
                ),
            );
            app.ensure_detail_cached();
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![make_ci_run(1, CiStatus::Passed)],
                false,
                0,
            );
            app.ensure_detail_cached();

            let expected_without_toasts = app.tabbable_panes();
            assert!(expected_without_toasts.contains(&PaneId::Cpu));
            let cpu_index = expected_without_toasts
                .iter()
                .position(|pane| *pane == PaneId::Cpu)
                .expect("CPU pane should be tabbable");
            let targets_index = expected_without_toasts
                .iter()
                .position(|pane| *pane == PaneId::Targets)
                .expect("Targets pane should be tabbable");
            assert!(cpu_index < targets_index);

            app.show_timed_toast("Settings", "Updated");
            let expected_with_toasts = app.tabbable_panes();

            assert_eq!(
                expected_with_toasts,
                expected_without_toasts
                    .iter()
                    .copied()
                    .chain(std::iter::once(PaneId::Toasts))
                    .collect::<Vec<_>>()
            );

            for &pane in &expected_with_toasts[1..] {
                press(&mut app, KeyCode::Tab, KeyModifiers::NONE);
                assert_eq!(app.focused_pane_id(), pane);
            }
            press(&mut app, KeyCode::Tab, KeyModifiers::SHIFT);
            assert_eq!(
                app.focused_pane_id(),
                expected_with_toasts[expected_with_toasts.len() - 2]
            );
        }

        #[test]
        fn cpu_pane_selection_persists_across_project_changes() {
            let project_a = make_project(Some("a"), "~/a");
            let project_b = make_project(Some("b"), "~/b");
            let mut app = make_app(&[project_a, project_b]);
            app.set_focus_to_pane(PaneId::Cpu);
            app.panes.cpu.viewport.set_pos(1);
            app.project_list.set_cursor(1);

            app.sync_selected_project();

            assert_eq!(app.panes.cpu.viewport.pos(), 1);
        }

        #[test]
        fn new_toasts_do_not_steal_focus() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);
            app.set_focus_to_pane(PaneId::Git);

            app.show_timed_toast("Settings", "Updated");
            assert_eq!(app.focused_pane_id(), PaneId::Git);

            let _ = app
                .framework
                .toasts
                .start_task("Startup lints", "Running startup lint jobs...");
            assert_eq!(app.focused_pane_id(), PaneId::Git);
        }

        #[test]
        fn metadata_arrival_populates_selected_tree_project_targets() {
            // Targets pane data comes exclusively from the `cargo metadata`
            // result. A CargoMetadata arrival with an Example target lights up
            // the pane.

            let project = make_project(Some("demo"), "/never-real/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.scan.state.phase = ScanPhase::Complete;
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            app.ensure_detail_cached();
            let example_count = app.panes.targets.content().map(|d| d.examples.len());
            assert_eq!(
                example_count,
                Some(0),
                "pre-metadata: Targets pane is empty"
            );
            assert!(!app.tabbable_panes().contains(&PaneId::Targets));

            let workspace_root = AbsolutePath::from("/never-real/demo");
            let manifest_path = AbsolutePath::from("/never-real/demo/Cargo.toml");
            let example = TargetRecord {
                name:              "tracked_row_paths".to_string(),
                kinds:             vec![TargetKind::Example],
                required_features: vec![],
                src_path:          AbsolutePath::from(
                    "/never-real/demo/examples/tracked_row_paths.rs",
                ),
            };
            let pkg_id = PackageId {
                repr: "demo-id".into(),
            };
            let pkg = PackageRecord {
                name: "demo".into(),
                version: Version::new(0, 1, 0),
                edition: "2021".into(),
                description: None,
                license: None,
                homepage: None,
                repository: None,
                manifest_path,
                targets: vec![example],
                publish: PublishPolicy::Any,
            };
            let mut packages = std::collections::HashMap::new();
            packages.insert(pkg_id, pkg);
            let workspace_metadata = WorkspaceMetadata {
                workspace_root: workspace_root.clone(),
                target_directory: AbsolutePath::from("/never-real/demo/target"),
                packages,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        std::collections::BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            };
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .next_generation(&workspace_root);
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root,
                generation,
                fingerprint: workspace_metadata.fingerprint.clone(),
                result: Ok(workspace_metadata),
            });
            app.ensure_detail_cached();
            let example_count = app.panes.targets.content().map(|d| d.examples.len());
            assert_eq!(
                example_count,
                Some(1),
                "metadata-arrival populates Targets from PackageRecord.targets"
            );
            assert!(app.tabbable_panes().contains(&PaneId::Targets));
        }

        #[test]
        fn first_non_empty_tree_build_focuses_project_list() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_items(&mut app, &[project]);

            assert_eq!(app.focused_pane_id(), PaneId::ProjectList);
            assert_eq!(app.project_list.cursor(), 0);
        }

        #[test]
        fn initial_disk_roots_groups_nested_projects_under_one_root() {
            let projects: Vec<RootItem> = [
                make_project(Some("bevy"), "~/rust/bevy"),
                make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
                make_project(Some("render"), "~/rust/bevy/crates/bevy_render"),
                make_project(Some("hana"), "~/rust/hana"),
                make_project(Some("hana_core"), "~/rust/hana/crates/hana"),
            ]
            .to_vec();

            assert_eq!(
                crate::tui::app::startup::initial_disk_roots(&super::as_entries(projects)).len(),
                2
            );
        }

        #[test]
        fn initial_metadata_roots_collects_every_rust_leaf() {
            // Contrast with `initial_disk_roots`: metadata needs one dispatch per
            // leaf (each Cargo.toml has its own resolved target_directory), not a
            // deduped-by-prefix set.
            let projects: Vec<RootItem> = [
                make_project(Some("bevy"), "~/rust/bevy"),
                make_project(Some("ecs"), "~/rust/bevy/crates/bevy_ecs"),
                make_project(Some("hana"), "~/rust/hana"),
            ]
            .to_vec();

            let roots = startup::initial_metadata_roots(&super::as_entries(projects));
            assert_eq!(roots.len(), 3, "each Rust leaf gets its own metadata root");
        }

        #[test]
        fn initial_metadata_roots_skips_non_rust_leaves() {
            let non_rust = RootItem::NonRust(crate::project::NonRustProject::new(
                super::test_path("~/notes"),
                Some("notes".into()),
            ));
            let pkg = make_project(Some("pkg"), "~/pkg");
            let roots = startup::initial_metadata_roots(&super::as_entries(vec![non_rust, pkg]));
            assert_eq!(roots.len(), 1, "non-rust leaves are not metadata roots");
        }

        #[test]
        fn overlays_restore_prior_focus() {
            let app_project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[app_project]);
            app.set_focus_to_pane(PaneId::Git);

            app.dispatch_framework_global_action(GlobalAction::OpenSettings);
            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(
                app.framework.overlay(),
                Some(tui_pane::FrameworkOverlayId::Settings)
            );

            app.dispatch_framework_global_action(GlobalAction::Dismiss);
            assert_eq!(app.focused_pane_id(), PaneId::Git);
            assert_eq!(app.framework.overlay(), None);
        }

        #[test]
        fn detail_panes_do_not_remember_selection_until_focused() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(&[project]);

            assert_eq!(
                app.pane_focus_state(PaneId::ProjectList),
                PaneFocusState::Active
            );
            assert_eq!(
                app.pane_focus_state(PaneId::Package),
                PaneFocusState::Inactive
            );
            assert_eq!(app.pane_focus_state(PaneId::Git), PaneFocusState::Inactive);
            assert_eq!(
                app.pane_focus_state(PaneId::Targets),
                PaneFocusState::Inactive
            );
            assert_eq!(
                app.pane_focus_state(PaneId::CiRuns),
                PaneFocusState::Inactive
            );

            app.set_focus_to_pane(PaneId::Package);
            app.set_focus_to_pane(PaneId::ProjectList);
            assert_eq!(
                app.pane_focus_state(PaneId::Package),
                PaneFocusState::Remembered
            );
        }

        #[test]
        fn project_change_resets_project_dependent_panes() {
            let project_a = make_project(Some("a"), "~/a");
            let project_b = make_project(Some("b"), "~/b");
            let mut app = make_app(&[project_a, project_b]);

            app.set_focus_to_pane(PaneId::Package);
            app.set_focus_to_pane(PaneId::Git);
            app.set_focus_to_pane(PaneId::Targets);
            app.set_focus_to_pane(PaneId::CiRuns);
            app.panes.package.viewport.set_pos(3);
            app.panes.git.viewport.set_pos(4);
            app.panes.targets.viewport.set_pos(5);
            app.ci.viewport.set_pos(6);
            app.project_list.set_cursor(1);
            app.sync_selected_project();

            assert_eq!(app.panes.package.viewport.pos(), 0);
            assert_eq!(app.panes.git.viewport.pos(), 0);
            assert_eq!(app.panes.targets.viewport.pos(), 0);
            assert_eq!(app.ci.viewport.pos(), 0);
            assert_eq!(
                app.pane_focus_state(PaneId::Package),
                PaneFocusState::Inactive
            );
            assert_eq!(app.pane_focus_state(PaneId::Git), PaneFocusState::Inactive);
            assert_eq!(
                app.pane_focus_state(PaneId::Targets),
                PaneFocusState::Inactive
            );
            assert_eq!(
                app.pane_focus_state(PaneId::CiRuns),
                PaneFocusState::Inactive
            );
            assert_eq!(
                app.project_list
                    .paths
                    .selected_project
                    .as_ref()
                    .map(crate::project::AbsolutePath::as_path),
                app.project_list.selected_project_path()
            );
        }

        #[test]
        fn apply_config_resets_column_layout_flag() {
            let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
            let mut cargo_port_config = CargoPortConfig::default();

            assert!(!app.project_list.cached_fit_widths.lint_enabled());

            cargo_port_config.lint.enabled = LintIndicator::Enabled;
            app.apply_config(&cargo_port_config);
            assert!(app.project_list.cached_fit_widths.lint_enabled());

            cargo_port_config.lint.enabled = LintIndicator::Disabled;
            app.apply_config(&cargo_port_config);
            assert!(!app.project_list.cached_fit_widths.lint_enabled());
        }

        #[test]
        fn zero_byte_update_marks_deleted_child_member() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let workspace_dir = tmp.path().join("hana");
            let member_dir = workspace_dir.join("crates").join("clay-layout");
            std::fs::create_dir_all(&member_dir).expect("create test directory");

            let ws_path = workspace_dir.to_string_lossy().to_string();
            let member_path = member_dir.to_string_lossy().to_string();
            let workspace = make_workspace_project(Some("hana"), &ws_path);
            let member = make_project(Some("clay-layout"), &member_path);

            let root = make_workspace_with_members(
                Some("hana"),
                &ws_path,
                vec![inline_group(vec![make_member(
                    Some("clay-layout"),
                    &member_path,
                )])],
            );

            let mut app = make_app(&[workspace, member]);
            apply_items(&mut app, &[root]);

            std::fs::remove_dir_all(&member_dir).expect("remove test directory");
            app.handle_disk_usage(Path::new(&member_path), 0);
        }

        #[test]
        fn top_level_deleted_project_enters_deleted_state_and_renders_as_deleted() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let project_path = project_dir.to_string_lossy().to_string();
            let project = make_project(Some("demo"), &project_path);
            let mut app = make_app(&[project]);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows().len(),
                1,
                "top-level project should render"
            );

            std::fs::remove_dir_all(&project_dir).expect("remove test directory");
            app.handle_disk_usage(Path::new(&project_path), 0);

            let abs_path = AbsolutePath::from(project_path.clone());
            assert!(
                app.project_list.is_deleted(&abs_path),
                "top-level project should be deleted"
            );
            assert_eq!(
                app.project_list
                    .at_path(&abs_path)
                    .expect("top-level project should still exist in hierarchy")
                    .visibility,
                Deleted
            );

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows().len(),
                1,
                "deleted top-level project should still render before dismiss"
            );

            app.project_list.set_cursor(0);
            assert!(
                app.focused_dismiss_target().is_some(),
                "deleted top-level project should expose dismiss affordance"
            );

            let item = &app.project_list[0].root_item;
            let row = columns::build_row_cells(ProjectRow {
                prefix:            PREFIX_ROOT_LEAF,
                name:              &item.root_directory_name().into_string(),
                name_segments:     None,
                git_status:        app.project_list.git_status_for(item.path()),
                lint:              app.lint_cell(&crate::tui::state::Lint::status_for_root(item)),
                disk:              "0.0",
                disk_style:        Style::default(),
                disk_suffix:       Some(" [x]"),
                disk_suffix_style: Some(Style::default().fg(Color::DarkGray)),
                lang_icon:         item.lang_icon(),
                git_origin_sync:   &app.project_list.git_sync(item.path()),
                git_main:          &app.project_list.git_main(item.path()),
                ci:                app
                    .project_list
                    .ci_status_for_root_item_using_lookup(item, &app.ci.status_lookup()),
                lifecycle:         RowLifecycle::Deleted,
                worktree_health:   Normal,
            });
            let widths = crate::tui::columns::ProjectListWidths::new(true);
            let line = columns::row_to_line(&row, &widths);

            let suffix = line
                .spans
                .iter()
                .find(|span| span.content.as_ref() == " [x]")
                .expect("deleted row should render dismiss suffix");
            assert_eq!(suffix.style.fg, Some(Color::DarkGray));
            assert!(
                !suffix.style.add_modifier.contains(Modifier::CROSSED_OUT),
                "dismiss suffix should not be crossed out"
            );

            let crossed_out_non_suffix = line
                .spans
                .iter()
                .filter(|span| span.content.as_ref() != " [x]")
                .all(|span| span.style.add_modifier.contains(Modifier::CROSSED_OUT));
            assert!(
                crossed_out_non_suffix,
                "deleted row content should be crossed out"
            );
        }

        #[test]
        fn top_level_deleted_project_can_be_dismissed_and_stops_rendering() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_dir = tmp.path().join("demo");
            std::fs::create_dir_all(&project_dir).expect("create test directory");

            let project_path = project_dir.to_string_lossy().to_string();
            let project = make_project(Some("demo"), &project_path);
            let mut app = make_app(&[project]);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows().len(),
                1,
                "top-level project should render"
            );

            std::fs::remove_dir_all(&project_dir).expect("remove test directory");
            app.handle_disk_usage(Path::new(&project_path), 0);

            let abs_path = AbsolutePath::from(project_path.clone());
            assert!(
                app.project_list.is_deleted(&abs_path),
                "top-level project should be deleted"
            );
            assert_eq!(
                app.project_list
                    .at_path(&abs_path)
                    .expect("top-level project should still exist in hierarchy")
                    .visibility,
                Deleted
            );

            app.project_list
                .lint_at_path_mut(&abs_path)
                .expect("top-level project should have lint state")
                .set_runs(vec![LintRun {
                    run_id:        "dismissed-run".to_string(),
                    started_at:    "2026-03-30T14:22:18-05:00".to_string(),
                    finished_at:   Some("2026-03-30T14:23:18-05:00".to_string()),
                    duration_ms:   Some(60_000),
                    status:        LintRunStatus::Passed,
                    commands:      Vec::new(),
                    archive_bytes: 0,
                }]);
            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   abs_path.clone(),
                status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            assert!(
                app.lint.running_toast_contains_path(abs_path.as_path()),
                "deleted project should have a running lint toast before dismiss"
            );

            app.project_list.set_cursor(0);
            let target = app
                .focused_dismiss_target()
                .expect("deleted top-level project should be dismissable");
            app.dismiss(target);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows().len(),
                0,
                "dismissed top-level deleted project should no longer render"
            );
            assert_eq!(
                app.project_list
                    .at_path(&abs_path)
                    .expect("top-level project should remain in hierarchy after dismiss")
                    .visibility,
                Dismissed
            );
            let lint_runs = app
                .project_list
                .lint_at_path(&abs_path)
                .expect("dismissed project lint state remains addressable");
            assert!(
                lint_runs.runs().is_empty(),
                "dismiss should clear in-memory lint runs for the deleted project"
            );
            assert!(
                !app.lint.running_toast_contains_path(abs_path.as_path()),
                "dismiss should clear running lint toast state for the deleted project"
            );
        }
    }
    mod rows {
        use tui_pane::ACTIVITY_SPINNER;

        use super::*;
        use crate::config::LintIndicator;
        use crate::constants::CI_PASSED;
        use crate::project::Submodule;
        use crate::tui::columns;
        use crate::tui::panes;
        use crate::tui::project_list::ExpandTarget;

        #[test]
        fn submodule_rows_render_disk_usage() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let root_dir = tmp.path().join("blender");
            let sub_dir = root_dir.join("lib").join("linux_x64");
            std::fs::create_dir_all(&sub_dir).expect("create test directory");

            let root_path = root_dir.to_string_lossy().to_string();
            let sub_path = sub_dir.to_string_lossy().to_string();
            let root = make_project(Some("blender"), &root_path);
            let mut app = make_app(&[root]);

            let root_info = app
                .project_list
                .at_path_mut(Path::new(&root_path))
                .expect("test project should exist in project list");
            root_info.submodules.push(Submodule {
                name:          "lib/linux_x64".to_string(),
                path:          AbsolutePath::from(sub_path.clone()),
                relative_path: "lib/linux_x64".to_string(),
                url:           None,
                branch:        None,
                commit:        None,
                project_info:  crate::project::ProjectInfo::default(),
                git_repo:      None,
            });

            app.handle_disk_usage(Path::new(&root_path), 2_000_000);
            app.handle_disk_usage(Path::new(&sub_path), 1_234_567);
            app.ensure_visible_rows_cached();

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root with submodule should expand");
            app.ensure_visible_rows_cached();

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| {
                    line.contains("lib/linux_x64 (s)")
                        && (line.contains("1.2 MiB") || line.contains("1.2 Mi"))
                }),
                "submodule row should render its disk usage: {rendered:?}"
            );
        }

        #[test]
        fn visible_rows_workspace_with_worktrees() {
            let member_a = make_member(Some("a"), "~/ws/a");
            let member_b = make_member(Some("b"), "~/ws/b");

            let primary = make_workspace_raw(
                None,
                "~/ws",
                vec![inline_group(vec![member_a.clone(), member_b.clone()])],
                None,
            );
            let linked = make_workspace_raw(
                None,
                "~/ws_feat",
                vec![named_group("crates", vec![member_a, member_b])],
                Some("ws_feat"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);

            let expanded: HashSet<ExpandKey> = [
                ExpandKey::Node(0),
                ExpandKey::Worktree(0, 0),
                ExpandKey::Worktree(0, 1),
                ExpandKey::WorktreeGroup(0, 1, 0),
            ]
            .into();

            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 8, "expected 8 rows, got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { node_index: 0 }));
            assert!(matches!(
                rows[1],
                VisibleRow::WorktreeEntry {
                    node_index:     0,
                    worktree_index: 0,
                }
            ));
            assert!(matches!(
                rows[2],
                VisibleRow::WorktreeMember {
                    node_index:     0,
                    worktree_index: 0,
                    group_index:    0,
                    member_index:   0,
                }
            ));
            assert!(matches!(
                rows[4],
                VisibleRow::WorktreeEntry {
                    node_index:     0,
                    worktree_index: 1,
                }
            ));
            assert!(matches!(
                rows[5],
                VisibleRow::WorktreeGroupHeader {
                    node_index:     0,
                    worktree_index: 1,
                    group_index:    0,
                }
            ));
            assert!(matches!(
                rows[7],
                VisibleRow::WorktreeMember {
                    node_index:     0,
                    worktree_index: 1,
                    group_index:    0,
                    member_index:   1,
                }
            ));
        }

        #[test]
        fn running_lint_renders_on_worktree_group_and_entry_rows() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );
            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/ws_feat"),
                status: LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });

            let rendered = rendered_root_name_cells(&mut app);
            let frame = ACTIVITY_SPINNER.frame_at(app.animation_started.elapsed());
            let root_row = rendered
                .iter()
                .find(|line| line.contains("ws"))
                .unwrap_or_else(|| panic!("worktree group row should render: {rendered:?}"));
            assert!(
                root_row.contains(frame),
                "worktree group row should render the running lint spinner: {rendered:?}"
            );
            let linked_row = rendered
                .iter()
                .find(|line| line.contains("ws_feat"))
                .unwrap_or_else(|| panic!("linked worktree row should render: {rendered:?}"));
            assert!(
                linked_row.contains(frame),
                "linked worktree row should render the running lint spinner: {rendered:?}"
            );
            assert!(
                app.lint
                    .running_toast_contains_path(test_path("~/ws_feat").as_path())
            );
        }

        #[test]
        fn single_live_worktree_workspace_renders_named_group_header() {
            let primary = make_workspace_raw(
                Some("hana"),
                "~/hana",
                vec![named_group(
                    "demos",
                    vec![
                        make_member(Some("wasm_node_demo"), "~/hana/demos/wasm_node_demo"),
                        make_member(
                            Some("wasm_node_simple"),
                            "~/hana/demos/wasm_node_demo/wasm_nodes/wasm_node_simple",
                        ),
                    ],
                )],
                None,
            );
            let root = make_workspace_worktrees_item(primary, Vec::new());
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));

            let rendered = rendered_root_name_cells(&mut app);

            assert!(
                rendered.iter().any(|line| line.contains("demos (2)")),
                "single-live worktree workspace should render the named group label: {rendered:?}"
            );
            assert!(
                !rendered.iter().any(|line| line.contains("(0)")),
                "single-live worktree workspace should not render the fallback group label: {rendered:?}"
            );
        }

        /// Fixture for `expand_linked_workspace_worktree_renders_its_members`.
        /// Builds the primary + linked worktree pair separately so the test
        /// body itself stays focused on row-layout assertions.
        fn linked_workspace_worktrees_fixture() -> RootItem {
            let member_a = make_member(Some("a"), "~/ws/a");
            let member_b = make_member(Some("b"), "~/ws/b");

            let primary = make_workspace_raw(
                None,
                "~/ws",
                vec![inline_group(vec![member_a.clone(), member_b.clone()])],
                None,
            );
            let linked = make_workspace_raw(
                None,
                "~/ws_feat",
                vec![named_group("crates", vec![member_a, member_b])],
                Some("ws_feat"),
            );
            make_workspace_worktrees_item(primary, vec![linked])
        }

        #[test]
        fn expand_state_round_trips_through_stable_targets() {
            let mut list = as_entries(vec![linked_workspace_worktrees_fixture()]);
            // The worktree group node, its primary entry (same path as the node), the
            // linked entry, and the linked entry's named group.
            let original: HashSet<ExpandKey> = [
                ExpandKey::Node(0),
                ExpandKey::Worktree(0, 0),
                ExpandKey::Worktree(0, 1),
                ExpandKey::WorktreeGroup(0, 1, 0),
            ]
            .into();
            list.expanded = original.clone();

            // The primary `Node` and `Worktree(0, 0)` share `~/ws`; the variant tag
            // keeps them distinct rather than collapsing to a single target.
            let targets = list.export_expanded();
            assert_eq!(targets.len(), 4, "got: {targets:?}");

            // A restart drops the positional keys; re-applying the stable targets to a
            // freshly built (identical) tree restores exactly the same keys.
            let mut rebuilt = as_entries(vec![linked_workspace_worktrees_fixture()]);
            rebuilt.apply_expanded(&targets);
            assert_eq!(rebuilt.expanded, original);
        }

        #[test]
        fn expand_state_apply_skips_targets_no_longer_in_the_tree() {
            // A target whose path is gone (project removed since the last run) is
            // silently dropped, and the surviving target still applies.
            let mut list = as_entries(vec![linked_workspace_worktrees_fixture()]);
            list.expanded = [ExpandKey::Node(0)].into();
            let mut targets = list.export_expanded();
            targets.push(ExpandTarget::Root(AbsolutePath::from("/nonexistent/gone")));

            let mut rebuilt = as_entries(vec![linked_workspace_worktrees_fixture()]);
            rebuilt.apply_expanded(&targets);
            assert_eq!(rebuilt.expanded, [ExpandKey::Node(0)].into());
        }

        #[test]
        fn expand_linked_workspace_worktree_renders_its_members() {
            let mut app = make_app(&[linked_workspace_worktrees_fixture()]);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[VisibleRow::Root { node_index: 0 }],
                "workspace worktree group should start collapsed"
            );

            assert!(app.expand(), "root workspace worktree group should expand");
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                ],
                "expanding the root should show primary and linked worktree rows"
            );

            app.project_list.set_cursor(2);
            assert!(app.expand(), "linked workspace worktree row should expand");
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                    VisibleRow::WorktreeGroupHeader {
                        node_index:     0,
                        worktree_index: 1,
                        group_index:    0,
                    },
                ],
                "expanding the linked workspace worktree should show its member group"
            );

            app.project_list.set_cursor(3);
            assert!(app.expand(), "linked workspace member group should expand");
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                    VisibleRow::WorktreeGroupHeader {
                        node_index:     0,
                        worktree_index: 1,
                        group_index:    0,
                    },
                    VisibleRow::WorktreeMember {
                        node_index:     0,
                        worktree_index: 1,
                        group_index:    0,
                        member_index:   0,
                    },
                    VisibleRow::WorktreeMember {
                        node_index:     0,
                        worktree_index: 1,
                        group_index:    0,
                        member_index:   1,
                    },
                ],
                "expanding the linked workspace group should render its members"
            );
        }

        #[test]
        fn visible_rows_non_workspace_worktrees() {
            let build_root = || {
                make_package_worktrees_item(
                    make_package_raw(Some("app"), "~/app", None),
                    vec![make_package_raw(
                        Some("app"),
                        "~/app_feat",
                        Some("app_feat"),
                    )],
                )
            };

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![build_root()]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 3, "got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { .. }));
            assert!(matches!(rows[1], VisibleRow::WorktreeEntry { .. }));
            assert!(matches!(rows[2], VisibleRow::WorktreeEntry { .. }));

            let expanded2: HashSet<ExpandKey> =
                [ExpandKey::Node(0), ExpandKey::Worktree(0, 0)].into();
            let rows2 =
                super::as_entries(vec![build_root()]).compute_visible_rows(&expanded2, true);
            assert_eq!(rows2.len(), 3, "no extra rows for non-workspace worktree");
        }

        #[test]
        fn package_worktree_entries_sort_alphabetically() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("zeta"), "~/zeta", None),
                vec![
                    make_package_raw(Some("alpha"), "~/alpha", Some("alpha")),
                    make_package_raw(Some("middle"), "~/middle", Some("middle")),
                ],
            );

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(
                rows,
                vec![
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 2,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                ]
            );
        }

        #[test]
        fn workspace_worktree_entries_sort_alphabetically() {
            let root = make_workspace_worktrees_item(
                make_workspace_raw(Some("zeta"), "~/zeta", Vec::new(), None),
                vec![
                    make_workspace_raw(Some("alpha"), "~/alpha", Vec::new(), Some("alpha")),
                    make_workspace_raw(Some("middle"), "~/middle", Vec::new(), Some("middle")),
                ],
            );

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(
                rows,
                vec![
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 2,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                ]
            );
        }

        #[test]
        fn primary_worktree_entry_renders_marker_with_three_visible_checkouts() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("zeta"), "~/zeta", None),
                vec![
                    make_package_raw(Some("alpha"), "~/alpha", Some("alpha")),
                    make_package_raw(Some("middle"), "~/middle", Some("middle")),
                ],
            );
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));

            let rendered = rendered_root_name_cells(&mut app);

            assert!(
                rendered.iter().any(|line| line.contains("zeta (p)")),
                "primary worktree entry should render the marker: {rendered:?}"
            );
            assert!(
                rendered
                    .iter()
                    .all(|line| { !line.contains("alpha (p)") && !line.contains("middle (p)") }),
                "linked worktree entries should not render the marker: {rendered:?}"
            );
        }

        #[test]
        fn primary_worktree_entry_omits_marker_with_two_visible_checkouts() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("app_feat"),
                    "~/app_feat",
                    Some("app_feat"),
                )],
            );
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));

            let rendered = rendered_root_name_cells(&mut app);

            assert!(
                rendered.iter().all(|line| !line.contains("(p)")),
                "two-checkout groups should not render the marker: {rendered:?}"
            );
        }

        #[test]
        fn primary_worktree_entry_omits_marker_when_deleted_rows_leave_one_visible_checkout() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("old_app"),
                    "~/old_app",
                    Some("old_app"),
                )],
            );
            let mut app = make_app(&[root]);
            app.project_list
                .at_path_mut(test_path("~/old_app").as_path())
                .expect("linked worktree should exist")
                .visibility = Deleted;
            app.project_list.expanded.insert(ExpandKey::Node(0));

            let rendered = rendered_root_name_cells(&mut app);

            assert!(
                rendered.iter().any(|line| line.contains("old_app")),
                "deleted linked worktree should still render until dismissed: {rendered:?}"
            );
            assert!(
                rendered.iter().all(|line| !line.contains("(p)")),
                "single visible checkout plus a deleted row should not render the marker: {rendered:?}"
            );
        }

        #[test]
        fn worktree_section_collapses_when_one_dismissed() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("app"),
                    "~/app_feat",
                    Some("app_feat"),
                )],
            );

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();

            let items = vec![root.clone()];
            let rows = super::as_entries(items).compute_visible_rows(&expanded, true);
            assert_eq!(rows.len(), 3, "root + 2 worktree entries");

            let mut items = vec![root];
            let linked_path = match &items[0] {
                RootItem::Worktrees(group) => group.linked[0].path().to_path_buf(),
                _ => unreachable!("expected package worktrees"),
            };
            items[0]
                .at_path_mut(&linked_path)
                .expect("linked worktree should exist")
                .visibility = Dismissed;
            let rows = super::as_entries(items).compute_visible_rows(&expanded, true);
            assert_eq!(
                rows.len(),
                1,
                "only the root should remain when one worktree is left"
            );
            assert_eq!(rows, vec![VisibleRow::Root { node_index: 0 }]);
        }

        #[test]
        fn dismissing_deleted_linked_worktree_promotes_primary_back_to_root() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("app");
            let linked_dir = tmp.path().join("app_feat");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), &primary_path, None),
                vec![make_package_raw(
                    Some("app"),
                    &linked_path,
                    Some("app_feat"),
                )],
            );
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();
            assert_eq!(app.visible_rows().len(), 3, "root + 2 worktree entries");

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            app.handle_disk_usage(Path::new(&linked_path), 0);

            let linked_abs = AbsolutePath::from(linked_path.clone());
            assert!(
                app.project_list.is_deleted(&linked_abs),
                "linked worktree should be deleted"
            );

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows().len(),
                3,
                "deleted worktree should still render until dismissed"
            );

            app.project_list.set_cursor(2);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked worktree should be dismissable");
            app.dismiss(target);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[VisibleRow::Root { node_index: 0 }],
                "dismissing the deleted worktree should collapse the group to the root row"
            );
            assert_eq!(
                match &app.project_list[0].root_item {
                    RootItem::Worktrees(wtg) if matches!(&wtg.primary, RustProject::Package(_)) => {
                        assert_eq!(wtg.live_entry_count(), 1);
                        usize::from(wtg.renders_as_group())
                    },
                    RootItem::Rust(_) | RootItem::NonRust(_) | RootItem::Worktrees(_) => 0,
                },
                0,
                "the remaining primary should no longer render as a worktree group"
            );
            assert_eq!(
                app.project_list.selected_project_path(),
                Some(Path::new(&primary_path)),
                "selection should move back to the surviving top-level project"
            );
            assert_eq!(
                app.project_list
                    .at_path(&linked_abs)
                    .expect("linked worktree should remain in the hierarchy")
                    .visibility,
                Dismissed
            );
        }

        #[test]
        fn dismissing_deleted_linked_workspace_worktree_promotes_primary_back_to_root() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("ws");
            let linked_dir = tmp.path().join("ws_feat");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let root = make_workspace_worktrees_item(
                make_workspace_raw(Some("ws"), &primary_path, Vec::new(), None),
                vec![make_workspace_raw(
                    Some("ws"),
                    &linked_path,
                    Vec::new(),
                    Some("ws_feat"),
                )],
            );
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();
            assert_eq!(app.visible_rows().len(), 3, "root + 2 worktree entries");

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  AbsolutePath::from(linked_path.clone()),
                    bytes: 0,
                },
            );

            let linked_abs = AbsolutePath::from(linked_path);
            assert!(
                app.project_list.is_deleted(&linked_abs),
                "linked workspace should be deleted"
            );
            assert_eq!(
                app.visible_rows().len(),
                3,
                "deleted linked workspace should still render until dismissed"
            );

            app.project_list.set_cursor(2);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked workspace should be dismissable");
            app.dismiss(target);
            app.ensure_visible_rows_cached();

            assert_eq!(
                app.visible_rows(),
                &[VisibleRow::Root { node_index: 0 }],
                "dismissing the deleted workspace worktree should collapse to the root row"
            );
            assert_eq!(
                match &app.project_list[0].root_item {
                    RootItem::Worktrees(wtg)
                        if matches!(&wtg.primary, RustProject::Workspace(_)) =>
                    {
                        assert_eq!(wtg.live_entry_count(), 1);
                        usize::from(wtg.renders_as_group())
                    },
                    RootItem::Rust(_) | RootItem::NonRust(_) | RootItem::Worktrees(_) => 0,
                },
                0,
                "the remaining primary should no longer render as a worktree group"
            );
        }

        #[test]
        fn dismissing_deleted_linked_workspace_worktree_keeps_primary_member_rows_rendered() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_style_fix");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let primary = make_workspace_raw(
                Some("bevy_brp"),
                &primary_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp"),
                    &format!("{primary_path}/crates/bevy_brp"),
                )])],
                None,
            );
            let linked = make_workspace_raw(
                Some("bevy_brp"),
                &linked_path,
                Vec::new(),
                Some("bevy_brp_style_fix"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  AbsolutePath::from(linked_path),
                    bytes: 0,
                },
            );

            app.project_list.set_cursor(2);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked workspace should be dismissable");
            app.dismiss(target);

            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    },
                ],
                "expanded root should keep rendering the surviving primary workspace members"
            );

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| line.contains("bevy_brp")),
                "member row should render its name instead of blank output: {rendered:?}"
            );
        }

        #[test]
        fn dismissing_deleted_linked_workspace_worktree_preserves_primary_member_disk_sizes() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_style_fix");
            let member_dir = primary_dir.join("crates").join("bevy_brp");
            std::fs::create_dir_all(&member_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let member_path = member_dir.to_string_lossy().to_string();
            let primary = make_workspace_raw(
                Some("bevy_brp"),
                &primary_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp"),
                    &member_path,
                )])],
                None,
            );
            let linked = make_workspace_raw(
                Some("bevy_brp"),
                &linked_path,
                Vec::new(),
                Some("bevy_brp_style_fix"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.handle_disk_usage(Path::new(&primary_path), 2_000_000);
            app.handle_disk_usage(Path::new(&member_path), 1_234_567);
            assert_eq!(
                app.project_list
                    .at_path(Path::new(&member_path))
                    .and_then(|info| info.disk_usage_bytes),
                Some(1_234_567)
            );
            app.ensure_visible_rows_cached();

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  AbsolutePath::from(linked_path),
                    bytes: 0,
                },
            );

            app.project_list.set_cursor(2);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked workspace should be dismissable");
            app.dismiss(target);
            app.ensure_visible_rows_cached();

            assert_eq!(
                app.project_list
                    .at_path(Path::new(&member_path))
                    .and_then(|info| info.disk_usage_bytes),
                Some(1_234_567),
                "member disk usage should remain stored after dismiss"
            );

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered
                    .iter()
                    .any(|line| line.contains("1.2 MiB") || line.contains("1.2 Mi")),
                "surviving member row should keep its disk usage after dismiss: {rendered:?}"
            );
        }

        #[test]
        fn deleted_linked_workspace_children_render_crossed_out_before_dismiss() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_test");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let primary = make_workspace_raw(
                Some("bevy_brp"),
                &primary_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp_extras"),
                    &format!("{primary_path}/bevy_brp_extras"),
                )])],
                None,
            );
            let linked = make_workspace_raw(
                Some("bevy_brp"),
                &linked_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp_extras"),
                    &format!("{linked_path}/bevy_brp_extras"),
                )])],
                Some("bevy_brp_test"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();
            app.project_list.set_cursor(2);
            assert!(app.expand(), "linked worktree row should expand");
            app.ensure_visible_rows_cached();

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  AbsolutePath::from(linked_path.clone()),
                    bytes: 0,
                },
            );

            assert!(
                app.project_list.is_deleted(Path::new(&linked_path)),
                "linked workspace should be marked deleted"
            );
            assert!(
                matches!(app.visible_rows()[3], VisibleRow::WorktreeMember { .. }),
                "expanded linked workspace member row should still be visible before dismiss"
            );

            let (buffer, widths) = render_tree_buffer(&mut app);
            assert!(
                row_has_crossed_out_content(&buffer, &widths, 2),
                "deleted linked workspace row should be crossed out"
            );
            assert!(
                row_has_crossed_out_content(&buffer, &widths, 3),
                "deleted linked workspace member row should inherit crossed-out styling"
            );
        }

        #[test]
        fn dismissing_deleted_linked_workspace_member_dismisses_whole_worktree() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_test");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let primary = make_workspace_raw(
                Some("bevy_brp"),
                &primary_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp_extras"),
                    &format!("{primary_path}/bevy_brp_extras"),
                )])],
                None,
            );
            let linked = make_workspace_raw(
                Some("bevy_brp"),
                &linked_path,
                vec![inline_group(vec![make_member(
                    Some("bevy_brp_extras"),
                    &format!("{linked_path}/bevy_brp_extras"),
                )])],
                Some("bevy_brp_test"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);
            let mut app = make_app(&[root]);

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();
            app.project_list.set_cursor(2);
            assert!(app.expand(), "linked worktree row should expand");
            app.ensure_visible_rows_cached();

            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  AbsolutePath::from(linked_path.clone()),
                    bytes: 0,
                },
            );

            app.project_list.set_cursor(3);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked workspace member should dismiss its worktree");
            match &target {
                DismissTarget::DeletedProject(path) => assert_eq!(path, Path::new(&linked_path)),
                DismissTarget::Toast(_) => panic!("expected deleted project target"),
            }
            app.dismiss(target);
            app.ensure_visible_rows_cached();

            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    },
                ],
                "dismissing a deleted linked workspace member should dismiss the whole linked worktree"
            );
        }

        #[test]
        fn mixed_visible_and_deleted_worktree_group_stays_visible() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("app"),
                    "~/app_feat",
                    Some("app_feat"),
                )],
            );
            let mut items = vec![root];
            items[0]
                .at_path_mut(test_path("~/app_feat").as_path())
                .expect("linked worktree should exist")
                .visibility = Deleted;

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(items.clone()).compute_visible_rows(&expanded, true);

            assert_eq!(items[0].visibility(), crate::project::Visibility::Visible);
            assert_eq!(rows.len(), 3, "deleted linked worktree should still render");
        }

        #[test]
        fn all_deleted_worktree_group_derives_deleted_visibility() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("app"),
                    "~/app_feat",
                    Some("app_feat"),
                )],
            );
            let mut items = vec![root];
            items[0]
                .at_path_mut(test_path("~/app").as_path())
                .expect("primary worktree should exist")
                .visibility = Deleted;
            items[0]
                .at_path_mut(test_path("~/app_feat").as_path())
                .expect("linked worktree should exist")
                .visibility = Deleted;

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(items.clone()).compute_visible_rows(&expanded, true);

            assert_eq!(items[0].visibility(), Deleted);
            assert_eq!(
                rows.len(),
                3,
                "deleted worktrees should still render until dismissed"
            );
        }

        #[test]
        fn all_dismissed_worktree_group_is_hidden() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("app"), "~/app", None),
                vec![make_package_raw(
                    Some("app"),
                    "~/app_feat",
                    Some("app_feat"),
                )],
            );
            let mut items = vec![root];
            items[0]
                .at_path_mut(test_path("~/app").as_path())
                .expect("primary worktree should exist")
                .visibility = Dismissed;
            items[0]
                .at_path_mut(test_path("~/app_feat").as_path())
                .expect("linked worktree should exist")
                .visibility = Dismissed;

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(items.clone()).compute_visible_rows(&expanded, true);

            assert_eq!(items[0].visibility(), Dismissed);
            assert!(
                rows.is_empty(),
                "all-dismissed worktree groups should not render"
            );
        }

        fn assert_worktree_fit_widths_use_display_name(
            item: RootItem,
            primary_label: &str,
            linked_label: &str,
        ) {
            let root_label = resolved_root_label(&item);
            let entries = super::as_entries(vec![item]);
            let widths = panes::compute_project_list_widths(
                &entries,
                std::slice::from_ref(&root_label),
                true,
                0,
            );
            let root_width =
                columns::display_width(PREFIX_ROOT_COLLAPSED) + columns::display_width(&root_label);
            let primary_entry_width = columns::display_width(PREFIX_WORKTREE_FLAT)
                + columns::display_width(primary_label);
            let linked_entry_width =
                columns::display_width(PREFIX_WORKTREE_FLAT) + columns::display_width(linked_label);

            assert_eq!(
                widths.get(COL_NAME),
                crate::tui::panes::name_width_with_gutter(
                    root_width.max(primary_entry_width).max(linked_entry_width)
                ),
                "fit widths should use rendered worktree labels, not the absolute primary worktree path"
            );
        }

        #[test]
        fn worktree_fit_widths_use_display_name_for_primary_entry() {
            assert_worktree_fit_widths_use_display_name(
                make_workspace_worktrees_item(
                    make_workspace_raw(
                        Some("obsidian_knife"),
                        "/tmp/really/long/path/to/obsidian_knife",
                        Vec::new(),
                        None,
                    ),
                    vec![make_workspace_raw(
                        Some("obsidian_knife"),
                        "/tmp/really/long/path/to/obsidian_knife_test",
                        Vec::new(),
                        Some("obsidian_knife_test"),
                    )],
                ),
                "obsidian_knife",
                "obsidian_knife_test",
            );
            assert_worktree_fit_widths_use_display_name(
                make_package_worktrees_item(
                    make_package_raw(
                        Some("cargo-port"),
                        "/tmp/really/long/path/to/cargo-port",
                        None,
                    ),
                    vec![make_package_raw(
                        Some("cargo-port"),
                        "/tmp/really/long/path/to/cargo-port_test",
                        Some("cargo-port_test"),
                    )],
                ),
                "cargo-port",
                "cargo-port_test",
            );
        }

        #[test]
        fn worktree_fit_widths_include_primary_marker_when_visible() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("zeta"), "~/zeta", None),
                vec![
                    make_package_raw(Some("alpha"), "~/alpha", Some("alpha")),
                    make_package_raw(Some("middle"), "~/middle", Some("middle")),
                ],
            );
            let root_label = resolved_root_label(&root);
            let entries = super::as_entries(vec![root]);
            let widths = panes::compute_project_list_widths(
                &entries,
                std::slice::from_ref(&root_label),
                true,
                0,
            );
            let root_width =
                columns::display_width(PREFIX_ROOT_COLLAPSED) + columns::display_width(&root_label);
            let primary_entry_width =
                columns::display_width(PREFIX_WORKTREE_FLAT) + columns::display_width("zeta (p)");

            assert_eq!(
                widths.get(COL_NAME),
                crate::tui::panes::name_width_with_gutter(root_width.max(primary_entry_width)),
                "fit widths should include the rendered primary marker"
            );
        }

        #[test]
        fn root_rows_disambiguate_same_directory_leaves_with_parent_suffix() {
            let mut app = make_app(&[
                make_project(Some("cargo-port"), "/tmp/rust/cargo-port"),
                make_project(Some("cargo-port"), "/tmp/archive/cargo-port"),
            ]);

            let names = rendered_root_name_cells(&mut app);

            assert!(
                names
                    .iter()
                    .any(|name| name.contains("cargo-port [rust/cargo-port]")),
                "colliding dir-leaf roots should disambiguate by parent path: {names:?}"
            );
            assert!(
                names
                    .iter()
                    .any(|name| name.contains("cargo-port [archive/cargo-port]")),
                "colliding dir-leaf roots should disambiguate by parent path: {names:?}"
            );
            assert_ne!(
                names[0], names[1],
                "colliding roots should render distinctly"
            );
        }

        #[test]
        fn root_rows_extend_dir_suffix_until_same_leaf_dirs_become_unique() {
            let mut app = make_app(&[
                make_package_worktrees_item(
                    make_package_raw(Some("cargo-port"), "/tmp/rust/cargo-port", None),
                    vec![make_package_raw(
                        Some("cargo-port"),
                        "/tmp/rust/cargo-port_test",
                        Some("cargo-port_test"),
                    )],
                ),
                make_project(Some("cargo-port"), "/tmp/archive/cargo-port"),
            ]);

            let names = rendered_root_name_cells(&mut app);

            assert!(
                names
                    .iter()
                    .any(|name| name.contains("cargo-port [rust/cargo-port]")),
                "root label should prepend parents until the suffix becomes unique: {names:?}"
            );
            assert!(
                names
                    .iter()
                    .any(|name| name.contains("cargo-port [archive/cargo-port]")),
                "root label should prepend parents until the suffix becomes unique: {names:?}"
            );
            assert!(
                names.iter().any(|name| name.contains(WORKTREE)),
                "worktree root should still render its badge after disambiguation: {names:?}"
            );
            assert_ne!(
                names[0], names[1],
                "same-name same-leaf roots should render distinctly"
            );
        }

        #[test]
        fn visible_rows_workspace_no_worktrees() {
            let root = make_workspace_with_members(
                None,
                "~/ws",
                vec![inline_group(vec![
                    make_member(Some("a"), "~/ws/a"),
                    make_member(Some("b"), "~/ws/b"),
                ])],
            );

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 3, "got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { .. }));
            assert!(matches!(
                rows[1],
                VisibleRow::Member {
                    member_index: 0,
                    ..
                }
            ));
            assert!(matches!(
                rows[2],
                VisibleRow::Member {
                    member_index: 1,
                    ..
                }
            ));
        }

        #[test]
        fn visible_rows_include_vendored_children() {
            let ws = Workspace {
                path: test_path("~/ws"),
                groups: vec![inline_group(vec![make_member(
                    Some("member"),
                    "~/ws/member",
                )])],
                rust: RustInfo {
                    vendored: vec![super::make_vendored(Some("vendored"), "~/ws/vendor/helper")],
                    ..RustInfo::default()
                },
                ..Workspace::default()
            };
            let root = RootItem::Rust(RustProject::Workspace(ws));

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 3, "got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { .. }));
            assert!(matches!(rows[1], VisibleRow::Member { .. }));
            assert!(matches!(
                rows[2],
                VisibleRow::Vendored {
                    node_index:     0,
                    vendored_index: 0,
                }
            ));
        }

        #[test]
        fn visible_rows_include_member_vendored_children_when_member_expanded() {
            let ws = Workspace {
                path: test_path("~/ws"),
                groups: vec![inline_group(vec![make_package_with_vendored(
                    Some("member"),
                    "~/ws/member",
                    vec![super::make_vendored(Some("vendored"), "~/ws/vendor/helper")],
                )])],
                ..Workspace::default()
            };
            let root = RootItem::Rust(RustProject::Workspace(ws));

            let expanded: HashSet<ExpandKey> = [ExpandKey::Node(0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 2, "got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { .. }));
            assert!(matches!(rows[1], VisibleRow::Member { .. }));
            assert!(
                !rows
                    .iter()
                    .any(|row| matches!(row, VisibleRow::MemberVendored { .. })),
                "collapsed member should hide vendored children: {rows:?}"
            );

            let ws = Workspace {
                path: test_path("~/ws"),
                groups: vec![inline_group(vec![make_package_with_vendored(
                    Some("member"),
                    "~/ws/member",
                    vec![super::make_vendored(Some("vendored"), "~/ws/vendor/helper")],
                )])],
                ..Workspace::default()
            };
            let root = RootItem::Rust(RustProject::Workspace(ws));
            let expanded: HashSet<ExpandKey> =
                [ExpandKey::Node(0), ExpandKey::Member(0, 0, 0)].into();
            let rows = super::as_entries(vec![root]).compute_visible_rows(&expanded, true);

            assert_eq!(rows.len(), 3, "got: {rows:?}");
            assert!(matches!(rows[0], VisibleRow::Root { .. }));
            assert!(matches!(rows[1], VisibleRow::Member { .. }));
            assert!(matches!(
                rows[2],
                VisibleRow::MemberVendored {
                    node_index:     0,
                    group_index:    0,
                    member_index:   0,
                    vendored_index: 0,
                }
            ));
        }

        #[test]
        fn member_vendored_rows_render_two_space_indents_and_markers() {
            let member = make_package_with_vendored(
                Some("member"),
                "~/ws/member",
                vec![super::make_vendored(Some("helper"), "~/ws/vendor/helper")],
            );
            let root =
                make_workspace_with_members(Some("ws"), "~/ws", vec![inline_group(vec![member])]);
            let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
            apply_items(&mut app, std::slice::from_ref(&root));

            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.project_list.recompute_visibility(true);
            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| line.starts_with("└─▶ member")),
                "member with hidden vendored children should render a collapsed marker: {rendered:?}"
            );
            assert!(
                !rendered.iter().any(|line| line.contains("helper (v)")),
                "collapsed member should hide vendored child rows: {rendered:?}"
            );

            app.project_list.expanded.insert(ExpandKey::Member(0, 0, 0));
            app.project_list.recompute_visibility(true);
            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| line.starts_with("└─▼ member")),
                "expanded member should render an expanded marker: {rendered:?}"
            );
            assert!(
                rendered
                    .iter()
                    .any(|line| line.starts_with("  └── helper (v)")),
                "vendored child should render two spaces deeper with an extended row marker: {rendered:?}"
            );
        }

        #[test]
        fn member_vendored_rows_render_box_drawing_continuations() {
            let first = make_package_with_vendored(
                Some("bevy_diegetic"),
                "~/ws/bevy_diegetic",
                vec![super::make_vendored(
                    Some("clay-layout"),
                    "~/ws/bevy_diegetic/vendor/clay-layout",
                )],
            );
            let second = make_package_raw(Some("bevy_lagrange"), "~/ws/bevy_lagrange", None);
            let third = make_package_raw(Some("fairy_dust"), "~/ws/fairy_dust", None);
            let root = make_workspace_with_members(
                Some("bevy_hana"),
                "~/ws",
                vec![inline_group(vec![first, second, third])],
            );
            let mut app = make_app(&[make_workspace_project(Some("bevy_hana"), "~/ws")]);
            apply_items(&mut app, std::slice::from_ref(&root));

            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.project_list.expanded.insert(ExpandKey::Member(0, 0, 0));
            app.project_list.recompute_visibility(true);

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered
                    .iter()
                    .any(|line| line.starts_with("├─▼ bevy_diegetic")),
                "expanded member should render as a non-final branch: {rendered:?}"
            );
            assert!(
                rendered
                    .iter()
                    .any(|line| line.starts_with("│ └── clay-layout (v)")),
                "vendored child should carry the ancestor continuation: {rendered:?}"
            );
            assert!(
                rendered
                    .iter()
                    .any(|line| line.starts_with("├── bevy_lagrange")),
                "middle member should extend the branch through the sibling marker slot: {rendered:?}"
            );
            assert!(
                rendered
                    .iter()
                    .any(|line| line.starts_with("└── fairy_dust")),
                "final member should extend the branch through the sibling marker slot: {rendered:?}"
            );
        }

        #[test]
        fn vendored_rows_do_not_render_parent_ci_status() {
            let vendored_path = "~/ws/vendor/helper";
            let member = make_package_with_vendored(
                Some("member"),
                "~/ws/member",
                vec![super::make_vendored(Some("helper"), vendored_path)],
            );
            let root =
                make_workspace_with_members(Some("ws"), "~/ws", vec![inline_group(vec![member])]);
            let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
            apply_items(&mut app, std::slice::from_ref(&root));

            set_loaded_ci(
                &mut app,
                root.path(),
                vec![make_ci_run(1, CiStatus::Passed)],
                false,
                1,
            );

            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.project_list.expanded.insert(ExpandKey::Member(0, 0, 0));
            app.project_list.recompute_visibility(true);

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered
                    .iter()
                    .any(|line| line.contains("ws") && line.contains(CI_PASSED)),
                "root row should still render CI status: {rendered:?}"
            );
            let vendored_row = rendered
                .iter()
                .find(|line| line.contains("helper (v)"))
                .unwrap_or_else(|| panic!("vendored row should render: {rendered:?}"));
            assert!(
                !vendored_row.contains(CI_PASSED),
                "vendored row should not inherit parent CI status: {vendored_row}"
            );
        }
    }
    mod state {
        use std::collections::BTreeMap;
        use std::collections::HashMap;
        use std::path::Path;
        use std::sync::atomic::Ordering;
        use std::time::Duration;

        use cargo_metadata::PackageId;
        use cargo_metadata::TargetKind;
        use cargo_metadata::semver::Version;

        use super::*;
        use crate::config;
        use crate::config::LintIndicator;
        use crate::constants::IN_SYNC;
        use crate::constants::NO_REMOTE_SYNC;
        use crate::lint::CachedLintStatus;
        use crate::lint::LintRun;
        use crate::lint::LintRunStatus;
        use crate::project::AbsolutePath;
        use crate::project::FileStamp;
        use crate::project::HeadState;
        use crate::project::ManifestFingerprint;
        use crate::project::Package;
        use crate::project::PackageRecord;
        use crate::project::ProjectPrData;
        use crate::project::ProjectPrInfo;
        use crate::project::PublishPolicy;
        use crate::project::PullRequestCompleteness;
        use crate::project::PullRequestGoneReason;
        use crate::project::PullRequestInfo;
        use crate::project::PullRequestState;
        use crate::project::RootItem;
        use crate::project::RustProject;
        use crate::project::WorkspaceMetadata;
        use crate::project::WorktreeGroup;
        use crate::project::WorktreeStatus;
        use crate::scan::CargoMetadataError;
        use crate::tui::app::phase_state::Denominator;
        use crate::tui::app::target_index::CleanSelection;
        use crate::tui::constants::STARTUP_ROW_MIN_VISIBLE;
        use crate::tui::keymap::CiRunsAction;
        use crate::tui::keymap::LintsAction;
        use crate::tui::panes;
        use crate::tui::state::StartupNetworkReadiness;
        use crate::tui::terminal::CleanMsg;

        fn test_pull_request_info(number: u32, title: &str) -> PullRequestInfo {
            test_pull_request_info_with_state(number, title, PullRequestState::Ready)
        }

        fn test_pull_request_info_with_state(
            number: u32,
            title: &str,
            state: PullRequestState,
        ) -> PullRequestInfo {
            PullRequestInfo {
                number,
                title: title.to_string(),
                url: format!("https://github.com/natepiano/cargo-port/pull/{number}"),
                state,
                head: "feat/open-prs".to_string(),
                head_owner: Some("natepiano".to_string()),
                head_repo: Some("cargo-port".to_string()),
                base: "main".to_string(),
            }
        }

        fn test_pr_info(open: Vec<PullRequestInfo>) -> ProjectPrInfo {
            ProjectPrInfo {
                open,
                default_branch: "main".to_string(),
                fetched_at: "2026-05-27T20:51:11Z".to_string(),
                completeness: PullRequestCompleteness::Complete,
                viewer_login: "natepiano".to_string(),
                owner_repo: crate::ci::OwnerRepo::new("natepiano", "cargo-port"),
            }
        }

        fn test_pr_data(open: Vec<PullRequestInfo>) -> ProjectPrData {
            ProjectPrData::Loaded(test_pr_info(open))
        }

        #[test]
        fn lint_runtime_waits_for_scan_completion() {
            let project = make_project(Some("demo"), "~/demo");
            let abs_path = test_path("~/demo");
            let mut app = make_app(&[project]);

            assert!(app.lint_runtime_projects().is_empty());

            app.scan.state.phase = ScanPhase::Complete;
            let projects = app.lint_runtime_projects();
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].abs_path, abs_path);
            assert_eq!(
                projects[0].project_label,
                crate::project::home_relative_path(&abs_path)
            );
        }

        #[test]
        fn workspace_members_show_parent_owner_ci_without_storing_member_state() {
            let workspace = make_workspace_project(Some("ws"), "~/ws");
            let member = make_project(Some("core"), "~/ws/core");
            let root = make_workspace_with_members(
                Some("ws"),
                "~/ws",
                vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
            );

            let mut app = make_app(&[workspace, member]);
            apply_items(&mut app, &[root]);

            app.insert_ci_runs(
                test_path("~/ws").as_path(),
                vec![make_ci_run(1, CiStatus::Passed)],
                0,
            );

            assert_eq!(
                app.project_list
                    .ci_status_using_lookup(test_path("~/ws").as_path(), &app.ci.status_lookup()),
                Some(CiStatus::Passed)
            );
            assert!(matches!(
                app.project_list.ci_data_for(test_path("~/ws").as_path()),
                Some(crate::project::ProjectCiData::Loaded(_))
            ));
            assert_eq!(
                app.project_list.ci_status_using_lookup(
                    test_path("~/ws/core").as_path(),
                    &app.ci.status_lookup()
                ),
                Some(CiStatus::Passed)
            );
            assert!(
                app.project_list
                    .ci_info_for(test_path("~/ws/core").as_path())
                    .is_some()
            );
            // Member resolves to the same entry-level ci_data as the workspace root.
            assert!(matches!(
                app.project_list
                    .ci_data_for(test_path("~/ws/core").as_path()),
                Some(crate::project::ProjectCiData::Loaded(_))
            ));
        }

        #[test]
        fn workspace_member_ci_toggle_branch_and_mode_match_workspace_root() {
            let workspace = make_workspace_project(Some("ws"), "~/ws");
            let member = make_project(Some("core"), "~/ws/core");
            let root = make_workspace_with_members(
                Some("ws"),
                "~/ws",
                vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
            );
            let mut app = make_app(&[workspace, member]);
            apply_items(&mut app, &[root]);

            apply_git_info(
                &mut app,
                test_path("~/ws").as_path(),
                make_git_info(Some("https://github.com/natepiano/ws")),
            );
            app.insert_ci_runs(
                test_path("~/ws").as_path(),
                vec![
                    CiRun {
                        branch: "main".to_string(),
                        ..make_ci_run(1, CiStatus::Passed)
                    },
                    CiRun {
                        branch: "feature".to_string(),
                        ..make_ci_run(2, CiStatus::Failed)
                    },
                ],
                0,
            );

            let ws = test_path("~/ws");
            let core = test_path("~/ws/core");

            // The member resolves its CI branch and toggle to the workspace root,
            // so the all/branch filter is offered on the member just like on the
            // parent, and the default branch-only view filters the shared runs to
            // the workspace branch.
            assert!(app.ci_toggle_available_for(ws.as_path()));
            assert!(app.ci_toggle_available_for(core.as_path()));
            assert_eq!(
                app.project_list.current_branch_for(core.as_path()),
                Some("main")
            );
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(core.as_path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main"]
            );

            // Toggling on the member writes the owner's mode, so the workspace
            // root sees All too — the toggle state is shared, not per-row.
            app.set_ci_display_mode_for(core.as_path(), CiRunDisplayMode::All);
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(ws.as_path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main", "feature"]
            );
        }

        #[test]
        fn vendored_crate_ci_toggle_and_branch_resolve_to_checkout_root() {
            let vendored_path = "~/app/vendor/helper";
            let member = make_package_with_vendored(
                Some("member"),
                "~/app/crates/member",
                vec![super::make_vendored(Some("helper"), vendored_path)],
            );
            let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some("app"),
                "~/app",
                vec![inline_group(vec![member])],
                None,
            )));
            let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
            apply_items(&mut app, &[root_item]);

            apply_git_info(
                &mut app,
                test_path("~/app").as_path(),
                make_git_info(Some("https://github.com/natepiano/app")),
            );
            app.insert_ci_runs(
                test_path("~/app").as_path(),
                vec![
                    CiRun {
                        branch: "main".to_string(),
                        ..make_ci_run(1, CiStatus::Passed)
                    },
                    CiRun {
                        branch: "feature".to_string(),
                        ..make_ci_run(2, CiStatus::Failed)
                    },
                ],
                0,
            );

            let helper = test_path(vendored_path);

            // A vendored crate is not a lint owner, but it still lives inside the
            // workspace checkout — so its CI branch and toggle resolve to the
            // workspace root just like a member's.
            assert!(app.ci_toggle_available_for(helper.as_path()));
            assert_eq!(
                app.project_list.current_branch_for(helper.as_path()),
                Some("main")
            );
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(helper.as_path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main"]
            );
        }

        #[test]
        fn pull_request_disappearance_pushes_deleted_toast() {
            let project = make_project(Some("cargo-port"), "~/cargo-port");
            let path = test_path("~/cargo-port");
            let mut app = make_app(&[project]);
            apply_git_info(
                &mut app,
                path.as_path(),
                make_git_info(Some("https://github.com/natepiano/cargo-port")),
            );
            let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");

            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequests {
                    repo: repo.clone(),
                    data: test_pr_data(vec![test_pull_request_info(1, "test: exercise PR toast")]),
                },
            );
            assert!(
                app.framework
                    .toasts
                    .active_now()
                    .iter()
                    .all(|toast| !toast.title().starts_with("Pull request")),
                "initial PR load should not announce deletion"
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequests {
                    repo: repo.clone(),
                    data: ProjectPrData::Loading(Some(test_pr_info(vec![test_pull_request_info(
                        1,
                        "test: exercise PR toast",
                    )]))),
                },
            );
            assert!(
                app.framework
                    .toasts
                    .active_now()
                    .iter()
                    .all(|toast| !toast.title().starts_with("Pull request")),
                "loading refresh should preserve the old PR without announcing deletion"
            );

            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequests {
                    repo: repo.clone(),
                    data: test_pr_data(Vec::new()),
                },
            );

            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequestDisappeared {
                    repo,
                    pull_request: test_pull_request_info(1, "test: exercise PR toast"),
                    reason: PullRequestGoneReason::Merged {
                        base: "main".to_string(),
                    },
                },
            );

            let toast = app
                .framework
                .toasts
                .active_now()
                .into_iter()
                .find(|toast| toast.title() == "Pull request merged")
                .expect("merged PR toast should be visible");
            assert!(toast.body().contains("natepiano/cargo-port"));
            assert!(toast.body().contains("#1 test: exercise PR toast"));
            assert!(toast.body().contains("merged into main"));
        }

        #[test]
        fn open_pull_request_count_does_not_change_project_list_label() {
            let project = make_project(Some("cargo-port"), "~/cargo-port");
            let path = test_path("~/cargo-port");
            let mut app = make_app(&[project]);
            apply_git_info(
                &mut app,
                path.as_path(),
                make_git_info(Some("https://github.com/natepiano/cargo-port")),
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequests {
                    repo: crate::ci::OwnerRepo::new("natepiano", "cargo-port"),
                    data: test_pr_data(vec![test_pull_request_info(
                        5,
                        "feat: poll PR check state",
                    )]),
                },
            );

            let labels = app
                .project_list
                .resolved_root_labels(app.config.include_non_rust().includes_non_rust());

            assert_eq!(labels, vec!["cargo-port"]);
        }

        #[test]
        fn pull_request_checks_finished_pushes_toast() {
            let project = make_project(Some("cargo-port"), "~/cargo-port");
            let path = test_path("~/cargo-port");
            let mut app = make_app(&[project]);
            apply_git_info(
                &mut app,
                path.as_path(),
                make_git_info(Some("https://github.com/natepiano/cargo-port")),
            );
            let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
            app.net.github.insert_pr_check_poll(repo.clone(), 7);

            apply_bg_msg(
                &mut app,
                BackgroundMsg::PullRequests {
                    repo,
                    data: test_pr_data(vec![test_pull_request_info_with_state(
                        7,
                        "test: exercise PR check marker",
                        PullRequestState::Ready,
                    )]),
                },
            );

            let toast = app
                .framework
                .toasts
                .active_now()
                .into_iter()
                .find(|toast| toast.title() == "Pull request checks finished")
                .expect("checks-finished toast should be visible");
            assert!(toast.body().contains("#7 test: exercise PR check marker"));
            assert!(toast.body().contains("is ready"));
        }

        #[test]
        fn active_pull_request_check_poll_keeps_animation_tick_live() {
            let project = make_project(Some("cargo-port"), "~/cargo-port");
            let mut app = make_app(&[project]);
            app.scan.state.phase = ScanPhase::Complete;

            assert_eq!(app.animation_timeout(), Duration::from_secs(1));

            app.net
                .github
                .insert_pr_check_poll(crate::ci::OwnerRepo::new("natepiano", "cargo-port"), 7);

            assert_eq!(app.animation_timeout(), Duration::from_millis(80));
        }

        #[test]
        fn ci_fetch_on_member_targets_workspace_owner_path() {
            let workspace = make_workspace_project(Some("ws"), "~/ws");
            let member = make_project(Some("core"), "~/ws/core");
            let root = make_workspace_with_members(
                Some("ws"),
                "~/ws",
                vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
            );

            let mut app = make_app(&[workspace, member.clone()]);
            apply_items(&mut app, &[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();
            app.project_list
                .select_project_in_tree(member.path(), false);

            apply_git_info(
                &mut app,
                test_path("~/ws").as_path(),
                make_git_info(Some("https://github.com/natepiano/demo")),
            );

            panes::handle_ci_runs_key(
                &mut app,
                &crossterm::event::KeyEvent::new(
                    KeyCode::Char('f'),
                    crossterm::event::KeyModifiers::NONE,
                ),
            );
            assert_eq!(
                app.inflight
                    .pending_ci_fetch_ref()
                    .as_ref()
                    .map(|fetch| fetch.project_path.clone()),
                Some(test_path("~/ws").display().to_string())
            );
        }

        #[test]
        fn linked_worktree_shares_github_metadata_with_primary_after_repo_meta_fetch() {
            // Regression: a linked worktree on a branch without an upstream
            // never fires its own GitHub fetch, so the About field would stay
            // empty even after the primary's fetch landed. `github_info` lives
            // on `GitRepo` (per ProjectEntry) so all checkouts of the same repo
            // see the same description.
            let primary_ws = make_workspace_raw(Some("ws"), "~/ws", vec![], None);
            let linked_ws =
                make_workspace_raw(Some("ws_feat"), "~/ws_feat", vec![], Some("ws_feat"));
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");

            let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws")]);
            apply_items(&mut app, &[root]);

            app.project_list.handle_repo_meta(
                primary_path.as_path(),
                42,
                Some("a great repo".to_string()),
            );

            let read_description = |p: &Path| {
                app.project_list
                    .entry_containing(p)
                    .and_then(|entry| entry.git_repo.as_ref())
                    .and_then(|repo| repo.github_info.as_ref())
                    .and_then(|gh| gh.description.clone())
            };

            assert_eq!(
                read_description(primary_path.as_path()),
                Some("a great repo".to_string()),
            );
            assert_eq!(
                read_description(linked_path.as_path()),
                Some("a great repo".to_string()),
                "linked worktree should see the primary's fetched description",
            );
        }

        #[test]
        fn worktree_group_shares_ci_data_across_primary_and_linked() {
            let member = make_project(Some("core"), "~/ws/core");

            let primary_ws = make_workspace_raw(
                Some("ws"),
                "~/ws",
                vec![inline_group(vec![make_member(Some("core"), "~/ws/core")])],
                None,
            );
            let linked_ws = make_workspace_raw(
                Some("ws_feat"),
                "~/ws_feat",
                vec![inline_group(vec![make_member(
                    Some("feat_core"),
                    "~/ws_feat/core",
                )])],
                Some("ws_feat"),
            );
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
            let root_path = test_path("~/ws");
            let feature_path = test_path("~/ws_feat");

            let mut app = make_app(&[make_workspace_project(Some("ws"), "~/ws"), member.clone()]);
            apply_items(&mut app, &[root]);

            set_loaded_ci(
                &mut app,
                root_path.as_path(),
                vec![make_ci_run(3, CiStatus::Passed)],
                false,
                0,
            );

            // Linked worktree resolves to the same per-repo ci_data slot.
            assert!(matches!(
                app.project_list.ci_data_for(feature_path.as_path()),
                Some(crate::project::ProjectCiData::Loaded(_))
            ));
            // Member inside the workspace also shares the entry-level ci_data.
            assert!(app.project_list.ci_info_for(member.path()).is_some());
        }

        #[test]
        fn ci_for_prefers_runs_matching_local_branch() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        Some("acme".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![
                    CiRun {
                        branch: "main".to_string(),
                        ..make_ci_run(9, CiStatus::Passed)
                    },
                    CiRun {
                        branch: "feat/demo".to_string(),
                        ..make_ci_run(8, CiStatus::Failed)
                    },
                ],
                false,
                0,
            );

            assert_eq!(
                app.project_list
                    .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
                Some(CiStatus::Failed)
            );
        }

        #[test]
        fn ci_for_default_branch_prefers_matching_branch_runs() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("main".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        Some("acme".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![
                    CiRun {
                        branch: "release".to_string(),
                        ..make_ci_run(9, CiStatus::Failed)
                    },
                    CiRun {
                        branch: "main".to_string(),
                        ..make_ci_run(8, CiStatus::Passed)
                    },
                ],
                false,
                0,
            );

            assert_eq!(
                app.project_list
                    .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
                Some(CiStatus::Passed)
            );
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(project.path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main"]
            );
        }

        #[test]
        fn ci_toggle_switches_non_default_branch_between_branch_only_and_all_runs() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        Some("acme".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![
                    CiRun {
                        branch: "main".to_string(),
                        ..make_ci_run(9, CiStatus::Passed)
                    },
                    CiRun {
                        branch: "feat/demo".to_string(),
                        ..make_ci_run(8, CiStatus::Failed)
                    },
                ],
                false,
                0,
            );

            assert_eq!(
                app.project_list
                    .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
                Some(CiStatus::Failed)
            );
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(project.path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["feat/demo"]
            );

            app.set_ci_display_mode_for(project.path(), CiRunDisplayMode::All);

            assert_eq!(
                app.project_list
                    .ci_status_using_lookup(project.path(), &app.ci.status_lookup()),
                Some(CiStatus::Passed)
            );
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(project.path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main", "feat/demo"]
            );
        }

        #[test]
        fn startup_lint_history_completes_when_loaded_from_disk() {
            let project_a = make_project(Some("a"), "~/a");
            let project_b = make_project(Some("b"), "~/b");
            let mut app = make_app(&[project_a.clone(), project_b.clone()]);
            app.scan.state.phase = ScanPhase::Complete;

            app.initialize_startup_phase_tracker();

            // The "Lint history" row is seeded with every Rust project whose history
            // will be read from disk — it tracks the load, never live lint runs.
            let expected = app
                .startup
                .lint_phase
                .expected
                .keys()
                .expect("lint expected");
            assert_eq!(expected.len(), 2);
            assert!(expected.contains(project_a.path().as_path()));
            assert!(expected.contains(project_b.path().as_path()));
            assert!(app.startup.lint_phase.complete_at.is_none());

            // The single off-thread history-load batch marks every project seen and
            // completes the row.
            app.handle_bg_msg(BackgroundMsg::LintHistoryLoaded {
                entries: vec![
                    (project_a.path().to_path_buf().into(), Vec::new()),
                    (project_b.path().to_path_buf().into(), Vec::new()),
                ],
            });

            assert!(app.startup.lint_phase.complete_at.is_some());
            app.prune_toasts();
        }

        #[test]
        fn startup_git_expected_uses_top_level_git_directories() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let non_rust_dir = tmp.path().join(".claude");
            let workspace_dir = tmp.path().join("bevy");
            let primary_dir = tmp.path().join("cargo-port");
            let linked_dir = tmp.path().join("cargo-port_feat");
            let member_dir = workspace_dir.join("crates").join("core");

            std::fs::create_dir_all(non_rust_dir.join(".git")).expect("create test directory");
            std::fs::create_dir_all(workspace_dir.join(".git")).expect("create test directory");
            std::fs::create_dir_all(primary_dir.join(".git")).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");
            std::fs::create_dir_all(&member_dir).expect("create test directory");

            let non_rust = RootItem::NonRust(NonRustProject::new(
                AbsolutePath::from(non_rust_dir.clone()),
                Some(".claude".to_string()),
            ));
            let workspace = RootItem::Rust(RustProject::Workspace(Workspace {
                path: AbsolutePath::from(workspace_dir.clone()),
                name: Some("bevy".to_string()),
                groups: vec![inline_group(vec![Package {
                    path: AbsolutePath::from(member_dir),
                    name: Some("core".to_string()),
                    ..Package::default()
                }])],
                ..Workspace::default()
            }));
            let primary = Package {
                path: AbsolutePath::from(primary_dir.clone()),
                name: Some("cargo-port".to_string()),
                worktree_status: WorktreeStatus::Primary {
                    root: AbsolutePath::from(primary_dir.clone()),
                },
                ..Package::default()
            };
            let linked = Package {
                path: AbsolutePath::from(linked_dir),
                name: Some("cargo-port_feat".to_string()),
                worktree_status: WorktreeStatus::Linked {
                    primary: AbsolutePath::from(primary_dir.clone()),
                },
                ..Package::default()
            };
            let worktrees = RootItem::Worktrees(WorktreeGroup::new(
                RustProject::Package(primary),
                vec![RustProject::Package(linked)],
            ));

            let mut app = make_app(&[]);
            apply_items(&mut app, &[non_rust, workspace, worktrees]);
            app.scan.state.phase = ScanPhase::Complete;

            app.initialize_startup_phase_tracker();

            assert_eq!(
                app.startup.git.expected.keys(),
                Some(&HashSet::from([
                    AbsolutePath::from(non_rust_dir.join(".git")),
                    AbsolutePath::from(workspace_dir.join(".git")),
                    AbsolutePath::from(primary_dir.join(".git")),
                ]))
            );
        }

        #[test]
        fn startup_git_seen_marks_owner_git_directory_for_member_updates() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let workspace_dir = tmp.path().join("bevy");
            let member_dir = workspace_dir.join("crates").join("core");
            std::fs::create_dir_all(workspace_dir.join(".git")).expect("create test directory");
            std::fs::create_dir_all(&member_dir).expect("create test directory");

            let workspace = RootItem::Rust(RustProject::Workspace(Workspace {
                path: AbsolutePath::from(workspace_dir.clone()),
                name: Some("bevy".to_string()),
                groups: vec![inline_group(vec![Package {
                    path: AbsolutePath::from(member_dir.clone()),
                    name: Some("core".to_string()),
                    ..Package::default()
                }])],
                ..Workspace::default()
            }));

            let mut app = make_app(&[]);
            apply_items(&mut app, &[workspace]);
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            apply_git_info(&mut app, member_dir.as_path(), make_git_info(None));

            assert!(
                app.startup
                    .git
                    .seen
                    .contains(workspace_dir.join(".git").as_path())
            );
        }

        #[test]
        fn lint_toast_reuses_existing_on_restart() {
            let temp_dir = tempfile::tempdir().expect("tempdir");
            let project_dir = temp_dir.path().join("a");
            std::fs::create_dir_all(&project_dir).expect("project dir");
            std::fs::write(
                project_dir.join("Cargo.toml"),
                "[package]\nname = \"a\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
            )
            .expect("cargo toml");
            let project = item_from_project_dir(&project_dir);
            let project_path = project.path().clone();
            let mut app = make_app(&[project]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            app.config.current_mut().lint.include =
                vec![project_path.to_string_lossy().to_string()];
            app.scan.state.phase = ScanPhase::Complete;

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path.clone(),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            let first_toast = app.lint.running_toast_id();
            assert!(first_toast.is_some());

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path.clone(),
                status: LintStatus::Passed(parse_ts("2026-03-30T14:23:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            assert_eq!(app.lint.running_toast_id(), first_toast);

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path,
                status: LintStatus::Running(parse_ts("2026-03-30T14:24:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            assert_eq!(app.lint.running_toast_id(), first_toast);
        }

        #[test]
        fn lint_toast_prunes_entries_that_are_not_running_in_project_state() {
            let project = make_project(Some("a"), "~/a");
            let mut app = make_app(std::slice::from_ref(&project));
            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/a"),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            assert!(
                app.lint
                    .running_toast_contains_path(test_path("~/a").as_path())
            );

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/a"),
                status: LintStatus::NoLog,
                origin: LintRunOrigin::Normal,
            });

            assert!(app.lint.running_toast_is_empty());
            assert!(lint_toast_running_items(&app).is_empty());
        }

        #[test]
        fn startup_catch_up_batch_titles_running_toast_distinctly() {
            let project = make_project(Some("a"), "~/a");
            let mut app = make_app(std::slice::from_ref(&project));

            // The runtime marks startup catch-up runs explicitly, so the first
            // catch-up running status creates the distinct catch-up toast.
            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/a"),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::CatchUp,
            });

            let titles: Vec<String> = app
                .framework
                .toasts
                .active_now()
                .iter()
                .map(|toast| toast.title().to_string())
                .collect();
            assert!(
                titles.iter().any(|title| title == "Catch-up lints"),
                "the catch-up batch titles the running toast distinctly: {titles:?}"
            );
            assert!(
                !titles.iter().any(|title| title == "Lints"),
                "no separate plain Lints toast is created for the catch-up batch: {titles:?}"
            );
        }

        #[test]
        fn normal_lints_do_not_append_to_active_catch_up_lint_toast() {
            let catch_up = make_project(Some("a"), "~/a");
            let normal = make_project(Some("b"), "~/b");
            let mut app = make_app(&[catch_up, normal]);

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/a"),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::CatchUp,
            });
            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   test_path("~/b"),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:19-05:00")),
                origin: LintRunOrigin::Normal,
            });

            let titles: Vec<String> = app
                .framework
                .toasts
                .active_now()
                .iter()
                .map(|toast| toast.title().to_string())
                .collect();
            assert!(
                titles.iter().any(|title| title == "Catch-up lints"),
                "catch-up run should keep its own toast: {titles:?}"
            );
            assert!(
                titles.iter().any(|title| title == "Lints"),
                "normal run should create its own toast: {titles:?}"
            );
            assert!(
                app.lint
                    .catch_up_running_toast_contains_path(test_path("~/a").as_path())
            );
            assert!(
                app.lint
                    .normal_running_toast_contains_path(test_path("~/b").as_path())
            );
            assert!(
                !app.lint
                    .catch_up_running_toast_contains_path(test_path("~/b").as_path())
            );
        }

        #[test]
        fn startup_lint_status_does_not_overwrite_live_running_lint() {
            let project = make_project(Some("a"), "~/a");
            let project_path = project.path().clone();
            let mut app = make_app(&[project]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            app.scan.state.phase = ScanPhase::Complete;

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path.clone(),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            let first_toast = app.lint.running_toast_id();

            app.handle_bg_msg(BackgroundMsg::LintStartupStatus {
                path:   project_path.clone(),
                status: CachedLintStatus::NoLog,
            });

            assert!(matches!(
                crate::tui::state::Lint::status_for_root(&app.project_list[0].root_item),
                LintStatus::Running(_)
            ));
            assert_eq!(app.lint.running_toast_id(), first_toast);
            assert!(app.lint.running_toast_contains_path(project_path.as_path()));
        }

        #[test]
        fn lint_history_load_does_not_overwrite_live_running_lint() {
            let project = make_project(Some("a"), "~/a");
            let project_path = project.path().clone();
            let mut app = make_app(&[project]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            app.scan.state.phase = ScanPhase::Complete;

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path.clone(),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            let first_toast = app.lint.running_toast_id();

            app.handle_bg_msg(BackgroundMsg::LintHistoryLoaded {
                entries: vec![(
                    project_path.clone(),
                    vec![LintRun {
                        run_id:        "previous".to_string(),
                        started_at:    "2026-03-30T13:22:18-05:00".to_string(),
                        finished_at:   Some("2026-03-30T13:23:18-05:00".to_string()),
                        duration_ms:   Some(60_000),
                        status:        LintRunStatus::Passed,
                        commands:      Vec::new(),
                        archive_bytes: 0,
                    }],
                )],
            });

            assert!(matches!(
                crate::tui::state::Lint::status_for_root(&app.project_list[0].root_item),
                LintStatus::Running(_)
            ));
            assert_eq!(app.lint.running_toast_id(), first_toast);
            assert!(app.lint.running_toast_contains_path(project_path.as_path()));
        }

        #[test]
        fn live_lint_status_updates_project_model_and_detail_cache() {
            let project = make_project(Some("a"), "~/a");
            let project_path = project.path().clone();
            let mut app = make_app(&[project]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            app.scan.state.phase = ScanPhase::Complete;
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            assert!(matches!(
                &app.panes.package.content().unwrap().lint_display,
                panes::LintDisplay::NoRuns
            ));
            let generation_before = app.scan.generation();

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path.clone(),
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });

            assert!(
                app.scan.generation() > generation_before,
                "live lint status must invalidate cached detail panes"
            );
            assert!(matches!(
                crate::tui::state::Lint::status_for_root(&app.project_list[0].root_item),
                LintStatus::Running(_)
            ));
            assert!(app.lint.running_toast_contains_path(project_path.as_path()));

            app.ensure_detail_cached();
            let display = app.panes.package.content().unwrap().lint_display.clone();
            assert!(
                matches!(
                    display,
                    panes::LintDisplay::Runs {
                        count:  0,
                        status: LintStatus::Running(_),
                    }
                ),
                "{display:?}"
            );
        }

        #[test]
        fn lint_runtime_projects_uses_workspace_root_not_members() {
            let workspace = make_workspace_project(Some("hana"), "~/rust/hana");
            let member_a = make_project(Some("hana_core"), "~/rust/hana/crates/hana_core");
            let member_b = make_project(Some("hana_ui"), "~/rust/hana/crates/hana_ui");
            let root = make_workspace_with_members(
                Some("hana"),
                "~/rust/hana",
                vec![inline_group(vec![
                    make_member(Some("hana_core"), "~/rust/hana/crates/hana_core"),
                    make_member(Some("hana_ui"), "~/rust/hana/crates/hana_ui"),
                ])],
            );

            let mut app = make_app(&[workspace, member_a, member_b]);
            apply_items(&mut app, &[root]);
            app.scan.state.phase = ScanPhase::Complete;

            let projects = app.lint_runtime_projects();
            assert_eq!(projects.len(), 1);
            assert_eq!(projects[0].abs_path, test_path("~/rust/hana"));
            assert_eq!(
                projects[0].project_label,
                crate::project::home_relative_path(test_path("~/rust/hana").as_path())
            );
        }

        #[test]
        fn lint_runtime_projects_deduplicates_primary_worktree_path() {
            let root_item = make_package_worktrees_item(
                make_package_raw(Some("ws"), "~/ws", None),
                vec![make_package_raw(
                    Some("ws_feat"),
                    "~/ws_feat",
                    Some("ws_feat"),
                )],
            );
            let feature_item = make_project(Some("ws_feat"), "~/ws_feat");

            let mut app = make_app(&[make_project(Some("ws"), "~/ws"), feature_item]);
            apply_items(&mut app, &[root_item]);
            app.scan.state.phase = ScanPhase::Complete;

            let projects = app.lint_runtime_projects();
            assert_eq!(projects.len(), 2);
            assert_eq!(projects[0].abs_path, test_path("~/ws"));
            assert_eq!(projects[1].abs_path, test_path("~/ws_feat"));
            assert_eq!(
                projects[0].project_label,
                crate::project::home_relative_path(test_path("~/ws").as_path())
            );
            assert_eq!(
                projects[1].project_label,
                crate::project::home_relative_path(test_path("~/ws_feat").as_path())
            );
        }

        #[test]
        fn vendored_path_dependency_becomes_ci_owner() {
            let root_item = {
                let pkg = Package {
                    path: test_path("~/app"),
                    name: Some("app".to_string()),
                    rust: RustInfo {
                        vendored: vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
                        ..RustInfo::default()
                    },
                    ..Package::default()
                };
                RootItem::Rust(RustProject::Package(pkg))
            };
            let vendored = make_project(Some("helper"), "~/app/vendor/helper");

            let mut app = make_app(&[make_project(Some("app"), "~/app"), vendored.clone()]);
            apply_items(&mut app, &[root_item]);

            assert!(app.project_list.is_vendored_path(vendored.path()));
            assert!(
                app.project_list.entry_containing(vendored.path()).is_some(),
                "vendored path should resolve to an owning ProjectEntry"
            );
        }

        #[test]
        fn member_vendored_path_receives_project_info_updates() {
            let vendored_path = test_path("~/app/vendor/helper");
            let member = make_package_with_vendored(
                Some("member"),
                "~/app/crates/member",
                vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
            );
            let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some("app"),
                "~/app",
                vec![inline_group(vec![member])],
                None,
            )));
            let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
            apply_items(&mut app, &[root_item]);

            app.handle_disk_usage(vendored_path.as_path(), 4097);
            app.project_list.handle_language_stats_batch(vec![(
                vendored_path.clone(),
                crate::project::LanguageStats {
                    entries: vec![crate::project::LangEntry {
                        language: "Rust".to_string(),
                        files:    1,
                        code:     7,
                        comments: 0,
                        blanks:   0,
                        children: Vec::new(),
                    }],
                },
            )]);
            app.project_list.handle_crates_io_version_msg(
                vendored_path.as_path(),
                "0.4.0".to_string(),
                None,
                3_208,
            );

            let vendored = app
                .project_list
                .vendored_at_path(vendored_path.as_path())
                .expect("member-owned vendored package should be addressable by path");
            assert_eq!(vendored.project_info.disk_usage_bytes, Some(4097));
            assert_eq!(
                vendored
                    .project_info
                    .language_stats
                    .as_ref()
                    .map(|s| s.entries.len()),
                Some(1)
            );
            assert_eq!(vendored.crates_version(), Some("0.4.0"));
            assert_eq!(vendored.crates_downloads(), Some(3_208));
        }

        #[test]
        fn project_refresh_preserves_crates_io_version() {
            let path = test_path("~/demo");
            let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);

            app.project_list.handle_crates_io_version_msg(
                path.as_path(),
                "0.20.2".to_string(),
                Some("0.21.0-rc.2".to_string()),
                663,
            );

            // A filesystem-triggered refresh re-scans the project. The fresh item has
            // no crates.io data (it is never persisted), so the refresh handler must
            // transfer the prior values rather than re-fetch from crates.io.
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectRefreshed {
                    item: make_project(Some("demo"), "~/demo"),
                },
            );

            let rust_info = app
                .project_list
                .rust_info_at_path(path.as_path())
                .expect("package should remain addressable after refresh");
            assert_eq!(rust_info.crates_version(), Some("0.20.2"));
            assert_eq!(rust_info.crates_prerelease(), Some("0.21.0-rc.2"));
            assert_eq!(rust_info.crates_downloads(), Some(663));
        }

        #[test]
        fn member_vendored_path_receives_cargo_metadata_fields() {
            let workspace_path = test_path("~/app");
            let vendored_path = test_path("~/app/vendor/helper");
            let member = make_package_with_vendored(
                Some("member"),
                "~/app/crates/member",
                vec![super::make_vendored(Some("helper"), "~/app/vendor/helper")],
            );
            let root_item = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some("app"),
                "~/app",
                vec![inline_group(vec![member])],
                None,
            )));
            let mut app = make_app(&[make_workspace_project(Some("app"), "~/app")]);
            apply_items(&mut app, &[root_item]);

            let record_id = PackageId {
                repr: "helper-id".into(),
            };
            let record = PackageRecord {
                name:          "helper".into(),
                version:       Version::new(0, 4, 0),
                edition:       "2024".into(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: AbsolutePath::from(vendored_path.as_path().join("Cargo.toml")),
                targets:       vec![crate::project::TargetRecord {
                    name:              "helper".into(),
                    kinds:             vec![TargetKind::Lib],
                    required_features: vec![],
                    src_path:          AbsolutePath::from(
                        vendored_path.as_path().join("src").join("lib.rs"),
                    ),
                }],
                publish:       PublishPolicy::Never,
            };
            let mut packages = HashMap::new();
            packages.insert(record_id, record);
            let workspace_metadata = WorkspaceMetadata {
                workspace_root: workspace_path,
                target_directory: test_path("~/app/target"),
                packages,
                fingerprint: fake_fingerprint(),
                out_of_tree_target_bytes: None,
            };

            app.project_list
                .apply_cargo_fields_from_workspace_metadata(&workspace_metadata);

            let cargo = &app
                .project_list
                .vendored_at_path(vendored_path.as_path())
                .expect("member-owned vendored package should receive cargo metadata")
                .cargo;
            assert!(
                cargo
                    .types()
                    .contains(&crate::project::ProjectType::Library)
            );
            assert!(!cargo.publishable());
        }

        #[test]
        fn git_status_suppresses_sync_for_untracked_and_ignored() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));

            let base_info = || -> (CheckoutInfo, RepoInfo) {
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        None,
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: Some((2, 0)),
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                )
            };

            apply_git_info(&mut app, project.path(), base_info());

            apply_git_info(&mut app, project.path(), {
                let mut info = base_info();
                info.0.status = GitStatus::Untracked;
                info
            });
            assert!(app.project_list.git_sync(project.path()).is_empty());

            apply_git_info(&mut app, project.path(), {
                let mut info = base_info();
                info.0.status = GitStatus::Ignored;
                info
            });
            assert!(app.project_list.git_sync(project.path()).is_empty());
        }

        #[test]
        fn background_git_info_updates_rendered_git_status() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.scan.state.phase = ScanPhase::Complete;

            apply_bg_msg(
                &mut app,
                BackgroundMsg::RepoInfo {
                    path: project.path().to_path_buf().into(),
                    info: RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        None,
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: Some((1, 0)),
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                },
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::CheckoutInfo {
                    path: project.path().to_path_buf().into(),
                    info: CheckoutInfo {
                        status:              GitStatus::Modified,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                },
            );
            assert_eq!(
                app.project_list.git_status_for(project.path()),
                Some(GitStatus::Modified)
            );

            apply_bg_msg(
                &mut app,
                BackgroundMsg::RepoInfo {
                    path: project.path().to_path_buf().into(),
                    info: RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        None,
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: Some((1, 0)),
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                },
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::CheckoutInfo {
                    path: project.path().to_path_buf().into(),
                    info: CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                },
            );
            assert_eq!(
                app.project_list.git_status_for(project.path()),
                Some(GitStatus::Clean)
            );
        }

        #[test]
        fn git_sync_shows_ascii_fill_for_local_only_branch() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));

            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  Some((3, 0)),
                        primary_tracked_ref: None,
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          None,
                            owner:        None,
                            repo:         None,
                            tracked_ref:  None,
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    None,
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );

            assert_eq!(app.project_list.git_sync(project.path()), NO_REMOTE_SYNC);
        }

        #[test]
        fn git_sync_shows_ascii_fill_for_branch_without_upstream() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));

            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feature/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  Some((2, 1)),
                        primary_tracked_ref: None,
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/natepiano/demo".to_string()),
                            owner:        Some("natepiano".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  None,
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );

            assert_eq!(app.project_list.git_sync(project.path()), NO_REMOTE_SYNC);
        }

        #[test]
        fn ci_pane_shows_all_runs_for_unpublished_branch_without_toggle() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.scan.state.phase = ScanPhase::Complete;
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("enh/various".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: None,
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/natepiano/demo".to_string()),
                            owner:        Some("natepiano".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  None,
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![CiRun {
                    branch: "main".to_string(),
                    ..make_ci_run(9, CiStatus::Passed)
                }],
                false,
                0,
            );

            // Unpublished branch (no upstream, not the default): the all/branch
            // toggle doesn't apply, so the pane shows every run unfiltered.
            assert!(!app.ci_toggle_available_for(project.path()));
            assert_eq!(
                app.project_list
                    .ci_runs_for_ci_pane(project.path(), &app.ci)
                    .iter()
                    .map(|run| run.branch.as_str())
                    .collect::<Vec<_>>(),
                vec!["main"]
            );

            let ci_data = panes::build_ci_data(&app);
            assert!(ci_data.mode_label.is_none());
            assert_eq!(ci_data.runs.len(), 1);
        }

        #[test]
        fn package_details_show_unpublished_branch_for_ci_when_branch_has_no_upstream() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.scan.state.phase = ScanPhase::Complete;
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("enh/various".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: None,
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/natepiano/demo".to_string()),
                            owner:        Some("natepiano".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  None,
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![CiRun {
                    branch: "main".to_string(),
                    ..make_ci_run(57, CiStatus::Passed)
                }],
                false,
                1,
            );
            app.ensure_detail_cached();

            let display = app
                .panes
                .package
                .content()
                .expect("package pane should have rendered test content")
                .ci_display;

            assert_eq!(display, crate::tui::state::CiDisplay::UnpublishedBranch);
        }

        #[test]
        fn git_main_shows_synced_for_non_main_branch_in_sync_with_main() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));

            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("feat/demo".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  Some((0, 0)),
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        None,
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: Some((0, 0)),
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );

            assert_eq!(app.project_list.git_main(project.path()), IN_SYNC);
        }

        #[test]
        fn git_first_commit_arriving_before_git_info_is_preserved() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            apply_bg_msg(
                &mut app,
                BackgroundMsg::GitFirstCommit {
                    path:         test_path("~/demo"),
                    first_commit: Some("2026-03-12T21:18:54-04:00".to_string()),
                },
            );
            let (checkout, repo) = make_git_info(Some("https://github.com/natepiano/demo"));
            apply_bg_msg(
                &mut app,
                BackgroundMsg::RepoInfo {
                    path: test_path("~/demo"),
                    info: repo,
                },
            );
            apply_bg_msg(
                &mut app,
                BackgroundMsg::CheckoutInfo {
                    path: test_path("~/demo"),
                    info: checkout,
                },
            );

            app.ensure_detail_cached();

            assert_eq!(
                app.project_list
                    .repo_info_for(test_path("~/demo").as_path())
                    .and_then(|repo| repo.first_commit.as_deref()),
                Some("2026-03-12T21:18:54-04:00")
            );
            assert!(
                app.panes
                    .git
                    .content()
                    .and_then(|g| g.inception.as_ref())
                    .is_some(),
                "detail panel should show Incept once git info arrives"
            );
        }

        #[test]
        fn git_info_invalidates_selected_git_pane_cache() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            assert_eq!(
                app.panes
                    .git
                    .content()
                    .and_then(|data| data.remotes.first())
                    .and_then(|row| row.full_url.as_deref()),
                None
            );

            apply_git_info(
                &mut app,
                test_path("~/demo").as_path(),
                make_git_info(Some("https://github.com/natepiano/demo")),
            );
            app.ensure_detail_cached();

            assert_eq!(
                app.panes
                    .git
                    .content()
                    .and_then(|data| data.remotes.first())
                    .and_then(|row| row.full_url.as_deref()),
                Some("https://github.com/natepiano/demo")
            );
        }

        #[test]
        fn ensure_detail_cached_short_circuits_when_nothing_changed() {
            let project_a = make_project(Some("alpha"), "~/alpha");
            let project_b = make_project(Some("beta"), "~/beta");
            let mut app = make_app(&[project_a, project_b]);
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            // Seed the cache.
            app.ensure_detail_cached();
            let after_seed = app.panes.pane_data.detail_build_count();
            assert!(after_seed >= 1, "first call must build");

            // Unchanged selection and generation — must NOT rebuild.
            app.ensure_detail_cached();
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.pane_data.detail_build_count(),
                after_seed,
                "idle frames must not rebuild the detail set"
            );

            // Bumping the data generation invalidates the stamp → must rebuild.
            app.scan.bump_generation();
            app.ensure_detail_cached();
            let after_generation_bump = app.panes.pane_data.detail_build_count();
            assert_eq!(
                after_generation_bump,
                after_seed + 1,
                "generation bump must trigger exactly one rebuild"
            );

            // Changing the selected row invalidates the stamp → must rebuild.
            app.project_list.set_cursor(1);
            app.sync_selected_project();
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.pane_data.detail_build_count(),
                after_generation_bump + 1,
                "selection change must trigger exactly one rebuild"
            );

            // Same selection, same generation, twice more — still no rebuild.
            app.ensure_detail_cached();
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.pane_data.detail_build_count(),
                after_generation_bump + 1,
                "further idle frames must not rebuild"
            );
        }

        #[test]
        fn worktree_summary_or_compute_caches_until_tree_mutation() {
            // Two distinct call sites for the *same* group root must hit the
            // cache on the second call — the closure must not run twice.
            let mut app = make_app(&[make_project(Some("demo"), "~/demo")]);
            let group_root = test_path("~/demo");
            let counter = std::sync::atomic::AtomicUsize::new(0);

            let _ = app
                .panes
                .git
                .worktree_summary_or_compute(group_root.as_path(), || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Vec::new()
                });
            let _ = app
                .panes
                .git
                .worktree_summary_or_compute(group_root.as_path(), || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Vec::new()
                });
            assert_eq!(
                counter.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "second lookup must hit the cache, not recompute"
            );

            // A `TreeMutation` guard going out of scope must invalidate the
            // cache via its `Drop` impl, regardless of whether any actual
            // mutation methods were called. This is the type-level guarantee
            // the guard exists to provide.
            {
                let _guard = app.mutate_tree();
                // Drop here.
            }

            let _ = app
                .panes
                .git
                .worktree_summary_or_compute(group_root.as_path(), || {
                    counter.fetch_add(1, Ordering::SeqCst);
                    Vec::new()
                });
            assert_eq!(
                counter.load(std::sync::atomic::Ordering::SeqCst),
                2,
                "after TreeMutation drops, the next lookup must recompute"
            );
        }

        #[test]
        fn background_message_for_unselected_path_does_not_invalidate_detail() {
            let project_a = make_project(Some("alpha"), "~/alpha");
            let project_b = make_project(Some("beta"), "~/beta");
            let mut app = make_app(&[project_a, project_b]);
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();
            let baseline = app.panes.pane_data.detail_build_count();

            // A disk-usage message for a *different* project must not bump the
            // detail-cache key. Watchers fire dozens of these per second; if they
            // each invalidate, the cache reduces to a no-op (the original
            // regression).
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  test_path("~/beta"),
                    bytes: 1024,
                },
            );
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.pane_data.detail_build_count(),
                baseline,
                "unrelated background messages must not invalidate the detail cache"
            );

            // A message for the *selected* path must still invalidate.
            apply_bg_msg(
                &mut app,
                BackgroundMsg::DiskUsage {
                    path:  test_path("~/alpha"),
                    bytes: 2048,
                },
            );
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.pane_data.detail_build_count(),
                baseline + 1,
                "messages affecting the selected path must rebuild exactly once"
            );
        }

        #[test]
        fn lint_rollups_distinguish_root_from_primary_worktree() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list
                .lint_at_path_mut(&test_path("~/ws"))
                .unwrap()
                .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
            app.project_list
                .lint_at_path_mut(&test_path("~/ws_feat"))
                .unwrap()
                .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

            let root_status = app.project_list.first().unwrap().lint_rollup_status();
            assert!(matches!(root_status, LintStatus::Failed(_)));

            let RootItem::Worktrees(g) = &app.project_list.first().unwrap().root_item else {
                panic!("expected Worktrees");
            };
            assert!(matches!(
                g.lint_status_for_worktree(0),
                LintStatus::Passed(_)
            ));
            assert!(matches!(
                g.lint_status_for_worktree(1),
                LintStatus::Failed(_)
            ));
        }

        #[test]
        fn lint_rollup_prefers_running_root_over_member_history() {
            let root = make_workspace_with_members(
                None,
                "~/ws",
                vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
            );

            let mut app = make_app(&[make_workspace_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list
                .lint_at_path_mut(&test_path("~/ws"))
                .unwrap()
                .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

            let root_status = app.project_list.first().unwrap().lint_rollup_status();
            assert!(matches!(root_status, LintStatus::Running(_)));
        }

        #[test]
        fn lint_rollup_prefers_running_worktree_over_failed_root_history() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list
                .lint_at_path_mut(&test_path("~/ws"))
                .unwrap()
                .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));
            app.project_list
                .lint_at_path_mut(&test_path("~/ws_feat"))
                .unwrap()
                .set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));

            let root_status = app.project_list.first().unwrap().lint_rollup_status();
            assert!(matches!(root_status, LintStatus::Running(_)));

            let RootItem::Worktrees(g) = &app.project_list.first().unwrap().root_item else {
                panic!("expected Worktrees");
            };
            assert!(matches!(
                g.lint_status_for_worktree(1),
                LintStatus::Running(_)
            ));
        }

        #[test]
        fn worktree_group_detail_lint_rollup_ignores_deleted_worktrees() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");
            app.project_list
                .at_path_mut(linked_path.as_path())
                .expect("linked worktree should exist")
                .visibility = Deleted;

            let make_lint_run = |run_id: &str, status| LintRun {
                run_id: run_id.to_string(),
                started_at: "2026-03-30T16:12:18-05:00".to_string(),
                finished_at: Some("2026-03-30T16:13:18-05:00".to_string()),
                duration_ms: Some(60_000),
                status,
                commands: Vec::new(),
                archive_bytes: 0,
            };
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![make_lint_run("primary", LintRunStatus::Passed)]);
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_runs(vec![make_lint_run("linked", LintRunStatus::Failed)]);
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let display = app.panes.package.content().unwrap().lint_display.clone();
            assert!(
                matches!(
                    display,
                    panes::LintDisplay::Runs {
                        count:  1,
                        status: LintStatus::Passed(_),
                    }
                ),
                "{display:?}"
            );
        }

        #[test]
        fn worktree_group_lints_pane_aggregates_every_checkout_newest_first() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");

            let run = |run_id: &str, started_at: &str| LintRun {
                run_id:        run_id.to_string(),
                started_at:    started_at.to_string(),
                finished_at:   None,
                duration_ms:   None,
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 0,
            };
            // Primary has one (older) run; the linked checkout has two newer ones.
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_runs(vec![
                    run("linked-2", "2026-03-30T12:00:00-04:00"),
                    run("linked-1", "2026-03-30T11:00:00-04:00"),
                ]);

            // Select the group parent (header) row.
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            let data = panes::build_lints_data(&app);

            // Every checkout's runs are merged, newest-first across checkouts.
            let ids: Vec<&str> = data.runs.iter().map(|r| r.run_id.as_str()).collect();
            assert_eq!(ids, vec!["linked-2", "linked-1", "primary-1"]);

            // Both checkouts are owners; each run resolves to the checkout it came
            // from, so its logs open against the right cache directory.
            assert_eq!(data.owner_paths.len(), 2);
            assert_eq!(data.owner_path_for_run(0), Some(&linked_path));
            assert_eq!(data.owner_path_for_run(1), Some(&linked_path));
            assert_eq!(data.owner_path_for_run(2), Some(&primary_path));
        }

        #[test]
        fn worktree_group_lints_pane_reindexes_when_a_new_run_lands() {
            // The owner index is not maintained incrementally — every new run bumps
            // the generation, which invalidates the detail cache and rebuilds the
            // whole merged list. This test drives that real refresh chain and
            // checks the rebuilt list re-sorts and the owner index follows.
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");

            let run = |run_id: &str, started_at: &str| LintRun {
                run_id:        run_id.to_string(),
                started_at:    started_at.to_string(),
                finished_at:   None,
                duration_ms:   None,
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 0,
            };
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00")]);

            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let before: Vec<&str> = app
                .lint
                .content()
                .unwrap()
                .runs
                .iter()
                .map(|r| r.run_id.as_str())
                .collect();
            assert_eq!(before, vec!["linked-1", "primary-1"]);

            // A newer run lands on the primary checkout (the loader replaces the
            // whole history per path). Bumping the generation invalidates the cache.
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![
                    run("primary-2", "2026-03-30T12:00:00-04:00"),
                    run("primary-1", "2026-03-30T10:00:00-04:00"),
                ]);
            app.scan.bump_generation();
            app.ensure_detail_cached();

            let data = app.lint.content().unwrap();
            let ids: Vec<&str> = data.runs.iter().map(|r| r.run_id.as_str()).collect();
            // Rebuilt newest-first, and the owner index realigns with the new order.
            assert_eq!(ids, vec!["primary-2", "linked-1", "primary-1"]);
            assert_eq!(data.owner_path_for_run(0), Some(&primary_path));
            assert_eq!(data.owner_path_for_run(1), Some(&linked_path));
            assert_eq!(data.owner_path_for_run(2), Some(&primary_path));
        }

        #[test]
        fn clear_history_on_group_parent_clears_every_checkout() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");

            let run = |run_id: &str, started_at: &str| LintRun {
                run_id:        run_id.to_string(),
                started_at:    started_at.to_string(),
                finished_at:   None,
                duration_ms:   None,
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 0,
            };
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![run("primary-1", "2026-03-30T10:00:00-04:00")]);
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00")]);

            // Select the group parent (header) row, where the pane aggregates every
            // checkout's history.
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            panes::dispatch_lints_action(LintsAction::ClearHistory, &mut app);

            // Both checkouts' histories are gone — not just the primary's — so the
            // rebuilt aggregate is empty instead of re-showing the linked runs.
            assert!(
                app.project_list
                    .lint_at_path_mut(&primary_path)
                    .unwrap()
                    .runs()
                    .is_empty()
            );
            assert!(
                app.project_list
                    .lint_at_path_mut(&linked_path)
                    .unwrap()
                    .runs()
                    .is_empty()
            );
            assert!(panes::build_lints_data(&app).runs.is_empty());
        }

        #[test]
        fn clear_history_toasts_run_count_and_freed_bytes_across_group() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            let primary_path = test_path("~/ws");
            let linked_path = test_path("~/ws_feat");

            let run = |run_id: &str, started_at: &str, archive_bytes: u64| LintRun {
                run_id: run_id.to_string(),
                started_at: started_at.to_string(),
                finished_at: None,
                duration_ms: None,
                status: LintRunStatus::Passed,
                commands: Vec::new(),
                archive_bytes,
            };
            // Two runs on the primary checkout, one on the linked checkout: three runs
            // totalling 3072 bytes (3.0 KiB) across the aggregate.
            app.project_list
                .lint_at_path_mut(&primary_path)
                .unwrap()
                .set_runs(vec![
                    run("primary-2", "2026-03-30T12:00:00-04:00", 1024),
                    run("primary-1", "2026-03-30T10:00:00-04:00", 1024),
                ]);
            app.project_list
                .lint_at_path_mut(&linked_path)
                .unwrap()
                .set_runs(vec![run("linked-1", "2026-03-30T11:00:00-04:00", 1024)]);

            app.project_list.set_cursor(0);
            app.sync_selected_project();

            panes::dispatch_lints_action(LintsAction::ClearHistory, &mut app);

            let toast = app
                .framework
                .toasts
                .active()
                .last()
                .expect("clearing lint history emits a toast");
            assert_eq!(toast.title(), "Lint history cleared");
            assert_eq!(toast.body_text(), "3 runs, 3.0 KiB freed");
        }

        #[test]
        fn clear_ci_cache_toasts_removed_run_count() {
            // Point the app cache root at a tempdir so the real on-disk CI cache is
            // untouched and `remove_dir_all` lands on the success branch (where the
            // run count is reported).
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.cache.root = tmp.path().to_string_lossy().into_owned();
            config::set_active_config(&cargo_port_config);

            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_git_info(
                &mut app,
                project.path(),
                (
                    CheckoutInfo {
                        status:              GitStatus::Clean,
                        head:                HeadState::Branch("main".to_string()),
                        last_commit:         None,
                        ahead_behind_local:  None,
                        primary_tracked_ref: Some("origin/main".to_string()),
                        bisect:              None,
                    },
                    RepoInfo {
                        remotes:           vec![RemoteInfo {
                            name:         "origin".to_string(),
                            url:          Some("https://github.com/acme/demo".to_string()),
                            owner:        Some("acme".to_string()),
                            repo:         Some("demo".to_string()),
                            tracked_ref:  Some("origin/main".to_string()),
                            ahead_behind: None,
                            kind:         RemoteKind::Clone,
                            push:         crate::project::PushState::Enabled {
                                push_url: String::new(),
                            },
                        }],
                        workflows:         WorkflowPresence::Present,
                        first_commit:      None,
                        last_fetched:      None,
                        default_branch:    Some("main".to_string()),
                        local_main_branch: Some("main".to_string()),
                    },
                ),
            );
            set_loaded_ci(
                &mut app,
                project.path(),
                vec![
                    make_ci_run(9, CiStatus::Passed),
                    make_ci_run(8, CiStatus::Failed),
                ],
                false,
                2,
            );
            // The repo's cache dir must exist for the clear to reach the success path.
            std::fs::create_dir_all(scan::ci_cache_dir_pub("acme", "demo").as_path())
                .expect("create CI cache directory");

            app.project_list.set_cursor(0);
            app.sync_selected_project();

            panes::dispatch_ci_runs_action(CiRunsAction::ClearCache, &mut app);

            let toast = app
                .framework
                .toasts
                .active()
                .last()
                .expect("clearing CI cache emits a toast");
            assert_eq!(toast.title(), "CI cache cleared");
            assert_eq!(toast.body_text(), "acme/demo: 2 runs");

            config::set_active_config(&CargoPortConfig::default());
        }

        #[test]
        fn worktree_group_detail_lint_rollup_rebuilds_when_linked_worktree_finishes() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list.set_cursor(0);
            app.sync_selected_project();

            let linked_path = test_path("~/ws_feat");
            let linked_lints = app.project_list.lint_at_path_mut(&linked_path).unwrap();
            linked_lints.set_runs(vec![LintRun {
                run_id:        "previous".to_string(),
                started_at:    "2026-03-30T16:12:18-05:00".to_string(),
                finished_at:   Some("2026-03-30T16:13:18-05:00".to_string()),
                duration_ms:   Some(60_000),
                status:        LintRunStatus::Passed,
                commands:      Vec::new(),
                archive_bytes: 0,
            }]);
            linked_lints.set_status(LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")));
            app.scan.bump_generation();
            app.ensure_detail_cached();
            let running_display = app.panes.package.content().unwrap().lint_display.clone();
            assert!(
                matches!(
                    running_display,
                    panes::LintDisplay::Runs {
                        status: LintStatus::Running(_),
                        ..
                    }
                ),
                "{running_display:?}"
            );

            apply_bg_msg(
                &mut app,
                BackgroundMsg::LintStatus {
                    path:   linked_path,
                    status: LintStatus::Passed(parse_ts("2026-03-30T16:23:18-05:00")),
                    origin: LintRunOrigin::Normal,
                },
            );
            app.ensure_detail_cached();
            let finished_display = app.panes.package.content().unwrap().lint_display.clone();

            assert!(
                !matches!(
                    finished_display,
                    panes::LintDisplay::Runs {
                        status: LintStatus::Running(_),
                        ..
                    }
                ),
                "{finished_display:?}"
            );
        }

        // ── CI fetch pipeline tests ───────────────────────────────────────────

        #[test]
        fn sync_does_not_mark_exhausted_when_no_new_runs() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let path = project.path().display().to_string();

            set_loaded_ci(
                &mut app,
                project.path(),
                vec![make_ci_run(5, CiStatus::Passed)],
                false,
                10,
            );

            // Sync returns the same run — no new runs found.
            app.handle_ci_fetch_complete(
                &path,
                CiFetchResult::Loaded {
                    runs:         vec![make_ci_run(5, CiStatus::Passed)],
                    github_total: 10,
                },
                CiFetchKind::Sync,
            );

            let state = loaded_ci(&app, project.path());
            assert!(
                !state.ci_pagination.is_exhausted(),
                "Sync should not mark exhausted when no new runs found"
            );
        }

        #[test]
        fn fetch_older_marks_exhausted_when_no_new_runs() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let path = project.path().display().to_string();

            set_loaded_ci(
                &mut app,
                project.path(),
                vec![make_ci_run(5, CiStatus::Passed)],
                false,
                10,
            );

            // `CiFetchKind::Older` returns the same run — no new runs found.
            app.handle_ci_fetch_complete(
                &path,
                CiFetchResult::Loaded {
                    runs:         vec![make_ci_run(5, CiStatus::Passed)],
                    github_total: 10,
                },
                CiFetchKind::Older,
            );

            let state = loaded_ci(&app, project.path());
            assert!(
                state.ci_pagination.is_exhausted(),
                "CiFetchKind::Older should mark exhausted when no new runs found"
            );
        }

        #[test]
        fn cache_only_preserves_github_total() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let path = project.path().display().to_string();

            set_loaded_ci(
                &mut app,
                project.path(),
                vec![make_ci_run(5, CiStatus::Passed)],
                false,
                57,
            );

            // CacheOnly (network failed) should preserve the previous github_total.
            app.handle_ci_fetch_complete(
                &path,
                CiFetchResult::CacheOnly(vec![make_ci_run(5, CiStatus::Passed)]),
                CiFetchKind::Sync,
            );

            let state = loaded_ci(&app, project.path());
            assert_eq!(
                state.github_total, 57,
                "CacheOnly should preserve previous github_total"
            );
        }

        #[test]
        fn sync_clears_exhaustion_when_new_runs_found() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let path = project.path().display().to_string();

            set_loaded_ci(
                &mut app,
                project.path(),
                vec![make_ci_run(5, CiStatus::Passed)],
                true,
                10,
            );

            // Sync finds a new run — should clear exhaustion.
            app.handle_ci_fetch_complete(
                &path,
                CiFetchResult::Loaded {
                    runs:         vec![
                        make_ci_run(6, CiStatus::Passed),
                        make_ci_run(5, CiStatus::Passed),
                    ],
                    github_total: 11,
                },
                CiFetchKind::Sync,
            );

            let state = loaded_ci(&app, project.path());
            assert!(
                !state.ci_pagination.is_exhausted(),
                "Sync should clear exhaustion when new runs found"
            );
            assert_eq!(state.runs.len(), 2);
        }

        #[test]
        fn fetch_more_uses_sync_when_no_cached_runs() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            apply_git_info(
                &mut app,
                project.path(),
                make_git_info(Some("https://github.com/natepiano/demo")),
            );

            // Empty CI state — no cached runs.
            set_loaded_ci(&mut app, project.path(), Vec::new(), false, 57);

            app.project_list
                .select_project_in_tree(project.path(), false);

            panes::handle_ci_runs_key(
                &mut app,
                &crossterm::event::KeyEvent::new(
                    KeyCode::Char('f'),
                    crossterm::event::KeyModifiers::NONE,
                ),
            );

            let fetch = app
                .inflight
                .pending_ci_fetch_ref()
                .expect("fetch should be set");
            assert!(
                matches!(fetch.ci_fetch_kind, CiFetchKind::Sync),
                "should use Sync when no cached runs exist"
            );
        }

        // ── Cargo metadata phase + arrival handling ─────────────────────────

        fn fake_fingerprint() -> ManifestFingerprint {
            // Fields are irrelevant to the handler's accept path: if
            // `capture()` on the workspace_root succeeds at runtime it will
            // produce a real fingerprint that (almost certainly) differs from
            // this one and the arrival gets dropped as drift. Tests use
            // workspace_root paths that don't exist on disk so `capture()`
            // fails (returns None) and the drift check becomes a no-op.
            ManifestFingerprint {
                manifest:       FileStamp {
                    content_hash: [0_u8; 32],
                },
                lockfile:       None,
                rust_toolchain: None,
                configs:        BTreeMap::new(),
            }
        }

        fn fake_metadata(workspace_root: &AbsolutePath) -> WorkspaceMetadata {
            WorkspaceMetadata {
                workspace_root:           workspace_root.clone(),
                target_directory:         AbsolutePath::from(
                    workspace_root.as_path().join("target"),
                ),
                packages:                 HashMap::new(),
                fingerprint:              fake_fingerprint(),
                out_of_tree_target_bytes: None,
            }
        }

        fn lint_toast_running_items(app: &App) -> Vec<String> {
            app.framework
                .toasts
                .active_now()
                .iter()
                .find(|toast| toast.title() == "Lints")
                .map(|toast| {
                    toast
                        .tracked_items()
                        .iter()
                        .filter(|item| item.linger_progress.is_none())
                        .map(|item| item.label.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        }

        #[test]
        fn initialize_startup_phase_seeds_metadata_expected() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let project_b = make_project(Some("b"), "~/never-real/b");
            let mut app = make_app(&[project_a.clone(), project_b.clone()]);
            app.scan.state.phase = ScanPhase::Complete;

            app.initialize_startup_phase_tracker();

            let expected = app
                .startup
                .metadata
                .expected
                .keys()
                .expect("metadata expected set is seeded at startup");
            assert_eq!(
                expected.len(),
                2,
                "one expected entry per Rust leaf, matching crate::tui::app::startup::initial_metadata_roots"
            );
            assert!(expected.contains(project_a.path()));
            assert!(expected.contains(project_b.path()));
        }

        /// Happy path: a successful arrival at the current generation inserts
        /// the metadata into the store and advances `metadata.seen`.
        #[test]
        fn successful_metadata_arrival_advances_phase() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("store lock")
                .next_generation(&workspace_root);

            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation,
                fingerprint: fake_fingerprint(),
                result: Ok(fake_metadata(&workspace_root)),
            });

            assert!(
                app.startup.metadata.seen.contains(&workspace_root),
                "metadata.seen records the arrived workspace"
            );
            assert!(
                app.scan
                    .metadata_store_handle()
                    .lock()
                    .expect("store lock")
                    .get(&workspace_root)
                    .is_some(),
                "successful metadata was upserted into the store"
            );
            assert!(
                app.startup.metadata.complete_at.is_some(),
                "with only one expected root, the phase completes on arrival"
            );
        }

        #[test]
        fn successful_metadata_arrival_clears_confirm_verifying() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));

            let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
            app.scan.set_confirm_verifying(Some(workspace_root.clone()));
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("store lock")
                .next_generation(&workspace_root);

            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation,
                fingerprint: fake_fingerprint(),
                result: Ok(fake_metadata(&workspace_root)),
            });

            assert!(
                app.scan.confirm_verifying().is_none(),
                "successful metadata arrival clears the Verifying flag"
            );
        }

        /// Race guard: an arrival stamped with a generation older than the
        /// current one is dropped. `metadata.seen` must not advance and the
        /// store must not upsert.
        #[test]
        fn stale_generation_metadata_arrival_is_dropped() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
            let store = app.scan.metadata_store_handle();
            let stale_gen = store
                .lock()
                .expect("store")
                .next_generation(&workspace_root);
            // A later dispatch bumps the generation; the stale arrival below
            // should be rejected.
            let _ = store
                .lock()
                .expect("store")
                .next_generation(&workspace_root);

            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation:     stale_gen,
                fingerprint:    fake_fingerprint(),
                result:         Ok(fake_metadata(&workspace_root)),
            });

            assert!(
                !app.startup.metadata.seen.contains(&workspace_root),
                "stale-generation arrival does not advance metadata.seen"
            );
            assert!(
                app.scan
                    .metadata_store_handle()
                    .lock()
                    .expect("store")
                    .get(&workspace_root)
                    .is_none(),
                "stale-generation arrival does not upsert"
            );
        }

        /// Error path: a failed arrival surfaces a "cargo metadata failed"
        /// timed toast and still ticks the phase forward (so startup doesn't
        /// wedge on a permanent failure).
        #[test]
        fn failed_metadata_arrival_surfaces_error_toast() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("store")
                .next_generation(&workspace_root);

            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation,
                fingerprint: fake_fingerprint(),
                result: Err(CargoMetadataError::Other(
                    "could not read Cargo.toml".into(),
                )),
            });

            let error_toast_present = app
                .framework
                .toasts
                .active_now()
                .iter()
                .any(|toast| toast.title().starts_with("cargo metadata failed"));
            assert!(
                error_toast_present,
                "failure raises a timed error toast starting with 'cargo metadata failed'"
            );
            assert!(
                app.startup.metadata.seen.contains(&workspace_root),
                "failure still ticks the phase forward so startup doesn't wedge"
            );
        }

        /// `WorkspaceMissing` (workspace deleted between dispatch and run, e.g. after
        /// the user removes a worktree) must NOT raise a user-facing toast — it's a
        /// stale-refresh race, not a real failure. Compare with the prior test which
        /// asserts `Other` does raise a toast: the two together pin down the
        /// dispatch contract on `CargoMetadataError`.
        #[test]
        fn cargo_metadata_workspace_missing_does_not_raise_toast() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let workspace_root = AbsolutePath::from(tmp.path().join("deleted_workspace"));
            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: workspace_root.clone(),
                name: Some("ghost".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);
            app.startup
                .metadata
                .reset_with_expected(std::iter::once(workspace_root.clone()).collect());

            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("store")
                .next_generation(&workspace_root);

            let toasts_before = app.framework.toasts.active_now().len();
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation,
                fingerprint: fake_fingerprint(),
                result: Err(CargoMetadataError::WorkspaceMissing),
            });

            assert_eq!(
                app.framework.toasts.active_now().len(),
                toasts_before,
                "WorkspaceMissing must not add any toast"
            );
            assert!(
                app.startup.metadata.seen.contains(&workspace_root),
                "WorkspaceMissing must still tick the phase forward"
            );
        }

        /// `start_clean` must prefer the workspace's resolved `target_directory`
        /// (from the metadata store) over the default `<project>/target` — that
        /// is the whole point of Step 2. Exercises three scenarios on a real
        /// tempdir to catch regressions in both the metadata lookup and the
        /// filesystem existence check.
        #[test]
        fn start_clean_prefers_resolved_target_dir_over_hardcoded_literal() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_path = AbsolutePath::from(tmp.path().join("proj"));
            let custom_target = AbsolutePath::from(tmp.path().join("out-of-tree-target"));
            std::fs::create_dir_all(project_path.as_path()).expect("create test directory");
            std::fs::create_dir_all(custom_target.as_path()).expect("create test directory");

            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);

            // Inject metadata pointing the project at the out-of-tree target.
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("store")
                .upsert(WorkspaceMetadata {
                    workspace_root:           project_path.clone(),
                    target_directory:         custom_target,
                    packages:                 HashMap::new(),
                    fingerprint:              fake_fingerprint(),
                    out_of_tree_target_bytes: None,
                });

            assert!(
                app.start_clean(&project_path),
                "out-of-tree target dir exists → clean is queued (would have missed with join(\"target\"))"
            );
            assert!(
                app.inflight
                    .clean()
                    .running
                    .contains_key(project_path.as_path())
            );
        }

        #[test]
        fn start_clean_reports_already_clean_when_resolved_target_is_missing() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_path = AbsolutePath::from(tmp.path().join("proj"));
            let custom_target = AbsolutePath::from(tmp.path().join("out-of-tree-target"));
            std::fs::create_dir_all(project_path.as_path()).expect("create test directory");
            // Also create the default `<project>/target` — this must NOT make the
            // check pass, because the *resolved* target sits elsewhere and doesn't
            // exist on disk.
            std::fs::create_dir_all(project_path.as_path().join("target"))
                .expect("create test directory");

            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("store")
                .upsert(WorkspaceMetadata {
                    workspace_root:           project_path.clone(),
                    target_directory:         custom_target,
                    packages:                 HashMap::new(),
                    fingerprint:              fake_fingerprint(),
                    out_of_tree_target_bytes: None,
                });

            assert!(
                !app.start_clean(&project_path),
                "resolved target doesn't exist → already clean; in-tree target/ decoy must not trip it"
            );
            assert!(app.inflight.clean().is_empty());
        }

        #[test]
        fn start_clean_falls_back_to_literal_target_when_no_metadata_yet() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_path = AbsolutePath::from(tmp.path().join("proj"));
            std::fs::create_dir_all(project_path.as_path().join("target"))
                .expect("create test directory");

            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);

            assert!(
                app.start_clean(&project_path),
                "no metadata → falls back to <project>/target, which exists → clean queued"
            );
            assert!(
                app.inflight
                    .clean()
                    .running
                    .contains_key(project_path.as_path())
            );
        }

        #[test]
        fn disk_usage_update_does_not_finish_running_clean() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_path = AbsolutePath::from(tmp.path().join("proj"));
            std::fs::create_dir_all(project_path.as_path().join("target"))
                .expect("create test directory");

            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);

            assert!(app.start_clean(&project_path));
            app.handle_disk_usage(project_path.as_path(), 0);

            assert!(
                app.inflight
                    .clean()
                    .running
                    .contains_key(project_path.as_path()),
                "disk usage can update before cargo clean exits, so it must not clear the running clean"
            );
        }

        #[test]
        fn clean_finished_message_finishes_running_clean() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let project_path = AbsolutePath::from(tmp.path().join("proj"));
            std::fs::create_dir_all(project_path.as_path().join("target"))
                .expect("create test directory");

            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);

            assert!(app.start_clean(&project_path));
            app.background
                .clean_sender()
                .send(CleanMsg::Finished(project_path.clone()))
                .expect("send clean finish");
            app.poll_background();

            assert!(
                !app.inflight
                    .clean()
                    .running
                    .contains_key(project_path.as_path()),
                "cargo clean process exit should clear the running clean"
            );
        }

        /// The metadata phase gates startup readiness: with disk, git, repo
        /// phases all resolved but metadata still pending, startup must not be
        /// marked complete. Once metadata arrives, the startup phase can close.
        #[test]
        fn startup_ready_waits_on_metadata_phase() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");

            // Force every phase except metadata complete (empty denominators
            // complete immediately) so only metadata gates startup readiness.
            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.maybe_complete_startup_disk(now, scan_started);
            app.maybe_complete_startup_git(now, scan_started);
            app.maybe_complete_startup_repo(now, scan_started);

            assert!(
                app.startup.disk.complete_at.is_some()
                    && app.startup.git.complete_at.is_some()
                    && app.startup.repo.complete_at.is_some(),
                "disk/git/repo phases are now complete"
            );
            assert!(
                app.startup.metadata.complete_at.is_none(),
                "metadata still pending"
            );
            app.maybe_complete_startup_ready(now, scan_started);
            assert!(
                app.startup.is_collecting(),
                "startup doesn't complete while metadata is still pending"
            );

            // Dispatch the metadata arrival → phase completes → startup ready.
            let workspace_root = AbsolutePath::from(project_a.path().as_path().to_path_buf());
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("store")
                .next_generation(&workspace_root);
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: workspace_root.clone(),
                generation,
                fingerprint: fake_fingerprint(),
                result: Ok(fake_metadata(&workspace_root)),
            });

            assert!(
                app.startup.metadata.complete_at.is_some(),
                "metadata phase completes after the arrival"
            );
            // Every phase has resolved, but the panel holds each row visible until
            // its minimum-visible floor elapses; advance past it and re-check.
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "startup is ready once every phase has resolved and the floor elapses"
            );
        }

        /// The languages row starts with project-root completion tokens, can add
        /// counted work tokens, and marks `seen` as final stats batches apply. The
        /// test-count row stays keyed on project roots.
        #[test]
        fn startup_languages_and_tests_rows_track_their_batches() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let root = project_a.path().clone();
            assert!(
                app.startup
                    .languages
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains(root.as_path())),
                "languages denominator is seeded from the project roots at scan start"
            );
            assert!(
                app.startup
                    .tests
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains(root.as_path())),
                "tests denominator is seeded from the project roots at scan start"
            );
            assert!(app.startup.languages.seen.is_empty());
            assert!(app.startup.tests.seen.is_empty());

            app.handle_bg_msg(BackgroundMsg::LanguageStatsProgressPlan { units: 1 });
            assert_eq!(
                app.startup.languages.work_expected, 1,
                "language progress plans add counted work tokens to the row denominator"
            );

            app.handle_bg_msg(BackgroundMsg::LanguageStatsBatch {
                entries: vec![(
                    root.clone(),
                    crate::project::LanguageStats { entries: vec![] },
                )],
            });
            assert_eq!(
                app.startup.languages.work_seen, 1,
                "language stats batches mark counted work tokens seen"
            );
            app.handle_bg_msg(BackgroundMsg::TestCountsBatch {
                entries: vec![(root.clone(), crate::project::TestCounts::default())],
            });

            assert!(
                app.startup.languages.seen.contains(root.as_path()),
                "a language-stats batch marks its project root seen on the languages row"
            );
            assert!(
                app.startup.tests.seen.contains(root.as_path()),
                "a test-counts batch marks its project root seen on the tests row"
            );
        }

        /// The crates.io row seeds its denominator upfront and holds the panel open
        /// until every seeded fetch reports complete.
        #[test]
        fn startup_crates_io_row_gates_until_fetches_complete() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");

            // Isolate crates.io: force every other row to an empty (immediately
            // complete) denominator, and seed crates.io with one expected crate.
            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected =
                Denominator::Stable(HashSet::from(["serde".to_string()]));
            app.startup.crates_io.stamp_first_seen(now);
            app.maybe_log_startup_phase_completions();

            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                app.startup.is_collecting(),
                "panel stays open while a crates.io fetch is still pending"
            );

            app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
                name: "serde".to_string(),
            });
            assert!(
                app.startup.crates_io.seen.contains("serde"),
                "a crates.io fetch-complete marks the crate seen on the row"
            );

            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "panel closes once the crates.io row finishes and the floor elapses"
            );
        }

        /// Regression for startup completing before the startup crates.io plan had
        /// been installed: zero-lint completion can fire immediately after startup
        /// begins, but the planned crates.io denominator must already be present.
        #[test]
        fn startup_plan_installs_crates_io_before_zero_lint_completion_can_close() {
            let project_a = make_project(Some("demo"), "~/never-real/demo");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            assert!(
                app.startup
                    .crates_io
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains("demo")),
                "startup plan seeds the crates.io row before completion checks can run"
            );

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.startup.details_declared.expected = Denominator::Stable(HashSet::new());
            app.handle_bg_msg(BackgroundMsg::LintStartupStatus {
                path:   project_a.path().clone(),
                status: CachedLintStatus::NoLog,
            });
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);

            assert!(
                app.startup.is_collecting(),
                "zero-lint completion cannot close Startup while planned crates.io work is pending"
            );

            app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
                name: "demo".to_string(),
            });
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "Startup can close after the planned crates.io fetch completes"
            );
        }

        #[test]
        fn startup_readiness_waits_for_project_detail_declarations() {
            let project_a = make_project(Some("demo"), "~/never-real/demo");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            let detail_path = AbsolutePath::from(project_a.path().as_path().to_path_buf());

            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.startup.details_declared.expected =
                Denominator::Stable(HashSet::from([detail_path.clone()]));
            app.startup.details_declared.complete_at = None;
            app.maybe_log_startup_phase_completions();
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);

            assert!(
                app.startup.is_collecting(),
                "Startup cannot close before planned detail workers declare follow-up work"
            );

            app.handle_bg_msg(BackgroundMsg::ProjectDetailsDeclared { path: detail_path });
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "Startup can close after detail declarations are complete"
            );
        }

        /// A repo fetch queued after the GitHub row already completed reopens the
        /// row, so the panel waits for the late fetch instead of closing early.
        #[test]
        fn startup_late_repo_fetch_reopens_github_row() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");

            // Git terminal + empty repo set → the GitHub row completes.
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.maybe_complete_startup_git(now, scan_started);
            app.maybe_complete_startup_repo(now, scan_started);
            assert!(
                app.startup.repo.complete_at.is_some(),
                "GitHub row completes when git is terminal and no repos are queued"
            );

            // A repo fetch queued after that completion reopens the row.
            let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
            app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });
            assert!(
                app.startup.repo.complete_at.is_none(),
                "a late repo fetch reopens the completed GitHub row"
            );
            assert!(
                app.startup
                    .repo
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains(&repo)),
                "the late repo joins the GitHub denominator"
            );

            // Completing it marks it seen and re-completes the row.
            app.handle_bg_msg(BackgroundMsg::RepoFetchComplete { repo: repo.clone() });
            assert!(
                app.startup.repo.seen.contains(&repo) && app.startup.repo.complete_at.is_some(),
                "completing the late fetch marks it seen and re-completes the row"
            );
        }

        /// A re-fetch of an already-seen repo un-marks it and reopens the GitHub row,
        /// so the panel cannot close while the live tracker still contains that repo.
        #[test]
        fn startup_repo_refetch_reopens_completed_github_row() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.maybe_complete_startup_git(now, scan_started);
            app.startup.repo.expected = Denominator::Stable(HashSet::from([repo.clone()]));
            app.startup.repo.seen.insert(repo.clone());
            app.maybe_complete_startup_repo(now, scan_started);
            assert!(
                app.startup.repo.complete_at.is_some(),
                "the seeded repo row starts complete"
            );

            app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });
            assert!(
                !app.startup.repo.seen.contains(&repo),
                "a queued re-fetch un-marks the repo"
            );
            assert!(
                app.startup.repo.complete_at.is_none(),
                "a queued re-fetch reopens the completed GitHub row"
            );
        }

        /// A crates.io fetch queued while the startup panel is open joins the
        /// denominator, and a re-fetch of an already-seen name un-marks it — so
        /// the row cannot read done while any registered fetch is in flight.
        #[test]
        fn startup_late_crates_io_fetch_reopens_row() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            // Seed one expected crate and complete it — the row is done.
            app.startup.crates_io.expected =
                Denominator::Stable(HashSet::from(["serde".to_string()]));
            app.startup
                .crates_io
                .stamp_first_seen(std::time::Instant::now());
            app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
                name: "serde".to_string(),
            });
            assert!(
                app.startup.crates_io.complete_at.is_some(),
                "row completes once the seeded fetch reports"
            );

            // A re-fetch of the same name un-marks it and reopens the row.
            app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
                name: "serde".to_string(),
            });
            assert!(
                !app.startup.crates_io.seen.contains("serde"),
                "a queued re-fetch un-marks the name"
            );
            assert!(
                app.startup.crates_io.complete_at.is_none(),
                "a queued re-fetch reopens the completed row"
            );

            // A fetch for a name outside the plan joins the denominator.
            app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
                name: "tokio".to_string(),
            });
            assert!(
                app.startup
                    .crates_io
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains("tokio")),
                "a late fetch joins the crates.io denominator"
            );

            // Completing both re-completes the row.
            app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
                name: "serde".to_string(),
            });
            app.handle_bg_msg(BackgroundMsg::CratesIoFetchComplete {
                name: "tokio".to_string(),
            });
            assert!(
                app.startup.crates_io.complete_at.is_some(),
                "completing the late fetches re-completes the row"
            );
        }

        // ── network-toast stage (startup-owned vs steady state) ────────────

        /// The network-toast stage is a three-state machine: it starts `StartupOwned`,
        /// `begin_steady_state_network_toasts` installs the slots, and
        /// `set_network_toasts_startup_owned` removes them again (the rescan path).
        #[test]
        fn network_toast_stage_round_trips_startup_owned_and_steady() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));

            assert!(
                app.net.network_toasts().is_none(),
                "construction starts in the startup-owned stage — no standalone slot"
            );
            begin_steady_state_network_toasts_for_test(&mut app);
            assert!(
                app.net.network_toasts().is_some(),
                "entering steady state installs the standalone-toast slots"
            );
            app.net.set_network_toasts_startup_owned();
            assert!(
                app.net.network_toasts().is_none(),
                "returning to startup-owned discards the slots"
            );
        }

        /// While the startup panel owns the network rows the stage is `StartupOwned`:
        /// a queued crates.io fetch is still tracked in flight (the panel's detail row
        /// reads it), but no standalone-toast slot exists, so the "Fetching crates.io
        /// info" toast cannot be created.
        #[test]
        fn startup_owned_stage_suppresses_crates_io_standalone_toast() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            assert!(
                app.net.network_toasts().is_none(),
                "the open startup panel owns the network rows — no standalone slot exists"
            );

            app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
                name: "serde".to_string(),
            });

            assert!(
                app.net.crates_io_running().running.contains_key("serde"),
                "the queued fetch is still tracked in flight for the panel's detail row"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "no standalone crates.io toast slot is created while the panel owns the row"
            );
        }

        /// While the startup panel owns the network rows, a queued GitHub fetch is
        /// tracked for the panel detail line but cannot create the standalone
        /// "Retrieving GitHub repo details" toast.
        #[test]
        fn startup_owned_stage_suppresses_github_standalone_toast() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            assert!(
                app.net.network_toasts().is_none(),
                "the open startup panel owns the network rows"
            );

            let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");
            app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });

            assert!(
                app.net.github_running().running.contains_key(&repo),
                "the queued fetch is tracked in flight for the panel's detail row"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "no standalone GitHub toast slot is created while startup owns the row"
            );
        }

        /// Even if row bookkeeping regresses and every visible startup row looks
        /// gate-satisfied, startup readiness is not constructible while a
        /// startup-owned GitHub tracker still has running work.
        #[test]
        fn startup_readiness_waits_for_running_github_tracker() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.maybe_log_startup_phase_completions();
            app.net
                .github_running_mut()
                .insert(repo, std::time::Instant::now());

            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                app.startup.is_collecting(),
                "startup cannot close while startup-owned GitHub work is still running"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "the failed handoff does not install standalone network-toast slots"
            );
        }

        /// The spawn→queue window: a repo-fetch worker is registered in
        /// `repo_fetch_in_flight` at spawn but only reaches the `github_running`
        /// tracker once it sends `RepoFetchQueued`. Startup must not hand off to
        /// steady state in that window, or the queue message lands after the panel
        /// closes and leaks a standalone "Retrieving GitHub repo details" toast.
        #[test]
        fn startup_readiness_waits_for_spawned_but_unqueued_repo_fetch() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());

            // The worker thread is spawned (registered in flight) but has not yet sent
            // `RepoFetchQueued`, so the `github_running` tracker stays empty — the row
            // and the network gate would both read drained without this guard. In the
            // real flow `RepoInfo` registers the fetch before the `CheckoutInfo` that
            // marks git terminal, so the row never completes first; clear the row's
            // init-time completion to model that ordering.
            app.net.github.repo_fetch_in_flight_mut().insert(repo);
            app.startup.repo.complete_at = None;
            app.maybe_log_startup_phase_completions();

            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                app.startup.is_collecting(),
                "startup cannot close while a repo fetch is spawned but not yet queued"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "the panel keeps owning the network rows until the spawned fetch drains"
            );
        }

        /// The reported leak: a crates.io fetch processed before the scan completes
        /// must not pop a standalone toast, and initializing the startup tracker must
        /// seed the row from that already-running startup-owned tracker.
        #[test]
        fn crates_io_fetch_before_startup_panel_is_suppressed() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            // No `initialize_startup_phase_tracker`: the scan has not completed.

            assert!(
                app.net.network_toasts().is_none(),
                "the network-toast stage starts `StartupOwned` before any panel exists"
            );

            app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
                name: "serde".to_string(),
            });

            assert!(
                app.net.crates_io_running().running.contains_key("serde"),
                "the fetch is tracked in flight even before the panel opens"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "a fetch processed before the panel exists cannot leak a standalone toast"
            );

            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();
            assert!(
                app.startup
                    .crates_io
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains("serde")),
                "startup initialization preserves the pre-panel crates.io obligation"
            );
        }

        /// Same pre-panel leak guard for GitHub: a repo queued before the Startup
        /// panel exists is owned by the startup network stage and seeds the GitHub row
        /// when the tracker initializes.
        #[test]
        fn github_fetch_before_startup_panel_seeds_startup_row() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            let repo = crate::ci::OwnerRepo::new("pcwalton", "glTF-IBL-Sampler");

            assert!(
                app.net.network_toasts().is_none(),
                "the network-toast stage starts `StartupOwned` before any panel exists"
            );

            app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo: repo.clone() });

            assert!(
                app.net.github_running().running.contains_key(&repo),
                "the fetch is tracked in flight before the panel opens"
            );
            assert!(
                app.net.network_toasts().is_none(),
                "a pre-panel GitHub fetch cannot create a standalone toast"
            );

            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();
            assert!(
                app.startup
                    .repo
                    .expected
                    .keys()
                    .is_some_and(|expected| expected.contains(&repo)),
                "startup initialization preserves the pre-panel GitHub obligation"
            );
        }

        /// When startup completes, the panel hands the network rows back: the stage
        /// flips to `SteadyState`, installing the toast slots. A crates.io fetch
        /// queued afterward then creates its standalone toast.
        #[test]
        fn startup_completion_enters_steady_state_and_emits_crates_io_toast() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");

            // Force every row to an empty (immediately complete) denominator so the
            // panel can close once the minimum-visible floor elapses.
            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.maybe_log_startup_phase_completions();

            assert!(
                app.net.network_toasts().is_none(),
                "the panel still owns the rows until it closes"
            );

            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "the panel closes once every row is complete past its floor"
            );
            assert!(
                app.net
                    .network_toasts()
                    .is_some_and(|toasts| toasts.crates_io.is_none()),
                "panel close enters steady state with empty slots — no fetch has run yet"
            );
            let startup_toast = app
                .framework
                .toasts
                .active_now()
                .into_iter()
                .find(|toast| toast.title() == "Startup")
                .expect("Startup countdown toast should still be visible");
            assert_eq!(
                startup_toast.linger_progress(),
                None,
                "Startup countdown must not use task linger fade"
            );
            assert!(
                startup_toast.remaining_secs().is_some(),
                "Startup countdown should still show Closing in N"
            );

            app.handle_bg_msg(BackgroundMsg::CratesIoFetchQueued {
                name: "serde".to_string(),
            });
            assert!(
                app.net
                    .network_toasts()
                    .is_some_and(|toasts| toasts.crates_io.is_some()),
                "a steady-state crates.io fetch creates the standalone toast"
            );
        }

        /// In steady state a GitHub repo fetch creates the standalone "Retrieving
        /// GitHub repo details" toast — the mirror of the crates.io path.
        #[test]
        fn steady_state_repo_fetch_emits_github_toast() {
            let project_a = make_project(Some("a"), "~/never-real/a");
            let mut app = make_app(std::slice::from_ref(&project_a));
            finish_startup_for_test(&mut app);

            let repo = crate::ci::OwnerRepo::new("natepiano", "cargo-port");
            app.handle_bg_msg(BackgroundMsg::RepoFetchQueued { repo });

            assert!(
                app.net
                    .network_toasts()
                    .is_some_and(|toasts| toasts.github.is_some()),
                "a steady-state repo fetch creates the standalone GitHub toast"
            );
        }

        fn begin_steady_state_network_toasts_for_test(app: &mut App) {
            let StartupNetworkReadiness::Ready(ready) =
                app.net.startup_network_readiness(false, false)
            else {
                panic!("startup network should be ready");
            };
            app.net.begin_steady_state_network_toasts(&ready);
        }

        fn finish_startup_for_test(app: &mut App) {
            app.scan.state.phase = ScanPhase::Complete;
            app.initialize_startup_phase_tracker();

            let now = std::time::Instant::now();
            let scan_started = app.startup.scan_complete_at.expect("scan complete at");
            app.startup.disk.expected = Denominator::Stable(HashSet::new());
            app.startup.git.expected = Denominator::Stable(HashSet::new());
            app.startup.repo.expected = Denominator::Stable(HashSet::new());
            app.startup.crates_io.expected = Denominator::Stable(HashSet::new());
            app.startup.metadata.expected = Denominator::Stable(HashSet::new());
            app.startup.languages.expected = Denominator::Stable(HashSet::new());
            app.startup.tests.expected = Denominator::Stable(HashSet::new());
            app.startup.lint_phase.expected = Denominator::Stable(HashSet::new());
            app.maybe_log_startup_phase_completions();
            app.maybe_complete_startup_ready(now + STARTUP_ROW_MIN_VISIBLE * 2, scan_started);
            assert!(
                !app.startup.is_collecting(),
                "test setup should close startup"
            );
        }

        // ── App::clean_selection (Step 6c gating) ──────────────────────────

        #[test]
        fn clean_selection_on_root_rust_project_returns_project_selection() {
            let project = make_project(Some("demo"), "~/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            app.project_list.set_cursor(0);

            let selection = app
                .project_list
                .clean_selection()
                .expect("Rust root should be clean-eligible");
            match selection {
                CleanSelection::Project { root } => {
                    assert_eq!(root, test_path("~/demo"));
                },
                CleanSelection::WorktreeGroup { .. } => {
                    panic!("single Rust root should not yield a worktree-group selection")
                },
            }
        }

        #[test]
        fn clean_selection_on_non_rust_root_is_none() {
            // The gating fix must not regress non-Rust rows: they stay
            // clean-ineligible so the shortcut is dimmed in the status bar.
            let non_rust = make_non_rust_project(Some("notes"), "~/notes");
            let mut app = make_app(std::slice::from_ref(&non_rust));
            app.project_list.set_cursor(0);
            assert!(app.project_list.clean_selection().is_none());
        }

        #[test]
        fn clean_selection_on_worktree_group_root_fans_out_to_primary_and_linked() {
            // A Root row whose RootItem is a WorktreeGroup produces a
            // CleanSelection::WorktreeGroup naming the primary checkout plus
            // every linked worktree. build_clean_plan then dedupes on
            // target_directory — shared-target worktrees collapse into a
            // single CleanTarget with multiple covering_projects.
            let primary_path = test_path("~/cargo-port");
            let linked_path = test_path("~/cargo-port_feat");
            let primary = Package {
                path: primary_path.clone(),
                name: Some("cargo-port".to_string()),
                worktree_status: WorktreeStatus::Primary {
                    root: primary_path.clone(),
                },
                ..Package::default()
            };
            let linked = Package {
                path: linked_path.clone(),
                name: Some("cargo-port_feat".to_string()),
                worktree_status: WorktreeStatus::Linked {
                    primary: primary_path.clone(),
                },
                ..Package::default()
            };
            let worktrees = RootItem::Worktrees(WorktreeGroup::new(
                RustProject::Package(primary),
                vec![RustProject::Package(linked)],
            ));
            let mut app = make_app(std::slice::from_ref(&worktrees));
            app.project_list.set_cursor(0);

            match app
                .project_list
                .clean_selection()
                .expect("group root is clean-eligible")
            {
                CleanSelection::WorktreeGroup { primary, linked } => {
                    assert_eq!(primary, primary_path);
                    assert_eq!(linked, vec![linked_path]);
                },
                CleanSelection::Project { .. } => {
                    panic!("WorktreeGroup root should fan out, not reduce to a single Project")
                },
            }
        }

        #[test]
        fn request_clean_confirm_opens_ready_when_fingerprint_matches() {
            // When the stored metadata's fingerprint still matches disk, the
            // confirm popup opens immediately — no verifying state, no extra
            // metadata dispatch. Covers the happy path.
            let project = make_project(Some("demo"), "~/never-real/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let workspace_root = AbsolutePath::from(project.path().as_path().to_path_buf());

            // Seed metadata with a fingerprint the real disk can't match
            // (the project path doesn't exist). capture() will fail on the
            // non-existent path, and `should_verify_before_clean` treats
            // capture failure as "no drift" → Ready.
            app.scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .upsert(fake_metadata(&workspace_root));

            app.request_clean_confirm(workspace_root);

            assert!(
                app.scan.confirm_verifying().is_none(),
                "capture failure (test path doesn't exist) → no verifying state"
            );
            assert!(app.confirm().is_some(), "popup opens immediately in Ready");
        }

        #[test]
        fn request_clean_confirm_marks_verifying_when_no_metadata_covers_path() {
            // No metadata → nothing to verify against → flag stays Verifying
            // until metadata arrives. `request_clean_confirm` also spawns
            // a cargo metadata refresh; the async task owns the eventual
            // arrival/generation, so this test only pins the synchronous flag.
            let project = make_project(Some("demo"), "~/never-real/demo");
            let mut app = make_app(std::slice::from_ref(&project));
            let workspace_root = AbsolutePath::from(project.path().as_path().to_path_buf());

            app.request_clean_confirm(workspace_root.clone());

            assert_eq!(
                app.scan.confirm_verifying(),
                Some(&workspace_root),
                "missing metadata → confirm opens in Verifying state, \
                 pending on this workspace root"
            );
        }

        #[test]
        fn out_of_tree_target_size_message_stamps_metadata() {
            // Inject metadata with an out-of-tree target, then route an
            // OutOfTreeTargetSize arrival through handle_bg_msg. The byte total
            // should land on `WorkspaceMetadata::out_of_tree_target_bytes`.
            let workspace_root = AbsolutePath::from(PathBuf::from("/ws"));
            let target_dir = AbsolutePath::from(PathBuf::from("/elsewhere/target"));
            let pkg = RootItem::Rust(RustProject::Package(Package {
                path: workspace_root.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg]);
            {
                let store = app.scan.metadata_store_handle();
                let mut guard = store.lock().expect("lock test metadata store");
                guard.upsert(WorkspaceMetadata {
                    workspace_root:           workspace_root.clone(),
                    target_directory:         target_dir.clone(),
                    packages:                 HashMap::new(),
                    fingerprint:              fake_fingerprint(),
                    out_of_tree_target_bytes: None,
                });
            }

            app.handle_bg_msg(BackgroundMsg::OutOfTreeTargetSize {
                workspace_root: workspace_root.clone(),
                target_dir,
                bytes: 1_234_567,
            });

            let stamped = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .get(&workspace_root)
                .and_then(|s| s.out_of_tree_target_bytes);
            assert_eq!(stamped, Some(1_234_567));
        }

        #[test]
        fn cargo_metadata_arrival_stamps_cargo_fields_onto_package() {
            let project_path = AbsolutePath::from(PathBuf::from("/abs/demo"));
            let pkg_item = RootItem::Rust(RustProject::Package(Package {
                path: project_path.clone(),
                name: Some("demo".into()),
                ..Package::default()
            }));
            let mut app = make_app(&[pkg_item]);

            // Before metadata arrival: Cargo::default() → publishable true but
            // empty types / examples / benches.
            let pre_types = app
                .project_list
                .rust_info_at_path(project_path.as_path())
                .map_or(0, |r| r.cargo.types().len());
            assert_eq!(pre_types, 0, "pre-metadata types stay empty");

            let manifest_path = AbsolutePath::from(project_path.as_path().join("Cargo.toml"));
            let example_src =
                AbsolutePath::from(project_path.as_path().join("examples").join("hello.rs"));
            let bin_src = AbsolutePath::from(project_path.as_path().join("src").join("main.rs"));
            let record_id = PackageId {
                repr: "demo-id".into(),
            };
            let record = PackageRecord {
                name: "demo".into(),
                version: Version::new(0, 1, 0),
                edition: "2024".into(),
                description: None,
                license: None,
                homepage: None,
                repository: None,
                manifest_path,
                targets: vec![
                    crate::project::TargetRecord {
                        name:              "demo".into(),
                        kinds:             vec![TargetKind::Bin],
                        required_features: vec![],
                        src_path:          bin_src,
                    },
                    crate::project::TargetRecord {
                        name:              "hello".into(),
                        kinds:             vec![TargetKind::Example],
                        required_features: vec![],
                        src_path:          example_src,
                    },
                ],
                publish: PublishPolicy::Never,
            };
            let mut packages = HashMap::new();
            packages.insert(record_id, record);

            let workspace_metadata = WorkspaceMetadata {
                workspace_root: project_path.clone(),
                target_directory: AbsolutePath::from(project_path.as_path().join("target")),
                packages,
                fingerprint: fake_fingerprint(),
                out_of_tree_target_bytes: None,
            };
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("lock test store")
                .next_generation(&project_path);
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root: project_path.clone(),
                generation,
                fingerprint: workspace_metadata.fingerprint.clone(),
                result: Ok(workspace_metadata),
            });

            let cargo = app
                .project_list
                .rust_info_at_path(project_path.as_path())
                .map(|r| r.cargo.clone())
                .expect("test project should have Rust info after metadata update");
            assert!(
                cargo.types().contains(&crate::project::ProjectType::Binary),
                "Bin TargetKind → ProjectType::Binary stamped from metadata"
            );
            assert_eq!(
                cargo.example_count(),
                1,
                "Example TargetKind populates Cargo.examples"
            );
            assert!(
                !cargo.publishable(),
                "PublishPolicy::Never → Cargo.publishable false after metadata"
            );
        }

        #[test]
        fn apply_lint_config_change_fans_out_to_inflight_scan_and_selection() {
            let project = make_project(Some("demo"), "~/demo");
            let project_path = project.path().clone();
            let mut app = make_app(&[project]);

            // Seed a real project-model running lint so we can prove the orchestrator
            // clears project lint state and reconciles the toast from it.
            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   project_path,
                status: LintStatus::Running(parse_ts("2026-03-30T14:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });
            assert!(!app.lint.running_toast_is_empty());

            // App-shell scan state: capture the pre-call generation.
            let gen_before = app.scan.generation();

            // Selection: replace fit_widths with a sentinel generation so we
            // can prove reset_fit_widths fired (reset re-seeds with
            // `generation: u64::MAX`, which is the construct-time default).
            {
                let widths = app.project_list.fit_widths_mut();
                widths.generation = 0;
            }
            assert_eq!(app.project_list.cached_fit_widths.generation, 0);

            let cargo_port_config = app.config.current().clone();
            app.apply_lint_config_change(&cargo_port_config);

            // Projection: running-lint paths cleared, lint runtime present
            // (re-spawned).
            assert!(
                app.lint.running_toast_is_empty(),
                "apply_lint_config_change must clear running lint projection"
            );
            // Scan: data_generation bumped exactly once.
            assert_eq!(
                app.scan.generation(),
                gen_before + 1,
                "apply_lint_config_change must bump data_generation"
            );
            // Selection: fit_widths reset (back to construct-time sentinel).
            assert_eq!(
                app.project_list.cached_fit_widths.generation,
                u64::MAX,
                "apply_lint_config_change must reset fit_widths"
            );
        }
    }
    mod worktrees {
        use std::collections::BTreeMap;
        use std::collections::HashMap;
        use std::time::Duration;

        use cargo_metadata::PackageId;
        use cargo_metadata::TargetKind;
        use cargo_metadata::semver::Version;
        use notify::event::DataChange;
        use notify::event::EventKind;
        use notify::event::ModifyKind;

        use super::*;
        use crate::config::DiscoveryLint;
        use crate::config::LintIndicator;
        use crate::lint;
        use crate::project::FileStamp;
        use crate::project::ManifestFingerprint;
        use crate::project::PackageRecord;
        use crate::project::PublishPolicy;
        use crate::project::TargetRecord;
        use crate::project::WorkspaceMetadata;
        use crate::scan;
        use crate::tui::keymap::TargetsAction;
        use crate::tui::panes;

        fn metadata_with_example(
            root: &AbsolutePath,
            package_name: &str,
            example_name: &str,
        ) -> WorkspaceMetadata {
            let target = TargetRecord {
                name:              example_name.to_string(),
                kinds:             vec![TargetKind::Example],
                src_path:          AbsolutePath::from(
                    root.as_path()
                        .join("examples")
                        .join(format!("{example_name}.rs")),
                ),
                required_features: Vec::new(),
            };
            let package = PackageRecord {
                name:          package_name.to_string(),
                version:       Version::new(0, 1, 0),
                edition:       "2021".to_string(),
                description:   None,
                license:       None,
                homepage:      None,
                repository:    None,
                manifest_path: AbsolutePath::from(root.as_path().join("Cargo.toml")),
                targets:       vec![target],
                publish:       PublishPolicy::Any,
            };
            let mut packages = HashMap::new();
            packages.insert(
                PackageId {
                    repr: format!("{package_name}-{}", root.display()),
                },
                package,
            );

            WorkspaceMetadata {
                workspace_root: root.clone(),
                target_directory: AbsolutePath::from(root.as_path().join("target")),
                packages,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            }
        }

        fn metadata_with_member_packages(
            workspace_root: &AbsolutePath,
            members: &[(&str, &AbsolutePath)],
        ) -> WorkspaceMetadata {
            let packages = members
                .iter()
                .map(|(name, member_root)| {
                    let example_name = format!("{name}_example");
                    let package = PackageRecord {
                        name:          (*name).to_string(),
                        version:       Version::new(0, 1, 0),
                        edition:       "2021".to_string(),
                        description:   None,
                        license:       None,
                        homepage:      None,
                        repository:    None,
                        manifest_path: AbsolutePath::from(member_root.as_path().join("Cargo.toml")),
                        targets:       vec![TargetRecord {
                            name:              example_name.clone(),
                            kinds:             vec![TargetKind::Example],
                            src_path:          AbsolutePath::from(
                                member_root
                                    .as_path()
                                    .join("examples")
                                    .join(format!("{example_name}.rs")),
                            ),
                            required_features: Vec::new(),
                        }],
                        publish:       PublishPolicy::Any,
                    };
                    (
                        PackageId {
                            repr: format!("{name}-{}", member_root.display()),
                        },
                        package,
                    )
                })
                .collect();

            WorkspaceMetadata {
                workspace_root: workspace_root.clone(),
                target_directory: AbsolutePath::from(workspace_root.as_path().join("target")),
                packages,
                fingerprint: ManifestFingerprint {
                    manifest:       FileStamp {
                        content_hash: [0_u8; 32],
                    },
                    lockfile:       None,
                    rust_toolchain: None,
                    configs:        BTreeMap::new(),
                },
                out_of_tree_target_bytes: None,
            }
        }

        fn deliver_metadata(app: &mut App, metadata: WorkspaceMetadata) {
            let workspace_root = metadata.workspace_root.clone();
            let generation = app
                .scan
                .metadata_store_handle()
                .lock()
                .expect("lock metadata store")
                .next_generation(&workspace_root);
            app.handle_bg_msg(BackgroundMsg::CargoMetadata {
                workspace_root,
                generation,
                fingerprint: metadata.fingerprint.clone(),
                result: Ok(metadata),
            });
        }

        #[test]
        fn cargo_metadata_arrival_adds_new_workspace_member_row() {
            let workspace_root = test_path("/__cargo_port_never_real/hana");
            let existing_member = test_path("/__cargo_port_never_real/hana/crates/hana");
            let new_member = test_path("/__cargo_port_never_real/hana/demos/wasm_node_demo");
            let root = make_workspace_with_members(
                Some("hana"),
                "/__cargo_port_never_real/hana",
                vec![inline_group(vec![make_member(
                    Some("hana"),
                    "/__cargo_port_never_real/hana/crates/hana",
                )])],
            );
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));

            deliver_metadata(
                &mut app,
                metadata_with_member_packages(
                    &workspace_root,
                    &[("hana", &existing_member), ("wasm_node_demo", &new_member)],
                ),
            );

            assert!(
                app.project_list
                    .is_workspace_member_path(new_member.as_path()),
                "new metadata member should be part of the workspace tree"
            );
            let info = app
                .project_list
                .rust_info_at_path(new_member.as_path())
                .expect("new member should have Rust info");
            assert_eq!(
                info.cargo.example_count(),
                1,
                "new member should get example targets from the same metadata payload"
            );
            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| line.contains("demos (1)")),
                "new member should render under its grouped folder without manual refresh: {rendered:?}"
            );
        }

        #[test]
        fn cargo_metadata_arrival_adds_new_linked_workspace_member_row() {
            let primary = make_workspace_raw(
                Some("hana"),
                "/__cargo_port_never_real/hana",
                vec![inline_group(vec![make_member(
                    Some("hana"),
                    "/__cargo_port_never_real/hana/crates/hana",
                )])],
                None,
            );
            let linked = make_workspace_raw_with_primary(
                Some("hana"),
                "/__cargo_port_never_real/hana_feature",
                vec![inline_group(vec![make_member(
                    Some("hana"),
                    "/__cargo_port_never_real/hana_feature/crates/hana",
                )])],
                Some("hana_feature"),
                Some("/__cargo_port_never_real/hana"),
            );
            let root = make_workspace_worktrees_item(primary, vec![linked]);
            let linked_root = test_path("/__cargo_port_never_real/hana_feature");
            let existing_member = test_path("/__cargo_port_never_real/hana_feature/crates/hana");
            let new_member =
                test_path("/__cargo_port_never_real/hana_feature/demos/wasm_node_demo");
            let mut app = make_app(&[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.project_list.expanded.insert(ExpandKey::Worktree(0, 1));

            deliver_metadata(
                &mut app,
                metadata_with_member_packages(
                    &linked_root,
                    &[("hana", &existing_member), ("wasm_node_demo", &new_member)],
                ),
            );

            assert!(
                app.project_list
                    .is_workspace_member_path(new_member.as_path()),
                "new metadata member should be part of the linked workspace tree"
            );
            let info = app
                .project_list
                .rust_info_at_path(new_member.as_path())
                .expect("new linked member should have Rust info");
            assert_eq!(
                info.cargo.example_count(),
                1,
                "new linked member should get example targets from the same metadata payload"
            );
            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|line| line.contains("demos (1)")),
                "linked workspace should render the new member group without manual refresh: {rendered:?}"
            );
        }

        #[test]
        fn detail_cache_separates_root_and_worktree_rows_with_same_path() {
            let primary_ws = make_workspace_raw(
                None,
                "~/ws",
                vec![inline_group(vec![make_member(Some("a"), "~/ws/a")])],
                None,
            );
            let linked_ws = make_workspace_raw(
                None,
                "~/ws_feat",
                vec![inline_group(vec![make_member(Some("b"), "~/ws_feat/b")])],
                Some("ws_feat"),
            );
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);

            let mut app = make_app(&[make_workspace_project(None, "~/ws")]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;
            apply_items(&mut app, &[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();

            app.project_list
                .lint_at_path_mut(&test_path("~/ws"))
                .unwrap()
                .set_status(LintStatus::Passed(parse_ts("2026-03-30T14:22:18-05:00")));
            app.project_list
                .lint_at_path_mut(&test_path("~/ws_feat"))
                .unwrap()
                .set_status(LintStatus::Failed(parse_ts("2026-03-30T15:22:18-05:00")));

            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();
            let root_worktrees = app.panes.git.content().map(|g| g.worktrees.clone());
            assert_eq!(root_worktrees.as_ref().map(Vec::len), Some(2));
            assert_eq!(
                root_worktrees
                    .as_ref()
                    .and_then(|wts| wts.get(1))
                    .map(|wt| wt.name.as_str()),
                Some("ws_feat")
            );

            app.project_list.set_cursor(1);
            app.sync_selected_project();
            app.ensure_detail_cached();
            assert_eq!(app.panes.git.content().map(|g| g.worktrees.len()), Some(0));
        }

        #[test]
        fn workspace_worktree_group_root_uses_worktree_group_title() {
            let primary_ws = make_workspace_raw(Some("bevy_brp"), "~/rust/bevy_brp", vec![], None);
            let linked_ws = make_workspace_raw(
                Some("bevy_brp_style_fix"),
                "~/rust/bevy_brp_style_fix",
                vec![],
                Some("bevy_brp_style_fix"),
            );
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);

            let mut app = make_app(&[]);
            apply_items(&mut app, &[root]);
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let package = app.panes.package.content().unwrap();
            assert_eq!(package.title, "Worktree Group");
            assert_eq!(package.name, "bevy_brp");
            assert_eq!(
                panes::DetailField::Targets.package_value(package),
                "workspace"
            );
            assert_eq!(
                package.worktree_group_summary.as_ref().map(|s| s.worktrees),
                Some(2)
            );
            assert_eq!(
                package.worktree_group_summary.as_ref().map(|s| s.deleted),
                Some(0)
            );

            let rows = panes::package_rows_from_data(package);
            assert_eq!(
                &rows[..6],
                &[
                    panes::PackageRow::Description,
                    panes::PackageRow::Section(panes::PackageSection::WorktreeGroupSummary),
                    panes::PackageRow::Field(panes::DetailField::Worktrees),
                    panes::PackageRow::Field(panes::DetailField::Lint),
                    panes::PackageRow::Field(panes::DetailField::Ci),
                    panes::PackageRow::Section(panes::PackageSection::PrimaryWorkspace),
                ]
            );
        }

        #[test]
        fn workspace_worktree_group_root_targets_include_each_checkout() {
            let primary_ws = make_workspace_raw(Some("hana"), "/tmp/hana", vec![], None);
            let linked_ws = make_workspace_raw(
                Some("hana"),
                "/tmp/hana_style_fix",
                vec![],
                Some("hana_style_fix"),
            );
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws]);
            let primary_path = test_path("/tmp/hana");
            let linked_path = test_path("/tmp/hana_style_fix");

            let mut app = make_app(&[]);
            apply_items(&mut app, &[root]);
            {
                let handle = app.scan.metadata_store_handle();
                let mut store = handle.lock().expect("lock test store");
                store.upsert(metadata_with_example(&primary_path, "hana", "showcase"));
                store.upsert(metadata_with_example(&linked_path, "hana", "showcase"));
            }

            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let targets = app.panes.targets.content().unwrap();
            let labels: Vec<String> = targets
                .examples
                .iter()
                .map(|entry| entry.source.label().to_string())
                .collect();
            assert_eq!(labels, vec!["hana/hana", "hana_style_fix/hana"]);

            let project_paths: Vec<String> = targets
                .examples
                .iter()
                .map(|entry| entry.project_path.display().to_string())
                .collect();
            assert_eq!(
                project_paths,
                vec![
                    primary_path.display().to_string(),
                    linked_path.display().to_string(),
                ]
            );
            assert!(
                targets
                    .examples
                    .iter()
                    .all(|entry| entry.package_name == "hana")
            );

            app.panes.targets.viewport.set_pos(1);
            panes::dispatch_targets_action(TargetsAction::ReleaseBuild, &mut app);
            let pending = app
                .inflight
                .take_pending_example_run()
                .expect("example run should be pending");
            assert_eq!(pending.abs_path, linked_path.display().to_string());
            assert_eq!(pending.package_name.as_deref(), Some("hana"));
            assert!(pending.build_mode.is_release());
        }

        #[test]
        fn package_worktree_group_root_reverts_to_package_after_linked_dismissed() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("cargo-mend");
            let linked_dir = tmp.path().join("cargo-mend_style_fix");
            std::fs::create_dir_all(&primary_dir).expect("create test directory");
            std::fs::create_dir_all(&linked_dir).expect("create test directory");

            let primary_path = primary_dir.to_string_lossy().to_string();
            let linked_path = linked_dir.to_string_lossy().to_string();
            let root = make_package_worktrees_item(
                make_package_raw(Some("cargo-mend"), &primary_path, None),
                vec![make_package_raw(
                    Some("cargo-mend"),
                    &linked_path,
                    Some("cargo-mend_style_fix"),
                )],
            );

            let mut app = make_app(&[root]);
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();
            assert_eq!(
                app.panes.package.content().map(|p| p.title.as_str()),
                Some("Worktree Group")
            );
            assert_eq!(app.panes.git.content().map(|g| g.worktrees.len()), Some(2));

            assert!(app.expand(), "root worktree group should expand");
            app.ensure_visible_rows_cached();
            std::fs::remove_dir_all(&linked_dir).expect("remove test directory");
            app.handle_disk_usage(Path::new(&linked_path), 0);
            app.ensure_visible_rows_cached();

            app.project_list.set_cursor(2);
            let target = app
                .focused_dismiss_target()
                .expect("deleted linked worktree should be dismissable");
            app.dismiss(target);
            app.ensure_detail_cached();

            let package = app.panes.package.content().unwrap();
            assert_eq!(package.title, "Package");
            assert_eq!(package.name, "cargo-mend");
            assert!(package.worktree_group_summary.is_none());
            assert_eq!(app.panes.git.content().map(|g| g.worktrees.len()), Some(0));
        }

        #[test]
        fn worktree_group_summary_counts_visible_and_deleted_entries() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("cargo-mend"), "~/rust/cargo-mend", None),
                vec![make_package_raw(
                    Some("cargo-mend"),
                    "~/rust/cargo-mend_style_fix",
                    Some("cargo-mend_style_fix"),
                )],
            );

            let mut app = make_app(&[root]);
            app.project_list
                .at_path_mut(test_path("~/rust/cargo-mend_style_fix").as_path())
                .expect("linked worktree should exist")
                .visibility = Deleted;
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let package = app.panes.package.content().unwrap();
            assert_eq!(package.title, "Worktree Group");
            assert_eq!(
                package.worktree_group_summary.as_ref().map(|s| s.worktrees),
                Some(1)
            );
            assert_eq!(
                package.worktree_group_summary.as_ref().map(|s| s.deleted),
                Some(1)
            );

            let rows = panes::package_rows_from_data(package);
            assert_eq!(
                &rows[..7],
                &[
                    panes::PackageRow::Description,
                    panes::PackageRow::Section(panes::PackageSection::WorktreeGroupSummary),
                    panes::PackageRow::Field(panes::DetailField::Worktrees),
                    panes::PackageRow::Field(panes::DetailField::DeletedWorktrees),
                    panes::PackageRow::Field(panes::DetailField::Lint),
                    panes::PackageRow::Field(panes::DetailField::Ci),
                    panes::PackageRow::Section(panes::PackageSection::PrimaryPackage),
                ]
            );
        }

        #[test]
        fn dismissed_linked_worktree_is_omitted_from_group_git_summary() {
            let root = make_package_worktrees_item(
                make_package_raw(Some("cargo-mend"), "~/rust/cargo-mend", None),
                vec![
                    make_package_raw(
                        Some("cargo-mend"),
                        "~/rust/cargo-mend_style_fix",
                        Some("cargo-mend_style_fix"),
                    ),
                    make_package_raw(
                        Some("cargo-mend"),
                        "~/rust/cargo-mend_old_fix",
                        Some("cargo-mend_old_fix"),
                    ),
                ],
            );

            let mut app = make_app(&[root]);
            let dismissed_path = test_path("~/rust/cargo-mend_old_fix");
            app.project_list
                .at_path_mut(dismissed_path.as_path())
                .expect("dismissed worktree should exist")
                .visibility = Dismissed;
            app.project_list.set_cursor(0);
            app.sync_selected_project();
            app.ensure_detail_cached();

            let git = app.panes.git.content().unwrap();
            let names: Vec<&str> = git.worktrees.iter().map(|wt| wt.name.as_str()).collect();
            assert_eq!(names, vec!["cargo-mend", "cargo-mend_style_fix"]);
            assert_eq!(
                app.panes.package.content().map(|p| p.title.as_str()),
                Some("Worktree Group")
            );
        }

        #[test]
        fn linked_worktree_entry_builds_detail_for_selected_row() {
            let primary_ws = make_workspace_raw(
                Some("cargo-port"),
                "~/rust/cargo-port",
                vec![inline_group(vec![make_member(
                    Some("cargo-port"),
                    "~/rust/cargo-port/crates/cargo-port",
                )])],
                None,
            );
            let linked_ws = make_workspace_raw_with_primary(
                Some("cargo-port_speedup"),
                "~/rust/cargo-port_speedup",
                vec![inline_group(vec![make_member(
                    Some("cargo-port"),
                    "~/rust/cargo-port_speedup/crates/cargo-port",
                )])],
                Some("cargo-port_speedup"),
                Some("~/rust/cargo-port"),
            );
            let root = make_workspace_worktrees_item(primary_ws, vec![linked_ws.clone()]);

            let mut app = make_app(&[]);
            apply_items(&mut app, &[root]);
            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();

            assert_eq!(
                app.visible_rows(),
                vec![
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                ]
            );

            app.project_list.set_cursor(2);
            app.sync_selected_project();
            app.ensure_detail_cached();

            assert_eq!(
                app.project_list
                    .selected_project_path()
                    .map(Path::to_path_buf),
                Some(linked_ws.path().to_path_buf())
            );
            assert_eq!(
                app.panes.package.content().map(|p| p.path.as_str()),
                Some("~/rust/cargo-port_speedup")
            );
            assert!(
                app.tabbable_panes().contains(&PaneId::Package),
                "linked worktree selection should expose the package pane"
            );
        }

        #[test]
        fn disk_rollup_deduplicates_primary_worktree_path() {
            let root = make_package_worktrees_item(
                make_package_raw(None, "~/ws", None),
                vec![make_package_raw(None, "~/ws_feat", Some("ws_feat"))],
            );

            let mut app = make_app(&[make_project(None, "~/ws")]);
            apply_items(&mut app, &[root]);
            app.handle_disk_usage(test_path("~/ws").as_path(), 15);
            app.handle_disk_usage(test_path("~/ws_feat").as_path(), 21);

            assert_eq!(app.project_list[0].disk_usage_bytes(), Some(36));
            assert_eq!(
                panes::formatted_disk_for_item(&app.project_list[0].root_item),
                crate::tui::render::format_bytes(36)
            );
        }

        #[test]
        fn handle_project_discovered_deduplicates_by_path() {
            let mut app = make_app(&[]);

            let pkg1 = RootItem::Rust(RustProject::Package(make_package_raw(
                Some("foo"),
                "/abs/foo",
                None,
            )));
            let pkg2 = RootItem::Rust(RustProject::Package(make_package_raw(
                Some("foo"),
                "/abs/foo",
                None,
            )));
            let pkg3 = RootItem::Rust(RustProject::Package(make_package_raw(
                Some("bar"),
                "/abs/bar",
                None,
            )));

            app.handle_project_discovered(pkg1);
            app.handle_project_discovered(pkg2);
            app.handle_project_discovered(pkg3);
            assert_eq!(app.project_list.len(), 2);
        }

        #[test]
        fn handle_project_discovered_inserts_new_root_in_sorted_position() {
            let mut app = make_app(&[
                make_project(Some("cargo-mend"), "~/rust/cargo-mend"),
                make_project(Some("cargo-port"), "~/rust/cargo-port"),
                make_project(Some("rust-template"), "~/rust/rust-template"),
            ]);

            assert!(app.handle_project_discovered(make_project(
                Some("cache-apt-pkgs-action"),
                "~/rust/cache-apt-pkgs-action",
            )));

            let actual: Vec<_> = app
                .project_list
                .iter()
                .map(|entry| entry.root_item.path())
                .collect();
            assert_eq!(
                actual,
                vec![
                    test_path("~/rust/cache-apt-pkgs-action").as_path(),
                    test_path("~/rust/cargo-mend").as_path(),
                    test_path("~/rust/cargo-port").as_path(),
                    test_path("~/rust/rust-template").as_path(),
                ]
            );
        }

        #[test]
        fn handle_project_discovered_registers_new_root_with_lint_runtime() {
            let project_dir = tempfile::tempdir().expect("create test tempdir");
            std::fs::create_dir_all(project_dir.path().join("src")).expect("create test directory");
            std::fs::write(
                project_dir.path().join("Cargo.toml"),
                manifest_contents("new_worktree", false),
            )
            .expect("write test file");
            std::fs::write(
                project_dir.path().join("src").join("lib.rs"),
                "pub fn demo() {}\n",
            )
            .expect("write test file");

            let cache_dir = tempfile::tempdir().expect("create test tempdir");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
            cargo_port_config.lint.enabled = LintIndicator::Enabled;
            cargo_port_config.lint.include = vec![project_dir.path().to_string_lossy().to_string()];
            cargo_port_config.lint.commands = vec![crate::config::LintCommandConfig {
                name:    "echo".to_string(),
                command: "echo lint ok".to_string(),
            }];
            let mut app = make_app_with_lint_runtime(&[], &cargo_port_config);

            assert!(app.handle_project_discovered(item_from_project_dir(project_dir.path())));
            let trigger = lint::classify_event_path(
                project_dir.path(),
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                &project_dir.path().join("src").join("lib.rs"),
            )
            .expect("lint trigger should classify test edit");
            app.lint
                .runtime()
                .expect("lint runtime fixture should own runtime")
                .lint_trigger(trigger);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut passed = false;
            while std::time::Instant::now() < deadline {
                app.poll_background();
                if matches!(
                    crate::tui::state::Lint::status_for_path(&app.project_list, project_dir.path()),
                    LintStatus::Passed(_)
                ) {
                    passed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            drop(app);
            assert!(
                passed,
                "newly discovered project should have an active lint worker for later edits"
            );
        }

        #[test]
        fn handle_project_discovered_registers_arbitrary_named_worktree_with_primary_lint_filter() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_hana");
            let linked_dir = tmp.path().join("test");
            init_git_project(&primary_dir, "bevy_hana", false);
            add_git_worktree(&primary_dir, &linked_dir, "test/bevy_hana");

            let cache_dir = tempfile::tempdir().expect("create test tempdir");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
            cargo_port_config.lint.enabled = LintIndicator::Enabled;
            cargo_port_config.lint.include = vec!["bevy_hana".to_string()];
            cargo_port_config.lint.on_discovery = DiscoveryLint::Immediate;
            cargo_port_config.lint.commands = vec![crate::config::LintCommandConfig {
                name:    "echo".to_string(),
                command: "echo lint ok".to_string(),
            }];
            let primary_item = item_from_project_dir(&primary_dir);
            let mut app = make_app_with_lint_runtime(&[primary_item], &cargo_port_config);

            assert!(app.handle_project_discovered(item_from_project_dir(&linked_dir)));
            let quiet_deadline = std::time::Instant::now() + std::time::Duration::from_millis(150);
            while std::time::Instant::now() < quiet_deadline {
                app.poll_background();
                assert!(matches!(
                    crate::tui::state::Lint::status_for_path(&app.project_list, &linked_dir),
                    LintStatus::NoLog
                ));
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            let trigger = lint::classify_event_path(
                &linked_dir,
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                &linked_dir.join("src").join("lib.rs"),
            )
            .expect("lint trigger should classify test edit");
            app.lint
                .runtime()
                .expect("lint runtime fixture should own runtime")
                .lint_trigger(trigger);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut passed = false;
            while std::time::Instant::now() < deadline {
                app.poll_background();
                if matches!(
                    crate::tui::state::Lint::status_for_path(&app.project_list, &linked_dir),
                    LintStatus::Passed(_)
                ) {
                    passed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }

            drop(app);
            assert!(
                passed,
                "linked worktree should be eligible through the primary checkout lint filter"
            );
        }

        #[test]
        fn refreshed_linked_worktree_registers_lint_after_stale_discovery() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_hana");
            let linked_dir = tmp.path().join("test");
            init_git_project(&primary_dir, "bevy_hana", false);
            add_git_worktree(&primary_dir, &linked_dir, "test/bevy_hana");

            let cache_dir = tempfile::tempdir().expect("create test tempdir");
            let mut cargo_port_config = CargoPortConfig::default();
            cargo_port_config.cache.root = cache_dir.path().to_string_lossy().to_string();
            cargo_port_config.lint.enabled = LintIndicator::Enabled;
            cargo_port_config.lint.include = vec!["bevy_hana".to_string()];
            cargo_port_config.lint.commands = vec![crate::config::LintCommandConfig {
                name:    "echo".to_string(),
                command: "echo lint ok".to_string(),
            }];
            let primary_item = item_from_project_dir(&primary_dir);
            let mut app = make_app_with_lint_runtime(&[primary_item], &cargo_port_config);

            let linked_path = linked_dir.to_string_lossy().to_string();
            let stale_discovery = RootItem::Rust(RustProject::Package(make_package_raw(
                Some("test"),
                &linked_path,
                None,
            )));
            assert!(app.handle_project_discovered(stale_discovery));
            assert!(app.handle_project_refreshed(item_from_project_dir(&linked_dir)));

            let trigger = lint::classify_event_path(
                &linked_dir,
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                &linked_dir.join("src").join("lib.rs"),
            )
            .expect("lint trigger should classify test edit");

            app.lint
                .runtime()
                .expect("lint runtime fixture should own runtime")
                .lint_trigger(trigger);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut passed = false;
            while std::time::Instant::now() < deadline {
                app.poll_background();
                if matches!(
                    crate::tui::state::Lint::status_for_path(&app.project_list, &linked_dir),
                    LintStatus::Passed(_)
                ) {
                    passed = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }

            drop(app);
            assert!(
                passed,
                "refreshed linked worktree should register a lint worker for later edits"
            );
        }

        #[test]
        fn handle_project_discovered_creates_worktree_group_from_single_primary() {
            for kind in [WorktreeProjectKind::Package, WorktreeProjectKind::Workspace] {
                expect_synthetic_discovery_creates_group(kind);
            }
        }

        #[test]
        fn handle_project_discovered_slots_new_worktree_into_existing_group() {
            for kind in [WorktreeProjectKind::Package, WorktreeProjectKind::Workspace] {
                expect_synthetic_discovery_appends_existing_group(kind);
            }
        }

        #[test]
        fn background_discovery_from_real_worktree_creates_group() {
            for kind in [WorktreeProjectKind::Package, WorktreeProjectKind::Workspace] {
                expect_real_discovery_creates_group(kind);
            }
        }

        #[test]
        fn discovered_workspace_worktree_with_members_expands_as_worktree_then_workspace() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_test");
            init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

            let primary_item = item_from_project_dir(&primary_dir);
            let mut app = make_app(&[primary_item]);

            add_git_worktree(&primary_dir, &linked_dir, "test/brp");
            let linked_item = scan::discover_project_item(&linked_dir)
                .expect("linked worktree should be discoverable");
            assert!(
                app.handle_bg_msg(BackgroundMsg::ProjectDiscovered { item: linked_item }),
                "discovery should request a derived-state rebuild"
            );

            let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                panic!("expected discovered workspace worktree to form a worktree group");
            };
            assert_eq!(group.linked.len(), 1);
            let RustProject::Workspace(linked_ws) = &group.linked[0] else {
                panic!("linked entry should be a workspace");
            };
            assert!(
                linked_ws.has_members(),
                "linked workspace worktree should arrive with member groups populated"
            );

            app.project_list.set_cursor(0);
            assert!(app.expand(), "root should expand into worktree entries");
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                ]
            );

            app.project_list.set_cursor(2);
            assert!(
                app.expand(),
                "linked workspace worktree should expand into its workspace members"
            );
            app.ensure_visible_rows_cached();
            assert!(
                app.visible_rows().iter().any(|row| matches!(
                    row,
                    VisibleRow::WorktreeMember {
                        node_index: 0,
                        worktree_index: 1,
                        ..
                    }
                )),
                "expanded linked workspace worktree should show member rows"
            );
        }

        #[test]
        fn expanded_workspace_root_discovery_immediately_renders_primary_workspace_and_linked_row()
        {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_test");
            init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

            let mut primary_item = item_from_project_dir(&primary_dir);
            let RootItem::Rust(RustProject::Workspace(primary_ws)) = &mut primary_item else {
                panic!("expected primary workspace root item");
            };
            *primary_ws.groups_mut() = vec![inline_group(vec![make_member(
                Some("extras"),
                &primary_dir.join("extras").to_string_lossy(),
            )])];
            let mut app = make_app(&[]);
            apply_items(&mut app, &[primary_item]);

            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    },
                ]
            );

            add_git_worktree(&primary_dir, &linked_dir, "test/brp");
            let linked_item = scan::discover_project_item(&linked_dir)
                .expect("linked worktree should be discoverable");
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectDiscovered { item: linked_item },
            );

            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeMember {
                        node_index:     0,
                        worktree_index: 0,
                        group_index:    0,
                        member_index:   0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                ],
                "discovering a linked workspace worktree while the primary root is expanded should preserve the primary workspace subtree immediately"
            );

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered
                    .iter()
                    .any(|row| row.contains("bevy_brp") && row.contains(":2")),
                "root row should still render the worktree badge after discovery: {rendered:?}"
            );
            assert!(
                rendered.iter().any(|row| row.contains("bevy_brp_test")),
                "linked worktree row should render immediately without a collapse/expand cycle: {rendered:?}"
            );
            assert!(
                rendered.iter().any(|row| row.contains("extras")),
                "primary workspace member rows should remain visible after the root becomes a worktree group: {rendered:?}"
            );
        }

        #[test]
        fn stale_workspace_regroup_immediately_renders_primary_workspace_and_linked_row() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("bevy_brp");
            let linked_dir = tmp.path().join("bevy_brp_test");
            init_workspace_git_project_with_member(&primary_dir, "bevy_brp", "extras");

            let mut primary_item = item_from_project_dir(&primary_dir);
            let RootItem::Rust(RustProject::Workspace(primary_ws)) = &mut primary_item else {
                panic!("expected primary workspace root item");
            };
            *primary_ws.groups_mut() = vec![inline_group(vec![make_member(
                Some("extras"),
                &primary_dir.join("extras").to_string_lossy(),
            )])];
            let mut app = make_app(&[]);
            apply_items(&mut app, &[primary_item]);

            app.project_list.expanded.insert(ExpandKey::Node(0));
            app.ensure_visible_rows_cached();
            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::Member {
                        node_index:   0,
                        group_index:  0,
                        member_index: 0,
                    },
                ]
            );

            add_git_worktree(&primary_dir, &linked_dir, "test/brp");
            let stale_discovery = RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                Some("bevy_brp"),
                &linked_dir.to_string_lossy(),
                Vec::new(),
                None,
            )));
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectDiscovered {
                    item: stale_discovery,
                },
            );

            let refreshed = item_from_project_dir(&linked_dir);
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectRefreshed { item: refreshed },
            );

            assert_eq!(
                app.visible_rows(),
                &[
                    VisibleRow::Root { node_index: 0 },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 0,
                    },
                    VisibleRow::WorktreeMember {
                        node_index:     0,
                        worktree_index: 0,
                        group_index:    0,
                        member_index:   0,
                    },
                    VisibleRow::WorktreeEntry {
                        node_index:     0,
                        worktree_index: 1,
                    },
                ],
                "refresh regroup should preserve the expanded primary workspace subtree immediately"
            );

            let rendered = rendered_root_name_cells(&mut app);
            assert!(
                rendered.iter().any(|row| row.contains("bevy_brp_test")),
                "regrouped linked worktree row should render immediately without a collapse/expand cycle: {rendered:?}"
            );
            assert!(
                rendered.iter().any(|row| row.contains("extras")),
                "regrouped primary workspace member rows should remain visible: {rendered:?}"
            );
        }

        #[test]
        fn background_discovery_from_real_worktree_appends_existing_group() {
            for kind in [WorktreeProjectKind::Package, WorktreeProjectKind::Workspace] {
                expect_real_discovery_appends_existing_group(kind);
            }
        }

        #[test]
        fn refreshed_worktree_metadata_regroups_stale_top_level_discovery() {
            for kind in [WorktreeProjectKind::Workspace, WorktreeProjectKind::Package] {
                expect_refresh_regroups_stale_top_level_discovery(kind);
            }
        }

        #[test]
        fn refreshed_worktree_metadata_appends_into_existing_group() {
            for kind in [WorktreeProjectKind::Workspace, WorktreeProjectKind::Package] {
                expect_refresh_appends_stale_discovery_into_existing_group(kind);
            }
        }

        #[test]
        fn refreshed_linked_worktree_preserves_lint_status() {
            let primary_path = "~/ws";
            let linked_path = "~/ws_feat";
            let root = make_package_worktrees_item(
                make_package_raw_with_primary(Some("ws"), primary_path, None, Some(primary_path)),
                vec![make_package_raw_with_primary(
                    Some("ws"),
                    linked_path,
                    Some("ws_feat"),
                    Some(primary_path),
                )],
            );
            let linked_abs = test_path(linked_path);
            let mut app = make_app(&[root]);
            app.config.current_mut().lint.enabled = LintIndicator::Enabled;

            app.handle_bg_msg(BackgroundMsg::LintStatus {
                path:   linked_abs,
                status: LintStatus::Running(parse_ts("2026-03-30T16:22:18-05:00")),
                origin: LintRunOrigin::Normal,
            });

            let refreshed = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                Some("ws"),
                linked_path,
                Some("ws_feat"),
                Some(primary_path),
            )));
            assert!(app.handle_bg_msg(BackgroundMsg::ProjectRefreshed { item: refreshed }));

            let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                panic!("expected worktree group");
            };
            assert!(matches!(
                group.lint_status_for_worktree(1),
                LintStatus::Running(_)
            ));
        }

        #[test]
        fn stale_discovery_refresh_then_delete_dismisses_to_root() {
            for kind in [WorktreeProjectKind::Workspace, WorktreeProjectKind::Package] {
                assert_stale_discovery_refresh_then_delete_dismisses_to_root(kind);
            }
        }

        fn assert_stale_discovery_refresh_then_delete_dismisses_to_root(kind: WorktreeProjectKind) {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join(kind.primary_name());
            let linked_dir = tmp.path().join(kind.linked_name());
            kind.init_primary_repo(&primary_dir);

            let primary_item = item_from_project_dir(&primary_dir);
            let mut app = make_app(&[primary_item]);

            add_git_worktree(
                &primary_dir,
                &linked_dir,
                &format!("test/{}", kind.branch_prefix()),
            );

            let stale_discovery = match kind {
                WorktreeProjectKind::Package => {
                    RootItem::Rust(RustProject::Package(make_package_raw(
                        Some(kind.primary_name()),
                        &linked_dir.to_string_lossy(),
                        None,
                    )))
                },
                WorktreeProjectKind::Workspace => {
                    RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                        Some(kind.primary_name()),
                        &linked_dir.to_string_lossy(),
                        Vec::new(),
                        None,
                    )))
                },
            };
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectDiscovered {
                    item: stale_discovery,
                },
            );
            let refreshed = item_from_project_dir(&linked_dir);
            apply_bg_msg(
                &mut app,
                BackgroundMsg::ProjectRefreshed { item: refreshed },
            );
            assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
        }

        #[test]
        fn background_disk_zero_from_real_package_worktree_can_be_dismissed_to_root() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("app");
            let linked_dir = tmp.path().join("app_test");
            init_git_project(&primary_dir, "app", false);
            add_git_worktree(&primary_dir, &linked_dir, "test/app");

            let primary_item = item_from_project_dir(&primary_dir);
            let linked_item = item_from_project_dir(&linked_dir);
            let mut app = make_app(&[primary_item, linked_item]);

            assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
        }

        #[test]
        fn background_disk_zero_from_real_workspace_worktree_can_be_dismissed_to_root() {
            let tmp = tempfile::tempdir().expect("create test tempdir");
            let primary_dir = tmp.path().join("obsidian_knife");
            let linked_dir = tmp.path().join("obsidian_knife_test");
            init_git_project(&primary_dir, "obsidian_knife", true);
            add_git_worktree(&primary_dir, &linked_dir, "test/obsidian");

            let primary_item = item_from_project_dir(&primary_dir);
            let linked_item = item_from_project_dir(&linked_dir);
            let mut app = make_app(&[primary_item, linked_item]);

            assert_deleted_linked_worktree_dismisses_to_root(&mut app, &linked_dir);
        }

        #[test]
        fn handle_project_discovered_does_not_allocate_per_comparison() {
            const DISCOVERY_PROJECTS: usize = 200;
            const MAX_DISCOVERY_ELAPSED: Duration = std::time::Duration::from_millis(500);

            let mut app = make_app(&[]);
            let start = std::time::Instant::now();
            for i in 0..DISCOVERY_PROJECTS {
                let path = format!("/abs/project_{i}");
                let item =
                    RootItem::Rust(RustProject::Package(make_package_raw(None, &path, None)));
                app.handle_project_discovered(item);
            }
            let elapsed = start.elapsed();
            assert_eq!(app.project_list.len(), DISCOVERY_PROJECTS);
            assert!(
                elapsed < MAX_DISCOVERY_ELAPSED,
                "discovery of {DISCOVERY_PROJECTS} projects took {elapsed:?}; expected less than {MAX_DISCOVERY_ELAPSED:?}"
            );
        }

        #[test]
        fn is_deleted_does_not_allocate_display_paths() {
            let mut app = make_app(&[]);
            for i in 0..200 {
                let path = format!("/abs/project_{i}");
                let item =
                    RootItem::Rust(RustProject::Package(make_package_raw(None, &path, None)));
                app.project_list.push(item);
            }
            let target = app.project_list[100].path().to_path_buf();
            app.project_list
                .at_path_mut(&target)
                .expect("target project should exist")
                .visibility = Deleted;
            let start = std::time::Instant::now();
            for _ in 0..1000 {
                let _ = app.project_list.is_deleted(&target);
            }
            let elapsed = start.elapsed();
            assert!(
                elapsed.as_millis() < 100,
                "1000 is_deleted calls took {elapsed:?} -- possible display_path allocation regression"
            );
        }
    }

    fn test_path(path: &str) -> AbsolutePath {
        let pb = if path == "~" {
            dirs::home_dir().unwrap_or_else(|| PathBuf::from(path))
        } else if let Some(rest) = path.strip_prefix("~/") {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join(rest)
        } else {
            PathBuf::from(path)
        };
        AbsolutePath::from(pb)
    }

    fn status_for(worktree_marker: Option<&str>, primary_abs_path: Option<&str>) -> WorktreeStatus {
        match (worktree_marker, primary_abs_path) {
            (None, None) => WorktreeStatus::NotGit,
            (Some(_), Some(p)) => WorktreeStatus::Linked {
                primary: test_path(p),
            },
            (None, Some(p)) => WorktreeStatus::Primary { root: test_path(p) },
            (Some(_), None) => WorktreeStatus::Linked {
                primary: test_path("~/unknown-primary"),
            },
        }
    }

    fn make_project(name: Option<&str>, path: &str) -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: test_path(path),
            name: name.map(String::from),
            ..Package::default()
        }))
    }

    fn make_app(projects: &[RootItem]) -> App { tui_test_support::make_app(projects) }

    fn make_app_with_config(projects: &[RootItem], cargo_port_config: &CargoPortConfig) -> App {
        tui_test_support::make_app_with_config(projects, cargo_port_config)
    }

    fn make_app_with_lint_runtime(
        projects: &[RootItem],
        cargo_port_config: &CargoPortConfig,
    ) -> TestApp {
        tui_test_support::make_app_with_lint_runtime(projects, cargo_port_config)
    }

    fn set_loaded_ci(
        app: &mut App,
        path: &Path,
        runs: Vec<CiRun>,
        exhausted: bool,
        github_total: u32,
    ) {
        let entry = app
            .project_list
            .entry_containing_mut(path)
            .expect("test project should exist in project list");
        let repo = entry.git_repo.get_or_insert_with(Default::default);
        repo.ci_data = ProjectCiData::Loaded(ProjectCiInfo {
            runs,
            github_total,
            ci_pagination: CiPagination::from(exhausted),
        });
    }

    fn loaded_ci<'a>(app: &'a App, path: &Path) -> &'a ProjectCiInfo {
        match &app
            .project_list
            .entry_containing(path)
            .and_then(|entry| entry.git_repo.as_ref())
            .expect("test project should have Git repo data")
            .ci_data
        {
            ProjectCiData::Loaded(info) => info,
            ProjectCiData::Unfetched => unreachable!("test project should have loaded CI data"),
        }
    }

    struct TestRenderCtxHolder {
        ci_status_lookup: CiStatusLookup,
    }

    fn build_project_list_render_ctx_for_test<'a>(
        app: &'a App,
        holder: &'a TestRenderCtxHolder,
    ) -> PaneRenderCtx<'a> {
        PaneRenderCtx {
            animation_elapsed:         app.animation_started.elapsed(),
            config:                    &app.config,
            project_list:              &app.project_list,
            selected_project_path:     app.selected_project_path_for_render(),
            inflight:                  &app.inflight,
            scan:                      &app.scan,
            ci_status_lookup:          &holder.ci_status_lookup,
            settings_render_inputs:    None,
            synced_description_height: crate::tui::panes::SyncedDescriptionHeight::default(),
            running_targets:           app.panes.running_targets.snapshot(),
        }
    }

    #[allow(
        dead_code,
        reason = "called by tests in rendered_root_name_cells / render_tree_buffer to seed the \
                  ProjectList pane's focus snapshot before invoking render_tree_items"
    )]
    fn sync_project_list_focus_for_test(app: &mut App) {
        let pane_focus_state = app.pane_focus_state(PaneId::ProjectList);
        app.panes.project_list.focus = RenderFocus { pane_focus_state };
    }

    fn rendered_root_name_cells(app: &mut App) -> Vec<String> {
        app.ensure_visible_rows_cached();
        let labels = app
            .project_list
            .resolved_root_labels(app.config.include_non_rust().includes_non_rust());
        let widths = crate::tui::panes::compute_project_list_widths(
            &app.project_list,
            &labels,
            app.config.lint_enabled(),
            0,
        );
        let items = {
            let viewport = app.panes.project_list.viewport.clone();
            let holder = TestRenderCtxHolder {
                ci_status_lookup: app.ci.status_lookup(),
            };
            let project_list_render_context = build_project_list_render_ctx_for_test(app, &holder);
            crate::tui::panes::render_tree_items(
                &project_list_render_context,
                &app.panes.project_list,
                &viewport,
                &widths,
            )
        };
        let area = Rect::new(
            0,
            0,
            u16::try_from(widths.total_width()).unwrap_or(u16::MAX),
            u16::try_from(items.len()).unwrap_or(u16::MAX),
        );
        let mut buffer = Buffer::empty(area);
        List::new(items).render(area, &mut buffer);

        (0..area.height)
            .map(|y| {
                let mut row = String::new();
                for x in 0..area.width {
                    row.push_str(buffer[(x, y)].symbol());
                }
                row.trim_end().to_string()
            })
            .collect()
    }

    fn render_tree_buffer(app: &mut App) -> (Buffer, ProjectListWidths) {
        app.ensure_visible_rows_cached();
        let labels = app
            .project_list
            .resolved_root_labels(app.config.include_non_rust().includes_non_rust());
        let widths = crate::tui::panes::compute_project_list_widths(
            &app.project_list,
            &labels,
            app.config.lint_enabled(),
            0,
        );
        let items = {
            let viewport = app.panes.project_list.viewport.clone();
            let holder = TestRenderCtxHolder {
                ci_status_lookup: app.ci.status_lookup(),
            };
            let project_list_render_context = build_project_list_render_ctx_for_test(app, &holder);
            crate::tui::panes::render_tree_items(
                &project_list_render_context,
                &app.panes.project_list,
                &viewport,
                &widths,
            )
        };
        let area = Rect::new(
            0,
            0,
            u16::try_from(widths.total_width()).unwrap_or(u16::MAX),
            u16::try_from(items.len()).unwrap_or(u16::MAX),
        );
        let mut buffer = Buffer::empty(area);
        List::new(items).render(area, &mut buffer);
        (buffer, widths)
    }

    fn row_has_crossed_out_content(
        buffer: &Buffer,
        widths: &ProjectListWidths,
        row: usize,
    ) -> bool {
        (0..widths.total_width()).any(|x| {
            let cell = &buffer[(
                u16::try_from(x).unwrap_or(u16::MAX),
                u16::try_from(row).unwrap_or(u16::MAX),
            )];
            !cell.symbol().trim().is_empty()
                && cell.style().add_modifier.contains(Modifier::CROSSED_OUT)
        })
    }

    fn resolved_root_label(item: &RootItem) -> String {
        ProjectList::new(vec![item.clone()]).resolved_root_labels(true)[0].clone()
    }

    /// Wrap owned `RootItem`s in a `ProjectList` for test helpers that pass
    /// them to finder/widths functions.
    pub(super) fn as_entries(items: Vec<RootItem>) -> ProjectList {
        crate::tui::project_list::ProjectList::new(items)
    }

    fn make_non_rust_project(name: Option<&str>, path: &str) -> RootItem {
        RootItem::NonRust(NonRustProject::new(test_path(path), name.map(String::from)))
    }

    fn make_workspace_project(name: Option<&str>, path: &str) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: test_path(path),
            name: name.map(String::from),
            ..Workspace::default()
        }))
    }

    fn make_workspace_with_members(
        name: Option<&str>,
        path: &str,
        groups: Vec<MemberGroup>,
    ) -> RootItem {
        RootItem::Rust(RustProject::Workspace(Workspace {
            path: test_path(path),
            name: name.map(String::from),
            groups,
            ..Workspace::default()
        }))
    }

    fn make_member(name: Option<&str>, path: &str) -> Package {
        Package {
            path: test_path(path),
            name: name.map(String::from),
            ..Package::default()
        }
    }

    fn make_workspace_worktrees_item(primary: Workspace, linked: Vec<Workspace>) -> RootItem {
        RootItem::Worktrees(WorktreeGroup::new(
            RustProject::Workspace(primary),
            linked.into_iter().map(RustProject::Workspace).collect(),
        ))
    }

    fn make_package_worktrees_item(primary: Package, linked: Vec<Package>) -> RootItem {
        RootItem::Worktrees(WorktreeGroup::new(
            RustProject::Package(primary),
            linked.into_iter().map(RustProject::Package).collect(),
        ))
    }

    fn make_package_raw(name: Option<&str>, path: &str, worktree_marker: Option<&str>) -> Package {
        make_package_raw_with_primary(name, path, worktree_marker, None)
    }

    fn make_package_raw_with_primary(
        name: Option<&str>,
        path: &str,
        worktree_marker: Option<&str>,
        primary_abs_path: Option<&str>,
    ) -> Package {
        Package {
            path: test_path(path),
            name: name.map(String::from),
            worktree_status: status_for(worktree_marker, primary_abs_path),
            ..Package::default()
        }
    }

    fn make_workspace_raw(
        name: Option<&str>,
        path: &str,
        groups: Vec<MemberGroup>,
        worktree_marker: Option<&str>,
    ) -> Workspace {
        make_workspace_raw_with_primary(name, path, groups, worktree_marker, None)
    }

    fn make_workspace_raw_with_primary(
        name: Option<&str>,
        path: &str,
        groups: Vec<MemberGroup>,
        worktree_marker: Option<&str>,
        primary_abs_path: Option<&str>,
    ) -> Workspace {
        Workspace {
            path: test_path(path),
            name: name.map(String::from),
            worktree_status: status_for(worktree_marker, primary_abs_path),
            groups,
            ..Workspace::default()
        }
    }

    fn inline_group(members: Vec<Package>) -> MemberGroup { MemberGroup::Inline { members } }

    fn named_group(name: &str, members: Vec<Package>) -> MemberGroup {
        MemberGroup::Named {
            name: name.to_string(),
            members,
        }
    }

    fn make_package_with_vendored(
        name: Option<&str>,
        path: &str,
        vendored: Vec<VendoredPackage>,
    ) -> Package {
        Package {
            path: test_path(path),
            name: name.map(String::from),
            rust: RustInfo {
                vendored,
                ..RustInfo::default()
            },
            ..Package::default()
        }
    }

    fn make_vendored(name: Option<&str>, path: &str) -> VendoredPackage {
        VendoredPackage {
            path: test_path(path),
            name: name.map(String::from),
            ..VendoredPackage::default()
        }
    }

    fn wait_for_tree_build(app: &mut App) {
        // Tree rebuilds no longer exist - just ensure derived state is fresh.
        app.ensure_visible_rows_cached();
    }

    fn git_binary() -> &'static str {
        if Path::new("/usr/bin/git").is_file() {
            "/usr/bin/git"
        } else {
            "git"
        }
    }

    fn manifest_contents(name: &str, workspace: bool) -> String {
        let workspace_section = if workspace { "\n[workspace]\n" } else { "" };
        format!(
            r#"[package]
    name = "{name}"
    version = "0.1.0"
    edition = "2024"
    {workspace_section}
    "#
        )
    }

    fn init_git_project(dir: &Path, name: &str, workspace: bool) {
        std::fs::create_dir_all(dir.join("src")).expect("create test directory");
        std::fs::write(dir.join("Cargo.toml"), manifest_contents(name, workspace))
            .expect("write test file");
        std::fs::write(dir.join("src").join("main.rs"), "fn main() {}\n").expect("write test file");

        Command::new(git_binary())
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["config", "user.name", "cargo-port-tests"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["config", "user.email", "cargo-port-tests@example.com"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
    }

    fn init_workspace_git_project_with_member(dir: &Path, name: &str, member_name: &str) {
        let member_dir = dir.join(member_name);
        std::fs::create_dir_all(member_dir.join("src")).expect("create test directory");
        std::fs::write(
            dir.join("Cargo.toml"),
            format!(
                "[workspace]\nmembers = [\"{member_name}\"]\n\n[workspace.package]\nrepository = \"https://example.com/{name}\"\n"
            ),
        ).expect("write test file");
        std::fs::write(
            member_dir.join("Cargo.toml"),
            format!(
                "[package]\nname = \"{member_name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"
            ),
        )
        .expect("write test file");
        std::fs::write(member_dir.join("src").join("lib.rs"), "pub fn demo() {}\n")
            .expect("write test file");

        Command::new(git_binary())
            .args(["init"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["config", "user.name", "cargo-port-tests"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["config", "user.email", "cargo-port-tests@example.com"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["add", "."])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
        Command::new(git_binary())
            .args(["commit", "-m", "init"])
            .current_dir(dir)
            .output()
            .expect("run git command in test project");
    }

    fn add_git_worktree(primary_dir: &Path, worktree_dir: &Path, branch: &str) {
        let status = Command::new(git_binary())
            .args([
                "worktree",
                "add",
                worktree_dir
                    .to_str()
                    .expect("test path should be valid UTF-8"),
                "-b",
                branch,
            ])
            .current_dir(primary_dir)
            .status()
            .expect("run git command in test project");
        assert!(status.success(), "git worktree add should succeed");
    }

    fn item_from_project_dir(dir: &Path) -> RootItem {
        let cargo_toml = dir.join("Cargo.toml");
        let parsed = project::from_cargo_toml(&cargo_toml)
            .unwrap_or_else(|_| panic!("parse test Cargo.toml"));
        scan::cargo_project_to_item(parsed)
    }

    fn apply_bg_msg(app: &mut App, msg: BackgroundMsg) {
        if app.handle_bg_msg(msg) {
            app.scan.bump_generation();
        }
        app.ensure_visible_rows_cached();
    }

    fn apply_items(app: &mut App, items: &[RootItem]) {
        app.apply_tree_build(ProjectList::new(items.to_vec()));
        app.ensure_visible_rows_cached();
    }

    fn parse_ts(ts: &str) -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339(ts).expect("parse test timestamp")
    }

    fn make_ci_run(run_id: u64, conclusion: CiStatus) -> CiRun {
        CiRun {
            run_id,
            created_at: "2026-03-30T14:22:18Z".to_string(),
            branch: "main".to_string(),
            url: format!("https://github.com/natepiano/demo/actions/runs/{run_id}"),
            ci_status: conclusion,
            jobs: Vec::new(),
            wall_clock_secs: Some(1),
            commit_title: Some(format!("run {run_id}")),
            updated_at: None,
            fetched: FetchStatus::Fetched,
        }
    }

    fn make_git_info(url: Option<&str>) -> (CheckoutInfo, RepoInfo) {
        let checkout = CheckoutInfo {
            status:              GitStatus::Clean,
            head:                HeadState::Branch("main".to_string()),
            last_commit:         None,
            ahead_behind_local:  None,
            primary_tracked_ref: Some("origin/main".to_string()),
            bisect:              None,
        };
        let repo = RepoInfo {
            remotes:           vec![RemoteInfo {
                name:         "origin".to_string(),
                url:          url.map(String::from),
                owner:        Some("natepiano".to_string()),
                repo:         None,
                tracked_ref:  Some("origin/main".to_string()),
                ahead_behind: None,
                kind:         RemoteKind::Clone,
                push:         crate::project::PushState::Enabled {
                    push_url: String::new(),
                },
            }],
            workflows:         WorkflowPresence::Present,
            first_commit:      None,
            last_fetched:      None,
            default_branch:    Some("main".to_string()),
            local_main_branch: Some("main".to_string()),
        };
        (checkout, repo)
    }

    /// Apply a `(CheckoutInfo, RepoInfo)` bundle through the same
    /// `BackgroundMsg` dispatch the runtime uses: `RepoInfo` first, then
    /// `CheckoutInfo`. Routing through `apply_bg_msg` keeps the helper in sync
    /// with generation bumps and any other dispatch-wide invariants.
    fn apply_git_info(app: &mut App, path: &Path, (checkout, repo): (CheckoutInfo, RepoInfo)) {
        let abs = AbsolutePath::from(path);
        apply_bg_msg(
            app,
            BackgroundMsg::RepoInfo {
                path: abs.clone(),
                info: repo,
            },
        );
        apply_bg_msg(
            app,
            BackgroundMsg::CheckoutInfo {
                path: abs,
                info: checkout,
            },
        );
    }

    #[derive(Clone, Copy)]
    enum WorktreeProjectKind {
        Package,
        Workspace,
    }

    impl WorktreeProjectKind {
        fn primary_name(self) -> &'static str {
            match self {
                Self::Package => "app",
                Self::Workspace => "obsidian_knife",
            }
        }

        fn linked_name(self) -> &'static str {
            match self {
                Self::Package => "app_test",
                Self::Workspace => "obsidian_knife_test",
            }
        }

        fn feature_name(self) -> &'static str {
            match self {
                Self::Package => "app_feat",
                Self::Workspace => "obsidian_knife_feat",
            }
        }

        fn branch_prefix(self) -> &'static str {
            match self {
                Self::Package => "app",
                Self::Workspace => "obsidian",
            }
        }

        fn init_primary_repo(self, dir: &Path) {
            init_git_project(dir, self.primary_name(), matches!(self, Self::Workspace));
        }

        fn root_item(dir: &Path) -> RootItem { item_from_project_dir(dir) }

        fn assert_group_layout(self, app: &App, linked_len: usize, context: &str) {
            assert_eq!(app.project_list.len(), 1, "{context}");
            let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                panic!("expected worktree group: {context}");
            };
            match (self, &group.primary) {
                (Self::Package, RustProject::Package(_))
                | (Self::Workspace, RustProject::Workspace(_)) => {
                    assert_eq!(group.linked.len(), linked_len, "{context}");
                },
                (Self::Package, _) => panic!("expected package worktree group: {context}"),
                (Self::Workspace, _) => panic!("expected workspace worktree group: {context}"),
            }
        }
    }

    fn expect_real_discovery_creates_group(kind: WorktreeProjectKind) {
        let tmp = tempfile::tempdir().expect("create test tempdir");
        let primary_dir = tmp.path().join(kind.primary_name());
        let linked_dir = tmp.path().join(kind.linked_name());
        kind.init_primary_repo(&primary_dir);

        let primary_item = WorktreeProjectKind::root_item(&primary_dir);
        let mut app = make_app(&[primary_item]);

        add_git_worktree(
            &primary_dir,
            &linked_dir,
            &format!("test/{}", kind.branch_prefix()),
        );
        let linked_item = WorktreeProjectKind::root_item(&linked_dir);
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectDiscovered { item: linked_item },
        );

        kind.assert_group_layout(
            &app,
            1,
            "real worktree discovery should create a worktree group",
        );

        app.project_list.set_cursor(0);
        assert!(app.expand(), "root should expand into worktree entries");
        app.ensure_visible_rows_cached();
        assert_eq!(app.visible_rows().len(), 3);
    }

    fn expect_real_discovery_appends_existing_group(kind: WorktreeProjectKind) {
        let tmp = tempfile::tempdir().expect("create test tempdir");
        let primary_dir = tmp.path().join(kind.primary_name());
        let linked_one_dir = tmp.path().join(kind.feature_name());
        let linked_two_dir = tmp.path().join(kind.linked_name());
        kind.init_primary_repo(&primary_dir);
        add_git_worktree(
            &primary_dir,
            &linked_one_dir,
            &format!("feat/{}", kind.branch_prefix()),
        );

        let primary_item = WorktreeProjectKind::root_item(&primary_dir);
        let linked_one_item = WorktreeProjectKind::root_item(&linked_one_dir);
        let mut app = make_app(&[primary_item, linked_one_item]);

        add_git_worktree(
            &primary_dir,
            &linked_two_dir,
            &format!("test/{}", kind.branch_prefix()),
        );
        let linked_two_item = WorktreeProjectKind::root_item(&linked_two_dir);
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectDiscovered {
                item: linked_two_item,
            },
        );

        kind.assert_group_layout(
            &app,
            2,
            "second real worktree discovery should append inside the existing group",
        );
    }

    fn expect_synthetic_discovery_creates_group(kind: WorktreeProjectKind) {
        match kind {
            WorktreeProjectKind::Package => {
                let primary_path = "/abs/app";
                let linked_path = "/abs/app_feat";
                let primary = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                    Some("app"),
                    primary_path,
                    None,
                    Some("/canonical/app"),
                )));
                let linked = RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                    Some("app"),
                    linked_path,
                    Some("app_feat"),
                    Some("/canonical/app"),
                )));

                let mut app = make_app(&[primary]);
                assert!(app.handle_project_discovered(linked));
                assert_eq!(app.project_list.len(), 1);

                let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                    panic!("expected discovered worktree to create a package worktree group");
                };
                assert!(matches!(&group.primary, RustProject::Package(_)));
                assert_eq!(
                    group.primary.path(),
                    crate::project::normalize_test_path(Path::new(primary_path)).as_path()
                );
                assert_eq!(group.linked.len(), 1);
                assert_eq!(
                    group.linked[0].path(),
                    crate::project::normalize_test_path(Path::new(linked_path)).as_path()
                );
            },
            WorktreeProjectKind::Workspace => {
                let primary_path = "/abs/obsidian_knife";
                let linked_path = "/abs/obsidian_knife_test";
                let primary =
                    RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                        Some("obsidian_knife"),
                        primary_path,
                        Vec::new(),
                        None,
                        Some("/canonical/obsidian_knife"),
                    )));
                let linked =
                    RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                        Some("obsidian_knife"),
                        linked_path,
                        Vec::new(),
                        Some("obsidian_knife_test"),
                        Some("/canonical/obsidian_knife"),
                    )));

                let mut app = make_app(&[primary]);
                assert!(app.handle_project_discovered(linked));
                assert_eq!(app.project_list.len(), 1);

                let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                    panic!("expected discovered workspace worktree to create a worktree group");
                };
                assert!(matches!(&group.primary, RustProject::Workspace(_)));
                assert_eq!(
                    group.primary.path(),
                    crate::project::normalize_test_path(Path::new(primary_path)).as_path()
                );
                assert_eq!(group.linked.len(), 1);
                assert_eq!(
                    group.linked[0].path(),
                    crate::project::normalize_test_path(Path::new(linked_path)).as_path()
                );
            },
        }
    }

    fn expect_synthetic_discovery_appends_existing_group(kind: WorktreeProjectKind) {
        match kind {
            WorktreeProjectKind::Package => {
                let primary_path = "/abs/app";
                let existing_linked_path = "/abs/app_feat";
                let new_linked_path = "/abs/app_fix";
                let root = make_package_worktrees_item(
                    make_package_raw_with_primary(
                        Some("app"),
                        primary_path,
                        None,
                        Some("/canonical/app"),
                    ),
                    vec![make_package_raw_with_primary(
                        Some("app"),
                        existing_linked_path,
                        Some("app_feat"),
                        Some("/canonical/app"),
                    )],
                );
                let new_linked =
                    RootItem::Rust(RustProject::Package(make_package_raw_with_primary(
                        Some("app"),
                        new_linked_path,
                        Some("app_fix"),
                        Some("/canonical/app"),
                    )));

                let mut app = make_app(&[root]);
                assert!(app.handle_project_discovered(new_linked));
                assert_eq!(app.project_list.len(), 1);

                let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                    panic!("expected existing root to remain a package worktree group");
                };
                assert!(matches!(&group.primary, RustProject::Package(_)));
                assert_eq!(group.linked.len(), 2);
                assert!(group.linked.iter().any(|l| {
                    l.path()
                        == crate::project::normalize_test_path(Path::new(existing_linked_path))
                            .as_path()
                }));
                assert!(group.linked.iter().any(|l| l.path()
                    == crate::project::normalize_test_path(Path::new(new_linked_path)).as_path()));
            },
            WorktreeProjectKind::Workspace => {
                let primary_path = "/abs/obsidian_knife";
                let existing_linked_path = "/abs/obsidian_knife_feat";
                let new_linked_path = "/abs/obsidian_knife_test";
                let root = make_workspace_worktrees_item(
                    make_workspace_raw_with_primary(
                        Some("obsidian_knife"),
                        primary_path,
                        Vec::new(),
                        None,
                        Some("/canonical/obsidian_knife"),
                    ),
                    vec![make_workspace_raw_with_primary(
                        Some("obsidian_knife"),
                        existing_linked_path,
                        Vec::new(),
                        Some("obsidian_knife_feat"),
                        Some("/canonical/obsidian_knife"),
                    )],
                );
                let new_linked =
                    RootItem::Rust(RustProject::Workspace(make_workspace_raw_with_primary(
                        Some("obsidian_knife"),
                        new_linked_path,
                        Vec::new(),
                        Some("obsidian_knife_test"),
                        Some("/canonical/obsidian_knife"),
                    )));

                let mut app = make_app(&[root]);
                assert!(app.handle_project_discovered(new_linked));
                assert_eq!(app.project_list.len(), 1);

                let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
                    panic!("expected existing root to remain a workspace worktree group");
                };
                assert!(matches!(&group.primary, RustProject::Workspace(_)));
                assert_eq!(group.linked.len(), 2);
                assert!(group.linked.iter().any(|l| {
                    l.path()
                        == crate::project::normalize_test_path(Path::new(existing_linked_path))
                            .as_path()
                }));
                assert!(group.linked.iter().any(|l| l.path()
                    == crate::project::normalize_test_path(Path::new(new_linked_path)).as_path()));
            },
        }
    }

    fn expect_refresh_regroups_stale_top_level_discovery(kind: WorktreeProjectKind) {
        let tmp = tempfile::tempdir().expect("create test tempdir");
        let primary_dir = tmp.path().join(kind.primary_name());
        let linked_dir = tmp.path().join(kind.linked_name());
        kind.init_primary_repo(&primary_dir);

        let primary_item = WorktreeProjectKind::root_item(&primary_dir);
        let mut app = make_app(&[primary_item]);
        add_git_worktree(
            &primary_dir,
            &linked_dir,
            &format!("test/{}", kind.branch_prefix()),
        );

        let stale_discovery = match kind {
            WorktreeProjectKind::Package => RootItem::Rust(RustProject::Package(make_package_raw(
                Some(kind.primary_name()),
                &linked_dir.to_string_lossy(),
                None,
            ))),
            WorktreeProjectKind::Workspace => {
                RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                    Some(kind.primary_name()),
                    &linked_dir.to_string_lossy(),
                    Vec::new(),
                    None,
                )))
            },
        };
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectDiscovered {
                item: stale_discovery,
            },
        );
        assert_eq!(app.project_list.len(), 2);

        let refreshed = WorktreeProjectKind::root_item(&linked_dir);
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectRefreshed { item: refreshed },
        );

        kind.assert_group_layout(
            &app,
            1,
            "refreshing the stale top-level row should regroup it under the primary worktree container",
        );
        let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
            unreachable!("refresh should regroup stale top-level discovery");
        };
        let _ = kind;
        assert_eq!(group.linked[0].path(), linked_dir.as_path());
    }

    fn expect_refresh_appends_stale_discovery_into_existing_group(kind: WorktreeProjectKind) {
        let tmp = tempfile::tempdir().expect("create test tempdir");
        let primary_dir = tmp.path().join(kind.primary_name());
        let linked_one_dir = tmp.path().join(kind.feature_name());
        let linked_two_dir = tmp.path().join(kind.linked_name());
        kind.init_primary_repo(&primary_dir);
        add_git_worktree(
            &primary_dir,
            &linked_one_dir,
            &format!("feat/{}", kind.branch_prefix()),
        );

        let primary_item = WorktreeProjectKind::root_item(&primary_dir);
        let linked_one_item = WorktreeProjectKind::root_item(&linked_one_dir);
        let mut app = make_app(&[primary_item, linked_one_item]);

        add_git_worktree(
            &primary_dir,
            &linked_two_dir,
            &format!("test/{}", kind.branch_prefix()),
        );
        let stale_discovery = match kind {
            WorktreeProjectKind::Package => RootItem::Rust(RustProject::Package(make_package_raw(
                Some(kind.primary_name()),
                &linked_two_dir.to_string_lossy(),
                None,
            ))),
            WorktreeProjectKind::Workspace => {
                RootItem::Rust(RustProject::Workspace(make_workspace_raw(
                    Some(kind.primary_name()),
                    &linked_two_dir.to_string_lossy(),
                    Vec::new(),
                    None,
                )))
            },
        };
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectDiscovered {
                item: stale_discovery,
            },
        );
        assert_eq!(app.project_list.len(), 2);

        let refreshed = WorktreeProjectKind::root_item(&linked_two_dir);
        apply_bg_msg(
            &mut app,
            BackgroundMsg::ProjectRefreshed { item: refreshed },
        );

        kind.assert_group_layout(
            &app,
            2,
            "refresh should fold the stale row into the existing worktree group",
        );
        let RootItem::Worktrees(group) = &app.project_list[0].root_item else {
            unreachable!("refresh should append stale discovery to worktree group");
        };
        let _ = kind;
        assert!(
            group
                .linked
                .iter()
                .any(|l| l.path() == linked_one_dir.as_path())
        );
        assert!(
            group
                .linked
                .iter()
                .any(|l| l.path() == linked_two_dir.as_path())
        );
    }

    fn assert_deleted_linked_worktree_dismisses_to_root(app: &mut App, linked_dir: &Path) {
        app.project_list.set_cursor(0);
        assert!(
            app.expand(),
            "root should expand into worktree entries after regroup"
        );
        app.ensure_visible_rows_cached();
        assert_eq!(app.visible_rows().len(), 3);

        std::fs::remove_dir_all(linked_dir).expect("remove test directory");
        apply_bg_msg(
            app,
            BackgroundMsg::DiskUsage {
                path:  linked_dir.to_path_buf().into(),
                bytes: 0,
            },
        );
        assert!(app.project_list.is_deleted(linked_dir));
        app.project_list.set_cursor(2);
        let target = app
            .focused_dismiss_target()
            .expect("deleted linked worktree should be dismissable");
        app.dismiss(target);
        app.ensure_visible_rows_cached();
        assert_eq!(app.visible_rows(), &[VisibleRow::Root { node_index: 0 }]);
    }
}
