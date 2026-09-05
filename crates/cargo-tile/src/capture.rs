//! Standing the capture shim up as the grid opens, and saying what
//! happened.
//!
//! The file system work is [`hook`]'s. This is the part that turns its
//! answer into something on screen: a toast for each thing that
//! changed, and a row under Notices in the settings overlay for
//! anything still wrong once the toasts have gone.

use std::io;

use tui_pane::ToastStyle;

use crate::app::App;
use crate::constants::CAPTURE_INSTALLED_TOAST_VISIBLE;
use crate::constants::LIST_SEPARATOR;
use crate::constants::NOTICE_TOAST_MIN_INTERIOR_LINES;
use crate::constants::NOTICE_TOAST_VISIBLE;
use crate::hook;
use crate::hook::Startup;

/// Put the shim in front of cargo, if `config.toml` allows it, and
/// report the outcome through the app.
pub(crate) fn stand_up(app: &mut App) {
    if !app.loaded_config.config.capture.auto_install {
        return;
    }
    report(app, hook::at_startup());
}

/// Turn what startup found into toasts and the settings notice.
fn report(app: &mut App, startup: io::Result<Startup>) {
    let startup = match startup {
        Ok(startup) => startup,
        Err(error) => {
            let note = format!("toolchains could not be read: {error}");
            app.framework.toasts.push_timed_styled(
                "Capture shim not installed",
                &note,
                NOTICE_TOAST_VISIBLE,
                NOTICE_TOAST_MIN_INTERIOR_LINES,
                ToastStyle::Warning,
            );
            app.capture_note = Some(note);
            return;
        },
    };
    if startup.is_quiet() {
        return;
    }
    if !startup.installed.is_empty() {
        app.framework.toasts.push_timed_styled(
            "Capture shim installed",
            format!(
                "In front of cargo for {}. Runs started from now on report progress; \
                 `cargo tile uninstall` gives cargo its name back.",
                startup.installed.join(LIST_SEPARATOR)
            ),
            CAPTURE_INSTALLED_TOAST_VISIBLE,
            NOTICE_TOAST_MIN_INTERIOR_LINES,
            ToastStyle::Success,
        );
    }
    if !startup.refreshed.is_empty() {
        app.framework.toasts.push_timed(
            "Capture shim updated",
            format!(
                "{}: brought up to this version of cargo-tile.",
                startup.refreshed.join(LIST_SEPARATOR)
            ),
            NOTICE_TOAST_VISIBLE,
            NOTICE_TOAST_MIN_INTERIOR_LINES,
        );
    }
    let mut notes = Vec::new();
    if !startup.orphaned.is_empty() {
        notes.push(format!(
            "{}: shim installed but the real cargo is missing; `rustup update` puts one back",
            startup.orphaned.join(LIST_SEPARATOR)
        ));
    }
    for (toolchain, error) in &startup.failed {
        notes.push(format!("{toolchain}: not installed: {error}"));
    }
    if notes.is_empty() {
        return;
    }
    let note = notes.join("; ");
    app.framework
        .toasts
        .push_styled("Capture shim", &note, ToastStyle::Warning);
    app.capture_note = Some(note);
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn titles(app: &App) -> Vec<String> {
        app.framework
            .toasts
            .active_views(Instant::now())
            .iter()
            .map(|view| view.title().to_owned())
            .collect()
    }

    #[test]
    fn a_launch_that_changed_nothing_says_nothing() {
        let mut app = App::new_for_test().unwrap();

        report(&mut app, Ok(Startup::default()));

        assert!(titles(&app).is_empty());
        assert!(app.capture_note.is_none());
    }

    #[test]
    fn an_install_names_the_toolchains_and_the_way_back() {
        let mut app = App::new_for_test().unwrap();

        report(
            &mut app,
            Ok(Startup {
                installed: vec!["nightly".to_owned(), "stable".to_owned()],
                ..Startup::default()
            }),
        );

        let toasts = app.framework.toasts.active_views(Instant::now());
        assert_eq!(toasts.len(), 1);
        assert_eq!(toasts[0].title(), "Capture shim installed");
        assert!(toasts[0].body().contains("nightly, stable"));
        assert!(toasts[0].body().contains("cargo tile uninstall"));
        // Installing is the expected outcome, not a problem to keep on
        // the settings overlay.
        assert!(app.capture_note.is_none());
    }

    #[test]
    fn a_refresh_is_a_passing_notice() {
        let mut app = App::new_for_test().unwrap();

        report(
            &mut app,
            Ok(Startup {
                refreshed: vec!["stable".to_owned()],
                ..Startup::default()
            }),
        );

        assert_eq!(titles(&app), vec!["Capture shim updated".to_owned()]);
        assert!(app.capture_note.is_none());
    }

    /// Something still wrong once the toast is gone has to be findable,
    /// so it goes on the settings overlay as well.
    #[test]
    fn an_orphan_and_a_failure_stay_on_the_settings_overlay() {
        let mut app = App::new_for_test().unwrap();

        report(
            &mut app,
            Ok(Startup {
                orphaned: vec!["stable".to_owned()],
                failed: vec![("nightly".to_owned(), "permission denied".to_owned())],
                ..Startup::default()
            }),
        );

        assert_eq!(titles(&app), vec!["Capture shim".to_owned()]);
        let note = app.capture_note.as_deref().unwrap();
        assert!(note.contains("stable: shim installed but the real cargo is missing"));
        assert!(note.contains("nightly: not installed: permission denied"));
    }

    #[test]
    fn unreadable_toolchains_are_reported_rather_than_fatal() {
        let mut app = App::new_for_test().unwrap();

        report(&mut app, Err(io::Error::other("no home directory")));

        assert_eq!(titles(&app), vec!["Capture shim not installed".to_owned()]);
        assert!(
            app.capture_note
                .as_deref()
                .unwrap()
                .contains("no home directory")
        );
    }

    #[test]
    fn a_config_that_turns_auto_install_off_touches_nothing() {
        let mut app = App::new_for_test().unwrap();
        app.loaded_config.config.capture.auto_install = false;

        stand_up(&mut app);

        assert!(titles(&app).is_empty());
        assert!(app.capture_note.is_none());
    }
}
