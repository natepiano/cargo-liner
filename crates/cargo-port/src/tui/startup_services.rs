use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
#[cfg(test)]
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

#[cfg(test)]
use tokio::runtime::Handle;
use tui_pane::Theme;
use tui_pane::ThemeRegistry;
use tui_pane::ThemeState;

use crate::channel;
use crate::channel::Receiver;
use crate::channel::SendError;
use crate::channel::Sender;
use crate::config;
use crate::config::CargoPortConfig;
use crate::config::NonRustInclusion;
use crate::constants::SERVICE_RETRY_SECS;
use crate::constants::SERVICE_UNAVAILABLE_GRACE;
use crate::http::HttpClient;
use crate::http::ServiceKind;
use crate::lint;
use crate::lint::RuntimeHandle;
use crate::project::AbsolutePath;
use crate::project::WorkspaceMetadataStore;
use crate::scan;
use crate::scan::BackgroundMsg;
use crate::themes;
use crate::watcher;
use crate::watcher::WatcherMsg;

#[derive(Clone, Debug)]
pub(crate) enum StartupProfile {
    Production,
    #[cfg(test)]
    QuietUnitTest(StartupEffects),
}

impl StartupProfile {
    pub(crate) const fn production() -> Self { Self::Production }

    #[cfg(test)]
    pub(crate) const fn quiet_unit_test() -> Self {
        Self::QuietUnitTest(StartupEffects::quiet_unit_test())
    }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StartupEffects {
    pub(crate) watcher:                  StartupEffect,
    pub(crate) lint_runtime:             StartupEffect,
    pub(crate) lint_history_hydration:   StartupEffect,
    pub(crate) lint_cache_scan:          StartupEffect,
    pub(crate) github_rate_limit_prime:  StartupEffect,
    pub(crate) service_retry_probes:     StartupEffect,
    pub(crate) cpu_monitor:              StartupEffect,
    pub(crate) theme_directory:          StartupEffect,
    pub(crate) process_globals:          StartupEffect,
    pub(crate) host_github_auth:         StartupEffect,
    pub(crate) running_targets_polling:  StartupEffect,
    pub(crate) priority_detail_fetch:    StartupEffect,
    pub(crate) startup_git_first_commit: StartupEffect,
    pub(crate) startup_project_details:  StartupEffect,
    pub(crate) streaming_scan:           StartupEffect,
}

#[cfg(test)]
impl StartupEffects {
    pub(crate) const fn quiet_unit_test() -> Self {
        Self {
            watcher:                  StartupEffect::Suppressed,
            lint_runtime:             StartupEffect::Suppressed,
            lint_history_hydration:   StartupEffect::Suppressed,
            lint_cache_scan:          StartupEffect::Suppressed,
            github_rate_limit_prime:  StartupEffect::Suppressed,
            service_retry_probes:     StartupEffect::Suppressed,
            cpu_monitor:              StartupEffect::Suppressed,
            theme_directory:          StartupEffect::Suppressed,
            process_globals:          StartupEffect::Suppressed,
            host_github_auth:         StartupEffect::Suppressed,
            running_targets_polling:  StartupEffect::Suppressed,
            priority_detail_fetch:    StartupEffect::Suppressed,
            startup_git_first_commit: StartupEffect::Suppressed,
            startup_project_details:  StartupEffect::Suppressed,
            streaming_scan:           StartupEffect::Suppressed,
        }
    }

    pub(crate) const fn quiet_unit_test_with_lint_runtime() -> Self {
        Self {
            lint_runtime: StartupEffect::Real,
            ..Self::quiet_unit_test()
        }
    }

