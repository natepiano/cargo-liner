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
use crate::capture_root::CleanupRefusal;
use crate::capture_root::RootOwner;
use crate::config;
use crate::constants::APPEARANCE_MODES;
use crate::constants::CAPTURE_ASSOCIATION;
use crate::constants::CAPTURE_ASSOCIATION_SUPPRESSED;
use crate::constants::CAPTURE_CLEANUP_ALLOWED;
use crate::constants::CAPTURE_CLEANUP_CHANGED;
use crate::constants::CAPTURE_CLEANUP_DISABLED;
use crate::constants::CAPTURE_CLEANUP_EFFECTIVE_USER;
use crate::constants::CAPTURE_CLEANUP_ENUMERATION;
use crate::constants::CAPTURE_CLEANUP_FOREIGN;
use crate::constants::CAPTURE_CLEANUP_REGISTRATION;
use crate::constants::CAPTURE_CLEANUP_WRITABLE;
use crate::constants::CAPTURE_FAILURE_PERMISSION;
use crate::constants::CAPTURE_OWNER_UID;
use crate::constants::CAPTURE_OWNER_UNAVAILABLE;
use crate::constants::CAPTURE_ROOT_ENV;
use crate::constants::CAPTURE_SETTINGS_ROOT;
use crate::constants::CAPTURE_SOURCE_CONFIG;
use crate::constants::CAPTURE_SOURCE_DEFAULT;
use crate::constants::CAPTURE_SOURCE_ENVIRONMENT;
use crate::constants::CAPTURE_STATUS_ACTIVE;
use crate::constants::CAPTURE_STATUS_ANNOTATION;
use crate::constants::CAPTURE_STATUS_BOOT;
use crate::constants::CAPTURE_STATUS_BOOT_FAILURE;
use crate::constants::CAPTURE_STATUS_CAPTURE;
use crate::constants::CAPTURE_STATUS_DEFAULT_NOT_CREATED;
use crate::constants::CAPTURE_STATUS_EMPTY;
use crate::constants::CAPTURE_STATUS_ENUMERATION;
use crate::constants::CAPTURE_STATUS_ENUMERATION_FAILED;
use crate::constants::CAPTURE_STATUS_IDENTITY;
use crate::constants::CAPTURE_STATUS_IDENTITY_BOOT;
use crate::constants::CAPTURE_STATUS_IDENTITY_RETRY;
use crate::constants::CAPTURE_STATUS_INVALID;
use crate::constants::CAPTURE_STATUS_INVALID_REGISTRATION;
use crate::constants::CAPTURE_STATUS_MISSING;
use crate::constants::CAPTURE_STATUS_PARTIAL;
use crate::constants::CAPTURE_STATUS_RETAINED;
use crate::constants::CAPTURE_STATUS_STAGING;
use crate::constants::CAPTURE_STATUS_UNREADABLE_LOG;
use crate::constants::CAPTURE_STATUS_UNREADABLE_REGISTRATION;
use crate::constants::CAPTURE_STATUS_UNVERIFIABLE;
use crate::constants::CONFIG_KEY_CAPTURE_ROOTS;
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
use crate::processes::CaptureDiagnostic;
use crate::processes::RootReadStatus;
use crate::processes::RootStatus;
use crate::progress::CaptureRootSource;
use crate::progress::PathFailure;

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

    out.rows.push(SettingsRow::section("Capture"));
    push_value(
        &mut out,
        &mut widths,
        "auto install",
        app.loaded_config.config.capture.auto_install.to_string(),
    );
    push_capture_roots(&mut out, &mut widths, &app.root_status);

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

    if app.startup_note.is_some() || app.capture_note.is_some() || app.loaded_config.error.is_some()
    {
        out.rows.push(SettingsRow::section("Notices"));
    }
    if let Some(note) = app.startup_note.clone() {
        push_value(&mut out, &mut widths, "theme", note);
    }
    if let Some(note) = app.capture_note.clone() {
        push_value(&mut out, &mut widths, "capture", note);
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

/// Each effective root adds one selectable value whose controls remain inert.
fn push_capture_roots(out: &mut SettingsRows, widths: &mut RowWidths, statuses: &[RootStatus]) {
    for (index, status) in statuses.iter().enumerate() {
        push_value(
            out,
            widths,
            &format!("{CAPTURE_SETTINGS_ROOT} {}", index + 1),
            capture_root_status(status),
        );
    }
}

/// Every spelling remains visible even when the scanner interns its root once.
fn capture_sources(sources: &[CaptureRootSource]) -> String {
    sources
        .iter()
        .map(|source| match source {
            CaptureRootSource::Default => CAPTURE_SOURCE_DEFAULT.to_string(),
            CaptureRootSource::Environment { path } => format!(
                "{CAPTURE_SOURCE_ENVIRONMENT} ({CAPTURE_ROOT_ENV}={})",
                path.display(),
            ),
            CaptureRootSource::Configuration { entry, path } => format!(
                "{CAPTURE_SOURCE_CONFIG} (capture.{CONFIG_KEY_CAPTURE_ROOTS}[{entry}]={})",
                path.display(),
            ),
        })
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}

/// Render one effective root entirely from observations retained on `App`.
fn capture_root_status(status: &RootStatus) -> String {
    let mut parts = vec![capture_sources(&status.root.sources)];
    if !matches!(status.state, RootReadStatus::DefaultNotCreated) {
        parts.push(capture_owner(status.owner));
    }
    if status.cleanup.is_empty() {
        match status.state {
            RootReadStatus::DefaultNotCreated => {},
            RootReadStatus::Readable => parts.push(CAPTURE_CLEANUP_ALLOWED.to_string()),
            RootReadStatus::Unavailable(_) | RootReadStatus::Invalid(_) => {
                parts.push(CAPTURE_CLEANUP_DISABLED.to_string());
            },
        }
    } else {
        parts.extend(status.cleanup.iter().map(cleanup_refusal));
    }
    parts.push(capture_read_status(status));
    parts.extend(status.diagnostics.iter().map(capture_diagnostic));
    if let Ok(path) = &status.root.path {
        parts.push(path.display().to_string());
        for association in &status.associations {
            parts.push(format!(
                "{CAPTURE_ASSOCIATION}: pid {} via registration {} from {}",
                association.pid,
                association.registration_pid,
                path.display(),
            ));
            for suppressed in &association.suppressed {
                parts.push(format!(
                    "{CAPTURE_ASSOCIATION_SUPPRESSED}: pid {} from {}",
                    association.pid,
                    suppressed.display(),
                ));
            }
        }
    }
    parts.join("; ")
}

/// Failed root reads have retry rules distinct from startup validation failures.
fn capture_read_status(status: &RootStatus) -> String {
    match &status.state {
        RootReadStatus::DefaultNotCreated => CAPTURE_STATUS_DEFAULT_NOT_CREATED.to_string(),
        RootReadStatus::Readable => {
            let captures = if status.confirmed == 0 {
                CAPTURE_STATUS_EMPTY.to_string()
            } else {
                format!(
                    "{CAPTURE_STATUS_ACTIVE} — {}",
                    counted(status.confirmed, CAPTURE_STATUS_CAPTURE),
                )
            };
            if status.diagnostics.is_empty() {
                return captures;
            }
            let summary = diagnostic_counts(&status.diagnostics);
            if status.confirmed == 0 {
                format!("{CAPTURE_STATUS_PARTIAL} — {summary}")
            } else {
                format!("{CAPTURE_STATUS_PARTIAL} — {summary}; {captures}")
            }
        },
        RootReadStatus::Unavailable(failure)
            if failure.failure.kind == std::io::ErrorKind::NotFound =>
        {
            format!("{CAPTURE_STATUS_MISSING}: {}", path_failure(failure))
        },
        RootReadStatus::Unavailable(failure) => path_failure(failure),
        RootReadStatus::Invalid(failure) => {
            format!("{CAPTURE_STATUS_INVALID}: {}", failure.message)
        },
    }
}

/// Aggregate artifact kinds without promoting any of them to confirmed captures.
fn diagnostic_counts(diagnostics: &[CaptureDiagnostic]) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for diagnostic in diagnostics {
        let label = diagnostic_label(diagnostic);
        if let Some((_, count)) = counts.iter_mut().find(|(known, _)| *known == label) {
            *count += 1;
        } else {
            counts.push((label, 1));
        }
    }
    counts
        .into_iter()
        .map(|(label, count)| counted(count, label))
        .collect::<Vec<_>>()
        .join(LIST_SEPARATOR)
}

