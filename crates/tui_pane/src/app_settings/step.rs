//! Stepping a settings row: which way it moves, the value it lands on,
//! and writing the edited config back.

use serde::Serialize;
use serde::de::DeserializeOwned;

use super::constants::APPEARANCE_MODES;
use crate::AppIdentity;
use crate::Appearance;
use crate::AppearanceConfig;
use crate::InitialRows;
use crate::LoadedConfig;
use crate::TileFill;
use crate::TileView;
use crate::app_config;
use crate::registry;
use crate::set_active_theme;
use crate::set_transparent_background;

/// Which way a stepper row moves.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingStep {
    /// Toward the previous value, wrapping at the start.
    Prev,
    /// Toward the next value, wrapping at the end.
    Next,
}

/// A settings row the framework owns and steps itself.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameworkSetting {
    /// `appearance.mode`: `auto`, `light`, `dark`.
    Mode,
    /// `appearance.light_theme`: the registry's light variants.
    LightTheme,
    /// `appearance.dark_theme`: the registry's dark variants.
    DarkTheme,
    /// `tiles.initial_rows`: one through eight.
    InitialRows,
    /// `tiles.fill`: `add_new`, `redistribute`.
    TileFill,
    /// `tiles.view`: `auto`, `summary`, `cells`.
    TileView,
    /// `tiles.widen_summary`: `true`, `false`.
    WidenSummary,
    /// `appearance.transparent`: `true`, `false`.
    Transparent,
}

/// Reach into an app's config for the keys the framework owns.
///
/// Implemented on the app's own serde config struct, which keeps
/// declaring its tables in the order its `config.toml` spells them.
pub trait AppConfig: Default + DeserializeOwned + Serialize {
    /// The app whose defaults and file paths the config answers to.
    type Identity: AppIdentity;

    /// The `[appearance]` table.
    fn appearance(&self) -> &AppearanceConfig<Self::Identity>;

    /// The `[appearance]` table, for the steppers to edit.
    fn appearance_mut(&mut self) -> &mut AppearanceConfig<Self::Identity>;

    /// `tiles.initial_rows`, for its stepper to edit.
    fn initial_rows_mut(&mut self) -> &mut InitialRows;

    /// `tiles.fill`, for its stepper to edit.
    fn tile_fill_mut(&mut self) -> &mut TileFill;

    /// `tiles.view`, for its stepper to edit.
    fn tile_view_mut(&mut self) -> &mut TileView;

    /// `tiles.widen_summary`, for its stepper to edit.
    fn widen_summary_mut(&mut self) -> &mut bool;
}

/// The value one step from `current`, wrapping at both ends.
///
/// A `current` that is not in `values` steps to the first entry, which
/// is how a hand-edited `config.toml` with an unknown id recovers. An
/// empty `values` leaves `current` as it is.
#[must_use]
pub fn stepped(values: &[String], current: &str, step: SettingStep) -> String {
    let Some(first) = values.first() else {
        return current.to_string();
    };
    let Some(index) = values.iter().position(|value| value == current) else {
        return first.clone();
    };
    let len = values.len();
    let next = match step {
        SettingStep::Prev => (index + len - 1) % len,
        SettingStep::Next => (index + 1) % len,
    };
    values.get(next).unwrap_or(first).clone()
}

/// Step a framework-owned setting one value, then re-resolve the theme
/// and save through [`apply_settings`].
pub fn step_framework_setting<C: AppConfig>(
    setting: FrameworkSetting,
    step: SettingStep,
    loaded: &mut LoadedConfig<C>,
    theme_note: &mut Option<String>,
) {
    let config = &mut loaded.config;
    match setting {
        FrameworkSetting::Mode => {
            let modes: Vec<String> = APPEARANCE_MODES
                .iter()
                .map(|mode| (*mode).to_string())
                .collect();
            let appearance = config.appearance_mut();
            appearance.mode = stepped(&modes, &appearance.mode, step);
        },
        FrameworkSetting::LightTheme => {
            let ids = theme_ids(Appearance::Light);
            let appearance = config.appearance_mut();
            appearance.light_theme = stepped(&ids, &appearance.light_theme, step);
        },
        FrameworkSetting::DarkTheme => {
            let ids = theme_ids(Appearance::Dark);
            let appearance = config.appearance_mut();
            appearance.dark_theme = stepped(&ids, &appearance.dark_theme, step);
        },
        FrameworkSetting::InitialRows => config.initial_rows_mut().step(step),
        FrameworkSetting::TileFill => config.tile_fill_mut().step(step),
        FrameworkSetting::TileView => config.tile_view_mut().step(step),
        // Two values, so a step either way lands on the other one.
        FrameworkSetting::WidenSummary => {
            let widen = config.widen_summary_mut();
            *widen = !*widen;
        },
        FrameworkSetting::Transparent => {
            let appearance = config.appearance_mut();
            appearance.transparent = !appearance.transparent;
        },
    }
    apply_settings(loaded, theme_note);
}

/// Re-resolve the active theme from the edited config and the remembered
/// system appearance, then write the file.
///
/// A configured theme id that matched nothing leaves its note in
/// `theme_note` (cleared when every id resolves); a failed write
/// leaves its text in [`LoadedConfig::error`]. Both show in the
/// overlay's Notices.
pub fn apply_settings<C: AppConfig>(loaded: &mut LoadedConfig<C>, theme_note: &mut Option<String>) {
    let (theme, note) = app_config::resolve_appearance(&registry(), loaded.config.appearance());
    *theme_note = note;
    set_active_theme(theme);
    set_transparent_background(loaded.config.appearance().transparent);
    loaded.save::<C::Identity>();
}

