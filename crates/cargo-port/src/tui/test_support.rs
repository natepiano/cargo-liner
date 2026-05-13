//! Shared TUI test constructors.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use tui_pane::SettingsFileSpec;
use tui_pane::SettingsStore;

use super::app::App;
use super::app::RetrySpawnMode;
use super::settings;
use super::settings::StartupSettings;
use crate::config;
use crate::config::CargoPortConfig;
use crate::constants::APP_NAME;
use crate::constants::CONFIG_FILE;
use crate::http::HttpClient;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::test_support;

pub(super) fn test_http_client() -> HttpClient {
    let runtime = test_support::test_runtime();
    HttpClient::new(runtime.handle().clone()).unwrap_or_else(|| std::process::abort())
}

pub(super) fn make_app(projects: &[RootItem]) -> App {
    make_app_with_config(projects, &CargoPortConfig::default())
}

pub(super) fn make_app_with_config(projects: &[RootItem], config: &CargoPortConfig) -> App {
    let mut config = config.clone();
    if config.tui.include_dirs.is_empty() {
        config.tui.include_dirs = vec!["/tmp/test".to_string()];
    }
    let config_path = test_config_path();
    let _config_guard = config::override_config_path_for_test(config_path.clone());
    let (bg_tx, bg_rx) = mpsc::channel();
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let settings_spec = SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(&config_path);
    let mut loaded_settings =
        SettingsStore::load_for_startup(settings_spec, settings::cargo_port_settings_registry())
            .unwrap_or_else(|_| std::process::abort());
    *loaded_settings.store.table_mut() =
        settings::settings_table_from_config(&config).unwrap_or_else(|_| std::process::abort());
    loaded_settings
        .toast_settings
        .write_to_table(loaded_settings.store.table_mut());
    loaded_settings
        .store
        .save()
        .unwrap_or_else(|_| std::process::abort());
    let startup_settings = StartupSettings {
        config,
        store: loaded_settings.store,
        toast_settings: loaded_settings.toast_settings,
    };
    let mut app = App::new(
        projects,
        bg_tx,
        bg_rx,
        startup_settings,
        test_http_client(),
        Instant::now(),
        metadata_store,
    )
    .unwrap_or_else(|_| std::process::abort());
    app.scan.set_retry_spawn_mode(RetrySpawnMode::Disabled);
    app.sync_selected_project();
    app
}

fn test_config_path() -> PathBuf {
    let file = tempfile::NamedTempFile::new().unwrap_or_else(|_| std::process::abort());
    file.into_temp_path()
        .keep()
        .unwrap_or_else(|_| std::process::abort())
}
