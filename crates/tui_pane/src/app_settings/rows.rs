//! [`SettingsRows`]: the overlay's rows as the app composes them, with
//! what each selectable row edits and the width the widest one needs.

use std::path::PathBuf;

use super::constants::APPEARANCE_SECTION;
use super::constants::CONFIG_LABEL;
use super::constants::CURSOR_WIDTH;
use super::constants::DARK_THEME_LABEL;
use super::constants::FILES_SECTION;
use super::constants::INITIAL_ROWS_LABEL;
use super::constants::KEYMAP_LABEL;
use super::constants::LABEL_VALUE_GAP;
use super::constants::LIGHT_THEME_LABEL;
use super::constants::MODE_LABEL;
use super::constants::NOTICES_SECTION;
use super::constants::STEPPER_DECORATION_WIDTH;
use super::constants::THEMES_LABEL;
use super::constants::UNRESOLVED_PATH;
use super::step::FrameworkSetting;
use crate::AppIdentity;
use crate::AppearanceConfig;
use crate::InitialRows;
use crate::SECTION_ITEM_INDENT;
use crate::SettingsRow;

/// What the pane's nth selectable row edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingTarget<S> {
    /// A setting the framework owns and steps itself.
    Framework(FrameworkSetting),
    /// One of the app's own settings, which the app steps.
    App(S),
    /// A reported value with nothing to change.
    ReadOnly,
}

/// The settings overlay's rows as the app composes them, plus the
/// setting each selectable row edits and the width the widest row
/// needs.
///
/// Rows appear in the order the builders are called. Selectable rows
/// are numbered in that order too, which is the numbering the settings
/// pane selects by, so [`Self::target`] answers for the pane's
/// selection directly.
#[derive(Debug)]
pub struct SettingsRows<S> {
    /// Rows to hand to [`crate::SettingsPane::render_rows`].
    rows:    Vec<SettingsRow>,
    /// `targets[selection]` is what the pane's nth selectable row edits.
    targets: Vec<SettingTarget<S>>,
    /// Widest label and value seen so far.
    widths:  RowWidths,
}

/// Widest label and widest value seen while building the rows.
#[derive(Debug, Default)]
struct RowWidths {
    /// Widest label in cells.
    label: usize,
    /// Widest value in cells, stepper decoration included.
    value: usize,
}

impl RowWidths {
    /// Fold one row's label and value in.
    fn observe(&mut self, label: &str, value: &str, decoration: usize) {
        self.label = self.label.max(label.chars().count());
        self.value = self.value.max(value.chars().count() + decoration);
    }

    /// Cells the widest row needs once every label is padded to match,
    /// laid out the way [`crate::SettingsPane::render_rows`] lays rows
    /// out: indent, selection cursor, labels padded to the widest
    /// label, separator, then the value with any stepper decoration.
    fn widest_row(&self) -> usize {
        SECTION_ITEM_INDENT.chars().count()
            + CURSOR_WIDTH
            + self.label
            + LABEL_VALUE_GAP
            + self.value
    }
}

impl<S: Copy> Default for SettingsRows<S> {
    fn default() -> Self { Self::new() }
}

