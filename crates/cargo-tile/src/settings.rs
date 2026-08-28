//! Rows rendered in the framework settings overlay, and the cycling
//! that edits them.
//!
//! The three `[appearance]` rows are steppers: Left/Right/Enter walk
//! them through their allowed values, write `config.toml`, and swap the
//! active theme in place. Every other row reports state and is inert —
//! nothing here opens a text editor, so the overlay never has a mode the
//! user has to type their way out of.

use std::path::PathBuf;

use tui_pane::Appearance;
use tui_pane::SECTION_ITEM_INDENT;
use tui_pane::SettingsRow;

use crate::app::App;
use crate::config;
use crate::constants::APPEARANCE_MODES;
use crate::constants::CURSOR_WIDTH;
use crate::constants::EMPTY_LIST;
use crate::constants::LABEL_VALUE_GAP;
use crate::constants::LIST_SEPARATOR;
use crate::constants::MAX_FADE_SECONDS;
use crate::constants::MAX_INITIAL_ROWS;
use crate::constants::MIN_FADE_SECONDS;
use crate::constants::MIN_INITIAL_ROWS;
use crate::constants::STEPPER_DECORATION_WIDTH;
use crate::constants::UNRESOLVED_PATH;

/// Which setting a selected row edits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingId {
    /// `appearance.mode` — cycles through [`APPEARANCE_MODES`].
    Mode,
    /// `appearance.light_theme` — cycles the registry's light variants.
    LightTheme,
    /// `appearance.dark_theme` — cycles the registry's dark variants.
    DarkTheme,
    /// `tiles.initial_rows` — cycles one through [`MAX_INITIAL_ROWS`].
    InitialRows,
    /// `tiles.fade_seconds` — cycles zero through [`MAX_FADE_SECONDS`].
    FadeSeconds,
    /// A reported value with nothing to change.
    ReadOnly,
}

/// Which way a cycling row steps.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Step {
    /// Toward the previous value, wrapping at the start.
    Prev,
    /// Toward the next value, wrapping at the end.
    Next,
}

/// The overlay's rows plus the setting each selectable row edits,
/// indexed the same way the settings pane indexes its selection.
pub(crate) struct SettingsRows {
    /// Rows to hand to [`tui_pane::SettingsPane::render_rows`].
    pub(crate) rows:       Vec<SettingsRow>,
    /// Cells the widest row needs, laid out the way
    /// [`tui_pane::SettingsPane::render_rows`] lays rows out: indent,
    /// selection cursor, labels padded to the widest label, separator,
    /// then the value with any stepper decoration.
    pub(crate) widest_row: usize,
    /// `ids[selection]` is the setting the pane's nth selectable row
    /// edits.
    ids:                   Vec<SettingId>,
}

/// Widest label and widest value seen while building the rows.
#[derive(Default)]
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

    /// Cells the widest row needs once every label is padded to match.
    fn widest_row(&self) -> usize {
        SECTION_ITEM_INDENT.chars().count()
            + CURSOR_WIDTH
            + self.label
            + LABEL_VALUE_GAP
            + self.value
    }
}

/// Build the settings rows for the current frame.
pub(crate) fn rows(app: &App) -> SettingsRows {
    let appearance = &app.loaded_config.config.appearance;
    let mut out = SettingsRows {
        rows:       Vec::new(),
        widest_row: 0,
        ids:        Vec::new(),
    };
    let mut widths = RowWidths::default();

    out.rows.push(SettingsRow::section("Appearance"));
    push_stepper(
        &mut out,
        &mut widths,
        SettingId::Mode,
        "mode",
        &appearance.mode,
    );
    push_stepper(
        &mut out,
        &mut widths,
        SettingId::LightTheme,
        "light theme",
        &appearance.light_theme,
    );
    push_stepper(
        &mut out,
        &mut widths,
        SettingId::DarkTheme,
        "dark theme",
        &appearance.dark_theme,
    );

    out.rows.push(SettingsRow::section("Tiles"));
    push_stepper(
        &mut out,
        &mut widths,
        SettingId::InitialRows,
        "initial rows",
        &app.loaded_config.config.tiles.initial_rows().to_string(),
    );
    push_stepper(
        &mut out,
        &mut widths,
        SettingId::FadeSeconds,
        "fade seconds",
        &app.loaded_config.config.tiles.fade().as_secs().to_string(),
    );

    out.rows.push(SettingsRow::section("Commands"));
    push_value(
        &mut out,
        &mut widths,
        "excluded",
        list(&app.loaded_config.config.commands.excluded),
    );
    push_value(
        &mut out,
        &mut widths,
        "hidden when idle",
        list(&app.loaded_config.config.commands.hidden_when_idle),
    );

    out.rows.push(SettingsRow::section("Files"));
    push_value(
        &mut out,
        &mut widths,
        "config",
        display_path(config::config_path()),
    );
    push_value(
        &mut out,
        &mut widths,
        "themes",
        display_path(config::themes_dir()),
    );
    push_value(
        &mut out,
        &mut widths,
        "keymap",
        display_path(config::keymap_path()),
    );

    if app.startup_note.is_some() || app.loaded_config.error.is_some() {
        out.rows.push(SettingsRow::section("Notices"));
    }
    if let Some(note) = app.startup_note.clone() {
        push_value(&mut out, &mut widths, "theme", note);
    }
    if let Some(error) = app.loaded_config.error.clone() {
        push_value(&mut out, &mut widths, "config", error);
    }
    out.widest_row = widths.widest_row();
    out
}