/// Theme ids registered for one appearance, in registry order.
fn theme_ids(appearance: Appearance) -> Vec<String> {
    registry()
        .variants_by_appearance(appearance)
        .map(|variant| variant.id.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde::Deserialize;
    use serde::Serialize;

    use super::*;
    use crate::ThemeId;
    use crate::ThemeRegistry;
    use crate::ThemeState;
    use crate::ThemeVariant;
    use crate::apply_system_appearance;
    use crate::fallback_theme;
    use crate::install_theme_state;
    use crate::theme;

    struct TestApp;

    impl AppIdentity for TestApp {
        const BINARY_NAME: &'static str = "test-app";
        const CONFIG_DIRNAME: &'static str = "test-app";
        const DEFAULT_LIGHT_THEME: &'static str = "App Light";
        const DEFAULT_DARK_THEME: &'static str = "App Dark";

        fn config_path() -> Option<PathBuf> { None }
    }

    #[derive(Default, Deserialize, Serialize)]
    struct TestConfig {
        appearance:    AppearanceConfig<TestApp>,
        initial_rows:  InitialRows,
        tile_fill:     TileFill,
        tile_view:     TileView,
        widen_summary: bool,
    }

    impl AppConfig for TestConfig {
        type Identity = TestApp;

        fn appearance(&self) -> &AppearanceConfig<Self::Identity> { &self.appearance }

        fn appearance_mut(&mut self) -> &mut AppearanceConfig<Self::Identity> {
            &mut self.appearance
        }

        fn initial_rows_mut(&mut self) -> &mut InitialRows { &mut self.initial_rows }

        fn tile_fill_mut(&mut self) -> &mut TileFill { &mut self.tile_fill }

        fn tile_view_mut(&mut self) -> &mut TileView { &mut self.tile_view }

        fn widen_summary_mut(&mut self) -> &mut bool { &mut self.widen_summary }
    }

    fn variant(id: &str, appearance: Appearance) -> ThemeVariant {
        ThemeVariant {
            id: ThemeId::new(id),
            appearance,
            theme: fallback_theme(appearance),
        }
    }

    fn install_test_state() {
        // Nextest gives each test its own process, so this process-wide state
        // belongs only to the caller.
        install_theme_state(ThemeState::with_registry(
            ThemeRegistry::new_with_builtins(vec![
                variant("App Dark", Appearance::Dark),
                variant("App Light", Appearance::Light),
            ]),
            fallback_theme(Appearance::Dark),
        ));
    }

    fn loaded_config() -> LoadedConfig<TestConfig> {
        LoadedConfig {
            config: TestConfig::default(),
            error:  None,
        }
    }

    fn values() -> Vec<String> { ["a", "b", "c"].map(String::from).to_vec() }

    #[test]
    fn stepping_wraps_at_both_ends() {
        assert_eq!(stepped(&values(), "c", SettingStep::Next), "a");
        assert_eq!(stepped(&values(), "a", SettingStep::Prev), "c");
        assert_eq!(stepped(&values(), "a", SettingStep::Next), "b");
    }

    #[test]
    fn an_unknown_value_steps_to_the_first_entry() {
        assert_eq!(stepped(&values(), "z", SettingStep::Prev), "a");
        assert_eq!(stepped(&values(), "z", SettingStep::Next), "a");
    }

    #[test]
    fn no_values_leaves_the_current_one() {
        assert_eq!(stepped(&[], "z", SettingStep::Next), "z");
    }

    #[test]
    fn the_view_row_steps_through_auto_summary_and_cells() {
        install_test_state();
        let mut loaded = loaded_config();
        let mut notice = None;

        step_framework_setting(
            FrameworkSetting::TileView,
            SettingStep::Next,
            &mut loaded,
            &mut notice,
        );
        assert_eq!(loaded.config.tile_view, TileView::Summary);
        step_framework_setting(
            FrameworkSetting::TileView,
            SettingStep::Next,
            &mut loaded,
            &mut notice,
        );
        assert_eq!(loaded.config.tile_view, TileView::Cells);
        step_framework_setting(
            FrameworkSetting::TileView,
            SettingStep::Next,
            &mut loaded,
            &mut notice,
        );
        assert_eq!(loaded.config.tile_view, TileView::Auto);
    }

    #[test]
    fn applying_settings_uses_the_remembered_system_appearance() {
        install_test_state();
        let mut loaded = loaded_config();
        let mut notice = None;

        assert!(apply_system_appearance(Appearance::Light, &loaded.config.appearance).is_none());
        apply_settings(&mut loaded, &mut notice);
        assert_eq!(*theme(), fallback_theme(Appearance::Light));

        assert!(apply_system_appearance(Appearance::Dark, &loaded.config.appearance).is_none());
        apply_settings(&mut loaded, &mut notice);
        assert_eq!(*theme(), fallback_theme(Appearance::Dark));

        loaded.config.appearance.mode = "light".to_string();
        apply_settings(&mut loaded, &mut notice);
        assert_eq!(*theme(), fallback_theme(Appearance::Light));
    }

    #[test]
    fn applying_settings_defaults_auto_to_dark_without_an_observation() {
        install_test_state();
        let mut loaded = loaded_config();
        let mut notice = None;

        apply_settings(&mut loaded, &mut notice);

        assert_eq!(*theme(), fallback_theme(Appearance::Dark));
    }
}
