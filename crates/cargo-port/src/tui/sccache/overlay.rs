use tui_pane::Action;
use tui_pane::ClipboardBackend;
use tui_pane::CopySelectionResult;
use tui_pane::FocusedPane;
use tui_pane::KeyBind;
use tui_pane::KeySequence;
use tui_pane::Navigation;
use tui_pane::SystemClipboard;

use crate::scan::BackgroundMsg;
use crate::sccache;
use crate::tui::app::App;
use crate::tui::integration::AppGlobalAction;
use crate::tui::integration::AppNavigation;
use crate::tui::integration::AppPaneId;
use crate::tui::keymap::SccacheAction;

pub(super) fn open_sccache_stats_overlay(app: &mut App) {
    app.overlays.close_finder();
    app.overlays.open_sccache();
    let request_id = app.overlays.sccache_pane.start_loading();
    let sender = app.background.background_sender();
    std::thread::spawn(move || {
        let result = sccache::read_stats();
        let _ = sender.send(BackgroundMsg::SccacheStats { request_id, result });
    });
}

pub(super) fn dispatch_sccache_overlay(app: &mut App, bind: &KeyBind) -> bool {
    dispatch_with_clipboard(app, bind, &mut SystemClipboard::new())
}

fn dispatch_with_clipboard(
    app: &mut App,
    bind: &KeyBind,
    clipboard: &mut impl ClipboardBackend,
) -> bool {
    if !app.overlays.is_sccache_open() {
        app.overlays.sccache_pane.pending_chord.clear();
        return false;
    }
    let keymap = std::rc::Rc::clone(&app.framework_keymap);
    let cancel = keymap.keys_for_toml_key(AppPaneId::Sccache, SccacheAction::Cancel.toml_key());
    let globals = keymap.globals::<AppGlobalAction>();
    let copy = globals.map_or(&[][..], |scope| {
        scope.display_keys_for(AppGlobalAction::Copy)
    });
    let navigation = keymap.navigation();
    app.overlays.sccache_pane.pending_chord.push(*bind);
    // On a missed chord, retry the final key as the start of a new binding.
    for _ in 0..2 {
        let pending = &app.overlays.sccache_pane.pending_chord;
        let sequence = KeySequence::new(pending.clone());
        if cancel
            .iter()
            .chain(copy)
            .any(|keys| keys.starts_with_strict(pending))
            || navigation.is_some_and(|scope| scope.has_prefix(pending))
        {
            return true;
        }
        if cancel.contains(&sequence) {
            super::dispatch_sccache_action(SccacheAction::Cancel, app);
            return true;
        }
        if copy.contains(&sequence) {
            app.overlays.sccache_pane.pending_chord.clear();
            copy_selected_value(app, clipboard);
            return true;
        }
        if let Some(action) = navigation.and_then(|scope| scope.action_for_sequence(&sequence)) {
            app.overlays.sccache_pane.pending_chord.clear();
            (AppNavigation::dispatcher())(action, FocusedPane::App(AppPaneId::Sccache), app);
            return true;
        }
        app.overlays.sccache_pane.pending_chord.clear();
        app.overlays.sccache_pane.pending_chord.push(*bind);
    }
    app.overlays.sccache_pane.pending_chord.clear();
    true
}

