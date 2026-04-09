use std::collections::HashMap;
use std::collections::HashSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use ratatui::widgets::ListState;

use super::types::App;
use super::types::AsyncBuildState;
use super::types::BuildChannels;
use super::types::ConfigFileStamp;
use super::types::DirtyState;
use super::types::FinderState;
#[cfg(test)]
use super::types::RetrySpawnMode;
use super::types::ScanState;
use super::types::SelectionPaths;
use super::types::SelectionSync;
use super::types::UiModes;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::keymap;
use crate::lint;
use crate::lint::RuntimeHandle;
use crate::project::RootItem;
use crate::project_list::ProjectList;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::tui::columns::ResolvedWidths;
use crate::tui::terminal::CiFetchMsg;
use crate::tui::terminal::CleanMsg;
use crate::tui::terminal::ExampleMsg;
use crate::tui::toasts::ToastManager;
use crate::tui::types::LayoutCache;
use crate::tui::types::Pane;
use crate::tui::types::PaneId;
use crate::watcher;
use crate::watcher::WatcherMsg;

fn initial_list_state(items: &[RootItem]) -> ListState {
    let mut state = ListState::default();
    if !items.is_empty() {
        state.select(Some(0));
    }
    state
}

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
    config_path:      Option<PathBuf>,
    config_last_seen: Option<ConfigFileStamp>,
    lint_warning:     Option<String>,
    lint_runtime:     Option<RuntimeHandle>,
    watch_tx:         mpsc::Sender<WatcherMsg>,
    projects:         ProjectList,
    list_state:       ListState,
}

impl AppInit {
    fn new(
        scan_root: &Path,
        projects: &[RootItem],
        bg_tx: &mpsc::Sender<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: &HttpClient,
    ) -> Self {
        crate::config::set_active_config(cfg);
        let config_path = crate::config::config_path();
        let config_last_seen = config_path.as_deref().and_then(App::config_file_stamp);
        let lint_spawn = lint::spawn(cfg, bg_tx.clone());
        let watch_tx = watcher::spawn_watcher(
            scan_root.to_path_buf(),
            bg_tx.clone(),
            cfg.tui.ci_run_count,
            cfg.tui.include_non_rust,
            cfg.tui.include_dirs.clone(),
            http_client.clone(),
        );
        let built = scan::build_tree(projects, &cfg.tui.inline_dirs);
        let list_state = initial_list_state(&built);
        let projects = crate::project_list::ProjectList::new(built);

        Self {
            config_path,
            config_last_seen,
            lint_warning: lint_spawn.warning,
            lint_runtime: lint_spawn.handle,
            watch_tx,
            projects,
            list_state,
        }
    }
}

struct CoreInputs {
    scan_root:       PathBuf,
    http_client:     HttpClient,
    bg_tx:           mpsc::Sender<BackgroundMsg>,
    bg_rx:           Receiver<BackgroundMsg>,
    cfg:             CargoPortConfig,
    scan_started_at: Instant,
    builds:          AsyncBuildState,
    channels:        AppChannels,
    init:            AppInit,
    status_flash:    Option<(String, Instant)>,
}

impl App {
    pub fn has_cached_non_rust_projects(&self) -> bool {
        let mut found = false;
        self.projects.for_each_leaf(|item| {
            if !item.is_rust() {
                found = true;
            }
        });
        found
    }

    pub fn new(
        scan_root: PathBuf,
        projects: &[RootItem],
        bg_tx: mpsc::Sender<BackgroundMsg>,
        bg_rx: Receiver<BackgroundMsg>,
        cfg: &CargoPortConfig,
        http_client: HttpClient,
        scan_started_at: Instant,
    ) -> Self {
        let channels = AppChannels::new();
        let builds = AsyncBuildState::new(BuildChannels::new());
        let init = AppInit::new(&scan_root, projects, &bg_tx, cfg, &http_client);
        let status_flash = init.lint_warning.clone().map(|w| (w, Instant::now()));
        let mut app = Self::build_core(CoreInputs {
            scan_root,
            http_client,
            bg_tx,
            bg_rx,
            cfg: cfg.clone(),
            scan_started_at,
            builds,
            channels,
            init,
            status_flash,
        });
        app.finish_new();
        app
    }