/// All count labels use regular plurals; one capture remains singular.
fn counted(count: usize, label: &str) -> String {
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {label}{plural}")
}

/// Each retained observation has its own operator-facing count category.
const fn diagnostic_label(diagnostic: &CaptureDiagnostic) -> &'static str {
    match diagnostic {
        CaptureDiagnostic::EnumerationIncomplete(_) => CAPTURE_STATUS_ENUMERATION,
        CaptureDiagnostic::EnumerationFailed(_) => CAPTURE_STATUS_ENUMERATION_FAILED,
        CaptureDiagnostic::RegistrationUnreadable(_) => CAPTURE_STATUS_UNREADABLE_REGISTRATION,
        CaptureDiagnostic::RegistrationInvalid(_) => CAPTURE_STATUS_INVALID_REGISTRATION,
        CaptureDiagnostic::AnnotationOnly(_) => CAPTURE_STATUS_ANNOTATION,
        CaptureDiagnostic::Unverifiable(_) => CAPTURE_STATUS_UNVERIFIABLE,
        CaptureDiagnostic::IdentityUnknown(_) | CaptureDiagnostic::IdentityBlockedByBoot(_) => {
            CAPTURE_STATUS_IDENTITY
        },
        CaptureDiagnostic::Staging(_) => CAPTURE_STATUS_STAGING,
        CaptureDiagnostic::LogUnreadable(_) => CAPTURE_STATUS_UNREADABLE_LOG,
        CaptureDiagnostic::BootUnavailable(_) => CAPTURE_STATUS_BOOT_FAILURE,
    }
}

