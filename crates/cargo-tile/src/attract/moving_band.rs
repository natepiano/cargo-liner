//! The keys that steer the moving band, and the run of presses a held
//! key arrives as.
//!
//! [`MovingBandAction`] is the band's own scope, registered against
//! [`AppPaneId::Attract`] rather than against the app globals. That is
//! what keeps the keys per-animation: a second [`AttractMode`] can bind
//! the same `+` to something else of its own, and `keymap.toml` gives
//! each its own table.
//!
//! The scope is only consulted while the attract screen has been asked
//! for outright -- see [`Attract::keyed_mode`](super::Attract::keyed_mode).
//! Left to come on by itself over an idle grid, the animation is
//! decoration and the ordinary keys keep their ordinary meanings.

use std::time::Instant;

use crossterm::event::KeyCode;
use tui_pane::Bindings;
use tui_pane::Mode;
use tui_pane::Pane;
use tui_pane::Shortcuts;
use tui_pane::TabStop;

use super::AttractMode;
use crate::app::App;
use crate::app::AppPaneId;
use crate::constants::ATTRACT_MOVING_BAND_SCOPE;
use crate::constants::ATTRACT_MOVING_BAND_SECTION;
use crate::constants::HELD_KEY_GAP;
use crate::constants::HELD_KEY_MAX_STEP;
use crate::constants::HELD_KEY_PRESSES_PER_STEP;

tui_pane::action_enum! {
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub(crate) enum MovingBandAction {
        Wider       => ("wider",        "Widen the band");
        Thinner     => ("thinner",      "Thin the band");
        TravelLeft  => ("travel_left",  "Send the band left");
        TravelRight => ("travel_right", "Send the band right");
        TravelUp    => ("travel_up",    "Send the band up");
        TravelDown  => ("travel_down",  "Send the band down");
        Faster      => ("faster",       "Speed the band up");
        Slower      => ("slower",       "Slow the band down");
        CycleFraying => ("cycle_fraying", "Cycle which of the band's edges fray");
        TailSlower  => ("tail_slower",  "Fray the trailing edge slower");
        TailFaster  => ("tail_faster",  "Fray the trailing edge faster");
    }
}

/// The keymap scope the moving band's keys live in.
///
/// A [`Pane`] with no rectangle of its own: the attract screen is drawn
/// over the whole terminal by [`crate::render`], and this exists so the
/// framework has somewhere to hang a scope that is neither the app
/// globals nor the tile grid.
pub(crate) struct MovingBandPane;

impl Pane<App> for MovingBandPane {
    const APP_PANE_ID: AppPaneId = AppPaneId::Attract(AttractMode::MovingBand);

    /// The band is steered, not scrolled, so the status line's
    /// navigation region has nothing to say about it.
    fn mode() -> fn(&App) -> Mode<App> { |_app| Mode::Static }

    /// Never a Tab stop. Tab walks the tile grid, and the grid is what
    /// the attract screen is covering -- a Tab that landed here would
    /// take the focus ring somewhere the reader cannot see it.
    fn tab_stop() -> TabStop<App> { TabStop::never() }
}

impl Shortcuts<App> for MovingBandPane {
    type Actions = MovingBandAction;

    const SCOPE_NAME: &'static str = ATTRACT_MOVING_BAND_SCOPE;
    const SECTION_NAME: &'static str = ATTRACT_MOVING_BAND_SECTION;