    /// Unit fixture policy for startup code that needs fixture-owned paths.
    ///
    /// Enables only `theme_directory`, which reads the temp directory installed
    /// by `FixtureDirs`. `watcher`, `lint_runtime`, `lint_history_hydration`,
    /// `lint_cache_scan`, `cpu_monitor`, `running_targets_polling`,
    /// `priority_detail_fetch`, `startup_git_first_commit`,
    /// `startup_project_details`, `streaming_scan`, and `process_globals` stay
    /// suppressed because `TestApp` does not own join handles for their worker
    /// threads, Rayon jobs, Tokio tasks, or process-global theme/config slots.
    pub(crate) const fn local_startup_unit_test() -> Self {
        Self {
            theme_directory: StartupEffect::Real,
            host_github_auth: StartupEffect::Suppressed,
            ..Self::quiet_unit_test()
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StartupEffect {
    Real,
    Suppressed,
}

impl StartupEffect {
    const fn runs(self) -> bool { matches!(self, Self::Real) }
}

#[derive(Clone, Debug)]
pub(crate) struct StartupServices {
    profile:                StartupProfile,
    counts:                 Rc<RefCell<StartupEffectCounts>>,
    #[cfg(test)]
    lint_runtime_shutdowns: Rc<RefCell<Vec<JoinHandle<()>>>>,
    #[cfg(test)]
    fixture_cache_root:     Rc<RefCell<Option<PathBuf>>>,
}

impl StartupServices {
    pub(crate) fn new(profile: StartupProfile) -> Self {
        Self {
            profile,
            counts: Rc::new(RefCell::new(StartupEffectCounts::default())),
            #[cfg(test)]
            lint_runtime_shutdowns: Rc::new(RefCell::new(Vec::new())),
            #[cfg(test)]
            fixture_cache_root: Rc::new(RefCell::new(None)),
        }
    }

    #[cfg(test)]
    pub(crate) fn production() -> Self { Self::new(StartupProfile::production()) }

    #[cfg(test)]
    pub(crate) fn quiet_unit_test() -> Self { Self::new(StartupProfile::quiet_unit_test()) }

    #[cfg(test)]
    pub(crate) fn quiet_unit_test_with_lint_runtime() -> Self {
        Self::new(StartupProfile::QuietUnitTest(
            StartupEffects::quiet_unit_test_with_lint_runtime(),
        ))
    }

    #[cfg(test)]
    pub(crate) fn local_startup_unit_test() -> Self {
        Self::new(StartupProfile::QuietUnitTest(
            StartupEffects::local_startup_unit_test(),
        ))
    }

    pub(crate) fn install_active_config(&self, cargo_port_config: &CargoPortConfig) {
        if self.allows(StartupEffectKind::ProcessGlobals) {
            config::set_active_config(cargo_port_config);
            self.record_real(StartupEffectKind::ProcessGlobals);
        } else {
            self.record_suppressed(StartupEffectKind::ProcessGlobals);
        }
    }

    pub(crate) fn themes_dir(&self) -> Option<PathBuf> {
        if self.allows(StartupEffectKind::ThemeDirectory) {
            self.record_real(StartupEffectKind::ThemeDirectory);
            themes::themes_dir()
        } else {
            self.record_suppressed(StartupEffectKind::ThemeDirectory);
            None
        }
    }

    pub(crate) fn install_theme_state(
        &self,
        registry: ThemeRegistry,
        initial_theme: Theme,
        focused_pane_tint: bool,
    ) {
        if self.allows(StartupEffectKind::ProcessGlobals) {
            tui_pane::install_theme_state(ThemeState::with_registry(registry, initial_theme));
            tui_pane::set_focused_pane_tint(focused_pane_tint);
            self.record_real(StartupEffectKind::ProcessGlobals);
        } else {
            self.record_suppressed(StartupEffectKind::ProcessGlobals);
        }
    }

    pub(crate) fn replace_theme_registry(&self, registry: ThemeRegistry) {
        if self.allows(StartupEffectKind::ProcessGlobals) {
            tui_pane::replace_registry(registry);
            self.record_real(StartupEffectKind::ProcessGlobals);
        } else {
            self.record_suppressed(StartupEffectKind::ProcessGlobals);
        }
    }

    pub(crate) fn publish_active_theme(&self, theme: Arc<Theme>, focused_pane_tint: bool) {
        if self.allows(StartupEffectKind::ProcessGlobals) {
            tui_pane::set_active_theme(theme);
            tui_pane::set_focused_pane_tint(focused_pane_tint);
            self.record_real(StartupEffectKind::ProcessGlobals);
        } else {
            self.record_suppressed(StartupEffectKind::ProcessGlobals);
        }
    }

    pub(crate) fn spawn_lint_runtime(
        &self,
        cargo_port_config: &CargoPortConfig,
        background_tx: Sender<BackgroundMsg>,
    ) -> LintRuntimeStartup {
        if !self.allows(StartupEffectKind::LintRuntime) {
            self.record_suppressed(StartupEffectKind::LintRuntime);
            return LintRuntimeStartup::default();
        }
        #[cfg(test)]
        let fixture_config = self.fixture_cache_root.borrow().clone().map(|cache_root| {
            let mut cargo_port_config = cargo_port_config.clone();
            cargo_port_config.cache.root = cache_root.display().to_string();
            cargo_port_config
        });
        #[cfg(test)]
        let cargo_port_config = fixture_config.as_ref().unwrap_or(cargo_port_config);
        let spawn = lint::spawn(cargo_port_config, background_tx);
        if spawn.handle.is_some() {
            self.record_real(StartupEffectKind::LintRuntime);
        }
        #[cfg(test)]
        if let Some(supervisor) = spawn.supervisor {
            self.lint_runtime_shutdowns.borrow_mut().push(supervisor);
        }
        LintRuntimeStartup {
            handle:  spawn.handle,
            warning: spawn.warning,
        }
    }

    pub(crate) fn spawn_watcher(&self, startup: WatcherStartup<'_>) -> WatcherHandle {
        if self.allows(StartupEffectKind::Watcher) {
            self.record_real(StartupEffectKind::Watcher);
            WatcherHandle::active(watcher::spawn_watcher(
                startup.watch_roots,
                startup.background_tx,
                startup.ci_run_count,
                startup.non_rust,
                startup.client,
                startup.lint_runtime,
                startup.metadata_store,
            ))
        } else {
            self.record_suppressed(StartupEffectKind::Watcher);
            WatcherHandle::disabled()
        }
    }

    pub(crate) fn spawn_streaming_scan(
        &self,
        startup: StreamingScanStartup<'_>,
    ) -> StreamingScanStart {
        let effect = self.streaming_scan_effect();
        self.record_streaming_scan(effect);
        if effect == StartupEffect::Suppressed {
            let (sender, receiver) = channel::unbounded();
            return StreamingScanStart {
                sender,
                receiver,
                effect,
            };
        }

        let (sender, receiver) = scan::spawn_streaming_scan(
            startup.scan_dirs,
            startup.inline_dirs,
            startup.non_rust,
            startup.client,
            startup.metadata_store,
        );
        StreamingScanStart {
            sender,
            receiver,
            effect,
        }
    }

    pub(crate) fn spawn_github_rate_limit_prime(&self, client: HttpClient) {
        if !self.allows(StartupEffectKind::GithubRateLimitPrime) {
            self.record_suppressed(StartupEffectKind::GithubRateLimitPrime);
            return;
        }
        self.record_real(StartupEffectKind::GithubRateLimitPrime);
        thread::spawn(move || {
            let (rate_limit, _signal) = client.fetch_rate_limit();
            if rate_limit.is_some() {
                tracing::info!("rate_limit_prime_ok");
            } else {
                tracing::info!("rate_limit_prime_failed");
            }
        });
    }

    pub(crate) fn spawn_service_retry_probe(
        &self,
        sender: Sender<BackgroundMsg>,
        client: HttpClient,
        service: ServiceKind,
    ) {
        if !self.allows(StartupEffectKind::ServiceRetryProbes) {
            self.record_suppressed(StartupEffectKind::ServiceRetryProbes);
            return;
        }
        self.record_real(StartupEffectKind::ServiceRetryProbes);
        thread::spawn(move || {
            thread::sleep(SERVICE_UNAVAILABLE_GRACE);
            if client.probe_service(service) {
                scan::emit_service_recovered(&sender, service);
                return;
            }
            let _ = sender.send(BackgroundMsg::ServiceUnreachableConfirmed { service });
            loop {
                if client.probe_service(service) {
                    scan::emit_service_recovered(&sender, service);
                    break;
                }
                thread::sleep(Duration::from_secs(SERVICE_RETRY_SECS));
            }
        });
    }

    pub(crate) const fn lint_history_hydration_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::LintHistoryHydration)
    }

    pub(crate) const fn lint_cache_scan_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::LintCacheScan)
    }

