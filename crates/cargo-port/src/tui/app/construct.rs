//! `App` construction pipeline as a typestate builder.
//!
//! Three phases, enforced at the type level:
//!
//! 1. [`AppBuilder<Inputs>`] — caller's raw arguments only. No I/O run yet.
//! 2. [`AppBuilder<Channeled>`] — internal channel pairs created.
//! 3. [`AppBuilder<Started>`] — startup policy applied: real services have been started and
//!    disabled services have installed explicit no-op handles.
//!
//! Each transition consumes the previous state and produces the next, so the
//! steps can't be skipped or reordered. `build()` is callable only on
//! `AppBuilder<Started>`. The thin shim `App::new` in `mod.rs` runs the chain
//! end-to-end and is the only `pub(super)` entry point — siblings in `tui/*`
//! reach construction through that one method.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Context;
use anyhow::Error;
use tui_pane::FocusedPane;
use tui_pane::Framework;
use tui_pane::Keymap as FrameworkKeymap;
use tui_pane::LoadedSettings;
use tui_pane::PERF_LOG_TARGET;
use tui_pane::SettingsStore;
use tui_pane::ThemeRuntime;
use tui_pane::ToastSettings;

use super::App;
use super::ConfirmationModalState;
use super::async_tasks::Startup;
use super::scan_state::ScanState;
use crate::channel;
use crate::channel::Receiver;
use crate::channel::Sender;
use crate::config;
use crate::config::CargoPortConfig;
use crate::config::CargoPortConfigurationPathResolution;
use crate::lint::RuntimeHandle;
use crate::process_observation::ProcessRefreshExecutionBackendSelection;
use crate::process_observation::ProcessRefreshExecutor;
use crate::process_observation::RunningTargetsRefreshSchedule;
use crate::project::CargoWorkspaceIndex;
use crate::project::RootItem;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::scan::ExcludeDirs;
use crate::tui::background::Background;
use crate::tui::background::BackgroundChannels;
use crate::tui::integration;
use crate::tui::integration::AppPaneId;
use crate::tui::keymap;
use crate::tui::overlays::Overlays;
use crate::tui::panes::Panes;
use crate::tui::project_list::ProjectList;
use crate::tui::running_targets::RUNNING_TARGETS_REFRESH_INTERVAL;
use crate::tui::sccache::SccacheStatusLine;
use crate::tui::settings::StartupSettings;
use crate::tui::startup_services::StartupEffect;
use crate::tui::startup_services::StartupEnvironment;
use crate::tui::startup_services::StartupServices;
use crate::tui::startup_services::WatcherHandle;
use crate::tui::startup_services::WatcherStartup;
use crate::tui::state::Ci;
use crate::tui::state::Config;
use crate::tui::state::GitStatusTracker;
use crate::tui::state::Inflight;
use crate::tui::state::Keymap;
use crate::tui::state::Lint;
use crate::tui::state::Net;
use crate::tui::state::Scan;
use crate::tui::state::SyncTracker;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::OwnedRunEvent;
use crate::tui::theme_roles;

/// Caller's raw arguments. Held by value (the slice and config
/// reference are cloned at the entry point so the builder can outlive
/// its caller's stack frame).
pub(super) struct Inputs {
    background_tx:       Sender<BackgroundMsg>,
    background_rx:       Receiver<BackgroundMsg>,
    cargo_port_config:   CargoPortConfig,
    raw_projects:        Vec<RootItem>,
    settings_store:      SettingsStore,
    toast_settings:      ToastSettings,
    startup_environment: StartupEnvironment,
}

/// `Inputs` plus the three internal channel pairs (example, CI
/// fetch, clean) routed through `Background`.
pub(super) struct Channeled {
    inputs:      Inputs,
    example_tx:  Sender<OwnedRunEvent>,
    example_rx:  Receiver<OwnedRunEvent>,
    ci_fetch_tx: Sender<CiFetchMsg>,
    ci_fetch_rx: Receiver<CiFetchMsg>,
    clean_tx:    Sender<CleanMsg>,
    clean_rx:    Receiver<CleanMsg>,
}

/// `Channeled` plus the products of applying the selected startup policy.
pub(super) struct Started {
    channeled:              Channeled,
    config_path_resolution: CargoPortConfigurationPathResolution,
    keymap_path_resolution: CargoPortConfigurationPathResolution,
    lint_warning:           Option<String>,
    lint_runtime:           Option<RuntimeHandle>,
    watcher:                WatcherHandle,
    themes_dir_resolution:  CargoPortConfigurationPathResolution,
    projects:               ProjectList,
}

/// Typestate builder. The state parameter is the stage struct itself
/// (not a marker), so each stage carries its own data.
pub(super) struct AppBuilder<S> {
    state: S,
}

