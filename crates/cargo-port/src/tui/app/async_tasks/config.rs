use std::time::Instant;

use tui_pane::ToastStyle::Error;

use crate::config;
use crate::config::CargoPortConfig;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::keymap;
use crate::keymap::KeymapError;
use crate::keymap::KeymapErrorReason::Parse;
use crate::lint;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::CargoPortToastAction;
use crate::tui::integration;
use crate::tui::integration::ReloadContext;
use crate::tui::integration::TreeReaction;
use crate::tui::keymap_ui;

impl App {
    pub(super) fn record_config_reload_failure(&mut self, err: &str) {
        self.overlays.set_status_flash(
            "Config reload failed; keeping previous settings".to_string(),
            Instant::now(),
        );
        self.show_timed_toast("Config reload failed", err.to_string());
    }
    pub fn load_initial_keymap(&mut self) {
        let vim_mode = self.config.current().tui.navigation_keys;
        let keymap_missing = self.keymap.path().is_some_and(|path| !path.exists());
        let result = keymap::load_keymap(vim_mode);
        self.keymap.replace_current(result.keymap);
        self.keymap.sync_stamp();
        if !result.errors.is_empty() {
            self.show_keymap_diagnostics(&result.errors);
        }
        if !result.missing_actions.is_empty() {
            keymap_ui::save_current_keymap_to_disk(self);
            self.show_timed_toast(
                "Keymap updated",
                format!(
                    "Defaults written for missing entries:\n{}",
                    result.missing_actions.join(", ")
                ),
            );
        } else if keymap_missing {
            keymap_ui::save_current_keymap_to_disk(self);
        }
    }
    pub fn maybe_reload_keymap_from_disk(&mut self) {
        let Some(path) = self.keymap.take_stamp_change() else {
            return;
        };
        let path = path.to_path_buf();
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                self.show_keymap_diagnostics(&[KeymapError {
                    scope:  String::new(),
                    action: String::new(),
                    key:    String::new(),
                    reason: Parse(format!("read error: {e}")),
                }]);
                return;
            },
        };

        let vim_mode = self.config.current().tui.navigation_keys;
        let result = keymap::load_keymap_from_str(&contents, vim_mode);
        self.keymap.replace_current(result.keymap);

        if result.errors.is_empty() {
            if let Err(err) = self.rebuild_framework_keymap_from_disk() {
                self.show_timed_toast("Keymap reload failed", err);
                return;
            }
            self.dismiss_keymap_diagnostics();
        } else {
            self.show_keymap_diagnostics(&result.errors);
        }

        if !result.missing_actions.is_empty() {
            keymap_ui::save_current_keymap_to_disk(self);
            self.show_timed_toast(
                "Keymap updated",
                format!(
                    "Defaults written for missing entries:\n{}",
                    result.missing_actions.join(", ")
                ),
            );
        }
    }

    pub(super) fn show_keymap_diagnostics(&mut self, errors: &[KeymapError]) {
        // Dismiss previous diagnostics toast if any.
        self.dismiss_keymap_diagnostics();

        let body = errors
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        let action_path = self
            .keymap
            .path()
            .map(|p| AbsolutePath::from(p.to_path_buf()));

        let id = self.framework.toasts.push_persistent(
            "Keymap errors (using defaults)",
            body,
            Error,
            action_path.map(CargoPortToastAction::from),
            1,
        );
        self.keymap.set_diagnostics_id(Some(id));
        let toast_len = self.framework.toasts.active_now().len();
        self.framework.toasts.viewport.set_len(toast_len);
    }
    pub(super) fn dismiss_keymap_diagnostics(&mut self) {
        if let Some(id) = self.keymap.take_diagnostics_id() {
            self.framework.toasts.dismiss(id);
        }
    }
    pub fn maybe_reload_config_from_disk(&mut self) {
        let Some(path) = self.config.take_stamp_change() else {
            return;
        };
        let path = path.to_path_buf();
        let path_buf = path.display().to_string();
        let previous_table = self.framework.settings_store().table().clone();
        let reload_result = self.framework.settings_store_mut().load_from_path(path);
        match reload_result {
            Ok(settings) => {
                match CargoPortConfig::from_table(self.framework.settings_store().table()) {
                    Ok(config) => {
                        self.framework.set_toast_settings(settings.toast_settings);
                        self.apply_config(&config);
                        self.config.sync_stamp();
                        self.show_timed_toast("Settings", "Reloaded from disk");
                    },
                    Err(err) => {
                        self.framework
                            .settings_store_mut()
                            .replace_table(previous_table);
                        self.record_config_reload_failure(&format!("{path_buf}: {err}"));
                    },
                }
            },
            Err(err) => self.record_config_reload_failure(&format!("{path_buf}: {err}")),
        }
    }
    pub fn apply_config(&mut self, cfg: &CargoPortConfig) {
        if self.config.current() == cfg {
            return;
        }

        let prev_force = self.config.current().debug.force_github_rate_limit;
        let next_force = cfg.debug.force_github_rate_limit;

        let actions = integration::collect_reload_actions(
            self.config.current(),
            cfg,
            ReloadContext {
                scan_complete:       self.scan.is_complete(),
                has_cached_non_rust: self.project_list.has_cached_non_rust_projects(),
            },
        );
        config::set_active_config(cfg);
        *self.config.current_mut() = cfg.clone();
        if !self.config.discovery_shimmer_enabled() {
            self.scan.discovery_shimmers_mut().clear();
        }

        if prev_force != next_force {
            self.net.set_force_github_rate_limit(next_force);
            // Synthesize a signal so the UI reflects the flag flip
            // immediately instead of waiting for the next natural
            // GitHub request — otherwise toggling the flag would look
            // broken until the next refresh. The force flag simulates a
            // rate-limit (not a network outage), so emit `RateLimited`.
            if next_force {
                self.apply_service_signal(ServiceSignal::RateLimited(ServiceKind::GitHub));
            } else {
                self.mark_service_recovered(ServiceKind::GitHub);
            }
        }
        if actions.refresh_cpu.should_apply() {
            self.reset_cpu_placeholder();
        }

        if actions.refresh_lint_runtime.should_apply() {
            self.refresh_lint_runtime_from_config(cfg);
        }

        match actions.tree {
            TreeReaction::FullRescan => {
                self.rescan();
                self.force_settings_if_unconfigured();
            },
            TreeReaction::RegroupMembers => {
                if actions.refresh_lint_runtime.should_apply() {
                    self.respawn_watcher_and_register_existing_projects();
                }
                self.project_list
                    .regroup_members(&self.config.current().tui.inline_dirs);
                self.scan.bump_generation();
            },
            TreeReaction::None => {
                if actions.refresh_lint_runtime.should_apply() {
                    self.respawn_watcher_and_register_existing_projects();
                }
            },
        }
    }
    /// Apply a lint configuration change. Cross-subsystem
    /// orchestration — not a method on any single subsystem because
    /// lint config changes fan out across three areas:
    ///
    /// - **Inflight**: respawns the lint runtime, clears in-flight lint paths, refreshes the lint
    ///   toast, syncs the running project list against the new runtime.
    /// - **Scan**: clears in-memory lint state on `projects`, refreshes lint runs from disk, bumps
    ///   `data_generation` so detail panes redraw.
    /// - **Selection**: recomputes `cached_fit_widths` because the project pane's column schema
    ///   depends on whether lints are enabled.
    ///
    /// Called from the per-tick config-reload handler (today via the
    /// `refresh_lint_runtime_from_config` shim below). New side-
    /// effects of a lint-config change MUST be added here (or in
    /// the relevant subsystem method this function calls), not in
    /// random callers — that's the point of having a single
    /// orchestrator. See "Recurring patterns" in
    /// `src/tui/app/mod.rs` for the cross-subsystem orchestrator
    /// pattern.
    pub fn apply_lint_config_change(&mut self, cfg: &CargoPortConfig) {
        // Inflight: respawn the lint runtime + clear in-flight tracking.
        let lint_spawn = lint::spawn(cfg, self.background.bg_sender());
        self.lint.set_runtime(lint_spawn.handle);
        self.lint.running_mut().clear();
        self.sync_running_lint_toast();
        self.sync_lint_runtime_projects();

        // Scan state on App: clear lint state, refresh from
        // disk, bump generation.
        self.clear_all_lint_state();
        self.refresh_lint_runs_from_disk();
        self.scan.bump_generation();

        // Recompute fit widths — column schema differs with lint
        // enabled / disabled.
        self.project_list
            .reset_fit_widths(self.config.lint_enabled());

        if let Some(warning) = lint_spawn.warning {
            self.overlays
                .set_status_flash(warning.clone(), Instant::now());
            self.show_timed_toast("Lint runtime", warning);
        }
    }
    /// Backwards-compatible shim. Existing callers (rescan, config
    /// reload) still call `refresh_lint_runtime_from_config`; the
    /// real orchestration lives in [`Self::apply_lint_config_change`].
    pub(super) fn refresh_lint_runtime_from_config(&mut self, cfg: &CargoPortConfig) {
        self.apply_lint_config_change(cfg);
    }
}
