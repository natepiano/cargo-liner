//! `App` construction pipeline as a typestate builder.
//!
//! Three phases, enforced at the type level:
//!
//! 1. [`AppBuilder<Inputs>`] — caller's raw arguments only. No I/O run yet.
//! 2. [`AppBuilder<Channeled>`] — internal mpsc channel pairs created.
//! 3. [`AppBuilder<Started>`] — startup I/O complete: lint runtime spawned, watcher thread spawned,
//!    project tree built, config loaded.
//!
//! Each transition consumes the previous state and produces the next, so the
//! steps can't be skipped or reordered. `build()` is callable only on
//! `AppBuilder<Started>`. The thin shim `App::new` in `mod.rs` runs the chain
//! end-to-end and is the only `pub(super)` entry point — siblings in `tui/*`
//! reach construction through that one method.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::time::Instant;

use super::App;
use super::types::ScanState;
use crate::config;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::keymap;
use crate::lint;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::background::Background;
use crate::tui::background::BackgroundChannels;
use crate::tui::config_state::Config;
use crate::tui::inflight::Inflight;
use crate::tui::keymap_state::Keymap;
use crate::tui::panes::PaneId;
use crate::tui::panes::Panes;
use crate::tui::scan_state::Scan;
use crate::tui::selection::Selection;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts::ToastManager;
use crate::watcher;
use crate::watcher::WatcherMsg;

/// Phase 1: caller's raw arguments. Held by value (the slice and config
/// reference are cloned at the entry point so the builder can outlive its
/// caller's stack frame).
pub(super) struct Inputs {
    bg_tx:           Sender<BackgroundMsg>,
    bg_rx:           Receiver<BackgroundMsg>,
    cfg:             CargoPortConfig,
    http_client:     HttpClient,
    scan_started_at: Instant,
    metadata_store:  Arc<Mutex<WorkspaceMetadataStore>>,
    raw_projects:    Vec<RootItem>,
}

/// Phase 2: phase 1 plus the three internal mpsc channel pairs (example,
/// CI fetch, clean) routed through `Background`.
pub(super) struct Channeled {
    inputs:      Inputs,
    example_tx:  Sender<ExampleMsg>,
    example_rx:  mpsc::Receiver<ExampleMsg>,
    ci_fetch_tx: Sender<CiFetchMsg>,
    ci_fetch_rx: mpsc::Receiver<CiFetchMsg>,
    clean_tx:    Sender<CleanMsg>,
    clean_rx:    mpsc::Receiver<CleanMsg>,
}

/// Phase 3: phase 2 plus the startup I/O products. Replaces the prior
/// `AppInit` scaffolding struct.
pub(super) struct Started {
    channeled:    Channeled,
    config_path:  Option<AbsolutePath>,
    lint_warning: Option<String>,
    lint_runtime: Option<RuntimeHandle>,
    watch_tx:     Sender<WatcherMsg>,
    projects:     ProjectList,
}

/// Builder progressing through the three phases. The phase parameter is the
/// state struct itself (not a marker), so each phase carries its own data.
pub(super) struct AppBuilder<S> {
    state: S,
}

impl AppBuilder<Inputs> {
    pub(super) fn new(
        projects: &[RootItem],
        bg_tx: Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        Self {
            state: Inputs {
                bg_tx,
                bg_rx,
                cfg: cfg.clone(),
                http_client,
                scan_started_at,
                metadata_store,
                raw_projects: projects.to_vec(),
            },
        }
    }