impl AppBuilder<Inputs> {
    pub(super) fn new(
        projects: &[RootItem],
        background_tx: Sender<BackgroundMsg>,
        background_rx: Receiver<BackgroundMsg>,
        startup_settings: StartupSettings,
        startup_environment: StartupEnvironment,
    ) -> Self {
        Self {
            state: Inputs {
                background_tx,
                background_rx,
                cargo_port_config: startup_settings.cargo_port_config,
                raw_projects: projects.to_vec(),
                settings_store: startup_settings.store,
                toast_settings: startup_settings.toast_settings,
                startup_environment,
            },
        }
    }

    pub(super) fn open_channels(self) -> AppBuilder<Channeled> {
        let (example_tx, example_rx) = channel::unbounded();
        let (ci_fetch_tx, ci_fetch_rx) = channel::unbounded();
        let (clean_tx, clean_rx) = channel::unbounded();
        AppBuilder {
            state: Channeled {
                inputs: self.state,
                example_tx,
                example_rx,
                ci_fetch_tx,
                ci_fetch_rx,
                clean_tx,
                clean_rx,
            },
        }
    }
}

impl AppBuilder<Channeled> {
    pub(super) fn run_startup(self) -> AppBuilder<Started> {
        let inputs = &self.state.inputs;
        let startup_services = &inputs.startup_environment.startup_services;
        startup_services.install_active_config(&inputs.cargo_port_config);
        let themes_dir_resolution = startup_services.themes_dir();
        let themes_dir = themes_dir_resolution.clone().into_external_path();
        let mut registry = tui_pane::ThemeRegistry::from_dir_with_builtins(
            themes_dir.as_deref(),
            crate::themes::builtins(),
        );
        theme_roles::apply_role_defaults_to_registry(&mut registry);
        // Resolve the initial theme from the loaded `[appearance]`
        // section against the just-built registry. Misses fall back to
        // the app's own appearance-matched variant silently here —
        // toast machinery is not yet wired this early in startup, and
        // surface for the miss arrives through the settings UI badge.
        let resolved = registry.resolve_active(
            &inputs.cargo_port_config.appearance.mode,
            &inputs.cargo_port_config.appearance.light_theme,
            &inputs.cargo_port_config.appearance.dark_theme,
            None,
        );
        let mut initial_theme = (*resolved.theme).clone();
        theme_roles::apply_role_defaults_to_theme(&mut initial_theme, None, resolved.appearance);
        startup_services.install_theme_state(
            registry,
            initial_theme,
            inputs
                .cargo_port_config
                .appearance
                .focused_pane_tint
                .is_enabled(),
        );
        let config_path_resolution = config::config_path();
        let keymap_path_resolution = keymap::keymap_path();
        let lint_spawn = startup_services
            .spawn_lint_runtime(&inputs.cargo_port_config, inputs.background_tx.clone());
        let watch_roots = scan::resolve_include_dirs(&inputs.cargo_port_config.tui.include_dirs);
        let watcher = startup_services.spawn_watcher(WatcherStartup {
            watch_roots:    &watch_roots,
            background_tx:  inputs.background_tx.clone(),
            ci_run_count:   inputs.cargo_port_config.tui.ci_run_count,
            non_rust:       inputs.cargo_port_config.tui.include_non_rust,
            exclude_dirs:   ExcludeDirs::from(inputs.cargo_port_config.tui.exclude_dirs.as_slice()),
            client:         inputs.startup_environment.http_client.clone(),
            lint_runtime:   lint_spawn.handle.clone(),
            metadata_store: Arc::clone(&inputs.startup_environment.metadata_store),
        });
        let built = scan::build_tree(
            &inputs.raw_projects,
            &inputs.cargo_port_config.tui.inline_dirs,
        );
        let projects = ProjectList::new(built);
        AppBuilder {
            state: Started {
                channeled: self.state,
                config_path_resolution,
                keymap_path_resolution,
                lint_warning: lint_spawn.warning,
                lint_runtime: lint_spawn.handle,
                watcher,
                themes_dir_resolution,
                projects,
            },
        }
    }
}

