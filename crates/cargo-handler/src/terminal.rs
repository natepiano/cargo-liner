//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with the grid's motion, the attract
//! screen's frames and the favorites overlay's fades as the work the
//! loop polls.

use std::process::ExitCode;
use std::time::Instant;

use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::install_theme;
use tui_pane::run_terminal;

use crate::app::App;
use crate::config::CargoHandler;
use crate::config::LoadedConfig;
use crate::constants::BINARY_NAME;
use crate::theme;

/// Load configuration, install the theme, build the keymap, and run the
/// event loop with the terminal in the alternate screen.
pub(crate) fn run() -> ExitCode {
    let loaded_config = LoadedConfig::load::<CargoHandler>();
    let startup_note = install_theme(&loaded_config.config.appearance, theme::builtins());
    // Read before the config is handed to the app, which takes it.
    let iterm2_profile = loaded_config.config.appearance.iterm2_profile.clone();
    let mut app = match App::new(loaded_config, startup_note) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("{BINARY_NAME}: keymap: {error}");
            return ExitCode::FAILURE;
        },
    };
    run_terminal(&mut app, &iterm2_profile, |_| Ticker)
}

/// What the loop folds in on every pass besides input: the favorites
/// overlay's removal fade, the grid's motion and the attract screen's
/// frames. None of it arrives from outside, so there is no channel to
/// hold.
struct Ticker;

impl PollWork<App> for Ticker {
    fn poll(&mut self, app: &mut App, now: Instant) -> Repaint {
        // Each of the three is asked every pass: asking is what moves it
        // on, so none may be skipped because another already wants a
        // frame.
        let favorites = tui_pane::poll_favorites(app, now);
        // A grid in motion repaints every poll until it settles.
        let grid_moving = app.tiles.tick();
        // The attract screen asks for frames on its own cadence while
        // it is on the screen, and for one more at the end of the quiet
        // it waits out before coming back.
        let attract = app.attract.frame_due();
        if favorites == Repaint::Needed || grid_moving || attract == Repaint::Needed {
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        }
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Rect;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::dispatch_key;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent { KeyEvent::new(code, KeyModifiers::NONE) }

    fn ctrl(character: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(character), KeyModifiers::CONTROL)
    }

    /// An idle app with a settled grid costs the loop nothing, and `+`
    /// sets the grid moving, which does. The grid is laid out by hand,
    /// as a drawn frame would leave it, so there is room for the cell.
    #[test]
    fn the_ticker_repaints_only_while_something_moves() {
        let mut app = App::new_for_test().expect("test app should build");
        let initial_rows = app.loaded_config.config.tiles.initial_rows();
        app.tiles.set_layout(Rect::new(0, 0, 80, 23), initial_rows);
        assert_eq!(Ticker.poll(&mut app, Instant::now()), Repaint::NotNeeded);

        dispatch_key(&mut app, key(KeyCode::Char('+')));

        assert_eq!(Ticker.poll(&mut app, Instant::now()), Repaint::Needed);
    }

    /// The favorites modal owns the keyboard while it is open: `?`
    /// opens no framework overlay behind it, and `Esc` closes it.
    #[test]
    fn the_favorites_overlay_swallows_keys_until_escape() {
        let mut app = App::new_for_test().expect("test app should build");
        dispatch_key(&mut app, ctrl('o'));
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Char('?')));
        assert_eq!(app.framework.overlay(), None);
        assert!(app.favorites_overlay.is_open());

        dispatch_key(&mut app, key(KeyCode::Esc));
        assert!(!app.favorites_overlay.is_open());
    }

    /// `s`, ctrl-k and `?` each open their framework overlay, and `Esc`
    /// closes it.
    #[test]
    fn each_framework_overlay_opens_and_escape_closes_it() {
        let mut app = App::new_for_test().expect("test app should build");
        for (overlay, opener) in [
            (FrameworkOverlayId::Settings, key(KeyCode::Char('s'))),
            (FrameworkOverlayId::Keymap, ctrl('k')),
            (FrameworkOverlayId::GlobalShortcuts, key(KeyCode::Char('?'))),
        ] {
            dispatch_key(&mut app, opener);
            assert_eq!(app.framework.overlay(), Some(overlay));
            dispatch_key(&mut app, key(KeyCode::Esc));
            assert_eq!(app.framework.overlay(), None);
        }
    }
}