    fn build_core(inputs: CoreInputs) -> Self {
        let init = inputs.init;
        let channels = inputs.channels;
        let cached_fit_widths = ResolvedWidths::new(inputs.cfg.lint.enabled);
        Self {
            current_config: inputs.cfg,
            scan_root: inputs.scan_root,
            http_client: inputs.http_client,
            projects: init.projects,
            ci_state: HashMap::new(),
            lint_status: HashMap::new(),
            lint_cache_usage: crate::lint::CacheUsage::default(),
            lint_runs: HashMap::new(),
            lint_rollup_status: HashMap::new(),
            lint_rollup_paths: HashMap::new(),
            lint_rollup_keys_by_path: HashMap::new(),
            git_path_states: HashMap::new(),
            cargo_active_paths: HashSet::new(),
            crates_versions: HashMap::new(),
            crates_downloads: HashMap::new(),
            stars: HashMap::new(),
            repo_descriptions: HashMap::new(),
            discovery_shimmers: HashMap::new(),
            pending_git_first_commit: HashMap::new(),
            bg_tx: inputs.bg_tx,
            bg_rx: inputs.bg_rx,
            fully_loaded: HashSet::new(),
            priority_fetch_path: None,
            expanded: HashSet::new(),
            list_state: init.list_state,
            search_query: String::new(),
            filtered: Vec::new(),
            settings_pane: Pane::new(),
            settings_edit_buf: String::new(),
            settings_edit_cursor: 0,
            focused_pane: PaneId::ProjectList,
            return_focus: None,
            visited_panes: std::iter::once(PaneId::ProjectList).collect(),
            package_pane: Pane::new(),
            git_pane: Pane::new(),
            targets_pane: Pane::new(),
            ci_pane: Pane::new(),
            toast_pane: Pane::new(),
            lint_pane: Pane::new(),
            pending_example_run: None,
            pending_ci_fetch: None,
            pending_cleans: VecDeque::new(),
            confirm: None,
            animation_started: Instant::now(),
            ci_fetch_tx: channels.ci_fetch_tx,
            ci_fetch_rx: channels.ci_fetch_rx,
            clean_tx: channels.clean_tx,
            clean_rx: channels.clean_rx,
            example_running: None,
            example_child: Arc::new(Mutex::new(None)),
            example_output: Vec::new(),
            example_tx: channels.example_tx,
            example_rx: channels.example_rx,
            running_clean_paths: HashSet::new(),
            clean_toast: None,
            running_lint_paths: HashMap::new(),
            lint_toast: None,
            watch_tx: init.watch_tx,
            lint_runtime: init.lint_runtime,
            unreachable_services: HashSet::new(),
            service_retry_active: HashSet::new(),
            #[cfg(test)]
            retry_spawn_mode: RetrySpawnMode::Enabled,
            selection_paths: SelectionPaths::new(),
            finder: FinderState::new(),
            cached_visible_rows: Vec::new(),
            cached_root_sorted: Vec::new(),
            cached_child_sorted: HashMap::new(),
            cached_fit_widths,
            builds: inputs.builds,
            data_generation: 0,
            detail_generation: 0,
            cached_detail: None,
            layout_cache: LayoutCache::default(),
            status_flash: inputs.status_flash,
            toasts: ToastManager::default(),
            config_path: init.config_path,
            config_last_seen: init.config_last_seen,
            current_keymap: keymap::ResolvedKeymap::defaults(),
            keymap_path: keymap::keymap_path(),
            keymap_last_seen: None,
            keymap_diagnostics_id: None,
            keymap_pane: Pane::new(),
            inline_error: None,
            ui_modes: UiModes::default(),
            dirty: DirtyState::initial(),
            scan: ScanState::new(inputs.scan_started_at),
            selection: SelectionSync::Stable,
        }
    }

    fn finish_new(&mut self) {
        self.load_initial_keymap();
        if let Some(warning) = self
            .status_flash
            .as_ref()
            .map(|(warning, _)| warning.clone())
        {
            self.show_timed_toast("Lint runtime", warning);
        }
        if self.current_config.tui.include_dirs.is_empty() {
            self.show_timed_toast(
                "Scan root",
                format!(
                    "Using {}. Set include_dirs in Settings to limit scan scope.",
                    crate::project::home_relative_path(&self.scan_root)
                ),
            );
        }
        self.recompute_cargo_active_paths();
        self.prune_inactive_project_state();
        self.register_existing_projects();
        if !self.projects.is_empty() {
            self.finish_watcher_registration_batch();
        }
        self.refresh_lint_runs_from_disk();
        self.rebuild_lint_rollups();
    }
}