impl AppBuilder<Started> {
    pub(super) fn build(self) -> Result<App, Error> {
        let started = self.state;
        let channeled = started.channeled;
        let inputs = channeled.inputs;
        let StartupEnvironment {
            http_client,
            scan_started_at,
            metadata_store,
            startup_services,
        } = inputs.startup_environment;
        let panes = Panes::new(&inputs.cargo_port_config.cpu, startup_services.clone());
        let mut projects = started.projects;
        projects.init_runtime_state(inputs.cargo_port_config.lint.enabled.is_enabled());
        let background = Background::new(BackgroundChannels {
            background: (inputs.background_tx, inputs.background_rx),
            ci_fetch:   (channeled.ci_fetch_tx, channeled.ci_fetch_rx),
            clean:      (channeled.clean_tx, channeled.clean_rx),
            example:    (channeled.example_tx, channeled.example_rx),
            watcher:    started.watcher,
        });
        let lint = Lint::new(started.lint_runtime);
        let inflight = Inflight::new();
        let config = Config::new(started.config_path_resolution, inputs.cargo_port_config);
        let keymap_path = started.keymap_path_resolution.clone().into_external_path();
        let keymap = Keymap::new(
            started.keymap_path_resolution,
            keymap::ResolvedKeymap::defaults(),
        );
        let themes = ThemeRuntime::new(started.themes_dir_resolution.into_external_path());
        let cargo_workspace_index = Arc::new(metadata_store.lock().map_or_else(
            |_| CargoWorkspaceIndex::default(),
            |metadata_store| {
                CargoWorkspaceIndex::from_metadata_store(&metadata_store, projects.revision())
            },
        ));
        let scan = Scan::new(ScanState::new(scan_started_at), metadata_store);
        let process_refresh_executor = process_refresh_executor(&startup_services);
        let mut overlays = Overlays::new();
        if let Some(warning) = started.lint_warning {
            overlays.set_status_flash(warning, Instant::now());
        }
        let mut framework = app_framework(inputs.settings_store, inputs.toast_settings);
        let framework_builder = FrameworkKeymap::<App>::builder().vim_mode(
            integration::vim_mode_from_config(config.current().tui.navigation_keys),
        );
        let framework_builder = if let Some(path) = keymap_path {
            let display_path = path.display().to_string();
            keymap::migrate_removed_action_keys_on_disk(&path)
                .with_context(|| format!("migrating removed keymap actions in {display_path}"))?;
            framework_builder
                .load_toml(path)
                .with_context(|| format!("loading keymap from {display_path}"))?
        } else {
            framework_builder
        };
        let framework_keymap =
            integration::build_framework_keymap(framework_builder, &mut framework)
                .with_context(|| "building framework keymap")?;
        let mut app = App {
            net: Net::new(http_client),
            panes,
            project_list: projects,
            cargo_workspace_index,
            process_refresh_executor,
            #[cfg(test)]
            running_target_attribution_collection_count: 0,
            background,
            inflight,
            lint,
            ci: Ci::new(),
            config,
            keymap,
            themes,
            sync_tracker: SyncTracker::default(),
            git_status_tracker: GitStatusTracker::default(),
            scan,
            startup: Startup::new(),
            startup_services,
            visited_panes: std::iter::once(AppPaneId::ProjectList).collect(),
            overlays,
            confirmation_modal_state: ConfirmationModalState::Closed,
            sccache_status_line: SccacheStatusLine::new(),
            animation_started: Instant::now(),
            storage_refresh_at: Instant::now(),
            crates_io_refresh_at: Instant::now(),
            crates_io_checked_at: HashMap::new(),
            mouse_pos: None,
            framework,
            framework_keymap: Rc::new(framework_keymap),
            pending_nav_chord: Vec::new(),
            #[cfg(test)]
            fixture_dirs: None,
        };
        app.finish_new();
        Ok(app)
    }
}

fn process_refresh_executor(startup_services: &StartupServices) -> ProcessRefreshExecutor {
    let running_targets_polling_effect = startup_services.running_targets_polling_effect();
    let (backend_selection, running_targets_refresh_schedule) = match running_targets_polling_effect
    {
        StartupEffect::Real => (
            ProcessRefreshExecutionBackendSelection::DedicatedWorker,
            RunningTargetsRefreshSchedule::Every(RUNNING_TARGETS_REFRESH_INTERVAL),
        ),
        StartupEffect::Suppressed => (
            ProcessRefreshExecutionBackendSelection::Synchronous,
            RunningTargetsRefreshSchedule::Suppressed,
        ),
    };
    Background::start_process_refresh_executor(
        backend_selection,
        running_targets_refresh_schedule,
        Instant::now(),
    )
}

fn app_framework(settings_store: SettingsStore, toast_settings: ToastSettings) -> Framework<App> {
    tui_pane::Framework::new_with_settings(
        FocusedPane::App(AppPaneId::ProjectList),
        LoadedSettings {
            store: settings_store,
            toast_settings,
        },
    )
}

impl App {
    fn finish_new(&mut self) {
        self.panes.install_cpu_placeholder();
        self.load_initial_keymap();
        if let Some(warning) = self
            .overlays
            .status_flash()
            .map(|(warning, _)| warning.clone())
        {
            self.show_timed_toast("Lint runtime", warning);
        }
        self.force_settings_if_unconfigured();
        self.restore_persisted_lint_pause();
        self.prune_inactive_project_state();
        self.register_existing_projects();
        let lint_registered = self.register_lint_for_root_items();
        tracing::trace!(
            target: PERF_LOG_TARGET,
            count = lint_registered,
            "startup_lint_runtime_registered_initial_projects"
        );
        if !self.project_list.is_empty() {
            self.finish_watcher_registration_batch();
        }
        self.refresh_lint_runs_from_disk();
        self.net.set_force_github_rate_limit(
            self.config
                .current()
                .debug
                .force_github_rate_limit
                .is_forced(),
        );
        self.net.spawn_rate_limit_prime(&self.startup_services);
        self.warn_if_github_unauthenticated();
    }
}