/// Path-qualified details explain retention without promising eventual deletion.
fn capture_diagnostic(diagnostic: &CaptureDiagnostic) -> String {
    let label = diagnostic_label(diagnostic);
    match diagnostic {
        CaptureDiagnostic::EnumerationIncomplete(path) | CaptureDiagnostic::Staging(path) => {
            format!("{label}: {}", path.display())
        },
        CaptureDiagnostic::RegistrationInvalid(path)
        | CaptureDiagnostic::AnnotationOnly(path)
        | CaptureDiagnostic::Unverifiable(path) => {
            format!("{label}: {} ({CAPTURE_STATUS_RETAINED})", path.display())
        },
        CaptureDiagnostic::IdentityUnknown(path) => {
            format!(
                "{label}: {} ({CAPTURE_STATUS_IDENTITY_RETRY})",
                path.display()
            )
        },
        CaptureDiagnostic::IdentityBlockedByBoot(path) => {
            format!(
                "{label}: {} ({CAPTURE_STATUS_IDENTITY_BOOT})",
                path.display()
            )
        },
        CaptureDiagnostic::EnumerationFailed(failure)
        | CaptureDiagnostic::RegistrationUnreadable(failure)
        | CaptureDiagnostic::LogUnreadable(failure) => {
            format!("{label}: {}", path_failure(failure))
        },
        CaptureDiagnostic::BootUnavailable(failure) => {
            format!("{CAPTURE_STATUS_BOOT}: {}", path_failure(failure))
        },
    }
}

/// Account identity comes from the scanned descriptor, never an account lookup.
fn capture_owner(owner: RootOwner) -> String {
    match owner {
        RootOwner::Uid(uid) => format!("{CAPTURE_OWNER_UID} {uid}"),
        RootOwner::Unavailable => CAPTURE_OWNER_UNAVAILABLE.to_string(),
    }
}

