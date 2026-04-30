use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use super::App;
use super::service_state::CratesIoState;
use super::service_state::GitHubState;
use super::types::ScanState;
use super::types::UiModes;
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

pub(super) struct AppChannels {
    example_tx:  mpsc::Sender<ExampleMsg>,
    example_rx:  mpsc::Receiver<ExampleMsg>,
    ci_fetch_tx: mpsc::Sender<CiFetchMsg>,
    ci_fetch_rx: mpsc::Receiver<CiFetchMsg>,
    clean_tx:    mpsc::Sender<CleanMsg>,
    clean_rx:    mpsc::Receiver<CleanMsg>,
}

impl AppChannels {
    fn new() -> Self {
        let (example_tx, example_rx) = mpsc::channel();
        let (ci_fetch_tx, ci_fetch_rx) = mpsc::channel();
        let (clean_tx, clean_rx) = mpsc::channel();
        Self {
            example_tx,
            example_rx,
            ci_fetch_tx,
            ci_fetch_rx,
            clean_tx,
            clean_rx,
        }
    }
}

struct AppInit {
    config_path:  Option<AbsolutePath>,
    lint_warning: Option<String>,
    lint_runtime: Option<RuntimeHandle>,
    watch_tx:     mpsc::Sender<WatcherMsg>,
    projects:     ProjectList,
}

impl AppInit {
    fn new(
        projects: &[RootItem],
        bg_tx: &mpsc::Sender<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: &HttpClient,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        config::set_active_config(cfg);
        let config_path = config::config_path();
        let lint_spawn = lint::spawn(cfg, bg_tx.clone());
        let watch_roots = scan::resolve_include_dirs(&cfg.tui.include_dirs);
        let watch_tx = watcher::spawn_watcher(
            &watch_roots,
            bg_tx.clone(),
            cfg.tui.ci_run_count,
            cfg.tui.include_non_rust,
            http_client.clone(),
            lint_spawn.handle.clone(),
            metadata_store,
        );
        let built = scan::build_tree(projects, &cfg.tui.inline_dirs);
        let projects = crate::project_list::ProjectList::new(built);

        Self {
            config_path,
            lint_warning: lint_spawn.warning,
            lint_runtime: lint_spawn.handle,
            watch_tx,
            projects,
        }
    }
}

struct CoreInputs {
    http_client:     HttpClient,
    bg_tx:           mpsc::Sender<BackgroundMsg>,
    bg_rx:           Receiver<BackgroundMsg>,
    cfg:             CargoPortConfig,
    scan_started_at: Instant,
    channels:        AppChannels,
    init:            AppInit,
    status_flash:    Option<(String, Instant)>,
    metadata_store:  Arc<Mutex<WorkspaceMetadataStore>>,
}

impl App {
    pub fn has_cached_non_rust_projects(&self) -> bool {
        let mut found = false;
        self.projects().for_each_leaf(|item| {
            if !item.is_rust() {
                found = true;
            }
        });
        found
    }

    pub fn new(
        projects: &[RootItem],
        bg_tx: mpsc::Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        let channels = AppChannels::new();
        let init = AppInit::new(
            projects,
            &bg_tx,
            cfg,
            &http_client,
            Arc::clone(&metadata_store),
        );
        let status_flash = init.lint_warning.clone().map(|w| (w, Instant::now()));
        let mut app = Self::build_core(CoreInputs {
            http_client,
            bg_tx,
            bg_rx,
            cfg: cfg.clone(),
            scan_started_at,
            channels,
            init,
            status_flash,
            metadata_store,
        });
        app.finish_new();
        app
    }

    fn build_core(inputs: CoreInputs) -> Self {
        let init = inputs.init;
        let channels = inputs.channels;
        let panes = Panes::new(&inputs.cfg.cpu);
        let selection = Selection::new(inputs.cfg.lint.enabled);
        let background = Background::new(BackgroundChannels {
            bg:       (inputs.bg_tx, inputs.bg_rx),
            ci_fetch: (channels.ci_fetch_tx, channels.ci_fetch_rx),
            clean:    (channels.clean_tx, channels.clean_rx),
            example:  (channels.example_tx, channels.example_rx),
            watch_tx: init.watch_tx,
        });
        let inflight = Inflight::new(init.lint_runtime);
        let config_path_buf = init.config_path.as_ref().map(|p| p.as_path().to_path_buf());
        let config = Config::new(config_path_buf, inputs.cfg);
        let keymap_path_buf = keymap::keymap_path()
            .as_ref()
            .map(|p| p.as_path().to_path_buf());
        let keymap = Keymap::new(keymap_path_buf, keymap::ResolvedKeymap::defaults());
        let scan = Scan::new(
            init.projects,
            ScanState::new(inputs.scan_started_at),
            inputs.metadata_store,
        );
        Self {
            http_client: inputs.http_client,
            github: GitHubState::new(),
            crates_io: CratesIoState::new(),
            panes,
            selection,
            background,
            inflight,
            config,
            keymap,
            scan,
            focused_pane: PaneId::ProjectList,
            return_focus: None,
            confirm: None,
            animation_started: Instant::now(),
            mouse_pos: None,
            status_flash: inputs.status_flash,
            toasts: ToastManager::default(),
            inline_error: None,
            ui_modes: UiModes::default(),
            layout_cache: crate::tui::panes::LayoutCache::default(),
        }
    }

    fn finish_new(&mut self) {
        self.panes_mut().install_cpu_placeholder();
        self.load_initial_keymap();
        if let Some(warning) = self
            .status_flash
            .as_ref()
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
        self.http_client
            .set_force_github_rate_limit(self.config.current().debug.force_github_rate_limit);
        self.spawn_rate_limit_prime();
    }
}