    /// An arrow key points the way the band is being sent, which is
    /// the one mapping nobody has to be told. `<` and `>` are the
    /// slower and faster pair a video player uses, and `,` and `.`
    /// are the same two keys unshifted, so neither hand position is
    /// wrong. `+` and `-` widen and thin it, with `=` standing in for
    /// `+` on the key it shares -- the same pair that opens and closes
    /// a tile on the grid underneath, and both readings are "more of
    /// this, less of this".
    ///
    /// `v` for varying is the one letter here, and it is one the app
    /// has never spent: every other lowercase letter keeps its ordinary
    /// meaning while the screen is up -- `f` still freezes, `s` still
    /// opens settings, `a` still gives the grid back. `[` and `]` are
    /// the third matched pair, and they slow and speed the fraying `v`
    /// cycles through, which is why they sit beside it in the listing.
    fn defaults() -> Bindings<Self::Actions> {
        tui_pane::bindings! {
            ['+', '='] => MovingBandAction::Wider,
            '-' => MovingBandAction::Thinner,
            KeyCode::Left => MovingBandAction::TravelLeft,
            KeyCode::Right => MovingBandAction::TravelRight,
            KeyCode::Up => MovingBandAction::TravelUp,
            KeyCode::Down => MovingBandAction::TravelDown,
            ['>', '.'] => MovingBandAction::Faster,
            ['<', ','] => MovingBandAction::Slower,
            'v' => MovingBandAction::CycleFraying,
            '[' => MovingBandAction::TailSlower,
            ']' => MovingBandAction::TailFaster,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one moving-band action.
fn dispatch(action: MovingBandAction, app: &mut App) { app.attract.moving_band(action); }

/// How far into a run of presses of the same key the reader is.
///
/// A terminal reports a held key as a run of presses arriving a few
/// tens of milliseconds apart rather than as a press and a release, so
/// there is nothing to ask how long a key has been down for -- the run
/// itself is the measurement. Presses arriving inside [`HELD_KEY_GAP`]
/// of each other continue the run; anything slower, or a different
/// action, starts a fresh one.
///
/// What the run buys is size: a key held down moves the band further
/// per press the longer it is held, so crossing the whole range of
/// widths or speeds does not cost sixty presses.
#[derive(Debug)]
pub(crate) struct HeldKey {
    /// The action the run is made of, or [`None`] before the first
    /// press of the session.
    action:     Option<MovingBandAction>,
    /// When the last press of the run arrived.
    pressed_at: Instant,
    /// How many presses into the run the last one was.
    presses:    u32,
}

impl HeldKey {
    /// A run that has not started.
    pub(crate) fn new() -> Self {
        Self {
            action:     None,
            pressed_at: Instant::now(),
            presses:    0,
        }
    }

    /// Fold a press of `action` arriving at `pressed_at` into the run,
    /// and say how many steps it is worth.
    ///
    /// Never fewer than one, so a single press always does something,
    /// and never more than [`HELD_KEY_MAX_STEP`], so a key left down
    /// settles into a steady climb rather than running away from the
    /// reader.
    pub(crate) fn step(&mut self, action: MovingBandAction, pressed_at: Instant) -> u32 {
        let continuing = self.action == Some(action)
            && pressed_at.duration_since(self.pressed_at) <= HELD_KEY_GAP;
        self.presses = if continuing {
            self.presses.saturating_add(1)
        } else {
            1
        };
        self.action = Some(action);
        self.pressed_at = pressed_at;
        (self.presses / HELD_KEY_PRESSES_PER_STEP).clamp(1, HELD_KEY_MAX_STEP)
    }
}

#[cfg(test)]
mod tests {
    use tui_pane::KeyBind;

    use super::*;

    /// Every steering key resolves to the action it is meant to, read
    /// out of the table the keymap is actually built from.
    #[test]
    fn the_steering_keys_resolve_to_their_actions() {
        let scope = MovingBandPane::defaults().into_scope_map();
        let cases = [
            ('v', MovingBandAction::CycleFraying),
            ('[', MovingBandAction::TailSlower),
            (']', MovingBandAction::TailFaster),
            ('+', MovingBandAction::Wider),
            ('=', MovingBandAction::Wider),
            ('-', MovingBandAction::Thinner),
            ('>', MovingBandAction::Faster),
            ('<', MovingBandAction::Slower),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key} should steer the band",
            );
        }
    }

    /// A gap short enough to read as the same key still being held.
    const HELD: std::time::Duration = HELD_KEY_GAP;

    #[test]
    fn a_single_press_is_worth_one_step() {
        let mut held_key = HeldKey::new();

        assert_eq!(held_key.step(MovingBandAction::Wider, Instant::now()), 1);
    }

    /// A run of presses is worth more per press the longer it runs, so
    /// a key held down crosses the range without sixty presses.
    #[test]
    fn a_run_of_presses_grows_the_step_and_then_stops_growing() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        let mut steps = Vec::new();

        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * (HELD_KEY_MAX_STEP + 2)) {
            steps.push(held_key.step(MovingBandAction::Wider, pressed_at));
            pressed_at += HELD;
        }

        assert_eq!(steps.first(), Some(&1));
        assert_eq!(steps.last(), Some(&HELD_KEY_MAX_STEP));
        assert!(
            steps.windows(2).all(|pair| pair[0] <= pair[1]),
            "the step should only ever grow within one run: {steps:?}",
        );
    }

    /// Turning to a different key starts the run over. Otherwise a long
    /// hold on `+` would leave the next press of `-` taking eight steps
    /// back the other way.
    #[test]
    fn a_different_key_starts_the_run_over() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * HELD_KEY_MAX_STEP) {
            held_key.step(MovingBandAction::Wider, pressed_at);
            pressed_at += HELD;
        }

        assert_eq!(held_key.step(MovingBandAction::Thinner, pressed_at), 1);
    }

    /// A press that arrives after the reader has let go is the start of
    /// a new run, not the continuation of the old one.
    #[test]
    fn a_press_after_the_gap_starts_the_run_over() {
        let mut held_key = HeldKey::new();
        let mut pressed_at = Instant::now();
        for _ in 0..(HELD_KEY_PRESSES_PER_STEP * HELD_KEY_MAX_STEP) {
            held_key.step(MovingBandAction::Wider, pressed_at);
            pressed_at += HELD;
        }

        let let_go = pressed_at + HELD_KEY_GAP * 2;

        assert_eq!(held_key.step(MovingBandAction::Wider, let_go), 1);
    }
}
