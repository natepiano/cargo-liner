//! Shared TUI test constructors.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Instant;

use tui_pane::SettingsFileSpec;
use tui_pane::SettingsStore;

use super::app::App;
use super::app::RetrySpawnMode;
use super::keymap;
use super::settings;
use super::settings::StartupSettings;
use super::startup_services::StartupEnvironment;
use super::startup_services::StartupServices;
use crate::channel;
use crate::config;
use crate::config::CargoPortConfig;
use crate::constants::APP_NAME;
use crate::constants::CONFIG_FILE;
use crate::http::HttpClient;
use crate::project::RootItem;
use crate::project::WorkspaceMetadataStore;
use crate::test_support;

pub(super) fn test_http_client() -> HttpClient {
    let startup_services = StartupServices::quiet_unit_test();
    test_http_client_with_startup_services(&startup_services)
}

fn test_http_client_with_startup_services(startup_services: &StartupServices) -> HttpClient {
    let runtime = test_support::test_runtime();
    startup_services
        .test_http_client(runtime.handle().clone())
        .expect("test HTTP client initializes")
}

pub(super) fn make_app(projects: &[RootItem]) -> App {
    make_app_with_config(projects, &CargoPortConfig::default())
}

pub(super) fn make_app_with_config(projects: &[RootItem], config: &CargoPortConfig) -> App {
    let app = make_app_with_startup_services(projects, config, StartupServices::quiet_unit_test());
    assert_eq!(
        app.startup_effect_counts().real_total(),
        0,
        "quiet test app should not start host startup effects"
    );
    app
}

pub(super) fn make_app_with_lint_runtime(projects: &[RootItem], config: &CargoPortConfig) -> App {
    make_app_with_startup_services(
        projects,
        config,
        StartupServices::quiet_unit_test_with_lint_runtime(),
    )
}

pub(super) fn make_app_with_startup_services(
    projects: &[RootItem],
    config: &CargoPortConfig,
    startup_services: StartupServices,
) -> App {
    let mut config = config.clone();
    if config.tui.include_dirs.is_empty() {
        config.tui.include_dirs = vec!["/tmp/test".to_string()];
    }
    let config_path = test_config_path();
    let _config_guard = config::override_config_path_for_test(config_path.clone());
    // Hermetic keymap: point keymap loading at a fresh tempfile so
    // tests never read the developer's `~/Library/Application
    // Support/cargo-port/keymap.toml`. The loader writes the
    // in-source default keymap into the empty file, giving every
    // test the same keymap regardless of the host machine.
    let keymap_path = test_keymap_path();
    let _keymap_guard = keymap::override_keymap_path_for_test_if_absent(keymap_path);
    let (background_tx, background_rx) = channel::unbounded();
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let settings_spec = SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(&config_path);
    let mut loaded_settings =
        SettingsStore::load_for_startup(settings_spec, settings::cargo_port_settings_registry())
            .expect("test settings store loads");
    *loaded_settings.store.table_mut() =
        settings::settings_table_from_config(&config).expect("test config becomes settings");
    loaded_settings
        .toast_settings
        .write_to_table(loaded_settings.store.table_mut());
    loaded_settings
        .store
        .save()
        .expect("test settings file is writable");
    let startup_settings = StartupSettings {
        config,
        store: loaded_settings.store,
        toast_settings: loaded_settings.toast_settings,
    };
    let http_client = test_http_client_with_startup_services(&startup_services);
    let startup_environment = StartupEnvironment::with_services(
        http_client,
        Instant::now(),
        metadata_store,
        startup_services,
    );
    let mut app = App::new_with_startup_environment(
        projects,
        background_tx,
        background_rx,
        startup_settings,
        startup_environment,
    )
    .expect("test app initializes");
    app.scan.set_retry_spawn_mode(RetrySpawnMode::Disabled);
    app.sync_selected_project();
    app
}

fn test_config_path() -> PathBuf {
    let file = tempfile::NamedTempFile::new().expect("test config path is available");
    file.into_temp_path()
        .keep()
        .expect("test config path persists")
}

fn test_keymap_path() -> PathBuf {
    let dir = tempfile::tempdir().expect("test keymap directory is available");
    let path = dir.path().join("keymap.toml");
    // Leak the TempDir so the directory survives long enough for
    // `load_keymap` to write the default TOML at `path`. The OS
    // reclaims `/tmp` entries on its own schedule.
    std::mem::forget(dir);
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_http_client_skips_host_github_auth() {
        let client = test_http_client();

        assert!(!client.has_github_token());
    }

    #[test]
    fn make_app_uses_quiet_startup_by_default() {
        let app = make_app(&[]);

        assert_eq!(app.startup_effect_counts().real_total(), 0);
    }
}