    pub(super) fn open_channels(self) -> AppBuilder<Channeled> {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let (clean_tx, clean_rx) = mpsc::channel();
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
        config::set_active_config(&inputs.cfg);
        let config_path = config::config_path();
        let lint_spawn = lint::spawn(&inputs.cfg, inputs.bg_tx.clone());
        let watch_roots = scan::resolve_include_dirs(&inputs.cfg.tui.include_dirs);
        let watch_tx = watcher::spawn_watcher(
            &watch_roots,
            inputs.bg_tx.clone(),
            inputs.cfg.tui.ci_run_count,
            inputs.cfg.tui.include_non_rust,
            inputs.http_client.clone(),
            lint_spawn.handle.clone(),
            Arc::clone(&inputs.metadata_store),
        );
        let built = scan::build_tree(&inputs.raw_projects, &inputs.cfg.tui.inline_dirs);
        let projects = ProjectList::new(built);
        AppBuilder {
            state: Started {
                channeled: self.state,
                config_path,
                lint_warning: lint_spawn.warning,
                lint_runtime: lint_spawn.handle,
                watch_tx,
                projects,
            },
        }
    }
}

impl AppBuilder<Started> {
    pub(super) fn build(self) -> App {
        let started = self.state;
        let channeled = started.channeled;
        let inputs = channeled.inputs;
        let panes = Panes::new(&inputs.cfg.cpu);
        let selection = Selection::new(inputs.cfg.lint.enabled);
        let background = Background::new(BackgroundChannels {
            bg:       (inputs.bg_tx, inputs.bg_rx),
            ci_fetch: (channeled.ci_fetch_tx, channeled.ci_fetch_rx),
            clean:    (channeled.clean_tx, channeled.clean_rx),
            example:  (channeled.example_tx, channeled.example_rx),
            watch_tx: started.watch_tx,
        });
        let lint = crate::tui::lint_state::Lint::new(started.lint_runtime);
        let inflight = Inflight::new();
        let config_path_buf = started
            .config_path
            .as_ref()
            .map(|p| p.as_path().to_path_buf());
        let config = Config::new(config_path_buf, inputs.cfg);
        let keymap_path_buf = keymap::keymap_path()
            .as_ref()
            .map(|p| p.as_path().to_path_buf());
        let keymap = Keymap::new(keymap_path_buf, keymap::ResolvedKeymap::defaults());
        let scan = Scan::new(
            ScanState::new(inputs.scan_started_at),
            inputs.metadata_store,
        );
        let mut overlays = crate::tui::overlays::Overlays::new();
        if let Some(warning) = started.lint_warning {
            overlays.set_status_flash(warning, Instant::now());
        }
        let mut app = App {
            net: crate::tui::net_state::Net::new(inputs.http_client),
            panes,
            selection,
            projects: started.projects,
            background,
            inflight,
            lint,
            ci: crate::tui::ci_state::Ci::new(),
            config,
            keymap,
            scan,
            startup: crate::tui::app::async_tasks::Startup::new(),
            focus: crate::tui::focus::Focus::new(PaneId::ProjectList),
            overlays,
            confirm: None,
            animation_started: Instant::now(),
            mouse_pos: None,
            toasts: ToastManager::default(),
            layout_cache: crate::tui::panes::LayoutCache::default(),
        };
        app.finish_new();
        app
    }
}

impl App {
    pub(super) fn has_cached_non_rust_projects(&self) -> bool {
        let mut found = false;
        self.projects().for_each_leaf(|item| {
            if !item.is_rust() {
                found = true;
            }
        });
        found
    }

    fn finish_new(&mut self) {
        self.panes_mut().install_cpu_placeholder();
        self.load_initial_keymap();
        if let Some(warning) = self
            .overlays
            .status_flash()
            .map(|(warning, _)| warning.clone())
        {
            self.show_timed_toast("Lint runtime", warning);
        }
        self.force_settings_if_unconfigured();
        self.prune_inactive_project_state();
        self.register_existing_projects();
        if !self.projects().is_empty() {
            self.finish_watcher_registration_batch();
        }
        self.refresh_lint_runs_from_disk();
        self.net
            .set_force_github_rate_limit(self.config.current().debug.force_github_rate_limit);
        self.spawn_rate_limit_prime();
    }
}
