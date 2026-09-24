//! The keys that steer the moving band.
//!
//! [`MovingBandAction`] is the band's own scope, registered against
//! [`AttractHost::MOVING_BAND_PANE`] rather than against the app
//! globals. That is what keeps the keys per-animation: a second
//! [`AttractMode`](super::AttractMode) can bind the same `+` to
//! something else of its own, and `keymap.toml` gives each its own
//! table.
//!
//! The scope is consulted while the attract screen is what the display
//! is showing -- see [`Attract::keyed_mode`](super::Attract::keyed_mode)
//! -- whether it was asked for or came on over an idle grid. Only the
//! keys it binds are taken either way, so `s` still opens settings and
//! `a` still gives the grid back.

use std::marker::PhantomData;

use crossterm::event::KeyCode;

use super::AttractHost;
use super::constants::ATTRACT_MOVING_BAND_SCOPE;
use super::constants::ATTRACT_MOVING_BAND_SECTION;
use crate::Bindings;
use crate::Mode;
use crate::Pane;
use crate::Shortcuts;
use crate::TabStop;

crate::action_enum! {
    /// One thing a key asks the moving band to do.
    #[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
    pub enum MovingBandAction {
        /// Widen the band.
        Wider       => ("wider",        "Widen the band");
        /// Thin the band.
        Thinner     => ("thinner",      "Thin the band");
        /// Send the band left.
        TravelLeft  => ("travel_left",  "Send the band left");
        /// Send the band right.
        TravelRight => ("travel_right", "Send the band right");
        /// Send the band up.
        TravelUp    => ("travel_up",    "Send the band up");
        /// Send the band down.
        TravelDown  => ("travel_down",  "Send the band down");
        /// Speed the band up.
        Faster      => ("faster",       "Speed the band up");
        /// Slow the band down.
        Slower      => ("slower",       "Slow the band down");
        /// Cycle which of the band's edges fray.
        CycleFraying => ("cycle_fraying", "Cycle which of the band's edges fray");
        /// Fray the trailing edge slower.
        TailSlower  => ("tail_slower",  "Fray the trailing edge slower");
        /// Fray the trailing edge faster.
        TailFaster  => ("tail_faster",  "Fray the trailing edge faster");
        /// Show the moving band.
        ShowMovingBand => ("show_moving_band", "Show the moving band");
        /// Show the moving text.
        ShowMovingText => ("show_moving_text", "Show the moving text");
        /// Show the pixelate screen.
        ShowPixelate   => ("show_pixelate",    "Show the pixelate screen");
    }
}

/// The keymap scope the moving band's keys live in.
///
/// A [`Pane`] with no rectangle of its own: the attract screen is drawn
/// over the whole terminal by the app's frame, and this exists so the
/// framework has somewhere to hang a scope that is neither the app
/// globals nor the tile grid. `A` is the app hosting it, which
/// registers `MovingBandPane::<A>::default()` with its keymap.
pub struct MovingBandPane<A>(PhantomData<fn() -> A>);

impl<A> Default for MovingBandPane<A> {
    fn default() -> Self { Self(PhantomData) }
}

impl<A: AttractHost> Pane<A> for MovingBandPane<A> {
    const APP_PANE_ID: A::AppPaneId = A::MOVING_BAND_PANE;

    /// The band is steered, not scrolled, so the status line's
    /// navigation region has nothing to say about it.
    fn mode() -> fn(&A) -> Mode<A> { |_app| Mode::Static }

    /// Never a Tab stop. Tab walks the tile grid, and the grid is what
    /// the attract screen is covering -- a Tab that landed here would
    /// take the focus ring somewhere the reader cannot see it.
    fn tab_stop() -> TabStop<A> { TabStop::never() }
}

impl<A: AttractHost> Shortcuts<A> for MovingBandPane<A> {
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
    /// `1`, `2` and `3` name the animations in the order they were
    /// written. Every scope binds all three, so each is reachable from
    /// the others and none is a door that only opens one way; pressing
    /// the one already showing does nothing.
    fn defaults() -> Bindings<Self::Actions> { default_bindings() }

    fn dispatcher() -> fn(Self::Actions, &mut A) { dispatch::<A> }
}

/// Run one moving-band action.
fn dispatch<A: AttractHost>(action: MovingBandAction, app: &mut A) {
    app.attract_mut().moving_band(action);
}

/// The keys the moving band is steered with until `keymap.toml` says otherwise.
fn default_bindings() -> Bindings<MovingBandAction> {
    crate::bindings! {
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
        '3' => MovingBandAction::ShowPixelate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KeyBind;

    /// The arrows point the way the band is sent, asserted apart from
    /// the character keys because they are the pair the grid underneath
    /// also spends.
    #[test]
    fn the_arrows_point_the_way_the_band_is_sent() {
        let scope = default_bindings().into_scope_map();
        let cases = [
            (KeyCode::Left, MovingBandAction::TravelLeft),
            (KeyCode::Right, MovingBandAction::TravelRight),
            (KeyCode::Up, MovingBandAction::TravelUp),
            (KeyCode::Down, MovingBandAction::TravelDown),
        ];

        for (key, action) in cases {
            assert_eq!(
                scope.action_for(&KeyBind::from(key)),
                Some(action),
                "{key:?} should send the band",
            );
        }
    }

    /// Every steering key resolves to the action it is meant to, read
    /// out of the table the keymap is actually built from.
    #[test]
    fn the_steering_keys_resolve_to_their_actions() {
        let scope = default_bindings().into_scope_map();
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
            ('3', MovingBandAction::ShowPixelate),
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