fn copy_selected_value(app: &mut App, clipboard: &mut impl ClipboardBackend) {
    let CopySelectionResult::Payload(payload) = super::copy_selection(app) else {
        app.show_timed_toast("Copy", "Nothing to copy");
        return;
    };
    let label = app
        .overlays
        .sccache_pane
        .selected_copy_value()
        .map(|target| target.label.clone())
        .unwrap_or_default();
    match clipboard.write_clipboard(&payload.text) {
        Ok(()) => app.show_timed_toast("Copy", format!("Copied {label}")),
        Err(reason) if reason.is_unavailable() => {
            app.show_timed_toast("Clipboard unavailable", reason.to_string());
        },
        Err(reason) => app.show_timed_toast("Copy failed", reason.to_string()),
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::Event;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use tui_pane::ClipboardError;
    use tui_pane::Keymap;
    use tui_pane::NavAction;
    use tui_pane::VimMode;

    use super::*;
    use crate::sccache::StatsResult;
    use crate::tui::input;
    use crate::tui::integration;
    use crate::tui::sccache::pane::SccacheSelectableLine;
    use crate::tui::sccache::pane::SccacheTarget;
    use crate::tui::test_support;

    fn configured_app(toml: &str) -> App {
        let mut app = test_support::make_app(&[]);
        let directory = tempfile::tempdir().expect("fixture directory");
        let path = directory.path().join("keymap.toml");
        std::fs::write(&path, toml).expect("write fixture bindings");
        let builder = Keymap::builder()
            .vim_mode(VimMode::Disabled)
            .load_toml(path)
            .expect("load bindings");
        app.framework_keymap = std::rc::Rc::new(
            integration::build_framework_keymap(builder, &mut app.framework).expect("build keymap"),
        );
        app.overlays.open_sccache();
        let request = app.overlays.sccache_pane.start_loading();
        app.overlays.sccache_pane.apply_result(
            request,
            StatsResult::Ready(vec!["Cache hits  7".into(), "Cache misses  3".into()]),
        );
        app.overlays.sccache_pane.set_selectable_lines(vec![
            SccacheSelectableLine {
                line_index: 1,
                target:     SccacheTarget {
                    label: "Cache hits".into(),
                    value: "7".into(),
                },
            },
            SccacheSelectableLine {
                line_index: 2,
                target:     SccacheTarget {
                    label: "Cache misses".into(),
                    value: "3".into(),
                },
            },
        ]);
        app
    }

    fn press(app: &mut App, sequence: &str) {
        for key in KeySequence::parse(sequence).expect("valid sequence").keys() {
            input::handle_event(app, &Event::Key(KeyEvent::new(key.code, key.mods)));
        }
    }

    #[test]
    fn sccache_cancel_defaults_rebinds_unbinds_and_keeps_the_owned_run_alive() {
        let mut app = configured_app("");
        app.inflight
            .set_example_output(vec!["running output".into()]);
        app.inflight.set_example_running(Some("demo".into()));
        let focus = *app.framework.focused();
        for key in ['q', 'e', 'x', '?'] {
            press(&mut app, &key.to_string());
        }
        assert_eq!(*app.framework.focused(), focus);
        assert!(app.overlays.is_sccache_open());
        press(&mut app, "escape");
        assert!(!app.overlays.is_sccache_open());
        assert!(app.inflight.example_running().is_some());
        assert!(!matches!(
            app.inflight.owned_run_termination(),
            crate::tui::state::OwnedRunTermination::RequestPending { .. }
        ));
        assert!(!dispatch_sccache_overlay(&mut app, &KeyBind::from('q')));

        let mut rebound = configured_app("[sccache]\ncancel = [\"ctrl-w\", \"z z\"]");
        press(&mut rebound, "escape");
        assert!(rebound.overlays.is_sccache_open());
        press(&mut rebound, "z");
        assert!(rebound.overlays.is_sccache_open());
        press(&mut rebound, "z");
        assert!(!rebound.overlays.is_sccache_open());
        assert_eq!(
            rebound.overlays.sccache_pane.pending_chord,
            [] as [tui_pane::KeyBind; 0]
        );
        rebound.overlays.open_sccache();
        press(&mut rebound, "ctrl-w");
        assert!(!rebound.overlays.is_sccache_open());

        let mut unbound = configured_app("[sccache]\ncancel = \"\"");
        press(&mut unbound, "escape");
        assert!(unbound.overlays.is_sccache_open());
    }

    #[test]
    fn sccache_navigation_uses_full_bindings_chords_and_resets_missed_prefixes() {
        let mut app = configured_app("[navigation]\ndown = [\"ctrl-j\", \"n n\"]\nhome = \"g g\"");
        let focus = *app.framework.focused();
        press(&mut app, "down");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 0);
        press(&mut app, "ctrl-j");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 1);
        press(&mut app, "g");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 1);
        press(&mut app, "g");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 0);
        press(&mut app, "n x n");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 0);
        press(&mut app, "n");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 1);
        press(&mut app, "ctrl-up");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 1);
        press(&mut app, "up");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 0);
        press(&mut app, "end");
        assert_eq!(app.overlays.sccache_pane.viewport.pos(), 1);
        assert_eq!(*app.framework.focused(), focus);
        assert!(
            app.pending_nav_chord.is_empty(),
            "handler may clear its buffer without losing overlay chords"
        );
    }

    #[derive(Default)]
    struct Clipboard {
        writes: Vec<String>,
    }
    impl ClipboardBackend for Clipboard {
        fn write_clipboard(&mut self, text: &str) -> Result<(), ClipboardError> {
            self.writes.push(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn sccache_copy_uses_its_selection_for_primary_secondary_and_chord_bindings() {
        let mut app = configured_app("[global]\ncopy = [\"ctrl-y\", \"v v\", \"Y\"]");
        let focus = *app.framework.focused();
        let mut clipboard = Clipboard::default();
        for sequence in ["y", "ctrl-y", "v v", "Y"] {
            for key in KeySequence::parse(sequence).expect("copy binding").keys() {
                assert!(dispatch_with_clipboard(&mut app, key, &mut clipboard));
            }
        }
        assert_eq!(clipboard.writes, ["7", "7", "7"]);
        app.overlays.sccache_pane.navigate(NavAction::End);
        dispatch_with_clipboard(&mut app, &KeyBind::ctrl('y'), &mut clipboard);
        assert_eq!(clipboard.writes.last().map(String::as_str), Some("3"));
        app.overlays.sccache_pane.set_selectable_lines(Vec::new());
        dispatch_with_clipboard(&mut app, &KeyBind::ctrl('y'), &mut clipboard);
        assert_eq!(clipboard.writes.len(), 4);
        assert_eq!(*app.framework.focused(), focus);
    }

    #[test]
    fn sccache_chord_state_clears_on_request_close_and_miss() {
        let mut app = configured_app("[sccache]\ncancel = \"z z\"");
        press(&mut app, "z x");
        assert_eq!(
            app.overlays.sccache_pane.pending_chord,
            [] as [tui_pane::KeyBind; 0]
        );
        press(&mut app, "z");
        app.overlays.sccache_pane.start_loading();
        press(&mut app, "z");
        assert!(app.overlays.is_sccache_open());
        app.overlays.close_sccache();
        assert!(!dispatch_sccache_overlay(
            &mut app,
            &KeyBind::from(KeyCode::Esc)
        ));
        assert_eq!(
            app.overlays.sccache_pane.pending_chord,
            [] as [tui_pane::KeyBind; 0]
        );
        assert_eq!(KeyBind::from(KeyCode::Esc).mods, KeyModifiers::NONE);
    }
}