/// Cleanup reasons retain their failing directory even when the root is readable.
fn cleanup_refusal(refusal: &CleanupRefusal) -> String {
    match refusal {
        CleanupRefusal::Foreign(path) => {
            format!("{CAPTURE_CLEANUP_FOREIGN}: {}", path.display())
        },
        CleanupRefusal::WritableByOthers(path) => {
            format!("{CAPTURE_CLEANUP_WRITABLE}: {}", path.display())
        },
        CleanupRefusal::EffectiveUserUnavailable => CAPTURE_CLEANUP_EFFECTIVE_USER.to_string(),
        CleanupRefusal::Access(failure) => {
            format!("{CAPTURE_CLEANUP_DISABLED} — {}", path_failure(failure))
        },
        CleanupRefusal::EnumerationIncomplete(path) => {
            format!("{CAPTURE_CLEANUP_ENUMERATION}: {}", path.display())
        },
        CleanupRefusal::RegistrationIncomplete(path) => {
            format!("{CAPTURE_CLEANUP_REGISTRATION}: {}", path.display())
        },
        CleanupRefusal::Changed(path) => {
            format!("{CAPTURE_CLEANUP_CHANGED}: {}", path.display())
        },
    }
}

/// Both the failed artifact and the original diagnostic survive rendering.
fn path_failure(failure: &PathFailure) -> String {
    match failure.failure.kind {
        std::io::ErrorKind::PermissionDenied => format!(
            "{CAPTURE_FAILURE_PERMISSION}: {} ({})",
            failure.path.display(),
            failure.failure.message,
        ),
        _ => format!("{}: {}", failure.path.display(), failure.failure.message),
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::collections::HashSet;
    use std::io::ErrorKind;

    use tui_pane::SettingsRow;

    use super::SettingId;
    use super::capture_sources;
    use super::rows;
    use crate::app::App;
    use crate::capture_root::CleanupRefusal;
    use crate::capture_root::RootOwner;
    use crate::constants::CAPTURE_STATUS_DEFAULT_NOT_CREATED;
    use crate::constants::CAPTURE_STATUS_IDENTITY_BOOT;
    use crate::processes::CaptureAssociation;
    use crate::processes::CaptureDiagnostic;
    use crate::processes::RootReadStatus;
    use crate::processes::RootStatus;
    use crate::progress::CaptureFailure;
    use crate::progress::CaptureRoot;
    use crate::progress::CaptureRootSource;
    use crate::progress::PathFailure;

    /// Construct retained evidence without resolving or accessing a fixture path.
    fn observed_root() -> RootStatus {
        RootStatus {
            root:         CaptureRoot {
                path:    Ok("/retained/captures".into()),
                sources: vec![CaptureRootSource::Default],
            },
            owner:        RootOwner::Uid(1000),
            cleanup:      Vec::new(),
            state:        RootReadStatus::Readable,
            confirmed:    0,
            diagnostics:  Vec::new(),
            associations: Vec::new(),
        }
    }

    /// Preserve the exact failed path and diagnostic without creating an I/O error.
    fn failure(path: &str, kind: ErrorKind) -> PathFailure {
        PathFailure {
            path:    path.into(),
            failure: CaptureFailure {
                kind,
                message: "retained scan error".into(),
            },
        }
    }

    /// Exercise the public row builder, including its inert selection mapping.
    fn root_row(status: RootStatus) -> SettingsRow {
        let mut app = App::new_for_test().expect("quiet settings app");
        app.root_status.push(status);
        let settings = rows(&app);
        let root = settings
            .rows
            .iter()
            .find(|row| row.label == "root 1")
            .expect("retained root row");
        assert_eq!(root.kind, SettingsRow::value(0, "", "").kind);
        let selection = root.payload.expect("read-only selection payload").get();
        assert_eq!(settings.ids[selection], SettingId::ReadOnly);
        assert!(settings.widest_row >= root.value.chars().count());
        root.clone()
    }

    #[test]
    fn default_source_is_quiet_and_does_not_claim_an_environment_override() {
        assert_eq!(capture_sources(&[CaptureRootSource::Default]), "default");
    }

    #[test]
    fn environment_source_preserves_a_relative_override() {
        assert_eq!(
            capture_sources(&[CaptureRootSource::Environment {
                path: "captures".into(),
            }]),
            "environment (CARGO_TILE_ROOT=captures)",
        );
    }

    #[test]
    fn deduplicated_sources_keep_all_original_spellings_and_config_positions() {
        assert_eq!(
            capture_sources(&[
                CaptureRootSource::Environment {
                    path: "/tmp/runner".into(),
                },
                CaptureRootSource::Configuration {
                    entry: 0,
                    path:  "/tmp/runner/state/..".into(),
                },
                CaptureRootSource::Configuration {
                    entry: 1,
                    path:  "/tmp/runner".into(),
                },
            ]),
            "environment (CARGO_TILE_ROOT=/tmp/runner), config (capture.roots[0]=/tmp/runner/state/..), config (capture.roots[1]=/tmp/runner)",
        );
    }

    #[test]
    fn readable_empty_root_displays_owner_cleanup_and_absolute_path() {
        assert_eq!(
            root_row(observed_root()).value,
            "default; owner uid 1000; cleanup allowed for proven ended captures; readable — no active captures; /retained/captures",
        );
    }

    #[test]
    fn unused_default_keeps_its_row_without_access_or_cleanup_warnings() {
        let mut status = observed_root();
        status.owner = RootOwner::Unavailable;
        status.state = RootReadStatus::DefaultNotCreated;
        assert_eq!(
            root_row(status).value,
            format!("default; {CAPTURE_STATUS_DEFAULT_NOT_CREATED}; /retained/captures"),
        );
    }

    #[test]
    fn explicitly_named_missing_roots_keep_access_and_cleanup_failures() {
        let configured = CaptureRootSource::Configuration {
            entry: 0,
            path:  "/retained/captures".into(),
        };
        let environment = CaptureRootSource::Environment {
            path: "/retained/captures".into(),
        };
        for sources in [
            vec![configured.clone()],
            vec![environment.clone()],
            vec![CaptureRootSource::Default, configured],
            vec![CaptureRootSource::Default, environment],
        ] {
            let mut status = observed_root();
            status.root.sources = sources;
            status.owner = RootOwner::Unavailable;
            let failure = failure("/retained/captures", ErrorKind::NotFound);
            status.state = RootReadStatus::Unavailable(failure.clone());
            status.cleanup.push(CleanupRefusal::Access(failure));
            let value = root_row(status).value;
            assert!(value.contains(
                "owner unavailable; cleanup disabled — /retained/captures: retained scan error"
            ));
            assert!(value.contains("missing directory: /retained/captures: retained scan error"));
            assert!(!value.contains(CAPTURE_STATUS_DEFAULT_NOT_CREATED));
        }
    }

    #[test]
    fn confirmed_captures_have_singular_and_plural_counts() {
        for (confirmed, wording) in [(1, "active — 1 capture;"), (2, "active — 2 captures;")] {
            let mut status = observed_root();
            status.confirmed = confirmed;
            assert!(root_row(status).value.contains(wording));
        }
    }

    #[test]
    fn all_cleanup_refusals_are_distinct_from_each_other_and_an_empty_root() {
        let path = "/retained/captures/state/pids";
        let cases = [
            (
                CleanupRefusal::EnumerationIncomplete(path.into()),
                "partial enumeration — cleanup disabled",
            ),
            (
                CleanupRefusal::RegistrationIncomplete(path.into()),
                "incomplete registration inventory — cleanup disabled",
            ),
            (CleanupRefusal::Foreign(path.into()), "read-only"),
            (
                CleanupRefusal::WritableByOthers(path.into()),
                "group/other writable — cleanup disabled",
            ),
            (
                CleanupRefusal::EffectiveUserUnavailable,
                "ownership unavailable — cleanup disabled for this session; restart to retry effective-user lookup",
            ),
            (
                CleanupRefusal::Access(failure(path, ErrorKind::PermissionDenied)),
                "cleanup disabled — permission denied: /retained/captures/state/pids",
            ),
            (
                CleanupRefusal::Changed(path.into()),
                "directory changed — cleanup disabled this scan",
            ),
        ];
        let mut rendered = HashSet::from([root_row(observed_root()).value]);
        for (refusal, wording) in cases {
            let mut status = observed_root();
            status.cleanup.push(refusal);
            let row = root_row(status);
            assert!(row.value.contains(wording), "{}", row.value);
            assert!(!row.value.contains("cleanup allowed"));
            assert!(rendered.insert(row.value));
        }
    }

    #[test]
    fn foreign_owner_is_read_only_without_a_permission_repair_instruction() {
        let mut status = observed_root();
        status.owner = RootOwner::Uid(2000);
        status
            .cleanup
            .push(CleanupRefusal::Foreign("/retained/captures".into()));
        let value = root_row(status).value;
        assert!(value.contains("owner uid 2000; read-only: /retained/captures"));
        assert!(!value.contains("restart"));
        assert!(!value.contains("writable"));
    }

    #[test]
    fn effective_user_failure_names_the_session_and_restart_retry() {
        let mut status = observed_root();
        status
            .cleanup
            .push(CleanupRefusal::EffectiveUserUnavailable);
        let value = root_row(status).value;
        assert!(value.contains("ownership unavailable — cleanup disabled for this session"));
        assert!(value.contains("restart to retry effective-user lookup"));
        assert!(!value.contains("read-only"));
    }

    #[test]
    fn missing_directory_is_distinct_from_failed_access_and_empty() {
        let mut status = observed_root();
        status.owner = RootOwner::Unavailable;
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::NotFound));
        let value = root_row(status).value;
        assert!(value.contains(
            "owner unavailable; cleanup disabled; missing directory: /retained/captures"
        ));
        assert!(!value.contains("no active captures"));
        assert!(!value.contains("restart"));
    }

    #[test]
    fn root_permission_failure_preserves_its_path_and_observed_message() {
        let mut status = observed_root();
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::PermissionDenied));
        assert!(
            root_row(status)
                .value
                .contains("permission denied: /retained/captures (retained scan error)")
        );
    }

    #[test]
    fn failed_directory_enumeration_names_state_pids_in_the_row() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::EnumerationFailed(failure(
                "/retained/captures/state/pids",
                ErrorKind::PermissionDenied,
            )));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 failed directory listing"));
        assert!(
            value
                .contains("permission denied: /retained/captures/state/pids (retained scan error)")
        );
        assert!(!value.contains("no active captures"));
    }

    #[test]
    fn bounded_enumeration_is_distinct_from_a_failed_enumeration() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::EnumerationIncomplete(
                "/retained/captures/state/pids".into(),
            ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 short directory listing"));
        assert!(value.contains("short directory listing: /retained/captures/state/pids"));
        assert!(!value.contains("failed directory listing"));
    }

    #[test]
    fn unreadable_registration_is_distinct_from_an_empty_root() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::RegistrationUnreadable(failure(
                "/retained/captures/state/pids/42-uuid",
                ErrorKind::PermissionDenied,
            )));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unreadable registration"));
        assert!(value.contains("permission denied: /retained/captures/state/pids/42-uuid"));
        assert!(!value.contains("no active captures"));
    }

    #[test]
    fn invalid_registration_is_visible_with_its_artifact_path() {
        let mut status = observed_root();
        status
            .diagnostics
            .push(CaptureDiagnostic::RegistrationInvalid(
                "/retained/captures/state/pids/42-uuid".into(),
            ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 invalid registration"));
        assert!(value.contains("invalid registration: /retained/captures/state/pids/42-uuid"));
    }

    #[test]
    fn mixed_root_counts_only_confirmed_captures_and_names_each_other_artifact() {
        let mut status = observed_root();
        status.confirmed = 1;
        status.diagnostics = vec![
            CaptureDiagnostic::LogUnreadable(failure(
                "/retained/captures/run-42-uuid.log",
                ErrorKind::PermissionDenied,
            )),
            CaptureDiagnostic::AnnotationOnly("/retained/captures/state/pids/43".into()),
            CaptureDiagnostic::Staging("/retained/captures/state/pids/44-uuid.tmp".into()),
        ];
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unreadable log, 1 annotation-only record, 1 staging file; active — 1 capture"));
        assert!(value.contains("permission denied: /retained/captures/run-42-uuid.log"));
        assert!(value.contains("annotation-only record: /retained/captures/state/pids/43"));
        assert!(value.contains("staging file: /retained/captures/state/pids/44-uuid.tmp"));
        assert!(!value.contains("active — 4"));
    }

    #[test]
    fn repeated_unreadable_logs_are_counted_together_and_keep_both_paths() {
        let mut status = observed_root();
        for path in [
            "/retained/captures/run-42.log",
            "/retained/captures/run-43.log",
        ] {
            status
                .diagnostics
                .push(CaptureDiagnostic::LogUnreadable(failure(
                    path,
                    ErrorKind::Other,
                )));
        }
        let value = root_row(status).value;
        assert!(value.contains("partial — 2 unreadable logs"));
        assert!(value.contains("/retained/captures/run-42.log: retained scan error"));
        assert!(value.contains("/retained/captures/run-43.log: retained scan error"));
    }

    #[test]
    fn boot_failure_names_restart_and_differs_from_identity_unknown_this_scan() {
        let mut cached = observed_root();
        cached
            .diagnostics
            .push(CaptureDiagnostic::BootUnavailable(failure(
                "/kernel/boot-id",
                ErrorKind::PermissionDenied,
            )));
        let cached = root_row(cached).value;
        assert!(cached.contains("partial — 1 cached boot failure"));
        assert!(cached.contains("boot identity unavailable — verification disabled for this session; restart to retry boot read"));
        assert!(cached.contains("/kernel/boot-id"));
        let mut retryable = observed_root();
        retryable
            .diagnostics
            .push(CaptureDiagnostic::IdentityUnknown(
                "/retained/captures/state/pids/42-uuid".into(),
            ));
        let retryable = root_row(retryable).value;
        assert!(retryable.contains("partial — 1 unconfirmed registration"));
        assert!(retryable.contains(
            "identity unknown this scan — retained; identity is checked again next scan"
        ));
        assert!(!retryable.contains("restart"));
        assert_ne!(cached, retryable);
    }

    #[test]
    fn boot_blocked_identity_keeps_retention_without_promising_a_next_scan_retry() {
        let mut status = observed_root();
        status.cleanup.push(CleanupRefusal::RegistrationIncomplete(
            "/retained/captures/state/pids".into(),
        ));
        status.diagnostics = vec![
            CaptureDiagnostic::IdentityBlockedByBoot(
                "/retained/captures/state/pids/42-uuid".into(),
            ),
            CaptureDiagnostic::BootUnavailable(failure(
                "/kernel/boot-id",
                ErrorKind::PermissionDenied,
            )),
        ];
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unconfirmed registration, 1 cached boot failure"));
        assert!(value.contains(&format!(
            "unconfirmed registration: /retained/captures/state/pids/42-uuid ({CAPTURE_STATUS_IDENTITY_BOOT})",
        )));
        assert!(value.contains(
            "incomplete registration inventory — cleanup disabled: /retained/captures/state/pids"
        ));
        assert!(value.contains("boot identity unavailable — verification disabled for this session; restart to retry boot read: permission denied: /kernel/boot-id"));
        assert!(!value.contains("next scan"));
        assert!(!value.contains("active —"));
    }

    #[test]
    fn permanently_unverifiable_artifact_does_not_promise_eventual_removal() {
        let mut status = observed_root();
        status.diagnostics.push(CaptureDiagnostic::Unverifiable(
            "/retained/captures/state/pids/42-uuid".into(),
        ));
        let value = root_row(status).value;
        assert!(value.contains("partial — 1 unverifiable artifact"));
        assert!(value.contains(
            "retained — identity cannot be established; cleanup cannot remove this artifact"
        ));
        assert!(!value.contains("next scan"));
        assert!(!value.contains("eventual"));
        assert!(!value.contains("will remove"));
    }

    #[test]
    fn validation_failure_keeps_original_config_entry_and_requires_correction_and_restart() {
        let mut status = observed_root();
        let error = CaptureFailure {
            kind:    ErrorKind::InvalidInput,
            message: "capture.roots[2]: capture root must be an absolute path".into(),
        };
        status.root.path = Err(error.clone());
        status.root.sources = vec![CaptureRootSource::Configuration {
            entry: 2,
            path:  "runner/state/..".into(),
        }];
        status.owner = RootOwner::Unavailable;
        status.state = RootReadStatus::Invalid(error);
        let value = root_row(status).value;
        assert!(value.contains("config (capture.roots[2]=runner/state/..)"));
        assert!(value.contains("invalid root — correct the path and restart to retry resolution: capture.roots[2]: capture root must be an absolute path"));
        assert!(!value.contains("missing directory"));
        assert!(!value.contains("cleanup allowed"));
    }

    #[test]
    fn relative_environment_spelling_displays_with_the_retained_absolute_root() {
        let mut status = observed_root();
        status.root.sources = vec![CaptureRootSource::Environment {
            path: "captures".into(),
        }];
        let value = root_row(status).value;
        assert!(value.contains("environment (CARGO_TILE_ROOT=captures)"));
        assert!(value.contains("readable — no active captures; /retained/captures"));
        assert!(!value.contains("invalid root"));
    }

    #[test]
    fn deduplicated_root_is_one_row_with_all_source_spellings() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let mut status = observed_root();
        status.root.sources = vec![
            CaptureRootSource::Environment {
                path: "/retained/captures".into(),
            },
            CaptureRootSource::Configuration {
                entry: 0,
                path:  "/retained/captures/state/..".into(),
            },
        ];
        app.root_status.push(status);
        let settings = rows(&app);
        let roots: Vec<_> = settings
            .rows
            .iter()
            .filter(|row| row.label.starts_with("root "))
            .collect();
        assert_eq!(roots.len(), 1);
        assert!(roots[0].value.contains("environment (CARGO_TILE_ROOT=/retained/captures), config (capture.roots[0]=/retained/captures/state/..)"));
    }

    #[test]
    fn association_names_its_supplier_and_any_unused_competing_proof() {
        let mut status = observed_root();
        status.associations = vec![CaptureAssociation {
            pid:              42,
            registration_pid: 40,
            suppressed:       vec!["/retained/other".into()],
        }];
        let value = root_row(status).value;
        assert!(
            value.contains(
                "capture association: pid 42 via registration 40 from /retained/captures"
            )
        );
        assert!(value.contains("another root's proof went unused: pid 42 from /retained/other"));
    }

    #[test]
    fn rows_use_retained_observations_without_filesystem_access() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let mut status = observed_root();
        // A NUL cannot occur in a real Unix pathname. Reopening this root could
        // never produce the retained readable status, owner, or confirmed count.
        status.root.path = Ok("/retained/\0/captures".into());
        status.confirmed = 2;
        app.root_status.push(status);
        let first = rows(&app);
        let second = rows(&app);
        assert_eq!(first.rows, second.rows);
        let root = first
            .rows
            .iter()
            .find(|row| row.label == "root 1")
            .expect("root row");
        assert!(root.value.contains("owner uid 1000"));
        assert!(root.value.contains("active — 2 captures"));
        assert!(root.value.contains("/retained/\0/captures"));
    }

    #[test]
    fn retained_access_recovery_changes_the_row_without_process_groups() {
        let mut app = App::new_for_test().expect("quiet settings app");
        let mut status = observed_root();
        status.state =
            RootReadStatus::Unavailable(failure("/retained/captures", ErrorKind::NotFound));
        app.root_status.push(status);
        let missing = rows(&app).rows;
        app.root_status[0].state = RootReadStatus::Readable;
        let readable = rows(&app).rows;
        assert_ne!(missing, readable);
        app.root_status[0]
            .diagnostics
            .push(CaptureDiagnostic::LogUnreadable(failure(
                "/retained/captures/run-42.log",
                ErrorKind::PermissionDenied,
            )));
        let unreadable_log = rows(&app).rows;
        assert_ne!(readable, unreadable_log);
        app.root_status[0].diagnostics.clear();
        assert_eq!(rows(&app).rows, readable);
    }
}
