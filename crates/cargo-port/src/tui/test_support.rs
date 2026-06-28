//! Shared TUI test constructors.

#![allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]

use std::ops::Deref;
use std::ops::DerefMut;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::OnceLock;
use std::time::Instant;

use tempfile::TempDir;
use tui_pane::Appearance;
use tui_pane::SettingsFileSpec;
use tui_pane::SettingsStore;

use super::app::App;
use super::app::RetrySpawnMode;
use super::keymap;
use super::keymap::KeymapPathOverrideGuard;
use super::settings;
use super::settings::StartupSettings;
use super::startup_services::StartupEffect;
use super::startup_services::StartupEffects;
use super::startup_services::StartupEnvironment;
use super::startup_services::StartupServices;
use crate::channel;
use crate::config;
use crate::config::CargoPortConfig;
use crate::config::ConfigPathOverrideGuard;
use crate::config::GitHubRateLimitMode;
use crate::config::LintIndicator;
use crate::constants::APP_NAME;
use crate::constants::CONFIG_FILE;
use crate::constants::KEYMAP_FILE;
use crate::http::HttpClient;
use crate::project::AbsolutePath;
use crate::project::Package;
use crate::project::RootItem;
use crate::project::RustProject;
use crate::project::WorkspaceMetadataStore;
use crate::test_support;
use crate::themes;
use crate::themes::ThemesDirOverrideGuard;

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

pub(super) fn make_app_with_config(
    projects: &[RootItem],
    cargo_port_config: &CargoPortConfig,
) -> App {
    let app = make_test_app_with_startup_services(
        projects,
        cargo_port_config,
        StartupServices::quiet_unit_test(),
    )
    .into_quiet_app();
    assert_eq!(app.startup_effect_counts().real_total(), 0);
    app
}

pub(super) fn make_app_with_lint_runtime(
    projects: &[RootItem],
    cargo_port_config: &CargoPortConfig,
) -> TestApp {
    make_test_app_with_startup_services(
        projects,
        cargo_port_config,
        StartupServices::quiet_unit_test_with_lint_runtime(),
    )
}

pub(super) fn make_local_startup_test_app(
    projects: &[RootItem],
    cargo_port_config: &CargoPortConfig,
) -> TestApp {
    make_test_app_with_startup_services(
        projects,
        cargo_port_config,
        StartupServices::local_startup_unit_test(),
    )
}

fn make_test_app_with_startup_services(
    projects: &[RootItem],
    cargo_port_config: &CargoPortConfig,
    startup_services: StartupServices,
) -> TestApp {
    assert!(
        !startup_services.has_production_profile_for_test(),
        "TestApp does not own production startup worker handles"
    );
    let mut cargo_port_config = cargo_port_config.clone();
    if cargo_port_config.tui.include_dirs.is_empty() {
        cargo_port_config.tui.include_dirs = vec!["/tmp/test".to_string()];
    }
    let serial = startup_services
        .serializes_process_globals_for_test()
        .then(acquire_startup_test_lock);
    let fixture_dirs = FixtureDirs::new();
    let config_path = fixture_dirs.config_path();
    let keymap_path = fixture_dirs.keymap_path();
    let themes_path = fixture_dirs.themes_path();
    if startup_services.requires_fixture_cache_root_for_test() {
        let cache_path = fixture_dirs.cache_path();
        cargo_port_config.cache.root = cache_path.display().to_string();
        startup_services.set_fixture_cache_root_for_test(cache_path);
    }
    let config_path_guard = config::override_config_path_for_test(config_path.clone());
    let keymap_path_guard = keymap::override_keymap_path_for_test_if_absent(keymap_path);
    let themes_dir_guard = themes::set_themes_dir_override_for_test(themes_path);
    let (background_tx, background_rx) = channel::unbounded();
    let metadata_store = Arc::new(Mutex::new(WorkspaceMetadataStore::new()));
    let settings_spec = SettingsFileSpec::new(APP_NAME, CONFIG_FILE).with_path(&config_path);
    let mut loaded_settings =
        SettingsStore::load_for_startup(settings_spec, settings::cargo_port_settings_registry())
            .expect("test settings store loads");
    *loaded_settings.store.table_mut() = settings::settings_table_from_config(&cargo_port_config)
        .expect("test config becomes settings");
    loaded_settings
        .toast_settings
        .write_to_table(loaded_settings.store.table_mut());
    loaded_settings
        .store
        .save()
        .expect("test settings file is writable");
    let startup_settings = StartupSettings {
        cargo_port_config,
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
    TestApp {
        app: Some(app),
        override_guards: Some(OverrideGuards {
            config_path: config_path_guard,
            keymap_path: keymap_path_guard,
            themes_dir:  themes_dir_guard,
        }),
        fixture_dirs: Some(fixture_dirs),
        serial,
    }
}

pub(super) struct TestApp {
    app:             Option<App>,
    override_guards: Option<OverrideGuards>,
    fixture_dirs:    Option<FixtureDirs>,
    serial:          Option<MutexGuard<'static, ()>>,
}

impl TestApp {
    fn app(&self) -> &App { self.app.as_ref().expect("test app should be live") }

    fn app_mut(&mut self) -> &mut App { self.app.as_mut().expect("test app should be live") }

    fn fixture_cache_root(&self) -> &Path {
        self.fixture_dirs
            .as_ref()
            .expect("test fixture directories should be live")
            .cache
            .path()
    }

    fn lint_runtime_shutdown_count(&self) -> usize {
        self.app()
            .startup_services
            .lint_runtime_shutdown_count_for_test()
    }

    fn into_quiet_app(mut self) -> App {
        let app = self.app.take().expect("test app should be live");
        assert_eq!(
            app.startup_effect_counts().real_total(),
            0,
            "only quiet fixtures may return an unowned App"
        );
        if let Some(fixture_dirs) = self.fixture_dirs.take() {
            fixture_dirs.persist_for_returned_app();
        }
        app
    }
}

impl Deref for TestApp {
    type Target = App;

    fn deref(&self) -> &Self::Target { self.app() }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut Self::Target { self.app_mut() }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let startup_services = self.app.as_ref().map(|app| app.startup_services.clone());
        drop(self.app.take());
        if let Some(startup_services) = startup_services {
            startup_services.join_lint_runtime_shutdowns_for_test();
        }
        if let Some(override_guards) = self.override_guards.as_ref() {
            override_guards.touch();
        }
        drop(self.override_guards.take());
        drop(self.fixture_dirs.take());
        drop(self.serial.take());
    }
}