/// Step the selected row's value, then persist and apply the result.
///
/// A read-only row is a no-op, so the keys stay harmless everywhere in
/// the overlay.
pub(crate) fn cycle(app: &mut App, step: Step) {
    let selection = app.framework.settings_pane.viewport().pos();
    let Some(&id) = rows(app).ids.get(selection) else {
        return;
    };
    if id == SettingId::InitialRows {
        let rows = initial_row_choices();
        let current = app.loaded_config.config.tiles.initial_rows().to_string();
        app.loaded_config.config.tiles.initial_rows = stepped(&rows, &current, step)
            .parse()
            .unwrap_or(MIN_INITIAL_ROWS);
        apply(app);
        return;
    }
    if id == SettingId::FadeSeconds {
        let seconds = fade_choices();
        let current = app.loaded_config.config.tiles.fade().as_secs().to_string();
        app.loaded_config.config.tiles.fade_seconds = stepped(&seconds, &current, step)
            .parse()
            .unwrap_or(MIN_FADE_SECONDS);
        apply(app);
        return;
    }
    let appearance = &mut app.loaded_config.config.appearance;
    match id {
        SettingId::Mode => {
            let modes: Vec<String> = APPEARANCE_MODES
                .iter()
                .map(|mode| (*mode).to_string())
                .collect();
            appearance.mode = stepped(&modes, &appearance.mode, step);
        },
        SettingId::LightTheme => {
            let ids = theme_ids(Appearance::Light);
            appearance.light_theme = stepped(&ids, &appearance.light_theme, step);
        },
        SettingId::DarkTheme => {
            let ids = theme_ids(Appearance::Dark);
            appearance.dark_theme = stepped(&ids, &appearance.dark_theme, step);
        },
        SettingId::FadeSeconds | SettingId::InitialRows | SettingId::ReadOnly => return,
    }
    apply(app);
}

/// Re-resolve the active theme from the edited config and write the
/// file, reporting either failure through the overlay's notice rows.
fn apply(app: &mut App) {
    let appearance = &app.loaded_config.config.appearance;
    let registry = tui_pane::registry();
    let resolved = registry.resolve_active(
        &appearance.mode,
        &appearance.light_theme,
        &appearance.dark_theme,
        None,
    );
    app.startup_note = resolved
        .miss
        .as_ref()
        .map(|missing| format!("theme `{missing}` not found — using a built-in"));
    tui_pane::set_active_theme(resolved.theme);
    app.loaded_config.error = config::save(&app.loaded_config.config);
}

/// The values `tiles.initial_rows` steps through.
fn initial_row_choices() -> Vec<String> {
    (MIN_INITIAL_ROWS..=MAX_INITIAL_ROWS)
        .map(|rows| rows.to_string())
        .collect()
}

/// The values `tiles.fade_seconds` steps through.
fn fade_choices() -> Vec<String> {
    (MIN_FADE_SECONDS..=MAX_FADE_SECONDS)
        .map(|seconds| seconds.to_string())
        .collect()
}

/// Theme ids registered for one appearance, in registry order.
fn theme_ids(appearance: Appearance) -> Vec<String> {
    tui_pane::registry()
        .variants_by_appearance(appearance)
        .map(|variant| variant.id.as_str().to_string())
        .collect()
}

/// The value one step from `current`, wrapping at both ends.
///
/// A `current` that is not in `values` steps to the first entry, which
/// is how a hand-edited `config.toml` with an unknown id recovers.
fn stepped(values: &[String], current: &str, step: Step) -> String {
    let Some(first) = values.first() else {
        return current.to_string();
    };
    let Some(index) = values.iter().position(|value| value == current) else {
        return first.clone();
    };
    let len = values.len();
    let next = match step {
        Step::Prev => (index + len - 1) % len,
        Step::Next => (index + 1) % len,
    };
    values.get(next).unwrap_or(first).clone()
}

/// Push a cycling row and record which setting it edits.
fn push_stepper(
    out: &mut SettingsRows,
    widths: &mut RowWidths,
    id: SettingId,
    label: &str,
    value: &str,
) {
    widths.observe(label, value, STEPPER_DECORATION_WIDTH);
    out.rows
        .push(SettingsRow::stepper(out.ids.len(), label, value));
    out.ids.push(id);
}

/// Push a reported row that nothing edits.
fn push_value(out: &mut SettingsRows, widths: &mut RowWidths, label: &str, value: String) {
    widths.observe(label, &value, 0);
    out.rows
        .push(SettingsRow::value(out.ids.len(), label, value));
    out.ids.push(SettingId::ReadOnly);
}

/// Render a list setting for reading.
///
/// The overlay steps through fixed sets of values and a config list is
/// not one, so this row reports what the file says and the file is
/// where it is changed -- which the `config` row under Files points at.
fn list(entries: &[String]) -> String {
    if entries.is_empty() {
        return EMPTY_LIST.to_string();
    }
    entries.join(LIST_SEPARATOR)
}

/// Render a resolved path, or the placeholder for a platform where the
/// OS config directory is unavailable.
fn display_path(path: Option<PathBuf>) -> String {
    path.map_or_else(
        || UNRESOLVED_PATH.to_string(),
        |path| path.display().to_string(),
    )
}
