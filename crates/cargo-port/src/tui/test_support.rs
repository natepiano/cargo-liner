//! Shared TUI test constructors.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Instant;

use super::app::App;
use super::app::RetrySpawnMode;
use crate::config;
use crate::config::CargoPortConfig;
use crate::http::HttpClient;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;

pub(super) fn test_http_client() -> HttpClient {
    let runtime = crate::test_support::test_runtime();
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
    let _config_guard = config::override_config_path_for_test(config_path);
    let (bg_tx, bg_rx) = mpsc::channel();
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let mut app = App::new(
        projects,
        bg_tx,
        bg_rx,
        &config,
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