    pub(crate) const fn cpu_monitor_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::CpuMonitor)
    }

    pub(crate) const fn running_targets_polling_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::RunningTargetsPolling)
    }

    pub(crate) const fn process_globals_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::ProcessGlobals)
    }

    pub(crate) const fn theme_directory_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::ThemeDirectory)
    }

    pub(crate) const fn priority_detail_fetch_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::PriorityDetailFetch)
    }

    pub(crate) const fn startup_git_first_commit_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::GitFirstCommit)
    }

    pub(crate) const fn startup_project_details_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::ProjectDetails)
    }

    pub(crate) const fn streaming_scan_effect(&self) -> StartupEffect {
        self.effect(StartupEffectKind::StreamingScan)
    }

    pub(crate) fn record_lint_history_hydration(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::LintHistoryHydration, effect);
    }

    pub(crate) fn record_lint_cache_scan(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::LintCacheScan, effect);
    }

    pub(crate) fn record_cpu_monitor(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::CpuMonitor, effect);
    }

    pub(crate) fn record_running_targets_polling(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::RunningTargetsPolling, effect);
    }

    pub(crate) fn record_process_globals(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::ProcessGlobals, effect);
    }

    pub(crate) fn record_theme_directory(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::ThemeDirectory, effect);
    }

    pub(crate) fn record_priority_detail_fetch(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::PriorityDetailFetch, effect);
    }

    pub(crate) fn record_startup_git_first_commit(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::GitFirstCommit, effect);
    }

    pub(crate) fn record_startup_project_details(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::ProjectDetails, effect);
    }

    pub(crate) fn record_streaming_scan(&self, effect: StartupEffect) {
        self.record_effect(StartupEffectKind::StreamingScan, effect);
    }

    #[cfg(test)]
    pub(crate) fn test_http_client(&self, handle: Handle) -> Option<HttpClient> {
        if self.allows(StartupEffectKind::HostGithubAuth) {
            self.record_real(StartupEffectKind::HostGithubAuth);
            HttpClient::new(handle)
        } else {
            self.record_suppressed(StartupEffectKind::HostGithubAuth);
            HttpClient::new_without_github_auth_for_test(handle)
        }
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> StartupEffectCounts { *self.counts.borrow() }

    #[cfg(test)]
    pub(crate) fn lint_runtime_shutdown_count_for_test(&self) -> usize {
        self.lint_runtime_shutdowns.borrow().len()
    }

    #[cfg(test)]
    pub(crate) fn join_lint_runtime_shutdowns_for_test(&self) {
        let supervisors: Vec<_> = self.lint_runtime_shutdowns.borrow_mut().drain(..).collect();
        for supervisor in supervisors {
            assert!(
                supervisor.join().is_ok(),
                "lint runtime supervisor should exit cleanly"
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn set_fixture_cache_root_for_test(&self, cache_root: PathBuf) {
        *self.fixture_cache_root.borrow_mut() = Some(cache_root);
    }

    #[cfg(test)]
    pub(crate) const fn has_production_profile_for_test(&self) -> bool {
        matches!(self.profile, StartupProfile::Production)
    }

    #[cfg(test)]
    pub(crate) const fn requires_fixture_cache_root_for_test(&self) -> bool {
        self.effect(StartupEffectKind::LintRuntime).runs()
            || self.effect(StartupEffectKind::LintHistoryHydration).runs()
            || self.effect(StartupEffectKind::LintCacheScan).runs()
    }

    #[cfg(test)]
    pub(crate) const fn serializes_process_globals_for_test(&self) -> bool {
        self.effect(StartupEffectKind::ProcessGlobals).runs()
    }

    const fn allows(&self, kind: StartupEffectKind) -> bool { self.effect(kind).runs() }

    const fn effect(&self, kind: StartupEffectKind) -> StartupEffect {
        match &self.profile {
            StartupProfile::Production => match kind {
                StartupEffectKind::Watcher
                | StartupEffectKind::LintRuntime
                | StartupEffectKind::LintHistoryHydration
                | StartupEffectKind::LintCacheScan
                | StartupEffectKind::GithubRateLimitPrime
                | StartupEffectKind::ServiceRetryProbes
                | StartupEffectKind::CpuMonitor
                | StartupEffectKind::ThemeDirectory
                | StartupEffectKind::ProcessGlobals
                | StartupEffectKind::RunningTargetsPolling
                | StartupEffectKind::PriorityDetailFetch
                | StartupEffectKind::GitFirstCommit
                | StartupEffectKind::ProjectDetails
                | StartupEffectKind::StreamingScan => StartupEffect::Real,
                #[cfg(test)]
                StartupEffectKind::HostGithubAuth => StartupEffect::Real,
            },
            #[cfg(test)]
            StartupProfile::QuietUnitTest(effects) => match kind {
                StartupEffectKind::Watcher => effects.watcher,
                StartupEffectKind::LintRuntime => effects.lint_runtime,
                StartupEffectKind::LintHistoryHydration => effects.lint_history_hydration,
                StartupEffectKind::LintCacheScan => effects.lint_cache_scan,
                StartupEffectKind::GithubRateLimitPrime => effects.github_rate_limit_prime,
                StartupEffectKind::ServiceRetryProbes => effects.service_retry_probes,
                StartupEffectKind::CpuMonitor => effects.cpu_monitor,
                StartupEffectKind::ThemeDirectory => effects.theme_directory,
                StartupEffectKind::ProcessGlobals => effects.process_globals,
                #[cfg(test)]
                StartupEffectKind::HostGithubAuth => effects.host_github_auth,
                StartupEffectKind::RunningTargetsPolling => effects.running_targets_polling,
                StartupEffectKind::PriorityDetailFetch => effects.priority_detail_fetch,
                StartupEffectKind::GitFirstCommit => effects.startup_git_first_commit,
                StartupEffectKind::ProjectDetails => effects.startup_project_details,
                StartupEffectKind::StreamingScan => effects.streaming_scan,
            },
        }
    }

    fn record_effect(&self, kind: StartupEffectKind, effect: StartupEffect) {
        if effect.runs() {
            self.record_real(kind);
        } else {
            self.record_suppressed(kind);
        }
    }

    fn record_real(&self, kind: StartupEffectKind) {
        self.counts.borrow_mut().count_for(kind).real += 1;
    }

    fn record_suppressed(&self, kind: StartupEffectKind) {
        self.counts.borrow_mut().count_for(kind).suppressed += 1;
    }
}

#[derive(Clone)]
pub(crate) enum WatcherHandle {
    Active(Sender<WatcherMsg>),
    Disabled,
}

impl WatcherHandle {
    pub(crate) const fn disabled() -> Self { Self::Disabled }

    pub(crate) const fn active(sender: Sender<WatcherMsg>) -> Self { Self::Active(sender) }

    pub(crate) fn send(&self, msg: WatcherMsg) -> Result<(), SendError<WatcherMsg>> {
        match self {
            Self::Active(sender) => sender.send(msg),
            Self::Disabled => Ok(()),
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_active(&self) -> bool { matches!(self, Self::Active(_)) }
}

#[derive(Clone)]
pub(crate) struct StartupEnvironment {
    pub(crate) http_client:      HttpClient,
    pub(crate) scan_started_at:  Instant,
    pub(crate) metadata_store:   Arc<Mutex<WorkspaceMetadataStore>>,
    pub(crate) startup_services: StartupServices,
}

impl StartupEnvironment {
    pub(crate) fn production(
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
    ) -> Self {
        Self {
            http_client,
            scan_started_at,
            metadata_store,
            startup_services: StartupServices::new(StartupProfile::production()),
        }
    }

    #[cfg(test)]
    pub(crate) const fn with_services(
        http_client: HttpClient,
        scan_started_at: Instant,
        metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
        startup_services: StartupServices,
    ) -> Self {
        Self {
            http_client,
            scan_started_at,
            metadata_store,
            startup_services,
        }
    }
}

pub(crate) struct WatcherStartup<'a> {
    pub(crate) watch_roots:    &'a [AbsolutePath],
    pub(crate) background_tx:  Sender<BackgroundMsg>,
    pub(crate) ci_run_count:   u32,
    pub(crate) non_rust:       NonRustInclusion,
    pub(crate) client:         HttpClient,
    pub(crate) lint_runtime:   Option<RuntimeHandle>,
    pub(crate) metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
}

pub(crate) struct StreamingScanStartup<'a> {
    pub(crate) scan_dirs:      Vec<AbsolutePath>,
    pub(crate) inline_dirs:    &'a [String],
    pub(crate) non_rust:       NonRustInclusion,
    pub(crate) client:         HttpClient,
    pub(crate) metadata_store: Arc<Mutex<WorkspaceMetadataStore>>,
}

pub(crate) struct StreamingScanStart {
    pub(crate) sender:   Sender<BackgroundMsg>,
    pub(crate) receiver: Receiver<BackgroundMsg>,
    pub(crate) effect:   StartupEffect,
}

#[derive(Clone, Default)]
pub(crate) struct LintRuntimeStartup {
    pub(crate) handle:  Option<RuntimeHandle>,
    pub(crate) warning: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StartupEffectCounts {
    watcher:                  EffectCount,
    lint_runtime:             EffectCount,
    lint_history_hydration:   EffectCount,
    lint_cache_scan:          EffectCount,
    github_rate_limit_prime:  EffectCount,
    service_retry_probes:     EffectCount,
    cpu_monitor:              EffectCount,
    theme_directory:          EffectCount,
    process_globals:          EffectCount,
    host_github_auth:         EffectCount,
    running_targets_polling:  EffectCount,
    priority_detail_fetch:    EffectCount,
    startup_git_first_commit: EffectCount,
    startup_project_details:  EffectCount,
    streaming_scan:           EffectCount,
}

impl StartupEffectCounts {
    #[cfg(test)]
    pub(crate) const fn real_total(self) -> usize {
        self.watcher.real
            + self.lint_runtime.real
            + self.lint_history_hydration.real
            + self.lint_cache_scan.real
            + self.github_rate_limit_prime.real
            + self.service_retry_probes.real
            + self.cpu_monitor.real
            + self.theme_directory.real
            + self.process_globals.real
            + self.host_github_auth.real
            + self.running_targets_polling.real
            + self.priority_detail_fetch.real
            + self.startup_git_first_commit.real
            + self.startup_project_details.real
            + self.streaming_scan.real
    }

    #[cfg(test)]
    pub(crate) const fn watcher(self) -> EffectCount { self.watcher }

    #[cfg(test)]
    pub(crate) const fn lint_runtime(self) -> EffectCount { self.lint_runtime }

    #[cfg(test)]
    pub(crate) const fn lint_history_hydration(self) -> EffectCount { self.lint_history_hydration }

    #[cfg(test)]
    pub(crate) const fn lint_cache_scan(self) -> EffectCount { self.lint_cache_scan }

    #[cfg(test)]
    pub(crate) const fn github_rate_limit_prime(self) -> EffectCount {
        self.github_rate_limit_prime
    }

    #[cfg(test)]
    pub(crate) const fn service_retry_probes(self) -> EffectCount { self.service_retry_probes }

    #[cfg(test)]
    pub(crate) const fn cpu_monitor(self) -> EffectCount { self.cpu_monitor }

    #[cfg(test)]
    pub(crate) const fn theme_directory(self) -> EffectCount { self.theme_directory }

    #[cfg(test)]
    pub(crate) const fn process_globals(self) -> EffectCount { self.process_globals }

    #[cfg(test)]
    pub(crate) const fn host_github_auth(self) -> EffectCount { self.host_github_auth }

    #[cfg(test)]
    pub(crate) const fn startup_project_details(self) -> EffectCount {
        self.startup_project_details
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EffectCount {
    real:       usize,
    suppressed: usize,
}

impl EffectCount {
    #[cfg(test)]
    pub(crate) const fn real(self) -> usize { self.real }

    #[cfg(test)]
    pub(crate) const fn suppressed(self) -> usize { self.suppressed }
}

impl StartupEffectCounts {
    const fn count_for(&mut self, kind: StartupEffectKind) -> &mut EffectCount {
        match kind {
            StartupEffectKind::Watcher => &mut self.watcher,
            StartupEffectKind::LintRuntime => &mut self.lint_runtime,
            StartupEffectKind::LintHistoryHydration => &mut self.lint_history_hydration,
            StartupEffectKind::LintCacheScan => &mut self.lint_cache_scan,
            StartupEffectKind::GithubRateLimitPrime => &mut self.github_rate_limit_prime,
            StartupEffectKind::ServiceRetryProbes => &mut self.service_retry_probes,
            StartupEffectKind::CpuMonitor => &mut self.cpu_monitor,
            StartupEffectKind::ThemeDirectory => &mut self.theme_directory,
            StartupEffectKind::ProcessGlobals => &mut self.process_globals,
            #[cfg(test)]
            StartupEffectKind::HostGithubAuth => &mut self.host_github_auth,
            StartupEffectKind::RunningTargetsPolling => &mut self.running_targets_polling,
            StartupEffectKind::PriorityDetailFetch => &mut self.priority_detail_fetch,
            StartupEffectKind::GitFirstCommit => &mut self.startup_git_first_commit,
            StartupEffectKind::ProjectDetails => &mut self.startup_project_details,
            StartupEffectKind::StreamingScan => &mut self.streaming_scan,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupEffectKind {
    Watcher,
    LintRuntime,
    LintHistoryHydration,
    LintCacheScan,
    GithubRateLimitPrime,
    ServiceRetryProbes,
    CpuMonitor,
    ThemeDirectory,
    ProcessGlobals,
    #[cfg(test)]
    HostGithubAuth,
    RunningTargetsPolling,
    PriorityDetailFetch,
    GitFirstCommit,
    ProjectDetails,
    StreamingScan,
}
