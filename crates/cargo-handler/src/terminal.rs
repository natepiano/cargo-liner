//! Starting the grid: load its configuration, build the app, and run it
//! under [`tui_pane::run_terminal`] with the grid's motion, the attract
//! screen's frames and the favorites overlay's fades as the work the
//! loop polls.

use std::process::ExitCode;
use std::sync::mpsc::Receiver;
use std::time::Instant;

use tui_pane::PollWork;
use tui_pane::Repaint;
use tui_pane::install_theme;
use tui_pane::run_terminal;

use crate::app::App;
use crate::census;
use crate::census::CensusUpdate;
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
    run_terminal(&mut app, &iterm2_profile, |app| {
        Ticker::new(census::schedule::spawn(app.remote_machines.clone()))
    })
}

/// What the loop folds in on every pass besides input: the census
/// answers, the ages ticking, the favorites overlay's removal fade, the
/// grid's motion and the attract screen's frames.
struct Ticker {
    /// Answers from the census scheduler.
    updates:      Receiver<CensusUpdate>,
    /// The unix second the ages were last drawn at, while any row is
    /// shown.
    shown_second: Option<u64>,
}

impl Ticker {
    /// Poll `updates` for census answers.
    const fn new(updates: Receiver<CensusUpdate>) -> Self {
        Self {
            updates,
            shown_second: None,
        }
    }

    /// Fold in the census answers that have arrived, answering whether
    /// any changed what the summary shows. An answer from a host taken
    /// out of the list since its probe went out is dropped.
    fn drain_census(&self, app: &mut App) -> bool {
        let mut changed = false;
        while let Ok(update) = self.updates.try_recv() {
            if let CensusUpdate::Remote { host, .. } = &update
                && !app.loaded_config.config.machines.remote.contains(host)
            {
                continue;
            }
            changed |= app.census.apply(update);
        }
        changed
    }

    /// Whether the ages need drawing again: some row is shown and the
    /// unix second has moved on from `unix_now` since they were drawn.
    fn ages_moved(&mut self, app: &App, unix_now: u64) -> bool {
        let shown = app
            .census
            .machines(&app.loaded_config.config.machines.remote)
            .iter()
            .any(|machine| !machine.state.rows().is_empty());
        if !shown || self.shown_second == Some(unix_now) {
            return false;
        }
        self.shown_second = Some(unix_now);
        true
    }

    /// [`PollWork::poll`] at `unix_now` in unix seconds.
    fn poll_at(&mut self, app: &mut App, now: Instant, unix_now: u64) -> Repaint {
        let census_changed = self.drain_census(app);
        let ages_moved = self.ages_moved(app, unix_now);
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
        if census_changed
            || ages_moved
            || favorites == Repaint::Needed
            || grid_moving
            || attract == Repaint::Needed
        {
            Repaint::Needed
        } else {
            Repaint::NotNeeded
        }
    }
}

impl PollWork<App> for Ticker {
    fn poll(&mut self, app: &mut App, now: Instant) -> Repaint {
        self.poll_at(app, now, census::unix_now())
    }
}

#[cfg(test)]
#[expect(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::mpsc;

    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::layout::Rect;
    use tui_pane::FrameworkOverlayId;
    use tui_pane::dispatch_key;

    use super::*;
    use crate::census::Agent;
    use crate::census::AgentRow;

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
        let growth = app.loaded_config.config.tiles.growth();
        app.tiles.set_layout(Rect::new(0, 0, 80, 23), growth);
        let (_sender, receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver);
        assert_eq!(ticker.poll(&mut app, Instant::now()), Repaint::NotNeeded);

        dispatch_key(&mut app, key(KeyCode::Char('+')));

        assert_eq!(ticker.poll(&mut app, Instant::now()), Repaint::Needed);
    }

    /// While a row is shown each new unix second repaints so the ages
    /// tick, and a census answer that changes something repaints at
    /// once.
    #[test]
    fn shown_rows_repaint_each_second_and_on_a_changed_answer() {
        let mut app = App::new_for_test().expect("test app should build");
        let (sender, receiver) = mpsc::channel();
        let mut ticker = Ticker::new(receiver);
        let now = Instant::now();
        let row = AgentRow {
            agent:       Agent::Claude,
            name:        "enh/handler".to_string(),
            status:      Some("busy".to_string()),
            started:     90,
            pid:         428_044,
            desktop:     None,
            directory:   "~/rust/handler".to_string(),
            launched_by: None,
            children:    Vec::new(),
        };
        sender
            .send(CensusUpdate::Local(vec![row.clone()]))
            .expect("the ticker should hold the receiver");

        assert_eq!(ticker.poll_at(&mut app, now, 100), Repaint::Needed);
        assert_eq!(ticker.poll_at(&mut app, now, 100), Repaint::NotNeeded);
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::Needed);

        sender
            .send(CensusUpdate::Local(vec![row.clone()]))
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::NotNeeded);
        sender
            .send(CensusUpdate::Local(vec![census::AgentRow {
                status: None,
                ..row
            }]))
            .expect("the ticker should hold the receiver");
        assert_eq!(ticker.poll_at(&mut app, now, 101), Repaint::Needed);
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
