use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Instant;

use ratatui::style::Color;
use tui_pane::Appearance;
use tui_pane::ToastStyle::Error;
use tui_pane::ToastStyle::Warning;

use crate::config;
use crate::config::CargoPortConfig;
use crate::http::ServiceKind;
use crate::http::ServiceSignal;
use crate::lint;
use crate::project::AbsolutePath;
use crate::tui::app::App;
use crate::tui::app::CargoPortToastAction;
use crate::tui::integration;
use crate::tui::integration::ReloadContext;
use crate::tui::integration::TreeReaction;
use crate::tui::keymap;
use crate::tui::keymap::KeymapError;
use crate::tui::keymap::KeymapErrorReason;
use crate::tui::keymap::KeymapErrorReason::Parse;
use crate::tui::keymap_ui;
use crate::tui::theme_roles;

/// The backdrop color to paint when the resolved theme appearance
/// disagrees with the terminal's detected background. Returns `None`
/// when they match, or when the terminal background is unknown (the OSC
/// 11 probe failed) — both cases leave the terminal showing through.
fn background_when_mismatched(
    terminal: Option<Appearance>,
    resolved: Appearance,
    background: Color,
) -> Option<Color> {
    terminal
        .filter(|appearance| *appearance != resolved)
        .map(|_| background)
}

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
        // The framework keymap (built in `ignore_unknown_entries` mode)
        // is the authoritative record of unknown actions/scopes across
        // every scope, so surface those. Reaching here means the
        // framework build succeeded, so the only legacy errors possible
        // are `UnknownAction`s already covered by the framework
        // warnings; keep any other legacy diagnostics in case the two
        // loaders disagree.
        let diagnostics: Vec<String> = result
            .errors
            .iter()
            .filter(|err| !matches!(err.reason, KeymapErrorReason::UnknownAction))
            .map(ToString::to_string)
            .collect();
        let mut warnings = result.warnings;
        warnings.extend(self.framework_keymap.unknown_warnings().iter().cloned());
        if diagnostics.is_empty() {
            self.dismiss_keymap_diagnostics();
        } else {
            self.show_keymap_diagnostics(&diagnostics);
        }
        if warnings.is_empty() {
            self.dismiss_keymap_warnings();
        } else {
            self.show_keymap_warnings(&warnings);
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
                }
                .to_string()]);
                self.dismiss_keymap_warnings();
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
            let messages: Vec<String> = result.errors.iter().map(ToString::to_string).collect();
            self.show_keymap_diagnostics(&messages);
        }
        let mut warnings = result.warnings;
        if !result.missing_actions.is_empty() {
            warnings.push(format!(
                "Missing keymap entries are using defaults until added:\n{}",
                result.missing_actions.join(", ")
            ));
        }
        if warnings.is_empty() {
            self.dismiss_keymap_warnings();
        } else {
            self.show_keymap_warnings(&warnings);
        }
    }

    pub(super) fn show_keymap_diagnostics(&mut self, messages: &[String]) {
        // Dismiss previous diagnostics toast if any.
        self.dismiss_keymap_diagnostics();

        let body = messages.join("\n");
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
    }
    pub(super) fn dismiss_keymap_diagnostics(&mut self) {
        if let Some(id) = self.keymap.take_diagnostics_id() {
            self.framework.toasts.dismiss(id);
        }
    }
    pub(super) fn show_keymap_warnings(&mut self, messages: &[String]) {
        self.dismiss_keymap_warnings();

        let body = messages.join("\n");
        let action_path = self
            .keymap
            .path()
            .map(|p| AbsolutePath::from(p.to_path_buf()));

        let id = self.framework.toasts.push_persistent(
            "Keymap warnings",
            body,
            Warning,
            action_path.map(CargoPortToastAction::from),
            1,
        );
        self.keymap.set_warnings_id(Some(id));
    }
    pub(super) fn dismiss_keymap_warnings(&mut self) {
        if let Some(id) = self.keymap.take_warnings_id() {
            self.framework.toasts.dismiss(id);
        }
    }
    /// Per-tick check for changes under the user themes directory.
    /// On a detected change, re-scan, build a fresh registry, swap
    /// it into `tui_pane`'s `THEME_STATE`, and surface a summary
    /// toast. Persistent parse-error toasts are dismissed when the
    /// next reload succeeds with zero failures, matching the keymap
    /// diagnostics flow.
    pub fn maybe_reload_themes_from_disk(&mut self) {
        if self.themes.take_change().is_none() {
            return;
        }
        let mut registry = tui_pane::ThemeRegistry::from_dir_with_builtins(self.themes.dir());
        theme_roles::apply_role_defaults_to_registry(&mut registry);
        let failed = registry.status().failed_files.clone();
        let overridden = registry.status().overridden.clone();
        let total = registry.len();
        tui_pane::replace_registry(registry);

        if let Some(id) = self.themes.take_diagnostics_id() {
            self.framework.toasts.dismiss(id);
        }

        if failed.is_empty() {
            let mut body = format!("{total} variants registered");
            if !overridden.is_empty() {
                let names = overridden
                    .iter()
                    .map(|id| id.as_str().to_owned())
                    .collect::<Vec<_>>()
                    .join(", ");
                let _ = write!(body, " ({names} overridden)");
            }
            self.show_timed_toast("Themes reloaded", body);
        } else {
            let body = failed
                .iter()
                .map(|(path, err)| format!("{}: {}", path.display(), err))
                .collect::<Vec<_>>()
                .join("\n");
            let id =
                self.framework
                    .toasts
                    .push_persistent("Themes reload errors", body, Error, None, 1);
            self.themes.set_diagnostics_id(Some(id));
        }
    }
    /// Resolve the active theme from the current config + cached OS
    /// appearance and publish it via [`tui_pane::set_active_theme`].
    ///
    /// Called from two places:
    /// 1. `apply_config` when the `[appearance]` section changed.
    /// 2. The [`crate::scan::BackgroundMsg::AppearanceChanged`] handler when the OS appearance
    ///    flips (Phase 5).
    ///
    /// On miss (configured id absent from the registry), surfaces a
    /// persistent "Theme not found" toast and stashes its id on
    /// `themes.miss_toast_id` so the next clean resolve dismisses it.
    /// An invalid `mode` string surfaces a timed toast separately.
    pub(super) fn resolve_and_apply_active_theme(&mut self) {
        let registry = tui_pane::registry();
        let appearance_cfg = &self.config.current().appearance;
        let resolved = registry.resolve_active(
            &appearance_cfg.mode,
            &appearance_cfg.light_theme,
            &appearance_cfg.dark_theme,
            self.themes.os_appearance(),
        );
        // When the resolved theme's appearance disagrees with the
        // terminal's actual background (e.g. a forced dark theme on a
        // light terminal), paint the theme's base background so the text
        // stays readable; otherwise leave the terminal showing through.
        let frame_background = background_when_mismatched(
            self.themes.terminal_appearance(),
            resolved.appearance,
            resolved.theme.text.bg_focus.color,
        );
        self.themes.set_frame_background(frame_background);
        let mut active_theme = (*resolved.theme).clone();
        theme_roles::apply_role_defaults_to_theme(&mut active_theme, None, resolved.appearance);
        tui_pane::set_active_theme(Arc::new(active_theme));
        tui_pane::set_focused_pane_tint(self.config.current().appearance.focused_pane_tint);

        // Dismiss the prior miss toast unconditionally; we'll push a
        // fresh one below if this resolve also missed.
        if let Some(id) = self.themes.take_miss_toast_id() {
            self.framework.toasts.dismiss(id);
        }
        if let Some(miss) = resolved.miss {
            let id = self.framework.toasts.push_persistent(
                "Theme not found",
                format!("{miss} (using built-in fallback)"),
                Error,
                None,
                1,
            );
            self.themes.set_miss_toast_id(Some(id));
        }
        if let Some(err) = resolved.mode_error {
            self.show_timed_toast("Appearance mode", err);
        }
    }

    /// Record the terminal's detected background appearance and re-resolve
    /// the active theme so the backdrop decision reflects it. Called once
    /// at startup after the OSC 11 probe, before the input thread starts.
    pub fn set_terminal_appearance(&mut self, appearance: Option<Appearance>) {
        self.themes.set_terminal_appearance(appearance);
        self.resolve_and_apply_active_theme();
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
    pub fn apply_config(&mut self, cargo_port_config: &CargoPortConfig) {
        if self.config.current() == cargo_port_config {
            return;
        }

        let appearance_changed = self.config.current().appearance != cargo_port_config.appearance;
        let prev_force = self.config.current().debug.force_github_rate_limit;
        let next_force = cargo_port_config.debug.force_github_rate_limit;

        let actions = integration::collect_reload_actions(
            self.config.current(),
            cargo_port_config,
            ReloadContext {
                scan_complete:       self.scan.is_complete(),
                has_cached_non_rust: self.project_list.has_cached_non_rust_projects(),
            },
        );
        config::set_active_config(cargo_port_config);
        *self.config.current_mut() = cargo_port_config.clone();
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
            self.refresh_lint_runtime_from_config(cargo_port_config);
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

        if appearance_changed {
            self.resolve_and_apply_active_theme();
        }
    }
    /// Apply a lint configuration change. Cross-subsystem
    /// orchestration — not a method on any single subsystem because
    /// lint config changes fan out across three areas:
    ///
    /// - **Runtime**: respawns the lint runtime and syncs registered projects.
    /// - **Toast**: reconciles the running-lint toast from cleared project lint state.
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
    pub fn apply_lint_config_change(&mut self, cargo_port_config: &CargoPortConfig) {
        // Runtime: respawn the lint runtime.
        let lint_spawn = lint::spawn(cargo_port_config, self.background.background_sender());
        self.lint.set_runtime(lint_spawn.handle);
        self.sync_lint_runtime_projects();

        // Scan state on App: clear lint state, refresh from
        // disk, bump generation.
        self.clear_all_lint_state();
        self.sync_running_lint_toast();
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
    pub(super) fn refresh_lint_runtime_from_config(&mut self, cargo_port_config: &CargoPortConfig) {
        self.apply_lint_config_change(cargo_port_config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_backdrop_only_when_terminal_disagrees() {
        // Theme matches the terminal → leave it transparent.
        assert_eq!(
            background_when_mismatched(Some(Appearance::Dark), Appearance::Dark, Color::Black),
            None
        );
        // Forced dark theme on a light terminal → paint the dark backdrop.
        assert_eq!(
            background_when_mismatched(Some(Appearance::Light), Appearance::Dark, Color::Black),
            Some(Color::Black)
        );
        // Terminal background unknown (probe failed) → never paint.
        assert_eq!(
            background_when_mismatched(None, Appearance::Dark, Color::Black),
            None
        );
    }
}