struct OverrideGuards {
    config_path: ConfigPathOverrideGuard,
    keymap_path: KeymapPathOverrideGuard,
    themes_dir:  ThemesDirOverrideGuard,
}

impl OverrideGuards {
    fn touch(&self) {
        let Self {
            config_path,
            keymap_path,
            themes_dir,
        } = self;
        let _ = (config_path, keymap_path, themes_dir);
    }
}

struct FixtureDirs {
    cache:  TempDir,
    config: TempDir,
    keymap: TempDir,
    themes: TempDir,
}

impl FixtureDirs {
    fn new() -> Self {
        Self {
            cache:  tempfile::tempdir().expect("test cache directory is available"),
            config: tempfile::tempdir().expect("test config directory is available"),
            keymap: tempfile::tempdir().expect("test keymap directory is available"),
            themes: tempfile::tempdir().expect("test themes directory is available"),
        }
    }

    fn cache_path(&self) -> PathBuf { self.cache.path().to_path_buf() }

    fn config_path(&self) -> PathBuf { self.config.path().join(CONFIG_FILE) }

    fn keymap_path(&self) -> PathBuf { self.keymap.path().join(KEYMAP_FILE) }

    fn themes_path(&self) -> PathBuf { self.themes.path().to_path_buf() }

    fn persist_for_returned_app(self) {
        let Self {
            cache,
            config,
            keymap,
            themes,
        } = self;
        std::mem::forget(cache);
        std::mem::forget(config);
        std::mem::forget(keymap);
        std::mem::forget(themes);
    }
}

fn acquire_startup_test_lock() -> MutexGuard<'static, ()> {
    startup_test_lock()
        .lock()
        .expect("startup test lock is available")
}