impl<S: Copy> SettingsRows<S> {
    /// No rows yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rows:    Vec::new(),
            targets: Vec::new(),
            widths:  RowWidths::default(),
        }
    }

    /// Push a section header.
    pub fn section(&mut self, label: &str) { self.rows.push(SettingsRow::section(label)); }

    /// Push a stepper row for one of the app's own settings.
    pub fn stepper(&mut self, setting: S, label: &str, value: &str) {
        self.push_stepper(SettingTarget::App(setting), label, value);
    }

    /// Push a reported row that nothing edits.
    pub fn value(&mut self, label: &str, value: String) {
        self.widths.observe(label, &value, 0);
        self.rows
            .push(SettingsRow::value(self.targets.len(), label, value));
        self.targets.push(SettingTarget::ReadOnly);
    }

    /// Push the Appearance section: the `mode`, `light theme` and
    /// `dark theme` steppers.
    pub fn appearance<I: AppIdentity>(&mut self, appearance: &AppearanceConfig<I>) {
        self.section(APPEARANCE_SECTION);
        self.push_stepper(
            SettingTarget::Framework(FrameworkSetting::Mode),
            MODE_LABEL,
            &appearance.mode,
        );
        self.push_stepper(
            SettingTarget::Framework(FrameworkSetting::LightTheme),
            LIGHT_THEME_LABEL,
            &appearance.light_theme,
        );
        self.push_stepper(
            SettingTarget::Framework(FrameworkSetting::DarkTheme),
            DARK_THEME_LABEL,
            &appearance.dark_theme,
        );
    }

    /// Push the `initial rows` stepper. No section: the app places it
    /// under a section of its own.
    pub fn initial_rows(&mut self, rows: InitialRows) {
        self.push_stepper(
            SettingTarget::Framework(FrameworkSetting::InitialRows),
            INITIAL_ROWS_LABEL,
            &rows.get().to_string(),
        );
    }

    /// Push the Files section: where `config.toml`, the `themes/`
    /// directory and `keymap.toml` live.
    pub fn files<I: AppIdentity>(&mut self) {
        self.section(FILES_SECTION);
        self.value(CONFIG_LABEL, display_path(I::config_path()));
        self.value(THEMES_LABEL, display_path(I::themes_dir()));
        self.value(KEYMAP_LABEL, display_path(I::keymap_path()));
    }

    /// Push the Notices section: one read-only row per `(label, text)`
    /// pair, in the order given, so outstanding startup notices stay
    /// visible after their toasts are gone. Pushes nothing when
    /// `notices` is empty.
    pub fn notices(&mut self, notices: &[(&str, &str)]) {
        if notices.is_empty() {
            return;
        }
        self.section(NOTICES_SECTION);
        for (label, text) in notices {
            self.value(label, (*text).to_string());
        }
    }

    /// Every row, in order, for [`crate::SettingsPane::render_rows`].
    #[must_use]
    pub fn rows(&self) -> &[SettingsRow] { &self.rows }

    /// What the pane's `selection`th selectable row edits; `None` past
    /// the last one.
    #[must_use]
    pub fn target(&self, selection: usize) -> Option<SettingTarget<S>> {
        self.targets.get(selection).copied()
    }

    /// Cells the widest row needs; see [`RowWidths::widest_row`].
    pub(super) fn widest_row(&self) -> usize { self.widths.widest_row() }

    /// Push a stepper row and record what it edits.
    fn push_stepper(&mut self, target: SettingTarget<S>, label: &str, value: &str) {
        self.widths.observe(label, value, STEPPER_DECORATION_WIDTH);
        self.rows
            .push(SettingsRow::stepper(self.targets.len(), label, value));
        self.targets.push(target);
    }
}

/// Render a resolved path, or the placeholder for a platform where the
/// OS config directory is unavailable.
fn display_path(path: Option<PathBuf>) -> String {
    path.map_or_else(
        || UNRESOLVED_PATH.to_string(),
        |path| path.display().to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::SettingTarget;
    use super::SettingsRows;
    use crate::SettingsRowIdentity;
    use crate::app_settings::FrameworkSetting;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TestSetting {
        Speed,
    }

    #[test]
    fn no_notices_push_nothing() {
        let mut rows = SettingsRows::<TestSetting>::new();
        rows.notices(&[]);
        assert!(rows.rows().is_empty());
        assert_eq!(rows.widest_row(), super::RowWidths::default().widest_row());
    }

    #[test]
    fn notices_follow_their_header_in_order() {
        let mut rows = SettingsRows::<TestSetting>::new();
        rows.notices(&[("theme", "one"), ("config", "two")]);
        let labels: Vec<_> = rows.rows().iter().map(|row| row.label.as_str()).collect();
        assert_eq!(labels, ["Notices", "theme", "config"]);
    }

    #[test]
    fn each_selectable_payload_indexes_its_own_target() {
        let mut rows = SettingsRows::new();
        rows.section("Top");
        rows.initial_rows(crate::InitialRows::default());
        rows.stepper(TestSetting::Speed, "speed", "3");
        rows.section("Info");
        rows.value("where", "here".to_string());
        let targets: Vec<_> = rows
            .rows()
            .iter()
            .filter_map(|row| match row.identity {
                SettingsRowIdentity::Selectable(payload) => rows.target(payload.get()),
                SettingsRowIdentity::Decoration => None,
            })
            .collect();
        assert_eq!(
            targets,
            [
                SettingTarget::Framework(FrameworkSetting::InitialRows),
                SettingTarget::App(TestSetting::Speed),
                SettingTarget::ReadOnly,
            ]
        );
        assert_eq!(rows.target(3), None);
    }
}
