//! The keys that steer the moving band.
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
        ShowMovingBand => ("show_moving_band", "Show the moving band");
        ShowMovingText => ("show_moving_text", "Show the moving text");
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
    ///
    /// `1` and `2` name the animations in the order they were written.
    /// Both scopes bind both, so either is reachable from the other and
    /// neither is a door that only opens one way; pressing the one
    /// already showing does nothing.
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
            '1' => MovingBandAction::ShowMovingBand,
            '2' => MovingBandAction::ShowMovingText,
        }
    }

    fn dispatcher() -> fn(Self::Actions, &mut App) { dispatch }
}

/// Run one moving-band action.
fn dispatch(action: MovingBandAction, app: &mut App) { app.attract.moving_band(action); }

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
            ('1', MovingBandAction::ShowMovingBand),
            ('2', MovingBandAction::ShowMovingText),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key} should steer the band",
            );
        }
    }
}