fn startup_test_lock() -> &'static Mutex<()> {
    static STARTUP_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    STARTUP_TEST_LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_project() -> RootItem {
        RootItem::Rust(RustProject::Package(Package {
            path: AbsolutePath::from("/tmp/cargo-port-test-project"),
            name: Some("demo".to_string()),
            ..Package::default()
        }))
    }

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

    #[test]
    fn quiet_startup_persists_through_reload_and_reset_paths() {
        let mut app = make_app(&[test_project()]);
        app.scan.set_retry_spawn_mode(RetrySpawnMode::Enabled);

        let mut reloaded = app.config.current().clone();
        reloaded.cpu.poll_ms = reloaded.cpu.poll_ms.saturating_add(1);
        reloaded.lint.enabled = LintIndicator::Enabled;
        reloaded.debug.force_github_rate_limit = GitHubRateLimitMode::Forced;
        reloaded.tui.include_dirs = vec!["/tmp/cargo-port-reloaded".to_string()];
        app.apply_config(&reloaded);
        app.set_terminal_appearance(Some(Appearance::Dark));
        app.maybe_priority_fetch();
        app.rescan();

        assert_eq!(
            app.startup_effect_counts().real_total(),
            0,
            "quiet reload and reset paths must keep every startup effect suppressed"
        );
        assert!(
            !app.background.watcher_is_active(),
            "quiet watcher respawn installs a disabled watcher handle"
        );
    }

    #[test]
    fn lint_runtime_opt_in_enables_only_lint_runtime_startup() {
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;

        let app = make_app_with_lint_runtime(&[], &cargo_port_config);
        let counts = app.startup_effect_counts();
        let configured_cache_root = Path::new(&app.config.current().cache.root);
        let lint_cache_root = crate::cache_paths::lint_runs_root_for(app.config.current());

        assert_eq!(configured_cache_root, app.fixture_cache_root());
        assert!(
            configured_cache_root.exists(),
            "fixture-owned cache root should exist before lint runtime startup"
        );
        assert!(
            lint_cache_root
                .as_path()
                .starts_with(app.fixture_cache_root()),
            "lint runtime cache path must stay under the fixture cache root"
        );
        assert_eq!(counts.lint_runtime().real(), 1);
        assert_eq!(app.lint_runtime_shutdown_count(), 1);
        assert_eq!(counts.real_total(), 1);
        assert_eq!(counts.watcher().real(), 0);
        assert_eq!(counts.github_rate_limit_prime().real(), 0);
        assert_eq!(counts.cpu_monitor().real(), 0);
        let startup_services = app.startup_services.clone();
        let fixture_cache_root = app.fixture_cache_root().to_path_buf();
        drop(app);
        assert_eq!(startup_services.lint_runtime_shutdown_count_for_test(), 0);
        assert!(
            !fixture_cache_root.exists(),
            "fixture-owned cache root should be removed after lint runtime shutdown"
        );
    }

    #[test]
    fn local_startup_fixture_owns_theme_dir_and_suppresses_unowned_effects() {
        let include_dir = tempfile::tempdir().expect("test include directory is available");
        let mut cargo_port_config = CargoPortConfig::default();
        cargo_port_config.lint.enabled = LintIndicator::Enabled;
        cargo_port_config.tui.include_dirs = vec![include_dir.path().display().to_string()];

        let app = make_local_startup_test_app(&[], &cargo_port_config);
        let counts = app.startup_effect_counts();
        let effects = StartupEffects::local_startup_unit_test();

        assert_eq!(effects.theme_directory, StartupEffect::Real);
        assert_eq!(effects.watcher, StartupEffect::Suppressed);
        assert_eq!(effects.lint_runtime, StartupEffect::Suppressed);
        assert_eq!(effects.lint_history_hydration, StartupEffect::Suppressed);
        assert_eq!(effects.lint_cache_scan, StartupEffect::Suppressed);
        assert_eq!(effects.cpu_monitor, StartupEffect::Suppressed);
        assert_eq!(effects.process_globals, StartupEffect::Suppressed);
        assert_eq!(effects.running_targets_polling, StartupEffect::Suppressed);
        assert_eq!(effects.priority_detail_fetch, StartupEffect::Suppressed);
        assert_eq!(effects.startup_git_first_commit, StartupEffect::Suppressed);
        assert_eq!(effects.startup_project_details, StartupEffect::Suppressed);
        assert_eq!(effects.streaming_scan, StartupEffect::Suppressed);
        assert!(app.config.path().is_some_and(std::path::Path::exists));
        assert!(app.keymap.path().is_some_and(std::path::Path::exists));
        assert!(app.themes.dir().is_some_and(std::path::Path::exists));
        assert!(!app.background.watcher_is_active());
        assert_eq!(counts.watcher().real(), 0);
        assert_eq!(counts.lint_runtime().real(), 0);
        assert_eq!(counts.lint_history_hydration().real(), 0);
        assert_eq!(counts.lint_cache_scan().real(), 0);
        assert_eq!(counts.cpu_monitor().real(), 0);
        assert_eq!(counts.theme_directory().real(), 1);
        assert_eq!(counts.process_globals().real(), 0);
        assert_eq!(counts.host_github_auth().real(), 0);
        assert_eq!(counts.host_github_auth().suppressed(), 1);
        assert_eq!(counts.github_rate_limit_prime().real(), 0);
        assert_eq!(counts.github_rate_limit_prime().suppressed(), 1);
        assert_eq!(counts.service_retry_probes().real(), 0);
        assert_eq!(counts.startup_project_details().real(), 0);
        drop(app);
    }
}
